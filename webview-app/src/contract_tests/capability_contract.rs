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
/// 本文件源码：事件监听那一段的字面量在这里。
const LIB_RS: &str = include_str!("../lib.rs");
/// 设置窗的创建处 —— 窗口标签的真源（`WebviewWindowBuilder::new(app, "…")`）。
const SETTINGS_RS: &str = include_str!("../settings.rs");
/// 主窗的自绘标题栏（跑在远程页面里）。
const TITLEBAR_JS: &str = include_str!("../titlebar.js");
/// 主窗（单窗口模式）能力声明。
const MAIN_CAP: &str = include_str!("../../capabilities/default.json");
/// 设置窗（无边框）能力声明 —— 权限刻意与主窗分开。
const SETTINGS_CAP: &str = include_str!("../../capabilities/settings-window.json");
/// 多页签 tab 窗口（远程内容）能力声明 —— include_str! 同样把「文件缺失」变成编译失败。
#[cfg(windows)]
const TABS_CAP: &str = include_str!("../../capabilities/tabs-remote.json");
/// 多页签模块：事件名常量与标题回传落地函数在这里。
#[cfg(windows)]
const TABS_RS: &str = concat!(
    include_str!("../tabs.rs"),
    include_str!("../tabs/state.rs"),
    include_str!("../tabs/window.rs"),
    include_str!("../tabs/ipc.rs"),
    include_str!("../tabs/tests.rs"),
);
/// 注入到每个 tab 远程页面的标题回传脚本。
#[cfg(windows)]
const TAB_TITLE_JS: &str = include_str!("../tab_title.js");
/// 注入到每次导航的加载层脚本（tab 首屏重试事件也由它发出）。
#[cfg(windows)]
const LOADING_JS: &str = include_str!("../loading.js");

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
