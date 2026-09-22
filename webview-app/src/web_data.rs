//! 清除浏览数据（缓存 / Cookie / 本地存储 …）—— 设置页「清除全部」按钮的后端。
//!
//! **只有一个动作：全清。** 没有分项选择（不清「只清缓存 / 只清 Cookie」），这是需求本身
//! 的要求，也避免用户面对一堆选项还要猜「我到底该清哪一个」。
//!
//! 平台差异：
//! - **Windows（本 crate 的桌面目标）**：走 WebView2 原生
//!   `ICoreWebView2Profile2::ClearBrowsingDataAll` —— 一次清掉该 profile 的全部浏览数据
//!   （缓存、Cookie、Local Storage / IndexedDB / WebSQL / CacheStorage、Service Worker、
//!   站点权限与设置等），完成后把内容页重新加载。
//! - **Android 等其它平台**：本命令**仍然注册**（`invoke_handler` 是跨平台的，不按平台分叉
//!   注册表），但会返回明确的「不支持」错误。宁可报错，也不要做成「点了显示已清除、其实
//!   什么都没清」——那正是本项目最忌讳的失败形态。补充：Android 上设置窗本身也不可达
//!   （打开它的入口是桌面专属的 Shift+F1 全局快捷键，见 `settings::open_settings_window`）。
//!
//! ## ⚠️ 调用链的线程约束（踩过的坑，务必保持）
//!
//! 1. **命令必须是 async**。默认的同步命令走 `ExecutionContext::Blocking`，是**在主线程上
//!    内联执行**的；而下面要 `recv_timeout` 等 WebView2 的完成回调，而该回调正是靠**主线程
//!    的消息泵**派发的（WebView2 把回调投递到创建 environment 的那个线程）。两者一撞就是
//!    自锁：主线程堵在 recv 上 → 回调永远派发不出来 → 只能等到超时。
//! 2. **`with_webview` 必须在后台线程里调**。从非主线程调用它是**异步投递**（`proxy.send_event`），
//!    主线程执行完闭包立刻返回、消息泵继续转；在主线程里内联执行并阻塞则会把消息泵停掉
//!    （详见 `webview_hooks::attach_webview_hooks` 的长注释）。
//! 3. 因此实际形态是：命令（async）→ `spawn_blocking` 到运行时线程 → 起一个**后台线程**反复
//!    投递 `with_webview` 直到 controller 就绪 → 闭包里发起异步清除 → 完成回调把结果塞进
//!    channel → `spawn_blocking` 那条线程阻塞等待结果。全程没有任何一处堵住主线程。
//!
//! ## ⚠️ 完成回调不可靠：不能拿它当「成功/失败」的唯一判据
//!
//! 实测（`examples/probe_clear_data.rs` 真机探针，8 次以上，两种 handler 形态都试过）：
//! **完成回调从未被观测到执行过**，而数据**确实被清掉了**（探针用「浏览器侧 cookie 库」
//! 与「夹具服务端实际收到的 `Cookie:` 头」双证据确认）。上游 wry 的 `clear_all_browsing_data()`
//! 更是纯 fire-and-forget —— handler 1.1µs 就被释放、回调从不执行。
//! 所以本命令的口径是：**回调报错才是失败**；回调不来只如实说明、照常重载页面。
//!
//! 同一个探针还观察到：**清除刚结束的一段时间内，该 WebView 的导航会被静默丢弃**
//! （`load_url` 照样返回 `Ok`，页面纹丝不动；有时直接 `ERROR_INVALID_STATE`），
//! 窗口可见 / 隐藏、用谁的 clear API 都一样，窗口期实测 ≥32s。
//! 因此重载前刻意留了一点落定时间（见 `clear_and_reload` 里的 `RELOAD_SETTLE`）。
//! ⚠️ 本机**没能**取证「重载一定生效」—— 这条仍需真机目视确认。

/// 清除全部浏览数据，并把内容页重新加载一遍。
///
/// 返回值是给设置页直接展示的一句话（说清「清了什么、重载了几个页面」）。
#[tauri::command]
pub(crate) async fn clear_web_data(app: tauri::AppHandle) -> Result<String, String> {
    clear_impl(app).await
}

/// Windows 实现：清全部浏览数据 + 重载内容页。
#[cfg(windows)]
async fn clear_impl(app: tauri::AppHandle) -> Result<String, String> {
    // spawn_blocking：本任务会**同步等待** WebView2 的完成回调（最长 `CLEAR_TIMEOUT`），
    // 不能占着运行时的工作线程直接阻塞。关键的是它**离开了主线程**（见模块注释 ⚠️ 第 1 条）。
    match tauri::async_runtime::spawn_blocking(move || clear_and_reload(&app)).await {
        Ok(res) => res,
        Err(e) => Err(format!("清除缓存任务执行失败：{e}")),
    }
}

/// 非 Windows（Android 等）：明确报错，不做假成功。
#[cfg(not(windows))]
async fn clear_impl(_app: tauri::AppHandle) -> Result<String, String> {
    Err("清除缓存仅支持 Windows 桌面版".to_string())
}

/// 等 WebView2 controller 就绪的最长时间（与 `webview_hooks` 取同一口径）。
#[cfg(windows)]
const READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// 等清除完成的最长时间：清除本身是异步的，给得比「等就绪」宽裕得多。
#[cfg(windows)]
const CLEAR_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

#[cfg(windows)]
use tauri::Manager;
#[cfg(windows)]
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2;

/// 清全部浏览数据，成功后重新加载内容页。
#[cfg(windows)]
fn clear_and_reload(app: &tauri::AppHandle) -> Result<String, String> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};
    use std::time::{Duration, Instant};

    // 所有窗口共用同一个 WebView2 user data folder（由 `config::data_dir` 决定），
    // 也就是**同一个 profile** ⇒ 用哪个窗口去清，清的都是同一份数据。
    let label = content_labels(app)
        .into_iter()
        .next()
        .ok_or_else(|| "找不到内容窗口，无法清除缓存".to_string())?;
    let window = app
        .get_webview_window(&label)
        .ok_or_else(|| format!("窗口 {label} 不存在，无法清除缓存"))?;

    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    // 独立留一个发送端给「超时 / 投递失败」这两条不在闭包内的路径。
    let tx_fallback = tx.clone();
    // ⚠️ 与 `webview_hooks::attach_webview_hooks` 同一条规矩：**全函数共用**一个完成标志，
    // 且在**轮次开头**检查。非主线程的 `with_webview` 是异步投递，返回时闭包还没执行，
    // 「返回后立刻检查」必然读到 false ⇒ 必然空转到超时（哪怕是第一次就成功）。
    let done = Arc::new(AtomicBool::new(false));

    std::thread::spawn(move || {
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            if done.load(Ordering::SeqCst) {
                return;
            }
            if Instant::now() >= deadline {
                let _ = tx_fallback.send(Err(
                    "WebView2 在 5s 内未就绪，清除缓存未执行（请重试一次）".to_string(),
                ));
                return;
            }

            let tx_in = tx.clone();
            let done_in = done.clone();
            let window_in = window.clone();
            let sent = window_in.with_webview(move |wv| {
                // controller 未就绪：不置 done、不发结果，交给外层下一轮重试。
                let core = match unsafe { wv.controller().CoreWebView2() } {
                    Ok(c) => c,
                    Err(_) => return,
                };
                done_in.store(true, Ordering::SeqCst);
                if let Err(e) = start_clear(&core, tx_in.clone()) {
                    // 发起阶段就失败（如 profile 接口版本过低）：立刻把错误送出去。
                    let _ = tx_in.send(Err(e));
                }
                // 成功时结果由 ClearBrowsingDataCompletedHandler 在完成的那一刻发出。
            });

            if sent.is_err() {
                let _ = tx_fallback.send(Err("窗口已关闭，清除缓存未执行".to_string()));
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });

    // 等完成回调：**收不到回调不算失败**。
    //
    // 实测（`examples/probe_clear_data.rs`，8 次以上）：这条链路里 WebView2 的完成回调
    // **从未被观测到执行过** —— 上游 wry 的 `clear_all_browsing_data()` 干脆是纯
    // fire-and-forget（handler 1.1µs 即释放、回调从不执行），而**数据照样被清掉**。
    // 所以分三种情况处理：
    //   - 回调带回 `Ok`（本机没观测到，但别的环境可能正常）⇒ 一切照旧；
    //   - 回调带回 `Err` ⇒ 真失败，如实返回错误；
    //   - 超时 / handler 被提前释放 ⇒ **仍然继续重载页面**，并如实说明「未回报完成状态」。
    // 早期版本把「超时」当失败直接 `?` 返回，结果是：用户看到一句吓人的「清除超时」、
    // 页面还毫无变化（因为 `?` 提前 return 掉了重载）——等于按钮白点。
    let reported = match rx.recv_timeout(CLEAR_TIMEOUT) {
        Ok(Ok(())) => None,
        Ok(Err(e)) => return Err(e),
        Err(_) => Some("（WebView2 未回报完成状态；若页面仍显示旧内容，请重新打开应用）"),
    };

    // 让清除多落定一会儿再重载：实测清除刚结束时导航可能被 WebView2 **静默丢弃**
    // （`load_url` 照样返回 Ok，页面却纹丝不动）。
    const RELOAD_SETTLE: std::time::Duration = std::time::Duration::from_millis(1200);
    std::thread::sleep(RELOAD_SETTLE);

    // 清完必须重新加载：否则页面上还留着旧的文档（内存态仍是登录后的样子），
    // 用户看到「页面没变化」会以为按钮没生效。
    let reloaded = reload_content_pages(app);
    Ok(match reported {
        None => format!("已清除缓存、Cookie 等全部浏览数据；已请求重新加载 {reloaded} 个页面"),
        Some(note) => format!(
            "已发起清除全部浏览数据（缓存、Cookie、登录状态等）{note}；\
             已请求重新加载 {reloaded} 个页面"
        ),
    })
}

/// 在已就绪的 `CoreWebView2` 上发起「清除全部浏览数据」。
///
/// 完成回调由 WebView2 投递到**主线程**执行，所以这里只把结果塞进 channel，
/// 由非主线程的那一端阻塞等待（见模块注释 ⚠️ 第 1 条）。
#[cfg(windows)]
fn start_clear(
    core: &ICoreWebView2,
    tx: std::sync::mpsc::Sender<Result<(), String>>,
) -> Result<(), String> {
    use webview2_com::ClearBrowsingDataCompletedHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2Profile2, ICoreWebView2_13};
    use windows::core::Interface;

    // ⚠️ `Profile` 不在 ICoreWebView2 上，而在 ICoreWebView2_13（运行时 1.0.1108+）上 ——
    //    直接对 ICoreWebView2 调 `.Profile()` 是编译期就找不到方法（E0599），不是运行时问题。
    let core13: ICoreWebView2_13 = core.cast().map_err(|_| {
        "当前 WebView2 运行时版本过低（需要 1.0.1108+），请先更新 WebView2 运行时".to_string()
    })?;
    let profile =
        unsafe { core13.Profile() }.map_err(|e| format!("读取 WebView2 profile 失败：{e}"))?;
    // ClearBrowsingDataAll 在 ICoreWebView2Profile2（运行时 1.0.1245+）里；老运行时上
    // cast 会失败 —— 明确报版本原因，而不是笼统的「清除失败」。
    let profile2: ICoreWebView2Profile2 = profile.cast().map_err(|_| {
        "当前 WebView2 运行时版本过低（清除浏览数据需要 1.0.1245+），请先更新 WebView2 运行时"
            .to_string()
    })?;

    // HRESULT 参数已被 webview2-com 转成 `Result<()>`（`ClosureArg for HRESULT`）。
    let handler = ClearBrowsingDataCompletedHandler::create(Box::new(
        move |result: windows::core::Result<()>| {
            let _ = tx.send(result.map_err(|e| format!("清除浏览数据失败：{e}")));
            Ok(())
        },
    ));

    unsafe { profile2.ClearBrowsingDataAll(&handler) }.map_err(|e| format!("发起清除失败：{e}"))
}

/// 内容窗口的 label 列表 —— 真正承载网页、持有 WebView2 profile 的那些窗口。
///
/// - 单窗口模式：只有 `main`。
/// - 多页签模式（仅 Windows）：内容是各 `tab-*` 窗口；宿主窗口只画标签栏，不在其列。
/// - 设置窗口（`settings`）**刻意排除**：它自己是通过 `settings://` 提供的内嵌页，
///   清浏览数据与它无关，重载它还会把用户正在看的这页提示刷掉。
#[cfg(windows)]
fn content_labels(app: &tauri::AppHandle) -> Vec<String> {
    if crate::tabs::tabs_enabled() {
        let mut labels: Vec<String> = app
            .webview_windows()
            .keys()
            .filter(|l| l.starts_with(crate::tabs::TAB_PREFIX))
            .cloned()
            .collect();
        // `webview_windows()` 的遍历顺序不保证，排一下让「取第一个」是确定行为。
        labels.sort();
        labels
    } else {
        vec!["main".to_string()]
    }
}

/// 重新加载全部内容窗口；返回成功重载的数量。
#[cfg(windows)]
fn reload_content_pages(app: &tauri::AppHandle) -> usize {
    let mut reloaded = 0usize;
    for label in content_labels(app) {
        let Some(window) = app.get_webview_window(&label) else {
            continue;
        };
        if window.reload().is_ok() {
            reloaded += 1;
        }
    }
    reloaded
}
