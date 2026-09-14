// ============================================================
// 许可证: Apache 2.0
// 根因**三分对照**探针（非产品代码），属于公开层 (Layer 1)。
// ============================================================
//
// 【为什么做这个】此前结论是"编码器对锚点↔记忆的分辨力不足"。但这个说法
// 其实把**两个完全不同的命题**混在一起：
//
//   命题 A（编码器问题）：bge 在"具体↔具体"上也不可分 ⇒ 换编码器能解决。
//   命题 B（映射问题）：bge 在"具体↔具体"上正常，只在
//        "抽象状态 ↔ 具体事件"之间不可分 ⇒ **换编码器解决不了**，
//        因为问题不在编码器，而在"道体状态语义"与"用户记忆语义"
//        之间**本就不存在稳定的对应关系**。
//
// 若命题 B 成立，则用户裁定的"方案四（换编码器）"即使隔离实施也无效。
// 故本探针必须先做这个区分，**再决定是否值得下载新模型**（成本 2GB+）。
//
// 三组对照（同编码器、同形式，只改语义关系）：
//   ① 抽象锚点 ↔ 抽象记忆（如「危险 困境」vs「处境艰难」）
//   ② 具体锚点 ↔ 具体记忆（如「线上系统报错」vs「服务超时被拒」）
//   ③ 抽象锚点 ↔ 具体记忆（当前生产用法，已知不可分）
//
// 判定：
//   · ①② 皆可分、③ 不可分 ⇒ **命题 B**（映射问题），换编码器无用。
//   · ② 即不可分            ⇒ **命题 A**（编码器问题），方案四才有意义。
//
//   $env:LRC_LUOSHU_MODEL_ID='BAAI/bge-base-zh'
//   cargo test --features server,ml --test state_gap_decompose -- --ignored --nocapture
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

/// 一组对照：锚点 + 同域记忆（相关）+ 异域记忆（无关）
struct Case {
    label: &'static str,
    anchor: &'static str,
    related: Vec<&'static str>,
    unrelated: Vec<&'static str>,
}

#[test]
#[ignore = "需本地 bge 权重；手动运行：$env:LRC_LUOSHU_MODEL_ID='BAAI/bge-base-zh'; cargo test --features server,ml --test state_gap_decompose -- --ignored --nocapture"]
fn abstract_vs_concrete_root_cause() {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("lrc_gap_{ts}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).ok();
    let dir_str = dir.to_str().unwrap_or_default().to_string();

    let Some(store) = ml_store(&dir_str) else {
        eprintln!("[根因三分探针] ML 编码器不可用，跳过");
        return;
    };

    // ---- 组①：抽象锚点 ↔ 抽象记忆 ----
    // 锚点是"状态描述"，记忆也是"状态描述"——同一抽象层次。
    let abstract_anchor = "危险 困境 艰难";
    let abstract_related = vec![
        "目前的处境比较艰难，遇到了不少阻碍",
        "情况不太妙，陷入了比较被动的局面",
        "这一关不好过，风险比较大需要谨慎",
        "处处受限，做什么都不太顺利",
    ];
    let abstract_unrelated = vec![
        "一切进展顺利，心情愉悦轻松",
        "气氛融洽，大家交流得很开心",
        "基础扎实稳定，运转一切正常",
        "局面开阔，机会很多值得把握",
    ];

    // ---- 组②：具体锚点 ↔ 具体记忆 ----
    // 锚点是"具体事件描述"，记忆也是"具体事件描述"——同一具体层次。
    // **关键**：这是 bge 训练目标最贴合的场景（其训练语料以实际检索对为主）。
    let concrete_anchor = "线上系统报错崩溃需要排查";
    let concrete_related = vec![
        "服务突然大量超时，日志里全是连接被拒绝",
        "部署脚本跑到一半中断，回滚也失败了",
        "数据库连接池耗尽，请求排队到超时",
        "接口返回 500，堆栈显示空指针异常",
    ];
    let concrete_unrelated = vec![
        "周末买了束花插在餐桌上，看着心情不错",
        "今晚想吃火锅，上次那家海底捞一直没去",
        "和同事把方案对齐了，双方都比较满意",
        "服务器扩容到八台，负载均衡配置也更新了",
    ];

    // ---- 组③：抽象锚点 ↔ 具体记忆（当前生产用法）----
    let mixed_related = concrete_related.clone();
    let mixed_unrelated = concrete_unrelated.clone();

    let cases = [
        Case {
            label: "① 抽象↔抽象",
            anchor: abstract_anchor,
            related: abstract_related,
            unrelated: abstract_unrelated,
        },
        Case {
            label: "② 具体↔具体",
            anchor: concrete_anchor,
            related: concrete_related,
            unrelated: concrete_unrelated,
        },
        Case {
            label: "③ 抽象↔具体(生产)",
            anchor: abstract_anchor,
            related: mixed_related,
            unrelated: mixed_unrelated,
        },
    ];

    println!("\n[根因三分探针] 抽象/具体层次的对应关系");
    println!("{}", "=".repeat(80));
    let mut verdict = [false; 3];
    for (i, c) in cases.iter().enumerate() {
        // 与生产同口径：加 BGE 检索指令前缀
        let instr = format!("为这个句子生成表示以用于检索相关文章：{}", c.anchor);
        let Some(a) = store.encode_sentence_vector(&instr) else {
            continue;
        };
        let mut rv = Vec::new();
        for t in &c.related {
            if let Some(v) = store.encode_sentence_vector(t) {
                rv.push(cosine(&a, &v));
            }
        }
        let mut uv = Vec::new();
        for t in &c.unrelated {
            if let Some(v) = store.encode_sentence_vector(t) {
                uv.push(cosine(&a, &v));
            }
        }
        if rv.is_empty() || uv.is_empty() {
            continue;
        }
        let rmin = rv.iter().cloned().fold(f32::INFINITY, f32::min);
        let umax = uv.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let gap = rmin - umax;
        verdict[i] = gap > 0.0;

        // **排序类指标**（对"放宽判据"方案决定性）：
        // 严格"间隔"判据要求**全部**相关 > **全部**无关，这是完美可分；
        // 但产品实际只看排在最前的少数几条，故须同时报告：
        //   · Top1 是否相关
        //   · 最佳相关项在 8 条中的排名
        //   · Top-3 中有几条相关
        let mut all_scores: Vec<(f32, bool)> = rv
            .iter()
            .map(|s| (*s, true))
            .chain(uv.iter().map(|s| (*s, false)))
            .collect();
        all_scores.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let top1_related = all_scores.first().map(|(_, r)| *r).unwrap_or(false);
        let best_related_rank = all_scores
            .iter()
            .position(|(_, r)| *r)
            .map(|p| p + 1)
            .unwrap_or(0);
        let top3_related = all_scores.iter().take(3).filter(|(_, r)| *r).count();

        println!("\n{}  锚点「{}」", c.label, c.anchor);
        for (t, s) in c.related.iter().zip(rv.iter()) {
            println!("   相关 {s:.4}  {t}");
        }
        for (t, s) in c.unrelated.iter().zip(uv.iter()) {
            println!("   无关 {s:.4}  {t}");
        }
        println!(
            "   → 严格间隔：相关最低 {rmin:.4} vs 无关最高 {umax:.4} ⇒ {gap:+.4} {}",
            if gap > 0.0 { "✅可分" } else { "❌重叠" }
        );
        println!(
            "   → 排序指标：Top1 相关={} | 最佳相关项排名={} | Top3 含相关 {}/3",
            if top1_related { "✅" } else { "❌" },
            best_related_rank,
            top3_related
        );
    }

    println!("\n{}", "=".repeat(80));
    println!("【判定】");
    println!(
        "  ① 抽象↔抽象 = {}",
        if verdict[0] { "可分" } else { "重叠" }
    );
    println!(
        "  ② 具体↔具体 = {}",
        if verdict[1] { "可分" } else { "重叠" }
    );
    println!(
        "  ③ 抽象↔具体 = {}",
        if verdict[2] { "可分" } else { "重叠" }
    );
    println!();
    if !verdict[1] {
        println!("  ⇒ 命题 A（编码器全局失效）：连\"具体↔具体\"都不可分，");
        println!("     换编码器（方案四）**有明确依据**。");
    } else if verdict[1] && !verdict[2] {
        println!("  ⇒ **失败被定位到\"抽象状态语义\"这一层**：");
        println!("     · 编码器在\"具体↔具体\"上工作正常（②可分）");
        println!("     · 但在\"抽象状态\"上失效（①③皆不可分）");
        println!();
        println!("     这**不是**\"编码器全局分辨力不足\"，而是 bge-zh 的");
        println!("     **抽象/状态类语义缺乏分辨力**（可能与训练语料以");
        println!("     具体检索对为主有关）。");
        println!();
        println!("     【对方案四的含义（须诚实区分，不得过度断言）】");
        println!("     · 方案四**未被排除**——不同编码器对抽象语义的覆盖");
        println!("       可能不同，这是训练数据层面的问题，有可能被换模型修复；");
        println!("     · 但方案四**也未获支持**——本探针只证明\"当前编码器在此");
        println!("       层面失效\"，不能推出\"换模型必然有效\"；");
        println!("     · 故正确做法：**先用新模型跑本探针**（而非先集成），");
        println!("       若 ①③ 转正再谈集成。");
        println!();
        println!("     【对\"是否用抽象状态做触发源\"的更深含义】");
        println!("     若换模型后 ①③ 仍不可分，则说明\"抽象状态\"这一**表征层次");
        println!("     本身**不适合驱动记忆匹配 —— 那时应改由行为信号主导（方案三），");
        println!("     而不是继续换模型。");
    } else {
        println!("  ⇒ 需结合 ① 的结论进一步分析。");
    }
}
