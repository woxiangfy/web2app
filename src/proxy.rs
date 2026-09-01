//! 嵌入式本地反向代理（HTTP + WebSocket）。
//!
//! 设计目标：绕过 WebView2 的“WebResourceRequested / BasicAuthenticationRequested 不作用于
//! WebSocket”这一硬限制。WebView 不再直接访问真实服务器，而是访问本机随机端口上的代理；
//! 由代理给【每一个】请求（含 WebSocket 握手）统一注入 `Authorization: Basic ...` 头，再转发到
//! 真实服务器。
//!
//! 说明：
//! - 本地代理**透传上游真实 Host**（即 `https://user:pass@host` 里的 host[:port]），让上游服务器
//!   收到的 `Host` 与“浏览器直连上游”时完全一致，避免内容/链接因 Host 不同而差异。cookie / localStorage
//!   仍落在 localhost 域（因为 WebView 实际连的是 127.0.0.1:随机端口），与系统浏览器天然隔离。
//! - 透传 `X-Forwarded-*` 头，帮助后端识别原始协议与主机名，降低某些框架生成链接或判断
//!   `Secure` cookie 时出错的可能。
//! - 代理对鉴权拥有【最终决定权】：只要 `APP_URL` 带了 userinfo，就【强制写入】 `Authorization: Basic ...`
//!   头（覆盖请求里任何已有值），确保上游收到的一定是合法凭据；避免「WebView2 原生事件是否注入了头」
//!   的竞态导致偶发 401。转发请求时不会造成重复头（见下方 header 透传跳过 authorization）。

use axum::{
    body::Body,
    extract::{FromRequestParts, Request, State, ws::{WebSocket, WebSocketUpgrade}},
    http::{header, HeaderMap, HeaderName, StatusCode, Uri},
    response::Response,
    Router,
};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use tokio::net::TcpListener;
use url::Url;

/// WebView 实际连接的本地代理主机（用于 Cookie Domain 重写）。
const PROXY_HOST: &str = "127.0.0.1";

#[derive(Clone)]
pub struct ProxyState {
    client: reqwest::Client,
    /// 去掉 userinfo 的上游真实服务器地址，仅保留 scheme/host/port。
    upstream: Url,
    /// 预先生成的 `Authorization: Basic ...` 头值。
    auth_header: Option<String>,
    /// 上游 authority（host[:port]），作为转发请求的 `Host` 头值，
    /// 让上游收到的 Host 与“浏览器直连上游”时完全一致，避免内容/链接因 Host 不同而差异。
    upstream_authority: String,
}

/// 从 URL 的 userinfo 生成 `Authorization: Basic ...` 头值；无账号则返回 None。
///
/// 纯函数、零平台依赖，便于单元测试。
pub fn build_basic_auth(upstream: &Url) -> Option<String> {
    if upstream.username().is_empty() {
        return None;
    }
    let creds = format!("{}:{}", upstream.username(), upstream.password().unwrap_or(""));
    Some(format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(creds.as_bytes())
    ))
}

/// 启动本地反向代理，返回 `(WebView 应加载的本地入口 URL, 代理任务 AbortHandle)`。
/// AbortHandle 供应用退出时优雅终止代理任务、释放 127.0.0.1 监听端口。
pub async fn start_proxy(
    app_url: &Url,
) -> Result<(String, tokio::task::AbortHandle), Box<dyn std::error::Error>> {
    let mut upstream = app_url.clone();

    // 从 APP_URL 的 userinfo 提取账号密码，构造 Basic 头。
    let auth_header = build_basic_auth(&upstream);

    // 清理上游 URL，避免 reqwest / tungstenite 又按 URL 里的 userinfo 自动发一份 Basic 头。
    let _ = upstream.set_username("");
    let _ = upstream.set_password(None);

    // 绑定随机端口（127.0.0.1:0），避免多实例冲突。
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let local_addr = listener.local_addr()?;
    let local_url = format!("http://127.0.0.1:{}/", local_addr.port());

    let client = reqwest::Client::builder()
        // 不做全局超时：上游可能有长连接 / 流式响应（SSE、长轮询、大文件下载），超时会被误杀。
        // 不重定向：把 302/301 原样返回给 WebView，让它继续走本地代理。
        .redirect(reqwest::redirect::Policy::none())
        // 不使用系统环境变量里的代理，避免与 WebView2 自身的 --proxy-server 重复/冲突。
        .no_proxy()
        .build()?;

    let upstream_authority = upstream.authority().to_string();
    let state = ProxyState {
        client,
        upstream: upstream.clone(),
        auth_header: auth_header.clone(),
        upstream_authority,
    };

    let app = Router::new()
        .fallback(proxy_handler)
        .with_state(state);

    // 用 tokio::task::spawn 跑 axum server：它返回真正的 tokio JoinHandle，
    // 其 abort_handle() 才能拿到 AbortHandle，便于应用退出时优雅终止该任务。
    // （tauri::async_runtime::spawn 返回的是 Tauri 自己的 JoinHandle 包装，没有 abort_handle()）
    let handle = tokio::task::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("本地反向代理异常退出: {}", e);
        }
    });

    Ok((local_url, handle.abort_handle()))
}

async fn proxy_handler(
    State(state): State<ProxyState>,
    req: Request,
) -> Result<Response, StatusCode> {
    // 手动判断是否为 WebSocket 升级请求。
    // 注意：axum 0.8 不允许 `Option<WebSocketUpgrade>` 与 `Request` 共存于同一 handler，
    // 因此这里改用 `Request` 单一提取器，由内部识别升级并手动构造 WebSocketUpgrade。
    let is_ws = req
        .headers()
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
        && req
            .headers()
            .get(header::CONNECTION)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.to_ascii_lowercase().contains("upgrade"))
            .unwrap_or(false);

    if is_ws {
        let (mut parts, _body) = req.into_parts();
        let uri = parts.uri.clone();
        let headers = parts.headers.clone();
        let ws = WebSocketUpgrade::from_request_parts(&mut parts, &())
            .await
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        return Ok(ws.on_upgrade(move |socket| {
            handle_websocket(socket, state, uri, headers)
        }));
    }

    // ---------- 普通 HTTP 转发 ----------
    let (parts, body) = req.into_parts();
    let body_bytes = body
        .collect()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .to_bytes();

    let target = state
        .upstream
        .join(parts.uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/"))
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let method = parts.method.clone();
    let mut rb = state.client.request(method.clone(), target.as_str());

    // 透传请求头（去掉 hop-by-hop，并跳过我们会重新设置的 Host / X-Forwarded-* / Authorization，
    // 以及 Content-Length——由 reqwest 按实际收集到的 body 重新计算，避免与原始值冲突/重复）。
    for (name, value) in &parts.headers {
        let name_str = name.as_str();
        if is_hop_by_hop(name)
            || name_str.eq_ignore_ascii_case("host")
            || name_str.eq_ignore_ascii_case("authorization")
            || name_str.eq_ignore_ascii_case("x-forwarded-proto")
            || name_str.eq_ignore_ascii_case("x-forwarded-host")
            || name_str.eq_ignore_ascii_case("x-forwarded-for")
            || name_str.eq_ignore_ascii_case("content-length")
        {
            continue;
        }
        rb = rb.header(name.clone(), value.clone());
    }

    // 上游收到的 Host 与“浏览器直连上游”时一致（例如直连时 Host=真实域名），
    // 避免服务器因 Host 不同返回不同内容 / 生成不同链接。
    rb = rb.header("Host", state.upstream_authority.as_str());
    rb = rb.header("X-Forwarded-Proto", state.upstream.scheme());
    rb = rb.header(
        "X-Forwarded-Host",
        state.upstream.host_str().unwrap_or("localhost"),
    );
    rb = rb.header("X-Forwarded-For", "127.0.0.1");

    // 代理对鉴权拥有【最终决定权】：只要配置了凭据，就强制写入 Authorization（覆盖任何已有值）。
    // 这样可彻底消除“上游 401”的竞态——无论 WebView2 原生事件是否也注入了头，上游收到的一定是
    // 合法、正确的 Basic 头。（上面的 header 透传循环已跳过 authorization，这里不会造成重复头。）
    if let Some(auth) = &state.auth_header {
        rb = rb.header("Authorization", auth.as_str());
    }

    let proxied_resp = rb
        .body(body_bytes)
        .send()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;

    let mut response_builder = Response::builder().status(proxied_resp.status());
    for (name, value) in proxied_resp.headers() {
        // 跳过逐跳头，以及 Content-Length：reqwest 默认自动解压，解压后 body 已变长，而上游带来的
        // Content-Length 仍是压缩时的大小；交给 axum/hyper 按实际 bytes 重新计算，避免响应被错误截断。
        if is_hop_by_hop(name) || name.as_str().eq_ignore_ascii_case("content-length") {
            continue;
        }
        // 上游若给 Set-Cookie 写了 `Domain=<真实host>`，而 WebView 当前连的是 127.0.0.1，浏览器会因
        // 域不匹配而丢弃该 cookie → 登录会话断。这里把 Domain 统一重写为代理主机（127.0.0.1），
        // 让 cookie 落在 localhost 域、与代理会话一致，从而保留登录态（其它属性原样保留）。
        if name.as_str().eq_ignore_ascii_case("set-cookie") {
            if let Ok(v) = value.to_str() {
                response_builder = response_builder.header(name, rewrite_cookie_domain(v, PROXY_HOST));
                continue;
            }
        }
        response_builder = response_builder.header(name, value);
    }

    let bytes = proxied_resp
        .bytes()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    Ok(response_builder.body(Body::from(bytes)).unwrap())
}

async fn handle_websocket(
    socket: WebSocket,
    state: ProxyState,
    uri: Uri,
    headers: HeaderMap,
) {
    let host = state.upstream.host_str().unwrap_or("localhost");
    let port = state
        .upstream
        .port_or_known_default()
        .unwrap_or_else(|| if state.upstream.scheme() == "https" { 443 } else { 80 });
    let ws_scheme = if state.upstream.scheme() == "https" { "wss" } else { "ws" };
    let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
    let upstream_url = format!("{}://{}:{}{}", ws_scheme, host, port, path_and_query);

    let mut builder = http::Request::builder()
        .uri(&upstream_url)
        .method("GET");

    // 透传浏览器原 handshake 头。WebSocket 透明代理的关键：浏览器发起握手时带的所有 WS 专用头
    // （Connection: Upgrade、Upgrade: websocket、Sec-WebSocket-Key/Version/Protocol/Extensions）
    // 必须原样转发给上游——tokio-tungstenite 的 connect_async 不会替我们补 Connection/Upgrade，
    // 缺了就会报 "Missing ... header connection"；Sec-WebSocket-Key 必须保留浏览器原始值，否则
    // 上游回的 Accept 与浏览器校验对不上（tungstenite 检测到 key 已存在时用 or_insert_with 保留）。
    // 仅去掉我们自行重设的 host/Authorization/X-Forwarded-*，以及真正的逐跳头（keep-alive 等）。
    for (name, value) in &headers {
        let name_str = name.as_str();
        if is_hop_by_hop(name)
            || name_str.eq_ignore_ascii_case("host")
            || name_str.eq_ignore_ascii_case("authorization")
            || name_str.eq_ignore_ascii_case("x-forwarded-proto")
            || name_str.eq_ignore_ascii_case("x-forwarded-host")
            || name_str.eq_ignore_ascii_case("x-forwarded-for")
        {
            continue;
        }
        builder = builder.header(name.clone(), value.clone());
    }

    builder = builder.header("Host", state.upstream_authority.as_str());
    builder = builder.header("X-Forwarded-Proto", state.upstream.scheme());
    builder = builder.header("X-Forwarded-Host", host);
    builder = builder.header("X-Forwarded-For", "127.0.0.1");

    // 代理对鉴权拥有最终决定权：强制写入 Authorization（覆盖任何已有值），确保 WebSocket 握手
    // 一定携带合法 Basic 头（这正是当初做本地代理的原因——WebView2 原生事件对 WS 不生效）。
    if let Some(auth) = &state.auth_header {
        builder = builder.header("Authorization", auth.as_str());
    }

    let request = match builder.body(()) {
        Ok(r) => r,
        Err(_) => return,
    };

    let (upstream_ws, _resp) = match tokio_tungstenite::connect_async(request).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("代理上游 WebSocket 连接失败: {}", e);
            return;
        }
    };

    let (mut client_write, mut client_read) = socket.split();
    let (mut upstream_write, mut upstream_read) = upstream_ws.split();

    let client_to_upstream = async {
        while let Some(Ok(msg)) = client_read.next().await {
            if let Some(tmsg) = axum_msg_to_tungstenite(msg) {
                if upstream_write.send(tmsg).await.is_err() {
                    break;
                }
            }
        }
    };

    let upstream_to_client = async {
        while let Some(Ok(msg)) = upstream_read.next().await {
            if let Some(amsg) = tungstenite_msg_to_axum(msg) {
                if client_write.send(amsg).await.is_err() {
                    break;
                }
            }
        }
    };

    tokio::select! {
        _ = client_to_upstream => {},
        _ = upstream_to_client => {},
    }
}

fn axum_msg_to_tungstenite(msg: axum::extract::ws::Message) -> Option<tokio_tungstenite::tungstenite::Message> {
    use axum::extract::ws::Message as A;
    use tokio_tungstenite::tungstenite::Message as T;
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
    match msg {
        A::Text(t) => Some(T::Text(t.as_str().into())),
        A::Binary(b) => Some(T::Binary(b)),
        A::Ping(b) => Some(T::Ping(b)),
        A::Pong(b) => Some(T::Pong(b)),
        A::Close(Some(cf)) => Some(T::Close(Some(tokio_tungstenite::tungstenite::protocol::frame::CloseFrame {
            code: CloseCode::Iana(cf.code),
            reason: cf.reason.as_str().into(),
        }))),
        A::Close(None) => Some(T::Close(None)),
    }
}

fn tungstenite_msg_to_axum(msg: tokio_tungstenite::tungstenite::Message) -> Option<axum::extract::ws::Message> {
    use axum::extract::ws::Message as A;
    use tokio_tungstenite::tungstenite::Message as T;
    match msg {
        T::Text(t) => Some(A::Text(t.as_str().into())),
        T::Binary(b) => Some(A::Binary(bytes::Bytes::from(b.to_vec()))),
        T::Ping(b) => Some(A::Ping(bytes::Bytes::from(b.to_vec()))),
        T::Pong(b) => Some(A::Pong(bytes::Bytes::from(b.to_vec()))),
        T::Close(Some(cf)) => Some(A::Close(Some(axum::extract::ws::CloseFrame {
            code: cf.code.into(),
            reason: cf.reason.as_str().into(),
        }))),
        T::Close(None) => Some(A::Close(None)),
        T::Frame(_) => None,
    }
}

/// 真正逐跳的 hop-by-hop 头（不含 connection/upgrade，因为 WebSocket 握手必须原样透传它们）。
/// 这些头若被转发会让上游的连接管理混乱，且在握手请求里本就不存在（无 body）。
fn is_hop_by_hop(name: &HeaderName) -> bool {
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

/// 重写 Set-Cookie 里的 `Domain` 属性：把上游真实主机改写为代理主机（127.0.0.1），
/// 防止 WebView 因当前域（localhost）与 Domain 不一致而丢弃 cookie（导致会话中断）。
/// 其它属性（Path / Expires / SameSite / Secure / HttpOnly 等）原样保留。
///
/// 例：`session=abc; Domain=realhost.example.com; Secure`
///  → `session=abc; Domain=127.0.0.1; Secure`
fn rewrite_cookie_domain(cookie: &str, proxy_host: &str) -> String {
    let mut parts = cookie.split(';');
    let mut out: Vec<String> = Vec::new();
    if let Some(first) = parts.next() {
        out.push(first.trim().to_string());
    }
    for part in parts {
        let p = part.trim();
        // 仅重写 Domain 属性；其值为 ASCII，可安全用字节切片前缀判断。
        if p.len() >= 7 && p[..7].eq_ignore_ascii_case("domain=") {
            out.push(format!("Domain={}", proxy_host));
        } else {
            out.push(p.to_string());
        }
    }
    out.join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderName;
    use url::Url;

    #[test]
    fn basic_auth_generated_for_userinfo() {
        let u = Url::parse("https://alice:secret@example.com").unwrap();
        let h = build_basic_auth(&u).expect("应有 Basic 头");
        // "alice:secret" 的 Base64 编码为 YWxpY2U6c2VjcmV0
        assert_eq!(h, "Basic YWxpY2U6c2VjcmV0");
    }

    #[test]
    fn basic_auth_none_without_user() {
        let u = Url::parse("https://example.com").unwrap();
        assert!(build_basic_auth(&u).is_none());
    }

    #[test]
    fn hop_by_hop_headers_recognized() {
        assert!(is_hop_by_hop(&HeaderName::from_static("keep-alive")));
        assert!(is_hop_by_hop(&HeaderName::from_static("transfer-encoding")));
        assert!(is_hop_by_hop(&HeaderName::from_static("proxy-authenticate")));
        // 这些头必须原样透传（尤其 WebSocket 握手），不应被当作逐跳头丢弃
        assert!(!is_hop_by_hop(&HeaderName::from_static("connection")));
        assert!(!is_hop_by_hop(&HeaderName::from_static("upgrade")));
        assert!(!is_hop_by_hop(&HeaderName::from_static("authorization")));
    }

    #[test]
    fn cookie_domain_rewritten_to_proxy_host() {
        let c = "session=abc; Path=/; Domain=realhost.example.com; Secure; HttpOnly";
        let r = rewrite_cookie_domain(c, PROXY_HOST);
        assert!(r.contains("session=abc"));
        assert!(r.contains("Domain=127.0.0.1"));
        assert!(!r.contains("Domain=realhost"));
        assert!(r.contains("Path=/"));
        assert!(r.contains("Secure"));
        assert!(r.contains("HttpOnly"));
    }

    #[test]
    fn cookie_without_domain_untouched() {
        let c = "sid=1; Path=/; HttpOnly";
        assert_eq!(rewrite_cookie_domain(c, PROXY_HOST), "sid=1; Path=/; HttpOnly");
    }

    #[test]
    fn cookie_domain_with_leading_dot_rewritten() {
        let c = "token=x; Domain=.example.com";
        assert_eq!(rewrite_cookie_domain(c, PROXY_HOST), "token=x; Domain=127.0.0.1");
    }
}
