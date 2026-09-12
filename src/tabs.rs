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

use std::collections::HashMap;
use std::sync::Mutex;

use tauri::{Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::bootstrap::Navigation;
use crate::config::{TABS_ENABLED, WINDOW_TITLE};

/// 宿主窗口（标签栏所在）与 tab 窗口的固定 label 前缀。
pub(crate) const HOST_LABEL: &str = "host";
pub(crate) const TAB_PREFIX: &str = "tab-";

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

/// 在指定 tab 窗口内打开（导航到）缓存的导航地址。
///
/// 新 tab 建窗后调用：复用 `TabManager.navigation` 缓存（首次启动已解析好），
/// 避免每个 tab 重复 DNS 解析 + 重启本地代理。缓存不存在（尚未就绪）时回落到
/// 完整 bootstrap 流程。
pub(crate) fn navigate_tab(app: &tauri::AppHandle, label: &str) {
    let nav = app
        .state::<TabManager>()
        .navigation
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();

    match nav {
        Some(nav) => {
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
    let label = next_tab_label(app);

    // tab 窗口：去边框、尺寸跟随宿主内容区，起始位置由 relayout 统一计算。
    // 先建窗但不立即定位——relayout() 会统一算所有 tab 的坐标。
    let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
        .title(WINDOW_TITLE)
        .decorations(false)
        // ⚠️ 标签不占独立任务栏按钮：只有宿主窗口出现在任务栏，视觉上是一个整体窗口。
        // （若任由每个 tab 出任务栏按钮，会看到“多个相同图标”重复，用户侧反馈的“有两个窗口”。）
        .skip_taskbar(true)
        .resizable(false)
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
    let _window = window
        .build()
        .map_err(|e| format!("创建 tab 窗口失败：{e}"))?;

    // 记录 tab 信息。
    {
        let state = app.state::<TabManager>();
        let mut tabs = state.tabs.lock().unwrap_or_else(|p| p.into_inner());
        tabs.push(TabInfo {
            label: label.clone(),
            title: WINDOW_TITLE.to_string(),
        });
    }

    // 导航（复用缓存）+ 挂 WebView2 钩子。
    navigate_tab(app, &label);

    // 激活新 tab（切到前台 + 重新布局）。
    activate_tab(app, &label);

    Ok(label)
}

/// 激活指定 tab：显示目标 tab、隐藏其余，并把标签栏状态同步给前端。
fn activate_tab(app: &tauri::AppHandle, label: &str) {
    let idx = {
        let state = app.state::<TabManager>();
        let tabs = state.tabs.lock().unwrap_or_else(|p| p.into_inner());
        tabs.iter().position(|t| t.label == label)
    };

    let Some(idx) = idx else { return };

    {
        let state = app.state::<TabManager>();
        let mut active = state.active.lock().unwrap_or_else(|p| p.into_inner());
        *active = idx;
    }

    // 显示目标 tab，隐藏其它 tab 窗口。
    for (i, t) in app
        .state::<TabManager>()
        .tabs
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .enumerate()
    {
        if let Some(w) = app.get_webview_window(&t.label) {
            if i == idx {
                let _ = w.show();
                let _ = w.set_focus();
            } else {
                let _ = w.hide();
            }
        }
    }

    // 通知前端标签栏刷新激活态。
    push_tabs_to_shell(app);

    // 重新布局：把目标 tab 吸附到宿主内容区并铺满（新建 tab 后也必须立即执行一次，
    // 否则 tab 窗口停留在建窗时的默认小尺寸，正文四周出现大片空隙——用户反馈“没铺满”）。
    relayout(app);
}

/// 关闭指定 tab：销毁窗口、移除记录，若关闭的是激活 tab 则激活相邻 tab。
/// 关闭【最后一个】tab 时，交由宿主窗口的关闭语义处理（由 lib.rs 决定退出还是隐藏到托盘）。
pub(crate) fn close_tab_inner(app: &tauri::AppHandle, label: &str) {
    // 不允许关闭最后一个 tab（等价于关闭宿主窗口，交给 lib.rs 的 CloseRequested 语义）。
    {
        let state = app.state::<TabManager>();
        let tabs = state.tabs.lock().unwrap_or_else(|p| p.into_inner());
        if tabs.len() <= 1 {
            return;
        }
    }

    let removed_idx = {
        let state = app.state::<TabManager>();
        let mut tabs = state.tabs.lock().unwrap_or_else(|p| p.into_inner());
        let idx = tabs.iter().position(|t| t.label == label);
        if let Some(i) = idx {
            tabs.remove(i);
        }
        idx
    };

    if let Some(window) = app.get_webview_window(label) {
        let _ = window.close();
    }

    // 若关闭的是当前激活 tab，则激活相邻 tab（优先右侧，否则左侧）。
    if let Some(removed_idx) = removed_idx {
        let active = {
            let state = app.state::<TabManager>();
            let guard = state.active.lock().unwrap_or_else(|p| p.into_inner());
            *guard
        };
        if removed_idx == active {
            let new_label = {
                let state = app.state::<TabManager>();
                let tabs = state.tabs.lock().unwrap_or_else(|p| p.into_inner());
                let new_idx = removed_idx.min(tabs.len().saturating_sub(1));
                tabs[new_idx].label.clone()
            };
            activate_tab(app, &new_label);
        } else if removed_idx < active {
            // 关闭的是激活 tab 之前的 tab，激活索引前移。
            let state = app.state::<TabManager>();
            let mut a = state.active.lock().unwrap_or_else(|p| p.into_inner());
            *a -= 1;
        }
    }

    push_tabs_to_shell(app);
}

/// 重新计算所有 tab 窗口的位置与尺寸，使其吸附到宿主窗口内容区（标签栏下方）。
///
/// 在宿主窗口 Moved / Resized 事件、tab 增删时调用。tab 窗口 top = 宿主窗口 top + 标签栏高度，
/// 其余三边与宿主窗口内容区对齐（此处用宿主窗口的外框近似，因宿主去边框，outer 即客户区）。
fn relayout(app: &tauri::AppHandle) {
    let Some(host) = app.get_webview_window(HOST_LABEL) else {
        return;
    };
    let Ok(pos) = host.outer_position() else {
        return;
    };
    let Ok(size) = host.outer_size() else {
        return;
    };

    // 内容区：宿主窗口 outer 即客户区（无边框），减去标签栏高度。
    // outer_position/outer_size 都是物理像素，而 shell.css 的标签栏高度是 38 逻辑像素，
    // 必须乘 DPI 缩放换算成物理值，否则高分屏下内容偏移/高度算错 → “内容不满/重叠”。
    let scale = host.scale_factor().unwrap_or(1.0);
    let bar_phys = ((TAB_BAR_HEIGHT as f64) * scale).round() as i32;
    let content_top = pos.y + bar_phys;
    let content_height = (size.height as i32 - bar_phys).max(0);

    let tabs = app
        .state::<TabManager>()
        .tabs
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();

    for t in tabs {
        if let Some(w) = app.get_webview_window(&t.label) {
            let _ = w.set_position(tauri::PhysicalPosition::new(pos.x, content_top));
            let _ = w.set_size(tauri::PhysicalSize::new(size.width, content_height as u32));
        }
    }
}

/// 把当前 tab 列表 + 激活态推送给前端标签栏（宿主窗口 eval）。
fn push_tabs_to_shell(app: &tauri::AppHandle) {
    let Some(host) = app.get_webview_window(HOST_LABEL) else {
        return;
    };
    let state = app.state::<TabManager>();
    let tabs = state.tabs.lock().unwrap_or_else(|p| p.into_inner());
    let active = *state.active.lock().unwrap_or_else(|p| p.into_inner());
    let titles = state.titles.lock().unwrap_or_else(|p| p.into_inner());

    // 组装成 JS 数组 [{label, title, active}]。
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

    // 宿主窗口拖动 / 缩放时，tab 窗口跟随。
    // 注意：`on_window_event` 返回 `()`，不能写 `let _ = ...`（会触发 clippy::let_unit_value）。
    let app_handle = app.clone();
    host.on_window_event(move |event| {
        use tauri::WindowEvent;
        if matches!(event, WindowEvent::Moved(_) | WindowEvent::Resized(_)) {
            relayout(&app_handle);
        }
    });

    Ok(host)
}

// ---------- IPC 命令 ----------

/// 前端：新建一个 tab。
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

/// 处理来自 WebView2 `NewWindowRequested` 的新窗口请求（target="_blank" / window.open()）。
///
/// 仅多页签模式下有意义：不拦截到当前窗，而是新建一个 tab。
/// `target_url` 是页面请求的目标地址（COM 读取失败时为 None）。
///
/// 同源策略：目标 URL 与上游（APP_URL 解析出的真实主机）【同源】时才导航到目标路径
/// （本地代理 / 直连都能正确路由）；跨域目标（外部站点链接）维持“打开主地址”的默认行为，
/// 避免把外部站点错误路由到内部上游，或在直连鉴权模式下向外部站点注入本应用凭据。
#[cfg(windows)]
pub(crate) fn open_new_tab_from_request(app: &tauri::AppHandle, target_url: Option<String>) {
    // 先建 tab（默认导航到缓存的主地址，并挂好鉴权/新窗口/下载钩子），拿到 label。
    let label = match spawn_tab(app) {
        Ok(l) => l,
        Err(_) => return,
    };
    let Some(target) = target_url else { return };
    let Some(window) = app.get_webview_window(&label) else {
        return;
    };
    let nav = app
        .state::<TabManager>()
        .navigation
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    let Some(nav) = nav else { return };

    // 同源校验：目标主机必须等于上游主机，否则维持主地址（外部链接不进代理）。
    let Ok(tu) = url::Url::parse(&target) else {
        return;
    };
    let Some(upstream_host) = nav.upstream_host.as_deref() else {
        return;
    };
    if tu.host_str() != Some(upstream_host) {
        return;
    }

    // 把目标 URL 的 path+query 拼到导航地址（本地代理入口 / 直连地址）后做第二次导航。
    let pq = match tu.query() {
        Some(q) => format!("{}?{}", tu.path(), q),
        None => tu.path().to_string(),
    };
    let Ok(base) = url::Url::parse(&nav.url) else {
        return;
    };
    let Ok(joined) = base.join(&pq) else { return };
    crate::webview_hooks::navigate_webview(&window, joined.as_str());
}
