// ============================================================
// 许可证: Apache 2.0
// 方向二**对策验证探针**（非产品代码），属于公开层 (Layer 1)。
// ============================================================
//
// 本探针回答两个问题（用户 2026-09-14 裁定）：
//
//   【问题一】方案一：PCA 白化能否把生产池上的"相关/无关间隔"从负变正？
//     做法：在**发现通道内独立**对向量做 PCA，移除前 k 个主成分
//     （假设它们是"领域/风格"方向而非"语义"方向），再看间隔。
//     不动检索通道（白化矩阵只从本探针的记忆池估计）。
//
//   【问题二】桥接有效性：道体的 64 维符号信号与 768 维语义空间之间，
//     是否存在**可用的桥接**？
//     做法：测 8 个母卦锚点**两两之间**的语义余弦。
//     若锚点彼此几乎不可分（余弦全部 >0.9），则说明"卦象 → 语义文本"
//     这一步本身就没有把状态差异搬进语义空间 —— 那么无论编码器多好，
//     基于语义匹配的路径都不会成功。这与"编码器分辨力"是**两个不同的根因**。
//
//   【对照】同时区分"极端无关"与"中庸无关"两类干扰项，
//     用于实证"探针可分性不能外推到生产池"这条方法论。
//
//   $env:LRC_LUOSHU_MODEL_ID='BAAI/bge-base-zh'
//   cargo test --features server,ml --test state_semantic_whiten -- --ignored --nocapture
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

/// PCA：求数据中心化后的前 `k` 个主成分（幂迭代 + 收缩）。
///
/// **为什么用幂迭代而非协方差矩阵特征分解**：维度 768，样本常只有几十条，
/// 显式构造 768×768 协方差矩阵再分解的成本与数值稳定性都不划算；
/// 幂迭代直接在"矩阵-向量乘"上工作，O(k·iters·N·dim)，且实现可审计。
fn top_components(data: &[Vec<f32>], k: usize, iters: usize) -> (Vec<f32>, Vec<Vec<f32>>) {
    let dim = data.first().map(|v| v.len()).unwrap_or(0);
    let n = data.len() as f32;
    if dim == 0 || data.is_empty() || k == 0 {
        return (vec![0.0; dim], Vec::new());
    }
    // 均值
    let mut mean = vec![0.0f32; dim];
    for v in data {
        for (m, x) in mean.iter_mut().zip(v.iter()) {
            *m += x;
        }
    }
    for m in mean.iter_mut() {
        *m /= n;
    }
    // 中心化后的残差（逐步移除主成分）
    let mut residual: Vec<Vec<f32>> = data
        .iter()
        .map(|v| v.iter().zip(mean.iter()).map(|(x, m)| x - m).collect())
        .collect();

    let mut comps: Vec<Vec<f32>> = Vec::new();
    for _ in 0..k {
        // 初始化：用方差最大的样本方向起步，避免全 1 初值在对称数据上停滞
        let mut u: Vec<f32> = residual
            .iter()
            .max_by(|a, b| {
                let na: f32 = a.iter().map(|x| x * x).sum();
                let nb: f32 = b.iter().map(|x| x * x).sum();
                na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .cloned()
            .unwrap_or_else(|| vec![1.0; dim]);
        let n0 = u.iter().map(|x| x * x).sum::<f32>().sqrt();
        if n0 <= 1e-12 {
            break;
        }
        for x in u.iter_mut() {
            *x /= n0;
        }
        for _ in 0..iters {
            // v = X^T X u = Σ_i (row_i·u) row_i
            let mut v = vec![0.0f32; dim];
            for row in &residual {
                let d: f32 = row.iter().zip(u.iter()).map(|(a, b)| a * b).sum();
                for (vi, r) in v.iter_mut().zip(row.iter()) {
                    *vi += d * r;
                }
            }
            let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm <= 1e-12 {
                break;
            }
            for x in v.iter_mut() {
                *x /= norm;
            }
            u = v;
        }
        // 收缩：从残差中移除该方向
        for row in residual.iter_mut() {
            let d: f32 = row.iter().zip(u.iter()).map(|(a, b)| a * b).sum();
            for (r, ui) in row.iter_mut().zip(u.iter()) {
                *r -= d * ui;
            }
        }
        comps.push(u);
    }
    (mean, comps)
}

/// 对向量应用"去掉前 k 个主成分"（均值已减）。
fn project_out(v: &[f32], mean: &[f32], comps: &[Vec<f32>]) -> Vec<f32> {
    let mut out: Vec<f32> = v.iter().zip(mean.iter()).map(|(x, m)| x - m).collect();
    for u in comps {
        let d: f32 = out.iter().zip(u.iter()).map(|(a, b)| a * b).sum();
        for (o, ui) in out.iter_mut().zip(u.iter()) {
            *o -= d * ui;
        }
    }
    out
}

/// 记忆池：区分"相关"、"中庸无关"（技术/日常同类句）、"极端无关"（跨域）。
struct Pool {
    texts: Vec<(&'static str, &'static str)>, // (标注, 文本)
}

fn pool_for(anchor_kind: usize) -> Pool {
    let middling = [
        ("中庸", "和同事把方案对齐了，双方都比较满意"),
        ("中庸", "服务器扩容到八台，负载均衡配置也更新了"),
        ("中庸", "把静态资源缓存起来，暂时不动这部分数据"),
        ("中庸", "周末买了束花插在餐桌上，看着心情不错"),
        ("中庸", "几个朋友约在咖啡馆聊了一下午，气氛很轻松"),
        ("中庸", "机房空调检修，机柜温度需要盯着"),
    ];
    let extreme = [
        ("极端", "今晚想吃火锅，上次那家海底捞一直没去"),
        ("极端", "量子纠缠的退相干时间与温度密切相关"),
        ("极端", "落霞与孤鹜齐飞，秋水共长天一色"),
        ("极端", "合同第七条约定违约金按日千分之三计算"),
    ];
    let related: Vec<(&'static str, &'static str)> = match anchor_kind {
        0 => vec![
            ("相关", "线上报错崩溃，排查了很久才发现是空指针导致失败"),
            ("相关", "服务突然大量超时，日志里全是连接被拒绝"),
            ("相关", "部署脚本跑到一半中断，回滚也失败了"),
            ("相关", "数据库连接池耗尽，请求排队到超时"),
        ],
        1 => vec![
            ("相关", "她很喜欢这家店的手冲咖啡，说下次还要来"),
            ("相关", "和同事把想法聊透了，大家都很认同"),
            ("相关", "聚会上大家聊得很投机，还加了联系方式"),
            ("相关", "给朋友写了封信，把近况都说了一遍"),
        ],
        _ => vec![
            ("相关", "底层基础设施：机房、网络、操作系统内核参数"),
            ("相关", "服务器扩容到八台，负载均衡配置也更新了"),
            ("相关", "把静态资源缓存起来，暂时不动这部分数据"),
            ("相关", "机房空调检修，机柜温度需要盯着"),
        ],
    };
    let mut texts = related;
    texts.extend(middling);
    texts.extend(extreme);
    Pool { texts }
}

#[test]
#[ignore = "需本地 bge 权重；手动运行：$env:LRC_LUOSHU_MODEL_ID='BAAI/bge-base-zh'; cargo test --features server,ml --test state_semantic_whiten -- --ignored --nocapture"]
fn pca_whitening_and_bridge_validity() {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("lrc_sem_whiten_{ts}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).ok();
    let dir_str = dir.to_str().unwrap_or_default().to_string();

    let Some(store) = ml_store(&dir_str) else {
        eprintln!("[白化探针] ML 编码器不可用，跳过");
        return;
    };

    let anchors = ["危险 困境 艰难", "喜悦 交流 沟通", "承载 基础 稳定"];
    println!("\n[白化探针] PCA 去主成分 + 桥接有效性");
    println!("{}", "=".repeat(78));

    for (ai, anchor) in anchors.iter().enumerate() {
        let p = pool_for(ai);
        // 编码锚点（加 BGE 检索指令，与生产同口径）+ 全部记忆
        let instr = format!("为这个句子生成表示以用于检索相关文章：{anchor}");
        let Some(a_raw) = store.encode_sentence_vector(&instr) else {
            continue;
        };
        let mut vecs: Vec<Vec<f32>> = Vec::new();
        for (_, t) in &p.texts {
            if let Some(v) = store.encode_sentence_vector(t) {
                vecs.push(v);
            }
        }
        if vecs.len() != p.texts.len() {
            eprintln!("  编码不全，跳过");
            continue;
        }

        // PCA 从"记忆池 + 锚点"整体估计（白化矩阵只在本通道内使用）
        let mut all = vecs.clone();
        all.push(a_raw.clone());
        let (mean, comps) = top_components(&all, 3, 60);

        // 四档对比（**必须各有明确语义，否则无法归因**）：
        //   raw      = 原始余弦（与生产当前行为一致）
        //   center0  = 仅减均值（不减主成分）—— 用于区分"中心化本身"与"去成分"的影响
        //   remove1/2/3 = 减均值 + 去掉前 k 个主成分（PCA 白化的核心动作）
        for (label, k, do_center) in [
            ("raw      ", 0usize, false),
            ("center-0 ", 0, true),
            ("remove-1 ", 1, true),
            ("remove-2 ", 2, true),
            ("remove-3 ", 3, true),
        ] {
            let use_c: Vec<Vec<f32>> = comps.iter().take(k).cloned().collect();
            let project = |v: &[f32]| -> Vec<f32> {
                if !do_center {
                    v.to_vec()
                } else {
                    project_out(v, &mean, &use_c)
                }
            };
            let a = project(&a_raw);
            let mut by_tag: std::collections::HashMap<&str, Vec<f32>> =
                std::collections::HashMap::new();
            for (i, (tag, _)) in p.texts.iter().enumerate() {
                let v = project(&vecs[i]);
                by_tag.entry(tag).or_default().push(cosine(&a, &v));
            }
            let fmt = |t: &str| {
                let v = by_tag.get(t).cloned().unwrap_or_default();
                if v.is_empty() {
                    (f32::NAN, f32::NAN)
                } else {
                    (
                        v.iter().cloned().fold(f32::INFINITY, f32::min),
                        v.iter().cloned().fold(f32::NEG_INFINITY, f32::max),
                    )
                }
            };
            let (rmin, _) = fmt("相关");
            let (_, mmax) = fmt("中庸");
            let (_, emax) = fmt("极端");
            let gap_m = rmin - mmax;
            let gap_e = rmin - emax;
            println!("\n锚点[{ai}] 「{anchor}」 {label}：");
            println!(
                "   相关最低 {rmin:+.4} | 中庸最高 {mmax:+.4} → 间隔 {gap_m:+.4} {}",
                if gap_m > 0.0 {
                    "✅可分"
                } else {
                    "❌重叠"
                }
            );
            println!(
                "   相关最低 {rmin:+.4} | 极端最高 {emax:+.4} → 间隔 {gap_e:+.4} {}",
                if gap_e > 0.0 {
                    "✅可分"
                } else {
                    "❌重叠"
                }
            );
        }
    }

    // ---- 桥接有效性：8 个母卦锚点两两余弦 ----
    println!("\n{}", "=".repeat(78));
    println!("【桥接有效性】8 母卦锚点在语义空间中的两两余弦");
    let bagua_anchors = [
        "领导 权威 权力", // 乾
        "喜悦 交流 沟通", // 兑
        "温暖 光明 明亮", // 离
        "启动 开始 出发", // 震
        "渗透 传播 散布", // 巽
        "危险 困境 艰难", // 坎
        "停止 阻碍 阻挡", // 艮
        "包容 承载 柔顺", // 坤
    ];
    let names = ["乾", "兑", "离", "震", "巽", "坎", "艮", "坤"];
    let mut av: Vec<Vec<f32>> = Vec::new();
    for a in &bagua_anchors {
        let instr = format!("为这个句子生成表示以用于检索相关文章：{a}");
        if let Some(v) = store.encode_sentence_vector(&instr) {
            av.push(v);
        }
    }
    if av.len() == 8 {
        let mut min_c = f32::INFINITY;
        let mut max_c = f32::NEG_INFINITY;
        print!("       ");
        for n in &names {
            print!("{n:>7}");
        }
        println!();
        for i in 0..8 {
            print!("  {:<4}", names[i]);
            for j in 0..8 {
                let c = cosine(&av[i], &av[j]);
                if i != j {
                    min_c = min_c.min(c);
                    max_c = max_c.max(c);
                }
                print!("{c:>7.3}");
            }
            println!();
        }
        println!("\n  两两余弦范围：[{min_c:.3}, {max_c:.3}]");
        let verdict = if min_c > 0.90 {
            "❌ 8 个状态锚点彼此几乎不可分（余弦全部 >0.90）"
        } else if min_c > 0.75 {
            "⚠ 锚点间区分度有限"
        } else {
            "✅ 锚点在语义空间中有可分辨的距离"
        };
        println!("  → 桥接有效性判定：{verdict}");
        println!("     含义：若最小余弦很高，说明\"卦象→语义文本\"这一步没有把");
        println!("     状态差异搬进语义空间 —— 该根因与\"编码器分辨力\"不同。");
    }
    println!("\n{}", "=".repeat(78));
    println!("注：内存 768 维、维度高，PCA 去主成分后余弦量纲会整体变化，");
    println!("    故只看**间隔符号**（是否从负变正），不比较绝对值。");
}
