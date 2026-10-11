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
use crate::config::{TABS_ENABLED, TITLE};

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

/// 同时存活的 tab 上限 —— **熔断保护**，不是产品约束。
///
/// 为什么需要：任何「一次输入触发多个建窗」的异常路径都会形成正反馈——窗口越多 → 主线程
/// 建窗越慢 → 更多建窗被排队放大 → 直到把整机拖死。历史真凶是 WebView2 钩子被重复注册
/// （见 `webview_hooks::HOOKS_REGISTERED`，一次点击打开 N 个窗口）。该 bug 已修，但这类
/// 故障的代价是「用户电脑卡死」，值得兜一个硬上限：最坏情况下机器还活着、还能关掉多余页签。
const MAX_TABS: usize = 32;

/// 一个 tab 的运行时信息。
#[derive(Clone)]
pub(crate) struct TabInfo {
    /// 窗口 label（`tab-0` / `tab-1` …）。
    pub label: String,
    /// 展示标题（优先取网页真实 title，缺省用应用显示名）。
    pub title: String,
    /// 标题回传事件的一次性防伪令牌（见 [`report_tab_title`]）。建窗时生成并注入该 tab
    /// 的初始化脚本；远程页面回传标题时必须原样带回，否则拒绝。
    pub token: String,
}

/// 标题回传事件名（tab 远程页面 → Rust）。与 `src/tab_title.js` 里的字面量逐字一致，
/// 受 `ipc_command_registry` / `capability_contract` 静态守卫钉着。
pub(crate) const TAB_TITLE_EVENT: &str = "web2app:tab-title";

/// tab 首屏（本地占位文档）上「重试」按钮触发的事件 → Rust 为该 tab 重走 bootstrap。
/// 与 `src/loading.js` 字面量逐字一致（静态守卫）。payload 同样带一次性 token 防伪。
pub(crate) const TAB_RETRY_EVENT: &str = "web2app:retry-bootstrap";

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
    /// 每 tab 的实际网页标题缓存：label -> title（由 tab 页面的 `web2app:tab-title`
    /// 核心事件回传，落地见 [`report_tab_title`]）。
    pub titles: Mutex<HashMap<String, String>>,
    /// 首屏 bootstrap 是否**已由多页签路径启动**。
    ///
    /// 用途（见 [`navigate_tab`]）：导航缓存 `navigation` 为空有两种截然不同的含义，
    /// 必须区分，否则要么「首个 tab 永远等不到 bootstrap」（30s 超时才回落，用户侧就是
    /// 首次打开极慢），要么「极早期 window.open 再起一轮 bootstrap 把在用代理 abort 掉」。
    ///   · `false`（首次置位者）＝还没人启动过 → 由本次建窗拉起完整 bootstrap；
    ///   · `true`（非首次）＝已有 bootstrap 在途 → 只轮询等缓存，不重复启动。
    ///
    /// 只在进程内单次翻转，不做复位：bootstrap 失败时缓存永远写不进来，后续等待方靠
    /// 超时回落再起一轮（等价于旧行为），无需把标志还原。
    pub bootstrap_started: AtomicBool,
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

/// 生成 128-bit 标题回传令牌（32 字符小写 hex）。
///
/// CSPRNG 取随机数失败属于环境级故障：返回 None，调用方据此**拒绝建 tab**
///（宁可开不出来，也不接受「任何页面都能回传标题」的降级 —— 那等于没有令牌）。
fn new_tab_token() -> Option<String> {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf).ok()?;
    Some(buf.iter().map(|b| format!("{b:02x}")).collect())
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
/// 用宿主**客户区**（inner_position/inner_size）作为基准：宿主与 tab 统一 `shadow(false)`
/// 后窗口矩形 == 客户区，两者等价；客户区语义在特殊窗口状态下更稳（如窗口刚创建、
/// 尚未显示时），且与 [`place_tab`] 里 `inner_position/inner_size` 的读回值同一坐标系。
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

/// 把 tab 窗口的**客户区**精确摆放到物理像素矩形 `(x, y, width, height)`。
///
/// 用**闭环校准**而不是一次性换算：`set_position` / `set_size` 各自还要经过 tao → Win32
/// 的一层换算（无边框窗口的 insets、DPI 取整、`min_inner_size` 约束等），任何一步与实际
/// 不符都会让 tab 偏出宿主边缘（即用户看到的「两个窗体不一致」）。这里直接
/// 「设置 → 读回 → 补偿误差」，不依赖任何关于边框厚度的假设，几轮内必然收敛。
///
/// 前提：宿主与 tab 都 `shadow(false)`，窗口矩形 == 客户区。若某天恢复阴影，
/// `set_position`（窗口矩形语义）与 `inner_position`（客户区语义）不再等值，本函数也照样
/// 收敛，只是多迭代一次。
fn place_tab(window: &WebviewWindow, x: i32, y: i32, width: u32, height: u32) {
    // ① 尺寸：先设一次，再按读回的客户区尺寸补偿误差。
    let mut tw = width as i32;
    let mut th = height as i32;
    for _ in 0..3 {
        let _ = window.set_size(tauri::PhysicalSize::new(tw.max(1) as u32, th.max(1) as u32));
        match window.inner_size() {
            Ok(size) => {
                let ew = width as i32 - size.width as i32;
                let eh = height as i32 - size.height as i32;
                if ew == 0 && eh == 0 {
                    break;
                }
                tw += ew;
                th += eh;
            }
            Err(_) => break,
        }
    }

    // ② 位置：让**客户区**左上角正好落在 (x, y)。
    let mut tx = x;
    let mut ty = y;
    for _ in 0..3 {
        let _ = window.set_position(tauri::PhysicalPosition::new(tx, ty));
        match window.inner_position() {
            Ok(inner) => {
                let ex = x - inner.x;
                let ey = y - inner.y;
                if ex == 0 && ey == 0 {
                    break;
                }
                tx += ex;
                ty += ey;
            }
            Err(_) => break,
        }
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
            // 缓存未就绪要先区分两种情形（这是「首次打开极慢 / 新 tab 落回首页」的根因，
            // 见字段 [`TabManager::bootstrap_started`] 的文档）：
            let first = {
                let state = app.state::<TabManager>();
                // swap 返回旧值：旧值 false ⇒ 本调用是第一个来的 ⇒ 由它负责启动 bootstrap。
                !state.bootstrap_started.swap(true, Ordering::SeqCst)
            };
            if first {
                // 首个 tab：直接起完整 bootstrap（解析真实地址 / 启动本地代理，完成后写入
                // `navigation` 缓存并在主线程导航本 tab）。旧实现在这里改成「只等缓存」，
                // 却没有人启动 bootstrap ⇒ 首个 tab 只能空等 30s 超时才回落，用户侧表现为
                // 「首次打开很慢」。首个 tab 由 lib.rs 以 `start = None` 创建，故无需携带落点。
                dlog!("[tabs] 首屏导航缓存未就绪，启动完整 bootstrap：{label}");
                crate::bootstrap::bootstrap_for_label(app.clone(), label.to_string());
            } else {
                // 已有 bootstrap 在途（首屏尚未就绪时的极早期 window.open 会走到这里）：
                // ⚠️ 不能直接再起一轮 bootstrap_for_label —— 两个 bootstrap 会互相 abort
                // 代理（B 成功后掐掉 A 刚给在用 tab 起好的代理，旧 tab 立刻断连）。改成
                // 后台轮询【等缓存】，就绪后回到本函数走缓存命中分支；超时（首轮 bootstrap
                // 失败、缓存永远不会写入）才回落完整 bootstrap。
                // ⚠️ 轮询期间必须原样保留 `start`（弹窗落点）：历史实现回调时硬编码传 None，
                // 于是首屏就绪前开出的新 tab 全被改成入口地址（用户反馈的「新标签页都是首页」）。
                let app_wait = app.clone();
                let label_wait = label.to_string();
                let start_wait = start;
                std::thread::spawn(move || {
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
                    loop {
                        let ready = app_wait
                            .state::<TabManager>()
                            .navigation
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .is_some();
                        if ready {
                            let a = app_wait.clone();
                            let l = label_wait.clone();
                            let s = start_wait.clone();
                            let _ = app_wait.run_on_main_thread(move || {
                                navigate_tab(&a, &l, s);
                            });
                            return;
                        }
                        if std::time::Instant::now() >= deadline {
                            dlog!(
                                "[tabs] 等待首屏导航缓存超时（30s），回落完整 bootstrap：{label_wait}"
                            );
                            crate::bootstrap::bootstrap_for_label(app_wait.clone(), label_wait);
                            return;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                });
            }
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
    // 熔断：已达上限直接拒绝建窗（详见 MAX_TABS）。必须在 next_tab_label 之前返回，
    // 否则会白白消耗一个 label（label 只用于日志/前端展示，但保持单调更干净）。
    {
        let state = app.state::<TabManager>();
        let live = state.tabs.lock().unwrap_or_else(|p| p.into_inner()).len();
        if live >= MAX_TABS {
            dlog!(
                "[tabs] 已达 tab 上限 {MAX_TABS}，拒绝新建；若这是单次点击触发的，说明存在\
                 「异常建窗风暴」——请把日志里连续的「拦截到新窗口请求」记录发出来定位"
            );
            return Err(format!("tab 数量已达上限 {MAX_TABS}"));
        }
    }

    let label = next_tab_label(app);
    // 标题回传令牌：CSPRNG 失败时拒绝建窗（见 new_tab_token 的注释）。
    let Some(token) = new_tab_token() else {
        return Err("生成页签令牌失败（系统随机源不可用）".to_string());
    };

    // ⚠️ 先登记（占位）再建窗：`purge_orphan_tabs` 会销毁“label 不在册”的 tab 窗口，若先建窗，
    // 并发（用户连点“+”）时新窗口可能被另一个线程的清理误杀。建窗失败再回滚登记。
    {
        let state = app.state::<TabManager>();
        let mut tabs = state.tabs.lock().unwrap_or_else(|p| p.into_inner());
        tabs.push(TabInfo {
            label: label.clone(),
            title: TITLE.to_string(),
            token: token.clone(),
        });
    }
    // tab 窗口：去边框 + 不占任务栏按钮，尺寸/位置按宿主客户区一次到位。
    // ⚠️ 建窗时先 `visible(false)`：否则会以框架默认尺寸（800×600）闪一帧再被 relayout 纠正。
    // 首次导航照样发起（页面在不可见状态下开始加载），最后由 activate_tab 显示。
    let mut window = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
        .title(TITLE)
        .decorations(false)
        // ⚠️ 必须与宿主窗口用**完全相同**的 `shadow(false)`：两边都不内缩客户区，
        // 窗口矩形 == 客户区，[`place_tab`] 的坐标换算才成立。
        // 详见 [`create_host_window`] 里对 MARKER_UNDECORATED_SHADOW 的说明。
        .shadow(false)
        // ⚠️ 标签不占独立任务栏按钮：只有宿主窗口出现在任务栏，视觉上是一个整体窗口。
        // （若任由每个 tab 出任务栏按钮，会看到“多个相同图标”重复，用户侧反馈的“有两个窗口”。）
        .skip_taskbar(true)
        .resizable(false)
        .visible(false)
        .background_color(crate::bootstrap::SPLASH_BG)
        // ⚠️ 模式标记必须排在 OVERLAY_SCRIPT【之前】（初始化脚本按注册顺序执行）：
        // tab 首文档是本地占位页（http://tauri.localhost/index.html），让 loading.js 以
        // splash 语义运行（不做内容就绪自动淡出：占位页永远等不到真实内容）；导航到远程
        // 地址后的新文档（127.0.0.1:<port> 或上游 https）则是 overlay 语义，渲染后淡出。
        // 不区分会导致占位页 ~240ms 就淡出露出占位文案、且后续 fail() 被 done 态吞掉。
        .initialization_script(
            "window.__web2appLoadingMode = (location.hostname === 'tauri.localhost' \
             || location.protocol === 'tauri:') ? 'splash' : 'overlay';",
        )
        .initialization_script(crate::loading::OVERLAY_SCRIPT)
        .initialization_script(format!(
            "window.web2appExtra = {};",
            crate::config::extra_json()
        ))
        // 注入当前 tab 的 label 与一次性令牌，供标题回传脚本（tab_title.js）读取后随事件回传。
        // ⚠️ 令牌只能进初始化脚本（与窗口 1:1），不要写进页面可读的其它共享位置。
        // ⚠️⚠️ initialization_script（WebView2 AddScriptToExecuteOnDocumentCreated）会注入
        //【所有 frame】，包括页面内嵌的第三方跨域 iframe；不做框架判断等于把防伪令牌白送给
        // 任意广告 frame（虽只能伪造本 tab 标题，仍是不必要的暴露）。故 label/token 只挂到
        // 主框架 window，子框架拿不到；tab_title.js 自身也有同样的主框架早退守卫。
        .initialization_script(format!(
            "if (window.self === window.top) {{ window.__web2appTabLabel = {}; }}",
            crate::loading::js_str(&label)
        ))
        // 标题回传：监听 document.title 变化 → emit(TAB_TITLE_EVENT, {label,title,token})
        // → 标签栏显示真实标题。走核心事件而非应用命令的原因见 tab_title.js 头注释。
        .initialization_script(format!(
            "if (window.self === window.top) {{ window.__web2appTabToken = {}; }}",
            crate::loading::js_str(&token)
        ))
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

    // 禁用拖放 handler + 注入拖拽 polyfill（与主窗口一致）。本模块整体 cfg(windows)，
    // 无需再写属性；rebinding 用 mut 因为后面的代理参数块还要再改这个 builder。
    let mut window = window
        .disable_drag_drop_handler()
        .initialization_script(include_str!("dnd_polyfill.js"));

    // 外部 HTTP 代理：与主窗口 / 宿主窗口同一组 --proxy-server（本模块整体 cfg(windows)，
    // additional_browser_args 在这里必然存在）。宿主窗口是同 user-data-folder 下第一个
    // 创建 WebView2 环境的窗口，这边属于双保险；两处都用 config::build_proxy_args() 同源。
    if let Some(proxy_args) = crate::config::build_proxy_args() {
        window = window.additional_browser_args(proxy_args.as_str());
    }

    // 数据目录：与主窗口共享同一 WebView2 数据目录（同账号、共享 cookie/localStorage）。
    // ⚠️ `config::data_dir` 在进程内只解析一次并缓存，所以这里拿到的一定与主窗口同一个目录 ——
    //    这正是「多个 tab 共享登录态」赖以成立的前提，不要改成各算各的。
    let window = match crate::config::data_dir(app) {
        Some(dir) => window.data_directory(dir),
        None => window,
    };

    // ⚠️⚠️ tab 窗口必须设成宿主窗口的 **owned window**（owner，不是 child / parent 定位）。
    //
    // 为什么（用户反馈「点标签栏空白处，页签下的网页会被隐藏，点标签才出来」的根因）：
    // 宿主（标签栏）与 tab（网页内容）是两个**互相独立**的顶层窗口。不设 owner 时二者在
    // z-order 上完全平等，于是**点宿主的任何位置**——包括标签栏里页签右侧那段空白拖动区
    // （`.tabs` 是 `-webkit-app-region: drag`）——都会让 Windows 激活宿主并把它抬到 tab
    // 之上。宿主的客户区在标签栏下方是一大片空底色，正好把 tab 的网页整个盖住 → 用户看到
    // 「页签下的网页被隐藏了」；再点一下页签时 `activate_tab` 里的 `set_focus()` 又把 tab
    // 抬起来 → 网页「又出来了」。所以这个现象看起来像随机闪烁，实际是确定的 z-order 关系。
    //
    // MSDN（owned windows）：owned window **永远在自己 owner 之上**，且 owner 最小化时由
    // 系统隐藏、owner 销毁时由系统销毁。因此设上 owner 后，这类遮挡从根上不可能发生，
    // 不需要在每次宿主获得焦点时手动 reorder 窗口（那种做法会打断拖动、还会抢焦点）。
    //
    // 实现细节：tao 的 `Parent::OwnedBy` 会给窗口加上 `POPUP`（不再有 WS_THICKFRAME /
    // 任务栏按钮）。对本应用正合适：tab 本来就是 `decorations(false) + resizable(false)
    // + shadow(false)`，客户区 == 窗口矩形，[`place_tab`] 的闭环校准照常收敛。
    //
    // ⚠️ 已知副作用（勿删此注释）：owner 最小化时**系统会自己隐藏** owned 窗口，而 tao 的
    // 窗口过程不处理 `WM_SHOWWINDOW`，它的 `VISIBLE` 标志可能与真实可见性短暂脱节；
    // 好在 `sync_tabs_with_host` 会在宿主每次 `Resized`（含最小化/还原）后重新
    // `set_tabs_visible`，两边的标志随即被纠正回来，闭环自洽。
    let window = match app.get_webview_window(HOST_LABEL) {
        Some(host) => match window.owner(&host) {
            Ok(w) => w,
            Err(e) => {
                unregister_tab(app, &label);
                return Err(format!("设置 tab 窗口的 owner（宿主窗口）失败：{e}"));
            }
        },
        None => window,
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

    // ②b 显示后再校准一次：窗口从隐藏变可见时，Win32 会按“可见”状态重算窗口矩形
    // （隐藏窗口读到的部分几何仍是创建时的占位值），这一次调用保证最终落点精确。
    relayout(app);

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

    // ⑤ 更新激活索引并激活相邻 tab（纯规则见 plan_active_after_close，与 Electron
    //    tab-order.ts 同一条：优先右邻，末位取左邻；关的是后台 tab 只前移索引）。
    let active = {
        let state = app.state::<TabManager>();
        let guard = state.active.lock().unwrap_or_else(|p| p.into_inner());
        *guard
    };
    let (new_active, needs_activate) =
        plan_active_after_close(active, removed_idx, remaining.len());
    {
        let state = app.state::<TabManager>();
        let mut a = state.active.lock().unwrap_or_else(|p| p.into_inner());
        *a = new_active;
    }
    if needs_activate {
        let new_label = remaining[new_active].clone();
        activate_tab(app, &new_label);
    } else {
        purge_orphan_tabs(app);
        push_tabs_to_shell(app);
    }
}

/// 关闭一个 tab 后的激活态计划（**纯顺序规则**，不依赖任何 Tauri 对象，可直接单测）。
///
/// 与 `electron-app/src/main/tab-order.ts::nextActiveLabelAfterClose` 同一条规则，
/// 区别仅在 Rust 还要维护「激活索引」本身：
/// - 关的就是激活 tab → 激活落到右邻（删除后右邻移到该下标），删的是末位则取左邻，
///   返回 `needs_activate = true`（调用方要显式 activate_tab 显示新窗口）；
/// - 关的是激活 tab **之前**的后台 tab → 激活索引前移一位（夹到合法范围）；
/// - 关的是之后的 tab → 激活索引不变。
///
/// `remaining_len` 为删除后的幸存数，调用方必须保证 ≥1（最后一个走关闭宿主分支）。
fn plan_active_after_close(
    active_before: usize,
    removed_idx: usize,
    remaining_len: usize,
) -> (usize, bool) {
    let last = remaining_len - 1;
    if removed_idx == active_before {
        (removed_idx.min(last), true)
    } else if removed_idx < active_before {
        ((active_before - 1).min(last), false)
    } else {
        (active_before.min(last), false)
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
/// 在宿主窗口 Moved / Resized / DPI 变化事件、tab 增删时调用；每个 tab 的落点由
/// [`place_tab`] 闭环校准到像素级（历史实现用 `outer - inner` 一次性换算，只要 tao 的
/// 边框假设与实际不符就会整体偏移，正是「两个窗体不一致」的直接原因）。
fn relayout(app: &tauri::AppHandle) {
    let Some((x, y, width, height)) = host_content_rect(app) else {
        return;
    };

    for label in tab_labels(app) {
        if let Some(w) = app.get_webview_window(&label) {
            place_tab(&w, x, y, width, height);
        }
    }
}

/// 把当前 tab 列表 + 激活态推送给前端标签栏（宿主窗口 eval）。
fn push_tabs_to_shell(app: &tauri::AppHandle) {
    let Some(host) = app.get_webview_window(HOST_LABEL) else {
        return;
    };

    // 组装成 JS 数组 [{label, title, active}]。快照在 `tab_rows` 内部取，
    // `host.eval` 在任何锁之外调用（锁不跨 FFI）。
    let mut items = String::from("[");
    for (i, (label, title, active)) in tab_rows(app).into_iter().enumerate() {
        if i > 0 {
            items.push(',');
        }
        let label_js = crate::loading::js_str(&label);
        let title_js = crate::loading::js_str(&title);
        let active_js = if active { "true" } else { "false" };
        items.push_str(&format!(
            "{{label:{label_js},title:{title_js},active:{active_js}}}"
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
    let mut builder = WebviewWindowBuilder::new(app, HOST_LABEL, shell_url())
        .title(TITLE)
        .decorations(false)
        // ⚠️ `.shadow(false)` 是宿主与 tab 对齐的关键，别删。
        //
        // 无边框窗口在 tao 里默认带 `MARKER_UNDECORATED_SHADOW`（来自 `decoration_shadow`
        // 的默认值 true）。该标志会让 `WM_NCCALCSIZE` 把客户区**向内缩一圈**：
        // `tao::platform_impl::windows::util::calculate_window_insets` →
        // `calculate_insets_for_dpi(dpi)`，每边 = SM_CXSIZEFRAME + SM_CXPADDEDBORDER
        // （150% 缩放下约 11 物理像素，两边合计 22）—— 这就是用户截图里 tab 与宿主
        // 「差 22 像素」的来源。
        //
        // ⚠️ 该内缩**只和 DPI 有关，和窗口的 `resizable` 无关**（源码里只有 dpi 参与计算），
        // 所以曾经试图「统一 resizable 让两边一致」是无效改动。真正有效的开关是 `shadow`：
        // 关掉后 MARKER_UNDECORATED_SHADOW 为 false，NCCALCSIZE 不再内缩，
        // 窗口矩形 == 客户区（outer == inner），宿主与 tab 的几何天然一致。
        //
        // 代价：窗口不再有系统投影。对「多窗口叠放伪装成标签页」来说反而是好事——
        // tab 窗口若各自带阴影，会互相盖到对方的标签栏/内容上。
        // 另：`resizable(false)` 后不能用鼠标拖边框改变大小；调整尺寸由应用层通过 Rust
        // 命令触发（启动时从 window_state 恢复）即可。
        .shadow(false)
        .resizable(false)
        .inner_size(1100.0, 650.0)
        .min_inner_size(400.0, 300.0)
        .background_color(crate::bootstrap::SPLASH_BG);

    // ⚠️⚠️ 外部 HTTP 代理开关必须挂在【宿主窗口】上，不能只挂 tab 窗口：WebView2 同一
    // user-data-folder 复用同一个 browser 进程，`--proxy-server` 仅在 browser 进程首次
    // 启动（第一个创建的环境）时生效；多页签里宿主窗口最先创建，它若以空参数把 browser
    // 进程拉起来，之后每个 tab 再带参数也会被忽略 → 全部页签绕过外部代理。tab builder
    // 那边（spawn_tab_with_target）也挂同一份参数做双保险，两处必须同源。
    if let Some(proxy_args) = crate::config::build_proxy_args() {
        builder = builder.additional_browser_args(proxy_args.as_str());
    }

    // 数据目录：与主窗口（lib.rs）/ tab 窗口（本文件 spawn_tab_with_target）**同一个** WebView2
    // 数据目录。
    //
    // ⚠️⚠️ 宿主窗原来漏设了这一句，而它恰是多页签模式下**最先创建 WebView2 环境**的窗口 ——
    //   后果有两条，都是静默的：
    //   ① 宿主的 WebView2 数据落到**系统默认目录**（`%LOCALAPPDATA%\<identifier>\EBWebView`），
    //      绿色版「不写程序目录之外」的承诺被破坏（tab / 主窗都写在 exe 同级 `data/`）；
    //   ② 上面那条注释的前提（「宿主是同 user-data-folder 下第一个创建环境的窗口，所以
    //      proxy 参数必须挂在它身上」）**不再成立**：宿主在默认 UDF、tab 在共享 UDF，
    //      二者分裂成两个环境，`--proxy-server` 的「同一 UDF 首个环境生效」推理失去落点。
    let builder = match crate::config::data_dir(app) {
        Some(dir) => builder.data_directory(dir),
        None => builder,
    };

    let host = builder
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
    // 与宿主标签栏（push_tabs_to_shell）共用同一份快照，保证 IPC 与界面永远一致。
    tab_rows(&app)
}

/// tab 列表的统一快照：`(label, 展示标题, 是否激活)`。
///
/// ⚠️ 为什么抽出来：[`get_tabs`]（IPC 命令）与 [`push_tabs_to_shell`]（宿主标签栏 eval）
/// 此前各写了一遍「锁 tabs / active / titles → 标题取 `titles` 缓存、缺失回落 `TabInfo::title`
/// → 拼 active 标志」。两处一旦漂移，现象是**IPC 返回的标题与标签栏显示的不是同一个**
/// —— 静默不一致，没有任何报错。
///
/// ⚠️ 三把锁的顺序必须固定为 `tabs → active → titles`：与[`set_tabs_visible`] 取锁的顺序
/// 一致，避免两处反向加锁形成 AB-BA 死锁。
fn tab_rows(app: &tauri::AppHandle) -> Vec<(String, String, bool)> {
    let state = app.state::<TabManager>();
    let tabs = state.tabs.lock().unwrap_or_else(|p| p.into_inner());
    let active = *state.active.lock().unwrap_or_else(|p| p.into_inner());
    let titles = state.titles.lock().unwrap_or_else(|p| p.into_inner());
    tabs.iter()
        .enumerate()
        .map(|(i, t)| {
            // 展示标题：优先用 tab 页面回传的真实网页标题，没有则回落到建窗时的
            // 占位标题（通常是应用显示名）。
            let title = titles
                .get(&t.label)
                .cloned()
                .unwrap_or_else(|| t.title.clone());
            (t.label.clone(), title, i == active)
        })
        .collect()
}

/// 校验「tab 一次性令牌」：label 与 token 必须同时命中在册记录。
///
/// ⚠️ 为什么必须有：[`report_tab_title`] 与 [`retry_tab_bootstrap`] 都接**核心事件**，
/// 而 Tauri 2.11 的 `app.listen` Event **不携带发送窗口标识**（只有 id / payload），
/// 无法知道事件来自哪个 webview。少了这道校验，任意外部页面都能伪造其它页签的标题或
/// 强制别的 tab 重启 bootstrap。两处此前各写一遍同样判断，抽出来保证不会只改一边。
fn tab_token_matches(app: &tauri::AppHandle, label: &str, token: &str) -> bool {
    let state = app.state::<TabManager>();
    let tabs = state.tabs.lock().unwrap_or_else(|p| p.into_inner());
    tabs.iter().any(|t| t.label == label && t.token == token)
}

/// tab 远程页面经核心事件回传真实网页标题的落地函数。
///
/// 历史上它是一条 `#[tauri::command]`，但 tab 里跑的是**远程 origin**（代理 127.0.0.1
/// 或直连上游），Tauri 2 对远程页面的应用自定义命令一律拒绝（框架自带测试
/// `remote_origin_blocked_for_custom_commands_without_app_manifest`），且工程没有 app
/// ACL manifest —— 前端 `.catch` 又静默吞掉，标签栏于是永远显示默认应用名。改走
/// `core:event:allow-emit`（可授权给远程 origin）后由本函数落地。
///
/// ⚠️ Tauri 2.11 的 `app.listen` Event **不携带发送窗口标识**（只有 id/payload），
/// 无法知道事件来自哪个 webview；因此 payload 必须带回建窗时注入该 tab 的一次性
/// [`TabInfo::token`]，与在册记录逐字相等才接受 —— 否则任意外部页面都能伪造其它
/// 页签的标题（钓鱼式欺骗）。title 最终经 shell.html 的 textContent 渲染，无 XSS 面。
pub(crate) fn report_tab_title(
    app: &tauri::AppHandle,
    label: String,
    token: String,
    title: String,
) {
    if !tab_token_matches(app, &label, &token) {
        dlog!("[tabs] 拒绝标题回传：label/token 与在册 tab 不匹配（label={label}）");
        return;
    }
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
    push_tabs_to_shell(app);
}

/// tab 首屏「重试」事件的落地：校验 label/token 后为该 tab 重走完整 bootstrap。
///
/// tab 首文档是本地占位页（不是 splash:// 协议页），loading.js 的重试按钮不能像单窗口
/// 那样导航到 `splash://localhost/retry`；改为发 [`TAB_RETRY_EVENT`]。token 校验与
/// [`report_tab_title`] 同理由（app.listen 的 Event 不带发送窗口标识）。
pub(crate) fn retry_tab_bootstrap(app: &tauri::AppHandle, label: String, token: String) {
    if !tab_token_matches(app, &label, &token) {
        dlog!("[tabs] 拒绝首屏重试：label/token 与在册 tab 不匹配（label={label}）");
        return;
    }
    dlog!("[tabs] 收到首屏重试事件，重新走 bootstrap：{label}");
    crate::bootstrap::bootstrap_for_label(app.clone(), label);
}

/// 计算 `target="_blank"` / `window.open()` 弹窗在**新 tab** 中应该落在哪里。
///
/// 背景（这是“弹窗打开的网址一直是 app_url”的根因）：启用本地反向代理时，WebView 实际访问的是
/// `http://127.0.0.1:<port>`，页面里所有相对链接、`window.open(path)` 解析出来的 URL 主机都是
/// `127.0.0.1`，而上游真实主机是另一回事。历史实现拿目标 URL 的主机去和**上游主机**
/// （`upstream_origin`）比对，必然不相等 → 一律放弃目标路径，新 tab 就只剩 app_url。
///
/// 规则：
/// 1. 目标与**入口主机**同源（代理模式下即 127.0.0.1:port）→ 取目标的 path/query/fragment
///    拼到入口地址上（代理会把所有 path 转发到同一上游，Host 头由代理透传）。
/// 2. 目标与**上游真实主机**同源（页面里写死的绝对链接）→ 同样映射到入口地址。
/// 3. 其它跨域外部链接：**直接打开该外链**（与 Electron 壳 `setWindowOpenHandler` →
///    `spawnTab(opened)` 的行为对齐）。凭据不会因此泄漏：直连模式下 Authorization 注入与
///    401 自动应答都受 [`crate::bootstrap::is_upstream_request`] 主机白名单约束，外域请求拿不到
///    入口账号密码；本地反向代理也只对固定上游注入凭据。
fn resolve_popup_url(app: &tauri::AppHandle, target: Option<&str>) -> Option<String> {
    let nav = app
        .state::<TabManager>()
        .navigation
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()?;
    resolve_popup_url_for(&nav, target)
}

/// [`resolve_popup_url`] 的**纯逻辑**部分（不依赖 `AppHandle`，因此可单测）。
///
/// 规则见 [`resolve_popup_url`] 的文档注释，这里只做计算：
///   1. 目标与入口主机同源 → 把 path/query/fragment 拼到入口地址上（代理会转发同上游）；
///   2. 目标与上游真实主机同源 → 同样映射到入口地址；
///   3. 其它跨域外链：直接打开目标地址（凭据由 [`crate::bootstrap::is_upstream_request`]
///      主机白名单把关，不会发给外域）。
fn resolve_popup_url_for(nav: &Navigation, target: Option<&str>) -> Option<String> {
    let target = target?;
    let tu = url::Url::parse(target).ok()?;
    if !matches!(tu.scheme(), "http" | "https") {
        return None;
    }

    let base = url::Url::parse(&nav.url).ok()?;
    let base_origin = crate::bootstrap::origin_of(&base);

    // 源（scheme://host[:默认端口归一化]）比对：旧实现只比 host_str，忽略 scheme 与端口，
    // https://realhost:8443 会被误判成上游 https://realhost。
    let target_origin = crate::bootstrap::origin_of(&tu);
    let same_as_entry = target_origin.is_some() && target_origin == base_origin;
    let same_as_upstream = nav
        .upstream_origin
        .as_deref()
        .is_some_and(|h| target_origin.as_deref() == Some(h));
    if same_as_entry || same_as_upstream {
        let mut joined = base.clone();
        joined.set_path(tu.path());
        joined.set_query(tu.query());
        joined.set_fragment(tu.fragment());
        return Some(joined.to_string());
    }

    // 跨域外部链接：直接打开目标地址（与 Electron 壳一致）。凭据注入受主机白名单约束，
    // 外域请求拿不到入口账号密码，故不存在「把本应用凭据发给外部站点」的风险。
    Some(target.to_string())
}

/// 处理来自 WebView2 `NewWindowRequested` 的新窗口请求（target="_blank" / window.open()）。
///
/// 仅多页签模式下有意义：不拦截到当前窗，而是新建一个 tab，并导航到弹窗请求的目标地址
/// （落点由 [`resolve_popup_url`] 计算；算不出来时回落到入口地址）。
pub(crate) fn open_new_tab_from_request(app: &tauri::AppHandle, target_url: Option<String>) {
    let start = resolve_popup_url(app, target_url.as_deref());
    // 建 tab 时直接带上目标地址做**一次**导航（历史实现是先建窗导航主地址、再二次导航改地址，
    // 既多一次请求，也与首次导航存在竞态）。
    let _ = spawn_tab_with_target(app, start);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个可单测的 Navigation（本地反向代理模式）。
    /// `entry`：代理入口（形如 http://127.0.0.1:PORT/）；`upstream`：上游真实**源**
    /// （scheme://host[:非默认端口]，与 bootstrap::origin_of 同形态）。
    fn proxy_nav(entry: &str, upstream: Option<&str>) -> Navigation {
        Navigation {
            url: entry.to_string(),
            user: String::new(),
            pass: String::new(),
            use_local_proxy: true,
            proxy_abort: None,
            upstream_origin: upstream.map(str::to_string),
        }
    }

    #[test]
    fn popup_same_as_entry_host_joins_path_query_fragment() {
        let nav = proxy_nav("http://127.0.0.1:4567/", None);
        let out =
            resolve_popup_url_for(&nav, Some("http://127.0.0.1:4567/notes?from=popup#x")).unwrap();
        assert_eq!(out, "http://127.0.0.1:4567/notes?from=popup#x");
    }

    #[test]
    fn popup_same_as_entry_replaces_root_path() {
        let nav = proxy_nav("http://127.0.0.1:4567/app", None);
        let out = resolve_popup_url_for(&nav, Some("http://127.0.0.1:4567/other")).unwrap();
        assert_eq!(out, "http://127.0.0.1:4567/other");
    }

    #[test]
    fn popup_same_as_upstream_remaps_through_entry() {
        // 页面里写死的绝对链接（指向上游真实主机）→ 映射到代理入口。
        let nav = proxy_nav("http://127.0.0.1:4567/", Some("https://real.example.com"));
        let out = resolve_popup_url_for(&nav, Some("https://real.example.com/path/a")).unwrap();
        assert_eq!(out, "http://127.0.0.1:4567/path/a");
        // 显式默认端口与省略端口同源
        let out = resolve_popup_url_for(&nav, Some("https://real.example.com:443/path/b")).unwrap();
        assert_eq!(out, "http://127.0.0.1:4567/path/b");
    }

    #[test]
    fn popup_non_default_port_is_not_same_origin() {
        // 旧实现只比 host：同主机的非默认端口会被误判同源、抹掉端口映射到错误服务。
        let nav = proxy_nav("http://127.0.0.1:4567/", Some("https://real.example.com"));
        assert_eq!(
            resolve_popup_url_for(&nav, Some("https://real.example.com:8443/path")),
            Some("https://real.example.com:8443/path".into()),
            "非默认端口不算上游源：代理模式下外链原样打开，不抹端口"
        );
        // scheme 不同也不同源（http 对 https）
        assert_eq!(
            resolve_popup_url_for(&nav, Some("http://real.example.com/path")),
            Some("http://real.example.com/path".into())
        );
    }

    #[test]
    fn popup_cross_domain_proxy_mode_opens_external() {
        // 代理只对固定上游注入凭据，跨域外链直接打开不会泄漏账号。
        let nav = proxy_nav("http://127.0.0.1:4567/", None);
        assert_eq!(
            resolve_popup_url_for(&nav, Some("https://other.com/x")),
            Some("https://other.com/x".into())
        );
    }

    #[test]
    fn popup_cross_domain_plain_mode_opens_external() {
        // 非代理（直连）模式下，跨域外链同样直接打开目标地址（与 Electron 壳一致）：
        // 凭据由 `is_upstream_request` 主机白名单把关，外域请求不会拿到入口账号密码。
        // 历史实现在这里返回 None，导致书签管理器点开外站书签时新 tab 落到入口首页
        // （用户反馈的「新标签页都是首页，不是指定的页面」）。
        let mut nav = proxy_nav("https://entry.example.com", None);
        nav.use_local_proxy = false;
        assert_eq!(
            resolve_popup_url_for(&nav, Some("https://other.com/x")),
            Some("https://other.com/x".into())
        );
        // 同源路径照常拼回入口。
        assert_eq!(
            resolve_popup_url_for(&nav, Some("https://entry.example.com/notes")),
            Some("https://entry.example.com/notes".into())
        );
    }

    #[test]
    fn popup_missing_or_unsupported_target_returns_none() {
        let nav = proxy_nav("http://127.0.0.1:4567/", None);
        assert_eq!(resolve_popup_url_for(&nav, None), None);
        assert_eq!(resolve_popup_url_for(&nav, Some("about:blank")), None);
        assert_eq!(
            resolve_popup_url_for(&nav, Some("javascript:void(0)")),
            None
        );
        assert_eq!(resolve_popup_url_for(&nav, Some("not a url")), None);
    }

    #[test]
    fn close_adjacency_plan_matches_tab_order_rule() {
        // 4 个页签 [0,1,2,3]，删除后幸存 3 个。
        // 关激活本身：右邻优先（删末位时夹到左邻）。
        assert_eq!(plan_active_after_close(0, 0, 3), (0, true));
        assert_eq!(plan_active_after_close(1, 1, 3), (1, true));
        assert_eq!(plan_active_after_close(2, 2, 3), (2, true));
        assert_eq!(plan_active_after_close(3, 3, 3), (2, true), "删末位 → 左邻");
        // 关激活【之前】的后台 tab：索引前移，不需要显式激活。
        assert_eq!(plan_active_after_close(2, 0, 3), (1, false));
        assert_eq!(plan_active_after_close(3, 1, 3), (2, false));
        // 关激活【之后】的 tab：索引不变。
        assert_eq!(plan_active_after_close(0, 2, 3), (0, false));
        assert_eq!(plan_active_after_close(1, 2, 3), (1, false));
        // 2 页签关 1（幸存 1）：左右邻都是它。
        assert_eq!(plan_active_after_close(0, 0, 1), (0, true));
        assert_eq!(plan_active_after_close(1, 1, 1), (0, true));
        assert_eq!(plan_active_after_close(1, 0, 1), (0, false));
    }

    /// 外部代理开关必须同时挂在宿主与 tab 两个 builder 上（少一个 → 多页签绕过外部代理，
    /// 见 create_host_window 的注释）。只统计 #[cfg(test)] 之前的生产代码段，避免本断言
    /// 自己的字符串被计入（自引用）。
    #[test]
    fn external_proxy_args_on_both_host_and_tab_builders() {
        let full = include_str!("tabs.rs");
        let src = full.split("#[cfg(test)]").next().unwrap_or(full);
        assert_eq!(
            src.matches(".additional_browser_args(").count(),
            2,
            "宿主窗口与 tab 窗口必须各有一处 additional_browser_args"
        );
        assert!(src.contains("crate::config::build_proxy_args()"));
    }

    /// 首屏 bootstrap 触发点必须存在，且导航缓存未就绪时的轮询回调不得把弹窗落点丢成 None。
    ///
    /// 两个都是「静默降级、只在真机上才现形」的缺陷，各配一条静态断言钉住：
    /// · 缺 bootstrap 触发点 → 首个 tab 空等 30s 超时才回落（首次打开极慢）；
    /// · 轮询回调硬编码 None → 首屏就绪前开出的 tab 全落回入口地址（新 tab 都是首页）。
    ///
    /// 只统计 #[cfg(test)] 之前的生产代码段（避免与测试里的注释互相引用）。
    #[test]
    fn first_tab_triggers_bootstrap_and_poll_keeps_start() {
        let full = include_str!("tabs.rs");
        let src = full.split("#[cfg(test)]").next().unwrap_or(full);
        assert!(
            src.contains("bootstrap_for_label(app.clone(), label.to_string())"),
            "首个 tab 必须直接启动完整 bootstrap（否则首次打开要空等 30s）"
        );
        assert!(
            !src.contains("navigate_tab(&a, &l, None)"),
            "轮询回调必须原样保留弹窗落点 start，不能硬编码 None"
        );
    }
}
