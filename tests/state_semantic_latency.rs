// ============================================================
// 许可证: Apache 2.0
// 方向二**单条编码延迟**精测（非产品代码），属于公开层 (Layer 1)。
// ============================================================
//
// 为什么必须精测（实测驱动）：
//   吞吐粗测显示 semantic_similarities 约 2.8s/条（n=20 需 61s），
//   远超前端 8s 超时。但该数字可能混入"首次前向预热"与"线程争用"，
//   不能直接作为设计依据。本用例做**纯稳态单条**测量：
//     - 先预热一次（排除惰性初始化）
//     - 再逐条测 encode_embedding 的稳态耗时
//     - 同时打印可用并行度，判断"并发编码"能否把墙钟压下来
//
//   $env:LRC_LUOSHU_MODEL_ID='BAAI/bge-base-zh'
//   cargo test --features server,ml --test state_semantic_latency -- --nocapture
#![cfg(feature = "ml")]

use code_memory::engine::luoshu_encoder_ml::LuoShuMlEncoder;

#[test]
#[ignore = "需本地 bge 权重；手动运行：$env:LRC_LUOSHU_MODEL_ID='BAAI/bge-base-zh'; cargo test --features server,ml --test state_semantic_latency -- --ignored --nocapture"]
fn measure_single_embedding_latency() {
    let Some(enc) = LuoShuMlEncoder::load().ok() else {
        eprintln!("[延迟精测] ML 编码器不可用，跳过");
        return;
    };
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(0);
    println!("\n[延迟精测] 可用并行度 = {cores}");
    println!("{}", "=".repeat(60));

    let text = "线上报错崩溃，排查了很久才发现是空指针导致失败";

    // 预热：排除惰性初始化（首次前向通常包含内存分配/线程池创建）
    let t_warm = std::time::Instant::now();
    let _ = enc.encode_embedding(text);
    println!(
        "  首次前向（含预热）: {:>8.2}s",
        t_warm.elapsed().as_secs_f32()
    );

    // 稳态：逐条测量
    let mut times = Vec::new();
    for _ in 0..5 {
        let t = std::time::Instant::now();
        let v = enc.encode_embedding(text);
        let el = t.elapsed().as_secs_f32();
        times.push(el);
        println!(
            "  稳态前向: {:>8.3}s  维度 {}",
            el,
            v.as_ref().map(|x| x.len()).unwrap_or(0)
        );
    }
    let avg = times.iter().sum::<f32>() / times.len() as f32;
    println!("{}", "=".repeat(60));
    println!("  稳态单条均值 = {avg:.3}s");
    println!(
        "  → 8s 预算内可编码 ≈ {:.0} 条（单线程）；若并行度 {cores} 有效则 ≈ {:.0} 条",
        8.0 / avg,
        8.0 / avg * cores as f32
    );
    println!("  注：本表是**上界估计**；并发是否真提速见吞吐用例（实测未见加速）。");

    // ---- 方案二可行性：cross-encoder 的单对耗时下界 ----
    //
    // **为什么必须实测**（用户裁定里估"每条约 50-100ms、总 1-2s"）：
    //   cross-encoder 的输入是 (query + doc) **拼接后的长序列**，其前向成本
    //   不低于双编码器编码同长度文本。本机双编码器已实测 ≈2.25s/条，
    //   故 50-100ms 的估计**很可能严重偏低**。若不先测就直接选型，
    //   会实现出一个"必然超时"的重排通道。
    //
    // 本测量用**同一编码器**编码"锚点+记忆"的拼接串，作为 cross-encoder
    // 单对耗时的**下界**（cross-encoder 还要多一个分类头与更长的注意力矩阵）。
    println!("\n{}", "=".repeat(60));
    println!("[方案二可行性] 拼接(query+doc) 的编码耗时 = cross-encoder 单对下界");
    let anchor = "危险 困境 艰难";
    let doc = "线上报错崩溃，排查了很久才发现是空指针导致失败";
    let pairs = [
        format!("{anchor} {doc}"),
        format!("{anchor}。{doc}"),
        format!("为这个句子生成表示以用于检索相关文章：{anchor} {doc}"),
    ];
    let mut pair_avg = 0.0f32;
    for p in &pairs {
        let t = std::time::Instant::now();
        let _ = enc.encode_embedding(p);
        let el = t.elapsed().as_secs_f32();
        pair_avg += el;
        println!("  拼接串(len={:>3} 字) 耗时 {:.3}s", p.chars().count(), el);
    }
    pair_avg /= pairs.len() as f32;
    println!("  拼接串均值 = {pair_avg:.3}s/对（cross-encoder 单对下界）");
    for n in [3usize, 5, 10, 20] {
        let total = pair_avg * n as f32;
        println!(
            "    top-{n:<2} 重排总耗时 ≈ {total:>6.1}s  {}",
            if total + avg < 8.0 {
                "✅ 可落在 8s 预算内"
            } else {
                "❌ 超 8s 预算"
            }
        );
    }
    println!("  注：并发度 {cores}，但上文实测并发未加速（CPU 饱和），故上表按串行估。");
}
