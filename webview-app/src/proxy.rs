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
fn generate_access_token() -> String {
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
fn request_is_authorized(headers: &HeaderMap, uri: &Uri, token: &str) -> bool {
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
fn has_access_cookie(headers: &HeaderMap, token: &str) -> bool {
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
fn strip_secret_query(uri: &Uri) -> String {
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
fn strip_access_cookie(header_value: &str) -> Option<String> {
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

async fn handle_websocket(
    socket: WebSocket,
    state: ProxyState,
    headers: HeaderMap,
    forward_path: String,
) {
    let dbg_path = forward_path.clone();
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
    let path_and_query = forward_path;
    let upstream_url = format!("{}://{}:{}{}", ws_scheme, host, port, path_and_query);

    let mut builder = http::Request::builder().uri(&upstream_url).method("GET");

    // 透传浏览器原 handshake 头。WebSocket 透明代理的关键：浏览器发起握手时带的所有 WS 专用头
    // （Connection: Upgrade、Upgrade: websocket、Sec-WebSocket-Key/Version/Protocol/Extensions）
    // 必须原样转发给上游——tokio-tungstenite 的 connect_async 不会替我们补 Connection/Upgrade，
    // 缺了就会报 "Missing ... header connection"；Sec-WebSocket-Key 必须保留浏览器原始值，否则
    // 上游回的 Accept 与浏览器校验对不上（tungstenite 检测到 key 已存在时用 or_insert_with 保留）。
    // 跳过集合 = 逐跳头 + 代理受管头（host / origin / Authorization / Cookie / X-Forwarded-*），
    // 与 HTTP 请求路径共用 is_managed_request_header；WS 无 body 故 content-length 传 false。
    // ⚠️⚠️ Origin 同样必须在循环里跳过（append 语义 → 双 Origin），详见该函数注释。
    for (name, value) in &headers {
        let name_str = name.as_str();
        if is_hop_by_hop(name) || is_managed_request_header(name_str, false) {
            continue;
        }
        builder = builder.header(name.clone(), value.clone());
    }

    // WS 握手的 Cookie 同样摘掉门禁 cookie 再转发（密钥不离开本机）。
    if let Some(cookie) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) {
        if let Some(kept) = strip_access_cookie(cookie) {
            builder = builder.header("Cookie", kept);
        }
    }

    builder = builder.header("Host", state.upstream_authority.as_str());
    builder = builder.header("X-Forwarded-Proto", state.upstream.scheme());
    builder = builder.header("X-Forwarded-Host", host);
    builder = builder.header("X-Forwarded-For", "127.0.0.1");

    // 重写 WS 上游握手的 Origin 头：WebView 连的是本地代理（127.0.0.1:随机端口），浏览器自发
    // 的 Origin 自然是 `http://127.0.0.1:端口`。而 Trilium 等后端常对 WS 握手做 Origin 校验
    // （防跨站 hijack），对照自身宿主；Origin 与真实宿主不一致时可能拒连——后果就是"页面能开
    // 但实时联动/拖动改序(moveBranchTo 走 WS)失效"。这里把 Origin 改写为等价"浏览器直连上游"
    // 的源 `{scheme}://{host[:port]}`，规避该拒绝。
    // ⚠️ 浏览器原值已在上面透传循环里**丢弃**（origin 在跳过列表里），所以这里是唯一的 Origin ——
    // 不能指望「后设覆盖前设」，`Builder::header` 是 append。
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
            dlog!("[proxy] WS 上游握手失败 {dbg_path}: {e}");
            return;
        }
    };
    // 诊断日志：打印上游握手实际状态。正常应为 101 Switching Protocols；若返回 4xx/5xx，
    // 说明上游在握手阶段就拒绝了（鉴权 / Origin / 路由），可据此定位拖动改序失效。
    dlog!("[proxy] WS 握手 {dbg_path} -> {}", resp.status());

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
        // `b` 已是 `bytes::Bytes`，axum 的 Message::Binary/Ping/Pong 同样携带 `Bytes`，
        // 直接复用即可，`to_vec()` 只是多做一次堆拷贝（每个 WS 帧省一次分配）。
        T::Binary(b) => Some(A::Binary(b)),
        T::Ping(b) => Some(A::Ping(b)),
        T::Pong(b) => Some(A::Pong(b)),
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
fn is_managed_request_header(name: &str, include_content_length: bool) -> bool {
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
fn is_response_hop_header(name: &HeaderName, connection_options: &[String]) -> bool {
    let n = name.as_str();
    is_hop_by_hop(name)
        || matches!(n, "connection" | "upgrade")
        || connection_options.iter().any(|h| h == n)
}

/// 解析响应 `Connection` 头里点名的附加逐跳头（RFC 7230 §6.1：该字段列出的头也必须
/// 逐跳处理，如 `Connection: close, x-foo`）。全部小写化，空值/非 ASCII 安全过滤。
fn connection_named_headers(value: &str) -> Vec<String> {
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

    /// 回归：本地代理的入口 URL 必须保留上游自己的路径。
    ///
    /// 曾经的实现把入口写死成 `http://127.0.0.1:PORT/?w2a_key=…`，于是配置了子路径的
    /// `app_url`（如 `https://example.com/app/`）在**开启本地代理**时会被送到站点根，
    /// 而直连 / 外部代理模式（`strip_userinfo`）却保留完整路径 —— 同一份配置两种落点，
    /// 且没有任何报错。只校验生成的入口 URL，不发起网络请求。
    #[tokio::test]
    async fn proxy_entry_url_keeps_upstream_path() {
        let app_url = Url::parse("https://alice:secret@example.com/app/index.html").unwrap();
        let (entry, abort) = super::start_proxy(&app_url).await.unwrap();
        abort.abort();
        let u = Url::parse(&entry).unwrap();
        assert_eq!(u.path(), "/app/index.html", "入口应保留上游路径：{entry}");
        assert_eq!(
            u.host_str(),
            Some("127.0.0.1"),
            "入口应指向本地回环：{entry}"
        );
        assert!(
            u.query().unwrap_or("").contains(&format!("{ACCESS_KEY}=")),
            "入口应自带门禁密钥：{entry}"
        );
    }

    /// 零回归护栏：上游不带路径时（`path()` 恒为 `"/"`），入口 URL 与旧实现逐字节一致。
    #[tokio::test]
    async fn proxy_entry_url_origin_only_unchanged() {
        let app_url = Url::parse("http://example.com").unwrap();
        let (entry, abort) = super::start_proxy(&app_url).await.unwrap();
        abort.abort();
        let u = Url::parse(&entry).unwrap();
        assert_eq!(u.path(), "/", "无路径上游的入口应仍落在根：{entry}");
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
    fn managed_request_headers_shared_set_with_content_length_gate() {
        // 两条转发路径共用的受管头：大小写不敏感。
        for n in [
            "host",
            "origin",
            "authorization",
            "cookie",
            "x-forwarded-proto",
            "x-forwarded-host",
            "x-forwarded-for",
        ] {
            assert!(
                is_managed_request_header(&n.to_ascii_uppercase(), true),
                "{n} 在 HTTP 路径必须跳过"
            );
            assert!(
                is_managed_request_header(n, false),
                "{n} 在 WS 路径必须跳过"
            );
        }
        // 唯一差异：Content-Length 仅 HTTP 路径跳过（body 由 reqwest 重算）；
        // WS 握手无 body，必须保留浏览器的原始头集合。
        assert!(is_managed_request_header("Content-Length", true));
        assert!(!is_managed_request_header("Content-Length", false));
        // 普通端到端头两条路径都不拦（交给原语逐跳判定）。
        assert!(!is_managed_request_header("accept", true));
        assert!(!is_managed_request_header("sec-websocket-key", false));
    }

    #[test]
    fn response_path_strips_connection_and_upgrade() {
        // 请求侧放行的 connection/upgrade，响应侧必须剥离（101 不走此路径）。
        assert!(is_response_hop_header(
            &HeaderName::from_static("connection"),
            &[]
        ));
        assert!(is_response_hop_header(
            &HeaderName::from_static("upgrade"),
            &[]
        ));
        assert!(is_response_hop_header(
            &HeaderName::from_static("keep-alive"),
            &[]
        ));
        // Connection 点名的附加逐跳头也要剥离
        let opts = connection_named_headers("close, X-Foo");
        assert_eq!(opts, vec!["close".to_string(), "x-foo".to_string()]);
        assert!(is_response_hop_header(
            &HeaderName::from_static("x-foo"),
            &opts
        ));
        // 正常端到端头不剥
        assert!(!is_response_hop_header(
            &HeaderName::from_static("content-length"),
            &opts
        ));
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

        // no_proxy：开发机若配了 HTTP(S)_PROXY 环境变量，reqwest 默认会读，测试流量
        // 被劫持到外部代理就会偶发红（CI 与本机都踩过）；回环测试必须显式绕过。
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        // ① GET 大文件：验证鉴权注入 + cookie 重写 + 流式响应无截断
        let resp = client
            .get(authed_url(&proxy_url, "/"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        // 门禁 cookie 已随响应回种（浏览器靠它放行后续根绝对路径资源 / WS 握手）。
        let mut gate_cookie_seen = false;
        for c in resp.headers().get_all("set-cookie") {
            if let Ok(s) = c.to_str() {
                if s.contains(&format!("{ACCESS_KEY}=")) && s.contains("Path=/") {
                    gate_cookie_seen = true;
                }
            }
        }
        assert!(
            gate_cookie_seen,
            "响应应回种门禁 cookie：{:?}",
            resp.headers()
        );

        // 回归断言：Content-Length 必须被原样转发（此前流式化改造把它跳过了，
        // 导致响应退化为无长度 chunked，WebView2 内嵌 PDF 查看器因此渲染空白）。
        let forwarded_len = resp
            .headers()
            .get(axum::http::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<usize>().ok());
        assert_eq!(
            forwarded_len,
            Some(1024 * 1024),
            "Content-Length 应原样转发为 1MB"
        );

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
            .post(authed_url(&proxy_url, "/echo"))
            .body(payload.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(echo.status(), axum::http::StatusCode::OK);
        assert_eq!(echo.bytes().await.unwrap(), payload, "上传体应原样回传");

        abort.abort();
    }

    /// 回归：代理给上游发出的 Origin 必须**恰好一个**，且是「等价浏览器直连上游」的源。
    ///
    /// 曾经这里是两个（浏览器原值 + 重写值，逗号拼成一个头）—— 因为 `http::request::Builder::header`
    /// 与 `reqwest::RequestBuilder::header` 都是 `HeaderMap::append` 而非 insert，
    /// 「后面再设一次来覆盖」的写法只会追加。上游若做 Origin 校验（WS 防跨站劫持、
    /// CSRF 防护）会直接拒连，表现为「页面能打开，但实时联动 / 表单提交全挂」。
    /// 由 Electron 侧的兼容契约探针首次抓到（`WebSocket 的 Origin 已被改写` 那条断言）。
    #[tokio::test]
    async fn proxy_sends_exactly_one_rewritten_origin() {
        use std::sync::{Arc, Mutex};

        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_in = seen.clone();

        let app = axum::Router::new().route(
            "/",
            axum::routing::get(move |headers: axum::http::HeaderMap| {
                let seen_in = seen_in.clone();
                async move {
                    let mut got = seen_in.lock().unwrap();
                    for v in headers.get_all("origin") {
                        got.push(v.to_str().unwrap_or("<非 ASCII>").to_string());
                    }
                    "ok"
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let app_url = url::Url::parse(&format!("http://{}", addr)).unwrap();
        let (proxy_url, abort) = super::start_proxy(&app_url).await.unwrap();

        // 故意带一个「浏览器原值」Origin（真实场景里 WebView 会发 127.0.0.1:代理端口）
        // no_proxy：避免测试机上的 HTTP_PROXY 环境变量把本地回环请求拐走。
        let resp = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(authed_url(&proxy_url, "/"))
            .header("Origin", "http://127.0.0.1:9999")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        let got = seen.lock().unwrap().clone();
        assert_eq!(got.len(), 1, "上游应只收到一个 Origin，实际 {got:?}");
        assert_eq!(
            got[0],
            format!("http://{}", addr),
            "Origin 应被改写为上游自身源"
        );

        abort.abort();
    }

    // ---------- 入口门禁（见模块文档「入口门禁」）----------

    /// 用入口 URL（query 里带密钥）拼出同源下的具体路径，密钥 query 原样保留。
    fn authed_url(entry: &str, path: &str) -> String {
        let mut u = Url::parse(entry).unwrap();
        u.set_path(path);
        u.to_string()
    }

    /// 从入口 URL 取出密钥，组装成浏览器会发的 Cookie 头值（`w2a_key=<token>`）。
    fn auth_cookie_header(entry: &str) -> String {
        let u = Url::parse(entry).unwrap();
        let token = u
            .query_pairs()
            .find(|(k, _)| k == ACCESS_KEY)
            .map(|(_, v)| v.into_owned())
            .expect("入口 URL 应带 w2a_key");
        format!("{ACCESS_KEY}={token}")
    }

    #[test]
    fn access_token_is_random_and_url_safe() {
        let a = generate_access_token();
        let b = generate_access_token();
        // 256-bit → URL_SAFE_NO_PAD base64 = 43 字符
        assert_eq!(a.len(), 43);
        assert_eq!(b.len(), 43);
        assert_ne!(a, b, "两次生成的密钥必须不同（CSPRNG）");
        assert!(
            a.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "密钥必须只含 URL 安全字符，实际：{a}"
        );
    }

    #[test]
    fn gate_accepts_query_or_cookie_and_rejects_rest() {
        let uri: Uri = "/?x=1&w2a_key=good".parse().unwrap();
        let mut h = HeaderMap::new();
        assert!(request_is_authorized(&h, &uri, "good"));
        assert!(!request_is_authorized(&h, &uri, "bad"), "错误密钥应拒绝");
        assert!(
            !request_is_authorized(&h, &"/?x=1".parse::<Uri>().unwrap(), "good"),
            "无密钥应拒绝"
        );

        // Cookie 形态：带其它 cookie + 正确/错误门禁 cookie
        h.insert(header::COOKIE, "sid=abc; w2a_key=good".parse().unwrap());
        assert!(request_is_authorized(
            &h,
            &"/assets/x.js".parse::<Uri>().unwrap(),
            "good"
        ));
        assert!(!request_is_authorized(
            &h,
            &"/assets/x.js".parse::<Uri>().unwrap(),
            "bad"
        ));
        // has_access_cookie 只在值正确时为 true
        let mut good = HeaderMap::new();
        good.insert(header::COOKIE, "w2a_key=good".parse().unwrap());
        assert!(has_access_cookie(&good, "good"));
        let mut wrong = HeaderMap::new();
        wrong.insert(header::COOKIE, "w2a_key=wrong".parse().unwrap());
        assert!(!has_access_cookie(&wrong, "good"));
    }

    #[test]
    fn secret_query_stripped_without_touching_others() {
        let f = |s: &str| strip_secret_query(&s.parse::<Uri>().unwrap());
        assert_eq!(f("/?w2a_key=tok"), "/");
        assert_eq!(f("/a?b=1&w2a_key=tok&c=2"), "/a?b=1&c=2");
        assert_eq!(f("/a?w2a_key=tok&b=1"), "/a?b=1");
        assert_eq!(f("/a?b=1&w2a_key=tok"), "/a?b=1");
        assert_eq!(f("/no-query"), "/no-query");
        // 只删同名参数，相似名不动
        assert_eq!(f("/a?w2a_key_extra=1"), "/a?w2a_key_extra=1");
    }

    #[test]
    fn access_cookie_stripped_without_losing_other_cookies() {
        assert_eq!(
            strip_access_cookie("sid=9; w2a_key=tok; ui=dark").as_deref(),
            Some("sid=9; ui=dark")
        );
        assert_eq!(
            strip_access_cookie("W2A_KEY=tok").as_deref(),
            None,
            "大小写无关；摘空后整个 Cookie 头不转发"
        );
        assert_eq!(
            strip_access_cookie("sid=9; other=w2a_key=notme").as_deref(),
            Some("sid=9; other=w2a_key=notme"),
            "只有属性名完全相等才摘"
        );
    }

    /// 门禁端到端：无/错密钥一律 404；query 通过后回种 cookie、密钥不转发给上游；
    /// 仅带 cookie 的后续根绝对路径请求通过；query 里的密钥在上游侧被剥掉。
    #[tokio::test]
    async fn proxy_gate_404_query_and_cookie_flow() {
        use std::sync::{Arc, Mutex};

        // 记录上游最后收到的 Cookie 与路径。
        let last_cookie: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let last_cookie2 = last_cookie.clone();
        let last_uri: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let last_uri2 = last_uri.clone();

        // axum 0.8：catch-all `/{*path}` 不匹配根路径 "/"，两个路由共用同一个 handler。
        let inspect = move |headers: axum::http::HeaderMap, uri: axum::http::Uri| {
            *last_cookie2.lock().unwrap() = headers
                .get("cookie")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            *last_uri2.lock().unwrap() = Some(uri.to_string());
            async { "ok" }
        };
        let app = axum::Router::new()
            .route("/", axum::routing::get(inspect.clone()))
            .route("/{*path}", axum::routing::get(inspect));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let app_url = url::Url::parse(&format!("http://alice:secret@{}", addr)).unwrap();
        let (entry, abort) = super::start_proxy(&app_url).await.unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let root: Url = Url::parse(&entry).unwrap();
        let root_no_token = format!("{}://{}/", root.scheme(), root.authority());

        // ① 无密钥 / 错误密钥（query 与 cookie 各试一遍）→ 一律 404，不透露代理身份。
        assert_eq!(
            client.get(&root_no_token).send().await.unwrap().status(),
            axum::http::StatusCode::NOT_FOUND
        );
        assert_eq!(
            client
                .get(format!("{root_no_token}?{ACCESS_KEY}=wrong"))
                .send()
                .await
                .unwrap()
                .status(),
            axum::http::StatusCode::NOT_FOUND
        );
        assert_eq!(
            client
                .get(&root_no_token)
                .header("Cookie", format!("{ACCESS_KEY}=wrong; sid=z"))
                .send()
                .await
                .unwrap()
                .status(),
            axum::http::StatusCode::NOT_FOUND
        );
        // 被 404 挡下时上游根本没被碰到（路径仍为初始 None）。
        assert!(last_cookie.lock().unwrap().is_none());

        // ② 正确 query 密钥 → 200，响应回种门禁 cookie；上游收不到 w2a_key（Cookie 与 query 都剥掉）。
        let resp = client
            .get(authed_url(&entry, "/"))
            .header("Cookie", "sid=keep-me; w2a_key=good-but-replaced")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        assert!(
            resp.headers().get_all("set-cookie").iter().any(|v| v
                .to_str()
                .is_ok_and(|s| s.starts_with(&format!("{ACCESS_KEY}=")))),
            "应回种门禁 cookie"
        );
        assert_eq!(
            last_cookie.lock().unwrap().as_deref(),
            Some("sid=keep-me"),
            "门禁 cookie 不应转发给上游，其它 cookie 保留"
        );
        assert_eq!(
            last_uri.lock().unwrap().as_deref(),
            Some("/"),
            "密钥 query 不应转发给上游"
        );

        // ③ 不带 query、只带正确 cookie 的根绝对路径请求 → 通过；带业务 query 时密钥不混入。
        let resp = client
            .get(format!("{root_no_token}assets/app.js"))
            .header("Cookie", auth_cookie_header(&entry))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let resp = client
            .get(format!("{root_no_token}seen?a=1"))
            .header("Cookie", format!("{}; q=2", auth_cookie_header(&entry)))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        assert_eq!(
            last_uri.lock().unwrap().as_deref(),
            Some("/seen?a=1"),
            "业务 query 原样转发，且没有 w2a_key 残留"
        );
        assert_eq!(
            last_cookie.lock().unwrap().as_deref(),
            Some("q=2"),
            "门禁 cookie 不转发"
        );

        abort.abort();
    }

    /// 无密钥的 WebSocket 握手必须被 404 挡在代理层（不能借隧道直达上游）。
    #[tokio::test]
    async fn ws_without_token_rejected_with_404() {
        // 门禁在连接上游之前生效，所以上游地址故意给一个不可达地址：能拒绝就证明没连上游。
        let app_url = url::Url::parse("http://127.0.0.1:1/").unwrap();
        let (entry, abort) = super::start_proxy(&app_url).await.unwrap();
        let mut u = Url::parse(&entry).unwrap();
        u.set_scheme("ws").unwrap();
        u.set_path("/ws");
        u.set_query(None);

        let err = tokio_tungstenite::connect_async(u.as_str())
            .await
            .expect_err("无密钥的 WS 握手应失败");
        match err {
            tokio_tungstenite::tungstenite::Error::Http(resp) => {
                assert_eq!(
                    resp.status(),
                    axum::http::StatusCode::NOT_FOUND,
                    "WS 无密钥应回 404（隐藏代理身份）"
                );
            }
            other => panic!("期望 HTTP 404 响应，实际错误：{other:?}"),
        }
        abort.abort();
    }
}
