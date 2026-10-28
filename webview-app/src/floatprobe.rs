//! 悬浮球**真机合成指针 harness**（仅 Windows；只有 `--float-probe` 才会跑到）。
//!
//! ## 为什么需要它
//! 在此之前，悬浮球的交互只有三类证据：① 纯几何函数单测（`floatwin::tests`）；② 静态契约
//! 守卫（`contract_tests::float_window_contract`，钉的是**源码字面量**）；③ 真机**人工**观察。
//! 它们都证不了「一次拖动 / 一次悬停 → 窗口**真的**按预期移动 / 吸附 / 缩回」。本 harness 就是
//! 补这一格：在**真实进程内**跑，真实建球窗、真实状态机分发 + 真实窗口操作，再读回**真实窗口
//! API** 的结果。
//!
//! ## v1.81 起球窗是原生分层窗口（不再是 WebView2 球页）
//! 球窗 = `WS_EX_LAYERED` + `UpdateLayeredWindow` 的 32bpp 预乘 ARGB 位图圆盘（见 `floatwin`
//! 模块文档）：**窗口 == 球**。旧实现的「活柱」（WebView2 撑宽宿主窗）、球页 `floaticon.html`、
//! 一次性令牌、`WM_NCHITTEST` 圆形命中子类化、窗口区域裁剪**全部退役** —— 命中就是**像素
//! alpha**（alpha=0 的像素把点击自动透传给下层），由系统合成器完成，没有应用侧实现可断言。
//! 因此旧探针的「页面链路诊断」（`phase_page_link`）与命中测试链（`has_window_region` /
//! `hit_hwnds` / `nchittest` / `check_circle_hit_testing`）一并删除，形态断言改为
//! 「原生 HWND + 扩展样式 + 球窗尺寸 == `ball_px(scale)`」。
//!
//! ## 驱动方式：直接注入事件（不经过 WndProc）
//! 交互事件经 [`floatwin::inject_page_event`] 直接喂给状态机（`on_down` / `on_move` /
//! `on_drag_end` / `on_click` …），走**真实**窗口操作 —— 唯一跳过的是「WndProc 收消息 + 其中
//! 的拖动阈值判定」那一段。`sx` / `sy` 是**物理**屏幕坐标（与 WndProc 换算后的口径一致）。
//!
//! ## 断言什么
//!   · [`floatwin::ball_rect`] / [`floatwin::ball_scale`] → 真实窗口几何（窗口 == 球）；
//!   · `dock_state` → 内部吸附态，与上面的几何**对照**断言（几何对、状态错同样是缺陷，
//!     只看一侧会漏）；
//!   · 四边吸附（左 / 右 / 上 / 下）：松手就近吸附 + 立即缩回半球（`peek_px` 露出）、悬停展开
//!     （`expanded_x` / `expanded_y`）、移开 ~300ms 延迟缩回（`collapsed_x` / `collapsed_y`；
//!     抖动吸收 + 真实光标位置双检）；
//!   · 自由态拖动 1:1 跟手 + 软钳制（`min_visible_px`）+ 松手钳回；单击 toggle 主窗；
//!     右键 = 关闭悬浮图标功能 + 销毁球 + 唤回主窗口。
//!
//! ## 隔离
//! ① 清掉数据目录里的 `settings.json`（起始几何回到「主显示器右上角自由态」，可重复）；
//! ② 把真实光标挪到主显示器左上角并在结束时**还原**（缩回逻辑会读真实光标位置 —— 光标若
//!    落在「缩回后矩形」内，缩回会被有意放弃，那会让断言偶发失败）；
//! ③ 逐行输出到 stdout + `%TEMP%\web2app-float-probe.txt`；
//! ④ 退出码 = 失败断言条数（0 = 全过；**97** = 事件循环没起来、**98** = 看门狗超时 —— 两者都是
//!    **环境性**故障，与悬浮球实现无关，别当成回归）。
//!
//! ## 建窗时机（本 harness 最容易踩的雷）
//! 真机实测：**在 `setup` 里请求建第二个窗**（`show_float_window` 内部是 `run_on_main_thread`
//! **排队**建窗）会让排队任务落在「事件循环刚启动」的那一刻，撞上主 WebView2 的初始化 ⇒ 主线程
//! 此后**不再响应任何任务**（连排队的 `app.exit` 都送不到），整轮挂死到看门狗 —— 而日志症状是
//! 「主线程往返 60s 不就绪」，看上去像环境故障，实际是自己埋的雷。生产侧不会踩到（托盘 / 设置页
//! 触发时事件循环已跑了很久）。建窗只有两种安全姿势：① `setup` 内**同步**建
//! （`build_fake_main_direct`，与生产 `build_main_window` 同姿势）；② 事件循环**跑起来之后**再
//! 排队建（本 harness 的球窗走这条：先 `wait_main_ready` 确认往返，再 `show_float_window`）。
//! （v1.81 起球窗本身是原生窗口、建得飞快，但「排队落点」的时序雷与建的是什么窗无关。）
//!
//! ## 运行
//! ```text
//! cd webview-app && cargo run -- --float-probe
//! ```
//! ⚠️ 建议**换一个全新目录**跑（把 exe 拷过去再执行）：假主窗的 WebView2 UDF 就在「exe 同级 /
//! `data_dir`」下，被强杀过的旧 UDF 可能带残留锁，让下一次启动卡死（本机实测）。全新目录
//! 天然隔离，也不必删除任何东西。
//! ⚠️ release 是 `windows_subsystem = "windows"`，没有控制台可打印（结果文件仍在，但退出码
//! 要靠调用方读）；要看输出就用 debug 构建。

use std::time::{Duration, Instant};

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use windows::Win32::UI::WindowsAndMessaging::{
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
};

use crate::floatwin::{
    ball_hwnd, ball_px, ball_rect, ball_scale, ball_visible, collapsed_x, collapsed_y, dock_state,
    expanded_x, expanded_y, min_visible_px, peek_px, show_float_window, snap_px,
};
use crate::store::DockEdge;

/// 触发探针的命令行开关。`main.rs` 不做参数解析，这里自己扫 argv。
pub(crate) const PROBE_FLAG: &str = "--float-probe";

/// 「假主窗」的 label：必须与 `nav::main_window_label()`（单窗口模式恒为 `"main"`）
/// 一致，否则「单击悬浮球 → toggle 主窗」这条断言没有观察点。
const MAIN_LABEL: &str = "main";

/// 建球的等待上限；到点还没「存在且可见」就如实记录并收工。
///
/// ⚠️ `show_float_window` 是「spawn 线程 + `run_on_main_thread` 排队建窗」的**异步**路径，
/// 建窗任务何时真正跑起来不由探针决定（旧版还叠着 WebView2 球页冷启动，真机实测曾慢到
/// 超过 120s；v1.81 起球窗是原生分层窗口，正常是毫秒级）。上限仍按最慢环境给足 180s：
/// 这是**环境等待**上限、不是对产品行为的断言，给宽了只是让慢机器也能取证，不掩盖任何产品
/// 缺陷（真挂死由看门狗兜底）。
///
/// ⚠️⚠️ 而且**绝不能"失败就 `destroy` 重来"**：`show_float_window` 是「spawn 线程 +
/// `run_on_main_thread` 排队建窗」的**异步**路径，建窗任务何时真正跑起来不由探针决定；
/// 在它跑完之前 `destroy_float_window`，就是在**反复打断建窗** ⇒ 球永远出不来。
/// `run-G / I / K / L` 四次全栽在这：每轮日志都写「窗口不存在」，而失败收尾打印的 labels 里
/// **始终有 `"float"`** —— 窗口其实一直在建，只是被一次次掐掉。
const BUILD_WAIT: Duration = Duration::from_secs(180);
/// 「复原」（v1.76 起 = **重建**球窗）的等待上限。比 `BUILD_WAIT` 小：此时窗口类已注册、
/// 进程环境已热（`wait_env` 早就等过一次），只差「排队建窗」这一段。若与 `BUILD_WAIT` 同取
/// 180s，整轮最坏值会顶穿看门狗（见下）。
const RESTORE_WAIT: Duration = Duration::from_secs(120);
/// 看门狗秒数：整轮探针正常约 15–60s，超过这个数一定是卡死了。
///
/// ⚠️ v1.76 起要给到 330：球上右键改成**销毁**球窗后，「复原」那一步是**重建**（见
/// `phase_hide` 尾部的轮询），于是整轮里出现**第二次**窗口构建等待。按
/// `wait_env(≤BUILD_WAIT 180) + 各 phase(~15) + 复原(≤RESTORE_WAIT 120)` 的最坏值 ≈ 315s，
/// 原本的 240s 会把「两次都慢但都在推进」的正常轮次误判成卡死（退出码 98）。
const WATCHDOG_SECS: u64 = 330;
/// 等主线程 / 事件循环就绪的上限（真机偶发启动停摆；超时按**环境性**故障单独报出）。
const MAIN_READY_WAIT_SECS: u64 = 60;
/// 单次主线程派发的等待上限。给得宽：受限环境下 WebView2 首次创建环境偶发要几十秒，
/// 短超时会把「慢」误报成「没执行」。
const MAIN_CALL_TIMEOUT: Duration = Duration::from_secs(90);

pub(crate) fn is_probe_mode() -> bool {
    std::env::args().any(|a| a == PROBE_FLAG)
}

/// 探针入口（在 `setup` 里、主线程调用）。
pub(crate) fn run(app: tauri::AppHandle) {
    // ⚠️⚠️ 看门狗必须**第一个**启动：它保护的是整轮探针，**包括下面这段同步建窗**。真机实测
    //    `WebviewWindowBuilder::build` 内部的 WebView2 环境创建会偶发停摆（HWND 已经建出来、
    //    标题都能读到，`build()` 却迟迟不返回）。看门狗若放在末尾启动，这种卡死就没人管 ——
    //    进程永远活着（实测只能人工杀），比断言失败更难定位。
    spawn_watchdog();

    // ---------- 建窗刻意留在这里（主线程、`setup` 内）同步做 ----------
    // ⚠️⚠️ **绝不能**改成「丢给 `run_on_main_thread` 排队去建」：真机实测 —— 在事件循环刚
    //    启动的那一刻执行「建 WebView2 窗」，会撞上嵌套初始化，主线程此后**不再响应任何
    //    任务**（整轮探针挂死；连排队的 `app.exit` 都送不到，只能靠看门狗强杀）。
    //    这正是 `settings.rs` 记的「不能在调用栈里直接 build()」的反面：建窗只有两种安全姿势
    //    —— ① `setup` 内直接建（正常路径的 `build_main_window` 就是）；② 事件循环跑起来之后
    //    再排队建（`show_float_window` 的做法）。唯独「刚启动就排队建」不行。
    let mut r = Report::new();
    r.section("准备");
    reset_store(&app, &mut r);
    // 假主窗：`WebviewWindowBuilder::build` 的**同步**建窗 —— 与生产 `build_main_window` 同一
    // 姿势。⚠️ 建窗**前后各打一行**：真机实测这一步会偶发卡在 WebView2 环境创建（HWND 已经
    // 建出来、标题都能读到，`build()` 却不返回），没有这两行就只看到「准备」标题，
    // 分不清是卡在建窗还是卡在别处。
    r.note("开始建「假主窗」（同步建窗；本机偶发卡在 WebView2 环境创建 → 看门狗 98）");
    let main_built = build_fake_main_direct(&app);
    r.note(format!("「假主窗」建窗返回：{main_built}"));
    let saved_cursor = park_cursor(&mut r);
    // ⚠️⚠️ 悬浮球**绝不能在这里建**：`show_float_window` 内部是 `run_on_main_thread` **排队**建窗，
    //    而这个排队任务会落在「事件循环刚启动」的那一刻 —— 真机实测它会撞上主 WebView2 的初始化，
    //    主线程此后**不再响应任何任务**（连排队的 `app.exit` 都送不到），整轮挂死到看门狗。
    //    症状极具误导性：日志停在「主线程往返 60s 不就绪」，看起来像环境故障，其实是本函数
    //    自己在 `setup` 里埋的雷。生产侧不会踩到（托盘 / 设置页 触发时事件循环已跑了很久）。
    //    ⇒ 球窗改到 `drive` 里、`wait_main_ready` **之后**再建（见那里的注释）。
    std::thread::spawn(move || {
        let code = drive(&app, r, main_built, saved_cursor);
        // 正常路径：`app.exit` 走 `RunEvent::Exit` 收尾，进程在 3s 内必然结束，兜底不会执行。
        app.exit(code);
        std::thread::sleep(Duration::from_secs(3));
        // 兜底：主线程若已停摆，`app.exit` 送不到 ⇒ 硬退，保证退出码语义可见（否则只剩看门狗 98）。
        eprintln!("[float-probe] app.exit({code}) 未被事件循环处理，改为强制退出");
        std::process::exit(code);
    });
}

/// 看门狗：真机探针**绝不允许挂死**。任何一步阻塞（建窗 / 窗口 API / 事件循环）到点
/// 直接退出并给一个专用退出码 —— 挂死会被 CI 判成超时，比失败更难定位。逐行输出在 `Report`
/// 里做（每行即刷），超时时也能看到停在哪一条。
///
/// 由 `run()` **在最早位置**启动（见那里的注释：建窗本身就可能卡死）。
fn spawn_watchdog() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(WATCHDOG_SECS));
        eprintln!(
            "[float-probe] 总超时（{WATCHDOG_SECS}s）：疑似某一步阻塞，强制退出（退出码 98）"
        );
        std::process::exit(98);
    });
}

// ===========================================================================
// 主流程
// ===========================================================================

fn drive(
    app: &tauri::AppHandle,
    mut r: Report,
    main_built: bool,
    saved_cursor: Option<(i32, i32)>,
) -> i32 {
    // ⚠️ 先等主线程「真的会执行任务」。`floatprobe::run` 是在 `setup` 里被调用的，那时事件循环
    //    还没起来；而且真机实测**偶发**整轮卡在「主线程任务永远不被执行」（受限环境下
    //    WebView2 / 事件循环初始化停摆，与悬浮球无关）。不做这一步，这种环境故障会伪装成
    //    十几条几何断言失败，把原因埋掉。
    if !wait_main_ready(app, &mut r) {
        let _ = r.finish();
        restore_cursor(saved_cursor);
        // ⚠️ 这里**不能**只 `return 97` 交给 `app.exit(97)`：事件循环没起来时 `app.exit` 本身也要
        //    派发到主线程，必然送不到 ⇒ 进程一直活到看门狗（退出码 98），把「环境没起来」误报成
        //    「某一步阻塞」。环境性故障要**确定性地快速退出**。
        std::process::exit(97);
    }
    r.check(
        "建「假主窗」(label=main，供单击 toggle 断言)",
        main_built,
        if main_built {
            ""
        } else {
            "WebviewWindowBuilder::build 失败（窗口环境没起来）"
        },
    );
    if !main_built {
        restore_cursor(saved_cursor);
        return r.finish();
    }

    // ⭐ 事件循环已稳（`wait_main_ready` 刚刚往返成功）—— **现在**才走生产路径建悬浮球窗，
    //    与托盘 / 设置页触发时的时序一致，避开「事件循环刚启动」那段初始化窗口期（见 `run()`）。
    show_float_window(app);

    let Some(env) = wait_env(app, &mut r) else {
        r.check("悬浮球窗能建出来并显现", false, "见上方重试记录");
        restore_cursor(saved_cursor);
        return r.finish();
    };
    r.note(format!(
        "缩放 {:.2}｜球窗 {}×{} 物理（窗口 == 球）｜主显示器 ({},{} {}×{})",
        env.scale, env.lw, env.lh, env.mx, env.my, env.mw, env.mh
    ));
    sleep(500);

    phase_ball_frame(app, &env, &mut r);
    phase_free_drag(app, &env, &mut r);
    phase_right_dock(app, &env, &mut r);
    phase_peek(app, &env, &mut r);
    phase_left_dock(app, &env, &mut r);
    phase_top_dock(app, &env, &mut r);
    phase_bottom_dock(app, &env, &mut r);
    phase_click(app, &env, &mut r);
    phase_hide(app, &mut r);

    r.section("说明");
    r.note(
        "本 harness 用**直接注入事件**驱动状态机（见模块文档「驱动方式」）：唯一跳过的是 \
         WndProc 收消息与其中的拖动阈值判定（5 逻辑像素，native::on_mouse_move）。因此 \
         「拖动阈值」「像素 alpha 点击穿透」（WS_EX_LAYERED 的系统合成行为）与「单击去抖的 \
         时序」（CLICK_DEBOUNCE_MS）不在这里覆盖。",
    );
    restore_cursor(saved_cursor);
    r.finish()
}

/// 球窗形态（v1.81 原生分层窗口）：存在 / 可见 / 尺寸 == `ball_px(scale)` / 扩展样式。
///
/// 命中不再有应用侧实现（像素 alpha 穿透是系统合成器行为），可断言的「形态」只剩
/// 「这是个原生分层球窗」本身：窗口 == 球（无活柱撑宽）、`WS_EX_LAYERED`（逐像素 alpha
/// 合成 + 命中穿透的前提）、`WS_EX_TOOLWINDOW` / `WS_EX_NOACTIVATE` / `WS_EX_TOPMOST`。
fn phase_ball_frame(app: &tauri::AppHandle, env: &Env, r: &mut Report) {
    r.section("零、球窗形态（v1.81 原生分层窗口；无球页 / 无命中圆，命中 = 像素 alpha 穿透）");

    // 球不再是 WebviewWindow：`get_webview_window("float")` 恒为 None（钉住「窗口 == 球」）。
    let still_webview = main_call(app, |a| a.get_webview_window("float").is_some()).unwrap_or(true);
    r.check(
        "球窗不再注册为 WebviewWindow（无宿主 / 无活柱）",
        !still_webview,
        &format!("get_webview_window(\"float\") = {still_webview}"),
    );

    let Some(hwnd) = main_call(app, |_| ball_hwnd()).flatten() else {
        r.check("球窗存在（原生 HWND）", false, "ball_hwnd = None");
        return;
    };
    r.check("球窗存在（原生 HWND）", true, &format!("hwnd = {hwnd:#x}"));
    r.check("球窗可见", ball_visible(), "");

    let rect = read_ball(app);
    r.check(
        "球窗尺寸 == ball_px(scale)（窗口 == 球，不再有活柱 / 宿主被撑宽）",
        rect.map(|(_, _, w, h)| (w, h)) == Some((env.lw, env.lh)),
        &format!(
            "实际 {:?}，期望 ({},{})",
            rect.map(|(_, _, w, h)| (w, h)),
            env.lw,
            env.lh
        ),
    );

    // 像素 alpha = 命中（环境诊断，**不计入失败**）：球心处 `WindowFromPoint` 应命中球窗本身，
    // 球窗角点（圆外 alpha=0）应**穿透**到下层窗口 —— 这是「alpha=0 透传点击」的系统合成器
    // 行为，没有应用侧实现可断言，只做取证（本机环境不同，判「产品坏了」不公平）。
    if let Some((bx, by, bw, bh)) = rect {
        let hit_center = window_from_point(bx + bw / 2, by + bh / 2);
        r.diag(
            "球心处 WindowFromPoint == 球窗（命中在球上）",
            hit_center == Some(hwnd),
            &format!("实际 {hit_center:?}，期望球窗 {hwnd:#x}"),
        );
        let hit_corner = window_from_point(bx + 1, by + 1);
        r.diag(
            "球窗角点（圆外 alpha=0）WindowFromPoint ≠ 球窗（点击穿透给下层）",
            hit_corner != Some(hwnd),
            &format!("实际 {hit_corner:?}"),
        );
    }

    // 扩展样式：建窗时一次定死（见 `floatwin/native.rs`），没有框架会再改它。
    let ex = exstyle_of(hwnd);
    let styles: [(&str, u32); 4] = [
        ("WS_EX_LAYERED（逐像素 alpha；命中穿透的前提）", WS_EX_LAYERED.0),
        ("WS_EX_TOOLWINDOW（不进任务栏 / Alt+Tab）", WS_EX_TOOLWINDOW.0),
        ("WS_EX_NOACTIVATE（显示 / 点击都不抢前台）", WS_EX_NOACTIVATE.0),
        ("WS_EX_TOPMOST（独立置顶）", WS_EX_TOPMOST.0),
    ];
    for (name, bit) in styles {
        r.check(name, ex & bit != 0, &format!("GWL_EXSTYLE = {ex:#x}"));
    }
}

/// 自由态拖动：① 1:1 跟手；② 拖动中软钳制；③ 松手完整钳回。
fn phase_free_drag(app: &tauri::AppHandle, env: &Env, r: &mut Report) {
    r.section("一、自由态拖动（1:1 跟手 / 软钳制 / 松手钳回）");
    let Some((x0, y0, _, _)) = read_ball(app) else {
        r.check("读初始球窗几何", false, "读不到");
        return;
    };

    // 抓取偏移 = 0（按下点 == 球窗左上角）：于是「光标位置」就等于「球窗目标位置」，
    // 断言可以写成纯等式，不必再推 anchor。真实用户抓的是球心附近，偏移 ∈ [0, 球宽)。
    press(app, x0, y0);
    sleep(80);
    let (tx, ty) = (x0 - 200, y0 + 60);
    drag_to(app, tx, ty);
    sleep(320);
    let got = read_ball(app).map(|b| (b.0, b.1));
    r.check(
        "拖动 1:1 跟手（球窗位移 == 光标位移；增量协议只有半速且抖）",
        got == Some((tx, ty)),
        &format!("实际 {got:?}，期望 ({tx},{ty})"),
    );

    // 软钳制：光标压到显示器右下角内 8px（必须在显示器**内** —— `monitor_bounds_at` 的
    // MONITOR_DEFAULTTONEAREST 对屏外点也能就近回显示器，但断言以屏内为准更直观）。
    let (cx, cy) = (env.mx + env.mw - 8, env.my + env.mh - 8);
    let minv = min_visible_px(env.scale);
    drag_to(app, cx, cy);
    sleep(320);
    let got = read_ball(app).map(|b| (b.0, b.1));
    r.check(
        "拖动中软钳制：右下各留最小可见条（球不会被整个甩出屏）",
        got == Some((env.mx + env.mw - minv, env.my + env.mh - minv)),
        &format!(
            "实际 {got:?}，期望 ({},{})",
            env.mx + env.mw - minv,
            env.my + env.mh - minv
        ),
    );

    release_drag(app);
    sleep(320);
    let got = read_ball(app).map(|b| (b.0, b.1));
    r.check(
        "自由态松手：完整钳回显示器内（右下角贴边）",
        got == Some((env.mx + env.mw - env.lw, env.my + env.mh - env.lh)),
        &format!(
            "实际 {got:?}，期望 ({},{})",
            env.mx + env.mw - env.lw,
            env.my + env.mh - env.lh
        ),
    );
}

/// 右缘吸附：松手即吸附 + 立即缩回半球。
fn phase_right_dock(app: &tauri::AppHandle, env: &Env, r: &mut Report) {
    r.section("二、右缘吸附");
    let Some((x0, y0, _, _)) = read_ball(app) else {
        r.check("读球窗几何", false, "读不到");
        return;
    };
    press(app, x0, y0);
    sleep(80);
    // 目标：贴右缘内 10px。10 必须落在[吸附容差]内才算 Right —— 用生产代码同一个 snap_px
    // 把它算出来记进结论，免得容差改了这里悄悄失效（跨 DPI 时它随缩放变大）。
    let snap = snap_px(env.scale);
    r.note(format!("吸附容差 snap = {snap} 物理像素；贴边测试点取 +10"));
    let (tx, ty) = (env.mx + env.mw - env.lw + 10, env.my + env.mh / 2);
    drag_to(app, tx, ty);
    sleep(320);
    release_drag(app);
    sleep(400);

    let (edge, collapsed) = dock_state(app);
    let got = read_ball(app);
    r.check(
        "松手贴右缘 → 内部吸附态 = Right 且已缩回半球",
        edge == DockEdge::Right && collapsed,
        &format!("dock={edge:?} collapsed={collapsed}"),
    );
    r.check(
        "缩回态球窗 x == 显示器右边 − 半球宽（只露半球）",
        got.map(|b| b.0) == Some(env.mx + env.mw - peek_px(env.scale)),
        &format!(
            "实际 {:?}，期望 {}",
            got.map(|b| b.0),
            env.mx + env.mw - peek_px(env.scale)
        ),
    );
    r.check(
        "缩回态球窗 y == 松手时钳制后的 y（纵向不越界）",
        got.map(|b| b.1) == Some(env.my + env.mh / 2),
        &format!("实际 {:?}，期望 {}", got.map(|b| b.1), env.my + env.mh / 2),
    );
}

/// 悬停展开 / 抖动吸收 / 真离开缩回（都在右吸附态下做）。
fn phase_peek(app: &tauri::AppHandle, env: &Env, r: &mut Report) {
    r.section("三、悬停展开 / 抖动吸收 / 延迟缩回");
    // 展开位来自与生产代码**同一个**契约函数（不在这里重写一遍几何公式）。
    let expanded = expanded_x(DockEdge::Right, env.mx, env.mw, env.lw);
    let collapsed = env.mx + env.mw - peek_px(env.scale);

    peek_enter(app);
    sleep(260);
    let (e1, c1) = dock_state(app);
    let l1 = read_ball(app).map(|b| b.0);
    r.check(
        "悬停半球 → 展开到屏内（球窗 x == 显示器右边 − 球宽）",
        e1 == DockEdge::Right && !c1 && l1 == Some(expanded),
        &format!("dock={e1:?} collapsed={c1} x={l1:?}，期望 x={expanded}"),
    );

    // 抖动吸收：离开后 300ms 未到就重新进入 —— 排队中的缩回必须作废。
    peek_leave(app);
    sleep(120);
    peek_enter(app);
    sleep(520);
    let (e2, c2) = dock_state(app);
    let l2 = read_ball(app).map(|b| b.0);
    r.check(
        "抖动吸收：300ms 内重新悬停保持展开（旧定时器作废，不是缩回去闪一下）",
        e2 == DockEdge::Right && !c2 && l2 == Some(expanded),
        &format!("dock={e2:?} collapsed={c2} x={l2:?}，期望 x={expanded}"),
    );

    peek_leave(app);
    sleep(600);
    let (e3, c3) = dock_state(app);
    let l3 = read_ball(app).map(|b| b.0);
    r.check(
        "真离开 300ms 后缩回半球",
        e3 == DockEdge::Right && c3 && l3 == Some(collapsed),
        &format!("dock={e3:?} collapsed={c3} x={l3:?}，期望 x={collapsed}"),
    );
}

/// 左缘吸附（换边判定）。
fn phase_left_dock(app: &tauri::AppHandle, env: &Env, r: &mut Report) {
    r.section("四、左缘吸附（换边）");
    // 先展开（缩回态起拖时 `on_move` 会先展开再取抓取偏移，锚点会带上半球平移，断言就要
    // 跟着复杂化；展开后起拖锚点恒为 0，等式最干净）。
    peek_enter(app);
    sleep(260);
    let Some((x0, y0, _, _)) = read_ball(app) else {
        r.check("读球窗几何", false, "读不到");
        return;
    };
    press(app, x0, y0);
    sleep(80);
    // 贴左缘内 8px（在 snap 容差内 → 判定为 Left）。
    let (tx, ty) = (env.mx + 8, env.my + env.mh / 2);
    drag_to(app, tx, ty);
    sleep(320);
    release_drag(app);
    sleep(400);

    let (edge, collapsed) = dock_state(app);
    let got = read_ball(app);
    let expect_x = collapsed_x(DockEdge::Left, env.mx, env.mw, env.lw, peek_px(env.scale));
    r.check(
        "松手贴左缘 → 吸附态换成 Left 且已缩回半球（换边判定生效）",
        edge == DockEdge::Left && collapsed,
        &format!("dock={edge:?} collapsed={collapsed}"),
    );
    r.check(
        "左吸附缩回态球窗 x == 显示器左边 − (球宽 − 半球宽)（只露右侧半球）",
        got.map(|b| b.0) == Some(expect_x),
        &format!("实际 {:?}，期望 {expect_x}", got.map(|b| b.0)),
    );
}

/// 上缘吸附（y 轴半隐藏；与右 / 左吸附在 y 轴上完全对称）+ y 轴的悬停展开。
fn phase_top_dock(app: &tauri::AppHandle, env: &Env, r: &mut Report) {
    r.section("五、上缘吸附（y 轴半隐藏）");
    // 先展开（理由同「四」：展开后起拖锚点恒为 0，等式最干净）。
    peek_enter(app);
    sleep(260);
    let Some((x0, y0, _, _)) = read_ball(app) else {
        r.check("读球窗几何", false, "读不到");
        return;
    };
    press(app, x0, y0);
    sleep(80);
    // 贴上缘内 10px（在 snap 容差内 → 判定为 Top）。
    let (tx, ty) = (env.mx + env.mw / 2, env.my + 10);
    drag_to(app, tx, ty);
    sleep(320);
    release_drag(app);
    sleep(400);

    let (edge, collapsed) = dock_state(app);
    let got = read_ball(app);
    let expect_y = collapsed_y(DockEdge::Top, env.my, env.mh, env.lh, peek_px(env.scale));
    r.check(
        "松手贴上缘 → 内部吸附态 = Top 且已缩回半球",
        edge == DockEdge::Top && collapsed,
        &format!("dock={edge:?} collapsed={collapsed}"),
    );
    r.check(
        "缩回态球窗 y == 显示器上边 − (球高 − 半球高)（只露下半球）",
        got.map(|b| b.1) == Some(expect_y),
        &format!("实际 {:?}，期望 {expect_y}", got.map(|b| b.1)),
    );
    r.check(
        "缩回态球窗 x == 松手时钳制后的 x（横向不越界）",
        got.map(|b| b.0) == Some(tx),
        &format!("实际 {:?}，期望 {tx}", got.map(|b| b.0)),
    );

    // y 轴的悬停展开（x 轴完整的展开 / 抖动吸收 / 延迟缩回见「三」）：展开态 y == expanded_y。
    peek_enter(app);
    sleep(260);
    let (e1, c1) = dock_state(app);
    let y1 = read_ball(app).map(|b| b.1);
    let expanded = expanded_y(DockEdge::Top, env.my, env.mh, env.lh);
    r.check(
        "悬停半球 → 沿 y 轴展开到屏内（球窗 y == 显示器上边）",
        e1 == DockEdge::Top && !c1 && y1 == Some(expanded),
        &format!("dock={e1:?} collapsed={c1} y={y1:?}，期望 y={expanded}"),
    );
}

/// 下缘吸附（y 轴换边；与「五」对称）。
fn phase_bottom_dock(app: &tauri::AppHandle, env: &Env, r: &mut Report) {
    r.section("六、下缘吸附（y 轴换边）");
    peek_enter(app);
    sleep(260);
    let Some((x0, y0, _, _)) = read_ball(app) else {
        r.check("读球窗几何", false, "读不到");
        return;
    };
    press(app, x0, y0);
    sleep(80);
    // 贴下缘内 10px（在 snap 容差内 → 判定为 Bottom）。
    let (tx, ty) = (env.mx + env.mw / 2, env.my + env.mh - env.lh + 10);
    drag_to(app, tx, ty);
    sleep(320);
    release_drag(app);
    sleep(400);

    let (edge, collapsed) = dock_state(app);
    let got = read_ball(app);
    let expect_y = collapsed_y(DockEdge::Bottom, env.my, env.mh, env.lh, peek_px(env.scale));
    r.check(
        "松手贴下缘 → 吸附态换成 Bottom 且已缩回半球（y 轴换边判定生效）",
        edge == DockEdge::Bottom && collapsed,
        &format!("dock={edge:?} collapsed={collapsed}"),
    );
    r.check(
        "缩回态球窗 y == 显示器下边 − 半球高（只露上半球）",
        got.map(|b| b.1) == Some(expect_y),
        &format!("实际 {:?}，期望 {expect_y}", got.map(|b| b.1)),
    );
    r.check(
        "缩回态球窗 x == 松手时钳制后的 x（横向不越界）",
        got.map(|b| b.0) == Some(tx),
        &format!("实际 {:?}，期望 {tx}", got.map(|b| b.0)),
    );
}

/// 单击 = toggle 主窗（对称语义）。点在**露在屏内的那半**上（缩回态半球有一半在屏外，
/// 点屏外坐标不真实）。
fn phase_click(app: &tauri::AppHandle, env: &Env, r: &mut Report) {
    r.section("七、单击 toggle 主窗");
    r.check(
        "前置：假主窗（label=main）已可见",
        main_visible(app) == Some(true),
        &format!("实际 {:?}", main_visible(app)),
    );

    let (edge, _) = dock_state(app);
    let Some((x0, y0, _, _)) = read_ball(app) else {
        r.check("读球窗几何", false, "读不到");
        return;
    };
    let (px, py) = peek_click_point(env, edge, x0, y0);

    click_at(app, px, py);
    sleep(260);
    r.check(
        "单击球 → 主窗隐藏（v1.71 toggle 语义：可见且非最小化则隐藏）",
        main_visible(app) == Some(false),
        &format!("实际 {:?}", main_visible(app)),
    );

    click_at(app, px, py);
    sleep(260);
    r.check(
        "再单击一次 → 主窗重新显示（toggle 是对称的）",
        main_visible(app) == Some(true),
        &format!("实际 {:?}", main_visible(app)),
    );
}

/// 右键 = **关闭悬浮图标功能 + 唤回主窗口**（v1.76），随后再把球与开关恢复。
///
/// ⚠️ 前置刻意把 `float_enabled` 置 true、并把假主窗藏起来：探针模式下 `SettingsState` 是
/// `default()`（`float_enabled` 本来就是 false），假主窗默认也是可见的 —— 不先制造出
/// 「开关为 on + 窗口被隐藏」的初态，后两条断言改不改代码都会绿（自我满足的假断言）。
fn phase_hide(app: &tauri::AppHandle, r: &mut Report) {
    r.section("八、右键：关闭悬浮图标功能并唤回主窗口");

    let a = app.clone();
    let primed = main_call(app, move |_| {
        let ok_set = crate::settings::set_float_enabled(&a, true).is_ok();
        let ok_main = match a.get_webview_window(MAIN_LABEL) {
            Some(w) => {
                let _ = w.hide();
                true
            }
            None => false,
        };
        ok_set && ok_main
    })
    .unwrap_or(false);
    sleep(220);
    r.check(
        "前置：float_enabled 置 true 且假主窗已隐藏（否则后两条断言没有区分度）",
        primed && float_enabled(app) && main_visible(app) == Some(false),
        &format!(
            "primed={primed} float_enabled={} main_visible={:?}",
            float_enabled(app),
            main_visible(app)
        ),
    );

    // 记下隐藏前的吸附态与球窗位置：「复原」（重建球窗）后必须**原样**回来（吸附态保留）。
    let (edge_before, _) = dock_state(app);
    let pos_before = read_ball(app).map(|b| (b.0, b.1));

    hide(app);
    sleep(420);

    r.check(
        "右键球 → 球窗已**销毁**（不是只隐藏：Some(false) 就是退回 v1.75 的死局）",
        float_visible(app).is_none(),
        &format!("实际 {:?}", float_visible(app)),
    );
    // ⚠️ detail 必须**在通过时也说人话**：`r.check` 无论成败都会把 detail 打进日志，
    //    早先这里直接传了失败提示（「float_enabled 仍为 true —— …」），于是 PASS 行读起来
    //    自相矛盾（`PASS … — float_enabled 仍为 true`）。改为「永远打实际值 + 失败才加解释」。
    let fe = float_enabled(app);
    r.check(
        "右键球 → float_enabled 已置 false（应用不再靠球驻留）",
        !fe,
        &format!(
            "实际 float_enabled={fe}{}",
            if fe {
                " —— 仍为 true：关窗后进程会继续驻留、桌面却没有入口"
            } else {
                ""
            }
        ),
    );
    r.check(
        "右键球 → 主窗口被唤回并显示",
        main_visible(app) == Some(true),
        &format!("实际 {:?}（None = 假主窗已不存在）", main_visible(app)),
    );

    // 复原：开关与球都恢复（探针的 settings.json 落在 `<exe>/data` 的隔离目录，不碰用户配置）。
    let a2 = app.clone();
    let _ = main_call(app, move |_| {
        let _ = crate::settings::set_float_enabled(&a2, true);
    });
    let a3 = app.clone();
    let _ = main_call(app, move |_| show_float_window(&a3));
    // ⚠️⚠️ v1.76 起球上右键会**销毁**球窗，所以这一步是**重建**，不是 v1.72 的「把藏起来的
    //     窗口再 show 一下」。重建走的是 `build_ball_on_main`（建完即绘即显），真机上仍要等
    //     排队建窗那一段 —— 固定 `sleep(700)` 可能赶不上，断言会被误判成「吸附态丢了」。
    //     改为轮询到「可见」为止，上限取 `RESTORE_WAIT`（比冷启动的 `BUILD_WAIT` 小：
    //     此刻窗口类已注册、进程环境已热，只差排队建窗）。
    let deadline = Instant::now() + RESTORE_WAIT;
    while Instant::now() < deadline && float_visible(app) != Some(true) {
        sleep(300);
    }
    let (edge, collapsed) = dock_state(app);
    let pos = read_ball(app).map(|b| (b.0, b.1));
    r.check(
        "复原：再次「显示悬浮球」可见，且吸附态与位置保留（不会被重置成自由态）",
        float_visible(app) == Some(true)
            && edge == edge_before
            && collapsed == (edge_before != DockEdge::None)
            && pos == pos_before,
        &format!(
            "visible={:?} dock={edge:?} collapsed={collapsed} pos={pos:?}，期望 dock={edge_before:?} pos={pos_before:?}",
            float_visible(app)
        ),
    );
}

/// 读运行期设置里的「启用桌面悬浮图标」开关（右键关功能断言用）。
fn float_enabled(app: &tauri::AppHandle) -> bool {
    app.state::<crate::settings::SettingsState>()
        .0
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .float_enabled
}

// ===========================================================================
// 观测原语（全部经主线程派发 —— 与排队中的建窗 / 销毁任务保序，与生产代码同约定）
// ===========================================================================

/// 主显示器几何 + 球窗尺寸的快照（物理像素；窗口 == 球，`lw/lh` 即球窗边长）。
#[derive(Clone, Copy)]
struct Env {
    scale: f64,
    lw: i32,
    lh: i32,
    mx: i32,
    my: i32,
    mw: i32,
    mh: i32,
}

/// 把闭包派发到主线程执行并取回结果。
fn main_call<T: Send + 'static>(
    app: &tauri::AppHandle,
    f: impl FnOnce(&tauri::AppHandle) -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    let a = app.clone();
    app.run_on_main_thread(move || {
        let _ = tx.send(f(&a));
    })
    .ok()?;
    rx.recv_timeout(MAIN_CALL_TIMEOUT).ok()
}

fn sleep(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

/// 读【球窗】矩形的物理几何 `(x, y, w, h)`。
///
/// v1.81 起**窗口 == 球**：`ball_rect()` 直接就是球的位置（旧版要按活柱偏移从宿主原点换算，
/// 那套 `live_px` / `live_offset` 已随活柱退役）。访问器跨线程安全，经 `main_call` 只为保序。
fn read_ball(app: &tauri::AppHandle) -> Option<(i32, i32, i32, i32)> {
    main_call(app, |_| ball_rect()).flatten()
}

/// 读 HWND 的扩展样式位（建窗形态断言用）。
fn exstyle_of(hwnd: isize) -> u32 {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, GWL_EXSTYLE};
    unsafe { GetWindowLongPtrW(HWND(hwnd as *mut core::ffi::c_void), GWL_EXSTYLE) as u32 }
}

/// 取物理屏幕点下的窗口 HWND（alpha 穿透取证用）。分层窗口 alpha=0 的像素不在命中范围内，
/// `WindowFromPoint` 会落到下层窗口上 —— 这正是「视觉 = 命中」的观察点。
fn window_from_point(x: i32, y: i32) -> Option<isize> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::WindowFromPoint;
    let h = unsafe { WindowFromPoint(POINT { x, y }) };
    if h.is_invalid() {
        None
    } else {
        Some(h.0 as isize)
    }
}

/// 缩回半球时「露在屏内的那半」的中心点：单击注入点（点屏外坐标不真实）。
fn peek_click_point(env: &Env, edge: DockEdge, x0: i32, y0: i32) -> (i32, i32) {
    let peek = peek_px(env.scale);
    match edge {
        DockEdge::Left => (x0 + env.lw - peek / 2, y0 + env.lh / 2),
        DockEdge::Right => (x0 + peek / 2, y0 + env.lh / 2),
        DockEdge::Top => (x0 + env.lw / 2, y0 + env.lh - peek / 2),
        DockEdge::Bottom => (x0 + env.lw / 2, y0 + peek / 2),
        DockEdge::None => (x0 + env.lw / 2, y0 + env.lh / 2),
    }
}

fn float_visible(app: &tauri::AppHandle) -> Option<bool> {
    // 球窗是原生分层窗口，**不在** `webview_windows()` 里（`get_webview_window("float")` 恒为
    // None）：存在性用 `ball_hwnd`、可见性用 `ball_visible`。返回 `None` = 球窗不存在
    // （已销毁），`Some(v)` = 存在且当前可见性为 v —— 与旧版 `Option<bool>` 口径一致。
    main_call(app, |_| ball_hwnd().map(|_| ball_visible())).flatten()
}

fn main_visible(app: &tauri::AppHandle) -> Option<bool> {
    main_call(app, |a| {
        a.get_webview_window(MAIN_LABEL)
            .and_then(|w| w.is_visible().ok())
    })
    .flatten()
}

// ===========================================================================
// 事件注入（等价于 WndProc 收到消息后的那一步；见模块文档「驱动方式」）
// ===========================================================================

/// 把一次交互事件直接喂给真实状态机（走真实窗口操作；`sx` / `sy` 为**物理**屏幕坐标）。
fn inject(app: &tauri::AppHandle, event: &str, sx: f64, sy: f64) {
    let ev = event.to_string();
    let _ = main_call(app, move |a| {
        crate::floatwin::inject_page_event(a, &ev, sx, sy)
    });
}

/// 「按下」：等价 WndProc 的 `WM_LBUTTONDOWN`（只暂存按下点）。物理坐标直传，不再按缩放换算。
fn press(app: &tauri::AppHandle, x: i32, y: i32) {
    inject(app, "down", x as f64, y as f64);
}

/// 「拖动中」：等价越过拖动阈值后的每一帧 `WM_MOUSEMOVE`（状态机逐帧跟随）。
fn drag_to(app: &tauri::AppHandle, x: i32, y: i32) {
    inject(app, "move", x as f64, y as f64);
}

/// 「松手且已越过拖动阈值」→ 吸附决策（等价 `end_press(moved=true)`）。
fn release_drag(app: &tauri::AppHandle) {
    inject(app, "dragend", 0.0, 0.0);
}

fn peek_enter(app: &tauri::AppHandle) {
    inject(app, "peekenter", 0.0, 0.0);
}

fn peek_leave(app: &tauri::AppHandle) {
    inject(app, "peekleave", 0.0, 0.0);
}

fn hide(app: &tauri::AppHandle) {
    inject(app, "hide", 0.0, 0.0);
}

/// 一次完整单击：按下 + `click`（未越过拖动阈值的松手在 WndProc 侧发它）。
fn click_at(app: &tauri::AppHandle, x: i32, y: i32) {
    press(app, x, y);
    sleep(60);
    inject(app, "click", 0.0, 0.0);
    sleep(60);
}

// ===========================================================================
// 准备 / 收尾
// ===========================================================================

/// 等 Tauri 主线程 / 事件循环就绪：反复派发一个空任务，直到真的**往返回来**。
///
/// 返回 false 表示窗口环境没起来（不是悬浮球缺陷）—— 此时后续断言毫无意义，调用方会**直接
/// `std::process::exit(97)`**（不经 `app.exit`：事件循环没起来时那条路同样送不到），把「这台机器
/// 没跑起来」与「我改坏了」区分开。
fn wait_main_ready(app: &tauri::AppHandle, r: &mut Report) -> bool {
    let t0 = Instant::now();
    let deadline = t0 + Duration::from_secs(MAIN_READY_WAIT_SECS);
    while Instant::now() < deadline {
        if main_call(app, |_| ()).is_some() {
            r.note(format!("主线程往返就绪（{} ms）", t0.elapsed().as_millis()));
            return true;
        }
        sleep(250);
    }
    r.check(
        "Tauri 主线程 / 事件循环就绪",
        false,
        &format!(
            "{MAIN_READY_WAIT_SECS}s 内主线程任务始终未被派发回来 —— 事件循环没起来，\
             属**环境性**故障（受限环境下 WebView2 初始化偶发停摆），与悬浮球实现无关；\
             本次未产生任何悬浮球断言"
        ),
    );
    false
}

/// 清掉数据目录里的 `settings.json`：让起始几何回到「主显示器右上角自由态」。
/// 不清的话上次探针落盘的吸附几何会被恢复，起始状态不稳定（断言仍成立，但读起来费解）。
fn reset_store(app: &tauri::AppHandle, r: &mut Report) {
    let Some(dir) = crate::config::store_dir(app) else {
        r.note("拿不到数据目录：跳过 settings.json 清理（起始几何可能沿用上次）");
        return;
    };
    let path = dir.join(crate::store::FILE);
    if path.exists() {
        match std::fs::remove_file(&path) {
            Ok(()) => r.note(format!("已清理 {}", path.display())),
            Err(e) => r.note(format!("清理 {} 失败（忽略）：{e}", path.display())),
        }
    }
}

/// 建一个 `label == "main"` 的真实窗口：单击悬浮球 toggle 的**观察点**。
///
/// 它只需要「可被 hide / show 且能读 is_visible」——刻意加载内置 splash 页（零网络、
/// 无脚本副作用），不跑 `bootstrap`（不解析域名、不起代理）。
///
/// ⚠️ 在**主线程 / `setup` 内直接建**，不经 `run_on_main_thread` 排队 —— 原因见 `run()` 的注释。
fn build_fake_main_direct(app: &tauri::AppHandle) -> bool {
    let b = WebviewWindowBuilder::new(
        app,
        MAIN_LABEL,
        WebviewUrl::CustomProtocol(crate::loading::splash_url()),
    )
    .title("web2app float probe (fake main)")
    .inner_size(360.0, 220.0)
    .position(80.0, 80.0)
    .visible(true);

    // ⚠️⚠️ 必须与生产主窗 / tab 宿主窗用**同一个** WebView2 用户数据目录。
    //    不给 `data_directory` 的话 wry 会落到**默认**目录，真机实测**建窗直接卡死**：
    //    HWND 已经建出来（进程窗口列表里能看到本探针的标题）、`build()` 却永不返回，
    //    只能等看门狗给 98。生产路径从没踩到这条，正是因为 `lib.rs` / `tabs.rs` 两处建窗
    //    **都**设了它 —— 探针漏掉一处，就复现了「WebView2 初始化偶发停摆」这个查了很久的
    //    假象。契约守卫见 `contract_tests.rs`。
    let b = match crate::config::data_dir(app) {
        Some(dir) => b.data_directory(dir),
        None => b,
    };

    b.build().is_ok()
}

/// 等球窗「存在且可见」；超时则如实记录（**不重试、不 destroy** —— 理由见 `BUILD_WAIT`）。
/// 返回环境快照；超时返回 None。
fn wait_env(app: &tauri::AppHandle, r: &mut Report) -> Option<Env> {
    let deadline = Instant::now() + BUILD_WAIT;
    let mut last: Option<bool> = None;
    while Instant::now() < deadline {
        match float_visible(app) {
            Some(true) => return capture_env(app),
            other => last = other,
        }
        // 轮询别太密：每次 `float_visible` 都是往主线程队列里排一个任务，而建窗任务也在那条
        // 队列上 —— 排得太密会拖慢（甚至饿到）建窗。
        sleep(500);
    }
    r.note(format!(
        "球窗 {}s 内未「存在且可见」—— 现状：{}",
        BUILD_WAIT.as_secs(),
        match last {
            None => "球窗不存在（CreateWindowExW 失败 / 建窗任务还没跑起来）",
            Some(false) => "球窗存在但不可见",
            Some(true) => "可见",
        }
    ));
    // 收尾诊断：球窗是原生窗口（**不在** `webview_windows()` 里），存在性看 `ball_hwnd`；
    // WebView 窗口的 labels 顺带列出，便于分诊（假主窗建没建出来一眼可见）。
    let labels = main_call(app, |a| {
        let mut v: Vec<String> = a.webview_windows().keys().cloned().collect();
        v.sort();
        v
    })
    .unwrap_or_default();
    r.note(format!(
        "建球失败时：ball_hwnd = {:?}；现有 WebView 窗口 labels = {labels:?}",
        ball_hwnd()
    ));
    None
}

/// 环境快照：DPI 用 `ball_scale`（GetDpiForWindow），球窗边长用 `ball_px`（窗口 == 球），
/// 显示器用 `primary_monitor_bounds`（探针的拖动目标都相对主显示器）。
fn capture_env(app: &tauri::AppHandle) -> Option<Env> {
    main_call(app, |_| {
        let scale = ball_scale();
        let (lw, lh) = ball_px(scale);
        let (mx, my, mw, mh) = crate::floatwin::primary_monitor_bounds()?;
        Some(Env {
            scale,
            lw,
            lh,
            mx,
            my,
            mw,
            mh,
        })
    })
    .flatten()
}

/// 把真实光标挪到主显示器左上角内 8px 并返回原位。
///
/// 为什么必须动它：缩回逻辑（`floatwin` 的 `finish_peek_leave`）到期时会读**真实光标位置**，
/// 光标若落在「缩回后矩形」内就有意放弃缩回（防抖设计）—— 那会让「真离开 → 缩回」
/// 这条断言偶发失败。探针期间球永远不去左上角，于是断言可重复。
fn park_cursor(r: &mut Report) -> Option<(i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, SetCursorPos};
    unsafe {
        let mut p = POINT::default();
        if GetCursorPos(&mut p).is_err() {
            r.note("读不到真实光标位置：缩回相关断言可能受真实光标干扰");
            return None;
        }
        if SetCursorPos(8, 8).is_err() {
            r.note("真实光标挪位失败：缩回相关断言可能受真实光标干扰");
            return None;
        }
        r.note(format!("真实光标暂存 ({},{}) 并挪到 (8,8)", p.x, p.y));
        Some((p.x, p.y))
    }
}

fn restore_cursor(saved: Option<(i32, i32)>) {
    use windows::Win32::UI::WindowsAndMessaging::SetCursorPos;
    if let Some((x, y)) = saved {
        unsafe {
            let _ = SetCursorPos(x, y);
        }
    }
}

// ===========================================================================
// 结果汇总
// ===========================================================================

struct Report {
    lines: Vec<String>,
    checks: usize,
    failures: usize,
    diags: usize,
    diag_bad: usize,
}

/// 结果文件的落点。**每行即时追加**：探针若被看门狗强杀，也能从文件里看到断在哪一条。
fn result_path() -> std::path::PathBuf {
    std::env::temp_dir().join("web2app-float-probe.txt")
}

fn append_result(line: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(result_path())
    {
        let _ = writeln!(f, "{line}");
    }
}

impl Report {
    fn new() -> Self {
        let _ = std::fs::write(
            result_path(),
            "web2app 悬浮球真机合成指针 harness（cargo run -- --float-probe）\n".as_bytes(),
        );
        Self {
            lines: Vec::new(),
            checks: 0,
            failures: 0,
            diags: 0,
            diag_bad: 0,
        }
    }

    fn note(&mut self, s: impl Into<String>) {
        self.emit(format!("·  {}", s.into()));
    }

    fn section(&mut self, s: &str) {
        self.emit(format!("\n== {s} =="));
    }

    /// 断言：计入失败数（决定退出码）。
    fn check(&mut self, name: &str, ok: bool, detail: &str) {
        self.checks += 1;
        if !ok {
            self.failures += 1;
        }
        self.emit(format!(
            "{}  {name}{}",
            if ok { "PASS" } else { "FAIL" },
            tail(detail)
        ));
    }

    /// 环境诊断：**不计入失败**（同一台机器上环境随时会变，判「产品坏了」不公平）。
    fn diag(&mut self, name: &str, ok: bool, detail: &str) {
        self.diags += 1;
        if !ok {
            self.diag_bad += 1;
        }
        self.emit(format!(
            "{}  {name}{}",
            if ok { "INFO" } else { "WARN" },
            tail(detail)
        ));
    }

    /// 逐行输出：stdout（Rust 的 stdout 是 LineWriter，重定向到文件也**每行即刷**）
    /// + 结果文件追加。宁可啰嗦，也不要「跑完才吐结果」——真机探针随时可能挂死 / 被强杀。
    fn emit(&mut self, line: String) {
        println!("{line}");
        append_result(&line);
        self.lines.push(line);
    }

    fn finish(self) -> i32 {
        let foot = format!(
            "\n{} 项断言，{} 项失败 —— {}；另 {} 项环境诊断（{} 项异常，不计入失败）\n",
            self.checks,
            self.failures,
            if self.failures == 0 {
                "ALL PASS"
            } else {
                "FAILED"
            },
            self.diags,
            self.diag_bad
        );
        print!("{foot}");
        append_result(foot.trim_end_matches('\n'));
        println!("（结果同时写入 {}）", result_path().display());
        self.failures.min(120) as i32
    }
}

fn tail(detail: &str) -> String {
    if detail.is_empty() {
        String::new()
    } else {
        format!("  — {detail}")
    }
}
