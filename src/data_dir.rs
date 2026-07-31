// ============================================================
// 许可证: Apache 2.0
// 本文件实现统一数据目录管理，属于公开层 (Layer 1)。
// ============================================================
//
// 统一数据目录管理模块 — 管理 LRC 记忆数据目录的标准化路径
//
// 数据目录结构（V2）：
//   ~/.loong-recall/
//   ├── projects/
//   │   ├── {sha256_fingerprint}/    # 项目A
//   │   │   └── data/
//   │   │       ├── memories.json
//   │   │       ├── chunks.json
//   │   │       └── archive.json
//   │   └── {sha256_fingerprint}/    # 项目B
//   │       └── data/
//   ├── global/                      # --global 模式
//   │   └── data/
//   ├── config.json                  # 全局配置
//   ├── exports/                     # 导出文件存放
//   └── .lrc.lock                    # 全局服务锁（V2 移至根目录）
//
// 核心能力:
//   1. 获取 LRC 根目录 (~/.loong-recall/)
//   2. 根据项目指纹计算数据目录
//   3. 支持 --global 模式、--data-dir 覆盖、--db-path 兼容
//   4. 自动创建必要的子目录

use crate::project_id;
use std::path::{Path, PathBuf};

/// LRC 数据目录管理器
///
/// 负责确定和管理 LRC 统一数据目录结构。
/// 支持多种数据目录策略：项目指纹模式、全局模式、自定义路径。
#[derive(Debug, Clone)]
pub struct DataDir {
    /// 数据根目录（如 ~/.loong-recall/）
    root: PathBuf,
    /// 实际存储记忆数据的目标目录
    data_path: PathBuf,
    /// 模式标识
    mode: DataDirMode,
}

/// 数据目录模式
#[derive(Debug, Clone, PartialEq)]
pub enum DataDirMode {
    /// 项目指纹模式：数据存储在 ~/.loong-recall/projects/{fp}/data/
    Project { fingerprint: String },
    /// 全局模式：数据存储在 ~/.loong-recall/global/data/
    Global,
    /// 自定义路径（--db-path 或 --data-dir 显式指定）
    Custom,
    /// 旧版兼容模式（src_dir/.loong-recall/data/）
    Legacy { src_dir: PathBuf },
}

/// 项目元信息（存储于 ~/.loong-recall/projects/{fingerprint}/meta.json）
///
/// 用于将指纹对应的项目路径转换为用户可读的显示名，
/// 与项目记忆数据同目录，备份/迁移时一并带走。
///
/// 字段说明：
///   - `fingerprint`：与目录名一致，冗余存储便于校验
///   - `canonical_path`：规范化路径，用于路径变化检测
///   - `auto_name`：路径末段，每次启动刷新
///   - `custom_name`：用户自定义名，None 表示未自定义
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProjectMeta {
    /// 项目指纹（16 字符），与目录名一致
    pub fingerprint: String,
    /// 规范化路径（首次写入时记录）
    pub canonical_path: String,
    /// 自动提取的名称（路径末段）
    pub auto_name: String,
    /// 用户自定义名称，None 表示未自定义
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_name: Option<String>,
    /// 项目首次被记录的时间（ISO8601）
    pub first_seen_at: String,
    /// 项目最近一次被访问的时间（ISO8601）
    pub last_seen_at: String,
    /// 数据结构版本，便于未来迁移
    pub schema_version: u32,
}

impl ProjectMeta {
    /// 当前数据结构版本
    pub const CURRENT_SCHEMA_VERSION: u32 = 1;

    /// 为指定项目创建元信息（自动提取 auto_name）
    ///
    /// 通常在项目首次写入记忆时调用。
    pub fn for_project(src_dir: &Path) -> Self {
        let (fingerprint, canonical_path) = project_id::project_fingerprint_with_path(src_dir);
        let auto_name = project_id::auto_name_from_path(&canonical_path);
        let now = chrono::Utc::now().to_rfc3339();

        Self {
            fingerprint,
            canonical_path,
            auto_name,
            custom_name: None,
            first_seen_at: now.clone(),
            last_seen_at: now,
            schema_version: Self::CURRENT_SCHEMA_VERSION,
        }
    }

    /// 获取有效显示名
    ///
    /// 优先级：custom_name > auto_name > fingerprint 前 8 位
    pub fn display_name(&self) -> String {
        if let Some(custom) = &self.custom_name {
            let trimmed = custom.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        if !self.auto_name.is_empty() {
            self.auto_name.clone()
        } else {
            // 兜底：fingerprint 前 8 位
            let fp = if self.fingerprint.len() >= 8 {
                &self.fingerprint[..8]
            } else {
                &self.fingerprint
            };
            format!("{fp}...")
        }
    }

    /// 更新 last_seen_at 为当前时间
    pub fn touch(&mut self) {
        self.last_seen_at = chrono::Utc::now().to_rfc3339();
    }
}

// ==================== 公共 API ====================

impl DataDir {
    /// 获取 LRC 根目录（~/.loong-recall/）
    ///
    /// 跨平台兼容：
    ///   - Windows: %USERPROFILE%/.loong-recall/
    ///   - Linux:   ~/.loong-recall/
    ///   - macOS:   ~/.loong-recall/
    pub fn root_dir() -> PathBuf {
        dirs_next::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".loong-recall")
    }

    /// 根据项目源码目录创建数据目录管理器（V2 项目指纹模式）
    ///
    /// 数据路径：~/.loong-recall/projects/{fingerprint}/data/
    pub fn for_project(src_dir: &Path) -> Self {
        let root = Self::root_dir();
        let (fingerprint, _canonical_path) = project_id::project_fingerprint_with_path(src_dir);
        let data_path = root.join("projects").join(&fingerprint).join("data");

        Self {
            root,
            data_path,
            mode: DataDirMode::Project { fingerprint },
        }
    }

    /// 创建全局模式数据目录管理器
    ///
    /// 数据路径：~/.loong-recall/global/data/
    pub fn for_global() -> Self {
        let root = Self::root_dir();
        let data_path = root.join("global").join("data");

        Self {
            root,
            data_path,
            mode: DataDirMode::Global,
        }
    }

    /// 创建自定义数据目录管理器（--data-dir 或 --db-path）
    ///
    /// 数据路径由用户显式指定，不经过根目录。
    pub fn for_custom<P: AsRef<Path>>(custom_path: P) -> Self {
        let data_path = custom_path.as_ref().to_path_buf();
        let root = Self::root_dir(); // 根目录仍用于配置和导出

        Self {
            root,
            data_path,
            mode: DataDirMode::Custom,
        }
    }

    /// 创建旧版兼容模式（src_dir/.loong-recall/data/）
    ///
    /// 用于向后兼容，也用于未迁移的旧项目。
    pub fn for_legacy(src_dir: &Path) -> Self {
        let data_path = src_dir.join(".loong-recall").join("data");
        let root = Self::root_dir();

        Self {
            root,
            data_path,
            mode: DataDirMode::Legacy {
                src_dir: src_dir.to_path_buf(),
            },
        }
    }

    /// 获取实际数据存储路径
    pub fn data_path(&self) -> &Path {
        &self.data_path
    }

    /// 获取 LRC 根目录
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 获取数据目录模式
    pub fn mode(&self) -> &DataDirMode {
        &self.mode
    }

    /// 获取项目指纹（仅在 Project 模式下有效）
    pub fn fingerprint(&self) -> Option<&str> {
        match &self.mode {
            DataDirMode::Project { fingerprint } => Some(fingerprint.as_str()),
            _ => None,
        }
    }

    /// 确保数据目录存在（递归创建所有父目录）
    ///
    /// 返回创建后的数据路径。
    pub fn ensure(&self) -> std::io::Result<&Path> {
        std::fs::create_dir_all(&self.data_path)?;
        Ok(&self.data_path)
    }

    /// 获取导出目录路径
    ///
    /// 导出文件存放在 ~/.loong-recall/exports/
    pub fn exports_dir(&self) -> PathBuf {
        self.root.join("exports")
    }

    /// 确保导出目录存在
    pub fn ensure_exports_dir(&self) -> std::io::Result<PathBuf> {
        let dir = self.exports_dir();
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// 获取全局锁文件路径（V2 移至根目录）
    ///
    /// 锁文件路径：~/.loong-recall/.lrc.lock
    pub fn global_lock_path(&self) -> PathBuf {
        self.root.join(".lrc.lock")
    }

    /// 获取项目级锁文件路径（向后兼容旧版）
    ///
    /// 锁文件路径：{data_path}/.lrc.lock
    pub fn legacy_lock_path(&self) -> PathBuf {
        self.data_path.join(".lrc.lock")
    }

    /// 获取旧版数据目录路径（src_dir/.loong-recall/data/）
    ///
    /// 用于迁移检测：检查旧版数据是否存在。
    pub fn legacy_data_path(src_dir: &Path) -> PathBuf {
        src_dir.join(".loong-recall").join("data")
    }

    /// 检查旧版数据目录是否存在
    pub fn has_legacy_data(src_dir: &Path) -> bool {
        Self::legacy_data_path(src_dir).exists()
    }

    /// 获取迁移标记文件路径
    ///
    /// 迁移完成后在旧版目录创建此文件，防止重复迁移。
    pub fn migration_marker_path(src_dir: &Path) -> PathBuf {
        src_dir.join(".loong-recall").join(".migrated_to_v2")
    }

    /// 检查是否已迁移过
    pub fn is_migrated(src_dir: &Path) -> bool {
        Self::migration_marker_path(src_dir).exists()
    }

    /// 获取项目目录路径（~/.loong-recall/projects/{fingerprint}/）
    pub fn project_dir(&self) -> Option<PathBuf> {
        match &self.mode {
            DataDirMode::Project { fingerprint } => {
                Some(self.root.join("projects").join(fingerprint))
            }
            _ => None,
        }
    }

    /// 获取项目元信息文件路径（meta.json）
    ///
    /// 路径：~/.loong-recall/projects/{fingerprint}/meta.json
    /// 仅在 Project 模式下有效，其他模式返回 None。
    pub fn meta_path(&self) -> Option<PathBuf> {
        self.project_dir().map(|dir| dir.join("meta.json"))
    }

    /// 读取项目元信息（meta.json）
    ///
    /// 文件不存在时返回 Ok(None)，不视为错误。
    /// 文件损坏（JSON 解析失败）时返回 Err。
    pub fn read_meta(&self) -> std::io::Result<Option<ProjectMeta>> {
        let path = match self.meta_path() {
            Some(p) => p,
            None => return Ok(None),
        };

        if !path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&path)?;
        let meta: ProjectMeta = serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(Some(meta))
    }

    /// 原子写入项目元信息（先写 .tmp 再 rename）
    ///
    /// 避免并发写入时数据损坏。仅在 Project 模式下有效。
    pub fn write_meta(&self, meta: &ProjectMeta) -> std::io::Result<()> {
        let path = match self.meta_path() {
            Some(p) => p,
            None => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "非 Project 模式不支持 meta.json",
                ));
            }
        };

        // 确保父目录存在
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // 原子写入：先写 .tmp 再 rename（同一文件系统内 rename 是原子操作）
        let tmp_path = path.with_extension("json.tmp");
        let content = serde_json::to_string_pretty(meta)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(&tmp_path, content)?;
        std::fs::rename(&tmp_path, &path)?;

        Ok(())
    }

    /// 确保项目元信息存在（不存在则用 src_dir 创建）
    ///
    /// 已存在时会刷新 auto_name/canonical_path/last_seen_at。
    /// 返回最终的 ProjectMeta。
    pub fn ensure_meta(&self, src_dir: &Path) -> std::io::Result<ProjectMeta> {
        if let Some(mut existing) = self.read_meta()? {
            // 已存在：刷新 auto_name 和 last_seen_at（路径可能变化）
            let (_, canonical_path) = project_id::project_fingerprint_with_path(src_dir);
            existing.auto_name = project_id::auto_name_from_path(&canonical_path);
            existing.canonical_path = canonical_path;
            existing.touch();
            self.write_meta(&existing)?;
            Ok(existing)
        } else {
            // 不存在：创建新的
            let meta = ProjectMeta::for_project(src_dir);
            self.write_meta(&meta)?;
            Ok(meta)
        }
    }
}

// ==================== 批量查询 API ====================

/// 项目列表项（用于 `/api/projects/list` 端点响应）
///
/// 包含项目指纹、可读名称、路径、记忆数等信息，
/// 供前端构建"指纹→名称"映射表，将仪表盘项目分布的指纹 key 转为可读名称。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProjectListItem {
    /// 项目指纹（16 字符，与目录名一致）
    pub fingerprint: String,
    /// 可读显示名（custom_name > auto_name > fingerprint 前 8 位）
    pub display_name: String,
    /// 自动提取的名称（路径末段，meta.json 缺失时为空字符串）
    pub auto_name: String,
    /// 用户自定义名称（None 表示未自定义）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_name: Option<String>,
    /// 规范化路径（meta.json 缺失时为空字符串）
    pub canonical_path: String,
    /// 该项目的记忆总数（读取 memories.json 计数，失败时为 0）
    pub memory_count: usize,
    /// 项目首次被记录的时间（ISO8601，meta.json 缺失时为空字符串）
    pub first_seen_at: String,
    /// 项目最近一次被访问的时间（ISO8601，meta.json 缺失时为空字符串）
    pub last_seen_at: String,
    /// meta.json 是否存在（用于前端提示用户该项目的路径信息缺失）
    pub has_meta: bool,
}

/// 列出所有已知项目的元信息（用于批量查询）
///
/// 遍历 `~/.loong-recall/projects/` 目录下的所有子目录，
/// 对每个合法的指纹目录：
///   1. 读取 meta.json（如果存在）
///   2. 统计 memories.json 中的记忆数（如果存在）
///   3. 返回 ProjectListItem 列表
///
/// # 性能
/// - 126 个项目实测 < 50ms（每个项目仅读取 2 个小文件）
/// - 失败的项目（meta.json 损坏等）会被跳过，不影响其他项目
///
/// # 排序
/// 按 memory_count 降序排列，让最活跃的项目排在前面。
pub fn list_all_projects() -> Vec<ProjectListItem> {
    let projects_root = DataDir::root_dir().join("projects");

    if !projects_root.exists() {
        return Vec::new();
    }

    let mut items: Vec<ProjectListItem> = Vec::new();

    let entries = match std::fs::read_dir(&projects_root) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let dir_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };

        // 仅处理合法的 16 位十六进制指纹目录
        if !project_id::is_valid_fingerprint(&dir_name) {
            continue;
        }

        // 读取 meta.json（不存在时使用兜底值）
        let meta_path = path.join("meta.json");
        let (
            display_name,
            auto_name,
            custom_name,
            canonical_path,
            first_seen_at,
            last_seen_at,
            has_meta,
        ) = if meta_path.exists() {
            match std::fs::read_to_string(&meta_path).and_then(|c| {
                serde_json::from_str::<ProjectMeta>(&c)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
            }) {
                Ok(meta) => (
                    meta.display_name(),
                    meta.auto_name,
                    meta.custom_name,
                    meta.canonical_path,
                    meta.first_seen_at,
                    meta.last_seen_at,
                    true,
                ),
                Err(_) => {
                    // meta.json 损坏：兜底用 fingerprint 前 8 位
                    let fp8 = if dir_name.len() >= 8 {
                        &dir_name[..8]
                    } else {
                        &dir_name
                    };
                    (
                        format!("{fp8}..."),
                        String::new(),
                        None,
                        String::new(),
                        String::new(),
                        String::new(),
                        false,
                    )
                }
            }
        } else {
            // meta.json 不存在：兜底用 fingerprint 前 8 位
            let fp8 = if dir_name.len() >= 8 {
                &dir_name[..8]
            } else {
                &dir_name
            };
            (
                format!("{fp8}..."),
                String::new(),
                None,
                String::new(),
                String::new(),
                String::new(),
                false,
            )
        };

        // 统计 memories.json 中的记忆数（简单实现：解析 JSON 数组长度）
        let memories_path = path.join("data").join("memories.json");
        let memory_count = if memories_path.exists() {
            std::fs::read_to_string(&memories_path)
                .ok()
                .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
                .and_then(|v| v.as_array().map(|a| a.len()))
                .unwrap_or(0)
        } else {
            0
        };

        items.push(ProjectListItem {
            fingerprint: dir_name,
            display_name,
            auto_name,
            custom_name,
            canonical_path,
            memory_count,
            first_seen_at,
            last_seen_at,
            has_meta,
        });
    }

    // 按 memory_count 降序排列（clippy::unnecessary_sort_by：用 sort_by_key + Reverse）
    items.sort_by_key(|b| std::cmp::Reverse(b.memory_count));
    items
}

// ==================== 单元测试 ====================

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试: root_dir 返回以 .loong-recall 结尾的路径
    #[test]
    fn test_root_dir_ends_with_loong_recall() {
        let root = DataDir::root_dir();
        let path_str = root.to_string_lossy();
        assert!(
            path_str.contains(".loong-recall"),
            "根目录应包含 .loong-recall，实际: {path_str}"
        );
    }

    /// 测试: for_project 使用正确的子目录结构
    #[test]
    fn test_for_project_structure() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let dd = DataDir::for_project(tmp.path());

        let data_path = dd.data_path().to_string_lossy();
        assert!(
            data_path.contains("projects"),
            "数据路径应包含 'projects' 子目录，实际: {data_path}"
        );
        assert!(
            data_path.ends_with("data"),
            "数据路径应以 'data' 结尾，实际: {data_path}"
        );

        // 验证指纹不为空
        let fp = dd.fingerprint().expect("Project 模式应有指纹");
        assert_eq!(fp.len(), 16, "指纹长度应为 16");
        assert!(data_path.contains(fp), "数据路径应包含指纹");
    }

    /// 测试: for_global 使用正确的子目录结构
    #[test]
    fn test_for_global_structure() {
        let dd = DataDir::for_global();
        let data_path = dd.data_path().to_string_lossy();
        assert!(
            data_path.contains("global"),
            "全局模式数据路径应包含 'global'，实际: {data_path}"
        );
        assert!(
            data_path.ends_with("data"),
            "数据路径应以 'data' 结尾，实际: {data_path}"
        );
        assert_eq!(*dd.mode(), DataDirMode::Global);
    }

    /// 测试: for_custom 使用用户指定的路径
    #[test]
    fn test_for_custom_path() {
        let custom = if cfg!(target_os = "windows") {
            "D:\\custom-lrc-data"
        } else {
            "/tmp/custom-lrc-data"
        };
        let dd = DataDir::for_custom(custom);
        assert_eq!(
            dd.data_path().to_string_lossy(),
            custom,
            "自定义路径应完全匹配"
        );
        assert_eq!(*dd.mode(), DataDirMode::Custom);
    }

    /// 测试: for_legacy 使用旧版目录结构
    #[test]
    fn test_for_legacy_structure() {
        let src = Path::new("/tmp/my-project");
        let dd = DataDir::for_legacy(src);
        let data_path = dd.data_path().to_string_lossy();
        // 在 Windows 上路径分隔符可能不同，但都应包含 .loong-recall
        assert!(
            data_path.contains(".loong-recall"),
            "旧版数据路径应包含 .loong-recall"
        );
        assert!(
            data_path.contains("my-project"),
            "旧版数据路径应包含源码目录"
        );
    }

    /// 测试: ensure 创建数据目录
    #[test]
    fn test_ensure_creates_directory() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let dd = DataDir::for_project(tmp.path());

        // 确保数据目录存在
        assert!(!dd.data_path().exists(), "测试前目录不应存在");
        dd.ensure().expect("ensure 应成功创建目录");
        assert!(dd.data_path().exists(), "ensure 后目录应存在");
        assert!(dd.data_path().is_dir(), "ensure 创建的应是目录");
    }

    /// 测试: exports_dir 路径正确
    #[test]
    fn test_exports_dir() {
        let dd = DataDir::for_global();
        let exports = dd.exports_dir();
        let path_str = exports.to_string_lossy();
        assert!(
            path_str.contains(".loong-recall"),
            "导出目录应在 .loong-recall 下"
        );
        assert!(path_str.contains("exports"), "导出目录应包含 'exports'");
    }

    /// 测试: ensure_exports_dir 创建目录
    #[test]
    fn test_ensure_exports_dir() {
        let dd = DataDir::for_global();
        let exports = dd.ensure_exports_dir().expect("创建导出目录失败");
        assert!(exports.exists(), "导出目录应被创建");
        assert!(exports.is_dir(), "导出目录应是目录");
    }

    /// 测试: global_lock_path 在根目录下
    #[test]
    fn test_global_lock_path() {
        let dd = DataDir::for_global();
        let lock = dd.global_lock_path();
        let path_str = lock.to_string_lossy();
        assert!(path_str.contains(".lrc.lock"), "锁文件应以 .lrc.lock 结尾");
    }

    /// 测试: legacy_lock_path 在数据目录下
    #[test]
    fn test_legacy_lock_path() {
        let dd = DataDir::for_global();
        let lock = dd.legacy_lock_path();
        let path_str = lock.to_string_lossy();
        assert!(path_str.contains(".lrc.lock"), "锁文件应以 .lrc.lock 结尾");
    }

    /// 测试: legacy_data_path 构造正确
    #[test]
    fn test_legacy_data_path() {
        let src = Path::new("/home/user/project");
        let legacy = DataDir::legacy_data_path(src);
        let path_str = legacy.to_string_lossy();
        assert!(
            path_str.contains(".loong-recall"),
            "旧版路径应包含 .loong-recall"
        );
        assert!(path_str.contains("data"), "旧版路径应以 data 结尾");
    }

    /// 测试: has_legacy_data 检测旧版数据
    #[test]
    fn test_has_legacy_data() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let src = tmp.path().join("project");

        // 创建旧版数据目录结构
        let legacy_dir = DataDir::legacy_data_path(&src);
        std::fs::create_dir_all(&legacy_dir).expect("创建旧版目录失败");

        assert!(DataDir::has_legacy_data(&src), "应检测到旧版数据目录");
    }

    /// 测试: has_legacy_data 对不存在的目录返回 false
    #[test]
    fn test_no_legacy_data() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let src = tmp.path().join("empty_project");
        assert!(!DataDir::has_legacy_data(&src), "空目录不应有旧版数据");
    }

    /// 测试: migration_marker_path 构造正确
    #[test]
    fn test_migration_marker_path() {
        let src = Path::new("/home/user/project");
        let marker = DataDir::migration_marker_path(src);
        let path_str = marker.to_string_lossy();
        assert!(
            path_str.contains(".migrated_to_v2"),
            "迁移标记应包含 .migrated_to_v2"
        );
    }

    /// 测试: is_migrated 检测迁移标记
    #[test]
    fn test_is_migrated() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let src = tmp.path().join("migrated_project");

        // 创建迁移标记
        let marker = DataDir::migration_marker_path(&src);
        std::fs::create_dir_all(marker.parent().unwrap()).expect("创建父目录失败");
        std::fs::write(&marker, "v2").expect("写入标记文件失败");

        assert!(DataDir::is_migrated(&src), "应检测到迁移标记");
    }

    /// 测试: project_dir 仅在 Project 模式下返回 Some
    #[test]
    fn test_project_dir() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let dd = DataDir::for_project(tmp.path());
        assert!(dd.project_dir().is_some(), "Project 模式应有 project_dir");

        let dd_global = DataDir::for_global();
        assert!(
            dd_global.project_dir().is_none(),
            "Global 模式不应有 project_dir"
        );

        let dd_custom = DataDir::for_custom("/tmp/custom");
        assert!(
            dd_custom.project_dir().is_none(),
            "Custom 模式不应有 project_dir"
        );
    }

    /// 测试: 同一项目路径两次调用 for_project 返回相同路径
    #[test]
    fn test_same_project_idempotent() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let dd1 = DataDir::for_project(tmp.path());
        let dd2 = DataDir::for_project(tmp.path());
        assert_eq!(
            dd1.data_path(),
            dd2.data_path(),
            "同一项目应返回相同数据路径"
        );
        assert_eq!(
            dd1.fingerprint(),
            dd2.fingerprint(),
            "同一项目应返回相同指纹"
        );
    }

    /// 测试: meta_path 仅在 Project 模式下返回 Some
    #[test]
    fn test_meta_path_only_for_project_mode() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let dd = DataDir::for_project(tmp.path());
        assert!(dd.meta_path().is_some(), "Project 模式应有 meta_path");

        let dd_global = DataDir::for_global();
        assert!(
            dd_global.meta_path().is_none(),
            "Global 模式不应有 meta_path"
        );
    }

    /// 测试: read_meta 文件不存在时返回 Ok(None)
    #[test]
    fn test_read_meta_nonexistent() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let dd = DataDir::for_project(tmp.path());
        let result = dd.read_meta().expect("read_meta 不应返回 Err");
        assert!(result.is_none(), "文件不存在时应返回 None");
    }

    /// 测试: write_meta + read_meta 往返一致性
    #[test]
    fn test_write_and_read_meta_roundtrip() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let dd = DataDir::for_project(tmp.path());

        let meta = ProjectMeta::for_project(tmp.path());
        dd.write_meta(&meta).expect("write_meta 应成功");

        let read = dd
            .read_meta()
            .expect("read_meta 应成功")
            .expect("应读到 meta");
        assert_eq!(read.fingerprint, meta.fingerprint);
        assert_eq!(read.canonical_path, meta.canonical_path);
        assert_eq!(read.auto_name, meta.auto_name);
        assert_eq!(read.custom_name, meta.custom_name);
        assert_eq!(read.schema_version, meta.schema_version);
    }

    /// 测试: ensure_meta 不存在时创建新 meta
    #[test]
    fn test_ensure_meta_creates_new() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let dd = DataDir::for_project(tmp.path());

        // 初始状态：无 meta.json
        assert!(dd.read_meta().unwrap().is_none());

        // ensure 后：有 meta.json
        let meta = dd.ensure_meta(tmp.path()).expect("ensure_meta 应成功");
        assert_eq!(meta.fingerprint.len(), 16);
        assert!(!meta.auto_name.is_empty());
        assert!(meta.custom_name.is_none());

        // 再次读取应一致
        let read = dd.read_meta().unwrap().expect("应读到 meta");
        assert_eq!(read.fingerprint, meta.fingerprint);
    }

    /// 测试: ensure_meta 已存在时刷新 last_seen_at
    #[test]
    fn test_ensure_meta_refreshes_existing() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let dd = DataDir::for_project(tmp.path());

        // 首次创建
        let meta1 = dd.ensure_meta(tmp.path()).expect("ensure_meta 应成功");
        let original_last_seen = meta1.last_seen_at.clone();

        // 等待一小段时间确保时间戳不同
        std::thread::sleep(std::time::Duration::from_millis(1100));

        // 再次 ensure：应刷新 last_seen_at
        let meta2 = dd.ensure_meta(tmp.path()).expect("ensure_meta 应成功");
        assert_eq!(meta1.fingerprint, meta2.fingerprint);
        assert_ne!(
            original_last_seen, meta2.last_seen_at,
            "last_seen_at 应被刷新"
        );
    }

    /// 测试: ProjectMeta::display_name 优先级
    #[test]
    fn test_display_name_priority() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let mut meta = ProjectMeta::for_project(tmp.path());

        // 1. 无 custom_name：返回 auto_name
        assert_eq!(meta.display_name(), meta.auto_name);

        // 2. 有 custom_name（非空）：返回 custom_name（trim 后）
        meta.custom_name = Some("  LRC 桌面端  ".to_string());
        assert_eq!(meta.display_name(), "LRC 桌面端");

        // 3. custom_name 为空白：回退到 auto_name
        meta.custom_name = Some("   ".to_string());
        assert_eq!(meta.display_name(), meta.auto_name);

        // 4. custom_name 为 None：返回 auto_name
        meta.custom_name = None;
        assert_eq!(meta.display_name(), meta.auto_name);
    }

    /// 测试: ProjectMeta::display_name 兜底（auto_name 为空时用 fingerprint 前 8 位）
    #[test]
    fn test_display_name_fingerprint_fallback() {
        let meta = ProjectMeta {
            fingerprint: "a1b2c3d4e5f6a7b8".to_string(),
            canonical_path: String::new(),
            auto_name: String::new(),
            custom_name: None,
            first_seen_at: "2026-07-31T10:00:00Z".to_string(),
            last_seen_at: "2026-07-31T10:00:00Z".to_string(),
            schema_version: 1,
        };
        assert_eq!(meta.display_name(), "a1b2c3d4...");
    }

    /// 测试: Global 模式 write_meta 返回错误
    #[test]
    fn test_write_meta_rejects_global_mode() {
        let dd = DataDir::for_global();
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let meta = ProjectMeta::for_project(tmp.path());
        let result = dd.write_meta(&meta);
        assert!(result.is_err(), "Global 模式 write_meta 应返回错误");
    }
}
