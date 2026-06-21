// 许可证: Apache 2.0
//
// MCP 协议服务端
// ===============
// 实现 Model Context Protocol (MCP) 服务端，通过 HTTP + JSON-RPC 2.0 暴露代码检索工具。
// IDE 可通过 MCP 协议调用 search_code 工具，自动获取项目代码上下文。
//
// 协议参考: https://spec.modelcontextprotocol.io/
// 当前暴露 search_code + codebase_stats 两个工具

use crate::memory_store::{ListFilter, MemoryStore, RecallFilter, SortBy, SortOrder};
use crate::persistence::json::JsonPersistence;
use crate::{
    ChunkStats, CodeMemoryManager, Importance, LlmApiConfig, Memory, MemoryType, PrivacyLevel,
    RecallResult, RetrievalResult,
};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

// ==================== JSON-RPC 2.0 类型 ====================

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
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
    /// LLM 是否已配置
    llm_configured: bool,
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
    pub manager: Arc<Mutex<Box<dyn IndexedCodebase>>>,
    pub memory_store: Arc<Mutex<MemoryStore<JsonPersistence>>>,
    pub src_dir: String,
    /// LLM API 配置（运行时可变，通过 /api/config/llm 动态更新）
    pub llm_api: Arc<RwLock<LlmApiConfig>>,
    /// 后台索引是否已完成（AtomicBool 支持无锁读取）
    pub indexing_complete: Arc<AtomicBool>,
    /// 服务启动时间（用于计算 uptime）
    pub started_at: chrono::DateTime<chrono::Utc>,
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
            description: "帮 AI 记住一件事——就像给 AI 装了个记事本。记住的内容会在后续对话中自动被检索到。【主动调用时机】1) 用户表达了技术偏好（'用 pnpm'）；2) 做了项目决策（'数据库选 PostgreSQL'）；3) 有重要的约定需要跨会话保留；4) 完成代码修改任务后，自动记录新增的模块、API 端点、架构变更等关键信息（这是任务的自然收尾，无需用户提醒）。".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "content": {
                        "type": "string",
                        "description": "记忆内容，如 '用户偏好使用 pnpm 而非 npm'"
                    },
                    "memory_type": {
                        "type": "string",
                        "description": "记忆类型: fact | preference | decision | code_context | conversation",
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
                    }
                }),
                required: vec!["content".into()],
            },
        },
        ToolDefinition {
            name: "batch_remember".into(),
            description: "批量记忆注入 — 一次性写入多条记忆，大幅提升大批量数据注入性能。适用于 LongMemEval 等需要注入大量会话历史的场景。单次最多 200 条。".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
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
                                    "description": "记忆类型: fact | preference | decision | code_context | conversation",
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

    let mut store = state.memory_store.lock().await;

    // LLM 记忆翻译：将问题翻译为答案可能包含的关键词，桥接语义鸿沟
    // v0.5.4 P2-11 修复：添加翻译状态日志，便于调试 LLM 响应问题
    let enriched_query = if state.llm_api.read().await.is_configured() {
        let keywords = crate::engine::llm_translator::translate_memory_query(
            &*state.llm_api.read().await,
            query,
        )
        .await;
        let translated: String = keywords.join(" ");
        if translated.is_empty() || translated.trim() == query {
            eprintln!(
                "[LRC·LLM] 增强检索翻译未产生有效结果，使用原始查询: {}",
                query
            );
            query.to_string()
        } else {
            eprintln!("[LRC·LLM] 增强检索翻译成功: {} → {}", query, translated);
            format!("{} {}", translated, query)
        }
    } else {
        query.to_string()
    };

    // 快速通路：关键词匹配，使用富化查询
    let fast_filter = RecallFilter {
        memory_type: memory_type.clone(),
        project: project.clone(),
        tags: tags.clone(),
        min_importance: None,
        top_k: top_k * 2,
        privacy_context: None,
    };
    let fast_result = store
        .recall(&enriched_query, &fast_filter)
        .unwrap_or(RecallResult {
            memories: vec![],
            scores: vec![],
            total: 0,
        });

    // 深度通路：深度语义检索，使用富化查询
    let deep_filter = RecallFilter {
        memory_type,
        project,
        tags,
        min_importance: None,
        top_k: top_k * 2,
        privacy_context: None,
    };
    let deep_result = store
        .trapezoid_focus_recall(&enriched_query, &deep_filter, 1)
        .unwrap_or(RecallResult {
            memories: vec![],
            scores: vec![],
            total: 0,
        });

    // 倒数排名融合 (RRF, Reciprocal Rank Fusion) — 使用共享 rrf_fuse
    let fused = crate::engine::rrf::rrf_fuse(
        &fast_result,
        &deep_result,
        top_k,
        crate::engine::rrf::RRF_DEFAULT_K,
    );
    let result_memories = fused.memories;
    let result_scores = fused.scores;
    let total = fused.total_candidates;

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

    let call_result = ToolCallResult {
        content: vec![TextContent {
            content_type: "text".into(),
            text,
        }],
    };
    make_response(id, to_json_value_safe(&call_result))
}

/// 处理 recall 工具调用 — 关键词匹配 / 深度语义检索
///
/// 支持 lrc_mode: "fast"（关键词匹配，默认）或 "deep"（深度语义检索）
/// 若配置了 LLM API，自动将查询翻译为答案关键词以桥接语义鸿沟
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
    };

    let mut store = state.memory_store.lock().await;

    // LLM 记忆翻译：将自然语言问题翻译为答案可能包含的关键词
    // 桥接"问题词与答案词不重叠"的语义鸿沟
    // v0.5.4 P2-11 修复：添加翻译状态日志，便于调试 LLM 响应问题
    let enriched_query = if state.llm_api.read().await.is_configured() {
        let keywords = crate::engine::llm_translator::translate_memory_query(
            &*state.llm_api.read().await,
            query,
        )
        .await;
        let translated: String = keywords.join(" ");
        if translated.is_empty() || translated.trim() == query {
            eprintln!("[LRC·LLM] 记忆翻译未产生有效结果，使用原始查询: {}", query);
            query.to_string()
        } else {
            eprintln!("[LRC·LLM] 记忆翻译成功: {} → {}", query, translated);
            format!("{} {}", translated, query)
        }
    } else {
        query.to_string()
    };

    // 根据 lrc_mode 选择检索方法（使用富化后的查询）
    let result = if lrc_mode == "deep" {
        store.trapezoid_focus_recall(&enriched_query, &filter, focus_depth)
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
                    text.push_str(&format!("\nID: `{}`\n\n", m.id));
                }
                text.push_str("💡 在回复中引用记忆时，请使用「（根据记忆 #N）」的格式标注来源，让用户能看见和信任记忆的存在。\n");
            }

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

        memories.push(memory);
    }

    let total = memories.len();
    let mut store = state.memory_store.lock().await;
    match store.remember_batch(memories) {
        Ok(saved) => {
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

    let memory = Memory::new(
        content.to_string(),
        memory_type,
        project,
        tags,
        importance,
        ttl_days,
    )
    .with_privacy(privacy_level, session_id, user_id);

    let mut store = state.memory_store.lock().await;
    match store.remember(memory) {
        Ok(saved) => {
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

                    text.push_str("\n### 项目分布\n");
                    let mut projects: Vec<(&String, &usize)> = stats.by_project.iter().collect();
                    projects.sort_by(|a, b| b.1.cmp(a.1));
                    for (proj, count) in projects {
                        text.push_str(&format!("- `{}`: {} 条\n", proj, count));
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
            let keywords = if state.llm_api.read().await.is_configured() {
                crate::engine::llm_translator::translate_query(&*state.llm_api.read().await, query)
                    .await
            } else {
                vec![query.to_string()]
            };

            let manager = state.manager.lock().await;
            let result = manager.multi_keyword_search(&keywords, top_k);

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
                    manager.get_stats().file_count
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
            let store = state.memory_store.lock().await;
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

    // 获取索引统计信息
    let (file_count, total_chunks) = if indexing_complete {
        let manager = state.manager.lock().await;
        let stats = manager.get_stats();
        (Some(stats.file_count), Some(stats.total_chunks))
    } else {
        (None, None)
    };

    // 获取记忆库统计
    let memory_total = {
        let store = state.memory_store.lock().await;
        store.stats().map(|s| s.total_memories).unwrap_or(0)
    };

    // 判断服务阶段
    let status = if indexing_complete {
        "running"
    } else if uptime_seconds < 5 {
        "starting"
    } else {
        "indexing"
    };

    let llm_configured = state.llm_api.read().await.is_configured();

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
        llm_configured,
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

/// 项目信息响应结构体（V2: 项目指纹 + 规范化路径）
#[derive(Debug, Serialize)]
struct ProjectInfoResponse {
    /// 项目源码目录
    src_dir: String,
    /// 规范化后的绝对路径
    canonical_path: String,
    /// 项目指纹（SHA256 前 16 字符）
    fingerprint: String,
}

/// GET /api/project/info — 获取当前项目的指纹和路径信息
async fn project_info_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    use crate::project_id;
    let src_path = std::path::Path::new(&state.src_dir);
    let (fingerprint, canonical_path) = project_id::project_fingerprint_with_path(src_path);
    Json(ProjectInfoResponse {
        src_dir: state.src_dir.clone(),
        canonical_path,
        fingerprint,
    })
}

/// 配置响应结构体
#[derive(Debug, Serialize)]
struct ConfigResponse {
    llm_configured: bool,
    llm_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    llm_model: Option<String>,
    /// v0.5.5 P1-3：LLM API 端点（用于仪表盘显示具体提供商）
    #[serde(skip_serializing_if = "Option::is_none")]
    llm_base_url: Option<String>,
}

/// GET /api/config — 获取当前 LLM 配置状态
async fn config_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let llm = state.llm_api.read().await;
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
    Json(ConfigResponse {
        llm_configured: configured,
        llm_type,
        llm_model,
        llm_base_url,
    })
}

/// POST /api/config/llm — 更新 LLM API Key 配置
///
/// 请求体: `{ "llm_api": "openai:sk-xxx:gpt-4o-mini" }`
/// 保存到全局配置文件，并立即生效用于后续查询翻译。
async fn config_llm_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
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
        let mut llm = state.llm_api.write().await;
        *llm = LlmApiConfig::None;
        // 保存到全局配置文件
        if let Err(e) = save_llm_to_config(None) {
            eprintln!("[配置] 清除 LLM API 配置失败: {e}");
        }
        // v0.5.5：同步清除 wizard.json 中的 LLM 配置
        if let Err(e) = save_llm_to_wizard_json("") {
            eprintln!("[配置] 同步清除 wizard.json LLM 配置失败: {e}");
        }
        // v0.5.5：更新 MemoryStore 的 LLM 配置状态
        {
            let store = state.memory_store.lock().await;
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
            let mut llm = state.llm_api.write().await;
            *llm = config;
            // 保存到全局配置文件
            if let Err(e) = save_llm_to_config(Some(&llm_str)) {
                eprintln!("[配置] 保存 LLM API 配置失败: {e}");
            }
            // v0.5.5：同步保存到 wizard.json，确保桌面端和仪表盘配置一致
            if let Err(e) = save_llm_to_wizard_json(&llm_str) {
                eprintln!("[配置] 同步保存 wizard.json LLM 配置失败: {e}");
            }
            // v0.5.5：更新 MemoryStore 的 LLM 配置状态
            {
                let store = state.memory_store.lock().await;
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

/// 保存 LLM API 配置到全局配置文件
fn save_llm_to_config(llm_api: Option<&str>) -> Result<(), String> {
    let mut cfg = crate::config::LrcConfig::load();
    cfg.llm_api = llm_api.map(|s| s.to_string());
    cfg.save()
}

/// v0.5.5 新增：同步保存 LLM 配置到 wizard.json
///
/// 仪表盘修改 LLM 配置后，同步到 wizard.json，确保桌面端和仪表盘配置一致。
/// wizard.json 路径：%APPDATA%\LoongRecall\wizard.json
/// API Key 使用 AES-256-GCM 加密存储（与桌面端一致）。
fn save_llm_to_wizard_json(llm_api: &str) -> Result<(), String> {
    let appdata =
        std::env::var("APPDATA").map_err(|e| format!("读取 APPDATA 环境变量失败: {}", e))?;
    let wizard_path = std::path::PathBuf::from(&appdata)
        .join("LoongRecall")
        .join("wizard.json");

    // 读取现有 wizard.json（如果存在），保留非 LLM 字段
    let mut wizard: serde_json::Value = if wizard_path.exists() {
        let content = std::fs::read_to_string(&wizard_path)
            .map_err(|e| format!("读取 wizard.json 失败: {}", e))?;
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
                        .map_err(|e| format!("加密 API Key 失败: {}", e))?;
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
        std::fs::create_dir_all(parent).map_err(|e| format!("创建 wizard.json 目录失败: {}", e))?;
    }

    // 写入 wizard.json
    let json_str = serde_json::to_string_pretty(&wizard)
        .map_err(|e| format!("序列化 wizard.json 失败: {}", e))?;
    std::fs::write(&wizard_path, json_str).map_err(|e| format!("写入 wizard.json 失败: {}", e))?;

    eprintln!(
        "[配置] LLM 配置已同步到 wizard.json: {}",
        wizard_path.display()
    );
    Ok(())
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

/// 创建 MCP 服务的 axum Router（合并 v1 REST API 端点 + 仪表盘）
///
/// 可嵌入到已有 axum 应用中，将 MCP 路由挂载到子路径。
pub fn build_mcp_router(state: Arc<AppState>) -> Router {
    // 创建 v1 API 路由（通过闭包捕获共享状态，状态类型为 ()）
    let v1_service =
        crate::v1_api::build_v1_router(state.memory_store.clone(), state.manager.clone())
            .into_service();

    Router::new()
        .route("/mcp", post(mcp_handler))
        .route("/health", get(health_handler))
        .route("/app.js", get(app_js_handler))
        .route("/app.css", get(app_css_handler))
        .nest_service("/v1", v1_service) // 将 v1 API 嵌套在 /v1 路径下
        // 仪表盘路由：静态文件 + 重定向
        .route("/dashboard", get(dashboard_handler))
        .route("/dashboard/", get(dashboard_handler))
        // 根路径重定向到仪表盘（方便桌面端直接加载）
        .route("/", get(root_redirect_handler))
        // 配置 API：仪表盘设置页面用
        .route("/api/config", get(config_handler))
        .route("/api/config/llm", post(config_llm_handler))
        // V2: 项目信息 API
        .route("/api/project/info", get(project_info_handler))
        .layer(tower_http::cors::CorsLayer::permissive())
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
    let app = build_mcp_router(state);

    let addr = format!("{}:{}", host, port);
    println!("Loong Recall (L-RC) 代码搜索 + 记忆服务");
    println!("   端点: http://{}", addr);
    println!("   仪表盘: http://{}/dashboard  ← 可视化记忆管理面板", addr);
    println!("   船长日志: GET  http://{}/v1/captains-log", addr);
    println!("   MCP 协议: POST http://{}/mcp", addr);
    println!("   状态检查: GET  http://{}/health", addr);

    axum::serve(listener, app).await
}

// ==================== MCP 协议单元测试 ====================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CodeMemoryManager;

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
            llm_api: Arc::new(RwLock::new(LlmApiConfig::None)),
            indexing_complete: Arc::new(AtomicBool::new(true)), // 测试环境默认索引已完成
            started_at: chrono::Utc::now(),
        })
    }

    fn to_json(resp: &JsonRpcResponse) -> serde_json::Value {
        serde_json::to_value(resp).unwrap()
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
            13,
            "应注册 13 个工具（8 个记忆 + 2 个代码 + 3 个新增）"
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
}
