// MCP 协议服务端
// ===============
// 实现 Model Context Protocol (MCP) 服务端，通过 HTTP + JSON-RPC 2.0 暴露代码检索工具。
// IDE 可通过 MCP 协议调用 search_code 工具，自动获取项目代码上下文。
//
// 协议参考: https://spec.modelcontextprotocol.io/
// 当前暴露 search_code + codebase_stats 两个工具

use crate::{
    ChunkStats, CodeMemoryManager, Importance, Memory, MemoryType,
    RetrievalResult,
};
use crate::memory_store::{ListFilter, MemoryStore, RecallFilter, SortBy, SortOrder};
use crate::persistence::json::JsonPersistence;
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
    fn get_stats(&self) -> ChunkStats;
}

// 为泛型 CodeMemoryManager<E> 自动实现 IndexedCodebase
impl<E: crate::engine::encoder::CodeEncoder> IndexedCodebase for CodeMemoryManager<E> {
    fn search(&self, query: &str, top_k: usize) -> RetrievalResult {
        CodeMemoryManager::search(self, query, top_k)
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
}

// ==================== MCP 请求处理 ====================

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
    make_response(id, serde_json::to_value(result).unwrap())
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
                    }
                }),
                required: vec!["content".into()],
            },
        },
        ToolDefinition {
            name: "recall".into(),
            description: "语义检索历史记忆。搜索所有已存储的记忆，返回最相关的结果。".into(),
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
    ];

    let result = ToolsListResult { tools };
    make_response(id, serde_json::to_value(result).unwrap())
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

    let arguments = params.get("arguments").cloned().unwrap_or(serde_json::Value::Null);

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
            let memory_type = MemoryType::try_parse(memory_type_str)
                .unwrap_or(MemoryType::Fact);

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

            let memory = Memory::new(
                content.to_string(),
                memory_type,
                project,
                tags,
                importance,
                ttl_days,
            );

            let mut store = state.memory_store.lock().await;
            match store.remember(memory) {
                Ok(saved) => {
                    let text = format!(
                        "已记住 (ID: {})\n\
                         ──────────────────\n\
                         内容: {}\n\
                         类型: {} | 重要性: {}/10\n\
                         \n\
                         ✅ 下次你问相关问题时，AI 会自动检索到这条记忆。",
                        saved.id,
                        saved.content,
                        saved.memory_type.as_str(),
                        saved.importance.value()
                    );
                    let call_result = ToolCallResult {
                        content: vec![TextContent { content_type: "text".into(), text }],
                    };
                    make_response(id, serde_json::to_value(call_result).unwrap())
                }
                Err(e) => make_error(id, -32603, &format!("写入失败: {}", e)),
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
                .min(20) as usize;

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
            };

            let mut store = state.memory_store.lock().await;
            match store.recall(query, &filter) {
                Ok(result) => {
                    let mut text = format!(
                        "记忆检索结果 (共 {} 条匹配，记忆库共 {} 条)\n\n",
                        result.memories.len(),
                        result.total
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
                        content: vec![TextContent { content_type: "text".into(), text }],
                    };
                    make_response(id, serde_json::to_value(call_result).unwrap())
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
                        content: vec![TextContent { content_type: "text".into(), text }],
                    };
                    make_response(id, serde_json::to_value(call_result).unwrap())
                }
                Ok(false) => {
                    let text = format!("未找到记忆: {}（可能已被删除）", memory_id);
                    let call_result = ToolCallResult {
                        content: vec![TextContent { content_type: "text".into(), text }],
                    };
                    make_response(id, serde_json::to_value(call_result).unwrap())
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
                        content: vec![TextContent { content_type: "text".into(), text }],
                    };
                    make_response(id, serde_json::to_value(call_result).unwrap())
                }
                Ok(None) => {
                    let text = format!("未找到记忆: {}", memory_id);
                    let call_result = ToolCallResult {
                        content: vec![TextContent { content_type: "text".into(), text }],
                    };
                    make_response(id, serde_json::to_value(call_result).unwrap())
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
                .min(100) as usize;

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
            };

            let store = state.memory_store.lock().await;
            match store.list_memories(&filter) {
                Ok((memories, total)) => {
                    let mut text = format!(
                        "记忆列表 (共 {} 条，本页 {} 条)\n\n",
                        total,
                        memories.len()
                    );

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
                        content: vec![TextContent { content_type: "text".into(), text }],
                    };
                    make_response(id, serde_json::to_value(call_result).unwrap())
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
                    text.push_str(&format!("- 存储大小: {} bytes\n\n", stats.storage_size_bytes));

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
                        content: vec![TextContent { content_type: "text".into(), text }],
                    };
                    make_response(id, serde_json::to_value(call_result).unwrap())
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
                        content: vec![TextContent { content_type: "text".into(), text }],
                    };
                    make_response(id, serde_json::to_value(call_result).unwrap())
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
                .min(20) as usize;

            let manager = state.manager.lock().await;
            let result = manager.search(query, top_k);

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
                        r.rank, r.chunk.name, r.score * 100.0
                    ));
                    text.push_str(&format!(
                        "`{}:L{}-L{}`\n",
                        r.chunk.file_path, r.chunk.start_line, r.chunk.end_line
                    ));
                    if let Some(ref doc) = r.chunk.doc_comment {
                        text.push_str(&format!("{}\n", doc));
                    }
                    text.push_str(&format!("```{}\n{}\n```\n\n", r.chunk.language, r.chunk.content));
                }
            }

            let call_result = ToolCallResult {
                content: vec![TextContent {
                    content_type: "text".into(),
                    text,
                }],
            };
            make_response(id, serde_json::to_value(call_result).unwrap())
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
            make_response(id, serde_json::to_value(call_result).unwrap())
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
                let err_resp = make_error(
                    None,
                    -32700,
                    &format!("JSON 解析失败: {}", e),
                );
                let _ = stdout
                    .write_all(
                        format!("{}\n", serde_json::to_string(&err_resp).unwrap()).as_bytes(),
                    )
                    .await;
                continue;
            }
        };

        let response =
            dispatch_request(&state, &request.method, request.params.as_ref(), request.id).await;

        // 通知类请求不返回响应，直接跳过
        if let Some(response) = response {
            let json_str = serde_json::to_string(&response).unwrap();
            let _ = stdout.write_all(format!("{}\n", json_str).as_bytes()).await;
            let _ = stdout.flush().await;
        }
    }

    eprintln!("Stdio 流已关闭，MCP 服务退出");
}

// ==================== 路由构建 ====================

/// 创建 MCP 服务的 axum Router
///
/// 可嵌入到已有 axum 应用中，将 MCP 路由挂载到子路径。
pub fn build_mcp_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/mcp", post(mcp_handler))
        .route("/health", axum::routing::get(health_handler))
        .layer(
            tower_http::cors::CorsLayer::permissive(),
        )
        .with_state(state)
}

/// 快速构建并绑定到指定地址
pub async fn serve(state: Arc<AppState>, host: &str, port: u16) -> std::io::Result<()> {
    let app = build_mcp_router(state);

    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("Loong Recall (L-RC) 代码搜索 + 记忆服务");
    println!("   端点: http://{}", addr);
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
        let persistence = crate::persistence::create_json_persistence(&data_dir)
            .expect("持久化创建失败");
        let memory_store = Arc::new(Mutex::new(MemoryStore::new(persistence)));

        Arc::new(AppState {
            manager: Arc::new(Mutex::new(Box::new(manager))),
            memory_store,
            src_dir: "fixture/src".into(),
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
        assert_eq!(tools.len(), 9, "应注册 9 个工具（7 个记忆 + 2 个代码）");

        // 验证记忆工具存在
        let tool_names: Vec<&str> = tools.iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(tool_names.contains(&"remember"), "缺少 remember 工具");
        assert!(tool_names.contains(&"recall"), "缺少 recall 工具");
        assert!(tool_names.contains(&"forget"), "缺少 forget 工具");
        assert!(tool_names.contains(&"update_memory"), "缺少 update_memory 工具");
        assert!(tool_names.contains(&"list_memories"), "缺少 list_memories 工具");
        assert!(tool_names.contains(&"memory_stats"), "缺少 memory_stats 工具");
        assert!(tool_names.contains(&"archive"), "缺少 archive 工具");
        assert!(tool_names.contains(&"search_code"), "缺少 search_code 工具");
        assert!(tool_names.contains(&"codebase_stats"), "缺少 codebase_stats 工具");
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
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(4.into())))
                .await;
        let json = to_json(&resp);

        let text = json["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(json["result"]["content"][0]["type"], "text");
        assert!(
            text.contains("memory"),
            "搜索结果应包含关键词: {}",
            text
        );
        assert!(
            text.contains("src/memory.rs"),
            "应包含文件路径: {}",
            text
        );
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
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(5.into())))
                .await;
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
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(6.into())))
                .await;
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
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(7.into())))
                .await;
        let json = to_json(&resp);

        let text = json["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("已索引文件"), "应包含统计信息: {}", text);
        assert!(text.contains("fn"), "应包含类型分布: {}", text);
    }

    // ---- 错误处理 ----

    #[test]
    fn test_unknown_method() {
        let resp = make_error(Some(serde_json::Value::Number(8.into())), -32601, "未知方法: foo");
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
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(9.into())))
                .await;
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
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(10.into())))
                .await;
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
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(100.into())))
                .await;
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
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(101.into())))
                .await;
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
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(102.into())))
                .await;
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
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(103.into())))
                .await;
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
        let remember_resp =
            handle_tools_call(&state, &remember_params, None).await;
        let remember_json = to_json(&remember_resp);
        // 从响应中提取记忆 ID
        let text = remember_json["result"]["content"][0]["text"].as_str().unwrap();
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
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(104.into())))
                .await;
        let json = to_json(&resp);

        assert!(json["result"].is_object());
        let forget_text = json["result"]["content"][0]["text"].as_str().unwrap();
        assert!(forget_text.contains("已删除"), "应确认删除: {}", forget_text);
    }

    #[tokio::test]
    async fn test_forget_missing_id() {
        let state = test_state();
        let params = serde_json::json!({
            "name": "forget",
            "arguments": {}
        });
        let resp =
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(105.into())))
                .await;
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
        let remember_resp =
            handle_tools_call(&state, &remember_params, None).await;
        let remember_json = to_json(&remember_resp);
        let text = remember_json["result"]["content"][0]["text"].as_str().unwrap();
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
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(106.into())))
                .await;
        let json = to_json(&resp);

        assert!(json["result"].is_object());
        let update_text = json["result"]["content"][0]["text"].as_str().unwrap();
        assert!(update_text.contains("已更新"), "应确认更新: {}", update_text);
        assert!(update_text.contains("新版本内容"), "应包含新内容: {}", update_text);
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
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(107.into())))
                .await;
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
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(108.into())))
                .await;
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
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(109.into())))
                .await;
        let json = to_json(&resp);

        assert!(json["result"].is_object(), "memory_stats 应返回成功结果");
        let stats_text = json["result"]["content"][0]["text"].as_str().unwrap();
        assert!(stats_text.contains("记忆库统计"), "应包含标题: {}", stats_text);
        assert!(stats_text.contains("fact"), "应包含 fact 类型: {}", stats_text);
        assert!(stats_text.contains("preference"), "应包含 preference 类型: {}", stats_text);
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
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(110.into())))
                .await;
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
            handle_tools_call(&state, &params, Some(serde_json::Value::Number(111.into())))
                .await;
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