//! 移动端底部导航栏（`mobile_bar`，仅移动端）。
//!
//! 移动端没有系统标题栏可自绘（桌面端那条注入的 `titlebar.js` 只在 Windows 注入），
//! 但纯网页壳里「回到首页 / 后退 / 前进」这三类动作在手机上**没有系统级入口**：
//! Android 只有返回键（且本仓库已把它限制成「不退回内置启动页」，见
//! `.github/scripts/patch-android-back.py`），没有前进、也没有「回到入口」。
//! `mobile_bar=true` 时就在屏幕**底部**常驻一条导航条补上这几个动作。
//!
//! 前端真源是 `src/mobilebar.js`，本模块负责四类事：
//!   1. 把「底栏高度 / 是否有页签可切」两个值注入给它（**必须在它之前**注册）；
//!   2. 缓存启动期解析出的首页导航信息，供「首页」按钮复用；
//!   3. 监听前端发来的核心事件并落地（远程页面调不了应用命令，只能发事件 —— 见
//!      `src/mobilebar.js` 文件头与 `capabilities/default.json` 的 `core:event:allow-emit`）；
//!   4. `tabs_enabled=true` 时维护移动端【单 webview 页签】记录（见下）。
//!
//! # 移动端页签：单 webview + 记录
//!
//! Tauri/wry 在 Android **不支持同一窗口挂多个子 webview**（`WebviewBuilder` 文档对
//! Android/iOS 标注 Not supported；2.12 的多窗口走的是 Activity Embedding 平板分屏，
//! 不是全屏页签叠放）。所以移动端的「页签」与桌面端（`tabs.rs`：多个真实 webview
//! 并行、状态各自保留）本质不同：
//!   · 壳侧只持有每个标签的 `{label,url,title}` 记录（[`MobileTabs`]），真正存活的页面
//!     永远只有主 webview 里的当前这一个；
//!   · 新建标签 = 增加一条记录并导航到首页；切换 = 按记录的 url **重新加载**（页面状态
//!     不保留）；关闭当前标签 = 删记录并导航到邻格；首页固定 0 号位、不可关闭（与
//!     桌面端 2026-10-13 起的位次 0 约定一致）；
//!   · 前端经 `web2app:mobile-doc` 上报当前文档 url/标题，壳侧经 eval
//!     `__web2appMobileBar.setTabs([[label,title,active,home],…])` 推回列表面板，
//!     四元组与桌面端 `tabs::get_tabs` 同形。
//!
//! ⚠️ 模块按 `#[cfg(any(mobile, test))]` 编译：
//!   · 生产构建里它**只在移动端存在**，桌面目标（Windows Tauri / Electron 绿色版）不编译
//!     这条路径 —— 这与「配了 `mobile_bar=true` 却被忽略」有本质区别：后者是静默行为，
//!     前者是「这条路根本不存在」，不可能在桌面端长出一条底栏；
//!   · `test` 那一半是为了让**本机 `cargo test` 也能跑到本模块的纯逻辑单测**
//!     （移动端是真机才能验的目标，本机跑不到；判据若只能靠真机，就等于没有护栏）。
//!     与 `bootstrap.rs` 里那组 `#[cfg(any(not(windows), test))]` 同一条写法。
//! ⚠️ 因此本模块里**只能**用跨平台 API，不得引用任何 `#[cfg(mobile)]` / `#[cfg(windows)]`
//!    的项（那样会让桌面端的测试构建编译不过）。
//!
//! # ⚠️⚠️ 本模块的所有入口都跑在 Android **UI 线程**上 ⇒ 禁调阻塞型 webview getter
//!
//! 调用链：`RustWebViewClient.onPageFinished`（UI 线程）→ JNI `Rust.onPageLoaded`
//! （`wry/src/android/binding.rs`，**同步**在该线程执行）→ tauri `on_page_load` 闭包
//! （`lib.rs` 里接线）→ 本模块的 [`on_page_finished`]。事件入口同理，都是 UI 线程。
//!
//! 而 `WebviewWindow::url()` 在 Android 上是
//! `MainPipe::send(GetUrl)` + `rx.recv_timeout(MAIN_PIPE_TIMEOUT = 10s)`
//! （`wry/src/android/mod.rs`），管道的**消费方就是这条 UI 线程**（`onFirstActivityCreateWry`
//! 把 `MAIN_PIPE[0]` 注册成 `ThreadLooper::for_thread()` 的 fd 回调）⇒ 在 UI 线程上问地址
//! **必然**僵住 10 秒，然后超时返回 `Err`。
//!
//! 更糟的是超时**不是终点**：那条 `GetUrl` 仍留在管道里，等 UI 线程腾出来被处理时，
//! `main_pipe.rs` 的 `WebViewMessage::GetUrl` 分支结尾是 `tx.send(url).unwrap()` ——
//! 而接收端已经随超时被丢弃 ⇒ `unwrap()` panic ⇒ 跨 FFI 的 panic 直接 abort 进程。
//!
//! 真机表象（2026-11-01，`tabs_enabled=true` + `mobile_bar=true` 的包）：
//! **打开白屏约 10 秒 → 应用自动退出**。所以本模块里当前地址一律走
//! [`CurrentUrl`] 这份**缓存**（页面加载完成事件与 `web2app:mobile-doc` 事件负责写入），
//! 绝不再向 webview 现问。静态守卫：`contract_tests::mobile_bar_contract` 的
//! `mobile_bar_never_queries_webview_on_ui_thread`。

use std::sync::Mutex;

use tauri::{AppHandle, Listener, Manager};

use crate::config::TABS_ENABLED;
use crate::nav::Navigation;

/// 底栏内容区高度（**CSS 逻辑像素**，不含手势导航条的安全区）。
///
/// 安全区那一段由页面里的 `env(safe-area-inset-bottom, 0px)` 自行补上 —— 它随设备与
/// 导航方式（三大键 / 手势）变化，编译期写死不了。
/// ⚠️ 这是**唯一真源**：前端 `mobilebar.js` 的底栏 CSS、底部让位高度、页面贴底元素上移的
///    距离全部来自这里（经 [`config_script`] 下发），改高度只改这一处。
pub(crate) const BAR_HEIGHT: i32 = 52;

/// 编译期钉住底栏高度的取值范围（比运行时断言更早一步，也不可能被人跳过）：
///   · ≥ 44：触摸目标的最低要求（Apple HIG / Material 同为 44–48dp），矮于此会「点不准」；
///   · ≤ 72：再高就明显占屏幕（手机竖屏常见 640–900 CSS px，72 已经是上限附近）。
const _: () = assert!(BAR_HEIGHT >= 44 && BAR_HEIGHT <= 72);

/// 当前构建是否有「可切换的页签」—— 底栏「列表」按钮的**唯一**显示判据。
///
/// 移动端取打包配置 `tabs_enabled`（[`crate::config::TABS_ENABLED`]）：开启后由本模块的
/// 单 webview 页签记录承接（模块头「移动端页签」一节），不是死按钮。
/// 桌面端（本模块仅以 `test` 形态编译）多页签由 `tabs.rs` 负责，与这里无关。
pub(crate) const fn tabs_available() -> bool {
    TABS_ENABLED
}

/// 注入给页面的配置脚本。
///
/// ⚠️⚠️ 必须排在 `mobilebar.js` **之前**注册（初始化脚本按注册顺序执行）：脚本开头要读
/// `window.__web2appMobileBarCfg`，顺序反了它就会落进「默认值」分支 —— 底栏照画、
/// 但高度与按钮清单不再由壳决定，是最难从表象反推的一类错位。
/// 与 `lib.rs::build_main_window` 里「模式标记必须排在 OVERLAY_SCRIPT 之前」同一条契约。
pub(crate) fn config_script() -> String {
    format!(
        "window.__web2appMobileBarCfg = {{\"h\":{},\"tabs\":{}}};",
        BAR_HEIGHT,
        if tabs_available() { "true" } else { "false" }
    )
}

/// 「首页」按钮的事件名（与 `src/mobilebar.js` 里的字面量逐字一致，契约测试钉着）。
pub(crate) const NAV_HOME_EVENT: &str = "web2app:nav-home";
/// 页签列表里点某一行（切换）的事件名。
pub(crate) const SWITCH_TAB_EVENT: &str = "web2app:switch-tab";
/// 页签列表里点关闭的事件名。
pub(crate) const CLOSE_TAB_EVENT: &str = "web2app:close-tab";
/// 列表面板里点「新建标签」的事件名（无 payload，落点固定为首页）。
pub(crate) const NEW_TAB_EVENT: &str = "web2app:new-tab";
/// 页面请求「在新页签里打开指定 url」的事件名（payload `{url}`）。
///
/// 与 [`NEW_TAB_EVENT`]（无 payload、落首页）区分开：这个事件承接网页里
/// `target="_blank"` 链接与 `window.open(url)`（见 `mobilebar.js::installBlankGuard`）——
/// wry Android 不支持多窗口，不拦截则链接静默无反应，拦截后落到单 webview 页签记录里。
pub(crate) const OPEN_TAB_EVENT: &str = "web2app:open-tab";
/// 打开列表面板时请求壳侧推一份最新行数据（无 payload）。
pub(crate) const REQUEST_TABS_EVENT: &str = "web2app:request-tabs";
/// 页面上报当前文档 `{url,title}`（单 webview 页签靠它刷新活动标签记录）。
pub(crate) const DOC_EVENT: &str = "web2app:mobile-doc";

/// 首页标签的固定 label —— 与「首页固定位次 0」是同一事实的两个面：label 也钉死，
/// 前端四元组的 home 标记与关闭拦截都不用另存状态。
pub(crate) const HOME_TAB_LABEL: &str = "home";

/// 启动期解析出的**首页导航信息**缓存（供底栏「首页」按钮复用）。
///
/// 为什么必须缓存而不是「点一下重新解析一遍」：入口地址可能是【域名数组】（DNS TXT 查询 +
/// 验签解密，实测可达 25s 预算）或走本地反向代理（真实地址是 `127.0.0.1:<随机端口>`）。
/// 页面手里既没有最终地址、也不该每次点首页都重跑一次解析 / 重启代理。
#[derive(Default)]
pub(crate) struct HomeNav(Mutex<Option<Navigation>>);

/// 一条移动端页签记录。
#[derive(Clone, PartialEq, Eq)]
struct MTab {
    /// 前端切换 / 关闭时回传的稳定标识（首页为 [`HOME_TAB_LABEL`]，其余 `m<序号>`）。
    label: String,
    /// 切到这个标签时要导航到的地址（最新一次观测到的文档 url）。
    url: String,
    /// 列表面板里显示的标题（可能为空，展示时回退「首页 / 主机名 / label」）。
    title: String,
}

/// 关闭标签的判定结果：首页挡回；关掉非当前标签只刷列表；关掉当前标签还要导航到邻格。
#[derive(Debug)]
enum CloseOutcome {
    /// 试图关闭首页（位次 0）—— 拒绝。
    Blocked,
    Removed {
        /// `Some(i)` = 关掉的是当前标签，要导航到新的活动位次 i；`None` = 关的是后台记录。
        navigate_to: Option<usize>,
    },
}

/// 移动端页签记录的状态机。
///
/// 纯数据、不碰任何 Tauri / 平台 API：所有分支在本机 `cargo test` 可直接覆盖
///（真机部分只有 eval / navigate 那层薄接线）。
#[derive(Default)]
struct MobileTabs {
    tabs: Vec<MTab>,
    active: usize,
    /// 下一个非首页标签的序号（只增不复用，避免关闭后 label 撞车）。
    seq: u32,
}

impl MobileTabs {
    /// bootstrap 解析成功（首启 / 重试）后调用：记录回到「只有一个首页标签」。
    fn reset_home(&mut self, url: &str) {
        self.tabs = vec![MTab {
            label: HOME_TAB_LABEL.to_string(),
            url: url.to_string(),
            title: String::new(),
        }];
        self.active = 0;
        self.seq = 1;
    }

    /// 新建标签（落在首页地址），返回新标签的位次。
    fn new_tab(&mut self, home_url: &str) -> usize {
        let label = format!("m{}", self.seq);
        self.seq += 1;
        self.tabs.push(MTab {
            label,
            url: home_url.to_string(),
            title: String::new(),
        });
        self.active = self.tabs.len() - 1;
        self.active
    }

    fn index_of(&self, label: &str) -> Option<usize> {
        self.tabs.iter().position(|t| t.label == label)
    }

    /// 切换活动标签：点的就是当前标签时返回 `None`（不导航）。
    fn switch(&mut self, label: &str) -> Option<usize> {
        let i = self.index_of(label)?;
        if i == self.active {
            return None;
        }
        self.active = i;
        Some(i)
    }

    /// 删除一条记录。首页（位次 0）永远 [`CloseOutcome::Blocked`]。
    fn close(&mut self, label: &str) -> CloseOutcome {
        let Some(i) = self.index_of(label) else {
            // 记录已不存在（重复点关闭）：当无事发生，也不需要导航。
            return CloseOutcome::Removed { navigate_to: None };
        };
        if i == 0 {
            return CloseOutcome::Blocked;
        }
        let was_active = i == self.active;
        self.tabs.remove(i);
        if was_active {
            // 活动格被删：左邻补位（i 处现在是原来的右邻，左邻是 i-1），取较小者。
            self.active = self.active.saturating_sub(1).min(self.tabs.len() - 1);
            CloseOutcome::Removed {
                navigate_to: Some(self.active),
            }
        } else {
            if i < self.active {
                self.active -= 1;
            }
            CloseOutcome::Removed { navigate_to: None }
        }
    }

    /// 把当前文档的 url / 标题记进活动标签。
    ///
    /// · 内置启动页 url 一律忽略（启动页不是任何标签的落点）；
    /// · 空标题不覆盖已有标题（文档加载早期 title 常短暂为空）；
    /// · 返回是否真的改了记录 —— 调用方只在变化时推回前端，避免无意义 eval。
    fn observe(&mut self, url: &str, title: Option<&str>) -> bool {
        if is_builtin_url(url) || self.tabs.is_empty() {
            return false;
        }
        let i = self.active.min(self.tabs.len() - 1);
        let mut changed = false;
        if !url.is_empty() && self.tabs[i].url != url {
            self.tabs[i].url = url.to_string();
            changed = true;
        }
        if let Some(t) = title {
            if !t.is_empty() && self.tabs[i].title != t {
                self.tabs[i].title = t.to_string();
                changed = true;
            }
        }
        changed
    }

    /// 推给 `mobilebar.js` 的四元组 JSON：`[[label,title,active,home],…]`（顺序即位次）。
    fn rows_json(&self) -> String {
        let rows: Vec<serde_json::Value> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| {
                serde_json::Value::Array(vec![
                    serde_json::Value::String(t.label.clone()),
                    serde_json::Value::String(display_title(t, i)),
                    serde_json::Value::Bool(i == self.active),
                    serde_json::Value::Bool(i == 0),
                ])
            })
            .collect();
        serde_json::to_string(&serde_json::Value::Array(rows)).unwrap_or_else(|_| "[]".to_string())
    }
}

/// 行标题：页面标题为空时，首页显示「首页」，其余回退到主机名（再退 label）。
fn display_title(tab: &MTab, index: usize) -> String {
    /// U+2028 / U+2029 在 JSON 里合法、但直接嵌进 `<script>` / eval 的 JS 源码会断句，
    /// 列表面板又是 innerHTML 文本，统一替换成空格最省事。
    fn clean(s: &str) -> String {
        s.replace(['\u{2028}', '\u{2029}'], " ")
    }
    if !tab.title.trim().is_empty() {
        return clean(tab.title.trim());
    }
    if index == 0 {
        return "首页".to_string();
    }
    if let Ok(u) = url::Url::parse(&tab.url) {
        if let Some(host) = u.host_str() {
            return host.to_string();
        }
    }
    tab.label.clone()
}

/// 内置启动页地址判定（与 `mobilebar.js::isBuiltinUrl` 同一条规则）：
/// `splash:` 协议 或 wry 改写后的 `http(s)://splash.localhost/`。
fn is_builtin_url(u: &str) -> bool {
    match url::Url::parse(u) {
        Ok(p) => p.scheme() == "splash" || p.host_str() == Some("splash.localhost"),
        Err(_) => false,
    }
}

/// 页签记录的托管状态（与 [`HomeNav`] 并列）。
#[derive(Default)]
pub(crate) struct MobileTabsState(Mutex<MobileTabs>);

/// 主 webview 的**当前地址缓存**（模块头「禁调阻塞型 getter」一节的唯一替代品）。
///
/// 写入方（都发生在 UI 线程，但都只是本地字符串赋值，不碰 webview）：
///   · `on_page_finished`（每次文档加载完成，url 由 wry 经 `on_page_load` payload 给出）；
///   · `doc_reported`（页面 `web2app:mobile-doc` 事件的 `{url}`，覆盖 SPA 内部跳转）。
/// 读取方：首页按钮 / 标签切换 / 标签关闭后的「是否已在同一落点」判定。
///
/// ⚠️ 空串 = 「还没观测到任何真实网页地址」（此时一律按「不在目标页」处理，
///    退化成直接导航 —— 与旧实现读地址失败时的分支一致）。
/// ⚠️ 内置启动页地址不入缓存（`is_builtin_url`）：启动页不是任何标签/首页的落点。
#[derive(Default)]
pub(crate) struct CurrentUrl(Mutex<String>);

/// 读取当前地址缓存（空串 = 未知）。
fn current_url(app: &AppHandle) -> String {
    app.state::<CurrentUrl>()
        .0
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

/// 写入当前地址缓存；空串与内置启动页地址一律忽略。
fn remember_url(app: &AppHandle, url: &str) {
    if url.is_empty() || is_builtin_url(url) {
        return;
    }
    if let Ok(mut g) = app.state::<CurrentUrl>().0.lock() {
        if g.as_str() != url {
            *g = url.to_string();
        }
    }
}

/// 记下本次解析出的首页导航信息（由 `bootstrap` 在解析成功后调用）。
pub(crate) fn set_home_nav(app: &AppHandle, nav: &Navigation) {
    if let Ok(mut g) = app.state::<HomeNav>().0.lock() {
        *g = Some(nav.clone());
    }
    // 首启与「重试 bootstrap」都走这里：页签记录一并复位成「只有首页」，避免上一轮
    // 残留的标签记录把新会话的列表面板弄脏。
    if tabs_available() {
        if let Ok(mut t) = app.state::<MobileTabsState>().0.lock() {
            t.reset_home(&nav.url);
        }
    }
}

/// 主 webview 每次文档加载完成时调用（由 `lib.rs` 的 on_page_load 闭包接线，仅移动端）：
/// 把最终落点 url 记进活动标签（标题由页面事件另补），变了就推回列表面板。
///
/// ⚠️⚠️ `url` **必须由调用方传进来**（wry 的 `on_page_load` payload 就带着它）。
///    绝不许在这里 `window.url()` 现问 —— 本函数跑在 Android UI 线程，而那个 getter 走
///    MainPipe 且消费方就是这条线程：必然僵 10 秒，随后触发 wry 侧
///    `tx.send(url).unwrap()` 的 panic ⇒ abort（真机表象「白屏约 10 秒后自动退出」）。
///    完整机制见模块头「禁调阻塞型 getter」一节；守卫
///    `contract_tests::mobile_bar_contract::mobile_bar_never_queries_webview_on_ui_thread`。
pub(crate) fn on_page_finished(app: &AppHandle, url: &str) {
    // 地址缓存与页签无关（首页按钮 / 切标签都要用它），故这一行在 tabs 判定**之前**。
    remember_url(app, url);
    if !tabs_available() {
        return;
    }
    let changed = app
        .state::<MobileTabsState>()
        .0
        .lock()
        .map(|mut t| t.observe(url, None))
        .unwrap_or(false);
    if changed {
        push_rows(app);
    }
}

/// 注册底栏所需的托管状态与事件监听（在 `setup` 里调用）。
///
/// 这些事件都由前端发**核心事件**（`plugin:event|emit`）到达：主窗加载的是远程页面，
/// Tauri 2 对非本地 origin 的 IPC 强制走 ACL，应用命令无法授权给远程 origin ⇒ 必被拒。
pub(crate) fn setup(app: &AppHandle) {
    app.manage(HomeNav::default());
    app.manage(MobileTabsState::default());
    app.manage(CurrentUrl::default());

    let home = app.clone();
    app.listen(NAV_HOME_EVENT, move |_event| {
        navigate_home(&home);
    });

    let new = app.clone();
    app.listen(NEW_TAB_EVENT, move |_event| {
        create_tab(&new);
    });

    let opn = app.clone();
    app.listen(OPEN_TAB_EVENT, move |event| {
        match open_tab_url_of(event.payload()) {
            Some(url) => create_tab_with_url(&opn, &url),
            None => unsupported_tab_action("打开", None),
        }
    });

    let sw = app.clone();
    app.listen(SWITCH_TAB_EVENT, move |event| {
        match tab_label_of(event.payload()) {
            Some(label) => switch_tab(&sw, &label),
            None => unsupported_tab_action("切换", None),
        }
    });

    let cl = app.clone();
    app.listen(CLOSE_TAB_EVENT, move |event| {
        match tab_label_of(event.payload()) {
            Some(label) => close_tab(&cl, &label),
            None => unsupported_tab_action("关闭", None),
        }
    });

    let req = app.clone();
    app.listen(REQUEST_TABS_EVENT, move |_event| {
        push_rows(&req);
    });

    let doc = app.clone();
    app.listen(DOC_EVENT, move |event| {
        doc_reported(&doc, event.payload());
    });
}

/// 取主窗（移动端永远只有一个 webview 窗口；桌面测试构建里 label 为 "main"）。
fn main_window(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    app.get_webview_window(crate::nav::main_window_label())
}

/// 缓存的首页地址（新建标签的落点）。
fn home_url(app: &AppHandle) -> Option<String> {
    app.state::<HomeNav>()
        .0
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|n| n.url.clone()))
}

/// 把 webview 当前所在 url 记进活动标签（切换 / 新建之前先保住旧标签的落点）。
///
/// ⚠️ 用 [`CurrentUrl`] 缓存而不是 `window.url()`：调用点（事件回调）在 Android 上同样是
///    UI 线程，现问地址会把 UI 线程钉住 10 秒并最终 abort 进程（见模块头）。
fn bind_current_url(app: &AppHandle) {
    let cur = current_url(app);
    if cur.is_empty() {
        return;
    }
    if let Ok(mut t) = app.state::<MobileTabsState>().0.lock() {
        t.observe(&cur, None);
    }
}

/// 新建标签：记录压栈 → 主 webview 导航到首页。
fn create_tab(app: &AppHandle) {
    if !tabs_available() {
        unsupported_tab_action("新建", None);
        return;
    }
    let Some(window) = main_window(app) else {
        return;
    };
    let Some(home) = home_url(app) else {
        // 启动尚未成功就点了新建：与「首页」按钮同策略，重走一次完整 bootstrap。
        dlog!("[mobilebar] 首页导航信息尚未就绪，重走一次完整启动流程");
        crate::nav::bootstrap(app.clone());
        return;
    };
    bind_current_url(app);
    let target = {
        let state = app.state::<MobileTabsState>();
        let Ok(mut t) = state.0.lock() else {
            return;
        };
        let i = t.new_tab(&home);
        t.tabs[i].url.clone()
    };
    navigate_to(app, &window, &target);
    push_rows(app);
}

/// 在新页签里打开**指定 url**（承接网页 target=_blank / window.open，见模块头）。
///
/// 与 [`create_tab`] 同一条「记录压栈 → bind 当前 url → 导航 → 推列表」链路，差别只在落点：
/// 新建标签落首页地址，本函数落页面给的链接。只接受 http(s) —— 非 http(s) 的外跳
/// （tel:/mailto:/weixin:…）交给 `patch-android-schemes.py` 的主框架拦截，不该在这里
/// 被压成一条页签记录。
fn create_tab_with_url(app: &AppHandle, raw_url: &str) {
    if !tabs_available() {
        unsupported_tab_action("打开", None);
        return;
    }
    let raw_url = raw_url.trim();
    if raw_url.is_empty() {
        return;
    }
    let Ok(url) = url::Url::parse(raw_url) else {
        dlog!("[mobilebar] open-tab 的 url 不合法，已忽略：{raw_url}");
        return;
    };
    if url.scheme() != "http" && url.scheme() != "https" {
        dlog!(
            "[mobilebar] open-tab 只承接 http(s) 链接（scheme={}），非网页外跳交给 scheme 拦截",
            url.scheme()
        );
        return;
    }
    let Some(window) = main_window(app) else {
        return;
    };
    bind_current_url(app);
    let target = {
        let state = app.state::<MobileTabsState>();
        let Ok(mut t) = state.0.lock() else {
            return;
        };
        // new_tab 的入参就是新记录的落点 url（首页场景传首页地址），这里传页面给的链接。
        let i = t.new_tab(raw_url);
        t.tabs[i].url.clone()
    };
    navigate_to(app, &window, &target);
    push_rows(app);
}

/// 切换标签：先保住当前标签落点，再让 webview 加载目标标签记录的 url。
fn switch_tab(app: &AppHandle, label: &str) {
    if !tabs_available() {
        unsupported_tab_action("切换", Some(label));
        return;
    }
    let Some(window) = main_window(app) else {
        return;
    };
    bind_current_url(app);
    let target = {
        let state = app.state::<MobileTabsState>();
        let Ok(mut t) = state.0.lock() else {
            return;
        };
        match t.switch(label) {
            Some(i) => t.tabs[i].url.clone(),
            None => return, // 点的就是当前标签
        }
    };
    navigate_to(app, &window, &target);
    push_rows(app);
}

/// 关闭标签：首页被拒；关当前标签要导航到邻格；关后台记录只刷列表。
fn close_tab(app: &AppHandle, label: &str) {
    if !tabs_available() {
        unsupported_tab_action("关闭", Some(label));
        return;
    }
    let outcome = app
        .state::<MobileTabsState>()
        .0
        .lock()
        .map(|mut t| t.close(label))
        .unwrap_or(CloseOutcome::Blocked);
    match outcome {
        CloseOutcome::Blocked => {
            dlog!("[mobilebar] 关闭标签被拒绝：首页固定在 0 号位（label={label}）");
        }
        CloseOutcome::Removed {
            navigate_to: nav_index,
        } => {
            if let Some(i) = nav_index {
                if let Some(window) = main_window(app) {
                    let target = app
                        .state::<MobileTabsState>()
                        .0
                        .lock()
                        .ok()
                        .and_then(|t| t.tabs.get(i).map(|t| t.url.clone()));
                    if let Some(url) = target {
                        navigate_to(app, &window, &url);
                    }
                }
            }
            push_rows(app);
        }
    }
}

/// 页面上报 `{url,title}`：刷新活动标签记录并推回面板（仅在记录变化时）。
///
/// 它同时是 [`CurrentUrl`] 的**第二个写入方**：SPA 的 `pushState` / `replaceState` 跳转
/// 不会再触发文档加载完成事件，只靠 `on_page_finished` 会把缓存留在旧页面上
/// （后果：首页按钮判定「已在首页」误判 → 点首页变 reload 而不是回入口）。
fn doc_reported(app: &AppHandle, payload: &str) {
    #[derive(serde::Deserialize, Default)]
    struct P {
        #[serde(default)]
        url: String,
        #[serde(default)]
        title: String,
    }
    let p = serde_json::from_str::<P>(payload).unwrap_or_default();
    if p.url.is_empty() {
        return;
    }
    // 与页签无关（首页按钮 / 切标签都要用），故在 tabs 判定之前。
    remember_url(app, &p.url);
    if !tabs_available() {
        return;
    }
    let changed = app
        .state::<MobileTabsState>()
        .0
        .lock()
        .map(|mut t| t.observe(&p.url, Some(p.title.as_str())))
        .unwrap_or(false);
    if changed {
        push_rows(app);
    }
}

/// 把最新页签行 eval 推给页面里的 `mobilebar.js`。
///
/// 远程 origin 页面调应用命令会被 ACL 拒，但**壳侧 eval 页面**没有这层限制；
/// 面板开着时 `setTabs` 内部会立即重渲染。
fn push_rows(app: &AppHandle) {
    if !tabs_available() {
        return;
    }
    let Some(window) = main_window(app) else {
        return;
    };
    let json = app
        .state::<MobileTabsState>()
        .0
        .lock()
        .map(|t| t.rows_json())
        .unwrap_or_else(|_| "[]".to_string());
    let js = format!(
        "try{{window.__web2appMobileBar&&window.__web2appMobileBar.setTabs({json});}}catch(e){{}}"
    );
    let _ = window.eval(&js);
}

/// 导航到指定地址：已在同一落点时退化为 reload（与 [`navigate_home`] 同一理由 ——
/// 同地址 navigate 在部分内核上是 no-op，用户会觉得「点了没反应」）。
///
/// ⚠️ 「当前在哪」取 [`CurrentUrl`] 缓存，不用 `window.url()`（UI 线程 + MainPipe 的
///    组合会僵 10 秒后 abort，见模块头）。
fn navigate_to(app: &AppHandle, window: &tauri::WebviewWindow, target: &str) {
    let Ok(url) = url::Url::parse(target) else {
        dlog!("[mobilebar] 标签地址不合法：{target}");
        return;
    };
    if at_same_target(app, &url) {
        let _ = window.eval("try{location.reload();}catch(e){}");
        return;
    }
    if let Err(e) = window.navigate(url) {
        dlog!("[mobilebar] 标签导航失败：{e}");
    }
}

/// 缓存的当前地址是否就是这个落点（缓存缺失 / 解析不了 ⇒ `false` = 直接导航）。
fn at_same_target(app: &AppHandle, target: &url::Url) -> bool {
    match url::Url::parse(&current_url(app)) {
        Ok(cur) => same_target(&cur, target),
        Err(_) => false,
    }
}

/// 事件 payload 里的页签 label（形如 `{"label":"m1"}`）；解析不出来就当未知。
fn tab_label_of(payload: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct P {
        label: String,
    }
    serde_json::from_str::<P>(payload).ok().map(|p| p.label)
}

/// open-tab 事件 payload 里的 url（形如 `{"url":"https://…"}`）；解析不出来就当未知。
/// 空白由调用方 [`create_tab_with_url`] 再 trim + 校验 scheme，这里只负责取出字符串。
fn open_tab_url_of(payload: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct P {
        url: String,
    }
    serde_json::from_str::<P>(payload).ok().map(|p| p.url)
}

/// 页签能力关闭时（旧缓存页面 / 异常事件），给一条**看得见**的结论而不是静默无反应。
fn unsupported_tab_action(kind: &str, label: Option<&str>) {
    dlog!(
        "[mobilebar] {}标签被忽略：当前构建未开 tabs_enabled（label={}）",
        kind,
        label.unwrap_or("未知")
    );
}

/// 底栏「首页」：回到入口地址（已在首页则重新加载）。
///
/// 三条路径：
///   1. 已缓存首页地址 **且** 当前就停在它上面 → `location.reload()`（用户要的是「刷新」，
///      而 `navigate` 到同一个地址在部分内核上是 no-op，点了会像没反应）；
///   2. 已缓存首页地址、当前在别的页面 → 直接导航过去（不重解析、不重启代理）；
///   3. **还没解析过**（用户停在启动页时就点了）→ 重走一次完整 bootstrap。
///      这是唯一会重新解析 / 重启代理的分支，代价可接受：它本来就是「启动尚未成功」的场景。
fn navigate_home(app: &AppHandle) {
    let label = crate::nav::main_window_label();
    let Some(window) = app.get_webview_window(label) else {
        return;
    };

    let cached: Option<Navigation> = app.state::<HomeNav>().0.lock().ok().and_then(|g| g.clone());

    let Some(nav) = cached else {
        // ③ 未就绪：完整重来一次（bootstrap 内部会先终止上一轮残留的代理任务）。
        dlog!("[mobilebar] 首页导航信息尚未就绪，重走一次完整启动流程");
        crate::nav::bootstrap(app.clone());
        return;
    };

    let target = match url::Url::parse(&nav.url) {
        Ok(u) => u,
        Err(e) => {
            dlog!("[mobilebar] 首页地址不合法（{}）：{}", nav.url, e);
            return;
        }
    };

    // ① 已在首页 → 重新加载
    // ⚠️ 「当前在哪」取 [`CurrentUrl`] 缓存，不许 `window.url()`：本函数由 `web2app:nav-home`
    //    事件驱动，而事件回调在 Android 上同样是 UI 线程（详见模块头）。
    if at_same_target(app, &target) {
        let _ = window.eval("try{location.reload();}catch(e){}");
        return;
    }

    // ② 在别的页面 → 导航回首页
    if let Err(e) = window.navigate(target) {
        dlog!("[mobilebar] 首页导航失败：{e}");
    }
}

/// 两个地址是否指向「同一个落点」：`scheme + host + 端口 + 路径` 全等。
///
/// ⚠️ 刻意**不比较** query / fragment：站点常把一次性参数（如 `?t=`、路由 state）挂在
/// 首页地址后面，逐一比对字符串会把「就在这个页面」误判成「不在」 ⇒ 用户点首页没反应。
/// ⚠️ 也不能只比 host：同一站点下的 `/a` 与 `/b` 是不同页面，点首页必须真的回到入口。
fn same_target(a: &url::Url, b: &url::Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port() == b.port()
        && a.path() == b.path()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_script_reports_height_and_tabs_flag() {
        let s = config_script();
        assert!(
            s.starts_with("window.__web2appMobileBarCfg = {"),
            "cfg 脚本必须是给页面赋值的语句：{s}"
        );
        assert!(
            s.contains(&format!("\"h\":{BAR_HEIGHT}")),
            "cfg 脚本必须带上底栏高度真源：{s}"
        );
        // tabs 段必须是字面量 true/false 之一（前端按 `=== true` 判定）
        assert!(
            s.contains("\"tabs\":true") || s.contains("\"tabs\":false"),
            "cfg 脚本必须带上页签可用性标记：{s}"
        );
    }

    #[test]
    fn tabs_available_follows_compile_flag_in_test_build() {
        // 本机 `cargo test` 默认不注入 TABS_ENABLED ⇒ 这里必须是 false（cfg 脚本因此
        // 下发 "tabs":false，前端不画列表按钮）。CI 的 `TABS_ENABLED=true cargo check`
        // 只编译不执行，不会跑到这条断言。
        assert!(
            !tabs_available(),
            "测试构建未注入 TABS_ENABLED，tabs_available() 应为 false"
        );
    }

    #[test]
    fn same_target_compares_scheme_host_port_path_only() {
        let u = |s: &str| url::Url::parse(s).unwrap();
        assert!(same_target(&u("https://a.com/"), &u("https://a.com/")));
        assert!(same_target(&u("https://a.com/"), &u("https://a.com")));
        // 一次性参数不影响「是不是这个页面」
        assert!(same_target(&u("https://a.com/?t=1"), &u("https://a.com/")));
        assert!(same_target(&u("https://a.com/#x"), &u("https://a.com/")));
        // 端口 / scheme / 路径 / 主机不同 ⇒ 不是同一个落点
        assert!(!same_target(
            &u("https://a.com:8443/"),
            &u("https://a.com/")
        ));
        assert!(!same_target(&u("http://a.com/"), &u("https://a.com/")));
        assert!(!same_target(&u("https://a.com/b"), &u("https://a.com/")));
        assert!(!same_target(&u("https://b.com/"), &u("https://a.com/")));
        // 代理模式下首页是 127.0.0.1:<随机端口>，与上游域名**不**同落点
        assert!(!same_target(
            &u("http://127.0.0.1:51234/"),
            &u("https://a.com/")
        ));
    }

    #[test]
    fn tab_label_of_parses_label_or_gives_up() {
        assert_eq!(
            tab_label_of("{\"label\":\"tab-1\"}").as_deref(),
            Some("tab-1")
        );
        assert_eq!(tab_label_of("not json"), None);
        assert_eq!(tab_label_of("{}"), None);
    }

    #[test]
    fn event_names_are_stable() {
        // 事件名是跨文件契约（前端字面量 ↔ 本模块常量），改这里必须同步改 mobilebar.js。
        assert_eq!(NAV_HOME_EVENT, "web2app:nav-home");
        assert_eq!(SWITCH_TAB_EVENT, "web2app:switch-tab");
        assert_eq!(CLOSE_TAB_EVENT, "web2app:close-tab");
        assert_eq!(NEW_TAB_EVENT, "web2app:new-tab");
        assert_eq!(OPEN_TAB_EVENT, "web2app:open-tab");
        assert_eq!(REQUEST_TABS_EVENT, "web2app:request-tabs");
        assert_eq!(DOC_EVENT, "web2app:mobile-doc");
    }

    #[test]
    fn open_tab_url_of_parses_url_or_gives_up() {
        assert_eq!(
            open_tab_url_of("{\"url\":\"https://a.com/x\"}").as_deref(),
            Some("https://a.com/x")
        );
        assert_eq!(open_tab_url_of("not json"), None);
        assert_eq!(open_tab_url_of("{}"), None);
        // 数字 / null 等非字符串 url：反序列化失败 = 未知（不当成合法落点）。
        assert_eq!(open_tab_url_of("{\"url\":123}"), None);
    }

    /// 页签记录状态机：与桌面端同一条「首页固定 0 号位」契约，加上单 webview 模型特有的
    /// 「切换/新建前由接线层把当前 url 记进活动标签」前提（observe）。
    #[test]
    fn mobile_tabs_lifecycle_and_home_pin() {
        let mut t = MobileTabs::default();
        t.reset_home("https://a.com/");

        // 初始：只有首页，位次 0，活动。
        assert_eq!(t.tabs.len(), 1);
        assert_eq!(t.tabs[0].label, HOME_TAB_LABEL);
        assert_eq!(t.active, 0);

        // 首页关不掉。
        assert!(matches!(t.close(HOME_TAB_LABEL), CloseOutcome::Blocked));
        assert_eq!(t.tabs.len(), 1);

        // 用户在首页内浏览，url 被 observe 记进活动标签。
        assert!(t.observe("https://a.com/news", Some("新闻")));
        assert_eq!(t.tabs[0].url, "https://a.com/news");
        assert!(!t.observe("https://a.com/news", Some("新闻"))); // 幂等
        assert!(!t.observe("https://a.com/news", Some(""))); // 空标题不覆盖
        assert_eq!(t.tabs[0].title, "新闻");

        // 新建两个标签：m1 / m2，活动位次随新建推进，序号只增不复用。
        assert_eq!(t.new_tab("https://a.com/"), 1);
        t.observe("https://a.com/m1", Some("页面1"));
        assert_eq!(t.new_tab("https://a.com/"), 2);
        t.observe("https://a.com/m2", Some("页面2"));
        assert_eq!(t.tabs.len(), 3);
        assert_eq!(t.tabs[1].label, "m1");
        assert_eq!(t.tabs[2].label, "m2");

        // 点当前标签不切换。
        assert!(t.switch("m2").is_none());
        // 切回首页：活动位次 0。
        assert_eq!(t.switch(HOME_TAB_LABEL), Some(0));
        // 在首页标签上继续浏览（接线层会 observe）。
        t.observe("https://a.com/again", None);
        assert_eq!(t.tabs[0].url, "https://a.com/again");

        // 关掉后台标签 m1：不导航（navigate_to=None），活动位次仍为 0。
        assert!(matches!(
            t.close("m1"),
            CloseOutcome::Removed { navigate_to: None }
        ));
        assert_eq!(t.tabs.len(), 2);
        assert_eq!(t.active, 0);
        // m2 因 m1 被删而前移到位次 1。
        assert_eq!(t.tabs[1].label, "m2");

        // 关掉当前标签（首页不可能；构造活动在 m2 后关闭它）：导航到左邻（首页）。
        assert_eq!(t.switch("m2"), Some(1));
        match t.close("m2") {
            CloseOutcome::Removed {
                navigate_to: Some(i),
            } => assert_eq!(i, 0),
            other => panic!("关活动标签应导航到左邻，实际 {other:?}"),
        }
        assert_eq!(t.tabs.len(), 1);
        assert_eq!(t.active, 0);

        // 再新建：序号继续增长（m3，不与已关的 m1/m2 撞）。
        assert_eq!(t.new_tab("https://a.com/"), 1);
        assert_eq!(t.tabs[1].label, "m3");
    }

    #[test]
    fn observe_ignores_builtin_and_empty_state() {
        let mut t = MobileTabs::default();
        // 还没有任何标签时 observe 不改东西。
        assert!(!t.observe("https://a.com/", None));
        t.reset_home("https://a.com/");
        // 两种启动页形态都不认。
        assert!(!t.observe("splash://localhost/retry", Some("启动页")));
        assert!(!t.observe("http://splash.localhost/", Some("启动页")));
        assert!(t.observe("https://a.com/x", None));
    }

    #[test]
    fn rows_json_is_four_tuple_with_home_and_active_flags() {
        let mut t = MobileTabs::default();
        t.reset_home("https://a.com/");
        t.new_tab("https://a.com/");
        t.observe("https://a.com/p", Some("内页"));
        // 切到首页，让两个 active 标记都好验。
        t.switch(HOME_TAB_LABEL);
        let s = t.rows_json();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        let rows = v.as_array().unwrap();
        assert_eq!(rows.len(), 2);
        // [label,title,active,home]
        assert_eq!(rows[0][0], HOME_TAB_LABEL);
        assert_eq!(rows[0][1], "首页"); // 首页无标题时回退「首页」
        assert_eq!(rows[0][2], true); // 当前活动
        assert_eq!(rows[0][3], true); // home 标记
        assert_eq!(rows[1][0], "m1");
        assert_eq!(rows[1][1], "内页");
        assert_eq!(rows[1][2], false);
        assert_eq!(rows[1][3], false);
    }

    #[test]
    fn display_title_fallbacks_and_builtin_detection() {
        assert!(is_builtin_url("splash://localhost/"));
        assert!(is_builtin_url("http://splash.localhost/retry"));
        assert!(is_builtin_url("https://splash.localhost/"));
        assert!(!is_builtin_url("https://a.com/"));
        assert!(!is_builtin_url("not a url"));

        let home = MTab {
            label: HOME_TAB_LABEL.into(),
            url: "https://a.com/".into(),
            title: String::new(),
        };
        assert_eq!(display_title(&home, 0), "首页");
        let other = MTab {
            label: "m1".into(),
            url: "https://shop.example.com/p".into(),
            title: String::new(),
        };
        assert_eq!(display_title(&other, 1), "shop.example.com");
        let titled = MTab {
            label: "m1".into(),
            url: "https://a.com/".into(),
            title: "  我的页面  ".into(),
        };
        assert_eq!(display_title(&titled, 1), "我的页面");
    }
}
