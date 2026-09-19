//! 许可证: Apache 2.0
//!
//! MCP 协议服务端
//! ===============
//! 实现 Model Context Protocol (MCP) 服务端，通过 HTTP + JSON-RPC 2.0 暴露代码检索工具。
//! IDE 可通过 MCP 协议调用 search_code 工具，自动获取项目代码上下文。
//!
//! 协议参考: <https://spec.modelcontextprotocol.io/>
//! 当前暴露 search_code + codebase_stats 两个工具

use crate::memory_store::{
    relation_label, AssociatedMemory, ListFilter, MemoryStore, RecallFilter, SortBy, SortOrder,
};
use crate::persistence::json::JsonPersistence;
use crate::{
    ChunkStats, CodeMemoryManager, EntityKind, EventEntity, Importance, LlmApiConfig, Memory,
    MemoryType, PrivacyLevel, RecallResult, RetrievalResult,
};
// v0.9.7（GLOBAL_CODE_REVIEW_REPORT P1-6）：wizard.json 同步路径错误由不可判别的
// `String` 收敛为带域分类的 [`crate::errors::LrcError`]（io / parse / config / crypto）。
// `Display` 仅输出 message，故前端/日志文案**零漂移**。
use crate::errors::{LrcError, LrcResult};
use axum::{
    extract::State,
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Json},
    routing::{get, post},
    serve::ListenerExt,
    Router,
};
use serde::{Deserialize, Serialize};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
// ★符号层超时时的结果槽：临界区只有"写一个 Option"这一条非 await 语句，
//   用 std 同步锁即可（不跨 await 持锁，也不引入 tokio 异步锁的开销）。
use std::sync::Mutex as StdMutex;
use tokio::sync::{Mutex, RwLock};

const SEARCH_LOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
const SEARCH_EXECUTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

#[derive(Debug)]
pub enum SearchError {
    /// 等待检索锁超时（其他请求长时间占用）
    LockTimeout,
    /// 检索执行超时（阻塞任务超过上限）
    ExecutionTimeout,
    /// 检索过程 panic（已由 catch_unwind 隔离）
    Panic,
}

// v0.9.7 修复（GLOBAL_CODE_REVIEW_REPORT P1-6「错误处理不统一」）：
//   SearchError 此前仅 derive(Debug)，未实现 Display / std::error::Error，
//   导致调用方无法用 `?` 融入 `Box<dyn Error>` 生态，也无法打印人类可读原因。
//   此处补齐两个 trait，使其与 GuardError / EmbedError / DownloadError 口径一致。
impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LockTimeout => write!(
                f,
                "获取检索锁超时（等待超过 {}s）",
                SEARCH_LOCK_TIMEOUT.as_secs()
            ),
            Self::ExecutionTimeout => write!(
                f,
                "代码检索执行超时（超过 {}s），底层任务可能仍在占用线程",
                SEARCH_EXECUTION_TIMEOUT.as_secs()
            ),
            Self::Panic => write!(f, "代码检索过程发生 panic（已被隔离，服务继续可用）"),
        }
    }
}

impl std::error::Error for SearchError {}

/// 统一执行代码搜索，隔离锁等待、阻塞计算和搜索 panic。
pub async fn safe_code_search(
    manager: Arc<Mutex<Box<dyn IndexedCodebase>>>,
    keywords: Vec<String>,
    top_k: usize,
) -> Result<RetrievalResult, SearchError> {
    safe_code_operation(manager, move |manager| {
        manager.multi_keyword_search(&keywords, top_k)
    })
    .await
}

/// 统一执行不带查询条件的代码检索，保持与关键词搜索相同的保护边界。
pub async fn safe_recent_code_search(
    manager: Arc<Mutex<Box<dyn IndexedCodebase>>>,
    top_k: usize,
) -> Result<RetrievalResult, SearchError> {
    safe_code_operation(manager, move |manager| manager.recent_chunks(top_k)).await
}

async fn safe_code_operation<F>(
    manager: Arc<Mutex<Box<dyn IndexedCodebase>>>,
    operation: F,
) -> Result<RetrievalResult, SearchError>
where
    F: FnOnce(&dyn IndexedCodebase) -> RetrievalResult + Send + 'static,
{
    let guard = tokio::time::timeout(SEARCH_LOCK_TIMEOUT, manager.clone().lock_owned())
        .await
        .map_err(|_| SearchError::LockTimeout)?;

    let task = tokio::task::spawn_blocking(move || {
        std::panic::catch_unwind(AssertUnwindSafe(|| operation(guard.as_ref())))
            .map_err(|_| SearchError::Panic)
    });

    tokio::time::timeout(SEARCH_EXECUTION_TIMEOUT, task)
        .await
        .map_err(|_| SearchError::ExecutionTimeout)?
        .map_err(|_| SearchError::Panic)?
}

// ==================== JSON-RPC 2.0 类型 ====================

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    /// JSON-RPC 协议版本字段（恒为 "2.0"）。
    ///
    /// v0.9.7 核实：移除实验证明该 allow 非冗余（仍报 `field 'jsonrpc' is never read`）。
    /// 保留原因：该字段是 JSON-RPC 2.0 报文的一部分，`serde` 需能解析它，
    /// 但当前服务端不校验版本值，故读取方为零——属协议兼容字段而非死代码。
    #[allow(dead_code)]
    jsonrpc: String,
    #[serde(default)]
    id: Option<serde_json::Value>,
    method: String,
    #[serde(default)]
    params: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

// ==================== 统一 API 错误类型（v0.7.1 P2-3） ====================

/// HTTP API 统一错误类型
///
/// 提供 HTTP API 的错误响应标准格式，确保所有错误响应具有一致的结构。
/// 后续新增 handler 应优先使用此类型返回 `Result<T, ApiError>`，
/// 现有 handler 可逐步迁移至此类型。
#[derive(Debug)]
pub enum ApiError {
    /// 请求参数错误（400）
    BadRequest(String),
    /// 资源未找到（404）
    NotFound(String),
    /// 内部服务器错误（500）
    Internal(String),
    /// 服务不可用（503）
    ServiceUnavailable(String),
}

// v0.9.7 修复（GLOBAL_CODE_REVIEW_REPORT P1-6「错误处理不统一」）：
//   ApiError 此前只实现 IntoResponse（面向 HTTP 响应），未实现 Display / Error，
//   故无法作为 `Box<dyn Error>` 传播，也无法在日志中直接打印原因。
//   补齐后它与全仓其余错误类型口径一致；HTTP 响应路径保持不变。
impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadRequest(msg) => write!(f, "请求参数错误(400): {}", msg),
            Self::NotFound(msg) => write!(f, "资源未找到(404): {}", msg),
            Self::Internal(msg) => write!(f, "内部服务器错误(500): {}", msg),
            Self::ServiceUnavailable(msg) => write!(f, "服务不可用(503): {}", msg),
        }
    }
}

impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match self {
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
            ApiError::ServiceUnavailable(msg) => (StatusCode::SERVICE_UNAVAILABLE, msg),
        };
        (
            status,
            Json(serde_json::json!({
                "success": false,
                "error": message,
            })),
        )
            .into_response()
    }
}

// ==================== MCP 协议类型 ====================

#[derive(Debug, Serialize)]
#[allow(non_snake_case)]
struct InitializeResult {
    protocolVersion: String,
    capabilities: ServerCapabilities,
    serverInfo: ServerInfo,
}

#[derive(Debug, Serialize)]
struct ServerCapabilities {
    tools: ToolsCapability,
}

#[derive(Debug, Serialize)]
struct ToolsCapability {}

#[derive(Debug, Serialize)]
struct ServerInfo {
    name: String,
    version: String,
}

#[derive(Debug, Serialize)]
struct ToolsListResult {
    tools: Vec<ToolDefinition>,
}

#[derive(Debug, Serialize)]
struct ToolDefinition {
    name: String,
    description: String,
    #[serde(rename = "inputSchema")]
    input_schema: ToolInputSchema,
}

#[derive(Debug, Serialize)]
struct ToolInputSchema {
    #[serde(rename = "type")]
    schema_type: String,
    properties: serde_json::Value,
    required: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ToolCallResult {
    content: Vec<TextContent>,
}

#[derive(Debug, Serialize)]
struct TextContent {
    #[serde(rename = "type")]
    content_type: String,
    text: String,
}

// ==================== 代码库索引抽象 trait ====================

/// 已索引代码库的最小接口 — 服务端只关心检索和统计，不关心编码器类型
pub trait IndexedCodebase: Send {
    fn search(&self, query: &str, top_k: usize) -> RetrievalResult;
    fn multi_keyword_search(&self, keywords: &[String], top_k: usize) -> RetrievalResult;
    fn get_stats(&self) -> ChunkStats;
    /// v0.6.1 P0-2 修复: 获取最近索引的 N 条代码片段(用于空查询回退)
    fn recent_chunks(&self, top_k: usize) -> RetrievalResult;
}

// 为泛型 CodeMemoryManager<E> 自动实现 IndexedCodebase
impl<E: crate::engine::encoder::CodeEncoder> IndexedCodebase for CodeMemoryManager<E> {
    fn search(&self, query: &str, top_k: usize) -> RetrievalResult {
        CodeMemoryManager::search(self, query, top_k)
    }
    fn multi_keyword_search(&self, keywords: &[String], top_k: usize) -> RetrievalResult {
        CodeMemoryManager::multi_keyword_search(self, keywords, top_k)
    }
    fn get_stats(&self) -> ChunkStats {
        CodeMemoryManager::get_stats(self)
    }
    fn recent_chunks(&self, top_k: usize) -> RetrievalResult {
        CodeMemoryManager::recent_chunks(self, top_k)
    }
}

// ==================== 共享状态 ====================

/// 健康检查响应 — 提供详细的服务状态信息
///
/// 供桌面端 sidecar_manager 健康检查和仪表盘状态页面使用。
/// 包含服务运行阶段、索引进度、记忆库统计等关键信息。
#[derive(Debug, Serialize)]
struct HealthResponse {
    /// 服务状态: "running" | "indexing" | "starting"
    status: &'static str,
    /// 服务名称
    service: &'static str,
    /// 版本号
    version: &'static str,
    /// 已运行秒数
    uptime_seconds: i64,
    /// 索引状态
    indexing: IndexingStatus,
    /// 记忆库统计
    memory: MemoryBrief,
    /// 源码目录
    src_dir: String,
    /// 记忆数据目录（供桌面端 sidecar 身份校验使用）
    /// v0.9.6 修复 P0 契约断裂：桌面端 check_sidecar_health 读取此字段做
    /// 数据目录身份匹配，缺失会导致开发模式身份校验恒失败、误报端口占用。
    data_dir: String,
    /// LLM 是否已配置
    llm_configured: bool,
    /// v0.8.21 P0-06：memory_store 锁是否被持有（后台合成中）
    /// 前端据此判断 /v1/health/system 等 API 是否会返回 503 lock_busy
    /// true 时前端应显示"后台合成中"而非"服务未启动"
    #[serde(default)]
    lock_busy: bool,
}

#[derive(Debug, Serialize)]
struct IndexingStatus {
    /// 索引是否已完成
    complete: bool,
    /// 已索引文件数（索引完成后有效）
    #[serde(skip_serializing_if = "Option::is_none")]
    file_count: Option<usize>,
    /// 代码片段总数（索引完成后有效）
    #[serde(skip_serializing_if = "Option::is_none")]
    total_chunks: Option<usize>,
}

#[derive(Debug, Serialize)]
struct MemoryBrief {
    /// 记忆总数
    total: usize,
}

pub struct AppState {
    /// FIX-006: manager 保持 Mutex（dyn IndexedCodebase 不满足 Sync，无法用 RwLock）
    pub manager: Arc<Mutex<Box<dyn IndexedCodebase>>>,
    pub memory_store: Arc<Mutex<MemoryStore<JsonPersistence>>>,
    pub src_dir: String,
    /// 记忆数据目录（供 /health 暴露给桌面端做身份校验）
    pub data_dir: String,
    /// LLM API 配置（运行时可变，通过 /api/config/llm 动态更新）
    pub llm_api: Arc<RwLock<LlmApiConfig>>,
    /// v0.8.22 P0-1 修复（hcse-resilience-validator Round3）：
    ///   LLM 配置状态的无锁缓存，避免 /health 中 llm_api.read().await 阻塞 worker 线程
    ///   在 LLM 配置更新时同步更新此 AtomicBool
    pub llm_configured_atomic: Arc<AtomicBool>,
    /// 后台索引是否已完成（AtomicBool 支持无锁读取）
    pub indexing_complete: Arc<AtomicBool>,
    /// 服务启动时间（用于计算 uptime）
    pub started_at: chrono::DateTime<chrono::Utc>,
    /// v0.9.0: 是否为开发模式（--dev CLI 标志）
    pub dev_mode: bool,
}

// ==================== MCP 请求处理 ====================

/// 安全地将可序列化值转为 JSON Value，序列化失败时返回 Null 而非 panic
fn to_json_value_safe<T: Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
}

fn make_response(id: Option<serde_json::Value>, result: serde_json::Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: Some(result),
        error: None,
    }
}

fn make_error(id: Option<serde_json::Value>, code: i32, message: &str) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.into(),
        }),
    }
}

fn handle_initialize(id: Option<serde_json::Value>) -> JsonRpcResponse {
    let result = InitializeResult {
        protocolVersion: "2024-11-05".into(),
        capabilities: ServerCapabilities {
            tools: ToolsCapability {},
        },
        serverInfo: ServerInfo {
            name: "loong-recall".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
    };
    make_response(id, to_json_value_safe(&result))
}

fn handle_tools_list(id: Option<serde_json::Value>) -> JsonRpcResponse {
    let tools = vec![
        ToolDefinition {
            name: "remember".into(),
            description: "帮 AI 记住一件事——就像给 AI 装了个记事本。记住的内容会在后续对话中自动被检索到。【主动调用时机】1) 用户表达了技术偏好（'用 pnpm'）；2) 做了项目决策（'数据库选 PostgreSQL'）；3) 有重要的约定需要跨会话保留；4) 完成代码修改任务后，自动记录新增的模块、API 端点、架构变更等关键信息（这是任务的自然收尾，无需用户提醒）；5) **经历类内容**（一次外出/一次对话/一件事），此时应带 `event_id` 把同一次经历的多条记忆串起来。【关于 event_id】同一次经历产生的多条记忆填相同值（如一次杭州之行写了『游西湖』和『吃楼外楼』两条），它们之间就能互相联想——即使内容看上去毫不相关。这是『联想』的唯一依据，不填则这些记忆彼此孤立。".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "content": {
                        "type": "string",
                        "description": "记忆内容，如 '用户偏好使用 pnpm 而非 npm'"
                    },
                    "memory_type": {
                        "type": "string",
                        "description": "记忆类型: fact | preference | decision | code_context | conversation | experience",
                        "default": "fact"
                    },
                    "project": {
                        "type": "string",
                        "description": "关联项目名称（空=全局记忆）"
                    },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "标签列表，如 ['pnpm', 'tooling']"
                    },
                    "importance": {
                        "type": "integer",
                        "description": "重要性 1-10（默认 5）",
                        "default": 5
                    },
                    "ttl_days": {
                        "type": "integer",
                        "description": "存活天数（默认 0=永久）"
                    },
                    "privacy_level": {
                        "type": "string",
                        "description": "隐私级别: session | user | global（默认 user）",
                        "default": "user"
                    },
                    "session_id": {
                        "type": "string",
                        "description": "会话 ID（privacy_level=session 时使用）"
                    },
                    "user_id": {
                        "type": "string",
                        "description": "用户 ID（privacy_level=user 时使用）"
                    },
                    "event_id": {
                        "type": "string",
                        "description": "事件 ID — 这条记忆来自哪一次经历/事件。同一次经历产生的多条记忆使用**相同** event_id，即可建立『共同经历』关联（即使内容语义不相似）。【生成规则】由调用方（你）生成并复用，不要由系统猜：用『类型-对象-时间窗』构成可读 ID，如 `trip-hangzhou-2026-09`、`dinner-2026-09-16`、`task-fix-login-20260916`。规则：① 同一次对话/外出/任务内写入的多条记忆，全部用同一个值；② 换一次经历就换新值；③ 只写一次的记忆可不填（填了也无害）。【为什么重要】不填则这些记忆之间无法互相联想——它们的关联依据（同一次经历）从未被记录。"
                    },
                    "entities": {
                        "type": "array",
                        "description": "事件实体 — 这条记忆涉及的人/地/时/物，用于建立『共享实体』关联（跨经历的同一对象，如两条不同记忆都提到『爸爸』）。",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string", "description": "实体名称，如 '小美'、'海底捞'" },
                                "kind": { "type": "string", "description": "实体类型: person | place | time | thing | other", "default": "other" }
                            },
                            "required": ["name"]
                        }
                    },
                    "daoti_preview_gua": {
                        "type": "string",
                        "description": "可选：道体写入时预判的六十四卦名称"
                    },
                    "daoti_preview_bagua": {
                        "type": "string",
                        "description": "可选：道体写入时预判的主导八卦名称"
                    },
                    "daoti_preview_version": {
                        "type": "string",
                        "description": "可选：道体预判编码版本"
                    }
                }),
                required: vec!["content".into()],
            },
        },
        ToolDefinition {
            name: "batch_remember".into(),
            description: "批量记忆注入 — 一次性写入多条记忆，大幅提升大批量数据注入性能。适用于 LongMemEval 等需要注入大量会话历史的场景。单次最多 200 条。【共同经历】若这批记忆来自同一次经历，在**批次级**传一次 event_id 即可（无需逐条重复）；若批次内混有不同经历，则在对应条目上写各自的 event_id 覆盖。".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "event_id": {
                        "type": "string",
                        "description": "批次级事件 ID — 这批记忆同属一次经历时填一次即可（等价于给每条填相同的 event_id）。条目级 event_id 优先于此值。"
                    },
                    "memories": {
                        "type": "array",
                        "description": "记忆列表，每条记忆包含 content、memory_type、project、tags、importance 等字段",
                        "items": {
                            "type": "object",
                            "properties": {
                                "content": {
                                    "type": "string",
                                    "description": "记忆内容"
                                },
                                "memory_type": {
                                    "type": "string",
                                    "description": "记忆类型: fact | preference | decision | code_context | conversation | experience",
                                    "default": "fact"
                                },
                                "project": {
                                    "type": "string",
                                    "description": "关联项目名称"
                                },
                                "tags": {
                                    "type": "array",
                                    "items": { "type": "string" },
                                    "description": "标签列表"
                                },
                                "importance": {
                                    "type": "integer",
                                    "description": "重要性 1-10（默认 5）",
                                    "default": 5
                                },
                                "event_id": {
                                    "type": "string",
                                    "description": "事件 ID — 同一次经历产生的多条记忆使用相同 event_id（建立『共同经历』关联）"
                                },
                                "entities": {
                                    "type": "array",
                                    "description": "事件实体（人/地/时/物）",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "name": { "type": "string" },
                                            "kind": { "type": "string", "description": "person | place | time | thing | other" }
                                        },
                                        "required": ["name"]
                                    }
                                }
                            },
                            "required": ["content"]
                        }
                    }
                }),
                required: vec!["memories".into()],
            },
        },
        ToolDefinition {
            name: "recall".into(),
            description: "语义检索历史记忆。支持两种模式：fast（关键词匹配，默认）和 deep（深度语义检索，使用编码器 + 聚焦检索）。【主动调用时机】1) 会话开始时，先调用 recall 检索项目架构概览（query='项目架构 模块组织 入口文件'），获取已有上下文；2) 遇到不确定的模块/函数/概念时，优先 recall 而非直接读源文件；3) 用户开始新任务时，recall 相关专题记忆。只有 recall 结果不足时才读取源文件，以减少上下文溢出。".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "query": {
                        "type": "string",
                        "description": "自然语言查询，如 '用户的包管理器偏好'"
                    },
                    "top_k": {
                        "type": "integer",
                        "description": "返回结果数（默认 5，最大 100）",
                        "default": 5
                    },
                    "lrc_mode": {
                        "type": "string",
                        "description": "检索模式: fast（关键词匹配，默认）| deep（深度语义检索）",
                        "default": "fast"
                    },
                    "focus_depth": {
                        "type": "integer",
                        "description": "检索深度（仅 lrc_mode=deep 时生效）。0=全量检索，1=标准，2=深度。默认 1",
                        "default": 1
                    },
                    "memory_type": {
                        "type": "string",
                        "description": "按类型过滤: fact | preference | decision | code_context | conversation"
                    },
                    "project": {
                        "type": "string",
                        "description": "按项目过滤"
                    },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "按标签过滤"
                    },
                    "min_importance": {
                        "type": "integer",
                        "description": "最低重要性阈值（0-10）"
                    },
                    "navigation": {
                        "type": "object",
                        "description": "道体导航信号（可选，需服务端 LRC_DAOTI_NAVIGATE=1）。由道体状态机在查询推演产出的检索方向：{\"palaces\":[\"兑\",\"坤\"],\"probes\":[[\"吃\",\"餐厅\"],[\"出行\",\"徒步\"]],\"version\":\"daoti-v23-pilot\"}。probes 可选（与 palaces 同序的探测词组，缺省回退卦宫宫义）。仅 deep 模式生效，改变检索候选集而非重排；缺省行为与既有版本一致",
                        "properties": {
                            "palaces": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "轨迹卦宫名序列（最多 4 个，如 乾/兑/坤/艮/震/巽/坎/离）"
                            },
                            "probes": {
                                "type": "array",
                                "items": { "type": "array", "items": { "type": "string" } },
                                "description": "每个卦宫的探测词组（与 palaces 同序，可选）"
                            },
                            "version": { "type": "string", "description": "推演版本标识" }
                        }
                    }
                }),
                required: vec!["query".into()],
            },
        },
        ToolDefinition {
            name: "forget".into(),
            description: "删除一条记忆。【主动调用时机】当模块/文件被删除时，调用此工具删除对应的记忆，保持记忆库与代码同步。".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "memory_id": {
                        "type": "string",
                        "description": "要删除的记忆 ID"
                    }
                }),
                required: vec!["memory_id".into()],
            },
        },
        ToolDefinition {
            name: "update_memory".into(),
            description: "更新一条已有记忆的内容。【主动调用时机】1) 修改了已有模块的职责或入口函数时；2) 重命名了文件或函数时；3) 修改了 API 端点的路径或方法时；4) 修改了项目配置（依赖、构建等）时。先用 recall 找到对应记忆的 memory_id，再调用此工具更新。".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "memory_id": {
                        "type": "string",
                        "description": "要更新的记忆 ID"
                    },
                    "content": {
                        "type": "string",
                        "description": "新的记忆内容"
                    },
                    "importance": {
                        "type": "integer",
                        "description": "新的重要性（可选）"
                    }
                }),
                required: vec!["memory_id".into(), "content".into()],
            },
        },
        ToolDefinition {
            name: "list_memories".into(),
            description: "列出记忆库中的记忆，支持分页、过滤和排序。".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "memory_type": {
                        "type": "string",
                        "description": "按类型过滤"
                    },
                    "project": {
                        "type": "string",
                        "description": "按项目过滤"
                    },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "按标签过滤"
                    },
                    "sort_by": {
                        "type": "string",
                        "description": "排序字段: created_at | importance | last_accessed",
                        "default": "created_at"
                    },
                    "order": {
                        "type": "string",
                        "description": "排序方向: desc | asc",
                        "default": "desc"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "分页大小（默认 20）",
                        "default": 20
                    },
                    "offset": {
                        "type": "integer",
                        "description": "分页偏移（默认 0）",
                        "default": 0
                    }
                }),
                required: vec![],
            },
        },
        ToolDefinition {
            name: "memory_stats".into(),
            description: "获取记忆库的统计信息：总数、类型分布、项目分布。".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: vec![],
            },
        },
        ToolDefinition {
            name: "archive".into(),
            description: "归档过期记忆。将已过期的记忆从活跃记忆库迁移到冷存储，释放检索空间。返回归档的记忆数量。".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: vec![],
            },
        },
        ToolDefinition {
            name: "search_code".into(),
            description: "在项目代码中查找代码片段。输入你记得的任何信息：函数名、变量名、文件路径，或者自然语言描述（如「处理用户登录的代码在哪？」）。默认使用精确关键词匹配——零延迟、零下载，适合你知道函数名但懒得手动翻文件的场景。如果你编译时启用了语义模式（--features ml），则能理解模糊的自然语言描述。".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "query": {
                        "type": "string",
                        "description": "你想找什么？输入函数名（如 'authenticate_user'）、变量名、或者自然语言描述（如 '处理登录的代码'）"
                    },
                    "top_k": {
                        "type": "integer",
                        "description": "返回结果数量（默认 5，最大 20）",
                        "default": 5
                    }
                }),
                required: vec!["query".into()],
            },
        },
        ToolDefinition {
            name: "codebase_stats".into(),
            description: "获取代码库索引统计信息：文件数、片段数、类型分布等。".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: vec![],
            },
        },
        ToolDefinition {
            name: "system_health".into(),
            description: "系统健康监控 — 获取记忆系统的健康度指标：一致性评分、分布熵、合成比率、编码/检索/合成/修正次数。".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: vec![],
            },
        },
        ToolDefinition {
            name: "correct_memory".into(),
            description: "用户修正记忆 — 修正一条已结晶的记忆，保留修正历史。适用于用户手动纠正 AI 记忆中的错误或过时信息。".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "memory_id": {
                        "type": "string",
                        "description": "要修正的记忆 ID"
                    },
                    "content": {
                        "type": "string",
                        "description": "修正后的正确内容"
                    },
                    "reason": {
                        "type": "string",
                        "description": "修正原因（如 '用户手动修正'、'信息已过时'）"
                    }
                }),
                required: vec!["memory_id".into(), "content".into()],
            },
        },
        ToolDefinition {
            name: "recall_enhanced".into(),
            description: "双路检索增强 — 快速通路（关键词匹配）+ 深度通路（深度语义检索），通过倒数排名融合（RRF）合并结果。适用于需要深度背景的查询。".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "query": {
                        "type": "string",
                        "description": "自然语言查询"
                    },
                    "top_k": {
                        "type": "integer",
                        "description": "返回结果数（默认 5，最大 100）",
                        "default": 5
                    },
                    "memory_type": {
                        "type": "string",
                        "description": "按类型过滤"
                    },
                    "project": {
                        "type": "string",
                        "description": "按项目过滤"
                    },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "按标签过滤"
                    }
                }),
                required: vec!["query".into()],
            },
        },
        ToolDefinition {
            name: "associations".into(),
            description: "记忆关联 — 给出一条记忆，返回它在记录层上的**多类型关联**（结构化关系网络，而非相似度排序）。关系类型：same_event（同一次经历产生，依据 event_id）/ shared_entity（共享人/地/时/物）/ derived_from（由该记忆结晶衍生）。每条关联都带人类可读的『为什么关联』依据。用于回答『我记得 A，什么会让我想起 B』——依据是共同经历与共享实体，不是语义相似。".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "memory_id": {
                        "type": "string",
                        "description": "起点记忆 ID"
                    },
                    "relation": {
                        "type": "string",
                        "description": "只返回某类关联: same_event（手填 event_id）| same_event_auto（系统按同项目+同窗口推断）| shared_entity（共享实体）| derived_from（由它衍生）| crystallized_into（被结晶为它）| evolved_from（自身被更新过）（缺省=全部类型并存）"
                    }
                }),
                required: vec!["memory_id".into()],
            },
        },
        ToolDefinition {
            name: "association_graph".into(),
            description: "联想图 — 给出一条记忆，返回以它为中心的**关联图**（节点=记忆，边=有类型/有方向/有解释的关系）。与 associations 的区别：associations 只给一层直接关联，本工具会做**结构性多跳推理**——若 A 与 B 同一次经历、B 与 C 共享实体，则推出 A 与 C 的**间接关联**（即使 A 与 C 之间没有任何直接记录）。这不是语义相似度匹配，而是由记录推出的、必然成立的结构关系；间接关联会附完整路径，供人工核验。用于发现『用户自己没想到但合理』的关联。".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "memory_id": {
                        "type": "string",
                        "description": "起点记忆 ID（图的中心）"
                    },
                    "max_nodes": {
                        "type": "integer",
                        "description": "节点数上限（默认 50，范围 2~500）。超出时结果会标记「已被截断」"
                    }
                }),
                required: vec!["memory_id".into()],
            },
        },
    ];

    let result = ToolsListResult { tools };
    make_response(id, to_json_value_safe(&result))
}

/// 处理 recall_enhanced 工具调用 — 双路检索增强（RRF 倒数排名融合）
///
/// 快速通路（关键词匹配）+ 深度通路（语义检索）→ RRF 融合 → 归一化排序
async fn handle_recall_enhanced(
    state: &AppState,
    arguments: &serde_json::Value,
    id: Option<serde_json::Value>,
) -> JsonRpcResponse {
    let query = match arguments.get("query").and_then(|q| q.as_str()) {
        Some(q) => q,
        None => return make_error(id, -32602, "缺少参数: query"),
    };
    let top_k = arguments
        .get("top_k")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .clamp(1, 100) as usize;

    let memory_type = arguments
        .get("memory_type")
        .and_then(|v| v.as_str())
        .and_then(MemoryType::try_parse);

    let project = arguments
        .get("project")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let tags: Vec<String> = arguments
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // 先完成可能发生网络等待的 LLM 翻译，再获取 memory_store 锁。
    // 这样网络超时不会阻塞其他记忆读写请求。
    let llm_config = state.llm_api.read().await.clone();
    let enriched_query = if llm_config.is_configured() {
        let keywords =
            crate::engine::llm_translator::translate_memory_query(&llm_config, query).await;
        let translated: String = keywords.join(" ");
        if translated.is_empty() || translated.trim() == query {
            query.to_string()
        } else {
            format!("{} {}", translated, query)
        }
    } else {
        query.to_string()
    };

    // 双路检索 + RRF 融合移入 spawn_blocking，避免持锁阻塞 Tokio worker；
    // 锁获取采用有界 try_lock 轮询（2 秒），锁被长期占用时快速返回忙态，
    // 不再让 async 上下文无限等待全局 Store 锁。
    // RRF 权重不依赖锁，提前在闭包外计算。
    let (fast_weight, deep_weight) = crate::engine::rrf::query_path_weights(query);
    let store_arc = state.memory_store.clone();
    let enrich_query = enriched_query.clone();
    let mem_type = memory_type.clone();
    let proj = project.clone();
    let tag_list = tags.clone();
    // 联想补全用的过滤条件（v0.9.8）：与检索**同一套可见性规则**，
    // 由下面两个 filter 复用同一份字段，避免"检索过滤了、联想没过滤"的越权口子。
    let filter_for_expand = RecallFilter {
        memory_type: mem_type.clone(),
        project: proj.clone(),
        tags: tag_list.clone(),
        min_importance: None,
        top_k: top_k * 2,
        privacy_context: None,
        explore_pure: false,
        regression_query: None,
        read_only: false,
    };

    let retrieval_result = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        tokio::task::spawn_blocking(move || {
            // 有界锁获取：轮询 try_lock，2 秒未获得则放弃
            let lock_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            let mut store = loop {
                match store_arc.try_lock() {
                    Ok(guard) => break guard,
                    Err(_) if std::time::Instant::now() < lock_deadline => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(_) => {
                        eprintln!("[recall_enhanced] 锁获取超时（2s），返回忙态");
                        return None;
                    }
                }
            };

            // 快速通路：关键词匹配，使用富化查询
            let fast_filter = RecallFilter {
                memory_type: mem_type.clone(),
                project: proj.clone(),
                tags: tag_list.clone(),
                min_importance: None,
                top_k: top_k * 2,
                privacy_context: None,
                explore_pure: false,
                regression_query: None,
                read_only: false,
            };
            let fast_result = store
                .recall(&enrich_query, &fast_filter)
                .unwrap_or(RecallResult::basic(vec![], vec![], 0));

            // 深度通路：深度语义检索，使用富化查询
            let deep_filter = RecallFilter {
                memory_type: mem_type,
                project: proj,
                tags: tag_list,
                min_importance: None,
                top_k: top_k * 2,
                privacy_context: None,
                explore_pure: false,
                regression_query: None,
                read_only: false,
            };
            let deep_result = store
                .trapezoid_focus_recall(&enrich_query, &deep_filter, 1)
                .unwrap_or(RecallResult::basic(vec![], vec![], 0));

            // 倒数排名融合 (RRF, Reciprocal Rank Fusion) — 使用共享 rrf_fuse
            let fused = crate::engine::rrf::rrf_fuse_weighted(
                &fast_result,
                &deep_result,
                top_k,
                crate::engine::rrf::RRF_DEFAULT_K,
                fast_weight,
                deep_weight,
            );

            // ═══ 记录层联想补全（v0.9.8）═══
            // 与 handle_recall 同源同口径：复用 SearchData.expand_associations。
            // 在锁内执行（避免再次加锁），失败静默。
            let seed_ids: Vec<String> = fused.memories.iter().map(|m| m.id.clone()).collect();
            let associated = store
                .expand_associations(&seed_ids, &filter_for_expand, ASSOCIATION_EXPAND_MAX)
                .unwrap_or_default();

            Some((
                fused.memories,
                fused.scores,
                fused.total_candidates,
                associated,
            ))
        }),
    )
    .await;

    let (result_memories, result_scores, total, associated) = match retrieval_result {
        Ok(Ok(Some(ok))) => ok,
        Ok(Ok(None)) => {
            // 锁获取超时：返回忙态错误，提示稍后重试
            return make_error(id.clone(), -32000, "搜索服务繁忙，请稍后重试");
        }
        Ok(Err(join_error)) => {
            // spawn_blocking 内部 panic 被捕获
            eprintln!(
                "[recall_enhanced] spawn_blocking 内部 panic: {}",
                join_error
            );
            return make_error(id.clone(), -32603, "搜索内部错误，服务已保持运行");
        }
        Err(_) => {
            // 15s 超时：返回超时错误
            return make_error(id.clone(), -32001, "搜索超时，请稍后重试");
        }
    };

    let mut text = format!(
        "双路检索增强结果 (共 {} 条候选，返回 {} 条)\n\
         ═══════════════════════════════════\n\
         快速通路: 关键词匹配 | 深度通路: 深度语义检索\n\
         融合算法: 倒数排名融合 (RRF, k=60)\n\n",
        total,
        result_memories.len()
    );

    if result_memories.is_empty() {
        text.push_str("未找到相关记忆。使用 remember 工具添加新记忆。\n");
    } else {
        for (i, m) in result_memories.iter().enumerate() {
            let score = result_scores.get(i).unwrap_or(&0.0);
            let mem_num = i + 1;
            text.push_str(&format!("（记忆 #{mem_num} · RRF 融合度 {:.3}）\n", score));
            text.push_str(&format!("内容: {}\n", m.content));
            if let Some(ref cat) = m.bagua_category {
                text.push_str(&format!("分类: {} | ", cat));
            }
            text.push_str(&format!(
                "类型: {} | 重要性: {}/10\n",
                m.memory_type.as_str(),
                m.importance.value()
            ));
            text.push_str(&format!("ID: `{}`\n\n", m.id));
        }
        text.push_str("💡 双路检索融合了快速关键词匹配和深度语义定位，兼顾了召回率和精度。\n");
    }

    // ═══ 记录层联想补全（v0.9.8）═══
    // 与 handle_recall 同款分区渲染（同样的 why/via 可追溯要求）。
    if !associated.is_empty() {
        append_associated_memories(&mut text, &associated);
    }

    // ═══ 符号层联想（道体 §4.4 状态机循环 + §5.4 落边，v0.9.8）═══
    //
    // ★两个分区统一走 `append_symbolic_layer`（2026-09-18 审查 G5b/G8 修复）：
    //   · G8：此前本段与 `handle_recall` 里的代码**逐字重复** ⇒ 只改一处会静默漏改
    //   · G5b：此前两次调用**串行**（最坏 4s + 6s = 10s 叠加在检索之后），
    //     现改为**并发 + 总预算 6s**（详见该函数文档）
    //
    // 锁已在 spawn_blocking 内释放，此处网络等待不持锁。
    let seeds_for_symbolic: Vec<(String, String)> = result_memories
        .iter()
        .take(8)
        .map(|m| (m.id.clone(), m.content.clone()))
        .collect();
    append_symbolic_layer(&mut text, query, &seeds_for_symbolic).await;

    let call_result = ToolCallResult {
        content: vec![TextContent {
            content_type: "text".into(),
            text,
        }],
    };
    make_response(id, to_json_value_safe(&call_result))
}

/// 从 daoti_daemon 获取导航信号（P2.5：LRC 侧降级客户端）。
///
/// 向 daemon 的 POST /deduce 发送查询，解析 NavigationSignal JSON。
/// daemon 不可达 / 超时 / 解析失败 → 返回 None → 调用方保持无导航基线
/// （行为与既有版本逐字节一致，navigation.rs 契约保证）。
///
/// base_url 参数用于测试注入 mock；为 None 时取环境变量 DAOTI_SERVICE_URL，
/// 缺省回退 http://127.0.0.1:3222。
/// session_id：daoti_daemon 会话标识（P6/CL2 前提②闭环要求 deduce 与 reflect
/// 同会话，状态演化才能累积历史上下文）。
async fn fetch_daoti_navigation_with_base(
    query: &str,
    session_id: &str,
    base: Option<&str>,
) -> Option<crate::engine::navigation::NavigationSignal> {
    // 端口约定：daoti_daemon 固定 127.0.0.1:3222，环境变量 DAOTI_SERVICE_URL 可覆盖
    let base = match base {
        Some(b) => b.to_string(),
        None => std::env::var("DAOTI_SERVICE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3222".to_string()),
    };
    let url = format!("{}/deduce", base.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        // 连接 + 读超时 2s：daemon 挂起时不拖慢检索主链路
        .connect_timeout(std::time::Duration::from_secs(2))
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .ok()?;
    let body = serde_json::json!({"query": query, "session_id": session_id});
    let resp = client.post(&url).json(&body).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let value: serde_json::Value = resp.json().await.ok()?;
    // 信号中携带 palaces/probes/version；缺字段或全部无效 → None（无导航）
    crate::engine::navigation::NavigationSignal::from_json(&value)
}

/// fetch_daoti_navigation 的生产入口（base_url 走环境变量/默认端口）。
/// `pub(crate)`：联想中心（v1_api.rs）P3.2 也消费该信号。
pub(crate) async fn fetch_daoti_navigation(
    query: &str,
) -> Option<crate::engine::navigation::NavigationSignal> {
    fetch_daoti_navigation_with_base(query, "lrc-recall", None).await
}

/// fetch_daoti_navigation 的会话感知入口（P6/CL2：联想中心闭环使用，
/// deduce 与后续 reflect 绑定同一 session_id，状态跨查询累积）。
pub(crate) async fn fetch_daoti_navigation_for_session(
    query: &str,
    session_id: &str,
) -> Option<crate::engine::navigation::NavigationSignal> {
    fetch_daoti_navigation_with_base(query, session_id, None).await
}

/// 向 daoti_daemon 回传检索结果（P6/CL2 前提②：reflect 闭环接入运行时）。
///
/// explore 成功后把结果摘要 POST 到 daemon /reflect，daemon 用结果修正
/// 主导宫 → 下次 /deduce 方向随之演化（闭环的 daoti_daemon 侧半环）。复用
/// fetch_daoti_navigation 的客户端模式：2s 超时、DAOTI_SERVICE_URL 可覆盖；
/// 失败静默返回 false（闭环任何环节失败 → 调用方行为不变，navigation.rs
/// 契约）。调用方以 tokio::spawn fire-and-forget，不阻塞检索响应。
///
/// 返回 true 表示 daemon 确认应用（响应 {"applied": true}）。
async fn post_daoti_reflect_with_base(
    memories: &[String],
    session_id: &str,
    base: Option<&str>,
) -> bool {
    if memories.is_empty() {
        return false;
    }
    let base = match base {
        Some(b) => b.to_string(),
        None => std::env::var("DAOTI_SERVICE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3222".to_string()),
    };
    let url = format!("{}/reflect", base.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(2))
        .timeout(std::time::Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    let body = serde_json::json!({"memories": memories, "session_id": session_id});
    match client.post(&url).json(&body).send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(v) => v.get("applied").and_then(|a| a.as_bool()).unwrap_or(false),
            Err(_) => false,
        },
        _ => false,
    }
}

/// post_daoti_reflect 的生产入口（base_url 走环境变量/默认端口）。
/// `pub(crate)`：联想中心（v1_api.rs）CL2 在 explore 成功后 fire-and-forget 回传。
pub(crate) async fn post_daoti_reflect(memories: &[String], session_id: &str) -> bool {
    post_daoti_reflect_with_base(memories, session_id, None).await
}

/// 道体联想服务的 HTTP 契约版本（`/cycle` 响应里的字段协商依据）。
///
/// 与 `fetch_daoti_navigation` 的 `source_version` 协商（navigation.rs
/// `"daoti-lexicon-v1"`）同一模式：LRC 只消费 JSON，不内置道体算法。
const DAOTI_CYCLE_VERSION: &str = "daoti-assoc-v1";

/// 道体**联想服务**（`daoti_assoc`）的基址。
///
/// ## ★为什么必须与 `DAOTI_SERVICE_URL` **分开**（2026-09-18 修）
///
/// 这是两个**不同的服务**，端点集合不同：
///
/// | 服务 | 默认端口 | 拥有的端点 |
/// |---|---|---|
/// | `daoti_daemon`（`daoti/daoti_daemon.py`） | **3222** | `/deduce` `/reflect` `/drift/*` |
/// | `daoti_assoc`（`temp/daoti_assoc/server.py`） | **3223** | `/cycle` `/build_edges` `/parse` `/associate` … |
///
/// 此前 `/cycle` 与 `/build_edges` 复用了 `DAOTI_SERVICE_URL`（默认 **3222**），
/// 而这两个端点在 3222 上**不存在** ⇒ 请求打到 daemon 的 404 路径 ⇒
/// 静默降态返回 `None` ⇒ **即使开了门控也永远无输出、且无任何日志**。
/// 这正是"接了但没生效"的失效形态（本案的头号阻断项）。
///
/// ## 为什么不做 `DAOTI_SERVICE_URL` 回退（刻意的）
///
/// 回退看似兼容，实则保留了同一个陷阱：一旦 `DAOTI_SERVICE_URL` 被显式设为
/// 3222（daemon 的**文档默认值**），联想端点就会再次静默打到错误服务。
/// ⇒ **宁可让配置缺失时落到正确默认值，也不让"配置存在但指向错服务"静默通过。**
/// 需要自定义时显式设 `DAOTI_ASSOC_URL`（如 `http://127.0.0.1:3223`）。
fn daoti_assoc_url() -> String {
    std::env::var("DAOTI_ASSOC_URL").unwrap_or_else(|_| "http://127.0.0.1:3223".to_string())
}

/// 向道体联想服务请求一次 §4.4 状态机循环（`POST /cycle`）。
///
/// ## 为什么需要这个调用（2026-09-17 接入盘点结论）
///
/// 实测发现 LRC ↔ daoti_assoc **此前只接了 1 个端点**（`/deduce` + `/reflect`），
/// 而 `assoc_service` 里 §6 定义的 `parse` / `associate` / `scheduler`
/// **从未被调用**——即"符号层联想"整条链在 LRC 里是断的：
///
/// | 道体端点 | 设计意图（§6） | 接入前 | 接入后 |
/// |---|---|---|---|
/// | `/deduce` | 取导航信号 | ✅ | ✅ |
/// | `/reflect` | 回传结果 | ✅ | ✅ |
/// | `/cycle` | **联想候选 + 目标层次** | ❌ 不存在 | ✅ |
///
/// ## `target` 为什么就是 query
///
/// 用户裁定：「目标就是召回」「不需要解析，因为很多召回它可能只是一个词，
/// 比如说召回一个苹果……你只能朝着苹果的目标去走就行了」。
/// ⇒ 这里把 **recall 的查询本身**作为 target 传入，不做任何解析。
///
/// ## 降态（§6）
///
/// daemon 不可达 / 超时 / 响应缺字段 / 版本不匹配 ⇒ 返回 None。
/// 调用方必须容忍 None（联想缺失是可接受的降态，宁缺勿错）。
/// 门控 `LRC_DAOTI_CYCLE=1` 默认关闭 ⇒ 行为与既有版本逐字节一致。
async fn fetch_daoti_cycle_with_base(
    text: &str,
    target: &str,
    base: Option<&str>,
) -> Option<serde_json::Value> {
    let base = match base {
        Some(b) => b.to_string(),
        // ★用联想服务专用变量（默认 3223），**不**回退到
        //   `DAOTI_SERVICE_URL`（默认 3222 是 daemon，无 /cycle）——见
        //   `daoti_assoc_url()` 的说明。
        None => daoti_assoc_url(),
    };
    let url = format!("{}/cycle", base.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        // ★超时给 4s（比 /deduce 的 2s 宽）：/cycle 内含一次 BGE 编码 +
        //   最多 6 次结构距离 BFS，比纯导航推导重。但仍**必须**有上限——
        //   道体挂起时绝不能拖慢检索主链路（承"绝不拖累用户可见请求"）。
        .connect_timeout(std::time::Duration::from_secs(2))
        .timeout(std::time::Duration::from_secs(4))
        .build()
        .ok()?;
    let body = serde_json::json!({"text": text, "target": target});
    let resp = client.post(&url).json(&body).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let value: serde_json::Value = resp.json().await.ok()?;
    // ★契约校验：`associations` 必须存在且是数组。
    //   缺失 ⇒ 对方不是本服务或缺字段 ⇒ None（不猜、不兜底）。
    if !value
        .get("associations")
        .map(|a| a.is_array())
        .unwrap_or(false)
    {
        return None;
    }
    // ★版本协商：道体在响应里回 `version`。缺失或非预期值 ⇒ 协议不认识 ⇒ None。
    //   这与 navigation.rs 的 source_version 协商同一纪律：**不做字段猜测**，
    //   否则对方改了字段语义后，我们会静默地按旧含义解读（比失败更危险）。
    match value.get("version").and_then(|v| v.as_str()) {
        Some(v) if v == DAOTI_CYCLE_VERSION => {}
        _ => return None,
    }
    Some(value)
}

/// fetch_daoti_cycle 的生产入口（base_url 走环境变量/默认端口）。
pub(crate) async fn fetch_daoti_cycle(text: &str, target: &str) -> Option<serde_json::Value> {
    fetch_daoti_cycle_with_base(text, target, None).await
}

/// 向 daoti_assoc 请求「标卦 → 结构候选 → 落边」（`POST /build_edges`，§5.4 写入端）。
///
/// ## 为什么必须接它（2026-09-18 盘点）
///
/// `/build_edges` 是 §5.4 闭环（候选命中 → 生成边 → **可检索**）的写入端。
/// 接入前它**没有任何调用方**——即"函数写好了却从没被调用"，
/// 与 `/associate` `/parse` `/scheduler` 此前是同一种失效形态。
///
/// ## ★但接它不等于"放开落边"（**必须说清，防误读**）
///
/// 该端点内部有**主动门控**：`「文本→卦」通路未过 §8.1 J1 判据`
/// ⇒ 默认返回 `blocked: "text_to_gua_judge_not_passed"` 且 `edges: []`。
///
/// ★原因码**必须与 Python 侧逐字一致**：真值取自
/// `temp/daoti_assoc/assoc_service.py` 的 `build_edges_for_seeds`
/// （`"blocked": "text_to_gua_judge_not_passed"`）。
/// 此前该注释写的是 `no_usable_text_to_gua_path`（**早已弃用的旧值**）——
/// 按注释去搜代码/日志会一无所获，属"文档与实现不一致"的一类。
/// ⇒ 契约串在两侧各写一份时，改一侧必须同步另一侧。
///
/// **这个阻断不能被"接上"消除**——它是实测结论（A 路线判别力不足
/// 1.50x/p=0.1479；B 路线分词表只覆盖易经古文），为了**不污染图**
/// 而存在：无依据的边比没有边更糟，用户会按错误关系理解记忆关联。
///
/// ⇒ 因此本函数的正确职责是：**把链路接通，并把"为什么没有边"透出**。
///   让用户/开发者看得见阻断原因，而不是让系统静默地"什么都没有"。
///
/// ## 降态
///
/// 不可达/超时/版本不符 ⇒ None（与 `/cycle` 同纪律）。
/// 门控 `LRC_DAOTI_BUILD_EDGES=1` 默认关闭；`write_back` 需再显式开启，
/// 因为写图是**有副作用**的动作，不能让一次检索意外改图。
async fn fetch_daoti_build_edges_with_base(
    seeds: &[(String, String)],
    write_back: bool,
    base: Option<&str>,
) -> Option<serde_json::Value> {
    if seeds.is_empty() {
        return None;
    }
    let base = match base {
        Some(b) => b.to_string(),
        // ★同 `/cycle`：用联想服务专用变量（默认 3223），不回退 3222。
        None => daoti_assoc_url(),
    };
    let url = format!("{}/build_edges", base.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        // 内含一次批量 BGE 编码（max_seeds 条）⇒ 比 /cycle 更重，给 6s。
        // 但仍必须有上限：道体挂起绝不能拖慢检索主链路。
        .connect_timeout(std::time::Duration::from_secs(2))
        .timeout(std::time::Duration::from_secs(6))
        .build()
        .ok()?;
    let seed_json: Vec<serde_json::Value> = seeds
        .iter()
        .map(|(mid, text)| serde_json::json!({"memory_id": mid, "text": text}))
        .collect();
    let body = serde_json::json!({"seeds": seed_json, "write_back": write_back});
    let resp = client.post(&url).json(&body).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let value: serde_json::Value = resp.json().await.ok()?;
    // ★契约校验：`edges` 必须存在且是数组（门控阻断时为空数组，也算合法）。
    //   缺失 ⇒ 对方不是本服务 ⇒ None（不猜）。
    if !value.get("edges").map(|a| a.is_array()).unwrap_or(false) {
        return None;
    }
    match value.get("version").and_then(|v| v.as_str()) {
        Some(v) if v == DAOTI_CYCLE_VERSION => {}
        _ => return None,
    }
    Some(value)
}

/// fetch_daoti_build_edges 的生产入口。
pub(crate) async fn fetch_daoti_build_edges(
    seeds: &[(String, String)],
    write_back: bool,
) -> Option<serde_json::Value> {
    fetch_daoti_build_edges_with_base(seeds, write_back, None).await
}

/// 落边结果 → MCP 文本块（v0.9.8）
///
/// ## ★这个函数的主要职责是**解释"为什么没有边"**
///
/// 当 `blocked` 存在时，必须把阻断原因与**解封条件**都写出来。
/// 若只显示"无符号层边"，用户会以为是功能坏了或还没做——
/// 而实际是**实测判据未过，系统主动不产出**（两者处置完全不同）。
fn append_daoti_build_edges(text: &mut String, res: &serde_json::Value) {
    let edges = res.get("edges").and_then(|e| e.as_array());
    let blocked = res.get("blocked").and_then(|b| b.as_str());

    text.push_str("\n═══ 联想 · 符号层落边（§5.4 写入端）═══\n");

    if let Some(reason) = blocked {
        // ★阻断路径：把"为什么"与"怎么解"都给出（缺任一项用户都无从处置）
        text.push_str("状态: **未产出边**（主动门控，非故障）\n");
        text.push_str(&format!("原因码: `{}`\n", reason));
        if let Some(detail) = res.get("blocked_detail").and_then(|d| d.as_str()) {
            text.push_str(&format!("依据: {}\n", detail));
        }
        if let Some(env) = res.get("unblock_env").and_then(|e| e.as_str()) {
            text.push_str(&format!(
                "解封条件: 实验结果达到判据后自动放行；\
                 仅做形态观察可用 `{}`（**仅供实验，产出的边不可作证据**）。\n",
                env
            ));
        }
        text.push_str("💡 这不影响上面的检索与记录层联想——它们不依赖「文本→卦」通路。\n");
        return;
    }

    match edges {
        Some(list) if !list.is_empty() => {
            text.push_str(&format!("状态: 产出 {} 条结构边\n", list.len()));
            for (i, e) in list.iter().take(10).enumerate() {
                let from = e.get("from_id").and_then(|v| v.as_str()).unwrap_or("?");
                let to = e.get("to_id").and_then(|v| v.as_str()).unwrap_or("?");
                let rel = e.get("rel_type").and_then(|v| v.as_str()).unwrap_or("?");
                let how = e.get("how").and_then(|v| v.as_str()).unwrap_or("");
                let gua = e.get("gua_name").and_then(|v| v.as_str()).unwrap_or("?");
                text.push_str(&format!(
                    "（结构边 #{} · {} → {} · {} · 算子 {} · 候选卦 {}）\n",
                    i + 1,
                    from.chars().take(8).collect::<String>(),
                    to.chars().take(8).collect::<String>(),
                    structural_rel_label(rel),
                    how,
                    gua
                ));
            }
            if let Some(wb) = res.get("write_back") {
                text.push_str(&format!(
                    "落图: 新增 {} / 跳过 {} / 共 {}\n",
                    wb.get("written").and_then(|v| v.as_u64()).unwrap_or(0),
                    wb.get("skipped").and_then(|v| v.as_u64()).unwrap_or(0),
                    wb.get("total").and_then(|v| v.as_u64()).unwrap_or(0),
                ));
            }
            // ★实验放行时必须显示警告（承"证据要可区分"纪律）
            if let Some(w) = res.get("unstable_warning").and_then(|v| v.as_str()) {
                text.push_str(&format!("⚠ {}\n", w));
            }
            // ★显示"被跳过的候选"数量，让"为什么边比预期少"可解释：
            //   不动点算子（候选卦 == 种子自身卦）会被主动跳过——它等价于
            //   「同卦建边」，而"同卦"在本数据上几近恒真（零区分度）。
            //   若不显示，用户会以为落边功能残缺。
            if let Some(n) = res
                .get("stats")
                .and_then(|s| s.get("fixed_point_skipped"))
                .and_then(|v| v.as_u64())
            {
                if n > 0 {
                    text.push_str(&format!(
                        "（另有 {} 个候选因「结构算子返回自身卦」被跳过：\
                         那等价于同卦建边，无区分度 ⇒ 宁缺勿错）\n",
                        n
                    ));
                }
            }
        }
        _ => {
            // 门控关了、但也没产出 ⇒ 如实说明，不静默。
            // ★成因必须**从数据读出**而非猜（三种成因处置完全不同）：
            //   ① 种子不足 2 条（边需要两端）
            //   ② 候选全是不动点算子（等价同卦建边，已主动跳过）
            //   ③ 其余（如候选卦下没有别的召回记忆）
            let stat = |k: &str| -> u64 {
                res.get("stats")
                    .and_then(|s| s.get(k))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0)
            };
            let seeds_used = stat("seeds_used");
            let fp = stat("fixed_point_skipped");
            if seeds_used < 2 {
                text.push_str(&format!(
                    "状态: 本次未产出边（**种子不足 2 条**，实际 {} 条；\
                     边需要两端，单条无从建边）\n",
                    seeds_used
                ));
            } else if fp > 0 {
                text.push_str(&format!(
                    "状态: 本次未产出边（候选卦均为结构算子的**不动点**\
                     ⇒ 等价于同卦建边，已跳过 {} 个；这是主动取舍，非故障）\n",
                    fp
                ));
            } else {
                text.push_str(
                    "状态: 本次未产出边（候选卦下没有其他召回记忆，\
                     即无结构相关者）\n",
                );
            }
        }
    }
}

/// 符号层（道体）两个分区的**统一接入点** —— 合并 G5b + G8 两项审查问题。
///
/// ## G8（本轮修复）：为什么必须抽成一个函数
///
/// 此前 `handle_recall` 与 `handle_recall_enhanced` 里各有一份**逐字重复**的
/// 接入代码（门控读取 → 种子构造 → 两次网络调用 → 渲染）。两处一致性靠人工同步
/// ⇒ 后续只改一处会静默漏改（如本轮的预算控制、契约告警）。
///
/// ## G5b（本轮修复）：为什么改为**并发**调用
///
/// ### 此前的问题
///
/// 两个调用是**串行 `await`**：
///   · `/cycle` 超时 = 连接 2s + 总 4s
///   · `/build_edges` 超时 = 连接 2s + 总 6s
///
/// ⇒ 最坏 `4 + 6 = 10s` 全部叠加在**用户可见的检索结果之后**。
/// 而 `handle_recall` **没有外层超时包裹**（只有 enhanced 的检索段有 15s），
/// 故最坏 = 检索耗时 **+ 10s**；enhanced 则是 **15s + 10s = 25s**。
///
/// 项目前端自身的请求预算是 **10s**（`static/app.js`），且既有纪律明确要求
/// "后端超时必须小于前端预算"（`server.rs` 的超时预算注释）。
/// ⇒ 用户会先撞上前端超时，看到"请求超时"而非检索结果 —— 与
/// 「联想是附加价值，不可影响主结果」的设计前提**直接冲突**。
///
/// ### 修法：并发 + 总预算
///
/// 1. **两次调用并发**（`tokio::join!`）⇒ 最坏从 10s 降到 **6s**（取较大者）
/// 2. 再包一层**总预算**（`SYMBOLIC_LAYER_BUDGET`，默认 6s）⇒ 即使两边都挂，
///    也**绝不**超过该预算
///
/// ⇒ 最坏 = 检索耗时 + 6s，仍在 10s 前端预算内（留 4s 给检索本身）。
///
/// ## 保持的行为（不得退化）
///
/// · 门控语义不变：`LRC_DAOTI_CYCLE` 控制 `/cycle`，`LRC_DAOTI_BUILD_EDGES`
///   控制 `/build_edges`，`LRC_DAOTI_WRITE_BACK` 控制是否写图
/// · 两个分区分开渲染、不混排（`append_daoti_cycle` / `append_daoti_build_edges`）
/// · 任一失败静默降态，**不影响**记录层联想与检索主结果
/// · 关掉门控时**不发任何请求**（与前次逐字节一致）
async fn append_symbolic_layer(text: &mut String, query: &str, seeds: &[(String, String)]) {
    let want_cycle = std::env::var("LRC_DAOTI_CYCLE")
        .map(|v| v == "1")
        .unwrap_or(false);
    let want_edges = std::env::var("LRC_DAOTI_BUILD_EDGES")
        .map(|v| v == "1")
        .unwrap_or(false);
    if !want_cycle && !want_edges {
        return; // 两个门控都关 ⇒ 不发任何请求（行为与既有版本一致）
    }
    let write_back = std::env::var("LRC_DAOTI_WRITE_BACK")
        .map(|v| v == "1")
        .unwrap_or(false);

    // ★并发：两次网络调用同时进行（此前是串行，最坏 4s + 6s = 10s）
    //
    // ★2026-09-18 审查修复：**分别持有结果槽**，而非依赖 `join!` 的返回值。
    //
    // 为什么必须这样改（原实现的缺陷）：
    //   `tokio::time::timeout(BUDGET, join!(cycle_fut, edges_fut))` 一旦超时，
    //   只能拿到 `Err(_)` —— **join 的整体返回值被丢弃**，于是**已完成的一侧
    //   也一并牺牲**。而 `/cycle` 的读超时是 4s、`/build_edges` 是 6s，
    //   当后者真的走到自己的 6s 超时时（正是它设计要处理的慢服务场景），
    //   外层预算（同为 6s、且计时更早）必然先触发 ⇒ **必然**把已经成功
    //   返回的候选卦丢掉。用户看到的信息量最低的降态块，
    //   而不是"候选照常 + 落边未跑完"。
    //
    // 修法：两个 future 各自把结果写进 `Arc<Mutex<Option<..>>>` 槽位
    //（写入发生在 future 内部，一旦完成即已落槽，不依赖 join 的返回），
    // 超时时直接读槽：**谁已完成就渲染谁**，只对未完成的给降态说明。
    let cyc_slot: Arc<StdMutex<Option<serde_json::Value>>> = Arc::new(StdMutex::new(None));
    let be_slot: Arc<StdMutex<Option<serde_json::Value>>> = Arc::new(StdMutex::new(None));
    let cyc_out = Arc::clone(&cyc_slot);
    let be_out = Arc::clone(&be_slot);

    let cycle_fut = async move {
        if want_cycle {
            let got = fetch_daoti_cycle(query, query).await;
            if let Some(v) = got {
                if let Ok(mut slot) = cyc_out.lock() {
                    *slot = Some(v);
                }
            }
        }
    };
    let edges_fut = async move {
        if want_edges {
            let got = fetch_daoti_build_edges(seeds, write_back).await;
            if let Some(v) = got {
                if let Ok(mut slot) = be_out.lock() {
                    *slot = Some(v);
                }
            }
        }
    };

    // ★总预算：两边都挂时也绝不超出（超过 ⇒ 未完成的分区降态，**已完成的不丢**）
    let joined = tokio::time::timeout(SYMBOLIC_LAYER_BUDGET, async {
        tokio::join!(cycle_fut, edges_fut)
    })
    .await;

    let timed_out = joined.is_err();

    // 从槽位取回各自结果（超时与非超时都走同一读取路径，行为一致）
    let cyc = cyc_slot.lock().ok().and_then(|mut s| s.take());
    let be = be_slot.lock().ok().and_then(|mut s| s.take());

    // 分区渲染：与记录层**分开**（两套证据性质不同，不混排）
    //
    // ★先记录"是否拿到"再移动取值：`Option<Value>` 非 Copy，若先 `if let Some(c) = cyc`
    //   把值 move 走，后面再读 `cyc.is_none()` 会触发 E0382（借用已部分移动的值）。
    let got_cyc = cyc.is_some();
    let got_be = be.is_some();
    if let Some(c) = cyc {
        append_daoti_cycle(text, &c);
    }
    if let Some(b) = be {
        append_daoti_build_edges(text, &b);
    }
    // 只有"确实超时且两边都没拿到"时才整体降态；
    // 若至少一侧成功，则用更精确的分区级降态说明（不掩盖已拿到的结果）。
    if timed_out && !got_cyc && !got_be {
        append_symbolic_layer_degraded(text);
    } else if timed_out {
        append_symbolic_layer_partial(text, want_cycle && !got_cyc, want_edges && !got_be);
    }
}

/// 符号层（道体）调用的**总预算**（2026-09-18 审查 G5b 修复）。
///
/// **取值依据**：前端请求预算是 10s（`static/app.js`），既有纪律要求
/// 后端超时 < 前端预算。两次道体调用并发后最坏为 6s（`/build_edges` 的读超时），
/// 故预算取 **6s** 与之对齐 —— 既覆盖正常并发路径，又给"检索本身"留出 4s。
///
/// **为什么需要它（而非只靠各调用的超时）**：两个调用各自的超时是
/// "连接 2s + 总 4/6s"，但**并发后仍需一个整体闸门** —— 否则将来若新增
/// 第三个道体调用，叠加风险会无声重现。总预算是**结构性**上限，
/// 不随调用个数增长。
const SYMBOLIC_LAYER_BUDGET: std::time::Duration = std::time::Duration::from_secs(6);

/// 符号层超预算降态时的分区标题（承 G9：标题是**契约**，抽成常量供测试锁定）。
///
/// 为什么值得抽出来：它是"符号层本次没跑"的**唯一**信号。若标题被改，
/// 用户看到的是一个陌生分区，无法与上一条"没跑完"对上号。
const SYMBOLIC_LAYER_DEGRADED_TITLE: &str = "联想 · 符号层（道体）";

/// 渲染「符号层本次未跑完」的降态块（超预算路径专用）。
///
/// 抽成独立函数的原因（承 G9）：该分支只在**真实超时**时才走到，
/// 而超时受 `SYMBOLIC_LAYER_BUDGET`（6s）控制 ⇒ 若写在内联里，
/// 测试要复现必须真的等 6 秒（或起一个永不响应的 mock）——
/// 成本高到没人会写，于是这条路径长期**无测试**。
/// 抽出来后可直接断言"降态时用户看到什么"，成本为零。
fn append_symbolic_layer_degraded(text: &mut String) {
    text.push_str(&format!(
        "\n═══ {} ═══\n\
         状态: 本次跳过（超过 {}s 预算，已降态）\n\
         💡 这不影响上面的检索与记录层联想——它们不依赖「文本→卦」通路。\n",
        SYMBOLIC_LAYER_DEGRADED_TITLE,
        SYMBOLIC_LAYER_BUDGET.as_secs()
    ));
}

/// 渲染「符号层**部分**未跑完」的说明块（2026-09-18 审查修复）。
///
/// # 与 [`append_symbolic_layer_degraded`] 的分工
///
/// · 整体降态：**两边都没拿到** ⇒ 用那个（"本次跳过"）
/// · 部分降态：**至少一侧成功** ⇒ 用本函数
///
/// # 为什么必须区分（原来的缺陷）
///
/// 原实现在超时就 `return` + 整体降态，即使 `/cycle` 已成功返回也照丢。
/// 用户看到的是"符号层本次跳过"，而**实际拿到了候选卦** ——
/// 这既丢能力，又对用户说错话（"没跑"与"跑了一半"是两件事）。
///
/// 故本函数按**各自的实际状态**分别说明，不让已完成的一侧被未完成的一侧掩盖。
///
/// 参数 `cyc_missing` / `edges_missing` 用布尔而非枚举：调用点已知道
/// "该门控是否开着且结果为空"，此处无需再理解门控语义。
fn append_symbolic_layer_partial(text: &mut String, cyc_missing: bool, edges_missing: bool) {
    if !cyc_missing && !edges_missing {
        return; // 两边都在超时前完成 ⇒ 无需任何降态说明
    }
    let mut parts: Vec<&str> = Vec::new();
    if cyc_missing {
        parts.push("符号层候选（结构方向）");
    }
    if edges_missing {
        parts.push("符号层落边（§5.4）");
    }
    text.push_str(&format!(
        "\n═══ {} ═══\n\
         状态: 部分完成（超过 {}s 预算）\n\
         未跑完: {}\n\
         💡 上面已展示的部分是**真实结果**，不是降级数据；未列出的分区本次超时未返回。\n",
        SYMBOLIC_LAYER_DEGRADED_TITLE,
        SYMBOLIC_LAYER_BUDGET.as_secs(),
        parts.join("、")
    ));
}

/// 符号层关系类型 → 中文标签（§5.3 五类逻辑关系）
/// ★为什么不复用 `relation_label`：那是**记录层**标签表
/// （same_event / shared_entity / …），符号层的 CONSTRAINT / COORDINATE
/// 等类型撞进去会全部落到兜底值「相关联」，把类型信息抹平。
/// 两张表语义不同，必须分开（承「证据要可区分」）。
///
/// 未知类型**原样回显**而非兜底：兜底会把"新类型/对方升级"伪装成已知类型，
/// 排查时看不出差异。
fn structural_rel_label(rel: &str) -> &str {
    match rel {
        "CAUSE" => "因果",
        "TEMPORAL" => "时序",
        "CONSTRAINT" => "相错（约束）",
        "FACILITATE" => "变爻（促成）",
        "COORDINATE" => "相综/互卦（协同）",
        other => other,
    }
}

/// 道体循环结果 → MCP 文本块（v0.9.8）
///
/// ## 为什么单独分区、且**必须显示层次与目标**
///
/// 用户对联想的价值判据原话是：「它会告诉你，**这是第几层**他能想到的
/// 这个记忆数据。然后你再进行调取」。
/// ⇒ 层次（`target_hops`）是**可核验的坐标**，不是装饰：
///    · 0 跳 = 候选卦就是目标卦（最近）
///    · 1 跳 = 一个结构算子可达
///    · 2~3 跳 = 需经中间卦
///    · 缺 `target_hops` = **算不出**（L3 不可用/超 3 跳），须如实标注，
///      不可默认成某个数——"算不出"与"很远"是两件事。
///
/// ## 与「记录层联想」的分区关系
///
/// 记录层（`append_associated_memories`）给的是**记忆**；
/// 本函数给的是**卦候选 + 目标坐标**（符号层，尚无记忆 ID）。
/// 二者证据性质不同，必须分区显示，不可混排。
fn append_daoti_cycle(text: &mut String, cyc: &serde_json::Value) {
    let assoc = match cyc.get("associations").and_then(|a| a.as_array()) {
        Some(a) if !a.is_empty() => a,
        _ => return,
    };
    let target = cyc.get("target").and_then(|t| t.as_str()).unwrap_or("");
    let target_gua = cyc
        .get("target_gua")
        .and_then(|t| t.as_str())
        .unwrap_or("（未知）");

    text.push_str(
        "\n═══ 联想 · 符号层候选（道体 §4.4 状态机循环）═══\n\
         以下不是记忆，而是**候选方向**：由结构算子（错/综/互/变爻）从当前卦\n 衍出，并标注**朝目标还差几跳**。\n",
    );
    text.push_str(&format!(
        "目标: 「{}」 → 目标卦: {}\n\n",
        target.chars().take(40).collect::<String>(),
        target_gua
    ));

    for (i, c) in assoc.iter().enumerate() {
        let gua = c.get("gua_name").and_then(|v| v.as_str()).unwrap_or("?");
        let rel = c.get("rel_type").and_then(|v| v.as_str()).unwrap_or("?");
        let how = c.get("how").and_then(|v| v.as_str()).unwrap_or("");
        let l3 = c.get("l3_source").and_then(|v| v.as_str()).unwrap_or("");
        // ★层次：None 必须显示为"算不出"，不可省略（否则读者会以为是 0 跳）
        let hops = match c.get("target_hops") {
            Some(v) if v.is_u64() => format!("第 {} 层", v.as_u64().unwrap_or(0)),
            Some(serde_json::Value::Null) | None => {
                "层次: 算不出（超 3 跳或 L3 不可用）".to_string()
            }
            Some(v) => format!("第 {} 层", v),
        };
        text.push_str(&format!(
            "（符号候选 #{} · {} · {} · 依据 {} · {}）\n",
            i + 1,
            hops,
            gua,
            structural_rel_label(rel),
            how
        ));
        if let Some(wx) = c.get("wuxing_relation").and_then(|v| v.as_str()) {
            // 生克只作辅助特征（§1.2 已否证其作主判据），故标为"附注"
            text.push_str(&format!("附注: 五行生克 {}（辅助特征，不参与排序）\n", wx));
        }
        if !l3.is_empty() {
            text.push_str(&format!("结构算子来源: {}\n", l3));
        }
        text.push('\n');
    }

    if let Some(d) = cyc.get("degraded").and_then(|v| v.as_array()) {
        if !d.is_empty() {
            let items: Vec<String> = d
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            text.push_str(&format!(
                "⚠ 降态项: {}（这些字段当前是默认值，非实测）\n",
                items.join(", ")
            ));
        }
    }
    text.push_str(
        "💡 符号层候选当前**不落图**（「文本→卦」通路未通过判据，见设计文档 §11）；\n\
         它给的是方向与层次，不是可直接引用的记忆。\n",
    );
}

/// 联想补全的条数上限（v0.9.8）
///
/// **为什么需要上限**：同一次经历的簇可能很大（实测有 32 条的同小时簇），
/// 若不设限，一次检索的输出会被联想结果淹没，主结果反而看不见。
///
/// # ★ 取值 3 是**诊断目的**（2026-09-16 用户裁定）
///
/// 原值 8 使**意外性排序机制在实际配置下不工作**：实测每种子通道数中位仅 1
/// ⇒ 每通道配额 ≈8，而**通道长度 > 8 的比例仅 13.9%**
/// ⇒ 86% 的种子其通道内候选全部输出，排序改变不了集合。
/// 实测提升：max_out=8 → +1.2pp（近零）；**max_out=3 → +7.2pp**。
///
/// ⇒ 取 3 是为**让排序机制进入可观测状态**，而非"提升精度"。
/// 否则无法回答"BGE 给不出的那些关联有没有意义"——它们根本没机会被输出。
/// 代价（如实记录）：联想总量减少（保留约 43%）。
/// 完整实测记录见内部预注册文档 §3.10（研究材料，不随产品发布）。
const ASSOCIATION_EXPAND_MAX: usize = 3;

/// 把「联想补全」结果渲染成 MCP 文本块（v0.9.8）
///
/// **为什么单独分区、不混进主结果列表**：
/// 两类结果的**证据性质不同**——主结果是相似度打分（可比大小），
/// 联想是记录层的确定性推导（只有"成立/不成立"，没有强弱）。
/// 混排会让调用方误以为二者可以按分数排序，也会破坏既有排序的 A/B 证据。
///
/// **为什么必须显示 `why` 与 `via`**：用户要判断"这个联想是否合理"，
/// 就必须知道①凭什么关联（`why`，含具体 event_id 或实体名）
/// ②从哪条记忆联想过来（`via`）。缺任一项，联想就成了无从检验的黑箱
/// （承方法论 105：解释必须解释到具体对象）。
///
/// # ★按证据来源**分区**渲染（2026-09-18 审查修复）
///
/// 本函数收到的 `assoc` 是**混合列表**：记录层（由记录必然成立）与
/// 符号层落盘边（结构算子推导，**可能不成立**）并存。此前两者共用
/// 一个分区标题「记录型关联（由记录推导…由记录必然关联）」与页脚
/// 「这些是「共同经历 / 共享实体」带来的连接」——对符号层边是**事实错误**。
///
/// 这与 UI 侧已修掉的失效模式同源（`v1_api.rs::explore_source_of`
/// 把符号层标成 `symbolic` 而非 `record`），只是漏在了 MCP 文本出口。
/// ⇒ 以 `is_symbolic_edge_type` 为**唯一判据**分流，两区各有各的标题与页脚
///（单一事实来源：不在此另列一份类型名，避免与 `memory_store.rs` 漂移）。
fn append_associated_memories(text: &mut String, assoc: &[AssociatedMemory]) {
    // 分流：符号层落盘边 vs 记录层关联。保持原有相对顺序（调用方已排好序）。
    let (sym, rec): (Vec<&AssociatedMemory>, Vec<&AssociatedMemory>) = assoc
        .iter()
        .partition(|a| crate::memory_store::is_symbolic_edge_type(&a.relation));

    if !rec.is_empty() {
        append_record_layer_section(text, &rec);
    }
    if !sym.is_empty() {
        append_symbolic_stored_section(text, &sym);
    }
}

/// 渲染**记录层**关联分区（由记录必然成立）。
fn append_record_layer_section(text: &mut String, rec: &[&AssociatedMemory]) {
    text.push_str(
        "\n═══ 联想 · 记录型关联（由记录推导，非语义相似）═══\n\
         以下记忆**与本次查询语义可能毫不相似**，但它们由记录必然关联——\n\
         这是相似度检索给不出的连接。\n\
         【层次】标注它距联想起点几跳：1 跳=记录直接成立；2 跳=经中间记忆的间接关联。\n\n",
    );
    for (i, a) in rec.iter().enumerate() {
        append_one_association(text, a, "记录型关联", i + 1, &a.relation);
    }
    text.push_str(
        "💡 这些是「共同经历 / 共享实体」带来的连接。若要它们更丰富，\
写入时给同一次经历的多条记忆填相同的 event_id。\n",
    );
}

/// 渲染**符号层落盘边**分区（结构算子推导，**可能不成立**）。
///
/// 与记录层分区的措辞必须**处处相反**（必然/可能、记录/推导）：
/// 两者混排或共用标题，用户就会把推测当事实引用。
fn append_symbolic_stored_section(text: &mut String, sym: &[&AssociatedMemory]) {
    text.push_str(
        "\n═══ 联想 · 符号层落边（结构算子推导，可能不成立）═══\n\
         以下连接来自图里**已落盘**的结构推导边（由结构算子从卦推出），\n\
         与「记录型关联」性质相反：它们**不是**记录事实，**可能不成立**，\n\
         请当作线索而非结论。\n\n",
    );
    for (i, a) in sym.iter().enumerate() {
        append_one_association(text, a, "符号层落边", i + 1, &a.relation);
    }
    text.push_str(
        "💡 这些是「结构推导」带来的连接，**不可直接当证据引用**。\
若要它们更可靠，需先让判据通过后再落边（详见落边分区的解封条件）。\n",
    );
}

/// 渲染**单条**联想（两区共用，仅"类型名"与"关系标签表"不同）。
///
/// `kind_name` 是分区性质名（记录型关联 / 符号层落边）；
/// `relation` 决定标签表——符号层类型必须走 `structural_rel_label`，
/// 否则 `COORDINATE` 等会落到记录层的兜底值「相关联」，把类型抹平。
fn append_one_association(
    text: &mut String,
    a: &AssociatedMemory,
    kind_name: &str,
    seq: usize,
    relation: &str,
) {
    // ★先绑定大写形式再借用：`structural_rel_label` 匹配的是大写键，
    //   若直接写 `&relation.to_uppercase()`，临时 String 会在语句末尾被
    //   drop，返回的 `&str` 立刻悬空（E0716）。
    let upper = relation.to_ascii_uppercase();
    let label = if crate::memory_store::is_symbolic_edge_type(relation) {
        structural_rel_label(&upper)
    } else {
        relation_label(relation)
    };
    // ★层次必须显示（用户对联想的价值判据：「告诉我这是第几层能想到的」）。
    //   `hops` 是寻路的坐标，不是分类标签——故与关系类型**并列**展示，
    //   让调用方一眼看出证据强度（1 跳强于 2 跳）。
    text.push_str(&format!(
        "（{} #{} · 第 {} 层 · {} · {}）\n",
        kind_name, seq, a.hops, label, a.memory_type
    ));
    text.push_str(&format!("内容: {}\n", a.content_preview));
    text.push_str(&format!("依据: {}\n", a.why));
    // 2 跳的"意外性"来自**路径本身**：写出完整路径可核，
    // 用户据此判断这个跳跃是否合理（只给终点则无法判断）。
    if a.hops >= 2 && a.path.len() >= 3 {
        let mut p = String::new();
        for (k, id) in a.path.iter().enumerate() {
            if k > 0 {
                p.push_str(" → ");
            }
            p.push_str(&id.chars().take(8).collect::<String>());
        }
        text.push_str(&format!("路径: {}\n", p));
    }
    text.push_str(&format!(
        "来路: 由「{}」联想到此（{}）\n",
        a.via_preview.chars().take(40).collect::<String>(),
        a.via_memory_id.chars().take(12).collect::<String>(),
    ));
    text.push_str(&format!("ID: `{}`\n\n", a.memory_id));
}

/// 处理 recall 工具调用 — 关键词匹配 / 深度语义检索
///
/// 支持 lrc_mode: "fast"（关键词匹配，默认）或 "deep"（深度语义检索）
/// 若配置了 LLM API，自动将查询翻译为答案关键词以桥接语义鸿沟。
/// v0.9.8 起，结果尾部追加「记录层联想」分区（见 [`append_associated_memories`]）。
async fn handle_recall(
    state: &AppState,
    arguments: &serde_json::Value,
    id: Option<serde_json::Value>,
) -> JsonRpcResponse {
    let query = match arguments.get("query").and_then(|q| q.as_str()) {
        Some(q) => q,
        None => return make_error(id, -32602, "缺少参数: query"),
    };
    let top_k = arguments
        .get("top_k")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .clamp(1, 100) as usize;

    // 检索模式：fast（关键词匹配）或 deep（深度语义检索）
    let lrc_mode = arguments
        .get("lrc_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("fast");
    let focus_depth = arguments
        .get("focus_depth")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .clamp(1, 3) as u32;

    let memory_type = arguments
        .get("memory_type")
        .and_then(|v| v.as_str())
        .and_then(MemoryType::try_parse);

    let project = arguments
        .get("project")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let tags: Vec<String> = arguments
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let min_importance = arguments
        .get("min_importance")
        .and_then(|v| v.as_u64())
        .map(|v| Importance::new(v as u8));

    let filter = RecallFilter {
        memory_type,
        project,
        tags,
        min_importance,
        top_k,
        privacy_context: None,
        explore_pure: false,
        regression_query: None,
        read_only: false,
    };

    // v0.9.7 导航层（预注册实验证实：导航改变候选集 > 随机方向扩展，
    // 配对 bootstrap P=96.3%）。门控 LRC_DAOTI_NAVIGATE=1 默认关闭 →
    // 行为与既有版本逐字节一致。信号由 daoti 研究资产推演产出、经 recall
    // 的 navigation 参数注入（显式携带优先）；P2.5 起支持自动向 daoti_daemon
    // 拉取信号（daemon 不可达 → None → 无导航基线）。
    let mut nav_signal = if std::env::var("LRC_DAOTI_NAVIGATE")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        arguments
            .get("navigation")
            .and_then(crate::engine::navigation::NavigationSignal::from_json)
    } else {
        None
    };
    // P2.5：显式信号缺省且门控开启时，尝试从常驻 daemon 获取（失败降级 None）
    if nav_signal.is_none()
        && std::env::var("LRC_DAOTI_NAVIGATE")
            .map(|v| v == "1")
            .unwrap_or(false)
    {
        nav_signal = fetch_daoti_navigation(query).await;
    }

    // 先完成可能发生网络等待的 LLM 翻译，再获取 memory_store 锁。
    // 这样网络超时不会阻塞其他记忆读写请求。
    let llm_config = state.llm_api.read().await.clone();
    let enriched_query = if llm_config.is_configured() {
        let keywords =
            crate::engine::llm_translator::translate_memory_query(&llm_config, query).await;
        let translated: String = keywords.join(" ");
        if translated.is_empty() || translated.trim() == query {
            query.to_string()
        } else {
            format!("{} {}", translated, query)
        }
    } else {
        query.to_string()
    };

    let mut store = state.memory_store.lock().await;

    // 根据 lrc_mode 选择检索方法（使用富化后的查询）
    let result = if lrc_mode == "deep" {
        // 导航信号在场时：多视图 deep 检索 + N 路 RRF（改变候选集，非重排）；
        // 导航内部已含基线视图，返回 None（信号无有效方向）时回退单查询 deep。
        match nav_signal {
            Some(sig) => {
                match crate::engine::navigation::navigated_deep_recall(
                    &mut store,
                    &enriched_query,
                    &filter,
                    focus_depth,
                    &sig,
                ) {
                    Some(rr) => Ok(rr),
                    None => store.trapezoid_focus_recall(&enriched_query, &filter, focus_depth),
                }
            }
            None => store.trapezoid_focus_recall(&enriched_query, &filter, focus_depth),
        }
    } else {
        store.recall(&enriched_query, &filter)
    };

    match result {
        Ok(result) => {
            let mut text = format!(
                "记忆检索结果 (共 {} 条匹配，记忆库共 {} 条，模式: {})\n\n",
                result.memories.len(),
                result.total,
                if lrc_mode == "deep" {
                    "深度语义检索"
                } else {
                    "关键词匹配"
                }
            );

            if result.memories.is_empty() {
                text.push_str("未找到相关记忆。使用 remember 工具添加新记忆。\n");
            } else {
                for (i, m) in result.memories.iter().enumerate() {
                    let score = result.scores.get(i).unwrap_or(&0.0);
                    let mem_num = i + 1;
                    text.push_str(&format!(
                        "（记忆 #{mem_num} · 匹配度 {:.1}%）\n",
                        score * 100.0
                    ));
                    text.push_str(&format!("内容: {}\n", m.content));
                    if let Some(ref cat) = m.bagua_category {
                        text.push_str(&format!("分类: {} | ", cat));
                    }
                    text.push_str(&format!(
                        "类型: {} | 重要性: {}/10",
                        m.memory_type.as_str(),
                        m.importance.value()
                    ));
                    if !m.tags.is_empty() {
                        text.push_str(&format!(" | 标签: {}", m.tags.join(", ")));
                    }
                    if let Some(ref proj) = m.project {
                        text.push_str(&format!(" | 项目: {}", proj));
                    }
                    if let Some(ref preview_gua) = m.daoti_preview_gua {
                        text.push_str(&format!(" | 道体预判卦: {}", preview_gua));
                    }
                    if let Some(ref preview_bagua) = m.daoti_preview_bagua {
                        text.push_str(&format!(" | 道体预判八卦: {}", preview_bagua));
                    }
                    if let Some(ref preview_version) = m.daoti_preview_version {
                        text.push_str(&format!(" | 道体预判版本: {}", preview_version));
                    }
                    text.push_str(&format!("\nID: `{}`\n\n", m.id));
                }
                text.push_str("💡 在回复中引用记忆时，请使用「（根据记忆 #N）」的格式标注来源，让用户能看见和信任记忆的存在。\n");
            }

            // ═══ 记录层联想补全（v0.9.8：真正的记忆联想）═══
            //
            // 上面所有通路（fast / deep / RRF / 状态机联想链）**全部是相似度驱动**，
            // 只能在"语义相近"的记忆里找。本段补的是另一类：
            // 由记录层（event_id 共同经历 / entities 共享实体 / source_ids 谱系）
            // 推导出的、**语义可以毫不相似**的关联。
            //
            // 典型形态：用户查「游西湖」→ 补出同一次杭州之行的「吃楼外楼」。
            // 这是任何相似度算法都给不出的连接，也是"记录层"存在的全部意义。
            //
            // 失败静默：联想是附加价值，不可因它失败而影响检索主结果
            //（与详情页间接关联的处理一致）。
            let seed_ids: Vec<String> = result.memories.iter().map(|m| m.id.clone()).collect();
            let assoc = store
                .expand_associations(&seed_ids, &filter, ASSOCIATION_EXPAND_MAX)
                .unwrap_or_default();
            if !assoc.is_empty() {
                append_associated_memories(&mut text, &assoc);
            }
            // 锁在此处释放：道体调用是**网络等待**，绝不能持锁进行
            //（否则道体挂起会阻塞所有记忆读写；与既有"先翻译再取锁"同一纪律）。
            drop(store);

            // ═══ 符号层联想（道体 §4.4 状态机循环 + §5.4 落边，v0.9.8）═══
            //
            // ★target = 本次查询本身（用户裁定「目标就是召回」，不做解析）。
            // 该调用返回**候选卦 + 朝目标还差几跳**，与上面的记录层联想
            // 是两套证据（符号层尚无记忆 ID），故分区显示、不混排。
            //
            // ★两个分区统一走 `append_symbolic_layer`（2026-09-18 审查 G5b/G8 修复）：
            //   · G8：此前本段与 `handle_recall_enhanced` 逐字重复
            //   · G5b：此前两次调用**串行**（最坏 4s + 6s = 10s 叠加），
            //     现为**并发 + 总预算 6s**（详见该函数文档）
            //
            // 门控默认全关 ⇒ 行为与既有版本逐字节一致（且不发任何请求）。
            let seeds_for_symbolic: Vec<(String, String)> = result
                .memories
                .iter()
                .take(8)
                .map(|m| (m.id.clone(), m.content.clone()))
                .collect();
            append_symbolic_layer(&mut text, query, &seeds_for_symbolic).await;

            let call_result = ToolCallResult {
                content: vec![TextContent {
                    content_type: "text".into(),
                    text,
                }],
            };
            make_response(id, to_json_value_safe(&call_result))
        }
        Err(e) => make_error(id, -32603, &format!("检索失败: {}", e)),
    }
}

/// 处理 batch_remember 工具调用 — 批量注入多条记忆
///
/// 批量上限为 200 条，每条记忆必须包含 content 字段
async fn handle_batch_remember(
    state: &AppState,
    arguments: &serde_json::Value,
    id: Option<serde_json::Value>,
) -> JsonRpcResponse {
    let memories_array = match arguments.get("memories").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return make_error(id, -32602, "缺少参数: memories (数组)"),
    };

    if memories_array.is_empty() {
        let text = "批量注入完成: 0 条记忆（空列表）";
        let call_result = ToolCallResult {
            content: vec![TextContent {
                content_type: "text".into(),
                text: text.to_string(),
            }],
        };
        return make_response(id, to_json_value_safe(&call_result));
    }

    if memories_array.len() > 200 {
        return make_error(
            id,
            -32602,
            &format!("批量注入上限为 200 条，收到 {} 条", memories_array.len()),
        );
    }

    // 批次级 event_id（v0.9.7 降低填写负担）：
    // 同一次经历往往就是"一次批量写入"——此时让调用方在**批次级**写一次 event_id，
    // 而不是在每条记忆里重复写 N 次。逐条的 event_id 优先（允许批次内混入其他经历）。
    // 这一设计不降低语义要求（仍需知情者给出经历标识），只消除**机械重复**。
    let batch_event_id = arguments
        .get("event_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut memories = Vec::with_capacity(memories_array.len());
    for item in memories_array {
        let content = match item.get("content").and_then(|v| v.as_str()) {
            Some(c) => c.to_string(),
            None => {
                return make_error(id, -32602, "每条记忆必须包含 content 字段");
            }
        };

        let memory_type_str = item
            .get("memory_type")
            .and_then(|v| v.as_str())
            .unwrap_or("fact");
        let memory_type = MemoryType::try_parse(memory_type_str).unwrap_or(MemoryType::Fact);

        let project = item
            .get("project")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let tags: Vec<String> = item
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let importance = item
            .get("importance")
            .and_then(|v| v.as_u64())
            .map(|v| Importance::new(v as u8))
            .unwrap_or_default();

        let memory = Memory::new(
            content,
            memory_type,
            project,
            tags,
            importance,
            None, // ttl_days
        );

        // 事件维度（批量路径同样支持"共同经历"）
        // 逐条的 event_id 优先；缺失时回退到批次级 event_id（避免逐个重复填写）
        let mut memory = memory;
        memory.event_id = item
            .get("event_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| batch_event_id.clone());
        memory.entities = parse_entities(item.get("entities"));

        memories.push(memory);
    }

    let total = memories.len();
    let mut store = state.memory_store.lock().await;
    match store.remember_batch(memories) {
        Ok(saved) => {
            // 批量写入成功后，确保项目元信息存在（用于前端显示项目名而非指纹）
            // 失败时仅记录日志，不阻塞记忆写入
            let src_dir_for_meta = state.src_dir.clone();
            if !src_dir_for_meta.is_empty() {
                tokio::task::spawn_blocking(move || {
                    let path = std::path::Path::new(&src_dir_for_meta);
                    let data_dir = crate::data_dir::DataDir::for_project(path);
                    if let Err(e) = data_dir.ensure_meta(path) {
                        eprintln!("[warn] 写入项目元信息失败（不影响记忆写入）: {}", e);
                    }
                })
                .await
                .ok();
            }

            let text = format!(
                "批量注入完成: {} 条记忆\n\
                 ══════════════════════\n\
                 总计: {} 条记忆已写入记忆库\n\
                 \n\
                 ID 列表:\n{}",
                total,
                saved.len(),
                saved
                    .iter()
                    .map(|m| format!("  - {}: {}", m.id, m.summary()))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            let call_result = ToolCallResult {
                content: vec![TextContent {
                    content_type: "text".into(),
                    text,
                }],
            };
            make_response(id, to_json_value_safe(&call_result))
        }
        Err(e) => make_error(id, -32603, &format!("批量注入失败: {}", e)),
    }
}

/// 处理 list_memories 工具调用 — 分页列出记忆
///
/// 支持按类型、项目、标签过滤，按重要性/时间排序，分页
async fn handle_list_memories(
    state: &AppState,
    arguments: &serde_json::Value,
    id: Option<serde_json::Value>,
) -> JsonRpcResponse {
    let memory_type = arguments
        .get("memory_type")
        .and_then(|v| v.as_str())
        .and_then(MemoryType::try_parse);

    let project = arguments
        .get("project")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let tags: Vec<String> = arguments
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let sort_by = arguments
        .get("sort_by")
        .and_then(|v| v.as_str())
        .map(|s| match s {
            "importance" => SortBy::Importance,
            "last_accessed" => SortBy::LastAccessed,
            _ => SortBy::CreatedAt,
        })
        .unwrap_or_default();

    let order = arguments
        .get("order")
        .and_then(|v| v.as_str())
        .map(|s| match s {
            "asc" => SortOrder::Asc,
            _ => SortOrder::Desc,
        })
        .unwrap_or_default();

    let limit = arguments
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20)
        .clamp(1, 100) as usize;

    let offset = arguments
        .get("offset")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    let filter = ListFilter {
        memory_type,
        project,
        tags,
        sort_by,
        order,
        limit,
        offset,
        privacy_context: None,
    };

    let store = state.memory_store.lock().await;
    match store.list_memories(&filter) {
        Ok((memories, total)) => {
            let mut text = format!("记忆列表 (共 {} 条，本页 {} 条)\n\n", total, memories.len());

            if memories.is_empty() {
                text.push_str("暂无记忆。使用 remember 工具添加记忆。\n");
            } else {
                for m in &memories {
                    text.push_str(&format!("### {}\n", m.summary()));
                    text.push_str(&format!("ID: `{}`\n", m.id));
                    text.push_str(&format!(
                        "类型: {} | 重要性: {}/10 | 创建: {}\n",
                        m.memory_type.as_str(),
                        m.importance.value(),
                        m.created_at.format("%Y-%m-%d %H:%M")
                    ));
                    if let Some(ref proj) = m.project {
                        text.push_str(&format!("项目: {}\n", proj));
                    }
                    if !m.tags.is_empty() {
                        text.push_str(&format!("标签: {}\n", m.tags.join(", ")));
                    }
                    if let Some(ref preview_gua) = m.daoti_preview_gua {
                        text.push_str(&format!("道体预判卦: {}\n", preview_gua));
                    }
                    if let Some(ref preview_bagua) = m.daoti_preview_bagua {
                        text.push_str(&format!("道体预判八卦: {}\n", preview_bagua));
                    }
                    if let Some(ref preview_version) = m.daoti_preview_version {
                        text.push_str(&format!("道体预判版本: {}\n", preview_version));
                    }
                    text.push('\n');
                }
            }

            let call_result = ToolCallResult {
                content: vec![TextContent {
                    content_type: "text".into(),
                    text,
                }],
            };
            make_response(id, to_json_value_safe(&call_result))
        }
        Err(e) => make_error(id, -32603, &format!("列表查询失败: {}", e)),
    }
}

/// 解析 entities 参数（人/地/时/物）
///
/// 容错策略：数组内非对象项、缺 name 或 name 为空白的项均跳过；
/// kind 非法时退化为 `EntityKind::Other`（不报错，避免因拼写问题丢整条记忆）。
fn parse_entities(v: Option<&serde_json::Value>) -> Vec<EventEntity> {
    let Some(arr) = v.and_then(|x| x.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in arr {
        let Some(name) = item.get("name").and_then(|n| n.as_str()) else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let kind = item
            .get("kind")
            .and_then(|k| k.as_str())
            .and_then(EntityKind::try_parse)
            .unwrap_or_default();
        out.push(EventEntity::new(name, kind));
    }
    out
}

/// 处理 remember 工具调用 — 写入单条记忆
///
/// 支持记忆类型、项目、标签、重要性、TTL、隐私级别等参数
async fn handle_remember(
    state: &AppState,
    arguments: &serde_json::Value,
    id: Option<serde_json::Value>,
) -> JsonRpcResponse {
    let content = match arguments.get("content").and_then(|q| q.as_str()) {
        Some(q) => q,
        None => return make_error(id, -32602, "缺少参数: content"),
    };

    let memory_type_str = arguments
        .get("memory_type")
        .and_then(|v| v.as_str())
        .unwrap_or("fact");
    let memory_type = MemoryType::try_parse(memory_type_str).unwrap_or(MemoryType::Fact);

    let project = arguments
        .get("project")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let tags: Vec<String> = arguments
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let importance = arguments
        .get("importance")
        .and_then(|v| v.as_u64())
        .map(|v| Importance::new(v as u8))
        .unwrap_or_default();

    let ttl_days = arguments
        .get("ttl_days")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    // 隐私权限参数
    let privacy_level = arguments
        .get("privacy_level")
        .and_then(|v| v.as_str())
        .and_then(PrivacyLevel::try_parse)
        .unwrap_or_default();

    let session_id = arguments
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let user_id = arguments
        .get("user_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // 事件维度：event_id（同一次经历）+ entities（人/地/时/物）
    let event_id = arguments
        .get("event_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let entities = parse_entities(arguments.get("entities"));

    let preview_gua = arguments
        .get("daoti_preview_gua")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let preview_bagua = arguments
        .get("daoti_preview_bagua")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let preview_version = arguments
        .get("daoti_preview_version")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let memory = Memory::new(
        content.to_string(),
        memory_type,
        project,
        tags,
        importance,
        ttl_days,
    )
    .with_privacy(privacy_level, session_id, user_id);

    let mut memory = memory;
    memory.event_id = event_id;
    memory.entities = entities;
    memory.daoti_preview_gua = preview_gua;
    memory.daoti_preview_bagua = preview_bagua;
    memory.daoti_preview_version = preview_version;

    let mut store = state.memory_store.lock().await;
    match store.remember(memory) {
        Ok(saved) => {
            // 写入成功后，确保项目元信息存在（用于前端显示项目名而非指纹）
            // 失败时仅记录日志，不阻塞记忆写入
            let src_dir_for_meta = state.src_dir.clone();
            if !src_dir_for_meta.is_empty() {
                tokio::task::spawn_blocking(move || {
                    let path = std::path::Path::new(&src_dir_for_meta);
                    let data_dir = crate::data_dir::DataDir::for_project(path);
                    if let Err(e) = data_dir.ensure_meta(path) {
                        eprintln!("[warn] 写入项目元信息失败（不影响记忆写入）: {}", e);
                    }
                })
                .await
                .ok();
            }

            let text = format!(
                "已记住 (ID: {})\n\
                 ──────────────────\n\
                 内容: {}\n\
                 类型: {} | 重要性: {}/10 | 隐私: {}\n\
                 拓扑深度: {:.2} | 版本: {}\n\
                 \n\
                 ✅ 下次你问相关问题时，AI 会自动检索到这条记忆。",
                saved.id,
                saved.content,
                saved.memory_type.as_str(),
                saved.importance.value(),
                saved.privacy_level.as_str(),
                saved.topological_depth,
                saved.version
            );
            let call_result = ToolCallResult {
                content: vec![TextContent {
                    content_type: "text".into(),
                    text,
                }],
            };
            make_response(id, to_json_value_safe(&call_result))
        }
        Err(e) => make_error(id, -32603, &format!("写入失败: {}", e)),
    }
}

/// 处理 MCP tools/call 请求 — 路由到对应的工具处理函数
async fn handle_tools_call(
    state: &AppState,
    params: &serde_json::Value,
    id: Option<serde_json::Value>,
) -> JsonRpcResponse {
    let name = match params.get("name").and_then(|n| n.as_str()) {
        Some(n) => n,
        None => return make_error(id, -32602, "缺少 tool name"),
    };

    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    match name {
        // === 写入记忆（已提取到 handle_remember）===
        "remember" => {
            return handle_remember(state, &arguments, id).await;
        }

        // === 批量注入（已提取到 handle_batch_remember）===
        "batch_remember" => {
            return handle_batch_remember(state, &arguments, id).await;
        }

        // === 记忆检索（已提取到 handle_recall）===
        "recall" => {
            return handle_recall(state, &arguments, id).await;
        }

        "forget" => {
            let memory_id = match arguments.get("memory_id").and_then(|q| q.as_str()) {
                Some(q) => q,
                None => return make_error(id, -32602, "缺少参数: memory_id"),
            };

            let mut store = state.memory_store.lock().await;
            match store.forget(memory_id) {
                Ok(true) => {
                    let text = format!("已删除记忆: {}", memory_id);
                    let call_result = ToolCallResult {
                        content: vec![TextContent {
                            content_type: "text".into(),
                            text,
                        }],
                    };
                    make_response(id, to_json_value_safe(&call_result))
                }
                Ok(false) => {
                    let text = format!("未找到记忆: {}（可能已被删除）", memory_id);
                    let call_result = ToolCallResult {
                        content: vec![TextContent {
                            content_type: "text".into(),
                            text,
                        }],
                    };
                    make_response(id, to_json_value_safe(&call_result))
                }
                Err(e) => make_error(id, -32603, &format!("删除失败: {}", e)),
            }
        }

        "update_memory" => {
            let memory_id = match arguments.get("memory_id").and_then(|q| q.as_str()) {
                Some(q) => q,
                None => return make_error(id, -32602, "缺少参数: memory_id"),
            };
            let new_content = match arguments.get("content").and_then(|q| q.as_str()) {
                Some(q) => q,
                None => return make_error(id, -32602, "缺少参数: content"),
            };
            let new_importance = arguments
                .get("importance")
                .and_then(|v| v.as_u64())
                .map(|v| Importance::new(v as u8));

            let mut store = state.memory_store.lock().await;
            match store.update_memory(memory_id, new_content, new_importance) {
                Ok(Some(old)) => {
                    let text = format!(
                        "已更新记忆: {}\n旧内容: {}\n新内容: {}",
                        memory_id, old.content, new_content
                    );
                    let call_result = ToolCallResult {
                        content: vec![TextContent {
                            content_type: "text".into(),
                            text,
                        }],
                    };
                    make_response(id, to_json_value_safe(&call_result))
                }
                Ok(None) => {
                    let text = format!("未找到记忆: {}", memory_id);
                    let call_result = ToolCallResult {
                        content: vec![TextContent {
                            content_type: "text".into(),
                            text,
                        }],
                    };
                    make_response(id, to_json_value_safe(&call_result))
                }
                Err(e) => make_error(id, -32603, &format!("更新失败: {}", e)),
            }
        }

        // === 记忆列表（已提取到 handle_list_memories）===
        "list_memories" => {
            return handle_list_memories(state, &arguments, id).await;
        }

        // === 记忆关联（记录层→多类型关系网络）===
        "associations" => {
            let memory_id = match arguments.get("memory_id").and_then(|v| v.as_str()) {
                Some(v) => v,
                None => return make_error(id, -32602, "缺少参数: memory_id"),
            };
            let relation_filter = arguments.get("relation").and_then(|v| v.as_str());

            let store = state.memory_store.lock().await;
            match store.associations(memory_id) {
                Ok(all_assoc) => {
                    // hub 实体（泛化实体）清单：**让过滤可见**（§3.45.5）
                    let hubs = store.hub_entities().unwrap_or_default();
                    let hub_note = if hubs.is_empty() {
                        String::new()
                    } else {
                        let list: Vec<String> = hubs
                            .iter()
                            .map(|(n, k, c, t)| {
                                format!(
                                    "`{}`({}, {}/{}={:.0}%)",
                                    n,
                                    k.as_str(),
                                    c,
                                    t,
                                    if *t == 0 {
                                        0.0
                                    } else {
                                        *c as f64 / *t as f64 * 100.0
                                    }
                                )
                            })
                            .collect();
                        format!(
                            "\n\n注：以下实体因过于泛化已被跳过（其「共享」近乎恒真、无区分度）。\
分母为该**记忆所属项目内**的记忆数（§3.49：按项目内占比判定，避免被其他项目稀释）：{}",
                            list.join("、")
                        )
                    };
                    let filtered: Vec<_> = all_assoc
                        .into_iter()
                        .filter(|a| relation_filter.map(|r| a.relation == r).unwrap_or(true))
                        .collect();
                    if filtered.is_empty() {
                        let text = format!(
                            "未找到关联。\n\n可能原因：该记忆未记录 event_id（无共同经历），\
没有可共享的实体（entities 为空），也不是结晶产物（无 source_ids）。\n\
记录层字段缺失时无法推导关联——这不是「没有关系」，而是「关系未被记录」。{}",
                            hub_note
                        );
                        let call_result = ToolCallResult {
                            content: vec![TextContent {
                                content_type: "text".into(),
                                text,
                            }],
                        };
                        return make_response(id, to_json_value_safe(&call_result));
                    }
                    let mut text = format!("关联图谱（{} 条）\n\n", filtered.len());
                    for a in &filtered {
                        text.push_str(&format!("- [{}] `{}`\n", a.relation, a.memory_id));
                        text.push_str(&format!("  依据: {}\n", a.why));
                        text.push_str(&format!("  内容: {}\n\n", a.content_preview));
                    }
                    text.push_str(&hub_note);
                    let call_result = ToolCallResult {
                        content: vec![TextContent {
                            content_type: "text".into(),
                            text,
                        }],
                    };
                    make_response(id, to_json_value_safe(&call_result))
                }
                Err(e) => make_error(id, -32603, &format!("关联查询失败: {}", e)),
            }
        }

        // === 联想（关联图 + 多跳结构化推理）===
        "association_graph" => {
            let memory_id = match arguments.get("memory_id").and_then(|v| v.as_str()) {
                Some(v) => v,
                None => return make_error(id, -32602, "缺少参数: memory_id"),
            };
            // 节点上限：默认 50（足以覆盖典型同经历簇），可由调用方收紧以防大簇爆图
            let max_nodes = arguments
                .get("max_nodes")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .unwrap_or(50)
                .clamp(2, 500);

            let store = state.memory_store.lock().await;
            match store.association_graph(memory_id, max_nodes) {
                Ok(g) => {
                    if g.nodes.is_empty() {
                        let text = "未找到该记忆，或它没有任何关联。\n\n\
记录层字段（event_id / entities / source_ids）缺失时无法推导关联——\
这不是「没有关系」，而是「关系未被记录」。"
                            .to_string();
                        let call_result = ToolCallResult {
                            content: vec![TextContent {
                                content_type: "text".into(),
                                text,
                            }],
                        };
                        return make_response(id, to_json_value_safe(&call_result));
                    }

                    let mut text = format!(
                        "关联图（以 `{}` 为中心）\n\
                         ══════════════════════\n\
                         节点 {} 个 | 直接关联 {} 条 | 间接关联 {} 条{}\n\n\
                         【直接关联】由记录直接推导\n",
                        memory_id,
                        g.nodes.len(),
                        g.direct_count,
                        g.indirect_count,
                        if g.truncated {
                            "  ⚠ 节点数已达上限，图被截断（结果不完整）"
                        } else {
                            ""
                        }
                    );
                    for e in g.edges.iter().filter(|e| e.hops == 1) {
                        text.push_str(&format!(
                            "- [{}] `{}`\n  依据: {}\n",
                            e.relation, e.to, e.why
                        ));
                    }
                    let indirect: Vec<_> = g.edges.iter().filter(|e| e.hops >= 2).collect();
                    if indirect.is_empty() {
                        text.push_str("\n【间接关联】无（未发现经中间记忆可达的新节点）\n");
                    } else {
                        text.push_str(
                            "\n【间接关联】由结构传递推出 —— 两条记忆间**无直接记录**，\
但经中间记忆可达。这类关联是「推理」而非「匹配」的结果：\n",
                        );
                        for e in indirect {
                            text.push_str(&format!(
                                "- `{}`\n  依据: {}\n  路径: {}\n",
                                e.to,
                                e.why,
                                e.path.join(" → ")
                            ));
                        }
                    }
                    let call_result = ToolCallResult {
                        content: vec![TextContent {
                            content_type: "text".into(),
                            text,
                        }],
                    };
                    make_response(id, to_json_value_safe(&call_result))
                }
                Err(e) => make_error(id, -32603, &format!("关联图构造失败: {}", e)),
            }
        }

        "memory_stats" => {
            let store = state.memory_store.lock().await;
            match store.stats() {
                Ok(stats) => {
                    let mut text = String::from("记忆库统计\n\n");
                    text.push_str(&format!("- 记忆总数: {} 条\n", stats.total_memories));
                    text.push_str(&format!("- 已过期: {} 条\n", stats.expired_count));
                    text.push_str(&format!(
                        "- 存储大小: {} bytes\n\n",
                        stats.storage_size_bytes
                    ));

                    text.push_str("### 类型分布\n");
                    let mut types: Vec<(&String, &usize)> = stats.by_type.iter().collect();
                    types.sort_by(|a, b| b.1.cmp(a.1));
                    for (t, count) in types {
                        text.push_str(&format!("- `{}`: {} 条\n", t, count));
                    }

                    // 记录层覆盖度（v0.9.7）：联想的前提是"共同经历"被记录。
                    // 若 with_event 长期为 0，关联推导必然为空——那是"前提缺失"，
                    // 不是"机制无效"（PREREG §3.42/§3.43、方法论 92）。
                    //
                    // **分母口径（重要，防误读）**：此处百分比的分母是**全库记忆**，
                    // 包含自动索引的 code_context 片段——而它们**本就不该带 event_id**。
                    // 实测（PREREG §3.47）某真实库 4477 条中 code_context 占绝大多数，
                    // 用全库做分母会**严重低估**填写率（0.0x% 量级），
                    // 让人误以为"没人填"。故此处**显式标注分母**，并额外给出
                    // "经历候选"口径（排除 code_context/导入语料/合成产物），
                    // 后者才是"应该填的"真实基数。
                    text.push_str("\n### 记录层覆盖度（联想的前提）\n");
                    let pct_of = |n: usize, d: usize| {
                        if d == 0 {
                            0.0
                        } else {
                            n as f64 / d as f64 * 100.0
                        }
                    };
                    // 经历候选 = 全库 - code_context - 导入语料 - 合成产物
                    let code_ctx = stats.by_type.get("code_context").copied().unwrap_or(0);
                    let synth = stats.by_type.get("synthesis").copied().unwrap_or(0);
                    let incident_base = stats.total_memories.saturating_sub(code_ctx + synth);
                    text.push_str(&format!(
                        "- 带事件 ID（共同经历）: {} 条（占全库 {:.2}%，占经历候选 {:.2}%）\n",
                        stats.with_event_count,
                        pct_of(stats.with_event_count, stats.total_memories),
                        pct_of(stats.with_event_count, incident_base),
                    ));
                    text.push_str(&format!(
                        "- 带实体（人/地/时/物）: {} 条（占经历候选 {:.2}%）\n",
                        stats.with_entity_count,
                        pct_of(stats.with_entity_count, incident_base),
                    ));
                    text.push_str(&format!(
                        "- 已形成事件簇: {} 个\n",
                        stats.event_cluster_count
                    ));
                    text.push_str(&format!(
                        "- 类型为「经历」: {} 条\n",
                        stats.experience_count
                    ));
                    text.push_str(&format!(
                        "  分母说明：全库 {} 条；经历候选 {} 条\
（= 全库 − code_context {} − synthesis {}，这两类本就不该带事件 ID）\n",
                        stats.total_memories, incident_base, code_ctx, synth
                    ));
                    if stats.with_event_count == 0 {
                        text.push_str(
                            "  ⚠ 尚无记忆携带事件 ID ⇒ 关联推导无输入。\
写入时请为同一次经历的多条记忆填相同的 `event_id`。\n",
                        );
                    }

                    text.push_str("\n### 项目分布\n");
                    let mut projects: Vec<(&String, &usize)> = stats.by_project.iter().collect();
                    projects.sort_by(|a, b| b.1.cmp(a.1));
                    // 构建项目指纹→可读名映射表（用于 MCP 工具返回可读项目名而非指纹）
                    // 性能：126 个项目 < 50ms；非项目指纹的 key（如 "_global_" / 自定义名称）不命中映射表，按原值显示
                    let project_map: std::collections::HashMap<String, String> =
                        crate::data_dir::list_all_projects()
                            .into_iter()
                            .map(|p| (p.fingerprint, p.display_name))
                            .collect();
                    for (proj, count) in projects {
                        // 优先显示可读名（命中映射表时），未命中时按原值显示
                        let display = project_map.get(proj).map(|s| s.as_str()).unwrap_or(proj);
                        // 若可读名与原值不同，附带显示原指纹（便于调试与跨 IDE 一致性校验）
                        if display != proj.as_str() {
                            text.push_str(&format!("- `{} ({})`: {} 条\n", display, proj, count));
                        } else {
                            text.push_str(&format!("- `{}`: {} 条\n", display, count));
                        }
                    }

                    let call_result = ToolCallResult {
                        content: vec![TextContent {
                            content_type: "text".into(),
                            text,
                        }],
                    };
                    make_response(id, to_json_value_safe(&call_result))
                }
                Err(e) => make_error(id, -32603, &format!("统计查询失败: {}", e)),
            }
        }
        "archive" => {
            let mut store = state.memory_store.lock().await;
            match store.archive_expired() {
                Ok(count) => {
                    let text = if count > 0 {
                        format!("已归档 {} 条过期记忆到冷存储。", count)
                    } else {
                        "当前无过期记忆需要归档。".to_string()
                    };
                    let call_result = ToolCallResult {
                        content: vec![TextContent {
                            content_type: "text".into(),
                            text,
                        }],
                    };
                    make_response(id, to_json_value_safe(&call_result))
                }
                Err(e) => make_error(id, -32603, &format!("归档失败: {}", e)),
            }
        }
        "search_code" => {
            let query = match arguments.get("query").and_then(|q| q.as_str()) {
                Some(q) => q,
                None => return make_error(id, -32602, "缺少参数: query"),
            };
            let top_k = arguments
                .get("top_k")
                .and_then(|v| v.as_u64())
                .unwrap_or(5)
                .clamp(1, 100) as usize;

            // LLM 查询翻译：如果配置了 LLM API，先将自然语言翻译为关键词
            let llm_config = state.llm_api.read().await.clone();
            let keywords = if llm_config.is_configured() {
                crate::engine::llm_translator::translate_query(&llm_config, query).await
            } else {
                vec![query.to_string()]
            };

            let result = match safe_code_search(state.manager.clone(), keywords, top_k).await {
                Ok(result) => result,
                Err(SearchError::LockTimeout) => {
                    return make_error(id, -32001, "搜索服务繁忙，请稍后重试")
                }
                Err(SearchError::ExecutionTimeout) => {
                    return make_error(id, -32002, "搜索超时，请缩小查询范围后重试")
                }
                Err(SearchError::Panic) => {
                    return make_error(id, -32003, "搜索内部错误，服务已保持运行")
                }
            };

            // 格式化为可读文本
            let mut text = format!(
                "代码检索结果 (共 {} 条，索引库 {} 个片段)\n\n",
                result.returned, result.total_indexed
            );

            if result.results.is_empty() {
                text.push_str("未找到相关代码片段。\n");
                text.push_str(&format!(
                    "提示: 索引库路径为 {}，当前已索引 {} 个文件。",
                    state.src_dir,
                    match tokio::time::timeout(
                        SEARCH_LOCK_TIMEOUT,
                        state.manager.clone().lock_owned(),
                    )
                    .await
                    {
                        Ok(manager) => manager.get_stats().file_count,
                        Err(_) => 0,
                    }
                ));
            } else {
                for r in &result.results {
                    text.push_str(&format!(
                        "### #{}. {} (相似度: {:.1}%)\n",
                        r.rank,
                        r.chunk.name,
                        r.score * 100.0
                    ));
                    text.push_str(&format!(
                        "`{}:L{}-L{}`\n",
                        r.chunk.file_path, r.chunk.start_line, r.chunk.end_line
                    ));
                    if let Some(ref doc) = r.chunk.doc_comment {
                        text.push_str(&format!("{}\n", doc));
                    }
                    text.push_str(&format!(
                        "```{}\n{}\n```\n\n",
                        r.chunk.language, r.chunk.content
                    ));
                }
            }

            let call_result = ToolCallResult {
                content: vec![TextContent {
                    content_type: "text".into(),
                    text,
                }],
            };
            make_response(id, to_json_value_safe(&call_result))
        }

        "codebase_stats" => {
            let manager = state.manager.lock().await;
            let stats = manager.get_stats();

            let mut text = String::from("代码库索引统计\n\n");
            text.push_str(&format!("- 已索引文件: {} 个\n", stats.file_count));
            text.push_str(&format!("- 代码片段: {} 个\n", stats.total_chunks));
            text.push_str(&format!("- 平均行数: {:.1} 行/片段\n\n", stats.avg_lines));
            text.push_str("### 类型分布\n");
            let mut types: Vec<(&String, &usize)> = stats.type_counts.iter().collect();
            types.sort_by(|a, b| b.1.cmp(a.1));
            for (t, count) in types {
                text.push_str(&format!("- `{}`: {} 个\n", t, count));
            }

            let call_result = ToolCallResult {
                content: vec![TextContent {
                    content_type: "text".into(),
                    text,
                }],
            };
            make_response(id, to_json_value_safe(&call_result))
        }

        // === 系统健康监控 ===
        "system_health" => {
            let store = match state.memory_store.try_lock() {
                Ok(store) => store,
                Err(_) => {
                    return make_response(
                        id,
                        serde_json::json!({
                            "status": "degraded",
                            "lock_busy": true,
                            "message": "记忆库正在后台更新，请稍后重试",
                        }),
                    );
                }
            };
            match store.dao_metrics_snapshot() {
                Ok(snapshot) => {
                    let mut text = String::from("═══════════════════════════════════\n");
                    text.push_str("  系统健康度监控仪表\n");
                    text.push_str("═══════════════════════════════════\n\n");

                    text.push_str("### 核心指标\n");
                    text.push_str(&format!(
                        "- 一致性评分: {:.1}%\n",
                        snapshot.dao_isomorphism_score * 100.0
                    ));
                    text.push_str(&format!(
                        "- 分布熵: {:.3} (最大 3.0)\n",
                        snapshot.bagua_entropy
                    ));
                    text.push_str(&format!(
                        "- 合成比率: {:.1}%\n\n",
                        snapshot.synthesis_ratio * 100.0
                    ));

                    text.push_str("### 记忆库统计\n");
                    text.push_str(&format!("- 活跃记忆: {} 条\n", snapshot.active_memories));
                    text.push_str(&format!(
                        "- 结晶记忆: {} 条\n",
                        snapshot.crystallized_memories
                    ));
                    text.push_str(&format!("- 已归档: {} 条\n\n", snapshot.archived_memories));

                    text.push_str("### 运行统计\n");
                    text.push_str(&format!("- 编码次数: {}\n", snapshot.encodings_total));
                    text.push_str(&format!("- 合成次数: {}\n", snapshot.compositions_total));
                    text.push_str(&format!("- 检索次数: {}\n", snapshot.recalls_total));
                    text.push_str(&format!("- 修正次数: {}\n", snapshot.corrections_total));

                    if snapshot.dao_isomorphism_score < 0.5 {
                        text.push_str("\n⚠️ 一致性评分偏低，建议检查编码器或增加训练数据。\n");
                    }
                    if snapshot.bagua_entropy < 0.5 && snapshot.active_memories > 10 {
                        text.push_str("\n⚠️ 分布过于集中，记忆可能存在类别偏差。\n");
                    }

                    let call_result = ToolCallResult {
                        content: vec![TextContent {
                            content_type: "text".into(),
                            text,
                        }],
                    };
                    make_response(id, to_json_value_safe(&call_result))
                }
                Err(e) => make_error(id, -32603, &format!("健康度采集失败: {}", e)),
            }
        }

        // === 用户修正记忆 ===
        "correct_memory" => {
            let memory_id = match arguments.get("memory_id").and_then(|q| q.as_str()) {
                Some(q) => q,
                None => return make_error(id, -32602, "缺少参数: memory_id"),
            };
            let new_content = match arguments.get("content").and_then(|q| q.as_str()) {
                Some(q) => q,
                None => return make_error(id, -32602, "缺少参数: content"),
            };
            let reason = arguments.get("reason").and_then(|v| v.as_str());

            let mut store = state.memory_store.lock().await;
            match store.correct_memory(memory_id, new_content, reason) {
                Ok(Some(memory)) => {
                    let text = format!(
                        "已修正记忆 (ID: {})\n\
                         ──────────────────\n\
                         新内容: {}\n\
                         修正原因: {}\n\
                         \n\
                         ✅ 记忆已更新，修正历史已保留。",
                        memory.id,
                        memory.content,
                        reason.unwrap_or("未提供")
                    );
                    let call_result = ToolCallResult {
                        content: vec![TextContent {
                            content_type: "text".into(),
                            text,
                        }],
                    };
                    make_response(id, to_json_value_safe(&call_result))
                }
                Ok(None) => {
                    let text = format!("未找到记忆: {}", memory_id);
                    let call_result = ToolCallResult {
                        content: vec![TextContent {
                            content_type: "text".into(),
                            text,
                        }],
                    };
                    make_response(id, to_json_value_safe(&call_result))
                }
                Err(e) => make_error(id, -32603, &format!("修正失败: {}", e)),
            }
        }

        // === 双路检索增强（已提取到 handle_recall_enhanced）===
        "recall_enhanced" => {
            return handle_recall_enhanced(state, &arguments, id).await;
        }

        _ => make_error(id, -32601, &format!("未知工具: {}", name)),
    }
}

// ==================== Axum 路由处理器 ====================

async fn mcp_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<JsonRpcRequest>,
) -> axum::response::Response {
    let response =
        dispatch_request(&state, &request.method, request.params.as_ref(), request.id).await;
    match response {
        Some(resp) => (StatusCode::OK, Json(resp)).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

/// 健康检查端点 — 返回 JSON 详细状态
///
/// 响应包含服务运行阶段、索引进度、记忆库统计等关键信息。
/// 供桌面端 sidecar_manager 健康检查和仪表盘状态页面使用。
///
/// 状态说明：
///   - "starting": 服务刚启动，后台索引尚未开始
///   - "indexing": 后台索引正在进行中，代码搜索可能不完整
///   - "running": 索引已完成，所有功能就绪
async fn health_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let indexing_complete = state
        .indexing_complete
        .load(std::sync::atomic::Ordering::Relaxed);
    let uptime = chrono::Utc::now() - state.started_at;
    let uptime_seconds = uptime.num_seconds().max(0);

    // P0-1 修复（G-014 / INV-008）：/health handler 改用 try_lock，避免长任务持锁时卡死
    // 根因：索引/结晶 task 长时间持有 manager.lock() 或 memory_store.lock() 时，
    //   /health 获取不到锁会卡死（实测 5049ms 超时），导致桌面端 SidecarHealthMonitor
    //   误判 sidecar 已死，显示"无法连接到 API 服务"。
    // 修复：使用 try_lock，获取不到锁时返回 None/0，/health 永远不会卡死。
    //   副作用：索引期间 /health 返回的 file_count/total_chunks/memory_total 可能为 None/0，
    //   但这是可接受的——/health 的核心职责是存活探测，不是精确统计。

    // 获取索引统计信息（try_lock，获取不到返回 None）
    let mut lock_busy = false;
    let (file_count, total_chunks) = if indexing_complete {
        match state.manager.try_lock() {
            Ok(manager) => {
                let stats = manager.get_stats();
                (Some(stats.file_count), Some(stats.total_chunks))
            }
            Err(_) => {
                // manager 锁被长任务持有，返回降级状态而不是伪造正常统计。
                lock_busy = true;
                (None, None)
            }
        }
    } else {
        (None, None)
    };

    // 获取记忆库统计（try_read，获取不到返回 0）
    // v0.8.21 P0-06：同时检测锁是否被持有，设置 lock_busy 标志
    let (memory_total, memory_lock_busy) = match state.memory_store.try_lock() {
        Ok(store) => (store.stats().map(|s| s.total_memories).unwrap_or(0), false),
        Err(_) => (0, true), // 锁被长任务持有，返回 0 + lock_busy=true
    };

    lock_busy |= memory_lock_busy;

    // 判断服务阶段
    let status = if indexing_complete {
        "running"
    } else if uptime_seconds < 5 {
        "starting"
    } else {
        "indexing"
    };

    // v0.8.22 P0-1 修复（hcse-resilience-validator Round3）：
    //   原实现：state.llm_api.read().await.is_configured() — 阻塞式读锁
    //   根因：当 tokio runtime 繁忙时，此 .await 点堆积请求，每个消耗一个 worker 线程，
    //         导致所有 16 个 worker 线程被耗尽，HTTP 服务器完全无法响应（12s 超时）
    //   修复：改用 AtomicBool 无锁读取，永远不阻塞
    let llm_configured = state
        .llm_configured_atomic
        .load(std::sync::atomic::Ordering::Relaxed);

    let response = HealthResponse {
        status,
        service: "loong-recall",
        version: env!("CARGO_PKG_VERSION"),
        uptime_seconds,
        indexing: IndexingStatus {
            complete: indexing_complete,
            file_count,
            total_chunks,
        },
        memory: MemoryBrief {
            total: memory_total,
        },
        src_dir: state.src_dir.clone(),
        data_dir: state.data_dir.clone(),
        llm_configured,
        lock_busy,
    };

    (StatusCode::OK, Json(response))
}

/// 仪表盘 CSS 端点 — 返回编译时嵌入的 app.css
///
/// 仪表盘 HTML 引用 app.css 作为外部样式表，此端点将编译时嵌入的
/// app.css 内容以 `text/css` MIME 类型返回。
async fn app_css_handler() -> axum::response::Response<String> {
    const APP_CSS: &str = include_str!("../static/app.css");
    axum::response::Response::builder()
        .header("Content-Type", "text/css; charset=utf-8")
        .body(APP_CSS.to_string())
        .unwrap_or_else(|e| {
            eprintln!("[server] app.css 响应构建失败: {}", e);
            axum::response::Response::builder()
                .body("/* app.css 加载失败 */".to_string())
                .unwrap_or_else(|_| {
                    axum::response::Response::new("/* app.css 加载失败 */".to_string())
                })
        })
}

/// 仪表盘 JavaScript 端点 — 返回编译时嵌入的 app.js
///
/// 仪表盘 HTML 引用 app.js 作为外部脚本，此端点将编译时嵌入的
/// app.js 内容以 `application/javascript` MIME 类型返回。
async fn app_js_handler() -> axum::response::Response<String> {
    const APP_JS: &str = include_str!("../static/app.js");
    axum::response::Response::builder()
        .header("Content-Type", "application/javascript; charset=utf-8")
        .body(APP_JS.to_string())
        .unwrap_or_else(|e| {
            eprintln!("[server] app.js 响应构建失败: {}", e);
            axum::response::Response::builder()
                .body("console.error('app.js 加载失败')".to_string())
                .unwrap_or_else(|_| {
                    axum::response::Response::new("console.error('app.js 加载失败')".to_string())
                })
        })
}

/// 龙忆设计系统 v1.0 — 色彩与排版变量（colors_and_type.css）
///
/// v0.6.0 UI 重构：仪表盘 HTML 引用 colors_and_type.css 作为设计系统基础变量，
/// 包含墨韵/宣纸/玉色/朱砂/金色/水蓝色阶、字体系统（无衬线/衬线/等宽）、
/// 8pt 间距系统、动效变量等。此端点将编译时嵌入的 CSS 内容返回。
async fn colors_and_type_css_handler() -> axum::response::Response<String> {
    const CSS: &str = include_str!("../static/colors_and_type.css");
    axum::response::Response::builder()
        .header("Content-Type", "text/css; charset=utf-8")
        .body(CSS.to_string())
        .unwrap_or_else(|e| {
            eprintln!("[server] colors_and_type.css 响应构建失败: {}", e);
            axum::response::Response::builder()
                .body("/* colors_and_type.css 加载失败 */".to_string())
                .unwrap_or_else(|_| {
                    axum::response::Response::new("/* colors_and_type.css 加载失败 */".to_string())
                })
        })
}

/// 龙忆设计系统 v1.0 — 全局组件库（components.css）
///
/// v0.6.0 UI 重构：仪表盘 HTML 引用 components.css 作为全局组件库，
/// 包含按钮、卡片、输入框、Tooltip、Skeleton 骨架屏、Toast 通知条、
/// 洛书九宫格加载动画等组件样式。此端点将编译时嵌入的 CSS 内容返回。
async fn components_css_handler() -> axum::response::Response<String> {
    const CSS: &str = include_str!("../static/components.css");
    axum::response::Response::builder()
        .header("Content-Type", "text/css; charset=utf-8")
        .body(CSS.to_string())
        .unwrap_or_else(|e| {
            eprintln!("[server] components.css 响应构建失败: {}", e);
            axum::response::Response::builder()
                .body("/* components.css 加载失败 */".to_string())
                .unwrap_or_else(|_| {
                    axum::response::Response::new("/* components.css 加载失败 */".to_string())
                })
        })
}

/// 龙忆设计系统 v1.0 — Logo 资源端点
///
/// v0.6.0 UI 重构：仪表盘引用 /assets/logo/*.svg 作为品牌 Logo，
/// 此端点将编译时嵌入的 SVG 内容以 `image/svg+xml` MIME 类型返回。
/// 支持的文件名：logo-primary.svg、logo-horizontal.svg
async fn logo_asset_handler(
    axum::extract::Path(filename): axum::extract::Path<String>,
) -> axum::response::Response<String> {
    // 编译时嵌入所有 Logo SVG 文件
    const LOGO_PRIMARY: &str = include_str!("../static/assets/logo/logo-primary.svg");
    const LOGO_HORIZONTAL: &str = include_str!("../static/assets/logo/logo-horizontal.svg");
    let content = match filename.as_str() {
        "logo-primary.svg" => Some(LOGO_PRIMARY),
        "logo-horizontal.svg" => Some(LOGO_HORIZONTAL),
        _ => None,
    };
    match content {
        Some(svg) => axum::response::Response::builder()
            .header("Content-Type", "image/svg+xml; charset=utf-8")
            .body(svg.to_string())
            .unwrap_or_else(|e| {
                eprintln!("[server] Logo SVG 响应构建失败: {}", e);
                axum::response::Response::new(String::new())
            }),
        None => axum::response::Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(format!("<!-- Logo not found: {} -->", filename))
            .unwrap_or_else(|_| axum::response::Response::new(String::new())),
    }
}

/// 龙忆设计系统 v1.0 — 图标资源端点
///
/// v0.6.0 UI 重构：仪表盘引用 /assets/icons/*.svg 作为导航和功能图标，
/// 此端点将编译时嵌入的 SVG 内容以 `image/svg+xml` MIME 类型返回。
/// 支持的图标：dashboard/search-lrc/captain-log/trust/benchmark/audit/
/// baga/health/decay/luoshu/memory/crystallization/privacy/network/integrity
async fn icon_asset_handler(
    axum::extract::Path(filename): axum::extract::Path<String>,
) -> axum::response::Response<String> {
    // 编译时嵌入所有图标 SVG 文件
    const ICON_DASHBOARD: &str = include_str!("../static/assets/icons/icon-dashboard.svg");
    const ICON_SEARCH: &str = include_str!("../static/assets/icons/icon-search-lrc.svg");
    const ICON_CAPTAIN_LOG: &str = include_str!("../static/assets/icons/icon-captain-log.svg");
    const ICON_TRUST: &str = include_str!("../static/assets/icons/icon-trust.svg");
    const ICON_BENCHMARK: &str = include_str!("../static/assets/icons/icon-benchmark.svg");
    const ICON_AUDIT: &str = include_str!("../static/assets/icons/icon-audit.svg");
    const ICON_BAGUA: &str = include_str!("../static/assets/icons/icon-bagua.svg");
    const ICON_HEALTH: &str = include_str!("../static/assets/icons/icon-health.svg");
    const ICON_DECAY: &str = include_str!("../static/assets/icons/icon-decay.svg");
    const ICON_LUOSHU: &str = include_str!("../static/assets/icons/icon-luoshu.svg");
    const ICON_MEMORY: &str = include_str!("../static/assets/icons/icon-memory.svg");
    const ICON_CRYSTALLIZATION: &str =
        include_str!("../static/assets/icons/icon-crystallization.svg");
    const ICON_PRIVACY: &str = include_str!("../static/assets/icons/icon-privacy.svg");
    const ICON_NETWORK: &str = include_str!("../static/assets/icons/icon-network.svg");
    const ICON_INTEGRITY: &str = include_str!("../static/assets/icons/icon-integrity.svg");
    // v0.8.7 Step 1：补全 21 个缺失的 icon-*.svg 嵌入（HCSE-P1 修复）
    const ICON_PROJECT: &str = include_str!("../static/assets/icons/icon-project.svg");
    const ICON_SAVE: &str = include_str!("../static/assets/icons/icon-save.svg");
    const ICON_EXPORT: &str = include_str!("../static/assets/icons/icon-export.svg");
    const ICON_IMPORT: &str = include_str!("../static/assets/icons/icon-import.svg");
    const ICON_LIGHTNING: &str = include_str!("../static/assets/icons/icon-lightning.svg");
    const ICON_CONFIG: &str = include_str!("../static/assets/icons/icon-config.svg");
    const ICON_INFO: &str = include_str!("../static/assets/icons/icon-info.svg");
    const ICON_SMILE: &str = include_str!("../static/assets/icons/icon-smile.svg");
    const ICON_CHART: &str = include_str!("../static/assets/icons/icon-chart.svg");
    const ICON_CHECK: &str = include_str!("../static/assets/icons/icon-check.svg");
    const ICON_CLOUD: &str = include_str!("../static/assets/icons/icon-cloud.svg");
    const ICON_DELETE: &str = include_str!("../static/assets/icons/icon-delete.svg");
    const ICON_DOWNLOAD: &str = include_str!("../static/assets/icons/icon-download.svg");
    const ICON_EMBED: &str = include_str!("../static/assets/icons/icon-embed.svg");
    const ICON_FOLDER: &str = include_str!("../static/assets/icons/icon-folder.svg");
    const ICON_LLM: &str = include_str!("../static/assets/icons/icon-llm.svg");
    const ICON_SEARCH_GENERIC: &str = include_str!("../static/assets/icons/icon-search.svg");
    const ICON_SETTINGS: &str = include_str!("../static/assets/icons/icon-settings.svg");
    const ICON_USER: &str = include_str!("../static/assets/icons/icon-user.svg");
    const ICON_USERS: &str = include_str!("../static/assets/icons/icon-users.svg");
    const ICON_WARNING: &str = include_str!("../static/assets/icons/icon-warning.svg");
    // v0.8.7 Step 2：补全 3 个 power-*.svg 嵌入（HCSE-P1 修复）
    const POWER_BALANCE: &str = include_str!("../static/assets/icons/power-balance.svg");
    const POWER_GROWTH: &str = include_str!("../static/assets/icons/power-growth.svg");
    const POWER_SHIELD: &str = include_str!("../static/assets/icons/power-shield.svg");
    // v0.9.7：emoji→SVG 图标体系迁移新增 15 个图标嵌入
    const ICON_BULB: &str = include_str!("../static/assets/icons/icon-bulb.svg");
    const ICON_CHECK_CIRCLE: &str = include_str!("../static/assets/icons/icon-check-circle.svg");
    const ICON_CLOCK: &str = include_str!("../static/assets/icons/icon-clock.svg");
    const ICON_CLOSE: &str = include_str!("../static/assets/icons/icon-close.svg");
    const ICON_DOCUMENT: &str = include_str!("../static/assets/icons/icon-document.svg");
    const ICON_EMPTY: &str = include_str!("../static/assets/icons/icon-empty.svg");
    const ICON_ERROR_CIRCLE: &str = include_str!("../static/assets/icons/icon-error-circle.svg");
    const ICON_MENU: &str = include_str!("../static/assets/icons/icon-menu.svg");
    const ICON_MORE: &str = include_str!("../static/assets/icons/icon-more.svg");
    const ICON_PLUG: &str = include_str!("../static/assets/icons/icon-plug.svg");
    const ICON_REFRESH: &str = include_str!("../static/assets/icons/icon-refresh.svg");
    const ICON_SPARKLE: &str = include_str!("../static/assets/icons/icon-sparkle.svg");
    const ICON_STAR: &str = include_str!("../static/assets/icons/icon-star.svg");
    const ICON_STAR_EMPTY: &str = include_str!("../static/assets/icons/icon-star-empty.svg");
    const ICON_TOOLS: &str = include_str!("../static/assets/icons/icon-tools.svg");

    let content = match filename.as_str() {
        "icon-dashboard.svg" => Some(ICON_DASHBOARD),
        "icon-search-lrc.svg" => Some(ICON_SEARCH),
        "icon-captain-log.svg" => Some(ICON_CAPTAIN_LOG),
        "icon-trust.svg" => Some(ICON_TRUST),
        "icon-benchmark.svg" => Some(ICON_BENCHMARK),
        "icon-audit.svg" => Some(ICON_AUDIT),
        "icon-bagua.svg" => Some(ICON_BAGUA),
        "icon-health.svg" => Some(ICON_HEALTH),
        "icon-decay.svg" => Some(ICON_DECAY),
        "icon-luoshu.svg" => Some(ICON_LUOSHU),
        "icon-memory.svg" => Some(ICON_MEMORY),
        "icon-crystallization.svg" => Some(ICON_CRYSTALLIZATION),
        "icon-privacy.svg" => Some(ICON_PRIVACY),
        "icon-network.svg" => Some(ICON_NETWORK),
        "icon-integrity.svg" => Some(ICON_INTEGRITY),
        // v0.8.7 Step 1：补全 21 个缺失的 icon-*.svg 路由匹配
        "icon-project.svg" => Some(ICON_PROJECT),
        "icon-save.svg" => Some(ICON_SAVE),
        "icon-export.svg" => Some(ICON_EXPORT),
        "icon-import.svg" => Some(ICON_IMPORT),
        "icon-lightning.svg" => Some(ICON_LIGHTNING),
        "icon-config.svg" => Some(ICON_CONFIG),
        "icon-info.svg" => Some(ICON_INFO),
        "icon-smile.svg" => Some(ICON_SMILE),
        "icon-chart.svg" => Some(ICON_CHART),
        "icon-check.svg" => Some(ICON_CHECK),
        "icon-cloud.svg" => Some(ICON_CLOUD),
        "icon-delete.svg" => Some(ICON_DELETE),
        "icon-download.svg" => Some(ICON_DOWNLOAD),
        "icon-embed.svg" => Some(ICON_EMBED),
        "icon-folder.svg" => Some(ICON_FOLDER),
        "icon-llm.svg" => Some(ICON_LLM),
        "icon-search.svg" => Some(ICON_SEARCH_GENERIC),
        "icon-settings.svg" => Some(ICON_SETTINGS),
        "icon-user.svg" => Some(ICON_USER),
        "icon-users.svg" => Some(ICON_USERS),
        "icon-warning.svg" => Some(ICON_WARNING),
        // v0.8.7 Step 2：补全 3 个 power-*.svg 路由匹配
        "power-balance.svg" => Some(POWER_BALANCE),
        "power-growth.svg" => Some(POWER_GROWTH),
        "power-shield.svg" => Some(POWER_SHIELD),
        // v0.9.7：emoji→SVG 图标体系迁移新增 15 个图标路由
        "icon-bulb.svg" => Some(ICON_BULB),
        "icon-check-circle.svg" => Some(ICON_CHECK_CIRCLE),
        "icon-clock.svg" => Some(ICON_CLOCK),
        "icon-close.svg" => Some(ICON_CLOSE),
        "icon-document.svg" => Some(ICON_DOCUMENT),
        "icon-empty.svg" => Some(ICON_EMPTY),
        "icon-error-circle.svg" => Some(ICON_ERROR_CIRCLE),
        "icon-menu.svg" => Some(ICON_MENU),
        "icon-more.svg" => Some(ICON_MORE),
        "icon-plug.svg" => Some(ICON_PLUG),
        "icon-refresh.svg" => Some(ICON_REFRESH),
        "icon-sparkle.svg" => Some(ICON_SPARKLE),
        "icon-star.svg" => Some(ICON_STAR),
        "icon-star-empty.svg" => Some(ICON_STAR_EMPTY),
        "icon-tools.svg" => Some(ICON_TOOLS),
        _ => None,
    };
    match content {
        Some(svg) => axum::response::Response::builder()
            .header("Content-Type", "image/svg+xml; charset=utf-8")
            .body(svg.to_string())
            .unwrap_or_else(|e| {
                eprintln!("[server] 图标 SVG 响应构建失败: {}", e);
                axum::response::Response::new(String::new())
            }),
        None => axum::response::Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(format!("<!-- Icon not found: {} -->", filename))
            .unwrap_or_else(|_| axum::response::Response::new(String::new())),
    }
}

/// 仪表盘端点 — 返回内嵌的 Web UI 仪表盘 HTML
///
/// 产品化核心入口：用户启动服务后访问 http://localhost:3099/dashboard
/// 即可看到可视化记忆统计、健康状态、船长日志和信任中心。
/// HTML 文件在编译时嵌入，无外部依赖，支持离线使用。
async fn dashboard_handler() -> axum::response::Html<&'static str> {
    // 编译时嵌入仪表盘 HTML（单文件，包含内联 CSS + JS）
    const DASHBOARD_HTML: &str = include_str!("../static/index.html");
    axum::response::Html(DASHBOARD_HTML)
}

/// 根路径重定向到仪表盘
///
/// 桌面端在 navigate_main_to_dashboard 时可能访问根路径 `/`，
/// 此 handler 将其重定向到 `/dashboard`。
/// 使用 302 临时重定向（兼容性更好）。
async fn root_redirect_handler() -> impl IntoResponse {
    (
        StatusCode::FOUND, // 302 Temporary Redirect
        [("Location", "/dashboard")],
    )
}

// ==================== 配置 API 端点（仪表盘设置页面用） ====================

/// 项目信息响应结构体（V2: 项目指纹 + 规范化路径 + 可读名称）
#[derive(Debug, Serialize)]
struct ProjectInfoResponse {
    /// 项目源码目录
    src_dir: String,
    /// 规范化后的绝对路径
    canonical_path: String,
    /// 项目指纹（SHA256 前 16 字符）
    fingerprint: String,
    /// 可读显示名（custom_name > auto_name > fingerprint 前 8 位）
    display_name: String,
    /// 自动提取的名称（路径末段）
    auto_name: String,
    /// 用户自定义名称（None 表示未自定义）
    #[serde(skip_serializing_if = "Option::is_none")]
    custom_name: Option<String>,
}

/// GET /api/project/info — 获取当前项目的指纹、路径和可读名称
async fn project_info_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    use crate::project_id;
    let src_path = std::path::Path::new(&state.src_dir);
    let (fingerprint, canonical_path) = project_id::project_fingerprint_with_path(src_path);

    // 读取项目元信息（meta.json），获取可读显示名
    // meta.json 不存在时用 auto_name 兜底
    let data_dir = crate::data_dir::DataDir::for_project(src_path);
    let (display_name, auto_name, custom_name) = match data_dir.read_meta() {
        Ok(Some(meta)) => {
            let dn = meta.display_name();
            (dn, meta.auto_name, meta.custom_name)
        }
        _ => {
            // meta.json 不存在或读取失败：用 auto_name_from_path 兜底
            let auto = project_id::auto_name_from_path(&canonical_path);
            (auto.clone(), auto, None)
        }
    };

    Json(ProjectInfoResponse {
        src_dir: state.src_dir.clone(),
        canonical_path,
        fingerprint,
        display_name,
        auto_name,
        custom_name,
    })
}

/// GET /api/projects/list — 列出所有已知项目的元信息（用于前端构建"指纹→名称"映射表）
///
/// 遍历 `~/.loong-recall/projects/` 目录下的所有指纹目录，
/// 返回每个项目的指纹、可读名称、路径、记忆数等信息。
///
/// 前端在仪表盘渲染前先调用此端点，构建 `fingerprintToName` 映射表，
/// 让项目分布区域显示可读名称而非 16 位指纹。
///
/// 性能：126 个项目实测 < 50ms
async fn projects_list_handler() -> impl IntoResponse {
    let items = crate::data_dir::list_all_projects();
    Json(serde_json::json!({
        "total": items.len(),
        "projects": items,
    }))
}

/// v0.8.1 抽取：获取 LLM 配置状态（供 /api/config 和 /v1/config 共用）
///
/// 返回 JSON 包含：llm_configured, llm_type, llm_model, llm_base_url
pub async fn get_llm_config_state(llm_api: &Arc<RwLock<LlmApiConfig>>) -> serde_json::Value {
    let llm = llm_api.read().await;
    let (configured, llm_type, llm_model, llm_base_url) = match &*llm {
        LlmApiConfig::OpenAI {
            model, endpoint, ..
        } => (
            true,
            "openai".to_string(),
            Some(model.clone()),
            Some(endpoint.clone()),
        ),
        LlmApiConfig::Ollama { model, host } => (
            true,
            "ollama".to_string(),
            Some(model.clone()),
            Some(host.clone()),
        ),
        LlmApiConfig::None => (false, "none".to_string(), None, None),
    };
    serde_json::json!({
        "llm_configured": configured,
        "llm_type": llm_type,
        "llm_model": llm_model,
        "llm_base_url": llm_base_url,
    })
}

/// GET /api/config — 获取当前 LLM 配置状态
async fn config_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(get_llm_config_state(&state.llm_api).await)
}

/// v0.8.1 抽取：更新 LLM API Key 配置（供 /api/config/llm 和 /v1/config/llm 共用）
///
/// 请求体: `{ "llm_api": "openai:sk-xxx:gpt-4o-mini" }`
/// 保存到全局配置文件，并立即生效用于后续查询翻译。
pub async fn update_llm_config(
    memory_store: &Arc<Mutex<MemoryStore<JsonPersistence>>>,
    llm_api: &Arc<RwLock<LlmApiConfig>>,
    llm_configured_atomic: &Arc<AtomicBool>,
    body: serde_json::Value,
) -> (StatusCode, Json<serde_json::Value>) {
    let llm_str = match body.get("llm_api").and_then(|v| v.as_str()) {
        Some(s) => s.trim().to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "success": false,
                    "message": "缺少 llm_api 字段"
                })),
            );
        }
    };

    // 空字符串表示清除配置
    if llm_str.is_empty() {
        // v0.7.1 P2-1 修复：先更新内存状态（持锁时间最短），再用 spawn_blocking 执行文件 I/O
        {
            let mut llm = llm_api.write().await;
            *llm = LlmApiConfig::None;
        }
        // v0.8.22 P0-1 修复：同步 AtomicBool 无锁缓存（与 RwLock 状态保持一致）
        llm_configured_atomic.store(false, std::sync::atomic::Ordering::Relaxed);
        // v0.7.1 P2-1 修复：用 spawn_blocking 包裹同步文件 I/O，避免阻塞 Tokio worker 线程
        let save_result = tokio::task::spawn_blocking(|| {
            save_llm_to_config(None)?;
            save_llm_to_wizard_json("")
        })
        .await;
        match save_result {
            Ok(Err(e)) => eprintln!("[配置] 清除 LLM API 配置失败: {e}"),
            Err(e) => eprintln!("[配置] 异步保存任务失败: {e}"),
            Ok(Ok(_)) => {}
        }
        // v0.5.5：更新 MemoryStore 的 LLM 配置状态
        {
            let store = memory_store.lock().await;
            store.set_llm_configured(false);
        }
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": "LLM API 配置已清除",
                "llm_configured": false
            })),
        );
    }

    // 解析配置
    match LlmApiConfig::parse(&llm_str) {
        Ok(config) => {
            // SSRF 防护：校验配置中的目标地址（持久化前拒绝 metadata/链路本地等）
            let target = match &config {
                LlmApiConfig::OpenAI { endpoint, .. } => endpoint.clone(),
                LlmApiConfig::Ollama { host, .. } => {
                    // Ollama host 可能为 host 或 host:port，构造 http URL 校验
                    if host.starts_with("http://") || host.starts_with("https://") {
                        host.clone()
                    } else {
                        format!("http://{}", host)
                    }
                }
                LlmApiConfig::None => String::new(),
            };
            if !target.is_empty() {
                if let Err(e) = crate::url_safety::validate_http_url(&target) {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({
                            "success": false,
                            "message": format!("目标地址校验失败: {}", e)
                        })),
                    );
                }
                // SSRF 防护：DNS 解析复核（防 DNS rebinding）
                if let Err(e) = crate::url_safety::check_dns_safety(&target).await {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({
                            "success": false,
                            "message": format!("目标地址校验失败: {}", e)
                        })),
                    );
                }
            }
            // v0.5.4 修复：unreachable!() 替换为安全的错误返回
            // parse() 方法理论上不会返回 None，但防御性编程应处理所有情况
            let (llm_type, model) = match &config {
                LlmApiConfig::OpenAI { model, .. } => ("openai", model.clone()),
                LlmApiConfig::Ollama { model, .. } => ("ollama", model.clone()),
                LlmApiConfig::None => {
                    let err_msg = "内部错误：LLM 配置解析返回了未预期的 None 变体";
                    eprintln!("[LRC·错误] {}", err_msg);
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "error": err_msg
                        })),
                    );
                }
            };
            // v0.7.1 P2-1 修复：先更新内存状态（持锁时间最短），再用 spawn_blocking 执行文件 I/O
            {
                let mut llm = llm_api.write().await;
                *llm = config;
            }
            // v0.8.22 P0-1 修复：同步 AtomicBool 无锁缓存（与 RwLock 状态保持一致）
            llm_configured_atomic.store(true, std::sync::atomic::Ordering::Relaxed);
            // v0.7.1 P2-1 修复：用 spawn_blocking 包裹同步文件 I/O，避免阻塞 Tokio worker 线程
            let llm_str_for_save = llm_str.clone();
            let save_result = tokio::task::spawn_blocking(move || {
                save_llm_to_config(Some(&llm_str_for_save))?;
                save_llm_to_wizard_json(&llm_str_for_save)
            })
            .await;
            match save_result {
                Ok(Err(e)) => eprintln!("[配置] 保存 LLM API 配置失败: {e}"),
                Err(e) => eprintln!("[配置] 异步保存任务失败: {e}"),
                Ok(Ok(_)) => {}
            }
            // v0.5.5：更新 MemoryStore 的 LLM 配置状态
            {
                let store = memory_store.lock().await;
                store.set_llm_configured(true);
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "success": true,
                    "message": format!("LLM API 配置成功 ({})", llm_type),
                    "llm_configured": true,
                    "llm_type": llm_type,
                    "llm_model": model
                })),
            )
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "success": false,
                "message": format!("配置格式错误: {}. 支持格式: openai:sk-xxx:gpt-4o-mini 或 ollama:localhost:llama3", e)
            })),
        ),
    }
}

/// POST /api/config/llm — 更新 LLM API Key 配置
///
/// 请求体: `{ "llm_api": "openai:sk-xxx:gpt-4o-mini" }`
/// 保存到全局配置文件，并立即生效用于后续查询翻译。
async fn config_llm_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    update_llm_config(
        &state.memory_store,
        &state.llm_api,
        &state.llm_configured_atomic,
        body,
    )
    .await
}

/// 保存 LLM API 配置到全局配置文件
fn save_llm_to_config(llm_api: Option<&str>) -> crate::errors::LrcResult<()> {
    let mut cfg = crate::config::LrcConfig::load();
    cfg.llm_api = llm_api.map(|s| s.to_string());
    // v0.9.7（GLOBAL_CODE_REVIEW_REPORT P1-6）：LrcConfig::save 已返回 LrcError，
    // 此处直接上浮，不再降级为 String。
    cfg.save()
}

/// v0.9.0 开发模式隔离：获取正确的 wizard.json 路径
///
/// 开发模式下使用 %APPDATA%\LoongRecall\dev\wizard.json，与稳定版完全隔离。
fn wizard_json_path() -> Option<std::path::PathBuf> {
    let appdata = std::env::var("APPDATA").ok()?;
    let loong_dir = std::path::PathBuf::from(appdata).join("LoongRecall");
    let is_dev = std::env::var("LRC_DEV_MODE").is_ok();
    let path = if is_dev {
        loong_dir.join("dev").join("wizard.json")
    } else {
        loong_dir.join("wizard.json")
    };
    Some(path)
}

/// 仪表盘修改 LLM 配置后，同步到 wizard.json，确保桌面端和仪表盘配置一致。
/// API Key 使用 AES-256-GCM 加密存储（与桌面端一致）。
fn save_llm_to_wizard_json(llm_api: &str) -> LrcResult<()> {
    let wizard_path =
        wizard_json_path().ok_or_else(|| LrcError::config("读取 APPDATA 环境变量失败"))?;

    // 读取现有 wizard.json（如果存在），保留非 LLM 字段
    let mut wizard: serde_json::Value = if wizard_path.exists() {
        let content = std::fs::read_to_string(&wizard_path)
            .map_err(|e| LrcError::io(format!("读取 wizard.json 失败: {}", e)))?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // 解析 LLM API 字符串并更新 wizard.json
    // 格式：openai:sk-xxx:gpt-4o:https://api.openai.com/v1
    //       ollama:llama3:http://localhost:11434
    let parts: Vec<&str> = llm_api.splitn(4, ':').collect();

    match parts.first() {
        Some(&"openai") => {
            wizard["llm_configured"] = serde_json::json!(true);
            wizard["llm_type"] = serde_json::json!("openai");
            // API Key 加密存储
            if let Some(api_key) = parts.get(1) {
                let cleaned_key: String = api_key
                    .trim()
                    .chars()
                    .filter(|c| !c.is_control() || *c == ' ')
                    .collect();
                if !cleaned_key.is_empty() {
                    let encrypted = crate::crypto::encrypt_api_key(&cleaned_key)
                        .map_err(|e| LrcError::crypto(format!("加密 API Key 失败: {}", e)))?;
                    wizard["encrypted_api_key"] = serde_json::json!(encrypted);
                }
            }
            if let Some(model) = parts.get(2) {
                if !model.is_empty() {
                    wizard["llm_model"] = serde_json::json!(model);
                }
            }
            if let Some(base_url) = parts.get(3) {
                if !base_url.is_empty() {
                    wizard["llm_base_url"] = serde_json::json!(base_url);
                }
            }
        }
        Some(&"ollama") => {
            wizard["llm_configured"] = serde_json::json!(true);
            wizard["llm_type"] = serde_json::json!("ollama");
            wizard["encrypted_api_key"] = serde_json::json!("");
            if let Some(model) = parts.get(1) {
                if !model.is_empty() {
                    wizard["llm_model"] = serde_json::json!(model);
                }
            }
            if let Some(host) = parts.get(2) {
                if !host.is_empty() {
                    wizard["llm_base_url"] = serde_json::json!(host);
                }
            }
        }
        _ => {
            // 清除 LLM 配置
            wizard["llm_configured"] = serde_json::json!(false);
            wizard["llm_type"] = serde_json::json!("none");
            wizard["encrypted_api_key"] = serde_json::json!("");
        }
    }

    // 确保目录存在
    if let Some(parent) = wizard_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| LrcError::io(format!("创建 wizard.json 目录失败: {}", e)))?;
    }

    // 写入 wizard.json
    let json_str = serde_json::to_string_pretty(&wizard)
        .map_err(|e| LrcError::parse(format!("序列化 wizard.json 失败: {}", e)))?;
    std::fs::write(&wizard_path, json_str)
        .map_err(|e| LrcError::io(format!("写入 wizard.json 失败: {}", e)))?;

    eprintln!(
        "[配置] LLM 配置已同步到 wizard.json: {}",
        wizard_path.display()
    );
    Ok(())
}

// ==================== 嵌入模型管理 API（v0.6.0+） ====================

/// 全局下载任务状态标志
///
/// 跟踪后台下载线程的运行状态：
/// - `false`：无下载任务或上次下载已结束
/// - `true`：下载任务正在运行中
static EMBEDDER_DOWNLOADING: AtomicBool = AtomicBool::new(false);

/// 可用的嵌入模型白名单
///
/// v0.9.7 修复（GLOBAL_CODE_REVIEW_REPORT P3 质量「模型 ID 常量重复」）：
///   根因：模型 ID 此前在 4 处独立硬编码（本处白名单、`bin/server.rs` 的 `RECOMMENDED_MODELS`、
///         `engine/model_resolver.rs::selected_model_id`、`engine/luoshu_encoder_ml.rs::detect_default_model_by_lang`），
///         且存在**值不一致**——本白名单要求 `intfloat/multilingual-e5-small`，
///         而 `bin/server.rs` 的 `model list` 展示 `multilingual-e5-small`（无 org 前缀），
///         用户照展示值调用会被白名单直接拒绝。
///   修复：收敛到 Layer 1 中立模块 [`crate::model_ids`]（无 feature 门控，
///         Layer 2 引擎亦可引用，避免 Layer 2 反向依赖本模块造成 feature 耦合）。
pub use crate::model_ids::{
    AVAILABLE_EMBEDDER_MODELS, MODEL_ALL_MINILM_L6_V2, MODEL_BGE_BASE_ZH, MODEL_BGE_SMALL_ZH,
    MODEL_MULTILINGUAL_E5_SMALL,
};

// ---------- 请求 / 响应结构体 ----------

/// 嵌入模型下载请求
#[derive(Debug, Deserialize)]
struct EmbedderDownloadRequest {
    model_id: String,
    /// 镜像源："hf-mirror" 或 "modelscope"（不区分大小写，未指定时默认 hf-mirror）
    mirror: Option<String>,
}

/// 嵌入模型应用请求
#[derive(Debug, Deserialize)]
struct EmbedderApplyRequest {
    model_id: String,
}

/// 嵌入模型连通性测试请求
#[derive(Debug, Deserialize)]
struct EmbedderTestRequest {
    model_id: String,
    mirror: Option<String>,
}

/// 嵌入模型状态响应
#[derive(Debug, Serialize)]
struct EmbedderStatusResponse {
    model_id: String,
    status: String,
    models_dir: String,
    available_models: Vec<String>,
}

/// 嵌入模型下载响应
#[derive(Debug, Serialize)]
struct EmbedderDownloadResponse {
    success: bool,
    message: String,
    model_id: String,
}

/// 嵌入模型应用响应
#[derive(Debug, Serialize)]
struct EmbedderApplyResponse {
    success: bool,
    message: String,
    model_id: String,
}

/// 嵌入模型连通性测试响应
#[derive(Debug, Serialize)]
struct EmbedderTestResponse {
    success: bool,
    mirror: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    latency_ms: Option<u64>,
    model_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

/// 工具检测结果项
#[derive(Debug, Serialize)]
struct ToolDetectItem {
    name: String,
    /// 工具类型："ide"、"agent" 或 "extension"
    #[serde(rename = "type")]
    tool_type: String,
    installed: bool,
    version: Option<String>,
    path: Option<String>,
    /// 只有快捷方式扫描命中时才允许进入自动展示。
    shortcut_confirmed: bool,
}

/// 工具检测响应
#[derive(Debug, Serialize)]
struct ToolsDetectResponse {
    tools: Vec<ToolDetectItem>,
}

// ---------- Handler 实现 ----------

/// GET /api/embedder/status — 获取嵌入模型状态
///
/// 检查 models/ 目录下是否已有已下载的模型文件，返回当前状态。
/// - `ready`：模型文件已就位
/// - `not_downloaded`：models/ 目录存在但模型文件缺失
/// - `unknown`：models/ 目录不存在
async fn embedder_status_handler(
    State(_state): State<Arc<AppState>>,
) -> (StatusCode, Json<serde_json::Value>) {
    // v0.9.0 修复：使用统一模型目录 ~/.loong-recall/models/（而非相对 cwd）
    let models_dir = crate::engine::model_resolver::default_models_dir();
    // 与实际编码器共用当前生效模型配置，避免设置页与状态接口各自使用固定模型。
    let default_model_id = crate::engine::model_resolver::selected_model_id();

    // 检查是否已下载：本地目录名以 "--" 替换 "/"
    let local_dir = default_model_id.replace('/', "--");
    let model_dir = models_dir.join(&local_dir);

    // v0.9.0 修复：ready 判断需校验权重文件（config.json + safetensors/pytorch_model.bin），
    // 不能只看 config.json，否则只有配置文件没有权重也会误判为 ready
    let status = if crate::engine::model_resolver::check_model_ready(&default_model_id) {
        "ready"
    } else if model_dir.exists() {
        "not_downloaded"
    } else {
        "unknown"
    };

    let resp = EmbedderStatusResponse {
        model_id: default_model_id,
        status: status.to_string(),
        models_dir: models_dir.to_string_lossy().to_string(),
        available_models: AVAILABLE_EMBEDDER_MODELS
            .iter()
            .map(|s| s.to_string())
            .collect(),
    };

    (
        StatusCode::OK,
        Json(serde_json::to_value(&resp).unwrap_or_else(|_| serde_json::json!({}))),
    )
}

/// POST /api/embedder/download — 启动嵌入模型下载
///
/// 由于实际下载是耗时操作，这里立即返回任务已启动，
/// 真正的下载在后台线程中执行，状态通过全局 AtomicBool 跟踪。
async fn embedder_download_handler(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<EmbedderDownloadRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let model_id = body.model_id.trim().to_string();
    if model_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "success": false,
                "message": "缺少 model_id 字段"
            })),
        );
    }

    // 校验 model_id 是否在白名单内（防止任意输入）
    if !AVAILABLE_EMBEDDER_MODELS.contains(&model_id.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "success": false,
                "message": format!("不支持的 model_id: {}，可选: {:?}", model_id, AVAILABLE_EMBEDDER_MODELS)
            })),
        );
    }

    // v0.6.0 安全加固：防御纵深——显式拒绝路径遍历字符
    // 即使白名单已阻止，也防止未来白名单变更时引入漏洞
    if model_id.contains("..") || model_id.contains('\\') || model_id.contains('\0') {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "success": false,
                "message": "model_id 包含非法字符"
            })),
        );
    }

    // 解析镜像源（不区分大小写，默认 hf-mirror）
    let mirror_str = body.mirror.as_deref().unwrap_or("hf-mirror").to_lowercase();
    let mirror = match mirror_str.as_str() {
        "modelscope" => crate::engine::model_downloader::MirrorSource::ModelScope,
        "auto" => crate::engine::model_downloader::MirrorSource::Auto,
        _ => crate::engine::model_downloader::MirrorSource::HfMirror,
    };

    // 抢占式设置下载标志：若已有任务在运行则拒绝
    if EMBEDDER_DOWNLOADING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "success": false,
                "message": "已有下载任务在运行中，请稍后通过状态接口查看进度"
            })),
        );
    }

    // 后台线程执行下载（fire-and-forget：JoinHandle 有意丢弃，生命周期由
    // EMBEDDER_DOWNLOADING 原子标志管理，前端轮询 /api/embedder/status）。
    // r4 修复：用 catch_unwind 包裹线程主体——若下载逻辑 panic，也能重置
    // EMBEDDER_DOWNLOADING 并记录日志，避免状态永久卡在"下载中"导致后续
    // 下载永远返回 409 CONFLICT。
    let model_id_clone = model_id.clone();
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            use crate::engine::model_downloader::{
                build_download_url, ConsoleProgress, ModelDownloader,
            };

            let downloader = ModelDownloader::with_defaults();
            let progress = ConsoleProgress::new();

            // 模型所需核心文件（按依赖顺序）
            // v0.9.0 修复：权重文件支持 safetensors / pytorch_model.bin 双格式 fallback。
            // bge-small-zh 等部分模型只有 pytorch_model.bin（无 model.safetensors），
            // 若只下载 model.safetensors 必然失败，导致模型不完整、始终降级。
            let config_files = ["config.json", "tokenizer.json"];
            let local_dir = model_id_clone.replace('/', "--");
            // v0.9.0 修复：下载到统一模型目录 ~/.loong-recall/models/
            let base_dir = crate::engine::model_resolver::default_models_dir().join(&local_dir);
            if let Err(e) = std::fs::create_dir_all(&base_dir) {
                eprintln!("[LRC·嵌入] 创建模型目录失败 {}: {}", base_dir.display(), e);
                return;
            }

            // 1. 下载必需的配置文件（config.json + tokenizer.json）
            for file in &config_files {
                let url = build_download_url(&model_id_clone, file, mirror);
                let dest = base_dir.join(file);
                eprintln!("[LRC·嵌入] 下载 {}: {}", file, url);
                if let Err(e) = downloader.download_with_retry(&url, &dest, &progress) {
                    eprintln!("[LRC·嵌入] 下载 {} 失败: {}", file, e);
                    return;
                }
            }

            // 2. 下载权重文件：safetensors 优先，失败则 fallback 到 pytorch_model.bin
            let weights_ok = {
                let url = build_download_url(&model_id_clone, "model.safetensors", mirror);
                let dest = base_dir.join("model.safetensors");
                eprintln!("[LRC·嵌入] 下载 model.safetensors: {}", url);
                match downloader.download_with_retry(&url, &dest, &progress) {
                    Ok(()) => true,
                    Err(e) => {
                        eprintln!(
                            "[LRC·嵌入] model.safetensors 下载失败: {}，尝试 pytorch_model.bin",
                            e
                        );
                        let alt_url =
                            build_download_url(&model_id_clone, "pytorch_model.bin", mirror);
                        let alt_dest = base_dir.join("pytorch_model.bin");
                        eprintln!("[LRC·嵌入] 下载 pytorch_model.bin: {}", alt_url);
                        match downloader.download_with_retry(&alt_url, &alt_dest, &progress) {
                            Ok(()) => true,
                            Err(e2) => {
                                eprintln!("[LRC·嵌入] pytorch_model.bin 下载也失败: {}", e2);
                                false
                            }
                        }
                    }
                }
            };

            if !weights_ok {
                eprintln!("[LRC·嵌入] 模型 {} 权重文件下载失败", model_id_clone);
                return;
            }

            eprintln!("[LRC·嵌入] 模型 {} 下载完成", model_id_clone);
        }));

        // 无论成功/失败/panic，最终都重置下载标志，保证状态机可恢复
        EMBEDDER_DOWNLOADING.store(false, Ordering::SeqCst);
        if let Err(panic_payload) = result {
            let msg = panic_payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| panic_payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "未知 panic 载荷".to_string());
            eprintln!("[LRC·嵌入] 下载线程 panic: {}", msg);
        }
    });

    let resp = EmbedderDownloadResponse {
        success: true,
        message: "下载任务已启动，请通过状态接口查看进度".to_string(),
        model_id,
    };

    (
        StatusCode::OK,
        Json(serde_json::to_value(&resp).unwrap_or_else(|_| serde_json::json!({"success": true}))),
    )
}

/// POST /api/embedder/apply — 将指定模型设为默认
///
/// 将模型 ID 写入 `~/.lrc/config.toml`，并提示用户也可通过环境变量
/// `LRC_LUOSHU_MODEL_ID` 覆盖（重启后生效）。
async fn embedder_apply_handler(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<EmbedderApplyRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let model_id = body.model_id.trim().to_string();
    if model_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "success": false,
                "message": "缺少 model_id 字段"
            })),
        );
    }

    if !AVAILABLE_EMBEDDER_MODELS.contains(&model_id.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "success": false,
                "message": format!("不支持的 model_id: {}", model_id)
            })),
        );
    }

    // 解析用户主目录（Windows 优先 USERPROFILE，Unix 用 HOME）
    let home_dir = match std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        Ok(p) => PathBuf::from(p),
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "success": false,
                    "message": "无法获取用户主目录（USERPROFILE / HOME 均未设置）"
                })),
            );
        }
    };
    let lrc_dir = home_dir.join(".lrc");
    let config_path = lrc_dir.join("config.toml");

    // 写入 TOML 格式配置（简单键值）
    // v0.6.0 P1-G 修复：对 model_id 进行 TOML 字符串转义
    // 虽然 model_id 已通过白名单校验，但防御性地转义特殊字符避免配置文件注入
    let toml_escaped_model_id = model_id
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    let toml_content = format!(
        "# LRC 嵌入模型配置（由仪表盘生成）\nmodel_id = \"{}\"\n",
        toml_escaped_model_id
    );

    // v0.7.1 P2-1 修复：用 spawn_blocking 包裹同步文件 I/O，避免阻塞 Tokio worker 线程
    let write_result = tokio::task::spawn_blocking(move || {
        // 创建配置目录（如不存在）
        if let Err(e) = std::fs::create_dir_all(&lrc_dir) {
            return Err(format!("创建配置目录失败: {}", e));
        }
        if let Err(e) = std::fs::write(&config_path, toml_content) {
            return Err(format!("写入配置文件失败: {}", e));
        }
        Ok(())
    })
    .await;

    match write_result {
        Ok(Err(e)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "success": false,
                    "message": e
                })),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "success": false,
                    "message": format!("异步写入任务失败: {}", e)
                })),
            );
        }
        Ok(Ok(_)) => {}
    }

    let resp = EmbedderApplyResponse {
        success: true,
        message: format!(
            "模型已设为默认，重启后生效。也可设置环境变量 {}={} 覆盖",
            crate::engine::embedder::EMBEDDER_MODEL_ENV_VAR,
            model_id
        ),
        model_id,
    };

    (
        StatusCode::OK,
        Json(serde_json::to_value(&resp).unwrap_or_else(|_| serde_json::json!({"success": true}))),
    )
}

/// POST /api/embedder/test — 测试镜像源连通性
///
/// 使用 ureq 发送 HEAD 请求，测量响应延迟（毫秒）。
async fn embedder_test_handler(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<EmbedderTestRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let model_id = body.model_id.trim().to_string();
    if model_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "success": false,
                "message": "缺少 model_id 字段"
            })),
        );
    }

    let mirror_str = body.mirror.as_deref().unwrap_or("hf-mirror").to_lowercase();
    let mirror = match mirror_str.as_str() {
        "modelscope" => crate::engine::model_downloader::MirrorSource::ModelScope,
        _ => crate::engine::model_downloader::MirrorSource::HfMirror,
    };

    // 测试 URL：取 config.json（体积小，能反映连通性）。
    // 使用 GET 而不是 HEAD，部分镜像对 HEAD 返回 404，但 GET 下载正常。
    let test_url =
        crate::engine::model_downloader::build_download_url(&model_id, "config.json", mirror);

    let start = std::time::Instant::now();
    // v0.9.6 修复（六钥匙·反向推导）：超时预算必须"后端 < 前端"。
    // 前端 fetchWithTimeout 预算为 10s，此前后端总超时 15s，镜像不可达时
    // 后端 21s 才返回，前端已提前抛 SidecarTimeoutError，用户看到误导性文案
    // （"请检查 LRC 服务是否正常运行"），而真实原因是外网镜像不可达。
    // 连接超时 4s + 总超时 8s，确保镜像不可达时后端先于前端返回真实失败原因。
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(4))
        .timeout(std::time::Duration::from_secs(8))
        .build();

    let resp = match agent.get(&test_url).call() {
        Ok(_) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            EmbedderTestResponse {
                success: true,
                mirror: mirror_str,
                latency_ms: Some(latency_ms),
                model_id,
                message: None,
            }
        }
        Err(e) => EmbedderTestResponse {
            success: false,
            mirror: mirror_str,
            latency_ms: None,
            model_id,
            message: Some(format!("连通性测试失败: {}", e)),
        },
    };

    (
        StatusCode::OK,
        Json(serde_json::to_value(&resp).unwrap_or_else(|_| serde_json::json!({"success": false}))),
    )
}

/// GET /api/tools/detect — 浏览器开发模式的快捷方式候选兼容接口。
///
/// 桌面端正式检测统一走 Tauri `discover_all_agents`。浏览器模式不再维护第二套
/// 工具数据库，也不使用 PATH、安装目录、CLI 或扩展推断 AI 工具；只返回快捷方式原名，
/// 并将其标记为未确认候选，避免把普通快捷方式误报为 AI 工具。
async fn tools_detect_handler(
    State(_state): State<Arc<AppState>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let tools = scan_desktop_shortcuts()
        .into_iter()
        .map(|name| ToolDetectItem {
            name,
            tool_type: "shortcut-candidate".to_string(),
            installed: false,
            version: None,
            path: None,
            shortcut_confirmed: false,
        })
        .collect::<Vec<_>>();

    let resp = ToolsDetectResponse { tools };
    (
        StatusCode::OK,
        Json(serde_json::to_value(&resp).unwrap_or_else(|_| serde_json::json!({"tools": []}))),
    )
}

/// 扫描桌面和开始菜单快捷方式，返回原始快捷方式名称。
///
/// 这里只收集事实，不判断它是否属于 AI 工具；判断和确认交给桌面端注册表
/// 与用户主动选择，避免维护第二套硬编码工具名单。
fn scan_desktop_shortcuts() -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    let mut dirs = Vec::new();

    if let Ok(user_profile) = std::env::var("USERPROFILE") {
        let home = PathBuf::from(user_profile);
        dirs.push(home.join("Desktop"));
        dirs.push(home.join("AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs"));
    }
    if let Ok(program_data) = std::env::var("PROGRAMDATA") {
        dirs.push(PathBuf::from(program_data).join("Microsoft\\Windows\\Start Menu\\Programs"));
    }

    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"))
            {
                if let Some(name) = path.file_stem().and_then(|value| value.to_str()) {
                    let name = name.trim();
                    if !name.is_empty()
                        && !result.iter().any(|item| item.eq_ignore_ascii_case(name))
                    {
                        result.push(name.to_string());
                    }
                }
            }
        }
    }

    result.sort_by_key(|name| name.to_lowercase());
    result
}

// ==================== Stdio 传输层（标准 MCP） ====================

/// MCP 请求分发结果
///
/// `None` 表示通知类请求，不需要返回响应。
type DispatchResult = Option<JsonRpcResponse>;

/// MCP stdio 请求分发器
///
/// 将 HTTP 和 stdio 共用的路由逻辑抽离为独立函数。
/// 通知类请求（`notifications/*`）返回 None，不发送响应。
async fn dispatch_request(
    state: &AppState,
    method: &str,
    params: Option<&serde_json::Value>,
    id: Option<serde_json::Value>,
) -> DispatchResult {
    match method {
        "initialize" => Some(handle_initialize(id)),
        "tools/list" => Some(handle_tools_list(id)),
        "tools/call" => {
            let params = match params {
                Some(p) => p,
                None => return Some(make_error(id, -32602, "缺少 params")),
            };
            Some(handle_tools_call(state, params, id).await)
        }
        // 通知类请求：MCP 协议规定通知不需要响应
        method if method.starts_with("notifications/") => None,
        _ => Some(make_error(id, -32601, &format!("未知方法: {}", method))),
    }
}

/// 启动 MCP stdio 模式（供 IDE 通过 stdin/stdout 通信）
///
/// 运行逻辑：
/// 1. 从 stdin 逐行读取 JSON-RPC 请求
/// 2. 调用统一的 dispatch 逻辑
/// 3. 将 JSON-RPC 响应写入 stdout
///
/// 这是标准的 MCP 通信方式，兼容所有支持 MCP 的 IDE。
pub async fn run_stdio(state: Arc<AppState>) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    while let Ok(Some(line)) = lines.next_line().await {
        // 跳过空行（管道通信可能产生空行）
        if line.trim().is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(req) => req,
            Err(e) => {
                let err_resp = make_error(None, -32700, &format!("JSON 解析失败: {}", e));
                let json_str = serde_json::to_string(&err_resp).unwrap_or_else(|_| {
                    r#"{"jsonrpc":"2.0","error":{"code":-32700,"message":"内部序列化错误"}}"#
                        .to_string()
                });
                let _ = stdout.write_all(format!("{}\n", json_str).as_bytes()).await;
                continue;
            }
        };

        let response =
            dispatch_request(&state, &request.method, request.params.as_ref(), request.id).await;

        // 通知类请求不返回响应，直接跳过
        if let Some(response) = response {
            let json_str = serde_json::to_string(&response).unwrap_or_else(|_| {
                r#"{"jsonrpc":"2.0","error":{"code":-32603,"message":"内部序列化错误"}}"#
                    .to_string()
            });
            let _ = stdout.write_all(format!("{}\n", json_str).as_bytes()).await;
            let _ = stdout.flush().await;
        }
    }

    eprintln!("Stdio 流已关闭，MCP 服务退出");
}

// ==================== 路由构建 ====================

/// 本地 API 认证中间件（安全加固：/v1、/mcp 等端点默认未认证）。
///
/// 兼容设计：仅当环境变量 LRC_API_TOKEN 非空且非空字符串时启用
/// Bearer Token 校验；未配置 token 时跳过认证，保持既有本地开发、
/// 仪表盘与桥接客户端行为完全不变。认证失败返回 401 Unauthorized。
///
/// # 认证默认策略（v0.9.7 明确化，GLOBAL_CODE_REVIEW_REPORT P1 安全「认证默认策略」）
///
/// **默认无认证**这一行为是**有意设计**，其安全边界如下，部署者须自行评估：
///
/// 1. **威胁模型假设**：进程绑定 `127.0.0.1`（回环），仅本机可访问；同机其它进程
///    与恶意软件本就可读取用户目录下的记忆数据，故"同机无认证"不额外扩大暴露面。
/// 2. **风险场景**：若将服务**绑定到非回环地址**（`--host 0.0.0.0`）却未设置
///    `LRC_API_TOKEN`，则**局域网内任意主机可读写全部记忆**（含明文与归档）。
///    此场景下必须显式设置 token。
/// 3. **强制保护手段**：设置 `LRC_API_TOKEN=<强随机串>` 即启用 Bearer 校验；
///    客户端须携带 `Authorization: Bearer <token>`。
/// 4. **比较方式**：token 比对使用 [`constant_time_eq`]（常量时间），
///    防止通过响应耗时逐字节推断 token。
/// 5. **未做之事**：不提供速率限制以外的暴力破解防护、不做 token 轮换。
///    面向不可信网络的部署应在反向代理层叠加 TLS 与访问控制。
async fn local_api_auth(
    req: Request<axum::body::Body>,
    next: Next,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    match std::env::var("LRC_API_TOKEN") {
        // 未配置 token：跳过认证（兼容本地开发模式）
        Err(_) => Ok(next.run(req).await),
        // token 为空字符串：同样视为未配置
        Ok(token) if token.trim().is_empty() => Ok(next.run(req).await),
        Ok(expected) => {
            let authorized = req
                .headers()
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
                .map(|token| constant_time_eq(token.trim().as_bytes(), expected.trim().as_bytes()))
                .unwrap_or(false);
            if authorized {
                Ok(next.run(req).await)
            } else {
                Err((
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({
                        "error": "unauthorized",
                        "message": "缺少或无效的 API Token，请携带 Authorization: Bearer <LRC_API_TOKEN>"
                    })),
                ))
            }
        }
    }
}

/// 非回环绑定且未设 Token 时的启动告警
///
/// v0.9.7 新增（GLOBAL_CODE_REVIEW_REPORT P1 安全「认证默认策略」）：
///   该函数把"默认无认证仅在回环下安全"这一文档约定，转为**启动期的可见告警**。
///   判定逻辑刻意保守（宁可误报不可漏报）：
///     - `host` 为 `127.0.0.1` / `::1` / `localhost` → 视为回环，不告警；
///     - 其余（含 `0.0.0.0`、`::`、具体内网/公网 IP、主机名）→ 视为对外可达，
///       若此时 `LRC_API_TOKEN` 未设置或为空，则打印多行醒目告警。
///   仅告警、不阻断启动：强制手段是在环境变量中设置 `LRC_API_TOKEN`。
fn warn_if_unauthenticated_non_loopback(host: &str) {
    let is_loopback = matches!(
        host.trim().trim_start_matches('[').trim_end_matches(']'),
        "127.0.0.1" | "::1" | "localhost"
    );
    if is_loopback {
        return;
    }
    let token_missing = std::env::var("LRC_API_TOKEN")
        .map(|t| t.trim().is_empty())
        .unwrap_or(true);
    if !token_missing {
        return;
    }
    eprintln!("================================================================");
    eprintln!("[安全告警] 服务绑定到非回环地址 '{host}'，且未设置 LRC_API_TOKEN。");
    eprintln!("          当前状态下，能访问该地址的任意主机均可读写全部记忆数据。");
    eprintln!("          若仅本机使用，请改绑 127.0.0.1；");
    eprintln!("          若确需对外提供服务，请设置 LRC_API_TOKEN=<强随机串> 后再启动。");
    eprintln!("================================================================");
}

/// 常量时间字节串比较（防时序侧信道）
/// v0.9.7 修复（GLOBAL_CODE_REVIEW_REPORT P2 安全「Token 非常量时间比较 @server.rs:3705」）：
///   根因：原实现用 `token == expected`（`str` 的 `PartialEq`）——比较在首个不同字节处提前返回，
///         攻击者可通过统计响应耗时逐字节推断 Token。
///   实现要点：
///     - 长度不同立即返回 false（长度差异本身不是秘密，Token 长度固定）；
///     - 逐字节累积 XOR 差异，**不使用短路**，使耗时与"首个不同字节的位置"无关；
///     - `black_box` 防止优化器把循环改回短路比较。
///   注：不引入 `subtle` crate，避免为单点需求新增供应链依赖。
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    std::hint::black_box(diff) == 0
}

/// 创建 MCP 服务的 axum Router（合并 v1 REST API 端点 + 仪表盘）
///
/// 可嵌入到已有 axum 应用中，将 MCP 路由挂载到子路径。
pub fn build_mcp_router(state: Arc<AppState>) -> Router {
    // 创建 v1 API 路由（通过闭包捕获共享状态，状态类型为 ()）
    let v1_service = crate::v1_api::build_v1_router(
        state.memory_store.clone(),
        state.manager.clone(),
        state.llm_api.clone(),
        state.llm_configured_atomic.clone(),
        state.dev_mode,
    )
    .into_service();

    // 受保护路由：/v1 API、/mcp 与仪表盘数据 API 需要 Bearer Token 认证。
    // 注意：使用 route_layer 而非 layer——layer 会改变 Request body 泛型，
    // 导致后续 merge 出现类型不匹配；route_layer 保持泛型不变，可安全合并。
    let protected_routes = Router::new()
        .route("/mcp", post(mcp_handler))
        .nest_service("/v1", v1_service) // 将 v1 API 嵌套在 /v1 路径下
        // 仪表盘路由：静态文件 + 重定向
        .route("/dashboard", get(dashboard_handler))
        .route("/dashboard/", get(dashboard_handler))
        // 根路径重定向到仪表盘（方便桌面端直接加载）
        .route("/", get(root_redirect_handler))
        // 配置 API：仪表盘设置页面用
        // v0.8.1：以下路由保留向后兼容，新代码应使用 /v1/config 和 /v1/config/llm
        .route("/api/config", get(config_handler)) // deprecated, use /v1/config
        .route("/api/config/llm", post(config_llm_handler)) // deprecated, use /v1/config/llm
        // V2: 项目信息 API
        .route("/api/project/info", get(project_info_handler))
        // V2: 项目列表 API（批量查询所有项目的元信息，供前端构建"指纹→名称"映射表）
        .route("/api/projects/list", get(projects_list_handler))
        // v0.6.0+：嵌入模型管理 API（仪表盘模型设置页用）
        .route("/api/embedder/status", get(embedder_status_handler))
        .route("/api/embedder/download", post(embedder_download_handler))
        .route("/api/embedder/apply", post(embedder_apply_handler))
        .route("/api/embedder/test", post(embedder_test_handler))
        // v0.6.0+：IDE / Agent 工具检测
        .route("/api/tools/detect", get(tools_detect_handler))
        .route_layer(middleware::from_fn(local_api_auth));

    // 公开路由：健康检查与静态资源（不涉及敏感数据，无需认证）
    let public_routes = Router::new()
        .route("/health", get(health_handler))
        .route("/app.js", get(app_js_handler))
        .route("/app.css", get(app_css_handler))
        // v0.6.0 龙忆设计系统：设计系统 CSS 资源
        .route("/colors_and_type.css", get(colors_and_type_css_handler))
        .route("/components.css", get(components_css_handler))
        // v0.6.0 龙忆设计系统：Logo 与图标 SVG 资源
        .route("/assets/logo/{filename}", get(logo_asset_handler))
        .route("/assets/icons/{filename}", get(icon_asset_handler));

    // 合并公开与受保护路由，再应用跨域/超时/并发限制
    Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        // v0.6.0 安全加固：CORS 从 permissive 收紧为显式白名单
        // 允许本地开发服务器和桌面端访问，拒绝任意来源
        .layer(
            tower_http::cors::CorsLayer::new()
                .allow_origin(tower_http::cors::AllowOrigin::predicate(|origin, _| {
                    // 允许的来源：localhost 任意端口、127.0.0.1、tauri 协议
                    // v0.6.0 P0 修复：Tauri 2.x Windows 使用 https://tauri.localhost 作为 WebView 源
                    // v0.6.0 P1-2 修复：Tauri 2.x 默认 Windows/Android 使用 http://tauri.localhost
                    if let Ok(s) = origin.to_str() {
                        // v0.7.1 P2-2 修复：移除 http://0.0.0.0: 白名单
                        // 0.0.0.0 不是真实客户端地址，允许其作为 Origin 存在安全风险
                        // 仅允许 localhost、127.0.0.1 和 tauri 协议
                        s.starts_with("http://localhost:")
                            || s.starts_with("http://127.0.0.1:")
                            || s.starts_with("https://localhost:")
                            || s.starts_with("tauri://")
                            || s == "https://tauri.localhost"
                            || s.starts_with("https://tauri.localhost")
                            || s == "http://tauri.localhost"
                            || s.starts_with("http://tauri.localhost")
                    } else {
                        false
                    }
                }))
                .allow_methods([
                    axum::http::Method::GET,
                    axum::http::Method::POST,
                    axum::http::Method::OPTIONS,
                ])
                .allow_headers([
                    axum::http::header::CONTENT_TYPE,
                    axum::http::header::AUTHORIZATION,
                ])
                .allow_credentials(false),
        )
        // v0.8.22 P1-1 修复（hcse-resilience-validator Round3 FM-02）：
        //   根因：handler 阻塞时 TCP 连接不关闭，CLOSE_WAIT 累积 27-49 个（阈值 <10）
        //   修复1：TimeoutLayer 30s — 单请求超时后自动关闭连接，防止 CLOSE_WAIT 堆积
        //   修复2：ConcurrencyLimitLayer 100 — 限制最大并发连接数，防止 worker 耗尽
        //   注意：30s 超时足够 lock_busy 路径返回降级数据（<1ms），只拦截真正卡死的请求
        .layer(tower_http::timeout::TimeoutLayer::with_status_code(
            axum::http::StatusCode::GATEWAY_TIMEOUT,
            std::time::Duration::from_secs(30),
        ))
        .layer(tower::limit::ConcurrencyLimitLayer::new(100))
        .with_state(state)
}

/// 快速构建并绑定到指定地址
pub async fn serve(state: Arc<AppState>, host: &str, port: u16) -> std::io::Result<()> {
    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    serve_on_listener(state, host, port, listener).await
}

/// 在已绑定的 TcpListener 上启动服务（供进程守护模块使用）
///
/// 与 serve() 的区别：接受外部预先绑定的 listener，
/// 以便在绑定之前执行端口自适应逻辑。
pub async fn serve_on_listener(
    state: Arc<AppState>,
    host: &str,
    port: u16,
    listener: tokio::net::TcpListener,
) -> std::io::Result<()> {
    let app = build_mcp_router(state.clone());

    let addr = format!("{}:{}", host, port);
    println!("Loong Recall (L-RC) 代码搜索 + 记忆服务");
    println!("   端点: http://{}", addr);
    println!("   仪表盘: http://{}/dashboard  ← 可视化记忆管理面板", addr);
    println!("   船长日志: GET  http://{}/v1/captains-log", addr);
    println!("   MCP 协议: POST http://{}/mcp", addr);
    println!("   状态检查: GET  http://{}/health", addr);

    // v0.9.7 修复（GLOBAL_CODE_REVIEW_REPORT P1 安全「认证默认策略」——强制保护）：
    //   默认无认证仅在"绑定回环"时安全（见 local_api_auth 文档）。一旦绑定到非回环
    //   地址（0.0.0.0 / :: / 局域网 IP）且未设置 LRC_API_TOKEN，则局域网内任意主机
    //   可读写全部记忆。此处把该风险从"文档约定"升级为"启动即告警"，越明显越好。
    //   注：告警而非拒绝启动——避免破坏既有内网自用部署；强制手段仍是设 token。
    warn_if_unauthenticated_non_loopback(host);

    // v0.8.1：连接池与超时优化（修复 Bug #7：sidecar API 间歇性超时）
    //
    // axum 0.8 的 Serve 移除了 tcp_nodelay/tcp_keepalive/http2_keep_alive 方法（axum 0.7 API），
    // 改用 ListenerExt::tap_io 对每个接入连接设置 TCP 选项：
    // 1. TCP_NODELAY：禁用 Nagle 算法，降低小请求延迟
    // 2. SO_KEEPALIVE：60 秒无数据后发送保活探测，自动回收泄漏连接
    let listener = listener.tap_io(|stream| {
        if let Err(e) = stream.set_nodelay(true) {
            eprintln!("[sidecar] 设置 TCP_NODELAY 失败: {e}");
        }
        let socket = socket2::SockRef::from(&*stream);
        let keepalive = socket2::TcpKeepalive::new().with_time(std::time::Duration::from_secs(60));
        if let Err(e) = socket.set_tcp_keepalive(&keepalive) {
            eprintln!("[sidecar] 设置 TCP keepalive 失败: {e}");
        }
    });

    // P1 调节器心跳：点亮 DaoRegulator（自适应调节的活性保证）。
    // 后台周期任务定期调用 MemoryStore::regulate()，并记录调节器心跳状态，
    // 供 /v1/health/system 与前端系统状态卡展示。无异常路径会改变检索行为。
    tokio::spawn(regulator_heartbeat_loop(state.clone()));

    axum::serve(listener, app).await
}

/// P1 调节器心跳后台循环。
///
/// 周期由环境变量 `LRC_REGULATE_INTERVAL_MIN` 控制（默认 30 分钟，便于测试缩短）。
/// 每次 tick：
///   1. 尝试 `try_lock` 获取 MemoryStore（锁忙时跳过本轮，不阻塞 HTTP worker）；
///   2. 在 `spawn_blocking` 中调用 `regulate()`（同步且可能涉及 IO，避免占用 async worker）；
///   3. `regulate()` 内部已记录心跳（RegulatorHeartbeat）与审计事件，此处仅输出日志。
async fn regulator_heartbeat_loop(state: Arc<AppState>) {
    let interval_min = std::env::var("LRC_REGULATE_INTERVAL_MIN")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(30)
        .max(1);
    eprintln!(
        "[LRC·心跳] 调节器心跳已上线，周期 {} 分钟（LRC_REGULATE_INTERVAL_MIN 可调）",
        interval_min
    );
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_min * 60));
    loop {
        ticker.tick().await;
        let store = state.memory_store.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let mut guard = match store.try_lock() {
                Ok(g) => g,
                Err(_) => return "locked", // 其他线程持锁（如后台合成），本轮跳过
            };
            match guard.regulate() {
                Some(_) => "regulated",
                None => "no_action",
            }
        })
        .await
        .unwrap_or("panic");
        // 心跳状态与审计已在 regulate() 内记录，此处仅输出调试日志
        if outcome != "no_action" {
            eprintln!("[LRC·心跳] 调节器本轮结果: {outcome}");
        }
    }
}

// ==================== MCP 协议单元测试 ====================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CodeMemoryManager;

    /// P2.5-1：fetch_daoti_navigation 在 daemon 在线时解析 NavigationSignal。
    /// 用本地 mock HTTP 服务模拟 daemon 的 /deduce 响应（不依赖外部进程）。
    #[tokio::test]
    async fn test_fetch_daoti_navigation_online() {
        // 起一个仅监听回环的 mock daemon
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mock = tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                // 仅响应 POST /deduce
                if req.starts_with("POST /deduce") {
                    let body = "{\"ok\":true,\"palaces\":[\"艮宫\",\"震宫\"],\"probes\":[[\"艮\",\"艮\"],[\"震\",\"震\"]],\"version\":\"daoti-lexicon-v1\"}";
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                } else {
                    let _ = sock
                        .write_all(b"HTTP/1.1 404\r\nContent-Length: 0\r\n\r\n")
                        .await;
                }
            }
        });

        // 通过注入 base_url 指向 mock（不依赖全局环境变量，避免测试并行竞态）
        let sig = fetch_daoti_navigation_with_base(
            "今晚吃什么",
            "lrc-recall",
            Some(&format!("http://{}", addr)),
        )
        .await;
        mock.abort();

        let sig = sig.expect("daemon 在线时应返回导航信号");
        assert_eq!(sig.palaces, vec!["艮宫".to_string(), "震宫".to_string()]);
        assert_eq!(sig.source_version.as_deref(), Some("daoti-lexicon-v1"));
    }

    /// P2.5-2：daemon 不可达时降级返回 None（行为与无导航基线一致）。
    #[tokio::test]
    async fn test_fetch_daoti_navigation_offline_degrades() {
        // 使用一个必然无人监听的本地端口（回环 + 端口 0 已释放）
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener); // 立刻释放 → 连接被拒绝 → 超时降级
        let sig = fetch_daoti_navigation_with_base(
            "今晚吃什么",
            "lrc-recall",
            Some(&format!("http://{}", addr)),
        )
        .await;
        assert!(sig.is_none(), "daemon 不可达时应返回 None（降级基线）");
    }

    /// P6/CL2-1：post_daoti_reflect 在 daemon 在线时回传结果摘要并解析 applied。
    /// 同时校验请求体携带 memories 与 session_id（闭环契约：deduce/reflect 同会话）。
    #[tokio::test]
    async fn test_post_daoti_reflect_online() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mock = tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                // 仅响应 POST /reflect，并校验闭环请求体契约
                if req.starts_with("POST /reflect") {
                    assert!(
                        req.contains("\"memories\""),
                        "reflect 请求必须携带 memories 结果摘要"
                    );
                    assert!(
                        req.contains("\"session_id\":\"lrc-explore\""),
                        "reflect 请求必须携带 session_id（与 deduce 同会话）"
                    );
                    let body =
                        "{\"applied\":true,\"reflection\":{\"reflected_gua\":\"兑\",\"palace\":\"兑宫\"}}";
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                } else {
                    let _ = sock
                        .write_all(b"HTTP/1.1 404\r\nContent-Length: 0\r\n\r\n")
                        .await;
                }
            }
        });

        let memories = vec![
            "火锅店在老街尽头".to_string(),
            "上周聚餐去了那家川菜馆".to_string(),
        ];
        let applied = post_daoti_reflect_with_base(
            &memories,
            "lrc-explore",
            Some(&format!("http://{}", addr)),
        )
        .await;
        mock.abort();
        assert!(applied, "daemon 在线且结果非空时应返回 applied=true");
    }

    /// P6/CL2-2：reflect 空结果不发请求；daemon 不可达时静默降级返回 false
    /// （闭环任何环节失败 → 调用方行为不变）。
    #[tokio::test]
    async fn test_post_daoti_reflect_empty_and_offline_degrade() {
        // 空结果：直接 false，不发起任何请求
        assert!(
            !post_daoti_reflect_with_base(&[], "lrc-explore", Some("http://127.0.0.1:1")).await
        );
        // 不可达端口：连接拒绝 → 静默降级 false
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let memories = vec!["任意内容".to_string()];
        assert!(
            !post_daoti_reflect_with_base(
                &memories,
                "lrc-explore",
                Some(&format!("http://{}", addr))
            )
            .await,
            "daemon 不可达时应静默降级返回 false"
        );
    }

    /// v0.9.8：/cycle 在线时应解析出候选与**目标层次**，
    /// 且请求体必须携带 text 与 target（target = 查询本身，不解析）。
    #[tokio::test]
    async fn test_fetch_daoti_cycle_online_parses_hops() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        // ★★ 断言必须**回传测试线程**（2026-09-18 修 S8）★★
        //
        // 此前契约断言写在 `tokio::spawn` 的 task 内，而 `JoinHandle` 从未被
        // await（末尾 `mock.abort()` 直接把它丢弃）⇒ **task 内 panic 不会传播
        // 到测试线程**，测试照常通过。后果：本轮最核心的契约断言
        // （target = 查询本身，用户裁定的关键设计）**实际从未生效**——
        // 即使把 body 改成不传 target，这条测试依然是绿的。
        //
        // ⇒ 改为「mock 只记录收到的请求，断言在主线程做」：
        //   这样断言失败会真的让测试红。
        let seen: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_mock = seen.clone();
        let mock = tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                if req.starts_with("POST /cycle") {
                    // 只记录，不 assert（assert 在主线程做）
                    if let Ok(mut g) = seen_mock.lock() {
                        g.push(req.clone());
                    }
                    let body = "{\"associations\":[{\"gua_name\":\"兑为泽\",\"rel_type\":\"COORDINATE\",\"how\":\"zong_gua\",\"target_hops\":1,\"wuxing_relation\":\"生\",\"l3_source\":\"rust_cli\"}],\"target\":\"今晚吃什么\",\"target_gua\":\"兑为泽\",\"degraded\":[\"scheduler.curiosity\"],\"version\":\"daoti-assoc-v1\"}";
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                } else {
                    let _ = sock
                        .write_all(b"HTTP/1.1 404\r\nContent-Length: 0\r\n\r\n")
                        .await;
                }
            }
        });

        let cyc = fetch_daoti_cycle_with_base(
            "今晚吃什么",
            "今晚吃什么",
            Some(&format!("http://{}", addr)),
        )
        .await;
        mock.abort();

        // ---- ★契约断言（在主线程，失败真的会红）----
        let reqs = seen.lock().map(|g| g.clone()).unwrap_or_default();
        assert!(
            !reqs.is_empty(),
            "mock 应至少收到一次 /cycle 请求（否则断言形同虚设）"
        );
        let req = &reqs[0];
        assert!(
            req.contains("\"text\":\"今晚吃什么\""),
            "cycle 请求必须携带 text，实际: {}",
            req
        );
        assert!(
            req.contains("\"target\":\"今晚吃什么\""),
            "cycle 请求必须携带 target（= 查询本身，不解析），实际: {}",
            req
        );

        let cyc = cyc.expect("道体在线且契约匹配时应返回循环结果");
        let assoc = cyc["associations"]
            .as_array()
            .expect("associations 应为数组");
        assert_eq!(assoc.len(), 1);
        assert_eq!(assoc[0]["target_hops"].as_u64(), Some(1));
        // 渲染层必须把层次与目标都呈现出来（用户判据：要能看见"第几层"）
        let mut text = String::new();
        append_daoti_cycle(&mut text, &cyc);
        assert!(text.contains("第 1 层"), "渲染必须显示层次: {}", text);
        assert!(text.contains("目标卦: 兑为泽"), "渲染必须显示目标卦");
        assert!(
            text.contains("相综/互卦（协同）"),
            "符号层关系标签须独立于记录层"
        );
        // 降态项必须透出（不能把默认值当实测读数）
        assert!(text.contains("降态项"), "降态项必须显示");
    }

    /// v0.9.8：/cycle 的三类降态 —— 不可达 / 版本不符 / 缺 associations。
    ///
    /// ★为什么必须逐条测：这三种都会让 `fetch_daoti_cycle` 返回 None，
    /// 但原因不同。若不区分，服务端升级改字段时会**静默变成"无联想"**，
    /// 排查时看不出是哪一类（承"不做字段猜测"纪律）。
    #[tokio::test]
    async fn test_fetch_daoti_cycle_degrades_on_unreachable_and_bad_contract() {
        // (1) 不可达端口
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        assert!(
            fetch_daoti_cycle_with_base("x", "x", Some(&format!("http://{}", addr)))
                .await
                .is_none(),
            "道体不可达必须降态为 None"
        );

        // (2) 版本不符 —— 协议不认识，必须拒绝而非按旧含义解读
        let body_v = "{\"associations\":[],\"version\":\"unknown-v9\"}";
        assert!(
            run_mock_cycle_and_fetch(body_v).await.is_none(),
            "版本不符必须降态为 None（不做字段猜测）"
        );

        // (3) 缺 associations 字段
        let body_m = "{\"version\":\"daoti-assoc-v1\"}";
        assert!(
            run_mock_cycle_and_fetch(body_m).await.is_none(),
            "缺 associations 必须降态为 None"
        );
    }

    /// 起一个只回固定 body 的 mock /cycle，返回客户端结果（供降态用例复用）。
    async fn run_mock_cycle_and_fetch(body: &'static str) -> Option<serde_json::Value> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mock = tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await.unwrap_or(0);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        let out = fetch_daoti_cycle_with_base("x", "x", Some(&format!("http://{}", addr))).await;
        mock.abort();
        out
    }

    /// v0.9.8（2026-09-18 修 G9）：**四个分区标题**必须有测试锁定。
    ///
    /// ★为什么标题值得单独测：四个分区冠名各自**声明了一种证据性质**——
    ///   · 「记录型关联（由记录推导，非语义相似）」⇒ 用户据此认为"必然成立"
    ///   · 「符号层落边（§5.4 写入端）」     ⇒ 结构算子**推导**，可能不成立
    ///   · 「符号层候选（道体 §4.4 状态机循环）」⇒ 给的是**方向**，不是记忆
    ///   · 「符号层（道体）」降态兜底          ⇒ 本次**没跑**，什么都没有
    ///
    /// 此前这些串只以字面量散落在三个 `push_str` 里，**无任何测试**：
    /// 改一个字（如把"由记录推导"删成"关联"）不会有测试变红，而这恰好
    /// 把"事实"与"推导"的区分抹掉了——正是 S4 修的那个失效模式。
    /// 本测试锁住：①四个标题各自存在 ②彼此**互不相同**（不可合并成同一顶帽子）。
    #[test]
    fn test_association_section_titles_are_locked_and_distinct() {
        // ---- ① 记录层 ----
        let mut t = String::new();
        append_associated_memories(
            &mut t,
            &[AssociatedMemory {
                memory_id: "11111111-aaaa".to_string(),
                content_preview: "吃楼外楼".to_string(),
                memory_type: "experience".to_string(),
                relation: "same_event".to_string(),
                why: "同一次经历（event_id=trip-hangzhou-2026-09）".to_string(),
                via_memory_id: "22222222-bbbb".to_string(),
                via_preview: "游西湖".to_string(),
                hops: 1,
                path: vec!["22222222-bbbb".to_string(), "11111111-aaaa".to_string()],
            }],
        );
        assert!(
            t.contains("联想 · 记录型关联（由记录推导，非语义相似）"),
            "记录层分区标题被改动（它声明了『必然成立』这一证据性质）: {}",
            t
        );

        // ---- ② 符号层落边（写入端）----
        let mut t_be = String::new();
        append_daoti_build_edges(
            &mut t_be,
            &serde_json::json!({
                "edges": [], "blocked": "any_reason", "version": "daoti-assoc-v1"
            }),
        );
        assert!(
            t_be.contains("联想 · 符号层落边（§5.4 写入端）"),
            "符号层落边分区标题被改动: {}",
            t_be
        );

        // ---- ③ 符号层候选（读端）----
        let mut t_cyc = String::new();
        append_daoti_cycle(
            &mut t_cyc,
            &serde_json::json!({
                "associations": [{"gua_name": "需", "rel_type": "COORDINATE",
                                  "how": "cuo_gua", "target_hops": 1}],
                "target": "游西湖", "target_gua": "兑为泽",
                "version": "daoti-assoc-v1"
            }),
        );
        assert!(
            t_cyc.contains("联想 · 符号层候选（道体 §4.4 状态机循环）"),
            "符号层候选分区标题被改动（它声明了『给方向而非记忆』）: {}",
            t_cyc
        );

        // ---- ④ 符号层降态（超预算）----
        // ★直接调**真实渲染函数**：若只比对字面量常量，测试无法发现
        //   "常量改了但渲染处忘了用"（渲染处会硬编码旧串，测试照样绿）。
        let mut t_deg = String::new();
        append_symbolic_layer_degraded(&mut t_deg);
        assert!(
            t_deg.contains(SYMBOLIC_LAYER_DEGRADED_TITLE),
            "降态块必须使用分区标题常量: {}",
            t_deg
        );
        assert!(
            t_deg.contains("本次跳过"),
            "降态块必须明说『本次跳过』（否则用户以为符号层没做）: {}",
            t_deg
        );

        // ---- ⑤ 四个标题互不相同 ----
        // 若有人把符号层两个分区合并成一个标题、或让记录层复用符号层标题，
        // 用户将无法分辨拿到的到底是"事实"、"推导"还是"方向"。
        let titles = [
            "联想 · 记录型关联（由记录推导，非语义相似）",
            "联想 · 符号层落边（§5.4 写入端）",
            "联想 · 符号层候选（道体 §4.4 状态机循环）",
            SYMBOLIC_LAYER_DEGRADED_TITLE,
        ];
        let uniq: std::collections::HashSet<&str> = titles.iter().copied().collect();
        assert_eq!(
            uniq.len(),
            titles.len(),
            "四个分区标题必须互不相同（证据性质不同，不可合并冠名）"
        );
        // 记录层标题必须含"由记录推导"——这是它与符号层最关键的区分词，
        // 单独断言以防有人只改这四个字（其余标题仍全绿）。
        assert!(
            titles[0].contains("由记录推导"),
            "记录层标题必须保留『由记录推导』，否则与符号层推导无法区分"
        );
    }

    /// v0.9.8：符号层渲染不得复用记录层的 `relation_label`
    /// （否则 CONSTRAINT/COORDINATE 全落到兜底「相关联」，类型信息被抹平）。
    #[test]
    fn test_structural_rel_label_is_distinct_from_record_layer() {
        assert_eq!(structural_rel_label("COORDINATE"), "相综/互卦（协同）");
        assert_eq!(structural_rel_label("CONSTRAINT"), "相错（约束）");
        // 未知类型原样回显（不伪造成已知类型）
        assert_eq!(structural_rel_label("NEW_KIND"), "NEW_KIND");
        // 与记录层标签表确实不同源
        assert_ne!(
            structural_rel_label("COORDINATE"),
            relation_label("COORDINATE")
        );
    }

    /// ★★审查发现 3：符号层落盘边**必须独立分区**，不得共用记录层的
    /// 「由记录必然关联」标题与「共同经历 / 共享实体」页脚。
    ///
    /// # 为什么必须有这条
    ///
    /// `append_associated_memories` 收到的是**混合列表**（记录层 + 符号层落盘边）。
    /// 修复前它对全部条目作记录层断言 —— 对 `coordinate`（结构推导、**可能不成立**）
    /// 是**事实错误**，用户会把推测当事实引用。
    ///
    /// 这与 UI 侧已修掉的失效模式同源（`explore_source_of`），只是漏在 MCP 文本出口。
    #[test]
    fn test_symbolic_stored_edges_get_own_section() {
        let mk = |id: &str, rel: &str, why: &str| AssociatedMemory {
            memory_id: id.to_string(),
            content_preview: format!("内容-{id}"),
            memory_type: "experience".to_string(),
            relation: rel.to_string(),
            why: why.to_string(),
            via_memory_id: "seed-1".to_string(),
            via_preview: "起点".to_string(),
            hops: 1,
            path: vec!["seed-1".to_string(), id.to_string()],
        };
        let assoc = vec![
            mk("rec-1", "same_event", "同一次经历（event_id=trip-x）"),
            mk(
                "sym-1",
                "coordinate",
                "图存储既有边（符号层推导边，相综/互卦（协同））",
            ),
        ];
        let mut text = String::new();
        append_associated_memories(&mut text, &assoc);

        // ① 两个分区标题都必须出现
        assert!(
            text.contains("联想 · 记录型关联（由记录推导，非语义相似）"),
            "记录层分区标题必须保留: {}",
            text
        );
        assert!(
            text.contains("联想 · 符号层落边（结构算子推导，可能不成立）"),
            "★符号层落盘边必须有独立分区标题: {}",
            text
        );
        // ② 符号层分区必须显式声明"不是记录事实 / 可能不成立"
        assert!(
            text.contains("可能不成立"),
            "★符号层分区必须声明可能不成立（否则用户当事实引用）: {}",
            text
        );
        // ③ 符号层条目必须用符号层标签表（而非记录层兜底「相关联」）
        assert!(
            text.contains("相综/互卦（协同）"),
            "★符号层类型必须走 structural_rel_label: {}",
            text
        );
        // ④ 符号层条目不得被冠以「记录型关联」的条目名
        let sym_line = text
            .lines()
            .find(|l| l.contains("sym-1") || l.contains("符号层落边 #"))
            .unwrap_or("");
        assert!(
            sym_line.contains("符号层落边 #"),
            "★符号层条目应以「符号层落边 #N」开头，实际: {sym_line}"
        );
        assert!(
            !sym_line.contains("记录型关联 #"),
            "★符号层条目不得被标为「记录型关联」: {sym_line}"
        );
        // ⑤ 记录层条目仍走「记录型关联 #N」
        assert!(
            text.contains("记录型关联 #1"),
            "记录层条目命名不得被改: {}",
            text
        );
    }

    /// ★★审查发现 3 的**边界**：只有一种来源时不得凭空产生另一个分区。
    #[test]
    fn test_only_one_source_renders_only_one_section() {
        let rec = vec![AssociatedMemory {
            memory_id: "rec-only".to_string(),
            content_preview: "只有记录层".to_string(),
            memory_type: "fact".to_string(),
            relation: "shared_entity".to_string(),
            why: "共享实体（杭州）".to_string(),
            via_memory_id: "seed".to_string(),
            via_preview: "起点".to_string(),
            hops: 1,
            path: vec!["seed".to_string(), "rec-only".to_string()],
        }];
        let mut t = String::new();
        append_associated_memories(&mut t, &rec);
        assert!(t.contains("记录型关联"), "应渲染记录层分区");
        assert!(
            !t.contains("符号层落边（结构算子推导"),
            "★无符号层条目时不得出现符号层分区（否则是虚假分区）: {}",
            t
        );

        let sym = vec![AssociatedMemory {
            memory_id: "sym-only".to_string(),
            content_preview: "只有符号层".to_string(),
            memory_type: "fact".to_string(),
            relation: "cause".to_string(),
            why: "图存储既有边（符号层推导边，因果）".to_string(),
            via_memory_id: "seed".to_string(),
            via_preview: "起点".to_string(),
            hops: 1,
            path: vec!["seed".to_string(), "sym-only".to_string()],
        }];
        let mut t2 = String::new();
        append_associated_memories(&mut t2, &sym);
        assert!(t2.contains("符号层落边"), "应渲染符号层分区");
        assert!(
            !t2.contains("记录型关联（由记录推导"),
            "★无记录层条目时不得出现记录层分区: {}",
            t2
        );
        // 符号层单独出现时也不得使用"由记录必然关联"这类断言
        assert!(
            !t2.contains("由记录必然关联"),
            "★符号层分区不得断言'由记录必然关联': {}",
            t2
        );
    }

    /// ★★审查发现 1：超时降态必须**分级**——部分完成时不得说成"本次跳过"。
    ///
    /// # 为什么必须有这条
    ///
    /// 修复前超时分支只 `return` + 整体降态，**已完成的一侧被一起丢弃**，
    /// 用户看到"符号层本次跳过"，而实际拿到了候选卦。
    /// "没跑"与"跑了一半"是两件事，对用户的处置完全不同。
    #[test]
    fn test_partial_degraded_is_distinct_from_total_skip() {
        // ① 部分完成：只说未跑完的那个分区
        let mut t1 = String::new();
        append_symbolic_layer_partial(&mut t1, true, false);
        assert!(
            t1.contains("部分完成"),
            "★部分完成必须明说『部分完成』而非『本次跳过』: {t1}"
        );
        assert!(
            !t1.contains("本次跳过"),
            "★部分完成不得说成『本次跳过』（会掩盖已拿到的结果）: {t1}"
        );
        assert!(
            t1.contains("符号层候选（结构方向）"),
            "★必须指明是哪个分区未跑完: {t1}"
        );
        assert!(
            !t1.contains("符号层落边（§5.4）"),
            "★已完成的分区不得被列为未跑完: {t1}"
        );
        assert!(
            t1.contains("真实结果"),
            "★必须说明上面展示的是真实结果（不是降级数据）: {t1}"
        );

        // ② 反向：只有落边未跑完
        let mut t2 = String::new();
        append_symbolic_layer_partial(&mut t2, false, true);
        assert!(t2.contains("符号层落边（§5.4）"));
        assert!(!t2.contains("符号层候选（结构方向）"));

        // ③ 两边都完成 ⇒ 不输出任何降态块（避免虚假降态）
        let mut t3 = String::new();
        append_symbolic_layer_partial(&mut t3, false, false);
        assert!(
            t3.is_empty(),
            "★两边都完成时不得输出降态块（否则是虚假降态）: {t3}"
        );

        // ④ 整体降态（两边都没拿到）仍走原函数，措辞与之必须不同
        let mut t4 = String::new();
        append_symbolic_layer_degraded(&mut t4);
        assert!(t4.contains("本次跳过"));
        assert_ne!(t1, t4, "两种降态措辞必须不同");
    }

    /// v0.9.8：`/build_edges` 被门控阻断时，渲染层必须输出**四要素**：
    /// 状态（主动门控，非故障）/ 原因码 / 判据依据 / 解封条件。
    ///
    /// ★为什么这条是核心：阻断本身是**正确行为**（防污染图）。
    /// 真正的失效模式是**静默阻断**——用户以为功能坏了或没做，
    /// 而实际是判据未过、有明确的解封路径。两种认知的处置完全不同。
    #[test]
    fn test_append_daoti_build_edges_explains_block() {
        // ★原因码取**Python 侧真实值**（`assoc_service.py` 的
        //   `build_edges_for_seeds` 实际返回 `text_to_gua_judge_not_passed`）。
        //   此前 mock 里写的是 `no_usable_text_to_gua_path`（旧值，早已弃用）
        //   ⇒ mock 与实现"一起错"，测试永远绿、却测不到真实契约。
        let res = serde_json::json!({
            "edges": [],
            "blocked": "text_to_gua_judge_not_passed",
            "blocked_detail": "四条判据未全过，未通过项: ['dispersion']",
            "blocked_is_gate": true,
            "unblock_env": "DAOTI_ALLOW_UNSTABLE_GUA=1",
            "version": "daoti-assoc-v1"
        });
        let mut text = String::new();
        append_daoti_build_edges(&mut text, &res);
        assert!(
            text.contains("主动门控"),
            "必须说明是主动门控而非故障: {}",
            text
        );
        // ★断言"回显了服务端给的原因码"，而**不拼死具体字面量**：
        //   渲染层的职责是透传，不该知道有哪些原因码。
        //   若在此写死某个串，Python 侧改原因码时这里会静默失配
        //   （这正是上一版 mock 的病因）。
        assert!(
            text.contains("text_to_gua_judge_not_passed"),
            "必须原样回显服务端给的原因码: {}",
            text
        );
        assert!(text.contains("四条判据未全过"), "必须给出判据依据");
        assert!(
            text.contains("DAOTI_ALLOW_UNSTABLE_GUA"),
            "必须给出解封条件"
        );
        assert!(text.contains("不依赖"), "必须说明不影响其它分区");
    }

    /// v0.9.8：原因码是**透传**的 —— 换任意值都必须原样回显。
    ///
    /// ★为什么单独测"任意值"：上面那条只验了一个具体串，若渲染层哪天
    ///   变成"只认某个白名单、其余显示 unknown"，上面那条仍会通过。
    ///   本测试用一个人为值，锁住"纯透传"这一契约。
    #[test]
    fn test_append_daoti_build_edges_echoes_any_reason_code() {
        let res = serde_json::json!({
            "edges": [],
            "blocked": "some_future_reason_v2",
            "blocked_detail": "未来某版新增的判据",
            "blocked_is_gate": true,
            "version": "daoti-assoc-v1"
        });
        let mut text = String::new();
        append_daoti_build_edges(&mut text, &res);
        assert!(
            text.contains("some_future_reason_v2"),
            "原因码必须纯透传（不得白名单化）: {}",
            text
        );
    }

    /// v0.9.8：实验放行（`DAOTI_ALLOW_UNSTABLE_GUA=1`）时，渲染层
    /// **必须**显示 `unstable_warning`——否则用户会把实验边当证据引用。
    #[test]
    fn test_append_daoti_build_edges_warns_on_unstable_release() {
        let res = serde_json::json!({
            "edges": [{
                "from_id": "aaaaaaaa-1111", "to_id": "bbbbbbbb-2222",
                "rel_type": "COORDINATE", "how": "cuo_gua", "gua_name": "需"
            }],
            "unstable_warning": "本次落边使用了未通过判据的通路",
            "write_back": {"written": 1, "skipped": 0, "total": 1},
            "version": "daoti-assoc-v1"
        });
        let mut text = String::new();
        append_daoti_build_edges(&mut text, &res);
        assert!(text.contains("产出 1 条结构边"), "应报告边数");
        assert!(text.contains("相综/互卦（协同）"), "关系名应走符号层标签表");
        assert!(text.contains("未通过判据"), "★必须显示不可作证据的警告");
        assert!(text.contains("落图: 新增 1"), "应报告写图统计");
    }

    /// v0.9.8：`/build_edges` 客户端的三类降态（不可达 / 版本不符 / 缺 edges）。
    ///
    /// 与 `/cycle` 同纪律：**不做字段猜测**——对方改字段语义后静默按旧含义
    /// 解读，比失败更危险。
    #[tokio::test]
    async fn test_fetch_daoti_build_edges_degrades() {
        // (1) 空种子：不发请求
        assert!(
            fetch_daoti_build_edges_with_base(&[], false, Some("http://127.0.0.1:1"))
                .await
                .is_none()
        );
        // (2) 不可达端口
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let seeds = vec![("a".to_string(), "文本".to_string())];
        assert!(
            fetch_daoti_build_edges_with_base(&seeds, false, Some(&format!("http://{}", addr)))
                .await
                .is_none(),
            "不可达必须降态为 None"
        );
        // (3) 版本不符 / (4) 缺 edges
        let bad_version = "{\"edges\":[],\"version\":\"unknown-v9\"}";
        assert!(
            run_mock_build_edges_and_fetch(bad_version).await.is_none(),
            "版本不符必须降态"
        );
        let no_edges = "{\"version\":\"daoti-assoc-v1\"}";
        assert!(
            run_mock_build_edges_and_fetch(no_edges).await.is_none(),
            "缺 edges 必须降态"
        );
        // (5) 合法但被阻断（空 edges + blocked）⇒ 必须**成功返回**（不是降态）
        // ★原因码用 Python 侧真实值（旧值 `no_usable_text_to_gua_path` 已弃用）
        let blocked = "{\"edges\":[],\"blocked\":\"text_to_gua_judge_not_passed\",\"version\":\"daoti-assoc-v1\"}";
        let got = run_mock_build_edges_and_fetch(blocked).await;
        assert!(got.is_some(), "阻断是合法业务结果，不得当降态丢弃");
        assert_eq!(
            got.unwrap().get("blocked").and_then(|v| v.as_str()),
            Some("text_to_gua_judge_not_passed")
        );
    }

    /// 起一个只回固定 body 的 mock `/build_edges`（供降态用例复用）。
    ///
    /// ★把收到的请求体一并回传（2026-09-18，同 S8 修法）：断言在主线程做，
    ///   不写在 spawn 的 task 内——task 内 panic 不会传播（见
    ///   `test_fetch_daoti_cycle_online_parses_hops` 的说明）。
    async fn run_mock_build_edges_and_fetch(body: &'static str) -> Option<serde_json::Value> {
        let (out, _) = run_mock_build_edges_and_fetch_capture(body).await;
        out
    }

    /// 同上，但额外回传 mock 收到的请求体（供契约断言）。
    async fn run_mock_build_edges_and_fetch_capture(
        body: &'static str,
    ) -> (Option<serde_json::Value>, Vec<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_mock = seen.clone();
        let mock = tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                if let Ok(mut g) = seen_mock.lock() {
                    g.push(String::from_utf8_lossy(&buf[..n]).to_string());
                }
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        let seeds = vec![("a".to_string(), "文本".to_string())];
        let out =
            fetch_daoti_build_edges_with_base(&seeds, false, Some(&format!("http://{}", addr)))
                .await;
        mock.abort();
        let reqs = seen.lock().map(|g| g.clone()).unwrap_or_default();
        (out, reqs)
    }

    /// v0.9.8：`/build_edges` 请求体必须携带 seeds（memory_id + text），
    /// 且 `write_back` 必须随门控如实传递（默认 false ⇒ 不得悄悄写图）。
    ///
    /// ★为什么必须测"不写图"：`write_back` 是**有副作用**的开关，
    ///   若默认被传成 true，一次普通检索就会改用户的图（承"写图需更严门控"）。
    #[tokio::test]
    async fn test_fetch_daoti_build_edges_sends_seeds_and_write_back() {
        let body = "{\"edges\":[],\"blocked\":\"x\",\"version\":\"daoti-assoc-v1\"}";
        let (out, reqs) = run_mock_build_edges_and_fetch_capture(body).await;

        assert!(!reqs.is_empty(), "mock 应至少收到一次 /build_edges 请求");
        let req = &reqs[0];
        assert!(
            req.contains("\"seeds\""),
            "请求必须携带 seeds，实际: {}",
            req
        );
        assert!(
            req.contains("\"memory_id\":\"a\""),
            "seeds 必须含 memory_id，实际: {}",
            req
        );
        assert!(
            req.contains("\"text\":\"文本\""),
            "seeds 必须含 text（该字段是服务端推导的输入），实际: {}",
            req
        );
        assert!(
            req.contains("\"write_back\":false"),
            "★write_back 必须如实传 false（写图有副作用，不得默认写），实际: {}",
            req
        );
        assert!(out.is_some(), "合法响应不应降态");
    }

    /// 构建测试用 AppState（带已索引的 manager 和记忆存储）
    fn test_state() -> Arc<AppState> {
        let mut manager = CodeMemoryManager::new();
        manager.index_file(
            "src/test.rs",
            "fn hello() {\n    println!(\"world\");\n}\n\nstruct Foo {\n    bar: i32,\n}\n",
        );
        manager.index_file(
            "src/memory.rs",
            "fn store_memory() {}\n\nfn retrieve_memory() {}\n",
        );

        // 为测试创建临时持久化后端
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let data_dir = tmp.path().to_string_lossy().to_string();
        let persistence =
            crate::persistence::create_json_persistence(&data_dir).expect("持久化创建失败");
        let memory_store = Arc::new(Mutex::new(MemoryStore::new(persistence)));

        Arc::new(AppState {
            manager: Arc::new(Mutex::new(Box::new(manager))),
            memory_store,
            src_dir: "fixture/src".into(),
            data_dir: data_dir.clone(),
            llm_api: Arc::new(RwLock::new(LlmApiConfig::None)),
            llm_configured_atomic: Arc::new(AtomicBool::new(false)), // v0.8.22 P0-1: 无锁缓存
            indexing_complete: Arc::new(AtomicBool::new(true)),      // 测试环境默认索引已完成
            started_at: chrono::Utc::now(),
            dev_mode: false,
        })
    }

    fn to_json(resp: &JsonRpcResponse) -> serde_json::Value {
        serde_json::to_value(resp).unwrap()
    }

    /// v0.9.6 P0 契约测试：/health 响应必须暴露非空 data_dir，
    /// 否则桌面端 sidecar_identity_matches 恒失败、开发模式复用路径失效。
    /// 该测试直接序列化 HealthResponse，堵住"手工构造 DTO 绕过真实序列化"的假绿。
    #[test]
    fn health_response_must_expose_nonempty_data_dir() {
        let state = test_state();
        let response = HealthResponse {
            status: "running",
            service: "loong-recall",
            version: env!("CARGO_PKG_VERSION"),
            uptime_seconds: 1,
            indexing: IndexingStatus {
                complete: true,
                file_count: Some(0),
                total_chunks: Some(0),
            },
            memory: MemoryBrief { total: 0 },
            src_dir: state.src_dir.clone(),
            data_dir: state.data_dir.clone(),
            llm_configured: false,
            lock_busy: false,
        };
        let json = serde_json::to_value(&response).expect("HealthResponse 应可序列化");
        assert!(
            json.get("data_dir").is_some(),
            "契约断裂：/health 未序列化 data_dir，桌面端身份校验将恒失败"
        );
        assert!(
            !json["data_dir"].as_str().unwrap_or("").is_empty(),
            "data_dir 不得为空串，否则开发模式身份匹配恒失败"
        );
    }

    struct PanicCodebase;

    impl IndexedCodebase for PanicCodebase {
        fn search(&self, _query: &str, _top_k: usize) -> RetrievalResult {
            panic!("测试搜索 panic")
        }

        fn multi_keyword_search(&self, _keywords: &[String], _top_k: usize) -> RetrievalResult {
            panic!("测试搜索 panic")
        }

        fn get_stats(&self) -> ChunkStats {
            ChunkStats {
                file_count: 0,
                total_chunks: 0,
                type_counts: std::collections::HashMap::new(),
                language_counts: std::collections::HashMap::new(),
                avg_lines: 0.0,
            }
        }

        fn recent_chunks(&self, _top_k: usize) -> RetrievalResult {
            panic!("测试搜索 panic")
        }
    }

    #[tokio::test]
    async fn test_safe_code_search_converts_panic_to_error() {
        let manager: Arc<Mutex<Box<dyn IndexedCodebase>>> =
            Arc::new(Mutex::new(Box::new(PanicCodebase)));
        let result = safe_code_search(manager, vec!["panic".into()], 1).await;
        assert!(matches!(result, Err(SearchError::Panic)));
    }

    #[tokio::test]
    async fn test_safe_code_search_returns_lock_timeout() {
        let state = test_state();
        let guard = state.manager.clone().lock_owned().await;
        let result = safe_code_search(state.manager.clone(), vec!["memory".into()], 1).await;
        drop(guard);
        assert!(matches!(result, Err(SearchError::LockTimeout)));
    }

    // ---- 初始化与能力协商 ----

    #[test]
    fn test_initialize() {
        let resp = handle_initialize(Some(serde_json::Value::Number(1.into())));
        let json = to_json(&resp);

        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 1);
        assert_eq!(json["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(json["result"]["serverInfo"]["name"], "loong-recall");
        assert!(json["result"]["capabilities"]["tools"].is_object());
    }

    // ---- 工具列表 ----

    #[test]
    fn test_tools_list() {
        let resp = handle_tools_list(Some(serde_json::Value::Number(2.into())));
        let json = to_json(&resp);

        let tools = json["result"]["tools"]
            .as_array()
            .expect("tools/list 响应中 tools 应为数组，检查工具注册逻辑");
        assert_eq!(
            tools.len(),
            15,
            "应注册 15 个工具（9 个记忆 + 2 个代码 + 4 个新增）"
        );

        // 验证记忆工具存在
        let tool_names: Vec<&str> = tools
            .iter()
            .map(|t| {
                t["name"]
                    .as_str()
                    .expect("工具列表中每个条目应有 name 字符串字段")
            })
            .collect();
        assert!(tool_names.contains(&"remember"), "缺少 remember 工具");
        assert!(
            tool_names.contains(&"batch_remember"),
            "缺少 batch_remember 工具"
        );
        assert!(tool_names.contains(&"recall"), "缺少 recall 工具");
        assert!(tool_names.contains(&"forget"), "缺少 forget 工具");
        assert!(
            tool_names.contains(&"update_memory"),
            "缺少 update_memory 工具"
        );
        assert!(
            tool_names.contains(&"list_memories"),
            "缺少 list_memories 工具"
        );
        assert!(
            tool_names.contains(&"memory_stats"),
            "缺少 memory_stats 工具"
        );
        assert!(
            tool_names.contains(&"associations"),
            "缺少 associations 工具（记录层→多类型关联）"
        );
        assert!(
            tool_names.contains(&"association_graph"),
            "缺少 association_graph 工具（联想图 + 多跳结构推理）"
        );
        assert!(tool_names.contains(&"archive"), "缺少 archive 工具");
        assert!(tool_names.contains(&"search_code"), "缺少 search_code 工具");
        assert!(
            tool_names.contains(&"codebase_stats"),
            "缺少 codebase_stats 工具"
        );
        assert!(
            tool_names.contains(&"system_health"),
            "缺少 system_health 工具"
        );
        assert!(
            tool_names.contains(&"correct_memory"),
            "缺少 correct_memory 工具"
        );
        assert!(
            tool_names.contains(&"recall_enhanced"),
            "缺少 recall_enhanced 工具"
        );
        assert!(
            tool_names.contains(&"associations"),
            "缺少 associations 工具"
        );
    }

    // ---- search_code 工具调用 ----

    #[tokio::test]
    async fn test_search_code() {
        let state = test_state();
        let params = serde_json::json!({
            "name": "search_code",
            "arguments": {
                "query": "memory retrieve",
                "top_k": 3
            }
        });
        let resp =
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(4.into()))).await;
        let json = to_json(&resp);

        let text = json["result"]["content"][0]["text"]
            .as_str()
            .expect("search_code 工具返回的 text 字段应为字符串");
        assert_eq!(json["result"]["content"][0]["type"], "text");
        assert!(text.contains("memory"), "搜索结果应包含关键词: {}", text);
        assert!(text.contains("src/memory.rs"), "应包含文件路径: {}", text);
    }

    #[tokio::test]
    async fn test_search_code_no_match() {
        let state = test_state();
        let params = serde_json::json!({
            "name": "search_code",
            "arguments": {
                "query": "zzz_nonexistent_concept_xxx"
            }
        });
        let resp =
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(5.into()))).await;
        let json = to_json(&resp);

        let text = json["result"]["content"][0]["text"]
            .as_str()
            .expect("search_code 工具返回的 text 字段应为字符串");
        assert!(
            text.contains("未找到") || text.contains("提示"),
            "无匹配时应给出提示: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_search_code_missing_query() {
        let state = test_state();
        let params = serde_json::json!({
            "name": "search_code",
            "arguments": {}
        });
        let resp =
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(6.into()))).await;
        let json = to_json(&resp);

        assert!(json["error"].is_object(), "缺少 query 应返回错误");
        assert_eq!(json["error"]["code"], -32602);
    }

    // ---- codebase_stats 工具调用 ----

    #[tokio::test]
    async fn test_codebase_stats() {
        let state = test_state();
        let params = serde_json::json!({
            "name": "codebase_stats",
            "arguments": {}
        });
        let resp =
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(7.into()))).await;
        let json = to_json(&resp);

        let text = json["result"]["content"][0]["text"]
            .as_str()
            .expect("codebase_stats 工具返回的 text 字段应为字符串");
        assert!(text.contains("已索引文件"), "应包含统计信息: {}", text);
        assert!(text.contains("fn"), "应包含类型分布: {}", text);
    }

    // ---- 错误处理 ----

    #[test]
    fn test_unknown_method() {
        let resp = make_error(
            Some(serde_json::Value::Number(8.into())),
            -32601,
            "未知方法: foo",
        );
        let json = to_json(&resp);

        assert!(json["error"].is_object());
        assert_eq!(json["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn test_unknown_tool() {
        let state = test_state();
        let params = serde_json::json!({
            "name": "nonexistent_tool",
            "arguments": {}
        });
        let resp =
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(9.into()))).await;
        let json = to_json(&resp);

        assert!(json["error"].is_object());
        assert_eq!(json["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn test_tools_call_missing_name() {
        let state = test_state();
        let params = serde_json::json!({
            "arguments": {}
        });
        let resp =
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(10.into()))).await;
        let json = to_json(&resp);

        assert!(json["error"].is_object());
        assert_eq!(json["error"]["code"], -32602);
    }

    // ---- 通知处理 ----

    #[test]
    fn test_notifications_response_is_empty() {
        // 验证通知的返回结构
        let resp = JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: None,
            result: None,
            error: None,
        };
        let json = to_json(&resp);
        assert!(json.get("result").is_none());
        assert!(json.get("error").is_none());
    }

    // ---- remember 记忆写入工具测试 ----

    #[tokio::test]
    async fn test_remember() {
        let state = test_state();
        let params = serde_json::json!({
            "name": "remember",
            "arguments": {
                "content": "用户偏好使用 pnpm 作为包管理器",
                "memory_type": "preference",
                "tags": ["pnpm", "tooling"],
                "importance": 8
            }
        });
        let resp =
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(100.into()))).await;
        let json = to_json(&resp);

        assert!(json["result"].is_object(), "remember 应返回成功结果");
        let text = json["result"]["content"][0]["text"]
            .as_str()
            .expect("remember 工具返回的 text 字段应为字符串");
        assert!(text.contains("已记住"), "应包含确认信息: {}", text);
        assert!(text.contains("pnpm"), "应包含记忆内容: {}", text);
    }

    #[tokio::test]
    async fn test_remember_missing_content() {
        let state = test_state();
        let params = serde_json::json!({
            "name": "remember",
            "arguments": {}
        });
        let resp =
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(101.into()))).await;
        let json = to_json(&resp);

        assert!(json["error"].is_object(), "缺少 content 应返回错误");
        assert_eq!(json["error"]["code"], -32602);
    }

    // ---- recall 记忆检索工具测试 ----

    #[tokio::test]
    async fn test_recall() {
        let state = test_state();

        // 先写入一条记忆
        let remember_params = serde_json::json!({
            "name": "remember",
            "arguments": {
                "content": "该项目使用 PostgreSQL 作为主数据库",
                "memory_type": "fact",
                "tags": ["database", "postgresql"]
            }
        });
        handle_tools_call(&state, &remember_params, None).await;

        // 再检索
        let params = serde_json::json!({
            "name": "recall",
            "arguments": {
                "query": "PostgreSQL 数据库",
                "top_k": 5
            }
        });
        let resp =
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(102.into()))).await;
        let json = to_json(&resp);

        assert!(json["result"].is_object(), "recall 应返回成功结果");
        let text = json["result"]["content"][0]["text"]
            .as_str()
            .expect("recall 工具返回的 text 字段应为字符串");
        assert!(text.contains("PostgreSQL"), "应包含检索到的内容: {}", text);
    }

    #[tokio::test]
    async fn test_recall_missing_query() {
        let state = test_state();
        let params = serde_json::json!({
            "name": "recall",
            "arguments": {}
        });
        let resp =
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(103.into()))).await;
        let json = to_json(&resp);

        assert!(json["error"].is_object());
        assert_eq!(json["error"]["code"], -32602);
    }

    // ---- forget 记忆删除工具测试 ----

    #[tokio::test]
    async fn test_forget() {
        let state = test_state();

        // 先写入一条记忆
        let remember_params = serde_json::json!({
            "name": "remember",
            "arguments": {
                "content": "待删除的测试记忆"
            }
        });
        let remember_resp = handle_tools_call(&state, &remember_params, None).await;
        let remember_json = to_json(&remember_resp);
        // 从响应中提取记忆 ID（LLM 响应格式：包含 "ID: xxx)" 模式）
        let text = remember_json["result"]["content"][0]["text"]
            .as_str()
            .expect("LLM 响应中 text 字段应为字符串，检查 remember 工具返回格式");
        let id_start = text
            .find("ID: ")
            .expect("LLM 响应中未找到 'ID: ' 前缀，检查 remember 工具输出格式")
            + 4;
        let id_end = text[id_start..]
            .find(')')
            .expect("LLM 响应中未找到 ID 结束括号 ')'，检查 remember 工具输出格式")
            + id_start;
        let memory_id = &text[id_start..id_end];

        // 删除该记忆
        let params = serde_json::json!({
            "name": "forget",
            "arguments": {
                "memory_id": memory_id
            }
        });
        let resp =
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(104.into()))).await;
        let json = to_json(&resp);

        assert!(json["result"].is_object());
        let forget_text = json["result"]["content"][0]["text"]
            .as_str()
            .expect("forget 工具返回的 text 字段应为字符串");
        assert!(
            forget_text.contains("已删除"),
            "应确认删除: {}",
            forget_text
        );
    }

    #[tokio::test]
    async fn test_forget_missing_id() {
        let state = test_state();
        let params = serde_json::json!({
            "name": "forget",
            "arguments": {}
        });
        let resp =
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(105.into()))).await;
        let json = to_json(&resp);

        assert!(json["error"].is_object());
        assert_eq!(json["error"]["code"], -32602);
    }

    // ---- update_memory 记忆更新工具测试 ----

    #[tokio::test]
    async fn test_update_memory() {
        let state = test_state();

        // 先写入一条记忆
        let remember_params = serde_json::json!({
            "name": "remember",
            "arguments": {
                "content": "旧版本内容"
            }
        });
        let remember_resp = handle_tools_call(&state, &remember_params, None).await;
        let remember_json = to_json(&remember_resp);
        let text = remember_json["result"]["content"][0]["text"]
            .as_str()
            .expect("LLM 响应中 text 字段应为字符串，检查 remember 工具返回格式");
        let id_start = text
            .find("ID: ")
            .expect("LLM 响应中未找到 'ID: ' 前缀，检查 remember 工具输出格式")
            + 4;
        let id_end = text[id_start..]
            .find(')')
            .expect("LLM 响应中未找到 ID 结束括号 ')'，检查 remember 工具输出格式")
            + id_start;
        let memory_id = &text[id_start..id_end];

        // 更新该记忆
        let params = serde_json::json!({
            "name": "update_memory",
            "arguments": {
                "memory_id": memory_id,
                "content": "新版本内容",
                "importance": 9
            }
        });
        let resp =
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(106.into()))).await;
        let json = to_json(&resp);

        assert!(json["result"].is_object());
        let update_text = json["result"]["content"][0]["text"]
            .as_str()
            .expect("update_memory 工具返回的 text 字段应为字符串");
        assert!(
            update_text.contains("已更新"),
            "应确认更新: {}",
            update_text
        );
        assert!(
            update_text.contains("新版本内容"),
            "应包含新内容: {}",
            update_text
        );
    }

    #[tokio::test]
    async fn test_update_memory_missing_params() {
        let state = test_state();
        let params = serde_json::json!({
            "name": "update_memory",
            "arguments": {
                "memory_id": "test-id"
            }
        });
        let resp =
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(107.into()))).await;
        let json = to_json(&resp);

        assert!(json["error"].is_object());
        assert_eq!(json["error"]["code"], -32602);
    }

    // ---- list_memories 记忆列表工具测试 ----

    #[tokio::test]
    async fn test_list_memories() {
        let state = test_state();

        // 写入多条记忆
        for content in &["记忆 A", "记忆 B", "记忆 C"] {
            let params = serde_json::json!({
                "name": "remember",
                "arguments": {
                    "content": *content
                }
            });
            handle_tools_call(&state, &params, None).await;
        }

        let params = serde_json::json!({
            "name": "list_memories",
            "arguments": {
                "limit": 10
            }
        });
        let resp =
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(108.into()))).await;
        let json = to_json(&resp);

        assert!(json["result"].is_object(), "list_memories 应返回成功结果");
        let list_text = json["result"]["content"][0]["text"]
            .as_str()
            .expect("list_memories 工具返回的 text 字段应为字符串");
        assert!(list_text.contains("记忆列表"), "应包含标题: {}", list_text);
        assert!(list_text.contains("共"), "应包含总数: {}", list_text);
    }

    // ---- associations 记忆关联工具测试 ----

    /// 记录层端到端：写入同一次经历的两条记忆 + 共享实体，
    /// 关联工具必须同时给出 same_event 与 shared_entity 两类关联。
    #[tokio::test]
    async fn test_associations_multi_type_end_to_end() {
        let state = test_state();

        // 同一次经历（event_id=e1）的两条记忆：语义上分属"吃"与"路况"
        let a = serde_json::json!({
            "name": "remember",
            "arguments": {
                "content": "和爸妈去杭州西湖，在苏堤走了一下午",
                "memory_type": "experience",
                "event_id": "e1",
                "entities": [{"name": "爸妈", "kind": "person"}]
            }
        });
        let resp_a = handle_tools_call(&state, &a, None).await;
        let text_a = to_json(&resp_a)["result"]["content"][0]["text"]
            .as_str()
            .expect("remember 应返回 text")
            .to_string();
        let id_start = text_a.find("ID: ").expect("未找到 ID 前缀") + 4;
        let id_end = text_a[id_start..]
            .find([')', '\n', ' '])
            .map(|i| id_start + i)
            .unwrap_or_else(|| text_a[id_start..].trim_end().len() + id_start);
        let mem_a = text_a[id_start..id_end].to_string();

        // 跨经历共享实体「爸妈」的记忆
        let b = serde_json::json!({
            "name": "remember",
            "arguments": {
                "content": "爸爸下个月生日，想送他一套钓鱼竿",
                "memory_type": "experience",
                "event_id": "e2",
                "entities": [{"name": "爸妈", "kind": "person"}]
            }
        });
        handle_tools_call(&state, &b, None).await;

        let params = serde_json::json!({
            "name": "associations",
            "arguments": { "memory_id": mem_a }
        });
        let resp =
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(200.into()))).await;
        let json = to_json(&resp);
        let text = json["result"]["content"][0]["text"]
            .as_str()
            .expect("associations 应返回 text");
        assert!(
            text.contains("shared_entity"),
            "应给出共享实体关联（依据是实体，不是语义相似）: {}",
            text
        );
        assert!(text.contains("依据:"), "每条关联须有人类可读依据: {}", text);
    }

    #[tokio::test]
    async fn test_associations_missing_memory_id() {
        let state = test_state();
        let params = serde_json::json!({
            "name": "associations",
            "arguments": {}
        });
        let resp =
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(201.into()))).await;
        let json = to_json(&resp);
        assert!(json["error"].is_object());
        assert_eq!(json["error"]["code"], -32602);
    }

    // ---- memory_stats 记忆统计工具测试 ----

    #[tokio::test]
    async fn test_memory_stats() {
        let state = test_state();

        // 写入不同类型的记忆
        let facts_params = serde_json::json!({
            "name": "remember",
            "arguments": {
                "content": "事实记忆",
                "memory_type": "fact"
            }
        });
        handle_tools_call(&state, &facts_params, None).await;

        let pref_params = serde_json::json!({
            "name": "remember",
            "arguments": {
                "content": "偏好记忆",
                "memory_type": "preference"
            }
        });
        handle_tools_call(&state, &pref_params, None).await;

        let params = serde_json::json!({
            "name": "memory_stats",
            "arguments": {}
        });
        let resp =
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(109.into()))).await;
        let json = to_json(&resp);

        assert!(json["result"].is_object(), "memory_stats 应返回成功结果");
        let stats_text = json["result"]["content"][0]["text"]
            .as_str()
            .expect("memory_stats 工具返回的 text 字段应为字符串");
        assert!(
            stats_text.contains("记忆库统计"),
            "应包含标题: {}",
            stats_text
        );
        assert!(
            stats_text.contains("fact"),
            "应包含 fact 类型: {}",
            stats_text
        );
        assert!(
            stats_text.contains("preference"),
            "应包含 preference 类型: {}",
            stats_text
        );
    }

    // ---- archive 记忆归档工具测试 ----

    #[tokio::test]
    async fn test_archive_no_expired() {
        let state = test_state();

        let params = serde_json::json!({
            "name": "archive",
            "arguments": {}
        });
        let resp =
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(110.into()))).await;
        let json = to_json(&resp);

        assert!(json["result"].is_object(), "archive 应返回成功结果");
        let archive_text = json["result"]["content"][0]["text"]
            .as_str()
            .expect("archive（空记忆库）工具返回的 text 字段应为字符串");
        assert!(
            archive_text.contains("无过期记忆"),
            "无过期记忆时应给出提示: {}",
            archive_text
        );
    }

    #[tokio::test]
    async fn test_archive_with_expired() {
        let state = test_state();

        // 写入一条过期记忆（2 天前创建，ttl=1 天）
        use chrono::{Duration, Utc};
        let mut expired_memory = crate::Memory::new(
            "已过期的记忆".to_string(),
            crate::MemoryType::Fact,
            None,
            vec![],
            crate::Importance::default(),
            Some(1),
        );
        expired_memory.created_at = Utc::now() - Duration::days(2);

        let mut store = state.memory_store.lock().await;
        store.remember(expired_memory).expect("应成功写入过期记忆");
        drop(store); // 释放锁

        let params = serde_json::json!({
            "name": "archive",
            "arguments": {}
        });
        let resp =
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(111.into()))).await;
        let json = to_json(&resp);

        assert!(json["result"].is_object(), "archive 应返回成功结果");
        let archive_text = json["result"]["content"][0]["text"]
            .as_str()
            .expect("archive（单条归档）工具返回的 text 字段应为字符串");
        assert!(
            archive_text.contains("已归档"),
            "应确认归档: {}",
            archive_text
        );
        assert!(
            archive_text.contains("条过期记忆"),
            "应显示归档数量: {}",
            archive_text
        );
    }

    // ---- v0.7.1 P3-4: 静态资源路径遍历防护测试 ----

    #[tokio::test]
    async fn test_logo_asset_valid_filename() {
        // 有效文件名应返回 200 和 SVG 内容
        let resp = logo_asset_handler(axum::extract::Path("logo-primary.svg".to_string())).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_logo_asset_path_traversal() {
        // 路径遍历注入应返回 404，不应泄露文件系统内容
        let resp = logo_asset_handler(axum::extract::Path("../../../etc/passwd".to_string())).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_logo_asset_traversal_encoded() {
        // URL 编码的路径遍历也应返回 404
        let resp =
            logo_asset_handler(axum::extract::Path("..%2F..%2Fetc%2Fpasswd".to_string())).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_icon_asset_path_traversal() {
        // 图标路径遍历注入应返回 404
        let resp = icon_asset_handler(axum::extract::Path("../../../etc/shadow".to_string())).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_icon_asset_valid_filename() {
        // 有效图标文件名应返回 200
        let resp = icon_asset_handler(axum::extract::Path("icon-dashboard.svg".to_string())).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
