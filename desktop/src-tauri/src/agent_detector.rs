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
///   - 国产 AI 工具：通义灵码 (阿里云), 豆包 MarsCode (字节), 智谱 CodeGeeX, 腾讯云 AI 代码助手, 华为 CodeArts Snap
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
    /// 当前检测策略仅使用 binary_paths，此字段保留供未来扩展
    #[allow(dead_code)]
    secondary_markers: &'static [&'static str],
    /// MCP 配置路径模板（相对于 %USERPROFILE%，None 表示项目级配置或无 MCP）
    mcp_config_template: Option<&'static str>,
    /// MCP 传输类型："stdio" 或 "http"
    mcp_transport: &'static str,
    /// 二进制可执行文件路径（多路径，按优先级，相对于 %LOCALAPPDATA% 或 %PROGRAMFILES%）
    /// 格式：支持环境变量 %LOCALAPPDATA%, %PROGRAMFILES%, %PROGRAMFILES(X86)%
    /// 例如：%LOCALAPPDATA%/Programs/Trae/Trae.exe
    binary_paths: &'static [&'static str],
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
        binary_paths: &["%LOCALAPPDATA%/Programs/Trae/Trae.exe", "%PROGRAMFILES%/Trae/Trae.exe"],
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
        binary_paths: &["%LOCALAPPDATA%/Programs/Trae CN/Trae CN.exe", "%PROGRAMFILES%/Trae CN/Trae CN.exe"],
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
        binary_paths: &["%LOCALAPPDATA%/Programs/Cursor/Cursor.exe", "%PROGRAMFILES%/Cursor/Cursor.exe"],
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
        binary_paths: &["%LOCALAPPDATA%/Programs/Microsoft VS Code/Code.exe", "%PROGRAMFILES%/Microsoft VS Code/Code.exe"],
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
        binary_paths: &["%LOCALAPPDATA%/Programs/Windsurf/Windsurf.exe", "%PROGRAMFILES%/Windsurf/Windsurf.exe"],
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
        binary_paths: &["%LOCALAPPDATA%/Programs/Kiro/Kiro.exe"],
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
        binary_paths: &["%LOCALAPPDATA%/AnthropicClaude/claude.exe"],
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
        binary_paths: &[],
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
        binary_paths: &[],
    },

    // ═══ AI 编码助手类 ═══
    KnownTool {
        id: "codebuddy",
        name: "CodeBuddy (腾讯)",
        icon: "🤝",
        category: "ai-assistant",
        supports_mcp: true,
        primary_marker: ".codebuddy",
        secondary_markers: &[],
        mcp_config_template: Some(".codebuddy/mcp.json"),
        mcp_transport: "stdio",
        binary_paths: &[],
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
        binary_paths: &[],
    },
    // ═══ 国产 AI 工具 ═══
    KnownTool {
        id: "tongyi-lingma",
        name: "通义灵码 (阿里云)",
        icon: "☁️",
        category: "ai-assistant",
        supports_mcp: false,
        primary_marker: ".lingma",
        secondary_markers: &[".tongyi-lingma"],
        mcp_config_template: None,
        mcp_transport: "stdio",
        binary_paths: &[],
    },
    KnownTool {
        id: "marscode",
        name: "豆包 MarsCode (字节)",
        icon: "🫘",
        category: "ai-assistant",
        supports_mcp: false,
        primary_marker: ".marscode",
        secondary_markers: &[],
        mcp_config_template: None,
        mcp_transport: "stdio",
        binary_paths: &[],
    },
    KnownTool {
        id: "codegeex",
        name: "智谱 CodeGeeX",
        icon: "🧬",  // v0.5.7 修复 L-1：从 🧠 改为 🧬（避免与 claude-desktop 重复）
        category: "ai-assistant",
        supports_mcp: false,
        primary_marker: ".codegeex",
        secondary_markers: &[],
        mcp_config_template: None,
        mcp_transport: "stdio",
        binary_paths: &[],
    },
    KnownTool {
        id: "tencent-ai-code",
        name: "腾讯云 AI 代码助手",
        icon: "🐧",  // v0.5.7 修复 L-1：从 ☁️ 改为 🐧（腾讯企鹅，避免与 tongyi-lingma 重复）
        category: "ai-assistant",
        supports_mcp: false,
        primary_marker: ".tencent-ai-code",
        secondary_markers: &[],
        mcp_config_template: None,
        mcp_transport: "stdio",
        binary_paths: &[],
    },
    KnownTool {
        id: "huawei-codearts",
        name: "华为 CodeArts Snap",
        icon: "🔷",
        category: "ai-assistant",
        supports_mcp: false,
        primary_marker: ".codearts-snap",
        secondary_markers: &[],
        mcp_config_template: None,
        mcp_transport: "stdio",
        binary_paths: &[],
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
        binary_paths: &[],
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
        binary_paths: &[],
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
        binary_paths: &[],
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
        binary_paths: &[],
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
        binary_paths: &[],
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
        binary_paths: &[],
    },
    KnownTool {
        id: "amazon-q",
        name: "Amazon Q Developer",
        icon: "📦",  // v0.5.7 修复 L-1：从 ☁️ 改为 📦（Amazon 包裹，避免与 tongyi-lingma 重复）
        category: "ai-assistant",
        supports_mcp: false,
        primary_marker: ".aws",
        secondary_markers: &[],
        mcp_config_template: None,
        mcp_transport: "stdio",
        binary_paths: &[],
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
        binary_paths: &[],
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
        binary_paths: &["%PROGRAMFILES%/JetBrains/IntelliJ IDEA/bin/idea64.exe", "%LOCALAPPDATA%/JetBrains/Toolbox/scripts/idea.cmd"],
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
        binary_paths: &[],
    },
    KnownTool {
        id: "zed",
        name: "Zed",
        icon: "🚀",  // v0.5.7 修复 L-1：从 ⚡ 改为 🚀（高性能编辑器，避免与 augment 重复）
        category: "ide",
        supports_mcp: false,
        primary_marker: ".zed",
        secondary_markers: &[],
        mcp_config_template: None,
        mcp_transport: "stdio",
        binary_paths: &["%LOCALAPPDATA%/Programs/Zed/Zed.exe"],
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
        binary_paths: &[],
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
        binary_paths: &[],
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
        binary_paths: &[],
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
        binary_paths: &[],
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
        binary_paths: &[],
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
        binary_paths: &[],
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
        binary_paths: &[],
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
        binary_paths: &[],
    },
    // v0.5.4 P2-20 修复：移除 loong-recall 条目
    // 原因：~/.loong-recall 是 LRC 桌面端自己的数据目录，不是独立的 AI 工具。
    //       将其作为独立工具检测会导致所有安装了 LRC 的用户都看到"Loong Recall 已安装"，
    //       这是误导性的。LRC 桌面端应用本身就是 LRC 的入口。
];

// ── 通用扫描函数 ──

/// 扫描条目数上限（防止在大目录如 Desktop 中扫描过多条目）
const MAX_SCAN_ENTRIES: usize = 200;

/// 扫描指定根目录下包含 marker 子目录的项目
fn scan_marker_projects(
    roots: &[PathBuf],
    marker: &str,
    ide_id: &str,
    ide_name: &str,
) -> Vec<ProjectInfo> {
    let mut projects = Vec::new();
    let mut scanned = 0usize;
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
                // 扫描条目数上限（防止大目录扫描过慢）
                if scanned >= MAX_SCAN_ENTRIES {
                    tracing::debug!(
                        "扫描根目录 {} 达到上限 {} 条，停止扫描",
                        root.display(),
                        MAX_SCAN_ENTRIES
                    );
                    break;
                }
                scanned += 1;
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
/// 
/// v0.5.4 修复：移除驱动根目录扫描（C:\, D:\ 等），太慢且无意义。
/// 只扫描用户主目录下的常见项目目录，外加 IDE 工作区配置中记录的项目。
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
            "",  // 主目录本身
        ] {
            let p = home.join(sub);
            if p.exists() {
                roots.push(p);
            }
        }
    }
    // v0.5.4 修复：移除驱动根目录扫描，避免扫描整个硬盘
    // 驱动根目录扫描会导致：
    // 1. 扫描时间过长（可能数分钟）
    // 2. 大量系统目录误报
    // 3. 用户体验极差
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
///
/// v0.5.4 安全加固：添加路径遍历防护，拒绝包含 ".." 的标记路径
fn resolve_marker(marker: &str) -> Option<PathBuf> {
    // v0.5.4 路径遍历防护：拒绝包含 ".." 路径组件的标记
    // 防止攻击者通过配置恶意标记路径访问系统敏感文件
    if marker.contains("..") {
        eprintln!("[LRC·安全] 拒绝包含路径遍历的标记: {}", marker);
        return None;
    }

    if let Some(rest) = marker.strip_prefix("%APPDATA%/") {
        return appdata_dir().map(|d| d.join(rest));
    }
    if let Some(rest) = marker.strip_prefix("%LOCALAPPDATA%/") {
        return local_appdata_dir().map(|d| d.join(rest));
    }
    // 相对于 USERPROFILE
    home_dir().map(|d| d.join(marker))
}

/// 解析二进制路径中的环境变量
/// 支持的变量：%LOCALAPPDATA%, %PROGRAMFILES%, %PROGRAMFILES(X86)%, %APPDATA%, %USERPROFILE%
fn resolve_binary_path(template: &str) -> Option<PathBuf> {
    let resolved = template
        .replace("%LOCALAPPDATA%", &std::env::var("LOCALAPPDATA").unwrap_or_default())
        .replace("%PROGRAMFILES(X86)%", &std::env::var("ProgramFiles(x86)").unwrap_or_default())
        .replace("%PROGRAMFILES%", &std::env::var("ProgramFiles").unwrap_or_default())
        .replace("%APPDATA%", &std::env::var("APPDATA").unwrap_or_default())
        .replace("%USERPROFILE%", &std::env::var("USERPROFILE").unwrap_or_default());
    let path = PathBuf::from(&resolved);
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

/// 检查二进制可执行文件是否存在
/// 返回 true 如果任意一个 binary_path 指向存在的文件
fn binary_exists(binary_paths: &[&str]) -> bool {
    binary_paths.iter().any(|p| resolve_binary_path(p).is_some())
}
/// 检查标记路径是否存在（保留供未来扩展使用）
#[allow(dead_code)]
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
    ///
    /// v0.5.5 增强：严格检测策略，避免残留目录误报
    ///
    /// 策略（按优先级）：
    ///   1. 有 binary_paths 的工具 → 检测二进制文件是否存在（最准确）
    ///   2. 无 binary_paths 但有 mcp_config_template 的工具 → 检测配置文件是否存在
    ///   3. 无 binary_paths 且无 mcp_config_template 的工具 → 不自动检测
    ///      （避免残留 dot 目录误报，用户可在向导中手动选择）
    fn check_known_tool(&self) -> bool {
        // 策略 1：检测二进制可执行文件（最准确）
        if !self.tool.binary_paths.is_empty() {
            return binary_exists(self.tool.binary_paths);
        }
        // 策略 2：无二进制路径但有 MCP 配置模板 → 检测配置文件是否存在
        if let Some(config_template) = self.tool.mcp_config_template {
            if let Some(cp) = resolve_marker(config_template) {
                if cp.exists() {
                    tracing::debug!(
                        "[Agent检测] {} — 通过配置文件检测到: {}",
                        self.tool.name,
                        cp.display()
                    );
                    return true;
                }
                tracing::debug!(
                    "[Agent检测] {} — 配置文件不存在: {}",
                    self.tool.name,
                    cp.display()
                );
                return false;
            }
        }
        // 策略 3：无 binary_paths 且无 mcp_config_template → 不自动检测
        // v0.5.5 修复：避免残留 dot 目录误报（如 .gemini、.codex、.continue 等）
        // 这些工具用户可在配置向导中手动选择
        tracing::debug!(
            "[Agent检测] {} — 无二进制路径且无配置模板，不自动检测（避免误报）",
            self.tool.name
        );
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
            .and_then(resolve_marker)
            // v0.5.1 修复：移除过于严格的父目录存在检查
            // write_or_merge_config 会自动创建父目录，无需在此过滤
    }

    fn generate_config(&self, port: u16) -> serde_json::Value {
        // v0.5.5 修复：LRC Desktop 总是启动 HTTP 模式的 sidecar
        // 因此所有工具都应使用 HTTP 模式配置，直接连接已运行的 sidecar
        // 避免 stdio 模式启动新 sidecar 进程导致"已有实例运行"冲突
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

        // 策略 2：检查安装目录（二进制可执行文件）
        let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
        if PathBuf::from(&local).join("Programs").join("Trae").join("Trae.exe").exists()
            || PathBuf::from(&local).join("Programs").join("Trae CN").join("Trae CN.exe").exists()
        {
            return true;
        }

        // 策略 3：检查 Program Files
        for pf in &["C:\\Program Files\\Trae\\Trae.exe", "C:\\Program Files (x86)\\Trae\\Trae.exe"] {
            if std::path::Path::new(pf).exists() {
                return true;
            }
        }

        // v0.5.3 修复：移除注册表查询（reg query），速度慢且不可靠
        // 注册表查询在 Windows 上可能需要 2-5 秒，且可能返回误报

        false
    }

    fn config_path(&self) -> Option<PathBuf> {
        // v0.5.4 P2-18 根因修复：Trae 国际版实际读取的是 %APPDATA%/Trae/User/mcp.json
        // 修复前：优先返回 ~/.trae/mcp.json，但 Trae GUI 不读取此文件，
        //         导致用户在 Trae 界面中看不到 LRC MCP 服务器
        // 修复后：优先返回 AppData 路径（Trae 实际读取的路径），
        //         只有 AppData 不存在时才回退到 ~/.trae/mcp.json

        let home = home_dir()?;
        let home_config = home.join(".trae").join("mcp.json");

        // 优先级 1：AppData 下的 Trae 配置（Trae 实际读取的路径）
        let appdata = appdata_dir();
        if let Some(ref appdata) = appdata {
            let appdata_config = appdata.join("Trae").join("User").join("mcp.json");
            if appdata_config.exists() {
                return Some(appdata_config);
            }
            // 优先级 2：如果 AppData/Trae 目录存在但 mcp.json 还没创建，返回该路径用于创建
            if appdata.join("Trae").exists() {
                return Some(appdata_config);
            }
        }

        // 优先级 3：回退到 ~/.trae/mcp.json（兼容旧路径）
        if home_config.exists() {
            return Some(home_config);
        }

        // 如果 ~/.trae 目录存在，返回对应路径
        if home.join(".trae").exists() {
            return Some(home_config);
        }

        // 最终回退：返回 AppData 路径（如果可用），否则返回 home 路径
        if let Some(ref appdata) = appdata {
            Some(appdata.join("Trae").join("User").join("mcp.json"))
        } else {
            Some(home_config)
        }
    }

    fn generate_config(&self, port: u16) -> serde_json::Value {
        // v0.5.4 P2-19 修复：改用 HTTP 模式连接到桌面端 sidecar，避免 stdio 模式启动新实例冲突
        // 修复前：使用 stdio 模式启动 lrc-sidecar，但桌面端应用已启动 sidecar（PID 锁定），
        //         新实例检测到已有实例运行后退出，导致 Trae 报 "Connection closed" 错误
        // 修复后：使用 HTTP 模式连接到桌面端 sidecar 的 MCP 端点（http://127.0.0.1:{port}/mcp）
        //         参考 Trae 官方文档：https://docs.trae.ai/ide/model-context-protocol
        let actual_port = if port == 0 { 3099 } else { port };
        serde_json::json!({
            "mcpServers": {
                "lrc-memory": {
                    "url": format!("http://127.0.0.1:{}/mcp", actual_port)
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
        // v0.5.4 P2-18 根因修复：Trae CN 实际读取的是 %APPDATA%/Trae CN/User/mcp.json
        // 修复前：优先返回 ~/.trae-cn/trae-mcp.json，但 Trae CN GUI 不读取此文件，
        //         导致用户在 Trae CN 界面中看不到 LRC MCP 服务器
        // 修复后：优先返回 AppData 路径（Trae CN 实际读取的路径），
        //         只有 AppData 不存在时才回退到 ~/.trae-cn/trae-mcp.json

        // 优先级 1：AppData 下的 Trae CN 配置（Trae CN 实际读取的路径）
        let appdata = appdata_dir()?;
        let appdata_config = appdata.join("Trae CN").join("User").join("mcp.json");
        if appdata_config.exists() {
            return Some(appdata_config);
        }

        // 优先级 2：如果 AppData/Trae CN 目录存在但 mcp.json 还没创建，返回该路径用于创建
        if appdata.join("Trae CN").exists() {
            return Some(appdata_config);
        }

        // 优先级 3：回退到 ~/.trae-cn/trae-mcp.json（兼容旧路径）
        let home = home_dir()?;
        let home_config = home.join(".trae-cn").join("trae-mcp.json");
        if home_config.exists() {
            return Some(home_config);
        }

        // 默认：返回 AppData 路径（即使不存在，也用于创建）
        Some(appdata_config)
    }

    fn generate_config(&self, port: u16) -> serde_json::Value {
        // v0.5.4 P2-19 修复：改用 HTTP 模式连接到桌面端 sidecar，避免 stdio 模式启动新实例冲突
        // 修复前：使用 stdio 模式启动 lrc-sidecar，但桌面端应用已启动 sidecar（PID 锁定），
        //         新实例检测到已有实例运行后退出，导致 Trae CN 报 "Connection closed" 错误
        // 修复后：使用 HTTP 模式连接到桌面端 sidecar 的 MCP 端点（http://127.0.0.1:{port}/mcp）
        //         参考 Trae 官方文档：https://docs.trae.ai/ide/model-context-protocol
        let actual_port = if port == 0 { 3099 } else { port };
        serde_json::json!({
            "mcpServers": {
                "lrc-memory": {
                    "url": format!("http://127.0.0.1:{}/mcp", actual_port)
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
        // v0.5.4 修复：Trae CN 使用 .trae-cn 标记，而非 .trae
        scan_marker_projects(&scan_roots(), ".trae-cn", "trae-cn", "Trae CN")
    }
}

/// Claude Desktop 专用检测器（需要多策略验证防止误报）
struct ClaudeDesktopDetector;

impl AgentDetector for ClaudeDesktopDetector {
    fn detect(&self) -> bool {
        #[cfg(target_os = "windows")]
        {
            let local_appdata = std::env::var("LOCALAPPDATA").unwrap_or_default();
            // 策略 1：检查默认安装路径下的 claude.exe
            let install_exe = PathBuf::from(&local_appdata)
                .join("AnthropicClaude")
                .join("claude.exe");
            if install_exe.exists() {
                return true;
            }
            // 策略 2：检查 Programs 子目录（部分版本安装在此）
            let programs_exe = PathBuf::from(&local_appdata)
                .join("Programs")
                .join("claude")
                .join("claude.exe");
            if programs_exe.exists() {
                return true;
            }
            // 策略 3：检查 Program Files
            for pf in &[
                r"C:\Program Files\Claude\claude.exe",
                r"C:\Program Files (x86)\Claude\claude.exe",
            ] {
                if std::path::Path::new(pf).exists() {
                    return true;
                }
            }
            // v0.5.7 修复：移除注册表查询（reg query），速度慢且不可靠
            // reg.exe 同步调用会阻塞 Tokio worker 线程 1-5 秒，
            // 在持有 agent_registry 锁的情况下导致"正在扫描AI工具..."卡死。
            // 参照 TraeDetector 的修复方式（v0.5.3），改用文件系统检查。
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
                Some(config)
            } else {
                None
            }
        }

        #[cfg(target_os = "macos")]
        {
            let home = home_dir()?;
            Some(
                home.join("Library/Application Support/Claude")
                    .join("claude_desktop_config.json"),
            )
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            let home = home_dir()?;
            Some(home.join(".config/Claude/claude_desktop_config.json"))
        }
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
        // v0.5.3 修复：不自动检测为"已安装"，避免误报
        // 通用 MCP Agent 仅作为手动配置的入口，不在自动检测列表中显示
        false
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
            // v0.5.4 P2-20 修复：与 detect() 返回值保持一致，避免误报
            installed: false,
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
    /// LRC 二进制文件的绝对路径（用于 MCP 配置中替换 "lrc" 命令）
    /// 在桌面端启动时自动设置为当前 exe 同目录下的 lrc-sidecar 路径
    lrc_binary_path: Option<String>,
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

        Self {
            detectors,
            lrc_binary_path: None,
        }
    }

    /// 设置 LRC 二进制文件的绝对路径
    ///
    /// 在桌面端启动时调用，将当前 exe 同目录下的 lrc-sidecar 路径传入。
    /// 此后所有 MCP 配置中的 "lrc" 命令将被替换为绝对路径，
    /// 确保 IDE 无需依赖 PATH 环境变量即可找到 LRC。
    pub fn set_lrc_binary_path(&mut self, path: String) {
        self.lrc_binary_path = Some(path);
    }

    /// 检测所有已安装的 Agent，返回信息列表
    pub fn detect_all(&self) -> Vec<AgentInfo> {
        self.detectors.iter().map(|d| d.info()).collect()
    }

    /// v0.5.4 新增：带进度回调的 Agent 检测
    /// 
    /// 每检测完一个 Agent 就调用 on_progress 回调，
    /// 前端可据此显示"正在检测 Trae... (3/22)"的进度反馈。
    pub fn detect_all_with_progress<F>(&self, on_progress: F) -> Vec<AgentInfo>
    where
        F: Fn(usize, usize, &AgentInfo),
    {
        let total = self.detectors.len();
        self.detectors
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let info = d.info();
                on_progress(i + 1, total, &info);
                info
            })
            .collect()
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
    /// v0.5.1 增强：
    ///   - 详细日志记录每个 Agent 的配置路径和结果
    ///   - 二进制路径替换的显式日志
    ///   - 配置写入失败时明确的错误信息
    ///
    /// 安全策略：
    ///   - 如果目标文件已存在，尝试合并现有配置（保留用户已有的其他 MCP 配置）
    ///   - 如果合并失败，创建备份后写入新配置
    ///   - API Key 不写入 MCP 配置文件
    ///   - 对于不支持 MCP 的工具，仅返回提示信息
    pub fn configure(&self, agent_ids: &[String], port: u16, project_dir: Option<&std::path::Path>) -> Result<Vec<String>, String> {
        // v0.5.6：project_dir 不再用于规则文件写入（改为全局），保留参数仅为向后兼容
        let _ = project_dir;
        let mut configured = Vec::new();

        tracing::info!(
            "[MCP配置] 开始为 {} 个 Agent 配置 MCP 连接（端口: {}）",
            agent_ids.len(),
            port
        );
        if let Some(ref binary_path) = self.lrc_binary_path {
            tracing::info!("[MCP配置] LRC 二进制路径: {}", binary_path);
        } else {
            tracing::warn!("[MCP配置] LRC 二进制路径未设置，MCP 配置将使用 'lrc' 命令（需要 PATH 环境变量）");
        }

        for id in agent_ids {
            if let Some(detector) = self.detectors.iter().find(|d| d.info().id == *id) {
                let info = detector.info();
                tracing::info!("[MCP配置] 正在配置 Agent: {} ({})", info.name, id);

                if !info.supports_mcp {
                    tracing::info!("[MCP配置] {} — 不支持 MCP 协议，跳过", info.name);
                    configured.push(format!(
                        "{} — 不支持 MCP 协议，无法自动配置",
                        info.name
                    ));
                    continue;
                }

                let mut config = detector.generate_config(port);

                // 如果设置了二进制路径，将配置中的 "lrc" / "lrc-sidecar" 命令替换为绝对路径
                // 这样 IDE 无需依赖 PATH 环境变量即可找到 LRC
                if let Some(ref binary_path) = self.lrc_binary_path {
                    if let Some(servers) = config.get_mut("mcpServers") {
                        if let Some(obj) = servers.as_object_mut() {
                            for (_name, server_config) in obj.iter_mut() {
                                if let Some(cmd) = server_config.get("command") {
                                    // v0.5.4 修复：同时匹配 "lrc" 和 "lrc-sidecar"，
                                    // 确保向后兼容历史配置和手动写入的配置
                                    let cmd_str = cmd.as_str().unwrap_or("").to_string();
                                    if cmd_str == "lrc" || cmd_str == "lrc-sidecar" {
                                        server_config["command"] = serde_json::Value::String(binary_path.clone());
                                        tracing::info!(
                                            "[MCP配置] {} — 已将命令 '{}' 替换为绝对路径: {}",
                                            info.name, cmd_str, binary_path
                                        );
                                    }
                                }
                            }
                        }
                    }
                }

                if let Some(path) = detector.config_path() {
                    let path_str = path.display().to_string();
                    tracing::info!("[MCP配置] {} — 目标配置文件: {}", info.name, path_str);
                    match self.write_or_merge_config(&path, &config) {
                        Ok(()) => {
                            tracing::info!("[MCP配置] {} — 配置写入成功: {}", info.name, path_str);
                            configured.push(format!("{} (全局配置)", info.name));

                            // v0.5.6 重构：MCP 配置写入成功后，自动写入全局 AI 规则文件
                            // 规则文件写入用户主目录，一次配置对所有项目生效
                            match Self::write_ai_rules(&info.id) {
                                Ok(()) => {
                                    tracing::info!(
                                        "[AI规则] {} — 全局规则文件写入成功",
                                        info.name
                                    );
                                }
                                Err(e) => {
                                    // AI 规则写入失败不影响主流程（MCP 配置已成功）
                                    tracing::warn!(
                                        "[AI规则] {} — 全局规则文件写入失败（不影响 MCP 功能）: {}",
                                        info.name, e
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!("[MCP配置] {} — 配置写入失败: {} — 错误: {}", info.name, path_str, e);
                            configured.push(format!("{} — 配置写入失败: {}", info.name, e));
                        }
                    }
                } else if info.id == "generic-mcp" {
                    configured.push(format!(
                        "{} — HTTP 端点: http://127.0.0.1:{}/mcp",
                        info.name, port
                    ));
                } else {
                    tracing::info!("[MCP配置] {} — 无全局配置路径，需手动配置项目级 mcp.json", info.name);
                    configured.push(format!(
                        "{} — 请手动配置项目级 mcp.json", info.name
                    ));
                }
            } else {
                tracing::warn!("[MCP配置] 未找到 Agent: {}", id);
            }
        }

        tracing::info!("[MCP配置] 完成，共配置 {} 个 Agent", configured.len());
        Ok(configured)
    }

    /// 一键配置所有已安装的支持 MCP 的工具
    pub fn configure_all_installed(&self, port: u16, project_dir: Option<&std::path::Path>) -> Result<Vec<String>, String> {
        let installed_ids: Vec<String> = self
            .detect_installed()
            .iter()
            .filter(|info| info.supports_mcp)
            .map(|info| info.id.clone())
            .collect();
        self.configure(&installed_ids, port, project_dir)
    }

    /// v0.5.6 新增：为指定 Agent 写入全局 IDE 规则文件（不修改 MCP 配置）
    ///
    /// 规则文件写入用户主目录，一次配置对所有项目生效。
    /// 与 configure() 不同，此方法不修改 MCP 配置文件，只写入规则文件。
    /// v0.5.6 重构：从项目级改为全局级，不再需要 project_dir 参数
    pub fn write_rules_for_agents(
        &self,
        agent_ids: &[String],
    ) -> Result<Vec<String>, String> {
        let mut results = Vec::new();
        tracing::info!(
            "[AI规则] 开始为 {} 个 Agent 写入全局规则文件",
            agent_ids.len()
        );

        for id in agent_ids {
            // 查找已知工具
            let known = KNOWN_TOOLS.iter().find(|t| t.id == *id);
            if known.is_none() {
                tracing::debug!("[AI规则] {} — 未知工具，跳过", id);
                continue;
            }

            match Self::write_ai_rules(id) {
                Ok(()) => {
                    tracing::info!("[AI规则] {} — 全局规则文件写入成功", id);
                    results.push(id.clone());
                }
                Err(e) => {
                    tracing::warn!("[AI规则] {} — 全局规则文件写入失败: {}", id, e);
                }
            }
        }

        tracing::info!("[AI规则] 完成，共写入 {} 个全局规则文件", results.len());
        Ok(results)
    }

    /// v0.5.5 新增：自动检测并升级旧版本 MCP 配置
    ///
    /// 在 sidecar 启动时自动调用，无需用户重新运行配置向导。
    /// 检测并修复以下旧配置：
    /// 1. 旧配置名称 `loong-recall`（stdio 模式）→ `lrc-memory`（HTTP 模式）
    /// 2. 旧路径规则文件 `.trae/rules.md` → `.trae/rules/lrc-memory.md`
    /// 3. 旧版本规则内容 → v0.5.5 版本（含 frontmatter）
    ///
    /// 这样用户升级 LRC Desktop 后，无需手动操作，配置自动升级。
    pub fn auto_upgrade_configs(&self, port: u16, project_dir: Option<&std::path::Path>) -> Result<Vec<String>, String> {
        // v0.5.6：project_dir 不再用于规则文件写入（改为全局），保留参数仅为向后兼容
        let _ = project_dir;
        let mut upgraded = Vec::new();

        tracing::info!("[自动升级] 开始检测旧版本 MCP 配置（端口: {}）", port);

        for detector in &self.detectors {
            let info = detector.info();
            if !info.supports_mcp {
                continue;
            }

            // 检查配置文件是否存在
            let config_path = match detector.config_path() {
                Some(p) => p,
                None => continue,
            };

            if !config_path.exists() {
                continue;
            }

            // 读取现有配置
            let existing_content = match std::fs::read_to_string(&config_path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("[自动升级] {} — 读取配置失败: {} ({})", info.name, config_path.display(), e);
                    continue;
                }
            };

            // 检查是否需要升级
            let needs_upgrade = self.config_needs_upgrade(&existing_content);
            if !needs_upgrade {
                continue;
            }

            tracing::info!("[自动升级] {} — 检测到旧版本配置，开始升级: {}", info.name, config_path.display());

            // 生成新配置并写入
            let new_config = detector.generate_config(port);
            match self.write_or_merge_config(&config_path, &new_config) {
                Ok(()) => {
                    tracing::info!("[自动升级] {} — MCP 配置升级成功: {}", info.name, config_path.display());
                    upgraded.push(format!("{} — MCP 配置已升级为 HTTP 模式", info.name));

                    // v0.5.6：同时升级全局规则文件（不再依赖 project_dir）
                    match Self::write_ai_rules(&info.id) {
                        Ok(()) => {
                            tracing::info!("[自动升级] {} — 全局规则文件升级成功", info.name);
                        }
                        Err(e) => {
                            tracing::warn!("[自动升级] {} — 全局规则文件升级失败（不影响 MCP）: {}", info.name, e);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("[自动升级] {} — 配置升级失败: {} ({})", info.name, config_path.display(), e);
                }
            }
        }

        if upgraded.is_empty() {
            tracing::info!("[自动升级] 所有配置均为最新版本，无需升级");
        } else {
            tracing::info!("[自动升级] 完成，共升级 {} 个配置", upgraded.len());
        }

        Ok(upgraded)
    }

    /// v0.5.5 新增：检查配置是否需要升级
    ///
    /// 需要升级的情况：
    /// 1. 包含旧配置名称 `loong-recall`
    /// 2. `lrc-memory` 配置使用 stdio 模式（有 `command` 字段）
    /// 3. `lrc-memory` 配置使用旧的端口号
    fn config_needs_upgrade(&self, content: &str) -> bool {
        // 解析 JSON
        let json: serde_json::Value = match serde_json::from_str(content) {
            Ok(v) => v,
            Err(_) => return false, // 无法解析，不升级
        };

        let servers = match json.get("mcpServers").and_then(|v| v.as_object()) {
            Some(s) => s,
            None => return false,
        };

        // 检查旧配置名称 `loong-recall`
        if servers.contains_key("loong-recall") {
            return true;
        }

        // 检查 `lrc-memory` 是否使用 stdio 模式（有 `command` 字段）
        if let Some(lrc_config) = servers.get("lrc-memory") {
            if lrc_config.get("command").is_some() {
                return true; // stdio 模式，需要升级为 HTTP 模式
            }
        }

        // 检查其他旧配置名称
        let legacy_names = ["lrc", "lrc-memory-stdio", "lrc-stdio"];
        for name in &legacy_names {
            if servers.contains_key(*name) {
                return true;
            }
        }

        false
    }

    /// 写入或与现有配置合并
    ///
    /// v0.5.1 增强：添加详细日志，记录合并/备份/创建操作
    fn write_or_merge_config(
        &self,
        path: &std::path::Path,
        new_config: &serde_json::Value,
    ) -> Result<(), String> {
        // 确保父目录存在
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                let msg = format!("创建目录失败: {} ({})", parent.display(), e);
                tracing::error!("[MCP配置] {}", msg);
                msg
            })?;
            tracing::debug!("[MCP配置] 已确保目录存在: {}", parent.display());
        }

        if path.exists() {
            let existing_content = std::fs::read_to_string(path).unwrap_or_default();
            tracing::info!("[MCP配置] 目标文件已存在，尝试合并 ({})", path.display());
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

                        // v0.5.5 修复：删除旧的 LRC 配置名称，避免 stdio/http 冲突
                        // 旧版本可能写入的名称：loong-recall, lrc, lrc-memory-stdio
                        let legacy_names = ["loong-recall", "lrc", "lrc-memory-stdio", "lrc-stdio"];
                        for name in &legacy_names {
                            if servers.remove(*name).is_some() {
                                tracing::info!("[MCP配置] 已移除旧配置项: {}", name);
                            }
                        }

                        // v0.5.5 修复：删除旧的 stdio 模式 lrc-memory 配置
                        // 如果 lrc-memory 配置中包含 "command" 字段（stdio 模式），则删除
                        if let Some(existing_lrc) = servers.get("lrc-memory") {
                            if existing_lrc.get("command").is_some() {
                                tracing::info!("[MCP配置] 检测到旧的 stdio 模式 lrc-memory 配置，将替换为 HTTP 模式");
                                servers.remove("lrc-memory");
                            }
                        }

                        if let Some(new_obj) = new_servers.as_object() {
                            servers.extend(new_obj.clone());
                        }
                        obj.insert(
                            "mcpServers".to_string(),
                            serde_json::Value::Object(servers),
                        );
                    }
                    // 备份原文件
                    let backup_path = path.with_extension("json.bak");
                    let _ = std::fs::write(&backup_path, &existing_content);
                    tracing::info!("[MCP配置] 已备份原配置到: {}", backup_path.display());
                    let json = serde_json::to_string_pretty(&merged).map_err(|e| {
                        let msg = format!("序列化合并配置失败: {}", e);
                        tracing::error!("[MCP配置] {}", msg);
                        msg
                    })?;
                    std::fs::write(path, json).map_err(|e| {
                        let msg = format!("写入合并配置失败: {} ({})", path.display(), e);
                        tracing::error!("[MCP配置] {}", msg);
                        msg
                    })?;
                    tracing::info!("[MCP配置] 配置已合并写入: {}", path.display());
                    return Ok(());
                }
            }
            // 无法合并，备份后覆盖
            let backup_path = path.with_extension("json.bak");
            let _ = std::fs::write(&backup_path, &existing_content);
            tracing::warn!("[MCP配置] 无法合并现有配置，已备份到 {}，将覆盖写入", backup_path.display());
        } else {
            tracing::info!("[MCP配置] 目标文件不存在，将创建新文件: {}", path.display());
        }

        let json = serde_json::to_string_pretty(new_config).map_err(|e| {
            let msg = format!("序列化配置失败: {}", e);
            tracing::error!("[MCP配置] {}", msg);
            msg
        })?;
        std::fs::write(path, json).map_err(|e| {
            let msg = format!("写入配置失败: {} ({})", path.display(), e);
            tracing::error!("[MCP配置] {}", msg);
            msg
        })?;
        tracing::info!("[MCP配置] 配置已写入: {}", path.display());
        Ok(())
    }

    /// v0.5.6 重构：获取 AI 工具的全局规则文件模板（相对于用户主目录）
    ///
    /// 规则文件告知 AI 在什么情况下使用 remember/recall 工具。
    /// 不同工具使用不同的规则文件格式和位置。
    ///
    /// v0.5.6 重大变更：从项目级规则改为全局规则
    /// - 旧版：规则文件写入项目目录（如 `.cursor/rules/lrc-memory.mdc`），每个项目都要重新配置
    /// - 新版：规则文件写入用户主目录（如 `~/.cursor/rules/lrc-memory.mdc`），一次配置全局生效
    ///
    /// v0.5.7 修复内容（基于官方文档调研）：
    /// - cline: `.cline/clinerules` → `Documents/Cline/Rules/lrc-memory.md`
    ///   官方文档：https://docs.cline.bot/customization/cline-rules
    ///   全局规则目录为 `~/Documents/Cline/Rules/`，非 `.cline/clinerules`
    /// - windsurf: `.windsurf/rules/lrc-memory.md` → `.codeium/windsurf/memories/global_rules.md`
    ///   官方文档：https://docs.windsurf.com/windsurf/cascade/memories
    ///   全局规则为单文件 `global_rules.md`，`.windsurf/rules/` 是 workspace 级别
    /// - roo-code: `.roo/rules.md` → `.roo/rules/lrc-memory.md`（目录而非单文件）
    ///   官方文档：https://roocodeinc.github.io/Roo-Code/features/custom-instructions/
    ///   全局规则目录为 `~/.roo/rules/`，规则文件放在该目录下
    /// - zed: 移除（Zed 不支持传统全局规则文件，使用 Handlebars 模板）
    ///
    /// v0.5.6 修复内容：
    /// - 所有路径改为相对于用户主目录（%USERPROFILE%）的全局路径
    /// - 修复 ID 不匹配：`roo` → `roo-code`，`jetbrains` → `jetbrains-ai`
    /// - 新增 gemini-cli 全局规则（`~/.gemini/GEMINI.md`）
    /// - 新增 aider 全局规则（`~/.aider/CONVENTIONS.md`）
    fn get_rules_file_template(tool_id: &str) -> Option<&'static str> {
        match tool_id {
            // IDE 类 — 全局规则目录
            "cursor" => Some(".cursor/rules/lrc-memory.mdc"),
            // v0.5.8 修复：Trae/Trae CN 的用户级（全局）规则在 user_rules/ 目录，不是 rules/
            // 修复前：写入 ~/.trae/rules/lrc-memory.md（项目级规则目录，仅在打开项目时读取项目根目录）
            // 修复后：写入 ~/.trae/user_rules/lrc-memory.md（用户级规则目录，全局生效）
            // 参考：Trae CN 官方文档 https://docs.trae.ai/ide/rules
            //       全局规则通过 IDE 设置面板配置，存储在 ~/.trae-cn/user_rules/ 目录
            "trae" => Some(".trae/user_rules/lrc-memory.md"),
            "trae-cn" => Some(".trae-cn/user_rules/lrc-memory.md"),
            // v0.5.7 修复：Windsurf 全局规则为单文件 global_rules.md（非 .windsurf/rules/）
            // 官方文档：https://docs.windsurf.com/windsurf/cascade/memories
            // .windsurf/rules/ 是 workspace 级别，全局规则在 ~/.codeium/windsurf/memories/global_rules.md
            "windsurf" => Some(".codeium/windsurf/memories/global_rules.md"),
            // v0.5.7 移除：Zed 不支持传统全局规则文件（使用 Handlebars 模板，无法通过文件注入）
            // "zed" => Some(".zed/rules/lrc-memory.md"),
            // AI 编码助手类 — 全局规则文件
            "codebuddy" => Some(".codebuddy/rules.md"),
            // v0.5.7 修复：Cline 全局规则目录为 ~/Documents/Cline/Rules/（非 .cline/clinerules）
            // 官方文档：https://docs.cline.bot/customization/cline-rules
            // .clinerules 是项目级，全局规则在 Documents/Cline/Rules/
            "cline" => Some("Documents/Cline/Rules/lrc-memory.md"),
            // v0.5.7 修复：Roo Code 全局规则目录为 ~/.roo/rules/（目录而非单文件 .roo/rules.md）
            // 官方文档：https://roocodeinc.github.io/Roo-Code/features/custom-instructions/
            // 规则文件放在 ~/.roo/rules/ 目录下，如 ~/.roo/rules/lrc-memory.md
            "roo-code" => Some(".roo/rules/lrc-memory.md"),
            "comate" => Some(".comate/rules.md"),
            // CLI 工具类 — 全局规则文件
            "gemini-cli" => Some(".gemini/GEMINI.md"),      // v0.5.6 新增
            "aider" => Some(".aider/CONVENTIONS.md"),        // v0.5.6 新增
            // VS Code + Copilot — 使用全局 instructions 文件
            "vscode" => Some(".vscode/copilot-instructions.md"),
            // JetBrains AI — 使用全局规则文件
            "jetbrains-ai" => Some(".jetbrains/ai-instructions.md"),  // v0.5.6 修复：ID 从 "jetbrains" 改为 "jetbrains-ai"
            _ => None, // 其他工具暂不支持规则文件自动配置
        }
    }

    /// v0.5.5 修复：生成 AI 规则文件内容
    ///
    /// 规则文件告诉 AI 助手在什么情况下调用 LRC 记忆工具。
    /// 内容根据工具类型生成适配的格式。
    ///
    /// v0.5.5 修复内容：
    /// - Trae: 添加 YAML frontmatter（`alwaysApply: true`），确保规则在所有对话中始终生效
    /// - Cursor: 添加 MDC frontmatter（`alwaysApply: true`），确保规则始终生效
    fn generate_ai_rules_content(tool_id: &str) -> String {
        // v0.5.5 增强：傻瓜式 AI 规则自动注入
        // 用户无需手动配置任何 rule，LRC 配置 MCP 时自动写入
        // 规则核心：会话开始必须 recall + 任务感知 recall + 完成后自动同步

        // v0.5.5 修复：为 Trae 和 Cursor 添加 frontmatter，确保规则始终生效
        let frontmatter = match tool_id {
            "trae" | "trae-cn" => "---\nalwaysApply: true\ndescription: LRC 记忆系统规则 — 会话开始时先 recall 检索项目记忆（新用户自动降级），完成任务后自动同步记忆库\n---\n\n",
            "cursor" => "---\ndescription: LRC 记忆系统规则 — 会话开始时先 recall，新用户自动降级，完成任务后自动同步记忆\nalwaysApply: true\n---\n\n",
            _ => "",
        };

        let header = match tool_id {
            "cursor" => "# Cursor AI Rules — LRC 记忆系统自动配置",
            "trae" | "trae-cn" => "# Trae AI Rules — LRC 记忆系统自动配置",
            "codebuddy" => "# CodeBuddy Rules — LRC 记忆系统自动配置",
            "cline" => "# Cline Rules — LRC 记忆系统自动配置",
            "windsurf" => "# Windsurf Rules — LRC 记忆系统自动配置",
            "roo-code" => "# Roo Code Rules — LRC 记忆系统自动配置",  // v0.5.7 修复：ID 从 "roo" 改为 "roo-code"
            "vscode" => "# GitHub Copilot Instructions — LRC 记忆系统自动配置",
            "comate" => "# Comate Rules — LRC 记忆系统自动配置",
            _ => "# AI Rules — LRC 记忆系统自动配置",
        };

        format!(
            r#"{frontmatter}{header}
<!-- 本文件由 LRC Desktop v0.5.9 自动生成，请勿手动删除 LRC 相关规则 -->
<!-- 如需自定义规则，请在本文件末尾添加 -->

## LRC 记忆系统（Loong Recall Code Memory）

你已连接 LRC 记忆系统——一个本地代码记忆库。**请严格遵循以下规则，主动使用记忆工具。** 这不是可选项，而是你工作流程的一部分。

---

### 规则 1：会话开始时先 recall 检索（新用户自动降级）

**在回复用户的任何问题之前，必须先调用 recall 检索项目记忆，获取已有上下文。** 这是强制性的第一步，不可跳过。

```
// 第一步：检索项目架构概览
recall(query="项目架构 模块组织 入口文件", top_k=5)

// 第二步：根据用户任务，提取关键词检索专题记忆
recall(query="<任务相关的关键词，如：路由 API 认证 数据库>", top_k=3)
```

**新用户降级策略**：如果记忆库为空（首次安装），recall 会返回"无匹配结果"，此时直接进入正常工作流程，并在完成任务后通过 `remember` 开始积累记忆。后续会话中 recall 即可命中历史记忆。

**只有当 recall 返回的结果不足以完成任务时，才读取源文件。** 这能减少重复读取源文件导致的上下文溢出。

---

### 规则 2：遇到不确定的模块时优先 recall

当需要了解某个模块、函数、概念或实现逻辑时，**优先使用 recall 检索记忆**，而不是直接读取源文件：

```
recall(query="<模块名或关键词>", top_k=3)
```

如果 recall 无相关结果，再读取源文件。读取源文件后，如果发现了重要的架构信息，应调用 remember 记录下来，供后续会话使用。

---

### 规则 3：完成任务后自动同步记忆

每次完成代码修改任务后，**自动检查并同步记忆库**，无需用户提醒。流程：

1. 回顾本次修改了哪些文件/模块
2. 对照下表判断是否需要同步：

| 本次做了什么 | 记忆同步操作 |
|---|---|
| 新增了模块/文件（>100 行） | `remember` 写入新记忆，描述模块职责和入口函数 |
| 修改了已有模块的职责或入口函数 | `update_memory` 更新对应记忆的 content |
| 重命名了文件或函数 | `update_memory` 更新记忆中的路径和函数名 |
| 删除了模块/文件 | `forget` 删除对应记忆 |
| 新增了 API 端点 | `remember` 写入新记忆，标注路由路径和 handler |
| 修改了 API 端点的路径或方法 | `update_memory` 更新对应记忆 |
| 修改了项目配置（依赖、构建等） | `update_memory` 更新依赖列表记忆 |
| 修改了架构级别的东西（新增分层、引擎等） | `remember` 写入新记忆，并检查是否需要更新架构总览 |
| 纯 bug 修复（不改变结构） | 无需同步 |

3. 向用户报告同步结果（一句话即可，如"已记录限流模块到记忆库"或"本次无需同步记忆"）

---

### 记忆工具说明

| 工具 | 用途 | 关键参数 |
|---|---|---|
| `remember` | 记录新记忆 | content（内容）、memory_type（类型）、tags（标签）、importance（重要性 1-10） |
| `recall` | 语义检索历史记忆 | query（自然语言查询）、top_k（返回数量，建议 3-5） |
| `update_memory` | 更新已有记忆 | memory_id（记忆 ID）、content（新内容） |
| `forget` | 删除记忆 | memory_id（记忆 ID） |
| `list_memories` | 列出记忆库 | 支持分页、过滤、排序 |

**记忆类型**：
- `code_context` — 代码位置和结构（如"路由模块位于 routes.rs，核心函数 setup_routes()"）
- `decision` — 架构决策（如"选择 PostgreSQL 因为需要事务支持"）
- `preference` — 约定偏好（如"用户偏好使用 pnpm 而非 npm"）
- `fact` — 事实信息（如"数据库连接字符串在 .env 文件中"）

---

### 最佳实践

- **recall 的 top_k 限制为 3-5**：不让搜索结果占用过多上下文窗口
- **recall 失败时降级**：如果记忆检索无结果，再读取源文件
- **不要记住一切**：只记住关键的架构信息、入口点、数据流，不记细枝末节
- **记录时包含足够的上下文**：文件路径、函数名、关键概念
- **使用标签分类**：如 ["architecture", "database", "api"]
- **重要信息设置较高 importance**（1-10，默认 5）
- **同步是任务的一部分**：完成代码修改后，同步记忆不是额外步骤，而是任务的自然收尾

---

### 示例

**新用户首次使用（记忆库为空）**：
```
用户：帮我新增一个用户登录的 API
AI：（先 recall）recall(query="认证 登录 API 路由 auth", top_k=3)
AI：（recall 返回空）记忆库为空，直接读取源文件分析项目结构...
AI：已完成用户登录 API，新增了 POST /api/auth/login 端点
AI：（自动同步）remember(content="用户登录 API：POST /api/auth/login → AuthHandler::login，使用 JWT 签发 token", memory_type="code_context", tags=["auth", "api", "login"], importance=7)
AI：已记录登录 API 到记忆库（首次记忆）
```

**后续会话（记忆库已有数据）**：
```
用户：帮我新增一个用户登录的 API
AI：（先 recall）recall(query="认证 登录 API 路由 auth", top_k=3)
AI：（根据 recall 结果）根据记忆，项目使用 JWT 认证，路由在 routes.rs...
```

**完成任务后**：
```
AI：已完成用户登录 API，新增了 POST /api/auth/login 端点
AI：（自动同步）remember(content="用户登录 API：POST /api/auth/login → AuthHandler::login，使用 JWT 签发 token", memory_type="code_context", tags=["auth", "api", "login"], importance=7)
AI：已记录登录 API 到记忆库
```
"#
        )
    }

    /// v0.5.6 重构：写入或合并 AI 全局规则文件
    ///
    /// 在用户主目录写入 AI 规则文件，告知 AI 何时使用记忆工具。
    /// 规则文件是全局的，一次配置对所有项目生效。
    /// 如果文件已存在，在末尾追加 LRC 规则（不覆盖用户自定义内容）。
    /// v0.5.5 增强：检测旧版本规则并自动升级（保留用户自定义内容）
    /// v0.5.6 重构：从项目级改为全局级，使用用户主目录
    fn write_ai_rules(tool_id: &str) -> Result<(), String> {
        let template = match Self::get_rules_file_template(tool_id) {
            Some(t) => t,
            None => return Ok(()), // 该工具不支持规则文件，静默跳过
        };

        // v0.5.6：使用用户主目录作为全局规则的基础路径
        let home_dir = dirs::home_dir().ok_or_else(|| {
            let msg = "无法获取用户主目录（USERPROFILE 环境变量未设置）";
            tracing::error!("[AI规则] {} — {}", tool_id, msg);
            msg.to_string()
        })?;

        let rules_path = home_dir.join(template);

        // v0.5.6：清理旧的项目级规则文件（如果存在）
        // 旧版写入项目目录的规则文件现在不再需要，但为避免冲突，仅记录日志
        let legacy_project_paths: Vec<&str> = match tool_id {
            "trae" | "trae-cn" => vec![".trae/rules.md"],
            "cursor" => vec![".cursorrules"],
            _ => vec![],
        };
        // 注意：旧的项目级路径无法在此清理（不知道项目目录），仅记录日志
        if !legacy_project_paths.is_empty() {
            tracing::debug!(
                "[AI规则] {} — 旧版项目级规则文件（{:?}）需要用户手动清理",
                tool_id,
                legacy_project_paths
            );
        }

        // v0.5.8 修复：清理 v0.5.7 写入的错误路径规则文件
        // v0.5.7 错误地将 Trae/Trae CN 的全局规则写入 ~/.trae-cn/rules/ 目录
        // 但 rules/ 是项目级规则目录，Trae CN 不会读取用户主目录下的 rules/ 目录
        // v0.5.8 修正为 ~/.trae-cn/user_rules/ 目录（用户级全局规则目录）
        let legacy_v057_paths: Vec<PathBuf> = match tool_id {
            "trae" => vec![home_dir.join(".trae").join("rules").join("lrc-memory.md")],
            "trae-cn" => vec![home_dir.join(".trae-cn").join("rules").join("lrc-memory.md")],
            _ => vec![],
        };
        for legacy_path in &legacy_v057_paths {
            if legacy_path.exists() {
                match std::fs::remove_file(legacy_path) {
                    Ok(()) => {
                        tracing::info!(
                            "[AI规则] {} — 已清理 v0.5.7 错误路径规则文件: {}",
                            tool_id,
                            legacy_path.display()
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[AI规则] {} — 清理 v0.5.7 错误路径规则文件失败: {} ({})",
                            tool_id,
                            legacy_path.display(),
                            e
                        );
                    }
                }
            }
        }

        // 确保父目录存在
        if let Some(parent) = rules_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!("创建规则文件目录失败: {} ({})", parent.display(), e)
            })?;
        }

        let lrc_rules = Self::generate_ai_rules_content(tool_id);

        // 如果文件已存在，检查是否已包含 LRC 规则
        if rules_path.exists() {
            let existing = std::fs::read_to_string(&rules_path).unwrap_or_default();
            if existing.contains("LRC 记忆系统") {
                // v0.5.9 增强：检测旧版本规则并自动升级（兼容 v0.5.5、v0.5.6、v0.5.7、v0.5.8）
                if !existing.contains("v0.5.9 自动生成") {
                    // 旧版本规则，需要升级
                    if let Some(pos) = existing.find("## LRC 记忆系统") {
                        let user_content = existing[..pos].trim_end();
                        let merged = if user_content.is_empty() {
                            lrc_rules.clone()
                        } else {
                            format!("{}\n\n{}", user_content, lrc_rules)
                        };
                        std::fs::write(&rules_path, merged).map_err(|e| {
                            format!("更新规则文件失败: {} ({})", rules_path.display(), e)
                        })?;
                        tracing::info!(
                            "[AI规则] {} — 已升级 LRC 规则到 v0.5.9 版本: {}",
                            tool_id,
                            rules_path.display()
                        );
                    } else {
                        let merged = format!("{}\n\n{}", existing.trim_end(), lrc_rules);
                        std::fs::write(&rules_path, merged).map_err(|e| {
                            format!("更新规则文件失败: {} ({})", rules_path.display(), e)
                        })?;
                        tracing::info!(
                            "[AI规则] {} — 已追加 LRC v0.5.9 规则到现有文件: {}",
                            tool_id,
                            rules_path.display()
                        );
                    }
                } else {
                    // 已是 v0.5.9 版本，跳过
                    tracing::info!(
                        "[AI规则] {} — 规则文件已是最新版本，跳过: {}",
                        tool_id,
                        rules_path.display()
                    );
                }
                return Ok(());
            }
            // 在已有内容末尾追加 LRC 规则
            let merged = format!("{}\n\n{}", existing.trim_end(), lrc_rules);
            std::fs::write(&rules_path, merged).map_err(|e| {
                format!("更新规则文件失败: {} ({})", rules_path.display(), e)
            })?;
            tracing::info!(
                "[AI规则] {} — 已追加 LRC 规则到现有文件: {}",
                tool_id,
                rules_path.display()
            );
        } else {
            // 创建新文件
            std::fs::write(&rules_path, &lrc_rules).map_err(|e| {
                format!("创建规则文件失败: {} ({})", rules_path.display(), e)
            })?;
            tracing::info!(
                "[AI规则] {} — 已创建全局规则文件: {}",
                tool_id,
                rules_path.display()
            );
        }

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

    /// TDD：测试 GenericMcpAgent 检测行为
    /// v0.5.6 修复：GenericMcpAgent::detect() 在 v0.5.3 中改为返回 false，
    ///   测试需同步更新，否则 cargo test 必定失败
    #[test]
    fn test_generic_mcp_always_available() {
        let detector = GenericMcpAgent;
        // v0.5.3 修复：GenericMcpAgent 不自动检测为已安装，需用户手动添加
        assert!(!detector.detect());
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