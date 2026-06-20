// ============================================================
// PoolingStrategy — BERT 编码器池化策略
//
// 从 encoder_codebert.rs 和 luoshu_encoder_ml.rs 中提取的公共枚举。
// 两个编码器共享相同的池化语义，统一在此定义以消除重复。
// ============================================================

/// BERT 编码器池化策略
///
/// - Cls: 使用 [CLS] token 的隐向量作为序列表示（适用于分类任务）
/// - Mean: 对所有 token 隐向量取平均（适用于语义相似度任务）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolingStrategy {
    Cls,
    Mean,
}