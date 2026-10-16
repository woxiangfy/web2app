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

/// 「首个文档是否已加载完成」的登记表（label → 已完成过一次页面加载）。
///
/// ⚠️⚠️ **这是 Android 启动期一切壳侧动作的前置条件**（2026-10-09 第二次真机取证定到的根因）。
///
/// Android 上壳侧对 WebView 的操作全部走 wry 的 main-pipe 消息队列
///（`wry/src/android/main_pipe.rs`），而队列里两条关键消息在**目标未就绪时被静默丢弃**：
///   · `WebViewMessage::LoadUrl` 与 `WebViewMessage::Eval` 的处理体都是
///     `if let Some(webview) = get_webview(activity_id) { … }` —— 取不到就什么也不做，
///     既不报错也不重试；
///   · 那个 webview 只在 `CreateWebView`（建窗时发出）**处理完之后**才写入登记表。
///
/// 更关键的是**初始 splash 导航是异步排队的**：`CreateWebView` 内部走
/// `load_url(…, main_thread = true)` → Kotlin `loadUrlMainThread`（`RustWebView.kt` 里是
/// `post { loadUrl(url) }`，排到视图消息队列尾）；而之后的 `navigate` 走的是**同步** `loadUrl`。
/// 直连配置下 `prepare_navigation` 只需毫秒级（无域名解析、无本地代理），于是「导航」几乎必然
/// **先于**那条 post 出去的 splash 加载执行 —— 随后 splash 加载把导航**整体覆盖**，页面永远停在
/// 启动页。真机截图与之逐条吻合：`地址: http://splash.localhost/`、`元素 3`、HTTP 200。
/// 同一时刻那几条进度 `eval` 落在 splash 文档 document-start 脚本**之前**，
/// `window.__web2appLoading` 尚未定义 ⇒ `try{window.__web2appLoading&&…}` 静默短路
///（真机证据：40s 卡上四条阶段进度**一条都没有**）。
///
/// 因此非 Windows（Android）启动期必须先等**首个文档加载完成**，再做解析 / 推送 / 导航。
/// Windows 不需要也不该加：WebView2 路径的脚本注入与导航时序由 wry 自己保证，凭空加等待只会
/// 推迟启动，还引入一个新的时序假设。
#[cfg(not(windows))]
#[derive(Default)]
pub(crate) struct BootReady(std::sync::Mutex<std::collections::HashSet<String>>);

#[cfg(not(windows))]
impl BootReady {
    fn mark(&self, label: &str) {
        if let Ok(mut g) = self.0.lock() {
            g.insert(label.to_string());
        }
    }

    /// ⚠️ `Started` 必须清标记：`/retry` 会**重新加载启动页**，若沿用第一次的就绪标记，
    /// 重试时又会把导航发在 splash 之前 —— 变成「首启修好了、重试仍然卡死」。
    fn clear(&self, label: &str) {
        if let Ok(mut g) = self.0.lock() {
            g.remove(label);
        }
    }

    fn is_ready(&self, label: &str) -> bool {
        self.0.lock().map(|g| g.contains(label)).unwrap_or(false)
    }
}

/// 页面加载事件的落点（由建窗时注册的 `on_page_load` 调用；非 Windows 才有意义）。
///
/// wry Android 的 `ON_LOAD_HANDLER` 由 `RustWebViewClient.onPageStarted / onPageFinished`
/// 驱动（`tauri-runtime-wry` 里是**无条件** `with_on_page_load_handler`），所以 Android 上这个
/// 信号真实可用 —— 这是本项目在 Android 侧唯一可靠的「文档已就绪」判据（轮询 URL 不行：
/// `GetUrl` 同样走 main-pipe，webview 未就绪时拿不到结果）。
#[cfg(not(windows))]
pub(crate) fn on_page_load(app: &tauri::AppHandle, label: &str, finished: bool) {
    let ready = app.state::<BootReady>();
    if finished {
        ready.mark(label);
        // 移动端单 webview 页签：每次真实文档加载完成，把最终 url 记进活动标签
        //（标题由 mobilebar.js 的 web2app:mobile-doc 事件另补；启动页 url 在其内部被忽略）。
        #[cfg(mobile)]
        crate::mobilebar::on_page_finished(app);
    } else {
        ready.clear(label);
    }
}

/// 启动期等待「首个文档加载完成」的预算（毫秒）。
///
/// 启动页是本地自定义协议页（真机实测 HTTP 200 / 125ms），正常几十毫秒即完成；5s 是给冷启动的
/// WebView 留余量。**超时不致命**：继续按原流程走（退化成「不等待」的旧行为），只在卡片上留一行
/// 说明 —— 绝不因为拿不到页面加载事件就把启动彻底卡死。
///
/// 分支口径沿用本模块既有约定（`not(windows)`，与可达性探测那部分一致）：本项目实际发布的
/// 非 Windows 目标就是 Android；若将来跑桌面 Linux 而 wry 不上报 page load，最坏也只是每次启动
/// 多等这个预算，仍然能正常启动。
#[cfg(not(windows))]
const SPLASH_READY_BUDGET_MS: u64 = 5000;

/// 在后台线程等待首个文档加载完成（仅非 Windows；理由见 [`BootReady`] 的长注释）。
///
/// 返回 `true` = 拿到了加载完成事件；`false` = 预算内没拿到（页面加载事件缺失）。
#[cfg(not(windows))]
fn wait_first_page_ready(app: &tauri::AppHandle, label: &str) -> bool {
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_millis(SPLASH_READY_BUDGET_MS);
    while std::time::Instant::now() < deadline {
        if app.state::<BootReady>().is_ready(label) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    false
}

// ===========================================================================
// 非 Windows（Android 等）：导航后的「落点校验 + 有限重试」
// ===========================================================================
//
// 为什么需要：即使启动期已经等了首个文档就绪（见 [`BootReady`]），Android 上「导航是否
// 真的落到目标地址」**仍然没有可靠信号** —— wry 的 `navigate()` 恒返回 `Ok(())`（它只是
// 往 main-pipe 投一条消息），`onReceivedError` 也不向壳层透出任何网络错误。真机因此出现过
// 「导航确实发起了、地址却永远停在 `splash.localhost`」。
//
// 与其继续推断「启动页的异步加载与导航谁先谁后」，这里直接看**结果**：导航后隔一小段
// 读一次 WebView 的真实地址，若仍停在启动页就重发导航。这条判据不依赖任何时序假设，
// 也不依赖页面加载事件是否送达 —— 它是「导航到底有没有生效」的终局检验。
//
// 重发是安全的：`loadUrl` 语义上是「加载这个地址」的幂等操作，重复发起只会以最后一次为准，
// 不会叠加副作用；而真出问题时（导航被启动页挤掉）重发正是所需的那一次。

/// 导航发起后隔多久校验一次落点（毫秒）。
///
/// 取值理由：启动页是本地页面（真机实测 125ms 完成），而目标地址的 `loadUrl` 一旦被 WebView
/// 接受，`getUrl()` 会**立即**返回新地址（不等加载完成）。500ms 足够区分「已提交」与「没提交」，
/// 又不会让正常启动多等。
#[cfg(not(windows))]
const NAV_COMMIT_CHECK_MS: u64 = 500;

/// 落点仍在启动页时最多重发几次导航。
///
/// 只重发 2 次：第一次命中「导航被启动页挤掉」这一根因；第二次兜住「重发时启动页又抢了一次」
/// 的极端情形。再多没有新增覆盖面，只会拖长失败时的等待。
#[cfg(not(windows))]
const NAV_COMMIT_MAX_RETRY: u32 = 2;

/// 该地址是否仍是本地启动页。
///
/// ⚠️ **两种形态都要认**：`splash://localhost/`（`loading::splash_url()` 构造的）与
/// `http://splash.localhost/` —— wry 的 `custom_protocol_workaround` 在 Windows/Android 都会
/// 把前者改写成后者（`use_https_scheme` 默认 false，本项目未开），所以 WebView 里**真实存在**的
/// 地址是后者。只认自定义协议会在真机上漏判（这正是 v1.62 修过的那类判据坑）。
#[cfg(not(windows))]
fn is_splash_url(url: &Url) -> bool {
    url.scheme() == "splash" || url.host_str() == Some("splash.localhost")
}

/// 导航后校验落点；仍停在启动页则重发导航（仅非 Windows）。
///
/// ⚠️ **必须跑在独立线程**：调用方 `navigate_main` 在主线程执行，而这里要 sleep 且读地址 ——
/// `WebviewWindow::url()` 在 Android 上走 main-pipe（`GetUrl` 消息由主线程 Looper 消费），
/// 放在主线程会把 UI 冻住。
///
/// 正常启动（导航生效）时它只是一次 500ms 后的读地址 + 返回，零副作用；只有异常路径才会重发。
#[cfg(not(windows))]
fn ensure_navigation_committed(app: &tauri::AppHandle, label: &str, target: &Url) {
    let app = app.clone();
    let label = label.to_string();
    let target = target.clone();
    std::thread::spawn(move || {
        for attempt in 1..=NAV_COMMIT_MAX_RETRY {
            std::thread::sleep(std::time::Duration::from_millis(NAV_COMMIT_CHECK_MS));
            let Some(window) = app.get_webview_window(&label) else {
                return;
            };
            match window.url() {
                // 已离开启动页 = 导航提交成功，正常启动走这条分支。
                Ok(cur) if !is_splash_url(&cur) => {
                    if attempt > 1 {
                        note(&app, &label, &format!("重发导航已生效（第 {attempt} 次）"));
                    }
                    return;
                }
                Ok(_) => note(
                    &app,
                    &label,
                    &format!(
                        "导航未落到目标地址（仍停在启动页），正在重发（第 {attempt}/{NAV_COMMIT_MAX_RETRY} 次）…"
                    ),
                ),
                // 读不到地址不是重发的理由，但也不该吞掉 —— 写进卡片供真机判读。
                Err(e) => note(
                    &app,
                    &label,
                    &format!("读取当前地址失败（{e}），仍尝试重发导航"),
                ),
            }
            let a = app.clone();
            let l = label.clone();
            let u = target.clone();
            let _ = app.run_on_main_thread(move || {
                if let Some(w) = a.get_webview_window(&l) {
                    let _ = w.navigate(u);
                }
            });
        }
        // 走到这里 = 每一轮校验都还停在启动页。把「重发也没用」说清楚 —— 它的分诊含义
        // 与「导航根本没发出去」不同：地址是对的、导航也发了，是 WebView 没提交。
        note(
            &app,
            &label,
            &format!(
                "重发导航 {NAV_COMMIT_MAX_RETRY} 次后仍未离开启动页（导航未提交，与网络无关）"
            ),
        );
    });
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
/// ⚠️ 复用 [`crate::proxy::strip_userinfo`]（sidecar 共享的单一真源），不再本地重复实现。
fn strip_userinfo(url: &Url) -> String {
    crate::proxy::strip_userinfo(url)
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
        // ⚠️⚠️ Android：先等首个文档（启动页）加载完成，再解析 / 推送 / 导航。
        // 不等它就会「导航被 post 出去的 splash 加载覆盖 + 进度 eval 落在 document-start 之前
        // 被静默丢弃」——根因、上游代码位置与真机证据见 [`BootReady`] 的长注释。
        // Windows 不加此等待（WebView2 的注入/导航时序由 wry 自己保证）。
        #[cfg(not(windows))]
        if !wait_first_page_ready(&app, &label) {
            // 拿不到加载事件**本身就是一个可判读的事实**，但绝不阻断启动：继续按原流程走
            //（退化成旧的「不等待」行为），只是把这件事写在卡片上。
            note(
                &app,
                &label,
                &format!(
                    "启动页未在 {}ms 内报告加载完成（页面加载事件缺失），按原流程继续",
                    SPLASH_READY_BUDGET_MS
                ),
            );
        }

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
            prepare_navigation(&app, &label, use_local_proxy)
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
                // 解析成功：缓存导航信息供后续复用。重试场景覆盖旧缓存也是正确的——
                // 重试后端口/地址可能已变，缓存必须跟着更新。多页签模式额外写一份
                // TabManager.navigation（页签新建路径复用）；单窗口模式无后续复用方。
                #[cfg(windows)]
                if cache_navigation && crate::tabs::tabs_enabled() {
                    if let Ok(mut cache) = app.state::<crate::tabs::TabManager>().navigation.lock()
                    {
                        *cache = Some(nav.clone());
                    }
                }
                // 移动端底栏的「首页」按钮：把解析结果缓存下来，点首页时直接复用
                //（不重跑 DNS 解析、不重启本地代理）。见 `crate::mobilebar::HomeNav`。
                #[cfg(mobile)]
                crate::mobilebar::set_home_nav(&app, &nav);
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

/// 启动期「解析目标地址」阶段的**总预算**（秒）。
///
/// 背景（2026-10-09 真机取证）：启动页的兜底卡是 40s（`loading.js` 的 `FAIL_MS`）。域名数组
/// 配置下解析是**串行**遍历域名的，每个域名的 hickory 默认 `attempts=2 × timeout=5s ×
/// 每个 nameserver` ⇒ 3 个域名最坏可到 30s+；一旦某个 nameserver 不可达，启动就会长时间停在
/// "卡住"状态（既无进度也无报错）。取值需**明显小于** 40s（给渲染与推送留时间），同时给弱网
/// 留足余量：整个解析阶段（含多域名 failover）超过它即判定失败，把「静默卡住」变成明确报错。
const RESOLVE_BUDGET_SECS: u64 = 25;

/// 从启动期后台线程往加载层推一行进度。
///
/// ⚠️ `eval` 必须回主线程（Tauri 的 WebView 操作都要求主线程），所以这里统一转一手 ——
/// 调用方（`prepare_navigation` 跑在 `std::thread::spawn` 里）不能直接调 `loading::show_note`。
fn note(app: &tauri::AppHandle, label: &str, line: &str) {
    let a = app.clone();
    let l = label.to_string();
    let s = line.to_string();
    let _ = app.run_on_main_thread(move || crate::loading::show_note(&a, &l, &s));
}

/// 解析真实地址并按需启动本地反向代理（在后台线程执行，可能耗时）。
///
/// `app` / `label` 只用于把各阶段进度推给启动页上的加载层（[`crate::loading::show_note`]）——
/// 启动期一旦偏慢或卡住，卡片上就留有「卡在哪一步」的记录，不必依赖 adb / logcat。
fn prepare_navigation(
    app: &tauri::AppHandle,
    label: &str,
    use_local_proxy: bool,
) -> Result<Navigation, String> {
    // 若 APP_URL 是【域名数组】，则依次对这些域名做 DNS TXT 查询 + Ed25519 验签
    // + AES-GCM 解密得到真实 URL；全部失败时返回可展示的错误文案。
    // ⚠️ `resolve_url_from_domains` 是 async（该模块刻意不依赖 Tauri，见 resolve.rs 头注释），
    // 由这里用 Tauri 的异步运行时阻塞驱动 —— 本函数跑在后台线程，不在 async 上下文内。
    let resolved_url: String = if let Some(domains) = crate::resolve::parse_domains(APP_URL) {
        let domains: Vec<String> = domains
            .into_iter()
            .map(|d| crate::resolve::normalize_domain(&d))
            .collect();
        note(
            app,
            label,
            &format!("正在解析目标地址（{} 个域名）…", domains.len()),
        );
        // ⚠️⚠️ 必须设**总预算**：解析是**串行**遍历域名的，每个域名 hickory 默认
        // `attempts=2 × timeout=5s × 每个 nameserver`，3 个域名最坏可达 30s+ ——
        // 一旦某个 nameserver 不可达，启动就会长时间"卡住"（既无进度也无报错），
        // 用户只看到启动页那张 40s 兜底卡。超时即给出可看懂的失败，而不是一直挂着。
        tauri::async_runtime::block_on(async {
            match tokio::time::timeout(
                std::time::Duration::from_secs(RESOLVE_BUDGET_SECS),
                crate::resolve::resolve_url_from_domains(&domains, PUBLIC_KEY),
            )
            .await
            {
                Ok(r) => r,
                Err(_) => Err(format!(
                    "解析目标地址超过 {} 秒仍未完成（网络受限或 DNS 不可达），请切换网络后重试",
                    RESOLVE_BUDGET_SECS
                )),
            }
        })
        // ⚠️ 不要把具体原因（`_`）丢掉只留一句「服务器连接失败」：启动期故障的具体成因
        //（读系统 DNS 配置失败 / 查询某域名失败 / 全部密文解不开 / 预算超时）是唯一能指导
        // 下一步的信息（项目约定：环境级故障也要可见）。
        .map_err(|e| format!("解析目标地址失败：{e}"))?
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

    // 地址已被接受：下面每一步都推给卡片 —— 启动期一旦偏慢或卡住，卡片上就留有「卡在哪一步」。
    note(
        app,
        label,
        &format!("正在打开 {}…", parsed.host_str().unwrap_or("目标站点")),
    );

    let (url, proxy_abort) = if use_local_proxy {
        note(app, label, "正在启动本地反向代理…");
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
                // ⚠️ `clone()`：`navigate` 拿走所有权，而下面的落点校验还要用同一份目标地址。
                match window.navigate(url.clone()) {
                    Ok(()) => {
                        // ⚠️⚠️ `Ok(())` 在这里**不代表导航成功**：wry 在 Android 上把
                        // `load_url` 实现成「投一条 main-pipe 消息后直接 `Ok(())`」
                        //（`wry/src/android/mod.rs`），且 `RustWebViewClient.onReceivedError`
                        // 只对 `ERROR_CONNECT` 做协议重定向特例、**不向壳层透出任何网络错误**
                        // ⇒ 主框架加载失败时壳侧完全无从得知，WebView 会把启动页留在屏幕上，
                        // 最终只剩加载层那张 40s 兜底卡（真机取证：2026-10-09）。所以这里
                        // ①必须留下「已发起导航」的痕迹 —— 这是区分「根本没导航」与
                        // 「导航了但没提交」的唯一线索；②并发一次可达性探测，把结论写回卡片。
                        note(app, label, "已发起导航，等待页面响应…");
                        // 探测本身：Android WebView 网络失败时停在「空白错误文档」（桌面端浏览器
                        // 那种带错误码的页面在 WebView 上不渲染），仅靠加载层固定超时只能给通用
                        // 文案；探测可在数秒内把 DNS / 连接 / 超时 / TLS 的具体原因推给失败卡。
                        spawn_reachability_probe(app.clone(), label, nav.url.clone());
                        // 落点校验：探测管「网络通不通」，这里管「导航到底有没有生效」。
                        // 两者互补 —— 真机上这两件事曾经都只能靠猜。见 `ensure_navigation_committed`。
                        ensure_navigation_committed(app, label, &url);
                    }
                    Err(e) => crate::loading::show_error(
                        app,
                        label,
                        "加载失败",
                        &format!("无法打开页面：{e}"),
                    ),
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

// ===========================================================================
// 非 Windows（Android 等）导航后的可达性探测
// ===========================================================================
//
// 为什么需要：Android WebView 加载失败（DNS 失败 / 连接被拒 / 超时 / TLS 握手失败）时
// 既不触发应用可订阅的错误回调，也不像桌面端 Chromium 那样画出带错误码的页面 ——
// 文档停在一个【空白错误文档】上，用户只能看到加载层转十几秒后淡出成纯白。
// 导航提交后并发一次 Rust 侧轻量 HTTP 探测（reqwest + rustls 是非 Windows 既有的
// 代理依赖，零新增 crate），在数秒内判定失败类别，把可看懂的原因推给加载层失败卡。
//
// 设计要点：
//   · 探测与导航【并发】，不阻塞、不延迟真实页面加载；
//   · 收到任意 HTTP 响应（含 4xx/5xx）即视为传输层可达 —— 白屏与网络无关，不插手；
//   · 推给加载层时带 soft 标记（loading.js `fail(t,d,soft)`）：页面若已渲染出正文，
//     探测结论让位（防止企业根证书等场景下 rustls 与系统信任库判定不一致的假阴性）。

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
/// 状态码。
#[cfg(any(not(windows), test))]
fn probe_ok_message(host: &str, status: Option<u16>, ms: u128) -> String {
    let st = match status {
        Some(code) => format!("HTTP {code}"),
        None => "已收到响应".to_string(),
    };
    format!(
        "可达性探测：{host} 已连通（{st}，{ms}ms），但页面未渲染 —— \
         问题多半在本机 WebView 内核（版本过旧 / TLS 协商 / 渲染），不在网络。\
         可先升级系统「Android System WebView」，或换一台设备比对同一网址。"
    )
}

/// 导航提交后发起可达性探测；把结论（**成功 / 失败分类**）在主线程 eval 给加载层卡片。
///
/// ⚠️ 成败**都要报**：成功时给一行「已连通但未渲染」的结论（见 [`probe_ok_message`]），失败时
/// 按类别给出带主机的具体原因（见 [`probe_failure_message`]）。只报失败会让「壳内不渲染」这类
/// 故障只剩一张零信息的兜底卡 —— 而那恰恰是最需要判据的情形。
///
/// `url` 即 WebView 实际导航的地址。⚠️ 已知局限：代理模式下它是 `127.0.0.1:<port>`，探到的是
/// **本地代理自身**（只能证明代理进程活着，证明不了上游），要修需改成探测真实上游。
/// `AppHandle` 按值传入：探测跑在 `'static` 的 Tokio 任务里，不能借用调用方的引用。
#[cfg(not(windows))]
fn spawn_reachability_probe(app: tauri::AppHandle, label: &str, url: String) {
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
                let line = probe_ok_message(&host, status, ms);
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
        let m = probe_ok_message("ai260926.osfipin.com", Some(200), 812);
        assert!(m.contains("ai260926.osfipin.com"), "应含主机名：{m}");
        assert!(m.contains("HTTP 200"), "应含状态码：{m}");
        assert!(m.contains("812ms"), "应含耗时：{m}");
        assert!(
            m.contains("WebView"),
            "结论必须指向「本机 WebView 内核」，否则用户会继续往网络上找：{m}"
        );
        // 老内核 / 重定向后拿不到状态码时，不得把 Option 的 Debug 形态漏到用户文案里。
        let m2 = probe_ok_message("h.example", None, 5);
        assert!(m2.contains("已收到响应"), "无状态码时应给中性表述：{m2}");
        assert!(
            !m2.contains("None"),
            "不得把 Option 的 Debug 形态漏进用户文案：{m2}"
        );
    }
}
