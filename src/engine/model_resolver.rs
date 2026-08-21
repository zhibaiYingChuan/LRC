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
        "BAAI/bge-small-zh".to_string()
    } else {
        "sentence-transformers/all-MiniLM-L6-v2".to_string()
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
