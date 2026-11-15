//! 多页签「状态与常量」子模块 —— P4 大文件拆分自 tabs.rs（零行为变更纯代码搬家）。

//! 条目与拆分前逐字一致；与根模块经 use super::* 互访。原模块内私有的条目一律 pub(super)：
//! 可见范围（tabs 模块树）与拆分前的模块内私有完全相同，只是搬家、不放宽对外可见性。

use super::*;

use std::time::Instant;

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
    /// 是否已提交过**真实网页文档**（tab 首文档是本地占位页，不算）。
    ///
    /// 由 `src/tab_doc_ready.js` 在真实文档创建时经核心事件回传（占位页早退），落地见
    /// [`mark_tab_doc_ready`]。**附件下载不产生文档** ⇒「只为下载而开的 tab」永远是 `false`，
    /// 下载落盘 / 取消后据此自动关页签（见 `window.rs::close_tab_after_download`，与
    /// Electron 侧 `tab-order.ts::shouldCloseTabAfterDownload` 同一判据）。
    pub committed: bool,
    /// 打开本 tab 的那个 tab label（只有 `target="_blank"` / `window.open()` 建的 tab 才有；
    /// 首个 tab 与 `+` 新建为 None）。下载后自动关页签时据此把激活态拉回「点击时的页面」。
    pub opener: Option<String>,
}

/// 标题回传事件名（tab 远程页面 → Rust）。与 `src/tab_title.js` 里的字面量逐字一致，
/// 受 `ipc_command_registry` / `capability_contract` 静态守卫钉着。
pub(crate) const TAB_TITLE_EVENT: &str = "web2app:tab-title";

/// tab 首屏（本地占位文档）上「重试」按钮触发的事件 → Rust 为该 tab 重走 bootstrap。
/// 与 `src/loading.js` 字面量逐字一致（静态守卫）。payload 同样带一次性 token 防伪。
pub(crate) const TAB_RETRY_EVENT: &str = "web2app:retry-bootstrap";

/// tab 页面里 Ctrl+W 触发的事件 → Rust 关闭该 tab（`src/tab_keys.js`）。
/// 与 `src/tab_keys.js` 里的字面量逐字一致（静态守卫 `tab_close_shortcut_contract`）。
/// payload 同样带一次性 token 防伪（app.listen 的 Event 不带发送窗口标识）。
pub(crate) const TAB_CLOSE_EVENT: &str = "web2app:close-tab";

/// tab 页面「真实网页文档已提交」事件名（`src/tab_doc_ready.js` → Rust）。
/// 与脚本里的字面量逐字一致（静态守卫在 `contract_tests/capability_contract.rs`）。
pub(crate) const TAB_DOC_READY_EVENT: &str = "web2app:tab-doc-ready";

/// 下载落盘 / 取消后，是否应自动关闭**发起下载的 tab**（纯规则，可单测）。
///
/// 对齐浏览器对 `target="_blank"` 指向附件地址的行为：新页签只为承接这次下载而开，
/// 下载响应不提交文档（没有可显示的页面）⇒ 下载开始后浏览器把它关掉、回到点击时的页面。
///
/// 判据只有「是否提交过真实文档」：未提交 → 关；已提交 → **不关**（页面里点下载时浏览器
/// 同样保留当前页签，关掉会弄丢用户正在看的页面）。首页那一签由 `close_tab_inner` 拒绝，
/// 本函数不重复判定（「首页判定」全项目只有 [`is_home_index`] 一份实现）。
///
/// ⚠️ 双壳单一判据：Electron 侧同一条规则在 `tab-order.ts::shouldCloseTabAfterDownload`，
/// 改这里必须同步改那边（探针 `probe --tabs` 的 P14 双向断言守着）。
pub(super) fn should_close_tab_after_download(committed: bool) -> bool {
    !committed
}

/// 「弹窗兜底重复下载」的识别窗口（毫秒）。
///
/// 站点兜底是近同步发生的（`window.open()` 返回 null 后立即 `location.href = 同一地址`），
/// 3s 足够宽；窗口越短越不容易误伤「用户真的又点了一次同一个下载链接」。与 Electron 侧
/// `tabs.ts::POPUP_FALLBACK_WINDOW_MS` 同一数值。
pub(super) const POPUP_FALLBACK_WINDOW_MS: u64 = 3000;

/// 「弹窗兜底重复下载」纯判据（可单测）：登记时的打开者就是本次下载的发起来源，且未超窗口。
///
/// ⚠️ 不能只按「打开者 + 时间窗」丢：页面在弹框遮挡前仍可能程序化批量下载多个文件，
///   那些地址不同，必须放行（地址逐字相同这条由调用方判定）。
pub(super) fn popup_fallback_is_duplicate(
    opener_in_record: Option<&str>,
    label: &str,
    age_ms: u64,
) -> bool {
    opener_in_record == Some(label) && age_ms < POPUP_FALLBACK_WINDOW_MS
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
    /// 每 tab 的实际网页标题缓存：label -> title（由 tab 页面的 `web2app:tab-title`
    /// 核心事件回传，落地见 [`report_tab_title`]）。
    pub titles: Mutex<HashMap<String, String>>,
    /// `window.open` 目标 URL → (打开者 tab label, 登记时刻)。
    ///
    /// WebView2 的 `NewWindowRequested` 被 `SetHandled(true)` 收走后（未 `SetNewWindow`），
    /// 页面里的 `window.open()` 返回 null；站点常据此判定「弹窗被拦截」并立刻对同一地址做
    /// `location.href` 兜底 —— 同一次点击两次下载、两个「另存为」，点掉第一个后第二个还在。
    /// 登记用于在那条兜底下载到达时识别并取消（见 `window.rs::should_drop_popup_fallback`）。
    pub expected_popup_downloads: Mutex<HashMap<String, (String, Instant)>>,
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
