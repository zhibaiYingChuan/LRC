// ============================================================
// 许可证: Apache 2.0
// 本文件实现记忆图存储，属于公开层 (Layer 1)。
// ============================================================
//
// 记忆图存储（Graph Memory Store）
//
// 图数据库的轻量替代方案 — 在 JSON 持久化层之上实现
// 记忆之间的关系边（contradicts / evolves / synthesizes_from / related_to）。
//
// 后续可平滑迁移到 Neo4j 或其它图数据库。

use crate::persistence::PersistenceError;
use serde::{Deserialize, Serialize};

/// 记忆关系类型
///
/// 洛书图结构中的边类型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EdgeType {
    /// 矛盾关系：两条记忆内容冲突
    Contradicts,
    /// 演进关系：新记忆是旧记忆的更新/演进版本
    Evolves,
    /// 合成来源：合成记忆来源于多条源记忆
    SynthesizesFrom,
    /// 一般关联：语义相关但非直接衍生
    RelatedTo,
}

impl EdgeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Contradicts => "contradicts",
            Self::Evolves => "evolves",
            Self::SynthesizesFrom => "synthesizes_from",
            Self::RelatedTo => "related_to",
        }
    }
}

/// 记忆图边
///
/// 连接两条记忆的有向关系边。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEdge {
    /// 边唯一标识
    pub id: String,
    /// 源记忆 ID
    pub source_id: String,
    /// 目标记忆 ID
    pub target_id: String,
    /// 关系类型
    pub edge_type: EdgeType,
    /// 关系权重（0.0 ~ 1.0，表示关联强度）
    pub weight: f32,
    /// 创建时间戳
    pub created_at: String,
}

impl MemoryEdge {
    /// 创建新的记忆边
    pub fn new(
        source_id: String,
        target_id: String,
        edge_type: EdgeType,
        weight: f32,
    ) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let created_at = chrono::Utc::now().to_rfc3339();
        Self {
            id,
            source_id,
            target_id,
            edge_type,
            weight: weight.clamp(0.0, 1.0),
            created_at,
        }
    }
}

/// 图查询结果
#[derive(Debug, Clone, Default)]
pub struct GraphQueryResult {
    /// 直接关联的记忆 ID 列表
    pub related_ids: Vec<String>,
    /// 演进链（从最旧到最新）
    pub evolution_chain: Vec<String>,
    /// 合成来源（Synthesis → 源记忆）
    pub synthesis_sources: Vec<String>,
    /// 子图大小（关联的记忆总数）
    pub subgraph_size: usize,
}

/// 记忆图存储
///
/// 在持久化层之上管理记忆之间的关系边。
/// 使用 JSON 文件持久化边数据。
pub struct GraphMemoryStore {
    /// 所有关系边
    edges: Vec<MemoryEdge>,
    /// 边持久化文件路径（相对于数据目录）
    edges_file: String,
}

impl GraphMemoryStore {
    /// 创建新的图存储实例
    pub fn new(data_dir: &str) -> Self {
        Self {
            edges: Vec::new(),
            edges_file: format!("{}/graph_edges.json", data_dir),
        }
    }

    /// 从文件加载已有边
    pub fn load(&mut self) -> Result<(), PersistenceError> {
        if let Ok(content) = std::fs::read_to_string(&self.edges_file) {
            if !content.trim().is_empty() {
                self.edges = serde_json::from_str(&content)
                    .map_err(PersistenceError::Serialization)?;
            }
        }
        Ok(())
    }

    /// 持久化边到文件
    pub fn save(&self) -> Result<(), PersistenceError> {
        let json = serde_json::to_string_pretty(&self.edges)
            .map_err(PersistenceError::Serialization)?;
        std::fs::write(&self.edges_file, json)
            .map_err(PersistenceError::Io)?;
        Ok(())
    }

    /// 添加一条关系边
    ///
    /// 自动去重：相同 source_id + target_id + edge_type 的边不会重复添加。
    pub fn add_edge(
        &mut self,
        source_id: &str,
        target_id: &str,
        edge_type: EdgeType,
        weight: f32,
    ) -> Result<(), PersistenceError> {
        // 去重检查
        let exists = self.edges.iter().any(|e| {
            e.source_id == source_id
                && e.target_id == target_id
                && e.edge_type == edge_type
        });

        if !exists {
            let edge = MemoryEdge::new(
                source_id.to_string(),
                target_id.to_string(),
                edge_type,
                weight,
            );
            self.edges.push(edge);
            self.save()?;
        }

        Ok(())
    }

    /// 删除一条边
    pub fn remove_edge(&mut self, edge_id: &str) -> Result<bool, PersistenceError> {
        let len_before = self.edges.len();
        self.edges.retain(|e| e.id != edge_id);
        let removed = self.edges.len() < len_before;
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    /// 查询与指定记忆相关的所有边
    pub fn query_edges(&self, memory_id: &str) -> Vec<&MemoryEdge> {
        self.edges
            .iter()
            .filter(|e| e.source_id == memory_id || e.target_id == memory_id)
            .collect()
    }

    /// 查询完整子图（指定记忆的 1-hop 邻居 + 边类型分布）
    pub fn query_subgraph(&self, memory_id: &str) -> GraphQueryResult {
        let edges = self.query_edges(memory_id);

        let mut related_ids: Vec<String> = Vec::new();
        let mut evolution_chain: Vec<String> = Vec::new();
        let mut synthesis_sources: Vec<String> = Vec::new();

        for e in &edges {
            let other = if e.source_id == memory_id {
                &e.target_id
            } else {
                &e.source_id
            };

            if !related_ids.contains(other) {
                related_ids.push(other.clone());
            }

            match e.edge_type {
                EdgeType::Evolves => {
                    if !evolution_chain.contains(other) {
                        evolution_chain.push(other.clone());
                    }
                }
                EdgeType::SynthesizesFrom => {
                    if !synthesis_sources.contains(other) {
                        synthesis_sources.push(other.clone());
                    }
                }
                _ => {}
            }
        }

        // BFS 获取子图大小
        let mut visited = std::collections::HashSet::new();
        let mut queue: Vec<String> = vec![memory_id.to_string()];
        visited.insert(memory_id.to_string());

        while let Some(current) = queue.pop() {
            for e in &self.edges {
                let neighbor = if e.source_id == current {
                    &e.target_id
                } else if e.target_id == current {
                    &e.source_id
                } else {
                    continue;
                };

                if visited.insert(neighbor.clone()) {
                    queue.push(neighbor.clone());
                }
            }
        }

        GraphQueryResult {
            related_ids,
            evolution_chain,
            synthesis_sources,
            subgraph_size: visited.len(),
        }
    }

    /// 自动建立关系（基于记忆内容相似度和类型）
    ///
    /// 在写入新记忆后调用，自动检测并建立：
    /// - Contradicts: 同类型但内容矛盾
    /// - Evolves: 内容相似 > 0.7
    /// - RelatedTo: 内容相似 > 0.3
    pub fn auto_link(
        &mut self,
        new_memory_id: &str,
        existing_memories: &[crate::Memory],
        similarity_fn: impl Fn(&str, &str) -> f32,
    ) -> Result<usize, PersistenceError> {
        let new_content = existing_memories
            .iter()
            .find(|m| m.id == new_memory_id)
            .map(|m| m.content.as_str())
            .unwrap_or("");

        let mut links_added = 0usize;

        for m in existing_memories {
            if m.id == new_memory_id {
                continue;
            }

            let sim = similarity_fn(new_content, &m.content);

            if sim >= 0.7 {
                // 高相似度 → 演进关系
                self.add_edge(new_memory_id, &m.id, EdgeType::Evolves, sim)?;
                links_added += 1;
            } else if sim >= 0.3 {
                // 中等相似度 → 一般关联
                self.add_edge(new_memory_id, &m.id, EdgeType::RelatedTo, sim)?;
                links_added += 1;
            }
        }

        Ok(links_added)
    }

    /// 获取所有边（用于调试和导出）
    pub fn all_edges(&self) -> &[MemoryEdge] {
        &self.edges
    }

    /// 获取边总数
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// 清空所有边
    pub fn clear(&mut self) -> Result<(), PersistenceError> {
        self.edges.clear();
        self.save()
    }
}

// === 测试 ===

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_graph_store() -> (TempDir, GraphMemoryStore) {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let store = GraphMemoryStore::new(&data_dir);
        (dir, store)
    }

    #[test]
    fn test_add_edge() {
        let (_dir, mut store) = make_graph_store();
        store
            .add_edge("mem-1", "mem-2", EdgeType::RelatedTo, 0.5)
            .expect("应成功添加边");
        assert_eq!(store.edge_count(), 1);
    }

    #[test]
    fn test_deduplicate_edges() {
        let (_dir, mut store) = make_graph_store();
        store.add_edge("a", "b", EdgeType::Evolves, 0.8).unwrap();
        store.add_edge("a", "b", EdgeType::Evolves, 0.9).unwrap(); // 重复
        assert_eq!(store.edge_count(), 1, "不应添加重复边");
    }

    #[test]
    fn test_query_edges() {
        let (_dir, mut store) = make_graph_store();
        store.add_edge("a", "b", EdgeType::Evolves, 0.8).unwrap();
        store.add_edge("a", "c", EdgeType::RelatedTo, 0.3).unwrap();
        store.add_edge("d", "a", EdgeType::Contradicts, 0.1).unwrap();

        let edges = store.query_edges("a");
        assert_eq!(edges.len(), 3, "a 应有 3 条关联边");
    }

    #[test]
    fn test_subgraph() {
        let (_dir, mut store) = make_graph_store();
        store.add_edge("a", "b", EdgeType::Evolves, 0.8).unwrap();
        store.add_edge("b", "c", EdgeType::Evolves, 0.7).unwrap();
        store.add_edge("a", "d", EdgeType::RelatedTo, 0.3).unwrap();

        let result = store.query_subgraph("a");
        assert_eq!(result.subgraph_size, 4, "子图应包含 a,b,c,d");
        assert_eq!(result.related_ids.len(), 2, "a 直接关联 b 和 d");
    }

    #[test]
    fn test_remove_edge() {
        let (_dir, mut store) = make_graph_store();
        store.add_edge("a", "b", EdgeType::Evolves, 0.8).unwrap();
        let edge_id = store.all_edges()[0].id.clone();

        let removed = store.remove_edge(&edge_id).unwrap();
        assert!(removed);
        assert_eq!(store.edge_count(), 0);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let (dir, mut store) = make_graph_store();
        store.add_edge("mem-1", "mem-2", EdgeType::Evolves, 0.85).unwrap();
        store.add_edge("mem-2", "mem-3", EdgeType::SynthesizesFrom, 0.92).unwrap();
        store.save().unwrap();

        // 重新加载
        let data_dir = dir.path().to_string_lossy().to_string();
        let mut store2 = GraphMemoryStore::new(&data_dir);
        store2.load().unwrap();
        assert_eq!(store2.edge_count(), 2);
    }
}