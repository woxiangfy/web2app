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
// base64 0.22 的 `.encode()` 是 Engine trait 上的方法，必须显式引入该 trait 才能调用
// （proxy.rs 顶部已 `use base64::Engine;`，这里的 WebView2 钩子里同样需要）。
use base64::Engine;
use tauri::Manager;
use windows::core::{Interface, HSTRING, PWSTR};
use windows::Win32::System::Com::{CoTaskMemFree, IStream};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::config::DEVTOOLS_ENABLED;

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

/// 计算下载的默认落盘路径：系统下载目录 + 文件名（Content-Disposition → URL 末段 → "download"）。
/// 拿不到系统下载目录时返回 None，调用方维持 WebView2 默认行为。
fn default_download_path(
    app: &tauri::AppHandle,
    uri: &Option<String>,
    cd: &Option<String>,
) -> Option<std::path::PathBuf> {
    let dir = app.path().download_dir().ok()?;
    let name = cd
        .as_deref()
        .and_then(content_disposition_filename)
        .or_else(|| uri.as_deref().and_then(url_path_filename))
        .map(|n| sanitize_filename(&n))
        .unwrap_or_else(|| "download".to_string());
    Some(dir.join(name))
}

/// 在 Windows 上给目标窗口注册 WebView2 原生事件（safe 接口）。
///
/// - 新窗口请求（`target="_blank"` 链接 / JS `window.open()`）：多页签模式下新开一个 tab
///   （落点见 [`crate::tabs::open_new_tab_from_request`]）；单窗口模式下用 `SetNewWindow`
///   把当前窗口设为新窗口目标，强制在当前窗口打开且 `window.open()` 返回值有效。
/// - Basic 鉴权（URL 带 userinfo 时）：
///   1) `WebResourceRequested` 拦截【每一个子资源请求】（css/js/xhr/fetch），主动注入 `Authorization`
///      头，这是“始终鉴权”的可靠做法，子资源不会丢凭据；
///   2) `BasicAuthenticationRequested` 兜底，主框架文档（不触发 WebResourceRequested 的硬限制）及
///      任何漏网请求收到 401 时自动应答账号密码。
/// - 右键菜单：隐藏「更多工具」「另存为」「打印」，新增「设置」项。
///
/// 本函数只负责**调度**，真正的注册逻辑在 [`register_webview_hooks`]。
///
/// 调度方式（踩过的坑，务必保持）：在**后台线程**里循环调用 `window.with_webview`，
/// 每次只尝试一次，拿到 `CoreWebView2` 就注册并结束；拿不到就隔 50ms 再试，最多 5s。
///
/// ⚠️ 重试意味着**多个闭包可能同时在途**（`with_webview` 从非主线程调用是异步投递，
/// 发完就返回，闭包要等主线程消息循环才执行），它们会**逐个**跑到注册那一步。
/// 这本身不是问题——真正的去重由 [`register_webview_hooks`] 开头的 [`HOOKS_REGISTERED`]
/// 闸门保证「一个窗口只挂一套处理器」。**不要**把这里的重试改成以 `done` 提前收尾来
/// 「避免重复」：`done` 只能说明「已有闭包跑到过注册那一步」，凭它无法在投递时判断
/// 后续闭包是否会重复注册，闸门才是唯一可靠的位置。
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
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            // 每次尝试用一个独立的 done 标志：`with_webview` 是异步投递的，
            // 只能靠它判断「闭包是否真的执行到了注册那一步」。
            let done = Arc::new(AtomicBool::new(false));
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

            if sent.is_err() {
                // 窗口已销毁 / 事件循环已退出：再试也没意义。
                return;
            }
            if done.load(Ordering::SeqCst) {
                return;
            }
            if std::time::Instant::now() >= deadline {
                eprintln!(
                    "[webview_hooks] CoreWebView2 在 5s 内未就绪，窗口 {window_label} 的钩子未注册"
                );
                return;
            }
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

    // --- 1) 新窗口请求：多页签模式新开 tab；单窗口模式强制在当前窗口打开 ---
    let core_for_nw = core.clone();
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
                    eprintln!(
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
                    // 单窗口模式：保持历史行为——强制在当前窗口打开。
                    let _ = args.SetNewWindow(&core_for_nw);
                    let _ = args.SetHandled(true);
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

    // --- 1c) 下载支持：把下载拦截到系统下载目录，不弹 WebView2 默认下载栏 ---
    // 需要 ICoreWebView2_4（较新运行时才暴露 DownloadStarting）；cast 失败（极老运行时）
    // 则跳过，保持 WebView2 默认下载行为，不影响其它钩子。
    // 文件名优先取 Content-Disposition（filename*/filename），否则取 URL 路径末段，
    // 均做 Windows 非法字符清洗；拿不到系统下载目录时放弃设置路径（走默认行为）。
    let app_for_dl = app.clone();
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
                    if let Some(path) = default_download_path(&app_for_dl, &uri, &cd) {
                        let _ =
                            args.SetResultFilePath(&HSTRING::from(path.to_string_lossy().as_ref()));
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
}
