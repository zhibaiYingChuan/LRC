// URL 安全实现唯一来源：主 crate 的 src/url_safety.rs。
// 桌面端与主 crate 共用同一份解析、IP、hostname 和 DNS 校验逻辑，避免实现漂移。
include!("../../../src/url_safety.rs");
