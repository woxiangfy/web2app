//! WebSocket 转发（P4 大文件拆分：自 proxy.rs 垂直拆出，零行为变更纯代码搬家）。
//!
//! 条目：WS 会话双向转发、axum / tungstenite 消息互转；入口门禁校验复用 auth 子模块。
//! 原 proxy.rs 内的私有条目收口为 `pub(super)`（有效可见范围不变）。

use super::*;

pub(super) async fn handle_websocket(
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

pub(super) fn axum_msg_to_tungstenite(
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

pub(super) fn tungstenite_msg_to_axum(
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
