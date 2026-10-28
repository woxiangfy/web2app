//! proxy 单元测试（P4 大文件拆分：自 proxy.rs 末尾 `mod tests` 整体搬出，零行为变更）。
//!
//! 经根文件 `#[cfg(test)] mod tests;` 挂载（主 crate 与 `#[path]` 复用的 sidecar 同源）；
//! `use super::*` 沿用原 `mod tests` 内的引入。

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
            axum::routing::post(
                |body: axum::body::Bytes| async move { axum::body::Body::from(body) },
            ),
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
