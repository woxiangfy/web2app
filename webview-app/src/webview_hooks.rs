//! Windows 专属：直接调用 WebView2 原生 API 处理 Basic 鉴权、新窗口请求与右键菜单定制。
//!
//! 注意：Tauri 的 `on_web_resource_request` 只支持 tauri:// 协议，对外部 https 网址不触发，
//! 所以鉴权必须走 WebView2 原生事件。
//!
//! 关键限制：WebView2 的 WebResourceRequested / BasicAuthenticationRequested 已确认【不作用于
//! WebSocket】（参见 MicrosoftEdge/WebView2Feedback #4303）。Trilium 的实时同步正走的是 WS，
//! 因此仅靠原生事件无法解决 WS 401 断开的问题。
//!
//! 最终方案（三层兜底，仅 Windows 生效）：
//!   1) 本地反向代理（条件启用，见 `bootstrap.rs`）：程序启动一个 127.0.0.1:随机端口 的
//!      HTTP/WS 代理，给【每一个】请求（含 WebSocket 握手）注入 `Authorization: Basic ...` 头。
//!   2) WebResourceRequested：拦截 css/js/xhr/fetch 等子资源，主动注入 Authorization 头。
//!   3) BasicAuthenticationRequested：最后兜底，任何收到 401 的请求自动应答账号密码。
//!
//! Android 等移动端：没有 WebView2，鉴权统一走本地反向代理，本模块在移动端被 cfg 剔除。

use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2, ICoreWebView2ContextMenuItem, ICoreWebView2ContextMenuItemCollection,
    ICoreWebView2Deferral, ICoreWebView2Environment9, ICoreWebView2_11, ICoreWebView2_13,
    ICoreWebView2_2, ICoreWebView2_4, COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_COMMAND,
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
};
use webview2_com::{
    BasicAuthenticationRequestedEventHandler, ContextMenuRequestedEventHandler,
    CustomItemSelectedEventHandler, DownloadStartingEventHandler, NewWindowRequestedEventHandler,
    WebResourceRequestedEventHandler,
};
// base64 0.23 的 `.encode()` 是 Engine trait 上的方法，必须显式引入该 trait 才能调用
// （proxy.rs 顶部已 `use base64::Engine;`，这里的 WebView2 钩子里同样需要）。
use base64::Engine;
use tauri::Manager;
use windows::core::{Interface, HSTRING, PWSTR};
use windows::Win32::System::Com::{CoTaskMemFree, IStream};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::config::DEVTOOLS_ENABLED;
use crate::save_dialog::SaveChoice;

/// 「钩子已注册」的窗口 label 集合 —— **去重闸门**，保证一个窗口的 WebView2 事件处理器
/// 只挂一次。
///
/// 为什么必须有它（2026-09-13 用户反馈「点一次不停的打开新窗口，电脑都被干死机」的根因）：
/// `window.with_webview(closure)` 从**非主线程**调用时走的是 `EventLoopProxy::send_event`
/// （见 `tauri_runtime_wry::send_user_message`），是**异步投递**——调用方发完消息立刻返回，
/// 闭包要等主线程下一次消息循环才执行。所以 [`attach_webview_hooks`] 的 5s 重试循环完全
/// 可能同时有多个闭包在途，它们随后**逐个**执行、**每个都**会调用 [`register_webview_hooks`]
/// → 同一个窗口挂上多个 `NewWindowRequested` 处理器 → WebView2 把**一次**点击分发给
/// **每一个**处理器 → 一次点击打开 N 个新窗口（N = 已挂载的处理器数）。
///
/// 更糟的是 N 会随主线程繁忙程度增长：窗口越多 → 建窗越慢 → 在途排队的闭包越多 → N 越大，
/// 形成正反馈，几十个 WebView2 实例一起启动就把整机拖死。
///
/// 闸门在**主线程**（`with_webview` 闭包内）执行，用 label 做一次性 check-and-insert，
/// 天然串行且原子：第一个跑到的闭包注册，其余闭包直接返回。
///
/// 注：2026-09-14 修掉 [`attach_webview_hooks`] 的 `done` 标志竞态后，重试循环「有一次落地
/// 就收工」，在途闭包通常只剩 1~2 个（此前是每 50ms 一个、最多约 100 个）。但闸门**仍然必需**：
/// 控制器就绪较慢时依然会有多个闭包先后跑到这一步，去重不能指望调度侧的时序。
///
/// 保留策略：label 在一个进程内唯一且永不复用（`tab-N` 由单调计数器生成），故无需清理。
static HOOKS_REGISTERED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// 读取右键菜单项集合的项数（COM 老式 out 参数封装）。
fn read_menu_count(col: &ICoreWebView2ContextMenuItemCollection) -> Option<u32> {
    unsafe {
        let mut n: u32 = 0;
        col.Count(&mut n).ok()?;
        Some(n)
    }
}

/// 读取右键菜单项的【非本地化 Name】（英文小驼峰，如 "saveAs"/"print"/"moreTools"）。
/// WebView2 用 CoTaskMemAlloc 分配该字符串，读完用 CoTaskMemFree 释放，避免内存泄漏。
fn read_menu_name(item: &ICoreWebView2ContextMenuItem) -> Option<String> {
    unsafe {
        let mut p: PWSTR = PWSTR::null();
        if item.Name(&mut p).is_err() {
            return None;
        }
        let s = p.to_string().ok();
        if !p.is_null() {
            CoTaskMemFree(Some(p.0 as *const core::ffi::c_void));
        }
        s
    }
}

/// 极简 percent-decode：把 `%XX` 还原为字节后按 UTF-8 解码。
/// 非法编码（如孤立 `%`、非十六进制位）或非 UTF-8 字节序列返回 None。
/// （依赖树中 url crate 的 percent_encoding 模块未开 feature，这里自实现，零新依赖。）
fn percent_decode_utf8(s: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hi = chars.next()?.to_digit(16)?;
            let lo = chars.next()?.to_digit(16)?;
            bytes.push((hi * 16 + lo) as u8);
        } else {
            let mut buf = [0u8; 4];
            bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
    }
    String::from_utf8(bytes).ok()
}

/// 从 `Content-Disposition` 头提取文件名（RFC 6266）。
/// 优先 `filename*=UTF-8''<percent-encoded>`（RFC 5987，中文文件名走这个），
/// 其次 `filename="..."` / `filename=...`。解析不到返回 None。
fn content_disposition_filename(cd: &str) -> Option<String> {
    // filename*=UTF-8''...（大小写不敏感前缀；ASCII 前缀长度与原串一致，可安全按长度切片）
    for part in cd.split(';') {
        let p = part.trim();
        let lower = p.to_ascii_lowercase();
        if lower.starts_with("filename*=utf-8''") {
            let encoded = &p["filename*=utf-8''".len()..];
            let decoded = percent_decode_utf8(encoded)?;
            if !decoded.is_empty() {
                return Some(decoded);
            }
        }
    }
    // filename="..." 或 filename=...
    for part in cd.split(';') {
        let p = part.trim();
        let lower = p.to_ascii_lowercase();
        if lower.starts_with("filename=") {
            let name = p["filename=".len()..].trim().trim_matches('"');
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// 从下载 URL 的路径末段提取文件名（percent-decode 后返回）；路径无文件名时返回 None。
fn url_path_filename(uri: &str) -> Option<String> {
    let u = url::Url::parse(uri).ok()?;
    let last = u.path().rsplit('/').find(|s| !s.is_empty())?;
    let decoded = percent_decode_utf8(last)?;
    if decoded.is_empty() {
        None
    } else {
        Some(decoded)
    }
}

/// 清洗文件名：把 Windows 路径非法字符与控制在 0x20 以下的字符替换为下划线，
/// 去掉首尾空白与末尾句点；结果为空时回落默认名 "download"。
///
/// 字符替换复用 [`crate::config::sanitize_windows_name`]（与目录名清洗同一套规则），
/// 仅「是否压缩空白 / 默认回落名」两处语义不同，故在此各自保留。
fn sanitize_filename(name: &str) -> String {
    let cleaned = crate::config::sanitize_windows_name(name, false);
    let trimmed = cleaned.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        "download".to_string()
    } else {
        trimmed.to_string()
    }
}

/// 计算下载的建议文件名：`Content-Disposition`（RFC 6266/5987）→ URL 路径末段 → `"download"`，
/// 并做 Windows 非法字符清洗。用作「另存为」对话框里的默认文件名。
fn suggested_filename(uri: Option<&str>, cd: Option<&str>) -> String {
    cd.and_then(content_disposition_filename)
        .or_else(|| uri.and_then(url_path_filename))
        .map(|n| sanitize_filename(&n))
        .unwrap_or_else(|| "download".to_string())
}

/// 在 Windows 上给目标窗口注册 WebView2 原生事件（safe 接口）。
///
/// - 新窗口请求（`target="_blank"` 链接 / JS `window.open()`）：多页签模式下新开一个 tab
///   （落点见 [`crate::tabs::open_new_tab_from_request`]）；单窗口模式下由 Rust 侧把当前窗口
///   **导航**到目标地址（**不能**用 `SetNewWindow(当前 core)`，原因见注册逻辑里的长注释）。
/// - Basic 鉴权（URL 带 userinfo 时）：
///   1) `WebResourceRequested` 拦截【每一个子资源请求】（css/js/xhr/fetch），主动注入 `Authorization`
///      头，这是“始终鉴权”的可靠做法，子资源不会丢凭据；
///   2) `BasicAuthenticationRequested` 兜底，主框架文档（不触发 WebResourceRequested 的硬限制）及
///      任何漏网请求收到 401 时自动应答账号密码。
/// - 右键菜单：隐藏「更多工具」「另存为」「打印」，新增「设置」项。
/// - 下载（`DownloadStarting`）：弹系统原生「另存为」对话框，由用户决定落盘位置
///   （见 [`crate::save_dialog`]）；用户取消则取消这次下载。
///
/// 本函数只负责**调度**，真正的注册逻辑在 [`register_webview_hooks`]。
///
/// 调度方式（踩过的坑，务必保持）：在**后台线程**里循环调用 `window.with_webview`，
/// 每次只尝试一次，拿到 `CoreWebView2` 就注册并结束；拿不到就隔 50ms 再试，最多 5s。
///
/// ⚠️⚠️ 判成功的 `done` 标志**必须全函数共用一个，且在【轮次开头】检查**。
/// 曾经是「每轮新建一个 `done`，`with_webview` 返回后立刻检查」，结果是 **100% 误报**：
/// `with_webview` 从非主线程调用是**异步投递**（`proxy.send_event`），返回时闭包还没执行，
/// 立刻检查必然读到 `false`；而闭包真正执行、置位的又是「上一轮」那个已被丢弃的标志。
/// 于是**哪怕第一次尝试就注册成功，循环也一定空转到 5s 超时**，打出
/// 「CoreWebView2 在 5s 内未就绪，窗口 X 的钩子未注册」的假告警（每次启动 / 每个窗口一条），
/// 同时白白排入约 100 个闭包去占主线程。
///
/// 去重职责**不在**这里，而在 [`register_webview_hooks`] 开头的 [`HOOKS_REGISTERED`] 闸门
/// （它保证「一个窗口只挂一套处理器」）。所以「看到 `done` 就提前收工」不破坏去重——
/// 前提是 `done` 的语义正确：**它表示「本窗口的注册已经跑到底」**（`register_webview_hooks`
/// 已返回，含闸门命中直接返回的情形），此时继续重试纯属浪费。
///
/// ⚠️ 绝不能改成在主线程里同步 sleep 轮询等 controller：`tauri_runtime_wry` 的
/// `send_user_message` 在**主线程**调用时是**内联同步执行**闭包的（源码即
/// `if current_thread().id() == context.main_thread_id { handle_user_message(..) }`），
/// 主线程的闭包里一 sleep，消息泵就停了；而 WebView2 的 controller 恰恰要靠这个消息泵
/// 派发创建回调（`CreateCoreWebView2EnvironmentWithOptions` 是异步的）→ 死锁，等到超时
/// 也拿不到 controller，钩子全部注册不上，界面还白卡 5 秒。（这正是上一版“改了等于没改”
/// 的原因。）从后台线程调用 `with_webview` 时 `send_user_message` 走 `proxy.send_event`
/// 分支（异步投递），主线程执行完闭包立刻返回，消息泵继续转，controller 才有机会就绪。
pub(crate) fn attach_webview_hooks(
    app: tauri::AppHandle,
    window: &tauri::WebviewWindow,
    auth_user: String,
    auth_pass: String,
    use_local_proxy: bool,
) {
    // label 拷出来用于错误日志；window 需要 clone 进后台线程（闭包要 'static）。
    let window_label = window.label().to_string();
    let window = window.clone();

    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let deadline = started + std::time::Duration::from_secs(5);
        // ⚠️ 全函数**共用**这一个标志（不要每轮新建）：任一次投递的闭包把注册跑到
        // 底就置位，外层据此收工。语义与检查位置的关系见函数文档的 ⚠️⚠️ 段。
        let done = Arc::new(AtomicBool::new(false));
        let mut attempts: u32 = 0;
        // 超时后的宽限只给一次，避免在真正注册不上时无限等待。
        let mut grace_used = false;

        loop {
            // ⚠️⚠️ 必须在【轮次开头】检查，不能像旧实现那样在 `with_webview` 返回后立刻检查：
            // 非主线程调用是异步投递，返回时闭包尚未执行，立刻检查必然是 false（→ 必然空转
            // 到 5s 并打出假告警）。放在这里则已经过了一整轮 50ms，标志有充分时间被置位。
            if done.load(Ordering::SeqCst) {
                dlog!(
                    "[webview_hooks] 窗口 {window_label} 的钩子已注册（尝试 {attempts} 次，耗时 {}ms）",
                    started.elapsed().as_millis()
                );
                return;
            }
            if std::time::Instant::now() >= deadline {
                // 宽限一次：在途闭包可能刚好在超时判定前后才跑到（控制器就绪回调与主线程
                // 消息泵都很忙时）。这里只 sleep 再 `continue`，让成功判定统一走上面那一处
                // ——否则会出现「其实注册上了却没有成功日志」的静默路径。
                if !grace_used {
                    grace_used = true;
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    continue;
                }
                dlog!(
                    "[webview_hooks] CoreWebView2 在 5s 内未就绪，窗口 {window_label} 的钩子未注册（已尝试 {attempts} 次）"
                );
                return;
            }

            let done_in = done.clone();
            let app_in = app.clone();
            let user_in = auth_user.clone();
            let pass_in = auth_pass.clone();
            let label_in = window_label.clone();

            let sent = window.with_webview(move |wv| {
                // controller 未就绪：不注册也不标记 done，交给外层下一轮重试。
                let core = match unsafe { wv.controller().CoreWebView2() } {
                    Ok(c) => c,
                    Err(_) => return,
                };
                register_webview_hooks(
                    &app_in,
                    core,
                    &user_in,
                    &pass_in,
                    use_local_proxy,
                    &label_in,
                );
                done_in.store(true, Ordering::SeqCst);
            });

            if let Err(e) = sent {
                // 窗口已销毁 / 事件循环已退出：再试也没意义。但这曾是**静默失败**——
                // 窗口钩子没挂上 = 鉴权 / 下载 / 右键菜单全部失效，用户侧零证据。
                // 落一条日志（debug 开启时）便于区分「钩子没触发」与「引擎忙」。
                dlog!(
                    "[webview_hooks] with_webview 投递失败（窗口可能已销毁 / 事件循环已退出），\
                    窗口 {window_label} 的钩子未注册：{e}"
                );
                return;
            }
            attempts += 1;
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    });
}

/// 在已就绪的 `CoreWebView2` 上真正注册各个事件处理器。
///
/// 由 [`attach_webview_hooks`] 在主线程（即 `with_webview` 闭包内）调用。
/// COM 事件处理器由 webview2_com 的 `create` 构造，`add_*` 会正确 AddRef；
/// 闭包捕获的 core / 凭据在 handler 存活期间有效。
///
/// ⚠️ 调用契约：调用方靠「本函数是否返回」判断这次尝试是否落地，因此本函数**必须**走到底
/// ——运行时接口版本太老（cast 失败）只跳过对应那一段，不能在中途 return。
/// 唯一的例外是开头的 [`HOOKS_REGISTERED`] 闸门：命中说明该窗口的钩子已经挂过（或正由
/// 另一个在途闭包挂上），直接返回即代表「注册已完成」，符合调用方预期。
fn register_webview_hooks(
    app: &tauri::AppHandle,
    core: ICoreWebView2,
    user: &str,
    pass: &str,
    use_local_proxy: bool,
    window_label: &str,
) {
    // --- 去重闸门：同一窗口只允许注册一次 ---
    // ⚠️ 别删。没有它，`attach_webview_hooks` 的异步重试会让同一个窗口挂上多个
    // NewWindowRequested 处理器 → 一次点击打开多个新窗口（详见 HOOKS_REGISTERED 的文档）。
    {
        let mut done = HOOKS_REGISTERED.lock().unwrap_or_else(|p| p.into_inner());
        if done.iter().any(|l| l == window_label) {
            return;
        }
        done.push(window_label.to_string());
    }

    // COM 事件处理器的闭包必须 'static：把借用的凭据拷成 owned String。
    // （下面所有逻辑与拿到 owned String 时完全一致，`user.as_str()` 仍然可用。）
    let user = user.to_string();
    let pass = pass.to_string();

    // --- 0) 开发者工具（F12）---
    if DEVTOOLS_ENABLED {
        if let Ok(settings) = unsafe { core.Settings() } {
            unsafe {
                let _ = settings.SetAreDevToolsEnabled(true);
            }
        }
    }

    // --- 1) 新窗口请求：多页签模式新开 tab；单窗口模式改为「当前窗口导航到该地址」---
    let app_for_nw = app.clone();
    let label_for_nw = window_label.to_string();
    let nw_handler = NewWindowRequestedEventHandler::create(Box::new(move |_sender, args| {
        if let Some(args) = args {
            unsafe {
                // 读出页面请求的目标 URL（window.open / target="_blank" 指向的地址），
                // 供多页签模式新开的 tab 导航到对应路径；读不出（内存分配失败等）则回落主地址。
                let mut uri_pw = PWSTR::null();
                let uri = args
                    .Uri(&mut uri_pw)
                    .ok()
                    .and_then(|_| uri_pw.to_string().ok());
                if !uri_pw.is_null() {
                    CoTaskMemFree(Some(uri_pw.0 as *const core::ffi::c_void));
                }

                // 多页签模式：target="_blank" / window.open() 直接新开一个 tab。
                if crate::tabs::tabs_enabled() {
                    // ⚠️ 不能在 COM 回调里【直接】建窗——回调运行在主线程，直接建窗会嵌套
                    // 事件循环导致卡死。先切到后台线程，再经 run_on_main_thread 排队到
                    // 下一个事件循环轮次建窗（与右键菜单「设置」的处理同理）。
                    // 诊断日志：确认弹窗拦截生效，以及 WebView2 给出的目标 URL。
                    // （`resolve_popup_url` 完全依赖它；读不到时会回落主地址。）
                    dlog!(
                        "[webview_hooks] 窗口 {label_for_nw} 拦截到新窗口请求：{}",
                        uri.as_deref().unwrap_or("<无法读取>")
                    );
                    let app_spawn = app_for_nw.clone();
                    let _ = std::thread::spawn(move || {
                        let app_main = app_spawn.clone();
                        let _ = app_spawn.run_on_main_thread(move || {
                            crate::tabs::open_new_tab_from_request(&app_main, uri);
                        });
                    });
                    let _ = args.SetHandled(true);
                } else {
                    // ---- 单窗口模式：把「新窗口请求」变成【当前窗口的一次导航】----
                    //
                    // ⚠️⚠️ 不要改回 `args.SetNewWindow(&当前 core)`。
                    //
                    // 现场对照实验（真实 app + 本地站点，页面回传 DOM 状态，2026-09-14）：
                    //   · 普通整页导航（`location.href`）：新文档里 `#__web2app_titlebar__` 存在、
                    //     rect [0,38,宽]、z-index 最高 → 初始化脚本正常执行；
                    //   · `target="_blank"` 走 `SetNewWindow(当前 core)`：新文档里标题栏**完全不存在**，
                    //     `document.body.children.length` 等于页面自身节点数，连加载层也没有
                    //     → **该文档不执行 `AddScriptToExecuteOnDocumentCreated` 注册的初始化脚本**。
                    //     这正是用户反馈的「点击打开其他页面后顶部菜单栏全部消失」。
                    //
                    // 语义上也不该这样用：MSDN 对 `NewWindow` 的定义是「另建一个 webview 承载新窗口，
                    // 该 webview 必须与 opener 同 environment 且**未被导航过**」。把自己的 core 传进去
                    // 属于滥用：WebView2 会复用当前 webview 承载新内容，同时丢掉脚本注册；页面里的
                    // `w.document.write(...)` 还会直接把主页面改写掉。
                    //
                    // 正确做法：`SetHandled(true)` 关掉默认行为，再由 Rust 侧把当前窗口**导航**到
                    // 目标地址。新页面因此走普通导航路径，初始化脚本（自绘标题栏 / 加载层）与
                    // WebView2 钩子（鉴权 / 下载 / 右键菜单）全部照常生效。
                    // 代价：`window.open()` 的返回值不再是可用的窗口对象——单窗口模式本就没有独立窗口，
                    // 属预期取舍；另见 examples/probe_new_window.rs 里踩过的坑。
                    dlog!(
                        "[webview_hooks] 单窗口模式拦截到新窗口请求：{}",
                        uri.as_deref().unwrap_or("<无法读取>")
                    );
                    let _ = args.SetHandled(true);
                    match uri.as_deref() {
                        Some(t) if !t.is_empty() && t != "about:blank" => {
                            let target = t.to_string();
                            let app_spawn = app_for_nw.clone();
                            let label_spawn = label_for_nw.clone();
                            let _ = std::thread::spawn(move || {
                                // ⚠️ 与右键菜单「设置」同理：本回调运行在主线程，直接
                                // `run_on_main_thread` 会被同步内联执行（嵌套事件循环 → 卡死）。
                                // 先切到后台线程再排队，等 COM 分发退栈后的下一个事件循环轮次
                                // 才在主线程导航。
                                let app_main = app_spawn.clone();
                                let _ = app_spawn.run_on_main_thread(move || {
                                    let Some(window) = app_main.get_webview_window(&label_spawn) else {
                                        return;
                                    };
                                    match url::Url::parse(&target) {
                                        Ok(u) => {
                                            let _ = window.navigate(u);
                                        }
                                        Err(e) => dlog!(
                                            "[webview_hooks] 新窗口目标地址不合法，已忽略：{e}（{target}）"
                                        ),
                                    }
                                });
                            });
                        }
                        // about:blank：典型用法是「先开空窗口再往里 document.write」。
                        // 单窗口模式无法提供独立窗口，硬套会把用户当前页面变成白板，故忽略。
                        _ => dlog!(
                            "[webview_hooks] 忽略无地址的 window.open()（单窗口模式无法提供独立窗口）"
                        ),
                    }
                }
            }
        }
        Ok(())
    }));
    let mut nw_token = 0i64;
    unsafe {
        let _ = core.add_NewWindowRequested(&nw_handler, &mut nw_token);
    }

    // --- 1b) 右键菜单定制 ---
    let app_for_menu = app.clone();
    let core_for_menu = core.clone();
    let cm_handler = ContextMenuRequestedEventHandler::create(Box::new(move |_sender, args| {
        if let Some(args) = args {
            unsafe {
                if let Ok(items) = args.MenuItems() {
                    if let Some(count) = read_menu_count(&items) {
                        let mut i = count;
                        while i > 0 {
                            i -= 1;
                            if let Ok(item) = items.GetValueAtIndex(i) {
                                if let Some(name) = read_menu_name(&item) {
                                    let n = name.to_lowercase();
                                    if n == "moretools" || n == "print" || n.starts_with("save") {
                                        let _ = items.RemoveValueAtIndex(i);
                                    }
                                }
                            }
                        }
                    }
                }
                // 追加自定义「设置」菜单项（点击后打开与 Shift+F1 相同的设置界面）
                if let Ok(core2) = core_for_menu.cast::<ICoreWebView2_2>() {
                    if let Ok(env) = core2.Environment() {
                        if let Ok(env9) = env.cast::<ICoreWebView2Environment9>() {
                            let label = HSTRING::from("设置");
                            if let Ok(setting_item) = env9.CreateContextMenuItem(
                                &label,
                                None::<&IStream>,
                                COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_COMMAND,
                            ) {
                                let ah = app_for_menu.clone();
                                let sel_handler = CustomItemSelectedEventHandler::create(Box::new(
                                    move |_sender, _args| {
                                        // ⚠️ 关键：不能在 COM 回调里【直接】调用 run_on_main_thread。
                                        // WebView2 的 CustomItemSelected/ContextMenuRequested 回调就运行在
                                        // 【主线程】上，而 tauri 的 run_on_main_thread 检测到"已在主线程"时
                                        // 会【同步内联】执行闭包，导致嵌套建窗 → 进程卡死。
                                        // 解法：先切到后台线程，让 run_on_main_thread 从【非主线程】走
                                        // event_loop proxy 排队，等 COM 分发退栈后的下一个事件循环轮次
                                        // 才在主线程建窗（等价于 Shift+F1 的调用上下文，安全）。
                                        let app_spawn = ah.clone();
                                        let _ = std::thread::spawn(move || {
                                            let app_main = app_spawn.clone();
                                            let _ = app_spawn.run_on_main_thread(move || {
                                                crate::settings::open_settings_window(&app_main);
                                            });
                                        });
                                        Ok(())
                                    },
                                ));
                                let mut sel_token = 0i64;
                                let _ = setting_item
                                    .add_CustomItemSelected(&sel_handler, &mut sel_token);
                                if let Ok(items) = args.MenuItems() {
                                    if let Some(count) = read_menu_count(&items) {
                                        let _ = items.InsertValueAtIndex(count, &setting_item);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }));
    // 只有较新的 WebView2 运行时才暴露 ICoreWebView2_11（ContextMenuRequested 在此接口上）。
    // cast 失败（极老运行时）则跳过右键定制，保留默认菜单，不影响其它钩子。
    if let Ok(core11) = core.cast::<ICoreWebView2_11>() {
        let mut cm_token = 0i64;
        unsafe {
            let _ = core11.add_ContextMenuRequested(&cm_handler, &mut cm_token);
        }
    }

    // --- 1c) 下载支持：弹系统「另存为」对话框，由用户决定落盘位置 ---
    // 需要 ICoreWebView2_4（较新运行时才暴露 DownloadStarting）；cast 失败（极老运行时）
    // 则跳过，保持 WebView2 默认下载行为，不影响其它钩子。
    //
    // ⚠️ 旧实现只调 `SetResultFilePath(下载目录 + 站点给的文件名)`：既不给任何交互，同名文件
    // 还会被 WebView2 直接覆盖（MSDN：ResultFilePath 指向已存在文件时「will be overwritten」），
    // 用户看到的现象就是「点下载没反应」。现在改为弹【系统原生另存为】（见 [`crate::save_dialog`]）：
    //   · 确认 → `SetResultFilePath(用户选的路径)` + `SetHandled(true)`
    //     （`Handled` 的语义按 MSDN 是「隐藏 WebView2 自带的下载对话框」，下载照常进行；
    //      不设它就会出现「系统另存为 + WebView2 下载气泡」两个 UI 抢注意力的怪现象）
    //   · 取消 → `SetCancel(true)`（MSDN：Cancel 为真时无论如何都不显示默认保存 UI）
    //   · 对话框建不出来 → 什么都不设，保持 WebView2 原生行为，绝不静默改动用户的数据
    let app_for_dl = app.clone();
    // owner 窗口句柄在主线程这里取一次：对话框要按「真模态」禁用宿主窗口，而在 COM 回调里
    // 再取窗口需要过 Tauri 的 WindowManager 锁 —— 不在回调执行期间持任何锁更安全。
    let owner_hwnd = app
        .get_webview_window(window_label)
        .and_then(|w| w.hwnd().ok());
    let dl_handler = DownloadStartingEventHandler::create(Box::new(move |_sender, args| {
        if let Some(args) = args {
            unsafe {
                if let Ok(op) = args.DownloadOperation() {
                    let mut uri_pw = PWSTR::null();
                    let mut cd_pw = PWSTR::null();
                    let uri = op
                        .Uri(&mut uri_pw)
                        .ok()
                        .and_then(|_| uri_pw.to_string().ok());
                    let cd = op
                        .ContentDisposition(&mut cd_pw)
                        .ok()
                        .and_then(|_| cd_pw.to_string().ok());
                    // WebView2 用 CoTaskMemAlloc 分配这些字符串，用完必须释放，避免内存泄漏。
                    if !uri_pw.is_null() {
                        CoTaskMemFree(Some(uri_pw.0 as *const core::ffi::c_void));
                    }
                    if !cd_pw.is_null() {
                        CoTaskMemFree(Some(cd_pw.0 as *const core::ffi::c_void));
                    }
                    let name = suggested_filename(uri.as_deref(), cd.as_deref());
                    let dir = app_for_dl.path().download_dir().ok();
                    // 这行日志是「下载到底有没有走到钩子」的唯一判据：先打日志再弹框，
                    // 用户报「点了没反应」时据此区分「钩子没触发」与「对话框没弹出来」。
                    dlog!("[webview_hooks] 下载：正在询问保存位置（建议文件名 {name}）");
                    // ⚠️ 这里**同步**跑模态对话框（含它自己的消息循环），全程在主线程。
                    // 理由见 `crate::save_dialog` 顶部注释：换线程会同时丢掉模态语义与 Send 保证。
                    match crate::save_dialog::ask_save_path(owner_hwnd, &name, dir.as_deref()) {
                        SaveChoice::Save(path) => {
                            dlog!(
                                "[webview_hooks] 下载：{name} → 用户选择保存到 {}",
                                path.display()
                            );
                            let _ = args
                                .SetResultFilePath(&HSTRING::from(path.to_string_lossy().as_ref()));
                            let _ = args.SetHandled(true);
                        }
                        SaveChoice::Cancelled => {
                            dlog!("[webview_hooks] 下载：用户取消了保存（{name}）");
                            let _ = args.SetCancel(true);
                        }
                        SaveChoice::Unavailable => {
                            dlog!(
                                "[webview_hooks] 下载：系统「另存为」对话框不可用，本次维持 \
                                 WebView2 默认下载行为（{name}）"
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }));
    if let Ok(core4) = core.cast::<ICoreWebView2_4>() {
        let mut dl_token = 0i64;
        unsafe {
            let _ = core4.add_DownloadStarting(&dl_handler, &mut dl_token);
        }
    }

    // --- 2) Basic 鉴权（仅当 URL 带 userinfo、且未走本地反向代理时）---
    if !user.is_empty() && !use_local_proxy {
        let creds = format!("{}:{}", user, pass);
        let b64 = base64::engine::general_purpose::STANDARD.encode(creds.as_bytes());
        let header_val = format!("Basic {}", b64);

        unsafe {
            let _ = core.AddWebResourceRequestedFilter(
                &HSTRING::from("*"),
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
            );
        }
        let wr_handler =
            WebResourceRequestedEventHandler::create(Box::new(move |_sender, args| {
                if let Some(args) = args {
                    unsafe {
                        if let Ok(req) = args.Request() {
                            if let Ok(headers) = req.Headers() {
                                let _ = headers.SetHeader(
                                    &HSTRING::from("Authorization"),
                                    &HSTRING::from(header_val.as_str()),
                                );
                            }
                        }
                    }
                }
                Ok(())
            }));
        let mut wr_token = 0i64;
        unsafe {
            let _ = core.add_WebResourceRequested(&wr_handler, &mut wr_token);
        }

        // 2b) 兜底：任何收到 401 的请求自动应答账号密码（Deferral 规范写法）。
        //
        // ⚠️ cast 失败（极老运行时）时**不能**提前 return：调用方靠「本函数是否跑完」
        // 判断注册是否成功，提前 return 会被误判成“没注册”并重试，导致上面已挂的
        // WebResourceRequested 等处理器被重复挂载。这里只跳过这一段即可。
        if let Ok(core13) = core.cast::<ICoreWebView2_13>() {
            let handler =
                BasicAuthenticationRequestedEventHandler::create(Box::new(move |_sender, args| {
                    if let Some(args) = args {
                        unsafe {
                            let deferral: Option<ICoreWebView2Deferral> = args.GetDeferral().ok();
                            if let Ok(response) = args.Response() {
                                let _ = response.SetUserName(&HSTRING::from(user.as_str()));
                                let _ = response.SetPassword(&HSTRING::from(pass.as_str()));
                            }
                            let _ = args.SetCancel(false);
                            if let Some(d) = deferral {
                                let _ = d.Complete();
                            }
                        }
                    }
                    Ok(())
                }));
            let mut token = 0i64;
            unsafe {
                let _ = core13.add_BasicAuthenticationRequested(&handler, &mut token);
            }
        }
    }

    // --- 3) 注册完毕。本函数不负责导航：首次导航由调用方用
    // `WebviewWindow::navigate` 触发（见 [`crate::bootstrap::navigate_window`]）。
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_disposition_rfc5987_utf8_filename() {
        // 中文文件名走 filename*=UTF-8''<percent-encoded>
        let cd = "attachment; filename=\"fallback.txt\"; filename*=UTF-8''%E6%8A%A5%E8%A1%A8.xlsx";
        assert_eq!(
            content_disposition_filename(cd).as_deref(),
            Some("报表.xlsx"),
            "应优先取 filename*（RFC 5987）"
        );
    }

    #[test]
    fn content_disposition_quoted_and_plain_filename() {
        assert_eq!(
            content_disposition_filename("attachment; filename=\"report v2.pdf\"").as_deref(),
            Some("report v2.pdf")
        );
        assert_eq!(
            content_disposition_filename("attachment; filename=plain.txt").as_deref(),
            Some("plain.txt")
        );
    }

    #[test]
    fn content_disposition_without_filename_returns_none() {
        assert_eq!(content_disposition_filename("inline"), None);
        assert_eq!(content_disposition_filename("attachment; name=x.bin"), None);
    }

    #[test]
    fn url_path_filename_from_last_segment() {
        assert_eq!(
            url_path_filename("https://host/a/b/report.xlsx?token=1").as_deref(),
            Some("report.xlsx"),
            "应取路径末段并忽略 query"
        );
        assert_eq!(
            url_path_filename("https://host/附件%20%2B%20名.xls").as_deref(),
            Some("附件 + 名.xls"),
            "路径段应做 percent-decode"
        );
        assert_eq!(url_path_filename("https://host/"), None, "根路径无文件名");
        assert_eq!(url_path_filename("not a url"), None);
    }

    #[test]
    fn sanitize_filename_replaces_windows_illegal_chars() {
        assert_eq!(
            sanitize_filename(r#"a<b>c:d"e/f\g|h?i*j"#),
            "a_b_c_d_e_f_g_h_i_j"
        );
        // 末尾句点 / 空白是 Windows 非法结尾，应剥掉；控制字符替换为下划线
        assert_eq!(sanitize_filename("report.txt. "), "report.txt");
        assert_eq!(sanitize_filename("a\tb"), "a_b");
        // 全部非法 → 空 → 回落默认名
        assert_eq!(sanitize_filename("..."), "download");
        assert_eq!(sanitize_filename(""), "download");
    }

    #[test]
    fn suggested_filename_prefers_content_disposition() {
        // Content-Disposition 优先于 URL（站点给的名字更准）
        assert_eq!(
            suggested_filename(
                Some("https://host/download?id=9"),
                Some("attachment; filename=\"报表.xlsx\"")
            ),
            "报表.xlsx"
        );
        // 没有 Content-Disposition 时取 URL 路径末段（percent-decode + 清洗）
        assert_eq!(
            suggested_filename(Some("https://host/a/报告%20v2.pdf?t=1"), None),
            "报告 v2.pdf"
        );
    }

    #[test]
    fn suggested_filename_falls_back_to_download() {
        // 两者都拿不到（如 blob: 下载且无 Content-Disposition）→ 非空默认名
        assert_eq!(suggested_filename(None, None), "download");
        assert_eq!(
            suggested_filename(Some("https://host/"), Some("attachment")),
            "download"
        );
    }
}
