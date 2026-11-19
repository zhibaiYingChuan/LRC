//! LRC 命令行客户端（v0.11 新增）
//!
//! ## 为什么是独立二进制
//!
//! 桌面端与 MCP 之外，给「任何能执行 shell 的 AI 工具」一条最低门槛通道。
//! 宿主不需要写 MCP 配置、不需要理解端口，只要会跑命令即可。
//!
//! ## 进程模型（双层，硬约束）
//!
//! ```text
//! 宿主 / 终端 ──argv──▶ lrc（本进程，纯 HTTP 客户端）
//!                          │ HTTP 127.0.0.1:3099
//!                          ▼
//!                       lrc-sidecar（常驻，持有 bge 模型与存储）
//! ```
//!
//! **本进程绝不加载语义模型**：bge 冷加载是秒级开销，若每次调用都付一次，
//! CLI 就失去了「低门槛快速通道」的意义。因此：
//! - 连不上 daemon 时，按 `LRC_NO_SPAWN` 决定是否拉起 sidecar；
//! - 拉起失败**明确报错**（退出码 3），绝不把记忆写到第二个地方。
//!
//! ## 退出码协议
//!
//! | 码 | 含义 |
//! |---|---|
//! | 0 | 成功 |
//! | 1 | 一般错误（服务端 5xx / 响应不可解析） |
//! | 2 | 参数错误 |
//! | 3 | daemon 不可用 |
//! | 4 | 超时 |
//! | 5 | 无结果（服务正常，只是没命中） |

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// `--json` 输出的 schema 版本。字段只增不改，宿主可据此判断兼容性。
const SCHEMA_VERSION: u32 = 1;

/// 正式端口。开发联调请用 `--base http://127.0.0.1:3111` 或 `LRC_BASE`。
const DEFAULT_BASE: &str = "http://127.0.0.1:3099";

const EXIT_OK: u8 = 0;
const EXIT_ERROR: u8 = 1;
const EXIT_ARG: u8 = 2;
const EXIT_DAEMON: u8 = 3;
const EXIT_TIMEOUT: u8 = 4;
const EXIT_EMPTY: u8 = 5;

/// 无取值的开关型选项（全局统一，未知选项直接报错，避免拼写错误被静默忽略）。
const KNOWN_FLAGS: &[&str] = &["json", "full", "help", "no-spawn", "yes", "include-expired"];

/// 需要取值的选项。
const KNOWN_VALUES: &[&str] = &[
    "top-k",
    "project",
    "type",
    "importance",
    "tags",
    "event-id",
    "privacy",
    "limit",
    "base",
    "timeout",
    "query",
    "user-id",
    "session-id",
    "reason",
    "relation",
    "file",
    "max-items",
    "transcript",
    "ttl-days",
];

/// CLI 统一错误。携带退出码语义，便于 main 直接映射。
enum CliError {
    /// 参数非法
    Arg(String),
    /// daemon 不可达 / 端口被占 / 拉起失败
    Daemon(String),
    /// 请求超时
    Timeout(String),
    /// 服务端返回错误或响应不可解析
    Server(String),
}

impl CliError {
    fn code(&self) -> u8 {
        match self {
            CliError::Arg(_) => EXIT_ARG,
            CliError::Daemon(_) => EXIT_DAEMON,
            CliError::Timeout(_) => EXIT_TIMEOUT,
            CliError::Server(_) => EXIT_ERROR,
        }
    }

    fn message(&self) -> &str {
        match self {
            CliError::Arg(m) | CliError::Daemon(m) | CliError::Timeout(m) | CliError::Server(m) => {
                m
            }
        }
    }
}

fn main() -> std::process::ExitCode {
    // Windows 控制台默认代码页可能是 GBK，中文会乱码。
    // 管道场景（AI 工具调用）本身按 UTF-8 解析，这里额外把控制台代码页也切到 65001。
    force_utf8_console();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match run(&args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("lrc: {}", error.message());
            if matches!(error, CliError::Daemon(_)) {
                eprintln!("提示: 运行 `lrc doctor` 查看具体卡在哪一项。");
            }
            error.code()
        }
    };
    std::process::ExitCode::from(code)
}

/// 顶层分派。
fn run(args: &[String]) -> Result<u8, CliError> {
    let Some(command) = args.first().map(String::as_str) else {
        print_usage();
        return Ok(EXIT_ARG);
    };

    let rest = &args[1..];
    match command {
        "recall" => cmd_recall(rest),
        "remember" => cmd_remember(rest),
        "list" => cmd_list(rest),
        "stats" => cmd_stats(rest),
        "search" => cmd_search(rest),
        "doctor" => cmd_doctor(rest),
        "mcp" => cmd_mcp(rest),
        "daemon" => cmd_daemon(rest),
        "forget" => cmd_forget(rest),
        "correct" => cmd_correct(rest),
        "associations" => cmd_associations(rest),
        "distill" => cmd_distill(rest),
        "migrate" => cmd_migrate(rest),
        "backup" => cmd_backup(rest),
        "archive" => cmd_archive(rest),
        "unarchive" => cmd_unarchive(rest),
        "purge-archived" => cmd_purge_archived(rest),
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(EXIT_OK)
        }
        "version" | "--version" | "-V" => {
            println!("lrc {}", env!("CARGO_PKG_VERSION"));
            Ok(EXIT_OK)
        }
        other => Err(CliError::Arg(format!(
            "未知命令 `{other}`，可用命令见 `lrc help`"
        ))),
    }
}

fn print_usage() {
    println!(
        r#"LRC 命令行客户端 {version}

用法:
  lrc recall <查询文本> [选项]       三路融合检索记忆
  lrc remember <内容|->  [选项]      写入一条记忆（`-` 表示从 stdin 读取）
  lrc search <查询文本> [选项]       检索代码库
  lrc list [选项]                    列出记忆
  lrc stats [选项]                   记忆库统计
  lrc doctor [选项]                  自检（daemon / 版本 / 模型 / 数据目录）
  lrc mcp [选项]                     以 stdio MCP server 运行（桥接到常驻 daemon）
  lrc daemon <start|stop|status>     管理常驻 daemon
  lrc forget <memory_id> --yes       删除一条记忆（不可撤销，必须显式确认）
  lrc correct <memory_id> <内容|->   修正一条记忆（保留版本历史）
  lrc associations <memory_id>       查看一条记忆的结构化关联
  lrc distill [文本|--transcript P]  从对话提炼候选记忆（不写库）
  lrc migrate < - | --file P >       把提炼结果批量写入（幂等，自动分批）
  lrc backup                         为当前实例创建记忆快照
  lrc archive                        归档当前实例的过期记忆到冷存储
  lrc unarchive <memory_id> [--ttl-days N]
                                     从归档恢复一条记忆（从现在重新计时；
                                     --ttl-days 0 表示永不过期，上限 36500）
  lrc purge-archived <id> --yes      从归档彻底删除一条记忆（不可恢复）

通用选项:
  --base <URL>        覆盖 daemon 地址（默认 {base}）
  --timeout <毫秒>    覆盖单次请求超时
  --json              输出稳定 JSON（含 schema_version）
  --no-spawn          禁止自动拉起 sidecar

recall 选项:
  --top-k <N>         返回条数 1-100，默认 5
  --project <P>       项目过滤（缺省回落 LRC_PROJECT）
  --user-id <U>       用户级隔离
  --session-id <S>    会话标识
  --full              不截断内容（默认截断 300 字符）

remember 选项:
  --type <T>          fact|preference|decision|code_context|conversation|experience|synthesis
  --importance <N>    1-10，默认 5
  --project <P>
  --tags <a,b,c>
  --event-id <ID>
  --privacy <L>       session|user|global，默认 user

forget / correct 选项:
  --user-id <U>       用户级隔离（多租户域必填）
  --session-id <S>    会话标识
  --reason <R>        correct 的修正原因

list 选项:
  --limit <N>         返回条数，默认 20
  --project <P>       项目过滤
  --include-expired   包含已过期记忆（默认隐藏）

环境变量:
  LRC_BASE            daemon 地址
  LRC_TOKEN           本地鉴权 Bearer
  LRC_PROJECT         默认项目名
  LRC_SIDECAR         sidecar 可执行文件路径
  LRC_NO_SPAWN        非空时禁止自动拉起
  LRC_TIMEOUT_MS      默认请求超时

退出码: 0 成功 | 1 一般错误 | 2 参数错误 | 3 daemon 不可用 | 4 超时 | 5 无结果"#,
        version = env!("CARGO_PKG_VERSION"),
        base = DEFAULT_BASE
    );
}

// ════════════════════════════════════════════════════════════════
// 参数解析
// ════════════════════════════════════════════════════════════════

#[derive(Default)]
struct Parsed {
    positional: Vec<String>,
    flags: HashSet<String>,
    values: HashMap<String, String>,
}

impl Parsed {
    fn flag(&self, key: &str) -> bool {
        self.flags.contains(key)
    }

    fn value(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    fn value_usize(&self, key: &str, default: usize) -> Result<usize, CliError> {
        match self.value(key) {
            None => Ok(default),
            Some(raw) => raw
                .parse::<usize>()
                .map_err(|_| CliError::Arg(format!("选项 --{key} 需要非负整数，收到 `{raw}`"))),
        }
    }

    fn value_u8(&self, key: &str, default: u8) -> Result<u8, CliError> {
        match self.value(key) {
            None => Ok(default),
            Some(raw) => raw
                .parse::<u8>()
                .map_err(|_| CliError::Arg(format!("选项 --{key} 需要 0-255 整数，收到 `{raw}`"))),
        }
    }
}

/// 手写解析，不引入 clap：release 使用 `opt-level="z"` + LTO，体积敏感。
fn parse_args(args: &[String]) -> Result<Parsed, CliError> {
    let mut parsed = Parsed::default();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];

        if arg == "-h" || arg == "--help" {
            parsed.flags.insert("help".to_string());
            index += 1;
            continue;
        }

        if let Some(rest) = arg.strip_prefix("--") {
            let (key, inline) = match rest.split_once('=') {
                Some((key, value)) => (key, Some(value.to_string())),
                None => (rest, None),
            };

            if KNOWN_FLAGS.contains(&key) {
                parsed.flags.insert(key.to_string());
                index += 1;
                continue;
            }

            if KNOWN_VALUES.contains(&key) {
                let value = match inline {
                    Some(value) => value,
                    None => {
                        index += 1;
                        args.get(index)
                            .cloned()
                            .ok_or_else(|| CliError::Arg(format!("选项 --{key} 缺少取值")))?
                    }
                };
                parsed.values.insert(key.to_string(), value);
                index += 1;
                continue;
            }

            return Err(CliError::Arg(format!("未知选项 --{key}")));
        }

        // 单独的 `-` 是 stdin 标记，属于位置参数，必须保留。
        if arg.starts_with('-') && arg.len() > 1 {
            return Err(CliError::Arg(format!("未知选项 {arg}")));
        }

        parsed.positional.push(arg.clone());
        index += 1;
    }
    Ok(parsed)
}

// ════════════════════════════════════════════════════════════════
// HTTP 客户端
// ════════════════════════════════════════════════════════════════

#[derive(Clone)]
struct Client {
    base: String,
    token: Option<String>,
    timeout: Duration,
    /// base 是否由用户显式指定（`--base` 或 `LRC_BASE`）。
    ///
    /// 显式指定时**不扫描邻域端口**：用户说连哪里就连哪里，静默切端口会让人困惑。
    /// 未指定时才允许「发现已有实例」。
    explicit_base: bool,
}

impl Client {
    fn send(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, CliError> {
        let url = format!("{}{}", self.base.trim_end_matches('/'), path);
        let agent = ureq::AgentBuilder::new().timeout(self.timeout).build();

        let mut request = match method {
            "GET" => agent.get(&url),
            "POST" => agent.post(&url),
            other => {
                return Err(CliError::Arg(format!("不支持的 HTTP 方法: {other}")));
            }
        };

        if let Some(token) = &self.token {
            request = request.set("Authorization", &format!("Bearer {token}"));
        }

        let outcome = match body {
            Some(body) => request
                .set("Content-Type", "application/json")
                .send_string(&body.to_string()),
            None => request.call(),
        };

        match outcome {
            Ok(response) => {
                let text = response
                    .into_string()
                    .map_err(|error| CliError::Server(format!("读取响应失败: {error}")))?;
                if text.trim().is_empty() {
                    return Ok(Value::Null);
                }
                serde_json::from_str(&text).map_err(|error| {
                    CliError::Server(format!(
                        "响应不是合法 JSON: {error}；原文: {}",
                        truncate(&text, 200)
                    ))
                })
            }
            Err(ureq::Error::Status(code, response)) => {
                let text = response.into_string().unwrap_or_default();
                // 只有 401 才是鉴权失败；403 可能是业务拒绝（如 transcript_path_forbidden），
                // 统一加鉴权提示会把用户引向错误方向。
                let hint = if code == 401 {
                    "（鉴权失败：请确认 LRC_API_TOKEN 与 daemon 启动时一致）"
                } else {
                    ""
                };
                Err(CliError::Server(format!(
                    "HTTP {code}{hint}: {}",
                    truncate(&text, 300)
                )))
            }
            Err(ureq::Error::Transport(transport)) => {
                // ureq 2.x 的 `ErrorKind` 没有独立的 Timeout 变体，超时与连接失败
                // 都归到 Io。这里按错误文案区分——这是该版本唯一可用的判据，
                // 已在 ureq 2.12.1 上核实文案为 "timed out ..."。
                let detail = transport.to_string();
                let lower = detail.to_ascii_lowercase();
                if lower.contains("timed out") || lower.contains("timeout") {
                    Err(CliError::Timeout(format!(
                        "请求超时（{}ms）",
                        self.timeout.as_millis()
                    )))
                } else {
                    Err(CliError::Daemon(format!(
                        "无法连接 {}: {detail}",
                        self.base
                    )))
                }
            }
        }
    }
}

/// 依据命令行与环境变量构造客户端。
fn build_client(parsed: &Parsed, default_timeout: Duration) -> Result<Client, CliError> {
    // 显式 base 与默认 base 的区别决定「能否扫描邻域端口复用已有实例」。
    let (base, explicit_base) = match parsed.value("base").map(str::to_string) {
        Some(base) => (base, true),
        None => match std::env::var("LRC_BASE").ok().filter(|v| !v.is_empty()) {
            Some(base) => (base, true),
            None => (DEFAULT_BASE.to_string(), false),
        },
    };

    let timeout = match parsed.value("timeout") {
        Some(raw) => Duration::from_millis(
            raw.parse::<u64>()
                .map_err(|_| CliError::Arg(format!("选项 --timeout 需要毫秒整数，收到 `{raw}`")))?,
        ),
        None => std::env::var("LRC_TIMEOUT_MS")
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .map(Duration::from_millis)
            .unwrap_or(default_timeout),
    };

    // 服务端只认 `LRC_API_TOKEN`（见 server.rs 的 local_api_auth）；
    // `LRC_TOKEN` 保留为向后兼容别名。
    let token = std::env::var("LRC_API_TOKEN")
        .ok()
        .or_else(|| std::env::var("LRC_TOKEN").ok())
        .filter(|token| !token.is_empty());

    // 复用项目既有的 URL 安全校验：拒绝内嵌 userinfo、非法 scheme/端口等。
    // 用户显式指定的 base 会被原样使用，必须在入口处拦一次。
    // 错误文案**不回显原始 base**：用户若传 `http://user:pass@host`，
    // 回显会把密码写进 stderr 与日志。
    if explicit_base {
        if let Err(error) = code_memory::url_safety::validate_http_url(&base) {
            return Err(CliError::Arg(format!("--base 不合法: {error}")));
        }
    }

    Ok(Client {
        base,
        token,
        timeout,
        explicit_base,
    })
}

/// 是否允许自动拉起 sidecar。
fn allow_spawn(parsed: &Parsed) -> bool {
    if parsed.flag("no-spawn") {
        return false;
    }
    std::env::var("LRC_NO_SPAWN")
        .map(|v| v.is_empty())
        .unwrap_or(true)
}

/// 从 base URL 中解析端口，用于拉起 sidecar。
///
/// 用 `url` crate 而非字符串切分：`http://[::1]:3111` 这类 IPv6 地址
/// 用字符串切分会解析错，进而拉起一个端口不对的实例。
fn port_from_base(base: &str) -> u16 {
    url::Url::parse(base)
        .ok()
        .and_then(|parsed| parsed.port_or_known_default())
        .unwrap_or(3099)
}

/// base 是否指向本机。
///
/// 只有本机地址才允许自动拉起 sidecar：对远端地址 spawn 一个本地进程
/// 既无意义，又会往本地记忆库写入无用数据。
///
/// 注意 `Url::host_str()` 对 IPv6 返回带方括号的 `"[::1]"`，
/// 用 `parse::<IpAddr>()` 会失败，因此必须用 `Url::host()` 取结构化 Host。
fn is_loopback_base(base: &str) -> bool {
    use url::Host;
    match url::Url::parse(base) {
        Ok(parsed) => match parsed.host() {
            Some(Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
            Some(Host::Ipv4(ip)) => ip.is_loopback(),
            Some(Host::Ipv6(ip)) => ip.is_loopback(),
            None => false,
        },
        Err(_) => false,
    }
}

/// 写入时的项目归属：`--project` > `LRC_PROJECT` > 当前目录名。
///
/// 与 `integrations/hooks/claude-code/pre-compact.sh` 的 `basename "$PWD"` 对齐，
/// 避免同一份对话经 CLI 与 Hook 写入时归属不同。
fn resolve_write_project(parsed: &Parsed) -> Option<String> {
    parsed
        .value("project")
        .map(str::to_string)
        .or_else(|| std::env::var("LRC_PROJECT").ok().filter(|v| !v.is_empty()))
        .or_else(|| {
            std::env::current_dir().ok().and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().to_string())
            })
        })
}

/// 读取时的项目过滤：**不**回落当前目录。
///
/// 读取若默认按 cwd 过滤，用户会「看不到」其他项目的记忆，属于隐性行为变化。
fn resolve_filter_project(parsed: &Parsed) -> Option<String> {
    parsed
        .value("project")
        .map(str::to_string)
        .or_else(|| std::env::var("LRC_PROJECT").ok().filter(|v| !v.is_empty()))
}

/// 探测某个 base 是否是**可用的 LRC daemon**（能连上且身份正确）。
///
/// 只判 `service == "loong-recall"`：端口上开着别的服务时不能当成功。
fn probe_health(base: &str, token: Option<&str>, timeout: Duration) -> Option<Value> {
    let client = Client {
        base: base.to_string(),
        token: token.map(str::to_string),
        timeout,
        explicit_base: true,
    };
    match client.send("GET", "/health", None) {
        Ok(health) if health.get("service").and_then(Value::as_str) == Some("loong-recall") => {
            Some(health)
        }
        _ => None,
    }
}

/// 扫描本机端口区间，寻找已在运行的 LRC 实例。
///
/// ## 为什么必须并发
///
/// 串行扫描 100 个端口时，每个端口若被防火墙静默丢包就要等满超时，
/// 实测最坏 58s——这对 CLI 完全不可接受。改为 8 线程分段并发，
/// 并把区间收窄到 LRC 常用端口（3099–3118），最坏约 750ms。
fn scan_for_daemon(token: Option<&str>) -> Option<String> {
    use std::sync::mpsc;

    const START_PORT: u16 = 3099;
    const END_PORT: u16 = 3118;
    const WORKERS: usize = 8;
    const PER_PORT_TIMEOUT: Duration = Duration::from_millis(250);

    let token = token.map(str::to_string);
    let ports: Vec<u16> = (START_PORT..=END_PORT).collect();
    let per_worker = ports.len().div_ceil(WORKERS).max(1);

    let (sender, receiver) = mpsc::channel::<String>();
    let mut handles = Vec::new();

    for chunk in ports.chunks(per_worker) {
        let chunk = chunk.to_vec();
        let sender = sender.clone();
        let token = token.clone();
        handles.push(std::thread::spawn(move || {
            for port in chunk {
                let base = format!("http://127.0.0.1:{port}");
                if probe_health(&base, token.as_deref(), PER_PORT_TIMEOUT).is_some() {
                    let _ = sender.send(base);
                    return;
                }
            }
        }));
    }
    drop(sender);

    // 只要有一个命中就够；其余线程会很快扫完自己的分片。
    let found = receiver.recv_timeout(Duration::from_secs(2)).ok();
    for handle in handles {
        let _ = handle.join();
    }
    found
}

/// 确保 daemon 可用：探测 → 扫描复用 → 必要时拉起 → 轮询就绪。
fn ensure_daemon(client: &mut Client, spawn_allowed: bool) -> Result<(), CliError> {
    let probe = Client {
        timeout: Duration::from_secs(2),
        ..client.clone()
    };

    if let Ok(health) = probe.send("GET", "/health", None) {
        let service = health
            .get("service")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if service == "loong-recall" {
            return Ok(());
        }
        // 端口上有响应但不是 LRC：不尝试扫描，直接报错，避免用户误以为连上了
        return Err(CliError::Daemon(format!(
            "端口被其他服务占用（service={service:?}），请检查 {}",
            client.base
        )));
    }

    // 未显式指定 base 时才允许「发现已有实例」；显式指定则严格遵守。
    if !client.explicit_base {
        if let Some(base) = scan_for_daemon(client.token.as_deref()) {
            client.base = base;
            return Ok(());
        }
    }

    if !spawn_allowed {
        return Err(CliError::Daemon(format!(
            "daemon 未运行（{}），且已禁用自动拉起",
            client.base
        )));
    }

    // 只有本机地址才允许拉起：对远端地址 spawn 本地进程没有意义，
    // 还会往本地记忆库写入无人使用的数据。
    if !is_loopback_base(&client.base) {
        return Err(CliError::Daemon(format!(
            "远端 daemon 不可达（{}）；CLI 只会在本机自动拉起 sidecar",
            client.base
        )));
    }

    // base 必须带显式端口：无端口时 url 会推出 80/443，
    // 拉起 sidecar 会尝试绑定特权端口，失败原因难以理解。
    let has_explicit_port = url::Url::parse(&client.base)
        .ok()
        .and_then(|parsed| parsed.port())
        .is_some();
    if !has_explicit_port {
        return Err(CliError::Arg(
            "--base 必须带端口（例如 http://127.0.0.1:3099）".to_string(),
        ));
    }

    let port = port_from_base(&client.base);
    spawn_sidecar(port)?;

    // 5s 内每 200ms 探测一次。注意：这里只等「端口起来」，
    // 不等模型加载完成——与现有服务端行为一致，recall 在模型未就绪时
    // 会自然退化为关键词结果。
    for _ in 0..25 {
        std::thread::sleep(Duration::from_millis(200));
        if probe_health(
            &client.base,
            client.token.as_deref(),
            Duration::from_secs(1),
        )
        .is_some()
        {
            return Ok(());
        }
    }
    Err(CliError::Daemon(format!(
        "已尝试拉起 sidecar，但 5s 内未就绪（{}）。sidecar 日志: {}。\
         若反复失败，建议直接启动 LRC 桌面端，由它常驻 sidecar。",
        client.base,
        sidecar_log_path().display()
    )))
}

/// CLI 拉起的 sidecar 的日志落点。
///
/// 为什么不直接丢 null：拉起失败时用户没有任何线索，只能盲猜。
/// 与桌面端 `lrc-sidecar.log` 分开放，避免互相覆盖。
fn sidecar_log_path() -> PathBuf {
    let home = dirs_next::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".loong-recall")
        .join("logs")
        .join("lrc-sidecar-cli.log")
}

/// 查找 sidecar 可执行文件。
///
/// 顺序：与本进程同目录 → `LRC_SIDECAR` → PATH 中的 `code-memory-server`。
fn find_sidecar() -> Option<PathBuf> {
    let sidecar_name = if cfg!(windows) {
        "lrc-sidecar.exe"
    } else {
        "lrc-sidecar"
    };

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(sidecar_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    if let Some(raw) = std::env::var_os("LRC_SIDECAR") {
        let candidate = PathBuf::from(raw);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    let fallback = if cfg!(windows) {
        "code-memory-server.exe"
    } else {
        "code-memory-server"
    };
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(fallback);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

/// 构建 sidecar 启动命令（不含平台 flag 与 spawn）。
///
/// 抽成函数是为了「脱离 Job 失败后降级重试」时能重建一份等价命令
/// （`Command::spawn` 会消耗自身）。
fn configure_sidecar_command(exe: &Path, port: u16) -> std::process::Command {
    let mut command = std::process::Command::new(exe);
    command.arg("--daemon").arg("--port").arg(port.to_string());

    // 默认全局模式；`LRC_GLOBAL=0` 时保留 sidecar 自身的项目模式。
    if std::env::var("LRC_GLOBAL")
        .map(|value| value != "0")
        .unwrap_or(true)
    {
        command.arg("--global");
    }
    if let Ok(dir) = std::env::var("LRC_DATA_DIR") {
        if !dir.is_empty() {
            command.arg("--data-dir").arg(dir);
        }
    }
    // 与桌面端项目模式对齐：未显式指定时用当前目录作为索引根，
    // 避免 sidecar 继承到意外的工作目录。
    let src_dir = std::env::var("LRC_SRC_DIR")
        .ok()
        .filter(|dir| !dir.is_empty())
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|path| path.to_string_lossy().to_string())
        });
    if let Some(dir) = src_dir {
        command.arg("--src-dir").arg(dir);
    }

    command.stdin(std::process::Stdio::null());

    // 日志文件：追加写，避免每次拉起覆盖上次的失败原因。
    let log_path = sidecar_log_path();
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        Ok(file) => {
            let cloned = file.try_clone();
            command.stdout(std::process::Stdio::from(file));
            command.stderr(match cloned {
                Ok(clone) => std::process::Stdio::from(clone),
                Err(_) => std::process::Stdio::null(),
            });
        }
        Err(_) => {
            command.stdout(std::process::Stdio::null());
            command.stderr(std::process::Stdio::null());
        }
    }
    command
}

/// 后台拉起 sidecar。
///
/// ## 为什么优先尝试脱离 Job Object
///
/// AI 工具的「一次工具调用」往往运行在独立的 Job Object 里，调用结束即回收
/// 整个进程树。若 daemon 留在 Job 内，CLI 一退出它就死，下次调用又要重新拉起，
/// 每次都要付启动代价——这与「常驻 daemon 让 recall 亚秒返回」的设计目标直接冲突。
///
/// 因此 Windows 上先带 `CREATE_BREAKAWAY_FROM_JOB` 启动；宿主 Job 不允许
/// breakaway 时（普通控制台常见）降级为普通启动。
fn spawn_sidecar(port: u16) -> Result<(), CliError> {
    let exe = find_sidecar().ok_or_else(|| {
        CliError::Daemon(
            "未找到 sidecar 可执行文件（已查找：同目录 lrc-sidecar、LRC_SIDECAR、PATH 中 code-memory-server）"
                .to_string(),
        )
    })?;

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        // CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB
        let mut detached = configure_sidecar_command(&exe, port);
        detached.creation_flags(0x0800_0000 | 0x0100_0000);
        if detached.spawn().is_ok() {
            return Ok(());
        }

        // 宿主不允许 breakaway：降级为普通后台启动。
        let mut fallback = configure_sidecar_command(&exe, port);
        fallback.creation_flags(0x0800_0000);
        fallback.spawn().map_err(|error| {
            CliError::Daemon(format!("拉起 sidecar 失败（{}）: {error}", exe.display()))
        })?;
        Ok(())
    }

    #[cfg(not(windows))]
    {
        configure_sidecar_command(&exe, port)
            .spawn()
            .map_err(|error| {
                CliError::Daemon(format!("拉起 sidecar 失败（{}）: {error}", exe.display()))
            })?;
        Ok(())
    }
}

// ════════════════════════════════════════════════════════════════
// 命令实现
// ════════════════════════════════════════════════════════════════

fn cmd_recall(args: &[String]) -> Result<u8, CliError> {
    let parsed = parse_args(args)?;
    if parsed.flag("help") {
        print_usage();
        return Ok(EXIT_OK);
    }

    // 位置参数只允许一个：多余参数通常意味着用户漏了引号。
    // 静默取第一个会把「这 是 一句话」写成「这」——属于静默数据损坏。
    if parsed.positional.len() > 1 {
        return Err(CliError::Arg("查询文本包含空格时请用引号包住".to_string()));
    }

    let query = parsed
        .positional
        .first()
        .cloned()
        .or_else(|| parsed.value("query").map(str::to_string))
        .ok_or_else(|| {
            CliError::Arg("recall 需要查询文本，如 `lrc recall \"数据库选型\"`".to_string())
        })?;

    // 20s：服务端 handler 预算为 15s，CLI 必须留出余量让服务端先降级返回，
    // 否则用户拿到的是「超时」而不是「降级结果」。
    let mut client = build_client(&parsed, Duration::from_secs(20))?;
    ensure_daemon(&mut client, allow_spawn(&parsed))?;

    let started = Instant::now();
    let body = json!({
        "query": query,
        "top_k": parsed.value_usize("top-k", 5)?,
        "project": resolve_filter_project(&parsed),
        "user_id": parsed.value("user-id"),
        "session_id": parsed.value("session-id"),
    });
    let response = client.send("POST", "/v1/memories/enrich", Some(&body))?;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    let items: Vec<Value> = response
        .get("memories")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let count = items.len();

    if parsed.flag("json") {
        let projected: Vec<Value> = items
            .iter()
            .map(|item| {
                json!({
                    "id": field(item, "id"),
                    "content": field(item, "content"),
                    "memory_type": field(item, "memory_type"),
                    "importance": field(item, "importance"),
                    "score": field(item, "score"),
                    "score_norm": field(item, "score_norm"),
                    "created_at": field(item, "created_at"),
                    "event_id": field(item, "event_id"),
                })
            })
            .collect();

        let output = json!({
            "schema_version": SCHEMA_VERSION,
            "ok": true,
            "query": query,
            "count": count,
            "items": projected,
            "elapsed_ms": elapsed_ms,
        });
        print_json(&output)?;
    } else {
        for item in &items {
            let kind = item
                .get("memory_type")
                .and_then(Value::as_str)
                .unwrap_or("?");
            let importance = item.get("importance").and_then(Value::as_u64).unwrap_or(0);
            let content = item.get("content").and_then(Value::as_str).unwrap_or("");
            let score_norm = item
                .get("score_norm")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let shown = if parsed.flag("full") {
                content.to_string()
            } else {
                truncate(content, 300)
            };
            println!("[{kind}·{importance}] {shown}  ({score_norm:.2})");
        }
        println!("共 {count} 条，用时 {elapsed_ms}ms");
    }

    Ok(if count == 0 { EXIT_EMPTY } else { EXIT_OK })
}

fn cmd_remember(args: &[String]) -> Result<u8, CliError> {
    let parsed = parse_args(args)?;
    if parsed.flag("help") {
        print_usage();
        return Ok(EXIT_OK);
    }

    // 多余位置参数意味着漏引号：静默取第一个会写入残缺内容。
    // `-` 除外——它必须独占。
    if parsed.positional.len() > 1 {
        return Err(CliError::Arg(
            "内容包含空格时请用引号包住；多行文本请用 `-` 从 stdin 读取".to_string(),
        ));
    }

    let raw =
        parsed.positional.first().cloned().ok_or_else(|| {
            CliError::Arg("remember 需要内容，或使用 `-` 从 stdin 读取".to_string())
        })?;

    // `-` 是 AI 工具的主路径：多行文本走 argv 会被 shell 转义破坏。
    let content = if raw == "-" { read_stdin()? } else { raw };
    // stdin 常见尾部换行（PowerShell 管道会补 `\r\n`），
    // 不清理会让 `list` 输出多出空行，也会污染记忆内容。
    let content = content.trim().to_string();

    if content.is_empty() {
        return Err(CliError::Arg("记忆内容不能为空".to_string()));
    }

    let project = resolve_write_project(&parsed);

    let tags: Vec<String> = parsed
        .value("tags")
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|tag| !tag.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let mut client = build_client(&parsed, Duration::from_secs(30))?;
    ensure_daemon(&mut client, allow_spawn(&parsed))?;

    let started = Instant::now();
    let body = json!({
        "content": content,
        "memory_type": parsed.value("type").unwrap_or("fact"),
        "importance": parsed.value_u8("importance", 5)?,
        "project": project,
        "tags": tags,
        "event_id": parsed.value("event-id"),
        "privacy_level": parsed.value("privacy"),
        "user_id": parsed.value("user-id"),
        "session_id": parsed.value("session-id"),
    });
    let response = client.send("POST", "/v1/memories/remember", Some(&body))?;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    // 服务端成功时返回 `{success: true, memory_id}`。
    // 缺 id 却打印「已写入」会把服务端异常伪装成成功。
    let memory_id = response
        .get("memory_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if memory_id.is_empty() {
        return Err(CliError::Server(format!(
            "服务端未返回 memory_id，写入结果不确定；响应: {}",
            truncate(&response.to_string(), 300)
        )));
    }

    if parsed.flag("json") {
        let output = json!({
            "schema_version": SCHEMA_VERSION,
            "ok": true,
            "memory_id": memory_id,
            "elapsed_ms": elapsed_ms,
        });
        print_json(&output)?;
    } else {
        println!("已写入记忆 {memory_id}（{elapsed_ms}ms）");
    }

    Ok(EXIT_OK)
}

fn cmd_list(args: &[String]) -> Result<u8, CliError> {
    let parsed = parse_args(args)?;
    if parsed.flag("help") {
        print_usage();
        return Ok(EXIT_OK);
    }

    let mut client = build_client(&parsed, Duration::from_secs(15))?;
    ensure_daemon(&mut client, allow_spawn(&parsed))?;

    let started = Instant::now();
    let body = json!({
        "limit": parsed.value_usize("limit", 20)?,
        "project": resolve_filter_project(&parsed),
        "user_id": parsed.value("user-id"),
        "session_id": parsed.value("session-id"),
        // v0.11：默认隐藏过期记忆；导出/清理场景可显式包含。
        "include_expired": parsed.flag("include-expired"),
    });
    let response = client.send("POST", "/v1/memories/list", Some(&body))?;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    let memories: Vec<Value> = response
        .get("memories")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let total = response.get("total").and_then(Value::as_u64).unwrap_or(0);

    if parsed.flag("json") {
        let output = json!({
            "schema_version": SCHEMA_VERSION,
            "ok": true,
            "count": memories.len(),
            "total": total,
            "elapsed_ms": elapsed_ms,
            "items": memories.iter().map(|item| json!({
                "id": field(item, "id"),
                "content": field(item, "content"),
                "memory_type": field(item, "memory_type"),
                "project": field(item, "project"),
                "importance": field(item, "importance"),
                "created_at_ms": field(item, "created_at_ms"),
                "tags": field(item, "tags"),
                "expired": field(item, "expired"),
            })).collect::<Vec<_>>(),
        });
        print_json(&output)?;
    } else {
        for item in &memories {
            let kind = item
                .get("memory_type")
                .and_then(Value::as_str)
                .unwrap_or("?");
            let importance = item.get("importance").and_then(Value::as_u64).unwrap_or(0);
            let content = item.get("content").and_then(Value::as_str).unwrap_or("");
            let expired = item
                .get("expired")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let tag = if expired { "·已过期" } else { "" };
            println!("[{kind}·{importance}{tag}] {}", truncate(content, 200));
        }
        println!("共 {total} 条（本次展示 {} 条）", memories.len());
    }

    Ok(if memories.is_empty() {
        EXIT_EMPTY
    } else {
        EXIT_OK
    })
}

fn cmd_stats(args: &[String]) -> Result<u8, CliError> {
    let parsed = parse_args(args)?;
    if parsed.flag("help") {
        print_usage();
        return Ok(EXIT_OK);
    }

    let mut client = build_client(&parsed, Duration::from_secs(15))?;
    ensure_daemon(&mut client, allow_spawn(&parsed))?;

    let started = Instant::now();
    let response = client.send("GET", "/v1/memories/stats", None)?;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    if parsed.flag("json") {
        let output = json!({
            "schema_version": SCHEMA_VERSION,
            "ok": true,
            "elapsed_ms": elapsed_ms,
            "stats": response,
        });
        print_json(&output)?;
    } else {
        let total = response
            .get("total_memories")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let expired = response
            .get("expired_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let size = response
            .get("storage_size_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        println!("记忆总数: {total}");
        println!("已过期:   {expired}");
        println!("存储大小: {:.2} MB", size as f64 / 1024.0 / 1024.0);

        if let Some(by_type) = response.get("by_type").and_then(Value::as_object) {
            println!("按类型:");
            for (kind, count) in by_type {
                println!("  {kind}: {count}");
            }
        }
    }

    Ok(EXIT_OK)
}

fn cmd_search(args: &[String]) -> Result<u8, CliError> {
    let parsed = parse_args(args)?;
    if parsed.flag("help") {
        print_usage();
        return Ok(EXIT_OK);
    }

    // 与 recall / remember 同款：多余位置参数意味着漏引号，不能静默截断。
    if parsed.positional.len() > 1 {
        return Err(CliError::Arg("查询文本包含空格时请用引号包住".to_string()));
    }

    let query = parsed
        .positional
        .first()
        .cloned()
        .or_else(|| parsed.value("query").map(str::to_string))
        .ok_or_else(|| CliError::Arg("search 需要查询文本".to_string()))?;

    let mut client = build_client(&parsed, Duration::from_secs(15))?;
    ensure_daemon(&mut client, allow_spawn(&parsed))?;

    let top_k = parsed.value_usize("top-k", 5)?;
    let path = format!(
        "/v1/code/search?query={}&top_k={}",
        url_encode(&query),
        top_k
    );
    let response = client.send("GET", &path, None)?;

    let results: Vec<Value> = response
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let count = results.len();

    if parsed.flag("json") {
        let output = json!({
            "schema_version": SCHEMA_VERSION,
            "ok": true,
            "query": query,
            "count": count,
            "total_indexed": field(&response, "total_indexed"),
            "items": results.iter().map(|item| json!({
                "rank": field(item, "rank"),
                "score": field(item, "score"),
                "score_norm": field(item, "score_norm"),
                "file_path": field(item, "file_path"),
                "name": field(item, "name"),
                "language": field(item, "language"),
                "start_line": field(item, "start_line"),
                "end_line": field(item, "end_line"),
                "content": field(item, "content"),
            })).collect::<Vec<_>>(),
        });
        print_json(&output)?;
    } else {
        for item in &results {
            let file = item.get("file_path").and_then(Value::as_str).unwrap_or("?");
            let start = item.get("start_line").and_then(Value::as_u64).unwrap_or(0);
            let end = item.get("end_line").and_then(Value::as_u64).unwrap_or(0);
            let name = item.get("name").and_then(Value::as_str).unwrap_or("");
            let score = item
                .get("score_norm")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            println!("{file}:{start}-{end}  {name}  ({score:.2})");
        }
        if count == 0 {
            println!("共 0 条（若 daemon 刚启动，代码索引可能仍在进行，可稍后重试）");
        } else {
            println!("共 {count} 条");
        }
    }

    Ok(if count == 0 { EXIT_EMPTY } else { EXIT_OK })
}

fn cmd_doctor(args: &[String]) -> Result<u8, CliError> {
    let parsed = parse_args(args)?;
    if parsed.flag("help") {
        print_usage();
        return Ok(EXIT_OK);
    }

    let client = build_client(&parsed, Duration::from_secs(5))?;
    let probe = Client {
        timeout: Duration::from_secs(2),
        ..client.clone()
    };

    let mut checks: Vec<Value> = Vec::new();
    let mut failed = false;

    // 1) daemon 可达性与身份
    let health = probe.send("GET", "/health", None);
    let mut is_lrc = false;
    match &health {
        Ok(value) => {
            let service = value
                .get("service")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let version = value.get("version").and_then(Value::as_str).unwrap_or("?");
            let uptime = value
                .get("uptime_seconds")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            // /health 没有 status 字段（见 server.rs 的 HealthResponse），
            // 用 indexing.complete 反映就绪度更有信息量。
            let indexing_done = value
                .get("indexing")
                .and_then(|v| v.get("complete"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let phase = if indexing_done { "ready" } else { "indexing" };

            if service == "loong-recall" {
                is_lrc = true;
                record(
                    &mut checks,
                    "daemon",
                    "ok",
                    format!("{service} {version} phase={phase} uptime={uptime}s"),
                );
                // 2) 版本一致性
                let cli_version = env!("CARGO_PKG_VERSION");
                let matched = version == cli_version;
                // 版本不一致只是提醒：daemon 仍然可用，不应拉高退出码。
                record(
                    &mut checks,
                    "version",
                    if matched { "ok" } else { "warn" },
                    if matched {
                        format!("CLI {cli_version} == daemon {version}")
                    } else {
                        format!("CLI {cli_version} != daemon {version}（建议升级到同一版本）")
                    },
                );
            } else {
                failed = true;
                record(
                    &mut checks,
                    "daemon",
                    "fail",
                    format!("端口被其他服务占用（service={service:?}）"),
                );
            }
        }
        Err(error) => {
            failed = true;
            record(&mut checks, "daemon", "fail", error.message().to_string());
        }
    }

    // 3) 模型与系统模式
    // 用身份判定作门：端口上是别的服务时，后续请求只会产生误导性的 FAIL。
    if is_lrc {
        match client.send("GET", "/v1/health/system", None) {
            Ok(system) => {
                let encoder_mode = system
                    .get("encoder")
                    .and_then(|v| v.get("mode"))
                    .and_then(Value::as_str)
                    .unwrap_or("?");
                let system_mode = system
                    .get("system_mode")
                    .and_then(Value::as_str)
                    .unwrap_or("?");
                // 非 ml 构建属于「可用但语义检索缺席」：这是合法的用户配置
                // （未下载模型），记为 WARN 而非 FAIL，不拉高退出码。
                let level = if encoder_mode == "ml" { "ok" } else { "warn" };
                record(
                    &mut checks,
                    "model",
                    level,
                    if encoder_mode == "ml" {
                        format!("encoder.mode={encoder_mode} system_mode={system_mode}")
                    } else {
                        format!(
                            "encoder.mode={encoder_mode} system_mode={system_mode}（语义检索缺席，仅关键词可用）"
                        )
                    },
                );
            }
            Err(error) => {
                failed = true;
                record(&mut checks, "model", "fail", error.message().to_string());
            }
        }

        // 4) 数据目录
        match client.send("GET", "/v1/trust/data-location", None) {
            Ok(location) => {
                // 服务端字段名是 `data_directory`（v1_api.rs 的 trust 端点）
                let path = location
                    .get("data_directory")
                    .and_then(Value::as_str)
                    .unwrap_or("?");
                let exists = location
                    .get("file_exists")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                record(
                    &mut checks,
                    "data-dir",
                    "ok",
                    format!("{path}（memories.json 存在={exists}）"),
                );
            }
            Err(error) => {
                failed = true;
                record(&mut checks, "data-dir", "fail", error.message().to_string());
            }
        }
    }

    if parsed.flag("json") {
        let output = json!({
            "schema_version": SCHEMA_VERSION,
            "ok": !failed,
            "base": client.base,
            "checks": checks,
        });
        print_json(&output)?;
    } else {
        println!("LRC 自检（base: {}）", client.base);
        for check in &checks {
            let name = check.get("name").and_then(Value::as_str).unwrap_or("?");
            let level = check.get("level").and_then(Value::as_str).unwrap_or("fail");
            let detail = check.get("detail").and_then(Value::as_str).unwrap_or("");
            let tag = match level {
                "ok" => "OK  ",
                "warn" => "WARN",
                _ => "FAIL",
            };
            println!("[{tag}] {name:<10} {detail}");
        }
        if failed {
            println!("结论: 存在问题，请按上面 FAIL 项排查。");
        } else if checks
            .iter()
            .any(|check| check.get("level").and_then(Value::as_str) == Some("warn"))
        {
            println!("结论: 可用，但有警告项（见上面 WARN）。");
        } else {
            println!("结论: 全部通过。");
        }
    }

    Ok(if failed { EXIT_DAEMON } else { EXIT_OK })
}

// ════════════════════════════════════════════════════════════════
// 记忆维护与上下文接管
// ════════════════════════════════════════════════════════════════

fn cmd_forget(args: &[String]) -> Result<u8, CliError> {
    let parsed = parse_args(args)?;
    if parsed.flag("help") {
        print_usage();
        return Ok(EXIT_OK);
    }
    if parsed.positional.len() != 1 {
        return Err(CliError::Arg(
            "forget 需要且只接受一个 memory_id".to_string(),
        ));
    }
    let memory_id = parsed.positional[0].clone();

    // 删除不可撤销，且 AI 工具无法交互确认：必须显式带 --yes。
    if !parsed.flag("yes") {
        return Err(CliError::Arg(format!(
            "删除 {memory_id} 不可撤销；确认请加 --yes"
        )));
    }

    let mut client = build_client(&parsed, Duration::from_secs(15))?;
    // 破坏性操作：不做邻域扫描、不自动拉起，避免落到非预期实例。
    client.explicit_base = true;
    ensure_daemon(&mut client, false)?;

    let started = Instant::now();
    let response = client.send(
        "POST",
        "/v1/memories/forget",
        Some(&json!({
            "memory_id": memory_id,
            "user_id": parsed.value("user-id"),
            "session_id": parsed.value("session-id"),
        })),
    )?;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    // 服务端删除是幂等的：不存在时 deleted=false + already_deleted=true。
    let deleted = response
        .get("deleted")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if parsed.flag("json") {
        print_json(&json!({
            "schema_version": SCHEMA_VERSION,
            "ok": true,
            "memory_id": memory_id,
            "deleted": deleted,
            "already_deleted": !deleted,
            "elapsed_ms": elapsed_ms,
        }))?;
    } else if deleted {
        println!("已删除 {memory_id}（{elapsed_ms}ms）");
    } else {
        println!("{memory_id} 不存在或已删除（幂等成功，{elapsed_ms}ms）");
    }

    Ok(EXIT_OK)
}

fn cmd_correct(args: &[String]) -> Result<u8, CliError> {
    let parsed = parse_args(args)?;
    if parsed.flag("help") {
        print_usage();
        return Ok(EXIT_OK);
    }
    if parsed.positional.len() != 2 {
        return Err(CliError::Arg(
            "correct 需要两个参数：memory_id 与修正后的内容（内容可用 `-` 从 stdin 读取）"
                .to_string(),
        ));
    }
    let memory_id = parsed.positional[0].clone();
    let raw_content = parsed.positional[1].clone();
    let content = if raw_content == "-" {
        read_stdin()?
    } else {
        raw_content
    };
    let content = content.trim().to_string();
    if content.is_empty() {
        return Err(CliError::Arg("修正后的内容不能为空".to_string()));
    }

    let mut client = build_client(&parsed, Duration::from_secs(20))?;
    // 破坏性操作：不做邻域扫描、不自动拉起。
    client.explicit_base = true;
    ensure_daemon(&mut client, false)?;

    let started = Instant::now();
    let response = client.send(
        "POST",
        "/v1/memories/correct",
        Some(&json!({
            "memory_id": memory_id,
            "content": content,
            "reason": parsed.value("reason"),
            "user_id": parsed.value("user-id"),
            "session_id": parsed.value("session-id"),
        })),
    )?;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    let new_version = response
        .get("new_version")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let history_versions = response
        .get("history_versions")
        .and_then(Value::as_u64)
        .unwrap_or(0);

    if parsed.flag("json") {
        print_json(&json!({
            "schema_version": SCHEMA_VERSION,
            "ok": true,
            "memory_id": memory_id,
            "new_version": new_version,
            "history_versions": history_versions,
            "elapsed_ms": elapsed_ms,
        }))?;
    } else {
        println!(
            "已修正 {memory_id} → v{new_version}（历史版本 {history_versions} 个，{elapsed_ms}ms）"
        );
    }

    Ok(EXIT_OK)
}

fn cmd_associations(args: &[String]) -> Result<u8, CliError> {
    let parsed = parse_args(args)?;
    if parsed.flag("help") {
        print_usage();
        return Ok(EXIT_OK);
    }
    if parsed.positional.len() != 1 {
        return Err(CliError::Arg(
            "associations 需要且只接受一个 memory_id".to_string(),
        ));
    }
    let memory_id = parsed.positional[0].clone();

    let mut client = build_client(&parsed, Duration::from_secs(15))?;
    ensure_daemon(&mut client, allow_spawn(&parsed))?;

    let started = Instant::now();
    let response = client.send(
        "POST",
        "/v1/memories/associations",
        Some(&json!({
            "memory_id": memory_id,
            "relation": parsed.value("relation"),
            "user_id": parsed.value("user-id"),
            "session_id": parsed.value("session-id"),
        })),
    )?;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    let items: Vec<Value> = response
        .get("associations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    if parsed.flag("json") {
        print_json(&json!({
            "schema_version": SCHEMA_VERSION,
            "ok": true,
            "memory_id": memory_id,
            "count": items.len(),
            "items": items,
            "elapsed_ms": elapsed_ms,
        }))?;
    } else {
        for item in &items {
            let relation = item.get("relation").and_then(Value::as_str).unwrap_or("?");
            let target = item.get("memory_id").and_then(Value::as_str).unwrap_or("?");
            let why = item.get("why").and_then(Value::as_str).unwrap_or("");
            let preview = item
                .get("content_preview")
                .and_then(Value::as_str)
                .unwrap_or("");
            println!("[{relation}] {target}  {why}");
            if !preview.is_empty() {
                println!("    {}", truncate(preview, 160));
            }
        }
        println!("共 {} 条关联，用时 {elapsed_ms}ms", items.len());
    }

    Ok(if items.is_empty() {
        EXIT_EMPTY
    } else {
        EXIT_OK
    })
}

fn cmd_distill(args: &[String]) -> Result<u8, CliError> {
    let parsed = parse_args(args)?;
    if parsed.flag("help") {
        print_usage();
        return Ok(EXIT_OK);
    }

    if parsed.positional.len() > 1 {
        return Err(CliError::Arg(
            "对话原文包含空格时请用引号包住；多行文本请用 `-` 从 stdin 读取".to_string(),
        ));
    }

    // 两种输入互斥：同时给出时静默取其一会让用户误以为另一个生效。
    if parsed.value("transcript").is_some() && !parsed.positional.is_empty() {
        return Err(CliError::Arg(
            "--transcript 与文本参数不能同时使用".to_string(),
        ));
    }

    // 三种输入：--transcript 路径 / 位置参数文本 / `-` 读 stdin。
    let mut body = json!({
        "project": resolve_write_project(&parsed),
        "max_items": parsed.value_usize("max-items", 40)?,
    });
    if let Some(path) = parsed.value("transcript") {
        body["transcript_path"] = Value::String(path.to_string());
    } else {
        let text = match parsed.positional.first() {
            Some(raw) if raw == "-" => read_stdin()?,
            Some(raw) => raw.clone(),
            None => {
                return Err(CliError::Arg(
                    "distill 需要对话文本（位置参数或 `-`）或 --transcript <路径>".to_string(),
                ));
            }
        };
        if text.trim().is_empty() {
            return Err(CliError::Arg("对话文本不能为空".to_string()));
        }
        body["text"] = Value::String(text);
    }

    let mut client = build_client(&parsed, Duration::from_secs(30))?;
    ensure_daemon(&mut client, allow_spawn(&parsed))?;

    let started = Instant::now();
    let response = client.send("POST", "/v1/context/distill", Some(&body))?;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    let items: Vec<Value> = response
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let index_line = response
        .get("index_line")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    if parsed.flag("json") {
        print_json(&json!({
            "schema_version": SCHEMA_VERSION,
            "ok": true,
            "count": items.len(),
            "items": items,
            "index_line": index_line,
            "elapsed_ms": elapsed_ms,
        }))?;
    } else {
        for item in &items {
            let kind = item
                .get("memory_type")
                .and_then(Value::as_str)
                .unwrap_or("?");
            let importance = item.get("importance").and_then(Value::as_u64).unwrap_or(0);
            let content = item.get("content").and_then(Value::as_str).unwrap_or("");
            let expired = item
                .get("expired")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let tag = if expired { "·已过期" } else { "" };
            println!("[{kind}·{importance}{tag}] {}", truncate(content, 200));
        }
        if !index_line.is_empty() {
            println!("{index_line}");
        }
        println!(
            "共提炼 {} 条（未写入，需 `lrc migrate` 提交），用时 {elapsed_ms}ms",
            items.len()
        );
    }

    Ok(if items.is_empty() {
        EXIT_EMPTY
    } else {
        EXIT_OK
    })
}

fn cmd_migrate(args: &[String]) -> Result<u8, CliError> {
    let parsed = parse_args(args)?;
    if parsed.flag("help") {
        print_usage();
        return Ok(EXIT_OK);
    }

    // 输入：`-` 读 stdin，或 --file <路径>。
    let raw = if parsed.positional.first().map(String::as_str) == Some("-") {
        read_stdin()?
    } else if let Some(path) = parsed.value("file") {
        std::fs::read_to_string(path)
            .map_err(|error| CliError::Arg(format!("读取 {path} 失败: {error}")))?
    } else {
        return Err(CliError::Arg(
            "migrate 需要输入：`lrc distill --json | lrc migrate -` 或 `--file <路径>`".to_string(),
        ));
    };

    let payload: Value = serde_json::from_str(raw.trim())
        .map_err(|error| CliError::Arg(format!("输入不是合法 JSON: {error}")))?;

    // 兼容三种输入：distill 完整输出 {items:[...]}、直接数组、单条对象。
    let items: Vec<Value> = match &payload {
        Value::Object(map) => match map.get("items").and_then(Value::as_array) {
            Some(array) => array.clone(),
            None => vec![payload.clone()],
        },
        Value::Array(array) => array.clone(),
        _ => {
            return Err(CliError::Arg(
                "输入应为 distill 的输出（含 items）或记忆条目数组".to_string(),
            ));
        }
    };

    if items.is_empty() {
        println!("没有可写入的条目");
        return Ok(EXIT_EMPTY);
    }

    let mut client = build_client(&parsed, Duration::from_secs(60))?;
    // 运维操作：不做邻域扫描、不自动拉起（与 archive 一致）。
    client.explicit_base = true;
    ensure_daemon(&mut client, false)?;

    // 服务端单次上限 200 条（CONTEXT_MIGRATE_MAX_ITEMS），且每批有 10s 时间预算：
    // 超预算会返回 `remaining > 0 / truncated: true`，HTTP 仍是 200。
    // CLI 必须按 remaining 续传，否则会静默少写。
    const BATCH: usize = 200;
    let project = resolve_write_project(&parsed);
    let started = Instant::now();
    let mut written = 0u64;
    let mut merged = 0u64;
    let mut failed = 0u64;
    let mut batches = 0u64;
    let mut interrupted = false;

    'outer: for chunk in items.chunks(BATCH) {
        let mut start = 0usize;
        while start < chunk.len() {
            let slice = &chunk[start..];
            let response = match client.send(
                "POST",
                "/v1/context/migrate",
                Some(&json!({ "items": slice, "project": project })),
            ) {
                Ok(value) => value,
                Err(error) => {
                    // 已提交的批次不回滚：先报告已完成量，再以失败退出。
                    eprintln!(
                        "批次中断（已完成 写入 {written} / 合并 {merged} / 失败 {failed}）: {}",
                        error.message()
                    );
                    interrupted = true;
                    break 'outer;
                }
            };

            written += response.get("written").and_then(Value::as_u64).unwrap_or(0);
            merged += response.get("merged").and_then(Value::as_u64).unwrap_or(0);
            failed += response.get("failed").and_then(Value::as_u64).unwrap_or(0);
            batches += 1;

            let remaining = response
                .get("remaining")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize;
            if remaining == 0 {
                break;
            }
            let processed = slice.len().saturating_sub(remaining);
            if processed == 0 {
                // 服务端一条都没处理：继续重试只会死循环，按失败退出。
                eprintln!("服务端预算耗尽且未处理任何条目，已停止续传");
                interrupted = true;
                break 'outer;
            }
            start += processed;
        }
    }
    let elapsed_ms = started.elapsed().as_millis() as u64;

    if parsed.flag("json") {
        print_json(&json!({
            "schema_version": SCHEMA_VERSION,
            "ok": failed == 0 && !interrupted,
            "total": items.len(),
            "written": written,
            "merged": merged,
            "failed": failed,
            "batches": batches,
            "interrupted": interrupted,
            "elapsed_ms": elapsed_ms,
        }))?;
    } else {
        println!(
            "已提交 {} 条（写入 {written}，合并 {merged}，失败 {failed}，{batches} 批，{elapsed_ms}ms）",
            items.len()
        );
    }

    Ok(if failed > 0 || interrupted {
        EXIT_ERROR
    } else {
        EXIT_OK
    })
}

fn cmd_backup(args: &[String]) -> Result<u8, CliError> {
    let parsed = parse_args(args)?;
    if parsed.flag("help") {
        print_usage();
        return Ok(EXIT_OK);
    }

    let mut client = build_client(&parsed, Duration::from_secs(60))?;
    // 运维操作：不做邻域扫描、不自动拉起（与 archive 一致）。
    client.explicit_base = true;
    ensure_daemon(&mut client, false)?;

    let started = Instant::now();
    let response = client.send("POST", "/v1/backup", None)?;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    let success = response
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if parsed.flag("json") {
        print_json(&json!({
            "schema_version": SCHEMA_VERSION,
            "ok": success,
            "report": response,
            "elapsed_ms": elapsed_ms,
        }))?;
    } else if success {
        println!(
            "备份完成: {}（{} 条记忆，{} 字节，{elapsed_ms}ms）",
            response
                .get("backup_path")
                .and_then(Value::as_str)
                .unwrap_or("?"),
            response
                .get("memory_count")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            response
                .get("backup_size")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        );
    } else {
        println!(
            "备份失败: {}",
            response
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("未知错误")
        );
    }

    Ok(if success { EXIT_OK } else { EXIT_ERROR })
}

fn cmd_archive(args: &[String]) -> Result<u8, CliError> {
    let parsed = parse_args(args)?;
    if parsed.flag("help") {
        print_usage();
        return Ok(EXIT_OK);
    }

    // 全库运维操作：不做邻域扫描、不自动拉起。
    let mut client = build_client(&parsed, Duration::from_secs(60))?;
    client.explicit_base = true;
    ensure_daemon(&mut client, false)?;

    let started = Instant::now();
    let response = client.send("POST", "/v1/memories/archive-expired", None)?;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let archived = response
        .get("archived")
        .and_then(Value::as_u64)
        .unwrap_or(0);

    if parsed.flag("json") {
        print_json(&json!({
            "schema_version": SCHEMA_VERSION,
            "ok": true,
            "archived": archived,
            "elapsed_ms": elapsed_ms,
        }))?;
    } else {
        println!("已归档 {archived} 条过期记忆（{elapsed_ms}ms）");
    }

    Ok(EXIT_OK)
}

fn cmd_unarchive(args: &[String]) -> Result<u8, CliError> {
    let parsed = parse_args(args)?;
    if parsed.flag("help") {
        print_usage();
        return Ok(EXIT_OK);
    }
    if parsed.positional.len() != 1 {
        return Err(CliError::Arg(
            "unarchive 需要且只接受一个 memory_id".to_string(),
        ));
    }
    let memory_id = parsed.positional[0].clone();

    let mut body = json!({ "memory_id": memory_id });
    if let Some(raw) = parsed.value("ttl-days") {
        let ttl: u32 = raw
            .parse()
            .map_err(|_| CliError::Arg(format!("--ttl-days 需要非负整数，收到 `{raw}`")))?;
        // 与服务端 MAX_RESTORE_TTL_DAYS 同值：100 年，防畸形值写库
        if ttl > 36_500 {
            return Err(CliError::Arg(format!(
                "--ttl-days 不得超过 36500 天，收到 `{raw}`"
            )));
        }
        body["ttl_days"] = json!(ttl);
    }

    // 运维操作：不做邻域扫描、不自动拉起。
    let mut client = build_client(&parsed, Duration::from_secs(30))?;
    client.explicit_base = true;
    ensure_daemon(&mut client, false)?;

    let started = Instant::now();
    let response = client.send("POST", "/v1/memories/unarchive", Some(&body))?;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let restored = response
        .get("restored")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if parsed.flag("json") {
        print_json(&json!({
            "schema_version": SCHEMA_VERSION,
            "ok": true,
            "memory_id": memory_id,
            "restored": restored,
            "elapsed_ms": elapsed_ms,
        }))?;
    } else {
        println!("已恢复 {memory_id} 到活跃库（{elapsed_ms}ms）");
    }

    Ok(EXIT_OK)
}

fn cmd_purge_archived(args: &[String]) -> Result<u8, CliError> {
    let parsed = parse_args(args)?;
    if parsed.flag("help") {
        print_usage();
        return Ok(EXIT_OK);
    }
    if parsed.positional.len() != 1 {
        return Err(CliError::Arg(
            "purge-archived 需要且只接受一个 memory_id".to_string(),
        ));
    }
    let memory_id = parsed.positional[0].clone();
    // 彻底删除不可恢复，必须显式确认。
    if !parsed.flag("yes") {
        return Err(CliError::Arg(format!(
            "彻底删除归档记忆 {memory_id} 不可恢复；确认请加 --yes"
        )));
    }

    let mut client = build_client(&parsed, Duration::from_secs(30))?;
    client.explicit_base = true;
    ensure_daemon(&mut client, false)?;

    let started = Instant::now();
    let response = client.send(
        "POST",
        "/v1/memories/purge-archived",
        Some(&json!({ "memory_id": memory_id })),
    )?;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let deleted = response
        .get("deleted")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if parsed.flag("json") {
        print_json(&json!({
            "schema_version": SCHEMA_VERSION,
            "ok": true,
            "memory_id": memory_id,
            "deleted": deleted,
            "elapsed_ms": elapsed_ms,
        }))?;
    } else {
        println!("已从归档彻底删除 {memory_id}（{elapsed_ms}ms）");
    }

    Ok(EXIT_OK)
}

// ════════════════════════════════════════════════════════════════
// stdio MCP 桥
// ════════════════════════════════════════════════════════════════

/// `lrc mcp`：把 stdin/stdout 上的 MCP JSON-RPC 转发给常驻 daemon 的 `/mcp`。
///
/// ## 为什么是桥，而不是再起一个 `--stdio` 实例
///
/// sidecar 的 stdio 模式同样会获取单例锁、加载语义模型、使用独立的数据目录。
/// 若 `lrc mcp` 直接 spawn 它，会与用户已有的 daemon 争抢同一把项目锁，
/// 并可能形成两套数据。桥接则保证：**一个 daemon、一份数据、一套模型**。
fn cmd_mcp(args: &[String]) -> Result<u8, CliError> {
    let parsed = parse_args(args)?;
    if parsed.flag("help") {
        print_usage();
        return Ok(EXIT_OK);
    }

    // MCP 请求可能包含较慢的 recall，超时必须明显高于单次业务预算。
    let mut client = build_client(&parsed, Duration::from_secs(120))?;
    ensure_daemon(&mut client, allow_spawn(&parsed))?;

    // stdout 专用于协议，诊断信息一律走 stderr。
    eprintln!("[lrc mcp] 已连接 {}，stdio 桥就绪", client.base);

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut output = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("[lrc mcp] 读取 stdin 失败: {error}");
                break;
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // 先校验 JSON：非协议内容不能原样发给服务端，否则只会得到难懂的 400。
        let request: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(error) => {
                let response = json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": { "code": -32700, "message": format!("Parse error: {error}") }
                });
                write_line(&mut output, &response);
                continue;
            }
        };

        // JSON-RPC 批量请求（数组）：服务端只接受单个对象，
        // 这里逐项转发再把有响应的结果聚合成数组返回。
        if let Value::Array(items) = &request {
            if items.is_empty() {
                write_line(
                    &mut output,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": Value::Null,
                        "error": { "code": -32600, "message": "Invalid Request: empty batch" }
                    }),
                );
                continue;
            }
            let mut responses: Vec<Value> = Vec::new();
            for item in items {
                match client.send("POST", "/mcp", Some(item)) {
                    Ok(value) if !value.is_null() => responses.push(value),
                    Ok(_) => {}
                    Err(error) => {
                        if item.get("id").is_some() {
                            responses.push(json!({
                                "jsonrpc": "2.0",
                                "id": item.get("id").cloned().unwrap_or(Value::Null),
                                "error": { "code": -32603, "message": error.message() }
                            }));
                        }
                    }
                }
            }
            if !responses.is_empty() {
                write_line(&mut output, &Value::Array(responses));
            }
            continue;
        }

        // 通知（没有 id）不需要响应；服务端返回空 body 时也不写 stdout。
        let is_notification = request.get("id").is_none();

        match client.send("POST", "/mcp", Some(&request)) {
            Ok(response) => {
                if !response.is_null() {
                    write_line(&mut output, &response);
                }
            }
            Err(error) => {
                if !is_notification {
                    let response = json!({
                        "jsonrpc": "2.0",
                        "id": request.get("id").cloned().unwrap_or(Value::Null),
                        "error": { "code": -32603, "message": error.message() }
                    });
                    write_line(&mut output, &response);
                }
            }
        }
    }

    Ok(EXIT_OK)
}

fn write_line(output: &mut impl Write, value: &Value) {
    if let Ok(text) = serde_json::to_string(value) {
        let _ = writeln!(output, "{text}");
        let _ = output.flush();
    }
}

// ════════════════════════════════════════════════════════════════
// daemon 管理
// ════════════════════════════════════════════════════════════════

fn cmd_daemon(args: &[String]) -> Result<u8, CliError> {
    let parsed = parse_args(args)?;
    if parsed.flag("help") {
        print_usage();
        return Ok(EXIT_OK);
    }

    match parsed.positional.first().map(String::as_str) {
        Some("start") => daemon_start(&parsed),
        Some("stop") => daemon_stop(&parsed),
        None | Some("status") => daemon_status(&parsed),
        Some(other) => Err(CliError::Arg(format!(
            "未知 daemon 子命令 `{other}`，可用: start | stop | status"
        ))),
    }
}

/// 当前 daemon 的运行信息（探测成功时）。
struct DaemonInfo {
    base: String,
    version: String,
    uptime_seconds: u64,
    pid: Option<u32>,
    data_dir: Option<String>,
}

fn probe_daemon(client: &Client) -> Option<DaemonInfo> {
    let probe = Client {
        timeout: Duration::from_secs(2),
        ..client.clone()
    };
    let health = probe.send("GET", "/health", None).ok()?;
    if health.get("service").and_then(Value::as_str) != Some("loong-recall") {
        return None;
    }

    let data_dir = client
        .send("GET", "/v1/trust/data-location", None)
        .ok()
        .and_then(|value| {
            value
                .get("data_directory")
                .and_then(Value::as_str)
                .map(str::to_string)
        });

    // 端口反查优先：多窗口时 `.lrc.lock` 会列出多个 PID，无法对应本次 base。
    let pid = find_pid_by_port(port_from_base(&client.base))
        .or_else(|| data_dir.as_deref().and_then(read_lock_pid));

    Some(DaemonInfo {
        base: client.base.clone(),
        version: health
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string(),
        uptime_seconds: health
            .get("uptime_seconds")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        pid,
        data_dir,
    })
}

fn read_lock_pid(data_dir: &str) -> Option<u32> {
    // 持久锁是 `.lrc.lock`（内容为逗号分隔的 PID 列表，支持多窗口）；
    // `.lrc.lock.guard` 只是获取锁时的临时互斥文件，成功后即删除。
    let path = Path::new(data_dir).join(".lrc.lock");
    let content = std::fs::read_to_string(path).ok()?;
    content
        .split(',')
        .filter_map(|part| part.trim().parse::<u32>().ok())
        .next()
}

/// 反查监听指定端口的进程 PID。
///
/// 为什么优先用它而不是锁文件：锁文件可能列出多个 PID（多窗口），
/// 无法判断哪个对应本次 `--base` 的端口；端口反查是权威映射。
#[cfg(windows)]
fn find_pid_by_port(port: u16) -> Option<u32> {
    let output = std::process::Command::new("netstat")
        .args(["-ano", "-p", "TCP"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let suffix = format!(":{port}");
    for line in text.lines() {
        let columns: Vec<&str> = line.split_whitespace().collect();
        if columns.len() < 5 || !columns[3].eq_ignore_ascii_case("LISTENING") {
            continue;
        }
        // 精确比对端口部分：`ends_with(":3099")` 之外的模糊匹配容易取错行。
        let Some((_, local_port)) = columns[1].rsplit_once(':') else {
            continue;
        };
        if local_port != suffix.trim_start_matches(':') {
            continue;
        }
        if let Ok(pid) = columns[4].parse::<u32>() {
            return Some(pid);
        }
    }
    None
}

#[cfg(not(windows))]
fn find_pid_by_port(port: u16) -> Option<u32> {
    let output = std::process::Command::new("lsof")
        .args(["-ti", &format!(":{port}")])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()?
        .trim()
        .parse()
        .ok()
}

fn daemon_status(parsed: &Parsed) -> Result<u8, CliError> {
    let client = build_client(parsed, Duration::from_secs(5))?;
    let info = probe_daemon(&client);

    if parsed.flag("json") {
        let output = match &info {
            Some(info) => json!({
                "schema_version": SCHEMA_VERSION,
                "ok": true,
                "running": true,
                "base": info.base,
                "version": info.version,
                "uptime_seconds": info.uptime_seconds,
                "pid": info.pid,
                "data_dir": info.data_dir,
            }),
            None => json!({
                "schema_version": SCHEMA_VERSION,
                "ok": true,
                "running": false,
                "base": client.base,
            }),
        };
        print_json(&output)?;
        return Ok(if info.is_some() { EXIT_OK } else { EXIT_DAEMON });
    }

    match info {
        Some(info) => {
            println!("运行中");
            println!("  地址:   {}", info.base);
            println!("  版本:   {}", info.version);
            println!("  运行:   {}s", info.uptime_seconds);
            println!(
                "  PID:    {}",
                info.pid
                    .map(|pid| pid.to_string())
                    .unwrap_or_else(|| "未知".to_string())
            );
            println!(
                "  数据:   {}",
                info.data_dir.unwrap_or_else(|| "未知".to_string())
            );
            Ok(EXIT_OK)
        }
        None => {
            println!("未运行（{}）", client.base);
            Ok(EXIT_DAEMON)
        }
    }
}

fn daemon_start(parsed: &Parsed) -> Result<u8, CliError> {
    let mut client = build_client(parsed, Duration::from_secs(5))?;

    if let Some(info) = probe_daemon(&client) {
        if parsed.flag("json") {
            print_json(&json!({
                "schema_version": SCHEMA_VERSION,
                "ok": true,
                "already_running": true,
                "base": info.base,
                "pid": info.pid,
            }))?;
        } else {
            println!(
                "已在运行（{}，PID {}）",
                info.base,
                info.pid
                    .map(|pid| pid.to_string())
                    .unwrap_or_else(|| "未知".to_string())
            );
        }
        return Ok(EXIT_OK);
    }

    ensure_daemon(&mut client, allow_spawn(parsed))?;

    let info = probe_daemon(&client);
    if parsed.flag("json") {
        print_json(&json!({
            "schema_version": SCHEMA_VERSION,
            "ok": true,
            "already_running": false,
            "base": client.base,
            "pid": info.as_ref().and_then(|info| info.pid),
        }))?;
    } else {
        println!("已启动（{}）", client.base);
    }
    Ok(EXIT_OK)
}

fn daemon_stop(parsed: &Parsed) -> Result<u8, CliError> {
    let client = build_client(parsed, Duration::from_secs(5))?;

    // 只允许停本机实例：远端进程不归本机管。
    if !is_loopback_base(&client.base) {
        return Err(CliError::Arg(
            "只允许停止本机 daemon（base 不是回环地址）".to_string(),
        ));
    }

    let Some(info) = probe_daemon(&client) else {
        println!("未运行（{}），无需停止", client.base);
        return Ok(EXIT_OK);
    };

    // 端口反查优先（多窗口时锁文件会有多个 PID，无法对应本次 base）；
    // 回退到锁文件首个 PID。
    let port = port_from_base(&client.base);
    let Some(pid) = find_pid_by_port(port).or(info.pid) else {
        return Err(CliError::Daemon(
            "无法确定 daemon 的 PID；请通过桌面端停止，或手动结束进程".to_string(),
        ));
    };

    // 校验 PID 确实是 LRC 进程：PID 会被系统复用，盲杀可能误伤其他程序。
    match process_name(pid) {
        Some(name) if is_lrc_process(&name) => {}
        Some(name) => {
            return Err(CliError::Daemon(format!(
                "PID {pid} 当前对应进程 `{name}`，不是 LRC；已拒绝终止"
            )));
        }
        None => {
            return Err(CliError::Daemon(format!(
                "无法确认 PID {pid} 的进程身份；已拒绝终止"
            )));
        }
    }

    terminate_pid(pid)?;
    // 等待端口释放，最多 3s
    for _ in 0..15 {
        std::thread::sleep(Duration::from_millis(200));
        if probe_daemon(&client).is_none() {
            println!("已停止（PID {pid}）");
            println!(
                "提示: 若 LRC 桌面端正在运行，它会在约 10s 内自动重启 sidecar；\
                 如需彻底停止请先退出桌面端。"
            );
            return Ok(EXIT_OK);
        }
    }
    Err(CliError::Daemon(format!(
        "已向 PID {pid} 发送终止信号，但 3s 内端口仍可访问"
    )))
}

fn is_lrc_process(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("lrc-sidecar") || lower.contains("code-memory-server")
}

#[cfg(windows)]
fn process_name(pid: u32) -> Option<String> {
    let output = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    // 输出形如: "lrc-sidecar.exe","1234","Console","1","12,345 K"
    // 中文系统「无匹配」时输出的是提示语而非 CSV，必须校验结构再取字段。
    let first = text.lines().next()?.trim();
    if !first.starts_with('"') {
        return None;
    }
    let columns: Vec<&str> = first.split(',').collect();
    if columns.len() < 2 {
        return None;
    }
    let name = columns[0].trim().trim_matches('"').to_string();
    let listed_pid = columns[1].trim().trim_matches('"');
    if listed_pid != pid.to_string() {
        return None;
    }
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

#[cfg(not(windows))]
fn process_name(pid: u32) -> Option<String> {
    if let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/comm")) {
        return Some(text.trim().to_string());
    }
    let output = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// 等待进程退出。用 `process_name` 判存活，跨平台一致。
fn wait_pid_gone(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if process_name(pid).is_none() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    process_name(pid).is_none()
}

#[cfg(windows)]
fn terminate_pid(pid: u32) -> Result<(), CliError> {
    // 1) 优雅终止（不带 /F）。对无窗口的 console 进程常直接失败，失败不算错。
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string()])
        .output();
    if wait_pid_gone(pid, Duration::from_secs(3)) {
        return Ok(());
    }

    // 2) 3s 未退出才强制。
    let forced = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .output()
        .map_err(|error| CliError::Daemon(format!("调用 taskkill 失败: {error}")))?;
    if !forced.status.success() {
        return Err(CliError::Daemon(format!(
            "taskkill 未能终止 PID {pid}: {}",
            String::from_utf8_lossy(&forced.stderr).trim()
        )));
    }
    if wait_pid_gone(pid, Duration::from_secs(2)) {
        Ok(())
    } else {
        Err(CliError::Daemon(format!("PID {pid} 在强制终止后仍存活")))
    }
}

#[cfg(not(windows))]
fn terminate_pid(pid: u32) -> Result<(), CliError> {
    let _ = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status();
    if wait_pid_gone(pid, Duration::from_secs(3)) {
        return Ok(());
    }
    let _ = std::process::Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .status();
    if wait_pid_gone(pid, Duration::from_secs(2)) {
        Ok(())
    } else {
        Err(CliError::Daemon(format!("PID {pid} 在 SIGKILL 后仍存活")))
    }
}

// ════════════════════════════════════════════════════════════════
// 工具函数
// ════════════════════════════════════════════════════════════════

/// 从 JSON 对象取字段，缺失时返回 `null`（保持 schema 稳定，不省略键）。
fn field(value: &Value, key: &str) -> Value {
    value.get(key).cloned().unwrap_or(Value::Null)
}

fn record(checks: &mut Vec<Value>, name: &str, level: &str, detail: String) {
    checks.push(json!({
        "name": name,
        "level": level,
        "ok": level != "fail",
        "detail": detail,
    }));
}

/// 按**字符**截断，绝不按字节——按字节切中文会 panic（项目曾因此在
/// `exploration_log.rs` 触发过服务退出）。
fn truncate(text: &str, max_chars: usize) -> String {
    let mut output = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index >= max_chars {
            output.push('…');
            break;
        }
        output.push(ch);
    }
    output
}

/// 最小 URL 编码，用于 query 参数。
fn url_encode(text: &str) -> String {
    let mut output = String::new();
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                output.push(*byte as char);
            }
            _ => output.push_str(&format!("%{byte:02X}")),
        }
    }
    output
}

/// 从 stdin 读取全部内容。非法 UTF-8 按替换字符处理，不 panic。
fn read_stdin() -> Result<String, CliError> {
    // 上限 32MB：防止一次管道把超大 JSON 全量载入内存。
    const MAX_STDIN_BYTES: u64 = 32 * 1024 * 1024;
    let mut buffer = Vec::new();
    std::io::stdin()
        .take(MAX_STDIN_BYTES)
        .read_to_end(&mut buffer)
        // 读 stdin 失败是运行时 IO 问题，不是参数错误（退出码应为 1）
        .map_err(|error| CliError::Server(format!("读取 stdin 失败: {error}")))?;
    Ok(String::from_utf8_lossy(&buffer).to_string())
}

fn print_json(value: &Value) -> Result<(), CliError> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| CliError::Server(format!("序列化输出失败: {error}")))?;
    println!("{text}");
    Ok(())
}

/// Windows 控制台切到 UTF-8 代码页。
///
/// 管道场景（AI 工具读取 stdout）本来就按 UTF-8 解析，本函数只影响
/// 人工在 cmd / PowerShell 里直接运行时的显示效果。
#[cfg(windows)]
fn force_utf8_console() {
    // SAFETY: SetConsoleOutputCP / SetConsoleCP 只修改当前控制台代码页，
    // 不涉及指针、内存所有权或线程状态；调用失败仅返回 0，不会造成 UB。
    unsafe {
        use windows_sys::Win32::System::Console::{SetConsoleCP, SetConsoleOutputCP};
        SetConsoleOutputCP(65001);
        SetConsoleCP(65001);
    }
}

#[cfg(not(windows))]
fn force_utf8_console() {}
