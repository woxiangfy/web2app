//! 桌面悬浮图标（悬浮球，仅 Windows）。
//!
//! 需求（v1.69 纠正 v1.67 的方向错误 —— v1.68 只是修了旧「小窗」的三条静默失败）：桌面上一个**悬浮图标**——独立置顶、透明、
//! 不占任务栏的小圆球，① 可由设置页开关 `float_enabled` 创建 / 销毁，设置页按钮与托盘
//! 菜单可显示 / 隐藏；② 可按住拖到任意位置；③ 拖到屏幕左 / 右缘松手即**半隐藏吸附**
//! （只露半球，鼠标悬停展开、移开缩回）；④ **单击球 = 切换【原来的主窗口】显隐**
//! （可见且非最小化 → 隐藏；否则显示 / 取消最小化 / 聚焦，见 [`crate::tray::toggle_window`]，
//! v1.71 由「只显示」改为 toggle）；⑤ **右键球 = 关闭悬浮图标功能并唤回主窗口**
//! （v1.76，见 [`disable_float_from_icon`]：`float_enabled` 置 false 落盘 + 销毁球窗 +
//! 显示主窗）。⚠️ ⑤ 不能退化成「只隐藏球」：球是**唯一**可见入口时（没启用托盘 +
//! 关闭窗口即隐藏到球），把它藏起来就是「进程还在跑、桌面上却再也唤不出窗口」的死局
//! （2026-10-11 使用者实测）。
//!
//! ⚠️ 悬浮球**不是第二个浏览器窗**：它加载的是内置本地页 `floaticon://localhost/`
//! （[`include_str!("floaticon.html")`]；应用图标经同协议 `/icon.png` 提供，
//! 编译期 `include_bytes!("../icons/icon.png")`，打包替换图标后天然跟随），无导航、
//! 无远程内容，也完全不碰 bootstrap / 本地代理（v1.67 的「小窗再开一份网页」已整版废弃）。
//! WebView2 用户数据目录与主窗 / tab 窗**同构显式指定**为应用数据目录（见建窗代码）：
//! 球页只有内置 HTML/PNG，不读不写任何网页会话；而若不指定，wry 回落默认 UDF
//! （`%LOCALAPPDATA%\<bundle>`），该目录在企业受控终端 / 沙箱里可能不可写，第二个
//! WebView2 环境创建失败且错误被 tauri-runtime-wry 的 `log::error!` 静默吞掉，表现为
//! `build()` 照常返回 Ok、球窗 HWND 却从不出现（v1.71 真机判别实验坐实）。
//!
//! ## 窗口扩展样式（不抢前台 + 真不进任务栏）
//! ⚠️⚠️ **所有把球窗显示出来的地方都必须经 [`show_ball`]**，它按
//! `show() → 补扩展样式 → 摘掉任务栏按钮` 的顺序做完：置 `WS_EX_NOACTIVATE`（不抢前台）与
//! `WS_EX_TOOLWINDOW`、清 `WS_EX_APPWINDOW`。顺序不是风格问题 —— tao 0.35 在**显示路径**
//! （`set_visible` / `set_minimized` → `apply_diff`）会用**缓存的窗口标志**整体重写
//! `GWL_EXSTYLE`，而缓存里的 `ON_TASKBAR`（创建时按「窗口无父」置位，与 `skip_taskbar` 无关）
//! 会把 `WS_EX_APPWINDOW` 写回来、抹掉我们补的两位 ⇒ **只改在 `show()` 之前等于没改**
//! （真机读回实测 `0x40118`）。不修的话球会带着任务栏按钮被显示出来，任务栏按钮的原生提示条 /
//! 缩略图浮层会被 always-on-top 球窗压到身后、从透明环带漏出浅色残片（真机现象
//! 「悬浮图标背景异样」，2026-10-11）。详见 [`show_ball`] 与 [`apply_ball_window_styles`]。
//!
//! ## 本地页面如何控制窗口（安全模型）
//! 页面是**本地自定义协议 origin**，但仍沿用最小授权：只发**核心事件**
//! （`core:event:allow-emit`，见 capabilities/float-icon.json，仅 windows `["float"]`），
//! 每个事件带建窗时注入主框架的**一次性令牌**；移动 / 吸附 / 显隐 / 切换主窗全部由
//! Rust 在主线程执行。初始化脚本会注入所有 frame，令牌只挂主框架；本页没有 iframe。
//!
//! ## 半隐藏的几何约定
//! 吸附后 `x/y` 持久化的是【展开态】左上角；半隐藏只是把窗口整体平移到屏外、留 `PEEK`
//! 物理像素（= 球宽的一半）在屏内，尺寸不变。落盘只需多记一条 [`crate::store::DockEdge`]。
//!
//! ## WebView2 最小宽度与「活柱」；视觉 / 命中的彻底分离（v1.71 取证，v1.80 定稿）
//! WebView2 运行时会把控制器宿主 HWND 的宽度强制撑到约 131 逻辑像素（200% DPI 下实测
//! 262 物理），任何 `set_size` 都会被立刻弹回；但高度不设限。我们的球只有 56 逻辑宽 ——
//! 接受这个「宿主窗」，真正的球是宿主内一个 56 逻辑宽的【活柱】：右吸附时活柱贴宿主窗
//! 右边、其余贴左边（偏移见 [`live_offset`]）。宿主被撑宽的部分与球的四角既不该看见、
//! 也不该吃到鼠标。v1.71→v1.80 真机四轮教训换来的正解是**视觉与命中用两套机制各管各**：
//!
//! ### 视觉：真逐像素 alpha（`.transparent(true).no_redirection_bitmap(true)`）
//! `WS_EX_NOREDIRECTIONBITMAP` 让宿主窗没有 GDI 重定向表面，DWM 直接把 WebView2 的
//! DirectComposition 视觉树逐像素合成到桌面 —— 与 Electron 完全相同的合成路径：页面 body
//! 真透明，border-radius 的抗锯齿、box-shadow 的半透明像素都带真 alpha 与桌面合成，窗口
//! **不再设任何 SetWindowRgn**（那是 1bit 硬掩码，会把 DComp 的软边重新切成阶梯）。
//! 版本门槛：该属性 tauri-runtime-wry **2.12.1** 才透传（2.11.4 只透传 transparent），
//! 故本仓 Tauri 2.11 → 2.12（wry 0.55 → 0.57 / tao 0.35 → 0.37）。
//! 失败链存档（勿再尝试）：
//!   · v1.77b 区域与球严格重合 ⇒ 旧链路上 1bit 硬边切 AA，白锯齿；
//!   · v1.78 区域四边外扩 4px 想保 AA ⇒ 半透明像素被压平白底，环带漏浅色异形；
//!   · v1.79 页面自绘不透明黑底绕过坏 alpha ⇒ 异形消失但窗口轮廓仍是整像素阶梯，暗锯齿
//!     （「跟硬边打补丁」的方向错误，v1.80 整版弃用：页面恢复真透明，不注入壳标记）。
//!
//! ### 命中：WM_NCHITTEST 圆形穿透（[`install_circle_hit_test`]）
//! 子类化父窗**以及** `EnumChildWindows` 枚举出的 WebView2 子窗的窗口过程，只拦截
//! `WM_NCHITTEST`：屏点落在「以活柱中心为圆心、活柱半径为半径」的圆内 → 调原过程
//! （父 / 子各自返回 HTCLIENT，页面正常收指针事件）；圆外一律回 `HTTRANSPARENT`，系统
//! 继续向更下层窗口投递（含其他进程，即系统级点击穿透）。命中圆的物理坐标存于原子量
//! （[`update_hit_circle`]），吸附 / DPI / 宿主宽变化时随几何一起更新。子窗必须一起
//! 子类化：WebView2 控制器是覆盖整个宿主客户区的子 HWND，鼠标命中先落在它身上，只改
//! 父窗过程拦不住。
//!
//! 本模块所有几何（拖动 / 吸附 / 钳制 / 落盘 / 恢复）一律以【活柱矩形】为准，宿主窗实际
//! 宽度只用于算活柱偏移 [`live_offset`]。页面侧由 `push_state` 下发 `anchor`（`'l'`/`'r'`），
//! floaticon.html 把球摆进对应活柱。

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::Deserialize;
use tauri::webview::PageLoadEvent;
use tauri::{
    Listener, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};
use url::Url;
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_SERVER};
use windows::Win32::UI::Shell::{ITaskbarList, TaskbarList};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, EnumChildWindows, GetWindowLongPtrW, SetWindowLongPtrW, GWLP_WNDPROC,
    GWL_EXSTYLE, HTTRANSPARENT, WM_NCHITTEST, WNDPROC, WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW,
};

use crate::store::{DockEdge, FloatWinGeom};

/// 悬浮球的固定窗口 label（主窗为 `"main"`、页签前缀 `"tab-"`、宿主 `"host"`）。
pub(crate) const FLOAT_LABEL: &str = "float";

/// 悬浮球内置页面的自定义协议名（URL 形如 `floaticon://localhost/`，
/// wry 在 Windows 会改写成 `http://floaticon.localhost/`，与 splash / shell 同机制）。
pub(crate) const FLOATICON_SCHEME: &str = "floaticon";

// ---- 本地页面 → Rust 的核心事件名（与 src/floaticon.html 里的字面量逐字一致，静态守卫钉着）----
/// pointerdown：只暂存按下点（物理屏幕坐标），第一帧 move 才据此算抓取偏移。
pub(crate) const FLOAT_DOWN_EVENT: &str = "web2app:float-down";
pub(crate) const FLOAT_MOVE_EVENT: &str = "web2app:float-move";
/// pointercancel 且未拖动：只清空暂存按下点 / 拖动态（不弹主窗、不做吸附判定）。
pub(crate) const FLOAT_CANCEL_EVENT: &str = "web2app:float-cancel";
pub(crate) const FLOAT_DRAG_END_EVENT: &str = "web2app:float-drag-end";
pub(crate) const FLOAT_PEEK_ENTER_EVENT: &str = "web2app:float-peek-enter";
pub(crate) const FLOAT_PEEK_LEAVE_EVENT: &str = "web2app:float-peek-leave";
/// 球上右键：**关闭悬浮图标功能并唤回主窗口**（v1.76，见 [`disable_float_from_icon`]）。
/// 事件名沿用 `float-hide`（球页 / preload / 双壳通道名是跨壳契约，不改），语义已升级。
pub(crate) const FLOAT_HIDE_EVENT: &str = "web2app:float-hide";
/// 单击悬浮球：切换原来的主窗口显隐（不是开新窗、不是导航）。
pub(crate) const FLOAT_CLICK_EVENT: &str = "web2app:float-click";

/// 活柱的固定尺寸（**逻辑**像素；正方形；球 = 与它严格重合的圆）。
///
/// ⚠️⚠️ 三层关系（v1.71→v1.80 真机反复取证，改动前必读模块文档「视觉 / 命中的彻底分离」）：
/// 1. **球 == 活柱 == 命中圆**：页面上球铺满活柱；命中测试（WM_NCHITTEST 子类化）的圆以
///    活柱中心为圆心、半径为活柱一半。球比活柱小 ⇒ 圆内球外有一圈透明但吃得到 / 吃得不到
///    鼠标的环带（v1.77b 的浅色残片与 v1.78 的异形本质都是几何与合成错配）。
/// 2. **视觉不经过任何窗口区域**：球缘抗锯齿 / 外阴影由 `.no_redirection_bitmap(true)` 的
///    真逐像素 alpha 合成（v1.80），既不外扩区域也不铺黑底 —— 那两条（v1.78 / v1.79）
///    都已真机证伪。
/// 3. 宿主窗实际宽度由 WebView2 最小宽度决定（见模块文档「活柱」），不以本常量为准。
///
/// 守卫：`contract_tests::float_window_contract::float_page_ball_fills_the_live_pillar`
/// 钉住「球 == 活柱」，`float_window_uses_per_pixel_alpha_and_circle_hittest` 钉住「真
/// alpha 合成 + NCHITTEST 圆形命中、不得再有 SetWindowRgn」，
/// `float_page_and_init_script_stay_truly_transparent` 钉住「页面真透明、不注入壳标记」。
const ICON_SIZE_LOGICAL: f64 = 56.0;
/// 半隐藏时留在屏幕内的宽度（逻辑像素）= 球窗的一半，即真正的「半」隐藏。
const PEEK_LOGICAL: f64 = 28.0;
/// 距屏幕左 / 右缘多近（逻辑像素）即判定为「吸附」。
const SNAP_LOGICAL: f64 = 28.0;
/// 默认摆放 / 纵向钳制时相对屏幕边缘的留白（逻辑像素）。
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

/// 悬浮球的进程级托管状态。
#[derive(Default)]
pub(crate) struct FloatState {
    /// 建窗时生成的一次性防伪令牌（随事件 payload 带回校验）。
    token: Mutex<String>,
    /// 当前吸附状态；None = 自由摆放。
    dock: Mutex<Option<Dock>>,
    /// 指针是否正在拖动球（拖动期间忽略 mouseleave 触发的缩回）。
    moving: AtomicBool,
    /// pointerdown 时指针的物理屏幕坐标（按下点）。
    ///
    /// 只在按下时暂存，**不**展开吸附态、**不**置 moving；第一帧 move 时取走：
    /// 先展开（若处于半隐藏）、再用「按下点 − 展开后窗口原点」算出 anchor。
    /// 绝不能在第一帧 move 才用当前光标记锚点 —— 那时光标已离开按下点若干像素，
    /// 首帧窗口会纹丝不动（合成指针真机取证：第一帧目标坐标 == 当前原点）。
    press: Mutex<Option<(i32, i32)>>,
    /// 本次拖动的抓取点偏移（按下点物理位置 − 展开后窗口左上角物理位置）。
    ///
    /// 第一帧 move 时由 press 计算，之后每帧 `窗口位置 = 光标 − 偏移`。
    /// 绝对坐标自纠错：IPC 丢帧 / 晚到都不会累积误差。v1.69 旧实现上报相对本窗视口的
    /// clientX 增量，而窗口每帧都在 set_position，下一帧增量被窗口自身位移抵消，
    /// 稳态只有半速跟手且来回抖（真机表现「拖不动 / 乱跳」）。
    anchor: Mutex<Option<(i32, i32)>>,
    /// 悬停缩回的「代际」计数器：每次展开 / 隐藏 / 销毁 / 起拖都 +1，
    /// 使此前排队等待的延迟缩回任务全部作废。
    peek_gen: AtomicU64,
}

/// 页面事件的统一 payload（move 带指针的屏幕绝对坐标 sx/sy，其余事件只验令牌）。
#[derive(Deserialize)]
struct FloatMsg {
    token: String,
    #[serde(default)]
    sx: f64,
    #[serde(default)]
    sy: f64,
}

// ===========================================================================
// 自定义协议：提供内置悬浮球页面（零网络、零外部文件依赖）
// ===========================================================================

/// `floaticon://localhost/` 的协议处理器：
/// * `/`（含空 path）→ 内嵌的 floaticon.html；
/// * `/icon.png` → **软件自己的图标**（`icons/icon.png`，编译期内嵌；打包时 `cargo tauri
///   icon` 会按打包配置重生成该文件，球上的图标天然跟随）；
/// * 其它路径 → 404（本协议只暴露这两个内置资源，无任何外部 / 文件系统内容）。
pub(crate) fn handle_floaticon(
    _ctx: tauri::UriSchemeContext<'_, tauri::Wry>,
    req: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    match req.uri().path() {
        "/icon.png" => tauri::http::Response::builder()
            .header("Content-Type", "image/png")
            .header("Cache-Control", "no-cache")
            .body(include_bytes!("../icons/icon.png").to_vec())
            .unwrap(),
        "/" | "" => {
            let html = include_str!("floaticon.html");
            tauri::http::Response::builder()
                .header("Content-Type", "text/html; charset=utf-8")
                .body(html.as_bytes().to_vec())
                .unwrap()
        }
        _ => tauri::http::Response::builder()
            .status(404)
            .body(Vec::new())
            .unwrap(),
    }
}

/// 悬浮球页面的自定义协议 URL。
fn floaticon_url() -> Url {
    Url::parse("floaticon://localhost/").expect("构造 floaticon 协议 URL 失败")
}

// ===========================================================================
// 对外入口（设置命令 / 托盘 / 启动 / 退出 / 关闭语义 调用）
// ===========================================================================

/// 显示悬浮球：已存在则显现（吸附态先缩回成半球）；不存在则（异步）建窗。
///
/// ⚠️⚠️ 与 `settings::open_settings_window` 同理：本函数会从 IPC 命令 / 保存设置的回调栈里
/// 被调用，**绝不能在调用栈内直接建窗**（嵌套事件循环 → 新 WebView2 永不初始化、白窗）。
/// 已存在时只是 show()，可立即执行；需要建窗时一律切后台线程 + `run_on_main_thread` 排队到
/// 下一个事件循环轮次。
pub(crate) fn show_float_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window(FLOAT_LABEL) {
        // 任何排队中的延迟缩回都作废（用户主动唤球，不能 300ms 后自己又缩回去）。
        app.state::<FloatState>()
            .peek_gen
            .fetch_add(1, Ordering::SeqCst);
        // ⚠️ 必须走 show_ball（不是裸 w.show()）：tao 会在显示时用缓存标志把
        //    GWL_EXSTYLE 整体重写一遍，把 WS_EX_APPWINDOW 写回来、抹掉不抢前台与不进任务栏
        //    两位 —— 详见 show_ball 文档。
        show_ball(&w);
        // ⚠️ 刻意**不** set_focus：球窗带 WS_EX_NOACTIVATE（见 apply_ball_window_styles），set_focus
        //    本就不会生效；而它一旦生效就会把前台从主窗抢到球上（真机实测到的那条缺陷）。
        // 重新显现时若处于吸附态，先缩回成半球（悬停再展开）。
        if let Some(dock) = current_dock(app) {
            if matches!(dock.edge, DockEdge::Left | DockEdge::Right) {
                collapse(app, &w, dock);
            }
        }
        return;
    }
    // 生成新窗口的一次性令牌（旧窗口若销毁过，其令牌随之失效）。
    if let Some(tok) = new_token() {
        if let Ok(mut g) = app.state::<FloatState>().token.lock() {
            *g = tok;
        }
    }
    let a = app.clone();
    std::thread::spawn(move || {
        let am = a.clone();
        let _ = a.run_on_main_thread(move || build_float_window_on_main(&am));
    });
}

/// 彻底销毁悬浮球（设置里关掉开关时调用）。几何已随拖动 / 退出落盘，下次开启原位恢复。
///
/// ⚠️⚠️ **必须 `destroy()`，不能 `close()`** —— `close()` 与 Alt+F4 同一入口，会被 lib.rs
/// 里「label == float ⇒ prevent_close + 隐藏」那段整个吞掉，窗口永远不会真销毁；
/// `destroy()` 不走 on_close_requested、不发事件，直接回收。用错的后果是「静默降级」：
/// 设置里取消勾选只是把球藏起来，托盘 / 设置页按钮还能把它唤出来（静态守卫钉着）。
pub(crate) fn destroy_float_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window(FLOAT_LABEL) {
        persist_geometry(app, &w);
        let _ = w.destroy();
    }
    reset_runtime_state(app);
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
/// （lib.rs 的 `CloseRequested` 拦截）走这里。
///
/// ⚠️ **球上右键不走这里**（v1.76 起改走 [`disable_float_from_icon`]）：右键只隐藏会留下
/// 「进程还在跑、桌面上却再无入口」的死局（见该函数与模块文档 ⑤）。
/// 上述两条老路径都不改设置，且都还有别的入口（托盘菜单 / 唤醒快捷键 / 设置页），故保持原样。
///
/// 吸附态（dock）保留：再次显现时恢复成半球；只清一次性的拖动 / 悬停运行期状态。
pub(crate) fn hide_float_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window(FLOAT_LABEL) {
        persist_geometry(app, &w);
        let _ = w.hide();
    }
    if let Ok(mut a) = app.state::<FloatState>().anchor.lock() {
        *a = None;
    }
    if let Ok(mut p) = app.state::<FloatState>().press.lock() {
        *p = None;
    }
    // 收窗即清拖动标志并作废延迟缩回：拖动途中被隐藏时 pointerup 不会再来，残留的
    // moving=true 会让下一次悬停展开 / 移开缩回永久失效（事件被「拖动中」早退挡掉）。
    let st = app.state::<FloatState>();
    st.moving.store(false, Ordering::SeqCst);
    st.peek_gen.fetch_add(1, Ordering::SeqCst);
}

/// 球上**右键**的完整语义（v1.76）：关闭悬浮图标功能 + 唤回主窗口。
///
/// 三步，顺序有意为之：
///   1. **先关功能**（`float_enabled` 置 false：写内存态 + 落盘 + 同步已打开的设置页复选框）。
///      判据先落地 —— 即便后面两步出问题，也不会退回到「进程因 `float_enabled` 驻留、
///      桌面上却没有任何入口」的状态；
///   2. **销毁**球窗（[`destroy_float_window`]，内部先落盘几何）。刻意用 `destroy` 而非
///      `hide`：功能已关，留一个隐藏的球窗既没意义，又会继续挡住「所有窗口关闭 ⇒ 退出」；
///   3. **唤回主窗口**（[`crate::tray::show_window`]：取消最小化 + 显示 + 聚焦，多页签模式
///      一并同步各 tab 窗可见性）。用户点了「关闭悬浮图标」，下一步必须是「回到窗口」，
///      而不是对着一片空桌面发呆。
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
    let visible = app
        .get_webview_window(FLOAT_LABEL)
        .map(|w| w.is_visible().unwrap_or(false))
        .unwrap_or(false);
    if visible {
        hide_float_window(app);
    } else {
        show_float_window(app);
    }
}

/// 退出时持久化悬浮球几何（lib.rs 的 `ExitRequested` 调用）。
pub(crate) fn persist_float_geometry(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window(FLOAT_LABEL) {
        persist_geometry(app, &w);
    }
}

/// 在 app 上注册全部悬浮球核心事件监听（启动时调用一次，仅 Windows）。
pub(crate) fn register_float_listeners(app: &tauri::AppHandle) {
    let listen = |name: &'static str, kind: EventKind| {
        let a = app.clone();
        app.listen(name, move |event| {
            handle_event(&a, kind, event.payload());
        });
    };
    listen(FLOAT_DOWN_EVENT, EventKind::Down);
    listen(FLOAT_MOVE_EVENT, EventKind::Move);
    listen(FLOAT_CANCEL_EVENT, EventKind::Cancel);
    listen(FLOAT_DRAG_END_EVENT, EventKind::DragEnd);
    listen(FLOAT_PEEK_ENTER_EVENT, EventKind::PeekEnter);
    listen(FLOAT_PEEK_LEAVE_EVENT, EventKind::PeekLeave);
    listen(FLOAT_HIDE_EVENT, EventKind::Hide);
    listen(FLOAT_CLICK_EVENT, EventKind::Click);
}

#[derive(Clone, Copy)]
enum EventKind {
    Down,
    Move,
    Cancel,
    DragEnd,
    PeekEnter,
    PeekLeave,
    Hide,
    Click,
}

// ===========================================================================
// 事件分发（令牌校验 + 动作执行；监听在主线程触发，窗口操作可直接做）
// ===========================================================================

fn handle_event(app: &tauri::AppHandle, kind: EventKind, raw: &str) {
    let msg: FloatMsg = match serde_json::from_str(raw) {
        Ok(m) => m,
        Err(_) => return, // 载荷不合法（甚至不是我们发的）直接忽略
    };
    if !token_matches(app, &msg.token) {
        // 令牌不对：可能是被改写的页面在伪造事件。绝不静默放行。
        dlog!("[float] 拒绝令牌不符的悬浮球事件");
        return;
    }
    match kind {
        // 单击悬浮球：切换【原来的主窗口】显隐（可见且非最小化 → 隐藏，否则显示并聚焦；
        // 多页签模式即宿主窗，含各 tab 可见性同步，见 tray::toggle_window）。
        // 单击也结束一次「未达拖动阈值」的按压：清掉暂存按下点 / moving，
        // 否则 pointerdown 置的状态会残留，让后续悬停缩回被「拖动中」早退永久挡掉。
        EventKind::Click => {
            clear_drag_runtime(app);
            crate::tray::toggle_window(app)
        }
        // 按下：只暂存按下点，不展开、不置 moving。
        EventKind::Down => {
            if let Some(w) = app.get_webview_window(FLOAT_LABEL) {
                on_down(&w, msg.sx, msg.sy)
            }
        }
        // 其余事件都操作悬浮球自身；窗口已销毁就忽略。
        EventKind::Move => {
            if let Some(w) = app.get_webview_window(FLOAT_LABEL) {
                on_move(app, &w, msg.sx, msg.sy)
            }
        }
        // 系统打断且未拖动：只复位按压 / 拖动态，不做吸附判定、不弹主窗。
        EventKind::Cancel => {
            if app.get_webview_window(FLOAT_LABEL).is_some() {
                clear_drag_runtime(app);
            }
        }
        EventKind::DragEnd => {
            if let Some(w) = app.get_webview_window(FLOAT_LABEL) {
                on_drag_end(app, &w)
            }
        }
        EventKind::PeekEnter => {
            if let Some(w) = app.get_webview_window(FLOAT_LABEL) {
                on_peek_enter(app, &w)
            }
        }
        EventKind::PeekLeave => on_peek_leave(app),
        // 右键：**关闭悬浮图标功能并唤回主窗口**（v1.76，见 [`disable_float_from_icon`]）。
        // ⚠️ 不是 `hide_float_window` —— 只把球藏起来会留下「进程还在跑、桌面上却再也
        //    唤不出窗口」的死局（2026-10-11 使用者实测），契约测试钉着这一条。
        EventKind::Hide => disable_float_from_icon(app),
    }
}

fn token_matches(app: &tauri::AppHandle, got: &str) -> bool {
    app.state::<FloatState>()
        .token
        .lock()
        .map(|g| !g.is_empty() && g.as_str() == got)
        .unwrap_or(false)
}

/// ⚠️ **仅供真机探针**（`floatprobe`）：把一次「球页事件」直接喂给事件分发。
///
/// 用当前**真实令牌**构造合法载荷，走**真实** [`token_matches`] + [`handle_event`] +
/// 真实窗口操作 —— 唯一跳过的是「JS 把事件发出去」那一段。
///
/// 为什么需要这个入口：真机实测（本机 + 远程桌面 / 受控终端类环境）WebView2 会把
/// 自定义协议页落成 `about:blank`，球页脚本根本不执行；此时「页面 → Rust」这一段恒为空，
/// 真机取证拿不到任何交互证据，而交互逻辑（拖动 / 吸附 / 活柱 / 切换）恰恰是 Tauri 侧
/// 最需要真机验证的部分。页面侧的发事件逻辑由 Electron 壳的同款 harness
/// （`tests/float-v170-harness`）用**同一份共享球页** `floaticon.html` 覆盖 —— 两壳互补，
/// 合起来才是完整链路。探针会把页面链路的真实状态一并打印出来（见 `phase_connectivity`）。
///
/// `sx` / `sy` 与页面一致，是**逻辑（DIP）像素**的屏幕绝对坐标。
pub(crate) fn inject_page_event(app: &tauri::AppHandle, event: &str, sx: f64, sy: f64) -> bool {
    let kind = match event {
        "down" => EventKind::Down,
        "move" => EventKind::Move,
        "cancel" => EventKind::Cancel,
        "dragend" => EventKind::DragEnd,
        "peekenter" => EventKind::PeekEnter,
        "peekleave" => EventKind::PeekLeave,
        "hide" => EventKind::Hide,
        "click" => EventKind::Click,
        _ => return false,
    };
    let token = app
        .state::<FloatState>()
        .token
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let payload = serde_json::json!({ "token": token, "sx": sx, "sy": sy }).to_string();
    handle_event(app, kind, &payload);
    true
}

fn current_dock(app: &tauri::AppHandle) -> Option<Dock> {
    app.state::<FloatState>()
        .dock
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .copied()
}

fn set_dock(app: &tauri::AppHandle, dock: Option<Dock>) {
    if let Ok(mut d) = app.state::<FloatState>().dock.lock() {
        *d = dock;
    }
}

/// 当前运行期吸附态 `(吸附边, 是否处于半隐藏缩回态)`；自由态为 `(None, false)`。
///
/// 只读访问器：真机合成指针 harness（`floatprobe`）用它把**内部状态**与读回的
/// **真实窗口几何**对照着断言 —— 几何对、状态错（或反过来）同样是缺陷，只看一侧会漏。
pub(crate) fn dock_state(app: &tauri::AppHandle) -> (DockEdge, bool) {
    match current_dock(app) {
        Some(d) => (d.edge, d.collapsed),
        None => (DockEdge::None, false),
    }
}

// ===========================================================================
// 建窗（透明小球 + 本地内置页；无导航、无远程内容）
// ===========================================================================

fn build_float_window_on_main(app: &tauri::AppHandle) {
    if app.get_webview_window(FLOAT_LABEL).is_some() {
        return;
    }
    let token = app
        .state::<FloatState>()
        .token
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();

    let app_for_load = app.clone();
    let builder = WebviewWindowBuilder::new(
        app,
        FLOAT_LABEL,
        WebviewUrl::CustomProtocol(floaticon_url()),
    )
    // ⚠️⚠️ 球窗**刻意不设标题**（v1.77，原为 `.title(TITLE)`）。球无边框、不占任务栏、
    //    不参与 Alt+Tab，标题本来就没有任何可见载体；而只要窗口有标题文本，**任何**由外壳
    //    展示标题的界面（任务栏按钮的提示条 / 缩略图浮层 / 任务视图）都会把它画成一个
    //    **独立的非置顶窗口** —— 球窗是 always-on-top，那个提示条只会被画在球窗**之后**、
    //    从球窗透明环带漏出一块浅色残片（真机现象「图标背景异样」）。与此同族的「球元素
    //    不挂 title」见 src/floaticon.html 与 v1.73；这一条堵的是**窗口级**的那一份文本。
    .title("")
    .decorations(false)
    .shadow(false)
    // 透明：球是圆形的，窗外矩形区域必须能点穿到桌面（页面 body 背景透明）。
    .transparent(true)
    // v1.80：WS_EX_NOREDIRECTIONBITMAP —— 宿主无 GDI 重定向表面，DWM 对 WebView2 的
    // DirectComposition 视觉树做**逐像素 alpha** 合成（与 Electron 同一条路径），球缘
    // 抗锯齿 / 外阴影才是真软边。需与 transparent(true) 同开；tao 此时跳过 blur-behind。
    // （tauri-runtime-wry ≥ 2.12.1 才透传本属性，见模块文档「视觉」。）
    .no_redirection_bitmap(true)
    // 不占任务栏按钮：悬浮球是「附属小工具」，入口在设置页 / 托盘 / 右键。
    .skip_taskbar(true)
    // 不允许用户缩放：球是定尺寸的小工具（尺寸由 WebView2 最小宽度 + 活柱区域决定）。
    .resizable(false)
    // 置顶：悬浮球的意义就是始终浮在其它窗口之上。
    .always_on_top(true)
    // 隐藏建窗：等内置页 Finished 再 show。透明 WebView2 在首帧绘制前显现会是一个
    // 「看不见的空窗」（页面还没画球），且建窗时立刻 eval 的 setState 也会因
    // __web2appFloat 未定义被 `&&` 短路丢弃（与 v1.63 splash 早期事件丢失同类）。
    .visible(false)
    .inner_size(ICON_SIZE_LOGICAL, ICON_SIZE_LOGICAL)
    // 一次性令牌只挂主框架（自定义协议页没有 iframe，这里仍是纵深防御）。
    // v1.80：真 alpha 合成后页面保持**真透明**即可，不再注入任何壳标记（v1.79 的 Tauri
    // 专用黑底是坏 alpha 链路上的补丁，已弃用，见模块文档失败链存档）。
    .initialization_script(format!(
        "if (window.self === window.top) {{ window.__web2appFloatToken = {}; }}",
        crate::loading::js_str(&token)
    ))
    // 内置页加载完成：显现 + 推一次当前吸附态（页面只加载一次，这也是 setState
    // 唯一不依赖文档时序的推送点；之后吸附态变化由各动作即时推）。
    .on_page_load(move |w, payload| {
        if payload.event() == PageLoadEvent::Finished {
            // 此时 WebView2 已完成初始化，宿主宽大概率已被最小宽度约束改写：
            // 建窗瞬间按 56 宽算的右吸附 offset 可能失效，这里用真实宿主宽重设命中圆 /
            // 重定位一次（幂等：宿主宽没变时位置结果不变）。
            // 再装一次圆形命中子类化：WebView2 部分子 HWND 可能在控制器初始化后才创建
            // （枚举幂等，已子类化的窗口会跳过）。
            let dock = current_dock(&app_for_load);
            install_circle_hit_test(&w);
            update_hit_circle(&w, dock.map(|d| d.edge));
            if let Some(d) = dock {
                reposition_for_dock(&w, d);
            }
            // ⚠️ 用 show_ball 而不是裸 w.show()：tao 在显示时会把 GWL_EXSTYLE 按缓存标志
            //    整体重写（APPWINDOW 回来、另两位丢失）—— 见 show_ball 文档。
            show_ball(&w);
            push_state(&w, dock);
        }
    });

    // WebView2 用户数据目录：与主窗（lib.rs）/ tab 窗（tabs.rs）完全同构 —— 显式落到应用
    // 数据目录（`config::data_dir` 进程内只解析一次并缓存，这里必然与主窗同一路径；返回 None
    // = 判定不可写并回退系统默认时不设，沿用框架行为）。
    // ⚠️ 不能省略：省略时 wry 给球窗另起默认 UDF（`%LOCALAPPDATA%\<bundle>\EBWebView`）。
    // v1.71 真机判别实验坐实——该目录在企业受控终端 / 沙箱下不可写时，第二个 WebView2 环境
    // 创建失败，tauri-runtime-wry 仅以 `log::error!` 吞掉错误（本仓未接 log 后端，无处可见），
    // `build()` 仍乐观返回 Ok：球窗 HWND 从不出现、on_page_load 永不触发、1s 兜底 show 打在
    // 空壳上，且没有任何 panic / 日志。球页只有内置 HTML 与图标 PNG，不读不写网页会话数据，
    // 与主窗共用 UDF 不引入隐私面（多 tab 窗共用同一 UDF 早已真机验证可行）。
    let builder = match crate::config::data_dir(app) {
        Some(dir) => builder.data_directory(dir),
        None => builder,
    };

    let window = match builder.build() {
        Ok(w) => w,
        Err(e) => {
            dlog!("[float] 悬浮球创建失败：{e}");
            return;
        }
    };

    // 建窗后先补一次扩展样式（不抢前台 + 真不进任务栏）：这是**纵深防御**，让首次显示前
    // 的样式就是干净的。⚠️ 它**不是**可靠保证 —— 之后任何 `show()` / `unminimize()` 都会
    // 被 tao 的 `apply_diff` 按缓存标志整体重写 `GWL_EXSTYLE`（真机读回实测 `0x40118`），
    // ⇒ 每次把球显示出来都必须走 [`show_ball`]，由它在 show 之后再补一次。
    apply_ball_window_styles(&window);
    // 装上圆形命中测试（父窗 + WebView2 子窗窗口过程子类化）。建窗时先装一次；控制器的
    // 部分子 HWND 可能尚未创建，on_page_load(Finished) 会再幂等补装一次。
    install_circle_hit_test(&window);

    // 宿主物理尺寸有两类迟到变化，都必须按真实宿主宽重设命中圆，右吸附 / 缩回态
    // 还要按新 offset 重定位宿主，保证【活柱】仍贴在原来的边上：
    //  ① Resized：WebView2 初始化后才把宿主撑到最小宽度（建窗瞬间 outer_size 可能还
    //     是 56，首算的右吸附偏移会失效）；
    //  ② ScaleFactorChanged：跨 DPI 缩放 / 拖到不同缩放的显示器，活柱物理宽随之改变。
    // 两者幂等（尺寸没变时结果不变），且只更新几何 / 定位，不会再触发 Resized，无反馈环。
    let app_for_event = app.clone();
    window.on_window_event(move |event| {
        if matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
        ) {
            if let Some(w) = app_for_event.get_webview_window(FLOAT_LABEL) {
                let dock = current_dock(&app_for_event);
                // 子 HWND 树也可能在尺寸变化后重建，顺手幂等补装一次命中子类化。
                install_circle_hit_test(&w);
                update_hit_circle(&w, dock.map(|d| d.edge));
                if let Some(d) = dock {
                    reposition_for_dock(&w, d);
                }
            }
        }
    });

    // 恢复上次几何（含吸附态）；无有效几何则按主显示器右上角默认摆放。
    // show 在 on_page_load 里做；这里先定位 + 设好命中圆，窗口显现时即在正确位置 / 形状，
    // 不会从默认位滑入或先露出一截被 WebView2 撑宽的透明空窗。
    let saved = crate::store::load(app).float_window;
    apply_initial_geometry(app, &window, saved);

    // 兜底：on_page_load(Finished) 万一不来（自定义协议异常 / 极旧内核），1s 后强制显现。
    // 宁可在极端环境轻微闪一下，也不能「勾选了悬浮球却永远看不到球」（与 Electron 侧
    // float-window.ts 的 ready-to-show 1s 兜底同语义）。
    let a = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1000));
        let am = a.clone();
        let _ = a.run_on_main_thread(move || {
            if let Some(w) = am.get_webview_window(FLOAT_LABEL) {
                if !w.is_visible().unwrap_or(true) {
                    // 与 on_page_load(Finished) 同语义：用真实宿主宽补一次命中圆 / 重定位。
                    let dock = current_dock(&am);
                    install_circle_hit_test(&w);
                    update_hit_circle(&w, dock.map(|d| d.edge));
                    if let Some(d) = dock {
                        reposition_for_dock(&w, d);
                    }
                    // ⚠️ 同 on_page_load(Finished)：显示必须经 show_ball 补样式。
                    show_ball(&w);
                    push_state(&w, dock);
                }
            }
        });
    });
}

/// 给球窗补两处**扩展样式**（一次 `GWL_EXSTYLE` 读-改-写；分两次写会互相覆盖）。
///
/// ⚠️ 调用时机：**每次 `show()` / `unminimize()` 之后**都要重补一次（见 [`show_ball`]）——
/// tao 的显示路径会用缓存标志整体重写 `GWL_EXSTYLE`，写在 show 之前的那一次会被擦掉。
/// 建窗后也调一次（纵深防御）。改样式本身不需要重新 `show`。
///
/// ## ① `WS_EX_NOACTIVATE`：永不成为前台窗口（v1.72）
///
/// 真机取证（`tests/float-v172-rc/focus-probe.ps1`）：冷启动按 `float_enabled` 建球时，
/// `WebviewWindow::show()` 走 `ShowWindow(SW_SHOW)` —— 那一步**会激活窗口**，实测
/// `GetForegroundWindow()` 落在球窗（136×56）而不是主窗（1114×658），于是启动后用户往页面
/// 里键入没有任何反应，必须先点一下主窗。设置页「立即显示」与托盘切换同理。
///
/// 修法是给球窗加 `WS_EX_NOACTIVATE`：`show()` 与**点击**都不再激活它（鼠标事件照常投递 ——
/// 球页只需要指针事件，不需要键盘焦点）。这是「附属小工具窗口」的标准做法，与 Electron 侧
/// 的 `showInactive()` 同语义。
///
/// ## ② `WS_EX_TOOLWINDOW` 置位 + `WS_EX_APPWINDOW` 清零：**真**不进任务栏（v1.77）
///
/// ⚠️⚠️ `.skip_taskbar(true)` **不够** —— 真机取证（枚举顶层窗口读 `GWL_EXSTYLE`）：
/// 球窗实测 `ex = 0x40118`，**含 `WS_EX_APPWINDOW`（0x40000）**。原因是 tao 0.35 的两段实现：
///   * `window_state.rs`：`if flags.contains(ON_TASKBAR) { style_ex |= WS_EX_APPWINDOW }`；
///   * `window.rs`：`ON_TASKBAR` 是**按「窗口无父」**置位的
///     —— `Parent::None => window_flags.set(ON_TASKBAR, true)`，**与 `skip_taskbar` 无关**；
///   * `window.rs::set_skip_taskbar`：`skip_taskbar(true)` 只额外调一次
///     `ITaskbarList::DeleteTab`，**既不清 `APPWINDOW`，又是在窗口还 `visible(false)` 时调的**
///     —— `DeleteTab` 对从未显示过的窗口无效，而 `APPWINDOW` 会让任务栏在 `show()` 时
///     把按钮**加回来**。
///
/// ⇒ 球会带着任务栏按钮被显示出来（Alt+Tab / 任务视图里也在），违反本模块「不占任务栏」的
/// 需求；更要命的是**任务栏按钮自带原生提示条 / 缩略图浮层**（内容是窗口标题），而球窗是
/// always-on-top ⇒ 那个非置顶浮层被画在球窗**之后**、从球窗透明环带漏出一块浅色残片，
/// 看起来就是「悬浮图标背景上有异样图形」（2026-10-11 使用者真机截图；与 v1.73 修的
/// 「球元素 title 提示条」同族，这里是**窗口级**的那一份）。Electron 壳的 `skipTaskbar: true`
/// 是真移除（自有窗口类写 `WS_EX_TOOLWINDOW`），所以该现象**仅 Tauri 侧**出现。
///
/// 修法：把 `GWL_EXSTYLE` 的 `APPWINDOW` 清掉、`TOOLWINDOW` 置上（tao 自己的消息窗就是
/// 这么做的），与 Electron 语义对齐。⚠️ 只改一次不够 —— 显示路径会把 `APPWINDOW` 写回来
/// ⇒ 每次 `show()` 之后都要经 [`show_ball`] 重补，并配 [`delete_taskbar_tab`] 回收可能
/// 已经建出的任务栏按钮。
/// 配套：球窗标题置空（见建窗处）—— 即使将来别的外壳界面再拿标题当文本，也不会有可漏的内容。
fn apply_ball_window_styles(w: &WebviewWindow) {
    let hwnd = match w.hwnd() {
        Ok(h) => h,
        Err(e) => {
            // 静默失败过的路径：写不进去的后果是任务栏按钮 / 抢前台双双回来，必须留痕。
            dlog!("[float] 取球窗 HWND 失败，扩展样式未改（任务栏按钮与抢前台都会回来）：{e}");
            return;
        }
    };
    unsafe {
        let cur = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let want = (WS_EX_NOACTIVATE.0 as isize) | (WS_EX_TOOLWINDOW.0 as isize);
        let next = (cur | want) & !(WS_EX_APPWINDOW.0 as isize);
        let prev = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next);
        // 读回校验：SetWindowLongPtrW 失败只回报 0，而 0 也是合法的旧样式值 ⇒ 只能读回看结果。
        let after = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        dlog!(
            "[float] 球窗扩展样式 ex {cur:#x} -> 目标 {next:#x}（旧值回报 {prev:#x}），读回 {after:#x}",
        );
    }
}

/// 把球窗从任务栏按钮列表里摘掉（`ITaskbarList::DeleteTab`，幂等；本来没有按钮时空操作）。
///
/// ⚠️⚠️ `ITaskbarList` 的契约：**调用任何方法之前必须先 `HrInit()`**（MSDN 明文要求，
/// 未初始化时 `DeleteTab` 返回 `E_FAIL` 且**什么都不做**）。v1.77 第一版漏了这一步、又把
/// HRESULT 用 `let _ =` 吞掉 ⇒ 典型的「静默失败」：读回 `GWL_EXSTYLE` 看着是对的，任务栏
/// 按钮却可能还在。这里补上 `HrInit()`，并让 `HrInit` / `DeleteTab` 的失败**留痕**。
///
/// 为什么非要有这一步：[`apply_ball_window_styles`] 虽然清了 `WS_EX_APPWINDOW`，但那是在
/// `show()` **之后**补的，而 `show()` 那一瞬间样式被 tao 写回成含 `APPWINDOW` 的样子 ——
/// 任务栏完全可能据此已经建好了按钮。清样式只是让**将来**不再重建，已经建出来的那一个
/// 要显式回收（Windows 不保证样式变化一定会让任务栏把按钮撤掉）。
/// Electron 侧不需要这一步：它在建窗时就把窗口类定成了 `WS_EX_TOOLWINDOW`（`skipTaskbar`）。
///
/// COM 取对象失败**不致命**（只留痕）：最坏退化成「球出现在任务栏里」，球本身照常可用。
fn delete_taskbar_tab(w: &WebviewWindow) {
    let Ok(hwnd) = w.hwnd() else {
        return;
    };
    unsafe {
        match CoCreateInstance::<_, ITaskbarList>(&TaskbarList, None, CLSCTX_SERVER) {
            Ok(list) => {
                if let Err(e) = list.HrInit() {
                    dlog!("[float] ITaskbarList::HrInit 失败（DeleteTab 将无效，任务栏按钮可能残留）：{e}");
                    return;
                }
                if let Err(e) = list.DeleteTab(hwnd) {
                    // DeleteTab 对「从未显示过的窗口」返回成功但也不做事；这里只记失败。
                    dlog!("[float] ITaskbarList::DeleteTab 失败（任务栏按钮可能残留）：{e}");
                }
            }
            Err(e) => dlog!("[float] 取 ITaskbarList 失败（任务栏按钮可能残留）：{e}"),
        }
    }
}

/// **显示**球窗（顺带取消最小化），并把 tao 在显示路径上写脏的扩展样式补回来。
///
/// ⚠️⚠️ 顺序不能反、也不能省 —— 所有「把球弄出来」的地方都必须走本函数：
/// tao 的 `set_visible` 走 `WindowState::set_window_flags` → `apply_diff`（`window_state.rs`），
/// 那里用**缓存的窗口标志**重新拼 `style_ex` 并整体 `SetWindowLongW(GWL_EXSTYLE, …)`；
/// 缓存里的 `ON_TASKBAR`（创建时按「窗口无父」置位，与 `skip_taskbar` 无关）会把
/// `WS_EX_APPWINDOW` **写回来**，同时抹掉我们补的 `WS_EX_NOACTIVATE` / `WS_EX_TOOLWINDOW`。
/// 真机实测（v1.77 取证，读回 `GWL_EXSTYLE`）：建窗后设好的样式，在 `show()` 之后仍是
/// `0x40118`（`APPWINDOW|TOPMOST`，另两位都没了）⇒ 只改在 show 之前等于没改。
/// `unminimize()` 同样触发一次重写，所以它也在本函数内完成。
fn show_ball(w: &WebviewWindow) {
    let _ = w.show();
    let _ = w.unminimize();
    apply_ball_window_styles(w);
    delete_taskbar_tab(w);
}

/// 建窗后摆放：优先恢复保存的几何（含吸附态）；几何失效（中心不在任何显示器 / 尺寸过小）
/// 则默认摆放到主显示器右上角。所有位置都是【活柱】几何（见模块文档）；宿主窗位置 =
/// 活柱位置 − [`live_offset`]。同时按最终吸附边设置窗口裁剪区域。
fn apply_initial_geometry(app: &tauri::AppHandle, w: &WebviewWindow, saved: Option<FloatWinGeom>) {
    let scale = w.scale_factor().unwrap_or(1.0);
    let host = w.outer_size().unwrap_or(PhysicalSize::new(
        (ICON_SIZE_LOGICAL * scale) as u32,
        (ICON_SIZE_LOGICAL * scale) as u32,
    ));
    let (lw, lh) = live_px(scale);
    if let Some(g) = saved {
        let center_ok = w
            .monitor_from_point(
                (g.x as f64) + (g.width as f64) / 2.0,
                (g.y as f64) + (g.height as f64) / 2.0,
            )
            .ok()
            .flatten()
            .is_some();
        if center_ok && g.width >= MIN_PHYS && g.height >= MIN_PHYS {
            if matches!(g.edge, DockEdge::Left | DockEdge::Right) {
                // 恢复吸附：找到几何所在显示器，记录 Dock 并直接摆成半隐藏半球态。
                if let Some(mon) = monitor_bounds_at(w, g.x, g.y) {
                    let y = clamp_y(g.y, lh, mon.1, mon.3);
                    let lx = collapsed_x(g.edge, mon.0, mon.2, lw, peek_px(scale));
                    let off = live_offset(host.width as i32, lw, g.edge);
                    update_hit_circle(w, Some(g.edge));
                    let _ = w.set_position(PhysicalPosition::new(lx - off, y));
                    set_dock(
                        app,
                        Some(Dock {
                            edge: g.edge,
                            mx: mon.0,
                            my: mon.1,
                            mw: mon.2,
                            mh: mon.3,
                            collapsed: true,
                        }),
                    );
                    return;
                }
            } else {
                // 自由态：活柱贴宿主窗左边，宿主原点即活柱原点。
                update_hit_circle(w, Some(DockEdge::None));
                let _ = w.set_position(PhysicalPosition::new(g.x, g.y));
                return;
            }
        }
    }
    // 默认（或几何失效）：主显示器右上角留白摆放（自由态，活柱贴左）。
    update_hit_circle(w, Some(DockEdge::None));
    if let Some(mon) = w.primary_monitor().ok().flatten() {
        let pos = mon.position();
        let m = (MARGIN_LOGICAL * scale).round() as i32;
        let x = pos.x + mon.size().width as i32 - lw - m;
        let y = pos.y + m;
        let _ = w.set_position(PhysicalPosition::new(x, y));
    }
}

// ===========================================================================
// 活柱（WebView2 最小宽度的应对，见模块文档）
// ===========================================================================

/// 活柱的物理宽 / 高（56 逻辑像素随 DPI 缩放）。
pub(crate) fn live_px(scale: f64) -> (i32, i32) {
    (
        (ICON_SIZE_LOGICAL * scale).round().max(32.0) as i32,
        (ICON_SIZE_LOGICAL * scale).round().max(32.0) as i32,
    )
}

/// 活柱左边相对宿主窗左边的物理偏移：右吸附贴宿主窗右边（保证半球露在右缘），
/// 左吸附 / 自由态贴左边。宿主宽若异常窄于活柱则钳到 0。
pub(crate) fn live_offset(host_w: i32, live_w: i32, edge: DockEdge) -> i32 {
    match edge {
        DockEdge::Right => (host_w - live_w).max(0),
        DockEdge::Left | DockEdge::None => 0,
    }
}

// ===========================================================================
// 圆形命中测试（v1.80：视觉走真 alpha，命中走 WM_NCHITTEST 子类化，见模块文档）
// ===========================================================================

/// 命中圆（父窗客户区物理坐标）：圆心 x。全局唯一球窗，用静态原子量即可。
static HIT_CENTER_X: AtomicI32 = AtomicI32::new(0);
/// 命中圆圆心 y。
static HIT_CENTER_Y: AtomicI32 = AtomicI32::new(0);
/// 命中圆半径；`0` = 几何尚未初始化（此时 fail-open，命中交原过程，球仍可点）。
static HIT_RADIUS: AtomicI32 = AtomicI32::new(0);
/// 父窗 HWND（`as usize`）：子窗过程收到的屏幕坐标要换算到**父窗**客户区再判圆。
static HIT_PARENT: AtomicUsize = AtomicUsize::new(0);

/// 已被子类化的窗口（HWND, 原窗口过程）。`OnceLock` + `Mutex` 只在装 / 卸与窗口销毁时
/// 短暂加锁；WM_NCHITTEST 热路径判圆不碰它。
fn subclasses() -> &'static Mutex<Vec<(isize, isize)>> {
    static SUBCLASSES: OnceLock<Mutex<Vec<(isize, isize)>>> = OnceLock::new();
    SUBCLASSES.get_or_init(|| Mutex::new(Vec::new()))
}

/// 按吸附边更新命中圆：圆心 = 活柱中心（右吸附时活柱贴宿主右边），半径 = 活柱一半。
/// 坐标是父窗**客户区物理坐标**；无边框窗客户区即整窗。必须在主线程调用。
/// 调用时机与旧版 SetWindowRgn 完全相同：建窗后 / on_page_load / Resized /
/// ScaleFactorChanged / 拖动结束 / 兜底显现（任一窗口几何变化都可能移动活柱中心）。
fn update_hit_circle(w: &WebviewWindow, edge: Option<DockEdge>) {
    let (Ok(scale), Ok(host)) = (w.scale_factor(), w.outer_size()) else {
        // 窗口句柄 / 尺寸尚不可用（建窗极早期）：on_page_load 与 Resized 还会再设。
        return;
    };
    let edge = edge.unwrap_or(DockEdge::None);
    let (lw, lh) = live_px(scale);
    let cx = live_offset(host.width as i32, lw, edge) + lw / 2;
    let cy = lh / 2;
    HIT_CENTER_X.store(cx, Ordering::Relaxed);
    HIT_CENTER_Y.store(cy, Ordering::Relaxed);
    HIT_RADIUS.store(lw / 2, Ordering::Relaxed);
}

/// 安装圆形命中：子类化父窗 + 全部 WebView2 后代子窗。幂等（重复调用只补装新出现的
/// HWND）。必须在主线程调用（与 SetWindowLongPtr 的所有用法同）。
/// 失败不致命：最坏退化是矩形窗四角 / 撑宽部分吃鼠标（视觉仍是圆的），球照常可用。
fn install_circle_hit_test(w: &WebviewWindow) {
    let Ok(parent) = w.hwnd() else {
        return;
    };
    HIT_PARENT.store(parent.0 as usize, Ordering::Relaxed);
    unsafe {
        // EnumChildWindows 递归枚举**全部后代**：WebView2 控制器子 HWND 及其内部 Chrome
        // 窗口都要拦 —— 鼠标命中先落在最内层子窗，漏一个它就是矩形吃点击。
        let mut children: Vec<HWND> = Vec::new();
        let _ = EnumChildWindows(
            Some(parent),
            Some(enum_children_proc),
            LPARAM(&mut children as *mut Vec<HWND> as isize),
        );
        subclass_window(parent);
        for child in children {
            subclass_window(child);
        }
    }
}

/// `EnumChildWindows` 回调：把后代 HWND 收集进 lParam 携带的 Vec。
unsafe extern "system" fn enum_children_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let found = &mut *(lparam.0 as *mut Vec<HWND>);
    found.push(hwnd);
    BOOL(1) // TRUE = 继续枚举
}

/// 给单个 HWND 换上 [`hit_test_wndproc`]；已装过则跳过。
unsafe fn subclass_window(hwnd: HWND) {
    let mut guard = subclasses()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let key = hwnd.0 as isize;
    if guard.iter().any(|(h, _)| *h == key) {
        return;
    }
    let new_proc: WNDPROC = Some(hit_test_wndproc);
    let new_proc_ptr = std::mem::transmute::<WNDPROC, isize>(new_proc);
    // 64 位 Windows 必须用 SetWindowLongPtrW（SetWindowLongW 装不下过程地址）。
    let old = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, new_proc_ptr);
    if old == 0 {
        dlog!("[float] 球窗 {key:#x} 命中子类化失败（SetWindowLongPtrW 返回 0），该窗退化矩形命中");
        return;
    }
    guard.push((key, old));
}

/// 球窗父 / 子窗口过程：只接管 WM_NCHITTEST，其余消息一律调原过程。
unsafe extern "system" fn hit_test_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCHITTEST && HIT_RADIUS.load(Ordering::Relaxed) > 0 {
        // lParam 低 / 高位是有符号的屏幕坐标（GET_X_LPARAM / GET_Y_LPARAM）。
        let sx = (lparam.0 as u16) as i16 as i32;
        let sy = ((lparam.0 >> 16) as u16) as i16 as i32;
        let parent = HWND(HIT_PARENT.load(Ordering::Relaxed) as *mut core::ffi::c_void);
        let mut pt = POINT { x: sx, y: sy };
        // 统一换算到父窗客户区：命中圆是按「活柱在父窗客户区里的位置」定义的。
        let _ = ScreenToClient(parent, &mut pt);
        let cx = HIT_CENTER_X.load(Ordering::Relaxed);
        let cy = HIT_CENTER_Y.load(Ordering::Relaxed);
        let r = HIT_RADIUS.load(Ordering::Relaxed) as i64;
        let dx = pt.x as i64 - cx as i64;
        let dy = pt.y as i64 - cy as i64;
        if dx * dx + dy * dy > r * r {
            // 圆外：让系统继续向更下层（含跨进程）窗口投递此次命中。
            return LRESULT(HTTRANSPARENT as isize);
        }
    }
    let result = call_original_proc(hwnd, msg, wparam, lparam);
    if msg == windows::Win32::UI::WindowsAndMessaging::WM_NCDESTROY {
        // 先让原过程处理完销毁，再把窗口过程还原（此后系统保证不再投递消息到本过程）。
        uninstall_window(hwnd);
    }
    result
}

/// 调回被替换前的原窗口过程；找不到记录（理论上不会发生）时用 DefWindowProcW 兜底。
unsafe fn call_original_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let old = {
        let guard = subclasses()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let key = hwnd.0 as isize;
        guard.iter().find(|(h, _)| *h == key).map(|(_, p)| *p)
    };
    if let Some(ptr) = old {
        let old_proc: WNDPROC = std::mem::transmute::<isize, WNDPROC>(ptr);
        CallWindowProcW(old_proc, hwnd, msg, wparam, lparam)
    } else {
        windows::Win32::UI::WindowsAndMessaging::DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

/// 窗口销毁时还原原窗口过程并移出子类化表。
unsafe fn uninstall_window(hwnd: HWND) {
    let old = {
        let mut guard = subclasses()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let key = hwnd.0 as isize;
        guard
            .iter()
            .position(|(h, _)| *h == key)
            .map(|i| guard.swap_remove(i).1)
    };
    if let Some(ptr) = old {
        let old_proc: WNDPROC = std::mem::transmute::<isize, WNDPROC>(ptr);
        let new_ptr = std::mem::transmute::<WNDPROC, isize>(old_proc);
        let _ = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, new_ptr);
    }
}

/// 宿主物理宽度可能迟到（WebView2 初始化 / DPI 变化后才把窗撑到最小宽度）：
/// 按当前 dock 与真实宿主宽，把【活柱】重新摆到吸附边的展开 / 缩回位置。
/// 自由态不需要动（活柱恒贴宿主左边，offset 为 0）。
fn reposition_for_dock(w: &WebviewWindow, d: Dock) {
    if !matches!(d.edge, DockEdge::Left | DockEdge::Right) {
        return;
    }
    if let (Ok(scale), Ok(host), Ok(hpos)) = (w.scale_factor(), w.outer_size(), w.outer_position())
    {
        let (lw, lh) = live_px(scale);
        let off = live_offset(host.width as i32, lw, d.edge);
        let y = clamp_y(hpos.y, lh, d.my, d.mh);
        let lx = if d.collapsed {
            collapsed_x(d.edge, d.mx, d.mw, lw, peek_px(scale))
        } else {
            expanded_x(d.edge, d.mx, d.mw, lw)
        };
        let _ = w.set_position(PhysicalPosition::new(lx - off, y));
    }
}

// ===========================================================================
// 拖动 / 吸附 / 悬停展开
// ===========================================================================

/// pointerdown：暂存按下点的物理屏幕坐标。不展开半隐藏态、不置 moving ——
/// 这次按压可能只是一次点击；展开 / moving 都推迟到越过阈值的第一帧 move。
fn on_down(w: &WebviewWindow, sx: f64, sy: f64) {
    let scale = w.scale_factor().unwrap_or(1.0);
    let p = ((sx * scale).round() as i32, (sy * scale).round() as i32);
    if let Ok(mut g) = w.state::<FloatState>().press.lock() {
        *g = Some(p);
    }
}

/// 清空一次按压 / 拖动的运行期状态（单击 / 系统打断 / 松手时调用；不动吸附态与几何）。
fn clear_drag_runtime(app: &tauri::AppHandle) {
    app.state::<FloatState>()
        .moving
        .store(false, Ordering::SeqCst);
    if let Ok(mut a) = app.state::<FloatState>().anchor.lock() {
        *a = None;
    }
    if let Ok(mut p) = app.state::<FloatState>().press.lock() {
        *p = None;
    }
}

fn on_move(app: &tauri::AppHandle, w: &WebviewWindow, sx: f64, sy: f64) {
    let state = app.state::<FloatState>();
    state.moving.store(true, Ordering::SeqCst);
    let scale = w.scale_factor().unwrap_or(1.0);
    // 页面上报的是屏幕绝对坐标（CSS 像素），换算到物理像素。
    let (cx, cy) = ((sx * scale).round() as i32, (sy * scale).round() as i32);
    let (lw, lh) = live_px(scale);

    // 首帧：用 pointerdown 暂存的【按下点】确定抓取偏移。若从半隐藏半球抓起，必须
    // 【先展开、后取偏移】——展开会把窗口平移，展开后再以新原点记偏移，这一下平移才不会
    // 变成球离手的跳动。注意不能用「当前帧光标」当按下点：第一帧 move 时光标已离开按下点，
    // 用它记偏移会让首帧窗口纹丝不动（合成指针真机取证实锤）。
    let anchor = {
        let mut guard = state.anchor.lock().unwrap_or_else(|p| p.into_inner());
        if guard.is_none() {
            // 取走按下点；缺失（异常时序 / 旧页面）时退回用当前帧坐标，保持可拖动。
            let press = state
                .press
                .lock()
                .ok()
                .and_then(|mut g| g.take())
                .unwrap_or((cx, cy));
            if let Some(dock) = current_dock(app) {
                if dock.collapsed {
                    // 起拖即作废旧一代悬停缩回任务（展开后不能被旧定时器缩回去）。
                    state.peek_gen.fetch_add(1, Ordering::SeqCst);
                    expand(app, w, dock);
                }
            }
            // 抓取点相对【活柱】原点：右吸附时活柱在宿主窗右侧，必须补上 live_offset。
            let edge = current_dock(app).map(|d| d.edge).unwrap_or(DockEdge::None);
            if let (Ok(pos), Ok(host)) = (w.outer_position(), w.outer_size()) {
                let off = live_offset(host.width as i32, lw, edge);
                *guard = Some((press.0 - pos.x - off, press.1 - pos.y));
            }
        }
        *guard
    };
    let (Some((ax, ay)), Ok(host)) = (anchor, w.outer_size()) else {
        return;
    };
    // 拖动期间吸附边不变（换边在松手时判定）；活柱偏移随当前边。
    let edge = current_dock(app).map(|d| d.edge).unwrap_or(DockEdge::None);
    let off = live_offset(host.width as i32, lw, edge);

    // 活柱新左上角 = 光标 − 抓取偏移（绝对坐标，丢帧也能在下一帧自愈）。
    let (mut nx, mut ny) = (cx - ax, cy - ay);
    // 拖动中只做「至少留一条窄边在屏内」的软钳制：允许跨屏，不允许整个球甩丢。
    // 钳制一律以活柱宽高为准（宿主窗被 WebView2 撑宽的部分不参与几何）。
    if let Some((mx, my, mw, mh)) = monitor_bounds_at(w, cx, cy) {
        let minv = min_visible_px(scale);
        nx = clamp_visible(nx, lw, mx, mx + mw, minv);
        ny = clamp_visible(ny, lh, my, my + mh, minv);
    }
    let _ = w.set_position(PhysicalPosition::new(nx - off, ny));
}

fn on_drag_end(app: &tauri::AppHandle, w: &WebviewWindow) {
    let (Ok(hpos), Ok(host), Ok(scale)) = (w.outer_position(), w.outer_size(), w.scale_factor())
    else {
        clear_drag_runtime(app);
        return;
    };
    let (lw, lh) = live_px(scale);
    // 拖动期间的吸附边 → 松手时的活柱偏移与活柱原点（换边判定基于活柱矩形）。
    let edge0 = current_dock(app).map(|d| d.edge).unwrap_or(DockEdge::None);
    let off0 = live_offset(host.width as i32, lw, edge0);
    let pos = PhysicalPosition::new(hpos.x + off0, hpos.y);

    // 找活柱中心所在显示器；找不到（被甩到屏间空隙）就用主显示器。
    let cx = pos.x + lw / 2;
    let cy = pos.y + lh / 2;
    let mon = monitor_bounds_at(w, cx, cy).or_else(|| {
        w.primary_monitor().ok().flatten().map(|m| {
            let p = m.position();
            let s = m.size();
            (p.x, p.y, s.width as i32, s.height as i32)
        })
    });
    let Some((mx, my, mw, mh)) = mon else {
        update_hit_circle(w, Some(DockEdge::None));
        set_dock(app, None);
        persist_geometry(app, w);
        push_state(w, None);
        clear_drag_runtime(app);
        return;
    };

    let snap = snap_px(scale);
    let edge = decide_edge((pos.x, pos.y, lw, lh), (mx, my, mw, mh), snap);
    match edge {
        DockEdge::Left | DockEdge::Right => {
            // 吸附：先把活柱锚点切到新边（区域裁剪立即生效，避免一帧错位），
            // 再按展开态贴边摆放并纵向钳制，落盘展开态坐标，随即缩回只露半球。
            let y = clamp_y(pos.y, lh, my, mh);
            let lx = expanded_x(edge, mx, mw, lw);
            let off = live_offset(host.width as i32, lw, edge);
            update_hit_circle(w, Some(edge));
            let _ = w.set_position(PhysicalPosition::new(lx - off, y));
            let dock = Dock {
                edge,
                mx,
                my,
                mw,
                mh,
                collapsed: false,
            };
            set_dock(app, Some(dock));
            persist_geometry(app, w);
            // 松手即缩回（悬停半球再展开）。
            collapse(app, w, dock);
        }
        DockEdge::None => {
            // 自由态：活柱贴宿主左边，把球完整钳回显示器内（旧实现不钳，球可被甩到屏外
            // 找不回来 —— 它没有标题栏，移出屏幕的部分无法再抓）。纵向同样钳。
            let x = pos.x.clamp(mx, (mx + mw - lw).max(mx));
            let y = clamp_y(pos.y, lh, my, mh);
            update_hit_circle(w, Some(DockEdge::None));
            if x != pos.x || y != pos.y || !matches!(edge0, DockEdge::None) {
                let _ = w.set_position(PhysicalPosition::new(x, y));
            }
            set_dock(app, None);
            persist_geometry(app, w);
            push_state(w, None);
        }
    }
    clear_drag_runtime(app);
}

fn on_peek_enter(app: &tauri::AppHandle, w: &WebviewWindow) {
    // 拖动中 / 非吸附态：无需动作。
    if app.state::<FloatState>().moving.load(Ordering::SeqCst) {
        return;
    }
    let Some(dock) = current_dock(app) else {
        return;
    };
    // 任何一次进入都必须先作废排队中的延迟缩回 —— 哪怕当前已展开（leave 已排队、
    // 300ms 未到又触发 enter），否则定时器到期会把光标下的球缩回（合成指针真机取证实锤）。
    app.state::<FloatState>()
        .peek_gen
        .fetch_add(1, Ordering::SeqCst);
    // 已展开无需进一步动作；只有半隐藏半球才展开。
    if dock.collapsed {
        expand(app, w, dock);
    }
}

fn on_peek_leave(app: &tauri::AppHandle) {
    let state = app.state::<FloatState>();
    if state.moving.load(Ordering::SeqCst) {
        return;
    }
    let Some(dock) = current_dock(app) else {
        return;
    };
    if dock.collapsed {
        return;
    }
    // 不立刻缩回：鼠标在屏幕边缘移动时，窗口 28px 的展开 / 缩回平移会连环触发进出事件，
    // 即时响应表现为球在边缘疯狂抖动。改为延迟 + 到期双重校验（代际 + 真实光标位置）。
    let gen = state.peek_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let a = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(COLLAPSE_DELAY_MS));
        let am = a.clone();
        let _ = a.run_on_main_thread(move || finish_peek_leave(&am, gen));
    });
}

/// 延迟到期后真正执行缩回（在主线程）：代际已换代 / 窗口没了 / 正在拖动 / 已缩回 → 放弃；
/// 真实光标若已落在「缩回后矩形」内也放弃（否则窗口滑到光标下会立刻再触发展开，形成抖动）。
fn finish_peek_leave(app: &tauri::AppHandle, gen: u64) {
    let state = app.state::<FloatState>();
    if state.peek_gen.load(Ordering::SeqCst) != gen || state.moving.load(Ordering::SeqCst) {
        return;
    }
    let Some(w) = app.get_webview_window(FLOAT_LABEL) else {
        return;
    };
    if !w.is_visible().unwrap_or(false) {
        return;
    }
    let Some(dock) = current_dock(app) else {
        return;
    };
    if dock.collapsed {
        return;
    }
    let (Ok(scale), Ok(hpos)) = (w.scale_factor(), w.outer_position()) else {
        collapse(app, &w, dock);
        return;
    };
    let (lw, lh) = live_px(scale);
    let y = clamp_y(hpos.y, lh, dock.my, dock.mh);
    let x = collapsed_x(dock.edge, dock.mx, dock.mw, lw, peek_px(scale));
    // 光标此刻若在缩回后的【活柱】矩形内，缩回会把窗口送到光标下，下一次鼠标移动必再展开
    // —— 直接保持展开，等下一次 leave。
    if let Ok(p) = w.cursor_position() {
        if point_in_rect(p.x.round() as i32, p.y.round() as i32, x, y, lw, lh) {
            return;
        }
    }
    collapse(app, &w, dock);
}

/// 展开到屏内（dock.collapsed=false）并通知前端。
fn expand(app: &tauri::AppHandle, w: &WebviewWindow, d: Dock) {
    if let (Ok(scale), Ok(hpos), Ok(host)) = (w.scale_factor(), w.outer_position(), w.outer_size())
    {
        let (lw, _lh) = live_px(scale);
        let off = live_offset(host.width as i32, lw, d.edge);
        let x = expanded_x(d.edge, d.mx, d.mw, lw);
        let _ = w.set_position(PhysicalPosition::new(x - off, hpos.y));
    }
    let next = Dock {
        collapsed: false,
        ..d
    };
    set_dock(app, Some(next));
    push_state(w, Some(next));
}

/// 缩回屏外只露半球（dock.collapsed=true）并通知前端。
fn collapse(app: &tauri::AppHandle, w: &WebviewWindow, d: Dock) {
    if let (Ok(scale), Ok(hpos), Ok(host)) = (w.scale_factor(), w.outer_position(), w.outer_size())
    {
        let (lw, _lh) = live_px(scale);
        let off = live_offset(host.width as i32, lw, d.edge);
        let x = collapsed_x(d.edge, d.mx, d.mw, lw, peek_px(scale));
        let _ = w.set_position(PhysicalPosition::new(x - off, hpos.y));
    }
    let next = Dock {
        collapsed: true,
        ..d
    };
    set_dock(app, Some(next));
    push_state(w, Some(next));
}

/// 通知页面切换吸附 / 缩回样式（在主框架执行）。
/// `anchor` 告诉页面活柱在宿主视口的哪一侧（右吸附 = `'r'`，其余 = `'l'`），
/// 因为 Tauri 宿主窗比球宽（WebView2 最小宽度），球必须摆进活柱而不是视口居中；
/// Electron 宿主窗正好 56 宽，两种 anchor 视觉等价。
fn push_state(w: &WebviewWindow, dock: Option<Dock>) {
    let (edge, collapsed, anchor) = match dock {
        Some(d) => (
            edge_str(d.edge),
            d.collapsed,
            if matches!(d.edge, DockEdge::Right) {
                "'r'"
            } else {
                "'l'"
            },
        ),
        None => ("'none'", false, "'l'"),
    };
    let js = format!(
        "window.__web2appFloat&&window.__web2appFloat.setState({{edge:{edge},collapsed:{collapsed},anchor:{anchor}}});"
    );
    let _ = w.eval(&js);
}

// ===========================================================================
// 持久化
// ===========================================================================

/// 抓取当前悬浮球几何落盘（读-改-写，只替换 `float_window` 段）。
/// 一律落【活柱】几何：吸附态下存展开态 x（半隐藏的偏移运行期算），y 取当前值，
/// 宽高恒为活柱物理宽高（宿主窗被 WebView2 撑宽的部分不入契约）。
fn persist_geometry(app: &tauri::AppHandle, w: &WebviewWindow) {
    let (Some(hpos), Some(host), Ok(scale)) = (
        w.outer_position().ok(),
        w.outer_size().ok(),
        w.scale_factor(),
    ) else {
        return;
    };
    let (lw, lh) = live_px(scale);
    let dock = current_dock(app);
    let (edge, x) = match dock {
        Some(d) if matches!(d.edge, DockEdge::Left | DockEdge::Right) => {
            // hpos.x 此刻可能是缩回到屏外的宿主坐标；落盘展开态的【活柱】x。
            (d.edge, expanded_x(d.edge, d.mx, d.mw, lw))
        }
        Some(d) => {
            let off = live_offset(host.width as i32, lw, d.edge);
            (DockEdge::None, hpos.x + off)
        }
        None => (DockEdge::None, hpos.x),
    };
    let geom = FloatWinGeom {
        x,
        y: hpos.y,
        width: lw as u32,
        height: lh as u32,
        edge,
    };
    if let Err(e) = crate::store::update(app, |doc| doc.float_window = Some(geom)) {
        dlog!("[float] 写入悬浮球几何失败：{e}");
    }
}

// ===========================================================================
// 纯函数（几何决策，单测覆盖）
// ===========================================================================

/// 距屏幕左 / 右缘多近（物理像素）即判定为「吸附」。
pub(crate) fn snap_px(scale: f64) -> i32 {
    (SNAP_LOGICAL * scale).round() as i32
}

/// 半球物理宽度（随 DPI 缩放）。
pub(crate) fn peek_px(scale: f64) -> i32 {
    (PEEK_LOGICAL * scale).round().max(10.0) as i32
}

/// 拖动中软钳制的最小可见物理宽度（随 DPI 缩放，下限 8px）。
pub(crate) fn min_visible_px(scale: f64) -> i32 {
    (MIN_VISIBLE_LOGICAL * scale).round().max(8.0) as i32
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
pub(crate) fn expanded_x(edge: DockEdge, mx: i32, _mw: i32, ww: i32) -> i32 {
    match edge {
        DockEdge::Left => mx,
        DockEdge::Right => mx + _mw - ww,
        DockEdge::None => mx,
    }
}

/// 半隐藏态左上角 x：把窗口一半移出屏外，只留 peek 宽在屏内。
pub(crate) fn collapsed_x(edge: DockEdge, mx: i32, mw: i32, ww: i32, peek: i32) -> i32 {
    match edge {
        DockEdge::Left => mx - (ww - peek),
        DockEdge::Right => mx + mw - peek,
        DockEdge::None => mx,
    }
}

/// 纵向把窗口钳制在显示器内（上下各留一点边），避免吸附后窗口顶 / 底超出屏幕。
fn clamp_y(y: i32, wh: i32, my: i32, mh: i32) -> i32 {
    y.clamp(my, (my + mh - wh).max(my))
}

/// 判定窗口是否贴近屏幕左 / 右缘：窗口左边距屏幕左边 ≤ snap（且纵向与屏幕有重叠）→ Left；
/// 右边距屏幕右边 ≤ snap → Right；否则 None。只做左 / 右吸附（需求是「靠边半隐藏」）。
///
/// `win` / `mon` 都是物理边界 `(x, y, 宽, 高)`；打包成元组是为避免参数过多（clippy）。
fn decide_edge(win: (i32, i32, i32, i32), mon: (i32, i32, i32, i32), snap: i32) -> DockEdge {
    let (wx, wy, ww, wh) = win;
    let (mx, my, mw, mh) = mon;
    // 纵向必须与显示器有重叠，否则贴的是别的屏 / 完全在上下方，不该吸附。
    let vertical_overlap = wy < my + mh && wy + wh > my;
    if !vertical_overlap || ww <= 0 || wh <= 0 {
        return DockEdge::None;
    }
    let left_gap = wx - mx;
    let right_gap = mx + mw - (wx + ww);
    if (-snap..=snap).contains(&left_gap) {
        DockEdge::Left
    } else if (-snap..=snap).contains(&right_gap) {
        DockEdge::Right
    } else {
        DockEdge::None
    }
}

fn edge_str(e: DockEdge) -> &'static str {
    match e {
        DockEdge::Left => "'left'",
        DockEdge::Right => "'right'",
        DockEdge::None => "'none'",
    }
}

/// 取物理坐标 (px,py) 所在显示器的物理边界 (x,y,w,h)；不在任何显示器上返回 None。
fn monitor_bounds_at(w: &WebviewWindow, px: i32, py: i32) -> Option<(i32, i32, i32, i32)> {
    let m = w.monitor_from_point(px as f64, py as f64).ok().flatten()?;
    let p = m.position();
    let s = m.size();
    Some((p.x, p.y, s.width as i32, s.height as i32))
}

/// 生成 128-bit 一次性令牌（32 字符小写 hex）；随机源失败返回 None（调用方沿用旧令牌）。
fn new_token() -> Option<String> {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf).ok()?;
    Some(buf.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decide_edge_detects_left_and_right() {
        // 显示器 1920×1080，悬浮球 56×56。
        let mon = (0, 0, 1920, 1080);
        let snap = 42;
        // 贴左缘（gap=0）→ Left
        assert_eq!(decide_edge((0, 100, 56, 56), mon, snap), DockEdge::Left);
        // 左缘外一点（gap=-10，仍在容差内）→ Left
        assert_eq!(decide_edge((-10, 100, 56, 56), mon, snap), DockEdge::Left);
        // 贴右缘：x = 1920-56 = 1864
        assert_eq!(decide_edge((1864, 100, 56, 56), mon, snap), DockEdge::Right);
        // 离两边都远 → None
        assert_eq!(decide_edge((500, 100, 56, 56), mon, snap), DockEdge::None);
        // 超出容差 → None
        assert_eq!(decide_edge((100, 100, 56, 56), mon, snap), DockEdge::None);
    }

    #[test]
    fn decide_edge_requires_vertical_overlap() {
        let mon = (0, 0, 1920, 1080);
        // 球完全在屏幕下方，即使 x 贴左缘也不吸附。
        assert_eq!(decide_edge((0, 2000, 56, 56), mon, 42), DockEdge::None);
    }

    #[test]
    fn collapsed_and_expanded_x_are_inverses() {
        let (mx, mw, ww, peek) = (0, 1920, 56, 28);
        // 左吸附：展开 x=0，半隐藏 x = 0-(56-28) = -28（只露右侧 28px = 半球）。
        assert_eq!(expanded_x(DockEdge::Left, mx, mw, ww), 0);
        assert_eq!(collapsed_x(DockEdge::Left, mx, mw, ww, peek), -28);
        // 右吸附：展开 x=1864，半隐藏 x = 1920-28 = 1892（只露左侧 28px）。
        assert_eq!(expanded_x(DockEdge::Right, mx, mw, ww), 1864);
        assert_eq!(collapsed_x(DockEdge::Right, mx, mw, ww, peek), 1892);
    }

    #[test]
    fn clamp_y_keeps_window_inside_monitor() {
        // 太高（y<上沿）→ 贴顶；太低 → 贴底（my+mh-wh）。
        assert_eq!(clamp_y(-50, 56, 0, 1080), 0);
        assert_eq!(clamp_y(2000, 56, 0, 1080), 1024);
        assert_eq!(clamp_y(200, 56, 0, 1080), 200);
    }

    #[test]
    fn peek_scales_with_dpi_but_never_below_floor() {
        assert_eq!(peek_px(1.0), 28);
        assert_eq!(peek_px(1.5), 42);
        // 异常的极小缩放也至少留 10px，保证半球可悬停。
        assert_eq!(peek_px(0.1), 10);
    }

    #[test]
    fn min_visible_scales_with_floor() {
        assert_eq!(min_visible_px(1.0), 16);
        assert_eq!(min_visible_px(1.5), 24);
        assert_eq!(min_visible_px(0.1), 8);
    }

    #[test]
    fn live_px_is_a_scaled_square_with_floor() {
        assert_eq!(live_px(1.0), (56, 56));
        assert_eq!(live_px(2.0), (112, 112));
        assert_eq!(live_px(1.5), (84, 84));
        // 异常小缩放下也至少 32 物理像素，避免区域退化。
        assert_eq!(live_px(0.1), (32, 32));
    }

    #[test]
    fn live_offset_anchors_column_right_only_for_right_edge() {
        // 200% DPI 真机：宿主被 WebView2 撑到 262 物理，活柱 112 物理。
        let (host_w, live_w) = (262, 112);
        // 右吸附：活柱贴宿主右边，偏移 150（宿主多出的透明 / 点击穿透区在左侧）。
        assert_eq!(live_offset(host_w, live_w, DockEdge::Right), 150);
        // 左吸附 / 自由态：活柱贴左边，柱外区在右侧。
        assert_eq!(live_offset(host_w, live_w, DockEdge::Left), 0);
        assert_eq!(live_offset(host_w, live_w, DockEdge::None), 0);
        // Electron / 未被撑宽的宿主：宿主宽 == 活柱宽，偏移恒 0。
        assert_eq!(live_offset(112, 112, DockEdge::Right), 0);
        // 防御：宿主异常窄于活柱时钳到 0（不允许负偏移把活柱甩出窗）。
        assert_eq!(live_offset(40, 112, DockEdge::Right), 0);
    }

    #[test]
    fn live_and_host_positions_convert_correctly_both_edges() {
        // 活柱几何 ↔ 宿主坐标的换算（on_move / on_drag_end / collapse / persist 共用约定）：
        // host_x = live_x - live_offset；右吸附时两者相差 host_w-live_w。
        let (host_w, live_w) = (262, 112);
        // 右吸附展开态贴显示器右边（显示器 3066 物理）：活柱 x = 3066-112=2954，
        // 宿主 x = 2954-150 = 2804。
        let edge = DockEdge::Right;
        let off = live_offset(host_w, live_w, edge);
        assert_eq!(expanded_x(edge, 0, 3066, live_w), 2954);
        assert_eq!(2954 - off, 2804);
        // 右吸附缩回留半球（peek=56）：活柱 x = 3066-56 = 3010，宿主 x = 2860。
        assert_eq!(collapsed_x(edge, 0, 3066, live_w, 56), 3010);
        assert_eq!(3010 - off, 2860);
        // 左吸附：偏移 0，宿主坐标 == 活柱坐标（-56 露半球）。
        assert_eq!(collapsed_x(DockEdge::Left, 0, 3066, live_w, 56), -56);
    }

    #[test]
    fn point_in_rect_uses_half_open_bounds() {
        // 矩形 [10, 66) × [20, 76)
        assert!(point_in_rect(10, 20, 10, 20, 56, 56));
        assert!(point_in_rect(65, 75, 10, 20, 56, 56));
        // 右 / 下沿是开区间：恰好落在外边上不算在内。
        assert!(!point_in_rect(66, 50, 10, 20, 56, 56));
        assert!(!point_in_rect(50, 76, 10, 20, 56, 56));
        assert!(!point_in_rect(9, 50, 10, 20, 56, 56));
    }

    #[test]
    fn clamp_visible_keeps_a_strip_but_allows_crossing() {
        // 显示器 x ∈ [0, 1920)，球宽 84（150% DPI），最小可见 24。
        let (ww, minv, lo, hi) = (84, 24, 0, 1920);
        // 正常位置原样。
        assert_eq!(clamp_visible(500, ww, lo, hi, minv), 500);
        // 甩向最右：最多到 1920-24=1896，留 24px 在屏内。
        assert_eq!(clamp_visible(3000, ww, lo, hi, minv), 1896);
        // 甩向最左：最多到 0-84+24=-60，仍留 24px 在屏内（其余可越过左缘跨屏）。
        assert_eq!(clamp_visible(-500, ww, lo, hi, minv), -60);
        // 负原点显示器（多显示器）同样成立。
        assert_eq!(clamp_visible(-2000, ww, -1920, 0, minv), -1980);
    }

    #[test]
    fn free_position_is_clamped_fully_inside_monitor_at_drag_end() {
        // 自由态松手时的钳制规则（与 on_drag_end 内联同式）：
        // x ∈ [mx, mx+mw-ww]，y 走 clamp_y。显示器 1920×1080、球 84 物理像素。
        let (mx, my, mw, mh, ww, wh) = (0, 0, 1920, 1080, 84, 84);
        let clamp_x = |x: i32| x.clamp(mx, (mx + mw - ww).max(mx));
        assert_eq!(clamp_x(500), 500);
        assert_eq!(clamp_x(2000), 1920 - 84);
        assert_eq!(clamp_x(-120), 0);
        assert_eq!(clamp_y(2000, wh, my, mh), 1080 - 84);
    }
}
