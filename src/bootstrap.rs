//! 启动引导：后台准备真实地址 → 就绪后导航。
//!
//! `run()`（lib.rs）只负责「建窗 + 加载本地启动页」，真正的耗时操作（域名解析、启动本地反向代理）
//! 放到这里的后台线程：这样"加载中…"从窗口出现的第一帧就可见，而不是等准备全部做完。
//! 准备完成后回到主线程发起导航；失败则在启动页上展示错误与重试入口。

use tauri::Manager;
use url::Url;

use crate::config::{APP_URL, PROXY_ENABLED, PUBLIC_KEY};
#[cfg(windows)]
use crate::config::build_proxy_args;

/// 窗口/WebView 的默认底色，与加载层（src/loading.js）的浅色底一致。
/// 用于盖住 WebView 初始化与导航提交瞬间的白闪，使"启动页 → 加载层 → 正式页面"全程平滑。
pub(crate) const SPLASH_BG: tauri::window::Color = tauri::window::Color(0xf7, 0xf8, 0xfa, 255);

/// 启动期在后台线程准备出来的导航信息。
struct Navigation {
    /// WebView 最终要访问的地址（启用本地反向代理时形如 http://127.0.0.1:<port>）。
    url: String,
    /// Basic 鉴权账号密码（从网址 userinfo 中拆出，供 Windows WebView2 钩子兜底）。
    user: String,
    pass: String,
    /// 是否走了本地反向代理（决定 Windows 上是否还需要注册原生鉴权钩子）。
    use_local_proxy: bool,
    /// 本地反向代理任务的终止句柄，存入 ProxyTask 以便退出或重试时优雅关闭。
    proxy_abort: Option<tokio::task::AbortHandle>,
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
pub(crate) fn bootstrap(app: tauri::AppHandle) {
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
                let a = app.clone();
                let _ = app.run_on_main_thread(move || navigate_main(&a, nav));
            }
            Err(msg) => {
                eprintln!("[bootstrap] {msg}");
                let a = app.clone();
                // 启动页仍然显示着，直接把错误态推上去（含重试按钮），
                // 不再像早期版本那样 std::process::exit —— 用户至少能看到发生了什么。
                let _ = app.run_on_main_thread(move || crate::loading::show_error(&a, "加载失败", &msg));
            }
        }
    });
}

/// 解析真实地址并按需启动本地反向代理（在后台线程执行，可能耗时）。
fn prepare_navigation(use_local_proxy: bool) -> Result<Navigation, String> {
    // 若 APP_URL 是【域名数组】，则依次对这些域名做 DNS TXT 查询 + Ed25519 验签
    // + AES-GCM 解密得到真实 URL；全部失败时返回可展示的错误文案。
    let resolved_url: String = if let Some(domains) = crate::resolve::parse_domains(APP_URL) {
        let domains: Vec<String> = domains
            .into_iter()
            .map(|d| crate::resolve::normalize_domain(&d))
            .collect();
        crate::resolve::resolve_url_from_domains(&domains, PUBLIC_KEY)
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
    })
}

/// 在主线程发起首次导航（准备工作已在后台线程完成）。
fn navigate_main(app: &tauri::AppHandle, nav: Navigation) {
    // 代理句柄先入库，保证退出 / 重试时能优雅终止。
    if let Some(abort) = nav.proxy_abort {
        if let Ok(mut g) = app.state::<ProxyTask>().0.lock() {
            *g = Some(abort);
        }
    }

    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    // Windows：必须先注册 WebView2 钩子（新窗口拦截 + Basic 鉴权）再导航，否则窗口首批
    // 请求会漏掉 Authorization 注入；attach_webview_hooks 内部在钩子注册完成后才发起导航。
    // 导航产生的新文档由加载层（overlay 模式）接力覆盖空白期，内容就绪后淡出。
    #[cfg(windows)]
    crate::webview_hooks::attach_webview_hooks(
        app.clone(),
        &window,
        nav.user,
        nav.pass,
        nav.url,
        nav.use_local_proxy,
    );

    // 其它平台（Android 等）：没有 WebView2，直接导航；鉴权统一由本地反向代理注入。
    #[cfg(not(windows))]
    {
        let _ = (&nav.user, &nav.pass, &nav.use_local_proxy);
        match Url::parse(&nav.url) {
            Ok(url) => {
                if let Err(e) = window.navigate(url) {
                    crate::loading::show_error(app, "加载失败", &format!("无法打开页面：{e}"));
                }
            }
            Err(_) => crate::loading::show_error(app, "加载失败", "应用配置的地址不合法"),
        }
    }
}
