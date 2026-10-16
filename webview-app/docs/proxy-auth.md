# 工作原理：本地反向代理与鉴权（webview-app）

> 自 `AGENTS.md` 拆出。代理链路 `src/proxy.rs` 是**双壳单一真源**（Electron 侧经 sidecar `w2a-proxy.exe`
> `#[path]` 复用），改动须双壳同步验证。Electron 侧的 sidecar 进程契约见 `../../electron-app/docs/probe-sidecar.md`。

现代 Chromium / WebView2 **已废弃**「URL 内嵌 userinfo 自动发送 Basic 鉴权」的行为，且 WebView2 的
`BasicAuthenticationRequested` 事件对 **WebSocket 握手不生效**（这是微软官方确认的限制）。

Web2App 的解法是内置一个**本地反向代理**（`src/proxy.rs`）：

1. 程序启动时监听 `127.0.0.1:随机端口`；
2. WebView 只访问这个本地代理地址（入口 URL 保留上游路径并自带一次性随机密钥 `?w2a_key=...`）；
3. 代理对**每一个 HTTP 请求和 WebSocket 握手**统一注入 `Authorization: Basic ...` 头，再转发到真实服务器；
4. 代理对 `Host` / `X-Forwarded-*` 头做透传，让上游收到的请求与「浏览器直连」时一致；
5. **入口门禁**：每个请求（含 WS 握手）必须在 query 或 `w2a_key` HttpOnly cookie 带上本次启动的
   256-bit 随机密钥，否则一律回 **404**（不透露代理存在，防同机进程扫描借道带凭据访问目标站）。
   首次带 query 访问时代理回种 cookie，页面里的根绝对路径资源与 WS 即靠 cookie 通过；
   转发上游前密钥会从 query 与 Cookie 中剥除，密钥不离开本机。

该代理是纯 Rust、零平台依赖，因此 **Windows 与 Android 共用同一份代码**。

> 当 `PROXY_ENABLED = false` 或「启用了外部 HTTP 代理」时，WebView 直连上游，由 WebView2 原生事件兜底鉴权。
> **凭据主机白名单（2026-09-27）**：直连模式下，Authorization 注入与 401 自动应答都只对
> **入口上游源**（scheme + host + 默认端口归一化，`bootstrap::is_upstream_request`）生效；
> 页面里的第三方子资源 / 外站 401 质询拿不到入口账号密码（与 Electron 壳 `login` 事件的
> host 校验是同一条规则）。URI 不可解析或源未知时失败关闭（不注入）。
