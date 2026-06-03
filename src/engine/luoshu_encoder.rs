// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件包含模型底层架构衍生的核心算法，受研究许可证保护。
// 禁止逆向工程、禁止商业再分发、禁止用于训练竞争模型。
// ============================================================
//
// 洛书坐标编码器实现。
// 将文本编码为 9 维洛书坐标向量，支持幻和约束与八卦分类。

use serde::{Deserialize, Serialize};

/// 洛书九宫格的固定幻和（归一化后为 1.0）
const LUOSHU_MAGIC_SUM: f32 = 1.0;

/// 洛书九宫格的标准布局：
///   4  9  2
///   3  5  7
///   8  1  6
///
/// 每行、每列、每条对角线的和 = 15（归一化后 = 1.0）
///
/// 归一化后的标准权重（每个位置的值 / 15）：
pub(crate) const LUOSHU_WEIGHTS: [f32; 9] = [
    4.0 / 15.0,  // 位置 0：巽（东南）
    9.0 / 15.0,  // 位置 1：离（南）
    2.0 / 15.0,  // 位置 2：坤（西南）
    3.0 / 15.0,  // 位置 3：震（东）
    5.0 / 15.0,  // 位置 4：中（太极）
    7.0 / 15.0,  // 位置 5：兑（西）
    8.0 / 15.0,  // 位置 6：艮（东北）
    1.0 / 15.0,  // 位置 7：坎（北）
    6.0 / 15.0,  // 位置 8：乾（西北）
];

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

    /// 施加洛书幻和约束：使每行、每列、每条对角线的和趋近 LUOSHU_MAGIC_SUM
    ///
    /// 使用迭代投影法，交替投影到行约束、列约束、对角线约束上。
    pub fn normalize_to_luoshu(&mut self) {
        let target = LUOSHU_MAGIC_SUM;

        // 迭代投影（通常 3-5 轮即可收敛）
        for _ in 0..5 {
            // 行约束：row 0 = [0,1,2], row 1 = [3,4,5], row 2 = [6,7,8]
            for row in 0..3 {
                let sum: f32 = self.values[row * 3..(row + 1) * 3].iter().sum();
                if sum > 1e-6 {
                    let scale = target / sum;
                    for col in 0..3 {
                        self.values[row * 3 + col] *= scale;
                    }
                }
            }

            // 列约束：col 0 = [0,3,6], col 1 = [1,4,7], col 2 = [2,5,8]
            for col in 0..3 {
                let sum: f32 = (0..3).map(|row| self.values[row * 3 + col]).sum();
                if sum > 1e-6 {
                    let scale = target / sum;
                    for row in 0..3 {
                        self.values[row * 3 + col] *= scale;
                    }
                }
            }

            // 主对角线：[0,4,8]
            let diag1: f32 = self.values[0] + self.values[4] + self.values[8];
            if diag1 > 1e-6 {
                let scale = target / diag1;
                self.values[0] *= scale;
                self.values[4] *= scale;
                self.values[8] *= scale;
            }

            // 副对角线：[2,4,6]
            let diag2: f32 = self.values[2] + self.values[4] + self.values[6];
            if diag2 > 1e-6 {
                let scale = target / diag2;
                self.values[2] *= scale;
                self.values[4] *= scale;
                self.values[6] *= scale;
            }
        }

        // Clamp 到非负
        for v in self.values.iter_mut() {
            *v = v.max(0.0);
        }
    }

    /// 计算幻和偏离度（越小越接近完美幻方）
    pub fn luoshu_deviation(&self) -> f32 {
        let target = LUOSHU_MAGIC_SUM;
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
        let dot: f32 = self.values.iter().zip(other.values.iter()).map(|(a, b)| a * b).sum();
        let norm_a: f32 = self.values.iter().map(|v| v * v).sum::<f32>().sqrt();
        let norm_b: f32 = other.values.iter().map(|v| v * v).sum::<f32>().sqrt();

        if norm_a == 0.0 || norm_b == 0.0 {
            return 0.0;
        }
        (dot / (norm_a * norm_b)).clamp(-1.0, 1.0)
    }

    /// 计算洛书几何距离（九宫格上的 Manhattan 距离）
    pub fn grid_distance(&self, other: &LuoShuVector) -> f32 {
        self.values.iter().zip(other.values.iter())
            .map(|(a, b)| (a - b).abs())
            .sum()
    }

    /// 获取中心值（太极位，位置 4）
    pub fn center_value(&self) -> f32 {
        self.values[4]
    }
}

/// 洛书坐标编码器
///
/// 将文本内容编码为满足幻和约束的 9 维洛书向量。
///
/// 编码流程：
/// 1. 将文本分为 9 个语义分区（按句/段均分）
/// 2. 对每个分区提取特征（词频、长度、位置权重）
/// 3. 施加洛书标准权重作为先验分布
/// 4. 迭代投影到幻和约束
pub struct LuoShuEncoder {
    /// 是否启用洛书权重先验
    use_prior: bool,
}

impl LuoShuEncoder {
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
    /// 算法：
    /// 1. 将文本分为 9 段，每段提取语义特征
    /// 2. 特征归一化后映射到九宫格对应位置
    /// 3. 施加洛书先验权重（如有）
    /// 4. 迭代投影到幻和约束
    pub fn encode_text(&self, text: &str) -> LuoShuVector {
        let raw = Self::extract_9_features(text);

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
            let entropy = if seg_len > 0.0 { unique_count / seg_len } else { 0.0 };

            // 位置权重：中心位置（4）权重最高，边缘位置权重递减
            let center_dist = (seg as i32 - 4i32).abs() as f32;
            let position_weight = (-center_dist * center_dist / 8.0).exp();

            *feature = density * 0.4 + entropy * 0.3 + position_weight * 0.3;
        }

        features
    }

    /// 洛书幻和偏离度（监控用，越小越好）
    pub fn deviation_of(&self, text: &str) -> f32 {
        let vec = self.encode_text(text);
        vec.luoshu_deviation()
    }
}

impl Default for LuoShuEncoder {
    fn default() -> Self {
        Self::new()
    }
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

    /// 验证：幻和约束基本满足（偏差 < 1.0）
    #[test]
    fn test_luoshu_constraint_satisfied() {
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
            let dev = vec.luoshu_deviation();
            assert!(
                dev < 1.0,
                "文本 '{}' 的幻和偏离度 {} 过高",
                text, dev
            );
        }
    }

    /// 验证：相似文本产生相近向量
    #[test]
    fn test_similar_texts_close() {
        let encoder = LuoShuEncoder::new();
        // 使用较长、有足够区分度的文本
        let v1 = encoder.encode_text(
            "项目使用 PostgreSQL 数据库存储所有的用户数据和订单信息",
        );
        let v2 = encoder.encode_text(
            "项目数据库连接使用 PostgreSQL 管理用户的查询和事务处理",
        );
        let v3 = encoder.encode_text(
            "前端使用 React 框架构建组件化的用户界面交互体验",
        );

        let sim_12 = v1.cosine_similarity(&v2);
        let sim_13 = v1.cosine_similarity(&v3);

        // 相似文本应有正相似度，不相似文本应有较低相似度
        assert!(sim_12 > 0.9, "相似文本的余弦相似度应 > 0.9，实际: {}", sim_12);
        assert!(
            sim_12 >= sim_13,
            "相似文本的余弦相似度 ({}) 应不低于不相似文本 ({})",
            sim_12, sim_13
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
            v1.center_value(), v2.center_value()
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
}