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
    ICoreWebView2_11, ICoreWebView2_13, ICoreWebView2_2, ICoreWebView2ContextMenuItem,
    ICoreWebView2ContextMenuItemCollection, ICoreWebView2Deferral, ICoreWebView2Environment9,
    COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_COMMAND, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
};
use webview2_com::{
    BasicAuthenticationRequestedEventHandler, ContextMenuRequestedEventHandler,
    CustomItemSelectedEventHandler, NewWindowRequestedEventHandler,
    WebResourceRequestedEventHandler,
};
// base64 0.22 的 `.encode()` 是 Engine trait 上的方法，必须显式引入该 trait 才能调用
// （proxy.rs 顶部已 `use base64::Engine;`，这里的 WebView2 钩子里同样需要）。
use base64::Engine;
use windows::core::{Interface, HSTRING, PWSTR};
use windows::Win32::System::Com::{CoTaskMemFree, IStream};

use crate::config::DEVTOOLS_ENABLED;

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

/// 在 Windows 上给主窗口注册 WebView2 原生事件（safe 接口）。
///
/// - 新窗口请求（`target="_blank"` 链接 / JS `window.open()`）：用 `SetNewWindow` 把当前窗口
///   设为新窗口目标，强制在当前窗口打开，且 `window.open()` 返回值有效。
/// - Basic 鉴权（URL 带 userinfo 时）：
///   1) `WebResourceRequested` 拦截【每一个子资源请求】（css/js/xhr/fetch），主动注入 `Authorization`
///      头，这是“始终鉴权”的可靠做法，子资源不会丢凭据；
///   2) `BasicAuthenticationRequested` 兜底，主框架文档（不触发 WebResourceRequested 的硬限制）及
///      任何漏网请求收到 401 时自动应答账号密码。
/// - 右键菜单：隐藏「更多工具」「另存为」「打印」，新增「设置」项。
///
/// 内部必要的 COM FFI 调用用局部 `unsafe` 块包裹，并收敛在此函数里，避免散落到业务代码。
/// 安全性依据：wry 在窗口 build 时已同步等待 WebView2 controller 就绪，`CoreWebView2()` 必返回
/// Ok；事件处理器由 webview2_com 的 `create` 构造为 COM 对象，`add_*` 会正确 AddRef，闭包捕获的
/// core/凭据在 handler 存活期间有效。
pub(crate) fn attach_webview_hooks(
    app: tauri::AppHandle,
    window: &tauri::WebviewWindow,
    auth_user: String,
    auth_pass: String,
    target_url: String,
    use_local_proxy: bool,
) {
    let user = auth_user;
    let pass = auth_pass;

    let _ = window.with_webview(move |wv| {
        // CoreWebView2() 是 unsafe fn，其余（controller 已就绪）为 safe。
        let core = match unsafe { wv.controller().CoreWebView2() } {
            Ok(c) => c,
            Err(_) => return,
        };

        // --- 0) 开发者工具（F12）---
        if DEVTOOLS_ENABLED {
            if let Ok(settings) = unsafe { core.Settings() } {
                unsafe {
                    let _ = settings.SetAreDevToolsEnabled(true);
                }
            }
        }

        // --- 1) 新窗口请求：强制在当前窗口打开 ---
        let core_for_nw = core.clone();
        let nw_handler = NewWindowRequestedEventHandler::create(Box::new(move |_sender, args| {
            if let Some(args) = args {
                unsafe {
                    let _ = args.SetNewWindow(&core_for_nw);
                    let _ = args.SetHandled(true);
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
        let cm_handler = ContextMenuRequestedEventHandler::create(Box::new(
            move |_sender, args| {
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
                                            if n == "moretools" || n == "print" || n.starts_with("save")
                                            {
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
                                        let sel_handler = CustomItemSelectedEventHandler::create(
                                            Box::new(move |_sender, _args| {
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
                                            }),
                                        );
                                        let mut sel_token = 0i64;
                                        let _ = setting_item
                                            .add_CustomItemSelected(&sel_handler, &mut sel_token);
                                        if let Ok(items) = args.MenuItems() {
                                            if let Some(count) = read_menu_count(&items) {
                                                let _ =
                                                    items.InsertValueAtIndex(count, &setting_item);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(())
            },
        ));
        // 只有较新的 WebView2 运行时才暴露 ICoreWebView2_11（ContextMenuRequested 在此接口上）。
        // cast 失败（极老运行时）则跳过右键定制，保留默认菜单，不影响其它钩子。
        if let Ok(core11) = core.cast::<ICoreWebView2_11>() {
            let mut cm_token = 0i64;
            unsafe {
                let _ = core11.add_ContextMenuRequested(&cm_handler, &mut cm_token);
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
            let core13 = match core.cast::<ICoreWebView2_13>() {
                Ok(c) => c,
                Err(_) => return,
            };
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

        // --- 3) 所有钩子注册完成后，发起首次导航（本地反向代理 URL）---
        if !target_url.is_empty() {
            unsafe {
                let _ = core.Navigate(&HSTRING::from(target_url.as_str()));
            }
        }
    });
}
