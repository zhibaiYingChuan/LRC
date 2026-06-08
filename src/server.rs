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
    routing::post,
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

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

pub struct AppState {
    pub manager: Arc<Mutex<Box<dyn IndexedCodebase>>>,
    pub memory_store: Arc<Mutex<MemoryStore<JsonPersistence>>>,
    pub src_dir: String,
    pub llm_api: LlmApiConfig,
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
            description: "帮 AI 记住一件事——就像给 AI 装了个记事本。记住的内容会在后续对话中自动被检索到。适用场景：用户表达了技术偏好（'用 pnpm'）、做了项目决策（'数据库选 PostgreSQL'）、或者有重要的约定需要跨会话保留。".into(),
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
            description: "语义检索历史记忆。支持两种模式：fast（关键词匹配，默认）和 luoshu（洛书几何检索，使用 LuoShuEncoder + TrapezoidFocus）。luoshu 模式将查询投影到洛书九宫格，通过梯形聚焦在几何空间中定位记忆，返回洛书空间中距离最近的记忆。".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "query": {
                        "type": "string",
                        "description": "自然语言查询，如 '用户的包管理器偏好'"
                    },
                    "top_k": {
                        "type": "integer",
                        "description": "返回结果数（默认 5，最大 20）",
                        "default": 5
                    },
                    "lrc_mode": {
                        "type": "string",
                        "description": "检索模式: fast（关键词匹配，默认）| luoshu（洛书几何检索，使用 LuoShuEncoder 编码 + TrapezoidFocus 梯形聚焦）",
                        "default": "fast"
                    },
                    "focus_depth": {
                        "type": "integer",
                        "description": "梯形聚焦深度（仅 lrc_mode=luoshu 时生效）。0=全量检索，1=4分，2=16分。默认 1",
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
            description: "删除一条记忆。".into(),
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
            description: "更新一条已有记忆的内容。".into(),
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
            name: "dao_metrics".into(),
            description: "道同构度监控仪表 — 获取洛书记忆系统的健康度指标：道同构度评分、八卦分布熵、合成比率、编码/检索/合成/修正次数。".into(),
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
            description: "双路检索增强 — 快速通路（关键词匹配）+ 深度通路（洛书几何检索），通过倒数排名融合（RRF）合并结果。适用于需要深度背景的查询。".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "query": {
                        "type": "string",
                        "description": "自然语言查询"
                    },
                    "top_k": {
                        "type": "integer",
                        "description": "返回结果数（默认 5，最大 20）",
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
        "remember" => {
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

        "batch_remember" => {
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
                let memory_type =
                    MemoryType::try_parse(memory_type_str).unwrap_or(MemoryType::Fact);

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

        "recall" => {
            let query = match arguments.get("query").and_then(|q| q.as_str()) {
                Some(q) => q,
                None => return make_error(id, -32602, "缺少参数: query"),
            };
            let top_k = arguments
                .get("top_k")
                .and_then(|v| v.as_u64())
                .unwrap_or(5)
                .clamp(1, 20) as usize;

            // 检索模式：fast（关键词匹配）或 luoshu（洛书几何检索）
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

            // 根据 lrc_mode 选择检索方法
            let result = if lrc_mode == "luoshu" {
                // 洛书几何检索：LuoShuEncoder 编码查询 + TrapezoidFocus 梯形聚焦
                store.trapezoid_focus_recall(query, &filter, focus_depth)
            } else {
                // 快速模式：关键词匹配（默认）
                store.recall(query, &filter)
            };

            match result {
                Ok(result) => {
                    let mut text = format!(
                        "记忆检索结果 (共 {} 条匹配，记忆库共 {} 条，模式: {})\n\n",
                        result.memories.len(),
                        result.total,
                        if lrc_mode == "luoshu" {
                            "洛书几何检索"
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
                            text.push_str(&format!("内容: {}\n", m.summary()));
                            // 洛书模式显示几何信息
                            if lrc_mode == "luoshu" {
                                if let Some(ref cat) = m.bagua_category {
                                    text.push_str(&format!("八卦类别: {} | ", cat));
                                }
                                text.push_str(&format!("拓扑深度: {:.2} | ", m.topological_depth));
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

        "list_memories" => {
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
                    let mut text =
                        format!("记忆列表 (共 {} 条，本页 {} 条)\n\n", total, memories.len());

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
                .clamp(1, 20) as usize;

            // LLM 查询翻译：如果配置了 LLM API，先将自然语言翻译为关键词
            let keywords = if state.llm_api.is_configured() {
                crate::engine::llm_translator::translate_query(&state.llm_api, query).await
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

        // === L5 道同构度监控仪表 ===
        "dao_metrics" => {
            let store = state.memory_store.lock().await;
            match store.dao_metrics_snapshot() {
                Ok(snapshot) => {
                    let mut text = String::from("═══════════════════════════════════\n");
                    text.push_str("  道同构度 (DAO Isomorphism) 监控仪表\n");
                    text.push_str("═══════════════════════════════════\n\n");

                    text.push_str("### 核心指标\n");
                    text.push_str(&format!(
                        "- 道同构度: {:.1}%\n",
                        snapshot.dao_isomorphism_score * 100.0
                    ));
                    text.push_str(&format!(
                        "- 八卦分布熵: {:.3} (最大 3.0)\n",
                        snapshot.bagua_entropy
                    ));
                    text.push_str(&format!(
                        "- 合成比率: {:.1}%\n\n",
                        snapshot.synthesis_ratio * 100.0
                    ));

                    text.push_str("### 记忆容量\n");
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
                        text.push_str("\n⚠️ 道同构度偏低，建议检查洛书编码器或增加训练数据。\n");
                    }
                    if snapshot.bagua_entropy < 0.5 && snapshot.active_memories > 10 {
                        text.push_str("\n⚠️ 八卦分布过于集中，记忆可能存在类别偏差。\n");
                    }

                    let call_result = ToolCallResult {
                        content: vec![TextContent {
                            content_type: "text".into(),
                            text,
                        }],
                    };
                    make_response(id, to_json_value_safe(&call_result))
                }
                Err(e) => make_error(id, -32603, &format!("道同构度采集失败: {}", e)),
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

        // === 双路检索增强 ===
        "recall_enhanced" => {
            let query = match arguments.get("query").and_then(|q| q.as_str()) {
                Some(q) => q,
                None => return make_error(id, -32602, "缺少参数: query"),
            };
            let top_k = arguments
                .get("top_k")
                .and_then(|v| v.as_u64())
                .unwrap_or(5)
                .clamp(1, 20) as usize;

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

            // 快速通路：关键词匹配（已有 recall 逻辑）
            let fast_filter = RecallFilter {
                memory_type: memory_type.clone(),
                project: project.clone(),
                tags: tags.clone(),
                min_importance: None,
                top_k: top_k * 2, // 快速通路取更多结果
                privacy_context: None,
            };
            let fast_result = store.recall(query, &fast_filter).unwrap_or(RecallResult {
                memories: vec![],
                scores: vec![],
                total: 0,
            });

            // 深度通路：洛书几何检索（LuoShuEncoder + TrapezoidFocus 梯形聚焦）
            let deep_filter = RecallFilter {
                memory_type,
                project,
                tags,
                min_importance: None,
                top_k: top_k * 2,
                privacy_context: None,
            };
            let deep_result = store
                .trapezoid_focus_recall(query, &deep_filter, 1)
                .unwrap_or(RecallResult {
                    memories: vec![],
                    scores: vec![],
                    total: 0,
                });

            // 倒数排名融合 (RRF, Reciprocal Rank Fusion)
            // RRF 公式: score = sum(1 / (k + rank_i))，其中 k = 60
            let rrf_k: f32 = 60.0;
            let mut fused_scores: std::collections::HashMap<String, f32> =
                std::collections::HashMap::new();
            let mut id_to_memory: std::collections::HashMap<String, Memory> =
                std::collections::HashMap::new();

            // 快速通路排名
            for (rank, m) in fast_result.memories.iter().enumerate() {
                let score = 1.0 / (rrf_k + (rank + 1) as f32);
                *fused_scores.entry(m.id.clone()).or_insert(0.0) += score;
                id_to_memory
                    .entry(m.id.clone())
                    .or_insert_with(|| m.clone());
            }

            // 深度通路排名
            for (rank, m) in deep_result.memories.iter().enumerate() {
                let score = 1.0 / (rrf_k + (rank + 1) as f32);
                *fused_scores.entry(m.id.clone()).or_insert(0.0) += score;
                id_to_memory
                    .entry(m.id.clone())
                    .or_insert_with(|| m.clone());
            }

            // 按融合分数排序
            let mut scored: Vec<(f32, String)> = fused_scores
                .into_iter()
                .map(|(id, score)| (score, id))
                .collect();
            scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

            // 截取 top_k
            let total = id_to_memory.len();
            let result_memories: Vec<Memory> = scored
                .into_iter()
                .take(top_k)
                .filter_map(|(_, id)| id_to_memory.remove(&id))
                .collect();
            let result_scores: Vec<f32> = (0..result_memories.len())
                .map(|i| {
                    // 归一化 RRF 分数到 0-1
                    let rank = (i + 1) as f32;
                    1.0 / (1.0 + rank.log10())
                })
                .collect();

            let mut text = format!(
                "双路检索增强结果 (共 {} 条候选，返回 {} 条)\n\
                 ═══════════════════════════════════\n\
                 快速通路: 关键词匹配 | 深度通路: 洛书几何检索\n\
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
                    text.push_str(&format!("内容: {}\n", m.summary()));
                    // 显示八卦分类信息
                    if let Some(ref cat) = m.bagua_category {
                        let bagua_idx = m.bagua_index.unwrap_or(0);
                        let bagua_names = ["乾", "兑", "离", "震", "巽", "坎", "艮", "坤"];
                        let name = bagua_names.get(bagua_idx as usize).copied().unwrap_or("?");
                        text.push_str(&format!("八卦: {}·{} | ", name, cat));
                    }
                    text.push_str(&format!(
                        "类型: {} | 重要性: {}/10\n",
                        m.memory_type.as_str(),
                        m.importance.value()
                    ));
                    text.push_str(&format!("ID: `{}`\n\n", m.id));
                }
                text.push_str(
                    "💡 双路检索融合了快速关键词匹配和洛书几何定位，兼顾了召回率和精度。\n",
                );
            }

            let call_result = ToolCallResult {
                content: vec![TextContent {
                    content_type: "text".into(),
                    text,
                }],
            };
            make_response(id, to_json_value_safe(&call_result))
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

/// 健康检查端点（非 MCP 协议，便于调试）
async fn health_handler() -> &'static str {
    "Loong Recall 运行中 — 代码搜索 & 记忆服务"
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
        .unwrap_or_else(|_| {
            axum::response::Response::builder()
                .body("console.error('app.js 加载失败')".to_string())
                .unwrap()
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
        .route("/health", axum::routing::get(health_handler))
        .route("/app.js", axum::routing::get(app_js_handler))
        .nest_service("/v1", v1_service) // 将 v1 API 嵌套在 /v1 路径下
        // 仪表盘路由：静态文件 + 重定向
        .route("/dashboard", axum::routing::get(dashboard_handler))
        .route("/dashboard/", axum::routing::get(dashboard_handler))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(state)
}

/// 快速构建并绑定到指定地址
pub async fn serve(state: Arc<AppState>, host: &str, port: u16) -> std::io::Result<()> {
    let app = build_mcp_router(state);

    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
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
            llm_api: LlmApiConfig::None,
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

        let tools = json["result"]["tools"].as_array().unwrap();
        assert_eq!(
            tools.len(),
            13,
            "应注册 13 个工具（8 个记忆 + 2 个代码 + 3 个新增）"
        );

        // 验证记忆工具存在
        let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(tool_names.contains(&"remember"), "缺少 remember 工具");
        assert!(tool_names.contains(&"batch_remember"), "缺少 batch_remember 工具");
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
        assert!(tool_names.contains(&"dao_metrics"), "缺少 dao_metrics 工具");
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

        let text = json["result"]["content"][0]["text"].as_str().unwrap();
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

        let text = json["result"]["content"][0]["text"].as_str().unwrap();
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

        let text = json["result"]["content"][0]["text"].as_str().unwrap();
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
        let text = json["result"]["content"][0]["text"].as_str().unwrap();
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
        let text = json["result"]["content"][0]["text"].as_str().unwrap();
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
        // 从响应中提取记忆 ID
        let text = remember_json["result"]["content"][0]["text"]
            .as_str()
            .unwrap();
        let id_start = text.find("ID: ").unwrap() + 4;
        let id_end = text[id_start..].find(')').unwrap() + id_start;
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
        let forget_text = json["result"]["content"][0]["text"].as_str().unwrap();
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
            .unwrap();
        let id_start = text.find("ID: ").unwrap() + 4;
        let id_end = text[id_start..].find(')').unwrap() + id_start;
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
        let update_text = json["result"]["content"][0]["text"].as_str().unwrap();
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
        let list_text = json["result"]["content"][0]["text"].as_str().unwrap();
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
        let stats_text = json["result"]["content"][0]["text"].as_str().unwrap();
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
        let archive_text = json["result"]["content"][0]["text"].as_str().unwrap();
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
        let archive_text = json["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            archive_text.contains("已归档"),
            "有过期记忆时应确认归档: {}",
            archive_text
        );
        assert!(
            archive_text.contains("条过期记忆"),
            "应显示归档数量: {}",
            archive_text
        );
    }
}
