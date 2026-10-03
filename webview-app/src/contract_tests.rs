//! 静态契约测试（只在 `cargo test` 下编译，由 lib.rs 的 `#[cfg(test)] mod contract_tests;` 挂入）：
//!   · `ipc_command_registry` —— IPC 命令注册表唯一调用点 + 前端命令 ⊆ 注册表；
//!   · `capability_contract` —— capabilities 授权 + Rust/JS 两侧事件名字面量。
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
