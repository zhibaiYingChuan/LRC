// ============================================================
// Loong Recall 独立基准测试工具
// ============================================================
//
// 这是一个独立的可执行文件，用于运行 LRC 的三层基准测试。
// 任何系统都可以使用这套测试框架进行评测——不仅是 LRC 自身。
//
// 用法:
//   code-memory-benchmark                    # 运行所有基准测试
//   code-memory-benchmark --json             # 输出 JSON 格式（机器可读）
//   code-memory-benchmark --layer 1          # 仅运行第一层（通用检索）
//   code-memory-benchmark --layer 2          # 仅运行第二层（高级记忆能力）
//   code-memory-benchmark --layer 3          # 仅运行第三层（综合能力与信任）
//   code-memory-benchmark --help             # 查看帮助
//
// 道枢映射：中宫（五）— 统摄八方，基准测试如中宫之统摄
// ============================================================

use code_memory::benchmark;
use std::process;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut json_mode = false;
    let mut target_layer: Option<u8> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => json_mode = true,
            "--layer" => {
                i += 1;
                if i < args.len() {
                    target_layer = args[i].parse().ok();
                }
            }
            "--help" | "-h" => {
                print_help();
                return;
            }
            _ => {}
        }
        i += 1;
    }

    if !json_mode {
        println!("═══════════════════════════════════════════════════════════");
        println!(
            "  Loong Recall 三层基准测试工具 v{}",
            env!("CARGO_PKG_VERSION")
        );
        println!("═══════════════════════════════════════════════════════════");
        println!();
        println!("  本工具对 LRC 进行三层基准测试，所有结果均可由第三方独立复现。");
        println!("  任何其他记忆系统也可以使用本工具进行对比评测。");
        println!();
        println!("  第一层：通用记忆检索基准（对标业界，证明不输于人）");
        println!("  第二层：高级记忆能力基准（公平版：测能力，不测架构）");
        println!("  第三层：综合能力与信任基准（公平版：测能力，不测架构）");
        println!();
        println!("  运行中...");
        println!();
    }

    let total_start = Instant::now();
    let report = benchmark::run_all_benchmarks(target_layer);
    let total_ms = total_start.elapsed().as_millis() as u64;

    if json_mode {
        // JSON 输出（机器可读）
        let json_str = serde_json::to_string_pretty(&serde_json::json!({
            "report_version": report.version,
            "generated_at": report.generated_at,
            "total_duration_ms": total_ms,
            "summary": {
                "total": report.total,
                "passed": report.passed,
                "failed": report.failed,
                "status": if report.failed == 0 { "PASS" } else { "FAIL" },
                "layers": report.layers.iter().map(|l| serde_json::json!({
                    "name": l.name,
                    "total": l.total,
                    "passed": l.passed,
                    "status": l.status,
                })).collect::<Vec<_>>(),
            },
            "radar_chart": report.radar_scores,
            "results": report.results.iter().map(|r| serde_json::json!({
                "name": r.name,
                "layer": r.layer,
                "description": r.description,
                "industry_problem": r.industry_problem,
                "passed": r.passed,
                "score": r.score,
                "details": r.details,
                "duration_ms": r.duration_ms,
            })).collect::<Vec<_>>(),
        }))
        .unwrap_or_else(|e| {
            eprintln!("JSON 序列化失败: {e}");
            process::exit(1);
        });
        println!("{json_str}");
    } else {
        // 人类可读输出
        for r in &report.results {
            let status = if r.passed { "✓ 通过" } else { "✗ 失败" };
            println!(
                "  [{}] L{}-{} — {} ({:.1}ms)",
                status, r.layer, r.name, r.description, r.duration_ms
            );
            println!("         评分: {:.2} | {}", r.score, r.details);
        }

        println!();
        println!("═══════════════════════════════════════════════════════════");
        println!("  基准测试报告");
        println!("═══════════════════════════════════════════════════════════");
        println!();
        println!(
            "  总计: {} 项 | 通过: {} | 失败: {} | 耗时: {}ms",
            report.total, report.passed, report.failed, total_ms
        );
        println!();
        for layer in &report.layers {
            let status_icon = if layer.status == "PASS" { "✓" } else { "✗" };
            println!(
                "  {} {}: {}/{} 通过",
                status_icon, layer.name, layer.passed, layer.total
            );
        }
        println!();
        println!("  雷达图数据（标准化评分 0.0~1.0）：");
        if let serde_json::Value::Object(map) = &report.radar_scores {
            for (key, val) in map {
                let bar_len = (val.as_f64().unwrap_or(0.0) * 20.0) as usize;
                let bar = "█".repeat(bar_len.min(20));
                println!(
                    "    {:<16} [{:<20}] {:.2}",
                    key,
                    bar,
                    val.as_f64().unwrap_or(0.0)
                );
            }
        }
        println!();
        if report.failed == 0 {
            println!("  ✓ 所有基准测试通过！LRC 在全部三个层面均表现优异。");
        } else {
            println!("  ⚠ {} 项测试未通过，请查看上方详情。", report.failed);
        }
        println!();
        println!("  提示：使用 --json 参数获取机器可读的 JSON 格式报告。");
        println!("  提示：使用 --layer 1/2/3 单独运行某一层测试。");
        println!();
        println!("  所有测试均可由第三方复现。一行命令：");
        println!("    cargo run --bin code-memory-benchmark --features server");
        println!();
    }

    if report.failed > 0 {
        process::exit(1);
    }
}

fn print_help() {
    println!("Loong Recall 基准测试工具 v{}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("用法: code-memory-benchmark [选项]");
    println!();
    println!("选项:");
    println!("  --json              输出 JSON 格式（机器可读，便于 CI/CD 集成）");
    println!("  --layer <1|2|3>     仅运行指定层的测试");
    println!("  --help, -h          显示此帮助信息");
    println!();
    println!("说明:");
    println!("  本工具对 Loong Recall 进行三层基准测试：");
    println!("  第一层：通用记忆检索基准（对标业界，证明不输于人）");
    println!("  第二层：高级记忆能力基准（公平版：测能力，不测架构）");
    println!("  第三层：综合能力与信任基准（公平版：测能力，不测架构）");
    println!();
    println!("  任何其他记忆系统也可使用本工具进行对比评测。");
    println!("  所有测试均可由第三方在一行命令中复现。");
    println!();
    println!("举个栗子:");
    println!("  # 运行所有基准测试");
    println!("  code-memory-benchmark");
    println!();
    println!("  # 输出 JSON 格式（供 CI/CD 或仪表盘使用）");
    println!("  code-memory-benchmark --json");
    println!();
    println!("  # 仅测试高级记忆能力（第二层）");
    println!("  code-memory-benchmark --layer 2");
    println!();
    println!("  # 用 cargo run 直接运行");
    println!("  cargo run --bin code-memory-benchmark --features server");
}
