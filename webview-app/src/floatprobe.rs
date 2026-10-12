//! 悬浮球**真机合成指针 harness**（仅 Windows；只有 `--float-probe` 才会跑到）。
//!
//! ## 为什么需要它
//! 在此之前，悬浮球的交互只有三类证据：① 纯几何函数单测（`floatwin::tests`）；② 静态契约
//! 守卫（`contract_tests::float_window_contract`，钉的是**源码字面量**）；③ 真机**人工**观察。
//! 它们都证不了「一次拖动 / 一次悬停 → 窗口**真的**按预期移动 / 吸附 / 缩回」。Electron 壳有
//! `tests/float-v170-harness`（真窗 + 合成 PointerEvent），Tauri 壳一直**零**这类证据 ——
//! 而活柱 / `WM_NCHITTEST` 圆形命中子类化 / `WS_EX_NOACTIVATE` / WebView2 最小宽度都是
//! **Tauri 独有**的实现细节。本 harness 就是补这一格。
//!
//! ## 它怎么做到「真机」
//! 在**真实进程内**跑：真实 `WebviewWindowBuilder` 建真实球窗（真 WebView2 + 真
//! `floaticon.html`），真实事件分发 + 真实窗口操作，再读回**真实窗口 API** 的结果：
//!   · `outer_position` / `outer_size` → 换算成【活柱】几何（宿主被 WebView2 撑宽，见
//!     `floatwin` 模块文档，不能直接拿宿主原点当球的位置）；
//!   · `GetWindowRgnBox` → 断言 v1.80 起窗口**没有区域**（视觉走真 alpha，不得再裁剪）；
//!   · 向父窗与 WebView2 子窗 `SendMessage(WM_NCHITTEST)` → 圆心必须 HTCLIENT、圆外必须
//!     HTTRANSPARENT（命中圆与活柱重合；父 / 子两窗都验，漏拦子窗则矩形吃鼠标）；
//!   · `dock_state` → 把内部吸附态与上面的几何**对照**断言（几何对、状态错同样是缺陷）。
//!
//! ## 驱动方式：直接注入事件（**不是**走 JS）
//! 交互事件经 [`floatwin::inject_page_event`] 注入：用**当前真实令牌**构造合法载荷，走真实
//! 令牌校验 + 真实 `handle_event` + 真实窗口操作 —— 唯一跳过的是「球页 JS 把事件发出去」那一段。
//!
//! 为什么退这一步：真机实测，在远程桌面 / 受控终端这类环境里 WebView2 会把自定义协议页
//! 落成 `about:blank`（本机的对照组：连内置 splash 页的窗口也是 `about:blank`），球页脚本
//! 根本不执行 ⇒「页面 → Rust」这一段恒为空，十几条交互断言会集体失败，却完全盖住真正的原因。
//! 探针仍会把页面链路的**真实状态**打出来（§零，环境诊断，不计入失败）；页面侧的发事件 +
//! 双击去抖由 Electron 壳的同款 harness 用**同一份** `floaticon.html` 覆盖（两壳互补），
//! 另有 Rust 静态契约守卫钉住页面里的字面量。
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
//!
//! ## 运行
//! ```text
//! cd webview-app && cargo run -- --float-probe
//! ```
//! ⚠️ 建议**换一个全新目录**跑（把 exe 拷过去再执行）：WebView2 的 UDF 就在「exe 同级 /
//! `data_dir`」下，被强杀过的旧 UDF 可能带残留锁，让下一次启动卡死（本机实测）。全新目录
//! 天然隔离，也不必删除任何东西。
//! ⚠️ release 是 `windows_subsystem = "windows"`，没有控制台可打印（结果文件仍在，但退出码
//! 要靠调用方读）；要看输出就用 debug 构建。

use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{Listener, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::WM_NCHITTEST;

use crate::floatwin::{
    collapsed_x, dock_state, expanded_x, live_offset, live_px, min_visible_px, peek_px,
    register_float_listeners, show_float_window, snap_px, FLOAT_LABEL,
};
use crate::store::DockEdge;

/// 触发探针的命令行开关。`main.rs` 不做参数解析，这里自己扫 argv。
pub(crate) const PROBE_FLAG: &str = "--float-probe";

/// 「假主窗」的 label：必须与 `bootstrap::main_window_label()`（单窗口模式恒为 `"main"`）
/// 一致，否则「单击悬浮球 → toggle 主窗」这条断言没有观察点。
const MAIN_LABEL: &str = "main";

/// 建球的等待上限；到点还没「存在且可见」就如实记录并收工。
///
/// ⚠️ 给到 120s 是**真机实测**调出来的：本机（受限桌面会话）从 `show_float_window` 到
/// 「窗口对象真的出现在 `webview_windows()` 里」要几十秒（透明窗 + 自定义协议 + WebView2
/// 冷启动）。给短了会正好卡在窗口**刚注册的前一刻**判负。
///
/// ⚠️⚠️ v1.76 起放宽到 **180s**：同一天同一台机器，下午实测球窗从建到可见稳定 **>120s**
/// （连续多轮都在 120s 判负，而失败收尾 labels 里**始终有 `float"`** —— 即窗口最终建出来了、
/// 只是比 120s 慢）。这是**环境等待**上限、不是对产品行为的断言，所以放宽只是让慢机器也能取证，
/// 不掩盖任何产品缺陷（真挂死由看门狗兜底）。
///
/// ⚠️⚠️ 而且**绝不能"失败就 `destroy` 重来"**：`show_float_window` 是「spawn 线程 +
/// `run_on_main_thread` 排队建窗」的**异步**路径，建窗任务何时真正跑起来不由探针决定；
/// 在它跑完之前 `destroy_float_window`，就是在**反复打断建窗** ⇒ 球永远出不来。
/// `run-G / I / K / L` 四次全栽在这：每轮日志都写「窗口不存在」，而失败收尾打印的 labels 里
/// **始终有 `"float"`** —— 窗口其实一直在建，只是被一次次掐掉。
const BUILD_WAIT: Duration = Duration::from_secs(180);
/// 「复原」（v1.76 起 = **重建**球窗）的等待上限。比 `BUILD_WAIT` 小：此时 WebView2 环境已经热了
/// （`wait_env` 早就等过一次），只差「建窗 + 球页加载」这一段。若与 `BUILD_WAIT` 同取 180s，
/// 整轮最坏值会顶穿看门狗（见下）。
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

/// 探针专用事件名（只用于页面链路诊断，与生产代码无关）。
const PROBE_PING_EVENT: &str = "web2app:probe-ping";
/// 收到探针 ping 的次数与最后一次的载荷（载荷里带球页看到的一次性令牌）。
static PING_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static PING_PAYLOAD: Mutex<String> = Mutex::new(String::new());

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

    register_float_listeners(&app);
    // 页面链路的观测点：球页 emit → Rust 收到，才是「JS 真的执行了 + IPC 真的通了」的回执
    // （Rust 侧拿不到 `eval` 的返回值，只能用这种回路观测）。
    app.listen(PROBE_PING_EVENT, |e| {
        PING_COUNT.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut g) = PING_PAYLOAD.lock() {
            *g = e.payload().to_string();
        }
    });

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
    //    自己在 `setup` 里埋的雷。生产侧不会踩到（托盘 / 设置页触发时事件循环已跑了很久）。
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

/// 看门狗：真机探针**绝不允许挂死**。任何一步阻塞（建窗 / 窗口 API / IPC / 事件循环）到点
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
        "缩放 {:.2}｜活柱 {}×{} 物理｜宿主 {}×{} 物理｜主显示器 ({},{} {}×{})",
        env.scale, env.lw, env.lh, env.host_w, env.host_h, env.mx, env.my, env.mw, env.mh
    ));
    sleep(500);

    phase_page_link(app, &mut r);
    phase_frame(app, &env, &mut r);
    phase_free_drag(app, &env, &mut r);
    phase_right_dock(app, &env, &mut r);
    phase_peek(app, &env, &mut r);
    phase_left_dock(app, &env, &mut r);
    phase_click(app, &env, &mut r);
    phase_hide(app, &env, &mut r);

    r.section("说明");
    r.note(
        "本 harness 用**直接注入事件**驱动（见 §零）：唯一跳过的是「球页 JS 把事件发出去」。\
         因此页面侧的「双击去抖」（floaticon.html 的 CLICK_DEBOUNCE_MS）不在这里覆盖；\
         它由 Electron harness（tests/float-v170-harness，驱动同一份共享球页）与 Rust \
         静态契约守卫 float_page_click_is_debounced_against_double_click 覆盖。",
    );
    restore_cursor(saved_cursor);
    r.finish()
}

/// 页面链路诊断（**不计入失败**）：球窗 URL、`eval` 下发、页面 → Rust 的 emit 回路。
///
/// 这一段的意义是「把环境事实钉在报告里」：页面链路不通时，上面的交互断言就不是产品结论的
/// 全部依据；页面链路通时，说明这台机器可以进一步跑「走 JS」的更强验证。
fn phase_page_link(app: &tauri::AppHandle, r: &mut Report) {
    r.section("零、页面链路诊断（不计入失败；本机已知 WebView2 会把自定义协议页落成 about:blank）");

    let float_url = window_url_of(app, FLOAT_LABEL);
    let main_url = window_url_of(app, MAIN_LABEL);
    r.diag(
        "球窗当前 URL",
        float_url.as_deref() != Some("about:blank"),
        &format!("{float_url:?}"),
    );
    r.diag(
        "假主窗 URL（对照组：同为自定义协议页）",
        main_url.as_deref() != Some("about:blank"),
        &format!("{main_url:?}"),
    );

    // `document.title` 读不回来：Tauri 的 `WebviewWindow::title()` 读的是**操作系统窗口标题**
    // （建窗时写死，v1.77 起球窗刻意置空 `.title("")`），页面改 `document.title` 不会同步过去。
    // 用 location.hash
    // 把页面内的状态搬到 URL 上读回来。
    eval(
        app,
        "location.hash = '#p' + (window.__web2appFloatToken ? window.__web2appFloatToken.length : 0) \
         + (window.__TAURI_INTERNALS__ ? 'i' : 'n');",
    );
    sleep(300);
    r.diag(
        "球页脚本可执行（#p<令牌长度><i=有 IPC / n=无 IPC>）",
        window_url_of(app, FLOAT_LABEL)
            .as_deref()
            .is_some_and(|u| u.contains("#p")),
        &format!("{:?}", window_url_of(app, FLOAT_LABEL)),
    );

    let down = eval_result(app, "1;");
    r.diag(
        "eval 下发成功（WebviewWindow::eval 返回 Ok）",
        down.is_ok(),
        down.as_ref().err().map(String::as_str).unwrap_or(""),
    );

    let js = format!(
        "(function(){{try{{window.__TAURI_INTERNALS__.invoke('plugin:event|emit',\
         {{event:'{PROBE_PING_EVENT}',payload:{{token:window.__web2appFloatToken||''}}}})\
         .then(null,function(e){{window.__w2fEmitErr=String(e);}});}}\
         catch(e){{window.__w2fEmitErr=String(e);}}}})();"
    );
    let before = PING_COUNT.load(Ordering::SeqCst);
    let down2 = eval_result(app, &js);
    sleep(700);
    let after = PING_COUNT.load(Ordering::SeqCst);
    let payload = PING_PAYLOAD.lock().map(|g| g.clone()).unwrap_or_default();
    r.diag(
        "球页 → Rust 的核心事件回路（emit 被 ACL 放行且 app.listen 收得到）",
        after > before,
        &format!(
            "收到 {before} → {after} 次；载荷 {payload:?}；eval={}",
            if down2.is_ok() { "Ok" } else { "Err" }
        ),
    );
    let token_len = serde_json::from_str::<serde_json::Value>(&payload)
        .ok()
        .and_then(|v| v.get("token").and_then(|t| t.as_str()).map(str::len))
        .unwrap_or(0);
    r.diag(
        "一次性令牌已注入球页主框架（32 位 hex）",
        token_len == 32,
        &format!("带回的 token 长度 = {token_len}"),
    );
}

/// 建窗形态 / 置顶 / 无边框 / 活柱圆形裁剪区域。
fn phase_frame(app: &tauri::AppHandle, env: &Env, r: &mut Report) {
    r.section("一、建窗形态与活柱裁剪");
    let got = main_call(app, |a| {
        let w = a.get_webview_window(FLOAT_LABEL)?;
        Some((
            w.is_always_on_top().unwrap_or(false),
            w.is_decorated().unwrap_or(true),
            w.outer_size().map(|s| s.height).unwrap_or(0),
            w.outer_size().map(|s| s.width).unwrap_or(0),
            has_window_region(&w),
        ))
    })
    .flatten();
    let Some((on_top, decorated, host_h, host_w, has_region)) = got else {
        r.check("能读到球窗属性", false, "窗口消失了");
        return;
    };
    r.check("置顶（always_on_top）", on_top, "");
    r.check("无系统边框（decorations=false）", !decorated, "");
    r.check(
        "宿主窗高 == 活柱高（WebView2 只强制最小**宽**，不动高）",
        host_h as i32 == env.lh,
        &format!("宿主高 {host_h} vs 活柱高 {}", env.lh),
    );
    r.check(
        "宿主窗被 WebView2 撑宽（≥ 活柱宽；这是「活柱」存在的前提）",
        host_w as i32 >= env.lw,
        &format!("宿主宽 {host_w} vs 活柱宽 {}", env.lw),
    );
    r.check(
        "v1.80 不再设 SetWindowRgn（视觉走真 alpha、命中走 NCHITTEST，区域必须为空）",
        !has_region,
        &format!("GetWindowRgnBox 仍读到区域：{has_region}"),
    );
    // 当前活柱偏移由真实吸附态决定（首次跑探针通常是自由态 / 默认摆放）。
    let (edge, _) = dock_state(app);
    let off = live_offset(host_w as i32, env.lw, edge);
    check_circle_hit_testing(r, app, env, off, "建窗形态");
}

/// 自由态拖动：① 1:1 跟手；② 拖动中软钳制；③ 松手完整钳回。
fn phase_free_drag(app: &tauri::AppHandle, env: &Env, r: &mut Report) {
    r.section("二、自由态拖动（1:1 跟手 / 软钳制 / 松手钳回）");
    let Some((x0, y0, _, _)) = live_rect(app, DockEdge::None) else {
        r.check("读初始活柱几何", false, "读不到");
        return;
    };

    // 抓取偏移 = 0（按下点 == 活柱左上角）：于是「光标位置」就等于「活柱目标位置」，
    // 断言可以写成纯等式，不必再推 anchor。真实用户抓的是球心附近，偏移 ∈ [0, 活柱宽)。
    press(app, env.scale, x0, y0);
    sleep(80);
    let (tx, ty) = (x0 - 200, y0 + 60);
    drag_to(app, env.scale, tx, ty);
    sleep(320);
    let live = live_rect(app, DockEdge::None);
    r.check(
        "拖动 1:1 跟手（活柱位移 == 光标位移；旧的增量协议只有半速且抖）",
        live.map(|l| (l.0, l.1)) == Some((tx, ty)),
        &format!("实际 {:?}，期望 ({tx},{ty})", live.map(|l| (l.0, l.1))),
    );

    // 软钳制：光标压到显示器右下角内 8px（必须在显示器**内** —— Tauri 的 monitor_from_point
    // 用 MONITOR_DEFAULTTONULL，屏外的点查不到显示器就整段跳过钳制，那样断言会落空）。
    let (cx, cy) = (env.mx + env.mw - 8, env.my + env.mh - 8);
    let minv = min_visible_px(env.scale);
    drag_to(app, env.scale, cx, cy);
    sleep(320);
    let live = live_rect(app, DockEdge::None);
    r.check(
        "拖动中软钳制：右下各留最小可见条（球不会被整个甩出屏）",
        live.map(|l| (l.0, l.1)) == Some((env.mx + env.mw - minv, env.my + env.mh - minv)),
        &format!(
            "实际 {:?}，期望 ({},{})",
            live.map(|l| (l.0, l.1)),
            env.mx + env.mw - minv,
            env.my + env.mh - minv
        ),
    );

    release_drag(app);
    sleep(320);
    let live = live_rect(app, DockEdge::None);
    r.check(
        "自由态松手：完整钳回显示器内（右下角贴边）",
        live.map(|l| (l.0, l.1)) == Some((env.mx + env.mw - env.lw, env.my + env.mh - env.lh)),
        &format!(
            "实际 {:?}，期望 ({},{})",
            live.map(|l| (l.0, l.1)),
            env.mx + env.mw - env.lw,
            env.my + env.mh - env.lh
        ),
    );
}

/// 右缘吸附：松手即吸附 + 立即缩回半球；并核对「活柱贴宿主右边」（live_offset）。
fn phase_right_dock(app: &tauri::AppHandle, env: &Env, r: &mut Report) {
    r.section("三、右缘吸附（含活柱右贴 + 圆形区域跟随）");
    let Some((x0, y0, _, _)) = live_rect(app, DockEdge::None) else {
        r.check("读活柱几何", false, "读不到");
        return;
    };
    press(app, env.scale, x0, y0);
    sleep(80);
    // 目标：贴右缘内 10px。10 必须落在[吸附容差]内才算 Right —— 用生产代码同一个 snap_px
    // 把它算出来记进结论，免得容差改了这里悄悄失效（跨 DPI 时它随缩放变大）。
    let snap = snap_px(env.scale);
    r.note(format!("吸附容差 snap = {snap} 物理像素；贴边测试点取 +10"));
    let (tx, ty) = (env.mx + env.mw - env.lw + 10, env.my + env.mh / 2);
    drag_to(app, env.scale, tx, ty);
    sleep(320);
    release_drag(app);
    sleep(400);

    let (edge, collapsed) = dock_state(app);
    let off = live_offset(env.host_w, env.lw, DockEdge::Right);
    let live = live_rect(app, DockEdge::Right);
    r.check(
        "松手贴右缘 → 内部吸附态 = Right 且已缩回半球",
        edge == DockEdge::Right && collapsed,
        &format!("dock={edge:?} collapsed={collapsed}"),
    );
    r.check(
        "缩回态活柱 x == 显示器右边 − 半球宽（只露半球）",
        live.map(|l| l.0) == Some(env.mx + env.mw - peek_px(env.scale)),
        &format!(
            "实际 {:?}，期望 {}",
            live.map(|l| l.0),
            env.mx + env.mw - peek_px(env.scale)
        ),
    );
    r.check(
        "缩回态活柱 y == 松手时钳制后的 y（纵向不越界）",
        live.map(|l| l.1) == Some(env.my + env.mh / 2),
        &format!("实际 {:?}，期望 {}", live.map(|l| l.1), env.my + env.mh / 2),
    );
    // Tauri 独有：宿主被 WebView2 撑宽，右吸附时命中圆必须贴宿主**右边**（圆心 x 平移
    // live_offset）—— 这是「半球露在屏内仍点得中、撑宽部分依旧穿透」的机械前提。
    check_circle_hit_testing(r, app, env, off, "右吸附缩回态");
}

/// 悬停展开 / 抖动吸收 / 真离开缩回（都在右吸附态下做）。
fn phase_peek(app: &tauri::AppHandle, env: &Env, r: &mut Report) {
    r.section("四、悬停展开 / 抖动吸收 / 延迟缩回");
    // 展开位来自与生产代码**同一个**契约函数（不在这里重写一遍几何公式）。
    let expanded = expanded_x(DockEdge::Right, env.mx, env.mw, env.lw);
    let collapsed = env.mx + env.mw - peek_px(env.scale);

    peek_enter(app);
    sleep(260);
    let (e1, c1) = dock_state(app);
    let l1 = live_rect(app, DockEdge::Right).map(|l| l.0);
    r.check(
        "悬停半球 → 展开到屏内（活柱 x == 显示器右边 − 活柱宽）",
        e1 == DockEdge::Right && !c1 && l1 == Some(expanded),
        &format!("dock={e1:?} collapsed={c1} x={l1:?}，期望 x={expanded}"),
    );

    // 抖动吸收：离开后 300ms 未到就重新进入 —— 排队中的缩回必须作废。
    peek_leave(app);
    sleep(120);
    peek_enter(app);
    sleep(520);
    let (e2, c2) = dock_state(app);
    let l2 = live_rect(app, DockEdge::Right).map(|l| l.0);
    r.check(
        "抖动吸收：300ms 内重新悬停保持展开（旧定时器作废，不是缩回去闪一下）",
        e2 == DockEdge::Right && !c2 && l2 == Some(expanded),
        &format!("dock={e2:?} collapsed={c2} x={l2:?}，期望 x={expanded}"),
    );

    peek_leave(app);
    sleep(600);
    let (e3, c3) = dock_state(app);
    let l3 = live_rect(app, DockEdge::Right).map(|l| l.0);
    r.check(
        "真离开 300ms 后缩回半球",
        e3 == DockEdge::Right && c3 && l3 == Some(collapsed),
        &format!("dock={e3:?} collapsed={c3} x={l3:?}，期望 x={collapsed}"),
    );
}

/// 左缘吸附（换边判定 + 左吸附的活柱左贴）。
fn phase_left_dock(app: &tauri::AppHandle, env: &Env, r: &mut Report) {
    r.section("五、左缘吸附（换边）");
    // 先展开（缩回态起拖时 `on_move` 会先展开再取抓取偏移，锚点会带上 28px 平移，断言就要
    // 跟着复杂化；展开后起拖锚点恒为 0，等式最干净）。
    peek_enter(app);
    sleep(260);
    let Some((x0, y0, _, _)) = live_rect(app, DockEdge::Right) else {
        r.check("读活柱几何", false, "读不到");
        return;
    };
    press(app, env.scale, x0, y0);
    sleep(80);
    // 贴左缘内 8px（在 snap 容差内 → 判定为 Left）。
    let (tx, ty) = (env.mx + 8, env.my + env.mh / 2);
    drag_to(app, env.scale, tx, ty);
    sleep(320);
    release_drag(app);
    sleep(400);

    let (edge, collapsed) = dock_state(app);
    let live = live_rect(app, DockEdge::Left);
    let expect_x = collapsed_x(DockEdge::Left, env.mx, env.mw, env.lw, peek_px(env.scale));
    r.check(
        "松手贴左缘 → 吸附态换成 Left 且已缩回半球（换边判定生效）",
        edge == DockEdge::Left && collapsed,
        &format!("dock={edge:?} collapsed={collapsed}"),
    );
    r.check(
        "左吸附缩回态活柱 x == 显示器左边 − (活柱宽 − 半球宽)（只露右侧半球）",
        live.map(|l| l.0) == Some(expect_x),
        &format!("实际 {:?}，期望 {expect_x}", live.map(|l| l.0)),
    );
    // 左吸附的活柱贴宿主左边 ⇒ 偏移 0（与右吸附形成对照，钉住 live_offset 只对 Right 生效）。
    check_circle_hit_testing(r, app, env, 0, "左吸附缩回态");
}

/// 单击 = toggle 主窗（对称语义）。
fn phase_click(app: &tauri::AppHandle, env: &Env, r: &mut Report) {
    r.section("六、单击 toggle 主窗");
    r.check(
        "前置：假主窗（label=main）已可见",
        main_visible(app) == Some(true),
        &format!("实际 {:?}", main_visible(app)),
    );

    let Some((x0, y0, _, _)) = live_rect(app, DockEdge::Left) else {
        r.check("读活柱几何", false, "读不到");
        return;
    };
    // 点在**露在屏内的那半**上（缩回态活柱有一半在屏外，点屏外坐标不真实），y 取竖直中心。
    let peek = peek_px(env.scale);
    let (px, py) = (x0 + env.lw - peek / 2, y0 + env.lh / 2);

    click_at(app, env.scale, px, py);
    sleep(260);
    r.check(
        "单击球 → 主窗隐藏（v1.71 toggle 语义：可见且非最小化则隐藏）",
        main_visible(app) == Some(false),
        &format!("实际 {:?}", main_visible(app)),
    );

    click_at(app, env.scale, px, py);
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
fn phase_hide(app: &tauri::AppHandle, env: &Env, r: &mut Report) {
    r.section("七、右键：关闭悬浮图标功能并唤回主窗口");

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
    //     窗口再 show 一下」。重建走的是 `build_float_window_on_main`（隐藏建窗 → 球页加载完
    //     才 `show`，另有 1s 兜底），真机上要等球页加载 —— 固定 `sleep(700)` 必然赶不上，
    //     断言会被误判成「吸附态丢了」（2026-10-11 实测 `visible=Some(false) dock=None`：
    //     窗口已建出来、只是还没 show）。改为轮询到「可见」为止，上限取 `RESTORE_WAIT`
    //     （比冷启动的 `BUILD_WAIT` 小：此刻 WebView2 环境已热，只差建窗 + 球页加载）。
    let deadline = Instant::now() + RESTORE_WAIT;
    while Instant::now() < deadline && float_visible(app) != Some(true) {
        sleep(300);
    }
    let (edge, collapsed) = dock_state(app);
    let live = live_rect(app, DockEdge::Left).map(|l| l.0);
    let expect_x = collapsed_x(DockEdge::Left, env.mx, env.mw, env.lw, peek_px(env.scale));
    r.check(
        "复原：再次「显示悬浮球」可见，且吸附态保留（仍是左吸附半球，不会被重置成自由态）",
        float_visible(app) == Some(true)
            && edge == DockEdge::Left
            && collapsed
            && live == Some(expect_x),
        &format!(
            "visible={:?} dock={edge:?} collapsed={collapsed} x={live:?}",
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
// 观测原语（全部经主线程派发 —— 窗口 API 只在主线程调用，与生产代码同约定）
// ===========================================================================

/// 主显示器几何 + 活柱 / 宿主尺寸的快照。
#[derive(Clone, Copy)]
struct Env {
    scale: f64,
    lw: i32,
    lh: i32,
    host_w: i32,
    host_h: i32,
    mx: i32,
    my: i32,
    mw: i32,
    mh: i32,
}

/// 把闭包派发到主线程执行并取回结果（窗口 / WebView API 只在主线程可用）。
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

/// 读【活柱】矩形的物理几何 `(x, y, w, h)`。
///
/// 宿主窗原点 **不是** 球的位置：右吸附时活柱贴宿主右边，活柱 x = 宿主 x + live_offset。
/// `edge` 由调用方给出（用同一份契约函数 [`live_offset`]，不重写一遍公式）。
fn live_rect(app: &tauri::AppHandle, edge: DockEdge) -> Option<(i32, i32, i32, i32)> {
    main_call(app, move |a| {
        let w = a.get_webview_window(FLOAT_LABEL)?;
        let scale = w.scale_factor().ok()?;
        let pos = w.outer_position().ok()?;
        let host = w.outer_size().ok()?;
        let (lw, lh) = live_px(scale);
        let off = live_offset(host.width as i32, lw, edge);
        Some((pos.x + off, pos.y, lw, lh))
    })
    .flatten()
}

/// 球窗是否**仍挂着窗口区域**（v1.80 起必须为 false：视觉走真 alpha，裁剪交给
/// WM_NCHITTEST；GetWindowRgnBox 返回 0 = 无区域 / 调用失败，这里只在真实 HWND 上调用，
/// 0 即视为无区域）。
fn has_window_region(w: &WebviewWindow) -> bool {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::GetWindowRgnBox;
    let Ok(hwnd) = w.hwnd() else {
        return false;
    };
    let mut rc = RECT::default();
    unsafe { GetWindowRgnBox(hwnd, &mut rc).0 != 0 }
}

/// 取球窗父 HWND 与其**第一个后代子窗**（WebView2 控制器 HWND）。命中断言两个窗口都要发：
/// 只拦父窗拦不住覆盖整个客户区的子窗（见 floatwin.rs 模块文档「命中」）。
fn hit_hwnds(app: &tauri::AppHandle) -> Option<(isize, isize)> {
    use windows::core::BOOL;
    use windows::Win32::UI::WindowsAndMessaging::EnumChildWindows;
    main_call(app, |a| {
        let w = a.get_webview_window(FLOAT_LABEL)?;
        let parent = w.hwnd().ok()?;
        let mut found: Option<HWND> = None;
        unsafe extern "system" fn first(hwnd: HWND, lparam: LPARAM) -> BOOL {
            *(lparam.0 as *mut Option<HWND>) = Some(hwnd);
            BOOL(0) // FALSE = 枚举到第一个就停
        }
        unsafe {
            let _ = EnumChildWindows(
                Some(parent),
                Some(first),
                LPARAM(&mut found as *mut Option<HWND> as isize),
            );
        }
        Some((parent.0 as isize, found?.0 as isize))
    })
    .flatten()
}

/// 向 `target` 发 WM_NCHITTEST（坐标给父窗客户区物理坐标，内部转屏幕坐标），
/// 返回命中码（HTCLIENT=1 / HTTRANSPARENT=-1 …）。
fn nchittest(parent: isize, target: isize, client_x: i32, client_y: i32) -> i32 {
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
    let parent = HWND(parent as *mut core::ffi::c_void);
    let target = HWND(target as *mut core::ffi::c_void);
    let mut pt = POINT {
        x: client_x,
        y: client_y,
    };
    unsafe {
        let _ = ClientToScreen(parent, &mut pt);
        let lparam = ((pt.x as u16) as isize) | (((pt.y as u16) as isize) << 16);
        SendMessageW(
            target,
            WM_NCHITTEST,
            Some(WPARAM(0)),
            Some(LPARAM(lparam)),
        )
        .0 as i32
    }
}

/// 圆形命中的一组断言（父窗 + 子窗各发一遍）。`off` = 活柱相对宿主左边的物理偏移：
/// 圆心 `(off + lw/2, lh/2)`；旁侧点朝**撑宽方向**外移 3px（保证仍在宿主客户区内）。
fn check_circle_hit_testing(
    r: &mut Report,
    app: &tauri::AppHandle,
    env: &Env,
    off: i32,
    label: &str,
) {
    let Some((parent, child)) = hit_hwnds(app) else {
        r.check(
            &format!("{label}：取父窗 / WebView2 子窗 HWND"),
            false,
            "EnumChildWindows 没枚举到子窗（子类化无从验证）",
        );
        return;
    };
    let cx = off + env.lw / 2;
    let cy = env.lh / 2;
    let side_x = if off > 0 { off - 3 } else { off + env.lw + 3 };
    let cases: [(&str, i32, i32, i32); 3] = [
        ("圆心（球内）", cx, cy, 1 /*HTCLIENT*/),
        ("客户区角点 (2,2)", 2, 2, -1 /*HTTRANSPARENT*/),
        ("活柱外的撑宽区", side_x, cy, -1),
    ];
    for (name, x, y, want) in cases {
        let on_parent = nchittest(parent, parent, x, y);
        let on_child = nchittest(parent, child, x, y);
        r.check(
            &format!("{label} NCHITTEST·父窗「{name}」"),
            on_parent == want,
            &format!("实际 {on_parent}，期望 {want}"),
        );
        r.check(
            &format!("{label} NCHITTEST·WebView2 子窗「{name}」（子窗漏拦则圆外矩形吃点击）"),
            on_child == want,
            &format!("实际 {on_child}，期望 {want}"),
        );
    }
}

/// 任意窗口的当前 URL（诊断用）。
fn window_url_of(app: &tauri::AppHandle, label: &'static str) -> Option<String> {
    main_call(app, move |a| {
        a.get_webview_window(label)
            .and_then(|w| w.url().ok())
            .map(|u| u.to_string())
    })
    .flatten()
}

fn float_visible(app: &tauri::AppHandle) -> Option<bool> {
    main_call(app, |a| {
        a.get_webview_window(FLOAT_LABEL)
            .and_then(|w| w.is_visible().ok())
    })
    .flatten()
}

fn main_visible(app: &tauri::AppHandle) -> Option<bool> {
    main_call(app, |a| {
        a.get_webview_window(MAIN_LABEL)
            .and_then(|w| w.is_visible().ok())
    })
    .flatten()
}

// ===========================================================================
// 事件注入（等价于球页 `send()` 的那一步；见模块文档「驱动方式」）
// ===========================================================================

/// 把一次球页事件交给真实分发（负载里放**当前真实令牌**，走真实校验）。
fn inject(app: &tauri::AppHandle, event: &str, sx: f64, sy: f64) {
    let ev = event.to_string();
    let _ = main_call(app, move |a| {
        crate::floatwin::inject_page_event(a, &ev, sx, sy)
    });
}

/// 「按下」：等价球页 `pointerdown` 里立即发的那一下（只暂存按下点）。
/// 传物理坐标，内部换算成球页那样的 DIP（`on_down` 会再乘回 scale）。
fn press(app: &tauri::AppHandle, scale: f64, x: i32, y: i32) {
    inject(app, "down", x as f64 / scale, y as f64 / scale);
}

/// 「拖动中」：等价球页 `requestAnimationFrame` 里发的 `move`。
fn drag_to(app: &tauri::AppHandle, scale: f64, x: i32, y: i32) {
    inject(app, "move", x as f64 / scale, y as f64 / scale);
}

/// 「松手且已越过拖动阈值」→ 球页发 `dragend`。
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

/// 一次完整单击：按下 + `click`（球页在「未越过拖动阈值」的 pointerup 上发它）。
fn click_at(app: &tauri::AppHandle, scale: f64, x: i32, y: i32) {
    press(app, scale, x, y);
    sleep(60);
    inject(app, "click", 0.0, 0.0);
    sleep(60);
}

/// 把脚本下发到球页（主线程执行）。仅用于页面链路诊断。
fn eval_result(app: &tauri::AppHandle, js: &str) -> Result<(), String> {
    let js = js.to_string();
    main_call(app, move |a| match a.get_webview_window(FLOAT_LABEL) {
        Some(w) => w.eval(&js).map_err(|e| e.to_string()),
        None => Err("球窗不存在".to_string()),
    })
    .unwrap_or_else(|| Err("主线程派发超时".to_string()))
}

fn eval(app: &tauri::AppHandle, js: &str) {
    let _ = eval_result(app, js);
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

    // ⚠️⚠️ 必须与生产主窗 / tab 宿主窗 / 球窗用**同一个** WebView2 用户数据目录。
    //    不给 `data_directory` 的话 wry 会落到**默认**目录，真机实测**建窗直接卡死**：
    //    HWND 已经建出来（进程窗口列表里能看到本探针的标题）、`build()` 却永不返回，
    //    只能等看门狗给 98。生产路径从没踩到这条，正是因为 `lib.rs` / `tabs.rs` /
    //    `floatwin.rs` 三处建窗**都**设了它 —— 探针漏掉一处，就复现了「WebView2 初始化
    //    偶发停摆」这个查了很久的假象。契约守卫见 `contract_tests.rs`。
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
            None => "窗口不存在（build() 失败 / 还没建出来）",
            Some(false) => "窗口存在但不可见（页面没加载完 ⇒ on_page_load 与 1s 兜底都没生效）",
            Some(true) => "可见",
        }
    ));
    // 收尾诊断：把当前所有窗口 label 打出来。「球窗 label 在不在列表里」是**关键分诊信息**：
    // 不在 = `build()` 没成功；在却读不到 = 时序 / 注册路径问题。没有这一行就只能看到
    // 「不可见」，分不清是没建出来还是建了没显示 —— 本轮「反复打断建窗」正是靠它才定位到。
    let labels = main_call(app, |a| {
        let mut v: Vec<String> = a.webview_windows().keys().cloned().collect();
        v.sort();
        v
    })
    .unwrap_or_default();
    r.note(format!("建球失败时现有窗口 labels = {labels:?}"));
    None
}

fn capture_env(app: &tauri::AppHandle) -> Option<Env> {
    main_call(app, |a| {
        let w = a.get_webview_window(FLOAT_LABEL)?;
        let scale = w.scale_factor().ok()?;
        let (lw, lh) = live_px(scale);
        let host = w.outer_size().ok()?;
        let m = w.primary_monitor().ok().flatten()?;
        let p = m.position();
        let s = m.size();
        Some(Env {
            scale,
            lw,
            lh,
            host_w: host.width as i32,
            host_h: host.height as i32,
            mx: p.x,
            my: p.y,
            mw: s.width as i32,
            mh: s.height as i32,
        })
    })
    .flatten()
}

/// 把真实光标挪到主显示器左上角内 8px 并返回原位。
///
/// 为什么必须动它：缩回逻辑（`floatwin::finish_peek_leave`）到期时会读**真实光标位置**，
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
