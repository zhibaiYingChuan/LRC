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
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

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

/// v0.8.0 "归一"：规则文件状态信息
///
/// 用于信任中心展示各 AI 工具的 LRC 规则写入状态，
/// 让用户能确认规则是否已正确写入。
#[derive(Debug, Clone, Serialize)]
pub struct RulesStatus {
    /// 工具 ID（如 "trae", "cursor"）
    pub tool_id: String,
    /// 规则文件绝对路径
    pub rules_path: String,
    /// 文件是否存在
    pub exists: bool,
    /// 文件中解析到的规则版本号（如 "0.8.0"）
    pub version: Option<String>,
    /// 是否需要更新（版本低于当前或文件不存在）
    pub needs_update: bool,
    /// 最后修改时间（UNIX 时间戳字符串）
    pub last_modified: Option<String>,
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
    /// v0.5.12 新增：可执行文件名列表（用于全盘扫描匹配，不区分大小写）
    /// Windows: .exe 文件名（如 "CodeBuddy.exe"）
    /// Linux/macOS: 可执行文件名（如 "codebuddy"）
    /// 当 binary_paths 为空时，通过扫描常见安装目录匹配这些文件名来检测
    exe_names: &'static [&'static str],
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
        exe_names: &["Trae.exe"],
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
        exe_names: &["Trae CN.exe"],
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
        exe_names: &["Cursor.exe"],
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
        exe_names: &["Code.exe"],
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
        exe_names: &["Windsurf.exe"],
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
        exe_names: &["Kiro.exe"],
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
        exe_names: &["claude.exe"],
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
        exe_names: &["gemini"],
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
        exe_names: &["codex"],
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
        exe_names: &["CodeBuddy.exe", "CodeBuddy CN.exe"],
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
        exe_names: &["comate.exe", "Comate.exe"],
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
        exe_names: &["tongyi-lingma.exe", "Lingma.exe"],
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
        exe_names: &["MarsCode.exe", "marscode.exe"],
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
        exe_names: &["CodeGeeX.exe", "codegeex.exe"],
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
        exe_names: &["tencent-ai-code.exe", "TencentAICode.exe"],
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
        exe_names: &["CodeArts.exe", "codearts-snap.exe"],
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
        exe_names: &[],
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
        exe_names: &[],
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
        exe_names: &[],
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
        exe_names: &[],
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
        exe_names: &["aider"],
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
        exe_names: &[],
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
        exe_names: &["amazon-q.exe", "AmazonQ.exe"],
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
        exe_names: &["tabby.exe", "Tabby.exe"],
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
        exe_names: &["idea64.exe", "pycharm64.exe", "webstorm64.exe", "clion64.exe", "goland64.exe"],
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
        exe_names: &["PearAI.exe", "pearai.exe"],
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
        exe_names: &["Zed.exe", "zed.exe"],
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
        exe_names: &["agent-browser.exe", "AgentBrowser.exe"],
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
        exe_names: &["opencode"],
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
        exe_names: &["z-brain.exe", "ZBrain.exe"],
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
        exe_names: &["functional-hub.exe", "FunctionalHub.exe"],
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
        exe_names: &["SillyTavern.exe", "sillytavern.exe"],
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
        exe_names: &["memorix.exe", "Memorix.exe"],
    },
    // ═══ v0.6.0 新增：补充遗漏的主流 AI 工具 ═══
    // Claude Code CLI — Anthropic 官方命令行工具（不同于 Claude Desktop）
    // 通过 npm install -g @anthropic-ai/claude-code 安装，命令为 claude
    KnownTool {
        id: "claude-code",
        name: "Claude Code CLI",
        icon: "💻",
        category: "cli",
        supports_mcp: true,
        primary_marker: ".claude",
        secondary_markers: &[".claude.json"],
        mcp_config_template: Some(".claude.json"),
        mcp_transport: "stdio",
        binary_paths: &[],
        exe_names: &["claude", "claude.exe"],
    },
    // Sublime Text — 老牌代码编辑器，通过插件支持 AI 功能
    KnownTool {
        id: "sublime-text",
        name: "Sublime Text",
        icon: "📝",
        category: "ide",
        supports_mcp: false,
        primary_marker: "AppData/Roaming/Sublime Text",
        secondary_markers: &["%APPDATA%/Sublime Text"],
        mcp_config_template: None,
        mcp_transport: "stdio",
        binary_paths: &[
            "%PROGRAMFILES%/Sublime Text/sublime_text.exe",
            "%LOCALAPPDATA%/Programs/Sublime Text/sublime_text.exe",
        ],
        exe_names: &["sublime_text.exe", "subl.exe"],
    },
    // Tabnine — 独立 AI 编码助手，有 VSCode 插件也有独立应用
    KnownTool {
        id: "tabnine",
        name: "Tabnine",
        icon: "🔢",
        category: "ai-assistant",
        supports_mcp: false,
        primary_marker: ".tabnine",
        secondary_markers: &["%APPDATA%/Tabnine"],
        mcp_config_template: None,
        mcp_transport: "stdio",
        binary_paths: &["%LOCALAPPDATA%/Programs/Tabnine/Tabnine.exe"],
        exe_names: &["Tabnine.exe", "tabnine.exe"],
    },
    // Qwen Code — 阿里通义千问 CLI 工具（与通义灵码不同）
    // 通过 npm install -g @qwen-code/qwen-code 安装，命令为 qwen
    KnownTool {
        id: "qwen-code",
        name: "Qwen Code (通义千问 CLI)",
        icon: "🌐",
        category: "cli",
        supports_mcp: true,
        primary_marker: ".qwen",
        secondary_markers: &[],
        mcp_config_template: Some(".qwen/settings.json"),
        mcp_transport: "stdio",
        binary_paths: &[],
        exe_names: &["qwen", "qwen-code", "qwen.exe"],
    },
    // Replit — 在线 IDE 的桌面端应用
    KnownTool {
        id: "replit",
        name: "Replit",
        icon: "🔄",
        category: "ide",
        supports_mcp: false,
        primary_marker: ".replit",
        secondary_markers: &["%APPDATA%/Replit"],
        mcp_config_template: None,
        mcp_transport: "stdio",
        binary_paths: &["%LOCALAPPDATA%/Programs/Replit/Replit.exe"],
        exe_names: &["Replit.exe", "replit.exe"],
    },
    // DeepSeek Coder — DeepSeek 的命令行编码助手
    KnownTool {
        id: "deepseek-coder",
        name: "DeepSeek Coder",
        icon: "🦈",
        category: "cli",
        supports_mcp: false,
        primary_marker: ".deepseek",
        secondary_markers: &[],
        mcp_config_template: None,
        mcp_transport: "stdio",
        binary_paths: &[],
        exe_names: &["deepseek", "deepseek-coder", "deepseek.exe"],
    },
    // v0.5.4 P2-20 修复：移除 loong-recall 条目
    // 原因：~/.loong-recall 是 LRC 桌面端自己的数据目录，不是独立的 AI 工具。
    //       将其作为独立工具检测会导致所有安装了 LRC 的用户都看到"Loong Recall 已安装"，
    //       这是误导性的。LRC 桌面端应用本身就是 LRC 的入口。
];

/// v0.8.0 "归一"：LRC 规则文件版本号
///
/// 用于规则文件的版本化管理和自动升级。
/// 当此版本号高于规则文件中的版本号时，自动升级规则内容。
/// 版本号遵循语义化版本规范（major.minor.patch）。
const LRC_RULES_VERSION: &str = "0.8.0";

/// v0.8.0 "归一"：从规则文件内容中解析版本号
///
/// 查找 `<!-- LRC_RULES_VERSION: x.y.z -->` 标记并提取版本号。
/// 兼容旧版本规则文件（无结构化标记时返回 None）。
fn parse_rules_version(content: &str) -> Option<String> {
    // 查找结构化版本标记
    let marker = "<!-- LRC_RULES_VERSION:";
    if let Some(pos) = content.find(marker) {
        let start = pos + marker.len();
        let rest = &content[start..];
        // 提取到 "-->" 为止的版本号
        if let Some(end) = rest.find("-->") {
            let version = rest[..end].trim();
            if !version.is_empty() {
                return Some(version.to_string());
            }
        }
    }

    // v0.8.0 兼容：旧版本规则文件（v0.5.12 格式）
    // 旧格式：<!-- 本文件由 LRC Desktop v0.5.12 自动生成 -->
    let legacy_marker = "LRC Desktop v";
    if let Some(pos) = content.find(legacy_marker) {
        let start = pos + legacy_marker.len();
        let rest = &content[start..];
        // 提取版本号（数字和点组成的字符串）
        let version: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        if !version.is_empty() {
            return Some(version);
        }
    }

    None
}

/// v0.8.0 "归一"：语义化版本比较
///
/// 将 "0.8.0" 和 "0.5.12" 这样的版本号比较大小。
/// 返回 std::cmp::Ordering::Less/Equal/Greater。
///
/// 比较规则：按 major.minor.patch 逐段比较数字大小。
/// 缺失的段视为 0（如 "0.8" 等于 "0.8.0"）。
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parse_ver = |s: &str| -> Vec<u32> {
        s.split('.')
            .filter_map(|part| part.parse::<u32>().ok())
            .collect()
    };

    let va = parse_ver(a);
    let vb = parse_ver(b);
    let len = va.len().max(vb.len());

    for i in 0..len {
        let na = va.get(i).copied().unwrap_or(0);
        let nb = vb.get(i).copied().unwrap_or(0);
        match na.cmp(&nb) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }

    std::cmp::Ordering::Equal
}

/// v0.6.0 新增：使用特殊检测器的工具 ID 列表
///
/// 这些工具有专用的检测器实现（如 TraeDetector、TraeCnDetector、
/// ClaudeDesktopDetector、GenericMcpAgent），不使用通用的 DotDirDetector。
///
/// 新增特殊检测器时，需在此列表中添加对应 ID，否则会导致同一工具被检测两次。
const SPECIAL_DETECTOR_IDS: &[&str] = &["trae", "trae-cn", "claude-desktop", "generic-mcp"];

// ════════════════════════════════════════════════════════════════
// v0.6.0 新增：supports_mcp=false 工具的手动配置指引
// ════════════════════════════════════════════════════════════════

/// 获取不支持 MCP 自动配置的工具的手动配置指引
///
/// 返回 None 表示该工具支持自动配置或无指引可用。
/// 返回 Some 包含配置文档（Markdown 格式），前端可展示给用户。
///
/// 指引内容规范：
///   - 配置文件路径
///   - 配置 JSON 模板（可直接复制）
///   - 官方文档链接
///   - 替代方案说明（如 REST API）
pub fn get_manual_config_guide(tool_id: &str) -> Option<&'static str> {
    match tool_id {
        // ── 国产 AI 编码助手 ──
        "tongyi-lingma" => Some(
            "通义灵码暂不支持 MCP 协议自动配置。\n\n\
             **替代方案**：\n\
             1. 使用通义灵码的 IDE 插件形式（VS Code / JetBrains 插件）\n\
             2. 在 IDE 中安装通义灵码插件后，可通过 IDE 的 MCP 配置间接使用 LRC\n\
             3. 官方文档：https://tongyi.aliyun.com/lingma\n\n\
             **配置示例**（如果你在 VS Code 中使用通义灵码插件）：\n\
             在项目根目录创建 `.vscode/mcp.json`，内容参考 VS Code 的 MCP 配置模板。"
        ),
        "marscode" => Some(
            "豆包 MarsCode 暂不支持 MCP 协议自动配置。\n\n\
             **替代方案**：\n\
             1. 使用 MarsCode 的 IDE 插件形式（VS Code 插件）\n\
             2. 在 VS Code 中安装 MarsCode 插件后，可通过 VS Code 的 MCP 配置间接使用 LRC\n\
             3. 官方文档：https://www.marscode.com\n\n\
             **配置示例**（如果你在 VS Code 中使用 MarsCode 插件）：\n\
             在项目根目录创建 `.vscode/mcp.json`，内容参考 VS Code 的 MCP 配置模板。"
        ),
        "codegeex" => Some(
            "智谱 CodeGeeX 暂不支持 MCP 协议自动配置。\n\n\
             **替代方案**：\n\
             1. 使用 CodeGeeX 的 IDE 插件形式（VS Code / JetBrains 插件）\n\
             2. 在 IDE 中安装 CodeGeeX 插件后，可通过 IDE 的 MCP 配置间接使用 LRC\n\
             3. 官方文档：https://codegeex.cn\n\n\
             **配置示例**（如果你在 VS Code 中使用 CodeGeeX 插件）：\n\
             在项目根目录创建 `.vscode/mcp.json`，内容参考 VS Code 的 MCP 配置模板。"
        ),
        "tencent-ai-code" => Some(
            "腾讯云 AI 代码助手暂不支持 MCP 协议自动配置。\n\n\
             **替代方案**：\n\
             1. 使用腾讯云 AI 代码助手的 IDE 插件形式（VS Code 插件）\n\
             2. 在 VS Code 中安装插件后，可通过 VS Code 的 MCP 配置间接使用 LRC\n\
             3. 官方文档：https://cloud.tencent.com/product/aco\n\n\
             **配置示例**（如果你在 VS Code 中使用腾讯云 AI 插件）：\n\
             在项目根目录创建 `.vscode/mcp.json`，内容参考 VS Code 的 MCP 配置模板。"
        ),
        "huawei-codearts" => Some(
            "华为 CodeArts Snap 暂不支持 MCP 协议自动配置。\n\n\
             **替代方案**：\n\
             1. 使用 CodeArts Snap 的 IDE 插件形式（VS Code / JetBrains 插件）\n\
             2. 在 IDE 中安装插件后，可通过 IDE 的 MCP 配置间接使用 LRC\n\
             3. 官方文档：https://www.huaweicloud.com/product/codeartside.html\n\n\
             **配置示例**（如果你在 VS Code 中使用 CodeArts Snap 插件）：\n\
             在项目根目录创建 `.vscode/mcp.json`，内容参考 VS Code 的 MCP 配置模板。"
        ),

        // ── 国际 AI 编码助手 ──
        "continue" => Some(
            "Continue.dev 支持通过 config.json 配置 MCP 服务器。\n\n\
             **配置文件路径**：`~/.continue/config.json`\n\n\
             **配置模板**（将以下内容添加到 config.json 的 `mcpServers` 数组中）：\n\
             ```json\n\
             {\n\
               \"name\": \"lrc-memory\",\n\
               \"transport\": {\n\
                 \"type\": \"streamingHttp\",\n\
                 \"url\": \"http://127.0.0.1:3099/mcp\"\n\
               }\n\
             }\n\
             ```\n\n\
             **官方文档**：https://docs.continue.dev/reference/Model%20Context%20Protocol"
        ),
        "cody" => Some(
            "Cody (Sourcegraph) 暂不支持 MCP 协议自动配置。\n\n\
             **替代方案**：\n\
             1. 使用 Cody 的 IDE 插件形式（VS Code / JetBrains 插件）\n\
             2. 在 IDE 中安装 Cody 插件后，可通过 IDE 的 MCP 配置间接使用 LRC\n\
             3. 官方文档：https://docs.sourcegraph.com/cody\n\n\
             **配置示例**（如果你在 VS Code 中使用 Cody 插件）：\n\
             在项目根目录创建 `.vscode/mcp.json`，内容参考 VS Code 的 MCP 配置模板。"
        ),
        "aider" => Some(
            "Aider 暂不原生支持 MCP 协议。\n\n\
             **替代方案**：\n\
             1. Aider 支持通过 `--read` 参数读取文件，可将 LRC 的记忆导出为文件供 Aider 读取\n\
             2. 使用 LRC 的 REST API（http://127.0.0.1:3099）手动集成\n\
             3. 官方文档：https://aider.chat\n\n\
             **配置示例**：\n\
             在终端运行 Aider 时，添加 LRC 导出的记忆文件：\n\
             `aider --read lrc-memory.md`"
        ),
        "augment" => Some(
            "Augment Code 暂不支持 MCP 协议自动配置。\n\n\
             **替代方案**：\n\
             1. 使用 Augment 的 IDE 插件形式（VS Code / JetBrains 插件）\n\
             2. 在 IDE 中安装 Augment 插件后，可通过 IDE 的 MCP 配置间接使用 LRC\n\
             3. 官方文档：https://www.augmentcode.com\n\n\
             **配置示例**（如果你在 VS Code 中使用 Augment 插件）：\n\
             在项目根目录创建 `.vscode/mcp.json`，内容参考 VS Code 的 MCP 配置模板。"
        ),
        "amazon-q" => Some(
            "Amazon Q Developer 暂不支持 MCP 协议自动配置。\n\n\
             **替代方案**：\n\
             1. 使用 Amazon Q 的 IDE 插件形式（VS Code / JetBrains 插件）\n\
             2. 在 IDE 中安装 Amazon Q 插件后，可通过 IDE 的 MCP 配置间接使用 LRC\n\
             3. 官方文档：https://aws.amazon.com/codewhisperer\n\n\
             **配置示例**（如果你在 VS Code 中使用 Amazon Q 插件）：\n\
             在项目根目录创建 `.vscode/mcp.json`，内容参考 VS Code 的 MCP 配置模板。"
        ),
        "tabnine" => Some(
            "Tabnine 暂不支持 MCP 协议自动配置。\n\n\
             **替代方案**：\n\
             1. 使用 Tabnine 的 IDE 插件形式（VS Code / JetBrains 插件）\n\
             2. 在 IDE 中安装 Tabnine 插件后，可通过 IDE 的 MCP 配置间接使用 LRC\n\
             3. 官方文档：https://www.tabnine.com\n\n\
             **配置示例**（如果你在 VS Code 中使用 Tabnine 插件）：\n\
             在项目根目录创建 `.vscode/mcp.json`，内容参考 VS Code 的 MCP 配置模板。"
        ),

        // ── CLI 工具 ──
        "codex-cli" => Some(
            "OpenAI Codex CLI 暂不支持 MCP 协议。\n\n\
             **替代方案**：\n\
             1. 使用 LRC 的 REST API（http://127.0.0.1:3099）手动集成\n\
             2. 通过 `codex --config` 参数传递配置\n\
             3. 官方文档：https://github.com/openai/codex\n\n\
             **配置示例**：\n\
             在终端运行 Codex CLI 时，可通过环境变量传递 LRC 端点：\n\
             `LRC_ENDPOINT=http://127.0.0.1:3099 codex`"
        ),
        "deepseek-coder" => Some(
            "DeepSeek Coder 暂不支持 MCP 协议。\n\n\
             **替代方案**：\n\
             1. 使用 DeepSeek API 直接集成（需配置 API Key）\n\
             2. 使用 LRC 的 REST API（http://127.0.0.1:3099）手动集成\n\
             3. 官方文档：https://www.deepseek.com\n\n\
             **配置示例**：\n\
             在终端运行 DeepSeek Coder 时，可通过环境变量传递 LRC 端点：\n\
             `LRC_ENDPOINT=http://127.0.0.1:3099 deepseek`"
        ),

        // ── IDE 类 ──
        "sublime-text" => Some(
            "Sublime Text 暂不支持 MCP 协议自动配置。\n\n\
             **替代方案**：\n\
             1. 安装 Sublime Text 的 AI 插件（如 Continue 或 Tabnine 插件）\n\
             2. 通过 AI 插件的 MCP 配置间接使用 LRC\n\
             3. 官方文档：https://www.sublimetext.com\n\n\
             **配置示例**：\n\
             如果你安装了 Continue 插件，参考 Continue 的配置指引。"
        ),
        "replit" => Some(
            "Replit 暂不支持 MCP 协议自动配置。\n\n\
             **替代方案**：\n\
             1. Replit 是在线 IDE，建议在本地使用 VS Code + Replit 插件\n\
             2. 在 VS Code 中安装 Replit 插件后，可通过 VS Code 的 MCP 配置间接使用 LRC\n\
             3. 官方文档：https://docs.replit.com\n\n\
             **配置示例**（如果你在 VS Code 中使用 Replit 插件）：\n\
             在项目根目录创建 `.vscode/mcp.json`，内容参考 VS Code 的 MCP 配置模板。"
        ),

        // ── 其他工具 ──
        "jetbrains-ai" => Some(
            "JetBrains AI Assistant 暂不支持 MCP 协议自动配置。\n\n\
             **替代方案**：\n\
             1. JetBrains IDE（IntelliJ IDEA / PyCharm / WebStorm 等）可通过插件支持 MCP\n\
             2. 安装 'MCP Server' 插件后，可在 IDE 设置中配置 MCP 服务器\n\
             3. 官方文档：https://www.jetbrains.com/ai\n\n\
             **配置示例**：\n\
             在 JetBrains IDE 中安装 MCP 插件后，添加 LRC 服务器：\n\
             URL: http://127.0.0.1:3099/mcp"
        ),
        "zed" => Some(
            "Zed 编辑器暂不支持 MCP 协议自动配置。\n\n\
             **替代方案**：\n\
             1. Zed 计划在未来版本支持 MCP，请关注官方更新\n\
             2. 目前可通过 Zed 的扩展插件系统手动集成 LRC\n\
             3. 官方文档：https://zed.dev\n\n\
             **配置示例**：\n\
             等待 Zed 官方 MCP 支持后，将在 LRC 中添加自动配置。"
        ),
        "pearai" => Some(
            "PearAI 暂不支持 MCP 协议自动配置。\n\n\
             **替代方案**：\n\
             1. PearAI 基于 VS Code，可参考 VS Code 的项目级 MCP 配置\n\
             2. 在项目根目录创建 `.pearai/mcp.json`（或 `.vscode/mcp.json`）\n\
             3. 官方文档：https://trypear.ai\n\n\
             **配置示例**：\n\
             在项目根目录创建 `.pearai/mcp.json`，内容参考 VS Code 的 MCP 配置模板。"
        ),
        "opencode" => Some(
            "OpenCode 暂不支持 MCP 协议自动配置。\n\n\
             **替代方案**：\n\
             1. 使用 LRC 的 REST API（http://127.0.0.1:3099）手动集成\n\
             2. 官方文档：https://github.com/opencode-ai/opencode\n\n\
             **配置示例**：\n\
             在 OpenCode 配置文件中添加 LRC 端点：\n\
             `LRC_ENDPOINT=http://127.0.0.1:3099`"
        ),
        "z-brain" | "functional-hub" | "sillytavern" | "memorix" => Some(
            "该工具暂不支持 MCP 协议自动配置。\n\n\
             **替代方案**：\n\
             1. 使用 LRC 的 REST API（http://127.0.0.1:3099）手动集成\n\
             2. 参考 LRC 官方文档了解 REST API 接口"
        ),

        // ── 支持 MCP 的工具返回 None ──
        _ => None,
    }
}

/// v0.6.0 新增：获取工具类别的扫描优先级（数值越小优先级越高）
///
/// 优先级规则：
///   1 (最高): ide — IDE 类工具（Trae/Cursor/VSCode/Windsurf/Kiro/JetBrains等）
///   2:        desktop — 桌面应用类（Claude Desktop/Gemini CLI等）
///   3:        cli — CLI 工具类（Codex CLI/Aider/Claude Code CLI等）
///   4:        ai-assistant — AI 编码助手类（CodeBuddy/Comate/Continue等）
///   5:        browser — 浏览器类（Agent Browser等）
///   6 (最低): custom — 自定义/未知工具
fn category_scan_priority(category: &str) -> u8 {
    match category {
        "ide" => 1,
        "desktop" => 2,
        "cli" => 3,
        "ai-assistant" => 4,
        "browser" => 5,
        "custom" => 6,
        _ => 7,
    }
}

// ── 通用扫描函数 ──

/// 扫描条目数上限（v0.5.12：从 200 增加到 5000，支持 SpaceSniffer 式全盘扫描）
const MAX_SCAN_ENTRIES: usize = 5000;

/// 扫描包含指定标记的项目目录
///
/// v0.5.12 重新设计：采用 SpaceSniffer 式递归扫描
///   - 递归扫描所有目录（最大深度 5 层）
///   - 跳过系统目录和依赖目录（Windows、Program Files、node_modules 等）
///   - 检测每个目录是否包含 IDE 项目标记（如 .trae、.codebuddy）
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

        // v0.5.12：使用 walkdir 递归扫描，类似 SpaceSniffer
        for entry in walkdir::WalkDir::new(root)
            .max_depth(5)
            .into_iter()
            .filter_entry(|e| !is_scan_ignored_dir(e.path()))
            .filter_map(|e| e.ok())
        {
            // 扫描条目数上限（防止大磁盘扫描过慢）
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
    projects.sort_by(|a, b| a.path.cmp(&b.path));
    projects.dedup_by(|a, b| a.path == b.path);
    projects
}

/// 获取项目扫描的根目录列表
///
/// v0.5.12 重新设计：SpaceSniffer 式扫描所有驱动器
///   - 扫描所有可用驱动器根目录（C:\, D:\, G:\ 等）
///   - 递归扫描由 scan_marker_projects 处理
///   - 跳过系统目录由 is_scan_ignored_dir 处理
fn scan_roots() -> Vec<PathBuf> {
    available_drives()
}

/// 获取所有可用的驱动器根路径（如 C:\, D:\, G:\ 等）
fn available_drives() -> Vec<PathBuf> {
    let mut drives = Vec::new();
    // Windows: 检查 A-Z 所有驱动器
    for letter in b'A'..=b'Z' {
        let drive = format!("{}:\\", letter as char);
        let path = PathBuf::from(&drive);
        if path.exists() {
            drives.push(path);
        }
    }
    drives
}

/// 判断目录是否应被扫描器忽略（递归扫描时跳过）
///
/// 跳过系统目录、依赖目录、构建产物等，减少扫描时间和内存占用。
fn is_scan_ignored_dir(path: &Path) -> bool {
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        let name_lower = name.to_lowercase();
        return matches!(
            name_lower.as_str(),
            // 系统目录
            "windows"
            | "program files"
            | "program files (x86)"
            | "programdata"
            | "$recycle.bin"
            | "system volume information"
            | "recovery"
            | "perflogs"
            | "msocache"
            | "config.msi"
            // 依赖目录
            | "node_modules"
            | ".cargo"
            | "vendor"
            | "bower_components"
            // 构建产物
            | "target"
            | "dist"
            | "build"
            | ".next"
            | ".nuxt"
            | ".output"
            | "__pycache__"
            | ".pytest_cache"
            | ".mypy_cache"
            | "bin"
            | "obj"
            // 版本控制
            | ".git"
            | ".svn"
            | ".hg"
            // IDE/工具缓存
            | ".idea"
            | ".vscode"
            | ".vs"
            // 其他
            | ".cache"
            | "coverage"
            | ".nyc_output"
        );
    }
    false
}

/// 获取用户主目录
fn home_dir() -> Option<PathBuf> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()
        .map(PathBuf::from)
}

/// 获取 AppData 目录
///
/// v0.6.0 跨平台支持：
///   - Windows: %APPDATA% (如 C:\Users\<user>\AppData\Roaming)
///   - macOS: ~/Library/Application Support
///   - Linux: ~/.config
fn appdata_dir() -> Option<PathBuf> {
    // Windows 优先使用 APPDATA 环境变量
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return Some(PathBuf::from(appdata));
        }
    }
    // v0.6.0：macOS/Linux 跨平台支持
    // 使用 dirs crate 获取平台标准的配置目录
    dirs::config_dir().or_else(|| home_dir().map(|h| h.join(".config")))
}

/// 获取 LocalAppData 目录
///
/// v0.6.0 跨平台支持：
///   - Windows: %LOCALAPPDATA% (如 C:\Users\<user>\AppData\Local)
///   - macOS: ~/Library/Application Support（macOS 不区分 Roaming/Local）
///   - Linux: ~/.local/share
fn local_appdata_dir() -> Option<PathBuf> {
    // Windows 优先使用 LOCALAPPDATA 环境变量
    #[cfg(target_os = "windows")]
    {
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            return Some(PathBuf::from(local_appdata));
        }
    }
    // v0.6.0：macOS/Linux 跨平台支持
    // macOS 的 data_dir 和 config_dir 都指向 ~/Library/Application Support
    // Linux 的 data_dir 指向 ~/.local/share
    dirs::data_dir().or_else(|| home_dir().map(|h| h.join(".local/share")))
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

/// v0.5.12 新增：在常见安装目录中扫描可执行文件
///
/// 用户建议：像 SpaceSniffer 一样扫描，通过 exe 文件名确定用户实际安装了哪些 AI 工具。
/// 此函数扫描常见安装目录（Programs、Program Files 等），匹配 exe_names 中的文件名。
///
/// 跨平台支持：
///   - Windows: 扫描 %LOCALAPPDATA%/Programs/*、%PROGRAMFILES%/*、%PROGRAMFILES(X86)%/*
///   - Linux: 扫描 /usr/local/bin、/usr/bin、/opt/*、~/.local/share/*、~/Applications
///   - macOS: 扫描 /Applications/*、~/Applications、/usr/local/bin
///
/// 参数：
///   - exe_names: 要匹配的可执行文件名列表（不区分大小写）
///
/// 返回：true 如果在任意安装目录中找到匹配的可执行文件
fn scan_exe_in_install_dirs(exe_names: &[&str]) -> bool {
    if exe_names.is_empty() {
        return false;
    }

    // 将 exe_names 转为小写，用于不区分大小写匹配
    let targets: Vec<String> = exe_names.iter().map(|n| n.to_lowercase()).collect();

    // 收集要扫描的根目录
    let scan_roots = collect_install_dirs();

    tracing::debug!(
        "[Agent检测] 扫描 {} 个安装目录，查找 {:?}",
        scan_roots.len(),
        exe_names
    );

    // 扫描每个根目录（最大深度 3 层，避免过深扫描）
    for root in &scan_roots {
        if !root.exists() {
            continue;
        }

        for entry in walkdir::WalkDir::new(root)
            .max_depth(3)
            .into_iter()
            .filter_entry(|e| !is_scan_ignored_dir(e.path()))
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            // 获取文件名（不区分大小写匹配）
            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                let file_name_lower = file_name.to_lowercase();

                // Windows: 直接匹配 .exe 文件
                #[cfg(target_os = "windows")]
                {
                    if targets.iter().any(|t| *t == file_name_lower) {
                        tracing::debug!(
                            "[Agent检测] 匹配到可执行文件: {}",
                            path.display()
                        );
                        return true;
                    }
                }

                // Linux/macOS: 匹配可执行文件名（无扩展名）
                #[cfg(not(target_os = "windows"))]
                {
                    // 移除扩展名后匹配
                    let stem = std::path::Path::new(&file_name_lower)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or(&file_name_lower);
                    if targets.iter().any(|t| {
                        let t_stem = std::path::Path::new(t)
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or(t);
                        *t_stem == *stem
                    }) {
                        // 检查文件是否有执行权限
                        if let Ok(metadata) = std::fs::metadata(path) {
                            use std::os::unix::fs::PermissionsExt;
                            if metadata.permissions().mode() & 0o111 != 0 {
                                tracing::debug!(
                                    "[Agent检测] 匹配到可执行文件: {}",
                                    path.display()
                                );
                                return true;
                            }
                        }
                    }
                }
            }
        }
    }

    false
}

/// 收集常见安装目录列表（跨平台）
fn collect_install_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    #[cfg(target_os = "windows")]
    {
        // Windows: 用户级安装目录
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            dirs.push(PathBuf::from(&local_appdata).join("Programs"));
        }
        // Windows: 系统级安装目录
        if let Ok(program_files) = std::env::var("ProgramFiles") {
            dirs.push(PathBuf::from(&program_files));
        }
        if let Ok(program_files_x86) = std::env::var("ProgramFiles(x86)") {
            dirs.push(PathBuf::from(&program_files_x86));
        }
        // Windows: AppData（部分工具安装在此）
        if let Ok(appdata) = std::env::var("APPDATA") {
            dirs.push(PathBuf::from(&appdata));
        }
    }

    #[cfg(target_os = "macos")]
    {
        // macOS: 应用程序目录
        dirs.push(PathBuf::from("/Applications"));
        if let Some(home) = home_dir() {
            dirs.push(home.join("Applications"));
        }
        dirs.push(PathBuf::from("/usr/local/bin"));
    }

    #[cfg(target_os = "linux")]
    {
        // Linux: 系统安装目录
        dirs.push(PathBuf::from("/usr/local/bin"));
        dirs.push(PathBuf::from("/usr/bin"));
        dirs.push(PathBuf::from("/opt"));
        if let Some(home) = home_dir() {
            dirs.push(home.join("Applications"));
            dirs.push(home.join(".local/share"));
        }
    }

    dirs
}

/// 检查标记路径是否存在（保留供未来扩展使用）
#[allow(dead_code)]
fn marker_exists(marker: &str) -> bool {
    resolve_marker(marker).is_some_and(|p| p.exists())
}

/// v0.5.12 新增：扫描桌面和开始菜单快捷方式，定位 AI 工具 exe
///
/// 用户建议：普通人安装程序后，桌面或开始菜单会有快捷方式（.lnk 文件）。
/// 通过解析快捷方式指向的目标路径，可以快速定位 AI 工具的实际安装位置，
/// 无论用户将工具安装在哪个磁盘或目录。
///
/// 扫描位置：
///   - 用户桌面（%USERPROFILE%\Desktop）
///   - 公共桌面（%PUBLIC%\Desktop）
///   - 用户开始菜单（%APPDATA%\Microsoft\Windows\Start Menu\Programs）
///   - 系统开始菜单（%ProgramData%\Microsoft\Windows\Start Menu\Programs）
///
/// 匹配方式：
///   - 读取 .lnk 文件的二进制内容
///   - 搜索 exe_names 中的文件名是否出现在 .lnk 文件中（UTF-16LE 和 ASCII 编码）
///   - .lnk 文件中目标路径通常以 UTF-16LE 编码存储
fn scan_shortcuts(exe_names: &[&str]) -> bool {
    if exe_names.is_empty() {
        return false;
    }

    let shortcut_dirs = collect_shortcut_dirs();
    if shortcut_dirs.is_empty() {
        return false;
    }

    // 将 exe_names 转为小写，用于不区分大小写匹配
    let targets: Vec<String> = exe_names.iter().map(|n| n.to_lowercase()).collect();

    for dir in &shortcut_dirs {
        if !dir.exists() {
            continue;
        }

        // 递归扫描快捷方式目录（最大深度 3 层）
        for entry in walkdir::WalkDir::new(dir)
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            // 只处理 .lnk 文件
            let is_lnk = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("lnk"))
                .unwrap_or(false);
            if !is_lnk {
                continue;
            }

            // 读取 .lnk 文件内容
            let content = match std::fs::read(path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // 搜索 exe_names 是否出现在 .lnk 文件中
            if search_exe_in_lnk(&content, &targets) {
                tracing::debug!(
                    "[Agent检测] 通过快捷方式定位到 AI 工具: {} -> {}",
                    path.display(),
                    exe_names.join("/")
                );
                return true;
            }
        }
    }

    false
}

/// 收集快捷方式扫描目录（桌面 + 开始菜单）
fn collect_shortcut_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    #[cfg(target_os = "windows")]
    {
        // 用户桌面
        if let Some(home) = home_dir() {
            dirs.push(home.join("Desktop"));
        }
        // 公共桌面
        if let Ok(public) = std::env::var("PUBLIC") {
            dirs.push(PathBuf::from(&public).join("Desktop"));
        }
        // 用户开始菜单
        if let Ok(appdata) = std::env::var("APPDATA") {
            dirs.push(PathBuf::from(&appdata).join("Microsoft/Windows/Start Menu/Programs"));
        }
        // 系统开始菜单
        if let Ok(programdata) = std::env::var("ProgramData") {
            dirs.push(PathBuf::from(&programdata).join("Microsoft/Windows/Start Menu/Programs"));
        }
    }

    #[cfg(target_os = "macos")]
    {
        // macOS: 应用程序目录（.app 包含快捷方式信息）
        dirs.push(PathBuf::from("/Applications"));
        if let Some(home) = home_dir() {
            dirs.push(home.join("Applications"));
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Linux: .desktop 文件目录
        if let Some(home) = home_dir() {
            dirs.push(home.join(".local/share/applications"));
        }
        dirs.push(PathBuf::from("/usr/share/applications"));
        dirs.push(PathBuf::from("/usr/local/share/applications"));
    }

    dirs
}

/// 在 .lnk 文件二进制内容中搜索 exe 文件名
///
/// .lnk 文件中目标路径通常以 UTF-16LE 编码存储，
/// 同时也检查 ASCII 编码以兼容不同格式。
fn search_exe_in_lnk(content: &[u8], targets: &[String]) -> bool {
    for target in targets {
        // 检查 ASCII 编码
        let ascii_bytes = target.as_bytes();
        if contains_subsequence(content, ascii_bytes) {
            return true;
        }

        // 检查 UTF-16LE 编码
        let utf16_bytes: Vec<u8> = target
            .encode_utf16()
            .flat_map(|c| c.to_le_bytes())
            .collect();
        if contains_subsequence(content, &utf16_bytes) {
            return true;
        }
    }
    false
}

/// 在 haystack 中搜索 needle 子序列（字节级匹配）
fn contains_subsequence(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

// ════════════════════════════════════════════════════════════════
// v0.5.12 性能优化：全局扫描缓存
// ════════════════════════════════════════════════════════════════
// 根因：每个工具都重复扫描安装目录和快捷方式目录（22 个工具 × 重复扫描 = 慢）
// 优化：一次性扫描所有目录，所有工具共享缓存，将总时间从 N×扫描 降低到 1×扫描

/// 扫描缓存（一次性扫描，所有工具共享）
struct ScanCache {
    /// 所有安装目录中的 exe 文件名（小写）
    exe_names: Vec<String>,
    /// 所有 .lnk 文件内容
    lnk_contents: Vec<Vec<u8>>,
}

/// 全局扫描缓存（首次调用时扫描，后续调用直接返回缓存）
static SCAN_CACHE: OnceLock<ScanCache> = OnceLock::new();

/// 获取扫描缓存（首次调用时扫描，后续调用直接返回缓存）
fn get_scan_cache() -> &'static ScanCache {
    SCAN_CACHE.get_or_init(|| {
        let exe_names = collect_all_exe_names();
        let lnk_contents = collect_all_lnk_contents();
        tracing::info!(
            "[Agent检测] 全局扫描缓存已建立: {} 个 exe 文件, {} 个 .lnk 文件",
            exe_names.len(),
            lnk_contents.len()
        );
        ScanCache {
            exe_names,
            lnk_contents,
        }
    })
}

/// 一次性扫描所有安装目录，收集所有 exe 文件名（小写）
fn collect_all_exe_names() -> Vec<String> {
    let mut exe_names = Vec::new();
    let scan_roots = collect_install_dirs();

    for root in &scan_roots {
        if !root.exists() {
            continue;
        }

        for entry in walkdir::WalkDir::new(root)
            .max_depth(3)
            .into_iter()
            .filter_entry(|e| !is_scan_ignored_dir(e.path()))
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                #[cfg(target_os = "windows")]
                {
                    if file_name.to_lowercase().ends_with(".exe") {
                        exe_names.push(file_name.to_lowercase());
                    }
                }

                #[cfg(not(target_os = "windows"))]
                {
                    if let Ok(metadata) = std::fs::metadata(path) {
                        use std::os::unix::fs::PermissionsExt;
                        if metadata.permissions().mode() & 0o111 != 0 {
                            exe_names.push(file_name.to_lowercase());
                        }
                    }
                }
            }
        }
    }

    exe_names
}

/// 一次性扫描所有快捷方式目录，收集所有 .lnk 文件内容
fn collect_all_lnk_contents() -> Vec<Vec<u8>> {
    let mut lnk_contents = Vec::new();
    let shortcut_dirs = collect_shortcut_dirs();

    for dir in &shortcut_dirs {
        if !dir.exists() {
            continue;
        }

        for entry in walkdir::WalkDir::new(dir)
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let is_lnk = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("lnk"))
                .unwrap_or(false);

            if is_lnk {
                if let Ok(content) = std::fs::read(path) {
                    lnk_contents.push(content);
                }
            }
        }
    }

    lnk_contents
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
    /// v0.5.12 重新设计：采用 exe 文件扫描检测，替代 dot 目录检测
    ///   根因：dot 目录检测会导致误报（如 .gemini、.trae 残留目录）
    ///         用户建议：像 SpaceSniffer 一样扫描 exe 文件确定实际安装的工具
    ///
    /// 策略（按优先级）：
    ///   1. 有 binary_paths 的工具 → 检测已知路径的二进制文件是否存在（最快）
    ///   2. 有 exe_names 的工具 → 扫描常见安装目录匹配可执行文件名（灵活）
    ///   3. 有 exe_names 的工具 → 扫描桌面和开始菜单快捷方式定位 exe（用户建议）
    ///   4. 以上均未匹配 → 不检测（返回 false，避免误报）
    fn check_known_tool(&self) -> bool {
        // 策略 1：检测已知路径的二进制文件（最快，最准确）
        if !self.tool.binary_paths.is_empty() {
            if binary_exists(self.tool.binary_paths) {
                tracing::debug!(
                    "[Agent检测] {} — 通过 binary_paths 检测到",
                    self.tool.name
                );
                return true;
            }
        }

        // 策略 2 & 3：使用全局缓存（避免每个工具都重复扫描安装目录和快捷方式目录）
        // v0.5.12 性能优化：一次性扫描所有目录，所有工具共享缓存
        if !self.tool.exe_names.is_empty() {
            let cache = get_scan_cache();
            let targets: Vec<String> = self.tool.exe_names.iter().map(|n| n.to_lowercase()).collect();

            // 策略 2：在缓存的 exe 文件名中搜索
            if cache.exe_names.iter().any(|exe| targets.iter().any(|t| exe == t)) {
                tracing::debug!(
                    "[Agent检测] {} — 通过 exe_names 扫描检测到（缓存）",
                    self.tool.name
                );
                return true;
            }

            // 策略 3：在缓存的 .lnk 文件内容中搜索
            if cache.lnk_contents.iter().any(|content| search_exe_in_lnk(content, &targets)) {
                tracing::debug!(
                    "[Agent检测] {} — 通过快捷方式扫描检测到（缓存）",
                    self.tool.name
                );
                return true;
            }
        }

        tracing::debug!(
            "[Agent检测] {} — 未检测到可执行文件（binary_paths、exe_names、快捷方式均未匹配）",
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

/// Trae 专用检测器（仅检测 Trae 国际版）
///
/// v0.5.12 重新设计：移除 dot 目录检测，改用 exe 文件扫描
///   根因：dot 目录检测会导致误报（.trae 残留目录存在但用户未安装 Trae 国际版）
///   修复：仅检测 Trae.exe 可执行文件是否存在
struct TraeDetector;

impl AgentDetector for TraeDetector {
    fn detect(&self) -> bool {
        // v0.5.12：仅通过 exe 文件检测，避免 dot 目录误报
        // 策略 1：检测已知安装路径的 Trae.exe
        let binary_paths = &[
            "%LOCALAPPDATA%/Programs/Trae/Trae.exe",
            "%PROGRAMFILES%/Trae/Trae.exe",
        ];
        if binary_exists(binary_paths) {
            return true;
        }

        // 策略 2 & 3：使用全局缓存（避免重复扫描安装目录和快捷方式目录）
        {
            let cache = get_scan_cache();
            let targets: Vec<String> = vec!["trae.exe".to_string()];
            if cache.exe_names.iter().any(|exe| targets.iter().any(|t| exe == t)) {
                return true;
            }
            if cache.lnk_contents.iter().any(|content| search_exe_in_lnk(content, &targets)) {
                return true;
            }
        }

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
///
/// v0.5.12 重新设计：移除 dot 目录检测，改用 exe 文件扫描
///   根因：dot 目录检测会导致误报（.trae-cn 残留目录存在但用户未安装 Trae CN）
///   修复：仅检测 Trae CN.exe 可执行文件是否存在
struct TraeCNDetector;

impl AgentDetector for TraeCNDetector {
    fn detect(&self) -> bool {
        // v0.5.12：仅通过 exe 文件检测，避免 dot 目录误报
        // 策略 1：检测已知安装路径的 Trae CN.exe
        let binary_paths = &[
            "%LOCALAPPDATA%/Programs/Trae CN/Trae CN.exe",
            "%PROGRAMFILES%/Trae CN/Trae CN.exe",
        ];
        if binary_exists(binary_paths) {
            return true;
        }

        // 策略 2 & 3：使用全局缓存（避免重复扫描安装目录和快捷方式目录）
        {
            let cache = get_scan_cache();
            let targets: Vec<String> = vec!["trae cn.exe".to_string()];
            if cache.exe_names.iter().any(|exe| targets.iter().any(|t| exe == t)) {
                return true;
            }
            if cache.lnk_contents.iter().any(|content| search_exe_in_lnk(content, &targets)) {
                return true;
            }
        }

        false
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
        // v0.6.0：使用模块级常量 SPECIAL_DETECTOR_IDS 替代局部变量
        for tool in KNOWN_TOOLS {
            if !SPECIAL_DETECTOR_IDS.contains(&tool.id) {
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
        // v0.6.0 优化：按 category 优先级排序，IDE 和 Agent 工具优先返回
        // 优先级：ide(1) > desktop(2) > cli(3) > ai-assistant(4) > browser(5) > custom(6)
        let mut agents: Vec<AgentInfo> = self.detectors.iter().map(|d| d.info()).collect();
        agents.sort_by_key(|a| category_scan_priority(&a.category));
        agents
    }

    /// v0.5.4 新增：带进度回调的 Agent 检测
    ///
    /// 每检测完一个 Agent 就调用 on_progress 回调，
    /// 前端可据此显示"正在检测 Trae... (3/22)"的进度反馈。
    ///
    /// v0.6.0 优化：按 category 优先级排序检测，IDE 和 Agent 工具优先。
    /// 这样前端进度条会先显示"正在检测 Trae/Cursor/VSCode..."等高频工具，
    /// 再显示 AI 编码助手类工具，提升用户感知速度。
    pub fn detect_all_with_progress<F>(&self, on_progress: F) -> Vec<AgentInfo>
    where
        F: Fn(usize, usize, &AgentInfo),
    {
        let total = self.detectors.len();
        // v0.6.0：先收集所有 (detector, category_priority) 对，按优先级排序
        let mut indexed_detectors: Vec<(u8, &Box<dyn AgentDetector + Send + Sync>)> = self
            .detectors
            .iter()
            .map(|d| {
                let info = d.info();
                (category_scan_priority(&info.category), d)
            })
            .collect();
        indexed_detectors.sort_by_key(|(priority, _)| *priority);

        indexed_detectors
            .iter()
            .enumerate()
            .map(|(i, (_, d))| {
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
                    // v0.6.0 优化：为不支持 MCP 的工具提供手动配置指引
                    let guide = get_manual_config_guide(&info.id)
                        .unwrap_or("该工具暂不支持 MCP 协议，无配置指引可用。");
                    configured.push(format!(
                        "{} — 不支持 MCP 自动配置。手动配置指引：\n{}",
                        info.name, guide
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

    /// v0.8.0 "归一" 新增：获取所有支持规则文件的工具 ID 列表
    ///
    /// 用于桌面端 setup() 时自动写入规则，无需依赖 sidecar 启动。
    /// 返回 KNOWN_TOOLS 中所有定义了 rules_file_template 的工具 ID。
    pub fn get_all_rules_capable_tool_ids() -> Vec<String> {
        KNOWN_TOOLS
            .iter()
            .filter_map(|t| {
                if Self::get_rules_file_template(t.id).is_some() {
                    Some(t.id.to_string())
                } else {
                    None
                }
            })
            .collect()
    }

    /// v0.8.0 "归一" 新增：获取所有工具的规则文件状态
    ///
    /// 用于信任中心展示各 AI 工具的 LRC 规则写入状态。
    /// 检查每个工具的规则文件是否存在、版本是否最新。
    pub fn get_rules_status() -> Vec<RulesStatus> {
        let home_dir = match dirs::home_dir() {
            Some(h) => h,
            None => {
                tracing::warn!("[AI规则] 无法获取用户主目录，返回空状态列表");
                return Vec::new();
            }
        };

        KNOWN_TOOLS
            .iter()
            .filter_map(|t| {
                let template = Self::get_rules_file_template(t.id)?;
                let rules_path = home_dir.join(template);
                let exists = rules_path.exists();

                let (version, needs_update) = if exists {
                    let content = std::fs::read_to_string(&rules_path).unwrap_or_default();
                    let ver = parse_rules_version(&content);
                    let needs = match &ver {
                        Some(v) => {
                            compare_versions(v, LRC_RULES_VERSION) == std::cmp::Ordering::Less
                        }
                        None => true, // 无法解析版本，需要更新
                    };
                    (ver, needs)
                } else {
                    (None, true) // 文件不存在，需要创建
                };

                let last_modified = if exists {
                    std::fs::metadata(&rules_path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs().to_string())
                } else {
                    None
                };

                tracing::debug!(
                    "[AI规则] {} — exists={}, version={:?}, needs_update={}",
                    t.id,
                    exists,
                    version,
                    needs_update
                );

                Some(RulesStatus {
                    tool_id: t.id.to_string(),
                    rules_path: rules_path.display().to_string(),
                    exists,
                    version,
                    needs_update,
                    last_modified,
                })
            })
            .collect()
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
            // v0.5.12：传入 port 参数，用于检测端口自适应后 MCP 配置是否需要更新
            let needs_upgrade = self.config_needs_upgrade(&existing_content, port);
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
    fn config_needs_upgrade(&self, content: &str, expected_port: u16) -> bool {
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

        // v0.5.12 新增：检查 lrc-memory 配置的端口号是否正确
        // 解决问题：sidecar 端口自适应后，MCP 配置仍指向旧端口
        // 用户反馈："如果服务启动失败，自动更换了端口。那不是白配置了"
        if let Some(lrc_config) = servers.get("lrc-memory") {
            if let Some(url) = lrc_config.get("url").and_then(|v| v.as_str()) {
                let expected_port_str = format!(":{}", expected_port);
                if !url.contains(&expected_port_str) {
                    tracing::info!(
                        "[自动升级] lrc-memory URL 端口不匹配: 配置={}, 期望端口={}",
                        url, expected_port
                    );
                    return true; // 端口号不匹配，需要升级
                }
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
            // v0.5.12 修复：CodeBuddy 全局规则目录为 ~/.codebuddy/rules/，文件格式为 .mdc
            // 修复前：写入 ~/.codebuddy/rules.md（单文件，CodeBuddy 不读取此位置）
            // 修复后：写入 ~/.codebuddy/rules/lrc-memory.mdc（全局规则目录，CodeBuddy 会读取）
            // 参考：用户反馈 CodeBuddy 全局规则路径为 C:\Users\<用户名>\.codebuddy\rules\
            //       .mdc 文件需要 frontmatter（description, alwaysApply, enabled）
            "codebuddy" => Some(".codebuddy/rules/lrc-memory.mdc"),
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
        // v0.5.12 修复：为 CodeBuddy 添加 .mdc frontmatter（description, alwaysApply, enabled）
        let frontmatter = match tool_id {
            "trae" | "trae-cn" => "---\nalwaysApply: true\ndescription: LRC 记忆系统规则 — 会话开始时先 recall 检索项目记忆（新用户自动降级），完成任务后自动同步记忆库\n---\n\n",
            "cursor" => "---\ndescription: LRC 记忆系统规则 — 会话开始时先 recall，新用户自动降级，完成任务后自动同步记忆\nalwaysApply: true\n---\n\n",
            "codebuddy" => "---\ndescription: LRC 记忆系统规则 — 会话开始时先 recall 检索项目记忆，完成任务后自动同步记忆库\nalwaysApply: true\nenabled: true\n---\n\n",
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
<!-- LRC_RULES_VERSION: 0.8.0 -->
<!-- LRC_RULES_GENERATED: 2026-07-29 -->
<!-- 本文件由 LRC Desktop v0.8.0 "归一" 自动生成，请勿手动删除 LRC 相关规则 -->
<!-- 如需自定义规则，请在本文件末尾添加 -->

## LRC 记忆系统（Loong Recall Code Memory）

你已连接 LRC 记忆系统——一个本地代码记忆库。**请严格遵循以下规则，主动使用记忆工具。** 这不是可选项，而是你工作流程的一部分。

---

### 数据安全承诺（v0.8.0 "归一"）

**数据统一存储位置**：`~/.loong-recall/global/data/`（桌面端默认全局模式）

- 桌面端使用全局模式（无项目指纹隔离），所有项目共享记忆库
- IDE 插件使用项目指纹模式，每个项目独立记忆库
- 数据操作（迁移/备份/恢复/导出/导入）自动记录到 `~/.loong-recall/data_operations.log`
- 备份文件存储在 `~/.loong-recall/backups/`，最多保留 4 个（自动清理最旧）
- 用户可通过信任中心查看数据位置、记忆数量、文件大小、最后备份时间

当用户询问数据存储位置时，请告知上述路径。当用户需要数据管理操作时，可建议使用信任中心的相关功能。

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

### v0.6.0~v0.8.0 新功能说明

**v0.6.0 合成引擎与道同构度**：
- 合成引擎（synthesize）：自动将碎片化记忆合成为高层抽象记忆，提升检索质量
- 道同构度调节器（dao-regulator）：评估记忆一致性，自动调节合成策略
- 探索日志（exploration_log）：记录关键方法的执行轨迹，便于调试和优化
- 本地语义模型：默认 `BAAI/bge-small-zh`，通过 `--embedding-model` 配置

**v0.7.0 洛书向量编码**：
- 洛书向量编码器（/v1/encode）：将代码转换为语义向量，支持相似度计算
- 船长日志（/v1/captains-log）：记录系统运行状态和关键决策
- 版本检查（/v1/version/check）：检查 LRC 是否有新版本

**v0.8.0 数据治理**：
- 数据迁移（POST /v1/migrate）：合并旧版本数据到统一存储位置
- 手动备份（POST /v1/backup）：创建数据快照
- 列出备份（GET /v1/backups）：查看可用备份列表
- 操作日志（GET /v1/data-logs）：查看数据操作历史
- 全局模式默认：桌面端默认使用全局模式，所有项目共享记忆库

当用户询问这些功能时，请主动调用对应的 MCP 工具或建议用户通过信任中心使用。

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

        // v0.8.0 "归一"：基于版本号的规则升级逻辑
        // 替代旧的字符串匹配（"v0.5.12 自动生成"），实现语义化版本比较
        if rules_path.exists() {
            let existing = std::fs::read_to_string(&rules_path).unwrap_or_default();

            // 解析现有规则文件的版本号
            let file_version = parse_rules_version(&existing);

            match file_version {
                Some(ref ver) => {
                    // 版本比较：当前版本 >= LRC_RULES_VERSION → 跳过（幂等）
                    if compare_versions(ver, LRC_RULES_VERSION) != std::cmp::Ordering::Less {
                        tracing::info!(
                            "[AI规则] {} — 规则文件版本 {} 已是最新（{}），跳过: {}",
                            tool_id, ver, LRC_RULES_VERSION, rules_path.display()
                        );
                        return Ok(());
                    }

                    // 版本较低 → 需要升级
                    tracing::info!(
                        "[AI规则] {} — 规则文件版本 {} 低于当前版本 {}，开始升级: {}",
                        tool_id, ver, LRC_RULES_VERSION, rules_path.display()
                    );

                    // v0.8.0 安全措施：升级前备份旧文件到 .bak
                    let bak_path = rules_path.with_extension("md.bak");
                    if bak_path.extension().is_none() {
                        // 非 .md 文件（如 .mdc）的备份路径处理
                        let bak_path = format!("{}.bak", rules_path.display());
                        let _ = std::fs::copy(&rules_path, &bak_path);
                    } else {
                        let _ = std::fs::copy(&rules_path, &bak_path);
                    }

                    // 保留用户自定义内容（LRC 规则之外的部分）
                    let merged = if existing.contains("## LRC 记忆系统") {
                        // 提取 LRC 规则之前的用户内容
                        if let Some(pos) = existing.find("## LRC 记忆系统") {
                            // 往前找 frontmatter 或 LRC 头部注释
                            let user_end = pos;
                            // 查找 LRC 规则的起始位置（包括前面的注释和 frontmatter）
                            let lrc_start = existing
                                .find("# AI Rules — LRC")
                                .or_else(|| existing.find("# Trae AI Rules"))
                                .or_else(|| existing.find("# Cursor AI Rules"))
                                .or_else(|| existing.find("# CodeBuddy Rules"))
                                .or_else(|| existing.find("# Cline Rules"))
                                .or_else(|| existing.find("# Windsurf Rules"))
                                .or_else(|| existing.find("# Roo Code Rules"))
                                .or_else(|| existing.find("# GitHub Copilot"))
                                .or_else(|| existing.find("# Comate Rules"))
                                .unwrap_or(0);

                            // 也查找 frontmatter 起始
                            let frontmatter_start = if existing.starts_with("---\n") {
                                existing.find("\n---\n").map(|p| p + 5).unwrap_or(0)
                            } else {
                                lrc_start
                            };

                            let user_content = existing[..frontmatter_start].trim_end();
                            if user_content.is_empty() {
                                lrc_rules.clone()
                            } else {
                                format!("{}\n\n{}", user_content, lrc_rules)
                            }
                        } else {
                            // 有 "## LRC 记忆系统" 但找不到标题，全覆盖
                            lrc_rules.clone()
                        }
                    } else {
                        // 不包含 LRC 规则标记，可能是纯用户文件，追加 LRC 规则
                        format!("{}\n\n{}", existing.trim_end(), lrc_rules)
                    };

                    std::fs::write(&rules_path, &merged).map_err(|e| {
                        format!("升级规则文件失败: {} ({})", rules_path.display(), e)
                    })?;
                    tracing::info!(
                        "[AI规则] {} — 已升级规则文件到 v{}: {}",
                        tool_id, LRC_RULES_VERSION, rules_path.display()
                    );
                }
                None => {
                    // 版本号解析失败 → 降级为全覆盖（备份后写入）
                    tracing::warn!(
                        "[AI规则] {} — 无法解析规则文件版本，降级为全覆盖策略: {}",
                        tool_id, rules_path.display()
                    );

                    // 备份旧文件
                    let bak_path = format!("{}.bak", rules_path.display());
                    let _ = std::fs::copy(&rules_path, &bak_path);

                    // 检查是否包含 LRC 规则标记
                    if existing.contains("LRC 记忆系统") {
                        // 包含 LRC 标记但无法解析版本，全覆盖
                        std::fs::write(&rules_path, &lrc_rules).map_err(|e| {
                            format!("覆盖规则文件失败: {} ({})", rules_path.display(), e)
                        })?;
                        tracing::info!(
                            "[AI规则] {} — 已全覆盖写入规则文件: {}",
                            tool_id, rules_path.display()
                        );
                    } else {
                        // 不包含 LRC 标记，追加
                        let merged = format!("{}\n\n{}", existing.trim_end(), lrc_rules);
                        std::fs::write(&rules_path, &merged).map_err(|e| {
                            format!("追加规则文件失败: {} ({})", rules_path.display(), e)
                        })?;
                        tracing::info!(
                            "[AI规则] {} — 已追加 LRC 规则到现有文件: {}",
                            tool_id, rules_path.display()
                        );
                    }
                }
            }
        } else {
            // 创建新文件
            std::fs::write(&rules_path, &lrc_rules).map_err(|e| {
                format!("创建规则文件失败: {} ({})", rules_path.display(), e)
            })?;
            tracing::info!(
                "[AI规则] {} — 已创建全局规则文件 v{}: {}",
                tool_id, LRC_RULES_VERSION, rules_path.display()
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

    // ════════════════════════════════════════════════════════════
    // v0.8.0 "归一"：规则版本管理单元测试
    // ════════════════════════════════════════════════════════════

    #[test]
    fn test_parse_rules_version_structured() {
        let content = "<!-- LRC_RULES_VERSION: 0.8.0 -->\n# Rules";
        assert_eq!(parse_rules_version(content), Some("0.8.0".to_string()));
    }

    #[test]
    fn test_parse_rules_version_legacy_v0512() {
        let content = "<!-- 本文件由 LRC Desktop v0.5.12 自动生成 -->\n# Rules";
        assert_eq!(parse_rules_version(content), Some("0.5.12".to_string()));
    }

    #[test]
    fn test_parse_rules_version_no_version() {
        let content = "# My Custom Rules\nNo version info here";
        assert_eq!(parse_rules_version(content), None);
    }

    #[test]
    fn test_parse_rules_version_empty() {
        assert_eq!(parse_rules_version(""), None);
    }

    #[test]
    fn test_parse_rules_version_structured_with_spaces() {
        let content = "<!-- LRC_RULES_VERSION:   1.2.3  -->\n# Rules";
        assert_eq!(parse_rules_version(content), Some("1.2.3".to_string()));
    }

    #[test]
    fn test_compare_versions_equal() {
        assert_eq!(compare_versions("0.8.0", "0.8.0"), std::cmp::Ordering::Equal);
        assert_eq!(compare_versions("0.8", "0.8.0"), std::cmp::Ordering::Equal);
    }

    #[test]
    fn test_compare_versions_less() {
        assert_eq!(compare_versions("0.5.12", "0.8.0"), std::cmp::Ordering::Less);
        assert_eq!(compare_versions("0.7.99", "0.8.0"), std::cmp::Ordering::Less);
    }

    #[test]
    fn test_compare_versions_greater() {
        assert_eq!(compare_versions("0.8.0", "0.5.12"), std::cmp::Ordering::Greater);
        assert_eq!(compare_versions("1.0.0", "0.8.0"), std::cmp::Ordering::Greater);
    }

    #[test]
    fn test_lrc_rules_version_constant() {
        assert_eq!(LRC_RULES_VERSION, "0.8.0");
    }

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

    /// v0.5.12 临时测试：打印已安装的 AI 工具列表（用于本地验证 exe 文件扫描检测）
    #[test]
    fn test_print_installed_agents() {
        let registry = AgentDetectorRegistry::new();
        let installed = registry.detect_installed();
        println!("\n=== v0.5.12 AI 工具检测结果 ===");
        println!("已安装的工具数量: {}", installed.len());
        for agent in &installed {
            println!(
                "  - {} ({}) [MCP: {}] category: {}",
                agent.name, agent.id, agent.supports_mcp, agent.category
            );
        }
        let mcp_agents: Vec<_> = installed.iter().filter(|a| a.supports_mcp).collect();
        println!("\n支持 MCP 的已安装工具数量: {}", mcp_agents.len());
        for agent in &mcp_agents {
            println!("  - {} ({})", agent.name, agent.id);
        }
        println!("=== 检测结束 ===\n");
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

    // ════════════════════════════════════════════════════════════════
    // v0.6.0 新增测试：验证 AI 工具扫描优化功能
    // ════════════════════════════════════════════════════════════════

    /// v0.6.0 测试：验证新增的 AI 工具已添加到 KNOWN_TOOLS
    #[test]
    fn test_v6_new_tools_added() {
        let ids: Vec<_> = KNOWN_TOOLS.iter().map(|t| t.id).collect();
        // v0.6.0 新增的 6 个工具
        assert!(ids.contains(&"claude-code"), "缺少 claude-code 工具");
        assert!(ids.contains(&"sublime-text"), "缺少 sublime-text 工具");
        assert!(ids.contains(&"tabnine"), "缺少 tabnine 工具");
        assert!(ids.contains(&"qwen-code"), "缺少 qwen-code 工具");
        assert!(ids.contains(&"replit"), "缺少 replit 工具");
        assert!(ids.contains(&"deepseek-coder"), "缺少 deepseek-coder 工具");
        // 工具总数应至少 36 个（原 30 + 新增 6）
        assert!(
            KNOWN_TOOLS.len() >= 36,
            "工具总数 {} 少于预期 36",
            KNOWN_TOOLS.len()
        );
    }

    /// v0.6.0 测试：验证 get_manual_config_guide 为不支持 MCP 的工具返回指引
    #[test]
    fn test_manual_config_guide_for_non_mcp_tools() {
        // 不支持 MCP 的工具应返回配置指引
        assert!(get_manual_config_guide("tongyi-lingma").is_some());
        assert!(get_manual_config_guide("marscode").is_some());
        assert!(get_manual_config_guide("codegeex").is_some());
        assert!(get_manual_config_guide("continue").is_some());
        assert!(get_manual_config_guide("aider").is_some());
        assert!(get_manual_config_guide("sublime-text").is_some());
        assert!(get_manual_config_guide("tabnine").is_some());
        assert!(get_manual_config_guide("deepseek-coder").is_some());

        // 指引内容应包含关键信息
        let guide = get_manual_config_guide("continue").unwrap();
        assert!(guide.contains("config.json"), "Continue 指引应包含配置文件路径");
        assert!(guide.contains("127.0.0.1:3099"), "指引应包含 LRC 端点");
    }

    /// v0.6.0 测试：验证 get_manual_config_guide 对支持 MCP 的工具返回 None
    #[test]
    fn test_manual_config_guide_for_mcp_tools() {
        // 支持 MCP 的工具应返回 None
        assert!(get_manual_config_guide("trae").is_none());
        assert!(get_manual_config_guide("cursor").is_none());
        assert!(get_manual_config_guide("claude-desktop").is_none());
        assert!(get_manual_config_guide("windsurf").is_none());
    }

    /// v0.6.0 测试：验证 category_scan_priority 优先级正确
    #[test]
    fn test_category_scan_priority() {
        // IDE 类优先级最高
        assert_eq!(category_scan_priority("ide"), 1);
        // 桌面应用类
        assert_eq!(category_scan_priority("desktop"), 2);
        // CLI 工具类
        assert_eq!(category_scan_priority("cli"), 3);
        // AI 编码助手类
        assert_eq!(category_scan_priority("ai-assistant"), 4);
        // 浏览器类
        assert_eq!(category_scan_priority("browser"), 5);
        // 自定义类
        assert_eq!(category_scan_priority("custom"), 6);
        // 未知类别
        assert_eq!(category_scan_priority("unknown"), 7);
        // 验证优先级顺序
        assert!(category_scan_priority("ide") < category_scan_priority("ai-assistant"));
    }

    /// v0.6.0 测试：验证 detect_all 按 category 优先级排序
    #[test]
    fn test_detect_all_sorted_by_priority() {
        let registry = AgentDetectorRegistry::new();
        let agents = registry.detect_all();
        assert!(!agents.is_empty(), "检测结果不应为空");

        // 找到第一个 ide 类工具的位置
        let first_ide_pos = agents.iter().position(|a| a.category == "ide");
        let first_ai_assistant_pos = agents.iter().position(|a| a.category == "ai-assistant");

        // 如果同时存在 ide 和 ai-assistant 类工具，ide 应该排在前面
        if let (Some(ide_pos), Some(ai_pos)) = (first_ide_pos, first_ai_assistant_pos) {
            assert!(
                ide_pos < ai_pos,
                "IDE 类工具应排在 AI 助手类前面（ide位置: {}, ai-assistant位置: {}）",
                ide_pos,
                ai_pos
            );
        }
    }

    /// v0.6.0 测试：验证 SPECIAL_DETECTOR_IDS 常量正确
    #[test]
    fn test_special_detector_ids() {
        assert!(SPECIAL_DETECTOR_IDS.contains(&"trae"));
        assert!(SPECIAL_DETECTOR_IDS.contains(&"trae-cn"));
        assert!(SPECIAL_DETECTOR_IDS.contains(&"claude-desktop"));
        assert!(SPECIAL_DETECTOR_IDS.contains(&"generic-mcp"));
    }
}