// 许可证: Apache 2.0
//
// Loong Recall (L-RC / 忆) MCP Server — 独立二进制入口
// ======================================================
// 作为独立进程运行，通过 MCP 协议向 IDE 暴露代码检索能力。
//
// 支持两种传输模式:
//   HTTP 模式: code-memory-server --src-dir ./src --port 3099
//   Stdio 模式: code-memory-server --src-dir ./src --stdio
//              （标准 MCP 通信，推荐全局部署用此模式）
//
// 启动后 IDE 可通过 MCP 配置连接此服务，AI 助手即可调用 search_code 工具。

use code_memory::{server, CodeMemoryManager, JsonPersistence, LlmApiConfig, MemoryStore};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

// 进程守护：单例锁避免僵尸进程、端口自适应避免冲突、优雅关闭自动清理
use code_memory::process_guard::{self, SingletonLock};
// 配置持久化：桌面端agent配置保存与加载
use code_memory::config::LrcConfig;
// V2 模块：项目指纹、统一数据目录、数据迁移
use code_memory::data_dir::DataDir;
use code_memory::migration;

#[tokio::main]
async fn main() {
    // 运行时防护：反调试 + 完整性校验（必须在任何业务逻辑之前执行）
    code_memory::guard::risk_aware_guard();

    // ════════════════════════════════════════════════════════════════
    // 全局镜像守卫 — 在所有代码路径之前强制设置
    // ════════════════════════════════════════════════════════════════
    if std::env::var("HF_ENDPOINT").is_err() {
        std::env::set_var("HF_ENDPOINT", "https://hf-mirror.com");
    }

    // 核心逻辑放入 try_run()，确保所有 Drop 析构函数执行完毕后再 exit
    // 这解决了此前 std::process::exit 跳过 SingletonLock Drop 导致僵尸锁残留的问题
    let exit_code = match try_run().await {
        Ok(()) => 0,
        Err(msg) => {
            eprintln!("{msg}");
            1
        }
    };

    // 这是唯一的 std::process::exit 调用点——此时所有局部变量（包括 _singleton_lock）
    // 的 Drop 已经执行完毕，锁文件已安全清理
    std::process::exit(exit_code);
}

/// 主运行逻辑，返回 Result 以避免 std::process::exit 跳过 Drop 析构
#[allow(unused_assignments)]
async fn try_run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();

    let mut src_dir = String::new();
    let mut host = String::from("127.0.0.1");
    let mut port: u16 = 3099;
    let mut port_explicitly_set = false; // 追踪用户是否显式指定了 --port
    let mut stdio_mode = false;
    let mut global_mode = false;
    let mut db_path: Option<String> = None;
    let mut data_dir: Option<String> = None; // V2: --data-dir 统一数据根目录
    let mut llm_api_raw: Option<String> = None;
    let mut proxy: Option<String> = None;
    #[allow(unused_variables, unused_assignments)]
    let mut mode = String::from("fast"); // 默认 Tier 1: 零网络、零下载
    let mut install_ide: Option<String> = None;
    let mut list_ides_mode = false; // --list-ides：列出支持的 IDE 和工具列表
    let mut benchmark_mode = false;
    let mut benchmark_json = false;
    let mut dashboard_mode = false;
    let mut multi_window: u32 = 1; // 默认单窗口，--multi-window N 可提高上限
    let mut daemon_mode = false; // --daemon：后台守护模式，供桌面端agent使用
    let mut tray_mode = false; // --tray：启用系统托盘图标
    let mut export_path: Option<String> = None; // V2: --export 导出记忆
    let mut import_path: Option<String> = None; // V2: --import 导入记忆

    // 加载已保存的全局配置（桌面端agent场景）
    let saved_config = LrcConfig::load();

    // CLI 参数解析
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--src-dir" => {
                i += 1;
                if i < args.len() {
                    src_dir = args[i].clone();
                }
            }
            "--host" => {
                i += 1;
                if i < args.len() {
                    host = args[i].clone();
                }
            }
            "--port" => {
                i += 1;
                if i < args.len() {
                    port = match args[i].parse() {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!("警告: 无效端口号 '{}', 使用默认端口 3099 ({})", args[i], e);
                            3099
                        }
                    };
                    port_explicitly_set = true; // 用户显式指定了端口，不覆盖
                }
            }
            "--stdio" => {
                stdio_mode = true;
            }
            "--global" => {
                global_mode = true;
            }
            "--db-path" => {
                i += 1;
                if i < args.len() {
                    db_path = Some(args[i].clone());
                }
            }
            "--data-dir" => {
                i += 1;
                if i < args.len() {
                    data_dir = Some(args[i].clone());
                } else {
                    return Err("错误: --data-dir 需要指定路径\n\
                         用法: code-memory-server --data-dir ~/my-lrc-data"
                        .to_string());
                }
            }
            "--llm-api" => {
                i += 1;
                if i < args.len() {
                    llm_api_raw = Some(args[i].clone());
                }
            }
            "--install-ide" => {
                i += 1;
                if i < args.len() {
                    install_ide = Some(args[i].clone());
                } else {
                    return Err("错误: --install-ide 需要指定 IDE 名称\n\
                         用法: code-memory-server --install-ide <trae|cursor|vscode|windsurf>\n\
                         多 IDE 用逗号分隔: code-memory-server --install-ide trae,cursor,vscode"
                        .to_string());
                }
            }
            "--list-ides" => {
                list_ides_mode = true;
            }
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            "--benchmark" => {
                benchmark_mode = true;
            }
            "--benchmark-json" => {
                benchmark_mode = true;
                benchmark_json = true;
            }
            "--proxy" => {
                i += 1;
                if i < args.len() {
                    proxy = Some(args[i].clone());
                }
            }
            "--mode" => {
                i += 1;
                if i < args.len() {
                    mode = args[i].clone();
                }
            }
            "--dashboard" => {
                dashboard_mode = true;
            }
            "--multi-window" => {
                i += 1;
                if i < args.len() {
                    multi_window = match args[i].parse::<u32>() {
                        Ok(n) if (1..=20).contains(&n) => n,
                        Ok(n) => {
                            eprintln!(
                                "警告: --multi-window 值 {} 不合理，已限制为 1~20，使用 1",
                                n
                            );
                            1
                        }
                        Err(e) => {
                            eprintln!("警告: 无效的窗口数 '{}', 使用默认值 1 ({})", args[i], e);
                            1
                        }
                    };
                }
            }
            "--daemon" => {
                daemon_mode = true;
                dashboard_mode = true; // 守护模式默认启用仪表盘
            }
            "--tray" => {
                tray_mode = true;
            }
            "--export" => {
                i += 1;
                if i < args.len() {
                    export_path = Some(args[i].clone());
                } else {
                    return Err("错误: --export 需要指定输出文件路径\n\
                         用法: code-memory-server --export ~/backup/lrc-export.json"
                        .to_string());
                }
            }
            "--import" => {
                i += 1;
                if i < args.len() {
                    import_path = Some(args[i].clone());
                } else {
                    return Err("错误: --import 需要指定导入文件路径\n\
                         用法: code-memory-server --import ~/backup/lrc-export.json"
                        .to_string());
                }
            }
            _ => {
                return Err(format!("未知参数: {}\n请使用 --help 查看可用选项", args[i]));
            }
        }
        i += 1;
    }

    // 无参启动（双击 exe）：默认进入仪表盘模式
    if args.len() == 1 && !dashboard_mode {
        dashboard_mode = true;
        eprintln!("[仪表盘] 无参启动，进入桌面仪表盘模式");
    }

    // ── 应用已保存的全局配置（仅当CLI未显式指定时） ──
    if !port_explicitly_set && saved_config.default_port != 3099 {
        port = saved_config.default_port;
    }
    if host == "127.0.0.1" && saved_config.default_host != "127.0.0.1" {
        host = saved_config.default_host.clone();
    }
    if llm_api_raw.is_none() {
        if let Some(ref saved_llm) = saved_config.llm_api {
            if !saved_llm.is_empty() {
                llm_api_raw = Some(saved_llm.clone());
                eprintln!(
                    "[配置] 从全局配置加载 LLM API: {}...",
                    &saved_llm[..saved_llm.len().min(30)]
                );
            }
        }
    }

    // 处理 --list-ides 命令（列出支持的 IDE 和工具列表）
    if list_ides_mode {
        print_ides_list();
        return Ok(());
    }

    // 处理 --install-ide 命令（自动配置 IDE 的 MCP 连接）
    // V2: 支持逗号分隔多 IDE
    if let Some(ref ide) = install_ide {
        let ides: Vec<&str> = ide
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if ides.is_empty() {
            return Err("错误: --install-ide 需要至少指定一个有效的 IDE 名称".to_string());
        }
        for single_ide in &ides {
            install_ide_config(single_ide);
        }
        return Ok(());
    }

    // 处理 --benchmark 命令（运行三层基准测试）
    if benchmark_mode {
        run_benchmark_mode(benchmark_json);
        return Ok(());
    }

    // ════════════════════════════════════════════════════════════
    // V2: --export 导出记忆数据
    // ════════════════════════════════════════════════════════════
    if let Some(ref path) = export_path {
        let dd = if global_mode {
            DataDir::for_global()
        } else {
            let src_path = std::path::Path::new(&src_dir);
            DataDir::for_project(src_path)
        };
        match code_memory::export::export_memories(
            &dd,
            Some(std::path::Path::new(path)),
            if global_mode {
                None
            } else {
                Some(std::path::Path::new(&src_dir))
            },
        ) {
            Ok(result) => {
                println!("导出成功!");
                println!("  文件: {}", result.file_path.display());
                println!("  记忆: {} 条", result.memory_count);
                println!("  代码片段: {} 个", result.chunk_count);
                println!("  文件大小: {} bytes", result.file_size);
            }
            Err(e) => {
                eprintln!("导出失败: {e}");
                return Err(e);
            }
        }
        return Ok(());
    }

    // ════════════════════════════════════════════════════════════
    // V2: --import 导入记忆数据
    // ════════════════════════════════════════════════════════════
    if let Some(ref path) = import_path {
        let dd = if global_mode {
            DataDir::for_global()
        } else {
            let src_path = std::path::Path::new(&src_dir);
            DataDir::for_project(src_path)
        };
        match code_memory::export::import_memories(std::path::Path::new(path), &dd, false) {
            Ok(result) => {
                println!("导入成功!");
                println!("  记忆: {} 条", result.memories_imported);
                println!("  代码片段: {} 个", result.chunks_imported);
                println!("  归档: {} 条", result.archive_imported);
            }
            Err(e) => {
                eprintln!("导入失败: {e}");
                return Err(e);
            }
        }
        return Ok(());
    }

    // 在 stdio 模式下，状态信息输出到 stderr（stdout 留给 MCP 协议）
    let log = move |msg: &str| {
        if stdio_mode {
            eprintln!("{msg}");
        } else {
            println!("{msg}");
        }
    };

    log("═══════════════════════════════════════════");
    log(&format!(
        "  Loong Recall (L-RC / 忆) v{}",
        env!("CARGO_PKG_VERSION")
    ));
    log("  你的私人记忆管家 — 代码搜索 + 记忆服务");
    log("═══════════════════════════════════════════");

    // 配置网络代理（在模型下载等网络操作之前设置）
    if let Some(ref proxy_url) = proxy {
        std::env::set_var("HTTP_PROXY", proxy_url);
        std::env::set_var("HTTPS_PROXY", proxy_url);
        log(&format!("   代理: {proxy_url} (已应用到 HTTP/HTTPS 请求)"));
    }

    // 确定源码目录：默认使用当前工作目录
    let src_dir = if src_dir.is_empty() {
        let default = std::path::PathBuf::from(".");
        if default.exists() {
            default
                .canonicalize()
                .unwrap_or(default)
                .to_string_lossy()
                .to_string()
        } else {
            String::from(".")
        }
    } else {
        src_dir
    };

    // 确定记忆数据目录 — V2 统一数据目录结构
    // 优先级: --db-path > --data-dir > --global > 项目指纹模式
    let (data_dir, data_dir_manager) = if let Some(ref custom_path) = db_path {
        // --db-path: 完全自定义路径（向后兼容）
        (custom_path.clone(), DataDir::for_custom(custom_path))
    } else if let Some(ref custom_root) = data_dir {
        // --data-dir: 自定义数据根目录
        let dd = DataDir::for_custom(custom_root);
        (custom_root.clone(), dd)
    } else if global_mode {
        // --global: 全局模式
        let dd = DataDir::for_global();
        (dd.data_path().to_string_lossy().to_string(), dd)
    } else {
        // V2 默认: 项目指纹模式
        let src_path = std::path::Path::new(&src_dir);
        let dd = DataDir::for_project(src_path);
        let (fingerprint, canonical) =
            code_memory::project_id::project_fingerprint_with_path(src_path);
        log(&format!("   项目指纹: {fingerprint}"));
        log(&format!("   规范化路径: {canonical}"));
        (dd.data_path().to_string_lossy().to_string(), dd)
    };

    log(&format!("   记忆数据目录: {data_dir}"));

    // ════════════════════════════════════════════════════════════
    // V2: 自动迁移旧版数据（如果存在）
    // ════════════════════════════════════════════════════════════
    let src_path = std::path::Path::new(&src_dir);
    if migration::needs_migration(src_path) {
        log("  检测到旧版数据，开始自动迁移...");
        match migration::migrate_legacy_to_v2(
            src_path,
            data_dir_manager.data_path(),
            false, // 实际执行迁移
        ) {
            Ok(result) => {
                log(&migration::format_migration_report(&result));
                if !result.is_success() {
                    log("  ⚠ 部分文件迁移失败，旧数据已保留，可稍后重试");
                }
            }
            Err(e) => {
                log(&format!("  ⚠ 迁移过程中出现错误: {e}，跳过迁移"));
            }
        }
    }

    // ========== 进程守护：单例锁（Drop 时自动清理 ==========
    let _singleton_lock = SingletonLock::acquire(std::path::Path::new(&data_dir), multi_window)
        .map_err(|e| e.to_string())?;
    log(&format!(
        "   进程锁: 已获取 (PID: {}, 窗口上限: {})",
        std::process::id(),
        multi_window
    ));

    // 前置验证：源码目录必须存在且为目录
    let src_path = std::path::Path::new(&src_dir);
    if !src_path.exists() {
        return Err(format!(
            "错误: 源码目录不存在: {src_dir}\n提示: 请使用 --src-dir 指定正确的项目路径"
        ));
    }
    if !src_path.is_dir() {
        return Err(format!("错误: 指定路径不是目录: {src_dir}"));
    }

    // 创建持久化后端和记忆存储
    let persistence = JsonPersistence::new(&data_dir).map_err(|e| {
        format!(
            "致命错误: 无法创建数据目录或初始化持久化后端\n  路径: {data_dir}\n  原因: {e}\n  建议: 检查磁盘空间和目录写入权限"
        )
    })?;
    let memory_store = Arc::new(Mutex::new(MemoryStore::new(persistence)));

    // ╔═══════════════════════════════════════════════════════════════╗
    // ║  第2位 — Tier 2: 配置 LLM API Key（优先引导）               ║
    // ╚═══════════════════════════════════════════════════════════════╝
    let mut llm_api_configured = llm_api_raw.is_some();
    if !stdio_mode && !daemon_mode && !llm_api_configured {
        if ask_user_confirmation("  是否现在配置 LLM API Key？") {
            use std::io::{self, Write};
            print!("  > ");
            io::stdout().flush().ok();
            let mut api_input = String::new();
            if io::stdin().read_line(&mut api_input).is_ok() {
                let trimmed = api_input.trim().to_string();
                if !trimmed.is_empty() {
                    llm_api_raw = Some(trimmed);
                    llm_api_configured = true;
                    log("  ✓ LLM API Key 已配置，查询翻译将在搜索时自动启用");
                }
            }
            if !llm_api_configured {
                log("  → 输入为空，保持 Tier 1 Fast Match");
            }
        } else {
            log("  → 跳过 LLM API 配置（可稍后在仪表盘「设置」页面配置）");
        }
    }

    if llm_api_configured {
        log("  搜索增强: Tier 2 LLM API 已启用 ✓");
    }

    // 解析 LLM API 配置
    let llm_api = match llm_api_raw {
        Some(ref raw) => LlmApiConfig::parse(raw).map_err(|e| {
            format!("错误: LLM API 配置解析失败: {e}\n提示: 格式为 openai:sk-xxx:model 或 ollama:host:model")
        })?,
        None => LlmApiConfig::None,
    };
    match &llm_api {
        LlmApiConfig::OpenAI { model, .. } => {
            log(&format!("   LLM 增强: OpenAI ({model}) → 查询翻译已启用"));
        }
        LlmApiConfig::Ollama { host: oh, model } => {
            log(&format!(
                "   LLM 增强: Ollama ({model}@{oh}) → 查询翻译已启用"
            ));
        }
        _ => {}
    }

    // ════════════════════════════════════════════════════════════
    // 创建搜索管理器 — Tier 1 Fast Match（零网络、零下载）
    // ════════════════════════════════════════════════════════════
    log("\n═══════════════════════════════════════════");
    log("  第1位: Tier 1 — Fast Match（已就绪）");
    if llm_api_configured {
        log("  第2位: Tier 2 — LLM API 增强（已启用）");
    }
    log("═══════════════════════════════════════════");
    log("   搜索引擎: 关键词匹配 · 零网络 · 零下载");
    let mgr = CodeMemoryManager::new();
    log(&format!("\n正在索引项目代码: {src_dir}..."));

    // ── 后台索引：不阻塞 HTTP 服务启动 ──
    let index_src = src_dir.clone();
    let index_log = Arc::new(move |msg: &str| {
        eprintln!("[索引] {msg}");
    });

    let state = Arc::new(server::AppState {
        manager: Arc::new(Mutex::new(Box::new(mgr))),
        memory_store: memory_store.clone(),
        src_dir: src_dir.clone(),
        llm_api: Arc::new(tokio::sync::RwLock::new(llm_api.clone())),
        // 后台索引状态跟踪（false = 索引中，true = 已完成）
        indexing_complete: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        // 服务启动时间（用于 /health 端点计算 uptime）
        started_at: chrono::Utc::now(),
    });

    // ════════════════════════════════════════════════════════════
    // 后台索引（所有模式都需要，不阻塞服务启动）
    // ════════════════════════════════════════════════════════════
    let index_state = state.clone();
    let index_log_bg = Arc::clone(&index_log);
    let indexing_flag = Arc::clone(&state.indexing_complete);
    tokio::spawn(async move {
        index_log_bg("[后台] 开始索引项目代码...");
        let mut bg_mgr = CodeMemoryManager::new();
        match bg_mgr.index_project(&index_src) {
            Ok(_count) => {
                let stats = bg_mgr.get_stats();
                index_log_bg(&format!(
                    "索引完成: {} 个文件 → {} 个代码片段",
                    stats.file_count, stats.total_chunks
                ));
                let mut state_mgr = index_state.manager.lock().await;
                *state_mgr = Box::new(bg_mgr);
                index_log_bg("索引已生效，搜索服务已就绪");
            }
            Err(e) => {
                index_log_bg(&format!("索引失败: {e}（部分文件可能无法检索）"));
            }
        }
        // 无论成功或失败，标记索引阶段已完成
        indexing_flag.store(true, std::sync::atomic::Ordering::Release);
        index_log_bg("[后台] 索引阶段结束");
    });

    // ── 根据运行模式选择通信协议 ──
    if stdio_mode {
        // Stdio 模式：通过 stdin/stdout 进行 JSON-RPC 通信
        // 这是 IDE MCP 标准通信方式，不启动 HTTP 服务器
        // 不绑定端口、不打开浏览器、不启动托盘
        log("\n📡 MCP Stdio 模式已就绪，等待 IDE 连接...");
        server::run_stdio(state).await;
    } else {
        // ════════════════════════════════════════════════════════════
        // HTTP 模式：端口绑定 + axum HTTP 服务启动
        // ════════════════════════════════════════════════════════════
        log("\n🚀 HTTP 服务启动中（端口自适应）...");
        log(&format!("   从端口 {} 开始尝试绑定...", port));

        let (listener, actual_port) = process_guard::find_available_port(&host, port, 100)
            .await
            .map_err(|e| {
                format!("{e}\n提示: 请关闭占用端口的程序后重试，或使用 --port 指定其他起始端口")
            })?;

        log(&format!(
            "   浏览器即将自动打开仪表盘: http://localhost:{actual_port}/dashboard"
        ));

        // 自动打开浏览器（延迟 500ms 确保服务就绪）
        let dashboard_url = format!("http://localhost:{actual_port}/dashboard");
        let open_url = dashboard_url.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            match code_memory::dashboard::open_dashboard(&open_url) {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("[仪表盘] 打开浏览器失败: {e}");
                    eprintln!("[仪表盘] 请手动访问: {open_url}");
                }
            }
        });

        // ── 保存配置到全局配置文件 ──
        let mut cfg = LrcConfig::load();
        cfg.default_port = actual_port;
        cfg.default_host = host.clone();
        cfg.max_multi_window = multi_window as u8;
        if let Some(ref llm) = llm_api_raw {
            if !llm.is_empty() {
                cfg.llm_api = Some(llm.clone());
            }
        }
        if let Err(e) = cfg.save() {
            eprintln!("[配置] 保存全局配置失败: {e}");
        } else {
            let config_path = LrcConfig::get_config_path().unwrap_or_default();
            eprintln!("[配置] 已保存到 {}", config_path.display());
        }

        // ── 系统托盘（Windows 原生） ──
        if tray_mode || daemon_mode {
            #[cfg(feature = "webbrowser")]
            {
                let tray_url = dashboard_url.clone();
                std::thread::spawn(move || {
                    if let Err(e) = code_memory::tray::start_tray(tray_url) {
                        eprintln!("[托盘] 启动失败: {e}");
                    }
                });
            }
        }

        // tokio::select! 实现优雅关闭 — _singleton_lock 的 Drop 自动清理锁文件
        tokio::select! {
            result = server::serve_on_listener(state, &host, actual_port, listener) => {
                result.map_err(|e| format!("服务启动失败: {e}"))?;
            }
            _ = process_guard::wait_for_shutdown_signal() => {
                log("\n收到关闭信号，正在优雅退出...");
                // _singleton_lock 的 Drop 在此作用域结束时自动清理锁文件
            }
        }
    }

    Ok(())
}

/// 交互式询问用户确认（带超时保护，防止 Hidden 窗口环境 stdin 阻塞）
///
/// 返回 true 表示用户确认，false 表示跳过或拒绝。
///
/// 安全机制：
///   - stdout 不是终端 → 立即返回 false（管道重定向/文件输出）
///   - 5 秒内无输入 → 超时返回 false（Hidden 窗口/后台进程/无人值守）
///   - 用户输入 y/yes → 返回 true，其他输入 → 返回 false
fn ask_user_confirmation(prompt: &str) -> bool {
    use std::io::{self, IsTerminal, Write};
    // 非 TTY 环境（管道重定向/文件输出）→ 立即跳过，避免 stdin 阻塞
    if !io::stdout().is_terminal() {
        return false;
    }
    print!("{prompt} (y/N，5秒后自动跳过): ");
    io::stdout().flush().ok();

    // 使用 channel + 线程实现带超时的 stdin 读取
    // 原因：Start-Process -WindowStyle Hidden 创建的进程，
    //       stdin/stdout 仍是"终端"但无人输入，read_line 会永久阻塞
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut input = String::new();
        let _ = io::stdin().read_line(&mut input);
        let _ = tx.send(input);
    });

    match rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(input) => {
            let trimmed = input.trim().to_lowercase();
            trimmed == "y" || trimmed == "yes"
        }
        Err(_) => {
            // 超时或 channel 断开 → 自动跳过，确保永远不会阻塞
            println!(); // 换行，保持输出整洁
            false
        }
    }
}

/// 索引项目并输出统计信息，失败时返回错误而非杀死进程
#[allow(dead_code)]
fn index_and_report<E: code_memory::engine::encoder::CodeEncoder>(
    mgr: &mut CodeMemoryManager<E>,
    src_dir: &str,
    log: &impl Fn(&str),
) -> Result<(), String> {
    match mgr.index_project(src_dir) {
        Ok(_count) => {
            let stats = mgr.get_stats();
            log(&format!(
                "   索引完成: {} 个文件 → {} 个代码片段",
                stats.file_count, stats.total_chunks
            ));
            log(&format!(
                "   类型: fn({}) struct({}) impl({}) trait({}) enum({}) mod({})",
                stats.type_counts.get("fn").unwrap_or(&0),
                stats.type_counts.get("struct").unwrap_or(&0),
                stats.type_counts.get("impl").unwrap_or(&0),
                stats.type_counts.get("trait").unwrap_or(&0),
                stats.type_counts.get("enum").unwrap_or(&0),
                stats.type_counts.get("mod").unwrap_or(&0),
            ));
            Ok(())
        }
        Err(e) => {
            let msg = format!("   索引失败: {e} (请检查 --src-dir 路径是否正确)");
            log(&msg);
            Err(msg)
        }
    }
}

/// 列出所有支持的 IDE 和工具（P3-07 修复）
///
/// 输出格式化的表格，包含 IDE 名称、配置文件路径和可安装性。
/// 支持通过 `code-memory-server --list-ides` 调用。
fn print_ides_list() {
    println!("支持的 IDE 和 AI 工具列表");
    println!("═══════════════════════════════════════════");
    println!();
    println!("  名称               类别        可安装");
    println!("  ────────────────   ──────────  ──────");

    // IDE 类
    for (name, cat, installable) in SUPPORTED_IDES {
        let mark = if *installable { "✓ 是" } else { "  —" };
        println!("  {:<20} {:<12} {}", name, cat, mark);
    }
    println!();
    println!("用法:");
    println!("  安装单个 IDE:   code-memory-server --install-ide <名称>");
    println!("  安装多个 IDE:   code-memory-server --install-ide <名称1>,<名称2>");
    println!("  列出所有 IDE:   code-memory-server --list-ides");
    println!("  查看完整帮助:   code-memory-server --help");
}

/// 支持的 IDE 数据库
/// (名称, 分类, 是否支持 --install-ide 自动配置)
const SUPPORTED_IDES: &[(&str, &str, bool)] = &[
    ("trae", "IDE", true),
    ("trae-cn", "IDE", true),
    ("cursor", "IDE", true),
    ("vscode", "IDE", true),
    ("windsurf", "IDE", true),
    ("kiro", "IDE", true),
    ("gemini", "CLI/桌面", true),
    ("gemini-cli", "CLI/桌面", true),
    ("comate", "AI 助手", true),
    ("roo", "AI 助手", true),
    ("roo-code", "AI 助手", true),
    ("cline", "AI 助手", true),
    ("cloudbase", "平台", true),
    ("cloudbase-mcp", "平台", true),
];

/// 根据 IDE 名称返回 MCP 配置文件路径
///
/// 支持的 IDE：trae, trae-cn, cursor, vscode, windsurf。
/// 不支持的 IDE 会打印错误信息并退出进程。
fn get_ide_config_path(ide: &str) -> PathBuf {
    let home = dirs_next::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    match ide.to_lowercase().as_str() {
        "trae" | "trae-cn" => {
            // Trae CN（中文版）使用 ~/.trae-cn/trae-mcp.json
            let trae_cn_cfg = home.join(".trae-cn").join("trae-mcp.json");
            // Trae（国际版）使用 ~/.trae/mcp.json
            let trae_cfg = home.join(".trae").join("mcp.json");

            // 根据安装的版本自动选择正确的配置路径
            if ide == "trae-cn" || trae_cn_cfg.exists() {
                trae_cn_cfg
            } else {
                trae_cfg
            }
        }
        "cursor" => home.join(".cursor").join("mcp.json"),
        "vscode" | "code" => {
            if cfg!(target_os = "windows") {
                home.join("AppData")
                    .join("Roaming")
                    .join("Code")
                    .join("User")
                    .join("globalStorage")
                    .join("mcp.json")
            } else if cfg!(target_os = "macos") {
                home.join("Library")
                    .join("Application Support")
                    .join("Code")
                    .join("User")
                    .join("globalStorage")
                    .join("mcp.json")
            } else {
                home.join(".config")
                    .join("Code")
                    .join("User")
                    .join("globalStorage")
                    .join("mcp.json")
            }
        }
        "windsurf" => home.join(".windsurf").join("mcp.json"),
        "kiro" => home.join(".kiro").join("settings").join("mcp.json"),
        "gemini" | "gemini-cli" => home.join(".gemini").join("settings.json"),
        "comate" => home.join(".comate").join("mcp.json"),
        "roo" | "roo-code" => home.join(".roo").join("mcp.json"),
        "cline" => home.join(".cline").join("mcp.json"),
        "cloudbase" | "cloudbase-mcp" => home.join(".cloudbase-mcp").join("mcp.json"),
        _ => {
            eprintln!("错误: 不支持的 IDE/工具: {ide}");
            eprintln!("支持的 IDE: trae, trae-cn, cursor, vscode, windsurf, kiro");
            eprintln!("支持的 CLI/桌面工具: gemini, gemini-cli, comate, roo, cline, cloudbase");
            eprintln!("用法: code-memory-server --install-ide <名称>");
            std::process::exit(1);
        }
    }
}

/// 读取或创建空配置
///
/// 如果配置文件存在，尝试解析 JSON；解析失败或文件不存在时返回空配置。
fn read_or_create_config(path: &std::path::Path) -> serde_json::Value {
    if path.exists() {
        match std::fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(v) => {
                    println!("  ✓ 已读取现有配置");
                    v
                }
                Err(e) => {
                    eprintln!("  ⚠ 现有配置文件格式错误，将创建新配置: {e}");
                    serde_json::json!({})
                }
            },
            Err(e) => {
                eprintln!("  ⚠ 无法读取现有配置，将创建新配置: {e}");
                serde_json::json!({})
            }
        }
    } else {
        println!("  配置文件不存在，将创建新配置");
        serde_json::json!({})
    }
}

/// 写入 IDE 配置并打印成功消息
fn write_ide_config(
    path: &std::path::Path,
    _config: &serde_json::Value,
    config_str: &str,
    ide: &str,
    _exe_path: &str,
) {
    match std::fs::write(path, config_str) {
        Ok(()) => {
            println!("  ✓ 配置已写入: {}", path.display());
            println!();
            println!("═══════════════════════════════════════════");
            println!("  安装完成！");
            println!("═══════════════════════════════════════════");
            println!();
            println!("  下一步:");
            println!("  1. 重启 {ide}");
            println!("  2. 在 AI 对话中即可使用以下工具:");
            println!("     - search_code: 搜索项目代码");
            println!("     - remember: 让 AI 记住重要信息");
            println!("     - recall: 检索历史记忆");
            println!("     - dao_metrics: 查看记忆系统健康度");
            println!();
            println!("  ⚠ 注意: 每个项目只能运行一个 LRC 实例。");
            println!("    如果你在同一个项目中打开了多个聊天窗口，");
            println!("    第二个窗口的 LRC 将不会重复启动（这是正常限制）。");
            println!("    不同项目之间完全隔离，互不影响。");
            println!();
            println!(
                "  💡 卸载保护: 记忆数据存储在用户目录 (~/.loong-recall/)，卸载 IDE 不会丢失。"
            );
            println!("    如需备份请使用: code-memory-server --export ~/backup/lrc-data.json");
            println!();
            println!("  提示: 启动 HTTP 服务查看可视化仪表盘:");
            println!("    code-memory-server --port 3099");
            println!("    然后访问 http://localhost:3099/dashboard");
        }
        Err(e) => {
            eprintln!("  ✗ 配置写入失败: {e}");
            eprintln!("  提示: 请检查目录权限或手动编辑配置文件");
            eprintln!("  手动配置内容:");
            println!("{config_str}");
            std::process::exit(1);
        }
    }
}

/// 自动检测 IDE 并写入 MCP 配置文件
///
/// 支持的 IDE：trae, cursor, vscode, windsurf
/// 工作原理：
/// 1. 检测 IDE 的 MCP 配置文件路径
/// 2. 读取现有配置（如果存在）
/// 3. 将 loong-recall 的 MCP 配置合并进去
/// 4. 写回配置文件
fn install_ide_config(ide: &str) {
    // 获取当前可执行文件路径
    let exe_path = match std::env::current_exe() {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(e) => {
            eprintln!("错误: 无法获取可执行文件路径: {e}");
            std::process::exit(1);
        }
    };

    // 确定 IDE 的 MCP 配置文件路径
    let config_path = get_ide_config_path(ide);

    println!("═══════════════════════════════════════════");
    println!("  Loong Recall IDE 自动配置工具");
    println!("═══════════════════════════════════════════");
    println!();
    println!("  目标 IDE: {ide}");
    println!("  配置文件: {}", config_path.display());
    println!("  可执行文件: {exe_path}");
    println!();

    // 创建目录（如果不存在）
    if let Some(parent) = config_path.parent() {
        if !parent.exists() {
            match std::fs::create_dir_all(parent) {
                Ok(()) => println!("  ✓ 已创建配置目录: {}", parent.display()),
                Err(e) => {
                    eprintln!("  ✗ 无法创建配置目录: {e}");
                    std::process::exit(1);
                }
            }
        }
    }

    // 读取现有配置
    let mut config = read_or_create_config(&config_path);

    // 构建 loong-recall 的 MCP 配置
    let mcp_entry = serde_json::json!({
        "command": exe_path,
        "args": ["--src-dir", ".", "--stdio"],
        "env": {}
    });

    // 检查是否已存在 loong-recall 配置
    if let Some(mcp_servers) = config.get("mcpServers") {
        if let Some(existing) = mcp_servers.get("loong-recall") {
            println!("  ⚠ 已存在 loong-recall 配置，将更新");
            println!("    旧配置: {existing}");
        }
    }

    // 合并配置
    if config.get("mcpServers").is_none() {
        config["mcpServers"] = serde_json::json!({});
    }
    config["mcpServers"]["loong-recall"] = mcp_entry;

    // 写回配置文件
    let config_str = serde_json::to_string_pretty(&config).unwrap_or_else(|e| {
        eprintln!("  ✗ 配置序列化失败: {e}");
        std::process::exit(1);
    });

    write_ide_config(&config_path, &config, &config_str, ide, &exe_path);
}

/// 运行基准测试模式（--benchmark / --benchmark-json）
///
/// 调用共享的 benchmark 模块，输出人类可读或 JSON 格式的三层基准测试报告。
fn run_benchmark_mode(json_output: bool) {
    use std::time::Instant;

    let total_start = Instant::now();
    let report = code_memory::benchmark::run_all_benchmarks(None);
    let total_ms = u64::try_from(total_start.elapsed().as_millis()).unwrap_or(u64::MAX);

    if json_output {
        let json_str = serde_json::to_string_pretty(&serde_json::json!({
            "report_version": report.version,
            "generated_at": report.generated_at,
            "total_duration_ms": total_ms,
            "summary": {
                "total": report.total,
                "passed": report.passed,
                "failed": report.failed,
                "status": if report.failed == 0 { "PASS" } else { "FAIL" },
                "layers": report.layers.iter().map(|l| serde_json::json!({
                    "name": l.name,
                    "total": l.total,
                    "passed": l.passed,
                    "status": l.status,
                })).collect::<Vec<_>>(),
            },
            "radar_chart": report.radar_scores,
            "results": report.results.iter().map(|r| serde_json::json!({
                "name": r.name,
                "layer": r.layer,
                "description": r.description,
                "industry_problem": r.industry_problem,
                "passed": r.passed,
                "score": r.score,
                "details": r.details,
                "duration_ms": r.duration_ms,
            })).collect::<Vec<_>>(),
        }))
        .unwrap_or_else(|e| format!(r#"{{"error":"序列化失败","message":"{e}"}}"#));
        println!("{json_str}");
    } else {
        println!("═══════════════════════════════════════════════════════════");
        println!("  Loong Recall 三层基准测试报告");
        println!("═══════════════════════════════════════════════════════════");
        println!();
        for r in &report.results {
            let status = if r.passed { "✓ 通过" } else { "✗ 失败" };
            println!(
                "  [{}] L{}-{} — {} ({:.1}ms)",
                status, r.layer, r.name, r.description, r.duration_ms
            );
            println!("         评分: {:.2} | {}", r.score, r.details);
        }
        println!();
        println!(
            "  总计: {} 项 | 通过: {} | 失败: {} | 耗时: {}ms",
            report.total, report.passed, report.failed, total_ms
        );
        println!();
        for layer in &report.layers {
            let status_icon = if layer.status == "PASS" { "✓" } else { "✗" };
            println!(
                "  {} {}: {}/{} 通过",
                status_icon, layer.name, layer.passed, layer.total
            );
        }
        println!();
        println!("  雷达图数据（标准化评分 0.0~1.0）：");
        if let serde_json::Value::Object(map) = &report.radar_scores {
            for (key, val) in map {
                let bar_len = (val.as_f64().unwrap_or(0.0).max(0.0) * 20.0) as usize;
                let bar = "█".repeat(bar_len.min(20));
                println!(
                    "    {:<16} [{:<20}] {:.2}",
                    key,
                    bar,
                    val.as_f64().unwrap_or(0.0)
                );
            }
        }
        println!();
        if report.failed == 0 {
            println!("  ✓ 所有基准测试通过！");
        } else {
            println!("  ⚠ {} 项测试未通过。", report.failed);
        }
        println!();
        println!("  提示：使用 --benchmark-json 获取 JSON 格式报告。");
        println!("  提示：也可使用独立工具 code-memory-benchmark 运行。");
        println!();
    }

    if report.failed > 0 {
        std::process::exit(1);
    }
}

fn print_help() {
    println!("Loong Recall (L-RC / 忆) — AI 编程助手的记忆与检索插件");
    println!();
    println!("用法: code-memory-server [选项]");
    println!();
    println!("搜索模式（三层优先级，第1位→第2位→最后）：");
    println!("  第1位 — Fast Match（默认）: 零网络、零下载、秒启动，关键词匹配");
    println!("  第2位 — LLM API Key: 交互式引导配置 API，用 LLM 翻译自然语言查询");
    println!("  最后   — Smart Match: 用户确认后才从国内镜像下载，绝不访问外网");
    println!();
    println!("选项:");
    println!("  --src-dir <路径>    要索引的项目源码目录 [默认: 当前目录]");
    println!("  --host <地址>       HTTP 绑定地址 [默认: 127.0.0.1]");
    println!("  --port <端口>       HTTP 绑定端口 [默认: 3099]");
    println!("  --stdio             使用 stdio 传输模式（IDE 标准 MCP，推荐）");
    println!("  --global            记忆跨项目共享 (~/.loong-recall/global/data/)");
    println!("  --db-path <路径>    自定义记忆数据存储路径（优先级最高）");
    println!("  --data-dir <路径>   自定义数据根目录（V2 统一数据目录）");
    println!("  --llm-api <配置>    配置 LLM 查询翻译 (Tier 2)，格式见下方说明");
    println!("  --proxy <代理地址>    HTTP/HTTPS 代理（如 http://127.0.0.1:7890）");
    println!(
        "  --mode <模式>        搜索模式: fast(默认/Tier1) | smart(Tier3,用户确认+国内镜像下载)"
    );
    println!("  --dashboard          启动桌面仪表盘模式（自动打开浏览器，含完整交互引导）");
    println!("  --daemon             后台守护模式（无控制台运行，供桌面端agent使用）");
    println!("  --tray               启用系统托盘图标（Windows 原生，右键菜单操作）");
    println!("  --multi-window <N>   多窗口上限 (1~20, 默认 1)，允许同项目多窗口运行");
    println!("  --install-ide <IDE>  自动配置 IDE/工具的 MCP 连接");
    println!("                       IDE: trae, trae-cn, cursor, vscode, windsurf, kiro");
    println!("                       CLI/桌面: gemini, comate, roo, cline, cloudbase");
    println!("                       多工具用逗号分隔: --install-ide trae,cursor");
    println!("  --benchmark         运行三层基准测试（人类可读输出）");
    println!("  --benchmark-json    运行三层基准测试（JSON 格式，供 CI/CD 或仪表盘使用）");
    println!("  --export <路径>     导出记忆数据到 JSON 文件（备份）");
    println!("  --import <路径>     从 JSON 文件导入记忆数据（恢复）");
    println!("  --help, -h          显示此帮助信息");
    println!();
    println!("举个栗子:");
    println!("  # Tier 1 — 默认快速模式（零网络、零下载、秒启动）");
    println!("  code-memory-server --src-dir ./src --port 3099");
    println!();
    println!("  # Tier 1 — 一键安装到 Trae IDE（自动配置 MCP）");
    println!("  code-memory-server --install-ide trae");
    println!();
    println!("  # Tier 2 — 配置 LLM 查询翻译，用自然语言搜索代码");
    println!("  code-memory-server --src-dir ./src --stdio --llm-api openai:sk-xxx:gpt-4o-mini");
    println!();
    println!("  # Tier 2 — 使用本地 Ollama 模型（零成本，无需下载）");
    println!("  code-memory-server --src-dir ./src --stdio --llm-api ollama:localhost:llama3");
    println!();
    println!("  # Tier 3 — 语义搜索（用户确认后才从 hf-mirror.com 国内镜像下载，约 1-3 分钟）");
    println!("  code-memory-server --mode smart");
    println!();
    println!("  # 全局记忆，跨项目共享偏好和知识");
    println!("  code-memory-server --global --stdio");
    println!();
    println!("  # 桌面端agent — 后台守护进程 + 系统托盘（供各种桌面agent调用）");
    println!("  code-memory-server --daemon --tray --src-dir ./src");
    println!();
    println!("  # 双击 exe 或 --dashboard 打开可视化仪表盘（含完整交互引导）");
    println!("  code-memory-server --dashboard");
    println!();
    println!("  # 多窗口模式：允许同项目最多 3 个窗口同时运行 LRC");
    println!("  code-memory-server --src-dir ./src --multi-window 3 --stdio");
    println!();
    println!("  code-memory-server --db-path D:/my-data --stdio");
    println!();
    println!("  # V2: 导出/导入记忆数据（备份与恢复）");
    println!("  code-memory-server --export ~/backup/lrc-2026-06-15.json --src-dir ./src");
    println!("  code-memory-server --import ~/backup/lrc-2026-06-15.json --src-dir ./src");
    println!();
    println!("LLM API 配置格式:");
    println!("  OpenAI:   openai:<sk-key>:<model-name>:<api-base-url>");
    println!("  Ollama:   ollama:<host>:<model-name>");
    println!();
    println!("启动后在 IDE 中配置 MCP 连接，AI 助手即可使用。");
    println!("访问 http://localhost:3099/dashboard 查看可视化记忆管理仪表盘。");
    println!("详细使用说明: https://github.com/zhibaiYingChuan/LRC/blob/main/docs/USER_GUIDE.md");
}
