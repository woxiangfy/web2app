//! 静态契约测试（只在 `cargo test` 下编译，由 lib.rs 的 `#[cfg(test)] mod contract_tests;` 挂入）：
//!   · `ipc_command_registry` —— IPC 命令注册表唯一调用点 + 前端命令 ⊆ 注册表；
//!   · `capability_contract` —— capabilities 授权 + Rust/JS 两侧事件名字面量；
//!   · `top_bar_style_contract` —— 多页签标签栏与单窗口标题栏的关键配色逐字一致；
//!   · `webview2_password_autosave` —— WebView2 密码自动保存开关（Windows）；
//!   · `loading_mode_contract` —— 加载层模式标记必须在 OVERLAY_SCRIPT 之前注册（跨平台）。
//!
//! 2026-10-03 从 lib.rs 整体迁出：入口文件不该混装数百行测试（既有架构评审 P1-1）。
//! ⚠️ 本文件与 lib.rs 同处 `src/` 目录 ⇒ 所有 `include_str!` 的相对路径与迁出前**逐字相同**
//!    （`"lib.rs"`、`"settings.rs"`、`"../capabilities/..."` 等都不用改）；
//!    include 进来的就是被测文件本体，将来若再移动本文件，必须同步调整这些路径。

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

    /// 平台专属的前端页面：多页签标签栏 —— 它们调用的命令本身也是 `#[cfg(windows)]` 注册的。
    ///
    /// ⚠️ `tab_title.js` **不在**这里：它注入在远程 tab 页面里，2026-09-27 起只发核心
    /// 事件（`plugin:event|emit`）、不调任何应用命令（远程 origin 调应用命令必被 ACL 拒）。
    /// 它的契约在 `capability_contract` 模块里按「事件名 + capability + token」另立守卫。
    ///
    /// ⚠️⚠️ 平台差异只允许落在**数据**上，绝不能落在**可变性**上。早先这里写的是
    /// `let mut sources = vec![跨平台页面];` + `#[cfg(windows)] { sources.push(页签页面) }`，
    /// 而那句 `push` 在**非 Windows 目标**（CI 的 Ubuntu `lint-and-test` / Android 作业）
    /// 上根本不参与编译 ⇒ `mut` 变成未使用 ⇒ `-D warnings` 下直接编译失败
    /// （2026-09-23 CI 报 `error: variable does not need to be mutable`）。
    /// 改写成「cfg 分叉只决定数组内容」后，任何目标上都不可能再出现 unused_mut。
    #[cfg(windows)]
    const PLATFORM_SOURCES: &[(&str, &str)] = &[("src/shell.html", include_str!("shell.html"))];
    /// 非 Windows 目标没有页签页面（命令未注册，验了必然假失败）⇒ 空表。
    #[cfg(not(windows))]
    const PLATFORM_SOURCES: &[(&str, &str)] = &[];

    /// 待校验的全部前端页面：跨平台页面（设置页）+ 平台专属页面（页签）。
    fn frontend_sources() -> Vec<(&'static str, &'static str)> {
        std::iter::once(("dist/settings.html", include_str!("../dist/settings.html")))
            .chain(PLATFORM_SOURCES.iter().copied())
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
        // 页面清单见 `frontend_sources()`（平台差异只影响清单内容，不影响可变性）。
        let sources = frontend_sources();

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
            // 设置页「立即显示悬浮窗」按钮：跨平台注册（settings.html 是跨平台资源），
            // 命令体在非 Windows 是空操作，见 lib.rs 的 show_float_window。
            "show_float_window",
            // 设置页「退出程序」按钮（v1.75）：常驻后台（托盘 / 悬浮图标）时**唯一**的退出入口，
            // 同样必须跨平台注册（settings.html 是跨平台资源）。
            "quit_app",
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
    /// 多页签 tab 窗口（远程内容）能力声明 —— include_str! 同样把「文件缺失」变成编译失败。
    #[cfg(windows)]
    const TABS_CAP: &str = include_str!("../capabilities/tabs-remote.json");
    /// 多页签模块：事件名常量与标题回传落地函数在这里。
    #[cfg(windows)]
    const TABS_RS: &str = include_str!("tabs.rs");
    /// 注入到每个 tab 远程页面的标题回传脚本。
    #[cfg(windows)]
    const TAB_TITLE_JS: &str = include_str!("tab_title.js");
    /// 注入到每次导航的加载层脚本（tab 首屏重试事件也由它发出）。
    #[cfg(windows)]
    const LOADING_JS: &str = include_str!("loading.js");

    /// 「打开设置」事件名两侧的字面量写法。
    /// ⚠️ 必须**拆成两段拼接**：本文件就是被测对象之一，直接写完整字面量会让断言
    /// **自我满足**（`ipc_command_registry` 首轮踩过同一个坑）。
    const EVENT_IN_LIB: &str = concat!("app.listen(\"", "web2app:open-settings", "\"");
    const EVENT_IN_TITLEBAR: &str = concat!("'", "web2app:open-settings", "'");

    /// tab 标题回传事件：tabs.rs 常量定义、tab_title.js 事件名、lib.rs 监听点三处逐字一致。
    #[cfg(windows)]
    const TAB_EVENT_IN_RUST: &str = concat!("\"", "web2app:tab-title", "\"");
    #[cfg(windows)]
    const TAB_EVENT_LIB_LISTEN: &str = "app.listen(tabs::TAB_TITLE_EVENT";
    #[cfg(windows)]
    const TAB_EVENT_IN_JS: &str = concat!("'", "web2app:tab-title", "'");

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

    /// ④ tab 远程页 capability：只给 tab-* 窗口 + 远程 http(s) URL + 最小权限 event:emit。
    #[cfg(windows)]
    #[test]
    fn tabs_capability_grants_only_remote_event_emit() {
        let v: serde_json::Value =
            serde_json::from_str(TABS_CAP).expect("capabilities/tabs-remote.json 不是合法 JSON");
        let windows = str_list(&v, "windows", "capabilities/tabs-remote.json");
        assert_eq!(
            windows,
            ["tab-*"],
            "tab capability 必须且只能用 glob 命中 tab-N 窗口（实际：{windows:?}）"
        );
        // remote.urls：代理入口是随机端口的 http://127.0.0.1:<port>，直连是 https 上游，
        // 两个带端口通配的模式缺一不可（default.json 顶部注释解释过「不写端口=只匹配默认端口」）。
        let remote_urls = v["remote"]["urls"]
            .as_array()
            .expect("tabs-remote.json 缺 remote.urls 数组");
        for want in ["http://*:*", "https://*:*"] {
            assert!(
                remote_urls.iter().any(|u| u.as_str() == Some(want)),
                "tabs-remote.json 的 remote.urls 必须包含 {want}（实际：{remote_urls:?}）"
            );
        }
        let (_, perms) = cap(TABS_CAP, "capabilities/tabs-remote.json");
        assert_grants(
            "capabilities/tabs-remote.json",
            &perms,
            "core:event:allow-emit",
        );
        // 最小权限：tab 无自绘标题栏，窗口控制类权限一律不得出现。
        for forbidden in [
            "core:window:allow-minimize",
            "core:window:allow-close",
            "core:window:allow-start-dragging",
        ] {
            assert!(
                !perms.iter().any(|p| p == forbidden),
                "tab capability 不应授予 {forbidden}（tab 内容页没有窗口按钮，最小权限）"
            );
        }
    }

    /// tab 标题回传事件：tabs.rs 常量 ↔ lib.rs 监听 ↔ tab_title.js 字面量三处逐字一致；
    ///    且脚本必须走事件 + 令牌，而不是（对远程 origin 必死的）应用命令。
    #[cfg(windows)]
    #[test]
    fn tab_title_event_contract_between_lib_tabs_and_script() {
        assert!(
            TABS_RS.contains(TAB_EVENT_IN_RUST),
            "tabs.rs 必须定义事件名字面量 {TAB_EVENT_IN_RUST}（TAB_TITLE_EVENT）"
        );
        assert!(
            LIB_RS.contains(TAB_EVENT_LIB_LISTEN),
            "lib.rs 必须用 app.listen(tabs::TAB_TITLE_EVENT, …) 监听标题回传"
        );
        assert!(
            TAB_TITLE_JS.contains(TAB_EVENT_IN_JS),
            "tab_title.js 必须包含事件名 {TAB_EVENT_IN_JS}"
        );
        assert!(
            TAB_TITLE_JS.contains("plugin:event|emit"),
            "tab_title.js 必须走核心事件通路（__TAURI_INTERNALS__ 兜底）"
        );
        assert!(
            TAB_TITLE_JS.contains("__web2appTabToken"),
            "tab_title.js 必须回传建窗时注入的一次性令牌（防伪造就靠它）"
        );
        assert!(
            !TAB_TITLE_JS.contains("set_tab_title"),
            "tab_title.js 不得再 invoke set_tab_title：远程 origin 的应用命令必被 ACL 拒"
        );
    }

    /// 标题回传只允许来自【主框架】：initialization_script 会被注入所有 frame
    /// （WebView2 AddScriptToExecuteOnDocumentCreated 无 frame 过滤），跨域广告 iframe
    /// 自己的 document.title（实测 bilibili 页内为 "Cross Origin Local Storage" →
    /// "Leader Iframe" → "cm.bilibili.com"）会顶掉标签栏真实标题。两处守卫必须同时存在：
    ///   1. tab_title.js 读到 label/token 之前先 `window.self !== window.top` 早退；
    ///   2. tabs.rs 只在主框架挂 __web2appTabLabel / __web2appTabToken（令牌不泄露给
    ///      第三方 frame，子框架即使被诱导也拿不到令牌）。
    #[cfg(windows)]
    #[test]
    fn tab_title_script_runs_in_top_frame_only() {
        assert!(
            TAB_TITLE_JS.contains("window.self !== window.top"),
            "tab_title.js 必须在子框架早退：初始化脚本会注入所有 frame，跨域广告 iframe 的 \
             document.title（如 bilibili 的 \"Leader Iframe\"）会顶掉标签栏真实标题"
        );
        // 两条注入各出现一次主框架守卫（拆成两个子串断言，避免只改了 label 漏掉 token）。
        assert!(
            TABS_RS.contains("window.self === window.top") && {
                let guards = TABS_RS.matches("window.self === window.top").count();
                guards >= 2
            },
            "tabs.rs 注入 __web2appTabLabel / __web2appTabToken 时必须都带主框架判断，\
             避免一次性防伪令牌暴露给第三方跨域 iframe"
        );
        assert!(
            TABS_RS.contains("window.__web2appTabLabel")
                && TABS_RS.contains("window.__web2appTabToken"),
            "tabs.rs 仍需注入 label/token（仅加上主框架门控，不得误删注入本身）"
        );
    }

    /// ⑥ tab 首屏重试事件：tabs.rs 常量 ↔ lib.rs 监听 ↔ loading.js 字面量逐字一致；
    ///    loading.js 必须带 label/token（占位文档是本地源，令牌仍是唯一防伪手段）。
    #[cfg(windows)]
    #[test]
    fn tab_retry_event_contract_between_lib_tabs_and_loading() {
        const RETRY_IN_RUST: &str = concat!("\"", "web2app:retry-bootstrap", "\"");
        assert!(
            TABS_RS.contains(RETRY_IN_RUST),
            "tabs.rs 必须定义 TAB_RETRY_EVENT 字面量 {RETRY_IN_RUST}"
        );
        assert!(
            LIB_RS.contains("app.listen(tabs::TAB_RETRY_EVENT"),
            "lib.rs 必须监听 tabs::TAB_RETRY_EVENT"
        );
        assert!(
            LOADING_JS.contains("'web2app:retry-bootstrap'"),
            "loading.js 必须包含重试事件名 'web2app:retry-bootstrap'"
        );
        assert!(
            LOADING_JS.contains("__web2appTabLabel") && LOADING_JS.contains("__web2appTabToken"),
            "loading.js 重试事件必须携带注入的 label 与 token"
        );
        assert!(
            LOADING_JS.contains("plugin:event|emit"),
            "loading.js 重试必须走核心事件通路"
        );
    }
}

/// 顶部通栏配色的静态契约测试。
///
/// 多页签标签栏（`src/shell.html`）与单窗口标题栏（`src/titlebar.js`）是**同一块 UI** 的两种
/// 形态：前者铺在宿主窗口顶部、后者叠在主窗口顶部。配色/分隔线一旦漂移，两种模式看起来就不像
/// 同一个应用（使用者 2026-10-08 反馈「多页签顶部通栏风格要与单页签的一致」）。这里把两处共用的
/// 关键色值钉死 —— 改配色必须两边一起改，否则本测试报错。
#[cfg(test)]
mod top_bar_style_contract {
    /// 多页签顶栏（宿主窗口 decorations(false) + 自绘标签栏）。
    const SHELL: &str = include_str!("shell.html");
    /// 单窗口 / 多页签 tab 页的自绘标题栏。
    const TITLEBAR: &str = include_str!("titlebar.js");

    /// 两处顶栏必须共用同一套关键色：浅底 / 深底 / 正文 / 深色正文 / 浅色分隔线 / 深色分隔线 /
    /// 按钮 hover 底色。字面量在两份文件里都写成小写、无多余空格，便于逐字比对。
    #[test]
    fn tab_bar_palette_matches_titlebar() {
        const SHARED: &[&str] = &[
            "#eef0f2",               // 浅色底
            "#1f2530",               // 深色底
            "#2c3440",               // 浅色正文
            "#e6ebf2",               // 深色正文
            "rgba(0,0,0,.08)",       // 浅色下沿分隔线
            "rgba(255,255,255,.07)", // 深色下沿分隔线
            "rgba(120,133,153,.16)", // 按钮 hover 底色
        ];
        for lit in SHARED {
            assert!(
                TITLEBAR.contains(lit),
                "titlebar.js 缺少顶栏配色 {lit}（本测试的真源之一）"
            );
            assert!(
                SHELL.contains(lit),
                "shell.html 缺少与 titlebar.js 一致的顶栏配色 {lit}：\
                 多页签标签栏与单页签标题栏必须同色（改配色请两份文件一起改）"
            );
        }
    }
}

/// WebView2 网页密码自动保存开关的静态契约（Windows）。
///
/// 背景：WebView2 的 `IsPasswordAutosaveEnabled` **默认 false** —— 不弹「保存/更新密码？」
/// 气泡、不保存新密码（一般表单自动填充是另一个独立开关、默认 true），与 Electron 壳的
/// Chromium 默认开启不对齐。`webview_hooks.rs` 必须在每个 CoreWebView2 的钩子注册阶段把它
/// 显式置 true。COM 调用无法在单测里执行，这里用源码断言把「开关确实接上、且置的是 true」
/// 钉死，防止误删或误改成 false。
#[cfg(test)]
mod webview2_password_autosave {
    /// 被测真源：Windows WebView2 钩子注册代码本体。
    const HOOKS_RS: &str = include_str!("webview_hooks.rs");

    #[cfg(windows)]
    #[test]
    fn enabled_via_settings4_on_every_webview() {
        assert!(
            HOOKS_RS.contains("ICoreWebView2Settings4"),
            "密码自动保存必须通过 ICoreWebView2Settings4（Runtime 1.0.902+）操作；\
             不要换成门槛更高的 ICoreWebView2Profile6（1.0.1823+）"
        );
        assert!(
            HOOKS_RS.contains("SetIsPasswordAutosaveEnabled(true)"),
            "register_webview_hooks 必须显式 SetIsPasswordAutosaveEnabled(true)：\
             WebView2 该属性默认 false，不写就不会弹「保存密码」"
        );
    }
}

/// 加载层「模式标记」的静态契约（跨平台）。
///
/// 背景（2026-10-09 排查 Android 打包后「转圈十几秒 → 白屏」时定位的**第一条必然路径**，
/// 与网络无关）：启动页由自定义协议提供（`splash://localhost/`，被 wry 改写成
/// `http://splash.localhost/`），它必须让注入的加载层以 **splash** 语义运行 —— 模式由
/// `window.__web2appLoadingMode` 决定，而该标记必须在 `OVERLAY_SCRIPT` **之前**注入
/// （初始化脚本按注册顺序执行）。
///
/// 漏掉标记 / 顺序写反的代价：**编译期零警告、clippy 与 CI 全绿**，只在运行时变成
/// 「启动页被当成目标页跑」⇒ 装上了 overlay 的空白文档硬上限（CAP_MS=15s；启动页 body 里
/// 只有 `<script>`，`bodyEmpty()` 恒真）⇒ 一次正常但偏慢的启动（DNS/TXT 往返 + 代理起停
/// 可能十几秒）被判「页面长时间为空白」。之所以能骗过所有人：`splash.html` 里内联的
/// `__web2appLoadingMode = 'splash'` 在 `<body>` 里，被 document-start 注入抢先；
/// 而 Electron 侧 preload 对内部页面整体跳过注入（只跑内联脚本），顺序恰好成立 ——
/// 于是同一个缺陷只在 Tauri 壳上现形。
#[cfg(test)]
mod loading_mode_contract {
    /// 被测真源：主窗口建窗处（`run()` 内的闭包）。
    const LIB_RS: &str = include_str!("lib.rs");

    /// ⚠️ 字面量必须**拆成两段拼接**：直接写全量会让断言在「标记被删掉」时仍然自我满足
    /// （`ipc_command_registry` / `capability_contract` 都踩过这个坑）。
    /// 取的是**注入语句本身**而不是裸 hostname —— 后者会命中紧邻的说明注释，守卫就咬不动了。
    const MODE_MARKER: &str = concat!(
        "window.__web2appLoadingMode = (location.hostname === '",
        "splash.localhost'"
    );
    /// 注册加载层那一句（`loading::` 限定符把它与 tabs.rs 的 `crate::loading::` 版本区分开）。
    const OVERLAY_REG: &str = concat!(".initialization_script(loading::", "OVERLAY_SCRIPT)");

    /// 主窗口：模式标记必须存在，且排在 OVERLAY_SCRIPT 之前。
    #[test]
    fn main_window_registers_mode_marker_before_overlay() {
        let marker = LIB_RS.find(MODE_MARKER).expect(
            "lib.rs 主窗口缺加载层模式标记 —— 启动页会被当成目标页跑，\
             overlay 的 15s 空白硬上限会把正常启动误报成「页面长时间为空白」",
        );
        let overlay = LIB_RS
            .find(OVERLAY_REG)
            .expect("lib.rs 主窗口应注册 loading::OVERLAY_SCRIPT");
        assert!(
            marker < overlay,
            "模式标记必须注册在 OVERLAY_SCRIPT【之前】：初始化脚本按注册顺序执行，\
             顺序反了标记来不及生效（等价于没写）"
        );
    }
}

/// 启动期「壳侧信息必须真的落到卡片上」的静态契约（跨平台）。
///
/// 背景（2026-10-09 真机取证第二轮）：真机只拿到一张「启动超时」兜底卡，卡上**零现场信息**。
/// 追下来是两条独立的信息黑洞，且都不报错、clippy 与 CI 全绿：
///   ① `loading.js` 在挂载完成前挂的是**纯 no-op 桩** ⇒ 而直连配置的启动只需毫秒级完成
///      （无域名解析、无本地代理）⇒ 它的失败推送**必然**落在挂载之前 ⇒ 被静默吃掉；
///   ② 壳侧根本没有「阶段进度」这条通路 ⇒ 40s 兜底卡出现时，没人知道卡在解析、代理还是导航。
/// 更狠的一层：Android 上 `navigate()` 与 `eval()` **恒返回 Ok**（wry 只是把消息投给
/// main pipe），且 wry **不向壳层透出 `onReceivedError`** ⇒ 主框架加载失败时壳侧完全无从
/// 得知，WebView 就把启动页一直留在屏幕上（`bootstrap.rs` 的 `Err(e) => show_error` 分支在
/// Android 上是**死代码**）。所以卡片上的信息是唯一的现场，这里把两侧的落点钉住。
#[cfg(test)]
mod startup_visibility_contract {
    const LOADING_RS: &str = include_str!("loading.rs");
    const BOOTSTRAP_RS: &str = include_str!("bootstrap.rs");
    const LIB_RS: &str = include_str!("lib.rs");

    /// `loading.rs` 必须提供往卡片追加信息的通路（`note`），且与 JS 侧 API 同名。
    #[test]
    fn loading_rs_exposes_note_channel() {
        assert!(
            LOADING_RS.contains("pub fn show_note(app: &AppHandle, label: &str, line: &str)"),
            "loading.rs 必须提供 show_note —— 没有它，壳侧无法把阶段进度 / 探测结论送到卡片上"
        );
        assert!(
            LOADING_RS.contains("window.__web2appLoading.note({l})"),
            "show_note 必须调用加载层对外暴露的 note API（改名会让通路静默失效）"
        );
    }

    /// 启动流程必须在关键节点留下进度，否则兜底卡无法判读。
    #[test]
    fn bootstrap_pushes_phase_progress() {
        for (needle, why) in [
            (
                "正在解析目标地址（{} 个域名）…",
                "域名数组配置必须报出「正在解析」并带域名个数",
            ),
            ("正在打开 {}…", "导航前必须报出「正在打开 <host>」"),
            (
                "正在启动本地反向代理…",
                "启用本地代理时必须报出「正在启动代理」",
            ),
            (
                "已发起导航，等待页面响应…",
                "导航发起后必须留痕 —— 这是区分「根本没导航」与「导航了但没提交」的唯一线索",
            ),
        ] {
            assert!(
                BOOTSTRAP_RS.contains(needle),
                "{why}（bootstrap.rs 缺 {needle}）"
            );
        }
        assert!(
            BOOTSTRAP_RS.contains("crate::loading::show_note(&a, &l, &s)"),
            "阶段进度必须经 loading::show_note 在【主线程】推送（eval 不允许从后台线程直接调）"
        );
    }

    /// 解析失败必须保留具体原因（旧实现统一吞成「服务器连接失败」，把唯一线索丢了）。
    #[test]
    fn resolve_failure_keeps_reason() {
        assert!(
            BOOTSTRAP_RS.contains("解析目标地址失败：{e}"),
            "解析失败必须把具体原因带进用户可见文案（读系统 DNS 配置失败 / 查询失败 / 预算超时…）"
        );
        assert!(
            BOOTSTRAP_RS.contains("RESOLVE_BUDGET_SECS"),
            "解析阶段必须设总预算：域名是串行查的，最坏可超过启动页 40s 的兜底卡"
        );
    }

    /// Android 启动期必须先等**首个文档加载完成**，再做解析 / 推送 / 导航。
    ///
    /// 背景（2026-10-09 第二次真机取证定到的根因）：wry Android 的 `main_pipe.rs` 里
    /// `WebViewMessage::LoadUrl` / `WebViewMessage::Eval` 两条消息的处理体都是
    /// `if let Some(webview) = get_webview(activity_id) { … }` —— 目标未就绪时**静默丢弃**；
    /// 而初始 splash 导航走的是 `post{}` 异步排队的 `loadUrlMainThread`，后续 `navigate` 走的
    /// 是同步 `loadUrl`。直连配置毫秒级完成 bootstrap ⇒ 导航先发、随后被那条 post 出去的
    /// splash 加载**覆盖**；同期的进度 `eval` 落在 document-start 之前被 `&&` 短路吃掉。
    /// 真机 40s 卡上因此**只有页面自产的四行**、四条阶段进度一条不见。
    #[test]
    fn android_boot_waits_for_first_page_load() {
        assert!(
            BOOTSTRAP_RS.contains("fn wait_first_page_ready"),
            "bootstrap.rs 必须提供「等首个文档加载完成」的实现"
        );
        assert!(
            BOOTSTRAP_RS.contains("if !wait_first_page_ready(&app, &label)"),
            "启动流程必须真的调用它 —— 少了这一句，导航会被 post 出去的 splash 加载覆盖、\
             进度 eval 会被静默吃掉（真机取证 2026-10-09）"
        );
        assert!(
            BOOTSTRAP_RS.contains(
                "pub(crate) fn on_page_load(app: &tauri::AppHandle, label: &str, finished: bool)"
            ),
            "必须有页面加载事件的落点（等待靠它置位，改名/改签名会让等待恒超时）"
        );
        assert!(
            BOOTSTRAP_RS.contains("ready.clear(label)"),
            "Started 必须清标记 —— 否则 /retry 重载启动页后沿用旧标记，重试仍会卡死"
        );
        assert!(
            BOOTSTRAP_RS.contains("SPLASH_READY_BUDGET_MS"),
            "等待必须有预算上限：拿不到页面加载事件时要退化成旧行为，而不是永远不启动"
        );
    }

    /// `on_page_load` 必须在**建主窗时**接到 `bootstrap::on_page_load`，否则等待恒超时。
    #[test]
    fn page_load_event_is_wired_on_main_window() {
        assert!(
            LIB_RS.contains("bootstrap::on_page_load("),
            "lib.rs 建主窗时必须把 on_page_load 接到 bootstrap::on_page_load"
        );
        assert!(
            LIB_RS.contains("bootstrap::BootReady::default()"),
            "BootReady 状态必须被 manage，否则 bootstrap 取状态时会 panic"
        );
    }

    /// 在源码里找一段**位于代码行上**的文字（排除注释行）。
    ///
    /// 为什么不用裸 `contains`：本项目已多次踩到「守卫的针被注释满足」—— 把调用点注释掉
    /// 之后（`// xxx(...)`），`contains` 依然为真、守卫形同虚设。本次反向对照已实测到这一点。
    fn has_code_line(src: &str, needle: &str) -> bool {
        src.lines().any(|line| {
            let t = line.trim();
            t.contains(needle) && !t.starts_with("//") && !t.starts_with('*')
        })
    }

    /// 导航后必须做「落点校验 + 有限重发」—— 这是「导航到底有没有生效」的终局检验。
    ///
    /// 存在理由：`navigate()` 在 Android 上恒返回 `Ok(())`（只是往 main-pipe 投一条消息），
    /// 所以**没有任何返回值能证明导航生效**；真机出现过「导航发起了、地址却停在
    /// `splash.localhost`」。只靠「等首个文档就绪」是时序推断，这条是结果检验，两者互补。
    #[test]
    fn android_navigation_is_verified_and_retried() {
        // ① 判据必须同时认两种启动页形态：自定义协议 + wry 改写后的 http://splash.localhost/。
        //    只认自定义协议会在真机漏判 —— v1.62 已经栽过一次同类坑。
        assert!(
            has_code_line(
                BOOTSTRAP_RS,
                r#"url.scheme() == "splash" || url.host_str() == Some("splash.localhost")"#
            ),
            "启动页判据必须同时认 splash:// 与 http://splash.localhost/ 两种形态"
        );
        // ② 调用点必须真的接上（取语句本体，不取裸函数名 —— 否则会被注释满足）。
        assert!(
            has_code_line(
                BOOTSTRAP_RS,
                "ensure_navigation_committed(app, label, &url);"
            ),
            "导航成功后必须挂上落点校验：navigate() 恒返回 Ok，返回值不能作为成功判据"
        );
        // ③ 重发次数必须是有限常量（不允许无上限重发）。
        assert!(
            has_code_line(BOOTSTRAP_RS, "const NAV_COMMIT_MAX_RETRY: u32 = 2;"),
            "重发次数必须是有限常量，否则异常时无限重发"
        );
        // ④ 校验必须跑在独立线程：主线程里 sleep + 读地址（Android 走 main-pipe，
        //    未就绪时最长阻塞 10s）会把 UI 冻住。
        let start = BOOTSTRAP_RS
            .find("fn ensure_navigation_committed")
            .expect("落点校验函数缺失");
        let seg = &BOOTSTRAP_RS[start..(start + 900).min(BOOTSTRAP_RS.len())];
        assert!(
            has_code_line(seg, "std::thread::spawn"),
            "落点校验必须跑在独立线程里（主线程会冻 UI）"
        );
    }
}

/// 「没有可退的真实页面时不触发后退，内置 splash 页不允许成为落点」的静态契约（跨平台）。
///
/// 背景：应用启动时主窗先加载内置 splash 启动页（`splash://localhost/`，wry 改写为
/// `http://splash.localhost/`），随后再导航到目标网址 —— splash 因此成为历史里的上一条。
/// 裸 `history.back()` / `WebView.goBack()` 在首个目标页就会退回到内置页。规则要求：
///   · 电脑端（Tauri / Electron）：无真实上一页时不触发后退，并置灰标题栏按钮；
///   · 移动端（Android）：系统返回键同样不得回到内置页，无真实历史时按系统默认结束页面。
///
/// 本模块把三处实现（注入脚本 titlebar.js、Rust history_nav 守卫脚本、Android CI 补丁）
/// 与其接线（两个 workflow）一并钉死。
#[cfg(test)]
mod back_navigation_guard_contract {
    /// 自绘标题栏注入脚本（两个壳单一真源）。
    const TITLEBAR_JS: &str = include_str!("titlebar.js");
    /// history_nav 命令实现（守卫脚本常量与调用点）。
    const LIB_RS: &str = include_str!("lib.rs");
    /// Android MainActivity 补丁脚本（CI 在 android init 之后调用）。
    const PATCH_PY: &str = include_str!("../../.github/scripts/patch-android-back.py");
    /// 两个会打出 Android 包的 workflow，必须接上补丁步骤。
    const BUILD_CUSTOM_YML: &str = include_str!("../../.github/workflows/build-custom.yml");
    const RELEASE_YML: &str = include_str!("../../.github/workflows/release.yml");

    /// ① titlebar.js：前进/后退必须经 Navigation API 落点判定，且内置页两种形态都要挡。
    #[test]
    fn titlebar_checks_navigation_entries_before_back() {
        for needle in [
            "function isBuiltinUrl(u)",
            "function navDirections()",
            "window.navigation",
            "n.currentEntry.index",
            "n.entries()",
            // 内置启动页的两种 URL 形态判据
            "/^splash:/i",
            "splash\\.localhost",
        ] {
            assert!(
                TITLEBAR_JS.contains(needle),
                "titlebar.js 后退守卫缺少要素：{needle}"
            );
        }
        // history.back() 只能有一处（守卫内）；裸调用残留意味着按钮仍会无条件后退。
        assert_eq!(
            TITLEBAR_JS.matches("window.history.back()").count(),
            1,
            "titlebar.js 中 history.back() 应只在 onNav 守卫内出现一次"
        );
        assert_eq!(
            TITLEBAR_JS.matches("window.history.forward()").count(),
            1,
            "titlebar.js 中 history.forward() 应只出现一次"
        );
        // 按钮必须随落点置灰。
        assert!(
            TITLEBAR_JS.contains("backBtn.disabled = !d.back")
                && TITLEBAR_JS.contains("fwdBtn.disabled = !d.fwd"),
            "标题栏前进/后退按钮必须按落点状态置灰"
        );
    }

    /// ② lib.rs：history_nav 必须改用守卫脚本，裸 history.back() 脚本须消失。
    #[test]
    fn history_nav_uses_guarded_scripts() {
        for needle in [
            "const HISTORY_BACK_SCRIPT: &str",
            "const HISTORY_FWD_SCRIPT: &str",
            "window.navigation",
            "n.currentEntry.index",
            r"splash\.localhost",
            "window.history.back();",
            "window.history.forward();",
            r#""back" => HISTORY_BACK_SCRIPT"#,
            r#""forward" => HISTORY_FWD_SCRIPT"#,
        ] {
            assert!(
                LIB_RS.contains(needle),
                "lib.rs history_nav 守卫缺少要素：{needle}"
            );
        }
        assert!(
            !LIB_RS.contains(r#""try{window.history.back();}catch(e){}""#),
            "裸 history.back() 脚本必须移除（会退回到内置 splash 页）"
        );
    }

    /// ③ Android 补丁脚本：要素齐全（覆盖 AppPlugin 的返回回调，内置页不可后退）。
    #[test]
    fn android_patch_script_keeps_guard_pieces() {
        for needle in [
            "web2app-android-back-guard",
            "override fun onWebViewCreate(webView: WebView)",
            "onBackPressedDispatcher.addCallback(this",
            "webView.copyBackForwardList()",
            "list.currentIndex",
            "isWeb2appBuiltInPage",
            "finish()",
            "splash.localhost",
        ] {
            assert!(
                PATCH_PY.contains(needle),
                "patch-android-back.py 缺少要素：{needle}"
            );
        }
    }

    /// ④ 两个 Android 打包 workflow 必须在 init 之后调用补丁脚本。
    #[test]
    fn workflows_invoke_android_back_patch() {
        for (name, yml) in [
            ("build-custom.yml", BUILD_CUSTOM_YML),
            ("release.yml", RELEASE_YML),
        ] {
            assert!(
                yml.contains("patch-android-back.py"),
                "{name} 必须接上 Android 返回键补丁步骤（gen/android 每次现生成）"
            );
        }
    }
}

/// 桌面悬浮球（仅 Windows）的静态契约（v1.68 纠正：球是悬浮图标，点击调出主窗口）。
///
/// 与多页签同一类「全是静默失败」的问题域：
///   · 本地球页 floaticon.html 若事件名和 Rust 监听不一致 ⇒ 拖动 / 隐藏 / 点击点了没反应，无报错；
///   · float-icon.json 少了 `core:event:allow-emit` 或窗口标签不匹配 ⇒ 所有事件发不出；
///   · 一次性令牌若漏进子 frame ⇒ 伪造事件可以拖着用户的悬浮球走；
///   · 球窗若导航远程页 / 自行 bootstrap ⇒ 违背悬浮图标语义，还会 abort 主窗在服的单实例代理。
/// 这里用源码 / JSON 断言把这些前提钉死。
#[cfg(test)]
#[cfg(windows)]
mod float_window_contract {
    /// 悬浮球 Rust 实现。
    const FLOAT_RS: &str = include_str!("floatwin.rs");
    /// 悬浮球内置本地页面（双壳单一真源；v1.68 起替代旧的 floatbar.js）。
    const FLOAT_HTML: &str = include_str!("floaticon.html");
    /// app 接线处。
    const LIB_RS: &str = include_str!("lib.rs");
    /// 设置模块（右键关功能要写设置 + 同步设置页复选框）。
    const SETTINGS_RS: &str = include_str!("settings.rs");
    /// 设置页（双壳单一真源；复选框 id 与 Rust 推送脚本必须一致）。
    const SETTINGS_HTML: &str = include_str!("../dist/settings.html");
    /// 托盘（单击球复用其 show_window）。
    const TRAY_RS: &str = include_str!("tray.rs");
    /// bootstrap（v1.67 的 NavCache 已随方向纠正删除，这里钉住「别再加回来」）。
    const BOOTSTRAP_RS: &str = include_str!("bootstrap.rs");
    /// 悬浮球本地页的能力声明 —— include_str! 同时把「文件漏 git add」变成编译失败。
    const FLOAT_CAP: &str = include_str!("../capabilities/float-icon.json");

    /// 八个事件名（常量拆两段拼接，避免本文件的注释 / 断言自我满足）。
    const EV_DOWN: &str = concat!("\"", "web2app:float-down", "\"");
    const EV_MOVE: &str = concat!("\"", "web2app:float-move", "\"");
    const EV_CANCEL: &str = concat!("\"", "web2app:float-cancel", "\"");
    const EV_DRAG_END: &str = concat!("\"", "web2app:float-drag-end", "\"");
    const EV_PEEK_ENTER: &str = concat!("\"", "web2app:float-peek-enter", "\"");
    const EV_PEEK_LEAVE: &str = concat!("\"", "web2app:float-peek-leave", "\"");
    const EV_HIDE: &str = concat!("\"", "web2app:float-hide", "\"");
    const EV_CLICK: &str = concat!("\"", "web2app:float-click", "\"");

    /// 只认真正的代码行（排除 `//` / `*` 注释），防止守卫被注释里的同名文字满足。
    fn has_code_line(src: &str, needle: &str) -> bool {
        src.lines().any(|line| {
            let t = line.trim();
            t.contains(needle) && !t.starts_with("//") && !t.starts_with('*')
        })
    }

    /// [`has_code_line`] 的反向版：代码行（剥纯注释行）里是否出现过 `needle`。用于
    /// 「某 API 不得复活」类反向钉死 —— 模块文档 / 存档注释里允许提到名字。
    fn code_has_occurrence(src: &str, needle: &str) -> bool {
        src.lines().any(|line| {
            let t = line.trim();
            t.contains(needle) && !t.starts_with("//") && !t.starts_with('*')
        })
    }

    /// 取某条 CSS 规则的**声明块**（选择器起，到第一个 `}` 为止）。用于把断言限定在
    /// 「那一条规则」内 —— 与本模块的 `fn_body` 同理：只断言「文件里出现过某个字面量」
    /// 会被别处的同名文字满足，是自我满足的假守卫。
    fn css_block(src: &str, selector: &str) -> String {
        let Some(start) = src.find(selector) else {
            return String::new();
        };
        let rest = &src[start..];
        match rest.find('}') {
            Some(end) => rest[..end].to_string(),
            None => String::new(),
        }
    }

    /// 取 `floatwin.rs` 里 `ICON_SIZE_LOGICAL` 的**整数值**（`56.0` → `"56"`）。
    ///
    /// 为什么不用字面量：球页的 CSS 尺寸必须与它相等，两边各写一个 56 迟早会漂移
    /// （这类「同一不变量在多处各写一遍」正是本项目反复踩的坑）。这里直接从常量声明里
    /// 抠出数值，改常量就会自动要求 CSS 跟着改。
    fn icon_size_logical_int() -> String {
        let line = FLOAT_RS
            .lines()
            .find(|l| l.contains("const ICON_SIZE_LOGICAL: f64 ="))
            .expect("floatwin.rs 里找不到 ICON_SIZE_LOGICAL 声明");
        let value = line
            .split('=')
            .nth(1)
            .expect("ICON_SIZE_LOGICAL 声明没有 `=`")
            .trim()
            .trim_end_matches(';')
            .trim();
        value
            .split('.')
            .next()
            .expect("ICON_SIZE_LOGICAL 的值解析不出整数部分")
            .to_string()
    }

    /// 取某个函数签名的**函数体**（签名行起，到第一个顶格 `}` 行为止）。
    ///
    /// 为什么需要它：只断言「文件里出现过某个调用」是**自我满足**的假守卫 —— 文件别处（别的
    /// 函数、甚至注释）有同名调用照样能绿。把断言限定进被测函数的函数体内，才真正钉住
    /// 「这一条路径做了那件事」。
    fn fn_body(src: &str, signature: &str) -> String {
        let Some(start) = src.find(signature) else {
            return String::new();
        };
        let mut out = String::new();
        for line in src[start..].lines() {
            out.push_str(line);
            out.push('\n');
            if line == "}" {
                break;
            }
        }
        out
    }

    /// ① capability：只作用于 float 窗口、**仅本地页**（无 remote 段）、仅 event:emit，
    ///    不给任何窗口控制权限。
    #[test]
    fn float_capability_is_local_and_minimal() {
        let v: serde_json::Value =
            serde_json::from_str(FLOAT_CAP).expect("capabilities/float-icon.json 不是合法 JSON");
        let windows = v["windows"]
            .as_array()
            .expect("float-icon.json 缺 windows 数组");
        let win_labels: Vec<&str> = windows.iter().filter_map(|x| x.as_str()).collect();
        assert_eq!(
            win_labels,
            vec!["float"],
            "float capability 必须且只能命中窗口标签 float（实际：{windows:?}）"
        );
        assert!(
            v.get("remote").is_none(),
            "悬浮球只加载内置本地页，float-icon.json 不得再带 remote 段（v1.67 的远程授权必须移除）"
        );
        let perms = v["permissions"]
            .as_array()
            .expect("float-icon.json 缺 permissions 数组");
        assert!(
            perms
                .iter()
                .any(|p| p.as_str() == Some("core:event:allow-emit")),
            "float-icon.json 必须授予 core:event:allow-emit，否则球页所有事件都发不出"
        );
        for forbidden in [
            "core:window:allow-minimize",
            "core:window:allow-close",
            "core:window:allow-start-dragging",
            "core:window:allow-set-position",
        ] {
            assert!(
                !perms.iter().any(|p| p.as_str() == Some(forbidden)),
                "float capability 不应授予 {forbidden}：窗口移动 / 显隐全部由 Rust 完成（最小权限）"
            );
        }
        // 旧的远程 capability 文件必须已删除（改名即落地，防止两份并存）。
        assert!(
            !std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("capabilities/float-remote.json")
                .exists(),
            "capabilities/float-remote.json 必须删除（已被本地页 float-icon.json 取代）"
        );
    }

    /// ② 八个事件名在 Rust 常量与 floaticon.html 字面量两侧逐字一致，且都走核心事件 + 令牌。
    #[test]
    fn float_event_names_match_between_rust_and_page() {
        for (rust_lit, js_lit) in [
            (EV_DOWN, "web2app:float-down"),
            (EV_MOVE, "web2app:float-move"),
            (EV_CANCEL, "web2app:float-cancel"),
            (EV_DRAG_END, "web2app:float-drag-end"),
            (EV_PEEK_ENTER, "web2app:float-peek-enter"),
            (EV_PEEK_LEAVE, "web2app:float-peek-leave"),
            (EV_HIDE, "web2app:float-hide"),
            (EV_CLICK, "web2app:float-click"),
        ] {
            assert!(
                FLOAT_RS.contains(rust_lit),
                "floatwin.rs 必须定义事件常量 {rust_lit}"
            );
            assert!(
                FLOAT_HTML.contains(js_lit),
                "floaticon.html 必须包含事件名 {js_lit}（与 Rust 逐字一致，否则点了没反应）"
            );
        }
        assert!(
            FLOAT_HTML.contains("plugin:event|emit"),
            "floaticon.html 必须走核心事件通路"
        );
        assert!(
            FLOAT_HTML.contains("__web2appFloatToken"),
            "floaticon.html 的每个事件都必须带一次性令牌 __web2appFloatToken"
        );
        // 八个事件都必须在 Rust 侧真正注册监听（不只是定义了常量）。
        for reg in [
            "listen(FLOAT_DOWN_EVENT",
            "listen(FLOAT_MOVE_EVENT",
            "listen(FLOAT_CANCEL_EVENT",
            "listen(FLOAT_DRAG_END_EVENT",
            "listen(FLOAT_PEEK_ENTER_EVENT",
            "listen(FLOAT_PEEK_LEAVE_EVENT",
            "listen(FLOAT_HIDE_EVENT",
            "listen(FLOAT_CLICK_EVENT",
        ] {
            assert!(
                FLOAT_RS.contains(reg),
                "floatwin::register_float_listeners 必须注册监听 {reg}"
            );
        }
    }

    /// ③ 令牌只注入主框架（初始化脚本会进所有 frame；球页虽无 iframe，仍作纵深防御）。
    #[test]
    fn float_token_is_top_frame_only() {
        assert!(
            FLOAT_RS.contains("window.self === window.top"),
            "floatwin.rs 注入 __web2appFloatToken 时必须带主框架判断，避免令牌漏给子 frame"
        );
        assert!(
            FLOAT_RS.contains("window.__web2appFloatToken"),
            "floatwin.rs 必须注入 __web2appFloatToken（不得误删注入本身）"
        );
        assert!(
            FLOAT_RS.contains("fn token_matches"),
            "Rust 必须校验事件令牌（对不上一律丢弃）"
        );
    }

    /// ④ 单击球 = **切换**【原来的主窗口】显隐（v1.71：v1.68 起只调 show_window，
    ///    主窗已经可见时点球毫无反应 —— 真机两轮复现的「点了没反应」）。
    ///    ⚠️ v1.76 起「不得调 show_window」只对 **Click 分支**成立：右键路径
    ///    （`disable_float_from_icon`）**要**调 show_window 把主窗还给用户，见 ⑫。
    #[test]
    fn float_click_toggles_the_original_main_window() {
        assert!(
            has_code_line(FLOAT_RS, "crate::tray::toggle_window(app)"),
            "floatwin 的 Click 事件必须调 tray::toggle_window 切换原来的主窗口显隐"
        );
        // 反向对照：**单击分支内部**不得退回「只显示」（show_window 仍供托盘菜单等
        // 「意图就是调出来」的调用方）。
        // ⚠️ v1.76 起断言必须收窄到 Click 分支：右键路径（`disable_float_from_icon`）
        //    **确实**要调 `crate::tray::show_window` 把主窗还给用户，全文判「floatwin.rs
        //    不含 show_window」已不再是有效判据（真机缺陷的判据是「Click 分支」，不是「整个文件」）。
        let click_branch = {
            let start = FLOAT_RS
                .find("EventKind::Click => {")
                .expect("floatwin.rs 必须有 Click 分支");
            let tail = &FLOAT_RS[start..];
            // 以「下一个 match 分支的起点」为界收口（比数括号稳）。
            let end = tail.find("\n        EventKind::").unwrap_or(tail.len());
            &tail[..end]
        };
        assert!(
            click_branch.contains("crate::tray::toggle_window(app)"),
            "Click 分支必须调 tray::toggle_window（单击球 = 切换主窗显隐）"
        );
        assert!(
            !click_branch.contains("show_window"),
            "Click 分支不得调 tray::show_window：单击球必须是 toggle（v1.71 真机缺陷回归）"
        );
        assert!(
            TRAY_RS.contains("pub(crate) fn toggle_window"),
            "tray.rs 必须提供 toggle_window（托盘左键 / 唤醒快捷键 / 单击球共用）"
        );
        for needle in [
            "window.hide()",
            "sync_tabs_visibility(app, false)",
            ".show()",
        ] {
            assert!(
                TRAY_RS.contains(needle),
                "tray::toggle_window 必须真正切换主窗口（找不到 {needle}）"
            );
        }
        // 页面侧：click 必须是「没有越过拖动阈值的 pointerup」，且发送 float-click。
        // v1.72 起判定改成两段式：拖动松手先走 dragend，其余才走（经双击去抖的）click，
        // 见 ⑪ `float_page_click_is_debounced_against_double_click`。
        assert!(
            FLOAT_HTML.contains("if (moved) { send('dragend'); return; }"),
            "floaticon.html 必须区分点击与拖动：越过阈值的松手发 dragend"
        );
        assert!(
            FLOAT_HTML.contains("send('click')"),
            "floaticon.html 未移动的松手必须发 click"
        );
    }

    /// ④b v1.71：球内必须是**软件自己的图标**（icons/icon.png 编译期内嵌），
    ///    不再是统一硬编码图形；页面经同协议 /icon.png 取图，协议不得开放其它路径。
    #[test]
    fn float_icon_is_the_app_icon_served_over_floaticon_protocol() {
        assert!(
            has_code_line(FLOAT_RS, "include_bytes!(\"../icons/icon.png\")"),
            "handle_floaticon 必须编译期内嵌 icons/icon.png（打包替换图标后球天然跟随）"
        );
        assert!(
            has_code_line(FLOAT_RS, "\"/icon.png\""),
            "floaticon 协议必须按路径分流 /icon.png"
        );
        assert!(
            FLOAT_RS.contains(".status(404)"),
            "floaticon 协议对未登记路径必须回 404（只暴露页面与图标两个内置资源）"
        );
        assert!(
            FLOAT_HTML.contains("src=\"icon.png\""),
            "floaticon.html 必须用 <img src=\"icon.png\"> 渲染软件图标"
        );
        assert!(
            !FLOAT_HTML.contains("<svg"),
            "floaticon.html 不得再内嵌统一的硬编码 SVG 图形（必须换成软件图标）"
        );
        // 图标文件真实存在（include_str 兄弟；include_bytes! 本身也会在编译期钉住这一点）。
        assert!(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("icons/icon.png")
                .exists(),
            "icons/icon.png 必须存在（悬浮球图标 + bundle 图标的同源文件）"
        );
    }

    /// ④c v1.73：球**不得**挂原生提示（不给球写 title 属性）。
    ///
    /// 球窗是 always-on-top，而原生提示条是**独立的非置顶窗口** ⇒ 提示条只会被画在球窗
    /// **之后**，从球窗的透明环带（圆内、球本体之外）漏出一块浅色残片，看起来就像
    /// 「图标背景上有异样图形」。2026-10-11 真机截图 + Electron 复刻本页双重取证坐实
    /// （hover 出现、移开消失）。作用说明改由 aria-label 承载（读屏可见）。
    #[test]
    fn float_ball_has_no_native_tooltip() {
        // 只取球元素所在的**那一个标签**做判定：页面注释 / 别处的同名字样不参与。
        let ball_tag = {
            let idx = FLOAT_HTML
                .find("id=\"w2f-ball\"")
                .expect("floaticon.html 里找不到球元素 #w2f-ball");
            let start = FLOAT_HTML[..idx].rfind('<').expect("球元素标签起点不完整");
            let end = idx + FLOAT_HTML[idx..].find('>').expect("球元素标签未闭合");
            &FLOAT_HTML[start..=end]
        };
        assert!(
            !ball_tag.contains("title="),
            "球不得挂 title 属性：原生提示条会被 always-on-top 球窗遮住、只从透明环带漏出残片（v1.73 真机 + Electron 复刻取证）。实际标签：{ball_tag}"
        );
        assert!(
            ball_tag.contains("aria-label="),
            "球的作用说明必须保留在 aria-label（没有原生提示，但读屏 / 源码可查）。实际标签：{ball_tag}"
        );
        assert!(
            FLOAT_HTML.contains("单击显示"),
            "球的作用说明文案不得丢失（现由 aria-label 承载）"
        );
    }

    /// ④c v1.80：WebView2 把宿主窗最小宽度强制撑到约 131 逻辑像素，球窗只有 56 逻辑宽
    ///    活柱。v1.77b→v1.79 三代方案（SetWindowRgn 严格裁圆 / 区域外扩 / 页面黑底）真机
    ///    全部证伪（锯齿或浅色异形），v1.80 改为**视觉 / 命中彻底分离**：
    ///    · 视觉：`.transparent(true)` + `.no_redirection_bitmap(true)` 同开，走
    ///      WS_EX_NOREDIRECTIONBITMAP + DirectComposition 的真逐像素 alpha（与 Electron
    ///      同路径；依赖 tauri-runtime-wry ≥ 2.12 才透传该开关），窗口**不得再有任何
    ///      SetWindowRgn**（1bit 硬掩码会把 DComp 软边重新切成阶梯）；
    ///    · 命中：父窗 + WebView2 子窗统一子类化，只拦 WM_NCHITTEST，圆外回
    ///      HTTRANSPARENT（圆 == 活柱 == 球）。所有吸附几何仍经 live_offset 换算。
    #[test]
    fn float_window_uses_per_pixel_alpha_and_circle_hittest() {
        // 真逐像素 alpha：两个开关必须并存（只 transparent 在 tao 上只是 DwmEnableBlurBehind）。
        assert!(
            has_code_line(FLOAT_RS, ".transparent(true)")
                && has_code_line(FLOAT_RS, ".no_redirection_bitmap(true)"),
            "悬浮球建窗必须同开 .transparent(true) 与 .no_redirection_bitmap(true)：\
             后者才会加 WS_EX_NOREDIRECTIONBITMAP 走 DComp 真逐像素 alpha（v1.80 架构核心）"
        );
        // 反向钉死：窗口区域 API 不得以任何形式回到代码里（注释存档不算）。
        for banned in ["SetWindowRgn", "CreateEllipticRgn", "CreateRectRgn", "REGION_PAD"] {
            assert!(
                !code_has_occurrence(FLOAT_RS, banned),
                "floatwin.rs 代码里不得再出现 {banned}：窗口区域是 1bit 硬掩码，会切掉 DComp \
                 软边（v1.77b 白锯齿 / v1.78 外扩漏异形，v1.80 已整体移除）"
            );
        }
        assert!(
            !code_has_occurrence(FLOAT_RS, "apply_live_region"),
            "旧的 apply_live_region 已由 update_hit_circle（只更新命中原子量）取代，不得复活"
        );
        // 命中圆必须随活柱几何更新，且几何来自同一套 live_px / live_offset。
        assert!(
            has_code_line(
                FLOAT_RS,
                "fn update_hit_circle(w: &WebviewWindow, edge: Option<DockEdge>)"
            ) && has_code_line(FLOAT_RS, "fn install_circle_hit_test(w: &WebviewWindow)"),
            "必须有 update_hit_circle（更新命中圆原子量）与 install_circle_hit_test（子类化）"
        );
        for needle in [
            "EnumChildWindows",
            "SetWindowLongPtrW(hwnd, GWLP_WNDPROC",
            "if msg == WM_NCHITTEST",
            "LRESULT(HTTRANSPARENT as isize)",
        ] {
            assert!(
                has_code_line(FLOAT_RS, needle),
                "圆形命中子类化缺少关键代码：{needle}（父窗 + 全部后代子窗都必须拦 WM_NCHITTEST）"
            );
        }
        // 子窗可能晚于父窗创建：建窗后 / on_page_load / Resized 都要幂等补装子类化。
        let installs = FLOAT_RS.matches("install_circle_hit_test(&w)").count();
        assert!(
            installs >= 3,
            "install_circle_hit_test 至少要在建窗后、PageLoadEvent::Finished、WindowEvent::Resized \
             三处幂等补装（WebView2 子窗 HWND 可能晚创建），实际 {installs} 处"
        );
        assert!(
            has_code_line(
                FLOAT_RS,
                "fn live_offset(host_w: i32, live_w: i32, edge: DockEdge) -> i32"
            ),
            "必须有 live_offset 纯函数：右吸附活柱贴右 host_w-live_w，其余贴左 0"
        );
        assert!(
            has_code_line(FLOAT_RS, "fn reposition_for_dock(w: &WebviewWindow, d: Dock)"),
            "必须有 reposition_for_dock：宿主宽迟到（WebView2 初始化 / DPI 变化）后按真实宽重定位活柱"
        );
        // on_page_load(Finished) 里必须更新命中圆 + 重定位 + 补装子类化。
        assert!(
            FLOAT_RS.contains("PageLoadEvent::Finished")
                && has_code_line(FLOAT_RS, "reposition_for_dock(&w, d)"),
            "页面 Finished 后必须按真实宿主宽重定位活柱 / 更新命中圆"
        );
        // 宿主被撑宽（Resized）与跨 DPI（ScaleFactorChanged）两个迟到时机都必须重算几何。
        assert!(
            FLOAT_RS.contains("WindowEvent::Resized")
                && FLOAT_RS.contains("WindowEvent::ScaleFactorChanged"),
            "Resized（WebView2 撑宽宿主）与 ScaleFactorChanged（跨 DPI）都必须重算活柱几何"
        );
        assert!(
            has_code_line(FLOAT_RS, "apply_initial_geometry(app, &window, saved);"),
            "建窗流程必须经 apply_initial_geometry（内部负责首置活柱 / 命中圆）"
        );
        // 真机排查期的临时插桩 / A/B 开关 / 页面诊断块不得残留。
        assert!(
            !FLOAT_RS.contains("[float-tmp]") && !FLOAT_RS.contains("W2A_FLOAT_NO_RGN"),
            "floatwin.rs 不得残留临时诊断 eprintln 或 W2A_FLOAT_NO_RGN A/B 开关"
        );
        assert!(
            !FLOAT_HTML.contains("w2a-dbg") && !FLOAT_HTML.contains("float-dbg"),
            "floaticon.html 不得残留临时红块 / float-dbg 诊断回传脚本"
        );
        // 几何各处必须以活柱宽高（live_px）为基准，而不是 outer_size 的宿主宽。
        for needle in [
            "fn live_px(scale: f64) -> (i32, i32)",
            "let (lw, lh) = live_px(scale);",
        ] {
            assert!(FLOAT_RS.contains(needle), "活柱几何缺少关键代码：{needle}");
        }
        // 页面必须能按主进程下发的 anchor 把球摆进左 / 右活柱（视口可能比球宽）。
        assert!(
            FLOAT_HTML.contains("data-anchor"),
            "floaticon.html 必须支持 data-anchor 属性切换球在视口内的左 / 右位置"
        );
        // push_state 经 format! 注入变量，源码里钉住两个字面量 'r'/'l' 的取值分支。
        assert!(
            FLOAT_RS.contains("\"'r'\"") && FLOAT_RS.contains("\"'l'\""),
            "push_state 必须下发 anchor 'l'/'r'（右吸附活柱贴右）"
        );
    }

    /// ④d v1.80：球页与初始化脚本必须保持**真透明**，页面里不允许任何「按壳铺不透明底」
    /// 的残留。v1.79 曾注入 data-w2f-shell 标记让 body 铺 #000 黑底以绕过旧透明链路
    /// （wry 0.55 HWND 控制器 + 无合成表面，半透明像素被压平白底）；v1.80 升级 Tauri 2.12
    /// 开 NOREDIRECTIONBITMAP 后根因消除，黑底反而带来整像素阶梯暗锯齿，必须整版删除且
    /// 不得复活（双壳共用同一份页面，任何写死的壳标记都会误伤 Electron 真透明）。
    #[test]
    fn float_page_and_init_script_stay_truly_transparent() {
        // ① 真 alpha 开关是「不需要黑底」的前提，和④c 互相咬合。
        assert!(
            has_code_line(FLOAT_RS, ".no_redirection_bitmap(true)"),
            "Rust 侧必须开 .no_redirection_bitmap(true)，页面真透明才有合成保证"
        );
        // ② 初始化脚本只许注入一次性令牌，不得注入任何壳标记（连注释里的脚本代码也算，
        //    用 code_has_occurrence 剥掉纯注释行；这里钉的是可执行字符串）。
        assert!(
            !code_has_occurrence(FLOAT_RS, "data-w2f-shell"),
            "initialization_script 不得再注入 data-w2f-shell（v1.79 黑底方案 v1.80 已废弃）"
        );
        // ③ 页面侧：黑底选择器 / 纯黑背景 / 壳标记字面量一律不得存在。
        assert!(
            !FLOAT_HTML.contains("data-w2f-shell"),
            "floaticon.html 不得再出现 data-w2f-shell（双壳统一真透明，不允许任何壳判断；\
             注释存档也请换个别名表述，避免本守卫永久误报）"
        );
        assert!(
            !FLOAT_HTML.contains("background: #000") && !FLOAT_HTML.contains("background:#000"),
            "floaticon.html 不得给 body 铺纯黑底（v1.79 绕路方案，v1.80 真 alpha 下是暗锯齿源）"
        );
        // ④ 基础规则必须仍是 transparent（双壳都依赖）。
        let base = css_block(FLOAT_HTML, "html, body {");
        assert!(
            base.contains("background: transparent;"),
            "html, body 基础规则必须保持 background: transparent（双壳真透明依赖它）"
        );
    }

    /// ⑤ 悬浮球只加载内置本地页，绝不导航 / bootstrap / 访问远程（v1.67 方向已废弃）。
    #[test]
    fn float_loads_builtin_local_page_and_never_navigates() {
        assert!(
            FLOAT_RS.contains("FLOATICON_SCHEME")
                && FLOAT_RS.contains("include_str!(\"floaticon.html\")"),
            "悬浮球必须经 floaticon 自定义协议提供内嵌页面"
        );
        assert!(
            has_code_line(FLOAT_RS, "WebviewUrl::CustomProtocol(floaticon_url())"),
            "悬浮球建窗必须加载 floaticon:// 本地页（不得再是 App 页或远程 URL）"
        );
        assert!(
            has_code_line(
                LIB_RS,
                ".register_uri_scheme_protocol(floatwin::FLOATICON_SCHEME, floatwin::handle_floaticon)"
            ),
            "lib.rs 必须注册 floaticon 自定义协议"
        );
        for forbidden in [
            "navigate_window",
            "NavCache",
            "crate::bootstrap::bootstrap(",
            "build_proxy_args",
            "floatbar.js",
        ] {
            assert!(
                !FLOAT_RS.contains(forbidden),
                "floatwin.rs 不得再出现 {forbidden}：悬浮球不导航、不碰代理、不注入旧脚本"
            );
        }
        // v1.71 真机根因：球窗 builder 省略数据目录时，wry 另起默认 UDF（%LOCALAPPDATA%\<bundle>），
        // 受控 / 沙箱环境不可写 ⇒ 第二个 WebView2 环境创建失败被框架 log::error! 吞掉，build() 仍
        // 返回 Ok、球窗 HWND 从不出现。必须与主窗（lib.rs）/ tab 窗（tabs.rs）同构显式指定。
        assert!(
            has_code_line(FLOAT_RS, "Some(dir) => builder.data_directory(dir),")
                && has_code_line(FLOAT_RS, "let builder = match crate::config::data_dir(app) {"),
            "悬浮球建窗必须与主窗 / tab 窗同构显式设置 WebView2 数据目录（v1.71：默认 UDF 不可写时球窗静默建不出）"
        );
        assert!(
            !BOOTSTRAP_RS.contains("NavCache"),
            "bootstrap.rs 的进程级 NavCache 必须随悬浮球改版一起删除（已无读取方）"
        );
        assert!(
            !LIB_RS.contains("bootstrap::NavCache"),
            "lib.rs 不得再 manage NavCache"
        );
        // 透明球窗是圆形外观的前提。
        assert!(
            has_code_line(FLOAT_RS, ".transparent(true)"),
            "悬浮球窗口必须透明（球是圆形，窗外矩形区域要露出桌面）"
        );
    }

    /// ⑥ 窗口属性：无边框 / 透明 / 置顶 / 不占任务栏 / 不可调大小。
    ///
    /// ⚠️ `.skip_taskbar(true)` **只是必要条件，不是充分条件** —— tao 0.35 的它只调一次
    /// `ITaskbarList::DeleteTab`（且是在窗口还 `visible(false)` 时），`WS_EX_APPWINDOW`
    /// 仍留在创建时的样式里、`show()` 会把任务栏按钮带回来。真正生效的一手补丁见 ⑥b。
    #[test]
    fn float_window_properties() {
        for needle in [
            ".decorations(false)",
            ".transparent(true)",
            ".always_on_top(true)",
            ".skip_taskbar(true)",
            ".resizable(false)",
        ] {
            assert!(FLOAT_RS.contains(needle), "悬浮球建窗必须包含属性 {needle}");
        }
    }

    /// ⑥b v1.77：球窗必须**真不进任务栏** + **不带窗口标题**。
    ///
    /// 真机现象「悬浮图标背景异样」的第二个源头：`.skip_taskbar(true)` 挡不住任务栏按钮
    /// （tao 的 `ON_TASKBAR` 按「窗口无父」置位 ⇒ 创建时写死 `WS_EX_APPWINDOW`；
    /// `DeleteTab` 在窗口还没显示时调用无效，`show()` 后按钮回来 —— 实测球窗 `ex = 0x40118`
    /// 含 `APPWINDOW`）。任务栏按钮自带**原生提示条 / 缩略图浮层**（内容是窗口标题），而球窗
    /// always-on-top ⇒ 那个非置顶浮层只从球窗透明环带漏出一块浅色残片。Electron 的
    /// `skipTaskbar: true` 是真移除 ⇒ 该现象**仅 Tauri 侧**。
    #[test]
    fn float_window_is_excluded_from_taskbar_and_has_no_title() {
        // ① 样式必须改到位，且是「或上 want + 按位清掉 APPWINDOW」，不是只或上。
        let body = fn_body(FLOAT_RS, "fn apply_ball_window_styles(w: &WebviewWindow)");
        assert!(
            !body.is_empty(),
            "找不到 apply_ball_window_styles：把球窗扩展样式改到位的那一处没了"
        );
        for needle in [
            "GWL_EXSTYLE",
            "WS_EX_NOACTIVATE",
            "WS_EX_TOOLWINDOW",
            "WS_EX_APPWINDOW",
            "SetWindowLongPtrW",
        ] {
            assert!(
                body.contains(needle),
                "apply_ball_window_styles 必须处理 {needle}（球窗扩展样式一条都不能少）"
            );
        }
        assert!(
            body.contains("& !(") && body.contains("WS_EX_APPWINDOW.0 as isize"),
            "必须**清掉** WS_EX_APPWINDOW：只 OR 上 TOOLWINDOW 不够 —— APPWINDOW 会让任务栏在 show() 时把按钮加回来"
        );

        // ② 调用点必须在建窗函数体内、且在 `builder.build()` **之后**（样式得在真实 HWND 上改）。
        let build = fn_body(
            FLOAT_RS,
            "fn build_float_window_on_main(app: &tauri::AppHandle)",
        );
        assert!(
            !build.is_empty(),
            "找不到 build_float_window_on_main 函数体"
        );
        let at_build = build
            .find("builder.build()")
            .expect("建窗函数体里找不到 builder.build()");
        let at_call = build
            .find("apply_ball_window_styles(&window);")
            .expect("建窗函数体里必须在拿到窗口后调用 apply_ball_window_styles");
        assert!(
            at_build < at_call,
            "apply_ball_window_styles 必须在 build() 拿到真实窗口之后调用（早于它就改不到 HWND）"
        );

        // ③ 球窗不得带窗口标题：任务栏按钮 / 任务视图的提示条内容就是它。
        assert!(
            has_code_line(FLOAT_RS, ".title(\"\")"),
            "球窗必须显式置空标题（标题文本是任务栏提示条 / 任务视图流层的唯一内容来源）"
        );
        assert!(
            !has_code_line(FLOAT_RS, ".title(TITLE)"),
            "球窗不得再挂显示名标题：球无边框、不进任务栏与 Alt+Tab，标题没有任何可见载体，只会被其它外壳界面画成浮层残片"
        );
    }

    /// ⑥c v1.77：所有「把球显示出来」的路径都必须经 [`show_ball`]，且内部顺序必须是
    /// `show() → unminimize() → 补扩展样式 → 摘任务栏按钮`。
    ///
    /// 为什么顺序是硬要求：tao 在 `show()` / `unminimize()` 里会用**缓存标志**整体重写
    /// `GWL_EXSTYLE`（真机读回 `0x40118`：`APPWINDOW` 回来、另两位丢失）⇒ 样式只能写在
    /// 显示**之后**；而 `ITaskbarList::DeleteTab` 又必须等窗口真的显示过、按钮可能已建出
    /// 来之后才收得掉。把补样式留在 show 之前 = 完全没改（本缺陷最初的修法就栽在这里）。
    #[test]
    fn float_every_show_path_goes_through_show_ball() {
        let sb = fn_body(FLOAT_RS, "fn show_ball(w: &WebviewWindow)");
        assert!(!sb.is_empty(), "找不到 show_ball：显示球窗的唯一入口没了");
        let at_show = sb.find("w.show()").expect("show_ball 必须显示球窗");
        let at_unmin = sb
            .find("w.unminimize()")
            .expect("show_ball 必须取消最小化（它同样会触发 tao 重写扩展样式）");
        let at_style = sb
            .find("apply_ball_window_styles(w)")
            .expect("show_ball 必须在**显示之后**补扩展样式（写在 show 之前会被 tao 擦掉）");
        let at_tab = sb
            .find("delete_taskbar_tab(w)")
            .expect("show_ball 必须在补完样式后摘掉任务栏按钮");
        assert!(
            at_show < at_unmin && at_unmin < at_style && at_style < at_tab,
            "show_ball 的顺序必须是 show → unminimize → 补样式 → 摘按钮（实测偏移 {at_show}/{at_unmin}/{at_style}/{at_tab}）"
        );

        // 摘按钮那一步必须真用 ITaskbarList::DeleteTab，不能只是「清了样式就当完事」。
        let dt = fn_body(FLOAT_RS, "fn delete_taskbar_tab(w: &WebviewWindow)");
        assert!(
            !dt.is_empty() && dt.contains("ITaskbarList") && dt.contains("DeleteTab"),
            "delete_taskbar_tab 必须经 ITaskbarList::DeleteTab 回收已建出的任务栏按钮"
        );
        // ⚠️ ITaskbarList 的契约：任何方法前必须先 HrInit()，否则 DeleteTab 返回 E_FAIL 且
        // 什么都不做（v1.77 第一版就漏了它，还把 HRESULT 用 `let _ =` 吞掉 ⇒ 纯静默失败）。
        assert!(
            dt.contains("HrInit()"),
            "delete_taskbar_tab 必须先调 ITaskbarList::HrInit()：未初始化时 DeleteTab 静默无效"
        );
        assert!(
            !dt.contains("let _ = list.DeleteTab"),
            "delete_taskbar_tab 不得用 `let _ =` 吞掉 DeleteTab 的 HRESULT（失败必须留痕，否则又是静默失败）"
        );

        // 反向对照：show_ball 之外不得残留任何裸 show / unminimize —— 那等于绕开补样式，
        // 球会以「带任务栏按钮 + 会抢前台」的姿态出现，而编译 / 单测全绿。
        let rest = FLOAT_RS.replace(&sb, "");
        assert!(
            !has_code_line(&rest, "w.show()"),
            "floatwin.rs 里不得有绕过 show_ball 的裸 w.show()：tao 会随即把扩展样式写脏"
        );
        assert!(
            !has_code_line(&rest, ".unminimize()"),
            "floatwin.rs 里不得有绕过 show_ball 的裸 unminimize()：它同样触发扩展样式重写"
        );
    }

    /// ⑦ lib.rs 接线：托管状态 / 事件注册 / 启动按开关建球 / 关闭转隐藏 / 退出落盘。
    #[test]
    fn lib_wires_float_state_listeners_lifecycle_and_persistence() {
        assert!(
            has_code_line(LIB_RS, ".manage(floatwin::FloatState::default())"),
            "lib.rs 必须 manage floatwin::FloatState"
        );
        assert!(
            has_code_line(LIB_RS, "floatwin::register_float_listeners(app.handle());"),
            "lib.rs 启动时必须注册悬浮球事件监听"
        );
        assert!(
            has_code_line(LIB_RS, "if initial_settings.float_enabled {")
                && has_code_line(LIB_RS, "floatwin::show_float_window(app.handle());"),
            "启动时必须按设置 float_enabled 建悬浮球"
        );
        assert!(
            has_code_line(LIB_RS, "if label == floatwin::FLOAT_LABEL {")
                && has_code_line(LIB_RS, "floatwin::hide_float_window(app_handle);"),
            "悬浮球的关闭（Alt+F4）必须被拦截转成隐藏，而不是销毁窗口"
        );
        assert!(
            has_code_line(LIB_RS, "floatwin::persist_float_geometry(app_handle);"),
            "退出时必须持久化悬浮球几何"
        );
    }

    /// ⑧ 「销毁」必须真的销毁：`close()` 会被上面那条 CloseRequested 拦截整个吞掉
    ///    （tauri-runtime-wry 把 `WindowMessage::Close` 直接交给 `on_close_requested`，
    ///    与 Alt+F4 同一入口），结果是设置里取消勾选只把球藏起来 —— 静默降级，
    ///    编译 / 单测 / CI 全绿。必须用 tauri 明示的强制路径 `destroy()`。
    #[test]
    fn float_destroy_path_forces_real_destroy_not_close() {
        assert!(
            has_code_line(FLOAT_RS, "let _ = w.destroy();"),
            "销毁悬浮球必须调 WebviewWindow::destroy()（close() 会被 CloseRequested 处理器拦成隐藏）"
        );
        // 反向对照：源码里不得残留 `w.close()`（settings 窗那种无需拦截的窗口才用 close）。
        assert!(
            !FLOAT_RS.contains("w.close()"),
            "floatwin.rs 不得使用 close() 来销毁悬浮球：它等价于「只隐藏」"
        );
        // 上面那条拦截是本缺陷成立的**前提**：若哪天不再拦截，这条守卫要一起重审。
        assert!(
            has_code_line(LIB_RS, "api.prevent_close();"),
            "float 窗口的 CloseRequested 拦截（Alt+F4 → 隐藏）是 destroy/close 差异的前提，不得静默删掉"
        );
    }

    /// ⑩ v1.75：**只启用悬浮图标**（未启用托盘）时，关闭主窗也必须【驻留】而不是真销毁。
    ///
    /// 真机缺陷（2026-10-11 使用者实测，策略 `tray_enabled:false / minimize_on_close:false /
    /// float_enabled:true`）：关窗走 `else` 分支 ⇒ 主窗被真销毁，而球窗还活着 ——
    ///   ① 球窗挡住「所有窗口关闭 ⇒ 退出」，进程留成既无界面也退不掉的僵尸；
    ///   ② 点球时 `tray::toggle_window` 的 `get_webview_window("main")` 返回 `None`，
    ///      一个动作都不做 ⇒ 用户看到「点了悬浮图标不回窗口」。
    /// 球的产品语义就是「唤回主窗的入口」（见 floaticon.html 的 aria-label），
    /// 故「球在 ⇒ 应用驻留」必须成立。
    #[test]
    fn main_close_stays_resident_when_float_icon_enabled() {
        // 空白归一化后再匹配：本守卫要钉的是**语义**，不该被换行 / 缩进调整搞红。
        let flat = LIB_RS.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            flat.contains("let stay_resident = (tray && min) || (cfg!(windows) && float_icon);"),
            "handle_main_close 的驻留判据必须含「Windows 且 float_enabled」：\
             少了它，只启用悬浮图标时关闭窗口会真销毁主窗 —— 球变成点不动的孤儿，进程还退不掉"
        );
        // ⚠️ 托盘那一路仍然必须**两个开关都要**（这条极易被「顺手放宽成 ||」弄坏：
        //    只勾「启用托盘」而不勾「关闭时最小化到托盘」时，关窗应当正常退出）。
        assert!(
            !flat.contains("let stay_resident = tray ||"),
            "托盘驻留不得放宽成只看 tray_enabled：必须 tray_enabled && minimize_on_close"
        );
        // 平台差异必须是 `cfg!` 布尔（只在**值**上分叉），不得用 `#[cfg]` 分叉语句：
        // 后者会让 `float_icon` 在非 Windows 上变成「未使用的变量」⇒ -D warnings 编译失败。
        assert!(
            has_code_line(LIB_RS, "let (tray, min, float_icon) = {"),
            "必须一次性读出三个开关（含 float_enabled）再统一判定驻留"
        );
        assert!(
            !has_code_line(LIB_RS, "#[cfg(windows)] let stay_resident"),
            "驻留判据不得用 #[cfg] 分叉语句（非 Windows 上 float_icon 会变成未使用变量）"
        );
    }

    /// ⑪ v1.75：设置页「退出程序」必须与托盘「退出」走**同一条** `app.exit()` 路径。
    ///
    /// 启用托盘 / 悬浮图标后「关闭窗口」只等于隐藏 ⇒ 没启用托盘的用户原本**没有任何退出入口**
    /// （托盘的「退出」菜单项并不存在）。这条按钮是那种状态下唯一的出口，所以：
    ///   ① 命令体必须 `app.exit(0)`（→ `RunEvent::ExitRequested`，**不经过窗口 CloseRequested**，
    ///      因此上面那条「驻留」分支不会把退出吞掉）；写成 `window.close()` 就会重现
    ///      v1.68 缺陷②「点了退出，窗口只是藏起来」的形态；
    ///   ② settings.html 里必须真的调它（跨文件字面量，漏一处就是「点了没反应」）。
    #[test]
    fn settings_page_quit_button_uses_exit_not_window_close() {
        assert!(
            has_code_line(LIB_RS, "app.exit(0);"),
            "设置页「退出程序」必须调 app.exit(0)（与托盘 quit 同路径；window.close() 会被驻留分支吞掉）"
        );
        assert!(
            has_code_line(LIB_RS, "fn quit_app(app: tauri::AppHandle) {"),
            "quit_app 命令体必须存在（settings.html → 垫片白名单 → 注册表 → 命令体 是一条链）"
        );
        // 前端调用点：settings.html 是双壳单一真源，字面量必须逐字一致。
        let html = include_str!("../dist/settings.html");
        assert!(
            html.contains("invoke(\"quit_app\")"),
            "dist/settings.html 必须调用 quit_app（否则「退出程序」按钮点了没反应）"
        );
        assert!(
            html.contains("id=\"quitBtn\""),
            "dist/settings.html 必须有 #quitBtn（元素 id 是与脚本 / 探针的接口）"
        );
    }

    /// ⑫ v1.76：**右键球 = 关闭悬浮图标功能 + 唤回主窗口**（不是「只把球藏起来」）。
    ///
    /// 真机缺陷（2026-10-11 使用者实测，策略 `tray_enabled:false / minimize_on_close:false /
    /// float_enabled:true`）：主窗「关闭」只等于隐藏到球（驻留理由②），球于是成了桌面上
    /// **唯一**可见的入口；此刻右键把球一藏 —— 进程因 `float_enabled` 仍为 true 继续驻留，
    /// 桌面上却再也唤不出任何窗口。右键在用户直觉里本来就是「我不要这个球了」，
    /// 正确语义是回到主窗口，所以三件事缺一不可：
    ///   ① `float_enabled` 置 false 并落盘（含同步已打开设置页的复选框）；
    ///   ② **销毁**球窗；③ 显示主窗口。
    ///
    /// ⚠️ 只钉「Hide 分支调了某个函数」远远不够：必须把**函数体里的三件事**逐条限定在
    ///    `disable_float_from_icon` 内断言（`fn_body`），否则将来把函数体改回
    ///    `hide_float_window` 仍会全绿 —— 死局静默复现。
    #[test]
    fn float_right_click_disables_feature_and_restores_main_window() {
        // ① 事件分发：Hide 分支必须指向 disable_float_from_icon，不得退回 hide_float_window。
        assert!(
            has_code_line(FLOAT_RS, "EventKind::Hide => disable_float_from_icon(app),"),
            "球上右键（float-hide）必须走 disable_float_from_icon"
        );
        assert!(
            !has_code_line(FLOAT_RS, "EventKind::Hide => hide_float_window(app),"),
            "Hide 分支不得退回 hide_float_window：只隐藏会留下「进程在跑、桌面再无入口」的死局"
        );

        // ② 三件事必须真的在函数体里（各自单独限定，缺哪件就红哪条）。
        let body = fn_body(FLOAT_RS, "pub(crate) fn disable_float_from_icon");
        assert!(
            !body.is_empty(),
            "floatwin.rs 必须有 disable_float_from_icon 函数"
        );
        assert!(
            body.contains("crate::settings::set_float_enabled(app, false)"),
            "右键必须把 float_enabled 置 false 并落盘（否则进程继续驻留、重启后球又回来）"
        );
        assert!(
            body.contains("destroy_float_window(app)"),
            "右键必须**销毁**球窗：只 hide 会继续挡住「所有窗口关闭 ⇒ 退出」，也留下无入口的隐患"
        );
        assert!(
            body.contains("crate::tray::show_window(app)"),
            "右键必须把主窗口显示出来（用户点完右键要看到窗口，而不是一片空桌面）"
        );

        // ③ 设置写入纪律：只翻转这一位 + 落盘成功才写内存态（与 save_settings 同）。
        let setter = fn_body(SETTINGS_RS, "pub(crate) fn set_float_enabled");
        assert!(
            !setter.is_empty(),
            "settings.rs 必须有进程内 set_float_enabled（右键那条路径没有前端 Settings 载荷）"
        );
        assert!(
            setter.contains("persist_settings(app, &next)?") && setter.contains("*guard = next;"),
            "set_float_enabled 必须先落盘、成功后才写内存态（失败不写内存态）"
        );

        // ④ 设置页复选框必须同步：设置页只在加载时读一次设置，不同步的话它仍打着勾，
        //    用户随手一点「保存」就把 float_enabled 推回 true（球自己跑回来）。
        assert!(
            SETTINGS_RS.contains("fn push_float_enabled_to_settings_window"),
            "settings.rs 必须把新值推给已打开的设置页"
        );
        assert!(
            SETTINGS_RS.contains("getElementById('floatEnabled')")
                && SETTINGS_HTML.contains("id=\"floatEnabled\""),
            "设置页复选框 id（floatEnabled）在 Rust 推送脚本与 dist/settings.html 之间必须逐字一致"
        );

        // ⑤ 球页文案 / aria 必须写明新语义（读屏与文档层面的回归）。
        assert!(
            FLOAT_HTML.contains("右键关闭悬浮图标"),
            "floaticon.html 的 aria-label 必须写明「右键关闭悬浮图标」"
        );
    }

    /// ⑨ v1.70：拖动必须走「页面屏幕绝对坐标 + 主进程抓取偏移」协议。
    ///    v1.69 上报 clientX 增量，窗口每帧 set_position，下一帧增量被窗口自身位移
    ///    抵消（稳态半速跟手 + 来回抖），真机表现「球拖不动 / 乱跳」。
    #[test]
    fn float_drag_tracks_by_absolute_screen_coords() {
        // 页面侧：真实代码行必须用 screenX/screenY（注释里的解释文字不算）。
        // 先用状态机剥掉 <!-- ... --> 块注释（不能逐行判前缀：块注释中间的行没有任何
        // 注释前缀，v1.70 的解释文字就写在那里，会骗过反向守卫），再剥行内 // 注释。
        let strip_html_comments = |s: &str| -> String {
            let bytes = s.as_bytes();
            let mut out = String::with_capacity(s.len());
            let mut i = 0;
            let mut in_comment = false;
            while i < bytes.len() {
                if !in_comment && i + 4 <= bytes.len() && &bytes[i..i + 4] == b"<!--" {
                    in_comment = true;
                    i += 4;
                } else if in_comment && i + 3 <= bytes.len() && &bytes[i..i + 3] == b"-->" {
                    in_comment = false;
                    i += 3;
                } else {
                    let ch = s[i..].chars().next().unwrap();
                    if !in_comment {
                        out.push(ch);
                    }
                    i += ch.len_utf8();
                }
            }
            out
        };
        let page_code_lines: Vec<String> = strip_html_comments(FLOAT_HTML)
            .lines()
            .map(|l| match l.find("//") {
                Some(i) => l[..i].to_string(),
                None => l.to_string(),
            })
            .collect();
        assert!(
            page_code_lines.iter().any(|l| l.contains("e.screenX")),
            "floaticon.html 拖动必须读取 e.screenX（屏幕绝对坐标）"
        );
        assert!(
            page_code_lines.iter().any(|l| l.contains("e.screenY")),
            "floaticon.html 拖动必须读取 e.screenY（屏幕绝对坐标）"
        );
        assert!(
            !page_code_lines
                .iter()
                .any(|l| l.contains("clientX") || l.contains("clientY")),
            "floaticon.html 真实代码不得再用 clientX/clientY（窗口移动会污染其增量）"
        );
        // Rust 侧：载荷字段是 sx/sy；on_move 按 sx/sy 收参并有抓取偏移锚点。
        assert!(
            has_code_line(FLOAT_RS, "sx: f64,"),
            "FloatMsg 必须含 sx 字段（屏幕绝对坐标）"
        );
        assert!(
            has_code_line(FLOAT_RS, "sy: f64,"),
            "FloatMsg 必须含 sy 字段（屏幕绝对坐标）"
        );
        assert!(
            has_code_line(
                FLOAT_RS,
                "fn on_move(app: &tauri::AppHandle, w: &WebviewWindow, sx: f64, sy: f64)"
            ),
            "on_move 必须接收屏幕绝对坐标 sx/sy，而不是增量"
        );
        assert!(
            FLOAT_RS.contains("anchor: Mutex<Option<(i32, i32)>>"),
            "FloatState 必须持有抓取点偏移 anchor"
        );
        // v1.70 真机取证补：抓取偏移必须源自 pointerdown 的【按下点】而不是首帧 move 的光标。
        assert!(
            FLOAT_RS.contains("press: Mutex<Option<(i32, i32)>>"),
            "FloatState 必须暂存 pointerdown 按下点 press"
        );
        assert!(
            has_code_line(FLOAT_RS, "fn on_down(w: &WebviewWindow, sx: f64, sy: f64)"),
            "必须有 on_down：只暂存按下点，不展开、不置 moving"
        );
        assert!(
            page_code_lines
                .iter()
                .any(|l| l.contains("send('down', e.screenX, e.screenY)")),
            "pointerdown 必须立即上报 down（按下点）；首帧 move 才取它算锚点"
        );
        assert!(
            page_code_lines
                .iter()
                .any(|l| l.contains("send(moved ? 'dragend' : 'cancel')")),
            "pointercancel 未拖动必须发 cancel（只复位），不得发 click 弹主窗"
        );
        assert!(
            has_code_line(FLOAT_RS, "fn clear_drag_runtime(app: &tauri::AppHandle)"),
            "必须有 clear_drag_runtime 供 click / cancel / dragend 统一清按压 / 拖动态"
        );
        assert!(
            has_code_line(FLOAT_RS, "clamp_visible("),
            "拖动中必须软钳制（至少留窄边在屏内，可跨屏，不甩丢）"
        );
    }

    /// ⑩ v1.70：悬停缩回必须延迟 + 到期校验真实光标位置，治边缘抖动；
    ///    自由态松手必须钳回显示器；球窗必须等页面加载完成再 show。
    #[test]
    fn float_peek_delay_cursor_check_clamp_and_show_after_load() {
        assert!(
            FLOAT_RS.contains("const COLLAPSE_DELAY_MS: u64 = 300;"),
            "缩回必须延迟 300ms（边缘进出事件连环触发时防抖）"
        );
        assert!(
            has_code_line(
                FLOAT_RS,
                "fn finish_peek_leave(app: &tauri::AppHandle, gen: u64)"
            ),
            "必须有延迟到期的统一收口 finish_peek_leave"
        );
        assert!(
            FLOAT_RS.contains("peek_gen: AtomicU64"),
            "必须有悬停代际计数器，使排队中的旧缩回任务作废"
        );
        assert!(
            has_code_line(FLOAT_RS, "w.cursor_position()"),
            "到期必须读真实光标位置：光标已在缩回后矩形内则保持展开"
        );
        assert!(
            has_code_line(FLOAT_RS, "fn point_in_rect("),
            "光标位置必须做点包含判定（纯函数，有单测）"
        );
        assert!(
            has_code_line(FLOAT_RS, "fn clamp_visible("),
            "拖动中软钳制必须抽成纯函数（有单测）"
        );
        assert!(
            has_code_line(FLOAT_RS, ".on_page_load("),
            "球窗必须注册 on_page_load：页面 Finished 后再 show + push_state"
        );
        assert!(
            FLOAT_RS.contains("PageLoadEvent::Finished"),
            "必须在 Finished（不是 Started）时显现，避免透明空窗 / 首帧前 setState 丢失"
        );
        // 建窗函数里不得再立刻 show（show 已移到 on_page_load 回调）。
        let build_fn = &FLOAT_RS[FLOAT_RS
            .find("fn build_float_window_on_main")
            .expect("build_float_window_on_main 存在")..];
        let build_end = build_fn
            .find("\n}\n")
            .map(|i| i + 2)
            .unwrap_or(build_fn.len());
        assert!(
            !build_fn[..build_end].contains("window.show()"),
            "建窗后不得立刻 show：透明 WebView2 首帧前是看不见的空窗，show 必须在 on_page_load"
        );
    }

    /// ⑪ v1.72：单击必须**双击去抖** —— 系统双击在 ~100ms 内送来两次 `pointerup`（两次
    ///    `click`），而 `click` 是 toggle 主窗：两次相互抵消，用户只看到主窗「闪现」一下。
    ///    窗口期内的第二次点击必须丢弃（双击只认第一次），窗口之外的单击照常各算一次。
    #[test]
    fn float_page_click_is_debounced_against_double_click() {
        assert!(
            FLOAT_HTML.contains("var CLICK_DEBOUNCE_MS = 250;"),
            "球页必须有双击去抖窗口常量 CLICK_DEBOUNCE_MS（缺了双击会让主窗闪现）"
        );
        assert!(
            has_code_line(
                FLOAT_HTML,
                "if (now - lastClickAt < CLICK_DEBOUNCE_MS) return;"
            ),
            "窗口期内的第二次单击必须早退（丢弃），不得再发一次 click"
        );
        assert!(
            has_code_line(FLOAT_HTML, "lastClickAt = now;"),
            "通过去抖的单击必须记录时间戳，否则去抖恒不生效"
        );
        assert!(
            !FLOAT_HTML.contains("send(moved ? 'dragend' : 'click')"),
            "旧的未去抖单击写法必须消失（它就是双击闪现的来源）"
        );
    }

    /// ⑫ v1.72：球页把球摆进活柱靠 `left: 28px`（= `ICON_SIZE_LOGICAL / 2`）。这个 28 与
    ///    `PEEK_LOGICAL = 28` **数值巧合相同、含义无关** ⇒ 用跨文件等式钉住：改活柱边长
    ///    必须同步页面，否则球会在活柱里偏心（外观错位、命中区与球错开）。
    #[test]
    fn float_page_ball_offset_equals_half_icon_size() {
        assert!(
            FLOAT_RS.contains("const ICON_SIZE_LOGICAL: f64 = 56.0;"),
            "活柱边长常量必须是 56.0（页面 left:28px 与 Electron DEFAULT_FLOAT_W 都对齐它）"
        );
        assert!(
            FLOAT_HTML.contains("left: 28px;"),
            "锚左（活柱贴宿主左边）时球心必须在左起 28px = ICON_SIZE_LOGICAL/2"
        );
        assert!(
            FLOAT_HTML.contains("left: calc(100% - 28px);"),
            "锚右（右吸附活柱贴宿主右边）时球心必须在视口右起 28px = ICON_SIZE_LOGICAL/2"
        );
    }

    /// ⑭ v1.77b：球**必须铺满活柱**（球直径 == `ICON_SIZE_LOGICAL` == 活柱边长）。
    ///
    /// 真机现象「不选中时 悬浮图标背景异样」的第三个源头（前两个：v1.73 的球元素 title、
    /// v1.77 的任务栏按钮 / 标题）。实测残片的**外边界是以球心为圆心、半径恒定的一个圆**
    /// ⇒ 它就是「球（当时 46px）」与「区域圆（56px）」之间那圈**透明环带**被区域裁出来的
    /// 形状：那圈环带会把窗口后面的东西透出来，露成一圈浅色残片（残片载体后证实为
    /// WebView2 的 title 提示条**子窗**与任务栏浮层，已分别在 v1.73 / v1.77 移除）。
    ///
    /// ⚠️ v1.80 定稿：不变量是「**球 == 活柱 == 命中圆**，三者严格等大；视觉不经过窗口
    /// 区域」。历史三案（v1.77b 严格裁圆白锯齿 / v1.78 区域外扩漏浅色异形 / v1.79 页面
    /// 黑底暗锯齿）全部真机证伪，v1.80 起视觉走 NOREDIRECTIONBITMAP 真逐像素 alpha（守卫
    /// ④c `float_window_uses_per_pixel_alpha_and_circle_hittest`），页面保持真透明（守卫
    /// ④d `float_page_and_init_script_stay_truly_transparent`），命中靠 WM_NCHITTEST。
    #[test]
    fn float_page_ball_fills_the_live_pillar() {
        let size = icon_size_logical_int();

        // ① 球元素那条规则：宽高必须都等于活柱边长，且不得再靠 transform 放大
        //    （球已铺满，scale>1 会超出圆形命中区 = 看着在球上却点不着；双壳同理）。
        let ball = css_block(FLOAT_HTML, "#w2f-ball {");
        assert!(
            !ball.is_empty(),
            "floaticon.html 里找不到 #w2f-ball 的 CSS 规则块"
        );
        for needle in [format!("width: {size}px;"), format!("height: {size}px;")] {
            assert!(
                ball.contains(&needle),
                "#w2f-ball 必须 {needle}（= ICON_SIZE_LOGICAL = 活柱边长）：球比活柱小就会留出一圈透明环带"
            );
        }
        assert!(
            !ball.contains("scale("),
            "#w2f-ball 不得用 transform: scale() 放大：球已铺满活柱，放大只会被裁掉并重新露出透明环带"
        );

        // ② 悬停 / 拖动反馈改为提亮（filter），且同样不得 scale。
        let hover = css_block(FLOAT_HTML, "html #w2f-ball:hover {");
        assert!(
            hover.contains("filter: brightness(1.12)"),
            "悬停反馈必须是 filter: brightness(1.12)（提亮），不能靠放大"
        );
        assert!(!hover.contains("scale("), "悬停反馈不得使用 scale()");

        // ③ 半隐藏态的「压暗」必须用 filter，不能用 opacity ——
        //    opacity 会让整颗球半透明，等于把刚堵上的那层保护自己拆掉。
        let collapsed = css_block(FLOAT_HTML, "html.w2f-collapsed #w2f-ball {");
        assert!(
            collapsed.contains("filter: brightness("),
            "半隐藏态必须用 filter: brightness() 压暗"
        );
        assert!(
            !collapsed.contains("opacity"),
            "半隐藏态不得用 opacity：整颗球半透明会把窗口底色透出来（正是本条要防的现象）"
        );
    }

    /// ⑬ v1.72：capability 的 `description` 也是安全文档，必须与实现同步 —— 它曾写着
    ///    「单击球调出主窗口」（v1.71 已改为 toggle）且只列 6 个事件（v1.70 新增的
    ///    `float-down` / `float-cancel` 不在表里）。这两处都是静默漂移：没人会去读 JSON 里的说明。
    #[test]
    fn float_capability_description_lists_all_events_and_toggle_semantics() {
        let v: serde_json::Value =
            serde_json::from_str(FLOAT_CAP).expect("capabilities/float-icon.json 不是合法 JSON");
        let desc = v["description"]
            .as_str()
            .expect("float-icon.json 必须有 description");
        assert!(
            desc.contains("切换主窗口显隐"),
            "description 必须写明单击是 toggle 主窗显隐（v1.71 起，不再是「调出来」）"
        );
        for lit in [
            concat!("float-", "down"),
            concat!("float-", "move"),
            concat!("float-", "cancel"),
            concat!("float-", "drag-end"),
            concat!("float-", "peek-enter"),
            concat!("float-", "peek-leave"),
            concat!("float-", "hide"),
            concat!("float-", "click"),
        ] {
            assert!(
                desc.contains(lit),
                "capability description 必须列出事件 {lit}（否则安全文档与实际事件集漂移）"
            );
        }
        assert!(
            !desc.contains("单击球调出主窗口"),
            "description 不得再写「单击球调出主窗口」：v1.71 已改为 toggle"
        );
    }

    /// ⑭ v1.72：悬浮球真机探针的「假主窗」也必须设 WebView2 用户数据目录。
    ///
    /// 漏设的后果**不是**「换了个缓存目录」这么轻：真机实测 `WebviewWindowBuilder::build()`
    /// **永不返回** —— HWND 已经建出来（进程窗口列表里能看到探针给它的标题）、主线程此后也
    /// 不再执行任何任务（连排队的 `app.exit` 都送不到），整轮探针只能等看门狗给 98。
    /// 症状伪装成「WebView2 初始化偶发停摆」：本机为此白查了一整轮（换 UDF、换运行目录、
    /// 清残留 WebView2 进程全都无效，`run-D/E/F` 三次复现）。生产三处建窗
    /// （`lib.rs` 主窗 / `tabs.rs` tab 窗 / `floatwin.rs` 球窗）都设了它，探针这一处不能例外。
    #[test]
    fn float_probe_fake_main_window_sets_data_directory() {
        const FLOATPROBE_RS: &str = include_str!("floatprobe.rs");
        const NEEDLE: &str = concat!("Some(dir) => b.", "data_directory(dir),");
        assert!(
            has_code_line(FLOATPROBE_RS, NEEDLE),
            "floatprobe 的假主窗必须 data_directory(dir)（与生产主窗同一个 UDF；漏设 = build() 卡死）"
        );
        // 反向对照：把调用挖掉后同一判据必须不再成立，否则这条守卫咬不住人。
        let broken = FLOATPROBE_RS.replace(NEEDLE, "Some(dir) => b,");
        assert_ne!(
            broken, FLOATPROBE_RS,
            "反向对照：被测 needle 必须真的存在于源码里"
        );
        assert!(
            !has_code_line(&broken, NEEDLE),
            "反向对照失败：挖掉 data_directory 后判据仍成立 ⇒ 守卫是自欺的"
        );
    }

    /// 多页签宿主窗口也必须显式设 WebView2 数据目录。
    ///
    /// ⚠️ 这是一处**真实修掉的偏差**（v1.72）：`create_host_window` 原来漏了这一句，而它恰是
    /// 多页签模式下**最先创建 WebView2 环境**的窗口 —— 后果是①宿主的 WebView2 数据落到系统
    /// 默认目录（绿色版「不写程序目录之外」被破坏）；②`tabs.rs` 里那段「宿主是同 UDF 下第一个
    /// 创建环境的窗口，所以 proxy 参数必须挂在它身上」的注释前提不再成立（宿主与 tab 分裂成两个
    /// 环境）。两处都静默，编译 / 单测 / 探针全绿，只有真机往盘上看才现形。
    ///
    /// 判据为什么用 `builder.` 作接收者：`create_host_window` 的局部变量正是 `builder`，
    /// 而 tab 窗那处是 `window.`、主窗是 `wb.`、球窗是 `b.` —— 四种接收者互不相同，
    /// 于是这条 needle 在本文件里只能来自宿主窗那一行（不会被别处误满足）。
    #[test]
    fn tabs_host_window_sets_data_directory() {
        const TABS_RS: &str = include_str!("tabs.rs");
        const NEEDLE: &str = concat!("Some(dir) => build", "er.data_directory(dir),");
        assert!(
            has_code_line(TABS_RS, NEEDLE)
                && has_code_line(
                    TABS_RS,
                    "let builder = match crate::config::data_dir(app) {"
                ),
            "create_host_window 必须与主窗 / tab 窗同构显式设置 WebView2 数据目录"
        );
        // 反向对照：挖掉后同判据必须不再成立。
        let broken = TABS_RS.replace(NEEDLE, "Some(dir) => builder,");
        assert_ne!(
            broken, TABS_RS,
            "反向对照：被测 needle 必须真的存在于 tabs.rs"
        );
        assert!(
            !has_code_line(&broken, NEEDLE),
            "反向对照失败：挖掉 data_directory 后判据仍成立 ⇒ 守卫是自欺的"
        );
    }
}
