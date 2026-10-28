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
//! - [`nav`]           导航 / 窗口会话登记数据枢纽（Navigation / BootReady / label 登记 / 导航原语）
//! - [`bootstrap`]     启动编排（后台准备真实地址 → 就绪后导航；入口经 `nav` 回调转发）
//! - [`webview_hooks`] Windows WebView2 原生事件钩子（鉴权/新窗口/下载/右键菜单）
//! - [`save_dialog`]   Windows 原生「另存为」对话框（下载落盘位置由用户决定）
//! - [`tray`]          系统托盘 + 设置命令 + 窗口切换
//! - [`window_state`]  窗口几何持久化（仅桌面端）
//! - [`debug_log`]     诊断日志（默认关闭；`dlog!` 宏 + 启动信息落盘）

// 允许各平台下未被使用的编译期常量（例如 Android 上不使用的 DEVTOOLS_ENABLED 等），
// 避免出现 dead_code 警告导致 `cargo clippy -D warnings` 在 CI 中失败。
// ⚠️ 纪律（架构审查 P1-3）：这是给「本机无法验证的 Android 目标」留的兜底，**新增代码一律
//    不得依赖它** —— 平台分叉 item 应自行挂精确 cfg（如 `#[cfg(any(not(windows), test))]`）
//    或行级 `#[allow(dead_code)]` 并注明该目标为何用不到。改完可用「临时注释本行 →
//    cargo check --all-targets」复查是否还有被豁免毯掩盖的真死代码。
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

// 运行期配置文档：`<数据目录>/settings.json` —— 设置 + 窗口几何**同住这一个文件**。
// 读-改-写工具与文件契约（键序 / 兼容性 / 落点）都在这里，两端（Tauri / Electron）同契约。
mod store;

// 清除浏览数据（缓存 / Cookie / 本地存储 …）：设置页「清除全部」按钮的后端。
// ⚠️ 模块与命令都**不按平台 gate**：`invoke_handler` 是跨平台的注册表，Android 上少了这条
// 命令，设置页的调用会变成「命令不存在」而不是一条能看懂的错误。平台差异收敛在模块内部
// （非 Windows 返回明确的「不支持」）。详见该模块文档。
mod web_data;

// 启动过渡：本地启动页（splash 自定义协议）+ 注入到每个文档的加载层脚本。
mod loading;

// 导航 / 窗口会话登记数据枢纽（架构审查 2026-10-25 P2-1）：功能模块一律 `→ nav`，
// 不再上行依赖 bootstrap；nav → config/store 单向。
mod nav;
use nav::ProxyTask;

// 启动编排（boot_entry / prepare_navigation / navigate_main）：唯一对外入口 boot_entry
// 反向经 nav 的 boot 回调转发（接线见 setup 里的 register_boot_entry）。
mod bootstrap;

// Windows 专属：WebView2 原生事件钩子。
#[cfg(windows)]
mod webview_hooks;

// Windows 专属：系统「另存为」对话框（下载时由用户决定落盘位置）。
#[cfg(windows)]
mod save_dialog;

// 多页签（tab）模式（仅 Windows）。
#[cfg(windows)]
mod tabs;

// 移动端底部导航栏（`mobile_bar`，仅移动端；`test` 那一半让本机也能跑它的纯逻辑单测）。
// ⚠️ 模块头写了完整的 cfg 理由（生产只在移动端编译，桌面端连这条路径都不存在）。
#[cfg(any(mobile, test))]
mod mobilebar;

// 桌面悬浮球（仅 Windows）：透明置顶小球，点击调出主窗 / 可拖动 / 靠边半隐藏 / 悬停展开。
#[cfg(windows)]
mod floatwin;

// 悬浮球**真机合成指针 harness**（仅 Windows；只有 `cargo run -- --float-probe` 会跑到）。
// 平时只是多一个不参与编译的模块（Windows 上编译进二进制但不被调用），不建窗、不联网。
#[cfg(windows)]
mod floatprobe;

// 系统托盘 / 设置命令 / 窗口切换（仅桌面端）。
#[cfg(desktop)]
mod tray;

// 窗口几何持久化（仅桌面端）。
#[cfg(desktop)]
mod window_state;

use settings::SettingsState;

/// 设置页「立即显示悬浮图标」按钮调用的命令。
///
/// ⚠️ 刻意做成【跨平台注册】的命令：`dist/settings.html` 是跨平台资源，契约测试
/// `every_frontend_invoked_command_is_registered` 在所有编译目标上都会扫描它 invoke 的命令名，
/// 若把本命令标成 `#[cfg(windows)]`，非 Windows 目标的契约测试 / IPC 表就会缺项。实际行为只在
/// Windows 存在（悬浮球模块整体 `#[cfg(windows)]`），其他平台调用是空操作——该开关在那些平台
/// 只是一个被持久化的布尔值。
#[tauri::command]
fn show_float_window(app: tauri::AppHandle) {
    #[cfg(windows)]
    floatwin::show_float_window(&app);
}

/// 设置页「退出程序」按钮调用的命令（v1.75）。
///
/// 为什么需要它：启用托盘或悬浮图标后，「关闭窗口」按设计只等于**隐藏**（应用继续驻留），
/// 而唯一的退出入口一直是托盘的「退出」菜单项 —— 没启用托盘的用户于是**根本没有退出路径**
/// （2026-10-11 使用者实测：只启用悬浮图标时，关窗后球还在、点球又不回窗口）。
/// Shift+F1 能随时打开设置页，把退出入口放在那里最稳。
///
/// ⚠️ 与托盘「退出」（`tray.rs` 的 `"quit"` 分支）走**同一条路径** `app.exit(0)`：
///    `AppHandle::exit` → `RunEvent::ExitRequested` → `RunEvent::Exit`，**不经过任何窗口的
///    `CloseRequested`** ⇒ 上面 `handle_main_close` 的「驻留」分支不会把退出吞掉。
///    （这正是本仓库 v1.68 缺陷②在 Electron 侧的形态：close 被 preventDefault 会取消
///    整个 `app.quit()`。Tauri 侧走 exit 天然免疫，但必须写下来，免得将来有人改成
///    `window.close()`。）窗口几何 / 悬浮球几何照常落盘、代理任务照常收尾 ——
///    这些都挂在 `RunEvent::ExitRequested` 上。
///
/// ⚠️ 必须【跨平台注册】：`dist/settings.html` 是跨平台资源，契约测试
///    `every_frontend_invoked_command_is_registered` 在任何编译目标上都会扫描其中的 invoke
///    命令名。移动端没有设置窗（`open_settings` 是 `cfg(desktop)`），故端上行为无实际影响。
#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

/// 是否以「悬浮球真机探针」模式启动（`--float-probe`；实现见 [`floatprobe`]）。
///
/// ⚠️ 刻意做成**函数**而不是 `run()` 里的一个 `bool` 变量：那个变量在非 Windows / 非桌面上
/// 会变成「未使用变量」，在 `-D warnings` 下直接编译失败（历史上踩过同款 cfg-only 陷阱）。
/// 函数只在有调用点的地方被引用，Android 上它只是一个被 `#![allow(dead_code)]` 放过的死函数。
fn probe_mode() -> bool {
    #[cfg(windows)]
    {
        floatprobe::is_probe_mode()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

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
        // 设置页：编译期内嵌 dist/settings.html，经自定义协议提供（不依赖 devUrl，
        // dev / release 行为完全一致；处理器实现见 settings::handle_settings）。
        .register_uri_scheme_protocol("settings", settings::handle_settings)
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
            tabs::minimize_host,
            #[cfg(windows)]
            tabs::close_host,
            // 设置页「立即显示悬浮图标」按钮。⚠️ 必须【跨平台注册】：dist/settings.html 是
            // 跨平台资源，契约测试 every_frontend_invoked_command_is_registered 会在所有目标上
            // 扫描其中的 invoke 名；命令体在非 Windows 是空操作（见下方 show_float_window）。
            show_float_window,
            // 设置页「退出程序」按钮（v1.75）：常驻后台（托盘 / 悬浮图标）时唯一稳当的退出入口。
            // 同样跨平台注册，理由同上。
            quit_app
        ]);

    // 多页签模式（仅 Windows）：注册 shell 标签栏协议 + TabManager 状态。
    // 悬浮球（仅 Windows）：只托管 FloatState（吸附 / 拖动 / 单击去抖）。v1.81 起球窗是
    // Win32 分层窗口（见 floatwin/native.rs），不再有球页 ⇒ floaticon 协议一并退役。
    // ⚠️ IPC 命令已并入上面**唯一一次** `invoke_handler`（见那段注释），别再在这里开第二次。
    #[cfg(windows)]
    let builder = builder
        .register_uri_scheme_protocol(tabs::SHELL_SCHEME, tabs::handle_shell)
        .manage(tabs::TabManager::default())
        .manage(floatwin::FloatState::default());

    // 首个文档「加载完成」登记表：非 Windows（Android）启动期必须等它再导航 ——
    // 不等就会被 wry「post 出去的 splash 加载」覆盖掉导航（根因见 `nav::BootReady` 长注释）。
    // Windows 不加此等待，故把状态一起 cfg 掉（留着会变成无人使用的死代码）。
    #[cfg(not(windows))]
    let builder = builder.manage(nav::BootReady::default());

    // 桌面插件（全局快捷键 / 单实例）。
    // ⚠️ 探针模式（`--float-probe`）**必须**跳过这两个插件，尤其是单实例：本机已有实例在跑
    //    时，单实例插件会把启动参数转交给那个实例并让**本进程直接退出** —— 探针于是「零输出、
    //    退出码 0」，从外面看像全部通过，实际一条断言都没跑（假绿，最坏的一种失败）。
    //    全局快捷键同理无意义：探针不建托盘、不建主窗、也不需要唤醒键。
    #[cfg(desktop)]
    let builder = if probe_mode() {
        builder
    } else {
        let settings_open = settings::settings_open_shortcut();
        builder
            .plugin(
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
            )
            // 单实例：第二个实例启动时不再新开窗口，而是把已存在的窗口带到前台。
            // （仅桌面端；Android 无此概念，故按 target_os 剔除。）
            .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
                // 多页签模式聚焦宿主窗口，单窗口模式聚焦 "main"。
                let label = nav::main_window_label();
                if let Some(w) = app.get_webview_window(label) {
                    let _ = w.unminimize();
                    let _ = w.show();
                    let _ = w.set_focus();
                }
                // 多页签模式：tab 窗口是独立顶层窗口，宿主恢复显示时要一并把当前 tab 显示出来
                // （否则只看到标签栏，内容区是空的）。
                tray::sync_tabs_visibility(app, true);
            }))
    };

    let builder = builder.setup(move |app| {
        // ⚠️ 诊断日志要**尽早**初始化：越早越能捕获启动早期的日志。
        //    默认（debug=false）时它是空操作 —— 不建目录、不落文件、也不打印任何东西。
        debug_log::init(app.handle());

        // ---------- 控制反转接线（架构审查 P2-1）：nav 保存上层动作的回调 ----------
        // 装配层是唯一允许「认识所有人」的地方：`nav` 只持有 fn 指针，启动编排入口与
        // WebView2 钩子装配在此登记（不接线则 nav 侧静默降级并留 dlog，理由见 nav 模块头）。
        nav::register_boot_entry(bootstrap::boot_entry);
        #[cfg(windows)]
        nav::register_attach_hooks(webview_hooks::attach_webview_hooks);

        // ---------- 悬浮球真机探针模式（`--float-probe`，仅 Windows）----------
        // 只跑悬浮球 harness，其余一律不建：不建主窗（探针自己建一个 label="main" 的假主窗
        // 作为 toggle 断言的观察点）、不启 bootstrap（不解析域名 / 不起本地代理 / 不联网）、
        // 不建托盘、不注册快捷键。见 [`floatprobe`] 模块文档。
        #[cfg(windows)]
        if probe_mode() {
            floatprobe::run(app.handle().clone());
            return Ok(());
        }

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

        // ---------- 多页签：tab 真实标题回传事件（仅 Windows） ----------
        // tab 页面是远程 origin，应用命令调不了（见 tabs::report_tab_title 的注释），
        // tab_title.js 改发核心事件；事件名 / payload 形状由静态契约测试钉着。
        // ⚠️ 监听不带发送者标识，伪造防护在 payload 的一次性 token（见 report_tab_title）。
        #[cfg(windows)]
        {
            let handle = app.handle().clone();
            let title_handle = handle.clone();
            app.listen(tabs::TAB_TITLE_EVENT, move |event| {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct TabTitlePayload {
                    label: String,
                    token: String,
                    title: String,
                }
                match serde_json::from_str::<TabTitlePayload>(event.payload()) {
                    Ok(p) => tabs::report_tab_title(&title_handle, p.label, p.token, p.title),
                    Err(e) => dlog!("[tabs] 标题回传事件 payload 无法解析：{e}"),
                }
            });

            // tab 首屏「重试」：本地占位页上的加载层按钮发出（loading.js），重走该 tab
            // 的 bootstrap。payload 形状与标题事件相同（label + 一次性 token）。
            let retry_handle = handle.clone();
            app.listen(tabs::TAB_RETRY_EVENT, move |event| {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct TabRetryPayload {
                    label: String,
                    token: String,
                }
                match serde_json::from_str::<TabRetryPayload>(event.payload()) {
                    Ok(p) => tabs::retry_tab_bootstrap(&retry_handle, p.label, p.token),
                    Err(e) => dlog!("[tabs] 首屏重试事件 payload 无法解析：{e}"),
                }
            });
        }

        // ---------- 移动端底栏（仅移动端）：注册首页导航缓存 + 三个核心事件监听 ----------
        // ⚠️ 必须在 bootstrap 之前就绪：首页按钮在「启动还没解析出地址」时也会被调用
        //    （用户停在启动页就点了），那时它要走「重跑一次 bootstrap」那条分支。
        #[cfg(mobile)]
        crate::mobilebar::setup(app.handle());

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
            .background_color(nav::SPLASH_BG)
            // ⚠️⚠️ 模式标记必须排在 OVERLAY_SCRIPT【之前】（初始化脚本按注册顺序执行），
            // 与 `tabs.rs::create_tab_window` 同一条契约（那里钉的是 tauri.localhost）。
            //
            // 首个文档是启动页：`splash://localhost/` —— 会被 wry 的 custom_protocol_workaround
            // 改写成 `http://splash.localhost/`（Windows 与 Android 都是；`use_https_scheme`
            // 默认 false，本项目未开）。它必须让 loading.js 以 **splash** 语义运行：
            //   · splash 语义：SLOW 5s / FAIL 40s 的「启动超时 + 重试」，**不装就绪监听**；
            //   · overlay 语义：SLOW 8s / FAIL 30s，且 startReadyWatch() 会装空白文档硬上限
            //     （CAP_MS=15s）—— 启动页 body 里只有 <script>，`bodyEmpty()` 恒真，
            //     于是【15 秒必然】弹出「页面长时间为空白」失败卡，把一次正常但偏慢的
            //     启动（DNS/TXT 往返 + 代理起停可能十几秒）误报成加载失败。
            //
            // ⚠️ 不能指望 `splash.html` 里内联的那行 `__web2appLoadingMode = 'splash'`：
            // 它在 <body> 里，永远晚于 document-start 注入的这段脚本。Electron 侧之所以
            // 一直没事，是因为它的 preload 对内部页面**整体跳过注入**、启动页只跑内联脚本，
            // 内联顺序才恰好成立（见 electron-app/src/renderer/preload.ts 的 isInternalPage）。
            .initialization_script(
                "window.__web2appLoadingMode = (location.hostname === 'splash.localhost' \
                 || location.protocol === 'splash:') ? 'splash' : 'overlay';",
            )
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

            // ⚠️⚠️ 非 Windows（Android）：注册**首个文档加载完成**信号 —— 启动期必须先等它
            // 再解析 / 推送 / 导航（根因与真机证据见 `nav::BootReady` 的长注释）。
            // 为什么用 `on_page_load` 而不是别的判据：wry Android 的 `ON_LOAD_HANDLER` 正是由
            // `RustWebViewClient.onPageStarted / onPageFinished` 驱动的，这是 Android 侧唯一
            // 可靠的「文档已就绪」信号；而 `GetUrl` 之类同样走 main-pipe，webview 未就绪时拿不到结果。
            #[cfg(not(windows))]
            let wb = wb.on_page_load(|window, payload| {
                let finished = matches!(payload.event(), tauri::webview::PageLoadEvent::Finished);
                nav::on_page_load(window.app_handle(), window.label(), finished);
                // 移动端单 webview 页签：每次真实文档加载完成，把最终 url 记进活动标签
                //（标题由 mobilebar.js 的 web2app:mobile-doc 事件另补；启动页 url 在其内部被忽略）。
                // ⚠️ 顺序与旧实现（原 bootstrap::on_page_load 内联）一致：先经 nav::on_page_load
                //    置位 BootReady，再记页签；这里不放回 nav 是为避免 nav → mobilebar 的层级环。
                #[cfg(mobile)]
                if finished {
                    crate::mobilebar::on_page_finished(window.app_handle());
                }
            });

            // ---------- 移动端底栏（`mobile_bar=true`）----------
            // ⚠️⚠️ cfg 脚本**必须**排在 mobilebar.js **之前**：初始化脚本按注册顺序执行，
            //    而 mobilebar.js 开头要读 `window.__web2appMobileBarCfg`（高度 / 是否有页签）。
            //    顺序反了它只会落进「默认值」分支 —— 底栏照画，但高度与按钮清单不再由壳决定，
            //    是最难从表象反推的一类错位（与上面「模式标记必须排在 OVERLAY_SCRIPT 之前」
            //    同一条契约，由 contract_tests::mobile_bar_contract 钉住）。
            #[cfg(mobile)]
            let wb = if crate::config::MOBILE_BAR {
                wb.initialization_script(crate::mobilebar::config_script())
                    .initialization_script(include_str!("mobilebar.js"))
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
            nav::bootstrap(app.handle().clone());
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
            nav::bootstrap(app.handle().clone());
        }

        // ---------- 桌面悬浮球（仅 Windows）----------
        // 设置勾选时建球。v1.81 起球窗是 Win32 分层窗口，交互全在它自己的 WndProc 里，
        // 不再需要「球页 → Rust」的事件监听注册（register_float_listeners 已退役）。
        #[cfg(windows)]
        {
            if initial_settings.float_enabled {
                floatwin::show_float_window(app.handle());
            }
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

        // 启动信息（实现已迁至 debug_log 模块：架构审查 P1-1）：与 Electron 侧
        // `index.ts::logStartup()` 一一对应，便于两壳的日志互相印证。
        // 默认 debug=false 时这里什么都没发生。
        debug_log::log_startup(app.handle(), &initial_settings);

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
                    // ⚠️ 悬浮球的 Alt+F4 / 系统关闭不再经过这里（v1.81 球窗是原生 Win32 窗口）：
                    //    `WM_CLOSE` 在 floatwin/native.rs 的 WndProc 里拦成「隐藏 + 落盘几何」；
                    //    球上右键走 `floatwin::disable_float_from_icon`（关功能 + 销毁 + 唤回主窗）。

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
                        window_state::persist_label(app_handle, nav::main_window_label());
                    }
                    // 悬浮球几何（吸附边 + 展开态坐标），仅 Windows 有该窗口。
                    #[cfg(windows)]
                    floatwin::persist_float_geometry(app_handle);
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

/// 处理「主窗口关闭」：满足任一**驻留理由** → 隐藏窗口、应用继续驻留；否则允许关闭
/// （最后一个窗口关闭后应用退出）。
///
/// 两个驻留理由（v1.75 起并列）：
///   1. `tray_enabled && minimize_on_close` —— 隐藏到托盘；
///   2. `float_enabled`（仅 Windows）—— 隐藏到悬浮图标。
///
/// ⚠️⚠️ 第 2 条不是「锦上添花」而是**修一个真机缺陷**（2026-10-11 使用者实测）：
///   只启用悬浮图标（未启用托盘）时，关闭窗口走的是 `else` 分支 ⇒ 主窗被**真销毁**，
///   而悬浮球窗仍活着 —— 于是 ① 球挡住了「所有窗口关闭 ⇒ 退出」，进程留成一个既无界面、
///   也退不掉的僵尸；② 点球时 `tray::toggle_window` 里 `get_webview_window("main")`
///   返回 `None`，**一个动作都不做**，用户看到的是「点了悬浮图标不回窗口」。
///   球在产品语义上就是「唤回主窗」的入口（`floaticon.html` 的 aria-label 亦如此写），
///   故「球在 ⇒ 应用驻留」必须成立，与托盘驻留同一条纪律。
///
/// `target_label` 由调用方决定：单窗口模式为 `"main"`，多页签模式为宿主窗口 label。
/// 这样 `window_state::persist_label` 记录的也是正确窗口的几何。
///
/// ⚠️ 本函数【不可】加 `cfg(desktop)`：Android 也会走到这里读
/// SettingsState 判断托盘语义（托盘未启用时等价于允许关闭）。只有「窗口几何持久化」
/// 依赖的 `window_state` 模块才是桌面端专属，故 cfg 下沉到那一行。
fn handle_main_close(app_handle: &tauri::AppHandle, api: tauri::CloseRequestApi) {
    let target_label = nav::main_window_label().to_string();

    // 记录主窗口当前大小/位置，供下次启动恢复（仅桌面端；Android 无窗口几何持久化）。
    #[cfg(desktop)]
    window_state::persist_label(app_handle, &target_label);

    let st = app_handle.state::<SettingsState>();
    let (tray, min, float_icon) = {
        let s = st.0.lock().unwrap_or_else(|p| p.into_inner());
        (s.tray_enabled, s.minimize_on_close, s.float_enabled)
    };
    // 驻留理由：托盘（两个开关都要）或悬浮图标（仅 Windows 有球）。
    // ⚠️ 用 `cfg!` 布尔而**不是** `#[cfg]` 分叉：非 Windows 上 `float_icon` 同样要被用到，
    //    分叉会把它变成「未使用的变量」⇒ `-D warnings` 直接编译失败
    //    （本仓库踩过同款 cfg-only 陷阱，见 contract_tests 的 frontend_sources 注释）。
    let stay_resident = (tray && min) || (cfg!(windows) && float_icon);
    if stay_resident {
        api.prevent_close();
        if let Some(w) = app_handle.get_webview_window(&target_label) {
            let _ = w.hide();
        }
        // 多页签模式：tab 窗口是独立顶层窗口，宿主隐藏时**必须一并隐藏**，
        // 否则桌面上会残留没有标签栏的“孤儿内容窗口”，用户会以为应用卡死了。
        //（`tray` 是 cfg(desktop)；Android 无 tabs，本就是空操作，行为不变。）
        #[cfg(desktop)]
        tray::sync_tabs_visibility(app_handle, false);
    } else {
        // 允许关闭（不隐藏到托盘）：tab 窗口同样不会随宿主销毁，必须显式清掉，
        // 否则「标签栏关了、内容还在」且 Tauri 因为仍有窗口而不触发退出。
        #[cfg(windows)]
        if crate::tabs::tabs_enabled() {
            crate::tabs::destroy_all_tabs(app_handle);
        }
    }
}

/// 后退守卫脚本（注入目标 webview 执行）：仅当相邻的上一条历史是【真实网页】时才执行
/// `history.back()`。应用启动时内置 splash 页是第一条历史，直接后退会退到它，
/// 故先用 Navigation API 检查上一条目 URL；没有 Navigation API（极老内核）时宁可不退。
/// 规则与注入脚本 `titlebar.js` 的 navDirections() 完全一致（电脑端 / 移动端同规则）。
const HISTORY_BACK_SCRIPT: &str = r#"(function(){try{
var n=window.navigation;
if(!n||!n.currentEntry||typeof n.currentEntry.index!=='number')return;
var i=n.currentEntry.index;
if(i<=0)return;
var e=n.entries()[i-1];
var u=e&&e.url?String(e.url):'';
if(/^splash:/i.test(u)||/^https?:\/\/splash\.localhost(?:[/?#]|$)/i.test(u))return;
window.history.back();
}catch(x){}})();"#;

/// 前进守卫脚本：同理，下一条目不存在或落点是内置 splash 页时不触发 `history.forward()`。
const HISTORY_FWD_SCRIPT: &str = r#"(function(){try{
var n=window.navigation;
if(!n||!n.currentEntry||typeof n.currentEntry.index!=='number')return;
var e=n.entries()[n.currentEntry.index+1];
if(!e)return;
var u=e.url?String(e.url):'';
if(/^splash:/i.test(u)||/^https?:\/\/splash\.localhost(?:[/?#]|$)/i.test(u))return;
window.history.forward();
}catch(x){}})();"#;

/// 标题栏「后退 / 前进」命令：对【当前】窗体对应的 webview 执行历史导航。
///
/// 目标窗体的确定：
/// - 多页签模式（仅 Windows）：当前激活的 tab 窗口（标题栏按钮在宿主标签栏左侧）。
/// - 单窗口模式：主窗 "main"（自绘标题栏注入到主窗每个文档）。
///
/// 注意不是裸执行 `window.history.back()`：脚本 [`HISTORY_BACK_SCRIPT`] 会先用
/// Navigation API 判定后退落点，没有真实上一页（或落点是内置 splash）时静默不动作。
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

    // 仅接受 'back' / 'forward' 两个方向，防误传；脚本本身带落点守卫。
    let script = match direction.as_str() {
        "back" => HISTORY_BACK_SCRIPT,
        "forward" => HISTORY_FWD_SCRIPT,
        _ => return Err("无效的历史导航方向（仅支持 back / forward）".to_string()),
    };

    window
        .eval(script)
        .map_err(|e| format!("历史导航失败：{e}"))
}

// 静态契约测试（IPC 注册表唯一调用点 / capability 授权 / 跨文件事件字面量）整组迁出至
// contract_tests/（2026-10-25 按领域拆为 12 个子模块）：入口只留生产代码（架构评审 P1-1）。
// 子模块位于 src/contract_tests/（比被测文件低一级），include_str! 路径统一带一层 ../。
#[cfg(test)]
mod contract_tests;
