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

// ===========================================================================
// Ctrl+W 关闭当前页签（多页签，双壳）
// ===========================================================================

/// 注入到每个 tab 远程页面的快捷键脚本（Ctrl+W → 核心事件）。
#[cfg(windows)]
const TAB_KEYS_JS: &str = include_str!("../tab_keys.js");
/// 双壳共用的标签栏页面 —— Ctrl+W 的「焦点在标签栏」那一半在这里。
#[cfg(windows)]
const SHELL_HTML: &str = include_str!("../shell.html");
/// Electron 侧同一判据（纯函数 `isCloseTabKey` + 挂载 `attachCloseTabShortcut`）。
#[cfg(windows)]
const ELECTRON_PAGE_BEHAVIOR: &str =
    include_str!("../../../electron-app/src/main/page-behavior.ts");
/// Electron 壳页面（标签栏）焦点路径的挂载点。
#[cfg(windows)]
const ELECTRON_INDEX: &str = include_str!("../../../electron-app/src/main/index.ts");
/// Electron 内容视图（远程页面）焦点路径的挂载点。
#[cfg(windows)]
const ELECTRON_TAB_SPAWN: &str = include_str!("../../../electron-app/src/main/tab-spawn.ts");

/// ⑦ tab 页面 Ctrl+W 事件：tabs.rs 常量 ↔ lib.rs 监听 ↔ tab_keys.js 字面量逐字一致；
///    脚本必须「带令牌 + 只在主框架 + capture + preventDefault」。
///
/// 为什么钉死：整条链路每一环漏掉都是**静默失效** —— 事件名差一个字符 ⇒ 永远不触发；
/// 少了 preventDefault ⇒ 页面自己的 Ctrl+W 同时生效（关页签 + 页面动作）；
/// 少了 token ⇒ 任何页面都能伪造关闭别的页签；少了主框架守卫 ⇒ 广告 iframe 里的按键
/// 也能关掉整个页签。
#[cfg(windows)]
#[test]
fn tab_close_event_contract_between_lib_tabs_and_script() {
    const CLOSE_IN_RUST: &str = concat!("\"", "web2app:close-tab", "\"");
    assert!(
        TABS_RS.contains(CLOSE_IN_RUST),
        "tabs.rs 必须定义 TAB_CLOSE_EVENT 字面量 {CLOSE_IN_RUST}"
    );
    assert!(
        LIB_RS.contains("app.listen(tabs::TAB_CLOSE_EVENT"),
        "lib.rs 必须监听 tabs::TAB_CLOSE_EVENT（否则 Ctrl+W 静默无效）"
    );
    const CLOSE_IN_JS: &str = concat!("'", "web2app:close-tab", "'");
    assert!(
        TAB_KEYS_JS.contains(CLOSE_IN_JS),
        "tab_keys.js 必须包含事件名 {CLOSE_IN_JS}"
    );
    assert!(
        TAB_KEYS_JS.contains("__web2appTabToken") && TAB_KEYS_JS.contains("__web2appTabLabel"),
        "tab_keys.js 必须携带建窗时注入的 label + 一次性令牌（伪造防护靠它）"
    );
    assert!(
        TAB_KEYS_JS.contains("plugin:event|emit"),
        "tab_keys.js 必须走核心事件通路（远程 origin 的应用命令必被 ACL 拒）"
    );
    assert!(
        TAB_KEYS_JS.contains("window.self !== window.top"),
        "tab_keys.js 必须在子框架早退：初始化脚本无 frame 过滤，广告 iframe 里按 Ctrl+W \
         不该关掉宿主页签"
    );
    assert!(
        TAB_KEYS_JS.contains("preventDefault()") && TAB_KEYS_JS.contains("stopPropagation()"),
        "tab_keys.js 必须吃掉这次按键：否则页面自己的 Ctrl+W 会与关页签同时生效"
    );
    // capture 阶段：站点自己的 keydown（编辑器类页面常见）不能抢在前面。
    assert!(
        TAB_KEYS_JS.contains("}, true);"),
        "tab_keys.js 的 keydown 必须用 capture 阶段监听（第三个参数 true）"
    );
    assert!(
        TAB_KEYS_JS.contains("e.repeat"),
        "tab_keys.js 必须排除长按重复（与 Electron 侧 isAutoRepeat 同一条取舍）"
    );
}

/// ⑧ Ctrl+W 的两条焦点路径都要接住，且双壳判据一致。
///
/// 漏任一环的后果都是「有时能关、有时关不掉」：
///   · Tauri：`tabs/window.rs` 未注入 tab_keys.js ⇒ 焦点在网页里时按键被页面吃掉；
///   · Tauri：`shell.html` 没有 keydown ⇒ 焦点在标签栏时按键没有任何效果；
///   · Electron：`tab-spawn.ts` / `index.ts` 少挂一处 ⇒ 同上的那一半失效。
///
/// ⚠️ `#[cfg(windows)]`：本函数要读 `TABS_RS` / `TAB_KEYS_JS`，而多页签模块整体
///    Windows-only —— 不加就会在 Linux / Android 目标上因「常量不存在」编译失败。
#[cfg(windows)]
#[test]
fn tab_close_shortcut_wired_on_both_focus_paths() {
    // Tauri：注入（焦点在网页里）
    assert!(
        TABS_RS.contains(r#"include_str!("../tab_keys.js")"#),
        "tabs/window.rs 的建窗 builder 必须注入 tab_keys.js（焦点在网页内时的唯一通路）"
    );
    // 双壳：壳页面 keydown（焦点在标签栏）
    assert!(
        SHELL_HTML.contains("document.addEventListener(\"keydown\"")
            && SHELL_HTML.contains(r#"invoke("close_tab", { label: active.label })"#),
        "shell.html 必须有 Ctrl+W → close_tab 的 keydown 处理（焦点在标签栏时的通路）"
    );
    // Electron：纯函数 + 两个挂载点
    assert!(
        ELECTRON_PAGE_BEHAVIOR.contains("export function isCloseTabKey")
            && ELECTRON_PAGE_BEHAVIOR.contains("export function attachCloseTabShortcut"),
        "page-behavior.ts 必须提供 isCloseTabKey / attachCloseTabShortcut（与 JS 侧同一判据）"
    );
    assert!(
        ELECTRON_INDEX.contains("attachCloseTabShortcut(win.webContents"),
        "index.ts 必须在壳页面上挂 Ctrl+W（焦点在标签栏时的通路）"
    );
    assert!(
        ELECTRON_TAB_SPAWN.contains("attachCloseTabShortcut(view.webContents"),
        "tab-spawn.ts 必须在每个内容视图上挂 Ctrl+W（焦点在网页内时的通路）"
    );
    // 双壳判据必须一致：都排除 Shift / Alt / Meta 与长按。
    for (file, needle) in [
        (TAB_KEYS_JS, "e.shiftKey || e.altKey || e.metaKey"),
        (SHELL_HTML, "e.shiftKey || e.altKey || e.metaKey"),
        (
            ELECTRON_PAGE_BEHAVIOR,
            "input.shift || input.alt || input.meta",
        ),
    ] {
        assert!(
            file.contains(needle),
            "Ctrl+W 的修饰键白名单必须双壳一致（缺 `{needle}`）：Ctrl+Shift+W 在浏览器里是\
             关窗，本壳不提供那条出口"
        );
    }
    // 双壳判据必须一致（之二）：都按**逻辑键位**（'w' / 'W'）认键，不用物理码。
    for (file, needle) in [
        (TAB_KEYS_JS, "e.key === 'w'"),
        (SHELL_HTML, "e.key === \"w\""),
        (ELECTRON_PAGE_BEHAVIOR, "input.key === 'w'"),
    ] {
        assert!(
            file.contains(needle),
            "Ctrl+W 必须按逻辑键位判（缺 `{needle}`）"
        );
    }
    // 反向：禁物理键位兜底。`e.code` 按 US 布局命名 —— AZERTY 上「生出 'w' 的那个物理键」
    // 的 code 是 KeyZ，用 code 匹配等于把 Ctrl+Z（撤销）变成关页签。
    // ⚠️ 字面量拆成两段写：整串出现在注释里也会让断言自我满足。
    for bad in [
        concat!("e.code ===", " 'KeyW'"),
        concat!("e.code ===", " \"KeyW\""),
    ] {
        assert!(
            !TAB_KEYS_JS.contains(bad) && !SHELL_HTML.contains(bad),
            "Ctrl+W 不得按物理键位匹配（{bad}）：AZERTY 上物理 KeyW 敲出的是 'z'，\
             会把 Ctrl+Z 撤销变成关页签；Electron 侧同样只看 key，双壳必须一致"
        );
    }
}
