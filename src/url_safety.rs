// ============================================================
// Loong Recall (LRC) — URL 安全校验模块（SSRF 防护）
// ============================================================
//
// 用途：集中校验所有由用户输入驱动的 HTTP(S) 目标地址，
//       防止 SSRF（服务端请求伪造）：
//   - 拒绝云 metadata 地址（169.254.169.254 等 link-local）
//   - 拒绝未指定/组播/广播地址
//   - 拒绝 URL 内嵌 userinfo（user:pass@host）混淆
//
// 私网策略（2026-08-30 用户决策）：
//   放行私网（10/8、172.16/12、192.168/16）与 IPv6 唯一本地地址，
//   以兼容局域网 Ollama 与内网 OpenAI 兼容代理；仅拒绝云 metadata、
//   链路本地、未指定/组播/广播等高风险网段。
//
// 兼容性说明：localhost / 127.0.0.1 / ::1（回环）被明确允许，
//   因为 Ollama 等本地 LLM 服务是核心合法用例。
//
// 使用场景：
//   - /v1/config/llm/test   —— 测试连接转发
//   - /v1/config/llm        —— 持久化自定义 base_url
//   - LlmApiConfig::parse   —— 配置解析入口（同步，仅字面量检查）
//   - Tauri test_llm_connection —— 桌面端测试连接
//
// ⚠ 本文件**必须**使用 `//` 行注释而非 `//!` 内部文档注释（v0.9.7 核实）：
//   桌面端 desktop/src-tauri/src/url_safety.rs 通过 `include!()` 引入本文件，
//   而 `include!` 展开处不允许出现内部文档注释，否则报 E0753
//   （expected outer doc comment）。改为 `//!` 会导致桌面端 crate 无法编译。
//   因此本文件是全仓唯一**刻意**不做模块级 `//!` 文档化的顶层模块。

use std::net::{IpAddr, Ipv4Addr};
use url::{Host, Url};

// ---------------------------------------------------------------------------
// 类型化错误（v0.9.7，GLOBAL_CODE_REVIEW_REPORT P1-6「统一错误处理」的安全关键子集）
//
// 背景：本模块原以 `Result<_, String>` 表达失败，调用方只能拿到一个不可判别的
//   字符串。SSRF 校验属安全边界，调用方需要区分"地址本身非法（用户输入问题）"
//   与"DNS 层失败（网络/重绑定问题）"，以便分别给出处置（改配置 / 重试）。
//
// 约束（务必遵守）：本文件被桌面端 `include!()` 引入，**不得引入任何新依赖**
//   （`thiserror` 未出现在主 crate 与桌面端 crate 的依赖表中），故此处手写
//   `Display` 与 `std::error::Error`。文案与改造前**逐字保持一致**，
//   以保证前端/日志中的用户可见消息零漂移。
// ---------------------------------------------------------------------------

/// URL 安全校验失败原因
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlSafetyError {
    /// 主机名使用十进制/十六进制/八进制等编码形式（防 SSRF 混淆绕过）
    EncodedHostname,
    /// URL 无法解析
    InvalidUrl(String),
    /// scheme 非 http/https
    UnsupportedScheme,
    /// URL 内嵌 userinfo（user:pass@host）
    UserInfoNotAllowed,
    /// 端口非法（0 / 非数字 / 格式错误）
    InvalidPort,
    /// URL 缺少主机名
    MissingHost,
    /// 目标 IP 位于受保护网段（云 metadata / 链路本地 / 未指定 / 组播 / 广播）
    ProtectedAddress(String),
    /// 主机名不符合命名规则
    InvalidHostname(String),
    /// DNS 解析超时
    DnsTimeout(String),
    /// DNS 解析失败（含底层错误描述）
    DnsResolveFailed {
        /// 被解析的主机名
        host: String,
        /// 底层解析错误描述
        detail: String,
    },
    /// 域名解析无任何结果（fail-closed）
    DnsNoResult(String),
    /// 域名解析到多个地址，HTTP 客户端无法无风险绑定完整校验结果（fail-closed）
    DnsAmbiguous(String),
    /// 域名解析到受保护地址，疑似 DNS rebinding
    DnsRebinding {
        /// 被解析的主机名
        host: String,
        /// 解析到的受保护地址
        ip: IpAddr,
    },
}

impl std::fmt::Display for UrlSafetyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EncodedHostname => write!(f, "不允许使用数字编码主机名"),
            Self::InvalidUrl(detail) => write!(f, "URL 格式非法: {detail}"),
            Self::UnsupportedScheme => write!(f, "不支持的 URL scheme（仅支持 http/https）"),
            Self::UserInfoNotAllowed => write!(f, "URL 不允许包含 userinfo（user:pass@host）"),
            Self::InvalidPort => write!(f, "URL 端口非法"),
            Self::MissingHost => write!(f, "URL 缺少主机名"),
            Self::ProtectedAddress(ip) => write!(f, "目标地址位于受保护网段，已拒绝: {ip}"),
            Self::InvalidHostname(host) => write!(f, "主机名非法: {host}"),
            Self::DnsTimeout(host) => write!(f, "DNS 解析超时（5s）: {host}"),
            Self::DnsResolveFailed { host, detail } => {
                write!(f, "DNS 解析失败（{host}）: {detail}")
            }
            Self::DnsNoResult(host) => write!(f, "域名 {host} 解析无结果，已拒绝"),
            Self::DnsAmbiguous(host) => write!(
                f,
                "域名 {host} 解析到多个地址，当前 HTTP 客户端无法无风险绑定完整校验结果，已拒绝"
            ),
            Self::DnsRebinding { host, ip } => write!(
                f,
                "域名 {host} 解析到受保护地址（{ip}），已拒绝（疑似 DNS rebinding）"
            ),
        }
    }
}

impl std::error::Error for UrlSafetyError {}

/// URL 安全校验结果类型别名
pub type UrlSafetyResult<T> = Result<T, UrlSafetyError>;

/// 校验 HTTP(S) URL 的字面量安全属性（不发起 DNS 解析）。
///
/// 规则：
/// 1. scheme 必须为 http/https（大小写不敏感，统一转小写比较）
/// 2. 拒绝 URL 内嵌 userinfo（`http://user:pass@host`）
/// 3. host 必须是合法的 IP 或域名
/// 4. host 解析为 IP 后执行危险网段检查
/// 5. 回环地址（localhost / 127.0.0.1 / ::1）允许
///
/// 注意：域名不做 DNS 解析（同步上下文无法异步解析），
/// 调用方在异步上下文中应额外调用 [check_dns_safety] 防 DNS rebinding。
pub fn validate_http_url(url: &str) -> UrlSafetyResult<()> {
    let trimmed = url.trim();
    let authority = trimmed
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or_default())
        .unwrap_or_default();
    let raw_host = authority
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(authority);
    let raw_host = raw_host
        .rsplit_once(':')
        .map(|(host, _)| host)
        .unwrap_or(raw_host)
        .trim_matches(['[', ']']);
    // 拒绝非标准 IPv4 编码形式（纯整数、十六进制 0x、八进制前导 0、省略段如 127.1）。
    // 这些混淆形式会被 url crate 规范化为回环/私网地址，若不预检可绕过网段检查。
    if is_encoded_ipv4(raw_host) {
        return Err(UrlSafetyError::EncodedHostname);
    }
    let parsed = Url::parse(trimmed).map_err(|e| UrlSafetyError::InvalidUrl(e.to_string()))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(UrlSafetyError::UnsupportedScheme);
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(UrlSafetyError::UserInfoNotAllowed);
    }
    if parsed.port() == Some(0)
        || url.trim().contains(":/")
            && parsed
                .host_str()
                .is_some_and(|h| url.trim().contains(&format!("{h}:/")))
    {
        return Err(UrlSafetyError::InvalidPort);
    }
    match parsed.host().ok_or(UrlSafetyError::MissingHost)? {
        Host::Ipv4(ip) => {
            if !is_safe_ipv4(ip) {
                return Err(UrlSafetyError::ProtectedAddress(ip.to_string()));
            }
        }
        Host::Ipv6(ip) => {
            if !is_safe_ip(IpAddr::V6(ip)) {
                return Err(UrlSafetyError::ProtectedAddress(ip.to_string()));
            }
        }
        Host::Domain(host) => {
            if !is_plausible_hostname(host) {
                return Err(UrlSafetyError::InvalidHostname(host.to_string()));
            }
        }
    }
    Ok(())
}

/// 异步 DNS 解析检查：解析 host 并验证所有返回的 IP 均为安全网段。
///
/// 用于防 DNS rebinding：域名首次解析可能指向公网，被允许后
/// 第二次解析却指向内网。此处一次性解析并复核所有候选 IP。
/// 仅在 server feature 下可用（依赖 tokio net）。
#[cfg(feature = "server")]
pub async fn check_dns_safety(url: &str) -> UrlSafetyResult<()> {
    resolve_and_check_dns(url).await.map(|_| ())
}

/// 在同一次 DNS 解析结果上完成校验，并返回可绑定的连接地址。
/// 调用方必须把返回地址注入 HTTP 客户端，避免校验解析与实际连接之间再次解析。
pub async fn resolve_and_check_dns(url: &str) -> UrlSafetyResult<Vec<std::net::IpAddr>> {
    // 先执行完整的 URL 字面量校验，确保直接返回的 IP 地址也经过
    // scheme、userinfo、端口及受保护网段检查。
    validate_http_url(url)?;
    let parsed = Url::parse(url.trim()).map_err(|e| UrlSafetyError::InvalidUrl(e.to_string()))?;
    let host = match parsed.host() {
        Some(Host::Domain(domain)) => domain.to_string(),
        Some(Host::Ipv4(ip)) => return Ok(vec![IpAddr::V4(ip)]),
        Some(Host::Ipv6(ip)) => return Ok(vec![IpAddr::V6(ip)]),
        None => return Err(UrlSafetyError::MissingHost),
    };

    // 2026-09-01 修复(P1)：DNS 校验必须拥有独立超时——调用方（如
    // test_llm_connection）的并发许可覆盖整个请求生命周期，若 DNS 解析
    // 无兜底超时，在 DNS 服务异常/网络栈阻塞时会长期占用唯一并发槽，
    // 形成可被恶意输入放大的拒绝服务窗口。
    let iter = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::net::lookup_host((host.clone(), 0)),
    )
    .await
    .map_err(|_| UrlSafetyError::DnsTimeout(host.clone()))?
    .map_err(|e| UrlSafetyError::DnsResolveFailed {
        host: host.clone(),
        detail: e.to_string(),
    })?;
    let addrs: Vec<std::net::SocketAddr> = iter.collect();
    // fail-closed：域名解析不到任何地址时直接拒绝，交由用户检查网络/拼写
    if addrs.is_empty() {
        return Err(UrlSafetyError::DnsNoResult(host));
    }
    let ips: Vec<IpAddr> = addrs.into_iter().map(|addr| addr.ip()).collect();
    // reqwest 0.12 的 resolve(host, addr) 只提供单一覆盖地址；重复调用
    // 不能可靠地表达经过校验的多地址候选集。为避免只绑定首个地址而把
    // 其他候选留给连接器重新解析，这里对多地址结果 fail-closed。
    if ips.len() > 1 {
        return Err(UrlSafetyError::DnsAmbiguous(host));
    }
    for ip in &ips {
        if !is_safe_ip(*ip) {
            return Err(UrlSafetyError::DnsRebinding {
                host: host.clone(),
                ip: *ip,
            });
        }
    }
    Ok(ips)
}

/// 从 URL authority 中提取主机名（兼容 host、host:port、[ipv6]:port 形式）。
/// 仅测试使用（生产路径经 url crate 直接解析）。
#[cfg(test)]
fn extract_host(authority: &str) -> UrlSafetyResult<String> {
    let parsed = Url::parse(&format!("http://{}", authority))
        .map_err(|e| UrlSafetyError::InvalidUrl(e.to_string()))?;
    if parsed.username().is_empty() {
        match parsed.host() {
            Some(Host::Domain(domain)) => Ok(domain.to_string()),
            Some(Host::Ipv4(ip)) => Ok(ip.to_string()),
            Some(Host::Ipv6(ip)) => Ok(ip.to_string()),
            None => Err(UrlSafetyError::MissingHost),
        }
    } else {
        Err(UrlSafetyError::UserInfoNotAllowed)
    }
}

/// 判断字符串是否为非标准形式的 IPv4 编码（防 SSRF 混淆绕过）。
///
/// url crate 会把以下形态规范化为普通 IPv4 地址，若不加预检，
/// 攻击者可借此把回环/私网地址伪装成看似合法的 host 绕过网段检查：
///   - 纯十进制整数（2130706433 → 127.0.0.1）
///   - 十六进制标签（0x7f.0.0.1 → 127.0.0.1）
///   - 八进制前导零（0177.0.0.1 → 127.0.0.1）
///   - 省略段（127.1 → 127.0.0.1）
///     标准四段点分十进制（如 127.0.0.1、192.168.1.10）不属于编码形式，返回 false。
fn is_encoded_ipv4(host: &str) -> bool {
    if host.is_empty() {
        return false;
    }
    // 纯数字且含十六进制/前导零标记的标签，视为编码形式
    let labels: Vec<&str> = host.split('.').collect();
    let all_numeric = labels
        .iter()
        .all(|label| !label.is_empty() && label.chars().all(|c| c.is_ascii_digit()));
    if all_numeric {
        // 纯整数形式（单标签，如 2130706433）或省略段（非四段，如 127.1）
        if labels.len() != 4 {
            return true;
        }
        // 八进制前导零（如 0177.0.0.1）
        if labels
            .iter()
            .any(|label| label.len() > 1 && label.starts_with('0'))
        {
            return true;
        }
        return false;
    }
    // 十六进制标签（如 0x7f.0.0.1 / 0X7f）
    labels
        .iter()
        .any(|label| label.starts_with("0x") || label.starts_with("0X"))
}

/// 判断 IP 是否位于受保护网段。
///
/// 允许：回环（127.0.0.0/8、::1）、私网、IPv6 唯一本地地址（用户决策放行局域网）
/// 拒绝：未指定、组播、广播、链路本地（含云 metadata）、保留段
fn is_safe_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_safe_ipv4(v4),
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return true;
            }
            // IPv4-mapped IPv6 地址（::ffff:192.168.1.1）按 IPv4 规则判断
            if let Some(v4) = v6.to_ipv4_mapped() {
                return !v4.is_loopback() && is_safe_ipv4(v4);
            }
            // 未指定/组播/链路本地拒绝；唯一本地（fd00::/8）放行（IPv6 私网）
            // 注：旧工具链无 Ipv6Addr::is_link_local，手动判断 fe80::/10
            let segs = v6.segments();
            let is_link_local = (segs[0] & 0xffc0) == 0xfe80;
            !v6.is_unspecified() && !v6.is_multicast() && !is_link_local
        }
    }
}

fn is_safe_ipv4(v4: Ipv4Addr) -> bool {
    if v4.is_loopback() {
        return true;
    }
    if v4.is_unspecified() || v4.is_multicast() || v4.is_broadcast() {
        return false;
    }
    // 链路本地 169.254.0.0/16（含云 metadata 169.254.169.254）
    if v4.octets()[0] == 169 && v4.octets()[1] == 254 {
        return false;
    }
    // 保留段 0.0.0.0/8
    if v4.octets()[0] == 0 {
        return false;
    }
    // 私网网段放行（10/8、172.16/12、192.168/16）——局域网 Ollama/内网代理合法用例
    true
}

/// 域名基本合法性：仅允许字母、数字、连字符、下划线与点，
/// 且标签级校验（非空、不以连字符开头/结尾、长度上限）。
/// 本机主机名（localhost 及其子域）单独放行。
fn is_plausible_hostname(host: &str) -> bool {
    let lower = host.to_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") {
        return true;
    }
    // 纯数字/纯十六进制等会被 url crate 规范化为 IP 字面量（如 2130706433 → 127.0.0.1），
    // 这里只处理真正的域名；拒绝任何可能被 DNS 解析器当 IP 处理的残留形态。
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    if host
        .chars()
        .any(|c| !c.is_ascii_alphanumeric() && c != '-' && c != '.' && c != '_')
    {
        return false;
    }
    // 拒绝十进制、十六进制、八进制及省略段的数字地址写法。
    let labels = host.split('.').collect::<Vec<_>>();
    if labels.len() > 1
        && labels
            .iter()
            .all(|label| label.chars().all(|c| c.is_ascii_digit()))
        && labels
            .iter()
            .any(|label| label.len() > 1 && label.starts_with('0'))
    {
        return false;
    }
    if labels.len() > 1 && labels.iter().all(|label| label.starts_with("0x")) {
        return false;
    }
    if labels.len() == 1 && host.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    for label in host.split('.') {
        if label.is_empty() || label.len() > 63 || label.starts_with('-') || label.ends_with('-') {
            return false;
        }
    }
    !host.starts_with('.') && !host.ends_with('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allow_loopback_and_public() {
        assert!(validate_http_url("http://localhost:11434").is_ok());
        assert!(validate_http_url("https://127.0.0.1:8080/v1").is_ok());
        assert!(validate_http_url("http://[::1]:11434/api").is_ok());
        assert!(validate_http_url("https://api.openai.com/v1").is_ok());
        assert!(validate_http_url("https://example.com:8443/path").is_ok());
        let expected = "example.com";
        assert_eq!(extract_host(&format!("{expected}:8443")).unwrap(), expected);
        assert_eq!(extract_host("[::1]:11434").unwrap(), "::1");
    }

    #[test]
    fn test_allow_private_networks() {
        // 用户决策：放行私网，兼容局域网 Ollama 与内网代理
        assert!(validate_http_url("http://192.168.1.10:8080").is_ok());
        assert!(validate_http_url("http://10.0.0.1/v1").is_ok());
        assert!(validate_http_url("http://172.16.0.1:11434").is_ok());
        // 云 metadata 与保留段仍拒绝
        assert!(validate_http_url("http://169.254.169.254/latest/meta-data").is_err());
        assert!(validate_http_url("http://0.0.0.0:8080").is_err());
    }

    #[test]
    fn test_reject_bad_scheme_and_userinfo() {
        assert!(validate_http_url("ftp://example.com").is_err());
        assert!(validate_http_url("file:///etc/passwd").is_err());
        assert!(validate_http_url("http://user:pass@example.com").is_err());
        assert!(validate_http_url("http://user@example.com").is_err());
        assert!(validate_http_url("http://user:pass@169.254.169.254/").is_err());
        assert!(validate_http_url("//example.com/path").is_err());
    }

    #[test]
    fn test_reject_ipv6_dangerous() {
        assert!(validate_http_url("http://[::]:8080").is_err());
        assert!(validate_http_url("http://[fe80::1]/").is_err());
        // IPv6 唯一本地（fd00::/8）与 IPv4-mapped 私网放行
        assert!(validate_http_url("http://[fd00::1]/").is_ok());
        assert!(validate_http_url("http://[::ffff:10.0.0.1]/").is_ok());
        // IPv4-mapped 回环必须拒绝（防绕避免检）
        assert!(validate_http_url("http://[::ffff:127.0.0.1]:8080/").is_err());
        assert!(validate_http_url("http://[::ffff:169.254.169.254]/").is_err());
        assert!(validate_http_url("http://[::ffff:7f00:1]/").is_err());
    }

    #[test]
    fn test_reject_special_ip_encodings() {
        // 特殊 IP 字面量：十进制/十六进制/八进制/省略零形式统一由 url
        // 规范化为 IP，其中指向回环/链路本地的必须被拒绝（防 SSRF 绕过）
        assert!(validate_http_url("http://2130706433:8080/").is_err());
        assert!(validate_http_url("http://0177.0.0.1/").is_err());
        assert!(validate_http_url("http://127.1/").is_err());
        assert!(validate_http_url("http://0x7f.0.0.1/").is_err());
        // 域名形式的 IP 绕过（如 *.nip.io）字面校验通过（hostname 合法），
        // 其拦截依赖 check_dns_safety 的解析复核（DNS rebinding 防护）。
        assert!(validate_http_url("http://127.0.0.1.nip.io/").is_ok());
    }

    #[test]
    fn test_reject_invalid_port_and_hostname() {
        assert!(validate_http_url("http://example.com:99999/").is_err());
        assert!(validate_http_url("http://example.com:abc/").is_err());
        assert!(validate_http_url("http://example.com:0/").is_err());
        assert!(validate_http_url("http://example.com:/").is_err());
        assert!(validate_http_url("http://-bad.example.com/").is_err());
        assert!(validate_http_url("http://bad-.example.com/").is_err());
        assert!(validate_http_url("http://.example.com/").is_err());
        assert!(validate_http_url("http://example..com/").is_err());
        assert!(validate_http_url("http://example.com./").is_err());
        assert!(validate_http_url("http://exa mple.com/").is_err());
        assert!(validate_http_url("http://example.com:1:2/").is_err());
        assert!(validate_http_url("http://127.0.0.1:0/").is_err());
        assert!(validate_http_url("http://example.com").is_ok());
        assert!(validate_http_url("http://host:65535/").is_ok());
    }

    #[cfg(feature = "server")]
    #[test]
    fn test_resolve_returns_the_validated_literal_target() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let ips = rt
            .block_on(resolve_and_check_dns("https://127.0.0.1:8443/v1"))
            .unwrap();
        assert_eq!(ips, vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]);
        assert!(rt
            .block_on(resolve_and_check_dns("https://169.254.169.254/latest"))
            .is_err());
    }

    // check_dns_safety 仅在 server 特性下编译，测试同步门控
    #[cfg(feature = "server")]
    #[test]
    fn test_dns_rebinding_references() {
        // DNS 空结果验证依赖真实 DNS，无法离线稳定测试：
        // 至少保证带尾点域名的校验路径可执行（结果依赖于 DNS 环境，
        // 因此仅断言对纯 IP 字面量快捷通过的行为）。
        let rt = tokio::runtime::Runtime::new().unwrap();
        // IPv4 字面量直接返 Ok（不触发 DNS）
        assert!(rt.block_on(check_dns_safety("http://10.0.0.1/")).is_ok());
    }
}
