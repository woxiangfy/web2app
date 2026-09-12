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
    extract::{
        ws::{WebSocket, WebSocketUpgrade},
        FromRequestParts, Request, State,
    },
    http::{header, HeaderMap, HeaderName, StatusCode, Uri},
    response::Response,
    Router,
};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
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

    let app = Router::new().fallback(proxy_handler).with_state(state);

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
        return Ok(ws.on_upgrade(move |socket| handle_websocket(socket, state, uri, headers)));
    }

    // ---------- 普通 HTTP 转发（流式，避免大文件全量缓冲到内存）----------
    let (parts, body) = req.into_parts();

    let target = state
        .upstream
        .join(
            parts
                .uri
                .path_and_query()
                .map(|pq| pq.as_str())
                .unwrap_or("/"),
        )
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

    // 请求体流式化：不再 `collect()` 全量缓冲，边收边转给上游。
    // 用 `Body::wrap_stream`（只需流 Send + 'static，axum Body 满足），无长度信息时
    // 自动以 chunked 分块上传（HTTP/1.1 标准，主流服务器均支持）。内存峰值从
    // 「请求体大小」降到常数级。注意：reqwest 0.12 没有已知长度流的构造器
    // （`Body::sized` 是 0.13 才有的），故统一走 wrap_stream。
    let body_stream = body.into_data_stream();
    rb = rb.body(reqwest::Body::wrap_stream(body_stream));

    let proxied_resp = rb.send().await.map_err(|_| StatusCode::BAD_GATEWAY)?;

    // 诊断日志：只打印"关心"的响应——非 200 / 重定向 / PDF / 二进制文件。正常页面的大量
    // JS/CSS 静默跳过，噪音可控。用于排查 PDF 预览（能否拿到 200 + 正确 MIME/Content-Disposition）
    // 与拖动改序（moveBranchTo 走 WS，见 handle_websocket 日志）。
    let dbg_path = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/")
        .to_string();
    let dbg_ct = proxied_resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let dbg_cd = proxied_resp
        .headers()
        .get(header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let dbg_status = proxied_resp.status();
    let is_interesting = dbg_status != StatusCode::OK
        || dbg_status.is_redirection()
        || dbg_ct.to_ascii_lowercase().contains("pdf")
        || dbg_ct.to_ascii_lowercase().contains("octet-stream");
    if is_interesting {
        eprintln!(
            "[proxy] {method} {dbg_path} -> {dbg_status} [content-type: {dbg_ct}] [content-disposition: {dbg_cd}]"
        );
    }

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
                response_builder =
                    response_builder.header(name, rewrite_cookie_domain(v, PROXY_HOST));
                continue;
            }
        }
        response_builder = response_builder.header(name, value);
    }

    // 响应体流式化：不再 `bytes()` 全量缓冲，边收边转给 WebView。长度未知时 hyper
    // 自动用 chunked 分块传输（WebView2 原生支持）；Content-Length 已在上面跳过，
    // 避免流中断时客户端按旧长度挂起等待。
    Ok(response_builder
        .body(Body::from_stream(proxied_resp.bytes_stream()))
        .unwrap())
}

async fn handle_websocket(socket: WebSocket, state: ProxyState, uri: Uri, headers: HeaderMap) {
    let dbg_path = uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/")
        .to_string();
    let host = state.upstream.host_str().unwrap_or("localhost");
    let port = state.upstream.port_or_known_default().unwrap_or_else(|| {
        if state.upstream.scheme() == "https" {
            443
        } else {
            80
        }
    });
    let ws_scheme = if state.upstream.scheme() == "https" {
        "wss"
    } else {
        "ws"
    };
    let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
    let upstream_url = format!("{}://{}:{}{}", ws_scheme, host, port, path_and_query);

    let mut builder = http::Request::builder().uri(&upstream_url).method("GET");

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

    // 重写 WS 上游握手的 Origin 头：WebView 连的是本地代理（127.0.0.1:随机端口），浏览器自发
    // 的 Origin 自然是 `http://127.0.0.1:端口`。而 Trilium 等后端常对 WS 握手做 Origin 校验
    // （防跨站 hijack），对照自身宿主；Origin 与真实宿主不一致时可能拒连——后果就是"页面能开
    // 但实时联动/拖动改序(moveBranchTo 走 WS)失效"。这里把 Origin 改写为等价"浏览器直连上游"
    // 的源 `{scheme}://{host[:port]}`，规避该拒绝。覆盖时机在透传循环之后，以覆盖浏览器原值。
    builder = builder.header(
        "Origin",
        format!("{}://{}", state.upstream.scheme(), state.upstream_authority).as_str(),
    );

    // 代理对鉴权拥有最终决定权：强制写入 Authorization（覆盖任何已有值），确保 WebSocket 握手
    // 一定携带合法 Basic 头（这正是当初做本地代理的原因——WebView2 原生事件对 WS 不生效）。
    if let Some(auth) = &state.auth_header {
        builder = builder.header("Authorization", auth.as_str());
    }

    let request = match builder.body(()) {
        Ok(r) => r,
        Err(_) => return,
    };

    let (upstream_ws, resp) = match tokio_tungstenite::connect_async(request).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[proxy] WS 上游握手失败 {dbg_path}: {e}");
            return;
        }
    };
    // 诊断日志：打印上游握手实际状态。正常应为 101 Switching Protocols；若返回 4xx/5xx，
    // 说明上游在握手阶段就拒绝了（鉴权 / Origin / 路由），可据此定位拖动改序失效。
    eprintln!("[proxy] WS 握手 {dbg_path} -> {}", resp.status());

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

fn axum_msg_to_tungstenite(
    msg: axum::extract::ws::Message,
) -> Option<tokio_tungstenite::tungstenite::Message> {
    use axum::extract::ws::Message as A;
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
    use tokio_tungstenite::tungstenite::Message as T;
    match msg {
        A::Text(t) => Some(T::Text(t.as_str().into())),
        A::Binary(b) => Some(T::Binary(b)),
        A::Ping(b) => Some(T::Ping(b)),
        A::Pong(b) => Some(T::Pong(b)),
        A::Close(Some(cf)) => Some(T::Close(Some(
            tokio_tungstenite::tungstenite::protocol::frame::CloseFrame {
                code: CloseCode::Iana(cf.code),
                reason: cf.reason.as_str().into(),
            },
        ))),
        A::Close(None) => Some(T::Close(None)),
    }
}

fn tungstenite_msg_to_axum(
    msg: tokio_tungstenite::tungstenite::Message,
) -> Option<axum::extract::ws::Message> {
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
        assert!(is_hop_by_hop(&HeaderName::from_static(
            "proxy-authenticate"
        )));
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
        assert_eq!(
            rewrite_cookie_domain(c, PROXY_HOST),
            "sid=1; Path=/; HttpOnly"
        );
    }

    #[test]
    fn cookie_domain_with_leading_dot_rewritten() {
        let c = "token=x; Domain=.example.com";
        assert_eq!(
            rewrite_cookie_domain(c, PROXY_HOST),
            "token=x; Domain=127.0.0.1"
        );
    }

    #[test]
    fn cookie_multibyte_attr_no_panic() {
        // 回归：旧实现 `p[..7]` 字节切片在多字节 UTF-8 且长度 >= 7 的属性上会 panic
        //（字节 7 落在「文」字中间）。新实现必须安全放行并原样保留非 Domain 属性。
        let c = "sid=1; 中文文=value; Path=/";
        let r = rewrite_cookie_domain(c, PROXY_HOST);
        assert!(r.contains("sid=1"), "cookie 名值对应保留");
        assert!(r.contains("中文文=value"), "多字节属性应原样保留而非 panic");
        assert!(r.contains("Path=/"));
        assert!(!r.contains("Domain="), "不应误写 Domain");
    }

    #[test]
    fn axum_to_tungstenite_all_message_kinds() {
        use axum::extract::ws::{CloseFrame as AClose, Message as A, Utf8Bytes as AUtf8};
        use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
        use tokio_tungstenite::tungstenite::Message as T;

        let t = axum_msg_to_tungstenite(A::Text(AUtf8::from("hi"))).unwrap();
        assert_eq!(t, T::Text("hi".into()));

        let t = axum_msg_to_tungstenite(A::Binary(bytes::Bytes::from(vec![1, 2, 3]))).unwrap();
        assert!(matches!(t, T::Binary(b) if b.as_ref() == [1, 2, 3]));

        let t = axum_msg_to_tungstenite(A::Ping(bytes::Bytes::from(vec![9]))).unwrap();
        assert!(matches!(t, T::Ping(b) if b.as_ref() == [9]));

        let t = axum_msg_to_tungstenite(A::Pong(bytes::Bytes::from(vec![8]))).unwrap();
        assert!(matches!(t, T::Pong(b) if b.as_ref() == [8]));

        let t = axum_msg_to_tungstenite(A::Close(Some(AClose {
            code: 1000,
            reason: AUtf8::from("bye"),
        })))
        .unwrap();
        match t {
            T::Close(Some(cf)) => {
                assert_eq!(cf.code, CloseCode::Iana(1000));
                assert_eq!(cf.reason.as_str(), "bye");
            }
            other => panic!("期望 Close(Some)，实际 {other:?}"),
        }

        let t = axum_msg_to_tungstenite(A::Close(None)).unwrap();
        assert!(matches!(t, T::Close(None)));
    }

    #[test]
    fn tungstenite_to_axum_all_message_kinds() {
        use axum::extract::ws::Message as A;
        use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
        use tokio_tungstenite::tungstenite::protocol::frame::{
            coding::Data as OpData, coding::OpCode, CloseFrame as TClose, Frame,
        };
        use tokio_tungstenite::tungstenite::Message as T;

        let a = tungstenite_msg_to_axum(T::Text("yo".into())).unwrap();
        assert_eq!(a, A::Text("yo".into()));

        let a = tungstenite_msg_to_axum(T::Binary(bytes::Bytes::from(vec![7, 8]))).unwrap();
        assert!(matches!(a, A::Binary(b) if b.as_ref() == [7, 8]));

        let a = tungstenite_msg_to_axum(T::Ping(bytes::Bytes::from(vec![1]))).unwrap();
        assert!(matches!(a, A::Ping(b) if b.as_ref() == [1]));

        let a = tungstenite_msg_to_axum(T::Pong(bytes::Bytes::from(vec![2]))).unwrap();
        assert!(matches!(a, A::Pong(b) if b.as_ref() == [2]));

        let a = tungstenite_msg_to_axum(T::Close(Some(TClose {
            code: CloseCode::Normal,
            reason: "done".into(),
        })))
        .unwrap();
        match a {
            A::Close(Some(cf)) => {
                assert_eq!(cf.code, 1000);
                assert_eq!(cf.reason.as_str(), "done");
            }
            other => panic!("期望 Close(Some)，实际 {other:?}"),
        }

        let a = tungstenite_msg_to_axum(T::Close(None)).unwrap();
        assert!(matches!(a, A::Close(None)));

        // 原始数据帧（frame）在代理层不转发（上游重打包为完整消息）。
        let framed = tungstenite_msg_to_axum(T::Frame(Frame::message(
            bytes::Bytes::from("x"),
            OpCode::Data(OpData::Text),
            true,
        )));
        assert!(framed.is_none(), "Frame 不应被转换成 axum Message");
    }

    // ---- 端到端集成测试：真实本地反向代理 + mock 上游 ----
    // 验证三条核心语义：① 代理给每个请求强制注入 Authorization: Basic 头；
    // ② 上游 Set-Cookie 的 Domain=真实host 被重写为 127.0.0.1；
    // ③ 大响应体（1MB）经流式转发完整到达、chunked 上传体原样到达（无截断）。

    #[tokio::test]
    async fn proxy_e2e_auth_cookie_and_streaming() {
        use std::sync::{Arc, Mutex};

        // ---- mock 上游：记录收到的 Authorization 头，返回 cookie + 大 body ----
        let auth_seen: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let auth_seen2 = auth_seen.clone();
        let big: Vec<u8> = vec![0xAB; 1024 * 1024]; // 1MB

        let app = axum::Router::new()
            .route(
                "/",
                axum::routing::get(move |headers: axum::http::HeaderMap| {
                    let auth_seen2 = auth_seen2.clone();
                    let big = big.clone();
                    async move {
                        if let Some(v) = headers.get("authorization") {
                            if let Ok(s) = v.to_str() {
                                *auth_seen2.lock().unwrap() = Some(s.to_string());
                            }
                        }
                        let mut resp = axum::response::Response::new(axum::body::Body::from(big));
                        resp.headers_mut().insert(
                            "set-cookie",
                            axum::http::HeaderValue::from_str(
                                "session=abc; Domain=real.example.com; Path=/; HttpOnly",
                            )
                            .unwrap(),
                        );
                        resp
                    }
                }),
            )
            // echo 路由：原样返回请求体，验证代理的 chunked 上传流式转发。
            .route(
                "/echo",
                axum::routing::post(|body: axum::body::Bytes| async move {
                    axum::body::Body::from(body)
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        // ---- 启动本地反向代理，上游指向 mock 服务器 ----
        let app_url = url::Url::parse(&format!("http://alice:secret@{}", addr)).unwrap();
        let (proxy_url, abort) = super::start_proxy(&app_url).await.unwrap();

        let client = reqwest::Client::new();

        // ① GET 大文件：验证鉴权注入 + cookie 重写 + 流式响应无截断
        let resp = client.get(&proxy_url).send().await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        let mut cookie_seen = false;
        for c in resp.headers().get_all("set-cookie") {
            let s = c.to_str().unwrap();
            if s.contains("session=abc") {
                assert!(
                    s.contains("Domain=127.0.0.1"),
                    "cookie Domain 应被重写为代理主机，实际：{s}"
                );
                assert!(
                    s.contains("Path=/") && s.contains("HttpOnly"),
                    "其余属性应保留：{s}"
                );
                cookie_seen = true;
            }
        }
        assert!(cookie_seen, "应收到 session cookie");

        let bytes = resp.bytes().await.unwrap();
        assert_eq!(bytes.len(), 1024 * 1024, "1MB 响应应完整到达（流式无截断）");
        assert!(bytes.iter().all(|&b| b == 0xAB));

        let seen = auth_seen.lock().unwrap().clone();
        assert_eq!(
            seen.as_deref(),
            Some("Basic YWxpY2U6c2VjcmV0"),
            "上游应收到 alice:secret 的 Basic 头"
        );

        // ② POST chunked 上传：请求体经代理流式转发后原样到达
        let payload: Vec<u8> = (0..300_000).map(|i| (i % 251) as u8).collect();
        let echo = client
            .post(format!("{proxy_url}echo"))
            .body(payload.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(echo.status(), axum::http::StatusCode::OK);
        assert_eq!(echo.bytes().await.unwrap(), payload, "上传体应原样回传");

        abort.abort();
    }
}
