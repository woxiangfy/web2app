//! 多页签（tab）模式（仅 Windows）。
//!
//! 需求定位：**同账号、共享数据**的多页签——一个宿主窗口 + 顶部标签栏，每个 tab 是
//! 一个独立的 WebviewWindow（共享同一个 WebView2 数据目录，cookie/localStorage 天然互通）。
//! tab 可随时新建 / 切换 / 关闭；`target="_blank"` / `window.open()` 直接新开一个 tab。
//!
//! 技术要点（这是「多窗口伪装成 tab」，而非浏览器原生 tab 抽象）：
//! - Tauri/Wry 无原生 tab，一个 WebviewWindow = 一个 WebView。tab 窗口去边框（`.decorations(false)`）、
//!   坐标吸附在宿主窗口标签栏下方；宿主窗口拖动 / 缩放时，所有 tab 窗口批量跟随。
//! - 标签栏由「宿主窗口」渲染（`shell://localhost/` 自定义协议，前端 HTML），tab 的增删切换通过
//!   IPC 命令回调 Rust；宿主窗口本身不承载网页内容，只画标签栏。
//! - 本地反向代理保持**单实例单端口**（所有 tab 访问同一上游 + 同一账号），`ProxyTask` 无需改动。
//! - 首个 tab 的域名解析 / 代理启动完成后，结果缓存到 `TabManager`，后续新建 tab 直接复用，
//!   不再重复 DNS TXT 查询。
//!
//! 本模块整体 `#[cfg(windows)]`：多页签能力仅在 Windows 实现（Android 无多窗口叠放 + 共享
//! WebView2 数据目录的对应能力）。

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::{Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::bootstrap::Navigation;
use crate::config::{TABS_ENABLED, WINDOW_TITLE};

/// 宿主窗口（标签栏所在）与 tab 窗口的固定 label 前缀。
pub(crate) const HOST_LABEL: &str = "host";
pub(crate) const TAB_PREFIX: &str = "tab-";

/// tab 窗口当前是否「因宿主最小化 / 隐藏而被收起」。
///
/// 用途：宿主最小化时 tab 窗口不会跟着最小化（它们是独立顶层窗口），必须手动 `hide()`，
/// 否则桌面/任务栏上会残留没有标签栏的内容窗口。宿主恢复时再 `show()` 回来。
///
/// 为什么需要这个标志：宿主**每次** `Resized`（拖拽缩放）都会走一遍同步逻辑，
/// 若不加状态判断，就会在缩放过程中反复 `show/hide` 所有 tab 窗口 → 肉眼可见闪烁。
/// 只在「收起/还原」这个状态真正翻转时才动窗口。
static TABS_SUSPENDED: AtomicBool = AtomicBool::new(false);

/// shell 自定义协议名（宿主窗口加载的标签栏页）。
pub(crate) const SHELL_SCHEME: &str = "shell";

/// 标签栏高度（**逻辑**像素）。该值与 shell.html 里的标签栏 CSS 高度保持一致，
/// 否则 tab 内容会与标签栏重叠或留缝。relayout 计算物理像素时再乘 DPI 缩放系数。
pub(crate) const TAB_BAR_HEIGHT: i32 = 38;

/// 一个 tab 的运行时信息。
#[derive(Clone)]
pub(crate) struct TabInfo {
    /// 窗口 label（`tab-0` / `tab-1` …）。
    pub label: String,
    /// 展示标题（优先取网页真实 title，缺省用应用显示名）。
    pub title: String,
}

/// 多页签的托管状态。
///
/// `navigation` 缓存首次启动解析出的导航信息（真实 URL / 账号密码 / 是否走本地代理），
/// 新建 tab 时复用，避免每个 tab 都重复 DNS 解析 + 重启本地代理。仅在「首个 tab 就绪」后写入。
#[derive(Default)]
pub(crate) struct TabManager {
    /// 当前所有存活的 tab（按创建顺序）。
    pub tabs: Mutex<Vec<TabInfo>>,
    /// 当前激活（前台显示）的 tab 索引。
    pub active: Mutex<usize>,
    /// tab label 自增计数器（用于生成唯一 label）。
    pub counter: Mutex<usize>,
    /// 首次启动解析好的导航信息缓存（新 tab 复用）。`Option` 表示「尚未就绪」。
    pub navigation: Mutex<Option<Navigation>>,
    /// 每 tab 的实际网页标题缓存：label -> title（由 `get_tab_title` 命令回传）。
    pub titles: Mutex<HashMap<String, String>>,
}

/// 判断多页签能力是否启用。纯计算，供 lib.rs / webview_hooks 在编译期分支复用。
pub(crate) fn tabs_enabled() -> bool {
    TABS_ENABLED
}

/// 生成下一个 tab 的 label（`tab-0` / `tab-1` …）。
fn next_tab_label(app: &tauri::AppHandle) -> String {
    let state = app.state::<TabManager>();
    let mut counter = state.counter.lock().unwrap_or_else(|p| p.into_inner());
    let n = *counter;
    *counter += 1;
    format!("{TAB_PREFIX}{n}")
}

/// 宿主窗口的标签栏页地址。
pub(crate) fn shell_url() -> WebviewUrl {
    let url = url::Url::parse("shell://localhost/").expect("构造 shell 协议 URL 失败");
    WebviewUrl::CustomProtocol(url)
}

/// shell 自定义协议处理器：返回内嵌的标签栏 HTML（前端）。
///
/// 与 settings:// 同理，编译期内嵌，不依赖 devUrl，dev / release 行为一致。
pub(crate) fn handle_shell(
    _ctx: tauri::UriSchemeContext<'_, tauri::Wry>,
    _req: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    let html = include_str!("shell.html");
    tauri::http::Response::builder()
        .header("Content-Type", "text/html; charset=utf-8")
        .body(html.as_bytes().to_vec())
        .unwrap()
}

/// 宿主窗口「内容区」（标签栏下方）的物理像素矩形：左上角 `(x, y)` 与尺寸 `(w, h)`。
///
/// ⚠️ 必须用**客户区**（inner）几何，而不是窗口矩形（outer）：宿主窗口虽然 `decorations(false)`，
/// 但 `resizable(true)` 时 Windows 仍保留不可见的调整边框（WS_THICKFRAME，高分屏下每边 8~16 物理
/// 像素）。历史的 `outer_position/outer_size` 算法让 tab 窗口比宿主客户区**更宽、更靠下** →
/// 内容右/下溢出宿主边框，露在桌面或任务栏上（用户反馈的“两个窗体不一致”）。
fn host_content_rect(app: &tauri::AppHandle) -> Option<(i32, i32, u32, u32)> {
    let host = app.get_webview_window(HOST_LABEL)?;
    // 最小化时 Windows 会把客户区尺寸报成 0（或极小的最小化尺寸），此时不能拿它去
    // `set_size` —— 否则 tab 窗口会被压成一条缝，恢复后位置/尺寸全乱。
    if host.is_minimized().unwrap_or(false) {
        return None;
    }
    let pos = host.inner_position().ok()?;
    let size = host.inner_size().ok()?;
    // shell.html 的标签栏是 38 **逻辑**像素；客户区坐标/尺寸是物理像素 → 按 DPI 换算再相减。
    // 用 floor 而不是 round：分数缩放（如 125% → 47.5）时向下取整让 tab 窗口与标签栏**重叠
    // 半个像素**，而不是留出半像素缝隙（缝里会露出宿主窗口的浅色底，形成一条白线）。
    let scale = host.scale_factor().unwrap_or(1.0);
    let bar_phys = ((TAB_BAR_HEIGHT as f64) * scale).floor() as i32;
    let content_top = pos.y + bar_phys;
    let content_height = (size.height as i32 - bar_phys).max(0) as u32;
    Some((pos.x, content_top, size.width, content_height))
}

/// tab 窗口「窗口矩形」相对「客户区」的偏移 `(dx, dy)`，即不可见边框的厚度。
///
/// `set_position` 设置的是**窗口矩形**位置（tao `set_outer_position`），而我们要把 tab 的
/// **客户区**对齐到宿主内容区，故需减去该偏移。无边框无阴影窗口为 (0, 0)，带阴影的无边框
/// 窗口在 Windows 上非 0（tao 的 `set_size` 走 `set_inner_size`，已自行补偿尺寸方向的偏移）。
fn frame_offset(window: &WebviewWindow) -> (i32, i32) {
    match (window.outer_position(), window.inner_position()) {
        (Ok(outer), Ok(inner)) => (outer.x - inner.x, outer.y - inner.y),
        _ => (0, 0),
    }
}

/// 取当前在册 tab 的 label 快照（**调用期间不持锁**，避免锁跨 FFI 调用）。
fn tab_labels(app: &tauri::AppHandle) -> Vec<String> {
    let state = app.state::<TabManager>();
    let tabs = state.tabs.lock().unwrap_or_else(|p| p.into_inner());
    tabs.iter().map(|t| t.label.clone()).collect()
}

/// 把某个 label 从在册列表与标题缓存中移除（窗口销毁由调用方负责）。
fn unregister_tab(app: &tauri::AppHandle, label: &str) {
    let state = app.state::<TabManager>();
    {
        let mut tabs = state.tabs.lock().unwrap_or_else(|p| p.into_inner());
        tabs.retain(|t| t.label != label);
    }
    state
        .titles
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(label);
}

/// 清理「幽灵 tab 窗口」：label 以 `tab-` 开头、但已不在在册列表中的窗口一律强制销毁。
///
/// 这类窗口来自历史 bug（关闭被 `CloseRequested` 拦截后未销毁），会残留成“第二个窗体”：
/// 既不受 [`relayout`] 管理（尺寸与宿主不一致），又占着一个 WebView2 渲染进程。
/// 作为自愈手段，在激活 / 关闭 tab 时顺手清理。
fn purge_orphan_tabs(app: &tauri::AppHandle) {
    let tracked: HashSet<String> = tab_labels(app).into_iter().collect();
    for (label, window) in app.webview_windows() {
        if label.starts_with(TAB_PREFIX) && !tracked.contains(&label) {
            // destroy 是强制销毁（不派发 CloseRequested），不会与关闭事件处理互相重入。
            let _ = window.destroy();
        }
    }
}

/// 在指定 tab 窗口内打开（导航到）目标地址。
///
/// 新 tab 建窗后调用：复用 `TabManager.navigation` 缓存（首次启动已解析好），
/// 避免每个 tab 重复 DNS 解析 + 重启本地代理。缓存不存在（尚未就绪）时回落到
/// 完整 bootstrap 流程。
///
/// `start`：`None` = 用缓存的入口地址；`Some(url)` = 用该地址替换入口地址做**一次**导航
/// （`target="_blank"` 弹窗的落点，见 [`open_new_tab_from_request`]）。历史实现是“先建窗导航
/// 到主地址，再发起第二次导航改到目标地址”，既多一次请求，也存在与首次导航的竞态。
fn navigate_tab(app: &tauri::AppHandle, label: &str, start: Option<String>) {
    let nav = app
        .state::<TabManager>()
        .navigation
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();

    match nav {
        Some(mut nav) => {
            if let Some(url) = start {
                nav.url = url;
            }
            // 缓存命中：直接导航。代理句柄是共享的（单实例），不需要再入库，
            // 因为 ProxyTask 已由首次 bootstrap 持有；这里只做导航 + 挂 WebView2 钩子。
            let Some(window) = app.get_webview_window(label) else {
                return;
            };
            crate::bootstrap::navigate_window(app, &window, nav);
        }
        None => {
            // 缓存未就绪（理论上只在极早期触发），走完整 bootstrap（内部会重新解析并导航到该 tab）。
            crate::bootstrap::bootstrap_for_label(app.clone(), label.to_string());
        }
    }
}

/// 创建一个新 tab：建窗（去边框、吸附定位）+ 导航 + 挂 WebView2 钩子 + 激活。
///
/// 返回新 tab 的 label，供前端标签栏渲染。
pub(crate) fn spawn_tab(app: &tauri::AppHandle) -> Result<String, String> {
    spawn_tab_with_target(app, None)
}

/// 创建新 tab 并指定其初始导航地址。
///
/// `start = Some(url)` 时用该地址替换入口地址做一次导航（`target="_blank"` 弹窗的落点），
/// 见 [`navigate_tab`]；`None` 时导航到缓存的入口地址。
pub(crate) fn spawn_tab_with_target(
    app: &tauri::AppHandle,
    start: Option<String>,
) -> Result<String, String> {
    let label = next_tab_label(app);

    // ⚠️ 先登记（占位）再建窗：`purge_orphan_tabs` 会销毁“label 不在册”的 tab 窗口，若先建窗，
    // 并发（用户连点“+”）时新窗口可能被另一个线程的清理误杀。建窗失败再回滚登记。
    {
        let state = app.state::<TabManager>();
        let mut tabs = state.tabs.lock().unwrap_or_else(|p| p.into_inner());
        tabs.push(TabInfo {
            label: label.clone(),
            title: WINDOW_TITLE.to_string(),
        });
    }
    // tab 窗口：去边框 + 不占任务栏按钮，尺寸/位置按宿主客户区一次到位。
    // ⚠️ 建窗时先 `visible(false)`：否则会以框架默认尺寸（800×600）闪一帧再被 relayout 纠正。
    // 首次导航照样发起（页面在不可见状态下开始加载），最后由 activate_tab 显示。
    let mut window = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
        .title(WINDOW_TITLE)
        .decorations(false)
        // ⚠️ 标签不占独立任务栏按钮：只有宿主窗口出现在任务栏，视觉上是一个整体窗口。
        // （若任由每个 tab 出任务栏按钮，会看到“多个相同图标”重复，用户侧反馈的“有两个窗口”。）
        .skip_taskbar(true)
        .resizable(false)
        .visible(false)
        .background_color(crate::bootstrap::SPLASH_BG)
        .initialization_script(crate::loading::OVERLAY_SCRIPT)
        .initialization_script(format!("window.web2appExtra = {};", crate::config::EXTRA))
        // 注入当前 tab 的 label，供标题回传脚本（tab_title.js）读取后回传真实网页标题。
        .initialization_script(format!(
            "window.__web2appTabLabel = {};",
            crate::loading::js_str(&label)
        ))
        // 标题回传：监听 document.title 变化 → invoke("set_tab_title") → 标签栏显示真实标题。
        .initialization_script(include_str!("tab_title.js"));

    // 建窗即给出与宿主内容区一致的初始**客户区**尺寸（逻辑像素；外框偏移由 tao 的
    // set_inner_size 自行补偿，位置由 relayout 负责），避免首帧尺寸错误。
    if let Some((_, _, width, height)) = host_content_rect(app) {
        if let Some(scale) = app
            .get_webview_window(HOST_LABEL)
            .and_then(|host| host.scale_factor().ok())
        {
            window = window.inner_size(width as f64 / scale, height as f64 / scale);
        }
    }

    // Windows 专属：禁用拖放 handler + 注入拖拽 polyfill（与主窗口一致）。
    #[cfg(windows)]
    let window = window
        .disable_drag_drop_handler()
        .initialization_script(include_str!("dnd_polyfill.js"));

    // 数据目录：与主窗口共享同一 WebView2 数据目录（同账号、共享 cookie/localStorage）。
    #[cfg(windows)]
    let window = if !crate::config::DATA_DIR_DEFAULT {
        match app.path().app_local_data_dir() {
            Ok(dir) => {
                let base = dir.parent().unwrap_or(&dir);
                let new_dir = base.join(crate::config::sanitize_dir_name(crate::config::APP_NAME));
                window.data_directory(new_dir)
            }
            Err(_) => window,
        }
    } else {
        window
    };

    // 建窗后无需持有句柄——后续导航 / 布局通过 get_webview_window(label) 重新获取。
    // 建窗失败：回滚上面写入的占位登记，保持列表与真实窗口一致。
    let _window = match window.build() {
        Ok(w) => w,
        Err(e) => {
            unregister_tab(app, &label);
            return Err(format!("创建 tab 窗口失败：{e}"));
        }
    };

    // 导航（复用缓存 + 可选的弹窗落点）+ 挂 WebView2 钩子。
    navigate_tab(app, &label, start);

    // 激活新 tab（切到前台 + 重新布局）。
    activate_tab(app, &label);

    Ok(label)
}

/// 激活指定 tab：显示目标 tab、隐藏其余，并把标签栏状态同步给前端。
fn activate_tab(app: &tauri::AppHandle, label: &str) {
    // ① 按 label 定位索引 + 快照 label 列表。**所有 MutexGuard 都在此块内释放**，
    //    后续窗口调用（show/hide/set_focus/relayout）不持任何锁 —— 避免锁跨 FFI/COM 调用。
    let (idx, labels) = {
        let state = app.state::<TabManager>();
        let tabs = state.tabs.lock().unwrap_or_else(|p| p.into_inner());
        let idx = tabs.iter().position(|t| t.label == label);
        let labels = tabs.iter().map(|t| t.label.clone()).collect::<Vec<_>>();
        (idx, labels)
    };

    // label 不在册（已被关闭 / 幽灵窗口）：保持现有界面不动，只做自愈清理。
    // （绝不能让 active 索引悬空——那会让“没有任何 tab 被显示”，界面看起来就是卡死白屏。）
    let Some(idx) = idx else {
        purge_orphan_tabs(app);
        push_tabs_to_shell(app);
        return;
    };

    {
        let state = app.state::<TabManager>();
        let mut active = state.active.lock().unwrap_or_else(|p| p.into_inner());
        *active = idx;
    }

    // ② 先布局再显隐：避免“尺寸尚未确定就被显示”的一帧错位。
    relayout(app);

    for (i, l) in labels.iter().enumerate() {
        if let Some(w) = app.get_webview_window(l) {
            if i == idx {
                let _ = w.show();
                let _ = w.set_focus();
            } else {
                let _ = w.hide();
            }
        }
    }

    // ③ 顺手清掉不在册的 tab 窗口（历史遗留幽灵窗口 / 关闭被拦截的残骸）。
    purge_orphan_tabs(app);

    // 通知前端标签栏刷新激活态。
    push_tabs_to_shell(app);
}

/// 关闭指定 tab：销毁窗口、移除记录，若关闭的是激活 tab 则激活相邻 tab。
///
/// 关闭【最后一个】tab 等价于关闭宿主窗口（复用既有托盘/退出语义，见 [`close_host_window`]）。
///
/// ⚠️ 三次踩坑都要靠本函数的顺序与 `destroy()` 兜住：
///   1) 窗口必须用 `destroy()`（强制销毁、不派发 `CloseRequested`）。历史实现用 `close()`，
///      而 lib.rs 的 `CloseRequested` 处理会 `prevent_close()` 后再回调本函数 —— 于是
///      “close → CloseRequested → close_tab_inner → close …” 无限循环，主线程 100% 占用、
///      界面完全无响应（用户反馈的“关掉一个标签后整个软件卡死”）。
///   2) 必须先判断 label 是否在册：不在册说明是上一条链路的重复进入，直接返回，
///      不能再走一遍销毁/激活逻辑。
///   3) 窗口销毁要在**更新列表之前**发起，保证“界面看到的 tab 数”与“实际窗口”一致。
pub(crate) fn close_tab_inner(app: &tauri::AppHandle, label: &str) {
    // ① 摘掉登记（若存在），并快照剩余 label。
    let (removed_idx, remaining) = {
        let state = app.state::<TabManager>();
        let mut tabs = state.tabs.lock().unwrap_or_else(|p| p.into_inner());
        let idx = tabs.iter().position(|t| t.label == label);
        if let Some(i) = idx {
            tabs.remove(i);
        }
        let remaining = tabs.iter().map(|t| t.label.clone()).collect::<Vec<_>>();
        (idx, remaining)
    };

    // ② 强制销毁窗口（async 投递，不会在事件回调栈里同步销毁）。不在册也照销毁，
    //    确保任何“幽灵窗口”都被清掉。
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.destroy();
    }
    unregister_tab(app, label);

    // ③ 重复进入（label 早已不在册）：到此为止，绝不能再走后面的激活流程。
    let Some(removed_idx) = removed_idx else {
        purge_orphan_tabs(app);
        push_tabs_to_shell(app);
        return;
    };

    // ④ 关闭的是最后一个 tab：交给宿主窗口的关闭语义（托盘最小化 / 退出）。
    if remaining.is_empty() {
        close_host_window(app);
        return;
    }

    // ⑤ 更新激活索引并激活相邻 tab（优先右侧，否则左侧）。
    let active = {
        let state = app.state::<TabManager>();
        let guard = state.active.lock().unwrap_or_else(|p| p.into_inner());
        *guard
    };
    if removed_idx == active {
        let new_idx = removed_idx.min(remaining.len() - 1);
        let new_label = remaining[new_idx].clone();
        activate_tab(app, &new_label);
    } else {
        if removed_idx < active {
            // 关闭的是激活 tab 之前的 tab：激活索引前移（并夹到合法范围，防越界悬空）。
            let state = app.state::<TabManager>();
            let mut a = state.active.lock().unwrap_or_else(|p| p.into_inner());
            *a = (*a - 1).min(remaining.len() - 1);
        }
        purge_orphan_tabs(app);
        push_tabs_to_shell(app);
    }
}

/// 请求关闭宿主窗口：走 `CloseRequested` → [`crate::handle_main_close`] 的统一语义
/// （启用托盘 + 关闭最小化 → 隐藏到托盘；否则退出应用）。
fn close_host_window(app: &tauri::AppHandle) {
    if let Some(host) = app.get_webview_window(HOST_LABEL) {
        let _ = host.close();
    }
}

/// 强制销毁所有 tab 窗口（宿主窗口确定关闭、且**不**隐藏到托盘时调用）。
///
/// 为什么必须显式销毁：tab 是独立顶层窗口，不是宿主的子窗口，宿主销毁不会带走它们。
/// 不清理就会出现「点了关闭 → 标签栏窗口没了 → 内容窗口还留在桌面上，且应用不退出」
/// （Tauri 要等**所有**窗口关闭才派发 `ExitRequested`）。
pub(crate) fn destroy_all_tabs(app: &tauri::AppHandle) {
    for label in tab_labels(app) {
        if let Some(w) = app.get_webview_window(&label) {
            // 用 destroy 而非 close：destroy 强制销毁且不派发 CloseRequested，
            // 避免再次进入 lib.rs 的关闭分派（历史卡死 bug 就是这条重入链）。
            let _ = w.destroy();
        }
    }
}

/// 批量显隐所有 tab 窗口（宿主窗口隐藏到托盘 / 恢复时调用，避免“窗口没了但内容还留在桌面上”）。
///
/// `visible = true` 时只显示当前激活 tab，其余保持隐藏；若此时**一个 tab 都没有**
/// （例如“关闭最后一个 tab → 隐藏到托盘”之后又被唤醒），自动补建一个空 tab，
/// 否则恢复出来的会是一个没有任何页签的空壳窗口。
pub(crate) fn set_tabs_visible(app: &tauri::AppHandle, visible: bool) {
    // 这里是所有 tab 显隐的唯一出口 → 顺带维护「已收起」标志，供 sync_tabs_with_host 判断
    // 是否需要真正动窗口（避免宿主每次 Resized 都重复 show/hide）。
    TABS_SUSPENDED.store(!visible, Ordering::SeqCst);

    let (idx, labels) = {
        let state = app.state::<TabManager>();
        let tabs = state.tabs.lock().unwrap_or_else(|p| p.into_inner());
        let idx = *state.active.lock().unwrap_or_else(|p| p.into_inner());
        let labels = tabs.iter().map(|t| t.label.clone()).collect::<Vec<_>>();
        (idx, labels)
    };

    if visible && labels.is_empty() {
        let _ = spawn_tab(app);
        return;
    }

    for (i, label) in labels.iter().enumerate() {
        if let Some(w) = app.get_webview_window(label) {
            if visible && i == idx {
                let _ = w.show();
            } else {
                let _ = w.hide();
            }
        }
    }

    if !visible {
        // 隐藏宿主时把幽灵窗口一并收掉（自愈）。
        purge_orphan_tabs(app);
    }
}

/// 按宿主窗口的「最小化 / 隐藏」状态同步 tab 窗口可见性（幂等，可高频调用）。
///
/// 覆盖**所有**最小化入口（标签栏按钮 / 任务栏右键 / Win+↓ 快捷键）：它们最终都走
/// Windows 的 `WM_SIZE`，tao 会转成 `WindowEvent::Resized` 回调，这里再读一次真实状态即可，
/// 不依赖是哪个入口触发的。
fn sync_tabs_with_host(app: &tauri::AppHandle) {
    let Some(host) = app.get_webview_window(HOST_LABEL) else {
        return;
    };
    let suspended = host.is_minimized().unwrap_or(false) || !host.is_visible().unwrap_or(true);
    // 只有状态真正翻转时才动窗口（Resized 会在拖拽缩放时高频触发）。
    if suspended != TABS_SUSPENDED.load(Ordering::SeqCst) {
        set_tabs_visible(app, !suspended);
    }
}

/// 重新计算所有 tab 窗口的位置与尺寸，使其**精确**贴合宿主窗口内容区（标签栏下方）。
///
/// 在宿主窗口 Moved / Resized / DPI 变化事件、tab 增删时调用。
///
/// 坐标语义（很重要，历史 bug 就出在这里）：
/// - 用宿主**客户区**（inner_position/inner_size）而非窗口矩形（outer_*）：宿主虽
///   `decorations(false)` 但 `resizable(true)`，Windows 下仍有不可见调整边框，
///   outer 比客户区大一圈 → tab 窗口溢出宿主右/下边缘（“两个窗体不一致”）。
/// - tab 侧：`set_size` 设置的是**客户区**尺寸（tao `set_inner_size` 会自行补偿无边框窗口的
///   隐藏偏移），而 `set_position` 设置的是**窗口矩形**位置 → 需减去该窗口自身的
///   `outer - inner` 偏移，才能让客户区左上角正好落在宿主内容区左上角。
fn relayout(app: &tauri::AppHandle) {
    let Some((x, y, width, height)) = host_content_rect(app) else {
        return;
    };

    let labels = tab_labels(app);
    for label in labels {
        if let Some(w) = app.get_webview_window(&label) {
            let (dx, dy) = frame_offset(&w);
            let _ = w.set_position(tauri::PhysicalPosition::new(x - dx, y - dy));
            let _ = w.set_size(tauri::PhysicalSize::new(width, height));
        }
    }
}

/// 把当前 tab 列表 + 激活态推送给前端标签栏（宿主窗口 eval）。
fn push_tabs_to_shell(app: &tauri::AppHandle) {
    let Some(host) = app.get_webview_window(HOST_LABEL) else {
        return;
    };

    // 组装成 JS 数组 [{label, title, active}]。锁的作用域收在块内，
    // `host.eval` 在锁外调用（锁不跨 FFI）。
    let items = {
        let state = app.state::<TabManager>();
        let tabs = state.tabs.lock().unwrap_or_else(|p| p.into_inner());
        let active = *state.active.lock().unwrap_or_else(|p| p.into_inner());
        let titles = state.titles.lock().unwrap_or_else(|p| p.into_inner());

        let mut items = String::from("[");
        for (i, t) in tabs.iter().enumerate() {
            if i > 0 {
                items.push(',');
            }
            let title = titles
                .get(&t.label)
                .cloned()
                .unwrap_or_else(|| t.title.clone());
            let title_js = crate::loading::js_str(&title);
            let active_js = if i == active { "true" } else { "false" };
            items.push_str(&format!(
                "{{label:{},title:{},active:{}}}",
                crate::loading::js_str(&t.label),
                title_js,
                active_js
            ));
        }
        items.push(']');
        items
    };

    let js =
        format!("try{{window.__web2appTabs&&window.__web2appTabs.render({items});}}catch(e){{}}");
    let _ = host.eval(&js);
}

/// 创建宿主窗口（标签栏所在），仅 Windows 多页签模式使用。
///
/// 宿主窗口本身不承载网页，只渲染 `shell://` 标签栏；首个 tab 随后由调用方创建。
pub(crate) fn create_host_window(app: &tauri::AppHandle) -> Result<WebviewWindow, String> {
    let host = WebviewWindowBuilder::new(app, HOST_LABEL, shell_url())
        .title(WINDOW_TITLE)
        .decorations(false)
        .resizable(true)
        .inner_size(1100.0, 650.0)
        .min_inner_size(400.0, 300.0)
        .background_color(crate::bootstrap::SPLASH_BG)
        .build()
        .map_err(|e| format!("创建宿主窗口失败：{e}"))?;

    // 宿主窗口拖动 / 缩放 / 换显示器（DPI 变化）/ 最小化 / 恢复时，tab 窗口跟随。
    // 注意：`on_window_event` 返回 `()`，不能写 `let _ = ...`（会触发 clippy::let_unit_value）。
    let app_handle = app.clone();
    host.on_window_event(move |event| {
        use tauri::WindowEvent;
        match event {
            // 最小化 / 恢复都会走 WM_SIZE → Resized：先同步显隐，再重排（最小化时
            // host_content_rect 会返回 None，relayout 自动空转）。
            WindowEvent::Resized(_) => {
                sync_tabs_with_host(&app_handle);
                relayout(&app_handle);
            }
            WindowEvent::Moved(_) | WindowEvent::ScaleFactorChanged { .. } => {
                relayout(&app_handle);
            }
            _ => {}
        }
    });

    Ok(host)
}

// ---------- IPC 命令 ----------

/// 前端：最小化宿主窗口（标签栏右侧的最小化按钮）。
///
/// 只调 `host.minimize()` 是不够的：tab 窗口是独立顶层窗口，不会跟着最小化，会留在桌面上。
/// 先收起 tab 窗口再最小化宿主，避免最小化动画期间出现「标签栏下去了、内容还在」的错位；
/// 宿主恢复时由 [`sync_tabs_with_host`] 自动还原（覆盖 Win+↓ / 任务栏右键等所有入口）。
#[tauri::command]
pub(crate) fn minimize_host(app: tauri::AppHandle) {
    set_tabs_visible(&app, false);
    if let Some(host) = app.get_webview_window(HOST_LABEL) {
        let _ = host.minimize();
    }
}

/// 前端：关闭宿主窗口（标签栏右侧的关闭按钮），等价于点宿主窗口的关闭按钮。
///
/// 走 `CloseRequested` → [`crate::handle_main_close`] 的统一语义：启用「关闭时最小化到托盘」
/// 则隐藏到托盘，否则退出应用（tab 窗口随进程一并销毁）。
#[tauri::command]
pub(crate) fn close_host(app: tauri::AppHandle) {
    close_host_window(&app);
}

/// 前端：新建一个 tab。
///
/// ⚠️ 标签栏已移除「+」按钮，当前没有前端调用方。仍保留并注册：`target="_blank"` /
/// `window.open()` 由 WebView2 的 `NewWindowRequested` 钩子在 Rust 侧直接走 [`spawn_tab`]，
/// 本命令留作程序化建 tab 的入口（调试 / 后续扩展）。
#[tauri::command]
pub(crate) fn create_tab(app: tauri::AppHandle) -> Result<String, String> {
    spawn_tab(&app)
}

/// 前端：切换到指定 tab。
#[tauri::command]
pub(crate) fn switch_tab(app: tauri::AppHandle, label: String) {
    activate_tab(&app, &label);
}

/// 前端：关闭指定 tab。
#[tauri::command]
pub(crate) fn close_tab(app: tauri::AppHandle, label: String) {
    close_tab_inner(&app, &label);
}

/// 前端：获取当前 tab 列表（label + title + active），用于初始化标签栏。
#[tauri::command]
pub(crate) fn get_tabs(app: tauri::AppHandle) -> Vec<(String, String, bool)> {
    let state = app.state::<TabManager>();
    let tabs = state.tabs.lock().unwrap_or_else(|p| p.into_inner());
    let active = *state.active.lock().unwrap_or_else(|p| p.into_inner());
    let titles = state.titles.lock().unwrap_or_else(|p| p.into_inner());
    tabs.iter()
        .enumerate()
        .map(|(i, t)| {
            let title = titles
                .get(&t.label)
                .cloned()
                .unwrap_or_else(|| t.title.clone());
            (t.label.clone(), title, i == active)
        })
        .collect()
}

/// 前端：上报某个 tab 的实际网页标题（页面 title 变化时由注入脚本回传）。
#[tauri::command]
pub(crate) fn set_tab_title(app: tauri::AppHandle, label: String, title: String) {
    {
        let state = app.state::<TabManager>();
        state
            .titles
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(label.clone(), title.clone());
    }
    {
        let state = app.state::<TabManager>();
        let mut tabs = state.tabs.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(t) = tabs.iter_mut().find(|t| t.label == label) {
            t.title = title.clone();
        }
    }
    push_tabs_to_shell(&app);
}

/// 计算 `target="_blank"` / `window.open()` 弹窗在**新 tab** 中应该落在哪里。
///
/// 背景（这是“弹窗打开的网址一直是 app_url”的根因）：启用本地反向代理时，WebView 实际访问的是
/// `http://127.0.0.1:<port>`，页面里所有相对链接、`window.open(path)` 解析出来的 URL 主机都是
/// `127.0.0.1`，而上游真实主机是另一回事。历史实现拿目标 URL 的主机去和**上游主机**比对
/// （`upstream_host`），必然不相等 → 一律放弃目标路径，新 tab 就只剩 app_url。
///
/// 规则：
/// 1. 目标与**入口主机**同源（代理模式下即 127.0.0.1:port）→ 取目标的 path/query/fragment
///    拼到入口地址上（代理会把所有 path 转发到同一上游，Host 头由代理透传）。
/// 2. 目标与**上游真实主机**同源（页面里写死的绝对链接）→ 同样映射到入口地址。
/// 3. 其它跨域外部链接：走本地反向代理时直接打开该外链（代理只对固定上游注入凭据，不会泄漏）；
///    非代理（直连 + URL 带 userinfo，靠 WebView2 原生钩子注入 Authorization）时返回 None，
///    维持入口地址，避免把本应用凭据发给外部站点。
fn resolve_popup_url(app: &tauri::AppHandle, target: Option<&str>) -> Option<String> {
    let target = target?;
    let tu = url::Url::parse(target).ok()?;
    if !matches!(tu.scheme(), "http" | "https") {
        return None;
    }

    let nav = app
        .state::<TabManager>()
        .navigation
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()?;
    let base = url::Url::parse(&nav.url).ok()?;

    let same_as_entry = tu.host_str() == base.host_str();
    let same_as_upstream = nav
        .upstream_host
        .as_deref()
        .is_some_and(|h| Some(h) == tu.host_str());
    if same_as_entry || same_as_upstream {
        let mut joined = base.clone();
        joined.set_path(tu.path());
        joined.set_query(tu.query());
        joined.set_fragment(tu.fragment());
        return Some(joined.to_string());
    }

    // 跨域外部链接：仅在本地反向代理模式下直接打开（无凭据泄漏风险）。
    if nav.use_local_proxy {
        return Some(target.to_string());
    }
    None
}

/// 处理来自 WebView2 `NewWindowRequested` 的新窗口请求（target="_blank" / window.open()）。
///
/// 仅多页签模式下有意义：不拦截到当前窗，而是新建一个 tab，并导航到弹窗请求的目标地址
/// （落点由 [`resolve_popup_url`] 计算；算不出来时回落到入口地址）。
#[cfg(windows)]
pub(crate) fn open_new_tab_from_request(app: &tauri::AppHandle, target_url: Option<String>) {
    let start = resolve_popup_url(app, target_url.as_deref());
    // 建 tab 时直接带上目标地址做**一次**导航（历史实现是先建窗导航主地址、再二次导航改地址，
    // 既多一次请求，也与首次导航存在竞态）。
    let _ = spawn_tab_with_target(app, start);
}
