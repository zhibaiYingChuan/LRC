// ============================================================
// 许可证: Apache 2.0
// 锚点**表示形式**对比探针（非产品代码），属于公开层 (Layer 1)。
// ============================================================
//
// 【为什么做这个】实测反常识现象（`tests/state_semantic_whiten.rs` + TRACE）：
//   锚点「危险 困境 艰难」与**无关**记忆"和同事把方案对齐了"的余弦 0.6544，
//   却**高于**与**相关**记忆"线上报错崩溃"的 0.6041。
//   "对齐方案"凭什么比"报错崩溃"更接近"危险/困境"？
//
// 假设 H1：**锚点形式**问题。当前生产锚点是"词堆叠"（取出词典前 3 个词
//   用空格拼接，如「危险 困境 艰难」），**不是自然语言**。bge 是句向量
//   模型，对非自然句的表示质量会下降（其训练语料全是自然句）。
//   ⇒ 若换用自然句，间隔可能转正。
//
// 假设 H2：**抽象状态 ↔ 具体事件**之间存在语义鸿沟（bge 层面固有）。
//   ⇒ 若换自然句仍不可分，则须上 cross-encoder（能建模 query-doc 交互）。
//
// 本探针 = 对 H1/H2 的**决定性区分**：同一批记忆、同一编码器，
// 只改锚点的**表述形式**，看间隔符号是否改变。
//
//   $env:LRC_LUOSHU_MODEL_ID='BAAI/bge-base-zh'
//   cargo test --features server,ml --test state_anchor_form -- --ignored --nocapture
#![cfg(feature = "ml")]

use code_memory::memory_store::MemoryStore;
use code_memory::JsonPersistence;

fn ml_store(dir: &str) -> Option<MemoryStore<JsonPersistence>> {
    use code_memory::engine::luoshu_encoder_ml::{HybridLuoShuEncoder, LuoShuMlEncoder};
    let ml = LuoShuMlEncoder::load().ok()?;
    let p = JsonPersistence::new(dir).ok()?;
    Some(MemoryStore::new_with_encoder(
        p,
        HybridLuoShuEncoder::new_with_ml(ml),
    ))
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
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

#[test]
#[ignore = "需本地 bge 权重；手动运行：$env:LRC_LUOSHU_MODEL_ID='BAAI/bge-base-zh'; cargo test --features server,ml --test state_anchor_form -- --ignored --nocapture"]
fn anchor_form_comparison() {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("lrc_anchor_form_{ts}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).ok();
    let dir_str = dir.to_str().unwrap_or_default().to_string();

    let Some(store) = ml_store(&dir_str) else {
        eprintln!("[锚点形式探针] ML 编码器不可用，跳过");
        return;
    };

    // 记忆池：相关（与"危险困境"语义相关、零词面交集）+ 中庸干扰
    let related = [
        "线上报错崩溃，排查了很久才发现是空指针导致失败",
        "服务突然大量超时，日志里全是连接被拒绝",
        "部署脚本跑到一半中断，回滚也失败了",
        "数据库连接池耗尽，请求排队到超时",
    ];
    let middling = [
        "和同事把方案对齐了，双方都比较满意",
        "服务器扩容到八台，负载均衡配置也更新了",
        "把静态资源缓存起来，暂时不动这部分数据",
        "周末买了束花插在餐桌上，看着心情不错",
        "几个朋友约在咖啡馆聊了一下午，气氛很轻松",
        "机房空调检修，机柜温度需要盯着",
    ];

    // **同一语义的多种表述形式**（H1 的直接检验）
    // 前 3 个是"词堆叠"（当前生产实现），后 4 个是不同程度的自然语言化。
    let anchors: [(&str, &str); 7] = [
        ("词堆叠(当前)", "危险 困境 艰难"),
        ("词堆叠-4词", "危险 困境 艰难 陷入"),
        ("词堆叠-全", "危险 困境 艰难 陷入 失落 痛苦 泪水 迷茫"),
        ("自然句-短语", "当前状态偏向危险与困境，比较艰难"),
        (
            "自然句-描述",
            "现在的情况有点棘手，遇到了不少阻碍和困难，需要谨慎处理",
        ),
        (
            "自然句-情境",
            "最近的工作推进不太顺利，接连遇到问题，感觉有些吃力",
        ),
        (
            "自然句-任务",
            "用户当前处于困难状态，可能正在排查问题或应对棘手情况",
        ),
    ];

    // 预编码记忆（一相关/无关组各一次，供所有锚点复用，省编码时间）
    let mut rv: Vec<Vec<f32>> = Vec::new();
    for t in &related {
        if let Some(v) = store.encode_sentence_vector(t) {
            rv.push(v);
        }
    }
    let mut mv: Vec<Vec<f32>> = Vec::new();
    for t in &middling {
        if let Some(v) = store.encode_sentence_vector(t) {
            mv.push(v);
        }
    }
    println!("\n[锚点形式探针] 同一语义、不同表述 → 间隔是否转正");
    println!("{}", "=".repeat(78));

    for (label, text) in anchors {
        // 与生产同口径：加 BGE 检索指令前缀
        let instr = format!("为这个句子生成表示以用于检索相关文章：{text}");
        let Some(a) = store.encode_sentence_vector(&instr) else {
            continue;
        };
        let rmax = rv
            .iter()
            .map(|v| cosine(&a, v))
            .fold(f32::NEG_INFINITY, f32::max);
        let rmin = rv
            .iter()
            .map(|v| cosine(&a, v))
            .fold(f32::INFINITY, f32::min);
        let mmax = mv
            .iter()
            .map(|v| cosine(&a, v))
            .fold(f32::NEG_INFINITY, f32::max);
        let gap = rmin - mmax;
        // Top-1 是否命中相关组（更贴近产品实际：只看排第一的那条）
        let top1_is_related = rmax > mmax;
        println!("\n  {label:<14} 「{text}」");
        println!(
            "     相关 min={rmin:.4} max={rmax:.4} | 中庸 max={mmax:.4} → \
             间隔 {gap:+.4} {} | Top1 命中相关: {}",
            if gap > 0.0 { "✅可分" } else { "❌重叠" },
            if top1_is_related { "✅" } else { "❌" }
        );
    }

    println!("\n{}", "=".repeat(78));
    println!("判定：");
    println!("  · 若自然句形式的间隔显著优于词堆叠 → H1 成立（改锚点表述即可，成本极低）");
    println!("  · 若自然句仍全部重叠           → H2 成立（须上 cross-encoder 或改方案）");
    println!("  · 关注 Top1 命中率：产品实际只看排第一的提示，其命中比'全体间隔'更重要");
}
