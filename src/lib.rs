// 允许各平台下未被使用的编译期常量（例如 Android 上不使用的 HOTKEY_* / DEVTOOLS_ENABLED 等），
// 避免出现 dead_code 警告导致 `cargo clippy -D warnings` 在 CI 中失败。
#![allow(dead_code)]

use tauri::{
    DragDropEvent, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
// 全局快捷键相关类型仅在桌面端使用（Android 无该插件依赖，见 Cargo.toml 的平台 gate）。
#[cfg(not(target_os = "android"))]
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, ShortcutState};
use url::Url;

// WebView2 拖拽 polyfill（仅 Windows 注入）：在不关闭原生拖放处理器的前提下，
// 让页面内 HTML5 拖动（[draggable] 元素）恢复。WebView2 的 OLE 拖放会吞掉页面内
// dragover/drop，polyfill 用鼠标事件合成 dragstart/dragover/drop/dragend。
// 参考：https://gist.github.com/iain-fraser/01d35885477f4e29a5a638364040d4f2
const DND_POLYFILL: &str = include_str!("dnd_polyfill.js");

mod proxy;
mod settings;
use settings::{Settings, SettingsState};

// Windows 专属：直接调用 WebView2 原生 API 处理 Basic 鉴权与新窗口请求。
// 注意：Tauri 的 on_web_resource_request 只支持 tauri:// 协议，对外部 https 网址不触发，
// 所以鉴权必须走 WebView2 原生事件。
// 关键限制：WebView2 的 WebResourceRequested / BasicAuthenticationRequested 已确认【不作用于
// WebSocket】（参见 MicrosoftEdge/WebView2Feedback #4303）。Trilium 的实时同步正走的是 WS，
// 因此仅靠原生事件无法解决 WS 401 断开的问题。
// 最终方案（三层兜底，仅 Windows 生效）：
//   1) 本地反向代理（条件启用）：当 PROXY_ENABLED=true 且未配置外部代理(PROXY_HTTP/HTTPS 为空)时，
//      程序启动一个 127.0.0.1:随机端口 的 HTTP/WS 代理，WebView 只访问该代理；代理给【每一个】请求
//      （含 WebSocket 握手）注入 `Authorization: Basic ...` 头，再转发到真实服务器。
//      这是唯一覆盖 WebSocket 的可靠方式（见 PROXY_* 常量与 build_proxy_args / proxy::start_proxy）。
//      若配置了外部代理或 PROXY_ENABLED=false，则不使用本层，由浏览器直连或走 --proxy-server。
//   2) WebResourceRequested（Windows WebView2 原生）：拦截 css/js/xhr/fetch 等子资源，主动
//      注入 Authorization 头；对 localhost 代理请求而言会被第 1 层代理强制覆盖为正确凭据（不会造成
//      重复头），对直接访问上游 URL 的请求（如下载/外链）仍能兜底。
//   3) BasicAuthenticationRequested（Windows WebView2 原生）：最后兜底，任何收到 401 的请求
//      自动应答账号密码。
//
// Android 等移动端：没有 WebView2，也没有系统托盘 / 全局快捷键。移动端统一走【本地反向代理】做鉴权
// （proxy 模块是纯 Rust，零平台依赖，跨平台通用），窗口直接导航到代理地址；托盘与快捷键在移动端被
// cfg 剔除。新窗口（target="_blank"）由 Tauri 移动端自身处理。
#[cfg(windows)]
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2_11, ICoreWebView2_13, ICoreWebView2_2, ICoreWebView2ContextMenuItem,
    ICoreWebView2ContextMenuItemCollection, ICoreWebView2Deferral, ICoreWebView2Environment9,
    COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_COMMAND, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
};
#[cfg(windows)]
use webview2_com::{
    BasicAuthenticationRequestedEventHandler, ContextMenuRequestedEventHandler,
    CustomItemSelectedEventHandler, NewWindowRequestedEventHandler,
    WebResourceRequestedEventHandler,
};
// base64 0.22 的 `.encode()` 是 Engine trait 上的方法，必须显式引入该 trait 才能调用
// （proxy.rs 顶部已 `use base64::Engine;`，main.rs 的 WebView2 钩子里同样需要）。
#[cfg(windows)]
use base64::Engine;
#[cfg(windows)]
use windows::core::{Interface, HSTRING, PWSTR};
#[cfg(windows)]
use windows::Win32::System::Com::{CoTaskMemFree, IStream};
// 注意：windows 0.61（webview2-com 0.38 依赖的版本）已把 Win32 的 EventRegistrationToken
// 重映射为普通 i64，`windows::Win32::System::WinRT::EventRegistrationToken` 路径已不存在，
// 注册事件时 token 直接用整数 0 即可（见下方 add_BasicAuthenticationRequested / add_NewWindowRequested）。

// 托盘 / 菜单（仅桌面端；Android 无系统托盘，相关模块在移动端被编译剔除）。
#[cfg(not(target_os = "android"))]
use tauri::menu::{Menu, MenuItem};
#[cfg(not(target_os = "android"))]
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

// =====================================================================
//  编译时配置（可由环境变量注入；不注入则使用下方默认值）
// ---------------------------------------------------------------------
//  这些常量在编译期读取环境变量（option_env!），因此只需在 `cargo build`
//  前设置对应环境变量、或 CI 的 `env:` 中注入，即可生成不同配置的应用，
//  而无需修改源码。未设置 / 为空时回落到默认值，保证任何环境都能直接编译。
//
//  可注入的变量：
//    APP_URL          要嵌入的网址（支持 https://user:pass@host 形式）
//    APP_NAME         运行期显示名（窗口标题 / 托盘提示 / 设置页标题）
//    WINDOW_TITLE     窗口标题（缺省等于 APP_NAME）
//    DATA_DIR_DEFAULT true=WebView2 系统默认目录 / false=%LOCALAPPDATA%
//    PROXY_ENABLED    true/false 是否启用本地反向代理
//    PROXY_HTTP       外部 HTTP 代理地址（留空=走自身代理）；仅 Windows
//    PROXY_HTTPS      外部 HTTPS 代理地址（留空=同 HTTP）
//    PROXY_BYPASS     不走代理的地址（分号分隔）
//  例：APP_URL="https://u:p@example.com" APP_NAME="MyApp" cargo build --release
// =====================================================================

// const 上下文可用的字符串相等判断：`str` 的 `==` 与 `match` 在常量求值中不可用
// （`PartialEq` 尚未稳定为 const trait，见 issue #143874），这里用字节逐个比较实现。
const fn str_eq(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    if ab.len() != bb.len() {
        return false;
    }
    let mut i = 0;
    while i < ab.len() {
        if ab[i] != bb[i] {
            return false;
        }
        i += 1;
    }
    true
}

// 编译期环境变量注入宏：读环境变量 $name，非空则用，否则用 $default。
macro_rules! env_str {
    ($name:literal, $default:expr) => {{
        match option_env!($name) {
            Some(s) => {
                if s.is_empty() {
                    $default
                } else {
                    s
                }
            }
            None => $default,
        }
    }};
}

// 布尔型注入宏：true/1/yes => true；false/0/no => false；其余用默认值。
// 注意：在 `const XXX: bool = env_bool!(...)` 的常量上下文中，既不能用 `==` / `match` 比较
// `str`（`PartialEq` 非 const trait），也不能用 `&&` / `||`（conditionally-const），都会触发
// E0658。故这里借助上面的 const fn `str_eq` + if-else 链实现（字节级比较，const 安全）。
macro_rules! env_bool {
    ($name:literal, $default:expr) => {{
        match option_env!($name) {
            Some(s) => {
                if str_eq(s, "true") {
                    true
                } else if str_eq(s, "1") {
                    true
                } else if str_eq(s, "yes") {
                    true
                } else if str_eq(s, "false") {
                    false
                } else if str_eq(s, "0") {
                    false
                } else if str_eq(s, "no") {
                    false
                } else {
                    $default
                }
            }
            None => $default,
        }
    }};
}

// 把应用名清洗为合法的目录名（Windows/Linux/macOS 通用）：
//   - 替换路径非法字符 < > : " / \ | ? * 与控制字符为下划线
//   - 压缩连续空白为单个空格、去首尾空白与首尾句点
//   - 清洗后为空则回落 "Web2App"
// 用于让 Windows 的 WebView2 数据目录 / 设置目录以自定义 app_name 命名，
// 而非固定的 tauri.conf.json identifier（com.example.web2app）。
pub(crate) fn sanitize_dir_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_ws = false;
    for c in name.chars() {
        let mapped = match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 32 => '_',
            c => c,
        };
        if mapped.is_whitespace() {
            // 压缩连续空白；首个字符前的空白不输出
            if !prev_ws && !out.is_empty() {
                out.push(' ');
            }
            prev_ws = true;
        } else {
            out.push(mapped);
            prev_ws = false;
        }
    }
    let trimmed = out.trim().trim_matches('.').to_string();
    if trimmed.is_empty() {
        "Web2App".to_string()
    } else {
        trimmed
    }
}

// 默认打开的网址；支持带账号密码的形式：
//   "https://用户名:密码@www.example.com"
// 注意：现代 Chromium / WebView2 已废弃“URL 内嵌 userinfo 自动发 Basic 鉴权”的行为，
// 不会自动发送。程序会把账号密码解析出来，通过本地反向代理给所有 HTTP/WebSocket 请求统一
// 注入 `Authorization: Basic ...` 头，再转发到真实服务器。
const APP_URL: &str = env_str!("APP_URL", "https://用户名:密码@www.example.com");

// 应用名称（用于窗口标题、托盘提示、单实例聚焦等）
const APP_NAME: &str = env_str!("APP_NAME", "Web2App");
// 窗口标题（取自主机名，留空则用下面这个默认标题）
const WINDOW_TITLE: &str = env_str!("WINDOW_TITLE", APP_NAME);

// 额外参数（编译期注入的 JSON 文本，默认 "{}"）。由手动打包 JSON 的 `extra` 字段提供，
// 会以 `window.web2appExtra = {...}` 注入到被包裹网页，前端 JS 可直接读取。
// 必须是合法 JSON（对象/数组/标量皆可），注入时原样内联为 JS 表达式。
const EXTRA: &str = env_str!("EXTRA", "{}");

// ---------- 开发者工具（F12 调试，仅 Windows WebView2 生效）----------
// true  = 启用 WebView2 DevTools，运行后按 F12 即可打开开发者工具，查看 Network / Console 排查问题。
// false = 关闭（发布时建议设为 false）。
// 开发者工具（F12 调试）。发布版（release）自动关闭、调试版（debug）自动开启，无需手动改这个值。
// 之前是固定 `true`，发布包里也开着 DevTools 既不安全也不必要，故改为按构建模式自动判定。
#[cfg(debug_assertions)]
const DEVTOOLS_ENABLED: bool = true;
#[cfg(not(debug_assertions))]
const DEVTOOLS_ENABLED: bool = false;

// ---------- 数据目录（WebView2 用户数据文件夹，存 cookie / localStorage / 缓存等，仅 Windows）----------
// true  = 使用 WebView2 系统默认位置（{exe 名}.WebView2，紧挨 exe 文件）。
//         注意：打包安装（msi/nsis）后 exe 通常在 Program Files，标准用户对该目录无写权限，
//         此时 WebView2 初始化会失败、窗口直接打不开。故【安装版强烈建议设为 false】。
// false = 固定到“用户本地数据目录 / 程序名”（%LOCALAPPDATA%\com.example.web2app），
//         始终可写，适合安装版与多用户场景（推荐）。
const DATA_DIR_DEFAULT: bool = env_bool!("DATA_DIR_DEFAULT", false);

// ---------- 代理配置 ----------
// 不需要代理时把 PROXY_ENABLED 设为 false 即可。
const PROXY_ENABLED: bool = env_bool!("PROXY_ENABLED", false);
// 【重要】自实现本地反向代理（HTTP+WS 统一注入 Basic 鉴权，覆盖 WebSocket）的启用条件是：
//   PROXY_ENABLED = true 且 PROXY_HTTP / PROXY_HTTPS 均为空 ""。
// 因此下面两个必须保持空串作为默认；只有你【确实要用外部 HTTP 代理】时才填。
// 不要像早期版本那样把这里写成 "http://...@127.0.0.1:7890" 占位值，否则 PROXY_ENABLED=true
// 时会错误地走进“外部代理”分支（根本不会启动本地代理，并把流量导到不存在的 127.0.0.1:7890 → 401）。
const PROXY_HTTP: &str = env_str!("PROXY_HTTP", "");
// HTTPS 代理地址；留空 "" 表示和 HTTP 代理用同一个（同样：留空即走自实现代理）。
const PROXY_HTTPS: &str = env_str!("PROXY_HTTPS", "");
// 不走代理的地址（分号分隔），可留空 ""
const PROXY_BYPASS: &str = env_str!("PROXY_BYPASS", "localhost;127.0.0.1;<-loopback>");

// ---------- 全局快捷键（按下切换 显示/隐藏 窗口，仅桌面端）----------
// 用下面三个开关组合修饰键，再指定一个主键。
#[cfg(not(target_os = "android"))]
const HOTKEY_CTRL: bool = false;
#[cfg(not(target_os = "android"))]
const HOTKEY_ALT: bool = true;
#[cfg(not(target_os = "android"))]
const HOTKEY_SHIFT: bool = false;
#[cfg(not(target_os = "android"))]
const HOTKEY_KEY: Code = Code::Digit4; // 数字键 4，配合 Alt → Alt+4；其它键如 Code::KeyS / Code::KeyQ ...

// =====================================================================

/// 根据编译时写死的常量生成 Chromium 代理启动参数；未启用则返回 None（仅 Windows WebView2 使用）。
#[cfg(windows)]
fn build_proxy_args() -> Option<String> {
    if !PROXY_ENABLED {
        return None;
    }
    let mut http = if PROXY_HTTP.trim().is_empty() {
        None
    } else {
        Some(PROXY_HTTP.trim().to_string())
    };
    let mut https = if PROXY_HTTPS.trim().is_empty() {
        None
    } else {
        Some(PROXY_HTTPS.trim().to_string())
    };
    // 只填了一个时，另一个也用同一个代理
    if http.is_some() && https.is_none() {
        https = http.clone();
    }
    if https.is_some() && http.is_none() {
        http = https.clone();
    }
    let mut servers = Vec::new();
    if let Some(h) = &http {
        servers.push(format!("http={}", h));
    }
    if let Some(h) = &https {
        servers.push(format!("https={}", h));
    }
    if servers.is_empty() {
        return None;
    }
    let mut args = format!("--proxy-server={}", servers.join(";"));
    if !PROXY_BYPASS.trim().is_empty() {
        args.push_str(&format!(" --proxy-bypass-list={}", PROXY_BYPASS.trim()));
    }
    Some(args)
}

/// 去掉 URL 中的 userinfo（用户名:密码），返回“干净”地址。
/// 用于直连 / 外部代理场景下 WebView 的导航地址（鉴权由 WebView2 原生事件或本地代理注入，
/// 不应把密码写进浏览器地址栏 / 交给外部代理字符串）。
fn strip_userinfo(url: &Url) -> String {
    let mut u = url.clone();
    let _ = u.set_username("");
    let _ = u.set_password(None);
    u.to_string()
}

/// 切换主窗口：正常可见则隐藏；最小化或已隐藏则恢复并显示置顶（仅桌面端）。
#[cfg(not(target_os = "android"))]
fn toggle_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let visible = window.is_visible().unwrap_or(false);
        let minimized = window.is_minimized().unwrap_or(false);
        if visible && !minimized {
            let _ = window.hide();
        } else {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

/// 读取右键菜单项集合的项数（COM 老式 out 参数封装）。
#[cfg(windows)]
fn read_menu_count(col: &ICoreWebView2ContextMenuItemCollection) -> Option<u32> {
    unsafe {
        let mut n: u32 = 0;
        col.Count(&mut n).ok()?;
        Some(n)
    }
}

/// 读取右键菜单项的【非本地化 Name】（英文小驼峰，如 "saveAs"/"print"/"moreTools"）。
/// WebView2 用 CoTaskMemAlloc 分配该字符串，读完用 CoTaskMemFree 释放，避免内存泄漏。
#[cfg(windows)]
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
///
///   注：WebSocket 握手已由本地反向代理（`src/proxy.rs`）统一注入 Authorization 头，这两层原生
///   事件主要作为外链/直接访问上游 URL 时的兜底。
/// - 所有钩子注册完成后，用 `target_url`（本地反向代理 URL）发起首次导航，避免首批请求
///   漏掉鉴权头（确定性）。真实服务器地址与 Basic 凭据由本地代理（`src/proxy.rs`）负责转发。
///
/// 内部必要的 COM FFI 调用用局部 `unsafe` 块包裹，并收敛在此函数里，避免散落到业务代码。
/// 安全性依据：wry 在窗口 build 时已同步等待 WebView2 controller 就绪，`CoreWebView2()` 必返回
/// Ok；事件处理器由 webview2_com 的 `create` 构造为 COM 对象，`add_*` 会正确 AddRef，闭包捕获的
/// core/凭据在 handler 存活期间有效。
#[cfg(windows)]
fn attach_webview_hooks(
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
        // NewWindowRequested 是 ICoreWebView2（最基础接口）的事件，所有 WebView2 版本都支持。
        // 用 SetNewWindow 把当前窗口设为新窗口目标：新窗口导航自动落在当前窗口，
        // 且 window.open() 返回值有效（避免页面 JS 拿到 null 报错）。
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

        // --- 1b) 右键菜单定制（Windows WebView2 原生 ContextMenuRequested）---
        // 需求：隐藏默认菜单里的「更多工具」「另存为」「打印」，新增一个「设置」项
        // （点击后打开与 Shift+F1 相同的设置界面）。
        // 按【非本地化 Name】（英文小驼峰，与系统语言无关）识别要移除的项，避免依赖中文标签：
        //   "moreTools"     更多工具（子菜单）
        //   "save*"         另存为（saveAs 页面 / saveLinkAs 链接 / saveImageAs 图片 / saveMediaAs 媒体）
        //   "print"         打印
        // 新增项用 Environment.CreateContextMenuItem 创建 + CustomItemSelected 事件响应，
        // 再插入 args.MenuItems() 集合末尾。
        // 注意：右键回调在线程/生命周期上都独立于本注册闭包，故把要用到的 core/app 各自 clone 一份捕获。
        let app_for_menu = app.clone();
        let core_for_menu = core.clone(); // base ICoreWebView2，回调里再 cast 到 _2 取 Environment
        let cm_handler = ContextMenuRequestedEventHandler::create(Box::new(
            move |_sender, args| {
                if let Some(args) = args {
                    unsafe {
                        // 1) 移除默认项：倒序遍历，命中则 RemoveValueAtIndex（保证索引有效）
                        if let Ok(items) = args.MenuItems() {
                            if let Some(count) = read_menu_count(&items) {
                                let mut i = count;
                                while i > 0 {
                                    i -= 1;
                                    if let Ok(item) = items.GetValueAtIndex(i) {
                                        if let Some(name) = read_menu_name(&item) {
                                            let n = name.to_lowercase();
                                            // "moreTools" 更多工具 / "print" 打印 / "save*" 各类"另存为"
                                            if n == "moretools" || n == "print" || n.starts_with("save")
                                            {
                                                let _ = items.RemoveValueAtIndex(i);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // 2) 追加自定义「设置」菜单项（点击后打开与 Shift+F1 相同的设置界面）
                        // 需先经 ICoreWebView2_2 取 Environment，再 cast 到 ICoreWebView2Environment9
                        // 调用 CreateContextMenuItem（该 winmd 在 Environment9 上声明此方法），
                        // 当前 WebView2 运行时（evergreen）均支持。
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
                                                // 会【同步内联】执行闭包（见 tauri-runtime-wry::send_user_message，
                                                // current_thread()==main_thread 分支直接 handle_user_message）。
                                                // 此时我们正嵌套在 WebView2 自身的 COM 事件分发里，同步再 build 一个
                                                // 新 WebView2 控制器（settings 窗）需要 COM STA 重入，无法在此上下文
                                                // 完成 → 整个进程卡死（右键"设置"弹出后无响应）。
                                                // 解法：先切到一个后台线程，让 run_on_main_thread 从【非主线程】走
                                                // event_loop proxy 排队，等 COM 分发完全退栈后的【下一个事件循环轮次】
                                                // 才在主线程建窗（等价于 Shift+F1 的调用上下文，安全）。
                                                let app_spawn = ah.clone();
                                                let _ = std::thread::spawn(move || {
                                                    // 后台线程上调用：与主线程 id 不同 → 走 proxy 排队，
                                                    // 在下一次事件循环轮次于主线程执行闭包。
                                                    let app_main = app_spawn.clone();
                                                    let _ = app_spawn.run_on_main_thread(move || {
                                                        settings::open_settings_window(&app_main);
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
        // 若已启用本地反向代理，所有请求的 Authorization 都由 proxy.rs 在转发时统一注入，
        // 此处再注册 WebResourceRequested 过滤器会让【每个】请求都经过一次 COM 回调，纯属多余开销，
        // 故直接跳过（下方的新窗口拦截与鉴权无关，仍然保留）。
        if !user.is_empty() && !use_local_proxy {
            // 2a) 主动给【所有子资源请求】注入 Authorization 头（css/js/xhr/fetch 等）。
            //     注意：顶层主框架文档导航不会触发 WebResourceRequested（WebView2 硬限制），
            //     主文档由下方 2b 的 BasicAuthenticationRequested 兜底。子资源靠这里提前注入，
            //     不依赖服务器先回 401，从根本上避免“主页面 200 打开、但子资源 401 导致内容空白”。
            //     （所有钩子注册完成后才 Navigate，见步骤 3，确保首批子资源也覆盖到。）
            let creds = format!("{}:{}", user, pass);
            let b64 = base64::engine::general_purpose::STANDARD.encode(creds.as_bytes());
            let header_val = format!("Basic {}", b64);

            unsafe {
                // 过滤器：匹配所有 URL、所有资源类型（含主框架文档与子资源）。
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
                                // 取请求头集合，就地 SetHeader，WebView2 会用修改后的请求继续发起网络请求。
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

            // 2b) 兜底：任何收到 401 的请求（含主框架文档、子资源）自动应答账号密码。采用 WebView2
            //     文档推荐的 Deferral 规范写法：取 Deferral → 写入凭据 → 显式 SetCancel(false)（不取消，
            //     让 WebView2 用凭据重试）→ Complete()。这样可排除“闭包同步返回导致凭据未被采纳”
            //     的时序问题。
            //     注意：WebSocket 握手已由本地反向代理统一注入 Authorization，本事件主要作为最后兜底。
            let core13 = match core.cast::<ICoreWebView2_13>() {
                Ok(c) => c,
                Err(_) => return,
            };
            let handler =
                BasicAuthenticationRequestedEventHandler::create(Box::new(move |_sender, args| {
                    if let Some(args) = args {
                        unsafe {
                            // Deferral：声明“稍后完成”，确保凭据在事件结束前写入。
                            let deferral: Option<ICoreWebView2Deferral> = args.GetDeferral().ok();
                            if let Ok(response) = args.Response() {
                                let _ = response.SetUserName(&HSTRING::from(user.as_str()));
                                let _ = response.SetPassword(&HSTRING::from(pass.as_str()));
                            }
                            // 关键：不取消挑战，WebView2 会用提供的凭据重试该请求。
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
        // 此时 WebResourceRequested 过滤器与 Basic 鉴权事件均已生效，首个文档请求及全部子资源
        // 都会正确携带凭据，不会出现“主文档 200、但子资源 401 导致空白”的竞态。
        if !target_url.is_empty() {
            unsafe {
                let _ = core.Navigate(&HSTRING::from(target_url.as_str()));
            }
        }
    });
}

/// 托管本地反向代理任务的 AbortHandle，便于应用退出时优雅终止它（释放 127.0.0.1 监听端口）。
struct ProxyTask(std::sync::Mutex<Option<tokio::task::AbortHandle>>);

impl Default for ProxyTask {
    fn default() -> Self {
        ProxyTask(std::sync::Mutex::new(None))
    }
}

// =====================================================================
//  窗口几何持久化（仅桌面端）：记住上次窗口的大小/位置/最大化状态，
//  下次启动自动恢复。数据目录与 settings 一致（<app_config>/<app_name>/），
//  独立文件 window-state.json，不与“设置”混在一起（窗口几何随移动频繁变）。
// =====================================================================
#[cfg(not(target_os = "android"))]
mod window_state {
    use serde::{Deserialize, Serialize};
    use std::fs;
    use std::path::PathBuf;
    use tauri::{AppHandle, Manager, WebviewWindow};

    use crate::sanitize_dir_name;
    use crate::APP_NAME;

    const FILE: &str = "window-state.json";

    /// 窗口几何快照。x/y = 窗口左上角在屏幕上的坐标（outer），
    /// width/height = 窗口外框尺寸（outer），均为【物理像素】，
    /// 以保证保存与 set_position/set_size 一致、跨会话不因 DPI 缩放而错位。
    #[derive(Serialize, Deserialize, Clone, Copy)]
    pub struct State {
        pub x: i32,
        pub y: i32,
        pub width: u32,
        pub height: u32,
        pub maximized: bool,
    }

    fn path(app: &AppHandle) -> Option<PathBuf> {
        // 目录与 settings_path 保持一致：桌面端 app_config_dir 末段是 identifier，
        // 取父目录后再拼上清洗后的自定义 app_name（不同 app 配置互相隔离）。
        let base = app.path().app_config_dir().ok()?;
        let root = base.parent().unwrap_or(&base).to_path_buf();
        Some(root.join(sanitize_dir_name(APP_NAME)).join(FILE))
    }

    /// 读取上次保存的几何；文件不存在 / 解析失败返回 None（用默认窗口尺寸）。
    pub fn load(app: &AppHandle) -> Option<State> {
        let p = path(app)?;
        let s = fs::read_to_string(&p).ok()?;
        serde_json::from_str::<State>(&s).ok()
    }

    /// 写入窗口几何。
    pub fn save(app: &AppHandle, st: &State) {
        let Some(p) = path(app) else { return; };
        if let Some(parent) = p.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(st) {
            let _ = fs::write(&p, json);
        }
    }

    /// 读取主窗口当前几何（若主窗口存在）。
    pub fn persist_now(app: &AppHandle) {
        let Some(win) = app.get_webview_window("main") else { return; };
        if let Some(st) = capture(&win) {
            save(app, &st);
        }
    }

    /// 从给定窗口抓取当前几何。
    fn capture(win: &WebviewWindow) -> Option<State> {
        let pos = win.outer_position().ok()?;
        let size = win.outer_size().ok()?;
        let maximized = win.is_maximized().unwrap_or(false);
        Some(State {
            x: pos.x,
            y: pos.y,
            width: size.width,
            height: size.height,
            maximized,
        })
    }

    /// 创建窗口后应用已保存的几何；保存的几何若已失效（上次最大化、尺寸过小、
    /// 或窗口中心落在屏外——显示器被拔掉/分辨率/DPI 变化所致）则**丢弃几何并直接
    /// 最大化**，保证窗口始终可见可用。无保存状态时不做任何事（保留窗口默认尺寸）。
    pub fn apply(win: &WebviewWindow, st: &State) {
        // 1) 上次是最大化 → 直接最大化，必然铺满且可见。
        if st.maximized {
            let _ = win.maximize();
            return;
        }

        // 2) 校验保存的位置/尺寸是否还“够用”：
        //    - 窗口中心点不在任何当前显示器上 → 窗口被留在屏外，用户看不到（且重开依旧，
        //      因为每次都恢复同样的屏外坐标）。此时丢弃几何、最大化兜底。
        //    - 尺寸小于最小窗口约束(创建时 min_inner_size=400x300，此处 outer 略留余量)
        //      → 视为损坏几何，最大化兜底。
        let center_on_screen = win
            .monitor_from_point(
                (st.x as f64) + (st.width as f64) / 2.0,
                (st.y as f64) + (st.height as f64) / 2.0,
            )
            .ok()
            .flatten()
            .is_some();
        let too_small = st.width < 420 || st.height < 320;
        if !center_on_screen || too_small {
            let _ = win.maximize();
            return;
        }

        // 3) 几何有效：恢复位置与尺寸。
        let _ = win.set_position(tauri::PhysicalPosition::new(st.x, st.y));
        let _ = win.set_size(tauri::PhysicalSize::new(st.width, st.height));
    }
}

// 移动端（Android/iOS）入口：tauri::mobile_entry_point 会生成 JNI/extern 绑定，
// 供移动端原生壳调用；桌面端该属性为空操作，等价于一个普通 pub fn。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 桌面端注册全局快捷键插件；移动端没有该能力，跳过。
    let builder = tauri::Builder::default()
        .register_uri_scheme_protocol("settings", |_ctx, _req| {
            // 编译期把 dist/settings.html 内嵌，经自定义协议提供设置页。
            // 这样不依赖 devUrl（tauri dev 会把 App 相对路径代理到 devUrl，
            // 导致设置页被解析到 example.com），dev / release 行为完全一致。
            let html = include_str!("../dist/settings.html");
            tauri::http::Response::builder()
                .header("Content-Type", "text/html; charset=utf-8")
                .body(html.as_bytes().to_vec())
                .unwrap()
        })
        .manage(ProxyTask::default())
        .manage(SettingsState::default())
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            close_settings_window
        ]);

    #[cfg(not(target_os = "android"))]
    let settings_open = settings::settings_open_shortcut();
    #[cfg(not(target_os = "android"))]
    let builder = builder.plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(move |app, shortcut, event| {
                // 只在“按下”那一刻触发一次。若忽略 event.state，按键的「按下」和「松开」
                // 会各自触发一次回调 → toggle 两次相互抵消，表现就是“按了没反应 / 乱跳”。
                if event.state != ShortcutState::Pressed {
                    return;
                }
                // 打开设置界面：Shift+F1（固定不可改）
                if shortcut.to_string() == settings_open.to_string() {
                    settings::open_settings_window(app);
                    return;
                }
                // 唤醒主窗口：使用当前设置中的快捷键
                let hk = app
                    .state::<SettingsState>()
                    .0
                    .lock()
                    .unwrap()
                    .wake_hotkey
                    .clone();
                if let Some(sc) = settings::hotkey_to_shortcut(&hk) {
                    if shortcut.to_string() == sc.to_string() {
                        toggle_window(app);
                    }
                }
            })
            .build(),
    );

    // 单实例：第二个实例启动时不再新开窗口，而是把已存在的窗口带到前台。
    // （仅桌面端；Android 无此概念，故按 target_os 剔除。）
    #[cfg(not(target_os = "android"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
        }
    }));

    let builder = builder.setup(move |app| {
        // 载入持久化设置（文件不存在时使用默认值），写入托管状态供运行时读取。
        let initial_settings = settings::load_settings(app.handle());
        *app.state::<SettingsState>().0.lock().unwrap() = initial_settings.clone();

        // 解析默认网址，拆出账号密码（用于 Basic 鉴权）。
        let parsed = Url::parse(APP_URL).expect("APP_URL 不是合法网址");

        // 去掉 userinfo 的“干净”上游地址，用于【直连 / 外部代理】两种场景的导航 URL
        // （鉴权交给 WebView2 原生事件兜底，见 attach_webview_hooks）。
        let clean_url = strip_userinfo(&parsed);

        // ============ 代理模式决策（由 PROXY_ENABLED / PROXY_HTTP / PROXY_HTTPS 共同决定）============
        //  PROXY_ENABLED = false
        //      → 不启用任何代理，WebView 直接导航到真实上游地址。
        //  PROXY_ENABLED = true 且 PROXY_HTTP/HTTPS 非空（仅 Windows --proxy-server 支持）
        //      → 使用【外部代理】，WebView 直接导航到真实上游地址。
        //  PROXY_ENABLED = true 且 PROXY_HTTP/HTTPS 为空（含所有移动端）
        //      → 启用【自身实现的本地反向代理】（HTTP + WebSocket 统一注入 Authorization，
        //        这是覆盖 WebSocket 的唯一可靠方式），WebView 导航到 127.0.0.1:随机端口。
        //  说明：Android 等移动端不支持 --proxy-server，故移动端“启用代理”一律走本地反向代理。
        // ========================================================================================
        #[cfg(windows)]
        let browser_args: Option<String> = if PROXY_ENABLED {
            build_proxy_args()
        } else {
            None
        };

        // 是否启用本地反向代理：Windows 上需“启用代理且无外部代理配置”；移动端只要“启用代理”即可。
        #[cfg(windows)]
        let use_local_proxy = PROXY_ENABLED && browser_args.is_none();
        #[cfg(not(windows))]
        let use_local_proxy = PROXY_ENABLED;

        let mut proxy_abort: Option<tokio::task::AbortHandle> = None;
        let target_url: String = if use_local_proxy {
            // 自身实现的本地反向代理：给每个请求（含 WebSocket 握手）注入 Authorization。
            let (url, abort) = tauri::async_runtime::block_on(proxy::start_proxy(&parsed))?;
            proxy_abort = Some(abort);
            url
        } else {
            // 直连 / 外部代理：直接导航到去掉 userinfo 的上游地址（鉴权由原生事件或代理注入）。
            clean_url
        };

        // 把代理任务的 AbortHandle 存入托管状态，供退出时优雅终止。
        if let Some(abort) = proxy_abort {
            app.state::<ProxyTask>().0.lock().unwrap().replace(abort);
        }

        // ---------- 创建主窗口 ----------
        // Windows：先用 about:blank 建窗，等所有 WebView2 钩子注册完成后再手动 Navigate 到目标地址，
        //          避免窗口 build 时抢先发出的首批请求漏掉 Authorization 注入（确定性，消除竞态）。
        // 其他平台（Android 等）：没有 WebView2，直接把窗口导航到目标地址（本地代理或上游）。
        #[cfg(windows)]
        let window = {
            let mut b = WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::External(Url::parse("about:blank").unwrap()),
            )
            .title(WINDOW_TITLE)
            .resizable(true)
            .inner_size(1100.0, 650.0)
            .min_inner_size(400.0, 300.0)
            // 注入 WebView2 拖拽 polyfill：保留原生拖放处理器（Rust 仍能拿到外部拖入文件
            // 路径，见下方 RunEvent::WindowEvent 的 DragDrop 分支），同时让页面内 HTML5 拖动
            // 恢复（WebView2 的 OLE 拖放会吞掉页面内 dragover/drop，polyfill 用鼠标事件合成
            // dragstart/dragover/drop/dragend）。initialization_script 会在每次导航（含初始
            // 文档）前注入，详见文件顶部 DND_POLYFILL 注释。
            .initialization_script(DND_POLYFILL)
            // 注入打包配置 JSON 里的 `extra`：以 `window.web2appExtra = <JSON>;` 定义在页面
            // 全局，前端 JS 在脚本执行时直接读取（在每次文档加载前注入，故早于页面业务脚本）。
            // EXTRA 来自合法 JSON（config.example 的 extra 对象），内联即为合法 JS 表达式。
            .initialization_script(format!("window.web2appExtra = {};", EXTRA));

            if let Some(proxy_args) = &browser_args {
                b = b.additional_browser_args(proxy_args.as_str());
            }
            // 数据目录：true = WebView2 系统默认（紧挨 exe，安装版可能因无写权限而失败）；
            // false = %LOCALAPPDATA%\{自定义 app_name}（始终可写，推荐）。
            // Tauri 默认按 tauri.conf.json 的 identifier（com.example.web2app）生成目录末段，
            // 这里改用清洗后的自定义 app_name，使不同 app 的存储目录互不干扰且直观。
            // 用 if-let 而非 `?`：app_local_data_dir 在极少数环境可能返回 Err，直接 `?` 会让整个
            // 启动失败崩溃；拿不到就退回到 WebView2 默认位置，保证能启动。
            if !DATA_DIR_DEFAULT {
                if let Ok(dir) = app.path().app_local_data_dir() {
                    // dir 形如 %LOCALAPPDATA%/com.example.web2app，用自定义 app_name 替换末段
                    let base = dir.parent().unwrap_or(&dir);
                    let new_dir = base.join(sanitize_dir_name(APP_NAME));
                    b = b.data_directory(new_dir);
                }
            }

            let w = b.build().expect("创建主窗口失败");

            // 应用上次保存的窗口大小/位置（若有）。仅桌面端。
            #[cfg(not(target_os = "android"))]
            if let Some(st) = window_state::load(app.handle()) {
                window_state::apply(&w, &st);
            }

            // 注册 WebView2 原生事件（新窗口拦截 + Basic 鉴权），函数内部会导航到本地代理 URL。
            let auth_user = parsed.username().to_string();
            let auth_pass = parsed.password().unwrap_or("").to_string();
            attach_webview_hooks(
                app.handle().clone(),
                &w,
                auth_user,
                auth_pass,
                target_url,
                use_local_proxy,
            );
            w
        };

        #[cfg(not(windows))]
        let window = WebviewWindowBuilder::new(
            app,
            "main",
            WebviewUrl::External(Url::parse(&target_url).unwrap()),
        )
        .title(WINDOW_TITLE)
        .resizable(true)
        .inner_size(1200.0, 800.0)
        .min_inner_size(400.0, 300.0)
        .build()
        .expect("创建主窗口失败");

        let _ = &window; // 部分平台窗口创建后即无需再引用，抑制未使用告警

        // 应用上次保存的窗口大小/位置（桌面端非 Windows：Linux/macOS；Windows 已在上面块内应用）。
        #[cfg(all(not(windows), not(target_os = "android")))]
        if let Some(st) = window_state::load(app.handle()) {
            window_state::apply(&window, &st);
        }

        // ---------- 系统托盘与全局快捷键（仅桌面端；Android 无）----------
        #[cfg(not(target_os = "android"))]
        {
            // 托盘图标：仅当设置中“启用托盘”时创建
            if initial_settings.tray_enabled {
                create_tray(app.handle())?;
            }

            // 注册全局快捷键：① 唤醒主窗口（取自设置） ② 打开设置界面（固定 Shift+F1）
            if let Some(sc) = settings::hotkey_to_shortcut(&initial_settings.wake_hotkey) {
                app.global_shortcut().register(sc)?;
            }
            app.global_shortcut()
                .register(settings::settings_open_shortcut())?;
        }

        Ok(())
    });

    builder
        .build(tauri::generate_context!())
        .expect("启动 Tauri 应用失败")
        .run(|app_handle, event| {
            match event {
                // 点击窗口关闭按钮：仅对主窗口生效。
                // 若“启用托盘 + 关闭时最小化到托盘”同时满足 → 隐藏到托盘；
                // 否则允许关闭（最后一个窗口关闭后应用退出）。
                RunEvent::WindowEvent {
                    label,
                    event: WindowEvent::CloseRequested { api, .. },
                    ..
                } => {
                    if label != "main" {
                        return;
                    }
                    // 记录主窗口当前大小/位置，供下次启动恢复（仅桌面端）。
                    #[cfg(not(target_os = "android"))]
                    window_state::persist_now(app_handle);
                    let st = app_handle.state::<SettingsState>();
                    let (tray, min) = {
                        let s = st.0.lock().unwrap();
                        (s.tray_enabled, s.minimize_on_close)
                    };
                    if tray && min {
                        api.prevent_close();
                        if let Some(w) = app_handle.get_webview_window("main") {
                            let _ = w.hide();
                        }
                    }
                }
                // 外部文件拖入：原生拖放处理器（dragDropEnabled 默认 true）捕获路径，转发给
                // 被包裹的网页。页面监听 window 上的 DOM 事件即可拿到路径列表：
                //   window.addEventListener('web2app:files-dropped',
                //       (e) => { const paths = e.detail.paths; /* 绝对路径数组 */ });
                // 注：页面内 HTML5 拖动由上面注入的 polyfill 处理，不走此分支；此分支只负责
                // 把【来自操作系统】拖入的文件路径交给 Rust，再由 Rust 回传网页。
                RunEvent::WindowEvent {
                    label,
                    event: WindowEvent::DragDrop(drag_drop_event),
                    ..
                } => {
                    if label != "main" {
                        return;
                    }
                    if let DragDropEvent::Drop { paths, .. } = drag_drop_event {
                        let detail = serde_json::json!({
                            "paths": paths
                                .iter()
                                .map(|p| p.to_string_lossy().to_string())
                                .collect::<Vec<String>>()
                        });
                        let js = format!(
                            "window.dispatchEvent(new CustomEvent('web2app:files-dropped', {{ detail: {} }}));",
                            detail
                        );
                        if let Some(w) = app_handle.get_webview_window("main") {
                            let _ = w.eval(&js);
                        }
                    }
                }
                // 应用即将退出：记录窗口几何（兜底，覆盖“托盘退出”等不经过关窗的路径），
                // 优雅终止本地反向代理任务，释放 127.0.0.1 监听端口。
                RunEvent::ExitRequested { .. } => {
                    #[cfg(not(target_os = "android"))]
                    window_state::persist_now(app_handle);
                    if let Some(abort) = app_handle
                        .state::<ProxyTask>()
                        .0
                        .lock()
                        .unwrap()
                        .take()
                    {
                        abort.abort();
                    }
                }
                _ => {}
            }
        });
}

// =====================================================================
//  设置界面 / 设置命令（桌面端）
// =====================================================================

/// 创建系统托盘图标（含右键菜单、左键切换窗口）。仅当设置“启用托盘”时调用。
#[cfg(not(target_os = "android"))]
fn create_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let show_i = MenuItem::with_id(app, "show", "显示窗口", true, None::<&str>)?;
    let hide_i = MenuItem::with_id(app, "hide", "隐藏到托盘", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_i, &hide_i, &quit_i])?;

    TrayIconBuilder::with_id("main-tray")
        .icon(
            app.default_window_icon()
                .cloned()
                .expect("缺少默认图标，请检查 bundle.icon 配置"),
        )
        .menu(&menu)
        .show_menu_on_left_click(false)
        // 悬停提示文本用"显示名"title（即 WINDOW_TITLE，CI 里由配置 JSON 的 title 导出），
        // 而非"安装名"APP_NAME，与窗口标题保持一致。
        .tooltip(WINDOW_TITLE)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.unminimize();
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "hide" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // 左键单击托盘图标 = 显示/隐藏
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

#[tauri::command]
fn get_settings(state: State<SettingsState>) -> Settings {
    state.0.lock().unwrap().clone()
}

#[tauri::command]
fn save_settings(
    app: tauri::AppHandle,
    state: State<SettingsState>,
    new_settings: Settings,
) -> Result<(), String> {
    // 1)+2) 桌面端：唤醒快捷键可留空（表示“不设置快捷键”）；若填写则必须可被解析，
    // 且不能与“打开设置(Shift+F1)”冲突。Android 无全局快捷键，跳过此校验
    // （也不引用 tauri_plugin_global_shortcut 的类型）。
    #[cfg(not(target_os = "android"))]
    let new_sc = {
        let key = new_settings.wake_hotkey.key.trim();
        if key.is_empty() {
            // 空 key = 未启用唤醒快捷键
            None
        } else {
            let sc = settings::hotkey_to_shortcut(&new_settings.wake_hotkey).ok_or_else(|| {
                format!(
                    "不支持的按键：{}，请换一个按键",
                    new_settings.wake_hotkey.key
                )
            })?;
            if sc == settings::settings_open_shortcut() {
                return Err("唤醒快捷键不能与“打开设置(Shift+F1)”冲突，请换一个".into());
            }
            Some(sc)
        }
    };

    let mut guard = state.0.lock().unwrap();
    let old = guard.clone();

    // 3) 桌面端：应用运行时的快捷键 / 托盘变更
    #[cfg(not(target_os = "android"))]
    {
        let old_sc = settings::hotkey_to_shortcut(&old.wake_hotkey);
        if new_sc != old_sc {
            if let Some(os) = old_sc {
                let _ = app.global_shortcut().unregister(os);
            }
            if let Some(ns) = new_sc {
                app.global_shortcut()
                    .register(ns)
                    .map_err(|e| e.to_string())?;
            }
        }
        if new_settings.tray_enabled != old.tray_enabled {
            if new_settings.tray_enabled {
                create_tray(&app).map_err(|e| e.to_string())?;
            } else {
                app.remove_tray_by_id("main-tray");
            }
        }
    }

    // 4) 持久化 + 更新托管状态
    settings::save_settings(&app, &new_settings).map_err(|e| e.to_string())?;
    *guard = new_settings;
    Ok(())
}

#[tauri::command]
fn close_settings_window(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.close();
    }
}
