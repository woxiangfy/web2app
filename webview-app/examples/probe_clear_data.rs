//! 真机取证探针：确证「清除全部浏览数据」这条链路在**真实 WebView2** 上确实生效。
//!
//! 运行：`cargo run --example probe_clear_data`
//! 退出码：**0** = 结论成立；**1** = 结论不成立；**2** = **本机无法取证**（环境限制，见坑 2）。
//! 「退出码即结论」是本探针存在的全部意义 —— 所以 2 必须与 1 分开：把「观测不到」当成
//! 「功能坏了」，等于用错的取证方法得出错的结论（本项目最爱踩的坑）。
//!
//! ## 对照组开关（刻意保留的分诊设施，不是临时调试残留）
//!
//! 没有它们，一旦流程卡住就分不清「清除把 profile 弄卡了」还是「我的探针写错了」。
//! - `PROBE_CHECK_ONLY=1`：不写数据、不清除，直接开 `/check`。只回答一个问题：
//!   新导航 + IPC 回传这条**脚手架**本身通不通。
//! - `PROBE_SKIP_CLEAR=1`：写数据但**不清除**，直接去 `/check`。证明「数据还在时 /check
//!   确实看得见 cookie」—— 否则主流程那句「看不到 cookie」什么也证明不了（判据就不成对了）。
//! - `PROBE_USE_WRY_CLEAR=1`：改用上游 wry 自带的 `clear_all_browsing_data()`。用来区分
//!   「问题出在我这套 handler / 等待形态」还是「WebView2 这条 API 在本机就是这个表现」。
//! - `PROBE_VISIBLE=1`：让窗口与 WebView 都可见（生产里窗口是可见的）。
//!
//! ## 为什么需要它
//!
//! 设置页「清除全部」在 Tauri 侧走的是 WebView2 原生
//! `ICoreWebView2_13::Profile() → ICoreWebView2Profile2::ClearBrowsingDataAll()` + 完成回调
//! （见 `src/web_data.rs`）。这类「编译通过、文档也这么说」的原生 API 必须拿真实 WebView2
//! 量一次 —— 否则「按钮点了、提示已清除、其实什么都没清」这种失败模式只能等用户反馈。
//!
//! ## 判据（**成对**，杜绝「页面本来就是空的」也算通过）
//!
//! ① 清除**前**：`cookies_for_url(url)` 返回 ≥1（浏览器侧 cookie 库里确实有东西）；
//! ② 清除**后**：**新导航**到 `/check` 的文档看到 `document.cookie` 为空；并且夹具服务端会
//!    打印它**从网络层实际收到**的 `Cookie:` 头（`REQ /check cookie=[]`）—— 这比页面里读 DOM 更硬。
//!
//! `cookies_for_url` 只在 ①（清除**前**）用一次。它的「清除后读数归 0」已在对照 C 里验到
//! （`cookies_for_url → 0 个`），但因为它在清除未落定时会**挂死**（见下），故不进主线判据。
//!
//! ⚠️ 判据为什么落在「新导航 + 服务端 Cookie 头」而不是「`cookies_for_url` 本尊」：
//! `cookies_for_url` 内部是 `wait_with_pump(rx)`（**阻塞 `GetMessageA`**），一旦清除还没落定、
//! 它的完成回调不来，它就会**永久挂死**——实测真的挂到全局兜底（见坑 2）。把它当主线判据等于
//! 把探针做成定时炸弹。所以它降级为「导航已成功 ⇒ profile 已落定」之后的旁证。
//!
//! ⚠️⚠️ **`localStorage.length` 不能当判据**（这条是**对照组 B 抓出来的**）：实测同一 origin 的
//! 新文档在**未清除**时读到的也是 `0`（`/` 页自报 `ls=1`，同 origin 的 `/check` 页却是 `0`）。
//! 所以它只做附带观测、不参与 PASS/FAIL —— 否则就是把「实现当前的表现」当成期望值锁死。
//!
//! ## ⚠️⚠️ 五条踩过的坑（结论留在这里，别再踩）
//!
//! 1. **夹具 HTTP 服务必须「每连接一线程」**。第一版是单线程 `accept` + 阻塞 `read`：浏览器
//!    一开预连接就把 accept 循环堵死 ⇒ 第二个请求（`/check`）永远不被服务。表现极具误导性：
//!    导航返回 `Ok` 却什么都没有，或者 `Navigate` 抛
//!    `HRESULT_FROM_WIN32(ERROR_INVALID_STATE) = 0x8007139F`。改成每连接一线程 + 读满请求头
//!    之后导航立刻恢复正常（对照 B 里服务端亲眼打出了 `REQ /check cookie=[probekey=probeval]`）。
//!    ⇒ 教训：**外部 API 报「状态不对」时，先怀疑自己这边的夹具/环境，别先怀疑 API。**
//! 2. ⚠️⚠️ **`ClearBrowsingDataAll` 是长时间异步操作，发起后必须等它落定**。实测：发起后一段
//!    时间内，profile 级 API 会返回 `ERROR_INVALID_STATE`（`CookieManager()` / `ExecuteScript` /
//!    `Navigate` 全中），**导航会被静默丢弃**（`load_url` 照样返回 `Ok`），而 `GetCookies` 甚至
//!    会一直不完成 ⇒ `cookies_for_url` 挂死。这个窗口期长短不定（实测 ≥830ms，有时 >15s）。
//!    本探针因此：等回调**并且**至少再等 [`SETTLE_AFTER_CLEAR`]，然后**导航失败就重试导航**
//!    （最多 [`NAV_ATTEMPTS`] 次）—— 而不是急着查询或断言。
//!    ⚠️⚠️ 但本机的窗口期**长到 ≥32s**（4 次导航 × 8s 全部落空），所以本探针在这台机器上
//!    **走不到 ②**，只能返回退出码 2。已用变量隔离确认过这不是我们自己的问题：
//!    `PROBE_VISIBLE=1/0`（窗口可见 / 隐藏）结果一样；改用上游 wry 自己的
//!    `clear_all_browsing_data()`（`PROBE_USE_WRY_CLEAR=1`）结果也一样 ——
//!    即**与「谁写的 clear 代码」和「窗口可见性」都无关**，属环境 / WebView2 层现象。
//!    ⇒ 同一条对**生产**同样重要：清完立刻重载页面，可能正落在「清除仍在进行」的窗口里被丢掉。
//! 3. **完成回调不可靠，「回调到了」不能当作清除成功的判据**。上游
//!    `wry::WebView::clear_all_browsing_data()` 是纯 fire-and-forget：handler 在 **1.1µs** 后就被
//!    释放、回调从不执行（实测 3/3 一致）；本探针自己的 handler 有时在 **830ms** 后被释放、
//!    有时被持有超过 15s，**同样从未被调用过**。但两种情况下**数据都真的被清掉了**
//!    （对照 C：`cookies_for_url → 0 个`、服务端 `REQ /check cookie=[]`、新文档 `document.cookie=[]`）。
//!    ⇒ 判据只能落在「数据有没有被清掉」上；回调到达性只作观测。
//! 4. ⚠️ 已试过并**否决**：给探针一个显式独占的 user data folder
//!    （`wry::WebContext::new(Some(dir))` + `WebViewBuilder::new_with_web_context`）。
//!    动机是 `ERROR_INVALID_STATE` 的官方语义指向「同一个 UDF 被另一个 WebView2 环境占用 /
//!    环境配置不一致」（WebView2Feedback #2703 官方回复：遇到它就**重试**；另见
//!    `ICoreWebView2EnvironmentOptions2::ExclusiveUserDataFolderAccess` 文档）。实测结果
//!    **更退化**：页面请求发出了，但连夹具页的 SEED 回传都收不到。故仍用 wry 缺省配置。
//! 5. ⚠️⚠️ **`cargo clippy --example` 不会重建 example 可执行文件**。跑探针前必须
//!    `cargo build --example`，否则 `./target/debug/examples/xxx.exe` 还是**上一次**的二进制
//!    —— 表现为「改了代码、输出却一模一样」，正是本项目教训里那条「先查你跑的是哪一版包」。
//!
//! ## 与生产代码的对应关系
//!
//! `src/web_data.rs` 的形态是「命令必须 `async`」+「后台线程 `recv_timeout` 等回调」——
//! 原因是第一版曾在 IPC 回调里**同步**等完成回调（`wait_for_async_operation` 就地抽消息泵，
//! 而完成回调正要靠这个泵派发）⇒ 自锁，20s 一次回调都没来。本探针与生产同形：
//! 只发起、立即返回，另起线程等结果，主线程继续抽消息。

#[cfg(windows)]
use std::time::Duration;

/// 全局阶段标记：清除是否已经发起。供**兜底线程**区分两种收尾 ——
/// 「还没走到清除就卡住」= 探针自身/环境坏了（FAIL），
/// 「清除已发起、之后观测不成」= 本机的环境限制（INCONCLUSIVE）。
/// 为什么需要它：清除之后 `load_url` 可能**直接阻塞**（不是返回错误），那时主线程卡在里面、
/// 什么也做不了，只有兜底线程能收尾 —— 它必须知道走到哪一步了。
#[cfg(windows)]
static CLEAR_INITIATED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 夹具页：**在 `load` 之后**才回传，保证第一份文档已经安顿好，第二次导航不会撞上
/// 「上一跳还没落定」的竞态。
#[cfg(windows)]
const PAGE_SEED: &str = r#"<!doctype html><meta charset="utf-8"><title>seed</title>
<script>
function report() {
  var ls = -1;
  try { localStorage.setItem('probekey', 'probeval'); ls = localStorage.length; } catch (e) {}
  document.cookie = 'probekey=probeval; path=/';
  window.chrome.webview.postMessage('SEED cookie=[' + document.cookie + '] ls=' + ls);
}
window.addEventListener('load', report);
</script>"#;

/// 只读页：**不写** Cookie、**不写** localStorage。
#[cfg(windows)]
const PAGE_CHECK: &str = r#"<!doctype html><meta charset="utf-8"><title>check</title>
<script>
function report() {
  var ls = -1;
  try { ls = localStorage.length; } catch (e) {}
  window.chrome.webview.postMessage('CHECK cookie=[' + document.cookie + '] ls=' + ls);
}
window.addEventListener('load', report);
</script>"#;

/// 等完成回调的最长时间（与生产 `CLEAR_TIMEOUT` 同口径）。回调到不到都不影响结论。
#[cfg(windows)]
const WAIT_CALLBACK: Duration = Duration::from_secs(15);
/// 发起清除后**至少**再等这么久才去导航：让清除真正落定（见文件头 ⚠️ 第 2 条）。
#[cfg(windows)]
const SETTLE_AFTER_CLEAR: Duration = Duration::from_secs(12);
/// 发起导航后等 `/check` 回传的时间；超时即重试导航。
#[cfg(windows)]
const WAIT_NAV: Duration = Duration::from_secs(8);
/// 导航重试次数上限。**故意只留 2 次**：清除窗口里 `load_url` 有可能直接阻塞
/// （实测卡满 87s），重试越多越容易把整轮跑成「无输出的超时」。
#[cfg(windows)]
const NAV_ATTEMPTS: u32 = 2;
/// 全局兜底：任何一步没走完都不能挂死（含「卡在 load_url 里」这种）。
#[cfg(windows)]
const TOTAL_TIMEOUT: Duration = Duration::from_secs(75);

/// 后台线程 → 主线程的事件信封。
#[cfg(windows)]
enum Msg {
    /// 夹具页已写入数据，该做「清除前」检查了。
    AfterSeed,
    /// 清除已发起并已等够落定时间；附上完成回调的观测结果。
    Cleared(Option<Result<(), String>>),
    /// 第 `n` 次导航之后 `/check` 迟迟没有回传。
    NavWatchdog(u32),
}

#[cfg(windows)]
fn fail(msg: impl std::fmt::Display) -> ! {
    println!("RESULT FAIL {msg}");
    std::process::exit(1);
}

/// 从回传文案里取出 `cookie=[...]` 与 `ls=N`。
#[cfg(windows)]
fn parse_state(msg: &str) -> (String, i64) {
    let cookie = msg
        .split("cookie=[")
        .nth(1)
        .and_then(|s| s.split(']').next())
        .unwrap_or("")
        .to_string();
    let ls = msg
        .split("ls=")
        .nth(1)
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(-1);
    (cookie, ls)
}

/// 夹具站点。**每个连接一个线程**（见文件头 ⚠️ 第 1 条：单线程 accept 会被浏览器
/// 预连接堵死，制造出极具误导性的 `0x8007139F`）。
#[cfg(windows)]
fn serve(listener: std::net::TcpListener) {
    use std::io::{Read, Write};

    for stream in listener.incoming() {
        let Ok(mut s) = stream else { continue };
        std::thread::spawn(move || {
            let _ = s.set_read_timeout(Some(Duration::from_secs(5)));

            // 读到请求头结束为止：浏览器可能分片发送，只 read 一次会拿不到 Cookie 头。
            let mut buf: Vec<u8> = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                match s.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&chunk[..n]);
                        if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() > 16 * 1024 {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }

            let head = String::from_utf8_lossy(&buf).to_string();
            let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
            let cookie = head
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("cookie:"))
                .and_then(|l| l.split_once(':'))
                .map(|(_, v)| v.trim().to_string())
                .unwrap_or_default();
            // 这行是**网络层**证据：清除前后 `/check` 请求上的 Cookie 头对比，比页面里读 DOM 更硬。
            println!("REQ {path} cookie=[{cookie}]");

            let body = if path == "/" {
                PAGE_SEED.to_string()
            } else if path.starts_with("/check") {
                PAGE_CHECK.to_string()
            } else {
                // favicon 之类：回空 200，别让浏览器反复重试。
                String::new()
            };

            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
                 Cache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = s.write_all(resp.as_bytes());
            let _ = s.flush();
        });
    }
}

/// **发起**「清除全部浏览数据」并立即返回；完成结果经 `tx` 送出。
///
/// ⚠️ 本函数不得阻塞等待完成回调：完成回调正是要靠主线程的消息泵派发的，
/// 而我们此刻就在主线程上（`wait_for_async_operation` 那种就地抽泵的写法会自锁）。
#[cfg(windows)]
fn start_clear(
    wv: &wry::WebView,
    tx: std::sync::mpsc::Sender<Result<(), String>>,
) -> Result<(), String> {
    use webview2_com::ClearBrowsingDataCompletedHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2Profile2, ICoreWebView2_13};
    use windows::core::Interface;
    use wry::WebViewExtWindows;

    let core = wv.webview();
    // ⚠️ `Profile` 在 ICoreWebView2_13 上（运行时 1.0.1108+），不在 ICoreWebView2 上 ——
    //    对 ICoreWebView2 直接调 `.Profile()` 是编译期就找不到方法（E0599）。
    let core13: ICoreWebView2_13 = core
        .cast()
        .map_err(|e| format!("cast ICoreWebView2_13 失败：{e}"))?;
    let profile = unsafe { core13.Profile() }.map_err(|e| format!("取 profile 失败：{e}"))?;
    // `ClearBrowsingDataAll` 在 ICoreWebView2Profile2（运行时 1.0.1245+）上。
    let profile2: ICoreWebView2Profile2 = profile
        .cast()
        .map_err(|e| format!("cast ICoreWebView2Profile2 失败：{e}"))?;

    // HRESULT 参数已被 webview2-com 转成 `Result<()>`（`ClosureArg for HRESULT`），
    // 不需要手动 `.ok()`。
    let handler = ClearBrowsingDataCompletedHandler::create(Box::new(
        move |result: windows::core::Result<()>| {
            println!("观测：完成回调闭包已执行，result={result:?}");
            let _ = tx.send(result.map_err(|e| format!("ClearBrowsingDataAll 报错：{e}")));
            Ok(())
        },
    ));

    unsafe { profile2.ClearBrowsingDataAll(&handler) }.map_err(|e| format!("发起清除失败：{e}"))
}

/// 「清除前」的 cookie 库快照：必须是**真的有东西**，否则后面的「清空」不算证据。
#[cfg(windows)]
fn assert_had_cookies(wv: &wry::WebView, root_url: &str) {
    let cookies = match wv.cookies_for_url(root_url) {
        Ok(c) => c,
        Err(e) => fail(format!("读取清除前 cookie 库失败：{e}")),
    };
    let names: Vec<String> = cookies
        .iter()
        .map(|c| format!("{}={}", c.name(), c.value()))
        .collect();
    println!("① 清除前 cookies_for_url → {} 个 {names:?}", cookies.len());
    if cookies.is_empty() {
        fail("清除前浏览器 cookie 库本来就是空的 ⇒ 后面「清空」什么也证明不了（判据不成对）");
    }
}

/// 去 `/check`（第 `attempt` 次）：先 `load_url`，失败即回退到渲染进程侧 `location.href`；
/// 同时挂一个看门狗，超时且还有次数就重试导航。
///
/// ⚠️ 「`load_url` 返回 Ok」**不等于**导航真的发生了 —— 清除的尾巴会把导航静默丢掉
/// （见文件头 ⚠️ 第 2 条）。所以判据只看 `/check` 有没有回传。
#[cfg(windows)]
fn try_navigate(
    wv: &wry::WebView,
    proxy: &tao::event_loop::EventLoopProxy<Msg>,
    port: u16,
    attempt: u32,
) {
    let target = format!("http://127.0.0.1:{port}/check");
    match wv.load_url(&target) {
        Ok(()) => println!("第 {attempt} 次请求导航到 /check（load_url 返回 Ok）"),
        Err(e) => {
            println!("⚠️ 第 {attempt} 次 load_url 失败（{e}），回退到渲染进程侧导航 location.href");
            // 回退也失败时**不在这里 fail**：这只是「导航不可用」的又一处证据，
            // 最终裁决交给看门狗（它能把「观测不到」与「数据没清掉」区分开）。
            if let Err(e2) = wv.evaluate_script(&format!("location.href = {target:?}")) {
                println!("⚠️ 回退导航也失败（{e2}），等看门狗处理");
            }
        }
    }

    let proxy_w = proxy.clone();
    std::thread::spawn(move || {
        std::thread::sleep(WAIT_NAV);
        let _ = proxy_w.send_event(Msg::NavWatchdog(attempt));
    });
}

/// 阶段一：断言「清除前有数据」→ 发起清除（只发起）→ 另起线程等回调 + 等落定。
#[cfg(windows)]
fn stage_after_seed(
    wv: &wry::WebView,
    proxy: &tao::event_loop::EventLoopProxy<Msg>,
    root_url: &str,
    port: u16,
    skip_clear: bool,
) {
    assert_had_cookies(wv, root_url);

    if skip_clear {
        println!("[对照 B] 跳过清除，直接去 /check（此时数据应当仍在）");
        try_navigate(wv, proxy, port, 1);
        return;
    }

    // 单变量对照（`PROBE_USE_WRY_CLEAR=1`）：上游 wry 的 `clear_all_browsing_data()` 是
    // **纯 fire-and-forget**（handler 里什么都不做、语句结束就把 handler 丢掉）。
    let use_wry_clear = std::env::var("PROBE_USE_WRY_CLEAR").is_ok();
    let (tx, rx) = std::sync::mpsc::channel();
    if use_wry_clear {
        println!("[对照 C] 改用 wry::WebView::clear_all_browsing_data()（不等回调）");
        if let Err(e) = wv.clear_all_browsing_data() {
            fail(format!("wry clear_all_browsing_data 失败：{e}"));
        }
    } else if let Err(e) = start_clear(wv, tx) {
        fail(e);
    }

    CLEAR_INITIATED.store(true, std::sync::atomic::Ordering::SeqCst);
    let proxy_cb = proxy.clone();
    std::thread::spawn(move || {
        let started = std::time::Instant::now();

        // 只观测回调到没到；真判据是「数据有没有被清掉」。
        // ⚠️ 必须区分 Timeout 与 Disconnected：后者意味着 handler（连着闭包）被提前释放、
        //    回调**根本没机会**执行 —— 那和「等在超时上」是完全不同的两个现象，
        //    而 `.ok()` 会把两者都塌成 None。
        let r = match rx.recv_timeout(WAIT_CALLBACK) {
            Ok(v) => {
                println!("观测：等完成回调用了 {:?}", started.elapsed());
                Some(v)
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                println!("观测：等满 {:?} 仍未收到完成回调", started.elapsed());
                None
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                println!(
                    "观测：等 {:?} 后 channel 断开 ⇒ handler 被提前释放（回调没机会执行）",
                    started.elapsed()
                );
                None
            }
        };

        // ⚠️ 关键：**无论回调到没到**都要再等一段落定时间，否则就是在清除仍在进行的
        //    窗口里做后续动作（见文件头 ⚠️ 第 2 条）。
        let elapsed = started.elapsed();
        if elapsed < SETTLE_AFTER_CLEAR {
            std::thread::sleep(SETTLE_AFTER_CLEAR - elapsed);
        }
        println!("观测：清除发起后共等 {:?} 才开始验证", started.elapsed());

        let _ = proxy_cb.send_event(Msg::Cleared(r));
    });
}

fn main() {
    #[cfg(windows)]
    run();
    #[cfg(not(windows))]
    println!("RESULT SKIP probe_clear_data 仅用于 Windows WebView2");
}

#[cfg(windows)]
fn run() {
    use std::cell::RefCell;
    use std::net::TcpListener;
    use std::rc::Rc;

    use tao::event::Event;
    use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder};
    use tao::window::WindowBuilder;
    use wry::{WebView, WebViewBuilder};

    let check_only = std::env::var("PROBE_CHECK_ONLY").is_ok();
    let skip_clear = std::env::var("PROBE_SKIP_CLEAR").is_ok();
    // 生产里的窗口是可见的；若「隐藏」是被 WebView2 当成异常状态的诱因，这个开关用来证伪。
    let visible = std::env::var("PROBE_VISIBLE").is_ok();

    // ---------- 夹具站点：/ 写数据，/check 只读 ----------
    let listener = TcpListener::bind("127.0.0.1:0").expect("绑定夹具端口失败");
    let port = listener.local_addr().expect("取端口失败").port();
    std::thread::spawn(move || serve(listener));

    let event_loop: EventLoop<Msg> = EventLoopBuilder::<Msg>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window = WindowBuilder::new()
        .with_title("web2app-probe-clear-data")
        .with_visible(visible)
        .build(&event_loop)
        .expect("创建探针窗口失败");

    let root_url = format!("http://127.0.0.1:{port}/");
    let start_url = if check_only {
        println!("[对照 A] 不写数据、不清除，只验证「新导航 + IPC 回传」脚手架");
        format!("http://127.0.0.1:{port}/check")
    } else {
        root_url.clone()
    };

    // WebView 句柄只能在 build 之后拿到 ⇒ 用 `Rc<RefCell<Option<WebView>>>` 在事件循环里共享
    // （同一线程，无需锁；且只用 `borrow()`、不 `borrow_mut`）。
    let holder: Rc<RefCell<Option<WebView>>> = Rc::new(RefCell::new(None));
    let holder_loop = Rc::clone(&holder);
    let proxy_ipc = proxy.clone();

    let webview = WebViewBuilder::new()
        .with_url(start_url)
        .with_visible(visible)
        .with_ipc_handler(move |req| {
            let msg = req.body().to_string();
            if msg.starts_with("SEED") {
                let (cookie, ls) = parse_state(&msg);
                // ① 的前置：清除前必须真的有数据，否则「清除后为空」什么都证明不了
                if !cookie.contains("probekey=probeval") || ls <= 0 {
                    fail(format!(
                        "清除前页面未观察到数据（cookie=[{cookie}] ls={ls}）"
                    ));
                }
                println!("夹具页已写入 cookie 与 localStorage（ls={ls}）");
                let _ = proxy_ipc.send_event(Msg::AfterSeed);
            } else if msg.starts_with("CHECK") {
                let (cookie, ls) = parse_state(&msg);
                println!("② 新文档：document.cookie=[{cookie}] localStorage.length={ls}");
                // ⚠️ 判据只看 cookie（localStorage 见文件头：未清除时也可能读到 0，
                //    拿它当判据等于把缺陷锁死）。真正的成对性来自对照组 B。
                let empty = cookie.trim().is_empty();
                if check_only {
                    println!(
                        "RESULT {} [对照 A] 脚手架可用（本次未写数据，故「空」是预期的）",
                        if empty { "PASS" } else { "FAIL" }
                    );
                    std::process::exit(if empty { 0 } else { 1 });
                }
                if skip_clear {
                    // 对照组 B 的期望是**反的**：数据应当仍在。
                    println!(
                        "RESULT {} [对照 B] 未清除时新文档{}看到 cookie（数据仍在 = 期望行为）",
                        if empty { "FAIL" } else { "PASS" },
                        if empty { "没能" } else { "能" }
                    );
                    std::process::exit(if empty { 1 } else { 0 });
                }
                if empty {
                    println!(
                        "RESULT PASS 清除前 cookie 库非空 → 清除后新文档与夹具服务端都看不到 cookie\
                         （成对判据成立；服务端侧请核上面的 REQ /check cookie=[]）"
                    );
                    std::process::exit(0);
                }
                fail(format!("清除后新文档仍能读到 cookie=[{cookie}]"));
            }
        })
        .build(&window)
        .expect("创建 WebView 失败");
    *holder.borrow_mut() = Some(webview);

    // 兜底：任何一步没走完都不能挂死 —— **包括「主线程卡在 `load_url` 里」那种**，
    // 那时只有这条线程还能收尾。所以它必须能看出走到哪一步了（见 `CLEAR_INITIATED`）。
    std::thread::spawn(|| {
        std::thread::sleep(TOTAL_TIMEOUT);
        if CLEAR_INITIATED.load(std::sync::atomic::Ordering::SeqCst) {
            println!(
                "RESULT INCONCLUSIVE 清除已发起，但本机在清除后 {}s 内无法继续观测（`load_url` \
                 不是返回错误而是**直接阻塞**，实测卡满 87s；窗口可见 / 隐藏、用谁的 clear API \
                 都一样）⇒ 端到端结论在本机做不成。已知数据会被清掉（对照 C 已证），\
                 请真机目视确认。",
                TOTAL_TIMEOUT.as_secs()
            );
            std::process::exit(2);
        }
        println!(
            "RESULT FAIL TIMEOUT（{}s 内还没走到清除就卡住了 —— 探针自身或环境有问题）",
            TOTAL_TIMEOUT.as_secs()
        );
        std::process::exit(1);
    });

    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(Msg::AfterSeed) => {
                let b = holder_loop.borrow();
                let wv = b.as_ref().expect("WebView 尚未就绪");
                stage_after_seed(wv, &proxy, &root_url, port, skip_clear);
            }
            Event::UserEvent(Msg::Cleared(r)) => {
                match r {
                    Some(Ok(())) => println!("观测：完成回调已到达（Ok）"),
                    Some(Err(e)) => println!("观测：完成回调已到达，但带回错误：{e}"),
                    None => println!("观测：本次没拿到完成回调（数据是否清掉以 ② 为准）"),
                }
                let b = holder_loop.borrow();
                let wv = b.as_ref().expect("WebView 尚未就绪");
                try_navigate(wv, &proxy, port, 1);
            }
            Event::UserEvent(Msg::NavWatchdog(attempt)) => {
                let b = holder_loop.borrow();
                let wv = b.as_ref().expect("WebView 尚未就绪");
                if attempt < NAV_ATTEMPTS {
                    println!(
                        "⚠️ 第 {attempt} 次导航 {}s 内没等到 CHECK 回传（清除的尾巴会静默丢导航），重试",
                        WAIT_NAV.as_secs()
                    );
                    try_navigate(wv, &proxy, port, attempt + 1);
                    return;
                }
                let cur = wv.url().unwrap_or_else(|e| format!("<取 url 失败：{e}>"));
                match wv.evaluate_script("window.__probeAlive = 1") {
                    Ok(()) => println!("观测：此时 evaluate_script 仍被接受（WebView 未死）"),
                    Err(e) => println!("观测：此时 evaluate_script 被拒：{e}"),
                }
                // ⚠️⚠️ 走到这里**不能报 FAIL**。实测清除之后该 WebView 的导航会被静默丢弃
                // （窗口可见 / 隐藏、用谁的 clear API 都一样，窗口期实测 ≥32s），
                // 所以「观测不到」是**环境限制**，不等于「数据没被清掉」——两者必须分开，
                // 否则就是把环境问题伪装成功能缺陷（教训：取证方法错 ⇒ 结论错）。
                // 退出码 2 = 无法在本机取证，与 1（真的不成立）刻意区分。
                println!(
                    "RESULT INCONCLUSIVE 清除已发起且 ①（清除前 cookie 库非空）成立，但本机 \
                     {WAIT_NAV:?} × {attempt} 次导航都没落定（当前 url = {cur}）⇒ 端到端观测在本机\
                     做不成。已知数据会被清掉（对照 C 已证：cookies_for_url→0、服务端 cookie 头为空）；\
                     请真机目视确认，或换环境跑。"
                );
                std::process::exit(2);
            }
            _ => {}
        }
    });
}
