// ============================================================
// 许可证: Apache 2.0
// 本文件实现 Neo4j 图存储后端，属于公开层 (Layer 1)。
// ============================================================
//
// Neo4j 图存储后端
//
// Neo4j 图存储后端：
//   通过递归合成产生的抽象节点及其关系边，
//   支持推理、规划与解释。
//
// 通过 Neo4j HTTP API（Bolt 协议太重量级），
// 使用 reqwest 发送 Cypher 查询。
//
// 图模型：
//   (:Memory {id, content, memory_type, importance, ...})
//   -[:CONTRADICTS]-> (:Memory)
//   -[:EVOLVES]-> (:Memory)
//   -[:SYNTHESIZES_FROM]-> (:Memory)
//   -[:RELATED_TO]-> (:Memory)

use crate::graph_store::{EdgeType, GraphMemoryStore, GraphQueryResult, MemoryEdge};
use crate::persistence::PersistenceError;
use serde::{Deserialize, Serialize};

/// Neo4j 连接配置
#[derive(Debug, Clone)]
pub struct Neo4jConfig {
    /// Neo4j HTTP API 地址（默认 http://localhost:7474）
    pub endpoint: String,
    /// 数据库名称（默认 "neo4j"）
    pub database: String,
    /// 用户名
    pub username: String,
    /// 密码
    pub password: String,
    /// HTTP 超时（秒）
    pub timeout_secs: u64,
}

impl Default for Neo4jConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:7474".to_string(),
            database: "neo4j".to_string(),
            username: "neo4j".to_string(),
            // v0.5.4 修复：不再硬编码密码，必须通过环境变量设置
            password: String::new(),
            timeout_secs: 15,
        }
    }
}

impl Neo4jConfig {
    /// 从环境变量创建配置
    ///
    /// 环境变量：
    /// - `LRC_NEO4J_URL`：Neo4j 服务地址
    /// - `LRC_NEO4J_USER`：用户名
    /// - `LRC_NEO4J_PASS`：密码（**必须设置**，v0.5.4 起不再使用硬编码默认值）
    /// - `LRC_NEO4J_DB`：数据库名
    pub fn from_env() -> Result<Self, String> {
        let endpoint =
            std::env::var("LRC_NEO4J_URL").unwrap_or_else(|_| "http://localhost:7474".to_string());
        let username = std::env::var("LRC_NEO4J_USER").unwrap_or_else(|_| "neo4j".to_string());
        // v0.5.4 修复：密码必须从环境变量读取，不使用硬编码默认值
        let password = match std::env::var("LRC_NEO4J_PASS") {
            Ok(p) if !p.is_empty() => p,
            _ => return Err(
                "LRC_NEO4J_PASS 环境变量未设置或为空，请设置 Neo4j 密码".to_string()
            ),
        };
        let database = std::env::var("LRC_NEO4J_DB").unwrap_or_else(|_| "neo4j".to_string());

        Ok(Self {
            endpoint,
            database,
            username,
            password,
            timeout_secs: 15,
        })
    }
}

// ==================== Neo4j HTTP API 类型 ====================

/// Neo4j 事务提交请求
#[derive(Debug, Serialize)]
struct Neo4jTransactionRequest {
    statements: Vec<Neo4jStatement>,
}

#[derive(Debug, Serialize)]
struct Neo4jStatement {
    statement: String,
    #[serde(default)]
    parameters: serde_json::Value,
}

/// Neo4j 事务提交响应
#[derive(Debug, Deserialize)]
struct Neo4jTransactionResponse {
    #[serde(default)]
    results: Vec<Neo4jStatementResult>,
    #[serde(default)]
    errors: Vec<Neo4jError>,
}

#[derive(Debug, Deserialize)]
struct Neo4jStatementResult {
    #[serde(default)]
    columns: Vec<String>,
    #[serde(default)]
    data: Vec<Neo4jDataRow>,
}

#[derive(Debug, Deserialize)]
struct Neo4jDataRow {
    #[serde(default)]
    row: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct Neo4jError {
    code: String,
    message: String,
}

/// Neo4j 图存储后端
///
/// 通过 HTTP API 与 Neo4j 图数据库通信，
/// 使用 Cypher 查询语言操作记忆节点和关系边。
#[cfg(feature = "neo4j")]
pub struct Neo4jGraphStore {
    config: Neo4jConfig,
    client: reqwest::Client,
    /// 本地 JSON 兜底存储（Neo4j 不可用时使用）
    fallback: std::sync::Mutex<GraphMemoryStore>,
}

#[cfg(feature = "neo4j")]
impl Neo4jGraphStore {
    /// 创建 Neo4j 图存储后端
    pub async fn new(config: Neo4jConfig) -> Result<Self, PersistenceError> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| {
                PersistenceError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("创建 HTTP 客户端失败: {}", e),
                ))
            })?;

        let this = Self {
            config,
            client,
            fallback: std::sync::Mutex::new(GraphMemoryStore::new("./data")),
        };

        // 尝试连接验证
        if let Err(e) = this.ping().await {
            eprintln!("[LRC·Neo4j] 连接验证失败: {}（将使用本地兜底）", e);
        } else {
            this.ensure_constraints().await?;
        }

        Ok(this)
    }

    /// 验证连接
    async fn ping(&self) -> Result<(), String> {
        let url = format!(
            "{}/db/{}/tx/commit",
            self.config.endpoint, self.config.database
        );
        let auth = base64_auth(&self.config.username, &self.config.password);

        let body = Neo4jTransactionRequest {
            statements: vec![Neo4jStatement {
                statement: "RETURN 1 AS ping".to_string(),
                parameters: serde_json::json!({}),
            }],
        };

        let response = self
            .client
            .post(&url)
            .header("Authorization", &auth)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("HTTP 请求失败: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }

        let tx_response: Neo4jTransactionResponse = response
            .json()
            .await
            .map_err(|e| format!("解析响应失败: {}", e))?;

        if !tx_response.errors.is_empty() {
            return Err(format!("Cypher 错误: {}", tx_response.errors[0].message));
        }

        eprintln!("[LRC·Neo4j] 连接验证成功");
        Ok(())
    }

    /// 确保索引和约束存在
    async fn ensure_constraints(&self) -> Result<(), PersistenceError> {
        let statements = vec![
            "CREATE CONSTRAINT lrc_memory_id IF NOT EXISTS FOR (m:Memory) REQUIRE m.id IS UNIQUE"
                .to_string(),
            "CREATE INDEX lrc_memory_type IF NOT EXISTS FOR (m:Memory) ON (m.memory_type)"
                .to_string(),
            "CREATE INDEX lrc_memory_project IF NOT EXISTS FOR (m:Memory) ON (m.project)"
                .to_string(),
        ];

        for stmt in &statements {
            if let Err(e) = self.execute_cypher(stmt, serde_json::json!({})).await {
                // 约束/索引已存在不是错误（IF NOT EXISTS 在某些版本不支持）
                eprintln!("[LRC·Neo4j] 约束创建提示: {}", e);
            }
        }

        Ok(())
    }

    /// 执行 Cypher 查询
    async fn execute_cypher(
        &self,
        cypher: &str,
        params: serde_json::Value,
    ) -> Result<Neo4jTransactionResponse, PersistenceError> {
        let url = format!(
            "{}/db/{}/tx/commit",
            self.config.endpoint, self.config.database
        );
        let auth = base64_auth(&self.config.username, &self.config.password);

        let body = Neo4jTransactionRequest {
            statements: vec![Neo4jStatement {
                statement: cypher.to_string(),
                parameters: params,
            }],
        };

        let response = self
            .client
            .post(&url)
            .header("Authorization", &auth)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                PersistenceError::Io(std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    format!("Neo4j 请求失败: {}", e),
                ))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(PersistenceError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Neo4j 返回 {}: {}", status, body),
            )));
        }

        let tx_response: Neo4jTransactionResponse = response.json().await.map_err(|e| {
            PersistenceError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("解析 Neo4j 响应失败: {}", e),
            ))
        })?;

        if !tx_response.errors.is_empty() {
            return Err(PersistenceError::Other(format!(
                "Cypher 错误: {}",
                tx_response.errors[0].message
            )));
        }

        Ok(tx_response)
    }

    /// 创建记忆节点
    pub async fn create_memory_node(
        &self,
        id: &str,
        content: &str,
        memory_type: &str,
        importance: u8,
        project: Option<&str>,
    ) -> Result<(), PersistenceError> {
        let cypher = "MERGE (m:Memory {id: $id}) \
                       SET m.content = $content, \
                           m.memory_type = $memory_type, \
                           m.importance = $importance, \
                           m.project = $project, \
                           m.updated_at = datetime()";

        let params = serde_json::json!({
            "id": id,
            "content": content,
            "memory_type": memory_type,
            "importance": importance,
            "project": project,
        });

        self.execute_cypher(cypher, params).await?;
        Ok(())
    }

    /// 创建关系边
    pub async fn create_edge(
        &self,
        from_id: &str,
        to_id: &str,
        edge_type: &EdgeType,
    ) -> Result<(), PersistenceError> {
        let relation = match edge_type {
            EdgeType::Contradicts => "CONTRADICTS",
            EdgeType::Evolves => "EVOLVES",
            EdgeType::SynthesizesFrom => "SYNTHESIZES_FROM",
            EdgeType::RelatedTo => "RELATED_TO",
        };

        // v0.5.4 修复：纵深防御 — 校验 relation 名称仅含大写字母和下划线
        // 防止未来新增 EdgeType 变体时引入非预期字符
        if !relation.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
            return Err(PersistenceError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("非法的关系类型名称: {}", relation),
            )));
        }

        let cypher = format!(
            "MATCH (a:Memory {{id: $from_id}}), (b:Memory {{id: $to_id}}) \
             MERGE (a)-[:{}]->(b)",
            relation
        );

        let params = serde_json::json!({
            "from_id": from_id,
            "to_id": to_id,
        });

        self.execute_cypher(&cypher, params).await?;
        Ok(())
    }

    /// 子图查询：获取指定节点的 N 跳邻域
    pub async fn subgraph(
        &self,
        node_id: &str,
        hops: u32,
    ) -> Result<GraphQueryResult, PersistenceError> {
        // v0.5.4 修复：hops 范围校验，防止超大值导致查询性能问题
        if hops == 0 || hops > 10 {
            return Err(PersistenceError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("hops 必须在 1-10 之间，当前值: {}", hops),
            )));
        }

        // 使用 Cypher 可变长度路径查询获取 N 跳子图
        let cypher = format!(
            "MATCH (m:Memory {{id: $node_id}}) \
             OPTIONAL MATCH path = (m)-[*1..{}]-(neighbor:Memory) \
             RETURN m, relationships(path) AS edges, neighbor",
            hops
        );

        let params = serde_json::json!({
            "node_id": node_id,
        });

        match self.execute_cypher(&cypher, params).await {
            Ok(response) => {
                let mut result = GraphQueryResult::default();
                for data_row in &response.results.first().map(|r| &r.data).unwrap_or(&vec![]) {
                    // 提取节点和边信息
                    if let Some(node_val) = data_row.row.get(0) {
                        if let Some(props) = node_val.as_object() {
                            result.nodes.push(props.clone());
                        }
                    }
                    if let Some(edges_val) = data_row.row.get(1) {
                        if let Some(edges_arr) = edges_val.as_array() {
                            for edge in edges_arr {
                                if let Some(edge_obj) = edge.as_object() {
                                    result.edges.push(edge_obj.clone());
                                }
                            }
                        }
                    }
                    if let Some(neighbor_val) = data_row.row.get(2) {
                        if let Some(props) = neighbor_val.as_object() {
                            if !result.nodes.iter().any(|n| {
                                n.get("id") == props.get("id")
                            }) {
                                result.nodes.push(props.clone());
                            }
                        }
                    }
                }
                Ok(result)
            }
            Err(e) => {
                // Neo4j 不可用时回退到本地兜底
                eprintln!("[LRC·Neo4j] 子图查询失败（回退到本地兜底）: {e}");
                if let Ok(guard) = self.fallback.lock() {
                    Ok(guard.query_subgraph(node_id))
                } else {
                    Ok(GraphQueryResult::default())
                }
            }
        }
    }
}

/// Base64 认证头构造
fn base64_auth(username: &str, password: &str) -> String {
    let credentials = format!("{}:{}", username, password);
    format!("Basic {}", base64_encode(credentials.as_bytes()))
}

/// 简易 Base64 编码（不引入额外依赖）
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity((data.len() + 2) / 3 * 4);

    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }

    result
}

/// 当 `neo4j` feature 未启用时的占位类型
#[cfg(not(feature = "neo4j"))]
#[derive(Debug, Clone)]
pub struct Neo4jGraphStore;

#[cfg(not(feature = "neo4j"))]
impl Neo4jGraphStore {
    pub async fn new(_config: Neo4jConfig) -> Result<Self, PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Neo4j 后端未启用，请在编译时启用 `neo4j` feature",
        )))
    }
}
