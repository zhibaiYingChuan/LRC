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

use code_memory::{server, CodeMemoryManager};
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut src_dir = String::new();
    let mut host = String::from("127.0.0.1");
    let mut port: u16 = 3099;
    let mut stdio_mode = false;

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
            "--help" | "-h" => {
                print_help();
                return;
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

    // 根据 feature 选择编码器类型
    #[cfg(feature = "ml")]
    let manager: Box<dyn server::IndexedCodebase> = {
        log("   使用 CodeBERT 语义编码器（candle 本地推理）");
        let encoder = code_memory::CodeBertEncoder::load()
            .expect("加载 CodeBERT 模型失败");
        Box::new(CodeMemoryManager::with_encoder(Arc::new(encoder)))
    };

    #[cfg(not(feature = "ml"))]
    let manager: Box<dyn server::IndexedCodebase> = {
        log("   使用 Mock 词袋编码器（快速但精度较低）");
        Box::new(CodeMemoryManager::new())
    };

    let state = Arc::new(server::AppState {
        manager: Arc::new(Mutex::new(manager)),
        src_dir: src_dir.clone(),
    });

    // 后台异步索引项目代码（不阻塞 MCP 握手）
    // IDE 启动进程后立即发送 initialize 请求，必须尽快响应
    let index_state = state.clone();
    let index_dir = src_dir.clone();
    tokio::spawn(async move {
        log(&format!("\n正在后台索引项目代码: {}", index_dir));

        #[cfg(feature = "ml")]
        let mut mgr = {
            let encoder = code_memory::CodeBertEncoder::load()
                .expect("加载 CodeBERT 模型失败");
            CodeMemoryManager::with_encoder(Arc::new(encoder))
        };
        #[cfg(not(feature = "ml"))]
        let mut mgr = CodeMemoryManager::new();

        index_and_report(&mut mgr, &index_dir, &log);

        // 索引完成后原子替换 manager
        let mut locked = index_state.manager.lock().await;
        *locked = Box::new(mgr);
    });

    // 启动 MCP 服务（立即开始监听，不等待索引完成）
    if stdio_mode {
        log("\nMCP Stdio 模式启动（通过 stdin/stdout 通信）");
        server::run_stdio(state).await;
    } else {
        // HTTP 模式也先启动服务，后台索引
        log("\nMCP HTTP 模式启动中...");
        if let Err(e) = server::serve(state, &host, port).await {
            eprintln!("服务启动失败: {}", e);
            std::process::exit(1);
        }
    }
}

/// 索引项目并输出统计信息
fn index_and_report<E: code_memory::engine::encoder::CodeEncoder>(
    mgr: &mut CodeMemoryManager<E>,
    src_dir: &str,
    log: &impl Fn(&str),
) {
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
        }
        Err(e) => {
            eprintln!("   索引失败: {}", e);
            eprintln!("   请检查 --src-dir 路径是否正确");
            std::process::exit(1);
        }
    }
}

fn print_help() {
    println!("Loong Recall (L-RC / 忆) — 代码语义记忆 MCP 服务");
    println!();
    println!("用法: code-memory-server [选项]");
    println!();
    println!("选项:");
    println!("  --src-dir <路径>    要索引的源码目录 [默认: 当前目录]");
    println!("  --host <地址>       HTTP 绑定地址 [默认: 127.0.0.1]");
    println!("  --port <端口>       HTTP 绑定端口 [默认: 3099]");
    println!("  --stdio             使用 stdio 传输模式（IDE 标准 MCP）");
    println!("  --help, -h          显示此帮助信息");
    println!();
    println!("HTTP 模式示例:");
    println!("  code-memory-server --src-dir ./src --port 3099");
    println!();
    println!("Stdio 模式示例（推荐 IDE 全局部署）:");
    println!("  code-memory-server --src-dir ./src --stdio");
    println!();
    println!("启动后在 IDE 中配置 MCP 连接即可使用。");
}