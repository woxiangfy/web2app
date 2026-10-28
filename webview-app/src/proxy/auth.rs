//! 入口门禁与凭据工具（P4 大文件拆分：自 proxy.rs 垂直拆出，零行为变更纯代码搬家）。
//!
//! 条目：Basic 头构造、userinfo 剥离、门禁令牌生成与校验、门禁 query / cookie 剥离。
//! 原 proxy.rs 内的私有条目收口为 `pub(super)`（有效可见范围不变）；对外路径由根重导出。

use super::*;

/// 从 URL 的 userinfo 生成 `Authorization: Basic ...` 头值；无账号则返回 None。
///
/// 纯函数、零平台依赖，便于单元测试。
pub fn build_basic_auth(upstream: &Url) -> Option<String> {
    if upstream.username().is_empty() {
        return None;
    }
    let creds = format!(
        "{}:{}",
        upstream.username(),
        upstream.password().unwrap_or("")
    );
    Some(format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(creds.as_bytes())
    ))
}

/// 去掉 URL 中的 userinfo（用户名:密码），返回“干净”地址（保留 scheme/host/port/路径/查询/片段）。
///
/// 用于直连 / 外部代理场景下 WebView 的导航地址，以及日志 / 握手 JSON 里的上游展示 ——
/// 鉴权由 WebView2 原生事件或本地代理注入，**绝不能**把密码写进浏览器地址栏、日志或
/// 交给外部代理字符串（泄漏一次等于把站点拱手让人）。
///
/// ⚠️ 放在本模块是因为它是 sidecar（`crates/proxy-server`）经 `#[path]` 复用的单一真源：
/// 主 crate（`bootstrap`）与 sidecar（`main.rs`）都复用这里，杜绝“同一份配置两种落点”
/// 的三处重复实现（历史：bootstrap.rs / proxy-server main.rs / 本文件 start_proxy 内联）。
pub fn strip_userinfo(u: &Url) -> String {
    let mut clean = u.clone();
    let _ = clean.set_username("");
    let _ = clean.set_password(None);
    clean.to_string()
}

/// 生成一次代理生命周期内的入口门禁密钥：256-bit 随机数 → URL 安全 base64（43 字符，无填充）。
///
/// 用 OS CSPRNG（`getrandom`）：密钥是「谁能借代理访问入口账号」的唯一凭据，
/// 可预测性（时间戳 / 自增）等于没有门禁。
pub(super) fn generate_access_token() -> String {
    use base64::Engine;
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("OS 随机源不可用，无法生成代理入口密钥");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// 生成 128-bit 一次性令牌（32 字符小写 hex），供“远程页面 → 壳”事件回传的防伪校验使用。
///
/// CSPRNG 取随机数失败属于环境级故障：返回 None，调用方据此**拒绝继续**（宁可开不出功能，
/// 也不接受「任何页面都能回传」的降级 —— 那等于没有令牌）。
///
/// ⚠️ 放本模块（而非 tabs / floatwin 各自一份）是因为两者**逐字节相同**，且本模块是 sidecar
/// 经 `#[path]` 复用的单一真源，天然适合承载“安全随机”类小工具（与 [`generate_access_token`] 并列）。
pub fn random_hex_token() -> Option<String> {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf).ok()?;
    Some(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// 判断请求是否通过入口门禁：query 或 Cookie 中任一带上正确密钥即可。
///
/// 纯函数（只看 headers/uri/token），HTTP 转发与 WS 握手共用，便于单测。
pub(super) fn request_is_authorized(headers: &HeaderMap, uri: &Uri, token: &str) -> bool {
    if let Some(query) = uri.query() {
        for pair in query.split('&') {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            if k == ACCESS_KEY && v == token {
                return true;
            }
        }
    }
    if let Some(cookie) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) {
        for pair in cookie.split(';').map(str::trim) {
            if let Some((_, v)) = pair
                .split_once('=')
                .filter(|(k, _)| k.eq_ignore_ascii_case(ACCESS_KEY))
            {
                if v == token {
                    return true;
                }
            }
        }
    }
    false
}

/// 判断 Cookie 头里是否已携带【正确的】门禁 cookie（决定本次响应要不要回种）。
pub(super) fn has_access_cookie(headers: &HeaderMap, token: &str) -> bool {
    headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|cookie| {
            cookie.split(';').map(str::trim).any(|pair| {
                pair.split_once('=')
                    .is_some_and(|(k, v)| k.eq_ignore_ascii_case(ACCESS_KEY) && v == token)
            })
        })
}

/// 剥掉 query 里的门禁令牌，返回要转发给上游的 path（含剩余 query）。
///
/// 必须按**原始字节**保留其余参数（不做 percent-decode/re-encode，避免改变语义）；
/// 密钥参数是唯一被删除的部分。
pub(super) fn strip_secret_query(uri: &Uri) -> String {
    let path = uri.path();
    match uri.query() {
        Some(query) => {
            let kept: Vec<&str> = query
                .split('&')
                .filter(|p| p.split('=').next().unwrap_or("") != ACCESS_KEY)
                .collect();
            if kept.is_empty() {
                path.to_string()
            } else {
                format!("{path}?{}", kept.join("&"))
            }
        }
        None => path.to_string(),
    }
}

/// 从转发给上游的 Cookie 头里摘掉门禁 cookie，避免把本机密钥泄漏给上游。
///
/// 返回 `None` 表示摘完为空（该 Cookie 头整个不转发）；其余 cookie 原样保留、顺序不变。
pub(super) fn strip_access_cookie(header_value: &str) -> Option<String> {
    let kept: Vec<String> = header_value
        .split(';')
        .map(str::trim)
        .filter(|p| {
            !p.split_once('=')
                .is_some_and(|(k, _)| k.eq_ignore_ascii_case(ACCESS_KEY))
        })
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    if kept.is_empty() {
        None
    } else {
        Some(kept.join("; "))
    }
}
