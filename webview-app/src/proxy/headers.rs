//! 转发头过滤与 Cookie 域重写（P4 大文件拆分：自 proxy.rs 垂直拆出，零行为变更纯代码搬家）。
//!
//! 条目：hop-by-hop 头判定、托管请求头、Connection 具名头解析、Set-Cookie 域重写。
//! 原 proxy.rs 内的私有条目收口为 `pub(super)`（有效可见范围不变）。

use super::*;

/// 真正逐跳的 hop-by-hop 头（不含 connection/upgrade，因为 WebSocket 握手必须原样透传它们）。
/// 这些头若被转发会让上游的连接管理混乱，且在握手请求里本就不存在（无 body）。
pub(super) fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "keep-alive"
            | "transfer-encoding"
            | "te"
            | "trailer"
            | "proxy-authorization"
            | "proxy-authenticate"
    )
}

/// 请求转发时由代理【统一接管】的端到端请求头：透传循环里必须跳过，由代理重设或单独处理
/// （Host / Origin / X-Forwarded-* / Authorization 后面统一重写；Cookie 在循环外摘掉
/// 门禁 cookie 后单独回种）。
///
/// 与 [`is_hop_by_hop`] 的分工：后者处理 HTTP 连接层的逐跳语义，本函数处理「代理改写」语义，
/// 两者正交、不可合并。HTTP 请求路径与 WebSocket 握手路径共用这一份清单（单一真源），
/// 唯一差异是 `content-length`：
/// * HTTP 路径传 `true` —— reqwest 按实际收集到的流式 body 重新计算 Content-Length，
///   透传原值会与之冲突/重复；
/// * WS 握手路径传 `false` —— 握手没有 body，保留浏览器原始头集合。
///
/// ⚠️⚠️ `origin` 必须在**透传循环里**跳过，不能靠「后面再设一次来覆盖」：
/// `http::request::Builder::header()`（reqwest 同名方法同理）是 **append** 而非 insert，
/// 事后再设只会得到逗号拼接的两个 Origin（`http://127.0.0.1:代理端口, …:上游端口`），
/// 做 Origin 校验的后端会直接拒连。2026-09-16 由 Electron 侧探针实测抓到，双壳同一份代码。
pub(super) fn is_managed_request_header(name: &str, include_content_length: bool) -> bool {
    name.eq_ignore_ascii_case("host")
        || name.eq_ignore_ascii_case("origin")
        || name.eq_ignore_ascii_case("authorization")
        || name.eq_ignore_ascii_case("cookie")
        || name.eq_ignore_ascii_case("x-forwarded-proto")
        || name.eq_ignore_ascii_case("x-forwarded-host")
        || name.eq_ignore_ascii_case("x-forwarded-for")
        || (include_content_length && name.eq_ignore_ascii_case("content-length"))
}

/// HTTP【响应路径】上应剥离的逐跳头：在 [`is_hop_by_hop`] 基础上再加 connection/upgrade。
///
/// 请求侧刻意保留这两个（WS 升级请求 `Connection: Upgrade` + `Upgrade: websocket` 必须
/// 原样转发）；但普通 HTTP 响应里它们同样是逐跳头——上游回的 `Connection: keep-alive/close`
/// 该由 hyper ↔ WebView 这段连接自己管理，原样转发会让两端连接语义打架。
/// ⚠️ 101 Switching Protocols 响应走的是 WebSocket 分支，不经过响应头复制循环，所以这里
/// 剥离 connection/upgrade 不影响 WS 握手。
pub(super) fn is_response_hop_header(name: &HeaderName, connection_options: &[String]) -> bool {
    let n = name.as_str();
    is_hop_by_hop(name)
        || matches!(n, "connection" | "upgrade")
        || connection_options.iter().any(|h| h == n)
}

/// 解析响应 `Connection` 头里点名的附加逐跳头（RFC 7230 §6.1：该字段列出的头也必须
/// 逐跳处理，如 `Connection: close, x-foo`）。全部小写化，空值/非 ASCII 安全过滤。
pub(super) fn connection_named_headers(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.is_ascii())
        .map(str::to_ascii_lowercase)
        .collect()
}

/// 重写 Set-Cookie 里的 `Domain` 属性：把上游真实主机改写为代理主机（127.0.0.1），
/// 防止 WebView 因当前域（localhost）与 Domain 不一致而丢弃 cookie（导致会话中断）。
/// 其它属性（Path / Expires / SameSite / Secure / HttpOnly 等）原样保留。
///
/// 例：`session=abc; Domain=realhost.example.com; Secure`
///  → `session=abc; Domain=127.0.0.1; Secure`
pub(super) fn rewrite_cookie_domain(cookie: &str, proxy_host: &str) -> String {
    let mut parts = cookie.split(';');
    let mut out: Vec<String> = Vec::new();
    if let Some(first) = parts.next() {
        out.push(first.trim().to_string());
    }
    for part in parts {
        let p = part.trim();
        // 仅重写 Domain 属性。⚠️ 不能用 `p[..7]` 直接切片：若属性含多字节 UTF-8
        // （上游返回的不规范 cookie 值），字节 7 可能落在字符中间 → 切片 panic。
        // 用 `get(..7)` 返回 Option 再做大小写无关比较，非 ASCII 属性安全放行。
        if p.get(..7)
            .is_some_and(|s| s.eq_ignore_ascii_case("domain="))
        {
            out.push(format!("Domain={}", proxy_host));
        } else {
            out.push(p.to_string());
        }
    }
    out.join("; ")
}
