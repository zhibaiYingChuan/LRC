//! ============================================================
//! 许可证: Apache 2.0
//! 本文件实现统一错误契约，属于公开层 (Layer 1)。
//! ============================================================
//!
//! 统一错误契约（v0.9.7，GLOBAL_CODE_REVIEW_REPORT P1-6）
//!
//! 背景
//! ----
//! 报告 P1-6 指出：全仓以 `Result<_, String>` 表达失败达 74+ 处，
//! 调用方只能拿到**不可判别**的字符串——无法据此分支（如区分
//! "用户输入非法"与"IO 失败"），也无法在测试中稳定断言错误类别。
//!
//! 设计取舍（六钥匙·简化 / 泛化）
//! -----------------------------
//! 现状取证表明：这 100+ 处绝大多数是**"带上下文的单一失败"**，
//! 调用方不需要为每个函数发明特有变体。因此这里**不**为每个模块定义
//! 独立错误枚举（那会制造大量一次性类型），而是提供一个
//! **统一类型 + 域分类标签**：
//!   - `ErrorKind` 提供**粗粒度可判别性**（调用方可 `match e.kind`）
//!   - `message` 保留**与改造前逐字一致**的文案（前端/日志零漂移）
//!
//! 兼容性保证（关键）
//! ----------------
//! 1. `impl Display` 仅输出 `message` → 既有 `format!("{}", e)` 输出不变；
//! 2. `impl From<LrcError> for String` → 处于 `Result<_, String>` 上下文的
//!    调用方仍可用 `?` 自动上浮（**允许渐进迁移，不必一次性改全仓**）；
//! 3. `impl std::error::Error` → 可与 `anyhow` / `source()` 链协作。
//!
//! 边界说明
//! --------
//! - `src/url_safety.rs` **不使用**本模块：该文件被桌面端 `include!()` 引入，
//!   不能引入主 crate 路径依赖，故自带 `UrlSafetyError`（见 c4）；
//! - 桌面端 Tauri 命令边界使用自身的 `CommandError`（序列化为字符串，
//!   保持前端 `catch (e)` 契约不变），不跨 crate 共享本类型。

use std::fmt;

/// 错误域分类（供调用方粗粒度判别失败性质）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// IO / 文件系统失败
    Io,
    /// 解析/反序列化失败
    Parse,
    /// 加解密与密钥管理失败
    Crypto,
    /// 配置读写失败
    Config,
    /// 网络/HTTP 失败
    Network,
    /// 调用方传入的参数非法
    InvalidInput,
    /// 目标不存在
    NotFound,
    /// 超时
    Timeout,
    /// 功能/后端不支持
    Unsupported,
    /// 内部不变量被破坏
    Internal,
}

impl ErrorKind {
    /// 稳定的机器可读标识（用于日志/序列化，不用于展示）
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Io => "io",
            Self::Parse => "parse",
            Self::Crypto => "crypto",
            Self::Config => "config",
            Self::Network => "network",
            Self::InvalidInput => "invalid_input",
            Self::NotFound => "not_found",
            Self::Timeout => "timeout",
            Self::Unsupported => "unsupported",
            Self::Internal => "internal",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 统一错误类型：域分类 + 用户可见消息
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LrcError {
    /// 失败所属域（可判别）
    pub kind: ErrorKind,
    /// 面向用户/日志的消息（与改造前文案逐字一致）
    pub message: String,
}

impl LrcError {
    /// 以指定域构造错误
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// IO / 文件系统失败
    pub fn io(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Io, message)
    }

    /// 解析失败
    pub fn parse(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Parse, message)
    }

    /// 加解密/密钥失败
    pub fn crypto(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Crypto, message)
    }

    /// 配置失败
    pub fn config(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Config, message)
    }

    /// 网络失败
    pub fn network(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Network, message)
    }

    /// 参数非法
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidInput, message)
    }

    /// 目标不存在
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, message)
    }

    /// 超时
    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Timeout, message)
    }

    /// 不支持
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unsupported, message)
    }

    /// 内部错误
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Internal, message)
    }

    /// 判断是否属于指定域
    pub fn is(&self, kind: ErrorKind) -> bool {
        self.kind == kind
    }
}

impl fmt::Display for LrcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 仅输出 message：保证与改造前的用户可见文案逐字一致
        f.write_str(&self.message)
    }
}

impl std::error::Error for LrcError {}

impl From<std::io::Error> for LrcError {
    fn from(error: std::io::Error) -> Self {
        Self::io(error.to_string())
    }
}

impl From<LrcError> for String {
    fn from(error: LrcError) -> Self {
        error.message
    }
}

/// 迁移桥接：存量 `String` 错误 → `LrcError`
///
/// **用途仅限渐进迁移**：P1-6 覆盖 100+ 处签名，无法一次性全部改完；
/// 在 `Result<_, LrcError>` 的新签名内用 `?` 上浮仍是 `String` 的旧调用时，
/// 由本实现兜底，归入 [`ErrorKind::Internal`]（域未知）。
///
/// 新代码**不应**依赖本实现隐式分类——请在出错点显式选择 `ErrorKind`
/// （如 `LrcError::io(...)` / `LrcError::invalid_input(...)`），
/// 否则调用方仍无法据 `kind` 分支，P1-6 的收益将被抵消。
impl From<String> for LrcError {
    fn from(message: String) -> Self {
        Self::new(ErrorKind::Internal, message)
    }
}

impl From<&str> for LrcError {
    fn from(message: &str) -> Self {
        Self::new(ErrorKind::Internal, message)
    }
}

/// 统一错误结果别名
pub type LrcResult<T> = Result<T, LrcError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_outputs_message_only() {
        // 契约：Display 不得附加域前缀，否则前端/日志文案会漂移
        let e = LrcError::crypto("DPAPI 加密失败");
        assert_eq!(e.to_string(), "DPAPI 加密失败");
    }

    #[test]
    fn into_string_keeps_message() {
        let e = LrcError::config("配置解析失败");
        let s: String = e.into();
        assert_eq!(s, "配置解析失败");
    }

    #[test]
    fn kind_is_discriminable() {
        let e = LrcError::timeout("DNS 解析超时");
        assert!(e.is(ErrorKind::Timeout));
        assert!(!e.is(ErrorKind::Io));
        assert_eq!(e.kind.as_str(), "timeout");
    }

    #[test]
    fn propagates_from_string_context() {
        // 处于 Result<_, String> 上下文的调用方应能用 `?` 自动上浮
        fn inner() -> LrcResult<()> {
            Err(LrcError::invalid_input("bad"))
        }
        fn outer() -> Result<(), String> {
            inner()?;
            Ok(())
        }
        assert_eq!(outer().unwrap_err(), "bad");
    }
}
