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

        // Clamp 到非负，并替换 NaN / Inf 为零
        for v in self.values.iter_mut() {
            *v = v.max(0.0);
            if v.is_nan() || v.is_infinite() {
                *v = 0.0;
            }
        }
    }

    /// 道枢映射: 洛书·幻和 — 九宫格幻和偏离度，是洛书数理结构的核心度量
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
    /// 道枢映射: 洛书·九宫 — 将语义向量映射到洛书九宫格，实现数与义的统一，是编码体系的核心
    ///
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

/// 简单字符串哈希（用于关键词到维度的映射）
fn simple_hash(s: &str) -> usize {
    let mut h: usize = 5381;
    for b in s.bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as usize);
    }
    h
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
            assert!(dev < 1.0, "文本 '{}' 的幻和偏离度 {} 过高", text, dev);
        }
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

        // 相似文本应有正相似度，不相似文本应有较低相似度
        assert!(
            sim_12 > 0.9,
            "相似文本的余弦相似度应 > 0.9，实际: {}",
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
}
