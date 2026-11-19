//! Android（非 Windows）导航后的可达性探测。
//!
//! 为什么需要：Android WebView 加载失败（DNS 失败 / 连接被拒 / 超时 / TLS 握手失败）时
//! 既不触发应用可订阅的错误回调，也不像桌面端 Chromium 那样画出带错误码的页面 ——
//! 文档停在一个【空白错误文档】上，用户只能看到加载层转十几秒后淡出成纯白。
//! 导航提交后并发一次 Rust 侧轻量 HTTP 探测（reqwest + rustls 是非 Windows 既有的
//! 代理依赖，零新增 crate），在数秒内判定失败类别，把可看懂的原因推给加载层失败卡。
//!
//! 设计要点：
//!   · 探测与导航【并发】，不阻塞、不延迟真实页面加载；
//!   · 收到任意 HTTP 响应（含 4xx/5xx）即视为传输层可达 —— 白屏与网络无关，不插手；
//!   · 推给加载层时带 soft 标记（loading.js `fail(t,d,soft)`）：页面若已渲染出正文，
//!     探测结论让位（防止企业根证书等场景下 rustls 与系统信任库判定不一致的假阴性）。
//!
//! # 模块边界（架构审查 2026-11-19）
//!
//! 本段原先以 banner 注释的形式住在 `bootstrap.rs` 尾部（约 270 行），与「启动编排」是两件
//! 不同的事：编排决定「何时导航」，这里只回答「导航之后为什么还是白屏」。抽出后
//! `bootstrap` 回到单一职责（准备 → 导航 → 失败上报），本模块可独立演进（改文案 / 加探测
//! 维度）而不触碰启动时序。
//!
//! # cfg 契约
//!
//!   · 模块本体按 `#[cfg(any(not(windows), test))]` 挂载（见 `lib.rs`）：Windows **发布**
//!     二进制不携带这组仅 Android 使用的诊断文案；Windows 宿主 `cargo test` 仍编译纯逻辑
//!     部分（目标选择 / 三条文案映射），保证 Android 逻辑在桌面 CI 上也有护栏。
//!   · 因此纯逻辑**不得**引用 `not(windows)` 专属项：reqwest 与 URL 解析只出现在
//!     `#[cfg(not(windows))]` 的探测本体里。

use crate::nav::Navigation;
#[cfg(not(windows))]
use tauri::Manager;
#[cfg(not(windows))]
use url::Url;

/// 探测使用的移动端浏览器 UA：与 Android WebView 的 UA 形态保持一致，
/// 避免个别按 UA 分流 / 挑战的网关（WAF、反爬）对探测与真实页面给出不同响应。
#[cfg(not(windows))]
const PROBE_USER_AGENT: &str = "Mozilla/5.0 (Linux; Android 15) AppleWebKit/537.36 \
(KHTML, like Gecko) Version/4.0 Chrome/126.0.0.0 Mobile Safari/537.36";

/// 连接超时：给移动网络（弱网 / 丢包）留余量，但要早于加载层 30s 失败超时，
/// 保证探测结论能先于通用超时文案到达。
#[cfg(not(windows))]
const PROBE_CONNECT_TIMEOUT_SECS: u64 = 8;

/// 整个请求（拿到响应头）的总超时。
#[cfg(not(windows))]
const PROBE_TOTAL_TIMEOUT_SECS: u64 = 20;

/// 可达性探测的失败类别。
///
/// 刻意与 reqwest 的具体错误类型解耦：`probe_failure_message` 是纯函数，可在
/// **Windows 宿主 `cargo test`** 上单测（Android 逻辑在桌面 CI 一并验证）；只有错误归类与
/// HTTP 探测本体依赖 reqwest，gate 在 `#[cfg(not(windows))]`。
///
/// ⚠️ cfg 取 `any(not(windows), test)` 而不是无条件编译：生产调用方全部是 `not(windows)`
/// 路径，无条件编译会让这三项在 Windows 发布构建里成为死代码（crate 根的
/// `#![allow(dead_code)]` 会把它静默兜住 —— 新增代码不允许再依赖那条豁免毯）。
/// 取并集后：Android 生产构建在编、Windows 宿主测试也在编，唯独 Windows 发布二进制不携带
/// 这组仅 Android 使用的诊断文案。
#[cfg(any(not(windows), test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeFailure {
    /// TCP 连接建立失败（含 DNS 解析失败、端口不可达）。
    Connect,
    /// 连接 / 等待响应超时（弱网或站点不可达）。
    Timeout,
    /// TLS 握手 / 证书校验失败。
    Tls,
    /// 请求构造本身不合法（正常不会出现：URL 已在 prepare_navigation 校验过）。
    BadRequest,
    /// 其它未细分错误。
    Other,
}

/// 把失败类别 + 主机名映射为可展示的「标题 + 详情」。
///
/// `host` 只放主机名，绝不放完整 URL（直连地址已剥离 userinfo，代理地址可能带入口令牌，
/// 都不应出现在面向用户的文案里）。
#[cfg(any(not(windows), test))]
fn probe_failure_message(kind: ProbeFailure, host: &str) -> (String, String) {
    match kind {
        ProbeFailure::Connect => (
            "无法连接服务器".to_string(),
            format!("无法与 {host} 建立连接（域名解析失败或网络不可达），请检查网络，或切换 Wi-Fi / 移动网络后重试。"),
        ),
        ProbeFailure::Timeout => (
            "连接超时".to_string(),
            format!("连接 {host} 超时，当前网络可能无法访问该站点，请切换网络后重试。"),
        ),
        ProbeFailure::Tls => (
            "安全连接失败".to_string(),
            format!("与 {host} 的加密连接未能建立（证书或 TLS 握手失败），请检查网络环境后重试。"),
        ),
        ProbeFailure::BadRequest => (
            "地址有误".to_string(),
            format!("无法向 {host} 发起请求，应用配置的地址可能不正确，请联系打包者检查。"),
        ),
        ProbeFailure::Other => (
            "页面无法打开".to_string(),
            format!("访问 {host} 失败，请检查网络连接后重试。"),
        ),
    }
}

/// 可达性探测**成功**时给卡片的一行结论（纯函数，跨平台可单测）。
///
/// 为什么成功也要报：先前只把**失败**推给卡片、成功时静默 —— 于是最需要的那条判据恰好丢了。
/// 真机取证（2026-10-09）中「壳内页面不渲染、而 Rust 侧 reqwest 能拿到响应」正是这种组合：
/// 两条信息合在一起才能把「网络不通」与「内核不渲染」分开；只报失败时，用户看到的是一张
/// 没有任何结论的兜底卡，完全无从下手。
///
/// `host` 只放主机名（理由同 [`probe_failure_message`]）；`status` 为 `None` 表示内核没给出
/// 状态码；`via_proxy` = 探测走的是「本地代理 → 上游」，文案必须点明，否则用户会以为
/// 结论说的是 `127.0.0.1`（清单 #15）。
#[cfg(any(not(windows), test))]
fn probe_ok_message(host: &str, status: Option<u16>, ms: u128, via_proxy: bool) -> String {
    let st = match status {
        Some(code) => format!("HTTP {code}"),
        None => "已收到响应".to_string(),
    };
    if via_proxy {
        return format!(
            "可达性探测：经本地代理访问上游 {host} 已连通（{st}，{ms}ms），但页面未渲染 —— \
             问题多半在本机 WebView 内核（版本过旧 / TLS 协商 / 渲染），不在网络。\
             可先升级系统「Android System WebView」，或换一台设备比对同一网址。"
        );
    }
    format!(
        "可达性探测：{host} 已连通（{st}，{ms}ms），但页面未渲染 —— \
         问题多半在本机 WebView 内核（版本过旧 / TLS 协商 / 渲染），不在网络。\
         可先升级系统「Android System WebView」，或换一台设备比对同一网址。"
    )
}

/// 代理模式下上游返回 401/403 时的结论（#15）。
///
/// 为什么单独一类：[`probe_once`] 把「拿到任意 HTTP 响应」都算可达，401 也算 —— 但对
/// **带鉴权**的站点而言，401 恰恰说明本地代理那层 Authorization 注入没生效。这与
/// 「内核不渲染」是两条完全不同的分诊线，混在一起会把排查引到错误方向。
///
/// `host` 只放上游主机名（不带路径，避免把入口令牌带进用户可见文案）。
#[cfg(any(not(windows), test))]
fn probe_auth_message(host: &str, status: u16) -> String {
    format!(
        "可达性探测：上游 {host} 返回 HTTP {status}（未授权）—— 上游可达，但本地代理注入的\
         鉴权未通过。请检查打包配置里的账号密码，或改用无需鉴权的入口地址。"
    )
}

/// 可达性探测的目标（#15）：代理模式下探**真实上游**，否则探导航地址本身。
///
/// 返回 `(url, via_proxy)`。旧实现一律探 `nav.url`，代理模式下那是
/// `http://127.0.0.1:<port>` —— 只能证明本地代理进程活着，证明不了上游可达；
/// 用户看到的「127.0.0.1 已连通」与我们真正想回答的问题（上游通不通）无关，
/// 属于「取证方法错 ⇒ 结论错」。
///
/// 上游 origin 缺失（理论上不该发生：`prepare_navigation` 一定会填）时退回探
/// `nav.url`，退化成旧行为但绝不 panic。
#[cfg(any(not(windows), test))]
fn probe_target(nav: &Navigation) -> (String, bool) {
    if nav.use_local_proxy {
        if let Some(origin) = nav
            .upstream_origin
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return (origin.to_string(), true);
        }
    }
    (nav.url.clone(), false)
}

/// 导航提交后发起可达性探测；把结论（**成功 / 失败分类**）在主线程 eval 给加载层卡片。
///
/// ⚠️ 成败**都要报**：成功时给一行「已连通但未渲染」的结论（见 [`probe_ok_message`]），失败时
/// 按类别给出带主机的具体原因（见 [`probe_failure_message`]）。只报失败会让「壳内不渲染」这类
/// 故障只剩一张零信息的兜底卡 —— 而那恰恰是最需要判据的情形。
///
/// `url` 是 [`probe_target`] 算出的**探测目标**：代理模式下是上游 origin，不再是
/// `127.0.0.1:<port>`（清单 #15 修的就是这一点）；`via_proxy` 决定成功 / 401 文案的措辞。
/// `AppHandle` 按值传入：探测跑在 `'static` 的 Tokio 任务里，不能借用调用方的引用。
#[cfg(not(windows))]
fn spawn_reachability_probe(app: tauri::AppHandle, label: &str, url: String, via_proxy: bool) {
    let host = Url::parse(&url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "目标服务器".to_string());
    // label 是调用方的 &str（非 'static），spawn 任务要求 'static 捕获，故先转成 owned。
    let label = label.to_string();

    tauri::async_runtime::spawn(async move {
        match probe_once(&url).await {
            ProbeOutcome::Failed(kind) => {
                let (title, detail) = probe_failure_message(kind, &host);
                let a = app.clone();
                let l = label.clone();
                // eval 必须回主线程（Tauri 的 WebView 操作都要求主线程）。
                let _ = app.run_on_main_thread(move || {
                    let Some(w) = a.get_webview_window(&l) else {
                        return;
                    };
                    // 第三个参数 1 = soft：页面若已渲染出正文，loading.js 会忽略本次结论。
                    let js = format!(
                        "try{{window.__web2appLoading&&window.__web2appLoading.fail({t},{d},1);}}catch(e){{}}",
                        t = crate::loading::js_str(&title),
                        d = crate::loading::js_str(&detail)
                    );
                    let _ = w.eval(js.as_str());
                });
            }
            ProbeOutcome::Reachable { status, ms } => {
                // 代理模式下的 401/403：上游可达但鉴权没过，单独一条结论（清单 #15）。
                let line = match (via_proxy, status) {
                    (true, Some(code)) if code == 401 || code == 403 => {
                        probe_auth_message(&host, code)
                    }
                    _ => probe_ok_message(&host, status, ms, via_proxy),
                };
                let a = app.clone();
                let l = label.clone();
                let _ = app.run_on_main_thread(move || crate::loading::show_note(&a, &l, &line));
            }
        }
    });
}

/// 探测的两种结果：可达（拿到任意 HTTP 响应，含 4xx/5xx）或按类别失败。
#[cfg(not(windows))]
enum ProbeOutcome {
    /// 收到了响应；`status` 为 `None` 表示内核未提供状态码（读不出来也不影响判读）。
    Reachable {
        status: Option<u16>,
        ms: u128,
    },
    Failed(ProbeFailure),
}

/// 发起一次轻量探测；收到任意 HTTP 响应（含 4xx/5xx）即视为传输层可达。
#[cfg(not(windows))]
async fn probe_once(url: &str) -> ProbeOutcome {
    use std::time::Duration;

    let client = match reqwest::Client::builder()
        .user_agent(PROBE_USER_AGENT)
        .connect_timeout(Duration::from_secs(PROBE_CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(PROBE_TOTAL_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return ProbeOutcome::Failed(ProbeFailure::Other),
    };

    // 只取响应头、不读正文（不额外消耗用户流量，也不关心正文内容）。耗时用于判读弱网。
    let t0 = std::time::Instant::now();
    match client.get(url).send().await {
        Ok(resp) => ProbeOutcome::Reachable {
            status: Some(resp.status().as_u16()),
            ms: t0.elapsed().as_millis(),
        },
        Err(e) => ProbeOutcome::Failed(classify_probe_error(&e)),
    }
}

/// 把 reqwest 错误归类为 [`ProbeFailure`]。
///
/// reqwest/rustls 的 TLS 错误通常也挂在 connect 阶段下，因此对 connect 错误
/// 再沿错误源链找证书 / TLS / 握手关键字做一次细分。
#[cfg(not(windows))]
fn classify_probe_error(e: &reqwest::Error) -> ProbeFailure {
    if e.is_timeout() {
        ProbeFailure::Timeout
    } else if e.is_connect() {
        let mut source: Option<&(dyn std::error::Error + 'static)> =
            Some(e as &(dyn std::error::Error + 'static));
        while let Some(err) = source {
            let msg = err.to_string().to_ascii_lowercase();
            if msg.contains("certificate") || msg.contains("tls") || msg.contains("handshake") {
                return ProbeFailure::Tls;
            }
            source = err.source();
        }
        ProbeFailure::Connect
    } else if e.is_request() {
        ProbeFailure::BadRequest
    } else {
        ProbeFailure::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_failure_messages_include_host_and_distinct_reason() {
        // 每种类别给出不同标题、且详情必须包含主机名（方便用户判断是哪个站点不通）。
        let host = "ai260926.osfipin.com";
        let cases = [
            (ProbeFailure::Connect, "无法连接服务器", "域名解析"),
            (ProbeFailure::Timeout, "连接超时", "超时"),
            (ProbeFailure::Tls, "安全连接失败", "TLS"),
            (ProbeFailure::BadRequest, "地址有误", "ai260926.osfipin.com"),
            (ProbeFailure::Other, "页面无法打开", "ai260926.osfipin.com"),
        ];
        let mut titles = std::collections::HashSet::new();
        for (kind, title, detail_kw) in cases {
            let (t, d) = probe_failure_message(kind, host);
            assert_eq!(t, title);
            assert!(d.contains(host), "{t:?} 详情应包含主机名：{d}");
            assert!(d.contains(detail_kw), "{t:?} 详情应含 {detail_kw:?}：{d}");
            assert!(titles.insert(t.clone()), "失败标题不能重复：{t}");
        }
    }

    #[test]
    fn probe_failure_message_never_embeds_credentials_or_full_url() {
        // 防护：即便调用方误把带 userinfo / 令牌的完整 URL 传进来，文案映射只接收 host。
        // 这里钉住函数签名语义：host 原样出现，而 userinfo 这类内容不会被函数自行引入。
        let (_t, d) = probe_failure_message(ProbeFailure::Connect, "127.0.0.1:59123");
        assert!(d.contains("127.0.0.1:59123"));
        assert!(!d.contains("token="));
    }

    #[test]
    fn probe_ok_message_reports_reachability_and_points_at_webview() {
        // 成功路径也必须给出结论：只报失败会让「Rust 侧能连通、壳内却不渲染」这类故障
        // 只剩一张零信息的兜底卡 —— 而那正是真机取证（2026-10-09）遇到的实际情况。
        let m = probe_ok_message("ai260926.osfipin.com", Some(200), 812, false);
        assert!(m.contains("ai260926.osfipin.com"), "应含主机名：{m}");
        assert!(m.contains("HTTP 200"), "应含状态码：{m}");
        assert!(m.contains("812ms"), "应含耗时：{m}");
        assert!(
            m.contains("WebView"),
            "结论必须指向「本机 WebView 内核」，否则用户会继续往网络上找：{m}"
        );
        // 老内核 / 重定向后拿不到状态码时，不得把 Option 的 Debug 形态漏到用户文案里。
        let m2 = probe_ok_message("h.example", None, 5, false);
        assert!(m2.contains("已收到响应"), "无状态码时应给中性表述：{m2}");
        assert!(
            !m2.contains("None"),
            "不得把 Option 的 Debug 形态漏进用户文案：{m2}"
        );
        // 代理模式：必须点明「经本地代理访问上游」，否则用户会把结论读成 127.0.0.1（#15）。
        let mp = probe_ok_message("up.example", Some(502), 30, true);
        assert!(
            mp.contains("经本地代理访问上游") && mp.contains("up.example"),
            "代理模式的结论必须说明经代理访问的是上游：{mp}"
        );
    }

    /// #15 的成对判据：代理模式探上游、上游缺失才退回导航地址；直连不受影响。
    #[test]
    fn probe_target_prefers_upstream_origin_when_proxied() {
        let nav = |url: &str, proxy: bool, upstream: Option<&str>| Navigation {
            url: url.to_string(),
            user: String::new(),
            pass: String::new(),
            use_local_proxy: proxy,
            proxy_abort: None,
            upstream_origin: upstream.map(str::to_string),
        };
        // 代理 + 上游已知 → 探上游。
        let (u, via) = probe_target(&nav(
            "http://127.0.0.1:51234/",
            true,
            Some("https://ai260926.osfipin.com"),
        ));
        assert_eq!(u, "https://ai260926.osfipin.com");
        assert!(via, "代理模式下必须把 via_proxy 传给文案层");
        // 代理但上游信息缺失 → 退回导航地址（不 panic）。
        let (u2, via2) = probe_target(&nav("http://127.0.0.1:51234/", true, None));
        assert_eq!(u2, "http://127.0.0.1:51234/");
        assert!(!via2);
        // 直连：导航地址本身就是上游。
        let (u3, via3) = probe_target(&nav("https://a.com/", false, Some("https://a.com")));
        assert_eq!(u3, "https://a.com/");
        assert!(!via3);
        // 上游是空白串也要退回（防「字段有值但没内容」）。
        let (u4, _) = probe_target(&nav("http://127.0.0.1:1/", true, Some("   ")));
        assert_eq!(u4, "http://127.0.0.1:1/");
    }

    /// #15：401/403 单独成类 —— 「鉴权没过」与「内核不渲染」不是同一条分诊线。
    #[test]
    fn probe_auth_message_separates_unauthorized_from_render_failure() {
        let m = probe_auth_message("ai260926.osfipin.com", 401);
        assert!(m.contains("ai260926.osfipin.com"), "应含上游主机名：{m}");
        assert!(m.contains("HTTP 401"), "应含状态码：{m}");
        assert!(m.contains("鉴权"), "必须点明是鉴权问题：{m}");
        assert!(
            !m.contains("WebView 内核"),
            "未授权不能引导用户去查内核渲染：{m}"
        );
        assert!(probe_auth_message("h.example", 403).contains("HTTP 403"));
    }
}
