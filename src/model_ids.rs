//! ============================================================
//! 许可证: Apache 2.0
//! 本文件定义嵌入模型 ID 常量，属于公开层 (Layer 1)。
//! ============================================================
//!
//! 嵌入模型 ID 单一真源（Single Source of Truth）
//!
//! 背景（v0.9.7 修复，GLOBAL_CODE_REVIEW_REPORT P3 质量「模型 ID 常量重复」）：
//!   模型 ID 此前在 4 处独立硬编码——`server.rs` 白名单、`bin/server.rs` 推荐列表、
//!   `engine/model_resolver.rs` 默认选择、`engine/luoshu_encoder_ml.rs` 语言默认选择。
//!   其中存在**值不一致**：白名单要求 `intfloat/multilingual-e5-small`，
//!   而 `model list` 展示 `multilingual-e5-small`（缺 org 前缀），
//!   用户照展示值调用会被白名单拒绝。
//!
//! 设计约束：
//!   - 本模块**不加 feature 门控**（Layer 1 中立），使 Layer 2 引擎与 feature-gated
//!     的 `server` 模块都能引用，避免 Layer 2 反向依赖 `server` 造成 feature 耦合。
//!   - 所有模型 ID 必须带完整 `org/repo` 形式（HuggingFace 仓库标识），
//!     不得只写 repo 名，否则下载与白名单校验会不一致。

/// 中文默认嵌入模型（~100MB，512 维）
pub const MODEL_BGE_SMALL_ZH: &str = "BAAI/bge-small-zh";

/// 中文高精度嵌入模型（~400MB，768 维）
pub const MODEL_BGE_BASE_ZH: &str = "BAAI/bge-base-zh";

/// 英文/多语言轻量嵌入模型（~80MB，384 维）
pub const MODEL_ALL_MINILM_L6_V2: &str = "sentence-transformers/all-MiniLM-L6-v2";

/// 多语言通用嵌入模型（~120MB，384 维）
///
/// 注意：必须带 `intfloat/` 前缀——白名单校验使用完整仓库标识，
/// 若只写 `multilingual-e5-small` 会被拒绝。
pub const MODEL_MULTILINGUAL_E5_SMALL: &str = "intfloat/multilingual-e5-small";

/// 代码搜索模型（~500MB，768 维，向后兼容）
pub const MODEL_GRAPHCODEBERT_BASE: &str = "microsoft/graphcodebert-base";

/// 可下载嵌入模型白名单（服务端校验用）
///
/// 顺序即前端/仪表盘的展示顺序（默认模型在前）。
pub const AVAILABLE_EMBEDDER_MODELS: &[&str] = &[
    MODEL_BGE_SMALL_ZH,
    MODEL_ALL_MINILM_L6_V2,
    MODEL_MULTILINGUAL_E5_SMALL,
    MODEL_BGE_BASE_ZH,
];

/// 根据语言代码选择默认嵌入模型 ID
///
/// - 中文（`zh_*`）→ [`MODEL_BGE_SMALL_ZH`]（中文 SOTA）
/// - 其他 → [`MODEL_ALL_MINILM_L6_V2`]（多语言轻量）
pub fn detect_default_model_by_lang(lang: &str) -> &'static str {
    if lang.to_lowercase().starts_with("zh") {
        MODEL_BGE_SMALL_ZH
    } else {
        MODEL_ALL_MINILM_L6_V2
    }
}
