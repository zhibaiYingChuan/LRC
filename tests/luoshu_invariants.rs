// ============================================================
// 参赛专用测试：洛书编码不变量验证
// ============================================================
//
// 验证洛书编码的核心数学性质，确保算法实现的正确性。
// 这些测试对应赛题"可验证性"要求，证明核心算法符合数学规范。
//
// 运行方式：
//   cargo test --features server --test luoshu_invariants

use code_memory::engine::luoshu_encoder::{EncoderStatus, LUOSHU_WEIGHTS};

// ============================================================
// 洛书九宫格数学不变量
// ============================================================

#[test]
fn test_luoshu_weights_count() {
    // 九宫格应有 9 个位置（八卦 + 中宫）
    assert_eq!(
        LUOSHU_WEIGHTS.len(),
        9,
        "洛书九宫格应有 9 个位置，实际为 {}",
        LUOSHU_WEIGHTS.len()
    );
}

#[test]
fn test_luoshu_magic_sum_invariant() {
    // 洛书九宫格幻和不变量：所有位置权重之和 = 3.0
    // 原始数字 1-9 之和 = 45，归一化后（除以 15）= 3.0
    let sum: f32 = LUOSHU_WEIGHTS.iter().sum();
    assert!(
        (sum - 3.0).abs() < 1e-6,
        "洛书权重之和应为 3.0，实际为 {}",
        sum
    );
}

#[test]
fn test_luoshu_center_weight() {
    // 中宫（位置 4）权重应为 5/15 ≈ 0.333
    let center = LUOSHU_WEIGHTS[4];
    assert!(
        (center - 5.0 / 15.0).abs() < 1e-6,
        "中宫权重应为 5/15，实际为 {}",
        center
    );
}

#[test]
fn test_luoshu_max_weight_at_south() {
    // 离卦（南，位置 1）权重应为 9/15 = 0.6（最大值）
    let south = LUOSHU_WEIGHTS[1];
    assert!(
        (south - 9.0 / 15.0).abs() < 1e-6,
        "离卦（南）权重应为 9/15，实际为 {}",
        south
    );

    // 验证是最大值
    for (i, &w) in LUOSHU_WEIGHTS.iter().enumerate() {
        if i != 1 {
            assert!(
                w < south,
                "位置 {} 的权重 {} 应小于离卦（南）的 {}",
                i,
                w,
                south
            );
        }
    }
}

#[test]
fn test_luoshu_min_weight_at_north() {
    // 坎卦（北，位置 7）权重应为 1/15（最小值）
    let north = LUOSHU_WEIGHTS[7];
    assert!(
        (north - 1.0 / 15.0).abs() < 1e-6,
        "坎卦（北）权重应为 1/15，实际为 {}",
        north
    );

    for (i, &w) in LUOSHU_WEIGHTS.iter().enumerate() {
        if i != 7 {
            assert!(
                w > north,
                "位置 {} 的权重 {} 应大于坎卦（北）的 {}",
                i,
                w,
                north
            );
        }
    }
}

#[test]
fn test_luoshu_row_sums_equal() {
    // 三行求和应相等（幻和约束）
    let w = LUOSHU_WEIGHTS;
    let row1 = w[0] + w[1] + w[2];
    let row2 = w[3] + w[4] + w[5];
    let row3 = w[6] + w[7] + w[8];

    assert!(
        (row1 - row2).abs() < 1e-6,
        "行和不等: row1={}, row2={}",
        row1,
        row2
    );
    assert!(
        (row2 - row3).abs() < 1e-6,
        "行和不等: row2={}, row3={}",
        row2,
        row3
    );
    assert!(
        (row1 - row3).abs() < 1e-6,
        "行和不等: row1={}, row3={}",
        row1,
        row3
    );
}

#[test]
fn test_luoshu_column_sums_equal() {
    // 三列求和应相等
    let w = LUOSHU_WEIGHTS;
    let col1 = w[0] + w[3] + w[6];
    let col2 = w[1] + w[4] + w[7];
    let col3 = w[2] + w[5] + w[8];

    assert!((col1 - col2).abs() < 1e-6, "列和不等");
    assert!((col2 - col3).abs() < 1e-6, "列和不等");
    assert!((col1 - col3).abs() < 1e-6, "列和不等");
}

#[test]
fn test_luoshu_diagonal_sums_equal() {
    // 两条对角线求和应相等
    let w = LUOSHU_WEIGHTS;
    let diag1 = w[0] + w[4] + w[8]; // 主对角线
    let diag2 = w[2] + w[4] + w[6]; // 副对角线

    assert!(
        (diag1 - diag2).abs() < 1e-6,
        "对角线和不等: diag1={}, diag2={}",
        diag1,
        diag2
    );
}

#[test]
fn test_luoshu_all_sums_match_magic_sum() {
    // 所有行、列、对角线的和均应等于 1.0（即 15/15 = 1.0）
    // 原始洛书每行/列/对角线和 = 15，归一化后（除以 15）= 1.0
    let w = LUOSHU_WEIGHTS;
    let expected = 1.0;

    let sums = [
        ("row1", w[0] + w[1] + w[2]),
        ("row2", w[3] + w[4] + w[5]),
        ("row3", w[6] + w[7] + w[8]),
        ("col1", w[0] + w[3] + w[6]),
        ("col2", w[1] + w[4] + w[7]),
        ("col3", w[2] + w[5] + w[8]),
        ("diag1", w[0] + w[4] + w[8]),
        ("diag2", w[2] + w[4] + w[6]),
    ];

    for (name, sum) in sums {
        assert!(
            (sum - expected).abs() < 1e-6,
            "{} 的和应为 {}，实际为 {}",
            name,
            expected,
            sum
        );
    }
}

#[test]
fn test_luoshu_opposite_positions_sum() {
    // 相对位置之和应相等（中心对称性）
    // 位置 0 ↔ 位置 8（巽 ↔ 乾）
    // 位置 1 ↔ 位置 7（离 ↔ 坎）
    // 位置 2 ↔ 位置 6（坤 ↔ 艮）
    // 位置 3 ↔ 位置 5（震 ↔ 兑）
    let w = LUOSHU_WEIGHTS;

    assert!((w[0] + w[8] - 10.0 / 15.0).abs() < 1e-6, "巽+乾 应为 10/15");
    assert!((w[1] + w[7] - 10.0 / 15.0).abs() < 1e-6, "离+坎 应为 10/15");
    assert!((w[2] + w[6] - 10.0 / 15.0).abs() < 1e-6, "坤+艮 应为 10/15");
    assert!((w[3] + w[5] - 10.0 / 15.0).abs() < 1e-6, "震+兑 应为 10/15");
}

// ============================================================
// 编码器状态测试
// ============================================================

#[test]
fn test_encoder_status_default_is_statistical() {
    // 默认编码器状态应为"统计模式"
    let status = EncoderStatus::default();
    assert_eq!(status.mode, "statistical");
    assert!(
        status.degradation_reason.is_some(),
        "统计模式应有降级原因说明"
    );
}

#[test]
fn test_encoder_status_quality_score_range() {
    // 质量评分应在 [0, 1] 范围内
    let status = EncoderStatus::default();
    assert!(
        status.quality_score >= 0.0 && status.quality_score <= 1.0,
        "质量评分应在 [0, 1] 范围内，实际为 {}",
        status.quality_score
    );
}

#[test]
fn test_encoder_status_statistical_quality_low() {
    // 统计模式的质量评分应 < 0.6（低于 ML 模式）
    let status = EncoderStatus::default();
    assert!(
        status.quality_score < 0.6,
        "统计模式质量评分应 < 0.6，实际为 {}",
        status.quality_score
    );
}
