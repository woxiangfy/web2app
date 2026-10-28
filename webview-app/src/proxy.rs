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
//!
//! ## 入口门禁（防本机扫描 / 防 CSRF 借道）
//!
//! 代理监听 127.0.0.1 随机端口，但「随机端口」挡不住同机其它进程逐个扫端口：谁能连上这个端口，
//! 谁就能借本代理**携带着入口站点的账号凭据**访问上游。因此入口 URL 带一次性随机密钥
//! （`http://127.0.0.1:PORT/?w2a_key=<43 字符随机串>`，每次启动重新生成）：
//! - 每个请求（含 WS 握手）必须在 **query (`w2a_key`) 或同源 Cookie (`w2a_key`)** 里带上正确密钥，
//!   否则一律 **404** —— 不返回 401/403、不带任何代理特征，对扫描者表现为「这个端口上什么都没有」；
//! - 首次访问带 query 密钥时，响应回种 `HttpOnly; SameSite=Lax; Path=/` 的 cookie，
//!   之后页面里的根绝对路径资源（`/assets/...`）、相对链接、WS 握手都靠 cookie 自动通过
//!   （根绝对路径不会自带 query，这是必须上 cookie 的原因。形态对齐 Jupyter 的 token 鉴权）；
//! - 转发给上游前，`w2a_key` 的 query 与 cookie 都会被**剥掉**，密钥不离开本机；
//! - 诊断日志里的路径同样经过脱敏，密钥不落日志。
//!
//! 副作用：入口站点的首个文档必须有至少一次响应回种 cookie（401/3xx 响应也会带 Set-Cookie，
//! 浏览器照常收）。这对正常 Web 应用恒成立。

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

/// 入口门禁令牌的参数名（query 与 Cookie 同名；见模块文档「入口门禁」）。
const ACCESS_KEY: &str = "w2a_key";

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
    /// 入口门禁随机密钥（每次启动重新生成；query / Cookie 二选一携带，见模块文档）。
    access_token: String,
}

// ===========================================================================
// 子模块（P4 大文件拆分：按功能垂直拆出，零行为变更纯代码搬家，条目见各文件头）
// ===========================================================================

// ⚠️ 子模块必须带显式 `#[path]`：本文件是 sidecar（crates/proxy-server）经 `#[path]` 复用的，
// 而 `#[path]` 加载的模块里裸 `mod auth;` 会按【本文件所在目录】（src/）而非 src/proxy/ 找文件
// （sidecar 报 E0583）；`#[path]` 相对本文件目录解析，两 crate 同源同路径。
#[path = "proxy/auth.rs"]
mod auth;
#[path = "proxy/headers.rs"]
mod headers;
#[path = "proxy/ws.rs"]
mod ws;

use auth::*;
use headers::*;
use ws::*;

/// 主 crate 的 tabs / floatwin 经 [`crate::proxy::random_hex_token`] 取防伪令牌；
/// sidecar 不用它（同源文件，故定点放行 unused）。
#[allow(unused_imports)]
pub use auth::random_hex_token;
/// 对外引用路径不变（crate::proxy::*；sidecar 经 `#[path]` 复用走 proxy::*）：
/// 子模块中对外的条目在此重导出（`start_proxy` 本体留在根文件）。
pub use auth::{build_basic_auth, strip_userinfo};

/// 启动本地反向代理，返回 `(WebView 应加载的本地入口 URL, 代理任务 AbortHandle)`。
/// AbortHandle 供应用退出时优雅终止代理任务、释放 127.0.0.1 监听端口。
pub async fn start_proxy(
    app_url: &Url,
) -> Result<(String, tokio::task::AbortHandle), Box<dyn std::error::Error>> {
    let mut upstream = app_url.clone();

    // 从 APP_URL 的 userinfo 提取账号密码，构造 Basic 头。
    let auth_header = build_basic_auth(&upstream);

    // 清理上游 URL，避免 reqwest / tungstenite 又按 URL 里的 userinfo 自动发一份 Basic 头。
    // （就地变异 `&mut Url` 语义，与 [`strip_userinfo`] 的 `&Url -> String` 共享同一套清理规则。）
    let _ = upstream.set_username("");
    let _ = upstream.set_password(None);

    // 绑定随机端口（127.0.0.1:0），避免多实例冲突。
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let local_addr = listener.local_addr()?;
    // 入口 URL 自带一次性门禁密钥（见模块文档「入口门禁」）。
    let access_token = generate_access_token();
    // ⚠️ 入口 URL 必须带上上游自己的路径：本代理是「以 origin 为根」的反向代理，
    //    入口若只写 `/`，配置了子路径的 app_url（如 `https://example.com/app/`）会被送到
    //    站点根，与直连 / 外部代理模式（`strip_userinfo` 保留完整路径，见 bootstrap.rs 与
    //    proxy-server/main.rs）行为不一致 —— 表现为「一开代理就打开错页」，且完全静默。
    //    上游无路径时 `path()` 恒为 `"/"`，产物与旧实现逐字节相同（零回归）。
    let local_url = format!(
        "http://127.0.0.1:{}{}?{ACCESS_KEY}={access_token}",
        local_addr.port(),
        upstream.path()
    );

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
        access_token,
    };

    let app = Router::new().fallback(proxy_handler).with_state(state);

    // 用 tokio::task::spawn 跑 axum server：它返回真正的 tokio JoinHandle，
    // 其 abort_handle() 才能拿到 AbortHandle，便于应用退出时优雅终止该任务。
    // （tauri::async_runtime::spawn 返回的是 Tauri 自己的 JoinHandle 包装，没有 abort_handle()）
    let handle = tokio::task::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            dlog!("本地反向代理异常退出: {}", e);
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
        // WS 握手同样要过入口门禁（密钥可在 query 或 Cookie 里）：不先校验就 on_upgrade，
        // 等于给扫描者留了一条能直达上游（且携带凭据）的隧道。
        if !request_is_authorized(&parts.headers, &parts.uri, &state.access_token) {
            return Err(StatusCode::NOT_FOUND);
        }
        let headers = parts.headers.clone();
        let forward_path = strip_secret_query(&parts.uri);
        let ws = WebSocketUpgrade::from_request_parts(&mut parts, &())
            .await
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        return Ok(
            ws.on_upgrade(move |socket| handle_websocket(socket, state, headers, forward_path))
        );
    }

    // ---------- 普通 HTTP 转发（流式，避免大文件全量缓冲到内存）----------
    let (parts, body) = req.into_parts();

    // 入口门禁：query / Cookie 密钥不对一律 404（不透露代理的存在，见模块文档）。
    if !request_is_authorized(&parts.headers, &parts.uri, &state.access_token) {
        return Err(StatusCode::NOT_FOUND);
    }
    // 剥掉密钥后的转发路径（诊断日志也只打它，密钥不落盘）。
    let forward_path = strip_secret_query(&parts.uri);
    // 本次是否已带正确的门禁 cookie：带了就不用回种 Set-Cookie。
    let cookie_authed = has_access_cookie(&parts.headers, &state.access_token);

    let target = state
        .upstream
        .join(&forward_path)
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let method = parts.method.clone();
    let mut rb = state.client.request(method.clone(), target.as_str());

    // 透传请求头：逐跳头 + 代理受管头（Host / Origin / X-Forwarded-* / Authorization /
    // Cookie / Content-Length）一律跳过 —— 完整清单与各头的跳过理由（含 Content-Length
    // 由 reqwest 按流式 body 重算、Cookie 循环外单独回种）见 is_managed_request_header。
    // ⚠️⚠️ Origin 必须在此跳过而非事后覆盖（append 语义会产生逗号拼接的双 Origin），
    // 事故细节见 is_managed_request_header 的注释（2026-09-16 Electron 侧探针抓出，双壳同代码）。
    for (name, value) in &parts.headers {
        let name_str = name.as_str();
        if is_hop_by_hop(name) || is_managed_request_header(name_str, true) {
            continue;
        }
        rb = rb.header(name.clone(), value.clone());
    }

    // 转发给上游的 Cookie 要摘掉门禁 cookie（本机密钥不离开本机，见 strip_access_cookie）。
    if let Some(cookie) = parts
        .headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(kept) = strip_access_cookie(cookie) {
            rb = rb.header("Cookie", kept);
        }
    }

    // 上游收到的 Host 与“浏览器直连上游”时一致（例如直连时 Host=真实域名），
    // 避免服务器因 Host 不同返回不同内容 / 生成不同链接。
    rb = rb.header("Host", state.upstream_authority.as_str());
    // Origin 同理：WebView 连的是本地代理，浏览器自发送出的 Origin 是 `http://127.0.0.1:端口`；
    // 改写为「等价浏览器直连上游」的源，让做 Origin 校验的后端（CSRF 防护、WS 防劫持）也放行。
    rb = rb.header(
        "Origin",
        format!("{}://{}", state.upstream.scheme(), state.upstream_authority).as_str(),
    );
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
    //
    // 微优化：content-type 的小写拷贝只做一次（原来 pdf 与 octet-stream 各 lower 一次）；
    // path 与 content-disposition 延迟到「确定要打印」时才构造，热路径（海量 JS/CSS）上
    // 少几次字符串分配。
    let dbg_status = proxied_resp.status();
    let dbg_ct = proxied_resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let ct_lower = dbg_ct.to_ascii_lowercase();
    let is_interesting = dbg_status != StatusCode::OK
        || ct_lower.contains("pdf")
        || ct_lower.contains("octet-stream");
    if is_interesting {
        // 只用剥掉密钥后的路径，密钥不进诊断日志。
        let dbg_path = forward_path.clone();
        let dbg_cd = proxied_resp
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        dlog!(
            "[proxy] {method} {dbg_path} -> {dbg_status} [content-type: {dbg_ct}] [content-disposition: {dbg_cd}]"
        );
    }

    let mut response_builder = Response::builder().status(proxied_resp.status());
    // 回种门禁 cookie：靠 query 密钥通过的请求（每个新会话的首次入口访问必然在此列）
    // 都种一次，幂等无害；之后根绝对路径资源（`/assets/...`）与 WS 握手不带 query 也能通过。
    // Path=/ 覆盖整个代理源；http://127.0.0.1 不能写 Secure；HttpOnly 让页面脚本读不到密钥。
    if !cookie_authed {
        response_builder = response_builder.header(
            header::SET_COOKIE,
            format!(
                "{ACCESS_KEY}={}; Path=/; HttpOnly; SameSite=Lax",
                state.access_token
            ),
        );
    }
    // Connection 头里点名的附加逐跳头（响应路径专用，见 is_response_hop_header）。
    let connection_options: Vec<String> = proxied_resp
        .headers()
        .get(header::CONNECTION)
        .and_then(|v| v.to_str().ok())
        .map(connection_named_headers)
        .unwrap_or_default();

    for (name, value) in proxied_resp.headers() {
        // 跳过逐跳头（响应路径含 connection/upgrade 及其点名的头）。Content-Length 与
        // Content-Encoding 都必须【原样转发】：本代理的 reqwest
        // 未开启 gzip/brotli/deflate 解压特性（见 Cargo.toml），响应体会按上游原样流式透传——
        // 体的字节长度恒等于上游 Content-Length，转发该头是正确的。一旦跳过 Content-Length，响应
        // 会退化为 chunked 且无长度；多数资源可容错，但 WebView2 内嵌 PDF 查看器严重依赖
        // Content-Length，缺了它就渲染空白（这是"笔记内 PDF 预览失效"的回归根因）。
        if is_response_hop_header(name, &connection_options) {
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

    // 响应体流式化：不再 `bytes()` 全量缓冲，边收边转给 WebView。我们已把上游的
    // Content-Length 原样转发上来（本代理不解压，流式长度恒等于该值），因此 hyper 会按
    // Content-Length 直接发送，不额外分块；仅当上游本来就没有长度时（如上游自身 chunked）
    // 才由 hyper 自动分块。
    Ok(response_builder
        .body(Body::from_stream(proxied_resp.bytes_stream()))
        .unwrap())
}

#[cfg(test)]
#[path = "proxy/tests.rs"]
mod tests;
