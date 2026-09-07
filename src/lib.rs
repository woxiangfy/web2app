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
//! - [`webview_hooks`] Windows WebView2 原生事件钩子（鉴权/新窗口/右键菜单）
//! - [`tray`]          系统托盘 + 设置命令 + 窗口切换
//! - [`window_state`]  窗口几何持久化（仅桌面端）

// 允许各平台下未被使用的编译期常量（例如 Android 上不使用的 HOTKEY_* / DEVTOOLS_ENABLED 等），
// 避免出现 dead_code 警告导致 `cargo clippy -D warnings` 在 CI 中失败。
#![allow(dead_code)]

use tauri::{Manager, RunEvent, WindowEvent};

// 全局快捷键插件（仅桌面端；Android 无该依赖，见 Cargo.toml 平台 gate）。
#[cfg(not(target_os = "android"))]
use tauri_plugin_global_shortcut::GlobalShortcutExt;

pub mod config;
mod proxy;
pub mod resolve;
mod settings;

// 启动过渡：本地启动页（splash 自定义协议）+ 注入到每个文档的加载层脚本。
mod loading;

// 后台准备真实地址 → 就绪后导航（bootstrap / prepare_navigation / navigate_main / ProxyTask）。
mod bootstrap;
use bootstrap::ProxyTask;

// Windows 专属：WebView2 原生事件钩子。
#[cfg(windows)]
mod webview_hooks;

// 多页签（tab）模式（仅 Windows）。
#[cfg(windows)]
mod tabs;

// 系统托盘 / 设置命令 / 窗口切换（仅桌面端）。
#[cfg(not(target_os = "android"))]
mod tray;

// 窗口几何持久化（仅桌面端）。
#[cfg(not(target_os = "android"))]
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
        .invoke_handler(tauri::generate_handler![
            settings::get_settings,
            settings::save_settings,
            settings::close_settings_window
        ]);

    // 多页签模式（仅 Windows）：注册 shell 标签栏协议 + TabManager 状态 + tab 相关 IPC 命令。
    #[cfg(windows)]
    let builder = {
        let b = builder
            .register_uri_scheme_protocol(tabs::SHELL_SCHEME, tabs::handle_shell)
            .manage(tabs::TabManager::default())
            .invoke_handler(tauri::generate_handler![
                tabs::create_tab,
                tabs::switch_tab,
                tabs::close_tab,
                tabs::get_tabs,
                tabs::set_tab_title
            ]);
        b
    };

    #[cfg(not(target_os = "android"))]
    let settings_open = settings::settings_open_shortcut();
    #[cfg(not(target_os = "android"))]
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
    #[cfg(not(target_os = "android"))]
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
    }));

    let builder = builder.setup(move |app| {
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
            .title(config::WINDOW_TITLE)
            .resizable(true)
            .inner_size(1100.0, 650.0)
            .min_inner_size(400.0, 300.0)
            // 窗口/WebView 的默认底色取加载层的底色：盖住 WebView 初始化与导航提交瞬间的白闪。
            .background_color(bootstrap::SPLASH_BG)
            // 加载层：在每个新文档的 document-created 阶段铺满视口，页面渲染出内容后淡出。
            .initialization_script(loading::OVERLAY_SCRIPT)
            // 注入打包配置 JSON 里的 `extra`：以 `window.web2appExtra = <JSON>;` 定义在页面全局。
            .initialization_script(format!("window.web2appExtra = {};", config::EXTRA));

            // Windows 专属链：禁用拖放 handler + 注入拖拽 polyfill。
            #[cfg(windows)]
            let wb = wb
                // ⚠️ 必须禁用 Tauri 默认的拖放 handler（Windows），否则系统文件拖入被 wry 拦截、
                // 页面收不到原生 DOM drop，且页面内 HTML5 拖动与宿主 OLE drop target 打架。
                // Tauri 官方文档原文："Disables the drag and drop handler. This is required to
                // use HTML5 drag and drop APIs on the frontend on Windows."
                .disable_drag_drop_handler()
                // 注入 WebView2 拖拽 polyfill：让页面内 HTML5 拖动（[draggable] 元素/列表排序）恢复。
                .initialization_script(include_str!("dnd_polyfill.js"));

            #[cfg(windows)]
            let wb = match &browser_args {
                Some(proxy_args) => wb.additional_browser_args(proxy_args.as_str()),
                None => wb,
            };

            // 数据目录：true = WebView2 系统默认；false = %LOCALAPPDATA%\{自定义 app_name}（推荐）。
            // 用 if-let 而非 `?`：app_local_data_dir 在极少数环境可能返回 Err，拿不到就退回默认。
            #[cfg(windows)]
            let wb = if !config::DATA_DIR_DEFAULT {
                match app.path().app_local_data_dir() {
                    Ok(dir) => {
                        let base = dir.parent().unwrap_or(&dir);
                        let new_dir = base.join(config::sanitize_dir_name(config::APP_NAME));
                        wb.data_directory(new_dir)
                    }
                    Err(_) => wb,
                }
            } else {
                wb
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

            #[cfg(not(target_os = "android"))]
            if let Some(st) = window_state::load(app.handle()) {
                window_state::apply(&window, &st);
            }
            let _ = window.show();
            bootstrap::bootstrap(app.handle().clone());
        }

        // ---------- 系统托盘与全局快捷键（仅桌面端；Android 无）----------
        #[cfg(not(target_os = "android"))]
        {
            // 托盘图标：仅当设置中“启用托盘”时创建
            if initial_settings.tray_enabled {
                tray::create_tray(app.handle())?;
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
                    #[cfg(not(target_os = "android"))]
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

/// 处理「主窗口关闭」：若“启用托盘 + 关闭时最小化到托盘”同时满足 → 隐藏到托盘；
/// 否则允许关闭（最后一个窗口关闭后应用退出）。
///
/// `target_label` 由调用方决定：单窗口模式为 `"main"`，多页签模式为宿主窗口 label。
/// 这样 `window_state::persist_label` 记录的也是正确窗口的几何。
///
/// ⚠️ 本函数【不可】加 `cfg(not(target_os = "android"))`：Android 也会走到这里读
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
    #[cfg(not(target_os = "android"))]
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
    }
}
