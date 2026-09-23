//! web2app 桌面/移动端 WebView 壳（Rust + Tauri 2）。
//!
//! 核心职责：嵌入指定网址、系统托盘最小化、全局快捷键切换显隐、HTTP(S) 代理与
//! `https://user:pass@host` Basic 鉴权（含 WebSocket）、动态域名解析（DNS TXT + Ed25519 + AES-GCM）。
//!
//! 模块划分：
//! - [`config`]        编译期常量与 env 注入宏
//! - [`settings`]      应用设置（持久化 + 设置窗口）
//! - [`loading`]       启动过渡（本地启动页 + 注入式加载层）
//! - [`proxy`]         本地反向代理（HTTP + WebSocket 统一注入 Basic 鉴权）
//! - [`resolve`]       DNS TXT 查询 + 验签解密的解析层
//! - [`bootstrap`]     后台准备真实地址 → 就绪后导航
//! - [`webview_hooks`] Windows WebView2 原生事件钩子（鉴权/新窗口/下载/右键菜单）
//! - [`save_dialog`]   Windows 原生「另存为」对话框（下载落盘位置由用户决定）
//! - [`tray`]          系统托盘 + 设置命令 + 窗口切换
//! - [`window_state`]  窗口几何持久化（仅桌面端）
//! - [`debug_log`]     诊断日志（默认关闭；`dlog!` 宏 + 启动信息落盘）

// 允许各平台下未被使用的编译期常量（例如 Android 上不使用的 DEVTOOLS_ENABLED 等），
// 避免出现 dead_code 警告导致 `cargo clippy -D warnings` 在 CI 中失败。
#![allow(dead_code)]

use tauri::{Manager, RunEvent, WindowEvent};

// 全局快捷键插件（仅桌面端；Android 无该依赖，见 Cargo.toml 平台 gate）。
#[cfg(desktop)]
use tauri_plugin_global_shortcut::GlobalShortcutExt;

// 事件监听（仅桌面端使用：见 setup 里的「打开设置」事件）。
// ⚠️ 与使用点的 cfg 保持一致 —— 无条件 `use` 在 Android 上就是「未使用导入」，
//    在 `-D warnings` 下直接编译失败。
#[cfg(desktop)]
use tauri::Listener;

/// 诊断日志宏：`eprintln!` 的同形替代品，**只在 `debug` 打开时**输出。
///
/// ⚠️ 诊断一律走这个宏而不是 `eprintln!`：Windows 上本程序是 GUI 子系统
/// （`main.rs` 的 `windows_subsystem = "windows"`），stderr 没有去处；而默认
/// （`config.json` 的 `debug=false`）**不允许**输出任何日志 —— 落盘与 stderr 都归它管，
/// 详见 [`debug_log`] 模块文档。
/// ⚠️ 用法与 `eprintln!` 一致（首参是格式字符串字面量）。关闭时连 `format!` 都不执行。
/// ⚠️⚠️ 宏必须定义在下面的 `mod` 声明**之前**（靠 `macro_rules!` 的文本作用域）；
///    挪到后面会让各模块里的 `dlog!` 全部找不到 —— 那是编译错误，不会静默降级。
macro_rules! dlog {
    ($($arg:tt)*) => {{
        if crate::debug_log::enabled() {
            crate::debug_log::log_line(&format!($($arg)*));
        }
    }};
}

// 诊断日志（默认关闭）：开启时 `dlog!` 同时写 stderr 与 `<数据目录>\logs\startup.log`。
mod debug_log;

pub mod config;
mod proxy;
pub mod resolve;
mod settings;

// 运行期配置文档：`settings.json` —— 设置 + 窗口几何**同住这一个文件**。
// 读-改-写工具与文件契约（键序 / 兼容性）都在这里，两端（Tauri / Electron）同契约。
mod store;

// 清除浏览数据（缓存 / Cookie / 本地存储 …）：设置页「清除全部」按钮的后端。
// ⚠️ 模块与命令都**不按平台 gate**：`invoke_handler` 是跨平台的注册表，Android 上少了这条
// 命令，设置页的调用会变成「命令不存在」而不是一条能看懂的错误。平台差异收敛在模块内部
// （非 Windows 返回明确的「不支持」）。详见该模块文档。
mod web_data;

// 启动过渡：本地启动页（splash 自定义协议）+ 注入到每个文档的加载层脚本。
mod loading;

// 后台准备真实地址 → 就绪后导航（bootstrap / prepare_navigation / navigate_main / ProxyTask）。
mod bootstrap;
use bootstrap::ProxyTask;

// Windows 专属：WebView2 原生事件钩子。
#[cfg(windows)]
mod webview_hooks;

// Windows 专属：系统「另存为」对话框（下载时由用户决定落盘位置）。
#[cfg(windows)]
mod save_dialog;

// 多页签（tab）模式（仅 Windows）。
#[cfg(windows)]
mod tabs;

// 系统托盘 / 设置命令 / 窗口切换（仅桌面端）。
#[cfg(desktop)]
mod tray;

// 窗口几何持久化（仅桌面端）。
#[cfg(desktop)]
mod window_state;

use settings::SettingsState;

// 移动端（Android/iOS）入口：tauri::mobile_entry_point 会生成 JNI/extern 绑定，
// 供移动端原生壳调用；桌面端该属性为空操作，等价于一个普通 pub fn。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 桌面端注册全局快捷键插件；移动端没有该能力，跳过。
    let builder = tauri::Builder::default()
        // 本地启动页：splash://localhost/。主窗口创建时立刻加载它（零网络、首帧即显示
        // "加载中…"），把域名解析 / 本地反向代理启动等耗时操作挪到后台线程（见 bootstrap），
        // 消除"进程已启动但窗口还没出来"这段完全无反馈的等待。
        .register_uri_scheme_protocol(loading::SPLASH_SCHEME, loading::handle_splash)
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
        // ⚠️⚠️ **全项目只能有这一次调用** —— `Builder::invoke_handler` 是【替换】语义、**不是追加**：
        //    tauri-2.11.5 `src/app.rs:1662` 就是 `self.invoke_handler = Box::new(invoke_handler);`，
        //    而整个 AppManager 只持有这一个 handler（`src/manager/mod.rs`）。
        //    本文件历史上按模块分成了两次调用（多页签那次写在下面的 `cfg(windows)` 块里），
        //    后一次把前一次**整体覆盖** ⇒ Windows 上 `get_settings` / `save_settings` /
        //    `close_settings_window` / `clear_web_data` / `history_nav` 全部消失。
        //    前端拿到的报错正是「保存失败：Command save_settings not found」：handler 对未知命令
        //    返回 `false`，tauri `src/webview/mod.rs:1911` 据此 `reject(format!("Command {command} not found"))`。
        //    注意这条路径**编译期零警告**、CI 也全绿，只在用户点按钮时才现形 —— 所以另配了
        //    `ipc_command_registry` 静态契约测试（含「前端调用的命令名 ⊆ 这里注册的名字」）。
        //    ⇒ 平台差异一律用**命令项上的 `#[cfg(...)]` 属性**表达（`generate_handler!` 支持逐项属性），
        //      **不要**为了分平台再开第二次这个同名调用（本文件的契约测试会直接断言次数）。
        .invoke_handler(tauri::generate_handler![
            settings::get_settings,
            settings::save_settings,
            settings::close_settings_window,
            // 打开设置窗：给**本地页面**用（多页签标签栏 shell.html）。
            // 远程页面（单窗口主窗的自绘标题栏）只能用事件，见下面 setup 里的 `web2app:open-settings`。
            #[cfg(desktop)]
            settings::open_settings,
            web_data::clear_web_data,
            history_nav,
            // 多页签（仅 Windows）：命令与 `tabs` 模块同一个 cfg —— 模块不编译时命令也不能注册，
            // 否则 Android 上会退化成「命令不存在」而不是一条能看懂的错误。
            #[cfg(windows)]
            tabs::create_tab,
            #[cfg(windows)]
            tabs::switch_tab,
            #[cfg(windows)]
            tabs::close_tab,
            #[cfg(windows)]
            tabs::get_tabs,
            #[cfg(windows)]
            tabs::set_tab_title,
            #[cfg(windows)]
            tabs::minimize_host,
            #[cfg(windows)]
            tabs::close_host
        ]);

    // 多页签模式（仅 Windows）：注册 shell 标签栏协议 + TabManager 状态。
    // ⚠️ IPC 命令已并入上面**唯一一次** `invoke_handler`（见那段注释），别再在这里开第二次。
    #[cfg(windows)]
    let builder = builder
        .register_uri_scheme_protocol(tabs::SHELL_SCHEME, tabs::handle_shell)
        .manage(tabs::TabManager::default());

    #[cfg(desktop)]
    let settings_open = settings::settings_open_shortcut();
    #[cfg(desktop)]
    let builder = builder.plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(move |app, shortcut, event| {
                // 只在“按下”那一刻触发一次。若忽略 event.state，按键的「按下」和「松开」
                // 会各自触发一次回调 → toggle 两次相互抵消，表现就是“按了没反应 / 乱跳”。
                if event.state != tauri_plugin_global_shortcut::ShortcutState::Pressed {
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
                    .unwrap_or_else(|p| p.into_inner())
                    .wake_hotkey
                    .clone();
                if let Some(sc) = settings::hotkey_to_shortcut(&hk) {
                    if shortcut.to_string() == sc.to_string() {
                        tray::toggle_window(app);
                    }
                }
            })
            .build(),
    );

    // 单实例：第二个实例启动时不再新开窗口，而是把已存在的窗口带到前台。
    // （仅桌面端；Android 无此概念，故按 target_os 剔除。）
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        // 多页签模式聚焦宿主窗口，单窗口模式聚焦 "main"。
        #[cfg(windows)]
        let label: &str = if tabs::tabs_enabled() {
            tabs::HOST_LABEL
        } else {
            "main"
        };
        #[cfg(not(windows))]
        let label: &str = "main";
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
        }
        // 多页签模式：tab 窗口是独立顶层窗口，宿主恢复显示时要一并把当前 tab 显示出来
        // （否则只看到标签栏，内容区是空的）。
        #[cfg(windows)]
        if tabs::tabs_enabled() {
            tabs::set_tabs_visible(app, true);
        }
    }));

    let builder = builder.setup(move |app| {
        // ⚠️ 诊断日志要**尽早**初始化：越早越能捕获启动早期的日志。
        //    默认（debug=false）时它是空操作 —— 不建目录、不落文件、也不打印任何东西。
        debug_log::init(app.handle());

        // ---------- 「打开设置」事件（自绘标题栏的「设置」按钮） ----------
        // 为什么不是一条 IPC 命令：单窗口模式的主窗加载的是**远程页面**，Tauri 2 对非本地
        // origin 的 IPC 强制走 ACL，而应用命令没有 app 级 ACL 清单可授权 ⇒ `open_settings`
        // 必被拒（与 history_nav 同一条约束，见 src/titlebar.js 顶部说明）。所以那边发的是
        // **核心插件的 event 命令**（`core:event:allow-emit` 可以授权给远程 origin，
        // 见 capabilities/default.json），在这里落地成真正的开窗动作。
        // ⚠️ 事件名必须与 src/titlebar.js 里的字面量逐字一致，改一处必须改另一处。
        // ⚠️ 这条事件只用来「开设置窗」，不携带任何参数 —— 远程页面能让壳做的事仅此一件，
        //    别在这里接别的东西。
        #[cfg(desktop)]
        {
            let handle = app.handle().clone();
            app.listen("web2app:open-settings", move |_event| {
                settings::open_settings_window(&handle);
            });
        }

        // 载入持久化设置（文件不存在时使用默认值），写入托管状态供运行时读取。
        let initial_settings = settings::load_settings(app.handle());
        *app.state::<SettingsState>()
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = initial_settings.clone();

        // ============ 代理模式决策（由 PROXY_ENABLED / PROXY_HTTP / PROXY_HTTPS 共同决定）============
        //  PROXY_ENABLED = false
        //      → 不启用任何代理，WebView 直接导航到真实上游地址。
        //  PROXY_ENABLED = true 且 PROXY_HTTP/HTTPS 非空（仅 Windows --proxy-server 支持）
        //      → 使用【外部代理】，WebView 直接导航到真实上游地址。
        //  PROXY_ENABLED = true 且 PROXY_HTTP/HTTPS 为空（含所有移动端）
        //      → 启用【自身实现的本地反向代理】（HTTP + WebSocket 统一注入 Authorization）。
        // ========================================================================================
        #[cfg(windows)]
        let browser_args: Option<String> = if config::PROXY_ENABLED {
            config::build_proxy_args()
        } else {
            None
        };

        // ⚠️ 解析真实地址、启动本地反向代理等【耗时操作不在这里做】——它们被挪到后台线程的
        //    bootstrap() 里（见本函数末尾）。现在的顺序是：先建窗 → 立刻显示启动页"加载中…"
        //    → 后台准备 → 就绪后导航。

        // ---------- 创建窗口 ----------
        // 单窗口模式：主窗口首帧加载本地启动页 splash://localhost/（零网络请求）。
        // 多页签模式（仅 Windows）：改为主窗口加载 shell:// 标签栏，网页内容在各 tab 窗口。
        // 两种模式都把耗时操作（域名解析 / 本地反向代理）挪到后台线程 bootstrap，先建窗
        // 立即显示"加载中…"，避免"进程已启动但窗口还没出来"的空等待。

        // 单窗口建窗逻辑（抽成局部闭包，多页签与单窗口模式共用其子步骤）。
        let build_main_window = |app: &tauri::AppHandle| -> tauri::WebviewWindow {
            let wb = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::CustomProtocol(loading::splash_url()),
            )
            .title(config::TITLE)
            .resizable(true)
            .inner_size(1100.0, 650.0)
            .min_inner_size(400.0, 300.0)
            // 窗口/WebView 的默认底色取加载层的底色：盖住 WebView 初始化与导航提交瞬间的白闪。
            .background_color(bootstrap::SPLASH_BG)
            // 加载层：在每个新文档的 document-created 阶段铺满视口，页面渲染出内容后淡出。
            .initialization_script(loading::OVERLAY_SCRIPT)
            // 注入打包配置 JSON 里的 `extra`：以 `window.web2appExtra = <JSON>;` 定义在页面全局。
            // 经 extra_json() 校验：EXTRA 不是合法 JSON 时回退 "{}"，避免注入非法 JSON 破坏页面脚本。
            .initialization_script(format!("window.web2appExtra = {};", config::extra_json()));

            // Windows 专属链：禁用拖放 handler + 注入拖拽 polyfill。
            #[cfg(windows)]
            let wb = wb
                // ⚠️ 必须禁用 Tauri 默认的拖放 handler（Windows），否则系统文件拖入被 wry 拦截、
                // 页面收不到原生 DOM drop，且页面内 HTML5 拖动与宿主 OLE drop target 打架。
                // Tauri 官方文档原文："Disables the drag and drop handler. This is required to
                // use HTML5 drag and drop APIs on the frontend on Windows."
                .disable_drag_drop_handler()
                // 注入 WebView2 拖拽 polyfill：让页面内 HTML5 拖动（[draggable] 元素/列表排序）恢复。
                .initialization_script(include_str!("dnd_polyfill.js"))
                // 去掉系统标题栏（decorations(false)），改由注入的自绘标题栏 titlebar.js 提供
                // “后退/前进 + 最小化/最大化/关闭”按钮（仅单窗口模式生效；多页签模式见
                // tabs.rs 的宿主窗口 + shell.html 标签栏）。依赖 withGlobalTauri 暴露的 __TAURI__。
                .decorations(false)
                .initialization_script(include_str!("titlebar.js"))
                // 兜底重注入：正常路径下每次导航都会执行上面的 initialization_script，但 WebView2
                // 存在「某些新文档拿不到 AddScriptToExecuteOnDocumentCreated 脚本」的路径（例如
                // 新窗口请求被 SetNewWindow 处理后创建的目标文档、页面自己 document.write 重写）。
                // 这里在每次页面加载完成后再 eval 一次 —— titlebar.js 自带 `__web2appTitlebar`
                // 防重标记，正常情况下是纯 no-op，只在标题栏真的没挂上时才补挂。
                .on_page_load(|window, payload| {
                    if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                        let _ = window.eval(include_str!("titlebar.js"));
                    }
                });

            #[cfg(windows)]
            let wb = match &browser_args {
                Some(proxy_args) => wb.additional_browser_args(proxy_args.as_str()),
                None => wb,
            };

            // 数据目录：空串 = 系统默认（%LOCALAPPDATA%\<app_name>，不受程序位置影响）；
            // 相对路径 = exe 所在目录下（默认 "data"）；绝对路径 = 原样。
            // 不可写会自动回退到系统默认目录（见 config::data_dir）。
            #[cfg(windows)]
            let wb = match config::data_dir(app) {
                Some(dir) => wb.data_directory(dir),
                None => wb,
            };

            wb.build().expect("创建主窗口失败")
        };

        // 多页签模式（仅 Windows）：建宿主窗口 + 首 tab。
        #[cfg(windows)]
        if tabs::tabs_enabled() {
            let host = tabs::create_host_window(app.handle()).expect("创建宿主窗口失败");
            // 应用上次保存的窗口几何（宿主窗口复用主窗口的几何持久化）。
            if let Some(st) = window_state::load(app.handle()) {
                window_state::apply(&host, &st);
            }
            let _ = host.show();
            // 首 tab：走完整 bootstrap（缓存导航信息，供后续新 tab 复用）。
            let _ = tabs::spawn_tab(app.handle());
        }
        // 单窗口模式：走历史建窗逻辑。
        #[cfg(windows)]
        if !tabs::tabs_enabled() {
            let window = build_main_window(app.handle());

            // 应用上次保存的窗口大小/位置（若有）。
            if let Some(st) = window_state::load(app.handle()) {
                window_state::apply(&window, &st);
            }
            let _ = window.show();
            bootstrap::bootstrap(app.handle().clone());
        }

        // 非 Windows（Android 等）：单窗口逻辑（tabs 能力不存在）。
        #[cfg(not(windows))]
        {
            let window = build_main_window(app.handle());

            #[cfg(desktop)]
            if let Some(st) = window_state::load(app.handle()) {
                window_state::apply(&window, &st);
            }
            let _ = window.show();
            bootstrap::bootstrap(app.handle().clone());
        }

        // ---------- 系统托盘与全局快捷键（仅桌面端；Android 无）----------
        #[cfg(desktop)]
        {
            // 托盘图标：仅当设置中“启用托盘”时创建
            if initial_settings.tray_enabled {
                tray::create_tray(app.handle())?;
            }

            // 注册全局快捷键：① 唤醒主窗口（取自设置） ② 打开设置界面（固定 Shift+F1）
            // ⚠️ 注册失败【不能】用 `?` 上抛：快捷键可能已被其它程序占用（或注册表被改），
            // 此时应用其它功能（窗口/托盘/代理）仍可正常使用，降级为“无该快捷键”并打日志，
            // 而非整个应用启动失败。占用冲突是最常见的桌面场景，不是“环境不支持”。
            if let Some(sc) = settings::hotkey_to_shortcut(&initial_settings.wake_hotkey) {
                if let Err(e) = app.global_shortcut().register(sc) {
                    dlog!("[shortcut] 唤醒快捷键注册失败（可能已被其它程序占用）：{e}");
                }
            }
            if let Err(e) = app
                .global_shortcut()
                .register(settings::settings_open_shortcut())
            {
                dlog!("[shortcut] 设置快捷键(Shift+F1)注册失败（可能已被其它程序占用）：{e}");
            }
        }

        // 启动信息（落地位置见 debug_log 模块）：与 Electron 侧 `index.ts::logStartup()`
        // 一一对应，便于两壳的日志互相印证。默认 debug=false 时这里什么都没发生。
        log_startup(app.handle(), &initial_settings);

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
                    // 多页签模式（仅 Windows）：区分宿主窗口 / tab 窗口 / 主窗口三种关闭语义。
                    #[cfg(windows)]
                    if tabs::tabs_enabled() {
                        // tab 窗口关闭：转交给 tabs 模块（关闭该 tab，而非退出应用）。
                        if label.starts_with(tabs::TAB_PREFIX) {
                            api.prevent_close();
                            tabs::close_tab_inner(app_handle, &label);
                            return;
                        }
                        // 宿主窗口关闭：等价于关闭整个应用窗口，走主窗口关闭语义。
                        if label == tabs::HOST_LABEL {
                            handle_main_close(app_handle, api);
                            return;
                        }
                    }

                    // 单窗口模式：仅对主窗口生效。
                    if label != "main" {
                        return;
                    }
                    handle_main_close(app_handle, api);
                }
                // 应用即将退出：记录窗口几何（兜底，覆盖“托盘退出”等不经过关窗的路径），
                // 优雅终止本地反向代理任务，释放 127.0.0.1 监听端口。
                RunEvent::ExitRequested { .. } => {
                    #[cfg(desktop)]
                    {
                        // 多页签模式持久化宿主窗口几何，单窗口模式持久化 "main"。
                        #[cfg(windows)]
                        let label: &str = if tabs::tabs_enabled() {
                            tabs::HOST_LABEL
                        } else {
                            "main"
                        };
                        #[cfg(not(windows))]
                        let label: &str = "main";
                        window_state::persist_label(app_handle, label);
                    }
                    if let Some(abort) = app_handle
                        .state::<ProxyTask>()
                        .0
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .take()
                    {
                        abort.abort();
                    }
                }
                _ => {}
            }
        });
}

/// 启动信息（写进诊断日志；默认 `debug=false` 时整段是空操作）。
///
/// 与 Electron 侧 `electron-app/src/main/index.ts::logStartup()` **一一对应**，目的是让两壳的
/// 日志能互相印证（同一个用户报同一个问题，换壳排查时不用重新学一遍字段名）。
///
/// ⚠️⚠️ **绝不记凭据**：地址一律经 `debug_log::mask_url()`（隐去 `user:pass@`），
///    Secret 只报「已配置 / 未配置」，公钥只报状态不报内容。
fn log_startup(app: &tauri::AppHandle, settings: &settings::Settings) {
    dlog!("[web2app] 启动信息");
    dlog!("  分发形态      : Tauri（配置是编译期常量 —— 改配置必须重新编译/打包）");
    dlog!("  应用名 / 标题 : {} / {}", config::APP_NAME, config::TITLE);
    dlog!("  目标地址      : {}", describe_target());
    #[cfg(windows)]
    dlog!(
        "  数据目录      : {}",
        match config::data_dir(app) {
            Some(d) => d.display().to_string(),
            None => "(不可用，改用 WebView2 自身默认位置)".to_string(),
        }
    );
    #[cfg(not(windows))]
    dlog!("  数据目录      : (非 Windows：由平台自行管理)");
    dlog!("  代理          : {}", describe_proxy());
    dlog!("  多页签        : {}", describe_tabs());
    dlog!(
        "  开发者工具    : {}",
        if config::DEVTOOLS_ENABLED {
            "已启用（按 F12 打开）"
        } else {
            "未启用"
        }
    );
    dlog!(
        "  配置文件      : {}（设置 + 窗口几何）",
        store::path(app)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(取不到配置目录)".to_string())
    );
    dlog!("  唤醒快捷键    : {}", hotkey_label(&settings.wake_hotkey));
    dlog!(
        "  托盘 / 关闭行为: {} / {}",
        if settings.tray_enabled {
            "已启用"
        } else {
            "未启用"
        },
        if settings.tray_enabled && settings.minimize_on_close {
            "关闭时隐藏到托盘"
        } else {
            "关闭即退出"
        }
    );
    dlog!(
        "  加解密 Secret : {}",
        if resolve_core::secret_is_configured() {
            "已由配置提供"
        } else {
            "未配置（用内置默认串：域名数组模式下安全门槛较低）"
        }
    );
    dlog!(
        "  日志文件      : {}",
        debug_log::log_file_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(debug 已开启但不可写，本次只有 stderr)".to_string())
    );
    dlog!("  web2appExtra  : {}", config::extra_json());
}

/// 目标地址的一行描述（**脱敏**）：域名数组只报「几个 / 都是谁」，普通 URL 隐去 userinfo。
fn describe_target() -> String {
    if let Some(domains) = resolve::parse_domains(config::APP_URL) {
        let list: Vec<String> = domains
            .iter()
            .map(|d| resolve::normalize_domain(d.as_str()))
            .collect();
        return format!(
            "{} 个跳板域名（DNS TXT 解析 + 验签解密）：{}；公钥{}",
            list.len(),
            list.join("、"),
            if config::PUBLIC_KEY.trim().is_empty() {
                "未配置（数组模式必须配，否则解析必然失败）"
            } else {
                "已配置"
            }
        );
    }
    debug_log::mask_url(config::APP_URL)
}

/// 代理模式的一行描述。判据与 `bootstrap::local_proxy_enabled()` 同源，别各写一份。
fn describe_proxy() -> String {
    if !config::PROXY_ENABLED {
        return "未启用（直连）".to_string();
    }
    if bootstrap::local_proxy_enabled() {
        return "本地反向代理（HTTP + WebSocket 统一注入 Basic 鉴权）".to_string();
    }
    #[cfg(windows)]
    {
        match config::build_proxy_args() {
            // ⚠️ 外部代理地址里可能带 `user:pass@`，同样要脱敏后才进日志
            Some(args) => format!("外部代理：{}", debug_log::mask_url(&args)),
            None => "已启用但没配任何代理地址（等价于直连）".to_string(),
        }
    }
    #[cfg(not(windows))]
    {
        "已启用（非 Windows 只有本地反向代理这一条路）".to_string()
    }
}

/// 多页签是否启用（仅 Windows 有该能力）。
fn describe_tabs() -> &'static str {
    #[cfg(windows)]
    {
        if tabs::tabs_enabled() {
            return "已启用";
        }
    }
    "未启用"
}

/// 唤醒快捷键的可读文案（空 key = 未启用）。与 Electron 侧 `hotkeyLabel()` 同形。
fn hotkey_label(h: &settings::Hotkey) -> String {
    if h.key.trim().is_empty() {
        return "（未设置）".to_string();
    }
    let mut parts: Vec<&str> = Vec::new();
    if h.ctrl {
        parts.push("Ctrl");
    }
    if h.alt {
        parts.push("Alt");
    }
    if h.shift {
        parts.push("Shift");
    }
    parts.push(h.key.as_str());
    parts.join("+")
}

/// 处理「主窗口关闭」：若“启用托盘 + 关闭时最小化到托盘”同时满足 → 隐藏到托盘；
/// 否则允许关闭（最后一个窗口关闭后应用退出）。
///
/// `target_label` 由调用方决定：单窗口模式为 `"main"`，多页签模式为宿主窗口 label。
/// 这样 `window_state::persist_label` 记录的也是正确窗口的几何。
///
/// ⚠️ 本函数【不可】加 `cfg(desktop)`：Android 也会走到这里读
/// SettingsState 判断托盘语义（托盘未启用时等价于允许关闭）。只有「窗口几何持久化」
/// 依赖的 `window_state` 模块才是桌面端专属，故 cfg 下沉到那一行。
fn handle_main_close(app_handle: &tauri::AppHandle, api: tauri::CloseRequestApi) {
    let target_label: String = {
        #[cfg(windows)]
        {
            if crate::tabs::tabs_enabled() {
                crate::tabs::HOST_LABEL.to_string()
            } else {
                "main".to_string()
            }
        }
        #[cfg(not(windows))]
        {
            "main".to_string()
        }
    };

    // 记录主窗口当前大小/位置，供下次启动恢复（仅桌面端；Android 无窗口几何持久化）。
    #[cfg(desktop)]
    window_state::persist_label(app_handle, &target_label);

    let st = app_handle.state::<SettingsState>();
    let (tray, min) = {
        let s = st.0.lock().unwrap_or_else(|p| p.into_inner());
        (s.tray_enabled, s.minimize_on_close)
    };
    if tray && min {
        api.prevent_close();
        if let Some(w) = app_handle.get_webview_window(&target_label) {
            let _ = w.hide();
        }
        // 多页签模式：tab 窗口是独立顶层窗口，宿主隐藏时**必须一并隐藏**，
        // 否则桌面上会残留没有标签栏的“孤儿内容窗口”，用户会以为应用卡死了。
        #[cfg(windows)]
        if crate::tabs::tabs_enabled() {
            crate::tabs::set_tabs_visible(app_handle, false);
        }
    } else {
        // 允许关闭（不隐藏到托盘）：tab 窗口同样不会随宿主销毁，必须显式清掉，
        // 否则「标签栏关了、内容还在」且 Tauri 因为仍有窗口而不触发退出。
        #[cfg(windows)]
        if crate::tabs::tabs_enabled() {
            crate::tabs::destroy_all_tabs(app_handle);
        }
    }
}

/// 标题栏「后退 / 前进」命令：对【当前】窗体对应的 webview 执行历史导航。
///
/// 目标窗体的确定：
/// - 多页签模式（仅 Windows）：当前激活的 tab 窗口（标题栏按钮在宿主标签栏左侧）。
/// - 单窗口模式：主窗 "main"（自绘标题栏注入到主窗每个文档）。
///
/// 统一用 [`tauri::WebviewWindow::eval`] 在目标 webview 内执行 `window.history.back()` /
/// `window.history.forward()`，命中 WebView 自己的会话历史（SPA 路由 / 前进后退均有效）。
#[tauri::command]
fn history_nav(app: tauri::AppHandle, direction: String) -> Result<(), String> {
    // 确定目标 webview label。
    let target_label: String = {
        #[cfg(windows)]
        if tabs::tabs_enabled() {
            let state = app.state::<tabs::TabManager>();
            let active = *state.active.lock().unwrap_or_else(|p| p.into_inner());
            let labels = state.tabs.lock().unwrap_or_else(|p| p.into_inner());
            labels
                .get(active)
                .map(|t| t.label.clone())
                .unwrap_or_else(|| "main".to_string())
        } else {
            "main".to_string()
        }
        #[cfg(not(windows))]
        {
            "main".to_string()
        }
    };

    let Some(window) = app.get_webview_window(&target_label) else {
        return Err(format!("历史导航失败：窗口 {target_label} 不存在"));
    };

    // 仅接受 'back' / 'forward' 两个方向，防误传。
    let script = match direction.as_str() {
        "back" => "try{window.history.back();}catch(e){}",
        "forward" => "try{window.history.forward();}catch(e){}",
        _ => return Err("无效的历史导航方向（仅支持 back / forward）".to_string()),
    };

    window
        .eval(script)
        .map_err(|e| format!("历史导航失败：{e}"))
}

/// IPC 命令注册表的静态契约测试。
///
/// 存在理由：`Builder::invoke_handler` 是**替换**语义，多调用一次就会静默丢掉前一次的
/// 全部命令 —— 编译期无警告、CI 全绿，只在用户点下按钮那一刻变成「Command … not found」
/// （真实事故：Windows 设置页「保存失败：Command save_settings not found」）。
/// 这里用两条静态断言把它钉死：① 调用点唯一；② 前端调用的命令名 ⊆ 注册表里的名字。
#[cfg(test)]
mod ipc_command_registry {
    /// 注册表的唯一真源：本文件源码。
    const LIB_RS: &str = include_str!("lib.rs");

    /// 计数用的「针」拆成两段拼接：避免本模块自己的注释 / 断言文案被当成调用点或注册表。
    const INVOKE_HANDLER: &str = concat!(".invoke", "_handler(");
    const GENERATE_HANDLER: &str = concat!("tauri::generate", "_handler![");

    /// 从 `invoke("cmd"` / `invoke('cmd'` 里抽出命令名（前端调用点）。
    fn invoked_commands(src: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = src;
        while let Some(pos) = rest.find("invoke(") {
            // ⚠️ 先推进游标再判断，任何分支都不会造成死循环。
            rest = &rest[pos + "invoke(".len()..];
            let t = rest.trim_start();
            let Some(quote) = t.chars().next().filter(|c| *c == '"' || *c == '\'') else {
                continue;
            };
            let body = &t[1..];
            let Some(end) = body.find(quote) else {
                continue;
            };
            let name = &body[..end];
            // 只收合法的命令名字面量：`invoke(cmd, …)`（变量）之类自然被排除。
            if !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':' || c == '-')
            {
                out.push(name.to_string());
            }
        }
        out
    }

    /// 从唯一那处 `generate_handler![…]` 里抽出已注册的命令名（取路径末段）。
    fn registered_commands() -> Vec<String> {
        let start = LIB_RS
            .find(GENERATE_HANDLER)
            .expect("lib.rs 里必须有 generate_handler! 注册表");
        let body = &LIB_RS[start..];
        let end = body
            .find("]);")
            .expect("注册表必须以 `]);` 收尾（否则本测试扫不到尾）");
        body[..end]
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with("//") && !l.starts_with("#["))
            .map(|l| l.trim_end_matches(','))
            // 只留「就是一个 Rust 路径」的行 —— 排掉 `tauri::generate_handler![` 自身之类的噪声。
            .filter(|l| {
                l.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
            })
            .map(|p| p.rsplit("::").next().unwrap_or(p).to_string())
            .collect()
    }

    /// ① 调用点唯一：`Builder::invoke_handler` 调第二次就是「命令集体消失」。
    #[test]
    fn exactly_one_invoke_handler_call_site() {
        let n = LIB_RS.matches(INVOKE_HANDLER).count();
        assert_eq!(
            n, 1,
            "lib.rs 里出现了 {n} 次 invoke_handler 调用。它是【替换】不是【追加】——\
             第二次调用会静默丢掉前一次注册的全部命令（Windows 上表现为设置页保存报\
             「Command … not found」）。平台差异请写成命令项上的 #[cfg(...)] 属性。"
        );
    }

    /// ② 前端真的会调的每个命令，都必须能在注册表里找到；否则运行时就是一条「命令不存在」。
    #[test]
    fn every_frontend_invoked_command_is_registered() {
        let registered = registered_commands();

        // 跨平台页面：设置页在 Windows / Android 上都要能存设置。
        let mut sources: Vec<(&str, &str)> =
            vec![("dist/settings.html", include_str!("../dist/settings.html"))];
        // 多页签（仅 Windows）页面：其命令本身也是 `#[cfg(windows)]` 注册的。
        #[cfg(windows)]
        {
            sources.push(("src/shell.html", include_str!("shell.html")));
            sources.push(("src/tab_title.js", include_str!("tab_title.js")));
        }

        let mut missing: Vec<String> = Vec::new();
        for (file, src) in sources {
            let found = invoked_commands(src);
            // ⚠️ 先堵「假绿」：抽不出任何命令时下面的子集断言会无条件通过。
            assert!(
                !found.is_empty(),
                "{file} 里一个 invoke(\"…\") 都没抽到 —— 要么前端改了调用姿势，\
                 要么提取器坏了；两种情况都必须人工确认，不能让本条断言静默通过"
            );
            for cmd in found {
                if !registered.contains(&cmd) {
                    missing.push(format!("{file} → {cmd}"));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "前端调用了未注册的 IPC 命令（运行时报「Command … not found」）：{missing:?}；\
             已注册：{registered:?}"
        );
    }

    /// ③ 注册表本身：基础命令跨平台必须在；多页签命令在 Windows 上必须在。
    #[test]
    fn required_commands_are_registered() {
        let registered = registered_commands();
        let base = [
            "get_settings",
            "save_settings",
            "close_settings_window",
            "clear_web_data",
            "history_nav",
        ];
        for name in base {
            assert!(
                registered.iter().any(|r| r == name),
                "IPC 命令 {name} 未注册（已注册：{registered:?}）"
            );
        }

        // 桌面端：打开设置窗（多页签标签栏 shell.html 会调它；Android 没有设置窗）。
        #[cfg(desktop)]
        {
            let name = "open_settings";
            assert!(
                registered.iter().any(|r| r == name),
                "IPC 命令 {name} 未注册（已注册：{registered:?}）"
            );
        }

        #[cfg(windows)]
        for name in [
            "create_tab",
            "switch_tab",
            "close_tab",
            "get_tabs",
            "set_tab_title",
            "minimize_host",
            "close_host",
        ] {
            assert!(
                registered.iter().any(|r| r == name),
                "多页签 IPC 命令 {name} 未注册（已注册：{registered:?}）"
            );
        }
    }
}

/// 能力声明（capability）与跨文件字面量的静态契约测试。
///
/// 存在理由：这一类错误**全是静默失败** —— 编译期零警告、clippy 与 CI 全绿，
/// 只有用户手动点下去才现形，而表象只有一句「点了没反应 / 窗口拖不动」：
///
/// * `capabilities/default.json` 少了 `core:event:allow-emit`
///   ⇒ 主窗（远程页面）里自绘标题栏的「设置」按钮发不出事件；
/// * `capabilities/settings-window.json` 少了 `core:window:allow-start-dragging`
///   ⇒ 无边框设置窗**拖不动**，控制台里只有一条被拒的 `plugin:window|start_dragging`；
/// * 事件名在 `lib.rs` 与 `src/titlebar.js` 两侧不逐字一致 ⇒ 同上。
///
/// 另外：`settings-window.json` 曾经**漏纳入版本控制**（新增文件忘了 `git add`）。
/// 用 `include_str!` 读它，可以让「文件缺失」直接变成**编译失败**，
/// 而不是让人克隆出来的仓库静默丢掉拖动能力。
#[cfg(test)]
mod capability_contract {
    /// 本文件源码：事件监听那一段的字面量在这里。
    const LIB_RS: &str = include_str!("lib.rs");
    /// 设置窗的创建处 —— 窗口标签的真源（`WebviewWindowBuilder::new(app, "…")`）。
    const SETTINGS_RS: &str = include_str!("settings.rs");
    /// 主窗的自绘标题栏（跑在远程页面里）。
    const TITLEBAR_JS: &str = include_str!("titlebar.js");
    /// 主窗（单窗口模式）能力声明。
    const MAIN_CAP: &str = include_str!("../capabilities/default.json");
    /// 设置窗（无边框）能力声明 —— 权限刻意与主窗分开。
    const SETTINGS_CAP: &str = include_str!("../capabilities/settings-window.json");

    /// 「打开设置」事件名两侧的字面量写法。
    /// ⚠️ 必须**拆成两段拼接**：本文件就是被测对象之一，直接写完整字面量会让断言
    /// **自我满足**（`ipc_command_registry` 首轮踩过同一个坑）。
    const EVENT_IN_LIB: &str = concat!("app.listen(\"", "web2app:open-settings", "\"");
    const EVENT_IN_TITLEBAR: &str = concat!("'", "web2app:open-settings", "'");

    /// 读 capability 的某个字符串数组字段，顺带把「字段缺失 / 非字符串项」变成可读失败。
    fn str_list(v: &serde_json::Value, key: &str, file: &str) -> Vec<String> {
        let Some(items) = v[key].as_array() else {
            panic!("{file} 缺少数组字段 `{key}`");
        };
        let mut out = Vec::new();
        for x in items {
            let Some(s) = x.as_str() else {
                panic!("{file} 的 `{key}` 里有非字符串项：{x}");
            };
            out.push(s.to_string());
        }
        out
    }

    /// 取出 capability 的 `windows` 与 `permissions`。
    fn cap(src: &str, file: &str) -> (Vec<String>, Vec<String>) {
        let v: serde_json::Value = match serde_json::from_str(src) {
            Ok(v) => v,
            Err(e) => panic!("{file} 不是合法 JSON：{e}"),
        };
        (
            str_list(&v, "windows", file),
            str_list(&v, "permissions", file),
        )
    }

    fn assert_grants(file: &str, perms: &[String], want: &str) {
        assert!(
            perms.iter().any(|p| p == want),
            "{file} 没有授予 {want}（现有：{perms:?}）—— 少了它不报错，只会静默失效"
        );
    }

    /// ① 主窗：远程页面里按钮 / 拖动 / 发事件所依赖的那几条。
    #[test]
    fn main_capability_grants_remote_titlebar_permissions() {
        let (windows, perms) = cap(MAIN_CAP, "capabilities/default.json");
        assert_eq!(windows, ["main"], "主窗 capability 必须只作用于标签 main");
        for want in [
            "core:window:allow-minimize",
            "core:window:allow-toggle-maximize",
            "core:window:allow-close",
            "core:window:allow-start-dragging",
            "core:window:allow-internal-toggle-maximize",
            // 「设置」按钮走事件而不是应用命令的原因，见 default.json 的 description。
            "core:event:allow-emit",
        ] {
            assert_grants("capabilities/default.json", &perms, want);
        }
    }

    /// ② 设置窗：无边框窗拖得动的前提；同时钉住「权限与主窗刻意分开」这个设计。
    #[test]
    fn settings_capability_matches_frameless_window() {
        let (windows, perms) = cap(SETTINGS_CAP, "capabilities/settings-window.json");

        // 标签必须与 settings.rs 里真实创建窗口时用的那个逐字一致，否则权限不会命中。
        let needle = "WebviewWindowBuilder::new(app, \"";
        let at = SETTINGS_RS
            .find(needle)
            .expect("settings.rs 里找不到设置窗的创建点 —— 本条断言扫不到窗口标签了");
        let label = &SETTINGS_RS[at + needle.len()..];
        let label = &label[..label.find('"').expect("设置窗标签没有收尾引号")];
        assert_eq!(
            windows,
            [label],
            "capability 的 windows 与 settings.rs 建窗时的标签不一致 ⇒ 权限不会生效"
        );

        assert_grants(
            "capabilities/settings-window.json",
            &perms,
            "core:window:allow-start-dragging",
        );

        // 「无边框」正是「必须靠 capability 才拖得动」的前提，两者要么一起在、要么一起改。
        assert!(
            SETTINGS_RS.contains(".decorations(false)"),
            "设置窗不再是无边框窗（decorations(false) 没了）⇒ \
             自绘面板头部与 start-dragging 的前提已变，请重新复核本模块"
        );
    }

    /// ③ 事件名两侧字面量：改一处忘另一处 ⇒ 「设置」按钮点了没反应，且没有任何报错。
    #[test]
    fn open_settings_event_name_matches_between_lib_and_titlebar() {
        assert!(
            LIB_RS.contains(EVENT_IN_LIB),
            "lib.rs 里找不到 `app.listen(\"…\")` 形式的设置事件监听（找的是 {EVENT_IN_LIB}）"
        );
        assert!(
            TITLEBAR_JS.contains(EVENT_IN_TITLEBAR),
            "src/titlebar.js 里找不到同名的单引号事件字面量（找的是 {EVENT_IN_TITLEBAR}）\
             —— 两侧必须逐字一致，否则按钮点了没反应"
        );
    }
}
