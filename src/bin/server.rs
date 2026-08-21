// 隐藏控制台窗口：后台进程不需要 CMD 窗口
// MCP stdio 模式下 stdin/stdout 仍然可用，不受影响
#![windows_subsystem = "windows"]

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
// v0.5.4 P2-10 修复：导入后台结晶流水线组件
use code_memory::consolidation::{
    run_consolidation_loop, ConsolidationConfig, ConsolidationPipeline, InMemorySource,
    SurfaceMemorySource,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

// 进程守护：单例锁避免僵尸进程、端口自适应避免冲突、优雅关闭自动清理
use code_memory::process_guard::{self, SingletonLock};
// 配置持久化：桌面端agent配置保存与加载
use code_memory::config::{LrcConfig, DEFAULT_PORT};
// V2 模块：项目指纹、统一数据目录
// v0.8.0：migration 模块改为通过 API（POST /v1/migrate）调用，不在启动时自动执行
use code_memory::data_dir::DataDir;

/// 全局退出码（v0.8.17 引入，解决 P0-2 退出码不区分问题）
///
/// 默认值 1（其他未分类错误）。当 SingletonLock::acquire 返回 GuardError 时，
/// try_run() 会将对应错误码写入此变量，main() 读取后用该码退出进程。
/// 桌面端 sidecar_manager.rs 通过 child.wait() 获取退出码，据此映射到
/// 不同的 SidecarStartError 变体，驱动差异化 UX（如"复用现有实例"按钮）。
///
/// 退出码协议：
///   0 = 正常退出
///   1 = 其他未分类错误（兜底，向后兼容旧版）
///   2 = 单例锁冲突（MultiWindowDisabled / AlreadyRunning）
///   3 = 端口绑定失败（NoAvailablePort）
///   4 = 数据目录错误（DataDirNotAvailable）
///   5 = 锁获取失败（LockAcquireFailed）
static EXIT_CODE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(1);

// v0.8.22 P0-A 修复（hcse-resilience-validator）：
//   增加 tokio worker 线程数，避免合成任务阻塞 axum HTTP handler
//   根因：合成任务是 CPU 密集型的，通过 tokio::spawn 启动后占用 worker 线程，
//         当所有 worker 线程被占用时，axum handler 无法执行，/health 等端点超时
//   修复：worker_threads 从默认值（CPU 核心数，通常 4-8）增加到 16，
//         确保合成任务占用部分线程后，axum handler 仍有足够线程处理请求
//   后续优化：将 run_cycle 中的 CPU 密集型操作放到 spawn_blocking 中（v0.8.23 计划）
#[tokio::main(flavor = "multi_thread", worker_threads = 16)]
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
            // 读取 EXIT_CODE 全局变量（try_run() 中 GuardError 会写入对应退出码）
            // 默认值 1（其他未分类错误），GuardError 会改为 2/3/4/5
            EXIT_CODE.load(std::sync::atomic::Ordering::SeqCst)
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
    let mut port: u16 = DEFAULT_PORT;
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
    let mut dev_mode = false; // v0.9.0: --dev 开发模式，端口锁定 3111

    // v0.6.0+ 参赛扩展：参照系实验 CLI 参数
    // 设计原则：默认启用所有功能（避免"无声失败"），由用户通过 flag 显式禁用
    let mut disable_synthesis = false; // --disable-synthesis：禁用合成引擎（基线 B）
    let mut disable_dao_regulator = false; // --disable-dao-regulator：禁用道同构度调节器（基线 B）
    let mut disable_memory = false; // --disable-memory：禁用记忆系统（基线 A）
    let mut exploration_log_path: Option<String> = None; // --exploration-log <path>：启用探索日志

    // 加载已保存的全局配置（仅在非 daemon 模式下加载）
    // daemon 模式下由桌面端统一管理配置，所有配置通过 CLI 参数传递
    // 避免 config.json 和 wizard.json 两套配置冲突
    let saved_config = if daemon_mode {
        LrcConfig::default()
    } else {
        LrcConfig::load()
    };

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
                            eprintln!(
                                "警告: 无效端口号 '{}', 使用默认端口 {} ({})",
                                args[i], DEFAULT_PORT, e
                            );
                            DEFAULT_PORT
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
            "--dev" => {
                // v0.9.0: 开发模式 — 端口锁定 3111，数据目录隔离
                dev_mode = true;
                port = 3111;
                port_explicitly_set = true; // 防止 saved_config 覆盖
                                            // 设置环境变量，供 downstream 代码（如 wizard.json 路径）识别开发模式
                std::env::set_var("LRC_DEV_MODE", "1");
                if data_dir.is_none() {
                    // 开发模式默认使用独立数据目录，防止污染生产数据
                    let home = std::env::var("USERPROFILE")
                        .or_else(|_| std::env::var("HOME"))
                        .unwrap_or_else(|_| ".".to_string());
                    data_dir = Some(format!("{}/.loong-recall/dev/data/", home));
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
            // v0.6.0 新增：model 子命令（模型管理）
            // 用法: code-memory-server model <list|download|use|remove> [args]
            "model" => {
                // 解析子命令
                if i + 1 >= args.len() {
                    return Err("错误: model 子命令需要指定操作\n\
                         用法: code-memory-server model <list|download|use|remove> [args]"
                        .to_string());
                }
                let subcommand = args[i + 1].clone();
                let sub_args = &args[i + 2..];

                // 分发到对应的处理函数
                match subcommand.as_str() {
                    "list" => handle_model_list(),
                    "download" => {
                        if sub_args.is_empty() {
                            return Err("错误: model download 需要指定模型 ID\n\
                                 用法: code-memory-server model download <model_id>\n\
                                 示例: code-memory-server model download BAAI/bge-small-zh"
                                .to_string());
                        }
                        handle_model_download(&sub_args[0])?
                    }
                    "use" => {
                        if sub_args.is_empty() {
                            return Err("错误: model use 需要指定模型 ID\n\
                                 用法: code-memory-server model use <model_id>\n\
                                 示例: code-memory-server model use BAAI/bge-small-zh"
                                .to_string());
                        }
                        handle_model_use(&sub_args[0])?
                    }
                    "remove" => {
                        if sub_args.is_empty() {
                            return Err("错误: model remove 需要指定模型 ID\n\
                                 用法: code-memory-server model remove <model_id>\n\
                                 示例: code-memory-server model remove BAAI/bge-small-zh"
                                .to_string());
                        }
                        handle_model_remove(&sub_args[0])?
                    }
                    _ => {
                        return Err(format!(
                            "错误: 未知的 model 子命令 '{}'\n\
                             可用子命令: list, download, use, remove",
                            subcommand
                        ));
                    }
                }
                return Ok(());
            }
            "--version" | "-V" => {
                println!("{}", env!("CARGO_PKG_VERSION"));
                return Ok(());
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
            // v0.6.0+ 参赛扩展：参照系实验参数
            "--disable-synthesis" => {
                disable_synthesis = true;
            }
            "--disable-dao-regulator" => {
                disable_dao_regulator = true;
            }
            "--disable-memory" => {
                disable_memory = true;
            }
            "--exploration-log" => {
                i += 1;
                if i < args.len() {
                    exploration_log_path = Some(args[i].clone());
                } else {
                    return Err("错误: --exploration-log 需要指定日志文件路径\n\
                         用法: code-memory-server --exploration-log ./exploration.jsonl"
                        .to_string());
                }
            }
            _ => {
                return Err(format!("未知参数: {}\n请使用 --help 查看可用选项", args[i]));
            }
        }
        i += 1;
    }

    // 无参启动（双击 exe）：静默退出，不自动打开任何界面
    // 桌面端由 Tauri 壳层管理 UI，sidecar 仅作为后台服务运行
    // 用户如需独立使用，请通过命令行指定参数
    if args.len() == 1 && !dashboard_mode && !daemon_mode {
        eprintln!("[LRC] 无参启动被拦截。sidecar 仅作为后台服务运行。");
        eprintln!("[LRC] 请使用桌面端程序启动，或通过命令行指定参数。");
        eprintln!("[LRC] 用法: code-memory-server --help");
        return Ok(());
    }

    // ── 应用已保存的全局配置（仅当CLI未显式指定时） ──
    // daemon 模式下由桌面端统一管理配置，跳过 config.json 的端口/主机加载
    // 避免 config.json 和 wizard.json 两套配置冲突
    if !daemon_mode {
        if !port_explicitly_set && saved_config.default_port != DEFAULT_PORT {
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
    }

    // v0.5.4 安全修复：从环境变量读取 LLM API（桌面端通过 env 而非命令行参数传递）
    // v0.5.4 P2-22 修复：环境变量加载移到 daemon 模式判断之外，
    // 确保 daemon 模式下也能从环境变量加载 LLM 配置。
    // 修复前：daemon 模式下完全跳过环境变量加载，如果桌面端未正确传递环境变量，
    //         sidecar 的 state.llm_api 为 None，仪表盘显示"LLM 未配置"。
    // 修复后：daemon 模式下优先从环境变量加载，确保仪表盘状态与桌面端一致。
    if llm_api_raw.is_none() {
        if let Ok(env_llm) = std::env::var("LRC_LLM_API") {
            if !env_llm.is_empty() {
                eprintln!(
                    "[配置] 从环境变量 LRC_LLM_API 加载 LLM API: {}...",
                    &env_llm[..env_llm.len().min(30)]
                );
                llm_api_raw = Some(env_llm);
            }
        }
    }

    // v0.5.4 P2-22 修复：daemon 模式下如果环境变量未传递 LLM 配置，
    // 从 config.json 加载作为后备，确保 sidecar 状态与桌面端一致。
    // 这解决了"桌面端已配置 LLM 但仪表盘显示未配置"的状态不同步问题。
    if daemon_mode && llm_api_raw.is_none() {
        if let Some(ref saved_llm) = saved_config.llm_api {
            if !saved_llm.is_empty() {
                llm_api_raw = Some(saved_llm.clone());
                eprintln!(
                    "[配置] daemon 模式从全局配置后备加载 LLM API: {}...",
                    &saved_llm[..saved_llm.len().min(30)]
                );
            }
        }
    }

    // v0.5.4 P2-22 修复：daemon 模式下如果以上都未加载到 LLM 配置，
    // 从 wizard.json（桌面端向导配置）加载 LLM 配置作为最终后备。
    // 这确保了 sidecar 状态与桌面端 wizard.json 配置一致。
    // 注意：wizard.json 的 encrypted_api_key 存储的是纯 API Key，
    //       需要结合 llm_type/llm_model/llm_base_url 构造完整的 LLM API 字符串。
    if daemon_mode && llm_api_raw.is_none() {
        if let Some(wizard_llm) = load_llm_from_wizard_json() {
            llm_api_raw = Some(wizard_llm.clone());
            eprintln!(
                "[配置] daemon 模式从 wizard.json 后备加载 LLM API: {}...",
                &wizard_llm[..wizard_llm.len().min(30)]
            );
        }
    }

    // 处理 --list-ides 命令（列出支持的 IDE 和工具列表）
    if list_ides_mode {
        print_ides_list();
        return Ok(());
    }

    // 处理 --install-ide 命令（自动配置 IDE 的 MCP 连接）
    // V2: 支持逗号分隔多 IDE
    // v0.9.0 开发模式隔离：开发模式下禁止安装 IDE 配置（避免修改稳定版用户的全局 IDE 配置）
    if let Some(ref ide) = install_ide {
        if dev_mode {
            eprintln!("[开发模式] --install-ide 被拒绝：开发模式下禁止修改全局 IDE 配置。请使用稳定版二进制执行此操作。");
            return Err("开发模式下不允许安装 IDE 配置（避免修改稳定版用户的全局 IDE 配置）。请使用稳定版二进制执行 --install-ide。".to_string());
        }
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
    let (data_dir, _data_dir_manager) = if let Some(ref custom_path) = db_path {
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
    // v0.8.0 "归一"：数据迁移改为 API 触发（POST /v1/migrate）
    // 旧版启动时自动迁移已移除，避免 sidecar 启动延迟和意外数据修改
    // 用户可通过桌面端"数据迁移向导"或直接调用 API 触发迁移
    // ════════════════════════════════════════════════════════════

    // ========== 进程守护：单例锁（Drop 时自动清理 ==========
    // v0.8.17 P0-2 修复：GuardError 时写入退出码到全局变量，main() 据此 exit
    // 桌面端 sidecar_manager.rs 通过 child.wait() 读取退出码，区分单例冲突 vs 其他错误
    let _singleton_lock = SingletonLock::acquire(std::path::Path::new(&data_dir), multi_window)
        .map_err(|e| {
            EXIT_CODE.store(e.exit_code(), std::sync::atomic::Ordering::SeqCst);
            e.to_string()
        })?;
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

    // 创建持久化后端
    let persistence = JsonPersistence::new(&data_dir).map_err(|e| {
        format!(
            "致命错误: 无法创建数据目录或初始化持久化后端\n  路径: {data_dir}\n  原因: {e}\n  建议: 检查磁盘空间和目录写入权限"
        )
    })?;

    // v0.9.0 修复：根据 mode 加载编码器
    // 此前 `--mode smart` 是死参数，MemoryStore::new 永远用统计编码器，
    // 导致"下载模型后仍降级"。现在 ml feature 下：
    //   - --mode smart 显式启用，或本地模型已就绪时，加载 ML 语义编码器
    //   - 否则降级到统计编码器（零网络、零下载、秒启动）
    #[cfg(feature = "ml")]
    let (store, ml_loaded) = {
        let want_ml = mode == "smart" || local_ml_model_ready();
        let (encoder, loaded) = if want_ml {
            // v0.9.1 算法泄露合规：通过 engine 层工厂函数创建编码器
            match code_memory::engine::create_smart_encoder() {
                Ok((enc, true)) => {
                    log("  ✓ ML 语义编码器加载成功（Smart Match 已启用）");
                    (enc, true)
                }
                _ => {
                    log("  ⚠ ML 编码器加载失败，降级统计模式");
                    (code_memory::engine::create_statistical_encoder(), false)
                }
            }
        } else {
            (code_memory::engine::create_statistical_encoder(), false)
        };
        (MemoryStore::new_with_encoder(persistence, encoder), loaded)
    };
    #[cfg(not(feature = "ml"))]
    let (store, ml_loaded) = (MemoryStore::new(persistence), false);
    let memory_store = Arc::new(Mutex::new(store));

    // ╔═══════════════════════════════════════════════════════════════╗
    // ║ v0.6.0+ 参赛扩展：应用参照系实验 CLI 参数                   ║
    // ║ 设计原则：默认启用所有功能，由 flag 显式禁用                ║
    // ╚═══════════════════════════════════════════════════════════════╝
    {
        let mut store = memory_store.lock().await;

        // --exploration-log <path>：注入探索日志记录器
        if let Some(ref log_path) = exploration_log_path {
            let logger = code_memory::engine::exploration_log::ExplorationLogger::new(
                std::path::PathBuf::from(log_path),
                format!(
                    "exp_{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                ),
            );
            // 先记录实验配置事件
            logger.log_experiment_config(serde_json::json!({
                "disable_synthesis": disable_synthesis,
                "disable_dao_regulator": disable_dao_regulator,
                "disable_memory": disable_memory,
                "src_dir": src_dir,
                "data_dir": data_dir,
                "port": port,
                "stdio_mode": stdio_mode,
                "timestamp": chrono::Utc::now().to_rfc3339(),
            }));
            // 记录 sidecar 启动事件
            logger.log_sidecar_started(0, &args.join(" "));
            store.set_exploration_logger(logger);
            log(&format!("   探索日志: 已启用 → {log_path}"));
        }

        // --disable-synthesis：禁用合成引擎（基线 B）
        // 通过设置 synthesis_min_cluster 为 usize::MAX，使合成永远不会触发
        if disable_synthesis {
            store.synthesis_min_cluster = usize::MAX;
            log("   ⚠ 合成引擎: 已禁用（基线 B：no_evolution）");
        }

        // --disable-dao-regulator：禁用道同构度调节器（基线 B）
        if disable_dao_regulator {
            store.dao_regulator.auto_regulate = false;
            log("   ⚠ 道同构度调节器: 已禁用（基线 B：no_evolution）");
        }

        // --disable-memory：禁用记忆系统（基线 A）
        // 通过环境变量标记，remember 请求将返回成功但不实际存储
        if disable_memory {
            std::env::set_var("LRC_DISABLE_MEMORY", "1");
            log("   ⚠ 记忆系统: 已禁用（基线 A：zero_memory）");
        }
    }

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

    // v0.5.5 P1-1：设置 MemoryStore 的 LLM 配置状态
    // LLM 配置后替代本地 ML 模型提供语义理解能力，编码器不再视为"降级"
    {
        let store_guard = memory_store.lock().await;
        store_guard.set_llm_configured(llm_api_configured);
        if llm_api_configured {
            log("  编码器模式: LLM 增强模式（语义理解由 LLM 提供）");
        } else if ml_loaded {
            log("  编码器模式: ML 语义模式（本地模型已加载）");
        } else {
            log("  编码器模式: 基础模式（建议配置 LLM 或下载 ML 模型增强语义理解）");
        }
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
        // v0.8.22 P0-1 修复：LLM 配置状态无锁缓存，避免 /health 阻塞
        llm_configured_atomic: Arc::new(std::sync::atomic::AtomicBool::new(
            llm_api.is_configured(),
        )),
        // 后台索引状态跟踪（false = 索引中，true = 已完成）
        indexing_complete: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        // 服务启动时间（用于 /health 端点计算 uptime）
        started_at: chrono::Utc::now(),
        // v0.9.0: 开发模式标志
        dev_mode,
    });

    // 启动时为当前项目创建/更新 meta.json（用于显示项目名而非指纹）
    // 失败时不阻塞服务启动，仅记录警告
    {
        let src_path = std::path::Path::new(&src_dir);
        let data_dir = code_memory::data_dir::DataDir::for_project(src_path);
        if let Err(e) = data_dir.ensure_meta(src_path) {
            eprintln!("[warn] 启动时写入项目元信息失败: {}", e);
        }
    }

    // ════════════════════════════════════════════════════════════
    // 后台索引（所有模式都需要，不阻塞服务启动）
    // ════════════════════════════════════════════════════════════
    let index_state = state.clone();
    let index_log_bg = Arc::clone(&index_log);
    let indexing_flag = Arc::clone(&state.indexing_complete);
    tokio::spawn(async move {
        index_log_bg("[后台] 开始索引项目代码...");
        let mut bg_mgr = CodeMemoryManager::new();
        // v0.8.22 P0-2 修复（hcse-resilience-validator Round3）：
        //   根因：index_project 是 CPU 密集型操作（遍历文件 + 编码），
        //         原实现直接在 tokio worker 线程上执行，当项目较大时会
        //         长时间占用 worker 线程，导致 HTTP 服务器无法响应其他请求
        //   修复：将 index_project 移入 spawn_blocking，在独立阻塞线程执行，
        //         执行完毕后将 bg_mgr 返回到 async 上下文继续使用
        let index_src_for_blocking = index_src.clone();
        let index_result = tokio::task::spawn_blocking(move || {
            let result = bg_mgr.index_project(&index_src_for_blocking);
            (bg_mgr, result)
        })
        .await;
        match index_result {
            Ok((mgr, Ok(_count))) => {
                let stats = mgr.get_stats();
                index_log_bg(&format!(
                    "索引完成: {} 个文件 → {} 个代码片段",
                    stats.file_count, stats.total_chunks
                ));
                let mut state_mgr = index_state.manager.lock().await;
                *state_mgr = Box::new(mgr);
                index_log_bg("索引已生效，搜索服务已就绪");
            }
            Ok((_, Err(e))) => {
                index_log_bg(&format!("索引失败: {e}（部分文件可能无法检索）"));
            }
            Err(e) => {
                index_log_bg(&format!("索引任务执行失败: {e}（spawn_blocking 异常）"));
            }
        }
        // 无论成功或失败，标记索引阶段已完成
        indexing_flag.store(true, std::sync::atomic::Ordering::Release);
        index_log_bg("[后台] 索引阶段结束");
    });

    // ════════════════════════════════════════════════════════════
    // v0.5.4 P2-10 修复：启动后台结晶流水线
    // 定期执行记忆合成，自动合并用户通过 API 写入的重复/相似记忆
    // 默认间隔 5 分钟，启动时立即执行一次
    // ════════════════════════════════════════════════════════════
    let (consolidation_shutdown_tx, consolidation_shutdown_rx) = tokio::sync::watch::channel(false);

    {
        let consolidation_config = ConsolidationConfig {
            poll_interval_secs: 300, // 5 分钟轮询
            batch_size: 100,
            synthesis_threshold: 3,
            synthesis_similarity: 0.4,
            run_on_start: false, // v0.5.5 内存优化：延迟首次合成，避免启动内存峰值
            auto_synthesize: true,
            verbose: 1,
        };
        // v0.5.18：传入 LLM 配置，启用高维 embedding 合成
        // LLM 未配置时自动降级到本地统计合成
        let pipeline = ConsolidationPipeline::new_with_llm(
            consolidation_config,
            memory_store.clone(),
            llm_api.clone(),
        );
        // 使用空数据源：用户通过 HTTP API / MCP 工具直接写入的记忆已在 store 中
        // 结晶流水线仅负责定期执行合成（合并重复记忆）
        let source: Arc<dyn SurfaceMemorySource> = Arc::new(InMemorySource::new("api", vec![]));
        tokio::spawn(async move {
            run_consolidation_loop(pipeline, source, consolidation_shutdown_rx).await;
        });
        if llm_api.is_configured() {
            log("[LRC·结晶] 后台结晶流水线已启动（间隔 5 分钟，LLM embedding 合成模式）");
        } else {
            log("[LRC·结晶] 后台结晶流水线已启动（间隔 5 分钟，本地统计合成模式，建议配置 LLM）");
        }
    }

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
            "   仪表盘地址: http://localhost:{actual_port}/dashboard"
        ));

        // 自动打开浏览器（延迟 500ms 确保服务就绪）
        // 仅在用户显式指定 --dashboard 时打开浏览器
        // daemon 模式下由桌面端管理 UI，绝不打开浏览器
        let dashboard_url = format!("http://localhost:{actual_port}/dashboard");
        if !daemon_mode && dashboard_mode {
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
        }

        // ── 保存配置到全局配置文件 ──
        // daemon 模式下不保存独立配置，由桌面端统一管理
        // 所有配置通过 CLI 参数传递，避免配置分裂
        if !daemon_mode {
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
        }

        // ── 系统托盘（Windows 原生）──
        // 【已废弃】daemon 模式下由桌面端 Tauri 壳层统一管理托盘
        // sidecar 仅作为无头 HTTP 服务，不再创建任何 UI 组件
        // 保留 --tray 参数仅用于独立命令行模式（非桌面端场景）
        if tray_mode && !daemon_mode {
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
        if tray_mode && daemon_mode {
            eprintln!("[托盘] daemon 模式下托盘由桌面端管理，sidecar 跳过托盘创建");
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

    // v0.5.4 P2-10 修复：通知后台结晶流水线停止
    let _ = consolidation_shutdown_tx.send(true);

    Ok(())
}

/// v0.5.4 P2-22 修复：从 wizard.json（桌面端向导配置）加载 LLM 配置
///
/// daemon 模式下，如果环境变量和 config.json 都未提供 LLM 配置，
/// 作为最终后备从 wizard.json 加载，确保 sidecar 状态与桌面端一致。
///
/// wizard.json 结构：
///   - llm_configured: bool — 是否已配置 LLM
///   - llm_type: String — "openai" 或 "ollama"
///   - llm_model: Option<String> — 模型名称
///   - llm_base_url: Option<String> — API 基础 URL
///   - encrypted_api_key: String — 加密的纯 API Key（非完整 LLM API 字符串）
///
/// 返回完整的 LLM API 字符串：
///   - openai: "openai:{api_key}:{model}:{base_url}"
///   - ollama: "ollama:{model}:{host}"
fn load_llm_from_wizard_json() -> Option<String> {
    // v0.9.0 开发模式隔离：使用独立的 wizard.json 路径
    let appdata = std::env::var("APPDATA").ok()?;
    let loong_dir = std::path::PathBuf::from(appdata).join("LoongRecall");
    let is_dev = std::env::var("LRC_DEV_MODE").is_ok();
    let wizard_path = if is_dev {
        loong_dir.join("dev").join("wizard.json")
    } else {
        loong_dir.join("wizard.json")
    };

    if !wizard_path.exists() {
        return None;
    }

    let content = match std::fs::read_to_string(&wizard_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[配置] 读取 wizard.json 失败: {}", e);
            return None;
        }
    };

    let wizard: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[配置] 解析 wizard.json 失败: {}", e);
            return None;
        }
    };

    // 检查 llm_configured 标志
    if !wizard
        .get("llm_configured")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return None;
    }

    let llm_type = wizard
        .get("llm_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if llm_type.is_empty() || llm_type == "none" {
        return None;
    }

    // 解密 encrypted_api_key 得到纯 API Key
    let encrypted_key = wizard
        .get("encrypted_api_key")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let api_key = if !encrypted_key.is_empty() {
        match code_memory::crypto::decrypt_api_key(encrypted_key) {
            Ok(k) => k,
            Err(e) => {
                eprintln!("[配置] 解密 wizard.json 的 API Key 失败: {}", e);
                return None;
            }
        }
    } else {
        String::new()
    };

    // 构造完整的 LLM API 字符串
    let llm_api_str = match llm_type {
        "openai" => {
            let model = wizard
                .get("llm_model")
                .and_then(|v| v.as_str())
                .unwrap_or("gpt-4o-mini");
            let base_url = wizard
                .get("llm_base_url")
                .and_then(|v| v.as_str())
                .unwrap_or("https://api.openai.com/v1");
            format!("openai:{}:{}:{}", api_key, model, base_url)
        }
        "ollama" => {
            let model = wizard
                .get("llm_model")
                .and_then(|v| v.as_str())
                .unwrap_or("llama3");
            let host = wizard
                .get("llm_base_url")
                .and_then(|v| v.as_str())
                .unwrap_or("http://localhost:11434");
            format!("ollama:{}:{}", model, host)
        }
        _ => return None,
    };

    Some(llm_api_str)
}

/// v0.9.0 新增：检查本地 ML 语义模型是否已就绪（不联网）
///
/// 复用 engine 层 model_resolver 的统一检测逻辑（models/ 目录 + 可执行文件同级 + ~/.loong-recall/models/ + HF 缓存），
/// 存在则返回 true，由调用方决定加载 ML 编码器；
/// 不存在则返回 false，直接使用统计编码器（避免联网下载导致启动延迟）。
#[cfg(feature = "ml")]
fn local_ml_model_ready() -> bool {
    let model_id = code_memory::engine::model_resolver::selected_model_id();
    code_memory::engine::model_resolver::check_model_ready(&model_id)
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
    ("codebuddy", "IDE", true),
    // Qoder：用户级配置 ~/.qoder/settings.json（官方文档确认支持 mcpServers 结构）
    ("qoder", "IDE", true),
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
        // CodeBuddy（腾讯）：用户级配置文件
        // 官方文档优先级（高→低）：~/.codebuddy/.mcp.json（推荐）→ ~/.codebuddy/mcp.json（已废弃）→ ~/.codebuddy.json
        // 写入规则：若存在任意一个则写入第一个存在的文件，否则创建最高优先级的 .mcp.json
        "codebuddy" => {
            let mcp_dot = home.join(".codebuddy").join(".mcp.json");
            let mcp_legacy = home.join(".codebuddy").join("mcp.json");
            if mcp_dot.exists() {
                mcp_dot
            } else if mcp_legacy.exists() {
                mcp_legacy
            } else {
                mcp_dot
            }
        }
        // Qoder：用户级配置 ~/.qoder/settings.json（保留原有 enabledPlugins 等字段）
        "qoder" => home.join(".qoder").join("settings.json"),
        "kiro" => home.join(".kiro").join("settings").join("mcp.json"),
        "gemini" | "gemini-cli" => home.join(".gemini").join("settings.json"),
        "comate" => home.join(".comate").join("mcp.json"),
        "roo" | "roo-code" => home.join(".roo").join("mcp.json"),
        "cline" => home.join(".cline").join("mcp.json"),
        "cloudbase" | "cloudbase-mcp" => home.join(".cloudbase-mcp").join("mcp.json"),
        _ => {
            eprintln!("错误: 不支持的 IDE/工具: {ide}");
            eprintln!(
                "支持的 IDE: trae, trae-cn, cursor, vscode, windsurf, codebuddy, qoder, kiro"
            );
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
            println!("    code-memory-server --port {}", DEFAULT_PORT);
            println!("    然后访问 http://localhost:{}/dashboard", DEFAULT_PORT);
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

    // 构建 loong-recall 的 MCP 配置（默认支持 3 窗口）
    let mcp_entry = serde_json::json!({
        "command": exe_path,
        "args": ["--src-dir", ".", "--stdio", "--multi-window", "3"],
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

// ════════════════════════════════════════════════════════════
// v0.6.0 模型管理子命令实现
// ════════════════════════════════════════════════════════════

/// 推荐模型列表（用于 model list 输出参考）
const RECOMMENDED_MODELS: &[(&str, &str, &str)] = &[
    ("BAAI/bge-small-zh", "512", "中文默认（~100MB）"),
    ("BAAI/bge-base-zh", "768", "中文高精度（~400MB）"),
    (
        "sentence-transformers/all-MiniLM-L6-v2",
        "384",
        "英文/多语言轻量（~80MB）",
    ),
    ("multilingual-e5-small", "384", "多语言通用（~120MB）"),
    (
        "microsoft/graphcodebert-base",
        "768",
        "代码搜索（~500MB，向后兼容）",
    ),
];

/// 获取 models/ 目录路径
///
/// 优先使用当前工作目录下的 models/，其次使用可执行文件同级目录的 models/
fn get_models_dir() -> PathBuf {
    // v0.9.0 修复：统一使用标准模型目录 ~/.loong-recall/models/
    // 模型下载、列表、使用都基于此目录，不依赖 cwd（避免 sidecar 运行时 cwd 不一致）
    code_memory::engine::model_resolver::default_models_dir()
}

/// 处理 `model list` 子命令
///
/// 扫描 models/ 目录，列出已下载的模型及其大小、维度信息。
fn handle_model_list() {
    let models_dir = get_models_dir();

    println!("═══════════════════════════════════════════");
    println!("  LRC 已下载模型列表");
    println!("═══════════════════════════════════════════");
    println!();
    println!("  模型目录: {}", models_dir.display());
    println!();

    if !models_dir.exists() {
        println!("  （暂无已下载模型）");
        println!();
        println!("  推荐模型:");
        for (id, dim, desc) in RECOMMENDED_MODELS {
            println!("    {:<45} {:>4}维  {}", id, dim, desc);
        }
        println!();
        println!("  下载模型: code-memory-server model download <model_id>");
        return;
    }

    // 扫描 models/ 目录下的子目录
    let entries = match std::fs::read_dir(&models_dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("错误: 无法读取 models 目录: {}", e);
            return;
        }
    };

    let mut found_any = false;
    let mut models: Vec<(String, u64, String)> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dir_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        // 检查是否包含必需文件（config.json + 模型权重）
        let has_config = path.join("config.json").exists();
        let has_weights =
            path.join("model.safetensors").exists() || path.join("pytorch_model.bin").exists();

        if !has_config || !has_weights {
            continue;
        }

        // 计算目录总大小
        let total_size = calculate_dir_size(&path);
        let model_id = dir_name.replace("--", "/");
        models.push((model_id, total_size, dir_name));
        found_any = true;
    }

    if !found_any {
        println!("  （暂无完整模型，仅下载了部分文件）");
        println!();
        println!("  推荐模型:");
        for (id, dim, desc) in RECOMMENDED_MODELS {
            println!("    {:<45} {:>4}维  {}", id, dim, desc);
        }
        println!();
        println!("  下载模型: code-memory-server model download <model_id>");
        return;
    }

    // 按模型 ID 排序
    models.sort_by(|a, b| a.0.cmp(&b.0));

    println!("  {:<45} {:>10}  目录名", "模型 ID", "大小");
    println!("  {}", "─".repeat(75));

    // 获取当前默认模型（从环境变量，常量定义在 engine 层避免公开层泄露）
    let current_default = std::env::var(code_memory::engine::embedder::EMBEDDER_MODEL_ENV_VAR).ok();

    for (model_id, size, dir_name) in &models {
        let size_str = format_size(*size);
        let marker = if Some(model_id.as_str()) == current_default.as_deref() {
            " ← 当前默认"
        } else {
            ""
        };
        println!(
            "  {:<45} {:>10}  {}{}",
            model_id, size_str, dir_name, marker
        );
    }

    println!();
    println!("  共 {} 个已下载模型", models.len());
    println!();
    println!("  推荐模型（尚未下载）:");
    let downloaded_ids: Vec<&str> = models.iter().map(|m| m.0.as_str()).collect();
    for (id, dim, desc) in RECOMMENDED_MODELS {
        if !downloaded_ids.contains(id) {
            println!("    {:<45} {:>4}维  {}", id, dim, desc);
        }
    }
    println!();
    println!("  切换默认模型: code-memory-server model use <model_id>");
    println!("  删除模型:     code-memory-server model remove <model_id>");
}

/// 处理 `model download <model_id>` 子命令
///
/// 使用 ModelDownloader 下载模型文件到 models/ 目录。
/// 需要启用 ml feature（默认未启用，需用 `cargo build --features server,ml` 编译）。
fn handle_model_download(model_id: &str) -> Result<(), String> {
    #[cfg(not(feature = "ml"))]
    {
        Err(format!(
            "错误: 模型下载功能需要启用 ml feature\n\
             当前编译未启用 ml feature，请使用以下命令重新编译：\n\
             cargo build --features server,ml\n\
             \n\
             或手动下载模型 {}：\n\
             1. 访问 https://hf-mirror.com/{}\n\
             2. 下载 config.json, tokenizer.json, model.safetensors\n\
             3. 放到 models/{}/ 目录下\n\
             详细步骤参考: docs/OFFLINE_MODEL_GUIDE.md",
            model_id,
            model_id,
            model_id.replace('/', "--")
        ))
    }

    #[cfg(feature = "ml")]
    {
        use code_memory::engine::model_downloader::{
            build_download_url, manual_download_guide, ConsoleProgress, DownloadConfig,
            MirrorSource, ModelDownloader,
        };

        println!("═══════════════════════════════════════════");
        println!("  LRC 模型下载");
        println!("═══════════════════════════════════════════");
        println!();
        println!("  模型 ID: {}", model_id);
        println!("  镜像源: {}", MirrorSource::from_env());
        println!();

        // 检查模型是否已下载
        let local_dir_name = model_id.replace('/', "--");
        let models_dir = get_models_dir();
        let dest_dir = models_dir.join(&local_dir_name);

        if dest_dir.join("config.json").exists()
            && (dest_dir.join("model.safetensors").exists()
                || dest_dir.join("pytorch_model.bin").exists())
        {
            println!("  ✓ 模型已存在: {}", dest_dir.display());
            println!(
                "  如需重新下载，请先删除: code-memory-server model remove {}",
                model_id
            );
            return Ok(());
        }

        // 需要下载的文件列表
        let files_to_download = ["config.json", "tokenizer.json", "model.safetensors"];

        let config = DownloadConfig::default();
        let downloader = ModelDownloader::new(config);
        let progress = ConsoleProgress::new();
        let mirror = MirrorSource::from_env();

        println!("  开始下载 {} 个文件...", files_to_download.len());
        println!();

        let mut failed_files = Vec::new();

        for (idx, filename) in files_to_download.iter().enumerate() {
            println!(
                "[{}/{}] 下载 {} ...",
                idx + 1,
                files_to_download.len(),
                filename
            );

            let url = build_download_url(model_id, filename, mirror);
            let dest = dest_dir.join(filename);

            match downloader.download_with_retry(&url, &dest, &progress) {
                Ok(()) => {
                    println!("  ✓ {} 下载完成", filename);
                }
                Err(e) => {
                    eprintln!("  ✗ {} 下载失败: {}", filename, e);
                    failed_files.push(*filename);
                    // model.safetensors 下载失败时尝试 pytorch_model.bin
                    if *filename == "model.safetensors" {
                        eprintln!("  → 尝试下载 pytorch_model.bin 作为替代...");
                        let alt_url = build_download_url(model_id, "pytorch_model.bin", mirror);
                        let alt_dest = dest_dir.join("pytorch_model.bin");
                        match downloader.download_with_retry(&alt_url, &alt_dest, &progress) {
                            Ok(()) => {
                                println!("  ✓ pytorch_model.bin 下载完成（替代 safetensors）");
                            }
                            Err(e2) => {
                                eprintln!("  ✗ pytorch_model.bin 下载也失败: {}", e2);
                                failed_files.push("pytorch_model.bin");
                            }
                        }
                    }
                }
            }
        }

        println!();
        if failed_files.is_empty() {
            println!("═══════════════════════════════════════════");
            println!("  ✓ 模型 {} 下载完成！", model_id);
            println!("═══════════════════════════════════════════");
            println!();
            println!("  存储位置: {}", dest_dir.display());
            println!();
            println!("  下一步:");
            println!(
                "    设为默认模型: code-memory-server model use {}",
                model_id
            );
            println!("    启动 LRC:     code-memory-server --src-dir ./src --stdio");
        } else {
            println!("═══════════════════════════════════════════");
            println!("  ⚠ 部分文件下载失败: {:?}", failed_files);
            println!("═══════════════════════════════════════════");
            println!();
            println!("{}", manual_download_guide(model_id));
            return Err(format!("下载失败，{} 个文件未成功下载", failed_files.len()));
        }

        Ok(())
    }
}

/// 处理 `model use <model_id>` 子命令
///
/// 设置默认嵌入模型。当前通过环境变量配置（变量名见 `EMBEDDER_MODEL_ENV_VAR` 常量）。
/// 未来版本将支持持久化到配置文件。
fn handle_model_use(model_id: &str) -> Result<(), String> {
    // 从 engine 层获取环境变量名（避免公开层直接出现受保护术语）
    let env_var = code_memory::engine::embedder::EMBEDDER_MODEL_ENV_VAR;
    println!("═══════════════════════════════════════════");
    println!("  LRC 设置默认模型");
    println!("═══════════════════════════════════════════");
    println!();
    println!("  目标模型: {}", model_id);

    // 检查模型是否已下载
    let local_dir_name = model_id.replace('/', "--");
    let models_dir = get_models_dir();
    let model_dir = models_dir.join(&local_dir_name);

    if !model_dir.exists() {
        println!();
        println!("  ⚠ 该模型尚未下载");
        println!("  请先下载: code-memory-server model download {}", model_id);
        return Err(format!("模型 {} 未下载", model_id));
    }

    if !model_dir.join("config.json").exists() {
        println!();
        println!("  ⚠ 模型目录不完整（缺少 config.json）");
        return Err(format!("模型 {} 文件不完整", model_id));
    }

    println!();
    println!("  ✓ 模型已就绪: {}", model_dir.display());
    println!();
    println!("  ─────────────────────────────────────────");
    println!("  设置方法（任选其一）：");
    println!("  ─────────────────────────────────────────");
    println!();
    println!("  方法 1: 环境变量（推荐，立即生效）");
    println!();
    println!("    PowerShell（当前会话）:");
    println!("      $env:{} = \"{}\"", env_var, model_id);
    println!();
    println!("    PowerShell（永久，当前用户）:");
    println!("      [Environment]::SetEnvironmentVariable(");
    println!("          \"{}\", \"{}\", \"User\")", env_var, model_id);
    println!();
    println!("    Bash/Zsh（Linux/macOS）:");
    println!("      export {}={}", env_var, model_id);
    println!("      # 永久生效请添加到 ~/.bashrc 或 ~/.zshrc");
    println!();
    println!("  方法 2: 启动参数");
    println!();
    println!("    code-memory-server --src-dir ./src --stdio");
    println!("    （LRC 启动时会自动检测系统语言选择默认模型）");
    println!();
    println!("  ─────────────────────────────────────────");
    println!("  设置后请重启 LRC 服务使配置生效。");
    println!("  ─────────────────────────────────────────");

    Ok(())
}

/// 处理 `model remove <model_id>` 子命令
///
/// 删除指定模型的本地文件。需要用户确认。
fn handle_model_remove(model_id: &str) -> Result<(), String> {
    println!("═══════════════════════════════════════════");
    println!("  LRC 删除模型");
    println!("═══════════════════════════════════════════");
    println!();
    println!("  目标模型: {}", model_id);

    let local_dir_name = model_id.replace('/', "--");
    let models_dir = get_models_dir();
    let model_dir = models_dir.join(&local_dir_name);

    if !model_dir.exists() {
        println!();
        println!("  ⚠ 模型目录不存在: {}", model_dir.display());
        return Err(format!("模型 {} 未下载，无需删除", model_id));
    }

    // 计算目录大小
    let dir_size = calculate_dir_size(&model_dir);
    let size_str = format_size(dir_size);

    println!("  目录路径: {}", model_dir.display());
    println!("  目录大小: {}", size_str);
    println!();
    println!("  ⚠ 警告: 此操作将永久删除模型文件，不可恢复！");
    println!();

    // 询问用户确认
    if !ask_user_confirmation(&format!("  确认删除模型 {} ({})？", model_id, size_str)) {
        println!("  → 已取消删除");
        return Ok(());
    }

    // 执行删除
    match std::fs::remove_dir_all(&model_dir) {
        Ok(()) => {
            println!();
            println!("  ✓ 模型已删除: {}", model_id);
            println!("  已释放空间: {}", size_str);
            Ok(())
        }
        Err(e) => Err(format!("删除失败: {}", e)),
    }
}

/// 计算目录总大小（递归）
fn calculate_dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if entry_path.is_dir() {
                total += calculate_dir_size(&entry_path);
            } else if entry_path.is_file() {
                if let Ok(metadata) = entry.metadata() {
                    total += metadata.len();
                }
            }
        }
    }
    total
}

/// 格式化文件大小为人类可读字符串
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
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
    println!(
        "  --port <端口>       HTTP 绑定端口 [默认: {}]",
        DEFAULT_PORT
    );
    println!("  --stdio             使用 stdio 传输模式（IDE 标准 MCP，推荐）");
    println!("  --global            记忆跨项目共享 (~/.loong-recall/global/data/)");
    println!("  --version, -V       显示版本号并退出");
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
    println!("                       IDE: trae, trae-cn, cursor, vscode, windsurf, codebuddy, qoder, kiro");
    println!("                       CLI/桌面: gemini, comate, roo, cline, cloudbase");
    println!("                       多工具用逗号分隔: --install-ide trae,cursor");
    println!("  --benchmark         运行三层基准测试（人类可读输出）");
    println!("  --benchmark-json    运行三层基准测试（JSON 格式，供 CI/CD 或仪表盘使用）");
    println!("  --export <路径>     导出记忆数据到 JSON 文件（备份）");
    println!("  --import <路径>     从 JSON 文件导入记忆数据（恢复）");
    println!("  --help, -h          显示此帮助信息");
    println!();
    println!("模型管理子命令（v0.6.0 新增）：");
    println!("  model list                       列出已下载的模型");
    println!("  model download <model_id>        下载模型（需 ml feature）");
    println!("  model use <model_id>             设置默认模型（输出环境变量配置指引）");
    println!("  model remove <model_id>          删除模型文件（需确认）");
    println!();
    println!("  示例:");
    println!("    code-memory-server model list");
    println!("    code-memory-server model download BAAI/bge-small-zh");
    println!("    code-memory-server model use BAAI/bge-small-zh");
    println!("    code-memory-server model remove BAAI/bge-small-zh");
    println!();
    println!("举个栗子:");
    println!("  # Tier 1 — 默认快速模式（零网络、零下载、秒启动）");
    println!(
        "  code-memory-server --src-dir ./src --port {}",
        DEFAULT_PORT
    );
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
    println!(
        "访问 http://localhost:{}/dashboard 查看可视化记忆管理仪表盘。",
        DEFAULT_PORT
    );
    println!("详细使用说明: https://github.com/zhibaiYingChuan/LRC/blob/main/docs/USER_GUIDE.md");
}
