//! 通用 AI 工具集成（v0.11.5）
//!
//! 目标：规则注入不依赖「是否检测到工具」「是否有 hook」。
//!
//! 三条通道，任一命中即可生效：
//!   1. MCP `initialize.instructions` —— 所有 MCP 客户端自动拿到强制规则；
//!   2. 规则文件 —— Codex（`~/.codex/AGENTS.md`）、DSH（`~/.agents/AGENTS.md`）、
//!      WorkBuddy（`~/.codebuddy/rules/lrc-memory.md`）；
//!   3. MCP 配置 —— Codex（`~/.codex/config.toml`）、WorkBuddy（`~/.workbuddy/mcp.json`）。
//!
//! DSH 没有 MCP 客户端，规则文件 + dsh-plugin-lrc 插件即可覆盖上下文管理。
//!
//! 所有写入都遵循：先备份、幂等、保留用户已有内容、原子替换、失败只告警。

use std::fs;
use std::path::{Path, PathBuf};

use toml_edit::{value, Array, Document, Item, Table};

use crate::agent_detector::AgentDetectorRegistry;

const BEGIN_MARKER: &str = "<!-- LRC_RULES_BEGIN -->";
const END_MARKER: &str = "<!-- LRC_RULES_END -->";

/// 一次注入的结果（用于日志与测试）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InjectReport {
    pub tool: &'static str,
    pub path: String,
    pub action: &'static str,
    pub error: Option<String>,
}

impl InjectReport {
    fn ok(tool: &'static str, path: &Path, action: &'static str) -> Self {
        Self {
            tool,
            path: path.display().to_string(),
            action,
            error: None,
        }
    }

    fn err(tool: &'static str, path: &Path, error: impl Into<String>) -> Self {
        Self {
            tool,
            path: path.display().to_string(),
            action: "error",
            error: Some(error.into()),
        }
    }
}

/// 启动时注入规则文件（不需要 sidecar 端口）。
///
/// 覆盖：
/// - 通用 `~/.agents/AGENTS.md`（DSH 及任何读取该位置的 AGENTS.md 宿主）；
/// - Codex `~/.codex/AGENTS.md`（仅当 `~/.codex` 存在）；
/// - WorkBuddy `~/.codebuddy/rules/lrc-memory.md`（仅当 WorkBuddy/CodeBuddy 存在）；
/// - `$DSH_HOME/AGENTS.md`（环境变量存在时）。
pub fn inject_universal_rules(home: &Path) -> Vec<InjectReport> {
    let mut reports = Vec::new();

    // 1) 通用 AGENTS.md：无条件写入，作为「未知工具」的兜底通道。
    //    内容只包含 LRC 规则块，已有用户内容会被保留。
    let generic = AgentDetectorRegistry::generate_ai_rules_content("generic");
    reports.push(write_rules_file(
        &home.join(".agents").join("AGENTS.md"),
        &generic,
        "agents",
    ));

    // 2) Codex：全局 AGENTS.md 是官方支持的持久化指令位置。
    if home.join(".codex").exists() {
        let content = AgentDetectorRegistry::generate_ai_rules_content("codex-cli");
        reports.push(write_rules_file(
            &home.join(".codex").join("AGENTS.md"),
            &content,
            "codex",
        ));
    }

    // 3) WorkBuddy：规则与 CodeBuddy 共用 ~/.codebuddy/rules/。
    //    只在 WorkBuddy 自身存在时写入，避免仅装 CodeBuddy 时重复写一份规则。
    if home.join(".workbuddy").exists() || home.join(".workbuddy-ai").exists() {
        let content = AgentDetectorRegistry::generate_ai_rules_content("workbuddy");
        reports.push(write_rules_file(
            &home.join(".codebuddy").join("rules").join("lrc-memory.md"),
            &content,
            "workbuddy",
        ));
    }

    // 4) DSH 自定义 DSH_HOME（默认的 ~/.agents/AGENTS.md 已由第 1 步覆盖）。
    if let Some(dsh_home) = std::env::var_os("DSH_HOME") {
        let path = PathBuf::from(dsh_home).join("AGENTS.md");
        let content = AgentDetectorRegistry::generate_ai_rules_content("dsh");
        reports.push(write_rules_file(&path, &content, "dsh"));
    }

    reports
}

/// sidecar 启动后写入 MCP 配置（需要真实端口）。
pub fn inject_mcp_configs(home: &Path, port: u16, lrc_binary: Option<&str>) -> Vec<InjectReport> {
    vec![
        inject_mcp_config_for("codex-cli", home, port, lrc_binary),
        inject_mcp_config_for("workbuddy", home, port, lrc_binary),
    ]
}

/// 按工具 ID 写入 MCP 配置（供 configure() 复用）。
pub fn inject_mcp_config_for(
    tool_id: &str,
    home: &Path,
    port: u16,
    lrc_binary: Option<&str>,
) -> InjectReport {
    match tool_id {
        "codex-cli" | "codex" => ensure_codex_config(home, port, lrc_binary),
        "workbuddy" => ensure_workbuddy_config(home, port),
        _ => InjectReport::ok("unknown", home, "skipped"),
    }
}

/// 解析 `lrc` CLI 的绝对路径（Codex `notify` 需要）。
///
/// 注意：桌面端 registry 里的 `lrc_binary_path` 是 `lrc-sidecar`，
/// 它没有 `absorb` 子命令，绝不能用于 notify。
pub fn resolve_lrc_cli() -> Option<String> {
    let exe = if cfg!(windows) { "lrc.exe" } else { "lrc" };

    if let Some(home) = dirs::home_dir() {
        let candidate = home.join(".loong-recall").join("bin").join(exe);
        if candidate.is_file() {
            return Some(candidate.display().to_string());
        }
    }

    if let Ok(current) = std::env::current_exe() {
        if let Some(dir) = current.parent() {
            let candidate = dir.join(exe);
            if candidate.is_file() {
                return Some(candidate.display().to_string());
            }
        }
    }

    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(exe);
            if candidate.is_file() {
                return Some(candidate.display().to_string());
            }
        }
    }

    None
}

/// Codex CLI：`~/.codex/config.toml`
///
/// - `[mcp_servers.lrc-memory] url = "http://127.0.0.1:<port>/mcp"`
/// - `notify = ["<lrc>", "absorb", "--host", "codex", ...]`（仅在用户没有配置 notify 时）
///
/// 用 toml_edit 保留用户注释与键顺序；解析失败时原样保留，不做任何修改。
fn ensure_codex_config(home: &Path, port: u16, lrc_binary: Option<&str>) -> InjectReport {
    let dir = home.join(".codex");
    let path = dir.join("config.toml");
    if !dir.exists() {
        return InjectReport::ok("codex", &path, "skipped");
    }

    let existing = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return InjectReport::err("codex", &path, format!("读取失败: {e}")),
    };

    let mut doc: Document = match existing.parse() {
        Ok(doc) => doc,
        Err(e) => {
            // 用户配置有语法错误时绝不覆盖，避免把 Codex 配置写坏。
            return InjectReport::err("codex", &path, format!("config.toml 解析失败，未修改: {e}"));
        }
    };

    let before = doc.to_string();
    let table = doc.as_table_mut();

    // ── MCP 服务 ──
    if !table.contains_key("mcp_servers") {
        table.insert("mcp_servers", Item::Table(Table::new()));
    }
    let servers = match table.get_mut("mcp_servers").and_then(Item::as_table_mut) {
        Some(servers) => servers,
        None => {
            return InjectReport::err("codex", &path, "mcp_servers 不是表，未修改");
        }
    };
    if !servers.contains_key("lrc-memory") {
        servers.insert("lrc-memory", Item::Table(Table::new()));
    }
    let lrc = match servers.get_mut("lrc-memory").and_then(Item::as_table_mut) {
        Some(lrc) => lrc,
        None => {
            return InjectReport::err("codex", &path, "mcp_servers.lrc-memory 不是表，未修改");
        }
    };
    lrc["url"] = value(format!("http://127.0.0.1:{port}/mcp"));

    // ── notify（自动沉淀；用户已有 notify 或找不到 lrc CLI 时不写）──
    if !table.contains_key("notify") {
        if let Some(lrc_cli) = lrc_binary {
            let mut args = Array::new();
            args.push(lrc_cli);
            for arg in ["absorb", "--host", "codex", "--no-spawn", "--timeout", "4000", "--json"] {
                args.push(arg);
            }
            table.insert("notify", value(args));
        }
    }

    let after = doc.to_string();
    if after == before {
        return InjectReport::ok("codex", &path, "unchanged");
    }

    backup_once(&path);
    match atomic_write(&path, &after) {
        Ok(()) => InjectReport::ok("codex", &path, "updated"),
        Err(e) => InjectReport::err("codex", &path, format!("写入失败: {e}")),
    }
}

/// WorkBuddy：`~/.workbuddy/mcp.json`（用户级，官方文档路径）。
///
/// 保留用户已有的其他 MCP 服务器；只新增/更新 `lrc-memory` 一项。
fn ensure_workbuddy_config(home: &Path, port: u16) -> InjectReport {
    let dir = home.join(".workbuddy");
    let path = dir.join("mcp.json");
    let installed = dir.exists() || home.join(".workbuddy-ai").exists();
    if !installed {
        return InjectReport::ok("workbuddy", &path, "skipped");
    }

    let existing = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return InjectReport::err("workbuddy", &path, format!("读取失败: {e}")),
    };

    let mut root: serde_json::Value = if existing.trim().is_empty() {
        serde_json::json!({})
    } else {
        match serde_json::from_str(&existing) {
            Ok(value) => value,
            Err(e) => {
                // 配置损坏时不覆盖，避免把 WorkBuddy 的 MCP 配置写坏。
                return InjectReport::err("workbuddy", &path, format!("mcp.json 解析失败，未修改: {e}"));
            }
        }
    };

    let Some(root_obj) = root.as_object_mut() else {
        return InjectReport::err("workbuddy", &path, "mcp.json 顶层不是对象，未修改");
    };
    let servers = root_obj
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    let Some(servers_obj) = servers.as_object_mut() else {
        return InjectReport::err("workbuddy", &path, "mcpServers 不是对象，未修改");
    };
    servers_obj.insert(
        "lrc-memory".to_string(),
        serde_json::json!({
            // WorkBuddy 官方文档使用 streamableHttp（不是 http）
            "type": "streamableHttp",
            "url": format!("http://127.0.0.1:{port}/mcp"),
            "description": "LRC — 本地代码记忆与语义搜索"
        }),
    );

    let after = match serde_json::to_string_pretty(&root) {
        Ok(text) => text,
        Err(e) => return InjectReport::err("workbuddy", &path, format!("序列化失败: {e}")),
    };
    if after == existing {
        return InjectReport::ok("workbuddy", &path, "unchanged");
    }

    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            return InjectReport::err("workbuddy", &path, format!("创建目录失败: {e}"));
        }
    }
    backup_once(&path);
    match atomic_write(&path, &after) {
        Ok(()) => InjectReport::ok("workbuddy", &path, "updated"),
        Err(e) => InjectReport::err("workbuddy", &path, format!("写入失败: {e}")),
    }
}

/// 写入/合并规则文件。
///
/// 合并策略（优先级从高到低）：
/// 1. 有 `<!-- LRC_RULES_BEGIN/END -->` 标记 → 只替换标记之间，前后用户内容都保留；
/// 2. 有旧版 `## LRC 记忆系统` 标题 → 替换旧 LRC 段（保留前缀）；
/// 3. 没有 LRC 内容 → 追加到文件末尾。
fn write_rules_file(path: &Path, content: &str, tool: &'static str) -> InjectReport {
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            return InjectReport::err(tool, path, format!("创建目录失败: {e}"));
        }
    }

    let existing = match fs::read(path) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => Some(text),
            Err(_) => {
                // 非 UTF-8 文件不动，避免破坏用户内容。
                return InjectReport::err(tool, path, "现有文件不是 UTF-8，未修改");
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return InjectReport::err(tool, path, format!("读取失败: {e}")),
    };

    let (merged, action) = match existing {
        None => (content.to_string(), "created"),
        Some(text) => merge_rules(&text, content),
    };

    if action == "unchanged" {
        return InjectReport::ok(tool, path, action);
    }

    backup_once(path);
    match atomic_write(path, &merged) {
        Ok(()) => InjectReport::ok(tool, path, action),
        Err(e) => InjectReport::err(tool, path, format!("写入失败: {e}")),
    }
}

pub(crate) fn merge_rules(existing: &str, content: &str) -> (String, &'static str) {    if let (Some(begin), Some(end)) = (
        existing.find(BEGIN_MARKER),
        existing.find(END_MARKER),
    ) {
        if end > begin {
            let prefix = &existing[..begin];
            // content 含 frontmatter/header；标记替换只取 BEGIN 之后的规则块，
            // 避免把 header 重复插入（prefix 已经包含它）。
            let block = content
                .find(BEGIN_MARKER)
                .map(|pos| &content[pos..])
                .unwrap_or(content);
            // block 自身以换行结尾；先剥掉标记后的换行，避免每轮多出一个空行。
            let suffix = existing[end + END_MARKER.len()..].trim_start_matches(['\r', '\n']);
            let merged = if suffix.is_empty() {
                format!("{prefix}{block}")
            } else {
                format!("{prefix}{block}\n{suffix}")
            };
            if merged == existing {
                return (merged, "unchanged");
            }
            return (merged, "updated");
        }
    }

    if let Some(pos) = existing.find("## LRC 记忆系统") {
        let start = legacy_lrc_start(existing, pos);
        let prefix = existing[..start].trim_end();
        // 旧文件若已带 frontmatter，prefix 已包含它；新内容要剥掉自己的
        // frontmatter，否则会写两份 YAML 头。
        let mut new_content = content;
        if prefix.starts_with("---") {
            if let Some(fm_end) = content.find("\n---\n") {
                new_content = &content[fm_end + 5..];
            }
        }
        let merged = if prefix.is_empty() {
            new_content.to_string()
        } else {
            format!("{prefix}\n\n{new_content}")
        };
        return (merged, "updated");
    }

    let merged = format!("{}\n\n{}", existing.trim_end(), content);
    (merged, "appended")
}

/// 旧版规则文件的 LRC 段起点：优先找已知标题，找不到就从 `## LRC 记忆系统` 所在行开始。
fn legacy_lrc_start(existing: &str, pos: usize) -> usize {
    let head = &existing[..pos];
    const HEADERS: &[&str] = &[
        "# AI Rules — LRC",
        "# Codex AGENTS.md",
        "# DSH AGENTS.md",
        "# Trae AI Rules",
        "# Cursor AI Rules",
        "# CodeBuddy Rules",
        "# WorkBuddy Rules",
        "# Cline Rules",
        "# Windsurf Rules",
        "# Roo Code Rules",
        "# GitHub Copilot",
        "# Comate Rules",
    ];
    HEADERS
        .iter()
        .filter_map(|marker| head.rfind(marker))
        .min()
        .unwrap_or(pos)
}

fn backup_once(path: &Path) {
    if !path.exists() {
        return;
    }
    let file_name = match path.file_name().and_then(|n| n.to_str()) {
        Some(name) => name,
        None => return,
    };
    let backup = path.with_file_name(format!("{file_name}.lrc-backup"));
    if !backup.exists() {
        let _ = fs::copy(path, &backup);
    }
}

fn atomic_write(path: &Path, content: &str) -> std::io::Result<()> {
    // 临时名带进程号 + 纳秒，避免启动时两个注入任务并发写同一文件互相踩踏。
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = path.with_file_name(format!(
        "{}.lrc-tmp-{}-{nonce}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("lrc-rules"),
        std::process::id()
    ));
    fs::write(&tmp, content)?;
    if path.exists() {
        // Windows 上 rename 不能覆盖已存在文件，先删除再改名。
        let _ = fs::remove_file(path);
    }
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lrc-universal-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).expect("创建临时目录");
        dir
    }

    #[test]
    fn inject_rules_creates_agents_file_and_is_idempotent() {
        let home = temp_home("agents");
        let first = inject_universal_rules(&home);
        let agents = home.join(".agents").join("AGENTS.md");
        assert!(agents.exists());
        assert!(fs::read_to_string(&agents)
            .expect("读取")
            .contains(BEGIN_MARKER));
        assert!(first.iter().any(|r| r.tool == "agents" && r.action == "created"));

        let second = inject_universal_rules(&home);
        assert!(second
            .iter()
            .any(|r| r.tool == "agents" && r.action == "unchanged"));
    }

    #[test]
    fn inject_rules_preserves_user_content_around_block() {
        let home = temp_home("preserve");
        let agents = home.join(".agents").join("AGENTS.md");
        fs::create_dir_all(agents.parent().unwrap()).unwrap();
        fs::write(
            &agents,
            "# 用户自己的规则\n\n<!-- LRC_RULES_BEGIN -->\n旧内容\n<!-- LRC_RULES_END -->\n\n## 用户后置规则\n",
        )
        .unwrap();

        inject_universal_rules(&home);
        let text = fs::read_to_string(&agents).unwrap();
        assert!(text.starts_with("# 用户自己的规则"));
        assert!(text.contains("## 用户后置规则"));
        assert!(text.contains("规则 6：无钩子宿主必须主动调用"));
    }

    #[test]
    fn codex_config_adds_mcp_and_notify_without_overwriting() {
        let home = temp_home("codex");
        let dir = home.join(".codex");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        fs::write(
            &path,
            "model = \"gpt-5\"\n\n[notice]\nhide = true\n",
        )
        .unwrap();

        let report = ensure_codex_config(&home, 3111, Some("C:/lrc/lrc.exe"));
        assert_eq!(report.action, "updated", "{report:?}");
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("model = \"gpt-5\""));
        assert!(text.contains("[mcp_servers.lrc-memory]"));
        assert!(text.contains("url = \"http://127.0.0.1:3111/mcp\""));
        assert!(text.contains("notify = ["));

        let again = ensure_codex_config(&home, 3111, Some("C:/lrc/lrc.exe"));
        assert_eq!(again.action, "unchanged", "{again:?}");
    }

    #[test]
    fn codex_config_keeps_user_notify() {
        let home = temp_home("codex-notify");
        let dir = home.join(".codex");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        fs::write(&path, "notify = [\"my-notifier\"]\n").unwrap();

        ensure_codex_config(&home, 3099, None);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("notify = [\"my-notifier\"]"));
        assert!(text.contains("[mcp_servers.lrc-memory]"));
    }

    #[test]
    fn codex_config_refuses_invalid_toml() {
        let home = temp_home("codex-bad");
        let dir = home.join(".codex");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let broken = "model = [unclosed\n";
        fs::write(&path, broken).unwrap();

        let report = ensure_codex_config(&home, 3099, None);
        assert_eq!(report.action, "error");
        assert_eq!(fs::read_to_string(&path).unwrap(), broken);
    }

    #[test]
    fn workbuddy_config_merges_without_dropping_other_servers() {
        let home = temp_home("workbuddy");
        let dir = home.join(".workbuddy");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mcp.json");
        fs::write(
            &path,
            r#"{"mcpServers":{"wecom":{"command":"uvx","args":["wecom"]}}}"#,
        )
        .unwrap();

        let report = ensure_workbuddy_config(&home, 3099);
        assert_eq!(report.action, "updated", "{report:?}");
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(value["mcpServers"]["wecom"].is_object());
        assert_eq!(
            value["mcpServers"]["lrc-memory"]["url"],
            "http://127.0.0.1:3099/mcp"
        );
        assert_eq!(
            value["mcpServers"]["lrc-memory"]["type"],
            "streamableHttp"
        );

        let again = ensure_workbuddy_config(&home, 3099);
        assert_eq!(again.action, "unchanged", "{again:?}");
    }

    #[test]
    fn merge_rules_keeps_single_frontmatter() {
        let existing = "---\ndescription: 用户规则\n---\n\n# CodeBuddy Rules — LRC 记忆系统自动配置\n\n## LRC 记忆系统\n旧内容\n";
        let content = AgentDetectorRegistry::generate_ai_rules_content("codebuddy");
        let (merged, action) = merge_rules(existing, &content);
        assert_eq!(action, "updated");
        assert_eq!(
            merged.matches("---\ndescription").count(),
            1,
            "不能写两份 frontmatter：\n{merged}"
        );
        assert_eq!(merged.matches("## LRC 记忆系统").count(), 1);
    }

    #[test]
    fn inject_rules_skips_non_utf8_file() {
        let home = temp_home("non-utf8");
        let agents = home.join(".agents").join("AGENTS.md");
        fs::create_dir_all(agents.parent().unwrap()).unwrap();
        let raw = vec![0xff, 0xfe, 0x00, 0x41];
        fs::write(&agents, &raw).unwrap();

        let reports = inject_universal_rules(&home);
        assert!(
            reports
                .iter()
                .any(|r| r.tool == "agents" && r.action == "error"),
            "非 UTF-8 文件必须报错跳过: {reports:?}"
        );
        assert_eq!(fs::read(&agents).unwrap(), raw, "不得覆盖非 UTF-8 文件");
    }

    #[test]
    fn workbuddy_rules_only_when_workbuddy_exists() {
        // 仅装 CodeBuddy：不应写 WorkBuddy 规则
        let codebuddy_home = temp_home("codebuddy-only");
        fs::create_dir_all(codebuddy_home.join(".codebuddy")).unwrap();
        inject_universal_rules(&codebuddy_home);
        assert!(!codebuddy_home
            .join(".codebuddy")
            .join("rules")
            .join("lrc-memory.md")
            .exists());

        // 只装 WorkBuddy：应写规则（含 CodeBuddy 兼容 frontmatter）
        let workbuddy_home = temp_home("workbuddy-only");
        fs::create_dir_all(workbuddy_home.join(".workbuddy")).unwrap();
        inject_universal_rules(&workbuddy_home);
        let rules = workbuddy_home
            .join(".codebuddy")
            .join("rules")
            .join("lrc-memory.md");
        assert!(rules.exists());
        let text = fs::read_to_string(&rules).unwrap();
        assert!(text.contains("alwaysApply: true"));
        assert!(text.contains("# WorkBuddy Rules"));
    }

    #[test]
    fn workbuddy_config_skips_when_not_installed() {
        let home = temp_home("workbuddy-skip");
        let report = ensure_workbuddy_config(&home, 3099);
        assert_eq!(report.action, "skipped");
        assert!(!home.join(".workbuddy").join("mcp.json").exists());
    }
}
