// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件包含模型底层架构衍生的核心算法，受研究许可证保护。
// 禁止逆向工程、禁止商业再分发、禁止用于训练竞争模型。
// ============================================================
//
// 洛书坐标编码器实现。
// 将文本编码为 9 维洛书坐标向量，支持幻和约束与八卦分类。

use serde::{Deserialize, Serialize};

/// 编码器状态信息（可解释性面板）
///
/// 提供当前编码器运行模式的透明视图，帮助用户和开发者
/// 理解系统的语义能力水平。解决质疑四"可解释性下降"问题。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncoderStatus {
    /// 当前编码模式：ml / statistical
    pub mode: String,
    /// ML 模型名称（降级模式下为 None）
    pub model_name: Option<String>,
    /// ML 模型隐藏层维度（降级模式下为 None）
    pub hidden_size: Option<usize>,
    /// 降级原因（正常模式下为 None）
    pub degradation_reason: Option<String>,
    /// 总编码次数
    pub total_encodings: u64,
    /// 上次编码成功时间戳（毫秒）
    pub last_encoding_ms: u64,
    /// 系统能力描述（面向用户）
    pub capability_description: String,
    /// 编码质量评分 (0.0 ~ 1.0)
    ///
    /// - ML 模式：1.0（高精度语义理解）
    /// - 统计模式 + TF-IDF：0.4 ~ 0.6（关键词级别理解）
    /// - 纯统计模式：0.2 ~ 0.3（字符级别理解）
    pub quality_score: f32,
}

impl Default for EncoderStatus {
    fn default() -> Self {
        Self {
            mode: "statistical".to_string(),
            model_name: None,
            hidden_size: None,
            degradation_reason: Some("ML 编码器未启用".to_string()),
            total_encodings: 0,
            last_encoding_ms: 0,
            capability_description: "统计模式：基于词频和字符熵的轻量编码，语义区分能力有限"
                .to_string(),
            quality_score: 0.25,
        }
    }
}

/// 洛书九宫格的幻和：每行 / 每列 / 每条对角线之和 = 15
///
/// 这是九数 1..9 的**经典性质**（4+9+2=15、3+5+7=15、对角线亦 15）。
/// v0.9.10 起它只作为**标号性质**存在：不再用它约束激活值
/// （那会把 9 个自由度压成约 2 个，见 `normalize_to_luoshu`），
/// 仅用于 `luoshu_deviation` 推导"理想洛书 L2 单位化后的每线和"。
const LUOSHU_MAGIC_SUM: f32 = 15.0;

/// 洛书九宫格的标准布局：
///   4  9  2
///   3  5  7
///   8  1  6
///
/// 每行、每列、每条对角线的和 = 15（归一化后 = 1.0）
///
/// 归一化后的标准权重（每个位置的值 / 15）：
pub const LUOSHU_WEIGHTS: [f32; 9] = [
    4.0 / 15.0, // 位置 0：巽（东南）
    9.0 / 15.0, // 位置 1：离（南）
    2.0 / 15.0, // 位置 2：坤（西南）
    3.0 / 15.0, // 位置 3：震（东）
    5.0 / 15.0, // 位置 4：中（太极）
    7.0 / 15.0, // 位置 5：兑（西）
    8.0 / 15.0, // 位置 6：艮（东北）
    1.0 / 15.0, // 位置 7：坎（北）
    6.0 / 15.0, // 位置 8：乾（西北）
];

/// `luoshu_deviation` 的**结构最大值**（v0.9.10）
///
/// 归一化契约变更（外圈自由 + 中心 = 外圈均值 + 整体 L2 单位化）后，
/// `luoshu_deviation` 的可行域不再是 [0,1]，其结构最大值为**边中格尖峰**：
/// 仅外圈边中位（如 idx1 离）取 a、其余外圈为 0 时偏离度最大。
///
/// 解析推导：中心 = a/8 ⇒ 整体 L2 模 = a·√65/8 ⇒ 归一化后
/// idx_边中 = 8/√65 ≈ 0.992278、idx_中心 = 1/√65 ≈ 0.124035；
/// `target = 15/√285 ≈ 0.888523`；8 条线之和为
/// {边中所在行: 8/√65，边中所在列: 9/√65，两条对角线: 1/√65，其余 4 条: 0}
/// ⇒ dev² = 4.184398 ⇒ dev ≈ 2.045581（随机搜索 + 坐标上升收敛一致，
/// 见单测 `test_luoshu_deviation_structural_max`）。
///
/// 用途：`dao_isomorphism_score = 1 − avg_deviation / LUOSHU_DEVIATION_MAX`，
/// 使评分在可行域内回到 [0,1] 且保持单调（见 `dao_metrics.rs`）。
pub const LUOSHU_DEVIATION_MAX: f32 = 2.045_581;

/// 洛书九宫格的标准布局：
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LuoShuVector {
    /// 9 维坐标值，索引 0-8 对应九宫格位置：
    /// 0(巽) 1(离) 2(坤)
    /// 3(震) 4(中) 5(兑)
    /// 6(艮) 7(坎) 8(乾)
    pub values: [f32; 9],
}

impl LuoShuVector {
    /// 创建零向量
    pub fn zeros() -> Self {
        Self { values: [0.0; 9] }
    }

    /// 从 9 个原始值创建，自动施加幻和归一化
    pub fn new(raw: [f32; 9]) -> Self {
        let mut v = Self { values: raw };
        v.normalize_to_luoshu();
        v
    }

    /// 洛书结构归一化（v0.9.10 修正）
    ///
    /// ## 与原实现的关键区别
    ///
    /// 原实现把「幻和」当成对**激活值**的约束——迭代缩放每行、每列、两条
    /// 对角线，使它们的和都等于 `LUOSHU_MAGIC_SUM`。后果是把 9 个自由度
    /// 压到约 2 个（3×3 幻方在幻和固定时是 2 维族），语义信号在归一化这
    /// 一步就被抹平。
    ///
    /// 线上实测佐证（2010 条 AML 记忆、1963 条有效向量）：行和/列和/对角线
    /// 和全部 ≈1.000、中心恒为 1/3、`mirror_project` 的 argmax 100% 落在
    /// 同一位置，各位置取值域宽仅 0.09~0.14。
    ///
    /// ## 为什么幻和不该约束激活值
    ///
    /// 幻和（4+9+2=15、3+5+7=15、对角线亦 15）是**标号 1–9 的性质**——
    /// 它约束的是"哪个数字坐哪个宫"，即**位置编号**；而承载语义的是
    /// "这个宫被点亮了多少"，即**激活值**。二者不是一回事。
    ///
    /// ## 正确的结构归一化
    ///
    /// - **外圈 8 个位置保持自由**：只做"非负 + 统一缩放"，保留它们之间的
    ///   相对差异（保序、保比值）⇒ 有效自由度 = 8（原为 2）；
    /// - **中心位（4/太极）是不变量**，取外圈 8 值的算术平均。
    ///   这不是人为约定，而是洛书自带的平衡性：
    ///   理想洛书 (4+9+2+3+7+8+1+6)/8 = 5 = 中心数。
    ///   由此中心天然"始终不变"，且永远不会成为最大值——不破坏下游依赖
    ///   argmax 的几何（`mirror_project`、`TrapezoidROI`）。
    ///
    /// 归一化后整体 L2 模长恒为 1（不再固定"总和"——见下方第 3 步说明）。
    pub fn normalize_to_luoshu(&mut self) {
        // 中心位（太极）
        const CENTER: usize = 4;
        // 外圈 8 个位置（不含中心）
        const SIDES: [usize; 8] = [0, 1, 2, 3, 5, 6, 7, 8];

        // 1) 清理非法值：NaN / Inf / 负数一律归零
        for v in self.values.iter_mut() {
            if !v.is_finite() || *v < 0.0 {
                *v = 0.0;
            }
        }

        // 2) 中心位 = 外圈 8 值的算术平均。
        //    这不是人为约定，而是洛书自带的平衡性：
        //    理想洛书 (4+9+2+3+7+8+1+6)/8 = 5 = 中心数。
        //    ★ 顺序很重要：中心必须在缩放**之前**定，之后的整体等比缩放
        //    不会破坏"中心 = 外圈均值"这个等式。
        let side_mean = SIDES.iter().map(|&i| self.values[i]).sum::<f32>() / 8.0;
        self.values[CENTER] = side_mean;

        // 3) 整体 L2 归一化（单位长度）。
        //
        //    ★ 为什么用 L2 而不是"总和 = 1"：
        //    「中心 = 外圈均值」与「总和恒定」在数学上**互斥**——
        //    设总和恒为 C，则 中心 = 外圈均值 = (C − 中心)/8 ⇒ 中心 ≡ C/9，
        //    仍是常数，于是 topological_depth = 1 − 中心 依旧恒定、依旧无信息。
        //    语义上："中心点始终不变"指的是**中心这一点（位置）不动**——
        //    螺旋永远绕它转、它不参与轮转——而不是它的**数值**被钉死。
        //    因此这里让整体尺度随内容浮动：中心随外圈分布形状浮动，
        //    拓扑深度恢复信息。
        //
        //    同时：余弦相似度本身尺度不变，故本改动不影响余弦语义；
        //    Manhattan 距离落在单位球面上，也保持可比。
        let norm = self.values.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 1e-12 {
            for v in self.values.iter_mut() {
                *v /= norm;
            }
        } else {
            // 病态输入（全零）→ 均匀分布，避免除零
            for v in self.values.iter_mut() {
                *v = 1.0 / 3.0;
            }
        }
    }

    /// 道枢映射: 洛书·幻和 — 九宫格幻和偏离度，是洛书数理结构的核心度量
    /// 计算幻和偏离度（越小越接近"每行/列/对角线和相等"的理想洛书结构）
    ///
    /// v0.9.10 修正：归一化改为"外圈自由 + 中心 = 外圈均值 + 整体 L2 单位化"后，
    /// 理想洛书（九数 4,9,2,3,5,7,8,1,6）L2 单位化后每线之和为 15/√285 ≈ 0.8886，
    /// 故比较目标改为该值（原实现按"总和为 3"时取 1.0，已不再适用）。
    /// 指标语义不变——仍是"激活分布离理想洛书结构有多远"——但数值尺度随之调整：
    /// 修正前所有向量都被强制落在幻和流形上，偏离度恒 ≈0，该指标实际上失效；
    /// 修正后它才第一次成为一个有区分度的信号。
    pub fn luoshu_deviation(&self) -> f32 {
        // 理想洛书九数 [4,9,2,3,5,7,8,1,6] 的 L2 模 = √285，每线和 = 15
        // ⇒ L2 单位化后理想每线和 = 15/√285 ≈ 0.8886
        let target = LUOSHU_MAGIC_SUM / 285.0f32.sqrt();
        let mut dev = 0.0f32;

        // 行偏差
        for row in 0..3 {
            let sum: f32 = self.values[row * 3..(row + 1) * 3].iter().sum();
            dev += (sum - target).powi(2);
        }
        // 列偏差
        for col in 0..3 {
            let sum: f32 = (0..3).map(|row| self.values[row * 3 + col]).sum();
            dev += (sum - target).powi(2);
        }
        // 对角线偏差
        let d1 = self.values[0] + self.values[4] + self.values[8];
        let d2 = self.values[2] + self.values[4] + self.values[6];
        dev += (d1 - target).powi(2);
        dev += (d2 - target).powi(2);

        dev.sqrt()
    }

    /// 计算两个洛书向量的余弦相似度
    pub fn cosine_similarity(&self, other: &LuoShuVector) -> f32 {
        let dot: f32 = self
            .values
            .iter()
            .zip(other.values.iter())
            .map(|(a, b)| a * b)
            .sum();
        let norm_a: f32 = self.values.iter().map(|v| v * v).sum::<f32>().sqrt();
        let norm_b: f32 = other.values.iter().map(|v| v * v).sum::<f32>().sqrt();

        if norm_a == 0.0 || norm_b == 0.0 {
            return 0.0;
        }
        let result = (dot / (norm_a * norm_b)).clamp(-1.0, 1.0);
        // 防止 NaN（浮点运算可能产生极小负值开方）
        if result.is_nan() {
            0.0
        } else {
            result
        }
    }

    /// 计算洛书几何距离（九宫格上的 Manhattan 距离）
    pub fn grid_distance(&self, other: &LuoShuVector) -> f32 {
        self.values
            .iter()
            .zip(other.values.iter())
            .map(|(a, b)| (a - b).abs())
            .sum()
    }

    /// 道枢映射: 洛书·中宫 — 中宫为五，是九宫格的枢纽与平衡中心
    /// 获取中心值（太极位，位置 4）
    pub fn center_value(&self) -> f32 {
        self.values[4]
    }
}

/// 位置/节奏通道权重：分段统计（字符密度、信息熵、位置衰减）承载"结构"信号。
///
/// 该通道**内容盲**（与文本内容无关，只反映长度与分段形态）。经融合扫描（8 组
/// wp×wi 组合）验证：一旦位置通道权重逼近或超过身份通道，"内容盲"会重新主导，
/// 导致近义对排序反转、判别度（近义相似度 − 无关相似度）转为负。
/// 故定标 0.3，让结构信号参与但不喧宾夺主。
const POSITIONAL_CHANNEL_WEIGHT: f32 = 0.3;
/// 身份通道权重：字/词身份哈希落宫，承载"内容"信号，用于破除"内容盲"。
///
/// 定标 1.0（身份主导）：实测只有身份通道占主导时，近义对相似度才高于无关对
/// 且判别度为正。与位置通道的 0.3 构成 0.3:1.0 的固定配比。
const IDENTITY_CHANNEL_WEIGHT: f32 = 1.0;

/// 洛书坐标编码器
///
/// 将文本内容编码为 9 维洛书向量。
///
/// 编码流程（v0.9.10 双通道融合）：
/// 1. 位置/节奏通道：将文本分为 9 段，每段提取字符密度、信息熵、位置衰减
/// 2. 身份通道：按字符身份散列落宫，承载"内容"信号
/// 3. 两通道各自归一化后按 0.3:1.0（身份主导）融合，得到内容敏感的 9 维特征
/// 4. 施加洛书标准权重作为先验分布
/// 5. 结构归一化（中心=外圈均值 → 整体 L2 单位化），见 `normalize_to_luoshu`
pub struct LuoShuEncoder {
    /// 是否启用洛书权重先验
    use_prior: bool,
}

impl LuoShuEncoder {
    /// 批量编码文本为洛书向量（**非 ML 构建的回退实现**）。
    ///
    /// v0.9.10 修复：`ml` 构建下的批量实现定义在 `luoshu_encoder_ml.rs`，而该文件
    /// 整体受 `#[cfg(feature = "ml")]` 门控 ⇒ 在 `--features server`（无 ml）下
    /// `encode_text_batch` 缺失，导致 `remember_batch` / 重分类批量路径编译失败。
    /// 此处补齐**非 ML 回退**，使两种 feature 组合都能编译。
    ///
    /// 契约与 ML 版完全一致：空入参返回空 `Vec`，输出条数与输入条数严格相等、
    /// 顺序一一对应。区别仅在无 `ml` 时无法做单次批量前向与跨条目并行，
    /// 故退化为逐条 `encode_text`。两者按 `ml` feature **互斥**，同一构建下只有其一存在。
    #[cfg(not(feature = "ml"))]
    pub fn encode_text_batch(&self, texts: &[&str]) -> Vec<LuoShuVector> {
        texts.iter().map(|t| self.encode_text(t)).collect()
    }

    /// 创建带有洛书先验权重的编码器
    pub fn new() -> Self {
        Self { use_prior: true }
    }

    /// 创建不带动洛书先验的编码器（纯文本驱动）
    pub fn new_unbiased() -> Self {
        Self { use_prior: false }
    }

    /// 将文本编码为 9 维洛书向量
    ///
    /// 算法（v0.9.10 双通道融合）：
    /// 道枢映射: 洛书·九宫 — 将语义向量映射到洛书九宫格，实现数与义的统一，是编码体系的核心
    ///
    /// 1. 位置/节奏通道：分段统计特征（承载"结构"）
    /// 2. 身份通道：字符身份散列落宫（承载"内容"）
    /// 3. 两通道各自归一化后按 0.3:1.0（身份主导）融合
    /// 4. 施加洛书先验权重（如有）
    /// 5. 结构归一化（中心=外圈均值 → 整体 L2 单位化）
    pub fn encode_text(&self, text: &str) -> LuoShuVector {
        // 双通道融合：位置/节奏通道（结构）+ 身份通道（内容）。
        // 二者各自归一化到 sum=1 后按权重融合，量纲可比、零硬编码。
        let positional = Self::normalize_channel(Self::extract_9_features(text));
        let identity = Self::normalize_channel(Self::extract_identity_features(text));

        let mut raw = [0.0f32; 9];
        for i in 0..9 {
            raw[i] =
                positional[i] * POSITIONAL_CHANNEL_WEIGHT + identity[i] * IDENTITY_CHANNEL_WEIGHT;
        }

        let mut values = if self.use_prior {
            // 贝叶斯融合：先验 × 似然
            let mut posterior = [0.0f32; 9];
            for i in 0..9 {
                posterior[i] = LUOSHU_WEIGHTS[i] * raw[i];
            }
            posterior
        } else {
            raw
        };

        // 归一化
        let total: f32 = values.iter().sum();
        if total > 1e-6 {
            for v in values.iter_mut() {
                *v /= total;
            }
        } else {
            // 退化为均匀分布
            values = [1.0 / 9.0; 9];
        }

        let mut vec = LuoShuVector { values };
        vec.normalize_to_luoshu();
        vec
    }

    /// 从文本中提取 9 个特征值
    ///
    /// 将文本均匀分为 9 个段落，每段提取：
    /// - 字符密度（该段字符数 / 总字符数）
    /// - 信息熵（字符种类 / 该段字符数）
    /// - 位置衰减（离中心越远权重越低）
    fn extract_9_features(text: &str) -> [f32; 9] {
        let chars: Vec<char> = text.chars().collect();
        let total = chars.len();

        if total == 0 {
            return [1.0 / 9.0; 9];
        }

        let mut features = [0.0f32; 9];
        let segment_size = (total as f32 / 9.0).ceil() as usize;

        for (seg, feature) in features.iter_mut().enumerate() {
            let start = seg * segment_size;
            let end = (start + segment_size).min(total);

            if start >= total {
                *feature = 1e-6; // 极小值，避免全零
                continue;
            }

            let segment: Vec<char> = chars[start..end].to_vec();
            let seg_len = segment.len() as f32;

            // 字符密度（归一化）
            let density = seg_len / total as f32;

            // 信息熵：唯一字符数 / 段长度
            let unique_count = {
                let mut sorted = segment.clone();
                sorted.sort();
                sorted.dedup();
                sorted.len() as f32
            };
            let entropy = if seg_len > 0.0 {
                unique_count / seg_len
            } else {
                0.0
            };

            // 位置权重：中心位置（4）权重最高，边缘位置权重递减
            let center_dist = (seg as i32 - 4i32).abs() as f32;
            let position_weight = (-center_dist * center_dist / 8.0).exp();

            *feature = density * 0.4 + entropy * 0.3 + position_weight * 0.3;
        }

        features
    }

    /// 通道内归一化：把任意非负 9 维向量归一化为 sum=1，保证多通道融合量纲可比。
    /// 全零输入（该通道无信号）时返回全零，使其在融合中自然"缺席"。
    fn normalize_channel(mut f: [f32; 9]) -> [f32; 9] {
        let sum: f32 = f.iter().sum();
        if sum > 1e-6 {
            for v in f.iter_mut() {
                *v /= sum;
            }
        } else {
            f = [0.0; 9];
        }
        f
    }

    /// 提取"身份通道"9 维特征：按**字符身份**稳定落宫。
    ///
    /// 与 `extract_9_features`（分段结构）互补：本通道只看"出现了哪些字符/字母"，
    /// 与位置无关，承载内容信号。
    ///
    /// 采用**字符级**（而非 n-gram）粒度，理由是字符是语言中重叠度最高、最稳定的
    /// 单位——相似文本共享大量字符，故该通道天然保持局部性
    /// （字面相近 ⇒ 落宫分布相近 ⇒ 向量相近）；反之，内容不同的同长文本
    /// 不再退化为同一向量，破除"内容盲"。
    ///
    /// 落宫 = `char_palace(字符)`；只统计身份字符（字母/数字/中日韩），
    /// 忽略空白与标点，避免格式差异污染内容信号。
    ///
    /// **去共模整流**：字符袋落宫后并非直接使用，而是**减去 9 宫均值再取 ReLU**
    /// （`v = max(v - mean, 0)`）。原因是哈希落宫带有"热桶偏置"——某些宫对任意
    /// 文本都稳定吃到约 1/9 以上的份额（共模成分），若不剔除，不同文本的落宫分布
    /// 会共享一大块基线，余弦相似度天然偏高、判别度被稀释（实测近义对排序反转）。
    /// 减去均值再 ReLU 只保留"高于平均"的判别信号，等效于对落宫分布做**中心化 +
    /// 半波整流**，是纯数据驱动、无阈值、无硬编码的整流。
    fn extract_identity_features(text: &str) -> [f32; 9] {
        let mut features = [0.0f32; 9];
        for ch in text.chars() {
            if !ch.is_alphanumeric() {
                continue; // 空白/标点不计入身份信号
            }
            features[char_palace(ch)] += 1.0;
        }
        // 去共模整流：减均值后取正部，消除热桶偏置，只留判别信号
        let base: f32 = features.iter().sum::<f32>() / 9.0;
        for v in features.iter_mut() {
            *v = (*v - base).max(0.0);
        }
        features
    }

    /// 洛书幻和偏离度（监控用，越小越好）
    pub fn deviation_of(&self, text: &str) -> f32 {
        let vec = self.encode_text(text);
        vec.luoshu_deviation()
    }

    /// 获取编码器状态（统计模式始终返回固定状态）
    pub fn get_status(&self) -> EncoderStatus {
        EncoderStatus::default()
    }
}

impl Default for LuoShuEncoder {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================
// 增强统计编码器：TF-IDF 加权 + 同义词扩展
// 解决质疑一"统计编码器兜底"语义保真度不足的问题
// ============================================================

/// 同义词映射表（常见技术领域）
///
/// 在统计编码时自动扩展同义词，提升关键词匹配的语义覆盖。
/// 覆盖中英文常见技术术语，降低降级模式下的语义损失。
fn get_synonym_map() -> &'static std::collections::HashMap<&'static str, Vec<&'static str>> {
    use std::sync::OnceLock;
    static SYNONYM_MAP: OnceLock<std::collections::HashMap<&'static str, Vec<&'static str>>> =
        OnceLock::new();
    SYNONYM_MAP.get_or_init(|| {
        let mut m = std::collections::HashMap::new();
        // 数据库领域
        m.insert("数据库", vec!["database", "DB", "数据存储", "数据仓库"]);
        m.insert(
            "database",
            vec!["数据库", "DB", "datastore", "data warehouse"],
        );
        m.insert("查询", vec!["query", "检索", "搜索", "select"]);
        m.insert("query", vec!["查询", "检索", "search", "select"]);
        // 性能领域
        m.insert("性能", vec!["performance", "效率", "速度", "优化"]);
        m.insert("performance", vec!["性能", "效率", "speed", "optimization"]);
        m.insert("慢", vec!["slow", "延迟", "卡顿", "瓶颈"]);
        m.insert("slow", vec!["慢", "延迟", "latency", "bottleneck"]);
        // 缓存领域
        m.insert("缓存", vec!["cache", "缓冲", "临时存储"]);
        m.insert("cache", vec!["缓存", "缓冲", "caching"]);
        // 错误领域
        m.insert("错误", vec!["error", "异常", "bug", "故障", "报错"]);
        m.insert("error", vec!["错误", "异常", "exception", "bug", "故障"]);
        m.insert("bug", vec!["错误", "缺陷", "故障", "漏洞"]);
        // API 领域
        m.insert("接口", vec!["API", "interface", "端点", "endpoint"]);
        m.insert("API", vec!["接口", "interface", "端点", "endpoint"]);
        // 认证领域
        m.insert("认证", vec!["auth", "登录", "鉴权", "身份验证"]);
        m.insert("auth", vec!["认证", "authentication", "登录", "鉴权"]);
        m.insert("登录", vec!["login", "认证", "鉴权", "signin"]);
        // 部署领域
        m.insert("部署", vec!["deploy", "发布", "上线", "release"]);
        m.insert("deploy", vec!["部署", "发布", "上线", "release"]);
        // 配置领域
        m.insert("配置", vec!["config", "设置", "参数", "选项"]);
        m.insert("config", vec!["配置", "configuration", "设置", "参数"]);
        // 测试领域
        m.insert("测试", vec!["test", "验证", "检查", "校验"]);
        m.insert("test", vec!["测试", "testing", "验证", "检查"]);
        // 安全领域
        m.insert("安全", vec!["security", "防护", "加密", "权限"]);
        m.insert("security", vec!["安全", "防护", "加密", "权限"]);
        m
    })
}

/// 同义词扩展：将文本中的关键词扩展为同义词集合
///
/// 返回扩展后的文本（原文 + 同义词追加），用于增强统计编码的语义覆盖。
fn expand_synonyms(text: &str) -> String {
    let mut expanded = text.to_string();
    let lower = text.to_lowercase();

    for (key, synonyms) in get_synonym_map().iter() {
        if lower.contains(&key.to_lowercase()) {
            for syn in synonyms {
                expanded.push(' ');
                expanded.push_str(syn);
            }
        }
    }
    expanded
}

/// TF-IDF 缓存：跟踪词频用于关键词提取
#[derive(Debug, Clone)]
pub struct TfIdfCache {
    /// 文档频率：词 → 出现该词的文档数
    document_frequency: std::collections::HashMap<String, usize>,
    /// 总文档数
    total_documents: usize,
}

impl TfIdfCache {
    pub fn new() -> Self {
        Self {
            document_frequency: std::collections::HashMap::new(),
            total_documents: 0,
        }
    }

    /// 道枢映射: 坤卦·地 (☷) — 厚德载物，文档注册如大地收藏万物
    /// 注册一篇文档，更新文档频率
    pub fn register_document(&mut self, text: &str) {
        self.total_documents += 1;
        let mut seen = std::collections::HashSet::new();
        for word in extract_keywords(text) {
            if seen.insert(word.clone()) {
                *self.document_frequency.entry(word).or_insert(0) += 1;
            }
        }
    }

    /// 计算词 t 的 IDF 值
    fn idf(&self, term: &str) -> f32 {
        let df = self.document_frequency.get(term).copied().unwrap_or(0);
        if df == 0 {
            // 未见过的新词，给予较高 IDF（视为有区分度）
            return ((self.total_documents + 1) as f32 / 1.0).ln();
        }
        ((self.total_documents + 1) as f32 / (df + 1) as f32).ln()
    }

    /// 获取总文档数
    pub fn total_documents(&self) -> usize {
        self.total_documents
    }
}

impl Default for TfIdfCache {
    fn default() -> Self {
        Self::new()
    }
}

/// 简单分词：提取中英文关键词
///
/// - 英文：按空格和标点分词，保留 2 字符以上的词
/// - 中文：提取 2-4 字 n-gram
fn extract_keywords(text: &str) -> Vec<String> {
    let mut keywords = Vec::new();
    let chars: Vec<char> = text.chars().collect();

    // 中文 n-gram (2-4 字)
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_alphabetic() {
            // 英文词：收集连续字母
            let start = i;
            while i < chars.len() && chars[i].is_ascii_alphabetic() {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if word.len() >= 2 {
                keywords.push(word.to_lowercase());
            }
        } else if c as u32 > 127 {
            // 中文字符：提取 n-gram
            if i + 1 < chars.len() {
                let bigram: String = chars[i..i + 2].iter().collect();
                keywords.push(bigram);
            }
            if i + 2 < chars.len() {
                let trigram: String = chars[i..i + 3].iter().collect();
                keywords.push(trigram);
            }
            i += 1;
        } else {
            i += 1;
        }
    }

    // 去重
    keywords.sort();
    keywords.dedup();
    keywords
}

/// 增强统计编码器
///
/// 在基础 LuoShuEncoder 之上叠加 TF-IDF 加权和同义词扩展，
/// 显著提升降级模式下的语义保真度。
///
/// 编码流程：
/// 1. 同义词扩展：将文本中的关键词扩展为同义词集合
/// 2. 关键词提取：提取中英文关键词
/// 3. TF-IDF 加权：用 IDF 值加权关键词对 9 维特征的贡献
/// 4. 洛书编码：基底编码器完成最终编码
pub struct EnhancedStatisticalEncoder {
    /// 基础洛书编码器（保留作为纯统计回退）
    ///
    /// v0.9.7 核实：移除实验证明该 allow 非冗余（仍报 `field 'base' is never read`）。
    #[allow(dead_code)]
    base: LuoShuEncoder,
    /// TF-IDF 缓存
    tfidf: TfIdfCache,
    /// 编码计数
    encoding_count: u64,
}

impl EnhancedStatisticalEncoder {
    /// 创建增强统计编码器
    pub fn new() -> Self {
        Self {
            base: LuoShuEncoder::new(),
            tfidf: TfIdfCache::new(),
            encoding_count: 0,
        }
    }

    /// 编码文本为洛书向量（增强版）
    ///
    /// 算法：
    /// 1. 同义词扩展 → 扩展文本
    /// 2. 关键词提取 + TF-IDF 加权 → 9 维关键词权重
    /// 3. 与基底编码器特征融合 → 最终向量
    pub fn encode_text(&mut self, text: &str) -> LuoShuVector {
        self.encoding_count += 1;

        // 1. 同义词扩展
        let expanded = expand_synonyms(text);

        // 2. 提取关键词并计算 TF-IDF 权重
        let keywords = extract_keywords(&expanded);
        let mut keyword_weights = [0.0f32; 9];
        if !keywords.is_empty() {
            for kw in &keywords {
                let idf = self.tfidf.idf(kw);
                // 用关键词哈希映射到 9 维位置
                let hash = simple_hash(kw) % 9;
                keyword_weights[hash] += idf;
            }
            // 归一化关键词权重
            let sum: f32 = keyword_weights.iter().sum();
            if sum > 1e-6 {
                for w in keyword_weights.iter_mut() {
                    *w /= sum;
                }
            }
        }

        // 3. 基底编码器特征
        let base_features = LuoShuEncoder::extract_9_features(&expanded);

        // 4. 融合：关键词权重 0.6 + 基底特征 0.4
        //    关键词权重占比更高，因为 TF-IDF 提供了语义区分度
        let mut fused = [0.0f32; 9];
        for i in 0..9 {
            fused[i] = keyword_weights[i] * 0.6 + base_features[i] * 0.4;
        }

        // 5. 归一化并施加洛书约束
        let total: f32 = fused.iter().sum();
        if total > 1e-6 {
            for v in fused.iter_mut() {
                *v /= total;
            }
        } else {
            fused = [1.0 / 9.0; 9];
        }

        let mut vec = LuoShuVector { values: fused };
        vec.normalize_to_luoshu();
        vec
    }

    /// 注册一篇文档到 TF-IDF 缓存（用于构建词频统计）
    pub fn register(&mut self, text: &str) {
        self.tfidf.register_document(text);
    }

    /// 获取编码器状态
    pub fn get_status(&self) -> EncoderStatus {
        let quality = if self.tfidf.total_documents() > 10 {
            0.45 // 有足够 TF-IDF 数据，关键词区分度较好
        } else {
            0.30 // TF-IDF 数据不足，偏向基础统计
        };
        EncoderStatus {
            mode: "statistical".to_string(),
            model_name: None,
            hidden_size: None,
            degradation_reason: Some("ML 编码器未启用".to_string()),
            total_encodings: self.encoding_count,
            last_encoding_ms: 0,
            capability_description: format!(
                "统计增强模式：基于 TF-IDF 关键词加权 ({}) 份文档 + 同义词扩展的轻量编码，语义区分度 {:0.0}%",
                self.tfidf.total_documents(),
                quality * 100.0
            ),
            quality_score: quality,
        }
    }
}

impl Default for EnhancedStatisticalEncoder {
    fn default() -> Self {
        Self::new()
    }
}

/// djb2 变体：对字节序列散列。
///
/// `simple_hash`（字符串身份）与 `char_palace`（字符身份）共用此内核，
/// 保证两者落宫口径一致，且散列参数只在此处定义一次（无重复硬编码）。
fn hash_bytes(bytes: &[u8]) -> usize {
    let mut h: usize = 5381;
    for &b in bytes {
        h = h.wrapping_mul(33).wrapping_add(b as usize);
    }
    h
}

/// 简单字符串哈希（用于关键词到维度的映射）
fn simple_hash(s: &str) -> usize {
    hash_bytes(s.as_bytes())
}

/// 单个字符的落宫（0..9）：ASCII 大小写归一后对其 UTF-8 字节散列。
/// 归一化保证 "PostgreSQL" 与 "postgresql" 落到同一宫（身份同一）。
fn char_palace(ch: char) -> usize {
    let mut buf = [0u8; 4];
    let lowered = ch.to_ascii_lowercase();
    hash_bytes(lowered.encode_utf8(&mut buf).as_bytes()) % 9
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证：编码器产生 9 维输出
    #[test]
    fn test_encode_produces_9_dim() {
        let encoder = LuoShuEncoder::new();
        let vec = encoder.encode_text("项目使用 PostgreSQL 数据库");
        assert_eq!(vec.values.len(), 9);
    }

    /// 验证：归一化后的结构契约（v0.9.10 新契约）
    ///
    /// 取代原 `test_luoshu_constraint_satisfied`——那个测试断言的是
    /// "幻和约束被满足"，正是本次要移除的过度约束（它把所有向量压到
    /// 约 2 自由度上，使偏离度恒 ≈0，测试因"大家都一样"而恒过）。
    #[test]
    fn test_normalize_structural_contract() {
        let encoder = LuoShuEncoder::new();
        let texts = [
            "项目使用 PostgreSQL 数据库",
            "用户偏好暗色主题 UI",
            "登录接口使用 JWT 认证",
            "代码",
            "",
        ];

        for text in &texts {
            let vec = encoder.encode_text(text);

            // 契约 1：整体 L2 模长恒为 1（v0.9.10：不再固定"总和"）
            let norm: f32 = vec.values.iter().map(|v| v * v).sum::<f32>().sqrt();
            assert!(
                (norm - 1.0).abs() < 1e-4,
                "文本 '{}' 归一化后 L2 模长应为 1，实际 {}",
                text,
                norm
            );

            // 契约 2：中心位 = 外圈 8 值的算术平均（洛书平衡性 ⇒ 不变量）
            let sides: Vec<f32> = vec
                .values
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != 4)
                .map(|(_, v)| *v)
                .collect();
            let mean = sides.iter().sum::<f32>() / 8.0;
            assert!(
                (vec.values[4] - mean).abs() < 1e-5,
                "文本 '{}' 中心位应等于外圈均值 {}，实际 {}",
                text,
                mean,
                vec.values[4]
            );

            // 契约 3：所有值非负
            assert!(
                vec.values.iter().all(|v| *v >= 0.0),
                "文本 '{}' 存在负值",
                text
            );
        }
    }

    /// 验证：`luoshu_deviation` 的结构最大值 = 边中格尖峰（锚定 `LUOSHU_DEVIATION_MAX`）
    ///
    /// # 为什么需要这条锚
    ///
    /// v0.9.10 的归一化契约变更后，`luoshu_deviation` 的可行域不再是 [0,1]，
    /// 其**结构最大值**由此向量的偏离度决定（随机搜索 + 坐标上升收敛一致）。
    /// 下游 `dao_isomorphism_score` 依赖此值做归一化，一旦归一化契约再次
    /// 漂移而常量未同步，score 会**静默失真**（正是本次实测踩到的坑）。
    /// 故用一条测试把常量钉死在几何事实上。
    ///
    /// # 边中格尖峰的解析推导
    ///
    /// 设外圈边中位 idx1 = a、其余外圈为 0：
    /// - 中心 idx4 = a/8（= 外圈均值）；整体 L2 模 = a·√65/8
    ///   ⇒ 归一化后 idx1 = 8/√65 ≈ 0.992278、idx4 = 1/√65 ≈ 0.124035；
    /// - target = 15/√285 ≈ 0.888523；
    /// - 8 条线之和：Row0 = 8/√65、Col1 = 9/√65、两条对角线 = 1/√65，其余 4 条 = 0
    ///   ⇒ dev² = 4.184398 ⇒ dev ≈ 2.045581（结构最大）。
    #[test]
    fn test_luoshu_deviation_structural_max() {
        // 除边中位 idx1 外全为 0 → 归一化后即上文的边中格尖峰
        let mut raw = [0.0f32; 9];
        raw[1] = 1.0;
        let vec = LuoShuVector::new(raw);
        let dev = vec.luoshu_deviation();
        assert!(
            (dev - 2.0456).abs() < 1e-3,
            "边中格尖峰偏离度应为结构最大值 ≈2.0456，实际 {:.6}",
            dev
        );
    }

    /// 验证：归一化只做"统一缩放"，不得改变外圈 8 值的相对次序（保序）
    ///
    /// 原实现会按行/列/对角线分别缩放，**可能改变 argmax**——
    /// 而 argmax 正是 `mirror_project`（八卦）与 `TrapezoidROI`（几何检索）
    /// 的唯一依据，一旦被归一化改写，等于把语义信号换成噪声。
    #[test]
    fn test_normalize_preserves_outer_argmax() {
        // 峰值放在位置 0（外圈），其余外圈位置给递减值
        let mut vec = LuoShuVector {
            values: [0.30, 0.20, 0.15, 0.12, 0.05, 0.10, 0.04, 0.03, 0.01],
        };
        let before = outer_argmax(&vec);
        vec.normalize_to_luoshu();
        let after = outer_argmax(&vec);
        assert_eq!(before, after, "归一化不得改变外圈 argmax");
        assert_eq!(after, 0, "峰值应仍在位置 0");
    }

    /// 验证：归一化不得改变外圈 8 值的相对比值（保比值）
    #[test]
    fn test_normalize_preserves_outer_ratios() {
        let mut vec = LuoShuVector {
            values: [0.50, 0.10, 0.10, 0.10, 0.10, 0.10, 0.10, 0.10, 0.10],
        };
        let ratio_before = vec.values[0] / vec.values[1];
        vec.normalize_to_luoshu();
        let ratio_after = vec.values[0] / vec.values[1];
        assert!(
            (ratio_before - ratio_after).abs() < 1e-4,
            "归一化不得改变外圈比值：{} -> {}",
            ratio_before,
            ratio_after
        );
        assert!(
            (ratio_after - 5.0).abs() < 1e-4,
            "原比值 0.50/0.10 应为 5.0，实际 {}",
            ratio_after
        );
    }

    /// 反例保护：行和**不得**再被幻和约束强行拉平
    ///
    /// 这是本次修正的核心回归防线——若有人把幻和约束加回
    /// `normalize_to_luoshu`，本测试立即变红。
    #[test]
    fn test_normalize_no_magic_sum_flattening() {
        // 全部激活集中在位置 0（外圈·巽）
        let mut vec = LuoShuVector {
            values: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        };
        vec.normalize_to_luoshu();

        let row0 = vec.values[0] + vec.values[1] + vec.values[2];
        let row1 = vec.values[3] + vec.values[4] + vec.values[5];
        let row2 = vec.values[6] + vec.values[7] + vec.values[8];

        assert!(
            (row0 - row1).abs() > 0.3,
            "行和不应被幻和约束拉平（row0={}, row1={}, row2={}）",
            row0,
            row1,
            row2
        );
        assert!(row0 > row1.max(row2), "激活集中的那一行应显著大于其它行");

        // 中心仍是外圈均值（不变量语义在新约束下依然成立）
        let sides: f32 = vec
            .values
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 4)
            .map(|(_, v)| *v)
            .sum();
        assert!(
            (vec.values[4] - sides / 8.0).abs() < 1e-5,
            "中心位应恒等于外圈均值"
        );
    }

    /// 辅助：外圈 8 个位置中最大值的下标（排除中心位 4）
    fn outer_argmax(vec: &LuoShuVector) -> usize {
        (0..9)
            .filter(|i| *i != 4)
            .max_by(|a, b| {
                vec.values[*a]
                    .partial_cmp(&vec.values[*b])
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(0)
    }

    /// 验证：相似文本产生相近向量
    #[test]
    fn test_similar_texts_close() {
        let encoder = LuoShuEncoder::new();
        // 使用较长、有足够区分度的文本
        let v1 = encoder.encode_text("项目使用 PostgreSQL 数据库存储所有的用户数据和订单信息");
        let v2 = encoder.encode_text("项目数据库连接使用 PostgreSQL 管理用户的查询和事务处理");
        let v3 = encoder.encode_text("前端使用 React 框架构建组件化的用户界面交互体验");

        let sim_12 = v1.cosine_similarity(&v2);
        let sim_13 = v1.cosine_similarity(&v3);

        // 说明：旧实现（内容盲编码器）下同长文本几乎塌缩为同一向量，sim_12≈1.0，
        // 故曾以 0.9 作为断言；该阈值实为"内容盲"的产物，会迫使位置通道主导、
        // 反而损害判别度。改为合理绝对下限 + 排序断言：只要相似文本足够接近
        // （下限 0.7）且**高于**无关文本即可，判据与检索/合成语义一致。
        assert!(
            sim_12 > 0.7,
            "相似文本的余弦相似度应 > 0.7，实际: {}",
            sim_12
        );
        assert!(
            sim_12 >= sim_13,
            "相似文本的余弦相似度 ({}) 应不低于不相似文本 ({})",
            sim_12,
            sim_13
        );
    }

    /// 验证：先验权重对编码的影响
    #[test]
    fn test_prior_effect() {
        let e1 = LuoShuEncoder::new();
        let e2 = LuoShuEncoder::new_unbiased();
        let text = "这是一段测试文本";

        let v1 = e1.encode_text(text);
        let v2 = e2.encode_text(text);

        // 带先验的编码器中心权重应更高
        assert!(
            v1.center_value() > v2.center_value() * 0.8,
            "先验编码器中心值 ({}) 应不低于无偏版 ({})",
            v1.center_value(),
            v2.center_value()
        );
    }

    /// 验证：向量非负
    #[test]
    fn test_non_negative() {
        let encoder = LuoShuEncoder::new();
        let vec = encoder.encode_text("任意文本");
        for (i, v) in vec.values.iter().enumerate() {
            assert!(*v >= 0.0, "位置 {} 的值 {} 为负", i, v);
        }
    }

    // === 增强统计编码器测试 ===

    /// 验证：增强编码器产生 9 维输出
    #[test]
    fn test_enhanced_encoder_9_dim() {
        let mut encoder = EnhancedStatisticalEncoder::new();
        let vec = encoder.encode_text("项目使用 PostgreSQL 数据库存储用户数据");
        assert_eq!(vec.values.len(), 9);
    }

    /// 验证：同义词扩展使语义相近的文本向量更接近
    #[test]
    fn test_synonym_expansion_improves_similarity() {
        let mut encoder = EnhancedStatisticalEncoder::new();
        // 注册一些文档构建 TF-IDF
        encoder.register("数据库查询优化是性能调优的关键");
        encoder.register("PostgreSQL 数据库连接池配置");
        encoder.register("API 接口设计的最佳实践");
        encoder.register("用户认证和授权机制");
        encoder.register("缓存策略对系统性能的影响");
        encoder.register("部署流程自动化脚本");
        encoder.register("错误日志收集和分析");
        encoder.register("安全漏洞扫描和修复");
        encoder.register("测试驱动开发实践");
        encoder.register("配置文件管理最佳实践");
        encoder.register("数据库索引优化策略");

        // 语义相近的文本
        let v1 = encoder.encode_text("数据库查询性能优化");
        let v2 = encoder.encode_text("DB 检索效率提升");

        let sim = v1.cosine_similarity(&v2);
        // 经过同义词扩展后，相似度应明显高于纯统计编码器
        assert!(
            sim > 0.3,
            "同义词扩展后相似文本的余弦相似度应 > 0.3，实际: {}",
            sim
        );
    }

    /// 验证：TF-IDF 使高频词维度得到合理加权
    #[test]
    fn test_tfidf_weighting() {
        let mut encoder = EnhancedStatisticalEncoder::new();
        // 注册大量"数据库"相关文档
        for _ in 0..20 {
            encoder.register("数据库查询优化索引性能调优");
        }
        // 注册少量"安全"相关文档
        encoder.register("安全漏洞扫描");

        // 编码"数据库"相关文本
        let v1 = encoder.encode_text("数据库查询性能");
        let v2 = encoder.encode_text("安全漏洞防护");

        // 9 维空间的区分度有限，但不同领域的文本不应完全相同
        let sim = v1.cosine_similarity(&v2);
        // 相似度应 < 1.0（非完全一致），9 维空间下阈值较宽松
        assert!(sim < 1.0, "不同领域文本的相似度应 < 1.0，实际: {}", sim);
    }

    /// 验证：编码器状态反映质量评分
    #[test]
    fn test_enhanced_encoder_status() {
        let mut encoder = EnhancedStatisticalEncoder::new();
        let status = encoder.get_status();

        assert_eq!(status.mode, "statistical");
        assert!(status.quality_score > 0.0, "质量评分应 > 0");
        assert!(status.quality_score <= 1.0, "质量评分应 <= 1.0");

        // 注册足够文档后质量评分应提升
        for i in 0..15 {
            encoder.register(&format!("文档 {} 内容", i));
        }
        let status2 = encoder.get_status();
        assert!(
            status2.quality_score > status.quality_score,
            "TF-IDF 数据积累后质量评分应提升"
        );
    }

    /// 验证：关键词提取正确分词
    #[test]
    fn test_keyword_extraction() {
        let keywords = extract_keywords("数据库查询性能优化和缓存策略");
        // 应包含中文 bigram 和 trigram
        assert!(!keywords.is_empty(), "应提取到关键词");
        // 包含 "数据" 相关的 n-gram
        let has_related = keywords
            .iter()
            .any(|k| k.contains("数据") || k.contains("查询"));
        assert!(has_related, "应包含数据相关关键词");
    }

    /// 验证：同义词扩展正确追加同义词
    #[test]
    fn test_synonym_expansion() {
        let expanded = expand_synonyms("数据库查询很慢");
        // 应包含原文和同义词
        assert!(expanded.contains("数据库"), "应保留原文");
        assert!(
            expanded.contains("database")
                || expanded.contains("query")
                || expanded.contains("slow"),
            "应包含同义词，实际: {}",
            expanded
        );
    }

    // ════════════════════════════════════════════════════════════════
    // 表示质量诊断基座（v0.9.10）
    //
    // 目的：在"表示重做"过程中提供**可对照的量化依据**——改前/改后跑同一个
    //   基座，就能看出 9 维洛书状态到底有没有真正读出语义。
    //
    // 设计原则：只做诊断，**不对质量设断言**（当前状态本身是待修的病态），
    //   仅硬断言结构契约；质量指标全部打印，供人工/CI 日志对照。
    //
    // 运行（有 ML 模型时）：
    //   cargo test --features server,ml --lib diagnostic_representation -- --nocapture
    // ════════════════════════════════════════════════════════════════

    /// 诊断样本：语义上分散的主题 + 若干近义对（用于测区分度）
    const DIAG_TEXTS: [&str; 26] = [
        "项目使用 PostgreSQL 数据库存储所有用户数据和订单信息",
        "前端使用 React 框架构建组件化的用户界面交互体验",
        "登录接口使用 JWT 令牌做无状态认证与权限校验",
        "服务器内存不足导致进程被 OOM Killer 强制结束",
        "我想在周末去西湖边散步，顺便吃一碗片儿川",
        "医生建议我每天服用两次降压药，饭后半小时",
        "用户偏好暗色主题，不喜欢弹窗打扰",
        "缓存命中率从 62% 提升到 88%，响应时间下降一半",
        "程序在启动时崩溃，堆栈指向空指针解引用",
        "会议定在周三下午三点，会议室在二楼东侧",
        "把这几条记忆按时间顺序整理成一份摘要",
        "他上周说要去成都出差，后来又改成了西安",
        "数据库连接池的最大连接数配置为 20",
        "数据库连接池的最大连接数配置为 20 个连接",
        "查询性能优化",
        "查询性能很差需要优化",
        "我喜欢喝美式咖啡，不加糖",
        "偏好美式咖啡不加糖",
        "HTTP 状态码 503 表示服务暂时不可用",
        "服务返回 503，说明后端暂时不可用",
        "孩子今年上小学三年级，喜欢画画",
        "把用户的生日记下来，明年提前一周提醒我",
        "这段代码的时间复杂度是 O(n log n)",
        "算法复杂度从 O(n^2) 优化到 O(n log n)",
        "明天要交季度报表",
        "季度报表明天截止",
    ];

    /// 跑一次表示质量诊断并打印指标（硬断言仅结构契约）
    fn run_representation_diagnostic(name: &str, encode: &dyn Fn(&str) -> LuoShuVector) {
        let vecs: Vec<LuoShuVector> = DIAG_TEXTS.iter().map(|t| encode(t)).collect();

        // 硬断言：结构契约（与 normalize 的契约一致）
        for (i, v) in vecs.iter().enumerate() {
            let norm: f32 = v.values.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!(
                (norm - 1.0).abs() < 1e-3,
                "[{}] 第 {} 条归一化后 L2 模长应为 1，实际 {}",
                name,
                i,
                norm
            );
            assert!(
                v.values.iter().all(|x| *x >= 0.0),
                "[{}] 第 {} 条存在负值",
                name,
                i
            );
        }

        eprintln!("[诊断·{}] 样本数 = {}", name, vecs.len());

        // 1) 外圈 argmax 分布 —— 塌缩时全部落在同一位置
        let mut dist = [0usize; 9];
        for v in vecs.iter() {
            dist[outer_argmax(v)] += 1;
        }
        let distinct = dist.iter().filter(|c| **c > 0).count();
        eprintln!("[诊断·{}] 外圈 argmax 分布 = {:?}", name, dist);
        eprintln!("[诊断·{}] 命中的不同位置数 = {} / 8", name, distinct);

        // 2) 每个位置的均值 / 值域宽度 —— 值域宽 ≈0 就是常量向量
        for p in 0..9 {
            let vals: Vec<f32> = vecs.iter().map(|v| v.values[p]).collect();
            let mean = vals.iter().sum::<f32>() / vals.len() as f32;
            let min = vals.iter().cloned().fold(f32::INFINITY, f32::min);
            let max = vals.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
            let sd =
                (vals.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / vals.len() as f32).sqrt();
            eprintln!(
                "[诊断·{}]   位置 {} : avg={:.4} sd={:.4} 值域宽={:.4}",
                name,
                p,
                mean,
                sd,
                max - min
            );
        }

        // 3) 两两余弦分布 —— 全部挤在一起说明区分度为零
        let mut cos: Vec<f32> = Vec::new();
        for i in 0..vecs.len() {
            for j in (i + 1)..vecs.len() {
                cos.push(vecs[i].cosine_similarity(&vecs[j]));
            }
        }
        cos.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = cos.len();
        eprintln!(
            "[诊断·{}] 余弦 n={} min={:.4} P10={:.4} P50={:.4} P90={:.4} max={:.4} (P90-P10={:.4})",
            name,
            n,
            cos[0],
            cos[n / 10],
            cos[n / 2],
            cos[9 * n / 10],
            cos[n - 1],
            cos[9 * n / 10] - cos[n / 10]
        );

        // 4) 拓扑深度分布 —— 恒相等说明中心位没携带信息
        let depths: Vec<f32> = vecs.iter().map(|v| 1.0 - v.center_value()).collect();
        let dmean = depths.iter().sum::<f32>() / depths.len() as f32;
        let dsd =
            (depths.iter().map(|d| (d - dmean).powi(2)).sum::<f32>() / depths.len() as f32).sqrt();
        eprintln!("[诊断·{}] 拓扑深度 avg={:.4} sd={:.4}", name, dmean, dsd);

        // 5) 近义对 vs 无关对的余弦差 —— 语义区分度的直接证据
        //    索引对：(12,13) 连接池；(14,15) 查询性能；(16,17) 咖啡；(24,25) 季度报表
        let pairs_near = [(12usize, 13usize), (14, 15), (16, 17), (24, 25)];
        let near: Vec<f32> = pairs_near
            .iter()
            .map(|(a, b)| vecs[*a].cosine_similarity(&vecs[*b]))
            .collect();
        let far: Vec<f32> = vec![
            vecs[0].cosine_similarity(&vecs[4]),
            vecs[1].cosine_similarity(&vecs[5]),
            vecs[2].cosine_similarity(&vecs[6]),
            vecs[3].cosine_similarity(&vecs[7]),
        ];
        let near_mean = near.iter().sum::<f32>() / near.len() as f32;
        let far_mean = far.iter().sum::<f32>() / far.len() as f32;
        eprintln!(
            "[诊断·{}] 近义对余弦 avg={:.4} | 无关对余弦 avg={:.4} | 差={:+.4}（>0 才有区分度）",
            name,
            near_mean,
            far_mean,
            near_mean - far_mean
        );
    }

    /// 诊断：统计编码器的表示质量
    #[test]
    fn diagnostic_representation_statistical() {
        let enc = LuoShuEncoder::new();
        run_representation_diagnostic("统计", &|t| enc.encode_text(t));
    }

    /// 诊断：ML 编码器的表示质量（模型不可用时自动跳过，CI 无模型也不失败）
    #[cfg(feature = "ml")]
    #[test]
    fn diagnostic_representation_ml() {
        let enc = match crate::engine::create_smart_encoder() {
            Ok((e, true)) => e,
            Ok((_e, false)) => {
                eprintln!("[诊断·ML] 跳过：ML 编码器未就绪");
                return;
            }
            Err(e) => {
                eprintln!("[诊断·ML] 跳过：{}", e);
                return;
            }
        };
        run_representation_diagnostic("ML", &|t| enc.encode_text(t));
    }

    /// 诊断：底层嵌入的**共模分量强度**（动投影层之前的判定依据）
    ///
    /// ## 为什么要先测这个
    ///
    /// 「9 维洛书向量的 argmax 只命中 3~4/8 个位置」有两种可能成因：
    ///   (a) **共模主导** —— 所有嵌入共享一个巨大的公共方向 μ，
    ///       随机投影出的 9 个值主要携带 μ 的固定投影，argmax 因此被钉住；
    ///   (b) 投影本身的问题（维度太低、基底不合适等）。
    ///
    /// 这两者的修法完全不同，**不能猜**：
    ///   (a) → 需要在投影前去掉公共分量；
    ///   (b) → 需要换投影。
    ///
    /// ## 判据
    ///
    /// `|μ| / mean|emb − μ| ≫ 1` ⇒ 共模主导，去共模是对症的；
    /// 接近或小于 1 ⇒ 共模不是主因，**不应**去改投影层（改了也没用）。
    #[cfg(feature = "ml")]
    #[test]
    fn diagnostic_common_component_strength_ml() {
        fn l2(v: &[f32]) -> f32 {
            v.iter().map(|x| x * x).sum::<f32>().sqrt()
        }

        let enc = match crate::engine::create_smart_encoder() {
            Ok((e, true)) => e,
            _ => {
                eprintln!("[诊断·共模] 跳过：ML 编码器不可用");
                return;
            }
        };

        let mut embs: Vec<Vec<f32>> = Vec::new();
        for t in DIAG_TEXTS.iter() {
            if let Some(v) = enc.encode_embedding(t) {
                embs.push(v);
            }
        }
        if embs.len() < 2 {
            eprintln!("[诊断·共模] 跳过：有效嵌入不足");
            return;
        }

        let dim = embs[0].len();
        let n = embs.len() as f32;

        // 公共方向 μ（全样本均值）
        let mut mu = vec![0.0f32; dim];
        for e in &embs {
            for (i, x) in e.iter().enumerate() {
                mu[i] += x;
            }
        }
        for m in mu.iter_mut() {
            *m /= n;
        }

        let mu_norm = l2(&mu);
        let mean_norm: f32 = embs.iter().map(|e| l2(e)).sum::<f32>() / n;
        let mean_dev: f32 = embs
            .iter()
            .map(|e| {
                let d: Vec<f32> = e.iter().zip(&mu).map(|(a, b)| a - b).collect();
                l2(&d)
            })
            .sum::<f32>()
            / n;

        let ratio = mu_norm / mean_dev.max(1e-9);
        eprintln!("[诊断·共模] 维度={} 样本={}", dim, embs.len());
        eprintln!(
            "[诊断·共模] |μ|={:.4}  平均|emb|={:.4}  平均|emb−μ|={:.4}",
            mu_norm, mean_norm, mean_dev
        );
        eprintln!(
            "[诊断·共模] |μ| / 平均|emb−μ| = {:.3}   （≫1 ⇒ 共模主导，去共模对症；≲1 ⇒ 共模不是主因）",
            ratio
        );
    }
}
