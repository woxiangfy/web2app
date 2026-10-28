//! 悬浮球「Win32 分层窗口」子模块 —— v1.81 根治渲染毛刺（替代旧的 WebView2 球窗 + 活柱 +
//! WM_NCHITTEST 子类化，那条链见 `super` 模块文档的失败链存档）。
//!
//! ## 为什么必须是原生分层窗口
//! 毛刺的根因是「圆的边缘经过浏览器的裁剪 / 合成管线」：`border-radius` 裁剪 + `filter`
//! 合成层 + `translate(-50%,-50%)` 落到分数物理像素。这里整条链都不存在：
//!   · 球面 = **32bpp 预乘 ARGB 位图**（[`render_ball`] 4×4 超采样画圆盘），
//!     经 `UpdateLayeredWindow` 提交给 DWM，**逐像素 alpha** 合成到桌面；
//!   · 命中 = 分层窗口的**像素 alpha 自动穿透**（alpha=0 的像素把点击原样透传给下层窗口，
//!     含跨进程）—— 不需要任何 `WM_NCHITTEST` 子类化 / 命中圆 / 窗口区域；
//!   · 交互（拖动 / 单击 / 右键 / 悬停）全部在 [`ball_wndproc`] 内完成，没有页面、没有 IPC、
//!     没有一次性令牌。
//!
//! ## 线程约束
//! 原生窗口与它的消息泵都在 Tauri 主线程：建窗 / 销毁 / 显示 / 移动一律经
//! [`super::on_main`] 排队；本模块的读取类访问（`ball_rect` / `ball_scale` 等）可以跨线程。
//!
//! ## 主题
//! 盘面跟随系统深浅色（`HKCU\…\Themes\Personalize\AppsUseLightTheme`）：浅色 = 平涂白底盘，
//! 深色 = 平涂近黑底盘。旧版的蓝色渐变（`#6a9bff → #274db8`）已废弃 —— 悬浮球是系统级
//! 小工具，观感应跟着系统走。`WM_SETTINGCHANGE` / `WM_DWMCOLORIZATIONCHANGED` 时重读重绘。

use super::*;

use std::sync::atomic::AtomicUsize;
use std::sync::OnceLock;

use windows::core::w;
use windows::Win32::Foundation::{COLORREF, ERROR_SUCCESS, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, GetMonitorInfoW,
    MonitorFromPoint, ReleaseDC, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, BI_RGB,
    BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HGDIOBJ, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_VALUE_TYPE,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
// SetCapture / ReleaseCapture / TrackMouseEvent 归属 Win32::UI::Input::KeyboardAndMouse
// （windows 0.62 的模块划分如此，不在 WindowsAndMessaging 里）。
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TRACKMOUSEEVENT, TME_LEAVE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, GetWindowRect, IsWindow,
    IsWindowVisible, LoadCursorW, RegisterClassW, SetWindowPos, ShowWindow, UpdateLayeredWindow,
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, HWND_TOPMOST, IDC_HAND, MA_NOACTIVATE, SWP_NOMOVE,
    SWP_NOACTIVATE, SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA, WM_CANCELMODE, WM_CLOSE,
    WM_DPICHANGED, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE,
    WM_NCDESTROY, WM_PAINT, WM_RBUTTONDOWN, WM_SETTINGCHANGE, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

/// `WM_DWMCOLORIZATIONCHANGED`（0x0320）：windows crate 0.62 未导出该常量。
const WM_DWMCOLORIZATIONCHANGED: u32 = 0x0320;
/// `WM_MOUSELEAVE`（0x02A3）：windows 0.62 只在 `Win32::UI::Controls` 导出；为免多拉一个
/// 巨型模块，与 `WM_DWMCOLORIZATIONCHANGED` 同样本地声明（数值与 SDK 逐字一致）。
const WM_MOUSELEAVE: u32 = 0x02A3;

// ===========================================================================
// 进程级状态（全局唯一球窗）
// ===========================================================================

/// 球窗 HWND（`as usize`）；`0` = 尚未创建 / 已销毁。
static BALL_HWND: AtomicUsize = AtomicUsize::new(0);
/// 建窗时登记的 AppHandle：WndProc 回调进 `events` 状态机要用。
static BALL_APP: Mutex<Option<tauri::AppHandle>> = Mutex::new(None);
/// 指针是否正悬在球上（盘面 hover 提亮用；边沿触发重绘）。
static BALL_HOVERED: AtomicBool = AtomicBool::new(false);
/// 当前是否处于半隐藏（缩回）态 —— 盘面压暗用。由 [`mark_collapsed`] 随吸附态同步。
static BALL_COLLAPSED: AtomicBool = AtomicBool::new(false);
/// 指针是否正按下（一次按压的原语状态，拖动阈值判定用）。
static PTR_DOWN: AtomicBool = AtomicBool::new(false);
/// 本次按压是否已越过拖动阈值（越过 = 拖动，否则松手算单击）。
static PTR_MOVED: AtomicBool = AtomicBool::new(false);
/// 按下点（物理屏幕坐标）：拖动阈值的判定基准。
static PTR_START: Mutex<Option<(i32, i32)>> = Mutex::new(None);
/// 已请求过 `TME_LEAVE` 追踪（每次进入球装一次，避免重复调用）。
static TRACKING_LEAVE: AtomicBool = AtomicBool::new(false);

// ===========================================================================
// 只读访问（跨线程安全；探针 floatprobe 也用）
// ===========================================================================

/// 球窗是否存在（未创建 / 已销毁为 false）。
pub(super) fn ball_exists() -> bool {
    hwnd().is_some()
}

/// 球窗当前是否可见。
pub(crate) fn ball_visible() -> bool {
    match hwnd() {
        Some(h) => unsafe { IsWindowVisible(h).as_bool() },
        None => false,
    }
}

/// 球窗 HWND（探针用）。
pub(crate) fn ball_hwnd() -> Option<isize> {
    hwnd().map(|h| h.0 as isize)
}

/// 球窗的物理外矩形 `(x, y, w, h)`。
pub(crate) fn ball_rect() -> Option<(i32, i32, i32, i32)> {
    rect_of(hwnd()?)
}

/// 球窗所在 DPI 的缩放系数（`GetDpiForWindow / 96`）；窗口不存在时回落 1.0。
pub(crate) fn ball_scale() -> f64 {
    match hwnd() {
        Some(h) => (unsafe { GetDpiForWindow(h) } as f64) / 96.0,
        None => 1.0,
    }
}

/// 当前光标的**物理**屏幕坐标。
pub(crate) fn cursor_screen_pos() -> Option<(i32, i32)> {
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p) }.ok()?;
    Some((p.x, p.y))
}

/// 取物理点所在显示器的物理边界 `(x, y, w, h)`；不在任何显示器上返回 None。
pub(crate) fn monitor_bounds_at(px: i32, py: i32) -> Option<(i32, i32, i32, i32)> {
    bounds_of(unsafe {
        MonitorFromPoint(POINT { x: px, y: py }, MONITOR_DEFAULTTONEAREST)
    })
}

/// 主显示器的物理边界。
pub(crate) fn primary_monitor_bounds() -> Option<(i32, i32, i32, i32)> {
    bounds_of(unsafe {
        MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY)
    })
}

fn bounds_of(hmon: windows::Win32::Graphics::Gdi::HMONITOR) -> Option<(i32, i32, i32, i32)> {
    if hmon.is_invalid() {
        return None;
    }
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(hmon, &mut mi) }.as_bool() {
        return None;
    }
    let r = mi.rcMonitor;
    Some((r.left, r.top, r.right - r.left, r.bottom - r.top))
}

fn hwnd() -> Option<HWND> {
    let h = BALL_HWND.load(Ordering::SeqCst);
    if h == 0 {
        return None;
    }
    let hwnd = HWND(h as *mut core::ffi::c_void);
    if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        Some(hwnd)
    } else {
        None
    }
}

fn rect_of(hwnd: HWND) -> Option<(i32, i32, i32, i32)> {
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut r) }.ok()?;
    Some((r.left, r.top, r.right - r.left, r.bottom - r.top))
}

fn app_handle() -> Option<tauri::AppHandle> {
    BALL_APP
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .cloned()
}

// ===========================================================================
// 建窗 / 显隐 / 销毁
// ===========================================================================

/// 建原生球窗（**必须在主线程**）。恢复上次几何（含吸附态）；无有效几何则按主显示器
/// 右上角默认摆放。建完即绘制并显现 —— 先画后显，避免任何一帧的空白闪烁。
pub(super) fn build_ball_on_main(app: &tauri::AppHandle) {
    if hwnd().is_some() {
        return;
    }
    let saved = crate::store::load(app).float_window;

    // 先按保存坐标（或默认坐标）落一个粗略位置 —— 真实 DPI 要等窗口存在后才知道，
    // 建完再按 DPI 校正尺寸与吸附态位置（见 apply_initial_geometry）。
    let (x0, y0) = provisional_origin(saved);
    let class = register_class();
    let hinst = unsafe {
        GetModuleHandleW(None)
            .map(|m| HINSTANCE(m.0))
            .unwrap_or(HINSTANCE(std::ptr::null_mut()))
    };
    let hwnd = match unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
            class,
            w!(""),
            WS_POPUP,
            x0,
            y0,
            ICON_SIZE_LOGICAL as i32,
            ICON_SIZE_LOGICAL as i32,
            None,
            None,
            Some(hinst),
            None,
        )
    } {
        Ok(h) => h,
        Err(e) => {
            dlog!("[float] 悬浮球窗口创建失败：{e}");
            return;
        }
    };
    BALL_HWND.store(hwnd.0 as usize, Ordering::SeqCst);
    if let Ok(mut g) = BALL_APP.lock() {
        *g = Some(app.clone());
    }

    apply_initial_geometry(app, hwnd, saved);
    paint(hwnd);
    show_ball();
}

/// 粗略建窗原点：保存坐标有效就用它，否则主显示器右上角留白。
fn provisional_origin(saved: Option<FloatWinGeom>) -> (i32, i32) {
    if let Some(g) = saved {
        if g.width >= MIN_PHYS && g.height >= MIN_PHYS {
            return (g.x, g.y);
        }
    }
    match primary_monitor_bounds() {
        Some((mx, my, mw, _mh)) => {
            let m = MARGIN_LOGICAL as i32;
            (mx + mw - ICON_SIZE_LOGICAL as i32 - m, my + m)
        }
        None => (CW_USEDEFAULT, CW_USEDEFAULT),
    }
}

/// 建窗后摆放：优先恢复保存的几何（含吸附态）；几何失效（中心不在任何显示器 / 尺寸过小）
/// 则默认摆放到主显示器右上角。吸附态直接摆成**半隐藏半球**（悬停再展开）。
fn apply_initial_geometry(app: &tauri::AppHandle, hwnd: HWND, saved: Option<FloatWinGeom>) {
    let scale = ball_scale();
    let (lw, lh) = ball_px(scale);
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            lw,
            lh,
            SWP_NOMOVE | SWP_NOACTIVATE,
        );
    }
    if let Some(g) = saved {
        let center_ok =
            monitor_bounds_at(g.x + g.width as i32 / 2, g.y + g.height as i32 / 2).is_some();
        if center_ok && g.width >= MIN_PHYS && g.height >= MIN_PHYS {
            if g.edge != DockEdge::None {
                // 恢复吸附：记录 Dock 并直接摆成半隐藏态（悬停展开 / 移开缩回照旧）。
                if let Some((mx, my, mw, mh)) = monitor_bounds_at(g.x, g.y) {
                    let peek = peek_px(scale);
                    let (x, y) = match g.edge {
                        DockEdge::Left | DockEdge::Right => {
                            (collapsed_x(g.edge, mx, mw, lw, peek), clamp_y(g.y, lh, my, mh))
                        }
                        _ => (
                            clamp_x(g.x, lw, mx, mw),
                            collapsed_y(g.edge, my, mh, lh, peek),
                        ),
                    };
                    set_pos(x, y);
                    set_dock(
                        app,
                        Some(Dock {
                            edge: g.edge,
                            mx,
                            my,
                            mw,
                            mh,
                            collapsed: true,
                        }),
                    );
                    return;
                }
            } else {
                set_pos(g.x, g.y);
                return;
            }
        }
    }
    // 默认（或几何失效）：主显示器右上角留白摆放（自由态）。
    if let Some((mx, my, mw, _mh)) = primary_monitor_bounds() {
        let m = (MARGIN_LOGICAL * scale).round() as i32;
        let x = mx + mw - lw - m;
        let y = my + m;
        set_pos(x, y);
    }
}

/// 显示球窗（不抢前台）。**所有**「把球弄出来」的地方都走这里。
///
/// ⚠️ v1.77 的「show 之后补 `GWL_EXSTYLE` + `ITaskbarList::DeleteTab`」整套补丁一并退役：
/// 那是在 tao 用缓存标志整体重写扩展样式之后的补救。原生窗口的样式**创建时一次定死**
/// （`WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TOPMOST`），
/// 没有任何框架会再改它。
pub(super) fn show_ball() {
    if let Some(h) = hwnd() {
        unsafe {
            // SW_SHOWNOACTIVATE：显示但不激活（与 WS_EX_NOACTIVATE 双保险）。
            let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
        }
    }
}

/// 隐藏球窗（不销毁）。
pub(super) fn hide_ball() {
    if let Some(h) = hwnd() {
        unsafe {
            let _ = ShowWindow(h, SW_HIDE);
        }
    }
    BALL_HOVERED.store(false, Ordering::SeqCst);
    TRACKING_LEAVE.store(false, Ordering::SeqCst);
}

/// 销毁球窗（必须在主线程）。**必须** DestroyWindow 而不是发 `WM_CLOSE` —— 后者被
/// [`ball_wndproc`] 拦成「隐藏」（Alt+F4 语义），功能关闭时只藏起来是静默降级。
pub(super) fn destroy_ball() {
    if let Some(h) = hwnd() {
        unsafe {
            let _ = DestroyWindow(h);
        }
    }
    BALL_HWND.store(0, Ordering::SeqCst);
    BALL_HOVERED.store(false, Ordering::SeqCst);
    BALL_COLLAPSED.store(false, Ordering::SeqCst);
    TRACKING_LEAVE.store(false, Ordering::SeqCst);
    if let Ok(mut g) = BALL_APP.lock() {
        *g = None;
    }
}

/// 移动球窗到物理坐标（左上角）。
pub(super) fn set_pos(x: i32, y: i32) {
    if let Some(h) = hwnd() {
        unsafe {
            let _ = SetWindowPos(h, Some(HWND_TOPMOST), x, y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }
}

fn set_size(hwnd: HWND, w: i32, h: i32) {
    unsafe {
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, w, h, SWP_NOMOVE | SWP_NOACTIVATE);
    }
}

/// 同步「半隐藏」视觉态（盘面压暗）并重绘。由 `events::set_dock` 调用。
pub(super) fn mark_collapsed(collapsed: bool) {
    BALL_COLLAPSED.store(collapsed, Ordering::SeqCst);
    repaint();
}

/// 重绘球面（圆盘 + 图标）。几何移动不需要重绘，只有视觉态 / 尺寸 / 主题变化才需要。
pub(super) fn repaint() {
    if let Some(h) = hwnd() {
        paint(h);
    }
}

// ===========================================================================
// 绘制（圆盘 + 图标 → 32bpp 预乘 ARGB → UpdateLayeredWindow）
// ===========================================================================

/// 重绘球面并提交给 DWM。尺寸 / 位置都取当前窗口矩形，幂等。
fn paint(hwnd: HWND) {
    let Some((x, y, w, h)) = rect_of(hwnd) else {
        return;
    };
    if w <= 0 || h <= 0 {
        return;
    }
    let dark = !apps_use_light_theme();
    let pixels = render_ball(
        w,
        h,
        ball_scale(),
        dark,
        BALL_HOVERED.load(Ordering::SeqCst),
        BALL_COLLAPSED.load(Ordering::SeqCst),
    );
    present(hwnd, &pixels, w, h, x, y);
}

/// 画一颗球：返回 32bpp **预乘** ARGB（内存字节序 BGRA）。
///
/// 圆盘用 4×4 超采样算覆盖率 —— 边缘是**单级覆盖率渐变**（一像素内从 100% 平滑过渡到 0），
/// 与浏览器 CSS 圆角裁剪产生的「白底合成毛刺」完全不同：这里没有任何中间白底。
/// 状态差异（hover 提亮 / 半隐藏压暗）只改平涂色，**不引入 filter / opacity / transform**。
fn render_ball(w: i32, h: i32, scale: f64, dark: bool, hover: bool, collapsed: bool) -> Vec<u8> {
    let n = w.max(h);
    let mut out = vec![0u8; (n * n * 4) as usize];
    if n <= 0 {
        return out;
    }
    let r = n as f64 / 2.0;
    let c = n as f64 / 2.0;
    // 平涂底盘：浅色主题白、深色主题近黑（旧版蓝色渐变已废弃，见模块文档）。
    let (br, bg, bb): (f64, f64, f64) = if dark {
        (30.0, 30.0, 32.0)
    } else {
        (255.0, 255.0, 255.0)
    };
    // 状态微调（替代旧 CSS `filter: brightness(.92 / 1.12)`）。
    let k: f64 = if collapsed {
        0.92
    } else if hover {
        1.06
    } else {
        1.0
    };
    let (br, bg, bb) = ((br * k).min(255.0), (bg * k).min(255.0), (bb * k).min(255.0));
    // 1 物理像素半透明描边：浅色压深、深色提亮；同样走超采样 ⇒ 描边内外都是软边。
    let (sr, sg, sb, sa): (f64, f64, f64, f64) = if dark {
        (255.0, 255.0, 255.0, 0.16)
    } else {
        (0.0, 0.0, 0.0, 0.10)
    };
    let stroke_w = 1.0;
    const SS: i32 = 4; // 4×4 = 16 子样本 / 像素

    for py in 0..n {
        for px in 0..n {
            let (mut cr, mut cg, mut cb, mut cnt) = (0.0, 0.0, 0.0, 0.0f64);
            for sy in 0..SS {
                for sx in 0..SS {
                    let fx = px as f64 + (sx as f64 + 0.5) / SS as f64;
                    let fy = py as f64 + (sy as f64 + 0.5) / SS as f64;
                    let dx = fx - c;
                    let dy = fy - c;
                    let d = (dx * dx + dy * dy).sqrt();
                    if d > r {
                        continue;
                    }
                    if d >= r - stroke_w {
                        cr += br * (1.0 - sa) + sr * sa;
                        cg += bg * (1.0 - sa) + sg * sa;
                        cb += bb * (1.0 - sa) + sb * sa;
                    } else {
                        cr += br;
                        cg += bg;
                        cb += bb;
                    }
                    cnt += 1.0;
                }
            }
            if cnt <= 0.0 {
                continue;
            }
            let alpha = cnt / (SS * SS) as f64; // 覆盖率
            let inv = 1.0 / cnt;
            let idx = ((py * n + px) * 4) as usize;
            // 先写「直通」RGBA，图标叠加后再统一预乘。
            out[idx] = (cb * inv).round().clamp(0.0, 255.0) as u8;
            out[idx + 1] = (cg * inv).round().clamp(0.0, 255.0) as u8;
            out[idx + 2] = (cr * inv).round().clamp(0.0, 255.0) as u8;
            out[idx + 3] = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    // 软件自己的图标（icons/icon.png，编译期内嵌；打包替换图标后天然跟随）。
    if let Some(icon) = load_icon() {
        composite_icon(&mut out, n, icon, scale);
    }
    premultiply_bgra(&mut out);
    out
}

/// 提交位图给 DWM（`UpdateLayeredWindow`，逐像素 alpha）。
fn present(hwnd: HWND, pixels: &[u8], w: i32, h: i32, x: i32, y: i32) {
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return;
        }
        let mem = CreateCompatibleDC(Some(screen));
        if mem.is_invalid() {
            ReleaseDC(None, screen);
            return;
        }
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // 顶向下（与像素缓冲区同序）
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: (w * h * 4) as u32,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bmp = match CreateDIBSection(Some(screen), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(b) => b,
            Err(e) => {
                dlog!("[float] CreateDIBSection 失败，球面未更新：{e}");
                let _ = DeleteDC(mem);
                ReleaseDC(None, screen);
                return;
            }
        };
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, pixels.len());
        let old: HGDIOBJ = SelectObject(mem, bmp.into());
        let pt_dst = POINT { x, y };
        let pt_src = POINT { x: 0, y: 0 };
        let size = SIZE { cx: w, cy: h };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        if let Err(e) = UpdateLayeredWindow(
            hwnd,
            Some(screen),
            Some(&pt_dst),
            Some(&size),
            Some(mem),
            Some(&pt_src),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        ) {
            dlog!("[float] UpdateLayeredWindow 失败，球面未更新：{e}");
        }
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
    }
}

// ---- 图标（icons/icon.png）----

struct IconImage {
    rgba: Vec<u8>,
    w: u32,
    h: u32,
}

/// 解码内嵌图标 PNG（只做一次；失败只留痕，球面退回「无图标」而不是崩）。
fn load_icon() -> &'static Option<IconImage> {
    static ICON: OnceLock<Option<IconImage>> = OnceLock::new();
    ICON.get_or_init(|| {
        match tauri::image::Image::from_bytes(include_bytes!("../../icons/icon.png")) {
            Ok(img) => {
                let (w, h) = (img.width(), img.height());
                if w == 0 || h == 0 {
                    return None;
                }
                Some(IconImage {
                    rgba: img.rgba().to_vec(),
                    w,
                    h,
                })
            }
            Err(e) => {
                dlog!("[float] 解码 icons/icon.png 失败，球面无图标：{e}");
                None
            }
        }
    })
}

/// 把图标按**面积平均**缩放到 `ICON_INSET_LOGICAL` 逻辑像素居中叠到盘面上（直通 alpha over）。
///
/// 面积平均（不是最近邻）是刻意的：图标 256² → 数十物理像素的下采样用最近邻会出噪点 /
/// 锯齿，而这颗球的卖点就是「边缘零毛刺」。
fn composite_icon(out: &mut [u8], n: i32, icon: &IconImage, scale: f64) {
    let side = (ICON_INSET_LOGICAL * scale).round().max(8.0) as i32;
    let side = side.min(n);
    if side <= 0 {
        return;
    }
    let (ox, oy) = ((n - side) / 2, (n - side) / 2);
    let (sw, sh) = (icon.w as f64, icon.h as f64);
    for dy in 0..side {
        let sy0 = dy as f64 * sh / side as f64;
        let sy1 = (dy + 1) as f64 * sh / side as f64;
        for dx in 0..side {
            let sx0 = dx as f64 * sw / side as f64;
            let sx1 = (dx + 1) as f64 * sw / side as f64;
            let y0 = sy0.floor() as u32;
            let y1 = (sy1.ceil() as u32).min(icon.h);
            let x0 = sx0.floor() as u32;
            let x1 = (sx1.ceil() as u32).min(icon.w);
            let (mut sr, mut sg, mut sb, mut sa, mut wsum) = (0.0, 0.0, 0.0, 0.0, 0.0f64);
            for sy in y0..y1 {
                let wy = (sy1.min(sy as f64 + 1.0) - sy0.max(sy as f64)).max(0.0);
                for sx in x0..x1 {
                    let wx = (sx1.min(sx as f64 + 1.0) - sx0.max(sx as f64)).max(0.0);
                    let wgt = wx * wy;
                    if wgt <= 0.0 {
                        continue;
                    }
                    let i = ((sy * icon.w + sx) * 4) as usize;
                    let a = icon.rgba[i + 3] as f64 / 255.0;
                    sr += icon.rgba[i] as f64 * wgt;
                    sg += icon.rgba[i + 1] as f64 * wgt;
                    sb += icon.rgba[i + 2] as f64 * wgt;
                    sa += a * wgt;
                    wsum += wgt;
                }
            }
            if wsum <= 0.0 || sa <= 0.0 {
                continue;
            }
            let (sr, sg, sb, sa) = (sr / wsum, sg / wsum, sb / wsum, sa / wsum);
            let idx = (((oy + dy) * n + (ox + dx)) * 4) as usize;
            let da = out[idx + 3] as f64 / 255.0;
            let oa = sa + da * (1.0 - sa);
            if oa <= 0.0 {
                continue;
            }
            let k = 1.0 - sa;
            out[idx] = ((sb * sa + out[idx] as f64 * da * k) / oa)
                .round()
                .clamp(0.0, 255.0) as u8;
            out[idx + 1] = ((sg * sa + out[idx + 1] as f64 * da * k) / oa)
                .round()
                .clamp(0.0, 255.0) as u8;
            out[idx + 2] = ((sr * sa + out[idx + 2] as f64 * da * k) / oa)
                .round()
                .clamp(0.0, 255.0) as u8;
            out[idx + 3] = (oa * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// 直通 RGBA（BGRA 字节序）→ **预乘** ARGB：`UpdateLayeredWindow` + `AC_SRC_ALPHA` 的硬要求。
fn premultiply_bgra(buf: &mut [u8]) {
    for px in buf.chunks_exact_mut(4) {
        let a = px[3] as u32;
        px[0] = ((px[0] as u32 * a + 127) / 255) as u8;
        px[1] = ((px[1] as u32 * a + 127) / 255) as u8;
        px[2] = ((px[2] as u32 * a + 127) / 255) as u8;
    }
}

// ---- 主题 ----

/// 读系统「应用使用浅色主题」开关；读不到（老系统 / 受限环境）按浅色处理。
fn apps_use_light_theme() -> bool {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            None,
            KEY_READ,
            &mut key,
        ) != ERROR_SUCCESS
        {
            return true;
        }
        let mut ty = REG_VALUE_TYPE(0);
        let mut val = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        let ok = RegQueryValueExW(
            key,
            w!("AppsUseLightTheme"),
            None,
            Some(&mut ty),
            Some(&mut val as *mut u32 as *mut u8),
            Some(&mut size),
        ) == ERROR_SUCCESS;
        let _ = RegCloseKey(key);
        ok && val != 0
    }
}

// ===========================================================================
// 窗口过程：鼠标交互 + 主题重绘
// ===========================================================================

unsafe extern "system" fn ball_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        // 不擦背景：整窗由 UpdateLayeredWindow 提交，GDI 擦除只会闪白。
        WM_ERASEBKGND => return LRESULT(1),
        WM_PAINT => {
            paint(hwnd);
            return LRESULT(0);
        }
        // 显示 / 点击都不激活（与 WS_EX_NOACTIVATE 双保险）。
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_MOUSEMOVE => {
            on_mouse_move(hwnd, lparam);
            return LRESULT(0);
        }
        WM_MOUSELEAVE => {
            TRACKING_LEAVE.store(false, Ordering::SeqCst);
            if BALL_HOVERED.swap(false, Ordering::SeqCst) {
                paint(hwnd);
            }
            if let Some(app) = app_handle() {
                events::on_peek_leave(&app);
            }
            return LRESULT(0);
        }
        WM_LBUTTONDOWN => {
            let p = screen_point(hwnd, lparam);
            PTR_DOWN.store(true, Ordering::SeqCst);
            PTR_MOVED.store(false, Ordering::SeqCst);
            if let Ok(mut g) = PTR_START.lock() {
                *g = Some(p);
            }
            let _ = SetCapture(hwnd);
            if let Some(app) = app_handle() {
                events::on_down(&app, p.0, p.1);
            }
            return LRESULT(0);
        }
        WM_LBUTTONUP => {
            end_press(false);
            return LRESULT(0);
        }
        // 系统打断（捕获被抢 / 菜单弹出等）：已拖动则照常收尾吸附，否则只复位按压态。
        WM_CANCELMODE => {
            end_press(true);
            return LRESULT(0);
        }
        // 右键：**关闭悬浮图标功能并唤回主窗口**（不是「只把球藏起来」，见 super 文档 ⑤）。
        WM_RBUTTONDOWN => {
            if let Some(app) = app_handle() {
                events::on_right_click(&app);
            }
            return LRESULT(0);
        }
        // Alt+F4 / 系统关闭：不销毁，改成隐藏（可再从设置页 / 托盘显示）。
        WM_CLOSE => {
            if let Some(app) = app_handle() {
                super::hide_float_window(&app);
            }
            return LRESULT(0);
        }
        // DPI 变化（跨缩放的显示器）：尺寸随 DPI 重算，位置按吸附态 / 建议矩形摆放。
        WM_DPICHANGED => {
            let scale = (wparam.0 as u32 as f64) / 96.0;
            let (lw, lh) = ball_px(scale);
            set_size(hwnd, lw, lh);
            match app_handle().as_ref().and_then(current_dock) {
                Some(d) => reposition_for_dock(d),
                None => {
                    if let Some((x, y, _, _)) = lparam_as_rect(lparam) {
                        set_pos(x, y);
                    }
                }
            }
            paint(hwnd);
            return LRESULT(0);
        }
        // 主题切换（浅色 ⇄ 深色）：重绘盘面（paint 内部会重读开关）。
        WM_SETTINGCHANGE | WM_DWMCOLORIZATIONCHANGED => {
            paint(hwnd);
            return LRESULT(0);
        }
        WM_NCDESTROY => {
            BALL_HWND.store(0, Ordering::SeqCst);
            BALL_HOVERED.store(false, Ordering::SeqCst);
            BALL_COLLAPSED.store(false, Ordering::SeqCst);
            if let Ok(mut g) = BALL_APP.lock() {
                *g = None;
            }
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// 一次按压收尾：越过阈值 = 拖动（吸附 / 落盘）；未越过则 `WM_LBUTTONUP` = 单击（切换主窗
/// 显隐，去抖）、`WM_CANCELMODE`（canceled=true）= 只复位按压态。与旧球页
/// `endPointer(e, canceled)` 同语义。
fn end_press(canceled: bool) {
    let moved = PTR_MOVED.load(Ordering::SeqCst);
    unsafe {
        let _ = ReleaseCapture();
    }
    PTR_DOWN.store(false, Ordering::SeqCst);
    PTR_MOVED.store(false, Ordering::SeqCst);
    if let Ok(mut g) = PTR_START.lock() {
        *g = None;
    }
    if let Some(app) = app_handle() {
        if moved {
            events::on_drag_end(&app);
        } else if canceled {
            events::on_cancel(&app);
        } else {
            events::on_click(&app);
        }
    }
}

/// 鼠标移动：悬停进出（吸附态展开 / 延迟缩回）+ 拖动阈值判定 + 逐帧跟随。
fn on_mouse_move(hwnd: HWND, lparam: LPARAM) {
    let p = screen_point(hwnd, lparam);
    // 悬停（未按下时才认）：TME_LEAVE 追踪每次进入球装一次。
    if !PTR_DOWN.load(Ordering::SeqCst) {
        if !TRACKING_LEAVE.swap(true, Ordering::SeqCst) {
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            unsafe {
                let _ = TrackMouseEvent(&mut tme);
            }
        }
        if !BALL_HOVERED.swap(true, Ordering::SeqCst) {
            paint(hwnd);
        }
        if let Some(app) = app_handle() {
            events::on_peek_enter(&app);
        }
        return;
    }
    // 按下中：越过阈值才开始拖动（未越过时的松手算单击，与旧球页 DRAG_THRESHOLD 一致）。
    if !PTR_MOVED.load(Ordering::SeqCst) {
        let start = PTR_START
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .unwrap_or(p);
        let thresh = (DRAG_THRESHOLD_LOGICAL * ball_scale()).round().max(2.0) as i32;
        if (p.0 - start.0).abs() + (p.1 - start.1).abs() < thresh {
            return;
        }
        PTR_MOVED.store(true, Ordering::SeqCst);
    }
    if !BALL_HOVERED.swap(true, Ordering::SeqCst) {
        paint(hwnd);
    }
    if let Some(app) = app_handle() {
        events::on_move(&app, p.0, p.1);
    }
}

/// WndProc 的 lparam 是客户区坐标；换算成物理屏幕坐标（拖动 / 吸附都用屏幕坐标）。
fn screen_point(hwnd: HWND, lparam: LPARAM) -> (i32, i32) {
    let mut p = POINT {
        x: (lparam.0 as u16) as i16 as i32,
        y: ((lparam.0 >> 16) as u16) as i16 as i32,
    };
    unsafe {
        let _ = ClientToScreen(hwnd, &mut p);
    }
    (p.x, p.y)
}

fn lparam_as_rect(lparam: LPARAM) -> Option<(i32, i32, i32, i32)> {
    let ptr = lparam.0 as *const RECT;
    if ptr.is_null() {
        return None;
    }
    let r = unsafe { *ptr };
    Some((r.left, r.top, r.right - r.left, r.bottom - r.top))
}

/// 按吸附边把球摆到展开 / 缩回位置（DPI 变化等需要重算时用）。
pub(super) fn reposition_for_dock(d: Dock) {
    let (lw, lh) = ball_px(ball_scale());
    let cur = hwnd().and_then(rect_of).unwrap_or((0, 0, lw, lh));
    let (x, y) = dock_target_pos(d, cur, ball_scale());
    set_pos(x, y);
}

// ===========================================================================
// 窗口类注册
// ===========================================================================

fn register_class() -> windows::core::PCWSTR {
    static DONE: OnceLock<bool> = OnceLock::new();
    let class = w!("web2app_float_ball");
    let ok = DONE.get_or_init(|| unsafe {
        let hinst = GetModuleHandleW(None)
            .map(|m| HINSTANCE(m.0))
            .unwrap_or(HINSTANCE(std::ptr::null_mut()));
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(ball_wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: Default::default(),
            // 悬浮球就是「可点的东西」：光标恒为手型（窗口 == 球，不会有圆外区域）。
            hCursor: LoadCursorW(None, IDC_HAND).unwrap_or_default(),
            hbrBackground: Default::default(),
            lpszMenuName: windows::core::PCWSTR::null(),
            lpszClassName: class,
        };
        RegisterClassW(&wc) != 0
    });
    if !*ok {
        dlog!("[float] 悬浮球窗口类注册失败");
    }
    class
}
