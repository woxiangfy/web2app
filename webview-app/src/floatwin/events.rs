//! 悬浮球「交互状态机」子模块 —— v1.81 起事件源是**原生球窗的 WndProc**（见 [`native`]），
//! 没有球页、没有 IPC、没有一次性令牌。这里只做状态机本身：按下 / 拖动锚点、松手吸附决策、
//! 单击去抖、悬停展开 / 延迟缩回。窗口操作一律经 [`native`] 的 set_pos / mark_collapsed 落地。
//!
//! 坐标约定：所有回调收到的都是**物理屏幕坐标**（WndProc 的 lparam 已换算），
//! 与 [`native::ball_rect`] / 显示器物理边界同一坐标系。

use super::*;

use std::time::Instant;

use tauri::Manager;

// ===========================================================================
// 吸附态存取
// ===========================================================================

pub(super) fn current_dock(app: &tauri::AppHandle) -> Option<Dock> {
    app.state::<FloatState>()
        .dock
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .copied()
}

/// 更新吸附态，并同步半隐藏视觉（盘面压暗）——两条状态永远一起走，不允许只改一半。
pub(super) fn set_dock(app: &tauri::AppHandle, dock: Option<Dock>) {
    let collapsed = dock.map(|d| d.collapsed).unwrap_or(false);
    if let Ok(mut d) = app.state::<FloatState>().dock.lock() {
        *d = dock;
    }
    mark_collapsed(collapsed);
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
// 按压 / 拖动 / 松手
// ===========================================================================

/// 按下：只暂存按下点，不展开、不置 moving。展开 / moving 都推迟到越过拖动阈值的第一帧
/// move（阈值判定在 [`native`]，越过才回调 [`on_move`]）—— 这次按压可能只是一次单击。
pub(super) fn on_down(app: &tauri::AppHandle, sx: i32, sy: i32) {
    if let Ok(mut g) = app.state::<FloatState>().press.lock() {
        *g = Some((sx, sy));
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

/// 拖动跟随（已越过阈值的每一帧）。
///
/// 首帧：用按下点确定抓取偏移。若从半隐藏半球抓起，必须**先展开、后取偏移** —— 展开会把
/// 窗口平移，展开后再以新原点记偏移，这一下平移才不会变成球离手的跳动。绝不能用「当前帧
/// 光标」当按下点：越过阈值时光标已离开按下点，用它记偏移会让首帧窗口纹丝不动
/// （合成指针真机取证实锤）。
pub(super) fn on_move(app: &tauri::AppHandle, cx: i32, cy: i32) {
    let state = app.state::<FloatState>();
    state.moving.store(true, Ordering::SeqCst);
    let scale = ball_scale();
    let (lw, lh) = ball_px(scale);

    let anchor = {
        let mut guard = state.anchor.lock().unwrap_or_else(|p| p.into_inner());
        if guard.is_none() {
            // 取走按下点；缺失（异常时序）时退回用当前帧坐标，保持可拖动。
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
                    expand(app, dock);
                }
            }
            if let Some((wx, wy, _, _)) = ball_rect() {
                *guard = Some((press.0 - wx, press.1 - wy));
            }
        }
        *guard
    };
    let Some((ax, ay)) = anchor else {
        return;
    };
    // 窗口新左上角 = 光标 − 抓取偏移（绝对坐标，丢帧也能在下一帧自愈）。
    let (mut nx, mut ny) = (cx - ax, cy - ay);
    // 拖动中只做「至少留一条窄边在屏内」的软钳制：允许跨屏，不允许整个球甩丢。
    if let Some((mx, my, mw, mh)) = monitor_bounds_at(cx, cy) {
        let minv = min_visible_px(scale);
        nx = clamp_visible(nx, lw, mx, mx + mw, minv);
        ny = clamp_visible(ny, lh, my, my + mh, minv);
    }
    set_pos(nx, ny);
}

/// 松手：四边就近择一吸附（半隐藏 + 悬停展开）或自由摆放（钳回显示器内），落盘几何。
pub(super) fn on_drag_end(app: &tauri::AppHandle) {
    let Some((x, y, ww, hh)) = ball_rect() else {
        clear_drag_runtime(app);
        return;
    };
    // 窗口 == 球：矩形宽高就是球边长。
    let mon = monitor_bounds_at(x + ww / 2, y + hh / 2).or_else(primary_monitor_bounds);
    let Some((mx, my, mw, mh)) = mon else {
        // 被甩到屏间空隙且主显示器都读不到（几乎不可能）：只清态，不动几何。
        set_dock(app, None);
        persist_geometry(app);
        clear_drag_runtime(app);
        return;
    };
    let snap = snap_px(ball_scale());
    let edge = decide_edge((x, y, ww, hh), (mx, my, mw, mh), snap);
    match edge {
        DockEdge::None => {
            // 自由态：把球完整钳回显示器内（不钳的话球可被甩到屏外找不回来 ——
            // 它没有标题栏，移出屏幕的部分无法再抓）。
            let nx = clamp_x(x, ww, mx, mw);
            let ny = clamp_y(y, hh, my, mh);
            if nx != x || ny != y {
                set_pos(nx, ny);
            }
            set_dock(app, None);
            persist_geometry(app);
            dlog!("[float] 松手：自由摆放（未吸附）");
        }
        e => {
            // 吸附：记录 Dock → 展开态贴边摆放 → 落盘展开态坐标 → 立即缩回只露半球。
            let dock = Dock {
                edge: e,
                mx,
                my,
                mw,
                mh,
                collapsed: false,
            };
            apply_dock(app, dock, false);
            persist_geometry(app);
            apply_dock(app, dock, true);
            dlog!("[float] 松手吸附：edge={} 已半隐藏", edge_str(e));
        }
    }
    clear_drag_runtime(app);
}

/// 单击（未越过拖动阈值的松手）：切换【原来的主窗口】显隐。
/// 单击也结束一次按压：清掉暂存按下点 / moving，否则残留的 moving 会让后续
/// 悬停缩回被「拖动中」早退永久挡掉。
pub(super) fn on_click(app: &tauri::AppHandle) {
    // 去抖：系统双击会送来两次松手，两次 toggle 相互抵消，用户只看到主窗「闪现」一下。
    let now = monotonic_ms();
    let last = app
        .state::<FloatState>()
        .last_click_ms
        .swap(now, Ordering::SeqCst);
    if now.saturating_sub(last) < CLICK_DEBOUNCE_MS {
        clear_drag_runtime(app);
        return;
    }
    clear_drag_runtime(app);
    crate::tray::toggle_window(app);
}

/// 系统打断（捕获被抢等）且未拖动：只复位按压 / 拖动态，不做吸附判定、不弹主窗。
pub(super) fn on_cancel(app: &tauri::AppHandle) {
    clear_drag_runtime(app);
}

/// 右键：**关闭悬浮图标功能并唤回主窗口**（v1.76，见 [`disable_float_from_icon`]）。
/// ⚠️ 不是 `hide_float_window` —— 只把球藏起来会留下「进程还在跑、桌面上却再也
///    唤不出窗口」的死局（2026-10-11 使用者实测），契约测试钉着这一条。
pub(super) fn on_right_click(app: &tauri::AppHandle) {
    disable_float_from_icon(app);
}

fn monotonic_ms() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

// ===========================================================================
// 悬停展开 / 延迟缩回
// ===========================================================================

pub(super) fn on_peek_enter(app: &tauri::AppHandle) {
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
        expand(app, dock);
    }
}

pub(super) fn on_peek_leave(app: &tauri::AppHandle) {
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
    // 不立刻缩回：鼠标在屏幕边缘移动时，窗口半球宽的展开 / 缩回平移会连环触发进出事件，
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
pub(super) fn finish_peek_leave(app: &tauri::AppHandle, gen: u64) {
    let state = app.state::<FloatState>();
    if state.peek_gen.load(Ordering::SeqCst) != gen || state.moving.load(Ordering::SeqCst) {
        return;
    }
    if !ball_visible() {
        return;
    }
    let Some(dock) = current_dock(app) else {
        return;
    };
    if dock.collapsed {
        return;
    }
    let cur = ball_rect().unwrap_or((dock.mx, dock.my, 0, 0));
    let (x, y) = dock_target_pos(
        Dock {
            collapsed: true,
            ..dock
        },
        cur,
        ball_scale(),
    );
    let (lw, lh) = ball_px(ball_scale());
    if let Some((px, py)) = cursor_screen_pos() {
        if point_in_rect(px, py, x, y, lw, lh) {
            return;
        }
    }
    collapse(app, dock);
}

// ===========================================================================
// 展开 / 缩回
// ===========================================================================

/// 展开到屏内（dock.collapsed=false）。
pub(super) fn expand(app: &tauri::AppHandle, d: Dock) {
    apply_dock(app, d, false);
}

/// 缩回屏外只露半球（dock.collapsed=true）。
pub(super) fn collapse(app: &tauri::AppHandle, d: Dock) {
    apply_dock(app, d, true);
}

/// 摆放球窗到展开 / 缩回位置并落吸附态（视觉压暗随 set_dock 同步）。
fn apply_dock(app: &tauri::AppHandle, d: Dock, collapsed: bool) {
    let next = Dock { collapsed, ..d };
    if let Some(cur) = ball_rect() {
        let (x, y) = dock_target_pos(next, cur, ball_scale());
        set_pos(x, y);
    }
    set_dock(app, Some(next));
}

// ===========================================================================
// 真机探针入口
// ===========================================================================

/// ⚠️ **仅供真机探针**（`floatprobe`）：把一次交互直接喂给状态机（[`on_down`] / [`on_move`] /
/// [`on_drag_end`] / [`on_click`] …），走**真实**窗口操作 —— 唯一跳过的是 WndProc 收消息
/// 那一段。v1.81 起球窗是原生窗口（无球页），但探针入口保留：真机取证依旧要能逐事件驱动
/// 状态机（拖动 / 吸附 / 展开缩回恰恰是真机最需要验证的部分）。
///
/// `sx` / `sy` 是**物理**屏幕坐标（与 WndProc 换算后的口径一致）。
pub(crate) fn inject_page_event(app: &tauri::AppHandle, event: &str, sx: f64, sy: f64) -> bool {
    let (x, y) = (sx.round() as i32, sy.round() as i32);
    match event {
        "down" => on_down(app, x, y),
        "move" => on_move(app, x, y),
        "cancel" => on_cancel(app),
        "dragend" => on_drag_end(app),
        "peekenter" => on_peek_enter(app),
        "peekleave" => on_peek_leave(app),
        "hide" => on_right_click(app),
        "click" => on_click(app),
        _ => return false,
    }
    true
}
