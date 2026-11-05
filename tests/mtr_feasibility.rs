//! M.T.R.（镜像梯形递归）作为"传输层"的可行性验证。
//!
//! 三个判据（对应 v3 文档 P3 的前置条件）：
//!   1. **稳定性**：同一段文本多次编码，宫格位置必须一致（确定性）。
//!   2. **分布**：8 个宫格不能严重倾斜（否则桶索引退化）。
//!   3. **召回**：已知因果对（A 导致 B）应落在同一宫格或相邻宫格。
//!
//! 运行：
//!   $env:MTR_CORPUS="temp/v2_matrix/work/corpus_export.json"
//!   cargo test --release --test mtr_feasibility -- --ignored --nocapture

// 实验性可行性验证（全部 `#[ignore]`，不进 CI）：分析脚本风格，不追求 clippy 整洁。
#![allow(clippy::all)]

use code_memory::engine::luoshu_encoder::LuoShuEncoder;
use code_memory::engine::{mirror_project, recursive_unfold, TrapezoidROI};
use std::collections::HashMap;

#[test]
#[ignore]
fn mtr_feasibility() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let enc = LuoShuEncoder::new();

    // ── 1. 稳定性：同一文本编码 10 次，宫格是否一致 ──
    let mut unstable = Vec::new();
    let mut cells: Vec<usize> = Vec::with_capacity(mems.len());
    // ★top-3 宫格：按投影分数取前 3 个（而非单 argmax）
    let mut top3: Vec<Vec<usize>> = Vec::with_capacity(mems.len());
    for m in mems {
        let content = m["content"].as_str().unwrap_or("");
        let key = m["key"].as_str().unwrap_or("?");
        let mut idxs = Vec::with_capacity(10);
        let mut last_scores = [0f32; 8];
        for _ in 0..10 {
            let vec = enc.encode_text(content);
            let proj = mirror_project(&vec);
            idxs.push(proj.best_index);
            last_scores = proj.scores;
        }
        if idxs.iter().any(|x| *x != idxs[0]) {
            unstable.push((key.to_string(), idxs.clone()));
        }
        cells.push(idxs[0]);
        // 按分数降序取前 3
        let mut order: Vec<usize> = (0..8).collect();
        order.sort_by(|&a, &b| {
            last_scores[b]
                .partial_cmp(&last_scores[a])
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        top3.push(order[..3].to_vec());
    }

    // ── 2. 分布：8 宫格计数 ──
    let mut dist = [0usize; 8];
    for c in &cells {
        dist[*c] += 1;
    }
    let max_share = dist.iter().copied().max().unwrap_or(0) as f64 / mems.len() as f64;

    // ── 3. 召回：因果对是否同格 / 相邻格 ──
    let key2idx: HashMap<&str, usize> = mems
        .iter()
        .enumerate()
        .map(|(i, m)| (m["key"].as_str().unwrap_or(""), i))
        .collect();
    let causal = v["causal"].as_array().cloned().unwrap_or_default();
    let mut same_cell = 0usize;
    let mut adjacent = 0usize;
    let mut within2 = 0usize;
    let mut far = Vec::new();
    // ★top-3 召回：两端的 top-3 集合是否有交集
    let mut top3_hit = 0usize;
    let mut top3_miss = Vec::new();
    for pair in &causal {
        let a = pair[0].as_str().unwrap_or("");
        let b = pair[1].as_str().unwrap_or("");
        let (Some(&ia), Some(&ib)) = (key2idx.get(a), key2idx.get(b)) else {
            continue;
        };
        let ca = cells[ia];
        let cb = cells[ib];
        let d = ring_distance(ca, cb);
        if ca == cb {
            same_cell += 1;
        } else if d <= 1 {
            adjacent += 1;
        } else {
            far.push((a.to_string(), b.to_string(), ca, cb));
        }
        if d <= 2 {
            within2 += 1;
        }
        // top-3 交集判定
        let hit = top3[ia].iter().any(|x| top3[ib].contains(x));
        if hit {
            top3_hit += 1;
        } else {
            top3_miss.push((
                a.to_string(),
                b.to_string(),
                top3[ia].clone(),
                top3[ib].clone(),
            ));
        }
    }

    println!("============================================================");
    println!("M.T.R. 可行性验证（语料 {} 条）", mems.len());
    println!("1. 稳定性：不稳定 {}/{} 条", unstable.len(), mems.len());
    for (k, idxs) in unstable.iter().take(5) {
        println!("   [UNSTABLE] {} -> {:?}", k, idxs);
    }
    println!("2. 分布：{:?}（最大占比 {:.1}%）", dist, max_share * 100.0);
    println!(
        "3. 因果对（单格）：同格 {} / 相邻 {} / 远隔 {}（共 {}）",
        same_cell,
        adjacent,
        far.len(),
        causal.len()
    );
    for (a, b, ca, cb) in far.iter().take(5) {
        println!("   [FAR] {} -> {}（宫 {} -> {}）", a, b, ca, cb);
    }
    println!(
        "4. 因果对（top-3 交集）：命中 {} / 未命中 {}（共 {}，召回率 {:.1}%）",
        top3_hit,
        top3_miss.len(),
        causal.len(),
        top3_hit as f64 / causal.len().max(1) as f64 * 100.0
    );
    for (a, b, ta, tb) in top3_miss.iter().take(5) {
        println!("   [TOP3-MISS] {} -> {}（{:?} vs {:?}）", a, b, ta, tb);
    }
    println!(
        "5. 几何邻域召回：距离≤1 = {:.1}%（候选约 3~5 格），距离≤2 = {:.1}%（候选约 6~9 格）",
        (same_cell + adjacent) as f64 / causal.len().max(1) as f64 * 100.0,
        within2 as f64 / causal.len().max(1) as f64 * 100.0
    );

    // ── 6. 81 格（九宫拓维）：cell = argmax(全文)，sub = argmax(中段窗口) ──
    let mut cell81: Vec<(usize, usize)> = Vec::with_capacity(mems.len());
    for m in mems {
        let content = m["content"].as_str().unwrap_or("");
        let whole = enc.encode_text(content);
        let cell = mirror_project(&whole).best_index;
        let mid = mid_window(content);
        let sub = mirror_project(&enc.encode_text(&mid)).best_index;
        cell81.push((cell, sub));
    }
    let mut same81 = 0usize;
    let mut adj81 = 0usize;
    let mut far81 = Vec::new();
    for pair in &causal {
        let a = pair[0].as_str().unwrap_or("");
        let b = pair[1].as_str().unwrap_or("");
        let (Some(&ia), Some(&ib)) = (key2idx.get(a), key2idx.get(b)) else {
            continue;
        };
        let (ca, sa) = cell81[ia];
        let (cb, sb) = cell81[ib];
        if ca == cb && sa == sb {
            same81 += 1;
        } else if (ca == cb && sa.abs_diff(sb) <= 1) || (ca.abs_diff(cb) <= 1 && sa == sb) {
            adj81 += 1;
        } else {
            far81.push((a.to_string(), b.to_string(), (ca, sa), (cb, sb)));
        }
    }
    println!(
        "6. 81 格（cell=全文宫, sub=中段宫）：同格 {} / 相邻 {} / 远隔 {}（共 {}，召回率 {:.1}%）",
        same81,
        adj81,
        far81.len(),
        causal.len(),
        (same81 + adj81) as f64 / causal.len().max(1) as f64 * 100.0
    );
    for (a, b, x, y) in far81.iter().take(5) {
        println!("   [81-FAR] {} -> {}（{:?} vs {:?}）", a, b, x, y);
    }
    println!("============================================================");

    // 判据（先记录，不强制失败——这是可行性观察，不是验收）
    let stable_ok = unstable.is_empty();
    let dist_ok = max_share <= 0.5;
    let recall_ok = (same_cell + adjacent) as f64 / causal.len().max(1) as f64 >= 0.5;
    let top3_ok = top3_hit as f64 / causal.len().max(1) as f64 >= 0.9;
    println!(
        "判据：稳定性={} 分布={} 单格召回={} top-3召回={}",
        if stable_ok { "通过" } else { "不通过" },
        if dist_ok { "通过" } else { "不通过" },
        if recall_ok { "通过" } else { "不通过" },
        if top3_ok { "通过" } else { "不通过" }
    );
}

/// 九宫格环形距离（0..8 按 3x3 网格的曼哈顿距离）。
fn ring_distance(a: usize, b: usize) -> usize {
    let (ar, ac) = (a / 3, a % 3);
    let (br, bc) = (b / 3, b % 3);
    ar.abs_diff(br) + ac.abs_diff(bc)
}

/// 取文本中段 160 字（短文本原样返回），用于 81 格的第二级坐标。
fn mid_window(content: &str) -> String {
    let chars: Vec<char> = content.chars().collect();
    if chars.len() <= 160 {
        return content.to_string();
    }
    let start = (chars.len() - 160) / 2;
    chars[start..start + 160].iter().collect()
}

// ============================================================
// 64 卦实验：上卦 = 全文八卦，下卦 = 候选独立特征
// ============================================================

/// 八卦索引 → 3 位爻（与 `BAGUA_BASES` 顺序一致）。
const TRIGRAM_BITS: [u8; 8] = [
    0b111, // 乾
    0b110, // 兑
    0b101, // 离
    0b100, // 震
    0b011, // 巽
    0b010, // 坎
    0b001, // 艮
    0b000, // 坤
];

fn hamming(a: u8, b: u8) -> u32 {
    (a ^ b).count_ones()
}

/// 候选下卦特征：把文本变换成另一种字符分布，再交给同一个编码器。
fn lower_feature(kind: &str, content: &str) -> String {
    match kind {
        // ASCII 标识符（文件名/数字/URL/英文）
        "ascii" => {
            let s: String = content
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || "/._-:".contains(*c))
                .collect();
            if s.trim().is_empty() {
                "无".to_string()
            } else {
                s
            }
        }
        // 仅 CJK 字符
        "cjk" => {
            let s: String = content
                .chars()
                .filter(|c| {
                    let u = *c as u32;
                    (0x4E00..=0x9FFF).contains(&u)
                })
                .collect();
            if s.trim().is_empty() {
                "无".to_string()
            } else {
                s
            }
        }
        // 仅标点/符号
        "punct" => {
            let s: String = content
                .chars()
                .filter(|c| c.is_ascii_punctuation() || "，。！？；：、（）【】「」".contains(*c))
                .collect();
            if s.trim().is_empty() {
                "无".to_string()
            } else {
                s
            }
        }
        // 字符逆序（改变 bigram 统计）
        "reverse" => content.chars().rev().collect(),
        _ => content.to_string(),
    }
}

#[test]
#[ignore]
fn mtr_hexagram_64() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let enc = LuoShuEncoder::new();
    let key2idx: HashMap<&str, usize> = mems
        .iter()
        .enumerate()
        .map(|(i, m)| (m["key"].as_str().unwrap_or(""), i))
        .collect();
    let causal = v["causal"].as_array().cloned().unwrap_or_default();

    // 上卦：全文八卦（已有）
    let upper: Vec<usize> = mems
        .iter()
        .map(|m| {
            let content = m["content"].as_str().unwrap_or("");
            mirror_project(&enc.encode_text(content)).best_index
        })
        .collect();

    println!("============================================================");
    println!(
        "64 卦实验（语料 {} 条，因果对 {} 对）",
        mems.len(),
        causal.len()
    );
    for kind in ["ascii", "cjk", "punct", "reverse"] {
        // 下卦：候选特征
        let lower: Vec<usize> = mems
            .iter()
            .map(|m| {
                let content = m["content"].as_str().unwrap_or("");
                let feat = lower_feature(kind, content);
                mirror_project(&enc.encode_text(&feat)).best_index
            })
            .collect();
        // 统计下卦与上卦相同的比例（独立性指标：越低越独立）
        let same_as_upper = upper
            .iter()
            .zip(lower.iter())
            .filter(|(u, l)| u == l)
            .count();
        // 因果对：同卦 / 变爻（Hamming≤1）/ 远隔
        let mut same = 0usize;
        let mut one_yao = 0usize;
        let mut far = 0usize;
        for pair in &causal {
            let a = pair[0].as_str().unwrap_or("");
            let b = pair[1].as_str().unwrap_or("");
            let (Some(&ia), Some(&ib)) = (key2idx.get(a), key2idx.get(b)) else {
                continue;
            };
            let ha = (TRIGRAM_BITS[upper[ia]] << 3) | TRIGRAM_BITS[lower[ia]];
            let hb = (TRIGRAM_BITS[upper[ib]] << 3) | TRIGRAM_BITS[lower[ib]];
            match hamming(ha, hb) {
                0 => same += 1,
                1 => one_yao += 1,
                _ => far += 1,
            }
        }
        println!(
            "下卦={:<8} 与上卦同率={:.1}% | 同卦 {} / 变爻 {} / 远隔 {} ⇒ 召回 {:.1}%",
            kind,
            same_as_upper as f64 / mems.len() as f64 * 100.0,
            same,
            one_yao,
            far,
            (same + one_yao) as f64 / causal.len().max(1) as f64 * 100.0
        );
    }
    println!("============================================================");
}

// ============================================================
// 召回路径实验：查询 → 目标记忆，M.T.R. 能否做候选预筛
// ============================================================

#[test]
#[ignore]
fn mtr_recall_path() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let queries = v["queries"].as_array().expect("queries 数组");
    let enc = LuoShuEncoder::new();

    // 每条记忆的宫格
    let mem_cell: Vec<usize> = mems
        .iter()
        .map(|m| {
            let content = m["content"].as_str().unwrap_or("");
            mirror_project(&enc.encode_text(content)).best_index
        })
        .collect();
    let key2idx: HashMap<&str, usize> = mems
        .iter()
        .enumerate()
        .map(|(i, m)| (m["key"].as_str().unwrap_or(""), i))
        .collect();

    // 每个查询的宫格 + 目标宫格
    let mut same = 0usize;
    let mut adjacent = 0usize;
    let mut far = Vec::new();
    let mut within2 = 0usize;
    for q in queries {
        let query = q["query"].as_str().unwrap_or("");
        let target = q["target"].as_str().unwrap_or("");
        let qcell = mirror_project(&enc.encode_text(query)).best_index;
        let Some(&ti) = key2idx.get(target) else {
            continue;
        };
        let tcell = mem_cell[ti];
        let d = ring_distance(qcell, tcell);
        if d == 0 {
            same += 1;
        } else if d == 1 {
            adjacent += 1;
        } else {
            far.push((q["qid"].as_str().unwrap_or("?").to_string(), qcell, tcell));
        }
        if d <= 2 {
            within2 += 1;
        }
    }
    let n = queries.len().max(1);
    println!("============================================================");
    println!("召回路径实验（{} 查询）", queries.len());
    println!(
        "查询宫 vs 目标宫：同格 {} / 相邻 {} / 远隔 {} ⇒ 距离≤1 召回 {:.1}%，距离≤2 召回 {:.1}%",
        same,
        adjacent,
        far.len(),
        (same + adjacent) as f64 / n as f64 * 100.0,
        within2 as f64 / n as f64 * 100.0
    );
    for (qid, qc, tc) in far.iter().take(8) {
        println!("   [FAR] {} 查询宫 {} -> 目标宫 {}", qid, qc, tc);
    }
    // 候选集大小：距离≤1 的宫格数（按 3x3 网格，中心 5 格、边 4 格、角 3 格）
    println!("候选集大小：距离≤1 约 3~5 格（33%~56%），距离≤2 约 6~9 格（67%~100%）");
    println!("============================================================");
}

// ============================================================
// 先天八卦（河图）召回实验
// ============================================================

/// 先天八卦在 3×3 网格中的位置（沿用后天位置索引 0..8）：
/// 乾=1(南) 兑=0(东南) 离=3(东) 震=6(东北) 巽=2(西南) 坎=5(西) 艮=8(西北) 坤=7(北)
const XIANTIAN_POS: [usize; 8] = [1, 0, 3, 6, 2, 5, 8, 7];

/// 先天"对待"（对宫）映射：乾↔坤、兑↔艮、离↔坎、震↔巽。
fn xiantian_opposite(pos: usize) -> usize {
    match pos {
        1 => 7,
        7 => 1,
        0 => 8,
        8 => 0,
        3 => 5,
        5 => 3,
        6 => 2,
        2 => 6,
        _ => 4,
    }
}

/// 先天八卦投影：与 `mirror_project` 同构，只是基底位置换成先天方位。
fn mirror_project_xiantian(values: &[f32; 9]) -> usize {
    let total: f32 = values.iter().sum();
    let mut best = 0usize;
    let mut best_score = f32::NEG_INFINITY;
    for (i, &pos) in XIANTIAN_POS.iter().enumerate() {
        let score = 0.9 * values[pos] + 0.1 * (total - values[pos]);
        if score > best_score {
            best_score = score;
            best = i;
        }
    }
    best
}

#[test]
#[ignore]
fn mtr_recall_xiantian() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let queries = v["queries"].as_array().expect("queries 数组");
    let enc = LuoShuEncoder::new();
    let key2idx: HashMap<&str, usize> = mems
        .iter()
        .enumerate()
        .map(|(i, m)| (m["key"].as_str().unwrap_or(""), i))
        .collect();

    // 每条记忆：后天宫格（argmax 位置）+ 先天宫格（先天投影）
    let mut mem_houtian: Vec<usize> = Vec::with_capacity(mems.len());
    let mut mem_xiantian: Vec<usize> = Vec::with_capacity(mems.len());
    for m in mems {
        let content = m["content"].as_str().unwrap_or("");
        let vec = enc.encode_text(content);
        mem_houtian.push(mirror_project(&vec).best_index);
        mem_xiantian.push(mirror_project_xiantian(&vec.values));
    }

    let mut same = 0usize;
    let mut adjacent = 0usize;
    let mut opposite = 0usize;
    let mut far = 0usize;
    let mut houtian_same = 0usize;
    let mut houtian_adj = 0usize;
    for q in queries {
        let query = q["query"].as_str().unwrap_or("");
        let target = q["target"].as_str().unwrap_or("");
        let qvec = enc.encode_text(query);
        let qx = mirror_project_xiantian(&qvec.values);
        let qh = mirror_project(&qvec).best_index;
        let Some(&ti) = key2idx.get(target) else {
            continue;
        };
        let tx = mem_xiantian[ti];
        let th = mem_houtian[ti];
        // 后天基线
        let dh = ring_distance(qh, th);
        if dh == 0 {
            houtian_same += 1;
        } else if dh == 1 {
            houtian_adj += 1;
        }
        // 先天：同宫 / 相邻（几何）/ 对待（对宫）
        if qx == tx {
            same += 1;
        } else if ring_distance(qx, tx) == 1 {
            adjacent += 1;
        } else if xiantian_opposite(qx) == tx {
            opposite += 1;
        } else {
            far += 1;
        }
    }
    let n = queries.len().max(1);
    println!("============================================================");
    println!("先天八卦（河图）召回实验（{} 查询）", queries.len());
    println!(
        "后天基线：同格 {} / 相邻 {} ⇒ 距离≤1 召回 {:.1}%",
        houtian_same,
        houtian_adj,
        (houtian_same + houtian_adj) as f64 / n as f64 * 100.0
    );
    println!(
        "先天：同宫 {} / 相邻 {} / 对待 {} / 远隔 {} ⇒ 同+邻+对待 召回 {:.1}%",
        same,
        adjacent,
        opposite,
        far,
        (same + adjacent + opposite) as f64 / n as f64 * 100.0
    );
    println!("============================================================");
}

// ============================================================
// 传输链确认实验：两条链路（后天/先天）坐标一致 ⇒ 无数据丢失
// ============================================================

/// 后天八卦在 3×3 网格中的位置（与 `BAGUA_BASES` 顺序一致）。
const HOUTIAN_POS: [usize; 8] = [8, 5, 1, 3, 0, 7, 6, 2];

fn houtian_position(vec: &code_memory::engine::luoshu_encoder::LuoShuVector) -> usize {
    HOUTIAN_POS[mirror_project(vec).best_index]
}

fn xiantian_position(vec: &code_memory::engine::luoshu_encoder::LuoShuVector) -> usize {
    XIANTIAN_POS[mirror_project_xiantian(&vec.values)]
}

#[test]
#[ignore]
fn mtr_transport_confirm() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let queries = v["queries"].as_array().expect("queries 数组");
    let enc = LuoShuEncoder::new();
    let key2idx: HashMap<&str, usize> = mems
        .iter()
        .enumerate()
        .map(|(i, m)| (m["key"].as_str().unwrap_or(""), i))
        .collect();

    // 每条记忆：两条链路的坐标 + 是否一致 + 展开保真度
    let mut mem_pos: Vec<usize> = Vec::with_capacity(mems.len());
    let mut mem_agree: Vec<bool> = Vec::with_capacity(mems.len());
    let mut mem_fidelity: Vec<f32> = Vec::with_capacity(mems.len());
    for m in mems {
        let content = m["content"].as_str().unwrap_or("");
        let vec = enc.encode_text(content);
        let hp = houtian_position(&vec);
        let xp = xiantian_position(&vec);
        mem_pos.push(hp);
        mem_agree.push(hp == xp);
        mem_fidelity.push(recursive_unfold(&vec, 0.01).fidelity);
    }
    let agree_count = mem_agree.iter().filter(|x| **x).count();
    let avg_fid: f32 = mem_fidelity.iter().sum::<f32>() / mems.len().max(1) as f32;

    // 查询-目标对：按"两端是否都确认"分组，比较召回
    let mut conf_total = 0usize;
    let mut conf_hit = 0usize;
    let mut unconf_total = 0usize;
    let mut unconf_hit = 0usize;
    for q in queries {
        let query = q["query"].as_str().unwrap_or("");
        let target = q["target"].as_str().unwrap_or("");
        let qvec = enc.encode_text(query);
        let qp = houtian_position(&qvec);
        let q_agree = qp == xiantian_position(&qvec);
        let Some(&ti) = key2idx.get(target) else {
            continue;
        };
        let tp = mem_pos[ti];
        let t_agree = mem_agree[ti];
        let hit = ring_distance(qp, tp) <= 1;
        if q_agree && t_agree {
            conf_total += 1;
            if hit {
                conf_hit += 1;
            }
        } else {
            unconf_total += 1;
            if hit {
                unconf_hit += 1;
            }
        }
    }

    println!("============================================================");
    println!(
        "传输链确认实验（{} 记忆，{} 查询）",
        mems.len(),
        queries.len()
    );
    println!(
        "两条链路坐标一致：{}/{}（{:.1}%）；展开平均保真度 {:.3}",
        agree_count,
        mems.len(),
        agree_count as f64 / mems.len().max(1) as f64 * 100.0,
        avg_fid
    );
    println!(
        "两端都确认：{} 对，召回 {:.1}%（{} 命中）",
        conf_total,
        conf_hit as f64 / conf_total.max(1) as f64 * 100.0,
        conf_hit
    );
    println!(
        "至少一端未确认：{} 对，召回 {:.1}%（{} 命中）",
        unconf_total,
        unconf_hit as f64 / unconf_total.max(1) as f64 * 100.0,
        unconf_hit
    );
    println!("============================================================");
}

// ============================================================
// 去五行偏置实验：纯几何坐标 vs 含偏置坐标
// ============================================================

/// 纯几何坐标：直接取 9 维向量的 argmax（无五行偏置）。
fn pure_position(values: &[f32; 9]) -> usize {
    let mut best = 0usize;
    for i in 1..9 {
        if values[i] > values[best] {
            best = i;
        }
    }
    best
}

#[test]
#[ignore]
fn mtr_no_bias_confirm() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let queries = v["queries"].as_array().expect("queries 数组");
    let enc = LuoShuEncoder::new();
    let key2idx: HashMap<&str, usize> = mems
        .iter()
        .enumerate()
        .map(|(i, m)| (m["key"].as_str().unwrap_or(""), i))
        .collect();

    // 每条记忆：纯几何坐标 + 含偏置坐标
    let mut mem_pure: Vec<usize> = Vec::with_capacity(mems.len());
    let mut mem_biased: Vec<usize> = Vec::with_capacity(mems.len());
    for m in mems {
        let content = m["content"].as_str().unwrap_or("");
        let vec = enc.encode_text(content);
        mem_pure.push(pure_position(&vec.values));
        mem_biased.push(houtian_position(&vec));
    }
    let agree = mem_pure
        .iter()
        .zip(mem_biased.iter())
        .filter(|(a, b)| a == b)
        .count();

    // 召回：纯几何坐标下的 同格 / 相邻 / 对待
    let mut same = 0usize;
    let mut adjacent = 0usize;
    let mut opposite = 0usize;
    let mut far = 0usize;
    for q in queries {
        let query = q["query"].as_str().unwrap_or("");
        let target = q["target"].as_str().unwrap_or("");
        let qvec = enc.encode_text(query);
        let qp = pure_position(&qvec.values);
        let Some(&ti) = key2idx.get(target) else {
            continue;
        };
        let tp = mem_pure[ti];
        if qp == tp {
            same += 1;
        } else if ring_distance(qp, tp) == 1 {
            adjacent += 1;
        } else if xiantian_opposite(qp) == tp {
            opposite += 1;
        } else {
            far += 1;
        }
    }
    let n = queries.len().max(1);
    println!("============================================================");
    println!(
        "去五行偏置实验（{} 记忆，{} 查询）",
        mems.len(),
        queries.len()
    );
    println!(
        "纯几何坐标 vs 含偏置坐标 一致：{}/{}（{:.1}%）",
        agree,
        mems.len(),
        agree as f64 / mems.len().max(1) as f64 * 100.0
    );
    println!(
        "纯几何召回：同格 {} / 相邻 {} / 对待 {} / 远隔 {}",
        same, adjacent, opposite, far
    );
    println!(
        "  同+邻 = {:.1}%；同+邻+对待 = {:.1}%",
        (same + adjacent) as f64 / n as f64 * 100.0,
        (same + adjacent + opposite) as f64 / n as f64 * 100.0
    );
    println!("============================================================");
}

// ============================================================
// 静态 vs 动态检验：河图卦 → 洛书卦 是否是一一映射
// ============================================================

/// 河图（先天）在 3×3 网格各位置的卦名（位置 0..8，4=中）。
const XIANTIAN_TRIGRAM: [&str; 9] = [
    "兑", "乾", "巽", // 0 东南, 1 南, 2 西南
    "离", "中", "坎", // 3 东, 4 中, 5 西
    "震", "坤", "艮", // 6 东北, 7 北, 8 西北
];

/// 洛书（后天）在 3×3 网格各位置的卦名（位置 0..8，4=中）。
const HOUTIAN_TRIGRAM: [&str; 9] = [
    "巽", "离", "坤", // 0 东南, 1 南, 2 西南
    "震", "中", "兑", // 3 东, 4 中, 5 西
    "艮", "坎", "乾", // 6 东北, 7 北, 8 西北
];

#[test]
#[ignore]
fn mtr_static_vs_dynamic() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let enc = LuoShuEncoder::new();

    // 每条记忆：河图卦（纯几何位置）→ 洛书卦（含偏置位置）
    let mut map: HashMap<&str, HashMap<&str, usize>> = HashMap::new();
    for m in mems {
        let content = m["content"].as_str().unwrap_or("");
        let vec = enc.encode_text(content);
        let hp = pure_position(&vec.values);
        let lp = houtian_position(&vec);
        let ht = XIANTIAN_TRIGRAM[hp];
        let lt = HOUTIAN_TRIGRAM[lp];
        *map.entry(ht).or_default().entry(lt).or_insert(0) += 1;
    }

    println!("============================================================");
    println!("静态 vs 动态检验（{} 记忆）", mems.len());
    println!("河图卦 → 洛书卦 的映射分布：");
    let mut static_ok = 0usize;
    let mut dynamic = 0usize;
    let mut keys: Vec<&&str> = map.keys().collect();
    keys.sort();
    for k in keys {
        let targets = &map[k];
        let total: usize = targets.values().sum();
        let mut items: Vec<(&str, usize)> = targets.iter().map(|(a, b)| (*a, *b)).collect();
        items.sort_by(|a, b| b.1.cmp(&a.1));
        let desc: Vec<String> = items.iter().map(|(t, c)| format!("{}×{}", t, c)).collect();
        let is_static = targets.len() == 1;
        if is_static {
            static_ok += 1;
        } else {
            dynamic += 1;
        }
        println!(
            "  河图[{}]（{} 条）→ {} 种洛书卦：{}",
            k,
            total,
            targets.len(),
            desc.join(", ")
        );
    }
    println!(
        "结论：{} 个河图卦中，{} 个是静态一一映射，{} 个是动态多映射",
        map.len(),
        static_ok,
        dynamic
    );
    println!("============================================================");
}

// ============================================================
// 扇出覆盖因果对实验：河图卦 → 2~3 个洛书卦，能否覆盖因果对
// ============================================================

#[test]
#[ignore]
fn mtr_fanout_causal() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let causal = v["causal"].as_array().cloned().unwrap_or_default();
    let enc = LuoShuEncoder::new();
    let key2idx: HashMap<&str, usize> = mems
        .iter()
        .enumerate()
        .map(|(i, m)| (m["key"].as_str().unwrap_or(""), i))
        .collect();

    // 每条记忆：河图卦（纯几何）+ 洛书卦（含偏置）
    let mut mem_ht: Vec<&str> = Vec::with_capacity(mems.len());
    let mut mem_lt: Vec<&str> = Vec::with_capacity(mems.len());
    for m in mems {
        let content = m["content"].as_str().unwrap_or("");
        let vec = enc.encode_text(content);
        mem_ht.push(XIANTIAN_TRIGRAM[pure_position(&vec.values)]);
        mem_lt.push(HOUTIAN_TRIGRAM[houtian_position(&vec)]);
    }

    // 扇出：河图卦 → 洛书卦集合
    let mut fanout: HashMap<&str, std::collections::HashSet<&str>> = HashMap::new();
    for i in 0..mems.len() {
        fanout.entry(mem_ht[i]).or_default().insert(mem_lt[i]);
    }
    let avg_fanout: f64 =
        fanout.values().map(|s| s.len() as f64).sum::<f64>() / fanout.len().max(1) as f64;

    // 因果对：B 的洛书卦是否在 A 的河图卦扇出内
    let mut causal_hit = 0usize;
    let mut causal_same_ht = 0usize;
    let mut causal_total = 0usize;
    for pair in &causal {
        let a = pair[0].as_str().unwrap_or("");
        let b = pair[1].as_str().unwrap_or("");
        let (Some(&ia), Some(&ib)) = (key2idx.get(a), key2idx.get(b)) else {
            continue;
        };
        causal_total += 1;
        if mem_ht[ia] == mem_ht[ib] {
            causal_same_ht += 1;
        }
        if fanout
            .get(mem_ht[ia])
            .is_some_and(|s| s.contains(mem_lt[ib]))
        {
            causal_hit += 1;
        }
    }

    // 随机对基线：随机取 A、B，B 的洛书卦是否在 A 的河图卦扇出内
    let mut rng_state: u64 = 20261104;
    let mut rand_hit = 0usize;
    let mut rand_same_ht = 0usize;
    let rand_total = 2000usize;
    for _ in 0..rand_total {
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let ia = (rng_state >> 33) as usize % mems.len();
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let ib = (rng_state >> 33) as usize % mems.len();
        if mem_ht[ia] == mem_ht[ib] {
            rand_same_ht += 1;
        }
        if fanout
            .get(mem_ht[ia])
            .is_some_and(|s| s.contains(mem_lt[ib]))
        {
            rand_hit += 1;
        }
    }

    println!("============================================================");
    println!(
        "扇出覆盖因果对实验（{} 记忆，{} 因果对）",
        mems.len(),
        causal_total
    );
    println!("平均扇出：{:.2} 个洛书卦 / 河图卦", avg_fanout);
    println!(
        "因果对：扇出命中 {}/{}（{:.1}%）；两端同河图卦 {}/{}（{:.1}%）",
        causal_hit,
        causal_total,
        causal_hit as f64 / causal_total.max(1) as f64 * 100.0,
        causal_same_ht,
        causal_total,
        causal_same_ht as f64 / causal_total.max(1) as f64 * 100.0
    );
    println!(
        "随机对：扇出命中 {}/{}（{:.1}%）；两端同河图卦 {}/{}（{:.1}%）",
        rand_hit,
        rand_total,
        rand_hit as f64 / rand_total as f64 * 100.0,
        rand_same_ht,
        rand_total,
        rand_same_ht as f64 / rand_total as f64 * 100.0
    );
    println!("============================================================");
}

// ============================================================
// 最小八卦落点实验：河图方位（身份通道）→ 洛书落点（位置通道）
// ============================================================

#[test]
#[ignore]
fn mtr_bagua_landing() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let causal = v["causal"].as_array().cloned().unwrap_or_default();
    let enc = LuoShuEncoder::new();
    let key2idx: HashMap<&str, usize> = mems
        .iter()
        .enumerate()
        .map(|(i, m)| (m["key"].as_str().unwrap_or(""), i))
        .collect();

    // 河图方位 = 身份通道 argmax（先天标签）；洛书落点 = 位置通道 argmax（后天标签）
    let mut mem_ht: Vec<&str> = Vec::with_capacity(mems.len());
    let mut mem_lt: Vec<&str> = Vec::with_capacity(mems.len());
    let mut lt_dist: HashMap<&str, usize> = HashMap::new();
    let mut ht_dist: HashMap<&str, usize> = HashMap::new();
    for m in mems {
        let content = m["content"].as_str().unwrap_or("");
        let (positional, identity) = enc.encode_channels(content);
        let hp = pure_position(&identity);
        let lp = pure_position(&positional);
        let ht = XIANTIAN_TRIGRAM[hp];
        let lt = HOUTIAN_TRIGRAM[lp];
        mem_ht.push(ht);
        mem_lt.push(lt);
        *ht_dist.entry(ht).or_insert(0) += 1;
        *lt_dist.entry(lt).or_insert(0) += 1;
    }

    // 因果对：两端是否同河图方位 / 同洛书落点
    let mut causal_total = 0usize;
    let mut same_ht = 0usize;
    let mut same_lt = 0usize;
    for pair in &causal {
        let a = pair[0].as_str().unwrap_or("");
        let b = pair[1].as_str().unwrap_or("");
        let (Some(&ia), Some(&ib)) = (key2idx.get(a), key2idx.get(b)) else {
            continue;
        };
        causal_total += 1;
        if mem_ht[ia] == mem_ht[ib] {
            same_ht += 1;
        }
        if mem_lt[ia] == mem_lt[ib] {
            same_lt += 1;
        }
    }

    // 随机基线
    let mut rng_state: u64 = 20261104;
    let mut rand_same_ht = 0usize;
    let mut rand_same_lt = 0usize;
    let rand_total = 2000usize;
    for _ in 0..rand_total {
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let ia = (rng_state >> 33) as usize % mems.len();
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let ib = (rng_state >> 33) as usize % mems.len();
        if mem_ht[ia] == mem_ht[ib] {
            rand_same_ht += 1;
        }
        if mem_lt[ia] == mem_lt[ib] {
            rand_same_lt += 1;
        }
    }

    println!("============================================================");
    println!(
        "最小八卦落点实验（{} 记忆，{} 因果对）",
        mems.len(),
        causal_total
    );
    let mut ht_items: Vec<(&str, usize)> = ht_dist.iter().map(|(a, b)| (*a, *b)).collect();
    ht_items.sort_by(|a, b| b.1.cmp(&a.1));
    println!(
        "河图方位分布（身份通道）：{}",
        ht_items
            .iter()
            .map(|(k, c)| format!("{}×{}", k, c))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut lt_items: Vec<(&str, usize)> = lt_dist.iter().map(|(a, b)| (*a, *b)).collect();
    lt_items.sort_by(|a, b| b.1.cmp(&a.1));
    println!(
        "洛书落点分布（位置通道）：{}",
        lt_items
            .iter()
            .map(|(k, c)| format!("{}×{}", k, c))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "因果对：同河图方位 {}/{}（{:.1}%）；同洛书落点 {}/{}（{:.1}%）",
        same_ht,
        causal_total,
        same_ht as f64 / causal_total.max(1) as f64 * 100.0,
        same_lt,
        causal_total,
        same_lt as f64 / causal_total.max(1) as f64 * 100.0
    );
    println!(
        "随机对：同河图方位 {:.1}%；同洛书落点 {:.1}%",
        rand_same_ht as f64 / rand_total as f64 * 100.0,
        rand_same_lt as f64 / rand_total as f64 * 100.0
    );
    println!("============================================================");
}

// ============================================================
// 去中心 + 五行生克实验：中心为恒定土，落点只在八卦
// ============================================================

/// 排除中心（位置 4）后的 argmax：落点只在八卦方位。
fn position_excluding_center(values: &[f32; 9]) -> usize {
    let mut best = usize::MAX;
    let mut best_val = f32::NEG_INFINITY;
    for i in 0..9 {
        if i == 4 {
            continue; // 中心是恒定点，不参与落点
        }
        if values[i] > best_val {
            best_val = values[i];
            best = i;
        }
    }
    best
}

/// 位置 → 五行（洛书方位）。
fn wuxing_of(pos: usize) -> &'static str {
    match pos {
        7 => "水",     // 坎·北
        1 => "火",     // 离·南
        3 | 0 => "木", // 震·东 / 巽·东南
        5 | 8 => "金", // 兑·西 / 乾·西北
        _ => "土",     // 4 中 / 2 坤·西南 / 6 艮·东北
    }
}

/// 五行关系：同 / 相生 / 相克 / 无关。
fn wuxing_relation(a: &str, b: &str) -> &'static str {
    if a == b {
        return "同";
    }
    let sheng = [
        ("木", "火"),
        ("火", "土"),
        ("土", "金"),
        ("金", "水"),
        ("水", "木"),
    ];
    let ke = [
        ("木", "土"),
        ("土", "水"),
        ("水", "火"),
        ("火", "金"),
        ("金", "木"),
    ];
    if sheng.contains(&(a, b)) || sheng.contains(&(b, a)) {
        return "相生";
    }
    if ke.contains(&(a, b)) || ke.contains(&(b, a)) {
        return "相克";
    }
    "无关"
}

#[test]
#[ignore]
fn mtr_wuxing_landing() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let causal = v["causal"].as_array().cloned().unwrap_or_default();
    let enc = LuoShuEncoder::new();
    let key2idx: HashMap<&str, usize> = mems
        .iter()
        .enumerate()
        .map(|(i, m)| (m["key"].as_str().unwrap_or(""), i))
        .collect();

    // 洛书落点 = 位置通道 argmax（排除中心）；五行 = 落点对应五行
    let mut mem_pos: Vec<usize> = Vec::with_capacity(mems.len());
    let mut mem_wx: Vec<&str> = Vec::with_capacity(mems.len());
    let mut pos_dist: HashMap<usize, usize> = HashMap::new();
    for m in mems {
        let content = m["content"].as_str().unwrap_or("");
        let (positional, _identity) = enc.encode_channels(content);
        let p = position_excluding_center(&positional);
        mem_pos.push(p);
        mem_wx.push(wuxing_of(p));
        *pos_dist.entry(p).or_insert(0) += 1;
    }

    // 因果对：五行关系分布
    let mut causal_total = 0usize;
    let mut rel_count: HashMap<&str, usize> = HashMap::new();
    for pair in &causal {
        let a = pair[0].as_str().unwrap_or("");
        let b = pair[1].as_str().unwrap_or("");
        let (Some(&ia), Some(&ib)) = (key2idx.get(a), key2idx.get(b)) else {
            continue;
        };
        causal_total += 1;
        *rel_count
            .entry(wuxing_relation(mem_wx[ia], mem_wx[ib]))
            .or_insert(0) += 1;
    }

    // 随机基线
    let mut rng_state: u64 = 20261104;
    let mut rand_rel: HashMap<&str, usize> = HashMap::new();
    let rand_total = 2000usize;
    for _ in 0..rand_total {
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let ia = (rng_state >> 33) as usize % mems.len();
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let ib = (rng_state >> 33) as usize % mems.len();
        *rand_rel
            .entry(wuxing_relation(mem_wx[ia], mem_wx[ib]))
            .or_insert(0) += 1;
    }

    println!("============================================================");
    println!(
        "去中心 + 五行生克实验（{} 记忆，{} 因果对）",
        mems.len(),
        causal_total
    );
    let mut pos_items: Vec<(usize, usize)> = pos_dist.iter().map(|(a, b)| (*a, *b)).collect();
    pos_items.sort_by(|a, b| b.1.cmp(&a.1));
    println!(
        "洛书落点分布（排除中心）：{}",
        pos_items
            .iter()
            .map(|(p, c)| format!("位{}×{}", p, c))
            .collect::<Vec<_>>()
            .join(", ")
    );
    for rel in ["同", "相生", "相克", "无关"] {
        let c = *rel_count.get(rel).unwrap_or(&0);
        let r = *rand_rel.get(rel).unwrap_or(&0);
        println!(
            "  {}：因果 {}/{}（{:.1}%） vs 随机 {:.1}%",
            rel,
            c,
            causal_total,
            c as f64 / causal_total.max(1) as f64 * 100.0,
            r as f64 / rand_total as f64 * 100.0
        );
    }
    println!("============================================================");
}

// ============================================================
// 五行生克链实验：河图方位 → 五行生克 → 洛书方位 → 下一个河图
// ============================================================

/// 五行相生：木→火→土→金→水→木。
fn wuxing_sheng(x: &str) -> &'static str {
    match x {
        "木" => "火",
        "火" => "土",
        "土" => "金",
        "金" => "水",
        "水" => "木",
        _ => "土",
    }
}

/// 五行相克：木→土→水→火→金→木。
fn wuxing_ke(x: &str) -> &'static str {
    match x {
        "木" => "土",
        "土" => "水",
        "水" => "火",
        "火" => "金",
        "金" => "木",
        _ => "土",
    }
}

/// 五行的主方位（洛书）：水=北7, 火=南1, 木=东3, 金=西5, 土=中4。
fn wuxing_primary_position(wx: &str) -> usize {
    match wx {
        "水" => 7,
        "火" => 1,
        "木" => 3,
        "金" => 5,
        _ => 4, // 土 = 中
    }
}

#[test]
#[ignore]
fn mtr_wuxing_chain() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let causal = v["causal"].as_array().cloned().unwrap_or_default();
    let enc = LuoShuEncoder::new();
    let key2idx: HashMap<&str, usize> = mems
        .iter()
        .enumerate()
        .map(|(i, m)| (m["key"].as_str().unwrap_or(""), i))
        .collect();

    // 每条记忆：河图方位（身份通道 argmax）→ 五行 → 生/克目标方位
    let mut mem_ht_pos: Vec<usize> = Vec::with_capacity(mems.len());
    let mut mem_wx: Vec<&str> = Vec::with_capacity(mems.len());
    let mut mem_sheng_pos: Vec<usize> = Vec::with_capacity(mems.len());
    let mut mem_ke_pos: Vec<usize> = Vec::with_capacity(mems.len());
    for m in mems {
        let content = m["content"].as_str().unwrap_or("");
        let (_positional, identity) = enc.encode_channels(content);
        let hp = pure_position(&identity);
        let wx = wuxing_of(hp);
        mem_ht_pos.push(hp);
        mem_wx.push(wx);
        mem_sheng_pos.push(wuxing_primary_position(wuxing_sheng(wx)));
        mem_ke_pos.push(wuxing_primary_position(wuxing_ke(wx)));
    }

    // 因果对：A 的洛书方位（生/克）是否等于 B 的河图方位
    let mut causal_total = 0usize;
    let mut sheng_hit = 0usize;
    let mut ke_hit = 0usize;
    let mut same_ht = 0usize;
    for pair in &causal {
        let a = pair[0].as_str().unwrap_or("");
        let b = pair[1].as_str().unwrap_or("");
        let (Some(&ia), Some(&ib)) = (key2idx.get(a), key2idx.get(b)) else {
            continue;
        };
        causal_total += 1;
        if mem_sheng_pos[ia] == mem_ht_pos[ib] {
            sheng_hit += 1;
        }
        if mem_ke_pos[ia] == mem_ht_pos[ib] {
            ke_hit += 1;
        }
        if mem_ht_pos[ia] == mem_ht_pos[ib] {
            same_ht += 1;
        }
    }

    // 随机基线
    let mut rng_state: u64 = 20261104;
    let mut rand_sheng = 0usize;
    let mut rand_ke = 0usize;
    let mut rand_same = 0usize;
    let rand_total = 2000usize;
    for _ in 0..rand_total {
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let ia = (rng_state >> 33) as usize % mems.len();
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let ib = (rng_state >> 33) as usize % mems.len();
        if mem_sheng_pos[ia] == mem_ht_pos[ib] {
            rand_sheng += 1;
        }
        if mem_ke_pos[ia] == mem_ht_pos[ib] {
            rand_ke += 1;
        }
        if mem_ht_pos[ia] == mem_ht_pos[ib] {
            rand_same += 1;
        }
    }

    println!("============================================================");
    println!(
        "五行生克链实验（{} 记忆，{} 因果对）",
        mems.len(),
        causal_total
    );
    println!(
        "A 的洛书方位（生）== B 的河图方位：因果 {}/{}（{:.1}%） vs 随机 {:.1}%",
        sheng_hit,
        causal_total,
        sheng_hit as f64 / causal_total.max(1) as f64 * 100.0,
        rand_sheng as f64 / rand_total as f64 * 100.0
    );
    println!(
        "A 的洛书方位（克）== B 的河图方位：因果 {}/{}（{:.1}%） vs 随机 {:.1}%",
        ke_hit,
        causal_total,
        ke_hit as f64 / causal_total.max(1) as f64 * 100.0,
        rand_ke as f64 / rand_total as f64 * 100.0
    );
    println!(
        "A 的河图方位 == B 的河图方位：因果 {}/{}（{:.1}%） vs 随机 {:.1}%",
        same_ht,
        causal_total,
        same_ht as f64 / causal_total.max(1) as f64 * 100.0,
        rand_same as f64 / rand_total as f64 * 100.0
    );
    println!("============================================================");
}

// ============================================================
// 体用实验：先天对待（体/河图） vs 后天流行（用/洛书）
// ============================================================

/// 后天八卦流行序列：震→巽→离→坤→兑→乾→坎→艮→震（帝出乎震…成言乎艮）。
fn houtian_next(pos: usize) -> usize {
    match pos {
        3 => 0, // 震→巽
        0 => 1, // 巽→离
        1 => 2, // 离→坤
        2 => 5, // 坤→兑
        5 => 8, // 兑→乾
        8 => 7, // 乾→坎
        7 => 6, // 坎→艮
        6 => 3, // 艮→震
        _ => 4,
    }
}

#[test]
#[ignore]
fn mtr_tidai_liuxing() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let causal = v["causal"].as_array().cloned().unwrap_or_default();
    let enc = LuoShuEncoder::new();
    let key2idx: HashMap<&str, usize> = mems
        .iter()
        .enumerate()
        .map(|(i, m)| (m["key"].as_str().unwrap_or(""), i))
        .collect();

    // 每条记忆的宫格位置（用融合向量的 argmax，作为"体"的静态坐标）
    let mut mem_pos: Vec<usize> = Vec::with_capacity(mems.len());
    for m in mems {
        let content = m["content"].as_str().unwrap_or("");
        let vec = enc.encode_text(content);
        mem_pos.push(pure_position(&vec.values));
    }

    // 因果对：对待（先天对立）/ 流行（后天相邻）
    let mut causal_total = 0usize;
    let mut tidai_hit = 0usize;
    let mut liuxing_hit = 0usize;
    let mut same_hit = 0usize;
    for pair in &causal {
        let a = pair[0].as_str().unwrap_or("");
        let b = pair[1].as_str().unwrap_or("");
        let (Some(&ia), Some(&ib)) = (key2idx.get(a), key2idx.get(b)) else {
            continue;
        };
        causal_total += 1;
        let pa = mem_pos[ia];
        let pb = mem_pos[ib];
        if xiantian_opposite(pa) == pb {
            tidai_hit += 1;
        }
        if houtian_next(pa) == pb {
            liuxing_hit += 1;
        }
        if pa == pb {
            same_hit += 1;
        }
    }

    // 随机基线
    let mut rng_state: u64 = 20261104;
    let mut rand_tidai = 0usize;
    let mut rand_liuxing = 0usize;
    let mut rand_same = 0usize;
    let rand_total = 2000usize;
    for _ in 0..rand_total {
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let ia = (rng_state >> 33) as usize % mems.len();
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let ib = (rng_state >> 33) as usize % mems.len();
        let pa = mem_pos[ia];
        let pb = mem_pos[ib];
        if xiantian_opposite(pa) == pb {
            rand_tidai += 1;
        }
        if houtian_next(pa) == pb {
            rand_liuxing += 1;
        }
        if pa == pb {
            rand_same += 1;
        }
    }

    println!("============================================================");
    println!("体用实验（{} 记忆，{} 因果对）", mems.len(), causal_total);
    println!(
        "对待（先天对立，体/河图）：因果 {}/{}（{:.1}%） vs 随机 {:.1}%",
        tidai_hit,
        causal_total,
        tidai_hit as f64 / causal_total.max(1) as f64 * 100.0,
        rand_tidai as f64 / rand_total as f64 * 100.0
    );
    println!(
        "流行（后天相邻，用/洛书）：因果 {}/{}（{:.1}%） vs 随机 {:.1}%",
        liuxing_hit,
        causal_total,
        liuxing_hit as f64 / causal_total.max(1) as f64 * 100.0,
        rand_liuxing as f64 / rand_total as f64 * 100.0
    );
    println!(
        "同宫（基线）：因果 {}/{}（{:.1}%） vs 随机 {:.1}%",
        same_hit,
        causal_total,
        same_hit as f64 / causal_total.max(1) as f64 * 100.0,
        rand_same as f64 / rand_total as f64 * 100.0
    );
    println!("============================================================");
}

// ============================================================
// 两级管道实验：五行生克 → 大类（5）→ 64 卦分位
// ============================================================

/// 位置（0..8，排除中心 4）→ 八卦索引（0..7）。
fn pos_to_idx(pos: usize) -> usize {
    match pos {
        0 => 0,
        1 => 1,
        2 => 2,
        3 => 3,
        5 => 4,
        6 => 5,
        7 => 6,
        8 => 7,
        _ => 0,
    }
}

#[test]
#[ignore]
fn mtr_64gua_pipeline() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let causal = v["causal"].as_array().cloned().unwrap_or_default();
    let enc = LuoShuEncoder::new();
    let key2idx: HashMap<&str, usize> = mems
        .iter()
        .enumerate()
        .map(|(i, m)| (m["key"].as_str().unwrap_or(""), i))
        .collect();

    // 两级管道：河图方位 → 五行生克 → 洛书大类 → 64 卦
    let mut mem_ht: Vec<usize> = Vec::with_capacity(mems.len());
    let mut mem_lt: Vec<usize> = Vec::with_capacity(mems.len());
    let mut mem_dalei: Vec<&str> = Vec::with_capacity(mems.len());
    let mut mem_gua: Vec<usize> = Vec::with_capacity(mems.len());
    let mut gua_dist: HashMap<usize, usize> = HashMap::new();
    let mut dalei_dist: HashMap<&str, usize> = HashMap::new();
    for m in mems {
        let content = m["content"].as_str().unwrap_or("");
        let vec = enc.encode_text(content);
        let hp = position_excluding_center(&vec.values);
        let wx = wuxing_of(hp);
        let lp = wuxing_primary_position(wuxing_sheng(wx));
        let upper = pos_to_idx(hp);
        let lower = pos_to_idx(lp);
        let gua = upper * 8 + lower;
        mem_ht.push(hp);
        mem_lt.push(lp);
        mem_dalei.push(wuxing_of(lp));
        mem_gua.push(gua);
        *gua_dist.entry(gua).or_insert(0) += 1;
        *dalei_dist.entry(wuxing_of(lp)).or_insert(0) += 1;
    }

    // 确定性：同一河图方位是否永远映射到同一 64 卦
    let mut ht_to_gua: HashMap<usize, std::collections::HashSet<usize>> = HashMap::new();
    for i in 0..mems.len() {
        ht_to_gua.entry(mem_ht[i]).or_default().insert(mem_gua[i]);
    }
    let deterministic = ht_to_gua.values().all(|s| s.len() == 1);

    // 因果对：同大类 / 同 64 卦
    let mut causal_total = 0usize;
    let mut same_dalei = 0usize;
    let mut same_gua = 0usize;
    for pair in &causal {
        let a = pair[0].as_str().unwrap_or("");
        let b = pair[1].as_str().unwrap_or("");
        let (Some(&ia), Some(&ib)) = (key2idx.get(a), key2idx.get(b)) else {
            continue;
        };
        causal_total += 1;
        if mem_dalei[ia] == mem_dalei[ib] {
            same_dalei += 1;
        }
        if mem_gua[ia] == mem_gua[ib] {
            same_gua += 1;
        }
    }

    // 随机基线
    let mut rng_state: u64 = 20261104;
    let mut rand_dalei = 0usize;
    let mut rand_gua = 0usize;
    let rand_total = 2000usize;
    for _ in 0..rand_total {
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let ia = (rng_state >> 33) as usize % mems.len();
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let ib = (rng_state >> 33) as usize % mems.len();
        if mem_dalei[ia] == mem_dalei[ib] {
            rand_dalei += 1;
        }
        if mem_gua[ia] == mem_gua[ib] {
            rand_gua += 1;
        }
    }

    println!("============================================================");
    println!(
        "两级管道实验（{} 记忆，{} 因果对）",
        mems.len(),
        causal_total
    );
    println!(
        "确定性（同河图方位 → 同 64 卦）：{}",
        if deterministic { "✅ 是" } else { "❌ 否" }
    );
    let mut dl_items: Vec<(&str, usize)> = dalei_dist.iter().map(|(a, b)| (*a, *b)).collect();
    dl_items.sort_by(|a, b| b.1.cmp(&a.1));
    println!(
        "洛书大类分布（5 类）：{}",
        dl_items
            .iter()
            .map(|(k, c)| format!("{}×{}", k, c))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "64 卦分布：{} 种不同卦，最大占比 {:.1}%",
        gua_dist.len(),
        gua_dist.values().max().copied().unwrap_or(0) as f64 / mems.len() as f64 * 100.0
    );
    println!(
        "因果对：同大类 {}/{}（{:.1}%） vs 随机 {:.1}%；同 64 卦 {}/{}（{:.1}%） vs 随机 {:.1}%",
        same_dalei,
        causal_total,
        same_dalei as f64 / causal_total.max(1) as f64 * 100.0,
        rand_dalei as f64 / rand_total as f64 * 100.0,
        same_gua,
        causal_total,
        same_gua as f64 / causal_total.max(1) as f64 * 100.0,
        rand_gua as f64 / rand_total as f64 * 100.0
    );
    println!("============================================================");
}

// ============================================================
// 64 卦（下卦=次高位置）实验：上卦=argmax，下卦=次高
// ============================================================

/// 排除中心与指定位置后的次高位置。
fn second_position_excluding_center(values: &[f32; 9], exclude: usize) -> usize {
    let mut best = usize::MAX;
    let mut best_val = f32::NEG_INFINITY;
    for i in 0..9 {
        if i == 4 || i == exclude {
            continue;
        }
        if values[i] > best_val {
            best_val = values[i];
            best = i;
        }
    }
    best
}

#[test]
#[ignore]
fn mtr_64gua_second() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let causal = v["causal"].as_array().cloned().unwrap_or_default();
    let enc = LuoShuEncoder::new();
    let key2idx: HashMap<&str, usize> = mems
        .iter()
        .enumerate()
        .map(|(i, m)| (m["key"].as_str().unwrap_or(""), i))
        .collect();

    // 上卦 = argmax（排除中心）；下卦 = 次高（排除中心与上卦）；64 卦 = 上×8+下
    let mut mem_upper: Vec<usize> = Vec::with_capacity(mems.len());
    let mut mem_lower: Vec<usize> = Vec::with_capacity(mems.len());
    let mut mem_gua: Vec<usize> = Vec::with_capacity(mems.len());
    let mut gua_dist: HashMap<usize, usize> = HashMap::new();
    let mut upper_dist: HashMap<usize, usize> = HashMap::new();
    let mut lower_dist: HashMap<usize, usize> = HashMap::new();
    for m in mems {
        let content = m["content"].as_str().unwrap_or("");
        let vec = enc.encode_text(content);
        let up = position_excluding_center(&vec.values);
        let lo = second_position_excluding_center(&vec.values, up);
        let gua = pos_to_idx(up) * 8 + pos_to_idx(lo);
        mem_upper.push(up);
        mem_lower.push(lo);
        mem_gua.push(gua);
        *gua_dist.entry(gua).or_insert(0) += 1;
        *upper_dist.entry(up).or_insert(0) += 1;
        *lower_dist.entry(lo).or_insert(0) += 1;
    }

    // 因果对：同上卦 / 同下卦 / 同 64 卦
    let mut causal_total = 0usize;
    let mut same_up = 0usize;
    let mut same_lo = 0usize;
    let mut same_gua = 0usize;
    for pair in &causal {
        let a = pair[0].as_str().unwrap_or("");
        let b = pair[1].as_str().unwrap_or("");
        let (Some(&ia), Some(&ib)) = (key2idx.get(a), key2idx.get(b)) else {
            continue;
        };
        causal_total += 1;
        if mem_upper[ia] == mem_upper[ib] {
            same_up += 1;
        }
        if mem_lower[ia] == mem_lower[ib] {
            same_lo += 1;
        }
        if mem_gua[ia] == mem_gua[ib] {
            same_gua += 1;
        }
    }

    // 随机基线
    let mut rng_state: u64 = 20261104;
    let mut rand_up = 0usize;
    let mut rand_lo = 0usize;
    let mut rand_gua = 0usize;
    let rand_total = 2000usize;
    for _ in 0..rand_total {
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let ia = (rng_state >> 33) as usize % mems.len();
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let ib = (rng_state >> 33) as usize % mems.len();
        if mem_upper[ia] == mem_upper[ib] {
            rand_up += 1;
        }
        if mem_lower[ia] == mem_lower[ib] {
            rand_lo += 1;
        }
        if mem_gua[ia] == mem_gua[ib] {
            rand_gua += 1;
        }
    }

    println!("============================================================");
    println!(
        "64 卦（下卦=次高）实验（{} 记忆，{} 因果对）",
        mems.len(),
        causal_total
    );
    println!(
        "上卦分布：{} 种；下卦分布：{} 种；64 卦分布：{} 种，最大占比 {:.1}%",
        upper_dist.len(),
        lower_dist.len(),
        gua_dist.len(),
        gua_dist.values().max().copied().unwrap_or(0) as f64 / mems.len() as f64 * 100.0
    );
    println!(
        "因果对：同上卦 {}/{}（{:.1}%） vs 随机 {:.1}%",
        same_up,
        causal_total,
        same_up as f64 / causal_total.max(1) as f64 * 100.0,
        rand_up as f64 / rand_total as f64 * 100.0
    );
    println!(
        "        同下卦 {}/{}（{:.1}%） vs 随机 {:.1}%",
        same_lo,
        causal_total,
        same_lo as f64 / causal_total.max(1) as f64 * 100.0,
        rand_lo as f64 / rand_total as f64 * 100.0
    );
    println!(
        "        同 64 卦 {}/{}（{:.1}%） vs 随机 {:.1}%",
        same_gua,
        causal_total,
        same_gua as f64 / causal_total.max(1) as f64 * 100.0,
        rand_gua as f64 / rand_total as f64 * 100.0
    );
    println!("============================================================");
}

// ============================================================
// 64 卦剪枝实验：候选集缩小率 + 目标保留率
// ============================================================

#[test]
#[ignore]
fn mtr_64gua_pruning() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let queries = v["queries"].as_array().expect("queries 数组");
    let enc = LuoShuEncoder::new();
    let key2idx: HashMap<&str, usize> = mems
        .iter()
        .enumerate()
        .map(|(i, m)| (m["key"].as_str().unwrap_or(""), i))
        .collect();

    // 每条记忆的 64 卦（上=argmax，下=次高）
    let mut mem_gua: Vec<usize> = Vec::with_capacity(mems.len());
    let mut bucket: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, m) in mems.iter().enumerate() {
        let content = m["content"].as_str().unwrap_or("");
        let vec = enc.encode_text(content);
        let up = position_excluding_center(&vec.values);
        let lo = second_position_excluding_center(&vec.values, up);
        let gua = pos_to_idx(up) * 8 + pos_to_idx(lo);
        mem_gua.push(gua);
        bucket.entry(gua).or_default().push(i);
    }

    // 查询：64 卦 → 候选桶 → 目标是否在桶内
    let mut total = 0usize;
    let mut hit = 0usize;
    let mut cand_sum = 0usize;
    let mut cand_max = 0usize;
    let mut empty_bucket = 0usize;
    for q in queries {
        let query = q["query"].as_str().unwrap_or("");
        let target = q["target"].as_str().unwrap_or("");
        let qvec = enc.encode_text(query);
        let up = position_excluding_center(&qvec.values);
        let lo = second_position_excluding_center(&qvec.values, up);
        let qgua = pos_to_idx(up) * 8 + pos_to_idx(lo);
        let Some(&ti) = key2idx.get(target) else {
            continue;
        };
        total += 1;
        let cands = bucket.get(&qgua);
        let size = cands.map_or(0, |c| c.len());
        if size == 0 {
            empty_bucket += 1;
        }
        cand_sum += size;
        cand_max = cand_max.max(size);
        if cands.is_some_and(|c| c.contains(&ti)) {
            hit += 1;
        }
    }

    // 8 卦基线（对比塌缩）
    let mut mem_bagua: Vec<usize> = Vec::with_capacity(mems.len());
    let mut bagua_bucket: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, m) in mems.iter().enumerate() {
        let content = m["content"].as_str().unwrap_or("");
        let vec = enc.encode_text(content);
        let b = mirror_project(&vec).best_index;
        mem_bagua.push(b);
        bagua_bucket.entry(b).or_default().push(i);
    }
    let mut b_total = 0usize;
    let mut b_hit = 0usize;
    let mut b_cand_sum = 0usize;
    for q in queries {
        let query = q["query"].as_str().unwrap_or("");
        let target = q["target"].as_str().unwrap_or("");
        let qvec = enc.encode_text(query);
        let qb = mirror_project(&qvec).best_index;
        let Some(&ti) = key2idx.get(target) else {
            continue;
        };
        b_total += 1;
        let cands = bagua_bucket.get(&qb);
        b_cand_sum += cands.map_or(0, |c| c.len());
        if cands.is_some_and(|c| c.contains(&ti)) {
            b_hit += 1;
        }
    }

    println!("============================================================");
    println!("64 卦剪枝实验（{} 记忆，{} 查询）", mems.len(), total);
    println!(
        "64 卦：平均候选 {:.1} 条（{:.1}%），最大 {} 条；目标保留 {}/{}（{:.1}%）；空桶 {}",
        cand_sum as f64 / total.max(1) as f64,
        cand_sum as f64 / total.max(1) as f64 / mems.len() as f64 * 100.0,
        cand_max,
        hit,
        total,
        hit as f64 / total.max(1) as f64 * 100.0,
        empty_bucket
    );
    println!(
        "8 卦：平均候选 {:.1} 条（{:.1}%）；目标保留 {}/{}（{:.1}%）",
        b_cand_sum as f64 / b_total.max(1) as f64,
        b_cand_sum as f64 / b_total.max(1) as f64 / mems.len() as f64 * 100.0,
        b_hit,
        b_total,
        b_hit as f64 / b_total.max(1) as f64 * 100.0
    );
    println!("============================================================");
}

// ============================================================
// 64 卦软加权实验：不排除候选，只给同卦/邻卦加分
// ============================================================

fn char_bigrams(s: &str) -> std::collections::HashSet<(char, char)> {
    let chars: Vec<char> = s.chars().collect();
    let mut set = std::collections::HashSet::new();
    for w in chars.windows(2) {
        set.insert((w[0], w[1]));
    }
    set
}

fn jaccard(
    a: &std::collections::HashSet<(char, char)>,
    b: &std::collections::HashSet<(char, char)>,
) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f32;
    let union = a.union(b).count() as f32;
    inter / union
}

fn gua_of(text: &str, enc: &LuoShuEncoder) -> usize {
    let vec = enc.encode_text(text);
    let up = position_excluding_center(&vec.values);
    let lo = second_position_excluding_center(&vec.values, up);
    pos_to_idx(up) * 8 + pos_to_idx(lo)
}

#[test]
#[ignore]
fn mtr_64gua_soft() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let queries = v["queries"].as_array().expect("queries 数组");
    let enc = LuoShuEncoder::new();
    let key2idx: HashMap<&str, usize> = mems
        .iter()
        .enumerate()
        .map(|(i, m)| (m["key"].as_str().unwrap_or(""), i))
        .collect();

    let mem_texts: Vec<&str> = mems
        .iter()
        .map(|m| m["content"].as_str().unwrap_or(""))
        .collect();
    let mem_bigrams: Vec<std::collections::HashSet<(char, char)>> =
        mem_texts.iter().map(|t| char_bigrams(t)).collect();
    let mem_gua: Vec<usize> = mem_texts.iter().map(|t| gua_of(t, &enc)).collect();

    // 三种打分：基线（bigram）、软加权（+0.3/0.15）、软加权（+0.1/0.05）
    let mut base_top1 = 0usize;
    let mut base_mrr = 0.0f64;
    let mut soft_top1 = 0usize;
    let mut soft_mrr = 0.0f64;
    let mut weak_top1 = 0usize;
    let mut weak_mrr = 0.0f64;
    let mut total = 0usize;

    for q in queries {
        let query = q["query"].as_str().unwrap_or("");
        let target = q["target"].as_str().unwrap_or("");
        let Some(&ti) = key2idx.get(target) else {
            continue;
        };
        total += 1;
        let qb = char_bigrams(query);
        let qgua = gua_of(query, &enc);

        let mut base_scores: Vec<(usize, f32)> = Vec::with_capacity(mems.len());
        let mut soft_scores: Vec<(usize, f32)> = Vec::with_capacity(mems.len());
        let mut weak_scores: Vec<(usize, f32)> = Vec::with_capacity(mems.len());
        for i in 0..mems.len() {
            let base = jaccard(&qb, &mem_bigrams[i]);
            let h = (qgua ^ mem_gua[i]).count_ones();
            let bonus = if h == 0 {
                0.3
            } else if h == 1 {
                0.15
            } else {
                0.0
            };
            let weak_bonus = if h == 0 {
                0.1
            } else if h == 1 {
                0.05
            } else {
                0.0
            };
            base_scores.push((i, base));
            soft_scores.push((i, base + bonus));
            weak_scores.push((i, base + weak_bonus));
        }
        base_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        soft_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        weak_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let rank_of = |scores: &[(usize, f32)]| -> Option<usize> {
            scores.iter().position(|(i, _)| *i == ti).map(|p| p + 1)
        };
        if let Some(r) = rank_of(&base_scores) {
            if r == 1 {
                base_top1 += 1;
            }
            base_mrr += 1.0 / r as f64;
        }
        if let Some(r) = rank_of(&soft_scores) {
            if r == 1 {
                soft_top1 += 1;
            }
            soft_mrr += 1.0 / r as f64;
        }
        if let Some(r) = rank_of(&weak_scores) {
            if r == 1 {
                weak_top1 += 1;
            }
            weak_mrr += 1.0 / r as f64;
        }
    }

    let n = total.max(1) as f64;
    println!("============================================================");
    println!("64 卦软加权实验（{} 记忆，{} 查询）", mems.len(), total);
    println!(
        "基线（bigram）：top1 {:.3}，MRR {:.3}",
        base_top1 as f64 / n,
        base_mrr / n
    );
    println!(
        "软加权（+0.3/+0.15）：top1 {:.3}，MRR {:.3}",
        soft_top1 as f64 / n,
        soft_mrr / n
    );
    println!(
        "软加权（+0.1/+0.05）：top1 {:.3}，MRR {:.3}",
        weak_top1 as f64 / n,
        weak_mrr / n
    );
    println!("============================================================");
}

// ============================================================
// 完全体实验：五行生克 + 镜像梯形链条（ROI）
// ============================================================

/// 完全体管道：河图方位 → 五行生克 → 洛书方位 → 梯形 ROI。
fn complete_roi(text: &str, enc: &LuoShuEncoder, depth: u32) -> TrapezoidROI {
    let vec = enc.encode_text(text);
    let ht = mirror_project(&vec).best_index;
    let wx = wuxing_of(HOUTIAN_POS[ht]);
    let lt = wuxing_primary_position(wuxing_sheng(wx));
    TrapezoidROI::centered(lt, depth)
}

#[test]
#[ignore]
fn mtr_complete() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let queries = v["queries"].as_array().expect("queries 数组");
    let enc = LuoShuEncoder::new();
    let key2idx: HashMap<&str, usize> = mems
        .iter()
        .enumerate()
        .map(|(i, m)| (m["key"].as_str().unwrap_or(""), i))
        .collect();

    let mem_texts: Vec<&str> = mems
        .iter()
        .map(|m| m["content"].as_str().unwrap_or(""))
        .collect();
    let mem_bigrams: Vec<std::collections::HashSet<(char, char)>> =
        mem_texts.iter().map(|t| char_bigrams(t)).collect();
    // 每条记忆的洛书方位（五行生克结果）
    let mem_lt: Vec<usize> = mem_texts
        .iter()
        .map(|t| {
            let vec = enc.encode_text(t);
            let ht = mirror_project(&vec).best_index;
            let wx = wuxing_of(HOUTIAN_POS[ht]);
            wuxing_primary_position(wuxing_sheng(wx))
        })
        .collect();

    for depth in [1u32, 2u32] {
        let mut total = 0usize;
        let mut hit = 0usize;
        let mut cand_sum = 0usize;
        let mut base_top1 = 0usize;
        let mut base_mrr = 0.0f64;
        let mut soft_top1 = 0usize;
        let mut soft_mrr = 0.0f64;

        for q in queries {
            let query = q["query"].as_str().unwrap_or("");
            let target = q["target"].as_str().unwrap_or("");
            let Some(&ti) = key2idx.get(target) else {
                continue;
            };
            total += 1;
            let roi = complete_roi(query, &enc, depth);
            let qb = char_bigrams(query);

            // 候选 = ROI 内的记忆
            let mut cand_count = 0usize;
            let mut target_in = false;
            for i in 0..mems.len() {
                if roi.contains_position(mem_lt[i]) {
                    cand_count += 1;
                    if i == ti {
                        target_in = true;
                    }
                }
            }
            cand_sum += cand_count;
            if target_in {
                hit += 1;
            }

            // 基线 vs 软加权（ROI 内 +0.1）
            let mut base_scores: Vec<(usize, f32)> = Vec::with_capacity(mems.len());
            let mut soft_scores: Vec<(usize, f32)> = Vec::with_capacity(mems.len());
            for i in 0..mems.len() {
                let base = jaccard(&qb, &mem_bigrams[i]);
                let bonus = if roi.contains_position(mem_lt[i]) {
                    0.1
                } else {
                    0.0
                };
                base_scores.push((i, base));
                soft_scores.push((i, base + bonus));
            }
            base_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            soft_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            let rank_of = |scores: &[(usize, f32)]| -> Option<usize> {
                scores.iter().position(|(i, _)| *i == ti).map(|p| p + 1)
            };
            if let Some(r) = rank_of(&base_scores) {
                if r == 1 {
                    base_top1 += 1;
                }
                base_mrr += 1.0 / r as f64;
            }
            if let Some(r) = rank_of(&soft_scores) {
                if r == 1 {
                    soft_top1 += 1;
                }
                soft_mrr += 1.0 / r as f64;
            }
        }

        let n = total.max(1) as f64;
        println!("============================================================");
        println!(
            "完全体实验（depth={}，{} 记忆，{} 查询）",
            depth,
            mems.len(),
            total
        );
        println!(
            "ROI 候选：平均 {:.1} 条（{:.1}%）；目标保留 {}/{}（{:.1}%）",
            cand_sum as f64 / n,
            cand_sum as f64 / n / mems.len() as f64 * 100.0,
            hit,
            total,
            hit as f64 / n * 100.0
        );
        println!(
            "基线（bigram）：top1 {:.3}，MRR {:.3}",
            base_top1 as f64 / n,
            base_mrr / n
        );
        println!(
            "软加权（ROI +0.1）：top1 {:.3}，MRR {:.3}",
            soft_top1 as f64 / n,
            soft_mrr / n
        );
    }
    println!("============================================================");
}

// ============================================================
// 五行流动实验：宫位 A → 五行生克 → 目标宫位集合（多对多）
// ============================================================

/// 五行对应的所有宫位（多对多，排除中宫位4——中宫不承载记忆）。
fn wuxing_positions(wx: &str) -> Vec<usize> {
    match wx {
        "水" => vec![7],
        "火" => vec![1],
        "木" => vec![0, 3],
        "金" => vec![5, 8],
        _ => vec![2, 6], // 土：坤(2)、艮(6)；中宫(4)不承载记忆
    }
}

#[test]
#[ignore]
fn mtr_wuxing_flow() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let causal = v["causal"].as_array().cloned().unwrap_or_default();
    let enc = LuoShuEncoder::new();
    let key2idx: HashMap<&str, usize> = mems
        .iter()
        .enumerate()
        .map(|(i, m)| (m["key"].as_str().unwrap_or(""), i))
        .collect();

    // 每条记忆的宫位（融合向量 argmax，含中心）
    let mut mem_pos: Vec<usize> = Vec::with_capacity(mems.len());
    for m in mems {
        let content = m["content"].as_str().unwrap_or("");
        let vec = enc.encode_text(content);
        mem_pos.push(pure_position(&vec.values));
    }

    // 因果对：B 的宫位是否在 A 的"生目标宫位集合" / "克目标宫位集合"里
    let mut causal_total = 0usize;
    let mut sheng_hit = 0usize;
    let mut ke_hit = 0usize;
    let mut same_hit = 0usize;
    for pair in &causal {
        let a = pair[0].as_str().unwrap_or("");
        let b = pair[1].as_str().unwrap_or("");
        let (Some(&ia), Some(&ib)) = (key2idx.get(a), key2idx.get(b)) else {
            continue;
        };
        causal_total += 1;
        let pa = mem_pos[ia];
        let pb = mem_pos[ib];
        let wx_a = wuxing_of(pa);
        let sheng_targets = wuxing_positions(wuxing_sheng(wx_a));
        let ke_targets = wuxing_positions(wuxing_ke(wx_a));
        if sheng_targets.contains(&pb) {
            sheng_hit += 1;
        }
        if ke_targets.contains(&pb) {
            ke_hit += 1;
        }
        if pa == pb {
            same_hit += 1;
        }
    }

    // 随机基线
    let mut rng_state: u64 = 20261104;
    let mut rand_sheng = 0usize;
    let mut rand_ke = 0usize;
    let mut rand_same = 0usize;
    let rand_total = 2000usize;
    for _ in 0..rand_total {
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let ia = (rng_state >> 33) as usize % mems.len();
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let ib = (rng_state >> 33) as usize % mems.len();
        let pa = mem_pos[ia];
        let pb = mem_pos[ib];
        let wx_a = wuxing_of(pa);
        if wuxing_positions(wuxing_sheng(wx_a)).contains(&pb) {
            rand_sheng += 1;
        }
        if wuxing_positions(wuxing_ke(wx_a)).contains(&pb) {
            rand_ke += 1;
        }
        if pa == pb {
            rand_same += 1;
        }
    }

    println!("============================================================");
    println!(
        "五行流动实验（{} 记忆，{} 因果对）",
        mems.len(),
        causal_total
    );
    println!(
        "生流动（A 宫位 → 生目标宫位）：因果 {}/{}（{:.1}%） vs 随机 {:.1}%",
        sheng_hit,
        causal_total,
        sheng_hit as f64 / causal_total.max(1) as f64 * 100.0,
        rand_sheng as f64 / rand_total as f64 * 100.0
    );
    println!(
        "克流动（A 宫位 → 克目标宫位）：因果 {}/{}（{:.1}%） vs 随机 {:.1}%",
        ke_hit,
        causal_total,
        ke_hit as f64 / causal_total.max(1) as f64 * 100.0,
        rand_ke as f64 / rand_total as f64 * 100.0
    );
    println!(
        "同宫（基线）：因果 {}/{}（{:.1}%） vs 随机 {:.1}%",
        same_hit,
        causal_total,
        same_hit as f64 / causal_total.max(1) as f64 * 100.0,
        rand_same as f64 / rand_total as f64 * 100.0
    );
    println!("============================================================");
}

// ============================================================
// 五行流动·结构实验：确定性 / 分布 / 覆盖 / 环完整性（不测语义）
// ============================================================

#[test]
#[ignore]
fn mtr_wuxing_flow_structure() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let enc = LuoShuEncoder::new();

    // 每条记忆：宫位 → 生目标宫位集合
    let mut pos_dist: HashMap<usize, usize> = HashMap::new();
    let mut flow_dist: HashMap<usize, usize> = HashMap::new();
    let mut pos_to_targets: HashMap<usize, std::collections::HashSet<usize>> = HashMap::new();
    for m in mems {
        let content = m["content"].as_str().unwrap_or("");
        let vec = enc.encode_text(content);
        let p = pure_position(&vec.values);
        *pos_dist.entry(p).or_insert(0) += 1;
        let wx = wuxing_of(p);
        let targets = wuxing_positions(wuxing_sheng(wx));
        pos_to_targets
            .entry(p)
            .or_default()
            .extend(targets.iter().copied());
        for t in &targets {
            *flow_dist.entry(*t).or_insert(0) += 1;
        }
    }

    let deterministic = pos_to_targets.values().all(|s| !s.is_empty());
    let pos_covered = pos_dist.len();
    let flow_covered = flow_dist.len();
    let cycle_ok = [7usize, 0, 3, 1, 2, 4, 6, 5, 8]
        .iter()
        .all(|&p| !wuxing_positions(wuxing_sheng(wuxing_of(p))).is_empty());

    println!("============================================================");
    println!("五行流动·结构实验（{} 记忆，不测语义）", mems.len());
    println!(
        "确定性（每个宫位都有生目标）：{}",
        if deterministic { "✅ 是" } else { "❌ 否" }
    );
    println!(
        "环完整性（水→木→火→土→金→水）：{}",
        if cycle_ok { "✅ 闭合" } else { "❌ 断裂" }
    );
    println!("宫位覆盖：{}/9 个宫位有记忆", pos_covered);
    println!("流动覆盖：{}/9 个宫位被流向", flow_covered);

    let mut pos_items: Vec<(usize, usize)> = pos_dist.iter().map(|(a, b)| (*a, *b)).collect();
    pos_items.sort_by(|a, b| b.1.cmp(&a.1));
    println!(
        "宫位分布：{}",
        pos_items
            .iter()
            .map(|(p, c)| format!("位{}×{}", p, c))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut flow_items: Vec<(usize, usize)> = flow_dist.iter().map(|(a, b)| (*a, *b)).collect();
    flow_items.sort_by(|a, b| b.1.cmp(&a.1));
    println!(
        "生流动分布（流向）：{}",
        flow_items
            .iter()
            .map(|(p, c)| format!("位{}×{}", p, c))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("============================================================");
}

// ============================================================
// 五行流动·身份通道版：用身份通道的宫位重跑结构实验
// ============================================================

#[test]
#[ignore]
fn mtr_wuxing_flow_identity() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let enc = LuoShuEncoder::new();

    let mut pos_dist: HashMap<usize, usize> = HashMap::new();
    let mut flow_dist: HashMap<usize, usize> = HashMap::new();
    for m in mems {
        let content = m["content"].as_str().unwrap_or("");
        let (_pos_ch, id_ch) = enc.encode_channels(content);
        let p = position_excluding_center(&id_ch);
        *pos_dist.entry(p).or_insert(0) += 1;
        let wx = wuxing_of(p);
        for t in wuxing_positions(wuxing_sheng(wx)) {
            *flow_dist.entry(t).or_insert(0) += 1;
        }
    }

    let pos_covered = pos_dist.keys().filter(|&&p| p != 4).count();
    let flow_covered = flow_dist.keys().filter(|&&p| p != 4).count();
    let cycle_ok = [7usize, 0, 3, 1, 2, 4, 6, 5, 8]
        .iter()
        .all(|&p| !wuxing_positions(wuxing_sheng(wuxing_of(p))).is_empty());

    println!("============================================================");
    println!("五行流动·身份通道版（{} 记忆）", mems.len());
    println!(
        "环完整性（水→木→火→土→金→水）：{}",
        if cycle_ok { "✅ 闭合" } else { "❌ 断裂" }
    );
    println!("宫位覆盖：{}/8（排除中宫）", pos_covered);
    println!("流动覆盖：{}/8（排除中宫）", flow_covered);

    let mut pos_items: Vec<(usize, usize)> = pos_dist.iter().map(|(a, b)| (*a, *b)).collect();
    pos_items.sort_by(|a, b| b.1.cmp(&a.1));
    println!(
        "宫位分布：{}",
        pos_items
            .iter()
            .map(|(p, c)| format!("位{}×{}", p, c))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut flow_items: Vec<(usize, usize)> = flow_dist.iter().map(|(a, b)| (*a, *b)).collect();
    flow_items.sort_by(|a, b| b.1.cmp(&a.1));
    println!(
        "生流动分布（流向）：{}",
        flow_items
            .iter()
            .map(|(p, c)| format!("位{}×{}", p, c))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("============================================================");
}

// ============================================================
// 64 卦 + 相邻过滤实验：Hamming ≤ 1 / ≤ 2 的候选与目标保留
// ============================================================

#[test]
#[ignore]
fn mtr_64gua_adjacent() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let queries = v["queries"].as_array().expect("queries 数组");
    let enc = LuoShuEncoder::new();
    let key2idx: HashMap<&str, usize> = mems
        .iter()
        .enumerate()
        .map(|(i, m)| (m["key"].as_str().unwrap_or(""), i))
        .collect();

    let mem_texts: Vec<&str> = mems
        .iter()
        .map(|m| m["content"].as_str().unwrap_or(""))
        .collect();
    let mem_gua: Vec<usize> = mem_texts.iter().map(|t| gua_of(t, &enc)).collect();

    for threshold in [0u32, 1u32, 2u32] {
        let mut total = 0usize;
        let mut hit = 0usize;
        let mut cand_sum = 0usize;
        let mut cand_max = 0usize;
        for q in queries {
            let query = q["query"].as_str().unwrap_or("");
            let target = q["target"].as_str().unwrap_or("");
            let Some(&ti) = key2idx.get(target) else {
                continue;
            };
            total += 1;
            let qgua = gua_of(query, &enc);
            let mut count = 0usize;
            let mut target_in = false;
            for i in 0..mems.len() {
                let h = (qgua ^ mem_gua[i]).count_ones();
                if h <= threshold {
                    count += 1;
                    if i == ti {
                        target_in = true;
                    }
                }
            }
            cand_sum += count;
            cand_max = cand_max.max(count);
            if target_in {
                hit += 1;
            }
        }
        let n = total.max(1) as f64;
        println!(
            "Hamming≤{}：平均候选 {:.1} 条（{:.1}%），最大 {} 条；目标保留 {}/{}（{:.1}%）",
            threshold,
            cand_sum as f64 / n,
            cand_sum as f64 / n / mems.len() as f64 * 100.0,
            cand_max,
            hit,
            total,
            hit as f64 / n * 100.0
        );
    }
}

// ============================================================
// 网格索引实验（记忆→记忆）：相似记忆是否落在同格/邻格
// ============================================================

#[test]
#[ignore]
fn mtr_grid_index() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let enc = LuoShuEncoder::new();

    let mem_texts: Vec<&str> = mems
        .iter()
        .map(|m| m["content"].as_str().unwrap_or(""))
        .collect();
    let mem_bigrams: Vec<std::collections::HashSet<(char, char)>> =
        mem_texts.iter().map(|t| char_bigrams(t)).collect();
    let mem_gua: Vec<usize> = mem_texts.iter().map(|t| gua_of(t, &enc)).collect();

    // 对每条记忆，找 top-10 最相似的其他记忆，看它们是否在同格/邻格
    let top_k = 10usize;
    for threshold in [0u32, 1u32, 2u32] {
        let mut total_pairs = 0usize;
        let mut in_cell = 0usize;
        for a in 0..mems.len() {
            let mut sims: Vec<(usize, f32)> = (0..mems.len())
                .filter(|&b| b != a)
                .map(|b| (b, jaccard(&mem_bigrams[a], &mem_bigrams[b])))
                .collect();
            sims.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap_or(std::cmp::Ordering::Equal));
            for &(b, _) in sims.iter().take(top_k) {
                total_pairs += 1;
                let h = (mem_gua[a] ^ mem_gua[b]).count_ones();
                if h <= threshold {
                    in_cell += 1;
                }
            }
        }
        println!(
            "Hamming≤{}：top-{} 相似记忆中，同格/邻格 {}/{}（{:.1}%）",
            threshold,
            top_k,
            in_cell,
            total_pairs,
            in_cell as f64 / total_pairs.max(1) as f64 * 100.0
        );
    }

    // 随机基线：随机配对的同格/邻格率
    let mut rng_state: u64 = 20261104;
    for threshold in [0u32, 1u32, 2u32] {
        let mut hit = 0usize;
        let rand_total = 5000usize;
        for _ in 0..rand_total {
            rng_state = rng_state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let a = (rng_state >> 33) as usize % mems.len();
            rng_state = rng_state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let b = (rng_state >> 33) as usize % mems.len();
            let h = (mem_gua[a] ^ mem_gua[b]).count_ones();
            if h <= threshold {
                hit += 1;
            }
        }
        println!(
            "随机基线 Hamming≤{}：{}/{}（{:.1}%）",
            threshold,
            hit,
            rand_total,
            hit as f64 / rand_total as f64 * 100.0
        );
    }
}

// ============================================================
// 连续坐标实验：相似记忆在 9 维空间里是否更近（余弦相似度）
// ============================================================

fn cosine9(a: &[f32; 9], b: &[f32; 9]) -> f32 {
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..9 {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na <= 1e-9 || nb <= 1e-9 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

#[test]
#[ignore]
fn mtr_continuous_coord() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let enc = LuoShuEncoder::new();

    let mem_texts: Vec<&str> = mems
        .iter()
        .map(|m| m["content"].as_str().unwrap_or(""))
        .collect();
    let mem_bigrams: Vec<std::collections::HashSet<(char, char)>> =
        mem_texts.iter().map(|t| char_bigrams(t)).collect();
    let mem_fused: Vec<[f32; 9]> = mem_texts
        .iter()
        .map(|t| enc.encode_text(t).values)
        .collect();
    let mem_identity: Vec<[f32; 9]> = mem_texts.iter().map(|t| enc.encode_channels(t).1).collect();

    let top_k = 10usize;
    for (name, coords) in [("融合向量", &mem_fused), ("身份通道", &mem_identity)] {
        let mut sim_sum = 0.0f64;
        let mut sim_count = 0usize;
        for a in 0..mems.len() {
            let mut sims: Vec<(usize, f32)> = (0..mems.len())
                .filter(|&b| b != a)
                .map(|b| (b, jaccard(&mem_bigrams[a], &mem_bigrams[b])))
                .collect();
            sims.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap_or(std::cmp::Ordering::Equal));
            for &(b, _) in sims.iter().take(top_k) {
                sim_sum += cosine9(&coords[a], &coords[b]) as f64;
                sim_count += 1;
            }
        }
        let avg_sim = sim_sum / sim_count.max(1) as f64;

        // 随机基线
        let mut rng_state: u64 = 20261104;
        let mut rand_sum = 0.0f64;
        let rand_total = 5000usize;
        for _ in 0..rand_total {
            rng_state = rng_state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let a = (rng_state >> 33) as usize % mems.len();
            rng_state = rng_state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let b = (rng_state >> 33) as usize % mems.len();
            rand_sum += cosine9(&coords[a], &coords[b]) as f64;
        }
        let avg_rand = rand_sum / rand_total as f64;
        println!(
            "{}：top-{} 相似记忆平均余弦 {:.4}；随机平均余弦 {:.4}；差值 {:+.4}",
            name,
            top_k,
            avg_sim,
            avg_rand,
            avg_sim - avg_rand
        );
    }
}

// ============================================================
// 查询入五行流动实验：查询坐标沿生流动移动，轨迹上的宫位为候选
// ============================================================

/// 从起始宫位沿生流动走 steps 步，返回轨迹上的所有宫位。
fn flow_trajectory(start: usize, steps: usize) -> std::collections::HashSet<usize> {
    let mut visited = std::collections::HashSet::new();
    visited.insert(start);
    let mut frontier = vec![start];
    for _ in 0..steps {
        let mut next = Vec::new();
        for p in frontier {
            let wx = wuxing_of(p);
            for t in wuxing_positions(wuxing_sheng(wx)) {
                if visited.insert(t) {
                    next.push(t);
                }
            }
        }
        frontier = next;
    }
    visited
}

#[test]
#[ignore]
fn mtr_wuxing_query_flow() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let queries = v["queries"].as_array().expect("queries 数组");
    let enc = LuoShuEncoder::new();
    let key2idx: HashMap<&str, usize> = mems
        .iter()
        .enumerate()
        .map(|(i, m)| (m["key"].as_str().unwrap_or(""), i))
        .collect();

    // 每条记忆的宫位（身份通道，排除中宫）
    let mem_pos: Vec<usize> = mems
        .iter()
        .map(|m| {
            let content = m["content"].as_str().unwrap_or("");
            let (_p, id_ch) = enc.encode_channels(content);
            position_excluding_center(&id_ch)
        })
        .collect();

    for steps in [0usize, 1, 2, 3] {
        let mut total = 0usize;
        let mut hit = 0usize;
        let mut cand_sum = 0usize;
        let mut cand_max = 0usize;
        for q in queries {
            let query = q["query"].as_str().unwrap_or("");
            let target = q["target"].as_str().unwrap_or("");
            let Some(&ti) = key2idx.get(target) else {
                continue;
            };
            total += 1;
            let (_p, id_ch) = enc.encode_channels(query);
            let qpos = position_excluding_center(&id_ch);
            let traj = flow_trajectory(qpos, steps);
            let mut count = 0usize;
            let mut target_in = false;
            for i in 0..mems.len() {
                if traj.contains(&mem_pos[i]) {
                    count += 1;
                    if i == ti {
                        target_in = true;
                    }
                }
            }
            cand_sum += count;
            cand_max = cand_max.max(count);
            if target_in {
                hit += 1;
            }
        }
        let n = total.max(1) as f64;
        println!(
            "生流动 {} 步：平均候选 {:.1} 条（{:.1}%），最大 {} 条；目标保留 {}/{}（{:.1}%）",
            steps,
            cand_sum as f64 / n,
            cand_sum as f64 / n / mems.len() as f64 * 100.0,
            cand_max,
            hit,
            total,
            hit as f64 / n * 100.0
        );
    }
}

// ============================================================
// 编码均衡化实验：A+D（bigram + 逆频率加权）
// ============================================================

fn hash_bigram(a: char, b: char) -> usize {
    let mut h: usize = 5381;
    let mut buf = [0u8; 4];
    for ch in [a, b] {
        for &byte in ch.to_ascii_lowercase().encode_utf8(&mut buf).as_bytes() {
            h = h.wrapping_mul(33).wrapping_add(byte as usize);
        }
    }
    h
}

/// A+D：字符 bigram + 逆频率加权（无状态）。
fn identity_bigram_idf(text: &str) -> [f32; 9] {
    let chars: Vec<char> = text.chars().filter(|c| c.is_alphanumeric()).collect();
    let mut counts: HashMap<(char, char), usize> = HashMap::new();
    for w in chars.windows(2) {
        *counts.entry((w[0], w[1])).or_insert(0) += 1;
    }
    let mut features = [0.0f32; 9];
    for ((a, b), count) in counts {
        let palace = hash_bigram(a, b) % 9;
        // 逆频率：出现越多，权重越低
        let weight = 1.0 / (1.0 + (count as f32).ln());
        features[palace] += weight;
    }
    let base: f32 = features.iter().sum::<f32>() / 9.0;
    for v in features.iter_mut() {
        *v = (*v - base).max(0.0);
    }
    features
}

#[test]
#[ignore]
fn mtr_encoding_balanced() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let enc = LuoShuEncoder::new();

    let mut baseline: HashMap<usize, usize> = HashMap::new();
    let mut bigram_idf: HashMap<usize, usize> = HashMap::new();
    for m in mems {
        let content = m["content"].as_str().unwrap_or("");
        let (_p, id_ch) = enc.encode_channels(content);
        *baseline
            .entry(position_excluding_center(&id_ch))
            .or_insert(0) += 1;
        let feat = identity_bigram_idf(content);
        *bigram_idf
            .entry(position_excluding_center(&feat))
            .or_insert(0) += 1;
    }

    let total = mems.len();
    println!("============================================================");
    println!("编码均衡化实验（{} 记忆，排除中宫）", total);
    for (name, dist) in [
        ("基线（单字符）", &baseline),
        ("A+D（bigram+逆频率）", &bigram_idf),
    ] {
        let (covered, max_share, entropy) = distribution_stats(dist, total);
        let mut items: Vec<(usize, usize)> = dist.iter().map(|(a, b)| (*a, *b)).collect();
        items.sort_by(|a, b| b.1.cmp(&a.1));
        println!(
            "{}：覆盖 {}/8，最大占比 {:.1}%，熵 {:.2} | {}",
            name,
            covered,
            max_share * 100.0,
            entropy,
            items
                .iter()
                .map(|(p, c)| format!("位{}×{}", p, c))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    println!("============================================================");
}

// ============================================================
// A+D 编码的网格索引 + 连续坐标实验（语言无关）
// ============================================================

#[test]
#[ignore]
fn mtr_grid_index_ad() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");

    let mem_texts: Vec<&str> = mems
        .iter()
        .map(|m| m["content"].as_str().unwrap_or(""))
        .collect();
    let mem_bigrams: Vec<std::collections::HashSet<(char, char)>> =
        mem_texts.iter().map(|t| char_bigrams(t)).collect();
    let mem_coords: Vec<[f32; 9]> = mem_texts.iter().map(|t| identity_bigram_idf(t)).collect();
    let mem_pos: Vec<usize> = mem_coords.iter().map(position_excluding_center).collect();

    let top_k = 10usize;
    // 网格索引：同格/邻格率
    for threshold in [0u32, 1u32, 2u32] {
        let mut total_pairs = 0usize;
        let mut in_cell = 0usize;
        for a in 0..mems.len() {
            let mut sims: Vec<(usize, f32)> = (0..mems.len())
                .filter(|&b| b != a)
                .map(|b| (b, jaccard(&mem_bigrams[a], &mem_bigrams[b])))
                .collect();
            sims.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap_or(std::cmp::Ordering::Equal));
            for &(b, _) in sims.iter().take(top_k) {
                total_pairs += 1;
                let h = (mem_pos[a] ^ mem_pos[b]).count_ones();
                if h <= threshold {
                    in_cell += 1;
                }
            }
        }
        println!(
            "A+D 网格 Hamming≤{}：同格/邻格 {}/{}（{:.1}%）",
            threshold,
            in_cell,
            total_pairs,
            in_cell as f64 / total_pairs.max(1) as f64 * 100.0
        );
    }

    // 随机基线
    let mut rng_state: u64 = 20261104;
    for threshold in [0u32, 1u32, 2u32] {
        let mut hit = 0usize;
        let rand_total = 5000usize;
        for _ in 0..rand_total {
            rng_state = rng_state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let a = (rng_state >> 33) as usize % mems.len();
            rng_state = rng_state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let b = (rng_state >> 33) as usize % mems.len();
            let h = (mem_pos[a] ^ mem_pos[b]).count_ones();
            if h <= threshold {
                hit += 1;
            }
        }
        println!(
            "A+D 随机 Hamming≤{}：{}/{}（{:.1}%）",
            threshold,
            hit,
            rand_total,
            hit as f64 / rand_total as f64 * 100.0
        );
    }

    // 连续坐标：相似记忆的余弦
    let mut sim_sum = 0.0f64;
    let mut sim_count = 0usize;
    for a in 0..mems.len() {
        let mut sims: Vec<(usize, f32)> = (0..mems.len())
            .filter(|&b| b != a)
            .map(|b| (b, jaccard(&mem_bigrams[a], &mem_bigrams[b])))
            .collect();
        sims.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap_or(std::cmp::Ordering::Equal));
        for &(b, _) in sims.iter().take(top_k) {
            sim_sum += cosine9(&mem_coords[a], &mem_coords[b]) as f64;
            sim_count += 1;
        }
    }
    let avg_sim = sim_sum / sim_count.max(1) as f64;
    let mut rng_state2: u64 = 20261104;
    let mut rand_sum = 0.0f64;
    let rand_total = 5000usize;
    for _ in 0..rand_total {
        rng_state2 = rng_state2
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let a = (rng_state2 >> 33) as usize % mems.len();
        rng_state2 = rng_state2
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let b = (rng_state2 >> 33) as usize % mems.len();
        rand_sum += cosine9(&mem_coords[a], &mem_coords[b]) as f64;
    }
    let avg_rand = rand_sum / rand_total as f64;
    println!(
        "A+D 连续坐标：相似记忆平均余弦 {:.4}；随机 {:.4}；差值 {:+.4}",
        avg_sim,
        avg_rand,
        avg_sim - avg_rand
    );
}

// ============================================================
// A+B+D 编码实验：bigram + TF-IDF（语料级文档频率）
// ============================================================

/// 构建 bigram 的 IDF 表：idf = ln(N / df)。
fn build_idf(texts: &[&str]) -> HashMap<(char, char), f32> {
    let mut df: HashMap<(char, char), usize> = HashMap::new();
    for text in texts {
        let chars: Vec<char> = text.chars().filter(|c| c.is_alphanumeric()).collect();
        let mut seen: std::collections::HashSet<(char, char)> = std::collections::HashSet::new();
        for w in chars.windows(2) {
            seen.insert((w[0], w[1]));
        }
        for bg in seen {
            *df.entry(bg).or_insert(0) += 1;
        }
    }
    let n = texts.len().max(1) as f32;
    df.into_iter()
        .map(|(k, v)| (k, (n / v as f32).ln().max(0.0)))
        .collect()
}

/// A+B+D：bigram + TF-IDF。
fn identity_tfidf(text: &str, idf: &HashMap<(char, char), f32>) -> [f32; 9] {
    let chars: Vec<char> = text.chars().filter(|c| c.is_alphanumeric()).collect();
    let mut counts: HashMap<(char, char), usize> = HashMap::new();
    for w in chars.windows(2) {
        *counts.entry((w[0], w[1])).or_insert(0) += 1;
    }
    let total = counts.values().sum::<usize>().max(1) as f32;
    let mut features = [0.0f32; 9];
    for ((a, b), count) in counts {
        let tf = count as f32 / total;
        let idf_val = idf.get(&(a, b)).copied().unwrap_or(0.0);
        let palace = hash_bigram(a, b) % 9;
        features[palace] += tf * idf_val;
    }
    let base: f32 = features.iter().sum::<f32>() / 9.0;
    for v in features.iter_mut() {
        *v = (*v - base).max(0.0);
    }
    features
}

#[test]
#[ignore]
fn mtr_encoding_tfidf() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let mem_texts: Vec<&str> = mems
        .iter()
        .map(|m| m["content"].as_str().unwrap_or(""))
        .collect();
    let mem_bigrams: Vec<std::collections::HashSet<(char, char)>> =
        mem_texts.iter().map(|t| char_bigrams(t)).collect();

    let idf = build_idf(&mem_texts);
    let coords: Vec<[f32; 9]> = mem_texts.iter().map(|t| identity_tfidf(t, &idf)).collect();

    // 分布
    let mut dist: HashMap<usize, usize> = HashMap::new();
    for c in &coords {
        *dist.entry(position_excluding_center(c)).or_insert(0) += 1;
    }
    let (covered, max_share, entropy) = distribution_stats(&dist, mems.len());
    let mut items: Vec<(usize, usize)> = dist.iter().map(|(a, b)| (*a, *b)).collect();
    items.sort_by(|a, b| b.1.cmp(&a.1));
    println!("============================================================");
    println!("A+B+D（TF-IDF）实验（{} 记忆）", mems.len());
    println!(
        "分布：覆盖 {}/8，最大占比 {:.1}%，熵 {:.2} | {}",
        covered,
        max_share * 100.0,
        entropy,
        items
            .iter()
            .map(|(p, c)| format!("位{}×{}", p, c))
            .collect::<Vec<_>>()
            .join(", ")
    );

    // 相似度集中：top-10 相似记忆的余弦
    let top_k = 10usize;
    let mut sim_sum = 0.0f64;
    let mut sim_count = 0usize;
    for a in 0..mems.len() {
        let mut sims: Vec<(usize, f32)> = (0..mems.len())
            .filter(|&b| b != a)
            .map(|b| (b, jaccard(&mem_bigrams[a], &mem_bigrams[b])))
            .collect();
        sims.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap_or(std::cmp::Ordering::Equal));
        for &(b, _) in sims.iter().take(top_k) {
            sim_sum += cosine9(&coords[a], &coords[b]) as f64;
            sim_count += 1;
        }
    }
    let avg_sim = sim_sum / sim_count.max(1) as f64;
    let mut rng_state: u64 = 20261104;
    let mut rand_sum = 0.0f64;
    let rand_total = 5000usize;
    for _ in 0..rand_total {
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let a = (rng_state >> 33) as usize % mems.len();
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let b = (rng_state >> 33) as usize % mems.len();
        rand_sum += cosine9(&coords[a], &coords[b]) as f64;
    }
    let avg_rand = rand_sum / rand_total as f64;
    println!(
        "相似度集中：top-{} 相似记忆平均余弦 {:.4}；随机 {:.4}；差值 {:+.4}",
        top_k,
        avg_sim,
        avg_rand,
        avg_sim - avg_rand
    );
    println!("============================================================");
}

// ============================================================
// 编码方案对比：单字符 / bigram / bigram+轻度IDF / bigram+全IDF
// ============================================================

fn encode_scheme(text: &str, scheme: &str, idf: &HashMap<(char, char), f32>) -> [f32; 9] {
    let chars: Vec<char> = text.chars().filter(|c| c.is_alphanumeric()).collect();
    let mut features = [0.0f32; 9];
    match scheme {
        "single" => {
            for ch in &chars {
                // 与库 char_palace 一致：单字符哈希一次
                let mut h: usize = 5381;
                let mut buf = [0u8; 4];
                for &byte in ch.to_ascii_lowercase().encode_utf8(&mut buf).as_bytes() {
                    h = h.wrapping_mul(33).wrapping_add(byte as usize);
                }
                features[h % 9] += 1.0;
            }
        }
        "bigram" => {
            let mut counts: HashMap<(char, char), usize> = HashMap::new();
            for w in chars.windows(2) {
                *counts.entry((w[0], w[1])).or_insert(0) += 1;
            }
            for ((a, b), _) in counts {
                features[hash_bigram(a, b) % 9] += 1.0;
            }
        }
        "bigram_mild" => {
            let mut counts: HashMap<(char, char), usize> = HashMap::new();
            for w in chars.windows(2) {
                *counts.entry((w[0], w[1])).or_insert(0) += 1;
            }
            for ((a, b), count) in counts {
                let w = 1.0 / (1.0 + 0.3 * (count as f32).ln());
                features[hash_bigram(a, b) % 9] += w;
            }
        }
        "bigram_tfidf" => {
            let mut counts: HashMap<(char, char), usize> = HashMap::new();
            for w in chars.windows(2) {
                *counts.entry((w[0], w[1])).or_insert(0) += 1;
            }
            let total = counts.values().sum::<usize>().max(1) as f32;
            for ((a, b), count) in counts {
                let tf = count as f32 / total;
                let idf_val = idf.get(&(a, b)).copied().unwrap_or(0.0);
                features[hash_bigram(a, b) % 9] += tf * idf_val;
            }
        }
        _ => {}
    }
    let base: f32 = features.iter().sum::<f32>() / 9.0;
    for v in features.iter_mut() {
        *v = (*v - base).max(0.0);
    }
    features
}

#[test]
#[ignore]
fn mtr_encoding_schemes() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let mem_texts: Vec<&str> = mems
        .iter()
        .map(|m| m["content"].as_str().unwrap_or(""))
        .collect();
    let idf = build_idf(&mem_texts);

    // 单测里的三句话
    let t1 = "项目使用 PostgreSQL 数据库存储所有的用户数据和订单信息";
    let t2 = "项目数据库连接使用 PostgreSQL 管理用户的查询和事务处理";
    let t3 = "前端使用 React 框架构建组件化的用户界面交互体验";

    println!("============================================================");
    println!("编码方案对比（{} 记忆）", mems.len());
    // 库的真实融合编码作基线
    {
        let enc = LuoShuEncoder::new();
        let mut dist: HashMap<usize, usize> = HashMap::new();
        for t in &mem_texts {
            let c = enc.encode_text(t).values;
            *dist.entry(position_excluding_center(&c)).or_insert(0) += 1;
        }
        let (covered, max_share, entropy) = distribution_stats(&dist, mems.len());
        let v1 = enc.encode_text(t1).values;
        let v2 = enc.encode_text(t2).values;
        let v3 = enc.encode_text(t3).values;
        let sim12 = cosine9(&v1, &v2);
        let sim13 = cosine9(&v1, &v3);
        println!(
            "{:<14} 覆盖 {}/8 最大 {:.1}% 熵 {:.2} | sim12={:.3} sim13={:.3} 排序{}",
            "fused(库)",
            covered,
            max_share * 100.0,
            entropy,
            sim12,
            sim13,
            if sim12 >= sim13 { "✅" } else { "❌反转" }
        );
    }
    for scheme in ["single", "bigram", "bigram_mild", "bigram_tfidf"] {
        // 分布
        let mut dist: HashMap<usize, usize> = HashMap::new();
        for t in &mem_texts {
            let c = encode_scheme(t, scheme, &idf);
            *dist.entry(position_excluding_center(&c)).or_insert(0) += 1;
        }
        let (covered, max_share, entropy) = distribution_stats(&dist, mems.len());
        // 相似度排序
        let v1 = encode_scheme(t1, scheme, &idf);
        let v2 = encode_scheme(t2, scheme, &idf);
        let v3 = encode_scheme(t3, scheme, &idf);
        let sim12 = cosine9(&v1, &v2);
        let sim13 = cosine9(&v1, &v3);
        println!(
            "{:<14} 覆盖 {}/8 最大 {:.1}% 熵 {:.2} | sim12={:.3} sim13={:.3} 排序{}",
            scheme,
            covered,
            max_share * 100.0,
            entropy,
            sim12,
            sim13,
            if sim12 >= sim13 { "✅" } else { "❌反转" }
        );
    }
    println!("============================================================");
}

fn distribution_stats(pos_dist: &HashMap<usize, usize>, total: usize) -> (usize, f64, f64) {
    // 排除中宫（位4）
    let covered = pos_dist.keys().filter(|&&p| p != 4).count();
    let max_share = pos_dist
        .iter()
        .filter(|(p, _)| **p != 4)
        .map(|(_, c)| *c)
        .max()
        .unwrap_or(0) as f64
        / total.max(1) as f64;
    // 香农熵（8 宫，排除中宫）
    let mut entropy = 0.0f64;
    for (&p, &c) in pos_dist.iter() {
        if p == 4 {
            continue;
        }
        let pr = c as f64 / total.max(1) as f64;
        if pr > 0.0 {
            entropy -= pr * pr.log2();
        }
    }
    (covered, max_share, entropy)
}

#[test]
#[ignore]
fn mtr_encoding_balance() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/corpus_export.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let enc = LuoShuEncoder::new();

    let mut fused: HashMap<usize, usize> = HashMap::new();
    let mut identity: HashMap<usize, usize> = HashMap::new();
    let mut positional: HashMap<usize, usize> = HashMap::new();
    let mut mean_sub: HashMap<usize, usize> = HashMap::new();

    for m in mems {
        let content = m["content"].as_str().unwrap_or("");
        let vec = enc.encode_text(content);
        let (pos_ch, id_ch) = enc.encode_channels(content);

        *fused
            .entry(position_excluding_center(&vec.values))
            .or_insert(0) += 1;
        *identity
            .entry(position_excluding_center(&id_ch))
            .or_insert(0) += 1;
        *positional
            .entry(position_excluding_center(&pos_ch))
            .or_insert(0) += 1;

        // 去共模：减均值 + ReLU
        let mean: f32 = vec.values.iter().sum::<f32>() / 9.0;
        let mut ms = [0.0f32; 9];
        for i in 0..9 {
            ms[i] = (vec.values[i] - mean).max(0.0);
        }
        *mean_sub.entry(position_excluding_center(&ms)).or_insert(0) += 1;
    }

    let total = mems.len();
    println!("============================================================");
    println!("编码分布均衡实验（{} 记忆，排除中宫）", total);
    for (name, dist) in [
        ("融合向量", &fused),
        ("身份通道", &identity),
        ("位置通道", &positional),
        ("去共模融合", &mean_sub),
    ] {
        let (covered, max_share, entropy) = distribution_stats(dist, total);
        let mut items: Vec<(usize, usize)> = dist.iter().map(|(a, b)| (*a, *b)).collect();
        items.sort_by(|a, b| b.1.cmp(&a.1));
        println!(
            "{}：覆盖 {}/8，最大占比 {:.1}%，熵 {:.2} | {}",
            name,
            covered,
            max_share * 100.0,
            entropy,
            items
                .iter()
                .map(|(p, c)| format!("位{}×{}", p, c))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    println!("============================================================");
}

/// A+B+D 库内实现的压测（生产库全量）。
///
/// 压测对象是**库内真实路径**（`LuoShuEncoder::build_idf` + `set_idf` + `encode_text`），
/// 不是实验辅助函数。指标：
///   1. IDF 构建耗时与表规模；
///   2. 全量编码耗时：A+D（无 IDF）vs A+B+D（有 IDF）；
///   3. 落宫分布（覆盖 / 最大占比 / 熵）——结构指标，不测语义；
///   4. 单条编码延迟分位；
///   5. 小样本（500 条）对照，验证小样本结论是否与全量一致。
///
/// 运行：
///   $env:MTR_CORPUS="temp/v2_matrix/work/prod_corpus.json"
///   cargo test --release --test mtr_feasibility mtr_abd_pressure -- --ignored --nocapture
#[test]
#[ignore]
fn mtr_abd_pressure() {
    let path = std::env::var("MTR_CORPUS")
        .unwrap_or_else(|_| "temp/v2_matrix/work/prod_corpus.json".to_string());
    let raw = std::fs::read_to_string(&path).expect("应能读取语料 JSON");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("语料 JSON 应可解析");
    let mems = v["memories"].as_array().expect("memories 数组");
    let texts: Vec<&str> = mems
        .iter()
        .map(|m| m["content"].as_str().unwrap_or(""))
        .collect();
    let total_chars: usize = texts.iter().map(|t| t.chars().count()).sum();
    println!("============================================================");
    println!(
        "A+B+D 压测：{} 条记忆，{} 字符，平均 {:.1} 字符/条",
        texts.len(),
        total_chars,
        total_chars as f64 / texts.len().max(1) as f64
    );

    // ── 1. IDF 构建耗时（3 次取中位数）──
    let mut build_ms: Vec<f64> = Vec::new();
    let mut idf_opt = None;
    for _ in 0..3 {
        let t = std::time::Instant::now();
        let built = LuoShuEncoder::build_idf(&texts);
        build_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        idf_opt = Some(built);
    }
    build_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let idf = idf_opt.expect("IDF 应已构建");
    let idf_bytes = idf.len() * (std::mem::size_of::<((char, char), f32)>() + 16);
    println!(
        "IDF 构建：中位 {:.1} ms（3 次：{}），条目 {}，最大权重 {:.3}，估算内存 {:.1} MB",
        build_ms[1],
        build_ms
            .iter()
            .map(|x| format!("{:.1}", x))
            .collect::<Vec<_>>()
            .join(" / "),
        idf.len(),
        idf.max_weight(),
        idf_bytes as f64 / 1024.0 / 1024.0
    );

    // ── 2. 全量编码耗时：A+D vs A+B+D ──
    let enc_ad = LuoShuEncoder::new();
    let enc_abd = LuoShuEncoder::new();
    enc_abd.set_idf(idf);

    let t = std::time::Instant::now();
    let vecs_ad: Vec<_> = texts.iter().map(|t| enc_ad.encode_text(t)).collect();
    let ad_ms = t.elapsed().as_secs_f64() * 1000.0;

    let t = std::time::Instant::now();
    let vecs_abd: Vec<_> = texts.iter().map(|t| enc_abd.encode_text(t)).collect();
    let abd_ms = t.elapsed().as_secs_f64() * 1000.0;

    println!(
        "全量编码：A+D {:.1} ms（{:.1} µs/条）| A+B+D {:.1} ms（{:.1} µs/条）| 倍率 {:.2}×",
        ad_ms,
        ad_ms * 1000.0 / texts.len().max(1) as f64,
        abd_ms,
        abd_ms * 1000.0 / texts.len().max(1) as f64,
        abd_ms / ad_ms.max(1e-9)
    );

    // ── 3. 落宫分布（结构指标）──
    for (name, vecs) in [("A+D", &vecs_ad), ("A+B+D", &vecs_abd)] {
        let mut dist: HashMap<usize, usize> = HashMap::new();
        for v in vecs.iter() {
            *dist
                .entry(position_excluding_center(&v.values))
                .or_insert(0) += 1;
        }
        let (covered, max_share, entropy) = distribution_stats(&dist, vecs.len());
        println!(
            "{} 融合向量落宫：覆盖 {}/8，最大占比 {:.1}%，熵 {:.2}",
            name,
            covered,
            max_share * 100.0,
            entropy
        );
    }

    // ── 3b. 身份通道单独落宫（与实验口径对齐：验证 A+B+D 对塌缩的修复）──
    for (name, enc) in [("A+D", &enc_ad), ("A+B+D", &enc_abd)] {
        let mut dist: HashMap<usize, usize> = HashMap::new();
        for t in &texts {
            let (_, identity) = enc.encode_channels(t);
            *dist
                .entry(position_excluding_center(&identity))
                .or_insert(0) += 1;
        }
        let (covered, max_share, entropy) = distribution_stats(&dist, texts.len());
        println!(
            "{} 身份通道落宫：覆盖 {}/8，最大占比 {:.1}%，熵 {:.2}",
            name,
            covered,
            max_share * 100.0,
            entropy
        );
    }

    // ── 4. 单条延迟分位（A+B+D）──
    let mut lat: Vec<f64> = texts
        .iter()
        .map(|t| {
            let s = std::time::Instant::now();
            std::hint::black_box(enc_abd.encode_text(t));
            s.elapsed().as_secs_f64() * 1e6
        })
        .collect();
    lat.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |q: f64| lat[((lat.len().saturating_sub(1)) as f64 * q) as usize];
    println!(
        "A+B+D 单条延迟：P50 {:.1} µs，P95 {:.1} µs，P99 {:.1} µs，max {:.1} µs",
        pct(0.5),
        pct(0.95),
        pct(0.99),
        lat[lat.len() - 1]
    );

    // ── 5. 小样本（500 条）对照 ──
    let n500 = texts.len().min(500);
    let sub = &texts[..n500];
    let t = std::time::Instant::now();
    let idf500 = LuoShuEncoder::build_idf(sub);
    let build500 = t.elapsed().as_secs_f64() * 1000.0;
    let enc500 = LuoShuEncoder::new();
    enc500.set_idf(idf500);
    let mut dist500: HashMap<usize, usize> = HashMap::new();
    for t in sub {
        *dist500
            .entry(position_excluding_center(&enc500.encode_text(t).values))
            .or_insert(0) += 1;
    }
    let (c500, m500, e500) = distribution_stats(&dist500, n500);
    println!(
        "小样本 {} 条：IDF 构建 {:.1} ms，覆盖 {}/8，最大占比 {:.1}%，熵 {:.2}",
        n500,
        build500,
        c500,
        m500 * 100.0,
        e500
    );
    println!("============================================================");
}
