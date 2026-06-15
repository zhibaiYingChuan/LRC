use std::path::{Path, PathBuf};

fn main() {
    // Tauri 构建基础配置
    tauri_build::build();

    // ════════════════════════════════════════════════════════════════
    // P1-01 修复：自动同步 Sidecar 二进制
    // 当使用自定义 target-dir（如 ~/.cargo/config.toml 中配置）
    // 时，编译产物不在默认的 target/ 目录，需要自动复制到
    // desktop/src-tauri/ 下，确保打包时包含最新版本。
    // ════════════════════════════════════════════════════════════════
    sync_sidecar_binary();
}

/// 自动同步 sidecar 二进制文件
///
/// 搜索策略（按优先级）：
/// 1. $CARGO_TARGET_DIR/release/code-memory-server.exe（自定义 target-dir）
/// 2. 项目根 target/release/code-memory-server.exe（默认 target-dir）
/// 3. 当前目录下已有的 code-memory-server.exe（无需更新）
fn sync_sidecar_binary() {
    let dest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dest_path = dest_dir.join("code-memory-server.exe");

    // 获取 workspace 根目录（桌面端在 workspace 子目录下）
    let workspace_root = find_workspace_root(&dest_dir);

    // 候选源路径列表
    let candidates: Vec<PathBuf> = {
        let mut paths = Vec::new();

        // 候选 1: $CARGO_TARGET_DIR/release/（自定义 target-dir）
        if let Ok(target_dir) = std::env::var("CARGO_TARGET_DIR") {
            let p = PathBuf::from(&target_dir)
                .join("release")
                .join("code-memory-server.exe");
            paths.push(p);
        }

        // 候选 2: workspace target/release/（默认 target-dir）
        if let Some(ref ws) = workspace_root {
            paths.push(ws.join("target").join("release").join("code-memory-server.exe"));
        }

        // 候选 3: 桌面端同级目录的 target/release/
        paths.push(dest_dir.join("target").join("release").join("code-memory-server.exe"));

        paths
    };

    // 查找最新的候选源文件
    let best_source = candidates
        .iter()
        .filter(|p| p.exists())
        .max_by_key(|p| {
            std::fs::metadata(p)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        });

    let Some(source) = best_source else {
        println!(
            "cargo:warning=未找到已编译的 code-memory-server.exe，请先构建主项目: cargo build --release -p code-memory"
        );
        return;
    };

    // 检查目标文件是否需要更新（比较修改时间和大小）
    if dest_path.exists() {
        match (
            std::fs::metadata(&dest_path),
            std::fs::metadata(source),
        ) {
            (Ok(dest_meta), Ok(src_meta)) => {
                if dest_meta.len() == src_meta.len() {
                    if let (Ok(dest_time), Ok(src_time)) =
                        (dest_meta.modified(), src_meta.modified())
                    {
                        if dest_time >= src_time {
                            println!(
                                "cargo:info=Sidecar 二进制已是最新版本 ({:.1} MB)",
                                src_meta.len() as f64 / 1_048_576.0
                            );
                            return;
                        }
                    }
                }
            }
            _ => {}
        }

        // ═══════════════════════════════════════════════════════════
        // P1-02 修复：构建前检测文件锁定冲突
        // 尝试重命名目标文件检测是否被占用，如果锁定则给出明确提示
        // ═══════════════════════════════════════════════════════════
        let lock_check = dest_path.with_extension("exe.lock_check");
        match std::fs::rename(&dest_path, &lock_check) {
            Ok(()) => {
                // 重命名成功 = 文件未被占用，恢复原名并继续
                let _ = std::fs::rename(&lock_check, &dest_path);
            }
            Err(e) => {
                // os error 32 = 文件被占用，os error 5 = 拒绝访问
                if e.raw_os_error() == Some(32) || e.raw_os_error() == Some(5) {
                    eprintln!();
                    eprintln!("  ╔════════════════════════════════════════════════════╗");
                    eprintln!("  ║  P1-02 构建警告：Sidecar 文件被占用              ║");
                    eprintln!("  ╠════════════════════════════════════════════════════╣");
                    eprintln!("  ║  code-memory-server.exe 正在被 MCP 服务使用      ║");
                    eprintln!("  ║  请先关闭 MCP 服务后再构建:                      ║");
                    eprintln!("  ║    taskkill /F /IM code-memory-server.exe         ║");
                    eprintln!("  ║  当前源文件: {:?}", source);
                    eprintln!("  ║  目标文件: {:?}", dest_path);
                    eprintln!("  ╚════════════════════════════════════════════════════╝");
                    eprintln!();
                    return;
                }
                // 其他错误继续尝试复制
            }
        }
    }

    // 执行复制
    match std::fs::copy(source, &dest_path) {
        Ok(bytes) => {
            println!(
                "cargo:info=Sidecar 二进制已同步: {} → {} ({:.1} MB)",
                source.display(),
                dest_path.display(),
                bytes as f64 / 1_048_576.0
            );
        }
        Err(e) => {
            eprintln!(
                "cargo:warning=Sidecar 二进制复制失败: {} → {} ({})",
                source.display(),
                dest_path.display(),
                e
            );
        }
    }
}

/// 向上查找 workspace 根目录（包含 workspace Cargo.toml）
fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        let cargo_toml = current.join("Cargo.toml");
        if cargo_toml.exists() {
            // 检查是否为 workspace 根（不止一个 member 的 Cargo.toml）
            if let Ok(content) = std::fs::read_to_string(&cargo_toml) {
                if content.contains("[workspace") {
                    return Some(current);
                }
            }
        }
        if !current.pop() {
            break;
        }
    }
    None
}