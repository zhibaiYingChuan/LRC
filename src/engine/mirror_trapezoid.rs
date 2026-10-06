// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件包含模型底层架构衍生的核心算法，受研究许可证保护。
// 禁止逆向工程、禁止商业再分发、禁止用于训练竞争模型。
// ============================================================
//
// 镜像梯形递归算子引擎（M.T.R. Operator Engine）
//
// 实现宇宙第一定律的四个基本操作：
//   分类 (MirrorProject)  — 先天八卦投影，自动判断记忆类别
//   选择 (TrapezoidFocus) — 梯形兴趣区域检索，对数级复杂度
//   组合 (RecursiveCompose) — 门控融合，多条记忆合成为抽象知识
//   拆解 (RecursiveUnfold) — 可逆展开，抽象记忆还原为具体步骤

use super::luoshu_encoder::LuoShuVector;

/// 先天八卦投影结果（MirrorProject 的输出）
#[derive(Debug, Clone)]
pub struct BaguaProjection {
    /// 投影到每个八卦基底的内积值（8 维）
    pub scores: [f32; 8],
    /// 最匹配的八卦索引（0-7）
    pub best_index: usize,
    /// 最匹配的八卦名称
    pub best_name: &'static str,
    /// 最匹配的类别含义
    pub best_category: &'static str,
}

/// 梯形兴趣区域（TrapezoidFocus 的输入）
#[derive(Debug, Clone)]
pub struct TrapezoidROI {
    /// 九宫格中梯形的四个顶点索引（0-8）
    pub vertices: [usize; 4],
    /// 递归细分深度（0 = 不分，1 = 分 4 子区，2 = 分 16 子区…）
    pub depth: u32,
}

/// 梯形聚焦检索结果：包含子区域索引和对应向量
#[derive(Debug, Clone)]
pub struct TrapezoidFocusResult {
    /// 最佳匹配的子区域索引
    pub best_region: usize,
    /// 子区域内的向量索引列表
    pub matched_indices: Vec<usize>,
    /// 子区域覆盖率（0.0 ~ 1.0）
    pub coverage: f32,
    /// 递归细分路径
    pub subdivision_path: Vec<usize>,
}

/// 合成结果
#[derive(Debug, Clone)]
pub struct ComposeResult {
    /// 合成后的洛书向量
    pub vector: LuoShuVector,
    /// 合成置信度（0.0 - 1.0）
    pub confidence: f32,
    /// 各源向量的融合权重
    pub weights: Vec<f32>,
    /// 信息增量（质疑二：防止模式坍塌）
    /// 合成向量与源向量的平均余弦距离。
    /// 过低（< 0.05）表示合成没有产生新信息，只是冗余压缩，
    /// 此时应阻止合成以防止记忆空间趋向少数抽象节点。
    pub information_gain: f32,
}

/// 拆解结果
#[derive(Debug, Clone)]
pub struct UnfoldResult {
    /// 拆解出的子向量列表
    pub sub_vectors: Vec<LuoShuVector>,
    /// 每个子向量的重构权重
    pub sub_weights: Vec<f32>,
    /// 重构保真度（展开再组合后与原向量的相似度）
    pub fidelity: f32,
}

// ============================================================
// 八卦基底常量
// ============================================================

/// 先天八卦基底向量（8 个方向，每个 9 维）
pub const BAGUA_BASES: [[f32; 9]; 8] = [
    // 乾（西北/天）：位置 8 为主
    [0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.9],
    // 兑（西/泽）：位置 5 为主
    [0.1, 0.1, 0.1, 0.1, 0.1, 0.9, 0.1, 0.1, 0.1],
    // 离（南/火）：位置 1 为主
    [0.1, 0.9, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1],
    // 震（东/雷）：位置 3 为主
    [0.1, 0.1, 0.1, 0.9, 0.1, 0.1, 0.1, 0.1, 0.1],
    // 巽（东南/风）：位置 0 为主
    [0.9, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1],
    // 坎（北/水）：位置 7 为主
    [0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.9, 0.1],
    // 艮（东北/山）：位置 6 为主
    [0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.9, 0.1, 0.1],
    // 坤（西南/地）：位置 2 为主
    [0.1, 0.1, 0.9, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1],
];

/// 八卦名称
pub const BAGUA_NAMES: [&str; 8] = [
    "乾·天", "兑·泽", "离·火", "震·雷", "巽·风", "坎·水", "艮·山", "坤·地",
];

/// 将 8 母卦名（如道体 `daoti_preview_bagua` 写入的单字 "乾"/"坎"）
/// 映射到 MirrorProject 八卦索引（0-7）。
///
/// 必须按名称映射而非按位置：道体 GUA_LEXICON 字典顺序
/// （乾,兑,坤,艮,震,巽,坎,离）与 `BAGUA_NAMES` 顺序
/// （乾,兑,离,震,巽,坎,艮,坤）不同——按位置映射会错位导致跨域污染。
///
/// 匹配规则：取名称首字符与 BAGUA_NAMES 首字符相同者（"乾·天".starts_with("乾")）。
pub fn bagua_name_to_index(name: &str) -> Option<u8> {
    let first_char = name.trim().chars().next()?;
    BAGUA_NAMES
        .iter()
        .position(|canonical| canonical.starts_with(first_char))
        .map(|index| index as u8)
}

/// 八卦环形距离（先天八卦圆环上的最短步数，取值 0..=4）。
///
/// 先天八卦按 `BAGUA_BASES` 顺序首尾相接围成一圈
/// （乾→兑→离→震→巽→坎→艮→坤→乾），卦与卦的"邻近度"必须沿圆环度量，
/// 而非线性索引差：例如 乾(0) 与 坤(7) 线性距离为 7，环形距离却是 1，
/// 二者在圆环上实为相邻卦。
///
/// 检索门禁据此剪除候选：环形距离 ≤1（同卦/相邻卦）保留，其余剪除。
pub fn bagua_ring_distance(a: u8, b: u8) -> u8 {
    let diff = (a as i16 - b as i16).unsigned_abs() as i16; // 0..=7
    diff.min(8 - diff) as u8 // 0..=4
}

/// 八卦索引 → 洛书九宫位置（与 `BAGUA_BASES` 严格同序）
///
/// v0.9.10 新增。此前的实现把「卦 index」与「九宫位置」混用（`mirror_project`
/// 由基底内积得到 index，却当成位置使用），两处顺序一旦漂移就会静默错位。
/// 现在显式固化，任何改动都必须与 `BAGUA_BASES` 同步。
///
/// 对应关系（由 `BAGUA_BASES` 逐一核出）：
///   乾→8  兑→5  离→1  震→3  巽→0  坎→7  艮→6  坤→2
///
/// 注意：**外圈 8 个位置各出现一次，唯独没有 4（中心）**——这正是
/// 「中心点始终不变」（中心不属于任何梯形）的体现。
pub const BAGUA_PALACE_POS: [usize; 8] = [8, 5, 1, 3, 0, 7, 6, 2];

/// 洛书九宫的中心位置。中心不参与螺旋轮转，只作为不变量存在。
pub const LUOSHU_CENTER_POS: usize = 4;

/// 八卦的先天类别含义（用于记忆分类）
pub const BAGUA_CATEGORIES: [&str; 8] = [
    "刚性法则", // 乾 — 核心规则、架构约束
    "愉悦表达", // 兑 — 用户偏好、界面交互
    "依附关联", // 离 — 依赖关系、调用链
    "变动触发", // 震 — 事件驱动、变更记录
    "渗透影响", // 巽 — 外部 API、环境配置
    "陷溺困境", // 坎 — Bug 修复、错误处理
    "止息积累", // 艮 — 静态资源、缓存数据
    "承载基础", // 坤 — 基础设施、底层模块
];

// ============================================================
// 五行生克偏置（逐字移植道体 `MirrorRecursiveCell`）
// ============================================================
//
// 出处：`12_记忆层与镜像递归_part3/modules/daoti_modules_part3.py:30-35, 496-536`
//
// 道体原式（`MirrorRecursiveCell.forward`）：
//   gua_scores  = gua_query(state) * gua_scale
//   top1_wuxing = gua_wuxing_idx[argmax(gua_scores)]   ← 先定主卦，取其五行
//   gua_scores += wuxing_sheng_bias[top1_wuxing] * sheng_scale
//   gua_scores += wuxing_ke_bias[top1_wuxing] * (-|ke_scale_raw|)
//   gua_scores  = clamp(-8, 8) → softmax
//
// 偏置表（`daoti_modules_part3.py:507-517`）：
//   同行   → 0.3
//   我生者 → +1.0
//   我克者 → +1.0，随后乘 -|ke_scale_raw|（默认 0.5 ⇒ 实际 −0.5）
//   其余   → 0
//
// **这是 `MirrorRecursiveCell` 里唯一能确定性移植到 LRC 的部分** —— 它是纯符号
// 先验、不含任何可学习权重。其余部分（`yang_net` / `yin_net` / `gua_prototypes`）
// 为随机初始化 + 在线学习，**无预训练权重**，移植到 Rust 侧等于注入噪声。

/// 八卦 → 五行索引（与 `BAGUA_NAMES` / 道体 `BAGUA_ORDER` **同序**）
///
/// 已核对：道体 `BAGUA_ORDER = ["乾","兑","离","震","巽","坎","艮","坤"]` 与
/// LRC 的 `BAGUA_NAMES` 顺序**完全一致**，故索引可直接对齐，无需名称转换。
/// 五行编码沿用道体 `WUXING_MAP = {金:0, 木:1, 水:2, 火:3, 土:4}`。
pub const BAGUA_WUXING: [usize; 8] = [0, 0, 3, 1, 1, 2, 4, 4];
//                                  乾 兑 离 震 巽 坎 艮 坤
//                                  金 金 火 木 木 水 土 土

/// 相生：我生者（道体 `WUXING_SHENG`：金生水、木生火、水生木、火生土、土生金）
pub const WUXING_SHENG: [usize; 5] = [2, 3, 0, 4, 1];

/// 相克：我克者（道体 `WUXING_KE`：金克木、木克土、水克火、火克金、土克水）
pub const WUXING_KE: [usize; 5] = [1, 4, 3, 0, 2];

/// 五行索引 → 名称（日志与排错用）
pub const WUXING_NAMES: [&str; 5] = ["金", "木", "水", "火", "土"];

/// 生项尺度默认值（对应道体 `sheng_scale = 1.0`）
pub const DEFAULT_SHENG_SCALE: f32 = 1.0;
/// 克项尺度默认值（对应道体 `ke_scale_raw = 0.5`，实际施加 −0.5）
pub const DEFAULT_KE_SCALE: f32 = 0.5;

/// 8 维取 argmax（平票取小索引，与 numpy / torch argmax 一致）
pub fn argmax8(v: &[f32; 8]) -> usize {
    let mut best = 0usize;
    for (i, &x) in v.iter().enumerate().skip(1) {
        if x > v[best] {
            best = i;
        }
    }
    best
}

/// 由主卦求对全部 8 卦的五行偏置（逐字移植道体偏置表）
pub fn wuxing_bias(dominant: usize, sheng_scale: f32, ke_scale: f32) -> [f32; 8] {
    let dw = BAGUA_WUXING[dominant.min(7)];
    let mut bias = [0.0f32; 8];
    for (j, b) in bias.iter_mut().enumerate() {
        let w = BAGUA_WUXING[j];
        if w == dw {
            *b = 0.3; // 同行
        } else if WUXING_SHENG[dw] == w {
            *b = sheng_scale; // 我生者
        } else if WUXING_KE[dw] == w {
            *b = -ke_scale.abs(); // 我克者
        }
    }
    bias
}

/// 施加五行生克偏置后的主卦索引
///
/// 对应道体 `argmax(gua_scores + sheng_bias + ke_bias)`：
/// 先取原始主卦 → 由其五行求偏置 → 再取一次 argmax（偏置可能改变主卦）。
pub fn dominant_with_wuxing(scores: &[f32; 8], sheng_scale: f32, ke_scale: f32) -> usize {
    let raw = argmax8(scores);
    let bias = wuxing_bias(raw, sheng_scale, ke_scale);
    let mut biased = [0.0f32; 8];
    for j in 0..8 {
        biased[j] = scores[j] + bias[j];
    }
    argmax8(&biased)
}

// ============================================================
// 操作 1：MirrorProject — 先天八卦分类
// ============================================================

// 镜像投影算子：将洛书向量投影到 8 个先天八卦基底上

#[cfg(test)]
mod wuxing_tests {
    use super::*;

    // === 五行生克偏置（移植自道体 MirrorRecursiveCell） ===

    /// 逐字核对移植自道体的两张表
    #[test]
    fn test_wuxing_tables_match_daoti() {
        // 道体 WUXING_MAP = {金:0, 木:1, 水:2, 火:3, 土:4}
        assert_eq!(WUXING_NAMES, ["金", "木", "水", "火", "土"]);
        // 道体 WUXING_SHENG = {0:2, 1:3, 2:0, 3:4, 4:1}
        assert_eq!(WUXING_SHENG, [2, 3, 0, 4, 1], "相生表必须与道体逐字一致");
        // 道体 WUXING_KE = {0:1, 1:4, 2:3, 3:0, 4:2}
        assert_eq!(WUXING_KE, [1, 4, 3, 0, 2], "相克表必须与道体逐字一致");
        // 道体 BAGUA_WUXING：乾兑=金、震巽=木、坎=水、离=火、艮坤=土
        // （索引顺序 = BAGUA_NAMES = 道体 BAGUA_ORDER）
        assert_eq!(
            BAGUA_WUXING,
            [0, 0, 3, 1, 1, 2, 4, 4],
            "八卦五行表必须与道体一致"
        );
    }

    /// 偏置取值：同行 0.3 / 我生者 +1.0 / 我克者 −0.5 / 其余 0
    ///
    /// 以乾（索引 0，金）为主卦：
    ///   同行 乾/兑 → 0.3
    ///   金生水 → 坎(索引 5) → +1.0
    ///   金克木 → 震/巽(索引 3/4) → −0.5
    ///   火/土 → 0
    #[test]
    fn test_wuxing_bias_values() {
        let b = wuxing_bias(0, DEFAULT_SHENG_SCALE, DEFAULT_KE_SCALE);
        let expect = [0.3, 0.3, 0.0, -0.5, -0.5, 1.0, 0.0, 0.0];
        for j in 0..8 {
            assert!(
                (b[j] - expect[j]).abs() < 1e-6,
                "索引 {} 的偏置应为 {}，实际 {}",
                j,
                expect[j],
                b[j]
            );
        }
    }

    /// 偏置确实能改变主卦（对应道体 `argmax(scores + sheng + ke)`）
    #[test]
    fn test_dominant_with_wuxing_can_change_choice() {
        // 原始主卦 = 乾（金）
        let scores = [0.9, 0.2, 0.1, 0.1, 0.1, 0.5, 0.1, 0.1];
        assert_eq!(argmax8(&scores), 0, "原始主卦应为乾");

        // 金生水 ⇒ 坎(5) 得 +1.0 → 1.5 > 0.9，主卦被改写
        let biased = dominant_with_wuxing(&scores, DEFAULT_SHENG_SCALE, DEFAULT_KE_SCALE);
        assert_eq!(biased, 5, "相生偏置应把主卦从乾改写为坎（金生水）");

        // 若关闭相生（sheng_scale = 0），主卦保持乾
        let no_sheng = dominant_with_wuxing(&scores, 0.0, DEFAULT_KE_SCALE);
        assert_eq!(no_sheng, 0, "关闭相生后不应改写主卦");
    }
}

// ============================================================
// 操作 1：MirrorProject — 先天八卦分类
// ============================================================

/// 镜像投影算子：将洛书向量投影到 8 个先天八卦基底上
///
/// 算法（v0.9.10 起含两层）：
/// 1. 对每个八卦基底计算与洛书向量的内积 → 原始匹配度
/// 2. **五行生克偏置**（移植自道体 `MirrorRecursiveCell`）：
///    先由原始匹配度定主卦 → 取其五行 → 对 8 卦施加 同行(0.3)/相生(+1.0)/
///    相克(−0.5) 偏置 → 重新取主卦。
///
/// 第 2 层把**符号因果**引入分类：主卦牵动其所生者、压制其所克者。这是道体
/// `MirrorRecursiveCell` 里唯一不依赖可学习权重的部分，因此可以确定性移植。
/// 返回的 `scores` 是**加偏置后**的得分，保证恒有 `best_index == argmax(scores)`。
///
/// 复杂度：O(8 × 9) = O(1)
pub fn mirror_project(vector: &LuoShuVector) -> BaguaProjection {
    // 1) 原始匹配度（内积）
    let mut raw = [0.0f32; 8];
    for i in 0..8 {
        raw[i] = vector
            .values
            .iter()
            .zip(BAGUA_BASES[i].iter())
            .map(|(a, b)| a * b)
            .sum();
    }

    // 2) 五行生克偏置
    let anchor = argmax8(&raw);
    let bias = wuxing_bias(anchor, DEFAULT_SHENG_SCALE, DEFAULT_KE_SCALE);
    let mut scores = [0.0f32; 8];
    for i in 0..8 {
        scores[i] = raw[i] + bias[i];
    }
    let best_index = argmax8(&scores);

    BaguaProjection {
        scores,
        best_index,
        best_name: BAGUA_NAMES[best_index],
        best_category: BAGUA_CATEGORIES[best_index],
    }
}

/// 将洛书向量分类到最匹配的八卦基底（简化接口）
pub fn classify(vector: &LuoShuVector) -> (&'static str, &'static str) {
    let proj = mirror_project(vector);
    (proj.best_name, proj.best_category)
}

// ============================================================
// 操作 0：镜像双梯形螺旋递归（符号/几何路线，无可学习权重）
// ============================================================
//
// 这是「镜像双梯形螺旋递归」在 LRC 本体中的符号/几何落地。
// 与神经网络设计稿（PyTorch `BiTrapezoidRecurrent`）的对应关系：
//
//   设计稿                            本实现
//   ────────────────────────────────  ─────────────────────────────
//   可学习线性层 W_f / W_b            固定几何算子（折叠/展开/镜像/螺旋）
//   tanh(W·concat(a,b) + b)           非负清理 + L2 单位化 + α 混合
//   T 次全层同步迭代                  同（每轮先正向后逆向，各自读旧状态）
//   最宽层逆向状态 h_b[0] 作输出       同
//
// 设计稿的三处实质缺陷在本实现中被显式处理：
//   1) 输入被首轮递归冲掉 → `MirrorRecursionConfig::re_inject`（默认 true）
//   2) b_f / b_b 是死参数 → 符号路线无偏置参数
//   3) 2 维瓶颈不可逆 → 本实现的瓶颈是「对称折叠」而非线性压缩，信息损失
//      发生在折叠时刻，展开只能复原「成对相等」的分量。
//
// 诚实的边界声明：本算子是**确定性固定参数映射**，做的是「整形/共振」
// （把洛书向量投影到镜像对称子空间并反复迭代），**无法凭空恢复编码阶段
// 已丢失的区分度**。八卦塔缩（96.88% 同卦）的根治必须靠编码先验修复，
// 而非本算子。

/// 螺旋序：以坎（九宫位置 7）为起点，相邻两两即为一对镜像。
///
/// 位置布局（洛书九宫，位置索引 = 行×3 + 列）：
/// ```text
///   0(4/巽) 1(9/离) 2(2/坤)
///   3(3/震) 4(5/中) 5(7/兑)
///   6(8/艮) 7(1/坎) 8(6/乾)
/// ```
/// 走完四对镜像恰好绕外圈一圈。
pub const SPIRAL_ORDER: [usize; 8] = [7, 1, 6, 2, 3, 5, 0, 8];

/// 四对镜像位置（洛书数之和恒为 10：1+9、8+2、3+7、4+6）。
///
/// 注意：这是**洛书数值**之和为 10，而非位置索引之和
/// （位置索引之和为 8：7+1、6+2、3+5、0+8）。
pub const MIRROR_PAIRS: [[usize; 2]; 4] = [[7, 1], [6, 2], [3, 5], [0, 8]];

/// 梯形各层宽度：全量 → 去中外圈 → 四镜像对 → 两半环 → 太一。
pub const TRAPEZOID_LEVELS: [usize; 5] = [9, 8, 4, 2, 1];

/// 九宫格绕中心的 180° 旋转：外圈 `i ↦ 8 − i`，中心 4 自映射。
#[inline]
pub fn mirror_index(i: usize) -> usize {
    if i == 4 {
        4
    } else {
        8 - i
    }
}

/// 镜像双梯形螺旋递归的配置
#[derive(Debug, Clone, Copy)]
pub struct MirrorRecursionConfig {
    /// 递归迭代次数 T
    pub iterations: usize,
    /// 每轮融合旧状态的比例 α（0.0 = 全用新折叠值，1.0 = 全用旧状态）
    pub mix_scale: f32,
    /// 是否每轮把输入重注入最宽层正向状态。
    ///
    /// true（默认）修正设计稿缺陷 #1；false 复刻设计稿原始行为（首轮即用
    /// 逆向零状态覆盖输入），仅用于对照实验。
    pub re_inject: bool,
}

impl Default for MirrorRecursionConfig {
    fn default() -> Self {
        Self {
            iterations: 5,
            mix_scale: 0.5,
            re_inject: true,
        }
    }
}

/// 镜像双梯形螺旋递归的输出
#[derive(Debug, Clone)]
pub struct MirrorRecursionResult {
    /// 迭代后的洛书向量（已归一化）
    pub vector: LuoShuVector,
    /// 每轮最宽层逆向状态相对上一轮的余弦相似度（收敛轨迹）
    pub trace: Vec<f32>,
    /// 实际迭代次数
    pub iterations: usize,
}

/// 相邻成对平均：n → ⌈n/2⌉，奇数时末位原样保留。
fn fold_adjacent(values: &[f32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(values.len().div_ceil(2));
    let mut i = 0;
    while i + 1 < values.len() {
        out.push(0.5 * (values[i] + values[i + 1]));
        i += 2;
    }
    if i < values.len() {
        out.push(values[i]);
    }
    out
}

/// 逐值复制展开：n → 2n（`fold_adjacent` 的伴随算子，`fold∘expand = id`）。
fn expand_adjacent(values: &[f32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(values.len() * 2);
    for &v in values {
        out.push(v);
        out.push(v);
    }
    out
}

/// 非负清理 + L2 单位化；全零时退化为均匀分布。
fn nonneg_l2(values: &mut [f32]) {
    for v in values.iter_mut() {
        if !v.is_finite() || *v < 0.0 {
            *v = 0.0;
        }
    }
    let norm: f32 = values.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm <= 1e-12 {
        let uniform = 1.0 / (values.len().max(1) as f32).sqrt();
        for v in values.iter_mut() {
            *v = uniform;
        }
    } else {
        for v in values.iter_mut() {
            *v /= norm;
        }
    }
}

/// 新折叠值与旧状态的线性混合，再非负单位化（对应设计稿的 tanh 非线性压缩）。
fn mix_normalize(new_part: &[f32], old_part: &[f32], alpha: f32) -> Vec<f32> {
    let n = new_part.len().min(old_part.len());
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        out.push((1.0 - alpha) * new_part[i] + alpha * old_part[i]);
    }
    nonneg_l2(&mut out);
    out
}

/// 九宫向量 → 外圈 8 维螺旋序（跳过中心位置 4）。
fn to_spiral8(values: &[f32; 9]) -> Vec<f32> {
    SPIRAL_ORDER.iter().map(|&pos| values[pos]).collect()
}

/// 外圈 8 维螺旋序 → 九宫向量（中心 = 外圈均值）。
fn from_spiral8(values: &[f32]) -> [f32; 9] {
    let mut out = [0.0f32; 9];
    let mut sum = 0.0f32;
    for (i, &pos) in SPIRAL_ORDER.iter().enumerate() {
        let v = values.get(i).copied().unwrap_or(0.0);
        out[pos] = v;
        sum += v;
    }
    out[LUOSHU_CENTER_POS] = sum / 8.0;
    out
}

/// 切片 → 固定 9 维数组（不足补零，超出截断）。
fn to_array9(values: &[f32]) -> [f32; 9] {
    let mut out = [0.0f32; 9];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = values.get(i).copied().unwrap_or(0.0);
    }
    out
}

/// 两个等长切片的余弦相似度；任一方范数为零时返回 0.0。
fn cosine_slice(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..n {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na <= 1e-12 || nb <= 1e-12 {
        0.0
    } else {
        (dot / (na.sqrt() * nb.sqrt())).clamp(-1.0, 1.0)
    }
}

/// 镜像双梯形螺旋递归（符号/几何路线）。
///
/// 在梯形 `TRAPEZOID_LEVELS = [9,8,4,2,1]` 上做 T 轮双向迭代：
///
/// * 正向（宽 → 窄）：`h_f[k] = mix(fold(h_f[k-1]), h_b[k])`
/// * 逆向（窄 → 宽）：`h_b[k] = mix(expand(h_b[k+1]), h_f[k])`
/// * 每轮全层**读旧状态、写新状态**（同步更新，对应设计稿的并行更新）
///
/// `k = 1` 的正向折叠用螺旋序 `to_spiral8`；`k = 0` 的逆向展开用
/// `from_spiral8`（中心取外圈均值）。输出为最宽层逆向状态 `h_b[0]`。
///
/// 复杂度：O(T · 9)
pub fn symbolic_mirror_recursion(
    input: &LuoShuVector,
    config: &MirrorRecursionConfig,
) -> MirrorRecursionResult {
    let x = input.values;

    // 初始化正向状态：第 0 层为输入，其后逐层折叠。
    let mut h_f: Vec<Vec<f32>> = Vec::with_capacity(TRAPEZOID_LEVELS.len());
    h_f.push(x.to_vec());
    h_f.push(to_spiral8(&x));
    for k in 2..TRAPEZOID_LEVELS.len() {
        let folded = fold_adjacent(&h_f[k - 1]);
        h_f.push(folded);
    }

    // 逆向状态全部初始化为零。
    let mut h_b: Vec<Vec<f32>> = TRAPEZOID_LEVELS.iter().map(|&w| vec![0.0f32; w]).collect();

    let alpha = config.mix_scale.clamp(0.0, 1.0);
    let last = TRAPEZOID_LEVELS.len() - 1;
    let mut trace = Vec::with_capacity(config.iterations);

    for _ in 0..config.iterations {
        let f_old: Vec<Vec<f32>> = h_f.clone();
        let b_old: Vec<Vec<f32>> = h_b.clone();

        // 最宽层正向：重注入输入（修正设计稿缺陷 #1）。
        if config.re_inject {
            h_f[0] = x.to_vec();
        } else {
            let mut injected = b_old[0].clone();
            nonneg_l2(&mut injected);
            h_f[0] = injected;
        }

        // 正向扫描：宽 → 窄。
        for k in 1..TRAPEZOID_LEVELS.len() {
            let folded = if k == 1 {
                to_spiral8(&to_array9(&f_old[0]))
            } else {
                fold_adjacent(&f_old[k - 1])
            };
            h_f[k] = mix_normalize(&folded, &b_old[k], alpha);
        }

        // 逆向扫描：窄 → 宽（读旧状态，同步更新）。
        h_b[last] = mix_normalize(&b_old[last], &f_old[last], alpha);
        for k in (0..last).rev() {
            let expanded = if k == 0 {
                from_spiral8(&b_old[1]).to_vec()
            } else {
                expand_adjacent(&b_old[k + 1])
            };
            h_b[k] = mix_normalize(&expanded, &f_old[k], alpha);
        }

        // 收敛轨迹：本轮最宽层逆向状态与上轮的余弦相似度。
        trace.push(cosine_slice(&h_b[0], &b_old[0]));
    }

    let mut vector = LuoShuVector {
        values: to_array9(&h_b[0]),
    };
    vector.normalize_to_luoshu();

    MirrorRecursionResult {
        vector,
        trace,
        iterations: config.iterations,
    }
}

// ============================================================
// 操作 2：TrapezoidFocus — 梯形兴趣区域检索
// ============================================================

/// 梯形聚焦算子：在九宫格坐标系中划定梯形兴趣区域 (ROI)
///
/// 算法：
/// 1. 在 3×3 九宫格中定义梯形（由 4 个顶点确定）
/// 2. 递归细分梯形为更小的子梯形（每层细分，区域缩小 4 倍）
/// 3. 仅检索落在 ROI 内的记忆向量
///
/// 复杂度：O(roi_ratio × N)，roi_ratio = 1 / 4^depth
///
/// 当 depth = 0 时退化为全量检索（roi_ratio = 1.0）
/// 当 depth = 2 时仅检索 1/16 区域（roi_ratio = 0.0625）
///
/// 梯形兴趣区域 — 对数级复杂度检索。
impl TrapezoidROI {
    /// 创建新的梯形 ROI
    ///
    /// vertices 必须包含 4 个有效位置索引（0-8），按顺时针排列
    pub fn new(vertices: [usize; 4], depth: u32) -> Self {
        Self { vertices, depth }
    }

    /// 创建覆盖全部九宫格的 ROI（depth=0 时等于全量检索）
    pub fn full(depth: u32) -> Self {
        Self {
            vertices: [0, 2, 8, 6], // 四角：巽→坤→乾→艮
            depth,
        }
    }

    /// 以某个九宫格位置为中心创建 ROI
    ///
    /// 梯形顶点从该位置向外扩展 1 格
    pub fn centered(center: usize, depth: u32) -> Self {
        let center = center.min(8);
        let row = center / 3;
        let col = center % 3;

        // 计算四个顶点（限制在 0..9 范围内）
        let r0 = if row > 0 { row - 1 } else { 0 };
        let r1 = (row + 1).min(2);
        let c0 = if col > 0 { col - 1 } else { 0 };
        let c1 = (col + 1).min(2);

        Self {
            vertices: [
                r0 * 3 + c0, // 左上
                r0 * 3 + c1, // 右上
                r1 * 3 + c1, // 右下
                r1 * 3 + c0, // 左下
            ],
            depth,
        }
    }

    /// 判断一个九宫格位置是否落在 ROI 内
    pub fn contains_position(&self, pos: usize) -> bool {
        if pos >= 9 || self.vertices.iter().any(|&v| v >= 9) {
            return false;
        }
        let row = pos / 3;
        let col = pos % 3;

        // 简化版：使用边界矩形判断（四个顶点的 min/max）
        let min_row = self.vertices.iter().map(|&v| v / 3).min().unwrap_or(0);
        let max_row = self.vertices.iter().map(|&v| v / 3).max().unwrap_or(2);
        let min_col = self.vertices.iter().map(|&v| v % 3).min().unwrap_or(0);
        let max_col = self.vertices.iter().map(|&v| v % 3).max().unwrap_or(2);

        row >= min_row && row <= max_row && col >= min_col && col <= max_col
    }

    /// 对洛书向量，判断其"重心位置"是否落在 ROI 内
    ///
    /// 重心位置 = argmax(values)，即向量中最大分量对应的九宫格位置
    pub fn contains_vector(&self, vector: &LuoShuVector) -> bool {
        if vector.values.iter().any(|value| !value.is_finite()) {
            return false;
        }
        let center = vector
            .values
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i)
            .unwrap_or(4); // 默认太极位
        self.contains_position(center)
    }

    /// 道枢映射: 离卦·火 (☲) — 明辨也，过滤如火光之照亮真实
    /// 过滤向量集合，仅保留落在 ROI 内的向量
    pub fn filter_vectors(&self, vectors: &[(usize, &LuoShuVector)]) -> Vec<usize> {
        vectors
            .iter()
            .filter(|(_, v)| self.contains_vector(v))
            .map(|(i, _)| *i)
            .collect()
    }

    /// 道枢映射: 坎卦·水 (☵) — 水流而不盈，面积比是梯形几何的数学精华
    /// 计算 ROI 面积占比（用于估算检索加速比）
    pub fn area_ratio(&self) -> f32 {
        let n_positions = (0..9).filter(|&p| self.contains_position(p)).count() as f32;
        n_positions / 9.0
    }

    /// 道枢映射: 坤卦·地 (☷) — 地势坤，细分如大地之纹理延展
    /// 递归细分：将 ROI 按深度拆分为 4^depth 个子区域
    ///
    /// 每个子区域是矩形边界框的 1/4^depth 分割。
    /// 返回子区域列表，每个子区域由其中心位置和边界框顶点表示。
    pub fn subdivide(&self) -> Vec<TrapezoidROI> {
        let min_row = self.vertices.iter().map(|&v| v / 3).min().unwrap_or(0);
        let max_row = self.vertices.iter().map(|&v| v / 3).max().unwrap_or(2);
        let min_col = self.vertices.iter().map(|&v| v % 3).min().unwrap_or(0);
        let max_col = self.vertices.iter().map(|&v| v % 3).max().unwrap_or(2);

        let rows = max_row - min_row + 1;
        let cols = max_col - min_col + 1;

        if self.depth == 0 || rows <= 1 || cols <= 1 {
            return vec![self.clone()];
        }

        // 计算每层细分因子，避免幂运算溢出。
        let Some(sub_divisions) = 2usize.checked_pow(self.depth) else {
            return vec![self.clone()];
        };
        // 2026-09-01 修复(P1)：细分存在物理上限——9 宫格单维最多 3 格，
        // 深度超过网格分辨率后继续细分只是重复切到单格，无新信息。
        // 将 sub_divisions 钳制到单维格子数（≤3），使双层循环规模恒 ≤ 9，
        // 杜绝 depth=63（2^63）时进入 2^126 次迭代的不可执行循环（DoS 卡死）。
        let sub_divisions = sub_divisions.min(rows.max(cols).max(1));
        let sub_rows = (rows as f32 / sub_divisions as f32).ceil() as usize;
        let sub_cols = (cols as f32 / sub_divisions as f32).ceil() as usize;

        let mut regions = Vec::new();
        for sr in 0..sub_divisions {
            for sc in 0..sub_divisions {
                let r0 = min_row + sr * sub_rows;
                let r1 = (r0 + sub_rows - 1).min(max_row);
                let c0 = min_col + sc * sub_cols;
                let c1 = (c0 + sub_cols - 1).min(max_col);

                if r0 <= r1 && c0 <= c1 {
                    regions.push(TrapezoidROI {
                        vertices: [r0 * 3 + c0, r0 * 3 + c1, r1 * 3 + c1, r1 * 3 + c0],
                        depth: 0, // 子区域不再递归
                    });
                }
            }
        }
        regions
    }

    /// 道枢映射: 离卦·火 (☲) — 日月丽乎天，聚焦召回如日光之聚照
    /// 梯形聚焦检索：在向量集合中执行递归细分检索
    ///
    /// 算法：
    /// 1. 将 ROI 递归细分为 4^depth 个子区域
    /// 2. 对每个子区域，统计落在其中的向量数量
    /// 3. 选择向量密度最高的子区域作为最佳匹配
    /// 4. 返回子区域内的向量索引
    ///
    /// 复杂度：O(n × 4^depth)，但由于子区域过滤，实际检索量 = n / 4^depth
    pub fn focused_recall(&self, vectors: &[(usize, &LuoShuVector)]) -> TrapezoidFocusResult {
        if self.depth == 0 {
            // 无细分，直接过滤
            let indices = self.filter_vectors(vectors);
            let coverage = indices.len() as f32 / vectors.len().max(1) as f32;
            return TrapezoidFocusResult {
                best_region: 0,
                matched_indices: indices,
                coverage,
                subdivision_path: vec![0],
            };
        }

        // 递归细分
        let sub_regions = self.subdivide();
        if sub_regions.is_empty() {
            return TrapezoidFocusResult {
                best_region: 0,
                matched_indices: Vec::new(),
                coverage: 0.0,
                subdivision_path: Vec::new(),
            };
        }

        // 对每个子区域统计向量密度
        let mut region_stats: Vec<(usize, Vec<usize>, f32)> = sub_regions
            .iter()
            .enumerate()
            .map(|(i, region)| {
                let indices = region.filter_vectors(vectors);
                let density = if !indices.is_empty() {
                    indices.len() as f32 / region.area_ratio().max(0.01)
                } else {
                    0.0
                };
                (i, indices, density)
            })
            .collect();

        // 按密度降序排序，选择密度最高的子区域
        region_stats.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

        let best = &region_stats[0];
        let coverage = best.1.len() as f32 / vectors.len().max(1) as f32;

        // 构建细分路径（从外层到内层）
        let path: Vec<usize> = region_stats.iter().map(|(i, _, _)| *i).collect();

        TrapezoidFocusResult {
            best_region: best.0,
            matched_indices: best.1.clone(),
            coverage,
            subdivision_path: path,
        }
    }
}

// ============================================================
// 操作 3：RecursiveCompose — 递归合成
// ============================================================

/// 递归合成算子：将多个洛书向量门控融合为一个抽象向量
///
/// 算法：
/// 1. 对每个源向量，计算其与聚类中心的相似度作为门控权重
/// 2. 加权平均：合成向量 = Σ(w_i × v_i) / Σ(w_i)
/// 3. 对结果重新施加幻和归一化
/// 4. 计算置信度（基于权重分布的集中度）
///
/// 复杂度：O(n × 9)，n = 源向量数量
///
/// 递归合成 — 门控融合多条记忆为抽象知识。
pub fn recursive_compose(vectors: &[LuoShuVector]) -> ComposeResult {
    if vectors.is_empty() {
        return ComposeResult {
            vector: LuoShuVector::zeros(),
            confidence: 0.0,
            weights: Vec::new(),
            information_gain: 0.0,
        };
    }

    if vectors.len() == 1 {
        return ComposeResult {
            vector: vectors[0].clone(),
            confidence: 1.0,
            weights: vec![1.0],
            information_gain: 0.0, // 单条无法合成，信息增量为 0
        };
    }

    // 步骤 1：计算聚类中心（简单的分量平均）
    let n = vectors.len();
    let mut center = [0.0f32; 9];
    for v in vectors {
        for (i, c) in center.iter_mut().enumerate() {
            *c += v.values[i];
        }
    }
    for c in center.iter_mut() {
        *c /= n as f32;
    }
    let center_vec = LuoShuVector { values: center };

    // 步骤 2：计算每个源向量与中心的相似度作为门控权重
    let mut weights: Vec<f32> = vectors
        .iter()
        .map(|v| {
            // 使用余弦相似度 + 一个小偏移确保所有权重为正
            let sim = v.cosine_similarity(&center_vec);
            (sim + 1.0) / 2.0 // 映射到 [0, 1]
        })
        .collect();

    // 归一化权重
    let weight_sum: f32 = weights.iter().sum();
    if weight_sum > 0.0 {
        for w in weights.iter_mut() {
            *w /= weight_sum;
        }
    } else {
        // 退化为均匀权重
        let uniform = 1.0 / n as f32;
        for w in weights.iter_mut() {
            *w = uniform;
        }
    }

    // 步骤 3：加权融合
    let mut fused = [0.0f32; 9];
    for (v, &w) in vectors.iter().zip(weights.iter()) {
        for (i, f) in fused.iter_mut().enumerate() {
            *f += v.values[i] * w;
        }
    }

    // 步骤 4：重新施加幻和约束
    let mut composed = LuoShuVector { values: fused };
    composed.normalize_to_luoshu();

    // 步骤 5：置信度计算
    // 权重分布越集中 → 置信度越低（说明聚类不够紧密）
    // 权重分布越均匀 → 置信度越高（说明各源向量都很接近中心）
    let confidence = {
        let entropy: f32 = weights
            .iter()
            .filter(|&&w| w > 1e-6)
            .map(|&w| -w * w.ln())
            .sum();
        let max_entropy = (n as f32).ln();
        if max_entropy > 0.0 {
            entropy / max_entropy // 归一化熵，1.0 = 完全均匀 = 高置信度
        } else {
            0.0
        }
    };

    // 步骤 6：信息增量计算（质疑二：防止模式坍塌）
    // 信息增量 = 合成向量与各源向量的平均余弦距离
    // 距离越大 → 合成产生了更多"新信息"（抽象层次提升）
    // 距离越小 → 合成只是"压缩"了冗余，没有产生新信息
    let information_gain = {
        let avg_distance: f32 = vectors
            .iter()
            .map(|v| 1.0 - composed.cosine_similarity(v).max(0.0))
            .sum::<f32>()
            / n as f32;
        // 限制在 [0, 1] 范围
        avg_distance.clamp(0.0, 1.0)
    };

    ComposeResult {
        vector: composed,
        confidence,
        weights,
        information_gain,
    }
}

// ============================================================
// 操作 4：RecursiveUnfold — 递归拆解
// ============================================================

/// 递归拆解算子：将一条抽象记忆向量展开为多条子向量
///
/// 算法：
/// 1. 对 9 维向量按九宫格区域分割为 3×3 子矩阵
/// 2. 每个非零子区域生成一个子向量（保持洛书约束）
/// 3. 计算重构保真度：展开再合成后与原向量的相似度
///
/// 复杂度：O(9) = O(1)
///
/// 可逆展开 — 将合成记忆还原为子记忆。
pub fn recursive_unfold(vector: &LuoShuVector, min_activation: f32) -> UnfoldResult {
    let threshold = min_activation.max(0.01);

    // 步骤 1：找出所有激活的九宫格位置（高于阈值）
    let active_positions: Vec<usize> = (0..9).filter(|&i| vector.values[i] >= threshold).collect();

    if active_positions.is_empty() {
        return UnfoldResult {
            sub_vectors: Vec::new(),
            sub_weights: Vec::new(),
            fidelity: 0.0,
        };
    }

    // 步骤 2：为每个激活位置生成一个子向量
    let mut sub_vectors = Vec::with_capacity(active_positions.len());
    let mut sub_weights = Vec::with_capacity(active_positions.len());

    let total_activation: f32 = active_positions.iter().map(|&i| vector.values[i]).sum();

    for &pos in &active_positions {
        // 创建以该位置为主的子向量
        // 主位置权重 = 0.7，其余 8 个位置均匀分配 0.3
        let mut sub = [0.0f32; 9];
        let main_val = 0.7;
        let side_val = 0.3 / 8.0;

        for (i, s) in sub.iter_mut().enumerate() {
            *s = if i == pos { main_val } else { side_val };
        }

        let mut sub_vec = LuoShuVector { values: sub };
        sub_vec.normalize_to_luoshu();
        sub_vectors.push(sub_vec);

        // 子向量权重 = 该位置激活值 / 总激活值
        sub_weights.push(vector.values[pos] / total_activation);
    }

    // 步骤 3：计算重构保真度
    // 将子向量重新合成，比较与原向量的相似度
    if sub_vectors.len() >= 2 {
        let recomposed = recursive_compose(&sub_vectors);
        let fidelity = recomposed.vector.cosine_similarity(vector);
        // 保真度不可能为负
        UnfoldResult {
            sub_vectors,
            sub_weights,
            fidelity: fidelity.max(0.0),
        }
    } else {
        // 只有一个子向量，保真度 = 1.0
        UnfoldResult {
            sub_vectors,
            sub_weights,
            fidelity: 1.0,
        }
    }
}

// ============================================================
// 组合操作：完整的记忆演化流程
// ============================================================

/// 道枢映射: 乾卦·天 (☰) — 天行健，演化周期如天道运行不息
/// 执行完整的记忆演化周期
///
/// 1. MirrorProject — 对所有记忆进行分类
/// 2. 按类别聚类
/// 3. RecursiveCompose — 每个类别内部合成
/// 4. 返回合成结果（按类别组织）
pub fn evolution_cycle(
    vectors: &[(String, LuoShuVector)], // (记忆ID, 洛书向量)
) -> Vec<(String, ComposeResult)> {
    // 步骤 1+2：分类 + 分组
    let mut groups: std::collections::HashMap<usize, Vec<&LuoShuVector>> =
        std::collections::HashMap::new();
    let mut group_names: std::collections::HashMap<usize, &str> = std::collections::HashMap::new();

    for (_id, vec) in vectors {
        let proj = mirror_project(vec);
        groups.entry(proj.best_index).or_default().push(vec);
        group_names.entry(proj.best_index).or_insert(proj.best_name);
    }

    // 步骤 3：每个类别内递归合成
    let mut results = Vec::new();
    for (bagua_idx, group_vecs) in &groups {
        if group_vecs.len() >= 2 {
            // 转为 owned
            let owned: Vec<LuoShuVector> = group_vecs.iter().map(|v| (*v).clone()).collect();
            let composed = recursive_compose(&owned);
            let name = group_names.get(bagua_idx).unwrap_or(&"未知");
            results.push((format!("{}-合成", name), composed));
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::super::luoshu_encoder::{LuoShuEncoder, LUOSHU_WEIGHTS};
    use super::*;

    fn make_vec(text: &str) -> LuoShuVector {
        LuoShuEncoder::new().encode_text(text)
    }

    // === MirrorProject 测试 ===

    #[test]
    fn test_mirror_project_classifies() {
        let v = make_vec("项目使用 PostgreSQL 数据库连接");
        let proj = mirror_project(&v);
        assert!(proj.best_index < 8);
        assert!(!proj.best_name.is_empty());
        assert!(!proj.best_category.is_empty());
    }

    /// 八卦分类的结构契约（统计编码器路径）
    ///
    /// v0.9.10 修正：原断言"不同文本应分到不同类别"是一条**语义属性**，但本
    /// 测试跑的是统计编码器——它自述"仅含字符密度 / 字符熵 / 位置权重"（见
    /// `luoshu_encoder.rs` 的编码器说明），**本就不承诺语义区分度**。
    /// 把语义断言压在它身上，等于让测试依赖一个组件从未声明的能力。
    /// 语义属性已移到唯一具备该能力的 ML 路径上验证
    /// （见 `test_mirror_project_semantic_discrimination_ml`）。
    #[test]
    fn test_mirror_project_produces_valid_classification() {
        for text in [
            "数据库 PostgreSQL 配置",
            "React 前端组件样式",
            "API 接口 JWT 认证",
        ] {
            let v = make_vec(text);
            let p = mirror_project(&v);

            assert!(
                p.best_index < 8,
                "八卦索引必须落在 0..8，实际 {}",
                p.best_index
            );
            assert!(!p.best_name.is_empty(), "卦名不可为空");
            assert!(!p.best_category.is_empty(), "类别名不可为空");

            // best_index 必须真的是 argmax（投影算子的核心契约）
            let max_score = p.scores.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
            assert_eq!(
                p.scores[p.best_index], max_score,
                "best_index 必须是 8 个基底得分的 argmax"
            );
        }
    }

    /// 八卦分类的**语义区分度**（ML 路径才有；无模型时优雅跳过）
    ///
    /// v0.9.10 新增，承担两个职责：
    ///   1. 把"不同语义的文本落到不同卦"这条语义断言放到唯一有语义能力的路径；
    ///   2. 作为**塌缩回归防线**——若上游编码再次退化（例如有人把固定的
    ///      洛书先验权重加回去、或把幻和约束重新压到激活值上），立即变红。
    #[cfg(feature = "ml")]
    #[test]
    fn test_mirror_project_semantic_discrimination_ml() {
        let enc = match crate::engine::create_smart_encoder() {
            Ok((e, true)) => e,
            _ => {
                eprintln!("[跳过] ML 编码器不可用（CI 无模型时属预期）");
                return;
            }
        };

        // 语义上相距较远的 6 段文本
        let texts = [
            "数据库 PostgreSQL 连接池配置",
            "React 前端组件样式与主题",
            "API 接口 JWT 认证流程",
            "医生建议每天服用两次降压药",
            "周末去西湖散步顺便吃片儿川",
            "服务器内存不足被 OOM Killer 杀掉",
        ];
        let cats: Vec<usize> = texts
            .iter()
            .map(|t| mirror_project(&enc.encode_text(t)).best_index)
            .collect();
        let unique: std::collections::HashSet<usize> = cats.iter().cloned().collect();

        eprintln!(
            "[诊断] 6 段远距文本的卦分布 = {:?}（不同卦数 {}）",
            cats,
            unique.len()
        );
        assert!(
            unique.len() >= 2,
            "ML 路径下语义相距较远的文本不应全部塌到同一卦，实际 {:?}",
            cats
        );
    }

    // === TrapezoidFocus 测试 ===

    #[test]
    fn test_roi_center_contains() {
        let roi = TrapezoidROI::centered(4, 0); // 太极位为中心
        assert!(roi.contains_position(4), "ROI 应包含中心");
        assert!(roi.contains_position(0), "ROI 应包含左上角");
        assert!(roi.contains_position(8), "ROI 应包含右下角");
    }

    #[test]
    fn test_roi_rejects_invalid_positions_and_nan_vectors() {
        let roi = TrapezoidROI::full(0);
        assert!(!roi.contains_position(9));
        let mut vector = LuoShuVector::zeros();
        vector.values[0] = f32::NAN;
        assert!(!roi.contains_vector(&vector));
    }

    #[test]
    fn test_roi_center_clamps_invalid_center() {
        assert_eq!(
            TrapezoidROI::centered(usize::MAX, 0).vertices,
            TrapezoidROI::centered(8, 0).vertices
        );
    }

    #[test]
    fn test_roi_extreme_depth_does_not_hang() {
        // 回归：depth=u32::MAX 此前会进入 2^126 次迭代的不可执行循环（DoS 卡死）。
        // 修复后 sub_divisions 被钳制到单维格子数（≤3），子区域总数恒 ≤ 9。
        let roi = TrapezoidROI::full(u32::MAX);
        let regions = roi.subdivide();
        assert!(
            regions.len() <= 9,
            "极端 depth 子区域数应受网格分辨率钳制，实际 {}",
            regions.len()
        );
    }

    #[test]
    fn test_roi_filter_vectors() {
        let encoder = LuoShuEncoder::new();
        let v0 = encoder.encode_text("西北方向的配置信息"); // 应落在乾位 (8)
        let v1 = encoder.encode_text("核心架构设计"); // 应落在中位 (4)
        let v2 = encoder.encode_text("北方水源相关的 Bug"); // 应落在坎位 (7)

        let vecs: Vec<(usize, &LuoShuVector)> = vec![(0, &v0), (1, &v1), (2, &v2)];

        // 以太极位为中心的 ROI 应至少包含大部分向量
        let roi = TrapezoidROI::centered(4, 0);
        let filtered = roi.filter_vectors(&vecs);
        assert!(!filtered.is_empty(), "ROI 应至少包含部分向量");
    }

    #[test]
    fn test_roi_area_ratio() {
        let roi_full = TrapezoidROI::full(0);
        assert!(
            (roi_full.area_ratio() - 1.0).abs() < 1e-6,
            "全量 ROI 面积比应为 1.0"
        );

        let roi_center = TrapezoidROI::centered(4, 0);
        // 以太极位为中心，含 3×3=9 个位置
        assert!(
            (roi_center.area_ratio() - 1.0).abs() < 1e-6,
            "太极位 ROI 应覆盖全部 9 格"
        );
    }

    // === RecursiveCompose 测试 ===

    #[test]
    fn test_recursive_compose_single() {
        let v = make_vec("项目使用 PostgreSQL");
        let result = recursive_compose(std::slice::from_ref(&v));
        assert!(
            (result.confidence - 1.0).abs() < 1e-6,
            "单向量合成置信度应为 1.0"
        );
        assert_eq!(result.weights, vec![1.0]);
    }

    #[test]
    fn test_recursive_compose_multiple() {
        let v1 = make_vec("PostgreSQL 数据库配置");
        let v2 = make_vec("数据库连接池设置");
        let v3 = make_vec("PostgreSQL 查询优化");

        let result = recursive_compose(&[v1, v2, v3]);
        assert_eq!(result.weights.len(), 3, "应有 3 个权重");
        assert!(result.confidence > 0.0, "置信度应 > 0");
        assert!(result.confidence <= 1.0, "置信度应 ≤ 1.0");

        // 权重的和应为 1.0
        let weight_sum: f32 = result.weights.iter().sum();
        assert!((weight_sum - 1.0).abs() < 1e-3, "权重和应 = 1.0");
    }

    #[test]
    fn test_recursive_compose_empty() {
        let result = recursive_compose(&[]);
        assert_eq!(result.confidence, 0.0);
        assert!(result.weights.is_empty());
    }

    // === RecursiveUnfold 测试 ===

    #[test]
    fn test_recursive_unfold_basic() {
        let v = make_vec("这是一段包含多个语义维度的复杂文本内容，用于测试拆解功能");
        let result = recursive_unfold(&v, 0.01);

        assert!(!result.sub_vectors.is_empty(), "应有至少一个子向量");
        assert_eq!(result.sub_vectors.len(), result.sub_weights.len());
    }

    #[test]
    fn test_recursive_unfold_fidelity() {
        let v = make_vec("测试拆解与重构的保真度");
        let result = recursive_unfold(&v, 0.01);

        if result.sub_vectors.len() >= 2 {
            // 拆解再合成后，保真度应该较高
            assert!(
                result.fidelity > 0.5,
                "重构保真度 {} 应 > 0.5",
                result.fidelity
            );
        }
    }

    #[test]
    fn test_recursive_unfold_empty() {
        let v = LuoShuVector::zeros();
        let result = recursive_unfold(&v, 0.01);
        assert!(result.sub_vectors.is_empty());
    }

    // === Evolution Cycle 测试 ===

    #[test]
    fn test_evolution_cycle() {
        let encoder = LuoShuEncoder::new();
        let vectors: Vec<(String, LuoShuVector)> = vec![
            (
                "mem1".into(),
                encoder.encode_text("PostgreSQL 数据库连接配置"),
            ),
            ("mem2".into(), encoder.encode_text("数据库查询优化策略")),
            ("mem3".into(), encoder.encode_text("React 前端组件设计")),
            ("mem4".into(), encoder.encode_text("前端样式布局方案")),
        ];

        let results = evolution_cycle(&vectors);
        assert!(!results.is_empty(), "演化周期应产生至少一个合成结果");
    }

    // === 镜像双梯形螺旋递归测试 ===

    /// 九宫格绕中心 180° 旋转是**对合**（自逆），且中心自映射。
    #[test]
    fn test_mirror_index_is_involution() {
        for i in 0..9usize {
            assert_eq!(
                mirror_index(mirror_index(i)),
                i,
                "mirror_index 必须是对合：mirror(mirror({})) == {}",
                i,
                i
            );
        }
        assert_eq!(mirror_index(4), 4, "中心位置 4 必须自映射");
    }

    /// 螺旋序必须恰好覆盖外圈 8 个位置（不含中心 4），且无重复。
    #[test]
    fn test_spiral_order_covers_outer_ring() {
        let mut sorted = SPIRAL_ORDER.to_vec();
        sorted.sort_unstable();
        assert_eq!(
            sorted,
            vec![0, 1, 2, 3, 5, 6, 7, 8],
            "螺旋序必须恰好覆盖外圈 8 个位置（不含中心 4）"
        );
    }

    /// 每对镜像位置的**洛书数值**之和恒为 10，位置索引之和恒为 8。
    #[test]
    fn test_mirror_pairs_numeral_sum_is_ten() {
        for pair in MIRROR_PAIRS.iter() {
            // 洛书数值 = 先验权重 × 15（权重已除 15 归一）
            let numeral_sum = LUOSHU_WEIGHTS[pair[0]] * 15.0 + LUOSHU_WEIGHTS[pair[1]] * 15.0;
            assert!(
                (numeral_sum - 10.0).abs() < 1e-3,
                "镜像对 {:?} 的洛书数之和应为 10，实际 {}",
                pair,
                numeral_sum
            );
            // 位置索引之和恒为 8（与洛书数之和 10 不同，勿混淆）
            assert_eq!(
                pair[0] + pair[1],
                8,
                "镜像对 {:?} 的位置索引之和应为 8",
                pair
            );
            // 两个位置必须互为镜像（180° 旋转）
            assert_eq!(mirror_index(pair[0]), pair[1]);
            assert_eq!(mirror_index(pair[1]), pair[0]);
        }
    }

    /// 梯形层级必须从宽到窄严格递减，且首尾为 9 / 1。
    #[test]
    fn test_trapezoid_levels_shrink() {
        assert_eq!(TRAPEZOID_LEVELS[0], 9, "最宽层应为全量 9");
        assert_eq!(*TRAPEZOID_LEVELS.last().unwrap(), 1, "最窄层应为太一 1");
        for w in TRAPEZOID_LEVELS.windows(2) {
            assert!(
                w[0] > w[1],
                "梯形层宽必须严格递减，出现 {} -> {}",
                w[0],
                w[1]
            );
        }
    }

    /// 同一输入 + 同一配置，输出必须完全一致（确定性算子）。
    #[test]
    fn test_recursion_is_deterministic() {
        let v = make_vec("数据库 PostgreSQL 配置与查询优化");
        let cfg = MirrorRecursionConfig::default();
        let a = symbolic_mirror_recursion(&v, &cfg);
        let b = symbolic_mirror_recursion(&v, &cfg);
        assert_eq!(a.vector.values, b.vector.values, "递归算子必须是确定性的");
    }

    /// 输出必须是合法的洛书向量：有限、非负、L2 单位化。
    #[test]
    fn test_recursion_output_is_valid_luoshu() {
        let v = make_vec("镜像双梯形螺旋递归验证文本");
        let result = symbolic_mirror_recursion(&v, &MirrorRecursionConfig::default());

        for (i, &x) in result.vector.values.iter().enumerate() {
            assert!(x.is_finite(), "第 {} 维必须有限，实际 {}", i, x);
            assert!(x >= 0.0, "第 {} 维必须非负，实际 {}", i, x);
        }
        let norm: f32 = result
            .vector
            .values
            .iter()
            .map(|x| x * x)
            .sum::<f32>()
            .sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-3,
            "输出必须 L2 单位化，实际范数 {}",
            norm
        );
    }

    /// 全零输入 → 均匀分布（1/√9 = 1/3）。
    #[test]
    fn test_recursion_zero_input_is_uniform() {
        let v = LuoShuVector::zeros();
        let result = symbolic_mirror_recursion(&v, &MirrorRecursionConfig::default());
        for (i, &x) in result.vector.values.iter().enumerate() {
            assert!(
                (x - 1.0 / 3.0).abs() < 1e-4,
                "零输入应退化为均匀分布，第 {} 维实际 {}",
                i,
                x
            );
        }
    }

    /// 镜像对称性保持：外圈镜像对称的输入，其输出同样镜像对称。
    #[test]
    fn test_recursion_preserves_mirror_symmetry() {
        // 构造外圈镜像对称的输入：每对镜像位置取均值。
        let base = make_vec("镜像对称保持性验证 PostgreSQL");
        let mut vals = base.values;
        for pair in MIRROR_PAIRS.iter() {
            let avg = 0.5 * (vals[pair[0]] + vals[pair[1]]);
            vals[pair[0]] = avg;
            vals[pair[1]] = avg;
        }
        // 中心取外圈均值（与算子输出口径一致）
        vals[LUOSHU_CENTER_POS] = 0.5 * (vals[0] + vals[1]);
        let sym = LuoShuVector { values: vals };

        let out = symbolic_mirror_recursion(&sym, &MirrorRecursionConfig::default());
        for pair in MIRROR_PAIRS.iter() {
            let diff = (out.vector.values[pair[0]] - out.vector.values[pair[1]]).abs();
            assert!(
                diff < 1e-4,
                "镜像对 {:?} 的输出应保持对称，偏差 {}",
                pair,
                diff
            );
        }
    }

    /// 不同输入必须产生不同输出（算子不能把一切压成同一常量）。
    #[test]
    fn test_recursion_output_depends_on_input() {
        // 两个互相镜像但明显不同的单热点输入。
        let mut a = [0.0f32; 9];
        a[7] = 1.0; // 坎
        let mut b = [0.0f32; 9];
        b[0] = 1.0; // 巽

        let cfg = MirrorRecursionConfig::default();
        let out_a = symbolic_mirror_recursion(&LuoShuVector { values: a }, &cfg);
        let out_b = symbolic_mirror_recursion(&LuoShuVector { values: b }, &cfg);

        let l1: f32 = out_a
            .vector
            .values
            .iter()
            .zip(out_b.vector.values.iter())
            .map(|(x, y)| (x - y).abs())
            .sum();
        assert!(l1 > 1e-3, "不同输入应产生不同输出，实际 L1 差 {}", l1);
    }

    /// 迭代次数边界：T=0（退化）与 T=100（深迭代）都不得 panic 或产生非法值。
    #[test]
    fn test_recursion_iteration_bounds() {
        let v = make_vec("迭代次数边界测试文本");

        let zero = symbolic_mirror_recursion(
            &v,
            &MirrorRecursionConfig {
                iterations: 0,
                ..Default::default()
            },
        );
        assert_eq!(zero.trace.len(), 0, "T=0 时不应有收敛轨迹");
        let norm0: f32 = zero.vector.values.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm0 - 1.0).abs() < 1e-3, "T=0 输出仍须是合法洛书向量");

        let deep = symbolic_mirror_recursion(
            &v,
            &MirrorRecursionConfig {
                iterations: 100,
                ..Default::default()
            },
        );
        assert_eq!(deep.trace.len(), 100, "T=100 应记录 100 条收敛轨迹");
        for (i, &x) in deep.vector.values.iter().enumerate() {
            assert!(x.is_finite() && x >= 0.0, "深迭代第 {} 维非法：{}", i, x);
        }
    }
}
