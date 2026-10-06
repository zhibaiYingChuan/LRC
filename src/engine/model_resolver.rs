// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件包含模型底层架构衍生的核心算法，受研究许可证保护。
// 禁止逆向工程、禁止商业再分发、禁止用于训练竞争模型。
// ============================================================
//
// 模型就绪解析器
//
// 提供统一的模型文件检测接口，供启动检查和测试跳过判断使用。
// 不重复实现下载逻辑（各编码器内部已有），只负责文件存在性检查。

use crate::engine::pooling::PoolingStrategy;

/// 获取当前生效的嵌入模型 ID。
///
/// 优先级：环境变量 > ~/.lrc/config.toml > 系统语言默认模型。
pub fn selected_model_id() -> String {
    if let Ok(model_id) = std::env::var("LRC_LUOSHU_MODEL_ID") {
        if !model_id.trim().is_empty() {
            return model_id.trim().to_string();
        }
    }

    if let Some(home) = home_dir() {
        let config_path = home.join(".lrc").join("config.toml");
        if let Ok(content) = std::fs::read_to_string(config_path) {
            for line in content.lines() {
                let line = line.trim();
                if let Some(value) = line.strip_prefix("model_id") {
                    let first = value.chars().next();
                    if !matches!(first, Some(c) if c.is_whitespace() || c == '=') {
                        continue;
                    }
                    if let Some(value) = value.split_once('=') {
                        let model_id = value.1.trim().trim_matches('"');
                        if !model_id.is_empty() {
                            return model_id.to_string();
                        }
                    }
                }
            }
        }
    }

    if std::env::var("LANG")
        .unwrap_or_default()
        .to_lowercase()
        .contains("zh")
        || std::env::var("LC_ALL")
            .unwrap_or_default()
            .to_lowercase()
            .contains("zh")
    {
        // v0.9.7 修复（模型 ID 常量重复）：改用 Layer 1 单一真源常量。
        // 注意：此处判定用 `contains("zh")`（宽匹配，可识别 "zh_CN.UTF-8"/"en_US.zh" 等），
        // 与 model_ids::detect_default_model_by_lang 的 `starts_with("zh")` **语义不同**，
        // 故仅替换字面量、保留原判定逻辑，不改为委托以避免行为变更。
        crate::model_ids::MODEL_BGE_SMALL_ZH.to_string()
    } else {
        crate::model_ids::MODEL_ALL_MINILM_L6_V2.to_string()
    }
}

// ════════════════════════════════════════════════════════════════
// v0.9.10 新增：模型家族的「输入约定」（前缀 / 池化）
//
// ## 为什么需要它
//
// 不同嵌入模型家族对**输入格式**与**池化方式**有硬性约定，用错会让嵌入质量
// 显著退化，甚至接近坍缩。实测佐证：LRC v0.9.9 线上 9 维洛书向量
// 1963/1963 全落同一八卦类别；上游成因之一就是"用了 e5 的模型、却按 bge 的
// 约定处理"（无前缀 + 注释按 bge 论证池化）。
//
//   · e5 系列（intfloat/*-e5-*）：输入需加 `query: ` / `passage: ` 前缀；
//     官方池化为 **Mean**。缺前缀会明显退化。
//   · bge 系列（BAAI/bge-*）：不要求前缀；以 CLS + 归一化对比学习训练，
//     官方检索用法取 **CLS**。
//   · 未知家族：不加前缀 + Mean（安全默认）。
//
// ## 设计原则（避免硬编码）
//
//   1. 家族判定基于模型 ID / 目录名的**子串匹配表**，新增家族只需加一行；
//   2. 两个维度都可用环境变量显式覆盖，不改代码即可做对照实验：
//        `LRC_MODEL_PREFIX_STYLE` = auto | none | e5
//        `LRC_MODEL_POOLING`      = auto | mean | cls
//   3. 本模块**不写死任何具体模型 ID**——具体 ID 仍由 `model_ids` 常量、
//      `LRC_*_MODEL_ID` 环境变量或 `~/.lrc/config.toml` 决定。
// ════════════════════════════════════════════════════════════════

/// 编码角色：决定使用哪一个前缀（e5 对查询与文档使用不同前缀）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeRole {
    /// 查询侧（e5 → `query: `）
    Query,
    /// 文档 / 记忆侧（e5 → `passage: `）
    Passage,
}

/// 模型家族的输入约定
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelFamilyProfile {
    /// 查询侧前缀（该家族不要求前缀时为空串）
    pub prefix_query: &'static str,
    /// 文档侧前缀（该家族不要求前缀时为空串）
    pub prefix_passage: &'static str,
    /// 池化策略
    pub pooling: PoolingStrategy,
}

impl ModelFamilyProfile {
    /// 按角色取前缀
    pub fn prefix_for(&self, role: EncodeRole) -> &'static str {
        match role {
            EncodeRole::Query => self.prefix_query,
            EncodeRole::Passage => self.prefix_passage,
        }
    }

    /// 给文本加上该角色应有的前缀（前缀为空时原样返回）
    pub fn apply_prefix(&self, text: &str, role: EncodeRole) -> String {
        let p = self.prefix_for(role);
        if p.is_empty() {
            text.to_string()
        } else {
            format!("{p}{text}")
        }
    }
}

/// 依据模型 ID 解析其家族输入约定（含环境变量覆盖）
///
/// 环境变量优先级高于家族表：
///   - `LRC_MODEL_POOLING`：`mean` / `cls`（其他值或未设 → 用家族表）
///   - `LRC_MODEL_PREFIX_STYLE`：`none`（强制无前缀）/ `e5`（强制 e5 前缀）
pub fn profile_for_model(model_id: &str) -> ModelFamilyProfile {
    let lower = model_id.to_lowercase();

    // 家族表：新增家族只需加一行（自上而下，先命中者生效）
    let base = if lower.contains("-e5-")
        || lower.contains("-e5")
        || lower.contains("e5/")
        || lower.contains("/e5")
    {
        ModelFamilyProfile {
            prefix_query: "query: ",
            prefix_passage: "passage: ",
            pooling: PoolingStrategy::Mean,
        }
    } else if lower.contains("bge") {
        ModelFamilyProfile {
            prefix_query: "",
            prefix_passage: "",
            pooling: PoolingStrategy::Cls,
        }
    } else {
        ModelFamilyProfile {
            prefix_query: "",
            prefix_passage: "",
            pooling: PoolingStrategy::Mean,
        }
    };

    let pooling = match std::env::var("LRC_MODEL_POOLING")
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "mean" => PoolingStrategy::Mean,
        "cls" => PoolingStrategy::Cls,
        _ => base.pooling,
    };

    let (prefix_query, prefix_passage) = match std::env::var("LRC_MODEL_PREFIX_STYLE")
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "none" => ("", ""),
        "e5" => ("query: ", "passage: "),
        _ => (base.prefix_query, base.prefix_passage),
    };

    ModelFamilyProfile {
        prefix_query,
        prefix_passage,
        pooling,
    }
}

#[cfg(test)]
mod profile_tests {
    use super::*;
    use std::sync::Mutex;

    /// 环境变量是**进程级全局状态**，而 cargo 默认多线程并行跑测试。
    /// `test_env_override_pooling_and_prefix` 会临时改写 `LRC_MODEL_POOLING` /
    /// `LRC_MODEL_PREFIX_STYLE`，若与其它用例并发，就会串改它们的读取结果
    /// （实测：全量跑时 `test_unknown_family_uses_safe_default` 因读到 `cls` 而失败）。
    /// 故凡依赖这两个环境变量的用例，先取此锁串行执行。
    /// 用 `unwrap_or_else` 容忍持锁用例 panic 导致的毒化，避免连带失败。
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn test_e5_family_requires_prefix_and_mean_pooling() {
        let _g = env_guard();
        let p = profile_for_model("intfloat/multilingual-e5-small");
        assert_eq!(p.prefix_query, "query: ");
        assert_eq!(p.prefix_passage, "passage: ");
        assert_eq!(p.pooling, PoolingStrategy::Mean);
    }

    #[test]
    fn test_bge_family_has_no_prefix_and_cls_pooling() {
        let _g = env_guard();
        let p = profile_for_model("BAAI/bge-base-zh");
        assert_eq!(p.prefix_query, "");
        assert_eq!(p.prefix_passage, "");
        assert_eq!(p.pooling, PoolingStrategy::Cls);
    }

    #[test]
    fn test_unknown_family_uses_safe_default() {
        let _g = env_guard();
        let p = profile_for_model("some/unknown-model");
        assert_eq!(p.prefix_query, "");
        assert_eq!(p.pooling, PoolingStrategy::Mean);
    }

    #[test]
    fn test_apply_prefix_by_role() {
        let _g = env_guard();
        let p = profile_for_model("intfloat/multilingual-e5-small");
        assert_eq!(p.apply_prefix("你好", EncodeRole::Query), "query: 你好");
        assert_eq!(p.apply_prefix("你好", EncodeRole::Passage), "passage: 你好");

        let q = profile_for_model("BAAI/bge-base-zh");
        assert_eq!(q.apply_prefix("你好", EncodeRole::Query), "你好");
    }

    #[test]
    fn test_env_override_pooling_and_prefix() {
        let _g = env_guard();
        // 环境变量覆盖家族表（用临时值，测试后恢复）
        let old_pooling = std::env::var("LRC_MODEL_POOLING").ok();
        let old_prefix = std::env::var("LRC_MODEL_PREFIX_STYLE").ok();

        std::env::set_var("LRC_MODEL_POOLING", "cls");
        std::env::set_var("LRC_MODEL_PREFIX_STYLE", "none");
        let p = profile_for_model("intfloat/multilingual-e5-small");
        assert_eq!(p.pooling, PoolingStrategy::Cls, "环境变量应覆盖家族表池化");
        assert_eq!(p.prefix_query, "", "环境变量应覆盖家族表前缀");

        match old_pooling {
            Some(v) => std::env::set_var("LRC_MODEL_POOLING", v),
            None => std::env::remove_var("LRC_MODEL_POOLING"),
        }
        match old_prefix {
            Some(v) => std::env::set_var("LRC_MODEL_PREFIX_STYLE", v),
            None => std::env::remove_var("LRC_MODEL_PREFIX_STYLE"),
        }
    }
}

/// 检查指定模型是否在本地就绪（models/ 目录或 HuggingFace 缓存）
///
/// 检测顺序：
/// 1. `models/{model_id}/` 目录（用户手动放置）
/// 2. `~/.cache/huggingface/hub/models--{org}--{repo}/blobs/`（自动下载缓存）
pub fn check_model_ready(model_id: &str) -> bool {
    let local_model_name = model_id.replace('/', "--");

    // 1. 检查项目根目录的 models/ 文件夹
    if let Ok(cwd) = std::env::current_dir() {
        let local_dir = cwd.join("models").join(&local_model_name);
        if model_files_exist(&local_dir) {
            return true;
        }
    }

    // 2. 检查可执行文件所在目录的 models/ 文件夹
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let exe_model_dir = exe_dir.join("models").join(&local_model_name);
            if model_files_exist(&exe_model_dir) {
                return true;
            }
        }
    }

    // v0.9.0 新增：检查 ~/.loong-recall/models/ 标准目录
    // 统一模型目录，不依赖 cwd 或 exe_dir，所有模型下载和管理都使用此目录
    if let Some(home) = home_dir() {
        let lrc_models = home
            .join(".loong-recall")
            .join("models")
            .join(&local_model_name);
        if model_files_exist(&lrc_models) {
            return true;
        }
    }

    // 3. 检查 HuggingFace 缓存（~/.cache/huggingface/hub/）
    if let Some(cache_dir) = dirs_next::cache_dir() {
        let folder_name = format!("models--{}", local_model_name);
        let snapshot_dir = cache_dir
            .join("huggingface")
            .join("hub")
            .join(&folder_name)
            .join("snapshots");
        if snapshot_dir.exists() {
            if let Ok(snapshots) = std::fs::read_dir(snapshot_dir) {
                for snapshot in snapshots.flatten() {
                    if model_files_exist(&snapshot.path()) {
                        return true;
                    }
                }
            }
        }
    }

    false
}

/// 检查模型目录是否包含必需文件
fn model_files_exist(dir: &std::path::Path) -> bool {
    dir.join("config.json").exists()
        && (dir.join("model.safetensors").exists() || dir.join("pytorch_model.bin").exists())
}

/// 获取用户主目录（跨平台：Windows USERPROFILE / Unix HOME）
fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()
        .map(std::path::PathBuf::from)
}

/// v0.9.0 新增：获取统一的模型根目录
///
/// 所有模型下载、列表、加载都使用此目录，避免 cwd 不一致导致找不到模型。
/// 目录解析优先级：
///   1. 环境变量 `LRC_MODELS_DIR`（显式指定）
///   2. `~/.loong-recall/models/`（默认标准目录）
///   3. `./models`（回退）
pub fn default_models_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("LRC_MODELS_DIR") {
        if !dir.trim().is_empty() {
            return std::path::PathBuf::from(dir);
        }
    }
    home_dir()
        .map(|h| h.join(".loong-recall").join("models"))
        .unwrap_or_else(|| std::path::PathBuf::from("models"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_model_ready_graphcodebert() {
        // 本地开发环境应有此模型
        let ready = check_model_ready("microsoft/graphcodebert-base");
        println!("GraphCodeBERT model ready: {}", ready);
        // 不强制 assert，因为 CI 环境可能没有模型
    }

    #[test]
    fn test_check_model_ready_nonexistent() {
        let ready = check_model_ready("nonexistent/fake-model-12345");
        assert!(!ready, "不存在的模型应返回 false");
    }

    #[test]
    fn test_model_files_exist_empty_dir() {
        let tmp = std::env::temp_dir().join("lrc_test_empty");
        let _ = std::fs::create_dir_all(&tmp);
        assert!(!model_files_exist(&tmp));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
