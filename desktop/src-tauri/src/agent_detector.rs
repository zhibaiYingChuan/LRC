/// Agent 自动检测与配置模块
///
/// 检测系统上安装的 AI Agent（IDE + 独立桌面应用 + CLI 工具），
/// 并自动生成 MCP 配置文件。
///
/// 设计原则：
///   - 数据驱动：通过 KnownTool 数据库定义工具元数据，而非为每个工具写 struct
///   - 动态发现：扫描 ~/.* 目录自动发现未知 AI 工具
///   - 多层检测：目录检测 → 注册表检测 → PATH 检测
///
/// 支持类型：
///   - IDE 内嵌 Agent：Trae, Cursor, VS Code, Windsurf, Kiro
///   - 独立桌面 Agent：Claude Desktop, Gemini CLI, Codex CLI
///   - AI 编码助手：CodeBuddy, Comate, Roo, Cline, Continue, Cody, Aider, Augment, Amazon Q
///   - AI 浏览器/平台：Agent Browser, CloudBase MCP, Playwright MCP, OpenCode
///   - 其他 AI 工具：Z-Brain, Functional Hub, Tabby, PearAI, Zed, JetBrains AI
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Agent 信息（返回给前端展示）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: String,          // 唯一标识符（如 "trae", "cursor"）
    pub name: String,        // 显示名称（如 "Trae"）
    pub installed: bool,     // 是否已安装
    pub config_path: Option<String>,  // MCP 配置文件路径
    pub icon: String,        // 图标（emoji）
    pub category: String,    // 分类："ide" / "desktop" / "cli" / "ai-assistant" / "browser" / "custom"
    pub supports_mcp: bool,  // 是否支持 MCP 协议（可自动配置）
}

/// Agent 检测器 Trait（契约优先）
trait AgentDetector {
    /// 检测该 Agent 是否已安装
    fn detect(&self) -> bool;

    /// 获取 Agent 的 MCP 配置文件路径（全局配置）
    fn config_path(&self) -> Option<PathBuf>;

    /// 扫描该 IDE 已配置的项目列表
    fn scan_projects(&self) -> Vec<ProjectInfo> {
        self.scan_project_dirs()
    }

    /// 扫描目录，查找包含 IDE 配置文件夹的项目
    fn scan_project_dirs(&self) -> Vec<ProjectInfo>;

    /// 生成 MCP 配置 JSON 内容
    fn generate_config(&self, port: u16) -> serde_json::Value;

    /// Agent 基本信息
    fn info(&self) -> AgentInfo;
}

/// IDE 项目信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub path: String,
    pub name: String,
    pub ide_id: String,
    pub ide_name: String,
}

// ════════════════════════════════════════════════════════════════
// 已知 AI 工具定义（数据驱动的工具数据库）
// ════════════════════════════════════════════════════════════════

/// 已知 AI 工具的元数据定义
struct KnownTool {
    /// 唯一标识符
    id: &'static str,
    /// 显示名称
    name: &'static str,
    /// 图标（emoji）
    icon: &'static str,
    /// 分类
    category: &'static str,
    /// 是否支持 MCP 协议
    supports_mcp: bool,
    /// 检测标记：主目录（相对于 %USERPROFILE% 或 %APPDATA%）
    primary_marker: &'static str,
    /// 辅助检测标记（可选）
    secondary_markers: &'static [&'static str],
    /// MCP 配置路径模板（相对于 %USERPROFILE%，None 表示项目级配置或无 MCP）
    mcp_config_template: Option<&'static str>,
    /// MCP 传输类型："stdio" 或 "http"
    mcp_transport: &'static str,
}

/// 所有已知 AI 工具的数据库
///
/// 添加新工具只需在此数组中添加一条记录，无需创建新的 struct。
/// 检测逻辑由 DotDirDetector 统一处理。
const KNOWN_TOOLS: &[KnownTool] = &[
    // ═══ IDE 类（支持 MCP） ═══
    KnownTool {
        id: "trae",
        name: "Trae",
        icon: "🖥️",
        category: "ide",
        supports_mcp: true,
        primary_marker: ".trae",
        secondary_markers: &[".trae-cn", "%APPDATA%/Trae", "%APPDATA%/Trae CN"],
        mcp_config_template: Some(".trae/mcp.json"),
        mcp_transport: "stdio",
    },
    KnownTool {
        id: "trae-cn",
        name: "Trae CN",
        icon: "🖥️",
        category: "ide",
        supports_mcp: true,
        primary_marker: ".trae-cn",
        secondary_markers: &["%APPDATA%/Trae CN"],
        mcp_config_template: Some(".trae-cn/trae-mcp.json"),
        mcp_transport: "stdio",
    },
    KnownTool {
        id: "cursor",
        name: "Cursor",
        icon: "🖱️",
        category: "ide",
        supports_mcp: true,
        primary_marker: ".cursor",
        secondary_markers: &["%APPDATA%/Cursor"],
        mcp_config_template: None, // 项目级 .cursor/mcp.json
        mcp_transport: "stdio",
    },
    KnownTool {
        id: "vscode",
        name: "VS Code",
        icon: "📝",
        category: "ide",
        supports_mcp: true,
        primary_marker: ".vscode",
        secondary_markers: &["%APPDATA%/Code", "%APPDATA%/Code - Insiders"],
        mcp_config_template: None, // 项目级 .vscode/mcp.json
        mcp_transport: "stdio",
    },
    KnownTool {
        id: "windsurf",
        name: "Windsurf",
        icon: "🌊",
        category: "ide",
        supports_mcp: true,
        primary_marker: ".windsurf",
        secondary_markers: &["%APPDATA%/Windsurf"],
        mcp_config_template: Some("%APPDATA%/Windsurf/User/globalStorage/mcp.json"),
        mcp_transport: "stdio",
    },
    KnownTool {
        id: "kiro",
        name: "Kiro",
        icon: "🔮",
        category: "ide",
        supports_mcp: true,
        primary_marker: ".kiro",
        secondary_markers: &[],
        mcp_config_template: Some(".kiro/settings/mcp.json"),
        mcp_transport: "stdio",
    },

    // ═══ 桌面应用类 ═══
    KnownTool {
        id: "claude-desktop",
        name: "Claude Desktop",
        icon: "🧠",
        category: "desktop",
        supports_mcp: true,
        primary_marker: ".claude",
        secondary_markers: &["%APPDATA%/Claude", "%LOCALAPPDATA%/AnthropicClaude"],
        mcp_config_template: Some("%APPDATA%/Claude/claude_desktop_config.json"),
        mcp_transport: "http",
    },
    KnownTool {
        id: "gemini-cli",
        name: "Gemini CLI",
        icon: "💎",
        category: "cli",
        supports_mcp: true,
        primary_marker: ".gemini",
        secondary_markers: &[],
        mcp_config_template: Some(".gemini/settings.json"),
        mcp_transport: "stdio",
    },
    KnownTool {
        id: "codex-cli",
        name: "OpenAI Codex CLI",
        icon: "🤖",
        category: "cli",
        supports_mcp: false,
        primary_marker: ".codex",
        secondary_markers: &[],
        mcp_config_template: None,
        mcp_transport: "stdio",
    },

    // ═══ AI 编码助手类 ═══
    KnownTool {
        id: "codebuddy",
        name: "CodeBuddy",
        icon: "🤝",
        category: "ai-assistant",
        supports_mcp: false,
        primary_marker: ".codebuddy",
        secondary_markers: &[],
        mcp_config_template: None,
        mcp_transport: "stdio",
    },
    KnownTool {
        id: "comate",
        name: "Comate (百度)",
        icon: "🐻",
        category: "ai-assistant",
        supports_mcp: true,
        primary_marker: ".comate",
        secondary_markers: &[],
        mcp_config_template: Some(".comate/mcp.json"),
        mcp_transport: "stdio",
    },
    KnownTool {
        id: "roo-code",
        name: "Roo Code",
        icon: "🦘",
        category: "ai-assistant",
        supports_mcp: true,
        primary_marker: ".roo",
        secondary_markers: &[],
        mcp_config_template: Some(".roo/mcp.json"),
        mcp_transport: "stdio",
    },
    KnownTool {
        id: "cline",
        name: "Cline",
        icon: "🧗",
        category: "ai-assistant",
        supports_mcp: true,
        primary_marker: ".cline",
        secondary_markers: &[],
        mcp_config_template: Some(".cline/mcp.json"),
        mcp_transport: "stdio",
    },
    KnownTool {
        id: "continue",
        name: "Continue.dev",
        icon: "🔄",
        category: "ai-assistant",
        supports_mcp: false,
        primary_marker: ".continue",
        secondary_markers: &[],
        mcp_config_template: None,
        mcp_transport: "stdio",
    },
    KnownTool {
        id: "cody",
        name: "Cody (Sourcegraph)",
        icon: "🦊",
        category: "ai-assistant",
        supports_mcp: false,
        primary_marker: ".cody",
        secondary_markers: &[],
        mcp_config_template: None,
        mcp_transport: "stdio",
    },
    KnownTool {
        id: "aider",
        name: "Aider",
        icon: "💬",
        category: "cli",
        supports_mcp: false,
        primary_marker: ".aider",
        secondary_markers: &[],
        mcp_config_template: None,
        mcp_transport: "stdio",
    },
    KnownTool {
        id: "augment",
        name: "Augment Code",
        icon: "⚡",
        category: "ai-assistant",
        supports_mcp: false,
        primary_marker: ".augment",
        secondary_markers: &[],
        mcp_config_template: None,
        mcp_transport: "stdio",
    },
    KnownTool {
        id: "amazon-q",
        name: "Amazon Q Developer",
        icon: "☁️",
        category: "ai-assistant",
        supports_mcp: false,
        primary_marker: ".aws",
        secondary_markers: &[],
        mcp_config_template: None,
        mcp_transport: "stdio",
    },
    KnownTool {
        id: "tabby",
        name: "Tabby",
        icon: "🐱",
        category: "ai-assistant",
        supports_mcp: false,
        primary_marker: ".tabby",
        secondary_markers: &[],
        mcp_config_template: None,
        mcp_transport: "stdio",
    },
    KnownTool {
        id: "jetbrains-ai",
        name: "JetBrains AI",
        icon: "🧩",
        category: "ide",
        supports_mcp: false,
        primary_marker: "%APPDATA%/JetBrains",
        secondary_markers: &[],
        mcp_config_template: None,
        mcp_transport: "stdio",
    },
    KnownTool {
        id: "pearai",
        name: "PearAI",
        icon: "🍐",
        category: "ai-assistant",
        supports_mcp: false,
        primary_marker: ".pearai",
        secondary_markers: &[],
        mcp_config_template: None,
        mcp_transport: "stdio",
    },
    KnownTool {
        id: "zed",
        name: "Zed",
        icon: "⚡",
        category: "ide",
        supports_mcp: false,
        primary_marker: ".zed",
        secondary_markers: &[],
        mcp_config_template: None,
        mcp_transport: "stdio",
    },

    // ═══ AI 浏览器 / 平台类 ═══
    KnownTool {
        id: "agent-browser",
        name: "Agent Browser",
        icon: "🌐",
        category: "browser",
        supports_mcp: false,
        primary_marker: ".agent-browser",
        secondary_markers: &[],
        mcp_config_template: None,
        mcp_transport: "http",
    },
    KnownTool {
        id: "cloudbase-mcp",
        name: "CloudBase MCP",
        icon: "🏗️",
        category: "browser",
        supports_mcp: true,
        primary_marker: ".cloudbase-mcp",
        secondary_markers: &[],
        mcp_config_template: Some(".cloudbase-mcp/mcp.json"),
        mcp_transport: "http",
    },
    KnownTool {
        id: "playwright-mcp",
        name: "Playwright MCP",
        icon: "🎭",
        category: "browser",
        supports_mcp: true,
        primary_marker: ".playwright-mcp",
        secondary_markers: &[],
        mcp_config_template: Some(".playwright-mcp/mcp.json"),
        mcp_transport: "http",
    },
    KnownTool {
        id: "opencode",
        name: "OpenCode",
        icon: "🔓",
        category: "ai-assistant",
        supports_mcp: false,
        primary_marker: ".opencode",
        secondary_markers: &[],
        mcp_config_template: None,
        mcp_transport: "stdio",
    },

    // ═══ 其他 AI 工具 ═══
    KnownTool {
        id: "z-brain",
        name: "Z-Brain",
        icon: "🧬",
        category: "desktop",
        supports_mcp: false,
        primary_marker: "%APPDATA%/Z-Brain",
        secondary_markers: &[],
        mcp_config_template: None,
        mcp_transport: "http",
    },
    KnownTool {
        id: "functional-hub",
        name: "Functional Hub Agent",
        icon: "🔧",
        category: "desktop",
        supports_mcp: false,
        primary_marker: "%APPDATA%/functional-hub-agent",
        secondary_markers: &["%APPDATA%/com.functional-hub.agent"],
        mcp_config_template: None,
        mcp_transport: "http",
    },
    KnownTool {
        id: "sillytavern",
        name: "酒馆 (SillyTavern)",
        icon: "🏮",
        category: "desktop",
        supports_mcp: false,
        primary_marker: "SillyTavern",
        secondary_markers: &["Documents/SillyTavern"],
        mcp_config_template: None,
        mcp_transport: "http",
    },
    KnownTool {
        id: "memorix",
        name: "Memorix",
        icon: "🧿",
        category: "desktop",
        supports_mcp: false,
        primary_marker: ".memorix",
        secondary_markers: &[],
        mcp_config_template: None,
        mcp_transport: "http",
    },
    KnownTool {
        id: "loong-recall",
        name: "Loong Recall (LRC)",
        icon: "🐉",
        category: "desktop",
        supports_mcp: true,
        primary_marker: ".loong-recall",
        secondary_markers: &[],
        mcp_config_template: Some(".loong-recall/mcp.json"),
        mcp_transport: "stdio",
    },
];

// ── 通用扫描函数 ──

/// 扫描指定根目录下包含 marker 子目录的项目
fn scan_marker_projects(
    roots: &[PathBuf],
    marker: &str,
    ide_id: &str,
    ide_name: &str,
) -> Vec<ProjectInfo> {
    let mut projects = Vec::new();
    for root in roots {
        if !root.exists() {
            continue;
        }
        if root.join(marker).exists() {
            projects.push(ProjectInfo {
                name: root
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| root.to_string_lossy().to_string()),
                path: root.to_string_lossy().to_string(),
                ide_id: ide_id.to_string(),
                ide_name: ide_name.to_string(),
            });
        }
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && path.join(marker).exists() {
                    projects.push(ProjectInfo {
                        name: path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default(),
                        path: path.to_string_lossy().to_string(),
                        ide_id: ide_id.to_string(),
                        ide_name: ide_name.to_string(),
                    });
                }
            }
        }
    }
    projects.sort_by(|a, b| a.path.cmp(&b.path));
    projects.dedup_by(|a, b| a.path == b.path);
    projects
}

/// 获取项目扫描的根目录列表
fn scan_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(home) = std::env::var("USERPROFILE") {
        let home = PathBuf::from(&home);
        for sub in &[
            "source\\repos",
            "Code",
            "Projects",
            "Documents\\GitHub",
            "Documents\\Projects",
            "Desktop",
            "Documents",
            "",
        ] {
            let p = home.join(sub);
            if p.exists() {
                roots.push(p);
            }
        }
    }
    // P3-09 修复：添加 C:\ 到扫描范围
    for drive in &["C:\\", "D:\\", "E:\\", "F:\\", "G:\\"] {
        let p = PathBuf::from(drive);
        if p.exists() {
            roots.push(p);
        }
    }
    roots
}

/// 获取用户主目录
fn home_dir() -> Option<PathBuf> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()
        .map(PathBuf::from)
}

/// 获取 AppData 目录
fn appdata_dir() -> Option<PathBuf> {
    std::env::var("APPDATA").ok().map(PathBuf::from)
}

/// 获取 LocalAppData 目录
fn local_appdata_dir() -> Option<PathBuf> {
    std::env::var("LOCALAPPDATA").ok().map(PathBuf::from)
}

/// 解析标记路径中的变量
/// - 以 "%APPDATA%/" 开头的路径替换为实际 APPDATA 路径
/// - 以 "%LOCALAPPDATA%/" 开头的路径替换为实际 LOCALAPPDATA 路径
/// - 其他路径相对于 %USERPROFILE%
fn resolve_marker(marker: &str) -> Option<PathBuf> {
    if let Some(rest) = marker.strip_prefix("%APPDATA%/") {
        return appdata_dir().map(|d| d.join(rest));
    }
    if let Some(rest) = marker.strip_prefix("%LOCALAPPDATA%/") {
        return local_appdata_dir().map(|d| d.join(rest));
    }
    // 相对于 USERPROFILE
    home_dir().map(|d| d.join(marker))
}

/// 检查标记路径是否存在
fn marker_exists(marker: &str) -> bool {
    resolve_marker(marker).is_some_and(|p| p.exists())
}

// ════════════════════════════════════════════════════════════════
// 通用 DotDir 检测器（数据驱动，替代每个工具单独写 struct）
// ════════════════════════════════════════════════════════════════

/// 基于 KnownTool 定义的通用检测器
struct DotDirDetector {
    tool: &'static KnownTool,
}

impl DotDirDetector {
    fn new(tool: &'static KnownTool) -> Self {
        Self { tool }
    }

    /// 使用已知工具数据库中的信息进行检测
    fn check_known_tool(&self) -> bool {
        // 检查主标记
        if marker_exists(self.tool.primary_marker) {
            return true;
        }
        // 检查辅助标记
        for marker in self.tool.secondary_markers {
            if marker_exists(marker) {
                return true;
            }
        }
        false
    }
}

impl AgentDetector for DotDirDetector {
    fn detect(&self) -> bool {
        self.check_known_tool()
    }

    fn config_path(&self) -> Option<PathBuf> {
        self.tool
            .mcp_config_template
            .and_then(|template| resolve_marker(template))
            .filter(|p| {
                // 返回配置路径，即使文件不存在也返回（父目录存在即可）
                p.exists() || p.parent().is_some_and(|parent| parent.exists())
            })
    }

    fn generate_config(&self, port: u16) -> serde_json::Value {
        if self.tool.mcp_transport == "http" {
            serde_json::json!({
                "mcpServers": {
                    "lrc-memory": {
                        "type": "http",
                        "url": format!("http://127.0.0.1:{}/mcp", port),
                        "description": "LRC — 本地代码记忆与语义搜索"
                    }
                }
            })
        } else {
            serde_json::json!({
                "mcpServers": {
                    "lrc-memory": {
                        "command": "lrc-desktop",
                        "args": ["--mcp", "--port", port.to_string()],
                        "env": {}
                    }
                }
            })
        }
    }

    fn info(&self) -> AgentInfo {
        AgentInfo {
            id: self.tool.id.to_string(),
            name: self.tool.name.to_string(),
            installed: self.detect(),
            config_path: self.config_path().map(|p| p.display().to_string()),
            icon: self.tool.icon.to_string(),
            category: self.tool.category.to_string(),
            supports_mcp: self.tool.supports_mcp,
        }
    }

    fn scan_project_dirs(&self) -> Vec<ProjectInfo> {
        // 只有 IDE 类工具需要扫描项目
        if self.tool.category == "ide" {
            let marker = if self.tool.primary_marker.starts_with('.') {
                &self.tool.primary_marker
            } else {
                return Vec::new();
            };
            scan_marker_projects(&scan_roots(), marker, self.tool.id, self.tool.name)
        } else {
            Vec::new()
        }
    }
}

// ════════════════════════════════════════════════════════════════
// 特殊检测器（需要复杂逻辑的工具）
// ════════════════════════════════════════════════════════════════

/// Trae 专用检测器（多策略检测，支持 Trae CN 特殊路径）
struct TraeDetector;

impl AgentDetector for TraeDetector {
    fn detect(&self) -> bool {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_default();
        let home_path = PathBuf::from(&home);

        // 策略 0：检查 ~/.trae-cn 和 ~/.trae
        if home_path.join(".trae-cn").exists() || home_path.join(".trae").exists() {
            return true;
        }

        // 策略 1：检查 %APPDATA%\Trae 或 %APPDATA%\Trae CN
        let appdata = std::env::var("APPDATA").unwrap_or_default();
        let appdata_path = PathBuf::from(&appdata);
        if appdata_path.join("Trae").exists() || appdata_path.join("Trae CN").exists() {
            return true;
        }

        // 策略 2：检查安装目录
        let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
        if PathBuf::from(&local).join("Programs").join("Trae").exists() {
            return true;
        }

        // 策略 3：检查 Program Files
        for pf in &["C:\\Program Files\\Trae", "C:\\Program Files (x86)\\Trae"] {
            if std::path::Path::new(pf).exists() {
                return true;
            }
        }

        // 策略 4：注册表检测
        #[cfg(target_os = "windows")]
        {
            use std::process::Command;
            for hive in &["HKCU", "HKLM"] {
                let key = if *hive == "HKCU" {
                    r"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Uninstall"
                } else {
                    r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"
                };
                let output = Command::new("reg")
                    .args(["query", key, "/f", "Trae", "/k", "/e"])
                    .output();
                if let Ok(out) = output {
                    if out.status.success() && !out.stdout.is_empty() {
                        return true;
                    }
                }
            }
        }

        false
    }

    fn config_path(&self) -> Option<PathBuf> {
        let home = home_dir()?;

        // Trae CN 优先
        let trae_cn_config = home.join(".trae-cn").join("trae-mcp.json");
        let trae_config = home.join(".trae").join("mcp.json");

        if trae_cn_config.exists() {
            return Some(trae_cn_config);
        }
        if trae_config.exists() {
            return Some(trae_config);
        }

        // 从 AppData 推断
        let appdata = appdata_dir()?;
        if appdata.join("Trae CN").exists() {
            return Some(trae_cn_config);
        }
        if appdata.join("Trae").exists() {
            return Some(trae_config);
        }

        None
    }

    fn generate_config(&self, port: u16) -> serde_json::Value {
        serde_json::json!({
            "mcpServers": {
                "lrc-memory": {
                    "command": "lrc-desktop",
                    "args": ["--mcp", "--port", port.to_string()],
                    "env": {}
                }
            }
        })
    }

    fn info(&self) -> AgentInfo {
        AgentInfo {
            id: "trae".into(),
            name: "Trae".into(),
            installed: self.detect(),
            config_path: self.config_path().map(|p| p.display().to_string()),
            icon: "🖥️".into(),
            category: "ide".into(),
            supports_mcp: true,
        }
    }

    fn scan_project_dirs(&self) -> Vec<ProjectInfo> {
        scan_marker_projects(&scan_roots(), ".trae", "trae", "Trae")
    }
}

/// Trae CN 专用检测器
struct TraeCNDetector;

impl AgentDetector for TraeCNDetector {
    fn detect(&self) -> bool {
        let home = home_dir().unwrap_or_default();
        home.join(".trae-cn").exists()
            || appdata_dir().is_some_and(|d| d.join("Trae CN").exists())
    }

    fn config_path(&self) -> Option<PathBuf> {
        let home = home_dir()?;
        let config = home.join(".trae-cn").join("trae-mcp.json");
        if config.exists() {
            return Some(config);
        }
        // 也返回 AppData 下的配置路径
        let appdata = appdata_dir()?;
        let appdata_config = appdata.join("Trae CN").join("User").join("mcp.json");
        if appdata_config.exists() {
            return Some(appdata_config);
        }
        Some(config) // 返回默认路径，即使不存在也用于创建
    }

    fn generate_config(&self, port: u16) -> serde_json::Value {
        serde_json::json!({
            "mcpServers": {
                "lrc-memory": {
                    "command": "lrc-desktop",
                    "args": ["--mcp", "--port", port.to_string()],
                    "env": {}
                }
            }
        })
    }

    fn info(&self) -> AgentInfo {
        AgentInfo {
            id: "trae-cn".into(),
            name: "Trae CN".into(),
            installed: self.detect(),
            config_path: self.config_path().map(|p| p.display().to_string()),
            icon: "🖥️".into(),
            category: "ide".into(),
            supports_mcp: true,
        }
    }

    fn scan_project_dirs(&self) -> Vec<ProjectInfo> {
        scan_marker_projects(&scan_roots(), ".trae", "trae-cn", "Trae CN")
    }
}

/// Claude Desktop 专用检测器（需要多策略验证防止误报）
struct ClaudeDesktopDetector;

impl AgentDetector for ClaudeDesktopDetector {
    fn detect(&self) -> bool {
        #[cfg(target_os = "windows")]
        {
            let local_appdata = std::env::var("LOCALAPPDATA").unwrap_or_default();
            let install_exe = PathBuf::from(&local_appdata)
                .join("AnthropicClaude")
                .join("claude.exe");
            if install_exe.exists() {
                return true;
            }

            use std::process::Command;
            let output = Command::new("reg")
                .args([
                    "query",
                    r"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Uninstall",
                    "/f",
                    "AnthropicClaude",
                    "/k",
                    "/e",
                ])
                .output();
            if output.is_ok_and(|out| out.status.success() && !out.stdout.is_empty()) {
                return true;
            }
        }

        #[cfg(target_os = "macos")]
        {
            let home = std::env::var("HOME").unwrap_or_default();
            if PathBuf::from(&home)
                .join("Library/Application Support/Claude/claude_desktop_config.json")
                .exists()
                || PathBuf::from("/Applications/Claude.app").exists()
            {
                return true;
            }
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            let home = std::env::var("HOME").unwrap_or_default();
            if PathBuf::from(&home)
                .join(".config/Claude/claude_desktop_config.json")
                .exists()
            {
                return true;
            }
        }

        false
    }

    fn config_path(&self) -> Option<PathBuf> {
        #[cfg(target_os = "windows")]
        {
            let appdata = appdata_dir()?;
            let config = appdata.join("Claude").join("claude_desktop_config.json");
            if config.exists() || config.parent().is_some_and(|p| p.exists()) {
                return Some(config);
            }
        }

        #[cfg(target_os = "macos")]
        {
            let home = home_dir()?;
            return Some(
                home.join("Library/Application Support/Claude")
                    .join("claude_desktop_config.json"),
            );
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            let home = home_dir()?;
            return Some(home.join(".config/Claude/claude_desktop_config.json"));
        }

        None
    }

    fn generate_config(&self, port: u16) -> serde_json::Value {
        serde_json::json!({
            "mcpServers": {
                "lrc-memory": {
                    "type": "http",
                    "url": format!("http://127.0.0.1:{}/mcp", port),
                    "description": "LRC — 本地代码记忆与语义搜索"
                }
            }
        })
    }

    fn info(&self) -> AgentInfo {
        AgentInfo {
            id: "claude-desktop".into(),
            name: "Claude Desktop".into(),
            installed: self.detect(),
            config_path: self.config_path().map(|p| p.display().to_string()),
            icon: "🧠".into(),
            category: "desktop".into(),
            supports_mcp: true,
        }
    }

    fn scan_project_dirs(&self) -> Vec<ProjectInfo> {
        Vec::new()
    }
}

/// 通用 MCP Agent（提供 HTTP 端点连接信息，不自动写入配置）
struct GenericMcpAgent;

impl AgentDetector for GenericMcpAgent {
    fn detect(&self) -> bool {
        true
    }

    fn config_path(&self) -> Option<PathBuf> {
        None
    }

    fn generate_config(&self, port: u16) -> serde_json::Value {
        serde_json::json!({
            "mcpServers": {
                "lrc-memory": {
                    "type": "http",
                    "url": format!("http://127.0.0.1:{}/mcp", port),
                    "description": "LRC — 本地代码记忆与语义搜索。将此配置添加到你的 Agent 的 MCP 配置文件中。"
                }
            }
        })
    }

    fn info(&self) -> AgentInfo {
        AgentInfo {
            id: "generic-mcp".into(),
            name: "通用 MCP Agent".into(),
            installed: true,
            config_path: None,
            icon: "🔌".into(),
            category: "custom".into(),
            supports_mcp: false,
        }
    }

    fn scan_project_dirs(&self) -> Vec<ProjectInfo> {
        Vec::new()
    }
}

// ════════════════════════════════════════════════════════════════
// Agent 检测器注册表
// ════════════════════════════════════════════════════════════════

pub struct AgentDetectorRegistry {
    detectors: Vec<Box<dyn AgentDetector + Send + Sync>>,
}

impl AgentDetectorRegistry {
    /// 创建包含所有支持 Agent 的注册表
    ///
    /// 包含：
    ///   - 特殊检测器（Trae, Trae CN, Claude Desktop, GenericMCP）
    ///   - 数据驱动的 DotDirDetector（基于 KNOWN_TOOLS 数据库）
    pub fn new() -> Self {
        let mut detectors: Vec<Box<dyn AgentDetector + Send + Sync>> = vec![
            // 特殊检测器（需要复杂逻辑的工具）
            Box::new(TraeDetector),
            Box::new(TraeCNDetector),
            Box::new(ClaudeDesktopDetector),
            Box::new(GenericMcpAgent),
        ];

        // 数据驱动的通用检测器（基于 KNOWN_TOOLS 数据库）
        // 排除已用特殊检测器覆盖的工具
        let special_ids = ["trae", "trae-cn", "claude-desktop", "generic-mcp"];
        for tool in KNOWN_TOOLS {
            if !special_ids.contains(&tool.id) {
                detectors.push(Box::new(DotDirDetector::new(tool)));
            }
        }

        Self { detectors }
    }

    /// 检测所有已安装的 Agent，返回信息列表
    pub fn detect_all(&self) -> Vec<AgentInfo> {
        self.detectors.iter().map(|d| d.info()).collect()
    }

    /// 仅返回已安装的 Agent（过滤掉未安装的）
    pub fn detect_installed(&self) -> Vec<AgentInfo> {
        self.detectors
            .iter()
            .map(|d| d.info())
            .filter(|info| info.installed)
            .collect()
    }

    /// 动态扫描：发现所有已知工具 + 未知的 dot 目录
    ///
    /// 返回 (已知工具列表, 未知工具列表)
    pub fn discover_all(&self) -> (Vec<AgentInfo>, Vec<AgentInfo>) {
        let known = self.detect_all();
        let unknown = self.discover_unknown_tools();
        (known, unknown)
    }

    /// 扫描用户目录下未知的 AI 工具（不在已知数据库中的 dot 目录）
    fn discover_unknown_tools(&self) -> Vec<AgentInfo> {
        let mut unknown = Vec::new();
        let home = match home_dir() {
            Some(h) => h,
            None => return unknown,
        };

        // 已知工具的 marker 集合（用于跳过已知目录）
        let known_markers: std::collections::HashSet<&str> =
            KNOWN_TOOLS.iter().map(|t| t.primary_marker).collect();

        // 扫描 ~/.* 目录
        if let Ok(entries) = std::fs::read_dir(&home) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                // 只处理以 . 开头的目录
                if !name.starts_with('.') {
                    continue;
                }
                // 跳过已知工具
                if known_markers.contains(name) {
                    continue;
                }
                // 跳过系统目录
                if is_system_dir(name) {
                    continue;
                }

                // 检查是否为 AI 工具（包含 mcp.json 或 settings.json 等配置）
                let has_mcp = path.join("mcp.json").exists()
                    || path.join("settings").join("mcp.json").exists()
                    || path.join("settings.json").exists();
                let has_config = path.join("config.json").exists()
                    || path.join("config.yaml").exists()
                    || path.join("config.yml").exists();

                if has_mcp || has_config {
                    unknown.push(AgentInfo {
                        id: format!("unknown-{}", &name[1..]), // 去掉开头的 .
                        name: format!("未知工具 ({})", &name[1..]),
                        installed: true,
                        config_path: if has_mcp {
                            Some(path.join("mcp.json").display().to_string())
                        } else {
                            None
                        },
                        icon: "❓".into(),
                        category: "custom".into(),
                        supports_mcp: has_mcp,
                    });
                }
            }
        }

        unknown
    }

    /// 扫描已安装 IDE 的项目列表
    pub fn scan_ide_projects(&self, ide_ids: &[String]) -> Vec<ProjectInfo> {
        let mut projects = Vec::new();
        for detector in &self.detectors {
            let info = detector.info();
            if ide_ids.contains(&info.id) {
                projects.extend(detector.scan_projects());
            }
        }
        projects
    }

    /// 为指定的 Agent 配置 MCP 连接
    ///
    /// 安全策略：
    ///   - 如果目标文件已存在，尝试合并现有配置（保留用户已有的其他 MCP 配置）
    ///   - 如果合并失败，创建备份后写入新配置
    ///   - API Key 不写入 MCP 配置文件
    ///   - 对于不支持 MCP 的工具，仅返回提示信息
    pub fn configure(&self, agent_ids: &[String], port: u16) -> Result<Vec<String>, String> {
        let mut configured = Vec::new();

        for id in agent_ids {
            if let Some(detector) = self.detectors.iter().find(|d| d.info().id == *id) {
                let info = detector.info();
                if !info.supports_mcp {
                    configured.push(format!(
                        "{} — 不支持 MCP 协议，无法自动配置",
                        info.name
                    ));
                    continue;
                }

                let config = detector.generate_config(port);

                if let Some(path) = detector.config_path() {
                    self.write_or_merge_config(&path, &config)?;
                    configured.push(format!("{} (全局配置)", info.name));
                } else if info.id == "generic-mcp" {
                    configured.push(format!(
                        "{} — HTTP 端点: http://127.0.0.1:{}/mcp",
                        info.name, port
                    ));
                } else {
                    configured.push(format!(
                        "{} — 请手动配置项目级 mcp.json", info.name
                    ));
                }
            }
        }

        Ok(configured)
    }

    /// 一键配置所有已安装的支持 MCP 的工具
    pub fn configure_all_installed(&self, port: u16) -> Result<Vec<String>, String> {
        let installed_ids: Vec<String> = self
            .detect_installed()
            .iter()
            .filter(|info| info.supports_mcp)
            .map(|info| info.id.clone())
            .collect();
        self.configure(&installed_ids, port)
    }

    /// 写入或与现有配置合并
    fn write_or_merge_config(
        &self,
        path: &std::path::Path,
        new_config: &serde_json::Value,
    ) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        if path.exists() {
            let existing_content = std::fs::read_to_string(path).unwrap_or_default();
            if let Ok(existing_json) = serde_json::from_str::<serde_json::Value>(&existing_content) {
                if let (Some(existing_servers), Some(new_servers)) = (
                    existing_json.get("mcpServers"),
                    new_config.get("mcpServers"),
                ) {
                    let mut merged = existing_json.clone();
                    if let Some(obj) = merged.as_object_mut() {
                        let mut servers = existing_servers
                            .as_object()
                            .cloned()
                            .unwrap_or_default();
                        if let Some(new_obj) = new_servers.as_object() {
                            servers.extend(new_obj.clone());
                        }
                        obj.insert(
                            "mcpServers".to_string(),
                            serde_json::Value::Object(servers),
                        );
                    }
                    let backup_path = path.with_extension("json.bak");
                    let _ = std::fs::write(&backup_path, &existing_content);
                    let json = serde_json::to_string_pretty(&merged).map_err(|e| e.to_string())?;
                    std::fs::write(path, json).map_err(|e| e.to_string())?;
                    return Ok(());
                }
            }
        }

        let json = serde_json::to_string_pretty(new_config).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| e.to_string())?;
        Ok(())
    }
}

impl Default for AgentDetectorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// 判断是否为系统目录（不应被识别为 AI 工具）
fn is_system_dir(name: &str) -> bool {
    let system_dirs = [
        ".cache",
        ".cargo",
        ".config",
        ".conda",
        ".gradle",
        ".local",
        ".matplotlib",
        ".rustup",
        ".ssh",
        ".streamlit",
        ".npm",
        ".node",
        ".yarn",
        ".pnpm",
        ".docker",
        ".kube",
        ".aws",
        ".azure",
        ".gcloud",
        ".android",
        ".bash",
        ".zsh",
        ".oh-my-zsh",
        ".git",
        ".svn",
        ".Trash",
        ".cache_",
        ".dbclient",
        ".IdentityService",
        ".oracle_jre_usage",
        ".vscode-server",
        ".vscode-insiders",
        ".vscode-cli",
        ".vscode-oss",
    ];
    system_dirs.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TDD：测试 detect_all 返回所有注册的检测器信息
    #[test]
    fn test_detect_all_returns_all_detectors() {
        let registry = AgentDetectorRegistry::new();
        let agents = registry.detect_all();
        // 应该包含：Trae, Trae CN, Claude Desktop, GenericMCP + 所有 KNOWN_TOOLS（排除特殊检测器）
        let expected_min = 4 + KNOWN_TOOLS.len() - 3; // 减去 3 个被特殊检测器覆盖的
        assert!(
            agents.len() >= expected_min,
            "期望至少 {} 个检测器，实际 {} 个",
            expected_min,
            agents.len()
        );
        let ids: Vec<_> = agents.iter().map(|a| a.id.clone()).collect();
        assert!(ids.contains(&"trae".to_string()));
        assert!(ids.contains(&"cursor".to_string()));
        assert!(ids.contains(&"vscode".to_string()));
        assert!(ids.contains(&"windsurf".to_string()));
        assert!(ids.contains(&"claude-desktop".to_string()));
        assert!(ids.contains(&"generic-mcp".to_string()));
        // 新增的工具
        assert!(ids.contains(&"kiro".to_string()));
        assert!(ids.contains(&"gemini-cli".to_string()));
        assert!(ids.contains(&"codebuddy".to_string()));
    }

    /// TDD：测试 AgentInfo 序列化 / 反序列化
    #[test]
    fn test_agent_info_serialization() {
        let info = AgentInfo {
            id: "trae".into(),
            name: "Trae".into(),
            installed: false,
            config_path: None,
            icon: "🖥️".into(),
            category: "ide".into(),
            supports_mcp: true,
        };
        let json = serde_json::to_string(&info).unwrap();
        let decoded: AgentInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.id, "trae");
        assert_eq!(decoded.name, "Trae");
        assert_eq!(decoded.category, "ide");
        assert!(decoded.supports_mcp);
    }

    /// TDD：测试 GenericMcpAgent 始终可用
    #[test]
    fn test_generic_mcp_always_available() {
        let detector = GenericMcpAgent;
        assert!(detector.detect());
        assert_eq!(detector.info().category, "custom");
    }

    /// TDD：测试 KNOWN_TOOLS 数据库没有重复 ID
    #[test]
    fn test_known_tools_no_duplicate_ids() {
        let mut ids = std::collections::HashSet::new();
        for tool in KNOWN_TOOLS {
            assert!(
                ids.insert(tool.id),
                "KNOWN_TOOLS 中存在重复 ID: {}",
                tool.id
            );
        }
    }

    /// TDD：测试 is_system_dir 正确识别系统目录
    #[test]
    fn test_is_system_dir() {
        assert!(is_system_dir(".cache"));
        assert!(is_system_dir(".cargo"));
        assert!(is_system_dir(".ssh"));
        assert!(!is_system_dir(".trae"));
        assert!(!is_system_dir(".cursor"));
        assert!(!is_system_dir(".kiro"));
    }
}