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

use code_memory::{
    server, CodeMemoryManager, JsonPersistence, LlmApiConfig, MemoryStore,
};
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::main]
#[allow(unused_assignments)]
async fn main() {
    // 运行时防护：反调试 + 完整性校验（必须在任何业务逻辑之前执行）
    code_memory::guard::risk_aware_guard();

    let args: Vec<String> = std::env::args().collect();

    let mut src_dir = String::new();
    let mut host = String::from("127.0.0.1");
    let mut port: u16 = 3099;
    let mut stdio_mode = false;
    let mut global_mode = false;
    let mut db_path: Option<String> = None;
    let mut llm_api_raw: Option<String> = None;
    let mut proxy: Option<String> = None;
    #[allow(unused_variables, unused_assignments)]
    let mut mode = String::from("auto"); // "auto" | "fast" | "smart"

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
                    port = args[i].parse().unwrap_or(3099);
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
            "--llm-api" => {
                i += 1;
                if i < args.len() {
                    llm_api_raw = Some(args[i].clone());
                }
            }
            "--help" | "-h" => {
                print_help();
                return;
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
            _ => {
                eprintln!("未知参数: {}", args[i]);
                print_help();
                std::process::exit(1);
            }
        }
        i += 1;
    }

    // 在 stdio 模式下，状态信息输出到 stderr（stdout 留给 MCP 协议）
    let log = move |msg: &str| {
        if stdio_mode {
            eprintln!("{}", msg);
        } else {
            println!("{}", msg);
        }
    };

    log("═══════════════════════════════════════════");
    log(&format!(
        "  Loong Recall (L-RC / 忆) MCP Server v{}",
        env!("CARGO_PKG_VERSION")
    ));
    log("═══════════════════════════════════════════");

    // 配置网络代理（在模型下载等网络操作之前设置）
    if let Some(ref proxy_url) = proxy {
        std::env::set_var("HTTP_PROXY", proxy_url);
        std::env::set_var("HTTPS_PROXY", proxy_url);
        log(&format!("   代理: {} (已应用到 HTTP/HTTPS 请求)", proxy_url));
    }

    // 设置 HF 镜像端点（在模型下载等网络操作之前，确保使用国内镜像）
    if std::env::var("HF_ENDPOINT").is_err() {
        std::env::set_var("HF_ENDPOINT", "https://hf-mirror.com");
    }

    // P0-2: 启动前检查 ML 模型就绪状态（提前告知用户，不阻塞启动）
    #[cfg(feature = "ml")]
    {
        log(&format!("   ML 模型检查: {}", 
            if code_memory::engine::model_resolver::check_model_ready("microsoft/graphcodebert-base") {
                "已就绪，语义搜索立即可用"
            } else {
                "未下载，首次使用时会自动下载（约 1-3 分钟）"
            }
        ));
    }

    // 确定源码目录：默认使用当前工作目录
    let src_dir = if src_dir.is_empty() {
        let default = std::path::PathBuf::from(".");
        if default.exists() {
            default.canonicalize()
                .unwrap_or(default)
                .to_string_lossy()
                .to_string()
        } else {
            String::from(".")
        }
    } else {
        src_dir
    };

    // 确定记忆数据目录
    // 优先级: --db-path > --global > 默认路径
    let data_dir = if let Some(ref custom_path) = db_path {
        custom_path.clone()
    } else if global_mode {
        // 全局记忆目录: ~/.loong-recall/data/
        let home = dirs_next::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        home.join(".loong-recall").join("data")
            .to_string_lossy()
            .to_string()
    } else {
        // 默认：源码目录下的 .loong-recall/data/
        std::path::PathBuf::from(&src_dir)
            .join(".loong-recall")
            .join("data")
            .to_string_lossy()
            .to_string()
    };

    log(&format!("   记忆数据目录: {}", data_dir));

    // 前置验证：源码目录必须存在且为目录
    let src_path = std::path::Path::new(&src_dir);
    if !src_path.exists() {
        eprintln!("错误: 源码目录不存在: {}", src_dir);
        eprintln!("提示: 请使用 --src-dir 指定正确的项目路径");
        std::process::exit(1);
    }
    if !src_path.is_dir() {
        eprintln!("错误: 指定路径不是目录: {}", src_dir);
        std::process::exit(1);
    }

    // 创建持久化后端和记忆存储
    let persistence = JsonPersistence::new(&data_dir).unwrap_or_else(|e| {
        eprintln!("致命错误: 无法创建数据目录或初始化持久化后端");
        eprintln!("  路径: {}", data_dir);
        eprintln!("  原因: {}", e);
        eprintln!("  建议: 检查磁盘空间和目录写入权限");
        std::process::exit(1);
    });
    let memory_store = Arc::new(Mutex::new(MemoryStore::new(persistence)));

    // 解析 LLM API 配置
    let llm_api = match llm_api_raw {
        Some(ref raw) => match LlmApiConfig::parse(raw) {
            Ok(config) => {
                match &config {
                    LlmApiConfig::OpenAI { model, .. } => {
                        log(&format!("   LLM 增强: OpenAI ({}) → 查询翻译已启用", model));
                    }
                    LlmApiConfig::Ollama { host, model } => {
                        log(&format!("   LLM 增强: Ollama ({}@{}) → 查询翻译已启用", model, host));
                    }
                    _ => {}
                }
                config
            }
            Err(e) => {
                eprintln!("错误: LLM API 配置解析失败: {}", e);
                eprintln!("提示: 格式为 openai:sk-xxx:model 或 ollama:host:model");
                std::process::exit(1);
            }
        },
        None => LlmApiConfig::None,
    };

    // 根据 feature 和 --mode 选择编码器类型并索引项目代码
    // 镜像启动策略：默认模式（auto）下，Fast Match 立即可用，后台异步升级 Smart Match
    #[cfg(feature = "ml")]
    let manager: Box<dyn server::IndexedCodebase> = {
        if mode == "fast" {
            // 用户显式指定 Fast Match：跳过模型加载，零延迟启动
            log("   搜索模式: Fast Match（关键词匹配 · 零延迟 · 零依赖）");
            log("   提示: 使用 --mode smart 切换到语义搜索模式");
            let mut mgr = CodeMemoryManager::new();
            log(&format!("\n正在索引项目代码: {}...", src_dir));
            match index_and_report(&mut mgr, &src_dir, &log) {
                Ok(()) => {}
                Err(e) => {
                    log(&format!("   索引警告: {}（部分文件可能无法检索）", e));
                }
            }
            Box::new(mgr)
        } else if mode == "smart" {
            // 用户显式指定 Smart Match：直接加载模型，同步等待
            log("   搜索模式: Smart Match（语义理解 · 首次启动需下载模型）");
            log("   模型: microsoft/graphcodebert-base (hf-mirror.com)");
            let encoder = code_memory::CodeBertEncoder::load()
                .expect("加载模型失败，请检查网络连接或手动下载模型");
            let mut mgr = CodeMemoryManager::with_encoder(Arc::new(encoder));

            // 尝试加载缓存，跳过重复编码
            if let Some(n) = mgr.load_embedding_cache(&data_dir) {
                log(&format!("\n  ✓ 从缓存恢复索引: {} 个代码片段（秒级加载）", n));
            } else {
                log(&format!("\n正在索引项目代码: {}...", src_dir));
                log("   （首次索引较慢，后续启动会使用缓存）");
                match index_and_report(&mut mgr, &src_dir, &log) {
                    Ok(()) => {
                        // 首次索引完成后保存缓存
                        if let Err(e) = mgr.save_embedding_cache(&data_dir) {
                            log(&format!("   缓存保存失败: {}", e));
                        } else {
                            log("   嵌入向量已缓存（下次启动秒加载）");
                        }
                    }
                    Err(e) => {
                        log(&format!("   索引警告: {}（部分文件可能无法检索）", e));
                    }
                }
            }
            Box::new(mgr)
        } else {
            // 默认镜像启动：Fast Match 立即可用，后台异步升级 Smart Match
            log("   搜索模式: 镜像启动（Fast Match 立即可用，后台升级 Smart Match）");
            log("   提示: 启动后搜索立即就绪，语义搜索在后台自动准备");
            let mut mgr = CodeMemoryManager::new();
            log(&format!("\n正在索引项目代码: {}...", src_dir));
            match index_and_report(&mut mgr, &src_dir, &log) {
                Ok(()) => {}
                Err(e) => {
                    log(&format!("   索引警告: {}（部分文件可能无法检索）", e));
                }
            }
            Box::new(mgr)
        }
    };

    #[cfg(not(feature = "ml"))]
    let manager: Box<dyn server::IndexedCodebase> = {
        log("   搜索模式: Fast Match（关键词匹配 · 零延迟 · 零依赖）");
        log("   适合按函数名/变量名查代码，日常开发首选");
        let mut mgr = CodeMemoryManager::new();
        log(&format!("\n正在索引项目代码: {}...", src_dir));
        match index_and_report(&mut mgr, &src_dir, &log) {
            Ok(()) => {}
            Err(e) => {
                log(&format!("   索引警告: {}（部分文件可能无法检索）", e));
            }
        }
        Box::new(mgr)
    };

    let state = Arc::new(server::AppState {
        manager: Arc::new(Mutex::new(manager)),
        memory_store: memory_store.clone(),
        src_dir: src_dir.clone(),
        llm_api: llm_api.clone(),
    });

    // 镜像启动：后台异步加载 Smart Match 模型并升级索引
    #[cfg(feature = "ml")]
    if mode != "fast" && mode != "smart" {
        let upgrade_state = state.clone();
        let upgrade_src = src_dir.clone();
        let upgrade_data = data_dir.clone();
        let is_stdio = stdio_mode;
        tokio::spawn(async move {
            let bg_log = |msg: &str| {
                if is_stdio {
                    eprintln!("{}", msg);
                } else {
                    println!("{}", msg);
                }
            };
            bg_log("\n[后台] 正在加载 Smart Match 语义模型...");
            match code_memory::CodeBertEncoder::load() {
                Ok(encoder) => {
                    bg_log("[后台] 模型加载成功，开始语义编码...");
                    let mut smart_mgr = CodeMemoryManager::with_encoder(Arc::new(encoder));

                    // 优先尝试加载缓存
                    let cache_hit = smart_mgr.load_embedding_cache(&upgrade_data).is_some();
                    if cache_hit {
                        bg_log("[后台] ✓ 从缓存恢复语义索引（秒级加载）");
                    } else {
                        bg_log(&format!(
                            "[后台] 正在语义编码项目代码: {}...",
                            upgrade_src
                        ));
                        match smart_mgr.index_project(&upgrade_src) {
                            Ok(_) => {
                                // 保存缓存供下次启动使用
                                if let Err(e) = smart_mgr.save_embedding_cache(&upgrade_data) {
                                    bg_log(&format!("[后台] 缓存保存失败: {}", e));
                                } else {
                                    bg_log("[后台] 嵌入向量已缓存（下次启动秒加载）");
                                }
                            }
                            Err(e) => {
                                bg_log(&format!("[后台] 语义索引失败: {}（Fast Match 继续服务）", e));
                                return;
                            }
                        }
                    }

                    // 原子替换 manager
                    let mut locked = upgrade_state.manager.lock().await;
                    *locked = Box::new(smart_mgr);
                    bg_log("[后台] ✓ Smart Match 已就绪，搜索精度已提升");
                }
                Err(e) => {
                    bg_log(&format!(
                        "[后台] 模型加载失败: {}（Fast Match 继续服务，可用 --mode smart 重试）",
                        e
                    ));
                }
            }
        });
    }

    // 启动 MCP 服务（索引已完成，搜索立即可用）
    if stdio_mode {
        log("\nMCP Stdio 模式启动（通过 stdin/stdout 通信）");
        server::run_stdio(state).await;
    } else {
        log("\nMCP HTTP 模式启动中...");
        if let Err(e) = server::serve(state, &host, port).await {
            eprintln!("服务启动失败: {}", e);
            std::process::exit(1);
        }
    }
}

/// 索引项目并输出统计信息，失败时返回错误而非杀死进程
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
            let msg = format!("   索引失败: {} (请检查 --src-dir 路径是否正确)", e);
            log(&msg);
            Err(msg)
        }
    }
}

fn print_help() {
    println!("Loong Recall (L-RC / 忆) — AI 编程助手的记忆与检索插件");
    println!();
    println!("用法: code-memory-server [选项]");
    println!();
    println!("选项:");
    println!("  --src-dir <路径>    要索引的项目源码目录 [默认: 当前目录]");
    println!("  --host <地址>       HTTP 绑定地址 [默认: 127.0.0.1]");
    println!("  --port <端口>       HTTP 绑定端口 [默认: 3099]");
    println!("  --stdio             使用 stdio 传输模式（IDE 标准 MCP，推荐）");
    println!("  --global            记忆跨项目共享 (~/.loong-recall/data/)");
    println!("  --db-path <路径>    自定义记忆数据存储路径（优先级最高）");
    println!("  --llm-api <配置>    配置 LLM 查询翻译（可选，不配就用 Fast Match）");
    println!("  --proxy <代理地址>    HTTP/HTTPS 代理（如 http://127.0.0.1:7890）");
    println!("  --mode <模式>        搜索模式: auto(默认) | fast(秒启动) | smart(语义)");
    println!("  --help, -h          显示此帮助信息");
    println!();
    println!("举个栗子:");
    println!("  # 给当前项目的 AI 助手装上记忆插件（最常用）");
    println!("  code-memory-server --src-dir ./src --stdio");
    println!();
    println!("  # HTTP 模式调试（看日志、测 API）");
    println!("  code-memory-server --src-dir ./src --port 3099");
    println!();
    println!("  # 快速模式：跳过模型下载，秒启动");
    println!("  code-memory-server --src-dir ./src --port 3099 --mode fast");
    println!();
    println!("  # 全局记忆，跨项目共享偏好和知识");
    println!("  code-memory-server --global --stdio");
    println!();
    println!("  # 配置 LLM 查询翻译，用自然语言搜索代码");
    println!("  code-memory-server --src-dir ./src --stdio --llm-api openai:sk-xxx:gpt-4o-mini");
    println!();
    println!("  # 使用本地 Ollama 模型（零成本）");
    println!("  code-memory-server --src-dir ./src --stdio --llm-api ollama:localhost:llama3");
    println!();
    println!("  # 自定义记忆存储路径");
    println!("  code-memory-server --db-path D:/my-data --stdio");
    println!();
    println!("启动后在 IDE 中配置 MCP 连接，AI 助手即可使用。");
    println!("详细使用说明: https://github.com/zhibaiYingChuan/LRC/blob/main/docs/USER_GUIDE.md");
}