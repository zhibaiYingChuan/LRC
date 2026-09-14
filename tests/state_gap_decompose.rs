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

    // ---- 组④：**具体化的状态锚点** ↔ 具体记忆 ----
    //
    // **为什么加这一组**（它是"映射方向"假设的决定性检验）：
    //   用户裁定的三分支之一要求区分"模型不行" vs "映射方向不行"。
    //   前者需换模型验证；后者可在**当前模型**上直接验证——
    //   只需把锚点从"抽象状态词"改写为**具体的状态描述**
    //   （即把"危险 困境"翻译成"系统出了故障，需要排查和修复"）。
    //   若 ④ 转正 ⇒ 问题在"抽象→具体"这一层，但**映射方向可救**
    //   （让道体输出更具体的状态描述即可，无需换模型）。
    //   若 ④ 仍不可分 ⇒ 连具体化的状态描述都无法匹配 ⇒ 映射方向本身有问题。
    let concretized_anchor = "系统出了故障，需要排查和修复问题";
    let concretized_related = concrete_related.clone();
    let concretized_unrelated = concrete_unrelated.clone();

    // ---- 组⑤/⑥：**跨场景泛化检验**（决定 ④ 是真规律还是单点现象）----
    //
    // **为什么必须做**：④ 只测了"故障排查"一个场景。若结论不能跨场景泛化，
    // 那它只是这一条锚点的巧合，不足以支撑"具体化锚点"这条路线。
    // 故为另两个状态各写一条"具体化锚点"，看是否同样转正。
    //
    // ⑤ 场景二：顺利/愉悦（对应卦象「喜悦 交流 沟通」）
    // ⑥ 场景三：稳定/基础（对应卦象「承载 基础 稳定」）
    let s2_related = vec![
        "她很喜欢这家店的手冲咖啡，说下次还要来",
        "和同事把想法聊透了，大家都很认同",
        "聚会上大家聊得很投机，还加了联系方式",
        "给朋友写了封信，把近况都说了一遍",
    ];
    let s2_unrelated = vec![
        "服务器扩容到八台，负载均衡配置也更新了",
        "底层基础设施：机房、网络、操作系统内核参数",
        "把静态资源缓存起来，暂时不动这部分数据",
        "机房空调检修，机柜温度需要盯着",
    ];
    let s3_related = vec![
        "底层基础设施：机房、网络、操作系统内核参数",
        "服务器扩容到八台，负载均衡配置也更新了",
        "把静态资源缓存起来，暂时不动这部分数据",
        "机房空调检修，机柜温度需要盯着",
    ];
    let s3_unrelated = vec![
        "她很喜欢这家店的手冲咖啡，说下次还要来",
        "线上报错崩溃，排查了很久才发现是空指针",
        "下周六小赵结婚，请柬写的是香格里拉宴会厅",
        "和同事把方案对齐了，双方都比较满意",
    ];

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
        Case {
            label: "④ 具体化状态↔具体记忆",
            anchor: concretized_anchor,
            related: concretized_related,
            unrelated: concretized_unrelated,
        },
        // 抽象锚点（对照组：应为重叠）
        Case {
            label: "⑤a 抽象(喜悦)",
            anchor: "喜悦 交流 沟通",
            related: s2_related.clone(),
            unrelated: s2_unrelated.clone(),
        },
        // 具体化锚点（实验组：期望转正）
        Case {
            label: "⑤b 具体化(喜悦)",
            anchor: "和朋友们聊得很开心，交流气氛很好",
            related: s2_related.clone(),
            unrelated: s2_unrelated.clone(),
        },
        Case {
            label: "⑥a 抽象(承载)",
            anchor: "承载 基础 稳定",
            related: s3_related.clone(),
            unrelated: s3_unrelated.clone(),
        },
        Case {
            label: "⑥b 具体化(承载)",
            anchor: "底层设施和服务运行得很稳定，基础扎实",
            related: s3_related.clone(),
            unrelated: s3_unrelated.clone(),
        },
    ];

    println!("\n[根因三分探针] 抽象/具体层次的对应关系");
    println!("{}", "=".repeat(80));
    let mut verdict = [false; 8];
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
    println!("【设计自检】④ 与 ② 的锚点是否实质相同？");
    // **为什么必须自检**（诚实性关键）：④ 的锚点「系统出了故障，需要排查和修复问题」
    // 与 ② 的锚点「线上系统报错崩溃需要排查」在语义上高度接近，且二者
    // **记忆集完全相同**。若锚点余弦很高，则 ④ 通过的真正原因可能只是
    // "重复验证了 ②（具体↔具体）"，而**不能**证明"把抽象状态具体化有效"
    // —— 因为 ④ 里根本没用上抽象状态。
    let a2 = "为这个句子生成表示以用于检索相关文章：线上系统报错崩溃需要排查";
    let a4 = "为这个句子生成表示以用于检索相关文章：系统出了故障，需要排查和修复问题";
    if let (Some(v2), Some(v4)) = (
        store.encode_sentence_vector(a2),
        store.encode_sentence_vector(a4),
    ) {
        let sim = cosine(&v2, &v4);
        println!("  ②锚点 vs ④锚点 余弦 = {sim:.4}");
        if sim > 0.90 {
            println!("  ⇒ ⚠ 两锚点**实质相同**（>0.90）：④ 通过**不能**证明");
            println!("     \"具体化抽象状态\"有效——它只是重复了 ② 的结论。");
            println!("     **正确解读：④ 是探针设计缺陷（锚点选在了 ② 的同一语义域）**。");
        } else {
            println!("  ⇒ 两锚点可分（<0.90）：④ 的结论独立于 ②。");
        }
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
        "  ③ 抽象↔具体（生产）= {}",
        if verdict[2] { "可分" } else { "重叠" }
    );
    println!(
        "  ④ 具体化状态↔具体记忆 = {}",
        if verdict[3] { "可分" } else { "重叠" }
    );
    println!(
        "  ⑤a 抽象(喜悦) = {}",
        if verdict[4] { "可分" } else { "重叠" }
    );
    println!(
        "  ⑤b 具体化(喜悦) = {}",
        if verdict[5] { "可分" } else { "重叠" }
    );
    println!(
        "  ⑥a 抽象(承载) = {}",
        if verdict[6] { "可分" } else { "重叠" }
    );
    println!(
        "  ⑥b 具体化(承载) = {}",
        if verdict[7] { "可分" } else { "重叠" }
    );
    println!();

    // 泛化判定：三个场景的"具体化锚点"是否都优于对应"抽象锚点"
    let gen_ok = verdict[3] && verdict[5] && verdict[7];
    let abstract_fail = !verdict[0] && !verdict[2] && !verdict[4] && !verdict[6];
    println!("【泛化判定】");
    println!(
        "  三个场景的具体化锚点（④⑤b⑥b）: {}",
        if gen_ok {
            "全部可分 ✅"
        } else {
            "未全部转正 ❌"
        }
    );
    println!(
        "  四个抽象锚点（①③⑤a⑥a）: {}",
        if abstract_fail {
            "全部重叠（一致）✅"
        } else {
            "存在可分项（需复查）"
        }
    );
    println!();
    if gen_ok && abstract_fail {
        println!("  ⇒ **规律成立且可泛化**：");
        println!("     · 抽象状态词锚点 → 4/4 场景不可分");
        println!("     · 具体化状态锚点 → 3/3 场景可分");
        println!("     ⇒ 结论不是单点现象，\"具体化锚点\"是一条**可落地**的路线");
        println!("       （无需换模型、无需下载 2GB、不改 Rust 加载器）。");
    } else if verdict[3] {
        println!("  ⇒ ④ 转正但**泛化不足**：部分场景仍不可分。");
        println!("     ⇒ \"具体化锚点\"路线**可能是单点现象**，需谨慎对待。");
    } else {
        println!("  ⇒ ④ 未转正：具体化锚点亦不可分，问题在映射方向本身。");
    }
}
