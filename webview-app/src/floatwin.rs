//! 桌面悬浮图标（悬浮球，仅 Windows）。
//!
//! 需求（v1.69 纠正 v1.67 的方向错误；v1.81 重新定稿渲染路线）：桌面上一个**悬浮图标**——
//! 独立置顶、不占任务栏的小圆球，① 可由设置页开关 `float_enabled` 创建 / 销毁，设置页按钮
//! 与托盘菜单可显示 / 隐藏；② 可按住拖到任意位置；③ 拖到屏幕**四边**（上 / 下 / 左 / 右）
//! 松手即**半隐藏吸附**（只露半球，鼠标悬停展开、移开缩回；左 / 右沿 x 半隐藏、上 / 下沿 y
//! 半隐藏，就近择一）；④ **单击球 = 切换【原来的主窗口】显隐**（见 [`crate::tray::toggle_window`]）；
//! ⑤ **右键球 = 关闭悬浮图标功能并唤回主窗口**（见 [`disable_float_from_icon`]）。
//! ⚠️ ⑤ 不能退化成「只隐藏球」：球是**唯一**可见入口时（没启用托盘 + 关闭窗口即隐藏到球），
//! 把它藏起来就是「进程还在跑、桌面上却再也唤不出窗口」的死局（2026-10-11 使用者实测）。
//!
//! ## ⚠️⚠️ v1.81 根治：球窗 = **Win32 分层窗口 + 位图 alpha**（不再是 WebView2）
//!
//! 毛刺三源都在「浏览器 CSS 画圆」这条链上：① 透明窗上 `border-radius` 圆角裁剪的抗锯齿
//! （Chromium 对白底合成再套 alpha 掩码 ⇒ 白边 / 毛刺）；② `filter: brightness()` 状态强制
//! 合成层、`radial-gradient` 渐变抖动；③ `translate(-50%,-50%)` 在非 1/4 整数倍 DPI 下落到
//! 分数物理像素（阶梯毛刺）。v1.77b–v1.80 反复失败正是因为**圆的边缘始终经过浏览器裁剪 /
//! 合成管线**。v1.81 起球窗是原生 [`native`] 分层窗口：
//!   · 视觉：`WS_EX_LAYERED` + `UpdateLayeredBitmap` 提交 **32bpp 预乘 ARGB** 位图；圆盘由
//!     4×4 超采样画出（平涂白 / 黑 + 1px 半透明描边），圆的轮廓**就是位图 alpha** ——
//!     系统合成器逐像素合成，**不经任何 CSS / 裁剪 / 区域**；
//!   · 命中：分层窗口天然按像素 alpha 穿透（alpha=0 自动透传点击）⇒ 旧的
//!     `WM_NCHITTEST` 子类化、命中圆原子量、「活柱」、WebView2 最小宽 hack **全部退役**；
//!   · 交互：鼠标消息全部在 WndProc 内处理（拖动 / 单击 / 右键 / 悬停），不再有球页 → Rust
//!     的令牌事件链（[`FloatState`] 里的 token 与 `capabilities/float-icon.json` 一并退役）。
//!
//! 失败链存档（勿再尝试）：
//!   · v1.77b `SetWindowRgn` 严格裁圆 ⇒ 1bit 硬边切 AA，白锯齿；
//!   · v1.78 区域四边外扩 4px ⇒ 半透明像素被压平白底，环带漏浅色异形；
//!   · v1.79 页面自绘不透明黑底 ⇒ 异形消失但窗口轮廓仍是整像素阶梯（暗锯齿）；
//!   · v1.80 真逐像素 alpha（`WS_EX_NOREDIRECTIONBITMAP`）—— 合成路径对了，但圆角仍在
//!     Chromium 的 border-radius 裁剪里，浅色壁纸下仍有极细白边（用户实测）。
//!
//! ## 窗口扩展样式（不抢前台 + 真不进任务栏）
//! `WS_EX_NOACTIVATE`（显示 / 点击都不激活，鼠标事件照常投递）+ `WS_EX_TOOLWINDOW`
//! （不进任务栏 / Alt+Tab）。v1.77 的 `ITaskbarList::DeleteTab` 补丁不再需要：那是在 tao
//! 用缓存标志整体重写 `GWL_EXSTYLE` 之后的补救；原生窗口的样式**创建时一次定死**，
//! 没有任何框架会再改它。
//!
//! ## 主题（背景自适应白 / 黑）
//! 旧球面是蓝色渐变（`radial-gradient(#6a9bff …)`），与系统观感无关。v1.81 起读
//! `HKCU\…\Themes\Personalize\AppsUseLightTheme` 决定**平涂白 / 黑底盘**，
//! 并监听 `WM_SETTINGCHANGE` / `WM_DWMCOLORIZATIONCHANGED` 重绘。
//!
//! ## 半隐藏的几何约定
//! 吸附后 `x/y` 持久化的是【展开态】左上角；半隐藏只是把窗口整体平移到屏外、留 `PEEK`
//! 物理像素（= 球宽 / 高的一半）在屏内，尺寸不变。落盘只需多记一条 [`crate::store::DockEdge`]。
//! 所有几何（拖动 / 吸附 / 钳制 / 落盘 / 恢复）一律以**球窗矩形**为准（窗口 == 球）。
//!
//! ## 与 Electron 壳的关系
//! Electron 没有原生窗口可用，仍走共享页 `src/floaticon.html`，但同样改成 **canvas 超采样
//! alpha 位图**（浏览器内上限）。`floaticon.html` 与 `floaticon://` 协议在 **Tauri 侧已退役**，
//! 该文件改为 **Electron 专属**（例外登记见根 `AGENTS.md` §3.4.1，仿 `mobilebar.js`）。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use tauri::Manager;

use crate::store::{DockEdge, FloatWinGeom};

// ===========================================================================
// 常量
// ===========================================================================

/// 悬浮球的固定边长（**逻辑**像素）：窗口 == 球 == 命中区，三者严格重合。
const ICON_SIZE_LOGICAL: f64 = 56.0;
/// 球内图标的边长（逻辑像素）：占球径 36/56 ≈ 64%，与旧页面 `#w2f-ball img` 同比例。
const ICON_INSET_LOGICAL: f64 = 36.0;
/// 半隐藏时留在屏幕内的宽度 / 高度（逻辑像素）= 球边长的一半，即真正的「半」隐藏。
const PEEK_LOGICAL: f64 = 28.0;
/// 距屏幕边多近（逻辑像素）即判定为「吸附」。
const SNAP_LOGICAL: f64 = 28.0;
/// 默认摆放时相对屏幕边缘的留白（逻辑像素）。
const MARGIN_LOGICAL: f64 = 12.0;
/// 拖动过程中球在光标所在显示器上至少要留在屏内的宽度 / 高度（逻辑像素）：
/// 既允许把球拖过屏缘跨到相邻显示器，又保证始终抓得回来、不会被整个甩出屏幕。
const MIN_VISIBLE_LOGICAL: f64 = 16.0;
/// 鼠标离开展开态的球后，延迟多久再缩回（毫秒）：
/// ① 鼠标在边缘小幅移动时进出事件会连环触发，延迟把抖动吸收；
/// ② 定时器到期还会再校验真实光标位置（不在「缩回后矩形」内才真缩回）。
const COLLAPSE_DELAY_MS: u64 = 300;
/// 恢复几何时认可的最小物理边长（球 56 逻辑像素在 100% 缩放下 = 56 物理像素）。
const MIN_PHYS: u32 = 40;
/// 拖动阈值（逻辑像素）：按下点到当前点的 |dx|+|dy| 超过它才算拖动，否则松手算单击。
/// 与旧球页 `DRAG_THRESHOLD = 5` 逐字一致。
const DRAG_THRESHOLD_LOGICAL: f64 = 5.0;
/// 单击去抖窗口（毫秒）：系统双击会送来两次松手，两次 toggle 相互抵消，用户只看到主窗
/// 「闪现」一下。与旧球页 `CLICK_DEBOUNCE_MS = 250` 逐字一致。
const CLICK_DEBOUNCE_MS: u64 = 250;

// ===========================================================================
// 运行期状态
// ===========================================================================

/// 一次吸附的运行期状态：吸附边 + 所在显示器物理边界 + 当前是否处于半隐藏（缩回）态。
#[derive(Clone, Copy)]
struct Dock {
    edge: DockEdge,
    mx: i32,
    my: i32,
    mw: i32,
    mh: i32,
    collapsed: bool,
}

/// 悬浮球的进程级托管状态（`app.manage`）。
///
/// v1.81 起不再有球页 → Rust 的事件链，**一次性令牌已退役**（连同
/// `capabilities/float-icon.json`）：鼠标交互直接发生在原生窗口的 WndProc 里，
/// 不存在「被改写的页面伪造事件」这一攻击面。
#[derive(Default)]
pub(crate) struct FloatState {
    /// 当前吸附状态；None = 自由摆放。
    dock: Mutex<Option<Dock>>,
    /// 指针是否正在拖动球（拖动期间忽略 mouseleave 触发的缩回）。
    moving: AtomicBool,
    /// 按下点的物理屏幕坐标。
    ///
    /// 只在按下时暂存，**不**展开吸附态、**不**置 moving；越过拖动阈值的第一帧 move 才取走：
    /// 先展开（若处于半隐藏）、再用「按下点 − 展开后窗口原点」算出 anchor。
    /// 绝不能在第一帧 move 才用当前光标记锚点 —— 那时光标已离开按下点若干像素，
    /// 首帧窗口会纹丝不动（合成指针真机取证：第一帧目标坐标 == 当前原点）。
    press: Mutex<Option<(i32, i32)>>,
    /// 本次拖动的抓取点偏移（按下点物理位置 − 展开后窗口左上角物理位置）。
    ///
    /// 越过阈值的第一帧 move 由 press 计算，之后每帧 `窗口位置 = 光标 − 偏移`。
    /// 绝对坐标自纠错：丢帧 / 晚到都不会累积误差。v1.69 旧实现上报相对本窗视口的
    /// clientX 增量，而窗口每帧都在 set_position，下一帧增量被窗口自身位移抵消，
    /// 稳态只有半速跟手且来回抖（真机表现「拖不动 / 乱跳」）。
    anchor: Mutex<Option<(i32, i32)>>,
    /// 悬停缩回的「代际」计数器：每次展开 / 隐藏 / 销毁 / 起拖都 +1，
    /// 使此前排队等待的延迟缩回任务全部作废。
    peek_gen: AtomicU64,
    /// 最近一次单击的时间戳（毫秒，`Instant` 的单调流逝量）：单击去抖用。
    last_click_ms: AtomicU64,
}

// ===========================================================================
// 子模块
// ===========================================================================

mod events;
mod native;

use events::*;
use native::*;

/// 对外引用路径不变（`crate::floatwin::*`）：子模块中对外的条目在此重导出。
pub(crate) use events::{dock_state, inject_page_event};
pub(crate) use native::{
    ball_hwnd, ball_rect, ball_scale, ball_visible, cursor_screen_pos, monitor_bounds_at,
    primary_monitor_bounds,
};

// ===========================================================================
// 对外入口（设置命令 / 托盘 / 启动 / 退出 / 关闭语义 调用）
// ===========================================================================

/// 显示悬浮球：已存在则显现（吸附态先缩回成半球）；不存在则建原生球窗。
///
/// ⚠️ 建窗必须切到主线程：原生窗口的消息循环在 Tauri 主线程，从 IPC 命令线程直接
/// `CreateWindowExW` 会得到一个没有消息泵的窗口。沿用「后台线程 + `run_on_main_thread`」
/// 的排队模式（与 `settings::open_settings_window` 同理），同时天然保证 create / destroy
/// 的先后顺序不会被并发调用打乱。
pub(crate) fn show_float_window(app: &tauri::AppHandle) {
    // 任何排队中的延迟缩回都作废（用户主动唤球，不能 300ms 后自己又缩回去）。
    app.state::<FloatState>()
        .peek_gen
        .fetch_add(1, Ordering::SeqCst);
    on_main(app, |app| {
        if ball_exists() {
            show_ball();
            // ⚠️ 刻意**不** SetFocus：球窗带 WS_EX_NOACTIVATE，抢焦点是真机实测过的缺陷。
            // 重新显现时若处于吸附态，先缩回成半球（悬停再展开）。
            if let Some(dock) = current_dock(app) {
                if dock.edge != DockEdge::None {
                    collapse(app, dock);
                }
            }
            return;
        }
        build_ball_on_main(app);
    });
}

/// 彻底销毁悬浮球（设置里关掉开关时调用）。几何已随拖动 / 退出落盘，下次开启原位恢复。
pub(crate) fn destroy_float_window(app: &tauri::AppHandle) {
    persist_geometry(app);
    on_main(app, |app| {
        destroy_ball();
        reset_runtime_state(app);
    });
}

/// 清掉全部运行期（非持久化）状态：吸附 / 拖动 / 按下点 / 抓取偏移，并作废旧一代延迟缩回任务。
fn reset_runtime_state(app: &tauri::AppHandle) {
    if let Ok(mut d) = app.state::<FloatState>().dock.lock() {
        *d = None;
    }
    if let Ok(mut a) = app.state::<FloatState>().anchor.lock() {
        *a = None;
    }
    if let Ok(mut p) = app.state::<FloatState>().press.lock() {
        *p = None;
    }
    let st = app.state::<FloatState>();
    st.moving.store(false, Ordering::SeqCst);
    st.peek_gen.fetch_add(1, Ordering::SeqCst);
}

/// 隐藏（不销毁）悬浮球——**托盘菜单「悬浮图标」的显隐 toggle** 与 **Alt+F4 / 系统关闭**
/// （WndProc 的 `WM_CLOSE` 拦截）走这里。
///
/// ⚠️ **球上右键不走这里**（v1.76 起改走 [`disable_float_from_icon`]）：右键只隐藏会留下
/// 「进程还在跑、桌面上却再无入口」的死局（见该函数与模块文档 ⑤）。
/// 上述两条老路径都不改设置，且都还有别的入口（托盘菜单 / 唤醒快捷键 / 设置页），故保持原样。
///
/// 吸附态（dock）保留：再次显现时恢复成半球；只清一次性的拖动 / 悬停运行期状态。
pub(crate) fn hide_float_window(app: &tauri::AppHandle) {
    persist_geometry(app);
    on_main(app, |app| {
        hide_ball();
        if let Ok(mut a) = app.state::<FloatState>().anchor.lock() {
            *a = None;
        }
        if let Ok(mut p) = app.state::<FloatState>().press.lock() {
            *p = None;
        }
        // 收窗即清拖动标志并作废延迟缩回：拖动途中被隐藏时松手不会再来，残留的
        // moving=true 会让下一次悬停展开 / 移开缩回永久失效。
        let st = app.state::<FloatState>();
        st.moving.store(false, Ordering::SeqCst);
        st.peek_gen.fetch_add(1, Ordering::SeqCst);
    });
}

/// 球上**右键**的完整语义（v1.76）：关闭悬浮图标功能 + 唤回主窗口。
///
/// 三步，顺序有意为之：
///   1. **先关功能**（`float_enabled` 置 false：写内存态 + 落盘 + 同步已打开的设置页复选框）。
///      判据先落地 —— 即便后面两步出问题，也不会退回到「进程因 `float_enabled` 驻留、
///      桌面上却没有任何入口」的状态；
///   2. **销毁**球窗（[`destroy_float_window`]，内部先落盘几何）。刻意用 destroy 而非
///      hide：功能已关，留一个隐藏的球窗既没意义，又会继续挡住「所有窗口关闭 ⇒ 退出」；
///   3. **唤回主窗口**（[`crate::tray::show_window`]：取消最小化 + 显示 + 聚焦，多页签模式
///      一并同步各 tab 窗可见性）。用户点了「关闭悬浮图标」，下一步必须是「回到窗口」。
///
/// ⚠️⚠️ 为什么必须这样（而不是像 v1.75 及以前那样只 `hide_float_window`）：常见组合
///    `tray_enabled:false / minimize_on_close:false / float_enabled:true` 下，主窗关闭
///    只是「隐藏到球」（驻留理由②），球就成了桌面上**唯一**的可见入口；此时右键把球一藏，
///    进程还在跑（`float_enabled` 仍为 true ⇒ 继续驻留）却再也唤不出任何窗口
///    —— 2026-10-11 使用者实测。右键在用户直觉里本来就是「我不要这个球了」，
///    那就该顺手回到主窗口，而不是把自己藏进一个没有出口的状态。
///
/// 置设置失败（磁盘只读等）**不阻断**后续两步：先把窗口还给用户，再让日志留痕。
pub(crate) fn disable_float_from_icon(app: &tauri::AppHandle) {
    if let Err(e) = crate::settings::set_float_enabled(app, false) {
        dlog!("[float] 右键关闭悬浮图标功能：写入设置失败（仍继续销毁球窗并唤回主窗）：{e}");
    }
    destroy_float_window(app);
    crate::tray::show_window(app);
}

/// 托盘菜单用的显隐切换：可见则隐藏，否则显示。
pub(crate) fn toggle_float_window(app: &tauri::AppHandle) {
    if ball_visible() {
        hide_float_window(app);
    } else {
        show_float_window(app);
    }
}

/// 退出时持久化悬浮球几何（lib.rs 的 `ExitRequested` 调用）。
pub(crate) fn persist_float_geometry(app: &tauri::AppHandle) {
    persist_geometry(app);
}

// ===========================================================================
// 持久化
// ===========================================================================

/// 抓取当前悬浮球几何落盘（读-改-写，只替换 `float_window` 段）。
/// 吸附态下存【展开态】坐标（半隐藏的偏移运行期算），宽高恒为球的物理边长。
fn persist_geometry(app: &tauri::AppHandle) {
    let Some((mut x, mut y, w, h)) = ball_rect() else {
        return;
    };
    let dock = current_dock(app);
    if let Some(d) = dock {
        // rect 此刻可能是缩回到屏外的坐标；落盘展开态坐标（按吸附轴取）。
        if matches!(d.edge, DockEdge::Left | DockEdge::Right) {
            x = expanded_x(d.edge, d.mx, d.mw, w);
        } else if matches!(d.edge, DockEdge::Top | DockEdge::Bottom) {
            y = expanded_y(d.edge, d.my, d.mh, h);
        }
    }
    let geom = FloatWinGeom {
        x,
        y,
        width: w as u32,
        height: h as u32,
        edge: dock.map(|d| d.edge).unwrap_or(DockEdge::None),
    };
    if let Err(e) = crate::store::update(app, |doc| doc.float_window = Some(geom)) {
        dlog!("[float] 写入悬浮球几何失败：{e}");
    }
}

// ===========================================================================
// 纯函数（几何决策，单测覆盖）
// ===========================================================================

/// 距屏幕边多近（物理像素）即判定为「吸附」。
pub(crate) fn snap_px(scale: f64) -> i32 {
    (SNAP_LOGICAL * scale).round() as i32
}

/// 半球物理边长（随 DPI 缩放）。
pub(crate) fn peek_px(scale: f64) -> i32 {
    (PEEK_LOGICAL * scale).round().max(10.0) as i32
}

/// 拖动中软钳制的最小可见物理边长（随 DPI 缩放，下限 8px）。
pub(crate) fn min_visible_px(scale: f64) -> i32 {
    (MIN_VISIBLE_LOGICAL * scale).round().max(8.0) as i32
}

/// 球窗的物理边长（56 逻辑像素随 DPI 缩放，下限 32）。
///
/// ⚠️ v1.81 起窗口 == 球，不再有「活柱」（WebView2 最小宽度撑宽宿主窗的时代已结束）。
pub(crate) fn ball_px(scale: f64) -> (i32, i32) {
    (
        (ICON_SIZE_LOGICAL * scale).round().max(32.0) as i32,
        (ICON_SIZE_LOGICAL * scale).round().max(32.0) as i32,
    )
}

/// 点 (px,py) 是否在物理矩形 [rx, rx+rw) × [ry, ry+rh) 内。
fn point_in_rect(px: i32, py: i32, rx: i32, ry: i32, rw: i32, rh: i32) -> bool {
    px >= rx && px < rx + rw && py >= ry && py < ry + rh
}

/// 拖动中的软钳制：允许窗口大部分越过显示器边缘（跨到相邻显示器），
/// 但至少留 `minv` 物理像素在本显示器内，防止整个球被甩出屏幕抓不回来。
/// `lo/hi` 是该轴的显示器起止（x 轴即 mx / mx+mw，y 轴即 my / my+mh）。
fn clamp_visible(v: i32, win_w: i32, lo: i32, hi: i32, minv: i32) -> i32 {
    v.clamp(lo - win_w + minv, hi - minv)
}

/// 展开态左上角 x：贴住吸附边、完整落在屏内。
/// 左 / 右吸附改 x；上 / 下吸附（改 y）时 x 由 [`clamp_x`] 另行钳制，这里回显示器左边。
pub(crate) fn expanded_x(edge: DockEdge, mx: i32, mw: i32, ww: i32) -> i32 {
    match edge {
        DockEdge::Left => mx,
        DockEdge::Right => mx + mw - ww,
        _ => mx,
    }
}

/// 半隐藏态左上角 x：把窗口一半移出屏外，只留 peek 宽在屏内。
/// 上 / 下吸附不改 x，原样回 `mx`（调用方另行钳制）。
pub(crate) fn collapsed_x(edge: DockEdge, mx: i32, mw: i32, ww: i32, peek: i32) -> i32 {
    match edge {
        DockEdge::Left => mx - (ww - peek),
        DockEdge::Right => mx + mw - peek,
        _ => mx,
    }
}

/// 展开态左上角 y：与 [`expanded_x`] 在 y 轴上完全对称。
pub(crate) fn expanded_y(edge: DockEdge, my: i32, mh: i32, wh: i32) -> i32 {
    match edge {
        DockEdge::Top => my,
        DockEdge::Bottom => my + mh - wh,
        _ => my,
    }
}

/// 半隐藏态左上角 y：与 [`collapsed_x`] 在 y 轴上完全对称。
pub(crate) fn collapsed_y(edge: DockEdge, my: i32, mh: i32, wh: i32, peek: i32) -> i32 {
    match edge {
        DockEdge::Top => my - (wh - peek),
        DockEdge::Bottom => my + mh - peek,
        _ => my,
    }
}

/// 纵向把窗口钳制在显示器内（上下各留一点边），避免吸附后窗口顶 / 底超出屏幕。
fn clamp_y(y: i32, wh: i32, my: i32, mh: i32) -> i32 {
    y.clamp(my, (my + mh - wh).max(my))
}

/// 横向把窗口钳制在显示器内：与 [`clamp_y`] 完全对称。
fn clamp_x(x: i32, ww: i32, mx: i32, mw: i32) -> i32 {
    x.clamp(mx, (mx + mw - ww).max(mx))
}

/// 吸附态下球窗的目标左上角：吸附轴按展开 / 半隐藏算，自由轴把当前位置钳在显示器内。
/// 拖动松手吸附、悬停展开 / 移开缩回、DPI 变化重摆共用同一份决策（纯函数，单测覆盖）。
/// `cur` 是球窗当前物理矩形 `(x, y, w, h)`。
fn dock_target_pos(d: Dock, cur: (i32, i32, i32, i32), scale: f64) -> (i32, i32) {
    let (lw, lh) = ball_px(scale);
    let peek = peek_px(scale);
    let x = match d.edge {
        DockEdge::Left | DockEdge::Right => {
            if d.collapsed {
                collapsed_x(d.edge, d.mx, d.mw, lw, peek)
            } else {
                expanded_x(d.edge, d.mx, d.mw, lw)
            }
        }
        _ => clamp_x(cur.0, lw, d.mx, d.mw),
    };
    let y = match d.edge {
        DockEdge::Top | DockEdge::Bottom => {
            if d.collapsed {
                collapsed_y(d.edge, d.my, d.mh, lh, peek)
            } else {
                expanded_y(d.edge, d.my, d.mh, lh)
            }
        }
        _ => clamp_y(cur.1, lh, d.my, d.mh),
    };
    (x, y)
}

/// 判定窗口吸附哪条屏幕边：**四边就近择一**。
///
/// 左 / 右要求窗口纵向与显示器有重叠、上 / 下要求横向有重叠（贴的是别的屏 / 完全在屏外
/// 时不该吸附）；多条边同时进入容差时取 `|gap|` 最小者 —— 这是为了治屏幕**角**上的歧义
/// （把球丢到左上角时，左和上都贴边，必须有个确定的赢家）。平局按
/// left → right → top → bottom 的声明顺序取先者（严格 `<`）。
///
/// `win` / `mon` 都是物理边界 `(x, y, 宽, 高)`；打包成元组是为避免参数过多（clippy）。
fn decide_edge(win: (i32, i32, i32, i32), mon: (i32, i32, i32, i32), snap: i32) -> DockEdge {
    let (wx, wy, ww, wh) = win;
    let (mx, my, mw, mh) = mon;
    if ww <= 0 || wh <= 0 {
        return DockEdge::None;
    }
    let vertical_overlap = wy < my + mh && wy + wh > my;
    let horizontal_overlap = wx < mx + mw && wx + ww > mx;
    let mut best: Option<(i32, DockEdge)> = None;
    let mut consider = |gap: i32, edge: DockEdge| {
        if (-snap..=snap).contains(&gap) {
            let score = gap.abs();
            if best.map_or(true, |(s, _)| score < s) {
                best = Some((score, edge));
            }
        }
    };
    if vertical_overlap {
        consider(wx - mx, DockEdge::Left);
        consider(mx + mw - (wx + ww), DockEdge::Right);
    }
    if horizontal_overlap {
        consider(wy - my, DockEdge::Top);
        consider(my + mh - (wy + wh), DockEdge::Bottom);
    }
    best.map_or(DockEdge::None, |(_, e)| e)
}

/// 吸附边的日志名（旧版还兼作 push_state 的 JS 字面量，v1.81 球页退役后只剩日志用途）。
fn edge_str(e: DockEdge) -> &'static str {
    match e {
        DockEdge::Left => "left",
        DockEdge::Right => "right",
        DockEdge::Top => "top",
        DockEdge::Bottom => "bottom",
        DockEdge::None => "none",
    }
}

// ===========================================================================
// 主线程调度
// ===========================================================================

/// 把一段需要「建窗 / 销毁 / 改窗」的闭包排到 Tauri 主线程执行（原生窗口的消息泵在那里）。
///
/// ⚠️ 调用栈里**绝不能**直接建窗：本函数会从 IPC 命令 / 保存设置的回调栈里被调用，
/// 沿用「后台线程 + `run_on_main_thread`」排队到下一个事件循环轮次（与
/// `settings::open_settings_window` 同一模式），同时保证 create / destroy 的先后顺序。
fn on_main<F: FnOnce(&tauri::AppHandle) + Send + 'static>(app: &tauri::AppHandle, f: F) {
    let a = app.clone();
    std::thread::spawn(move || {
        let am = a.clone();
        let _ = a.run_on_main_thread(move || f(&am));
    });
}

#[cfg(test)]
mod tests;
