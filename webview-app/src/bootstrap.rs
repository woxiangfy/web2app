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
    /// 上游真实主机（host[:port]，来自解析出的 APP_URL），供多页签判断
    /// `NewWindowRequested` 的目标 URL 是否与上游同源（决定是否路由到该路径）。
    pub(crate) upstream_host: Option<String>,
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

/// 去掉 URL 中的 userinfo（用户名:密码），返回“干净”地址。
/// 用于直连 / 外部代理场景下 WebView 的导航地址（鉴权由 WebView2 原生事件或本地代理注入，
/// 不应把密码写进浏览器地址栏 / 交给外部代理字符串）。
fn strip_userinfo(url: &Url) -> String {
    let mut u = url.clone();
    let _ = u.set_username("");
    let _ = u.set_password(None);
    u.to_string()
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

        match prepare_navigation(use_local_proxy) {
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
        upstream_host: parsed.host_str().map(str::to_string),
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
}
