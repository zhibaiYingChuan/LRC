//! v3 写入瓶颈验证：图 journal 增量写的规模与并发判据。
//!
//! 对应 `docs/LRC_推进计划_v3_结构记忆与可逆联想_2026-11-04.md` 第一部分判据：
//!   1. 库 1 万 / 10 万条，各写一条，写入 P95 不随库规模线性增长
//!   2. 64 并发写 + 同时查询，查询 P95 不超时
//!
//! 运行（默认被 `#[ignore]` 跳过，避免拖慢常规 CI）：
//!   cargo test --release --test graph_journal_bench -- --ignored --nocapture

use code_memory::graph_store::{EdgeType, GraphMemoryStore};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// 预填充 n 条边，然后测 100 次单条写入的 P50/P95（毫秒）。
fn bench_write(n: usize) -> (f64, f64) {
    let dir = tempfile::TempDir::new().expect("应创建临时目录");
    let data_dir = dir.path().to_string_lossy().to_string();
    let mut store = GraphMemoryStore::new(&data_dir);

    // 预填充：分批插入，避免逐条 O(E²)
    let batch: Vec<(String, String, EdgeType, f32)> = (0..n)
        .map(|i| {
            (
                format!("s{}", i),
                format!("t{}", i),
                EdgeType::RelatedTo,
                0.5f32,
            )
        })
        .collect();
    for chunk in batch.chunks(1000) {
        store.add_edges_batch(chunk).expect("预填充应成功");
    }
    assert_eq!(store.edge_count(), n);

    // 测单条写入延迟
    let mut lat = Vec::with_capacity(100);
    for i in 0..100 {
        let t0 = Instant::now();
        store
            .add_edge(
                &format!("x{}", i),
                &format!("y{}", i),
                EdgeType::RelatedTo,
                0.5,
            )
            .expect("写入应成功");
        lat.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    lat.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (lat[50], lat[95])
}

#[test]
#[ignore]
fn bench_write_scaling() {
    let (p50_10k, p95_10k) = bench_write(10_000);
    let (p50_100k, p95_100k) = bench_write(100_000);
    println!("[bench] 10k : p50={:.3}ms p95={:.3}ms", p50_10k, p95_10k);
    println!("[bench] 100k: p50={:.3}ms p95={:.3}ms", p50_100k, p95_100k);
    // 判据：100k 的 P95 不应超过 10k 的 3 倍（留 1ms 绝对余量吸收噪声）
    assert!(
        p95_100k < p95_10k * 3.0 + 1.0,
        "★写入 P95 随库规模线性增长：10k={:.3}ms 100k={:.3}ms",
        p95_10k,
        p95_100k
    );
}

#[test]
#[ignore]
fn bench_concurrent_write_and_query() {
    let dir = tempfile::TempDir::new().expect("应创建临时目录");
    let data_dir = dir.path().to_string_lossy().to_string();
    let mut store = GraphMemoryStore::new(&data_dir);
    // 预填充 5 万条，模拟生产规模
    let batch: Vec<(String, String, EdgeType, f32)> = (0..50_000)
        .map(|i| {
            (
                format!("s{}", i),
                format!("t{}", i),
                EdgeType::RelatedTo,
                0.5f32,
            )
        })
        .collect();
    for chunk in batch.chunks(1000) {
        store.add_edges_batch(chunk).expect("预填充应成功");
    }
    let store = Arc::new(Mutex::new(store));

    // 64 个写线程，各写 100 条
    let mut handles = Vec::new();
    for t in 0..64 {
        let s = Arc::clone(&store);
        handles.push(std::thread::spawn(move || {
            for i in 0..100 {
                let mut g = s.lock().unwrap();
                g.add_edge(
                    &format!("w{}-{}", t, i),
                    &format!("v{}-{}", t, i),
                    EdgeType::RelatedTo,
                    0.5,
                )
                .expect("并发写应成功");
            }
        }));
    }

    // 同时查询，测查询延迟
    let mut query_lat = Vec::new();
    for i in 0..200 {
        let t0 = Instant::now();
        {
            let g = store.lock().unwrap();
            let _ = g.query_edges(&format!("s{}", i * 7));
        }
        query_lat.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    for h in handles {
        h.join().expect("写线程不应 panic");
    }
    query_lat.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = query_lat[query_lat.len() / 2];
    let p95 = query_lat[(query_lat.len() as f64 * 0.95) as usize];
    println!(
        "[bench] 64 并发写 + 查询：query p50={:.3}ms p95={:.3}ms",
        p50, p95
    );
    // 判据：查询 P95 不超时（这里以 500ms 为"不超时"的保守上限）
    assert!(p95 < 500.0, "★并发写入下查询 P95 超时：{:.3}ms", p95);
}
