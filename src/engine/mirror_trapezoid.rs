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
// 操作 1：MirrorProject — 先天八卦分类
// ============================================================

/// 镜像投影算子：将洛书向量投影到 8 个先天八卦基底上
///
/// 算法：
/// 1. 对每个八卦基底，计算与洛书向量的内积
/// 2. 返回匹配度最高的基底及其类别
///
/// 复杂度：O(8 × 9) = O(1)
///
/// 先天八卦投影 — 将洛书向量映射到八卦类别。
pub fn mirror_project(vector: &LuoShuVector) -> BaguaProjection {
    let mut scores = [0.0f32; 8];
    let mut best_index = 0usize;
    let mut best_score = f32::NEG_INFINITY;

    for i in 0..8 {
        // 内积计算
        let dot: f32 = vector
            .values
            .iter()
            .zip(BAGUA_BASES[i].iter())
            .map(|(a, b)| a * b)
            .sum();
        scores[i] = dot;

        if dot > best_score {
            best_score = dot;
            best_index = i;
        }
    }

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

        // 计算每层细分因子
        let sub_divisions = 2usize.pow(self.depth);
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
    use super::super::luoshu_encoder::LuoShuEncoder;
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

    #[test]
    fn test_mirror_project_different_texts() {
        let v1 = make_vec("数据库 PostgreSQL 配置");
        let v2 = make_vec("React 前端组件样式");
        let v3 = make_vec("API 接口 JWT 认证");

        let p1 = mirror_project(&v1);
        let p2 = mirror_project(&v2);
        let p3 = mirror_project(&v3);

        // 三者不应全部分到同一类别（至少有两种不同分类）
        let categories = vec![p1.best_index, p2.best_index, p3.best_index];
        let unique: std::collections::HashSet<usize> = categories.into_iter().collect();
        assert!(unique.len() >= 2, "不同文本应分到不同类别");
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
}
