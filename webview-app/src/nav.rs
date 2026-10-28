//! 导航 / 窗口会话登记数据枢纽（架构审查 2026-10-25 P2-1）。
//!
//! 收敛 `bootstrap ↔ {tabs, loading, mobilebar, webview_hooks}` 双向耦合后抽出的**数据枢纽**：
//! 启动期解析出来的导航会话 [`Navigation`]、本地反向代理任务句柄 [`ProxyTask`]、首文档就绪
//! 登记 [`BootReady`]、窗口 label 登记（[`HOST_LABEL`] / [`main_window_label`]）与各模块共用
//! 的导航原语（[`origin_of`] / [`is_upstream_request`] / [`local_proxy_enabled`] /
//! [`navigate_window`]）统一放在这里，功能模块按「→ nav / config」的方向取用。
//!
//! 依赖方向（P2-1 的目标形态）：
//!   · `bootstrap` → `nav` / 功能模块 / `config`（编排层向下看）；
//!   · `nav` → `config` / `store`（**只**向下看，不回调任何功能模块）；
//!   · 功能模块（tabs / loading / mobilebar / webview_hooks / settings / debug_log …）→ `nav` / `config`。
//!
//! ⚠️⚠️ 为什么「重走启动 / 挂 WebView2 钩子」是**回调**而不是直接调用（最小控制反转的理由）：
//! 这两个动作的实现在上层（`bootstrap::boot_entry` / `webview_hooks::attach_webview_hooks`），
//! 若在本模块直接调用，`nav → bootstrap`、`nav → webview_hooks` 会与既有的 `bootstrap → nav`、
//! `webview_hooks → nav` 构成新的层级环 —— 双向耦合只是换了个名字，正是本次收敛要消灭的形态。
//! 所以改成 fn 指针，由 `lib.rs` 装配时登记（装配层是唯一允许「认识所有人」的地方）：
//!   · [`register_boot_entry`]：登记启动编排入口；[`bootstrap`] / [`bootstrap_for_label`] 经它
//!     转发，mobilebar（首页 / 新建标签）与 tabs / loading（重试）因此只依赖本模块；
//!   · [`register_attach_hooks`]（仅 Windows）：登记 WebView2 钩子装配；[`navigate_window`]
//!     先经它挂钩子、再真正导航。
//! 移动端的 `mobilebar::on_page_finished` 同理由 `lib.rs` 的 `on_page_load` 闭包直接接线
//! （不进本模块），与 [`on_page_load`] 的就绪登记保持原有先后顺序：先置位、后登记。

use std::sync::OnceLock;

#[cfg(not(windows))]
use tauri::Manager;
use url::Url;

#[cfg(windows)]
use crate::config::build_proxy_args;
use crate::config::PROXY_ENABLED;

/// 窗口/WebView 的默认底色，与加载层（src/loading.js）的浅色底一致。
/// 用于盖住 WebView 初始化与导航提交瞬间的白闪，使"启动页 → 加载层 → 正式页面"全程平滑。
pub(crate) const SPLASH_BG: tauri::window::Color = tauri::window::Color(0xf7, 0xf8, 0xfa, 255);

/// 启动期在后台线程准备出来的导航信息。
///
/// `Clone`：多页签模式下，首次启动解析出的导航信息会被缓存到 `TabManager.navigation`，
/// 新建 tab 时克隆复用（本地代理单实例共享，`proxy_abort` 仍由 `ProxyTask` 唯一持有，
/// 此处克隆出的句柄仅作导航用途、不重复入库，见 [`navigate_window`]）。
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

    /// 是否已就绪（`bootstrap::wait_first_page_ready` 的轮询判据）。
    pub(crate) fn is_ready(&self, label: &str) -> bool {
        self.0.lock().map(|g| g.contains(label)).unwrap_or(false)
    }
}

/// 页面加载事件的落点（由建窗时注册的 `on_page_load` 闭包调用；非 Windows 才有意义）。
///
/// wry Android 的 `ON_LOAD_HANDLER` 由 `RustWebViewClient.onPageStarted / onPageFinished`
/// 驱动（`tauri-runtime-wry` 里是**无条件** `with_on_page_load_handler`），所以 Android 上这个
/// 信号真实可用 —— 这是本项目在 Android 侧唯一可靠的「文档已就绪」判据（轮询 URL 不行：
/// `GetUrl` 同样走 main-pipe，webview 未就绪时拿不到结果）。
///
/// ⚠️ 移动端的 `mobilebar::on_page_finished`（把最终 url 记进活动标签）**不在这里调**：
/// 那会造出 `nav → mobilebar` 的层级环（mobilebar 依赖本模块的 `Navigation` / [`bootstrap`]）。
/// 改由 `lib.rs` 的 `on_page_load` 闭包在本函数**之后**直接接线，先后顺序与旧实现一致。
#[cfg(not(windows))]
pub(crate) fn on_page_load(app: &tauri::AppHandle, label: &str, finished: bool) {
    let ready = app.state::<BootReady>();
    if finished {
        ready.mark(label);
    } else {
        ready.clear(label);
    }
}

/// 宿主窗口（多页签标签栏所在）的固定 label（`tabs.rs` 经 `pub(crate) use` 重导出，
/// 保持既有 `tabs::HOST_LABEL` 引用路径不变）。
///
/// ⚠️ 仅 Windows 多页签引用（`main_window_label` 的 `cfg(windows)` 分支 + `tabs.rs`）；
/// Android 无 tabs，`main_window_label` 恒返回 `"main"`。
#[cfg(windows)]
pub(crate) const HOST_LABEL: &str = "host";

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
///
/// ⚠️ 多页签判据读 `config::TABS_ENABLED`（= `tabs::tabs_enabled()` 的同一常量）而不去调
/// `tabs` —— 否则 `nav → tabs` 与 `tabs → nav` 就成了新的层级环（P2-1 要消灭的形态）。
pub(crate) fn main_window_label() -> &'static str {
    #[cfg(windows)]
    {
        if crate::config::TABS_ENABLED {
            return HOST_LABEL;
        }
    }
    "main"
}

/// 是否启用【本地反向代理】。
///
/// Windows：需“启用代理且没有配置外部代理”（有外部代理时走 `--proxy-server`）。
/// 移动端 / 其它平台：不支持 `--proxy-server`，因此只要“启用代理”就一律走本地反向代理。
/// 由编译期常量决定，纯计算、零开销；setup（建窗参数）与启动编排（后台准备）共用。
#[cfg(windows)]
pub(crate) fn local_proxy_enabled() -> bool {
    PROXY_ENABLED && build_proxy_args().is_none()
}
#[cfg(not(windows))]
pub(crate) fn local_proxy_enabled() -> bool {
    PROXY_ENABLED
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

/// 启动编排入口的签名：`label = None` = 单窗口模式（目标窗 `"main"`）；
/// `Some(label)` = 导航到指定窗口（多页签首 tab / splash `/retry` 传入）。
pub(crate) type BootEntry = fn(tauri::AppHandle, Option<String>);

/// 启动编排入口（`bootstrap::boot_entry`），由 `lib.rs` 装配时经 [`register_boot_entry`] 登记。
static BOOT_ENTRY: OnceLock<BootEntry> = OnceLock::new();

/// 登记启动编排入口（控制反转：本模块不直接调 `bootstrap`，理由见模块头）。
/// 幂等：重复登记忽略（只认第一次）。
pub(crate) fn register_boot_entry(entry: BootEntry) {
    let _ = BOOT_ENTRY.set(entry);
}

/// 启动 / 重试流程入口：单窗口模式（目标窗 `"main"`）。
/// 实现在 `bootstrap::boot_entry`（启动编排），这里只经登记的回调转发。
pub(crate) fn bootstrap(app: tauri::AppHandle) {
    run_boot(app, None);
}

/// 启动 / 重试流程入口，参数化目标窗口 label（多页签首 tab / splash `/retry`）。
/// ⚠️ 与 [`bootstrap`] 一样**必须缓存**解析结果（`cache_navigation = true`）——
/// 缓存纪律由 `bootstrap::boot_entry` 那一侧保证，理由见它的文档。
pub(crate) fn bootstrap_for_label(app: tauri::AppHandle, label: String) {
    run_boot(app, Some(label));
}

fn run_boot(app: tauri::AppHandle, label: Option<String>) {
    let Some(entry) = BOOT_ENTRY.get() else {
        // 接线缺失是装配层 bug（lib.rs setup 必登记），但**不能 panic**（本函数跑在事件回调里）：
        // 留一条可判读的日志、静默返回，别把用户正用着的壳打崩。
        dlog!(
            "[nav] 启动编排入口未接线（lib.rs setup 缺 register_boot_entry），本次启动流程被跳过"
        );
        return;
    };
    entry(app, label);
}

/// WebView2 钩子装配回调的签名（仅 Windows）：`(app, window, user, pass, use_local_proxy, upstream_origin)`。
#[cfg(windows)]
pub(crate) type AttachHooks =
    fn(tauri::AppHandle, &tauri::WebviewWindow, String, String, bool, Option<String>);

/// WebView2 钩子装配（`webview_hooks::attach_webview_hooks`），由 `lib.rs` 装配时登记。
#[cfg(windows)]
static ATTACH_HOOKS: OnceLock<AttachHooks> = OnceLock::new();

/// 登记 WebView2 钩子装配（控制反转：本模块不直接调 `webview_hooks`，理由见模块头）。
/// 幂等：重复登记忽略（只认第一次）。
#[cfg(windows)]
pub(crate) fn register_attach_hooks(attach: AttachHooks) {
    let _ = ATTACH_HOOKS.set(attach);
}

/// 对给定的窗口执行导航（挂 WebView2 钩子 + 导航）。
///
/// 与 `bootstrap` 里首次导航的区别：不负责代理句柄入库（复用缓存时句柄已入过库），
/// 供多页签模式新建 tab 时复用缓存导航信息。仅 Windows 有意义（tabs 模块整体 cfg(windows)）。
///
/// 1) 钩子装配经 [`register_attach_hooks`] 登记的回调完成（不负责 Navigate）：
///    必须**先**挂钩子再导航，否则窗口首批请求会漏掉 Authorization 注入；
/// 2) 用 Tauri 的 `WebviewWindow::navigate` 真正导航——wry 内部会等 CoreWebView2
///    controller 就绪再触发 WebView2 的 Navigation。
///    历史实现直接在 `attach_webview_hooks` 末尾调 `core.Navigate`，但此时 controller
///    可能未就绪（多页签模式新建 tab 时由 Rust 同步发起，与 wry 的就绪回调有时序差），
///    `with_webview` 直接 return → 钩子没注册、Navigate 没触发，tab 停在 builder 默认入口
///    —— 也就是用户看到的『弹窗新 tab 永远停在 app_url』。
#[cfg(windows)]
pub(crate) fn navigate_window(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    nav: Navigation,
) {
    match ATTACH_HOOKS.get() {
        Some(attach) => attach(
            app.clone(),
            window,
            nav.user,
            nav.pass,
            nav.use_local_proxy,
            nav.upstream_origin,
        ),
        None => dlog!(
            "[nav] WebView2 钩子装配未接线（lib.rs setup 缺 register_attach_hooks），本次导航不带鉴权钩子"
        ),
    }
    if let Ok(parsed) = Url::parse(&nav.url) {
        let _ = window.navigate(parsed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
