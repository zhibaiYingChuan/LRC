// ============================================================
// 道体导航信号 — 检索前方向注入（导航层接口）
//
// 架构定位（2026-09-05 预注册实验证实）：
//   道体状态机在 LRC 检索【之前】产出导航信号（目标卦宫序列），
//   LRC 带着方向做多视图检索并 RRF 融合——改变的是候选集本身，
//   而非对已捞结果做后处理重排（后者已被公平实验证伪：+0.7pp、互补率 0）。
//   导航方向显著优于随机方向扩展（+2.3pp，配对 bootstrap P=96.3%）。
//
// 生产者契约：导航信号由 daoti/ 研究资产（DaotiInferenceEngineV23）在
//   查询时推演产生，经 recall/enrich 的 navigation 参数注入产品侧。
//   产品侧不计算、不内置道体引擎（DaoTi Research License），只消费信号。
//   信号缺省 → 行为与既有版本逐字节一致。
// ============================================================

use crate::engine::mirror_trapezoid::bagua_name_to_index;
use crate::memory_store::{RecallFilter, RecallResult};

/// 一次检索允许的最大导航视图数（含基线视图）。
/// 防御失控输入：视图数 × 每视图 top_k 会线性放大检索成本。
const MAX_NAV_VIEWS: usize = 5;

/// 道体导航信号：查询经道体状态机推演后产出的检索方向序列。
#[derive(Debug, Clone)]
pub struct NavigationSignal {
    /// 轨迹卦宫名（如 ["兑","坤"]，按推演先后排列，最多 4 个）。
    /// 每一项代表道体认为"值得探测"的一个语义方向。
    pub palaces: Vec<String>,
    /// 生产者自带的探测词（palaces 同序，每项为该卦宫的代表词组）。
    /// 提供时优先于产品侧 BAGUA_CATEGORIES 回退——精确词典属于研究资产
    /// （DaoTi License），由信号生产者携带穿越接口边界。
    pub probes: Option<Vec<Vec<String>>>,
    /// 产出信号的推演版本（版本不识别时忽略信号，行为回退基线）
    pub source_version: Option<String>,
}

impl NavigationSignal {
    /// 从 JSON（recall/enrich 的 navigation 字段）解析。
    /// 结构：{ "palaces": ["兑", "坤"], "version": "daoti-v23-pilot" }
    /// palaces 中无法识别的卦名静默丢弃；全部无效或缺字段 → None（=无导航）。
    pub fn from_json(v: &serde_json::Value) -> Option<Self> {
        let palaces_raw = v.get("palaces").and_then(|p| p.as_array())?;
        let mut palaces = Vec::new();
        for p in palaces_raw {
            if let Some(name) = p.as_str() {
                // 必须是可映射的八卦名（接受 "乾" 或 "乾·天" 形式）
                if bagua_name_to_index(name).is_some() {
                    palaces.push(name.to_string());
                }
            }
            if palaces.len() >= MAX_NAV_VIEWS - 1 {
                break; // 基线视图占一席，导航视图 ≤ 4
            }
        }
        if palaces.is_empty() {
            return None;
        }
        // 可选 probes：与 palaces 同序的探测词组；palaces 被截断时 probes
        // 取前 N 项对齐，长度不足则整体忽略回退宫义
        let probes = v.get("probes").and_then(|p| p.as_array()).and_then(|arr| {
            if arr.len() < palaces.len() {
                return None;
            }
            let parsed: Option<Vec<Vec<String>>> = arr
                .iter()
                .take(palaces.len())
                .map(|item| {
                    item.as_array().map(|ws| {
                        ws.iter()
                            .filter_map(|w| w.as_str().map(str::to_string))
                            .collect()
                    })
                })
                .collect();
            parsed
        });
        Some(Self {
            palaces,
            probes,
            source_version: v
                .get("version")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string()),
        })
    }

    /// 卦宫的探测词组（检索视图的构造材料）。
    ///
    /// 优先使用生产者随信号携带的 `probes`（精确词典属研究资产，由 daoti
    /// pilot 产出时直接带上）；缺省时回退到卦宫名 + 宫义（`BAGUA_CATEGORIES`，
    /// LRC 自有的先天八卦分类语义，非道体词表）。
    pub fn probe_words(&self) -> Vec<(String, String)> {
        // (palace, query 扩展词) —— 由调用方拼接进视图查询
        self.palaces
            .iter()
            .enumerate()
            .filter_map(|(i, name)| {
                if let Some(words) = self.probes.as_ref().and_then(|p| p.get(i)) {
                    if !words.is_empty() {
                        return Some((name.clone(), words.join(" ")));
                    }
                }
                let idx = bagua_name_to_index(name)? as usize;
                let category = crate::engine::mirror_trapezoid::BAGUA_CATEGORIES[idx];
                Some((name.clone(), category.to_string()))
            })
            .collect()
    }
}

/// 多视图导航检索 + RRF 融合。
///
/// 视图 0：基线查询（原始 query，导航缺席时唯一视图）。
/// 视图 i>0：查询文本拼接第 i 个导航方向词（如 "今晚吃什么 愉悦表达"），
///   以同一 filter 走 deep 检索 —— 语义上等于"朝那个卦宫方向再探测一次"。
///
/// 各视图结果用 RRF（k=60，与 fast/deep 融合同参）聚合：多视图同时召回的
/// 记忆获得跨视图累加分，单视图独有记忆保底 1/(60+rank) —— 这正是导航要
/// 改变【候选集】而非权重的机制表达。
///
/// 返回 None 表示导航未生效（信号为空/解析失败），调用方保持基线行为。
pub fn navigated_deep_recall<P>(
    store: &mut crate::memory_store::MemoryStore<P>,
    query: &str,
    base_filter: &RecallFilter,
    depth: u32,
    signal: &NavigationSignal,
) -> Option<RecallResult>
where
    P: crate::persistence::Persistence,
{
    let probes = signal.probe_words();
    if probes.is_empty() {
        return None;
    }
    // 视图 0：基线
    let base = store
        .trapezoid_focus_recall(query, base_filter, depth)
        .ok()?;
    let mut views: Vec<RecallResult> = vec![base];
    for (_palace, word) in &probes {
        let probe_query = format!("{} {}", query, word);
        // 每视图取适度宽度（与基线 top_k 同量级，跨视图靠 RRF 收敛）
        if let Ok(r) = store.trapezoid_focus_recall(&probe_query, base_filter, depth) {
            views.push(r);
        }
    }
    if views.len() < 2 {
        return None; // 所有探测视图都失败 → 回退基线
    }
    Some(fuse_views_rrf(&views, base_filter.top_k.max(1)))
}

/// N 路视图的 RRF 融合（复用 fusion 键语义：source + 内容指纹）。
fn fuse_views_rrf(views: &[RecallResult], top_k: usize) -> RecallResult {
    const RRF_K: f32 = 60.0;
    let mut fused: std::collections::HashMap<String, f32> = std::collections::HashMap::new();
    let mut key_memory: std::collections::HashMap<String, crate::memory_types::Memory> =
        std::collections::HashMap::new();
    let mut total = 0usize;
    for view in views {
        total = total.max(view.total);
        for (rank, m) in view.memories.iter().enumerate() {
            let source = m.source.as_deref().unwrap_or("");
            let key = format!("{}::{}", source, content_fingerprint(&m.content));
            *fused.entry(key.clone()).or_insert(0.0) += 1.0 / (RRF_K + (rank + 1) as f32);
            key_memory.entry(key).or_insert_with(|| m.clone());
        }
    }
    let mut scored: Vec<(String, f32)> = fused.into_iter().collect();
    // 同分按记忆 id 稳定排序（与 rrf.rs 的确定性一致）
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    let mut memories = Vec::with_capacity(top_k.min(scored.len()));
    let mut scores = Vec::with_capacity(top_k.min(scored.len()));
    for (key, score) in scored.into_iter().take(top_k) {
        if let Some(m) = key_memory.remove(&key) {
            memories.push(m);
            scores.push(score);
        }
    }
    RecallResult {
        memories,
        scores,
        total,
        regression_evidence: std::collections::HashMap::new(),
    }
}

/// 记忆内容指纹（与 rrf.rs 的聚合键算法保持一致）。
fn content_fingerprint(content: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in content.trim().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 信号解析接受合法卦名并丢弃非法项() {
        let v = serde_json::json!({"palaces": ["兑", "坤", "不存在", 3], "version": "daoti-v23"});
        let s = NavigationSignal::from_json(&v).unwrap();
        assert_eq!(s.palaces, vec!["兑".to_string(), "坤".to_string()]);
        assert_eq!(s.source_version.as_deref(), Some("daoti-v23"));
    }

    #[test]
    fn 全非法或缺字段信号返回无导航() {
        assert!(NavigationSignal::from_json(&serde_json::json!({"palaces": ["假"] })).is_none());
        assert!(NavigationSignal::from_json(&serde_json::json!({})).is_none());
        assert!(NavigationSignal::from_json(&serde_json::json!({"palaces": []})).is_none());
    }

    #[test]
    fn 视图数上限保护不误伤合法短信号() {
        let v = serde_json::json!({"palaces": ["乾", "兑", "坤", "艮", "震", "巽"]});
        let s = NavigationSignal::from_json(&v).unwrap();
        assert_eq!(s.palaces.len(), MAX_NAV_VIEWS - 1, "导航视图最多 4");
    }

    #[test]
    fn 方向词产出非空且含宫义() {
        let s = NavigationSignal {
            palaces: vec!["兑".into()],
            probes: None,
            source_version: None,
        };
        let probes = s.probe_words();
        assert_eq!(probes.len(), 1);
        assert_eq!(probes[0].0, "兑");
        assert!(probes[0].1.contains("愉悦") || probes[0].1.len() >= 2);
    }

    #[test]
    fn 生产者probes优先于宫义回退() {
        let v = serde_json::json!({"palaces": ["兑", "坤"], "probes": [["吃", "餐厅"], ["出行", "徒步"]]});
        let s = NavigationSignal::from_json(&v).unwrap();
        let probes = s.probe_words();
        assert_eq!(probes[0].1, "吃 餐厅");
        assert_eq!(probes[1].1, "出行 徒步");
        // probes 长度不匹配 → 忽略，回退宫义
        let bad = serde_json::json!({"palaces": ["兑", "坤"], "probes": [["吃"]]});
        let s2 = NavigationSignal::from_json(&bad).unwrap();
        assert!(s2.probe_words()[0].1.contains("愉悦"));
    }

    #[test]
    fn rrf融合_跨视图共现记忆排名更高() {
        use crate::memory_store::RecallResult;
        use crate::memory_types::{Importance, Memory, MemoryType};
        let mk = |id: &str, content: &str| {
            let mut m = Memory::new(
                content.to_string(),
                MemoryType::Fact,
                Some("t".into()),
                vec![],
                Importance::new(5),
                None,
            );
            m.id = id.to_string();
            m
        };
        // 视图1：A B C；视图2：X A Y —— A 双视图共现，应融合第一
        let v1 = RecallResult {
            memories: vec![mk("a", "共同"), mk("b", "仅一"), mk("c", "仅一2")],
            scores: vec![0.9, 0.8, 0.7],
            total: 3,
            regression_evidence: std::collections::HashMap::new(),
        };
        let v2 = RecallResult {
            memories: vec![mk("x", "新记忆"), mk("a2", "共同"), mk("y", "新记忆2")],
            scores: vec![0.9, 0.85, 0.8],
            total: 3,
            regression_evidence: std::collections::HashMap::new(),
        };
        let fused = fuse_views_rrf(&[v1, v2], 6);
        assert_eq!(fused.memories.len(), 5, "同内容跨 id 聚合为一个桶");
        assert_eq!(fused.memories[0].content, "共同");
        assert_eq!(
            fused.memories[1].content, "新记忆",
            "单视图 rank1 优于双视图 rank3"
        );
    }
}
