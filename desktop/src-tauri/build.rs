use std::path::{Path, PathBuf};

fn main() {
    // Tauri 构建基础配置
    tauri_build::build();

    // ════════════════════════════════════════════════════════════════
    // P1-01 修复：自动同步 Sidecar 二进制
    // 当使用自定义 target-dir（如 ~/.cargo/config.toml 中配置）
    // 时，编译产物不在默认的 target/ 目录，需要自动复制到
    // desktop/src-tauri/ 下，确保打包时包含最新版本。
    //
    // v0.5.1 增强：
    //   - 使用 SHA-256 哈希而非时间戳判断文件是否需要更新
    //   - 解决 build.rs 缓存导致跳过复制的问题
    //   - 添加详细的构建日志，方便排查问题
    // ════════════════════════════════════════════════════════════════
    sync_sidecar_binary();
}

/// 计算文件的 SHA-256 哈希值
fn compute_sha256(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = sha2::Sha256::new();
    use sha2::Digest;
    let mut buffer = [0u8; 8192];
    loop {
        let bytes_read = file.read(&mut buffer).ok()?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }
    Some(format!("{:x}", hasher.finalize()))
}

/// 自动同步 sidecar 二进制文件
///
/// 搜索策略（按优先级）：
/// 1. $CARGO_TARGET_DIR/{target_triple}/release/code-memory-server[.exe]（自定义 target-dir + 交叉编译）
/// 2. $CARGO_TARGET_DIR/release/code-memory-server[.exe]（自定义 target-dir）
/// 3. 项目根 target/{target_triple}/release/code-memory-server[.exe]（交叉编译）
/// 4. 项目根 target/release/code-memory-server[.exe]（默认 target-dir）
/// 5. 当前目录下已有的 code-memory-server.exe（无需更新）
///
/// v0.5.1 增强：使用 SHA-256 哈希验证，确保即使时间戳相同也能检测到内容变化
/// v0.5.6 修复：支持 macOS/Linux 平台（二进制文件名不带 .exe 后缀）
fn sync_sidecar_binary() {
    // 尝试使用 sha2 crate，如果不可用则回退到简单的时间戳比较
    let use_hash = true; // 优先使用哈希验证

    let dest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // 根据目标平台确定源/目标二进制文件名
    // Windows: code-memory-server.exe / lrc-sidecar.exe
    // macOS/Linux: code-memory-server / lrc-sidecar（无扩展名）
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_triple = std::env::var("TARGET").unwrap_or_default();
    let binary_name = if target_os == "windows" {
        "code-memory-server.exe"
    } else {
        "code-memory-server"
    };
    // v0.5.7 修复：目标文件名根据平台决定（与 find_sidecar_binary 的 EXE_SUFFIX 一致）
    let dest_name = if target_os == "windows" {
        "lrc-sidecar.exe"
    } else {
        "lrc-sidecar"
    };
    let dest_path = dest_dir.join(dest_name);

    println!("cargo:info=桌面端构建目标平台: {} (triple: {})", target_os, target_triple);

    // 获取 workspace 根目录（桌面端在 workspace 子目录下）
    let workspace_root = find_workspace_root(&dest_dir);

    // 候选源路径列表
    let candidates: Vec<PathBuf> = {
        let mut paths = Vec::new();

        // 候选 1: $CARGO_TARGET_DIR/{target_triple}/release/（自定义 target-dir + 交叉编译）
        if let Ok(target_dir) = std::env::var("CARGO_TARGET_DIR") {
            if !target_triple.is_empty() {
                let p = PathBuf::from(&target_dir)
                    .join(&target_triple)
                    .join("release")
                    .join(binary_name);
                paths.push(p);
            }
            // 候选 1b: $CARGO_TARGET_DIR/release/（自定义 target-dir，无交叉编译）
            let p = PathBuf::from(&target_dir)
                .join("release")
                .join(binary_name);
            paths.push(p);
        }

        // 候选 1c: 从 ~/.cargo/config.toml 读取 target-dir（全局 cargo 配置）
        // v0.5.1 修复：当 target-dir 通过 cargo config 而非环境变量设置时，
        // build.rs 无法通过 CARGO_TARGET_DIR 环境变量获取，需要手动解析配置
        if let Ok(home) = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
        {
            let cargo_config = PathBuf::from(&home).join(".cargo").join("config.toml");
            if let Ok(content) = std::fs::read_to_string(&cargo_config) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("target-dir") {
                        if let Some(dir) = trimmed.split('=').nth(1)
                            .or_else(|| trimmed.split_whitespace().nth(1)) {
                            let dir = dir.trim().trim_matches('"');
                            // 交叉编译路径
                            if !target_triple.is_empty() {
                                let p = PathBuf::from(dir)
                                    .join(&target_triple)
                                    .join("release")
                                    .join(binary_name);
                                if p.exists() {
                                    println!("cargo:info=从 cargo config 找到 target-dir (交叉编译): {}", dir);
                                    paths.push(p);
                                }
                            }
                            // 非交叉编译路径
                            let p = PathBuf::from(dir)
                                .join("release")
                                .join(binary_name);
                            if p.exists() {
                                println!("cargo:info=从 cargo config 找到 target-dir: {}", dir);
                                paths.push(p);
                            }
                        }
                        break;
                    }
                }
            }
        }

        // 候选 2: workspace target/{target_triple}/release/（交叉编译）
        if let Some(ref ws) = workspace_root {
            if !target_triple.is_empty() {
                paths.push(ws.join("target").join(&target_triple).join("release").join(binary_name));
            }
            // 候选 2b: workspace target/release/（默认 target-dir）
            paths.push(ws.join("target").join("release").join(binary_name));
        }

        // 候选 3: 桌面端同级目录的 target/release/
        paths.push(dest_dir.join("target").join("release").join(binary_name));

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
        println!("cargo:warning=搜索路径: {:?}", candidates.iter().map(|p| p.display().to_string()).collect::<Vec<_>>());
        return;
    };

    let src_size = std::fs::metadata(source)
        .map(|m| m.len())
        .unwrap_or(0);

    // 检查目标文件是否需要更新
    if dest_path.exists() {
        let dest_size = std::fs::metadata(&dest_path)
            .map(|m| m.len())
            .unwrap_or(0);

        // v0.5.1 增强：使用 SHA-256 哈希验证（比时间戳更可靠）
        if use_hash {
            match (compute_sha256(&dest_path), compute_sha256(source)) {
                (Some(dest_hash), Some(src_hash)) => {
                    if dest_hash == src_hash {
                        println!(
                            "cargo:info=Sidecar 二进制已是最新版本 (哈希: {}..., {:.1} MB)",
                            &dest_hash[..16],
                            src_size as f64 / 1_048_576.0
                        );
                        return;
                    }
                    println!(
                        "cargo:info=Sidecar 二进制哈希不匹配，需要更新 (目标: {}..., 源: {}...)",
                        &dest_hash[..16], &src_hash[..16]
                    );
                }
                _ => {
                    // 哈希计算失败，回退到大小+时间戳比较
                    println!("cargo:warning=SHA-256 哈希计算失败，回退到大小+时间戳比较");
                    if dest_size == src_size {
                        if let (Ok(dest_time), Ok(src_time)) = (
                            std::fs::metadata(&dest_path).and_then(|m| m.modified()),
                            std::fs::metadata(source).and_then(|m| m.modified()),
                        ) {
                            if dest_time >= src_time {
                                println!(
                                    "cargo:info=Sidecar 二进制已是最新版本 (大小: {:.1} MB)",
                                    src_size as f64 / 1_048_576.0
                                );
                                return;
                            }
                        }
                    }
                }
            }
        } else {
            // 传统时间戳+大小比较
            if dest_size == src_size {
                if let (Ok(dest_time), Ok(src_time)) = (
                    std::fs::metadata(&dest_path).and_then(|m| m.modified()),
                    std::fs::metadata(source).and_then(|m| m.modified()),
                ) {
                    if dest_time >= src_time {
                        println!(
                            "cargo:info=Sidecar 二进制已是最新版本 ({:.1} MB)",
                            src_size as f64 / 1_048_576.0
                        );
                        return;
                    }
                }
            }
        }

        // ═══════════════════════════════════════════════════════════
        // P1-02 修复：构建前检测文件锁定冲突
        // 尝试重命名目标文件检测是否被占用，如果锁定则给出明确提示
        // ═══════════════════════════════════════════════════════════
        // v0.5.7 修复：lock check 文件名根据平台决定
        let lock_check = dest_dir.join(format!("{}.lock_check", dest_name));
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
            // 验证复制后的哈希
            if use_hash {
                if let Some(hash) = compute_sha256(&dest_path) {
                    println!("cargo:info=Sidecar SHA-256: {}...", &hash[..16]);
                }
            }
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