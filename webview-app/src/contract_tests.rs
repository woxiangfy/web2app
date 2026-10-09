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
