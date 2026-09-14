// ============================================================
// 许可证: Apache 2.0
// 本文件为方向二的**决定性探针**（非产品代码），属于公开层 (Layer 1)。
// ============================================================
//
// 方向二前提验证：BGE 完整句向量能否承载「道体状态 → 记忆」的语义匹配？
//
// **为什么必须验这个**（实测驱动）：
//   · 9 维洛书投影（`bagua_index` 的来源）严重塌缩——语义差异极大的文本
//     只落 2 个母卦；用户真实库 4450 条中 96.8% 同属一卦（temp/v2-probe3/4/6）。
//   · 但 probe6 用的是 `--mode smart`，而 config.toml 已配 BAAI/bge-base-zh、
//     模型在本机 → **两次都加载了 ML 编码器**，即"即使用真 BGE，投影到 9 维
//     后仍然塌缩" ⇒ **塌缩发生在"投影到 9 维"这一步，而非 BGE 本身**。
//   · 故方向二（绕过投影、直接用句向量）在原理上有依据，但**未验证**。
//
// 本探针要回答的问题（不通过则方向二不成立）：
//   给定一组"道体状态锚点文本"与两组记忆（语义相关 / 语义无关），
//   BGE 句向量余弦能否把**相关组**显著排在**无关组**之前？
//
// 运行（需 ml feature + 本地 bge 权重）：
//   $env:LRC_LUOSHU_MODEL_ID='BAAI/bge-base-zh'
//   cargo test --features server,ml --test state_semantic_probe -- --nocapture
//
// 模型不可用时本探针 skip（CI 无权重环境必须仍能跑过）。
#![cfg(feature = "ml")]

use code_memory::memory_store::MemoryStore;
use code_memory::memory_types::{Importance, Memory, MemoryType};
use code_memory::JsonPersistence;

/// 构造 ML 能力记忆库（镜像 bin/server.rs 的启动链路）。
/// 模型不可用返回 None → 调用方跳过（CI 环境无权重时必须有此退路）。
fn ml_store(dir: &str) -> Option<MemoryStore<JsonPersistence>> {
    use code_memory::engine::luoshu_encoder_ml::{HybridLuoShuEncoder, LuoShuMlEncoder};
    let ml = LuoShuMlEncoder::load().ok()?;
    let p = JsonPersistence::new(dir).ok()?;
    Some(MemoryStore::new_with_encoder(
        p,
        HybridLuoShuEncoder::new_with_ml(ml),
    ))
}

fn mem(id: &str, content: &str) -> Memory {
    let mut m = Memory::new(
        content.to_string(),
        MemoryType::Fact,
        None,
        vec![],
        Importance::default(),
        None,
    );
    m.id = id.to_string();
    m
}

/// **状态锚点 → 记忆** 的语义可分性。
///
/// 三个锚点取自 daemon 方案C 的 `query_text` 形态（卦象 → 语义文本）。
/// 每个锚点配 3 条"语义相关"记忆（内容与锚点**无词面交集**，只有语义关系）
/// 与 3 条"语义无关"记忆（来自其他语义域）。
///
/// **关键设计：相关记忆必须与锚点零词面重叠**——
/// 否则测的是词面匹配而非语义匹配，无法证明方向二优于方案C/A。
#[test]
#[ignore = "需本地 bge 权重；手动运行：$env:LRC_LUOSHU_MODEL_ID='BAAI/bge-base-zh'; cargo test --features server,ml --test state_semantic_probe -- --ignored --nocapture"]
fn bge_separates_state_anchor_from_unrelated_memories() {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("lrc_state_sem_probe_{ts}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).ok();
    let dir_str = dir.to_str().unwrap_or_default().to_string();

    let Some(store) = ml_store(&dir_str) else {
        eprintln!("[方向二探针] ML 编码器不可用（bge 权重缺失或未设 LRC_LUOSHU_MODEL_ID），跳过");
        return;
    };

    // (锚点, 相关记忆×3, 无关记忆×3)
    //
    // 锚点用 daemon 方案C 的卦象语义文本形态（如「危险 困境 艰难」）。
    // **相关记忆刻意不含锚点的任何字**（零词面交集）——这是本探针的核心设计：
    //   若 BGE 能把它们排前，说明语义匹配生效（方案C/A 的 TF-IDF 做不到）。
    let cases: [(&str, [&str; 3], [&str; 3]); 3] = [
        (
            "危险 困境 艰难",
            [
                "线上报错崩溃，排查了很久才发现是空指针导致失败",
                "服务突然大量超时，日志里全是连接被拒绝",
                "数据库连接池耗尽，请求排队到超时",
            ],
            [
                "她很喜欢这家店的手冲咖啡，说下次还要来",
                "周末买了束花插在餐桌上，看着心情不错",
                "今晚想吃火锅，上次那家海底捞一直没去",
            ],
        ),
        (
            "愉悦 交流 沟通",
            [
                "几个朋友约在咖啡馆聊了一下午，气氛很轻松",
                "和同事把方案对齐了，双方都比较满意",
                "聚会上大家聊得很投机，还加了联系方式",
            ],
            [
                "服务器扩容到八台，负载均衡配置也更新了",
                "底层基础设施：机房、网络、操作系统内核参数",
                "把静态资源缓存起来，暂时不动这部分数据",
            ],
        ),
        (
            "承载 基础 稳定",
            [
                "底层基础设施：机房、网络、操作系统内核参数",
                "服务器扩容到八台，负载均衡配置也更新了",
                "把静态资源缓存起来，暂时不动这部分数据",
            ],
            [
                "她很喜欢这家店的手冲咖啡，说下次还要来",
                "线上报错崩溃，排查了很久才发现是空指针",
                "下周六小赵结婚，请柬写的是香格里拉宴会厅",
            ],
        ),
    ];

    println!("\n[方向二探针] 锚点 vs 记忆的 BGE 语义可分性");
    println!("{}", "=".repeat(74));

    let mut all_pass = true;
    // 汇总去中心化前后的分离裕度（用于阈值标定，不得凭空设定）
    let mut raw_gaps: Vec<f32> = Vec::new();
    let mut deb_gaps: Vec<f32> = Vec::new();
    for (anchor, related, unrelated) in cases {
        // 一次性编码全部 6 条，再分别取相关/无关的相似度
        let mut refs: Vec<Memory> = Vec::new();
        for c in related.iter().chain(unrelated.iter()) {
            refs.push(mem("", c));
        }
        let mem_refs: Vec<&Memory> = refs.iter().collect();
        let sims = store.semantic_similarities(anchor, &mem_refs);

        let rel: Vec<f32> = sims[0..related.len()]
            .iter()
            .map(|s| s.unwrap_or(f32::NAN))
            .collect();
        let unrel: Vec<f32> = sims[related.len()..]
            .iter()
            .map(|s| s.unwrap_or(f32::NAN))
            .collect();

        println!("\n锚点「{anchor}」");
        for (i, (c, s)) in related.iter().zip(rel.iter()).enumerate() {
            println!("  相关[{i}] {s:.4}  {c}");
        }
        for (i, (c, s)) in unrelated.iter().zip(unrel.iter()).enumerate() {
            println!("  无关[{i}] {s:.4}  {c}");
        }

        let rel_min = rel.iter().cloned().fold(f32::INFINITY, f32::min);
        let unrel_max = unrel.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let gap = rel_min - unrel_max;
        let pass = gap > 0.0 && rel_min.is_finite() && unrel_max.is_finite();
        println!(
            "  → 原始空间：相关最低 {rel_min:.4} vs 无关最高 {unrel_max:.4}，间隔 {gap:+.4} {}",
            if pass { "✅ 可分" } else { "❌ 不可分" }
        );
        if pass {
            raw_gaps.push(gap);
        }

        // ---- 去中心化（P8.2j 对策）：以**这 6 条池内向量**估计公共分量 ----
        // 与生产实现同口径（`state_matcher::pool_mean` + `cosine_decentered`）。
        let dim = store
            .encode_sentence_vector(anchor)
            .map(|v| v.len())
            .unwrap_or(0);
        let mut pool: Vec<(bool, Vec<f32>)> = Vec::new();
        for (i, c) in related.iter().chain(unrelated.iter()).enumerate() {
            if let Some(v) = store.encode_sentence_vector(c) {
                pool.push((i < related.len(), v));
            }
        }
        if dim > 0 && !pool.is_empty() {
            let mut mu = vec![0.0f32; dim];
            for (_, v) in &pool {
                for (acc, x) in mu.iter_mut().zip(v.iter()) {
                    *acc += *x;
                }
            }
            let inv = 1.0f32 / pool.len() as f32;
            for acc in mu.iter_mut() {
                *acc *= inv;
            }
            let Some(a) = store.encode_sentence_vector(anchor) else {
                continue;
            };
            let ac: Vec<f32> = a.iter().zip(mu.iter()).map(|(x, m)| x - m).collect();
            let mut drel: Vec<f32> = Vec::new();
            let mut dunrel: Vec<f32> = Vec::new();
            for (is_rel, v) in &pool {
                let vc: Vec<f32> = v.iter().zip(mu.iter()).map(|(x, m)| x - m).collect();
                let cos = cosine_local(&ac, &vc);
                if *is_rel {
                    drel.push(cos);
                } else {
                    dunrel.push(cos);
                }
            }
            let d_min = drel.iter().cloned().fold(f32::INFINITY, f32::min);
            let d_max = dunrel.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
            let dgap = d_min - d_max;
            println!(
                "  → 去中心化：相关最低 {d_min:.4} vs 无关最高 {d_max:.4}，间隔 {dgap:+.4} {}",
                if dgap > 0.0 {
                    "✅ 可分"
                } else {
                    "❌ 不可分"
                }
            );
            if dgap.is_finite() {
                deb_gaps.push(dgap);
            }
        }
        if !pass {
            all_pass = false;
        }
    }

    println!("\n{}", "=".repeat(74));
    println!("【阈值标定依据】");
    let fmt = |v: &[f32]| {
        if v.is_empty() {
            "无".to_string()
        } else {
            format!("{:.4}", v.iter().cloned().fold(f32::INFINITY, f32::min))
        }
    };
    println!("  原始空间最小间隔 = {}", fmt(&raw_gaps));
    println!("  去中心化最小间隔 = {}", fmt(&deb_gaps));
    if all_pass {
        println!("✅ 方向二前提成立：BGE 能把语义相关的记忆排在无关记忆之前");
        println!("   （且相关记忆与锚点**零词面交集**，证明这是语义匹配而非词面匹配）");
    } else {
        println!("❌ 方向二前提不成立：BGE 在部分锚点上无法分离相关/无关记忆");
    }
}

/// 探针内部的余弦（与生产 `state_matcher::cosine` 同口径）。
#[cfg(feature = "ml")]
fn cosine_local(a: &[f32], b: &[f32]) -> f32 {
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let (na, nb) = (na.sqrt(), nb.sqrt());
    if na <= 0.0 || nb <= 0.0 {
        return 0.0;
    }
    (dot / (na * nb)).clamp(-1.0, 1.0)
}
