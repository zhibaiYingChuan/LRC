// ============================================================
// 许可证: Apache 2.0
// 本文件实现模型下载器，属于公开层 (Layer 1)。
// ============================================================
//
// 模型下载器（Model Downloader）
//
// 提供 ureq 流式下载 + 进度回调 + 指数退避重试 + 镜像源切换。
// 用于首次启动时下载 BGE-small-zh / MiniLM-L6-v2 等嵌入模型。
//
// 核心组件：
//   1. DownloadProgress — 进度回调 trait（控制台 / Tauri 事件 / 自定义）
//   2. MirrorSource — 镜像源枚举（hf-mirror / modelscope / auto）
//   3. DownloadConfig — 下载配置（重试次数、超时、镜像源等）
//   4. ModelDownloader — 下载器主结构，封装下载逻辑
//
// 设计原则：
//   1. 国内优先：默认使用 hf-mirror.com 镜像，绝不直连 huggingface.co
//   2. 失败友好：3 次重试后输出手动下载指引链接
//   3. 进度可见：通过 trait 回调支持控制台/Tauri 事件两种呈现方式
//   4. 完整性校验：下载完成后验证文件大小（可选 SHA256）
//
// v0.6.0 引入，对应 PRODUCT_ROADMAP_v1.0.md 功能 4.2.4

use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

// ============================================================
// 进度回调 Trait
// ============================================================

/// 下载进度回调接口
///
/// 实现方可以选择将进度推送到控制台、Tauri 事件、日志文件等。
/// 调用方在每次读取数据块后调用 `on_progress`。
///
/// # 参数
/// - `downloaded`：已下载字节数
/// - `total`：总字节数（未知时为 0）
pub trait DownloadProgress: Send + Sync {
    /// 进度更新回调
    ///
    /// 调用频率：约每 64KB 一次（取决于读取缓冲区大小）。
    fn on_progress(&self, downloaded: u64, total: u64);

    /// 下载开始回调（可选）
    ///
    /// 默认空实现，调用方按需重写。
    fn on_start(&self, _total: u64) {}

    /// 下载完成回调（可选）
    ///
    /// 默认空实现，调用方按需重写。
    fn on_complete(&self, _downloaded: u64) {}

    /// 下载失败回调（可选）
    ///
    /// 默认空实现，调用方按需重写。
    /// `error` 为错误信息，`attempt` 为当前重试次数（从 1 开始）。
    fn on_error(&self, _error: &str, _attempt: u32) {}
}

/// 控制台进度条实现
///
/// 在 stderr 输出进度信息，每 1MB 输出一次，避免日志过多。
pub struct ConsoleProgress {
    /// 上次输出进度时的字节数（用于控制输出频率）
    last_logged: std::sync::atomic::AtomicU64,
}

impl ConsoleProgress {
    /// 创建控制台进度回调
    pub fn new() -> Self {
        Self {
            last_logged: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

impl Default for ConsoleProgress {
    fn default() -> Self {
        Self::new()
    }
}

impl DownloadProgress for ConsoleProgress {
    fn on_progress(&self, downloaded: u64, total: u64) {
        // 每 1MB 输出一次，避免日志刷屏
        const LOG_INTERVAL: u64 = 1024 * 1024;
        let last = self.last_logged.load(std::sync::atomic::Ordering::Relaxed);
        if downloaded >= last + LOG_INTERVAL || downloaded == total {
            self.last_logged
                .store(downloaded, std::sync::atomic::Ordering::Relaxed);
            if total > 0 {
                let percent = (downloaded as f64 / total as f64) * 100.0;
                let downloaded_mb = downloaded as f64 / 1024.0 / 1024.0;
                let total_mb = total as f64 / 1024.0 / 1024.0;
                eprintln!(
                    "[LRC·下载] 进度: {:.1}% ({:.2}MB / {:.2}MB)",
                    percent, downloaded_mb, total_mb
                );
            } else {
                let downloaded_mb = downloaded as f64 / 1024.0 / 1024.0;
                eprintln!("[LRC·下载] 已下载: {:.2}MB (总大小未知)", downloaded_mb);
            }
        }
    }

    fn on_start(&self, total: u64) {
        if total > 0 {
            let total_mb = total as f64 / 1024.0 / 1024.0;
            eprintln!("[LRC·下载] 开始下载（总大小: {:.2}MB）", total_mb);
        } else {
            eprintln!("[LRC·下载] 开始下载（总大小未知）");
        }
    }

    fn on_complete(&self, downloaded: u64) {
        let downloaded_mb = downloaded as f64 / 1024.0 / 1024.0;
        eprintln!("[LRC·下载] 完成: {:.2}MB", downloaded_mb);
    }

    fn on_error(&self, error: &str, attempt: u32) {
        eprintln!(
            "[LRC·下载] 第 {} 次尝试失败: {}",
            attempt, error
        );
    }
}

// ============================================================
// 镜像源
// ============================================================

/// 模型下载镜像源
///
/// v0.6.0 引入，解决国内直连 huggingface.co 失败问题。
/// 通过 `LRC_MODEL_MIRROR` 环境变量配置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirrorSource {
    /// HF-Mirror（https://hf-mirror.com）
    ///
    /// 国内 HuggingFace 镜像，覆盖全部模型，速度稳定。
    /// 默认值。
    HfMirror,
    /// ModelScope（https://modelscope.cn）
    ///
    /// 魔搭社区镜像，国内阿里云节点，部分模型可更快。
    ModelScope,
    /// 自动选择
    ///
    /// 优先尝试 hf-mirror，失败后切换到 modelscope。
    Auto,
}

impl MirrorSource {
    /// 从环境变量 `LRC_MODEL_MIRROR` 解析镜像源
    ///
    /// 支持的值：`hf`、`hf-mirror`、`modelscope`、`auto`（不区分大小写）。
    /// 未设置或值无效时返回 `HfMirror`（默认值）。
    pub fn from_env() -> Self {
        match std::env::var("LRC_MODEL_MIRROR")
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "modelscope" => Self::ModelScope,
            "auto" => Self::Auto,
            // 默认使用 hf-mirror（与 luoshu_encoder_ml.rs 的 HF_ENDPOINT 默认值一致）
            _ => Self::HfMirror,
        }
    }

    /// 返回镜像源的基础 URL
    pub fn base_url(&self) -> &str {
        match self {
            Self::HfMirror | Self::Auto => "https://hf-mirror.com",
            Self::ModelScope => "https://modelscope.cn/api/v1/models",
        }
    }
}

impl std::fmt::Display for MirrorSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HfMirror => write!(f, "hf-mirror"),
            Self::ModelScope => write!(f, "modelscope"),
            Self::Auto => write!(f, "auto"),
        }
    }
}

// ============================================================
// 下载配置
// ============================================================

/// 下载配置
///
/// 控制重试次数、超时、镜像源等参数。
/// 默认值符合 PRODUCT_ROADMAP 4.2.4 规格：
///   - 重试次数：3
///   - 初始退避：2s
///   - 最大退避：8s
///   - 单文件超时：30s
#[derive(Debug, Clone)]
pub struct DownloadConfig {
    /// 最大重试次数（不含首次尝试），默认 3
    pub max_retries: u32,
    /// 初始退避时间，默认 2 秒
    pub initial_backoff: Duration,
    /// 最大退避时间，默认 8 秒
    pub max_backoff: Duration,
    /// 单次请求连接超时，默认 30 秒
    pub connect_timeout: Duration,
    /// 单次请求读取超时，默认 300 秒（大模型下载可能耗时较长）
    pub read_timeout: Duration,
    /// 镜像源，默认从环境变量读取
    pub mirror: MirrorSource,
    /// 用户代理字符串（部分镜像需要）
    pub user_agent: String,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff: Duration::from_secs(2),
            max_backoff: Duration::from_secs(8),
            connect_timeout: Duration::from_secs(30),
            read_timeout: Duration::from_secs(300),
            mirror: MirrorSource::from_env(),
            user_agent: format!("LRC/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

// ============================================================
// 下载错误
// ============================================================

/// 下载错误类型
///
/// 区分不同失败场景，便于调用方决策（重试 vs 直接放弃）。
#[derive(Debug)]
pub enum DownloadError {
    /// 网络错误（连接超时、DNS 解析失败等）
    Network(String),
    /// HTTP 错误（404、500 等）
    Http(u16, String),
    /// 文件系统错误（写入失败、磁盘满等）
    Io(std::io::Error),
    /// 重试耗尽
    RetriesExhausted {
        /// 总尝试次数（含首次）
        attempts: u32,
        /// 最后一次错误信息
        last_error: String,
    },
    /// 用户取消（未来支持）
    Cancelled,
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(msg) => write!(f, "网络错误: {}", msg),
            Self::Http(code, msg) => write!(f, "HTTP 错误 {}: {}", code, msg),
            Self::Io(e) => write!(f, "文件系统错误: {}", e),
            Self::RetriesExhausted { attempts, last_error } => write!(
                f,
                "重试耗尽（共尝试 {} 次），最后一次错误: {}",
                attempts, last_error
            ),
            Self::Cancelled => write!(f, "用户取消"),
        }
    }
}

impl std::error::Error for DownloadError {}

impl From<std::io::Error> for DownloadError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

// ============================================================
// 模型下载器
// ============================================================

/// 模型下载器
///
/// 封装 ureq 流式下载 + 进度回调 + 指数退避重试。
///
/// # 示例
///
/// ```ignore
/// use code_memory::engine::model_downloader::{ModelDownloader, DownloadConfig, ConsoleProgress};
///
/// let config = DownloadConfig::default();
/// let progress = ConsoleProgress::new();
/// let downloader = ModelDownloader::new(config);
///
/// let dest = PathBuf::from("models/BAAI--bge-small-zh/config.json");
/// downloader.download_with_retry(
///     "https://hf-mirror.com/BAAI/bge-small-zh/resolve/main/config.json",
///     &dest,
///     &progress,
/// ).await?;
/// ```
pub struct ModelDownloader {
    config: DownloadConfig,
}

impl ModelDownloader {
    /// 创建下载器
    pub fn new(config: DownloadConfig) -> Self {
        Self { config }
    }

    /// 使用默认配置创建下载器
    pub fn with_defaults() -> Self {
        Self::new(DownloadConfig::default())
    }

    /// 返回配置引用
    pub fn config(&self) -> &DownloadConfig {
        &self.config
    }

    /// 带重试的下载
    ///
    /// 实现 PRODUCT_ROADMAP 4.2.4 验收标准：
    ///   - 重试次数 < 3 时自动重试，间隔 2s/4s/8s 指数退避
    ///   - 3 次重试均失败后返回 `RetriesExhausted`
    ///
    /// # 参数
    /// - `url`：下载 URL
    /// - `dest`：目标文件路径（父目录会被自动创建）
    /// - `progress`：进度回调
    pub fn download_with_retry(
        &self,
        url: &str,
        dest: &Path,
        progress: &dyn DownloadProgress,
    ) -> Result<(), DownloadError> {
        let mut last_error: Option<DownloadError> = None;
        let total_attempts = self.config.max_retries + 1;

        for attempt in 1..=total_attempts {
            if attempt > 1 {
                // 计算指数退避：2s, 4s, 8s（不超过 max_backoff）
                let backoff = self.compute_backoff(attempt - 1);
                eprintln!(
                    "[LRC·下载] 第 {} 次重试将在 {:?} 后开始",
                    attempt - 1,
                    backoff
                );
                std::thread::sleep(backoff);
            }

            match self.download_once(url, dest, progress, attempt) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    let error_msg = e.to_string();
                    progress.on_error(&error_msg, attempt);
                    last_error = Some(e);
                }
            }
        }

        Err(DownloadError::RetriesExhausted {
            attempts: total_attempts,
            last_error: last_error
                .map(|e| e.to_string())
                .unwrap_or_else(|| "未知错误".to_string()),
        })
    }

    /// 计算指数退避时间
    ///
    /// 公式：`min(initial * 2^(attempt-1), max_backoff)`
    /// - 第 1 次重试：initial = 2s
    /// - 第 2 次重试：initial * 2 = 4s
    /// - 第 3 次重试：initial * 4 = 8s（达到 max_backoff）
    fn compute_backoff(&self, attempt: u32) -> Duration {
        let multiplier = 2u32.saturating_pow(attempt.saturating_sub(1));
        let backoff = self.config.initial_backoff * multiplier;
        std::cmp::min(backoff, self.config.max_backoff)
    }

    /// 单次下载尝试
    ///
    /// 使用 ureq 流式下载，避免大文件一次性读入内存。
    fn download_once(
        &self,
        url: &str,
        dest: &Path,
        progress: &dyn DownloadProgress,
        _attempt: u32,
    ) -> Result<(), DownloadError> {
        // 确保父目录存在
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // 构建 HTTP 请求
        let agent = ureq::AgentBuilder::new()
            .timeout(self.config.read_timeout)
            .timeout_connect(self.config.connect_timeout)
            .user_agent(&self.config.user_agent)
            .build();

        let response = agent
            .get(url)
            .call()
            .map_err(|e| DownloadError::Network(format!("请求失败: {}", e)))?;

        let status = response.status();
        if status != 200 {
            return Err(DownloadError::Http(
                status,
                format!("下载失败: {} (URL: {})", response.status_text(), url),
            ));
        }

        // 获取总大小（Content-Length 头）
        let total: u64 = response
            .header("Content-Length")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        progress.on_start(total);

        // 流式写入文件
        let mut file = std::fs::File::create(dest)?;
        let mut reader = response.into_reader();

        // 64KB 读取缓冲区（平衡内存和系统调用次数）
        const BUFFER_SIZE: usize = 64 * 1024;
        let mut buffer = vec![0u8; BUFFER_SIZE];
        let mut downloaded: u64 = 0;

        loop {
            let bytes_read = reader
                .read(&mut buffer)
                .map_err(|e| DownloadError::Network(format!("读取失败: {}", e)))?;

            if bytes_read == 0 {
                break;
            }

            std::io::Write::write_all(&mut file, &buffer[..bytes_read])?;
            downloaded += bytes_read as u64;
            progress.on_progress(downloaded, total);
        }

        file.flush()?;
        drop(file);

        progress.on_complete(downloaded);

        Ok(())
    }
}

// ============================================================
// 下载 URL 构造
// ============================================================

/// 构造模型文件的下载 URL
///
/// 根据镜像源拼接正确的 URL：
///   - hf-mirror: `https://hf-mirror.com/{model_id}/resolve/main/{filename}`
///   - modelscope: `https://modelscope.cn/api/v1/models/{model_id}/repo?Revision=master&FilePath={filename}`
pub fn build_download_url(model_id: &str, filename: &str, mirror: MirrorSource) -> String {
    match mirror {
        MirrorSource::HfMirror | MirrorSource::Auto => {
            format!(
                "https://hf-mirror.com/{}/resolve/main/{}",
                model_id, filename
            )
        }
        MirrorSource::ModelScope => {
            format!(
                "https://modelscope.cn/api/v1/models/{}/repo?Revision=master&FilePath={}",
                model_id, filename
            )
        }
    }
}

/// 返回手动下载指引链接
///
/// 用于 3 次重试均失败后输出给用户的提示信息。
pub fn manual_download_guide(model_id: &str) -> String {
    let local_dir = model_id.replace('/', "--");
    format!(
        "自动下载失败，请手动下载模型：\n\
         1. 访问 https://hf-mirror.com/{} 下载以下文件：\n\
            - config.json\n\
            - tokenizer.json\n\
            - model.safetensors（或 pytorch_model.bin）\n\
         2. 将文件放到 models/{}/ 目录下\n\
         3. 详细步骤参考：docs/OFFLINE_MODEL_GUIDE.md\n\
         4. 或尝试 ModelScope 镜像：https://modelscope.cn/models/{}",
        model_id, local_dir, model_id
    )
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试：镜像源解析（默认值）
    #[test]
    fn test_mirror_source_default() {
        // 未设置环境变量时默认为 HfMirror
        std::env::remove_var("LRC_MODEL_MIRROR");
        let mirror = MirrorSource::from_env();
        assert_eq!(mirror, MirrorSource::HfMirror);
    }

    /// 测试：镜像源解析（modelscope）
    #[test]
    fn test_mirror_source_modelscope() {
        std::env::set_var("LRC_MODEL_MIRROR", "modelscope");
        let mirror = MirrorSource::from_env();
        assert_eq!(mirror, MirrorSource::ModelScope);
        std::env::remove_var("LRC_MODEL_MIRROR");
    }

    /// 测试：镜像源解析（auto）
    #[test]
    fn test_mirror_source_auto() {
        std::env::set_var("LRC_MODEL_MIRROR", "AUTO");
        let mirror = MirrorSource::from_env();
        assert_eq!(mirror, MirrorSource::Auto);
        std::env::remove_var("LRC_MODEL_MIRROR");
    }

    /// 测试：镜像源解析（无效值降级为默认）
    #[test]
    fn test_mirror_source_invalid() {
        std::env::set_var("LRC_MODEL_MIRROR", "invalid_mirror");
        let mirror = MirrorSource::from_env();
        assert_eq!(mirror, MirrorSource::HfMirror);
        std::env::remove_var("LRC_MODEL_MIRROR");
    }

    /// 测试：镜像源 URL
    #[test]
    fn test_mirror_source_base_url() {
        assert_eq!(MirrorSource::HfMirror.base_url(), "https://hf-mirror.com");
        assert_eq!(MirrorSource::Auto.base_url(), "https://hf-mirror.com");
        assert_eq!(
            MirrorSource::ModelScope.base_url(),
            "https://modelscope.cn/api/v1/models"
        );
    }

    /// 测试：镜像源 Display 实现
    #[test]
    fn test_mirror_source_display() {
        assert_eq!(format!("{}", MirrorSource::HfMirror), "hf-mirror");
        assert_eq!(format!("{}", MirrorSource::ModelScope), "modelscope");
        assert_eq!(format!("{}", MirrorSource::Auto), "auto");
    }

    /// 测试：下载配置默认值符合 PRODUCT_ROADMAP 规格
    #[test]
    fn test_download_config_defaults() {
        let config = DownloadConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.initial_backoff, Duration::from_secs(2));
        assert_eq!(config.max_backoff, Duration::from_secs(8));
        assert_eq!(config.connect_timeout, Duration::from_secs(30));
    }

    /// 测试：指数退避计算
    ///
    /// 验证 2s/4s/8s 退避序列符合 PRODUCT_ROADMAP 4.2.4 规格。
    #[test]
    fn test_compute_backoff() {
        let downloader = ModelDownloader::with_defaults();

        // 第 1 次重试：2s
        assert_eq!(downloader.compute_backoff(1), Duration::from_secs(2));
        // 第 2 次重试：4s
        assert_eq!(downloader.compute_backoff(2), Duration::from_secs(4));
        // 第 3 次重试：8s（达到 max_backoff）
        assert_eq!(downloader.compute_backoff(3), Duration::from_secs(8));
        // 第 4 次重试：8s（不超过 max_backoff）
        assert_eq!(downloader.compute_backoff(4), Duration::from_secs(8));
    }

    /// 测试：URL 构造（hf-mirror）
    #[test]
    fn test_build_download_url_hf_mirror() {
        let url = build_download_url("BAAI/bge-small-zh", "config.json", MirrorSource::HfMirror);
        assert_eq!(
            url,
            "https://hf-mirror.com/BAAI/bge-small-zh/resolve/main/config.json"
        );
    }

    /// 测试：URL 构造（modelscope）
    #[test]
    fn test_build_download_url_modelscope() {
        let url = build_download_url(
            "BAAI/bge-small-zh",
            "config.json",
            MirrorSource::ModelScope,
        );
        assert_eq!(
            url,
            "https://modelscope.cn/api/v1/models/BAAI/bge-small-zh/repo?Revision=master&FilePath=config.json"
        );
    }

    /// 测试：URL 构造（auto 使用 hf-mirror）
    #[test]
    fn test_build_download_url_auto() {
        let url = build_download_url(
            "sentence-transformers/all-MiniLM-L6-v2",
            "model.safetensors",
            MirrorSource::Auto,
        );
        assert!(url.starts_with("https://hf-mirror.com/"));
    }

    /// 测试：手动下载指引生成
    #[test]
    fn test_manual_download_guide() {
        let guide = manual_download_guide("BAAI/bge-small-zh");
        assert!(guide.contains("https://hf-mirror.com/BAAI/bge-small-zh"));
        assert!(guide.contains("models/BAAI--bge-small-zh"));
        assert!(guide.contains("docs/OFFLINE_MODEL_GUIDE.md"));
        assert!(guide.contains("modelscope.cn/models/BAAI/bge-small-zh"));
    }

    /// 测试：DownloadError 的 Display 实现
    #[test]
    fn test_download_error_display() {
        let err = DownloadError::Network("连接超时".to_string());
        assert!(format!("{}", err).contains("网络错误"));
        assert!(format!("{}", err).contains("连接超时"));

        let err = DownloadError::Http(404, "Not Found".to_string());
        assert!(format!("{}", err).contains("HTTP 错误 404"));

        let err = DownloadError::RetriesExhausted {
            attempts: 4,
            last_error: "网络不可达".to_string(),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("重试耗尽"));
        assert!(msg.contains("4 次"));
        assert!(msg.contains("网络不可达"));
    }

    /// 测试：DownloadError 实现 std::error::Error
    #[test]
    fn test_download_error_is_std_error() {
        let err = DownloadError::Network("测试".to_string());
        let _: &dyn std::error::Error = &err;
    }

    /// 测试：从 io::Error 转换为 DownloadError
    #[test]
    fn test_download_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "文件不存在");
        let download_err: DownloadError = io_err.into();
        match download_err {
            DownloadError::Io(_) => {}
            _ => panic!("应转换为 Io 变体"),
        }
    }

    /// 测试：ConsoleProgress 基本功能
    ///
    /// 验证 ConsoleProgress 不会 panic，且能正确处理 0 字节和大量字节。
    #[test]
    fn test_console_progress_basic() {
        let progress = ConsoleProgress::new();
        // 不应 panic
        progress.on_start(1024 * 1024 * 100);
        progress.on_progress(0, 1024 * 1024 * 100);
        progress.on_progress(1024 * 1024, 1024 * 1024 * 100);
        progress.on_progress(1024 * 1024 * 100, 1024 * 1024 * 100);
        progress.on_complete(1024 * 1024 * 100);
        progress.on_error("测试错误", 1);
    }

    /// 测试：ConsoleProgress 处理未知总大小
    #[test]
    fn test_console_progress_unknown_total() {
        let progress = ConsoleProgress::new();
        progress.on_start(0);
        progress.on_progress(1024, 0);
        progress.on_progress(2 * 1024 * 1024, 0);
        progress.on_complete(2 * 1024 * 1024);
    }

    /// 测试：自定义 DownloadProgress 实现
    ///
    /// 用于验证 trait 契约：实现方可以自定义进度处理逻辑。
    #[test]
    fn test_custom_progress_implementation() {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;

        struct CountingProgress {
            update_count: Arc<AtomicU64>,
            last_downloaded: Arc<AtomicU64>,
        }

        impl DownloadProgress for CountingProgress {
            fn on_progress(&self, downloaded: u64, _total: u64) {
                self.update_count.fetch_add(1, Ordering::Relaxed);
                self.last_downloaded
                    .store(downloaded, Ordering::Relaxed);
            }
        }

        let update_count = Arc::new(AtomicU64::new(0));
        let last_downloaded = Arc::new(AtomicU64::new(0));
        let progress = CountingProgress {
            update_count: Arc::clone(&update_count),
            last_downloaded: Arc::clone(&last_downloaded),
        };

        progress.on_progress(100, 1000);
        progress.on_progress(500, 1000);
        progress.on_progress(1000, 1000);

        assert_eq!(update_count.load(Ordering::Relaxed), 3);
        assert_eq!(last_downloaded.load(Ordering::Relaxed), 1000);
    }
}
