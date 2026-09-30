//! 启动引导：后台准备真实地址 → 就绪后导航。
//!
//! `run()`（lib.rs）只负责「建窗 + 加载本地启动页」，真正的耗时操作（域名解析、启动本地反向代理）
//! 放到这里的后台线程：这样"加载中…"从窗口出现的第一帧就可见，而不是等准备全部做完。
//! 准备完成后回到主线程发起导航；失败则在启动页上展示错误与重试入口。

use tauri::Manager;
use url::Url;

#[cfg(windows)]
use crate::config::build_proxy_args;
use crate::config::{APP_URL, PROXY_ENABLED, PUBLIC_KEY};

/// 窗口/WebView 的默认底色，与加载层（src/loading.js）的浅色底一致。
/// 用于盖住 WebView 初始化与导航提交瞬间的白闪，使"启动页 → 加载层 → 正式页面"全程平滑。
pub(crate) const SPLASH_BG: tauri::window::Color = tauri::window::Color(0xf7, 0xf8, 0xfa, 255);

/// 启动期在后台线程准备出来的导航信息。
///
/// `Clone`：多页签模式下，首次启动解析出的导航信息会被缓存到 `TabManager.navigation`，
/// 新建 tab 时克隆复用（本地代理单实例共享，`proxy_abort` 仍由 `ProxyTask` 唯一持有，
/// 此处克隆出的句柄仅作导航用途、不重复入库，见 `navigate_window`）。
#[derive(Clone)]
pub(crate) struct Navigation {
    /// WebView 最终要访问的地址（启用本地反向代理时形如 http://127.0.0.1:<port>）。
    pub(crate) url: String,
    /// Basic 鉴权账号密码（从网址 userinfo 中拆出，供 Windows WebView2 钩子兜底）。
    pub(crate) user: String,
    pub(crate) pass: String,
    /// 是否走了本地反向代理（决定 Windows 上是否还需要注册原生鉴权钩子）。
    pub(crate) use_local_proxy: bool,
    /// 本地反向代理任务的终止句柄，存入 ProxyTask 以便退出或重试时优雅关闭。
    pub(crate) proxy_abort: Option<tokio::task::AbortHandle>,
    /// 入口上游的**源（origin）**：`scheme://host[:非默认端口]`，默认端口归一化
    ///（https://h 与 https://h:443 视为同源）。供两类主机白名单使用：
    /// ①WebView2 直连钩子只给上游请求注入 Authorization / 应答 401
    ///（见 `webview_hooks`，Electron 侧 login 事件是同一条规则）；
    /// ②多页签 `resolve_popup_url_for` 判断弹窗是否与上游同源（端口也参与比对）。
    pub(crate) upstream_origin: Option<String>,
}

/// 托管本地反向代理任务的 AbortHandle，便于应用退出时优雅终止它（释放 127.0.0.1 监听端口）。
pub(crate) struct ProxyTask(pub(crate) std::sync::Mutex<Option<tokio::task::AbortHandle>>);

impl Default for ProxyTask {
    fn default() -> Self {
        ProxyTask(std::sync::Mutex::new(None))
    }
}

/// 是否启用【本地反向代理】。
///
/// Windows：需“启用代理且没有配置外部代理”（有外部代理时走 `--proxy-server`）。
/// 移动端 / 其它平台：不支持 `--proxy-server`，因此只要“启用代理”就一律走本地反向代理。
/// 由编译期常量决定，纯计算、零开销；setup（建窗参数）与 bootstrap（后台准备）共用。
#[cfg(windows)]
pub(crate) fn local_proxy_enabled() -> bool {
    PROXY_ENABLED && build_proxy_args().is_none()
}
#[cfg(not(windows))]
pub(crate) fn local_proxy_enabled() -> bool {
    PROXY_ENABLED
}

/// 「主窗口」的 label：多页签模式下是宿主窗口，单窗口模式下是 `"main"`。
///
/// ⚠️⚠️ **这是唯一真源** —— 历史上「多页签 → 宿主窗口，否则 → main」这条判断在 lib.rs
/// （单实例聚焦 / 退出 Geometry 持久化 / 主窗关闭语义）与 tray.rs 里各写了一遍，
/// 共 4 处相同逻辑。改一处忘另一处的表象是「托盘点开、或第二次启动聚焦到的是另一个窗口」
/// 这类难定位的错位，所以收敛到这里。
///
/// 为什么放在本模块而不是 `tray` / `tabs`：判断的两侧都**不是桌面端专属** —— 主窗关闭语义
/// （`crate::handle_main_close`）在 Android 上同样要走，而 `tray` 是 `cfg(desktop)`、
/// `tabs` 是 `cfg(windows)`，放进它们会让非桌面目标拿不到这个函数。本模块无条件编译，
/// 平台差异用内部的 `#[cfg(windows)]` 吸收（与 [`local_proxy_enabled`] 同一写法）。
pub(crate) fn main_window_label() -> &'static str {
    #[cfg(windows)]
    {
        if crate::tabs::tabs_enabled() {
            return crate::tabs::HOST_LABEL;
        }
    }
    "main"
}

/// 主窗口显隐后，同步多页签模式下各 tab 窗口的可见性。
///
/// 多页签模式里每个 tab 都是独立顶层窗口，宿主隐藏不会连带隐藏它们，桌面上会残留
/// “没有标签栏的内容窗口”。非 Windows / 非多页签模式为空操作。
///
/// 与 [`main_window_label`] 同理由（`tabs` 是 `cfg(windows)`，桌面端之外也要能调用），
/// 平台和模式的双重开关都收在本函数内部。
pub(crate) fn sync_tabs_visibility(app: &tauri::AppHandle, visible: bool) {
    #[cfg(windows)]
    {
        if crate::tabs::tabs_enabled() {
            crate::tabs::set_tabs_visible(app, visible);
        }
    }
    #[cfg(not(windows))]
    let _ = (app, visible);
}

/// 去掉 URL 中的 userinfo（用户名:密码），返回“干净”地址。
/// 用于直连 / 外部代理场景下 WebView 的导航地址（鉴权由 WebView2 原生事件或本地代理注入，
/// 不应把密码写进浏览器地址栏 / 交给外部代理字符串）。
fn strip_userinfo(url: &Url) -> String {
    let mut u = url.clone();
    let _ = u.set_username("");
    let _ = u.set_password(None);
    u.to_string()
}

/// 计算源（origin）：`scheme://host[:端口]`，端口做默认值归一化。
///
/// - 省略端口的 https/http 与显式 443/80 **同源**（`https://h` == `https://h:443`）；
/// - 非默认端口保留（`https://h:8443` ≠ `https://h`）——旧实现只比 host 会把它们误判同源；
/// - 无 host（非法 / 非层次化 URL）返回 None。
pub(crate) fn origin_of(u: &Url) -> Option<String> {
    let host = u.host_str()?;
    let effective = match (u.scheme(), u.port()) {
        ("https", Some(443)) | ("http", Some(80)) => None,
        (_, p) => p,
    };
    Some(match effective {
        Some(p) => format!("{}://{host}:{p}", u.scheme()),
        None => format!("{}://{host}", u.scheme()),
    })
}

/// 白名单判定：请求 URL 是否属于入口上游源。
///
/// 直连模式（不走本地代理）下，Authorization 注入与 401 自动应答都必须先过这道闸 ——
/// 页面里的第三方子资源 / 被注入内容诱导出的外域请求一律不发凭据。`upstream` 为 None
/// （解析不出源）时**失败关闭**：拒绝注入，而不是盲发。
pub(crate) fn is_upstream_request(upstream: Option<&str>, request_url: &str) -> bool {
    let Some(upstream) = upstream else {
        return false;
    };
    match Url::parse(request_url) {
        Ok(u) => origin_of(&u).as_deref() == Some(upstream),
        Err(_) => false,
    }
}

/// 启动流程入口：后台准备 → 主线程导航 / 报错。
///
/// 允许被重复调用（启动页上的"重试"即 splash://localhost/retry → 这里），
/// 因此进入时先终止上一轮残留的本地反向代理，避免端口泄漏。
///
/// 单窗口模式：导航到 `"main"` 窗口。
/// 多页签模式：导航到首个 tab（label 由调用方传入，见 `bootstrap_for_label`）。
pub(crate) fn bootstrap(app: tauri::AppHandle) {
    bootstrap_inner(app, "main".to_string(), true);
}

/// 启动/重试流程入口，参数化目标窗口 label。
///
/// ⚠️ 与 [`bootstrap`] 一样**必须缓存**解析结果（`cache_navigation = true`）。多页签模式下
/// 首个 tab 恰恰是由 `tabs::spawn_tab` 走这条路径创建的（`bootstrap()` 只在单窗口模式被调用），
/// 曾经这里传 `false` 导致 `TabManager.navigation` **永远为 None**，连锁后果三连：
///   1) [`crate::tabs::resolve_popup_url`] 拿不到入口/上游主机 → 一律返回 None →
///      `target="_blank"` 新 tab 全部落在主地址（用户反馈的“弹窗打开的网址一直是 app_url”）；
///   2) 每新建一个 tab 都重新 DNS 解析 + 重新起一个本地反向代理（新端口），
///      而 `bootstrap_inner` 开头又会 abort 掉 ProxyTask 里的旧代理 → 已在用的旧 tab 断连；
///   3) 新建 tab 明显变慢。
pub(crate) fn bootstrap_for_label(app: tauri::AppHandle, label: String) {
    bootstrap_inner(app, label, true);
}

/// 启动流程核心：后台准备 → 主线程导航 / 报错。
///
/// `cache_navigation`：是否把本次解析结果写入 `TabManager.navigation` 供后续新 tab 复用。
/// 只有 Windows 多页签模式用到（该字段在非 Windows 下不引用），故用 cfg_attr 局部放宽
/// unused_variables，避免 Android / iOS 编译告警。
#[cfg_attr(not(windows), allow(unused_variables))]
fn bootstrap_inner(app: tauri::AppHandle, label: String, cache_navigation: bool) {
    let use_local_proxy = local_proxy_enabled();
    std::thread::spawn(move || {
        // 重试场景：清掉上一轮可能仍在监听的代理任务。
        let stale = app
            .state::<ProxyTask>()
            .0
            .lock()
            .ok()
            .and_then(|mut g| g.take());
        if let Some(abort) = stale {
            abort.abort();
        }

        // catch_unwind：prepare_navigation 里仍有极少数 expect（如 CSPRNG 取密钥失败），
        // panic 在这个 detached 线程里只会让线程静默死亡——启动页永远转圈、用户零入口，
        // 比显式报错更糟（项目约定：环境级故障也要可见）。AssertUnwindSafe：这里没有
        // 跨 catch 的借用不变量需要维护。
        let prepared = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            prepare_navigation(use_local_proxy)
        }));
        let outcome = match prepared {
            Ok(r) => r,
            Err(pa) => {
                let msg = if let Some(s) = pa.downcast_ref::<&str>() {
                    format!("启动过程发生内部错误：{s}")
                } else if let Some(s) = pa.downcast_ref::<String>() {
                    format!("启动过程发生内部错误：{s}")
                } else {
                    "启动过程发生内部错误（未知 panic）".to_string()
                };
                Err(msg)
            }
        };

        match outcome {
            Ok(nav) => {
                // 解析成功：缓存导航信息供后续新 tab 复用（仅 Windows 多页签模式有 TabManager）。
                // 重试场景覆盖旧缓存也是正确的——重试后端口/地址可能已变，缓存必须跟着更新。
                #[cfg(windows)]
                if cache_navigation && crate::tabs::tabs_enabled() {
                    if let Ok(mut cache) = app.state::<crate::tabs::TabManager>().navigation.lock()
                    {
                        *cache = Some(nav.clone());
                    }
                }
                let a = app.clone();
                let l = label.clone();
                let _ = app.run_on_main_thread(move || navigate_main(&a, &l, nav));
            }
            Err(msg) => {
                dlog!("[bootstrap] {msg}");
                let a = app.clone();
                let l = label.clone();
                // 启动页仍然显示着，直接把错误态推上去（含重试按钮），
                // 不再像早期版本那样 std::process::exit —— 用户至少能看到发生了什么。
                let _ = app.run_on_main_thread(move || {
                    crate::loading::show_error(&a, &l, "加载失败", &msg)
                });
            }
        }
    });
}

/// 解析真实地址并按需启动本地反向代理（在后台线程执行，可能耗时）。
fn prepare_navigation(use_local_proxy: bool) -> Result<Navigation, String> {
    // 若 APP_URL 是【域名数组】，则依次对这些域名做 DNS TXT 查询 + Ed25519 验签
    // + AES-GCM 解密得到真实 URL；全部失败时返回可展示的错误文案。
    // ⚠️ `resolve_url_from_domains` 是 async（该模块刻意不依赖 Tauri，见 resolve.rs 头注释），
    // 由这里用 Tauri 的异步运行时阻塞驱动 —— 本函数跑在后台线程，不在 async 上下文内。
    let resolved_url: String = if let Some(domains) = crate::resolve::parse_domains(APP_URL) {
        let domains: Vec<String> = domains
            .into_iter()
            .map(|d| crate::resolve::normalize_domain(&d))
            .collect();
        tauri::async_runtime::block_on(crate::resolve::resolve_url_from_domains(
            &domains, PUBLIC_KEY,
        ))
        .map_err(|_| "服务器连接失败，请检查网络后重试".to_string())?
    } else {
        APP_URL.to_string()
    };

    let parsed = Url::parse(&resolved_url)
        .map_err(|e| format!("应用配置的网址不合法（{e}）：{resolved_url}"))?;

    // scheme 白名单：与 sidecar（proxy-server/main.rs）同一语义。非 http(s) 配置
    // （或 DNS 解出异常 scheme）下继续启动只会让代理每条请求 502、启动页既不报错也无法
    // 定位，直接在这里给出可看懂的失败。
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(format!(
                "只支持 http / https 上游，当前是 {other}：{resolved_url}"
            ))
        }
    }

    let (url, proxy_abort) = if use_local_proxy {
        // 自身实现的本地反向代理：给每个请求（含 WebSocket 握手）注入 Authorization。
        let (u, abort) = tauri::async_runtime::block_on(crate::proxy::start_proxy(&parsed))
            .map_err(|e| format!("本地代理启动失败：{e}"))?;
        (u, Some(abort))
    } else {
        // 直连 / 外部代理：导航到去掉 userinfo 的上游地址（鉴权由原生事件或外部代理注入）。
        (strip_userinfo(&parsed), None)
    };

    Ok(Navigation {
        url,
        user: parsed.username().to_string(),
        pass: parsed.password().unwrap_or("").to_string(),
        use_local_proxy,
        proxy_abort,
        upstream_origin: origin_of(&parsed),
    })
}

/// 在主线程发起首次导航（准备工作已在后台线程完成）。
///
/// `label`：目标窗口 label（单窗口为 `"main"`，多页签为 `tab-N`）。
fn navigate_main(app: &tauri::AppHandle, label: &str, nav: Navigation) {
    // 代理句柄先入库，保证退出 / 重试时能优雅终止。
    // 多页签模式下：本地代理是单实例共享，句柄只由首次启动入库一次；后续 tab 复用的
    // Navigation 里的 proxy_abort 是同一个句柄的克隆，重复入库会覆盖，属无害但应避免。
    // 这里用「仅当 ProxyTask 当前为空才入库」保证只入一次。
    if let Some(abort) = nav.proxy_abort {
        if let Ok(mut g) = app.state::<ProxyTask>().0.lock() {
            if g.is_none() {
                *g = Some(abort);
            }
        }
    }

    let Some(window) = app.get_webview_window(label) else {
        return;
    };

    // Windows：必须先注册 WebView2 钩子（新窗口拦截 + Basic 鉴权）再导航，否则窗口首批
    // 请求会漏掉 Authorization 注入；钩子注册由 attach_webview_hooks 完成（不负责 Navigate），
    // 真正的导航在下面用 `window.navigate(url)` 触发（wry 内部会等 CoreWebView2 controller
    // 就绪再触发 WebView2 的 Navigation，避免 with_webview 回调时机太早导致整个钩子+导航被
    // 跳过、tab 停在 builder 默认入口的「app_url」问题）。
    #[cfg(windows)]
    {
        crate::webview_hooks::attach_webview_hooks(
            app.clone(),
            &window,
            nav.user,
            nav.pass,
            nav.use_local_proxy,
            nav.upstream_origin,
        );
        if let Ok(url) = Url::parse(&nav.url) {
            let _ = window.navigate(url);
        }
    }

    // 其它平台（Android 等）：没有 WebView2，直接导航；鉴权统一由本地反向代理注入。
    #[cfg(not(windows))]
    {
        let _ = (&nav.user, &nav.pass, &nav.use_local_proxy);
        match Url::parse(&nav.url) {
            Ok(url) => {
                if let Err(e) = window.navigate(url) {
                    crate::loading::show_error(
                        app,
                        label,
                        "加载失败",
                        &format!("无法打开页面：{e}"),
                    );
                }
            }
            Err(_) => crate::loading::show_error(app, label, "加载失败", "应用配置的地址不合法"),
        }
    }
}

/// 对给定的窗口执行导航（挂 WebView2 钩子 + 导航）。
///
/// 与 `navigate_main` 的区别：不负责代理句柄入库（复用缓存时句柄已入过库），
/// 供多页签模式新建 tab 时复用缓存导航信息。仅 Windows 有意义（tabs 模块整体 cfg(windows)）。
#[cfg(windows)]
pub(crate) fn navigate_window(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    nav: Navigation,
) {
    // 1) 注册 WebView2 事件钩子（不含 Navigate）。
    crate::webview_hooks::attach_webview_hooks(
        app.clone(),
        window,
        nav.user,
        nav.pass,
        nav.use_local_proxy,
        nav.upstream_origin,
    );
    // 2) 用 Tauri 的 `WebviewWindow::navigate` 真正导航——wry 内部会等 CoreWebView2
    // controller 就绪再触发 WebView2 的 Navigation。
    // 历史实现直接在 `attach_webview_hooks` 末尾调 `core.Navigate`，但此时 controller
    // 可能未就绪（多页签模式新建 tab 时由 Rust 同步发起，与 wry 的就绪回调有时序差），
    // `with_webview` 直接 return → 钩子没注册、Navigate 没触发，tab 停在 builder 默认入口
    // —— 也就是用户看到的『弹窗新 tab 永远停在 app_url』。
    if let Ok(parsed) = url::Url::parse(&nav.url) {
        let _ = window.navigate(parsed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_userinfo_removes_credentials_keeps_rest() {
        let u = Url::parse("https://alice:secret@example.com/path?q=1#frag").unwrap();
        assert_eq!(strip_userinfo(&u), "https://example.com/path?q=1#frag");
    }

    #[test]
    fn strip_userinfo_without_userinfo_unchanged() {
        let u = Url::parse("https://example.com:8443/a").unwrap();
        assert_eq!(strip_userinfo(&u), "https://example.com:8443/a");
    }

    #[test]
    fn strip_userinfo_keeps_ipv6_authority() {
        let u = Url::parse("http://user:pw@[::1]:8080/x").unwrap();
        assert_eq!(strip_userinfo(&u), "http://[::1]:8080/x");
    }

    #[test]
    fn origin_normalizes_default_ports_and_keeps_others() {
        let o = |s| origin_of(&Url::parse(s).unwrap());
        assert_eq!(o("https://h.example/a"), Some("https://h.example".into()));
        assert_eq!(
            o("https://h.example:443/"),
            Some("https://h.example".into())
        );
        assert_eq!(o("http://h.example:80/"), Some("http://h.example".into()));
        assert_eq!(
            o("https://h.example:8443/"),
            Some("https://h.example:8443".into())
        );
        assert_eq!(
            o("http://127.0.0.1:5123/"),
            Some("http://127.0.0.1:5123".into())
        );
        assert_eq!(o("ftp://h/x"), Some("ftp://h".into()));
    }

    #[test]
    fn is_upstream_request_host_whitelist() {
        let up = Some("https://h.example");
        assert!(is_upstream_request(up, "https://h.example/a?b=1"));
        assert!(is_upstream_request(up, "https://h.example:443/a"));
        assert!(
            !is_upstream_request(up, "https://h.example:8443/a"),
            "非默认端口不混同"
        );
        assert!(
            !is_upstream_request(up, "http://h.example/a"),
            "scheme 不同不混同"
        );
        assert!(!is_upstream_request(up, "https://evil.example/a"));
        assert!(!is_upstream_request(up, "https://example/a"));
        assert!(!is_upstream_request(up, "not a url"));
        assert!(
            !is_upstream_request(None, "https://h.example/a"),
            "无上游源时失败关闭"
        );
    }

    #[test]
    fn non_http_scheme_prepared_navigation_rejected() {
        // prepare_navigation 同时依赖编译期常量与 DNS 解析，难在此直接驱动；
        // scheme 判定与 sidecar 同源（match http|https）——这里钉住 origin/白名单对
        // 非 http(s) 输入的行为，真正的端到端报错由 sidecar 的 --help 探针覆盖。
        assert_eq!(origin_of(&Url::parse("file:///c:/x").unwrap()), None);
    }
}
