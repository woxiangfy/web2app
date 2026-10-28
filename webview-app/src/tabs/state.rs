//! 多页签「状态与常量」子模块 —— P4 大文件拆分自 tabs.rs（零行为变更纯代码搬家）。

//! 条目与拆分前逐字一致；与根模块经 use super::* 互访。原模块内私有的条目一律 pub(super)：
//! 可见范围（tabs 模块树）与拆分前的模块内私有完全相同，只是搬家、不放宽对外可见性。

use super::*;

pub(crate) const TAB_PREFIX: &str = "tab-";

/// tab 窗口当前是否「因宿主最小化 / 隐藏而被收起」。
///
/// 用途：宿主最小化时 tab 窗口不会跟着最小化（它们是独立顶层窗口），必须手动 `hide()`，
/// 否则桌面/任务栏上会残留没有标签栏的内容窗口。宿主恢复时再 `show()` 回来。
///
/// 为什么需要这个标志：宿主**每次** `Resized`（拖拽缩放）都会走一遍同步逻辑，
/// 若不加状态判断，就会在缩放过程中反复 `show/hide` 所有 tab 窗口 → 肉眼可见闪烁。
/// 只在「收起/还原」这个状态真正翻转时才动窗口。
pub(super) static TABS_SUSPENDED: AtomicBool = AtomicBool::new(false);

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
/// （首页占其中一个名额，见 [`is_home_index`]。）
pub(super) const MAX_TABS: usize = 32;

/// 首页页签在标签栏里的**位次**：永远是第 0 个。
///
/// 业务含义：多页签模式下打开默认入口地址（app_url）的那一签是「首页」—— 它**固定在第一位**
/// 且**不允许关闭**，页签名字前带一个房子图标作标记（标记的渲染在 `shell.html::render`）。
///
/// 为什么判「位次」而不是给 [`TabInfo`] 加一个 `home: bool` 字段：首页的定义就是「入口那一签」，
/// 而入口那一签是本进程**第一个**建出来的 tab，且它不可关闭 ⇒ 它永远占着下标 0。写成位次判定
/// 就同时拿到三条性质，不需要再多一份可能漂移的状态：
///   · 固定第一位 —— 列表其余项只做尾部追加（`tabs.push`）与按位删除，0 号位不会被顶走；
///   · 一定存在   —— 唯一的删除入口 [`close_tab_inner`] 对本位次直接拒绝；
///   · 自动补齐   —— [`set_tabs_visible`] 在「一个 tab 都没有」时会补建一个，补出来的自然就是首页。
///
/// ⚠️ 双壳单一真源：Electron 侧必须给出同一个答案（`tabs.ts::isPinnedHome`）。改这里必须同步
/// 改那边，并跑 Electron 侧 `probe --tabs`（它逐条断言首页的图标 / 无关闭按钮 / 拒绝关闭）。
pub(super) fn is_home_index(i: usize) -> bool {
    i == 0
}

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
