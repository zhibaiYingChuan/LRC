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

    // 3. 检查 HuggingFace 缓存（~/.cache/huggingface/hub/）
    if let Some(cache_dir) = dirs_next::cache_dir() {
        let folder_name = format!("models--{}", local_model_name);
        let hf_cache = cache_dir
            .join("huggingface")
            .join("hub")
            .join(&folder_name)
            .join("blobs");
        if hf_cache.exists() {
            // HF 缓存用 blob 哈希命名，只需检查目录非空
            if let Ok(entries) = std::fs::read_dir(&hf_cache) {
                if entries.count() > 0 {
                    return true;
                }
            }
        }
    }

    false
}

/// 检查模型目录是否包含必需文件
fn model_files_exist(dir: &std::path::Path) -> bool {
    dir.join("config.json").exists()
        && (dir.join("model.safetensors").exists()
            || dir.join("pytorch_model.bin").exists())
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