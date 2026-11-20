/// 移动端底栏（mobile_bar）的静态契约测试。
///
/// 存在理由：这一整块功能**本机跑不到**（移动端目标只能靠 CI / 真机），而它又全是
/// 「静默失败」体质 —— 事件名两侧不逐字一致、cfg 脚本与脚本注册顺序颠倒、按钮无条件渲染，
/// 三者的表象都是「点了没反应」且零报错。所以每一条都写成**成对可观测判据**：
/// 改坏了必须 FAIL，而不是无声降级。
///
/// 覆盖：
///   · 前端 ↔ Rust 三个事件名逐字一致；
///   · cfg 脚本必须排在 底栏脚本 之前（初始化脚本按注册顺序执行）；
///   · 按钮清单与顺序（首页 / 后退 / 前进 / [列表] / 更多）；
///   · 「列表」按钮**由页签可用性决定**，不是无条件渲染（死按钮守卫）；
///   · 「更多」面板无条件渲染且固定最后：两个动作（清除浏览数据 / 退出应用，2026-11-20
///     按产品需求精简）+ Android 原生桥要素 + 密钥 + 两步清数据；
///   · 底部空间契约（与顶部空间契约镜像）+ 前进后退落点守卫 + 启动页不画底栏；
///   · 打包配置字段 `mobile_bar` / `android_permissions` 的**语义**（白名单三值、
///     归一函数、`#[cfg(mobile)]` 形态、include_str! 唯一注入点）；
///     「字段在各真源里都存在」已数据驱动化 → packaging_field_contract.rs。
const LIB_RS: &str = include_str!("../lib.rs");
const BAR_RS: &str = include_str!("../mobilebar.rs");
/// 底栏脚本 = `lib.rs` 里 `include_str!("mobilebar/<part>.js")` 的**按序拼接**：
/// 运行时就是**同一个 IIFE**（五片共享闭包作用域）。这里从 `lib.rs` 派生分片清单，
/// 不手抄第二份，拼出与运行时逐字相同的整脚本。
fn bar() -> &'static str {
    use std::sync::OnceLock;
    static SCRIPT: OnceLock<String> = OnceLock::new();
    SCRIPT.get_or_init(|| {
        let mut out = String::new();
        let mut rest = LIB_RS;
        while let Some(i) = rest.find("include_str!(\"mobilebar/") {
            let start = i + "include_str!(\"".len();
            let end = rest[start..]
                .find('"')
                .map(|x| start + x)
                .unwrap_or_else(|| panic!("lib.rs 的 include_str!(\"mobilebar/ 缺闭合引号"));
            let rel = &rest[start..end];
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join(rel);
            out.push_str(
                &std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("读不到底栏分片 {}：{e}", path.display())),
            );
            rest = &rest[end..];
        }
        assert!(
            !out.is_empty(),
            "lib.rs 没有按 `include_str!(\"mobilebar/…)` 注入底栏脚本"
        );
        out
    })
}
const LOADING_JS: &str = include_str!("../loading.js");
const CONFIG_RS: &str = include_str!("../config.rs");
/// 「更多」面板的原生侧实现（Android 打包期注入，本机跑不到，只能静态钉住桥要素）。
const EXTRAS_PY: &str = include_str!("../../../.github/scripts/patch-android-extras.py");
const BUILD_CUSTOM_YML: &str = include_str!("../../../.github/workflows/build-custom.yml");
const RELEASE_YML: &str = include_str!("../../../.github/workflows/release.yml");
const PARSER_PY: &str = include_str!("../../../.github/scripts/parse-packaging-config.py");
const PARSER_MJS: &str = include_str!("../../../scripts/packlib/constants.mjs");
/// Node 复刻的解析主体（DEFAULTS/白名单在 constants.mjs，归一函数在 parser.mjs）。
const PARSER_MAIN_MJS: &str = include_str!("../../../scripts/packlib/parser.mjs");
const ELECTRON_CONFIG: &str = include_str!("../../../electron-app/src/main/config.ts");
/// #5 按需运行时权限的 Manifest 注入脚本（Android 打包期执行，本机只能静态钉住）。
const PERMISSIONS_PY: &str = include_str!("../../../.github/scripts/patch-android-permissions.py");
/// #6 H5 视频全屏的 RustWebChromeClient 补丁脚本（同上，只能静态钉住要素）。
const FULLSCREEN_PY: &str = include_str!("../../../.github/scripts/patch-android-fullscreen.py");
/// #9 / #10 返回键补丁（浮层优先 + 再按一次退出），与 JS 侧 closeSheet 是跨文件契约。
const BACK_PY: &str = include_str!("../../../.github/scripts/patch-android-back.py");
/// #18 明文流量开关补丁（network_security_config，两壳 workflow 共用同一脚本）。
const CLEARTEXT_PY: &str = include_str!("../../../.github/scripts/patch-android-cleartext.py");
// 字段在各真源里的**存在性**（含两份 config.example.json）由 packaging_field_contract.rs
// 按 `scripts/packaging-fields.json` 数据驱动巡检；本文件只钉语义形态。

/// 事件名：拆成两段拼接，避免本文件自己的字面量把断言「自我满足」。
/// ⚠️ JS 侧用的是单引号字面量，Rust 侧是双引号常量值 —— 两侧写法不同，各按各的找。
const NAV_HOME_IN_JS: &str = concat!("'", "web2app:nav-home", "'");
const NAV_HOME_IN_RUST: &str = concat!("\"", "web2app:nav-home", "\"");
const SWITCH_IN_JS: &str = concat!("'", "web2app:switch-tab", "'");
const SWITCH_IN_RUST: &str = concat!("\"", "web2app:switch-tab", "\"");
const CLOSE_IN_JS: &str = concat!("'", "web2app:close-tab", "'");
const CLOSE_IN_RUST: &str = concat!("\"", "web2app:close-tab", "\"");
const OPEN_IN_JS: &str = concat!("'", "web2app:open-tab", "'");
const OPEN_IN_RUST: &str = concat!("\"", "web2app:open-tab", "\"");
const REQUEST_IN_JS: &str = concat!("'", "web2app:request-tabs", "'");
const REQUEST_IN_RUST: &str = concat!("\"", "web2app:request-tabs", "\"");
const DOC_IN_JS: &str = concat!("'", "web2app:mobile-doc", "'");
const DOC_IN_RUST: &str = concat!("\"", "web2app:mobile-doc", "\"");

/// 只看「代码行」：剥掉整行注释后再匹配，否则针会被注释满足（见 `mod.rs::has_code_line`）。
use super::has_code_line;

/// ① 三个事件名：前端字面量 ↔ Rust 常量 ↔ Rust 监听点，缺一侧就是「点了没反应」。
///
/// ⚠️ Rust 侧用的是**常量**（`NAV_HOME_EVENT` 等）而不是字面量，所以「监听点」这一侧
///    按常量名找；常量值与前端字面量的逐字一致由 `mobilebar.rs` 自己的单测钉住
///    （`event_names_are_stable`），两侧合起来才是完整闭环。
#[test]
fn event_names_match_between_page_and_rust() {
    for (js, listen, name) in [
        (
            NAV_HOME_IN_JS,
            "app.listen(NAV_HOME_EVENT",
            "web2app:nav-home",
        ),
        (
            SWITCH_IN_JS,
            "app.listen(SWITCH_TAB_EVENT",
            "web2app:switch-tab",
        ),
        (
            CLOSE_IN_JS,
            "app.listen(CLOSE_TAB_EVENT",
            "web2app:close-tab",
        ),
        (OPEN_IN_JS, "app.listen(OPEN_TAB_EVENT", "web2app:open-tab"),
        (
            REQUEST_IN_JS,
            "app.listen(REQUEST_TABS_EVENT",
            "web2app:request-tabs",
        ),
        (DOC_IN_JS, "app.listen(DOC_EVENT", "web2app:mobile-doc"),
    ] {
        assert!(
            bar().contains(js),
            "src/mobilebar/ 缺少事件字面量 {name}（找的是 {js}）"
        );
        assert!(
            has_code_line(BAR_RS, listen),
            "src/mobilebar.rs 必须监听 {name}（找的是 {listen}），否则按钮点了没人接"
        );
    }
    // 常量值本身必须与前端字面量同源（改一处忘另一处 = 静默失效）。
    for (rust, name) in [
        (NAV_HOME_IN_RUST, "web2app:nav-home"),
        (SWITCH_IN_RUST, "web2app:switch-tab"),
        (CLOSE_IN_RUST, "web2app:close-tab"),
        (OPEN_IN_RUST, "web2app:open-tab"),
        (REQUEST_IN_RUST, "web2app:request-tabs"),
        (DOC_IN_RUST, "web2app:mobile-doc"),
    ] {
        assert!(
            BAR_RS.contains(rust),
            "src/mobilebar.rs 的事件常量值必须是 {name}（找的是 {rust}）"
        );
    }
}

/// ② 前端必须走**核心事件**通路：远程页面调应用命令必被 ACL 拒（与 titlebar.js 同约束）。
#[test]
fn page_emits_core_events_not_app_commands() {
    assert!(
        bar().contains("plugin:event|emit"),
        "src/mobilebar/ 必须走核心事件通路（__TAURI_INTERNALS__ 兜底）"
    );
    // 首页要能工作，capability 必须已经授予 emit —— default.json 那条由
    // capability_contract::main_capability_grants_remote_titlebar_permissions 钉着。
}

/// ③ cfg 脚本必须排在底栏脚本**之前**（初始化脚本按注册顺序执行）。
///    顺序反了 ⇒ 底栏照画，但高度与「列表」按钮不再由壳决定（静默错位）。
#[test]
fn config_script_is_registered_before_bar_script() {
    let cfg_at = LIB_RS
        .find("mobilebar::config_script()")
        .unwrap_or_else(|| panic!("lib.rs 必须注入 mobilebar::config_script()"));
    let js_at = LIB_RS
        .find("include_str!(\"mobilebar/")
        .unwrap_or_else(|| panic!("lib.rs 必须按 `include_str!(\"mobilebar/…)` 注入底栏脚本"));
    assert!(
        cfg_at < js_at,
        "cfg 脚本必须排在底栏脚本之前注册（当前 cfg@{cfg_at} js@{js_at}）：\
             初始化脚本按注册顺序执行，反了会让底栏用默认值渲染"
    );
}

/// ④ 按钮清单与顺序：首页 / 后退 / 前进 / [列表] / 更多（更多恒在最后；列表仅多页签时
///    插在更多之前）。
#[test]
fn bar_renders_home_back_forward_list_then_more() {
    let kinds = [
        ("home", "首页"),
        ("back", "后退"),
        ("fwd", "前进"),
        ("list", "列表"),
        ("more", "更多"),
    ];
    let mut last = 0usize;
    for (cls, label) in kinds {
        let needle = format!("btn('{cls}',");
        let at = bar()
            .find(&needle)
            .unwrap_or_else(|| panic!("src/mobilebar/ 必须渲染「{label}」按钮（{needle}）"));
        assert!(
            at > last,
            "底栏按钮顺序必须是 首页 → 后退 → 前进 → [列表] → 更多（{label} 位置不对）"
        );
        last = at;
        assert!(
            bar().contains(&format!("w2m-{cls}")),
            "按钮 {label} 必须带 w2m-{cls} 类名（样式与交互选择器都靠它）"
        );
    }
}

/// ④b 「更多」按钮必须**无条件**渲染（不像列表受 HAS_TABS 控制）—— 清数据/退出等
///     动作与是否多页签无关；它要是也被 HAS_TABS 门控，单页签包就没有任何动作入口。
#[test]
fn more_button_is_unconditional() {
    let push = "items.push(btn('more'";
    let at = bar()
        .find(push)
        .unwrap_or_else(|| panic!("必须无条件 items.push 更多按钮（找 {push}）"));
    // push 之前的同一行文本只允许是空白 —— 列表按钮那行的形态是
    // `if (HAS_TABS) items.push(...)`，更多按钮绝不能带这个条件前缀。
    let line_start = bar()[..at].rfind('\n').map_or(0, |i| i + 1);
    let before_push = bar()[line_start..at].trim();
    assert!(
        before_push.is_empty(),
        "「更多」按钮必须独立成行无条件 push（行首不得有 if 等前缀），实际前缀：{before_push:?}"
    );
    // 列表仍必须保持条件渲染（防顺手把条件也删了）。
    assert!(
        has_code_line(bar(), "if (HAS_TABS) items.push(btn('list'"),
        "列表按钮的 HAS_TABS 条件渲染必须保留"
    );
}

/// ④c 「更多」面板：**只保留两个动作**（清除浏览数据 / 退出应用，2026-11-20 产品需求
///     精简，其余动作与地址展示全部下线）；两动作必须走 `__web2appAndroid` 桥并带一次性
///     密钥；清数据必须两步确认；危险动作必须有主框架门控（iframe 拿不到密钥 / 调不了桥）。
#[test]
fn more_panel_actions_and_bridge_contract() {
    // 两个菜单项（文案即真机可见契约，改文案先改测试）。
    for label in ["清除浏览数据", "退出应用"] {
        assert!(bar().contains(label), "「更多」面板必须含菜单项「{label}」");
    }
    // 反例锁定：已下线的四行与地址头一个都不许回来（改回即本测试 FAIL）。
    for gone in [
        "moreRow(ICO_REFRESH",
        "moreRow(ICO_COPY",
        "moreRow(ICO_SHARE",
        "moreRow(ICO_EXTERNAL",
        "w2m-morehead",
        "copyCurrentLink",
    ] {
        assert!(
            !bar().contains(gone),
            "「更多」面板已精简为两行，不得再出现：{gone}"
        );
    }
    // 桥名 / 密钥 JS 变量必须与 patch-android-extras.py 注入侧逐字一致。
    for needle in [
        "__web2appAndroid",
        "__web2appAndroidKey",
        "b.clearData(bridgeKey()",
        "b.quitApp(bridgeKey()",
    ] {
        assert!(
            bar().contains(needle),
            "底栏脚本 必须包含桥调用要素：{needle}"
        );
    }
    // 主框架门控：第三方 iframe 内即便脚本被注入也不许调危险动作。
    assert!(
        bar().contains("window.top === window.self"),
        "桥调用前必须判定主框架（window.top === window.self），挡住第三方 iframe"
    );
    // 清数据两步确认（首次进入 armed，二次才调桥；3s 自动撤防）。
    assert!(
        bar().contains("再点一次确认清除") && bar().contains("clearArmed"),
        "清除浏览数据必须两步确认（armed 态 + 再点一次文案）"
    );
    assert!(
        bar().contains("}, 3000)"),
        "清数据确认态必须有超时自动撤防（3s）"
    );
    // 反馈通道：toast 元素与桥缺失提示。
    assert!(
        bar().contains("w2m-toast") && bar().contains("当前版本不支持该操作"),
        "面板操作必须有 toast 反馈，且桥缺失时不能静默"
    );
}

/// ④d 原生桥补丁脚本（patch-android-extras.py）的静态契约：tauri 在 Android 的
///     PlatformWebview 没有 wry 清数据入口，只能 Kotlin @JavascriptInterface；且 wry 0.57
///     自带的 clearAllBrowsingData() 漏清 Cookie 与 WebStorage，所以桥里必须自己补全。
///     密钥在主框架 onPageFinished 轮换并经 evaluateJavascript 注入（iframe 读不到）。
#[test]
fn extras_patch_script_has_bridge_and_full_cleardata() {
    for needle in [
        "addJavascriptInterface",
        "\"__web2appAndroid\"",
        "@JavascriptInterface",
        // 四个桥方法（JS 侧 ④c 调 clearData/quitApp；share/openBrowser 为预留桥能力，
        // 2026-11-20 起没有 UI 入口）。
        "fun share(",
        "fun openBrowser(",
        "fun clearData(",
        "fun quitApp(",
        // 清数据配方：wry 漏的两项必须在桥里补齐。
        "CookieManager.getInstance().removeAllCookies",
        "WebStorage.getInstance().deleteAllData()",
        "clearCache(true)",
        "deleteDatabase(\"webview.db\")",
        // 清完在 Cookie 回调里 reload（页面要带着干净状态重新走鉴权）。
        "location.reload()",
        // 密钥模型：主框架 onPageFinished 轮换 + evaluateJavascript 注入。
        "object Web2appBridge",
        "fun rotate()",
        "onPageFinished(view: WebView, url: String)",
        // 密钥变量名与 JS 侧逐字一致；注入语句是 .format 模板（{key_var} 占位）。
        "KEY_VAR = \"__web2appAndroidKey\"",
        "window.{key_var}=",
        // 退出语义：移除任务 + 杀进程兜底。
        "finishAndRemoveTask()",
        "Process.killProcess",
        // 幂等标记（两处独立）。
        "web2app-android-extras",
        "web2app-android-bridge-key",
    ] {
        assert!(
            EXTRAS_PY.contains(needle),
            "patch-android-extras.py 必须包含：{needle}"
        );
    }
}

/// ④e 两个 Android 打包 workflow 必须都挂 extras 补丁步，ci.yml 的 selftest 门禁名单
///     也必须收录（否则脚本改坏不会红）。
#[test]
fn workflows_wire_extras_patch() {
    for (name, yml) in [
        ("build-custom.yml", BUILD_CUSTOM_YML),
        ("release.yml", RELEASE_YML),
    ] {
        assert!(
            yml.contains("patch-android-extras.py"),
            "{name} 的 Android job 必须调用 patch-android-extras.py（更多面板原生桥）"
        );
    }
    // ci.yml 的自测循环已改为**按目录发现**（不再手写名单）⇒ 覆盖条件是「脚本自带 --selftest」。
    assert!(
        EXTRAS_PY.contains("--selftest"),
        "patch-android-extras.py 必须自带 --selftest，否则 ci.yml 的目录发现循环会跳过它"
    );
}

/// ⑤ 死按钮守卫：「列表」必须**由页签可用性决定**，不是无条件渲染；而壳侧判据就是
///    打包配置 `TABS_ENABLED`（移动端已落地单 webview 页签，不再按平台压死成 false）。
#[test]
fn list_button_is_conditional_on_tab_availability() {
    assert!(
        has_code_line(bar(), "if (HAS_TABS) items.push("),
        "「列表」按钮必须由 HAS_TABS 决定是否渲染（无条件渲染 = 移动端出现死按钮）"
    );
    // 判据的另一半在壳侧：tabs_available() 必须直接取 TABS_ENABLED —— 移动端单
    // webview 页签已落地，不能再像旧实现那样 cfg!(windows) && 把 Android 恒压成 false。
    assert!(
        has_code_line(BAR_RS, "TABS_ENABLED"),
        "mobilebar.rs::tabs_available() 必须按 TABS_ENABLED 判定（移动端页签已落地）"
    );
    // config.rs 也不得再把非 Windows 钳制成 false（否则开了 tabs_enabled 也不出按钮）。
    assert!(
        !has_code_line(
            CONFIG_RS,
            "cfg!(windows) && env_bool!(\"TABS_ENABLED\", false)"
        ),
        "config.rs 不得再用 cfg!(windows) 钳制 TABS_ENABLED：移动端底栏要按它渲染列表按钮"
    );
    // cfg 里必须把这个结论下发给页面，否则前端无从判断。
    assert!(
        BAR_RS.contains("__web2appMobileBarCfg"),
        "mobilebar.rs::config_script() 必须下发 __web2appMobileBarCfg（页面读的就是它）"
    );
    assert!(
        BAR_RS.contains("tabs_available()"),
        "config_script() 必须把 tabs_available() 的结论一起下发"
    );
    assert!(
        bar().contains("CFG.tabs === true"),
        "src/mobilebar/ 必须按下发的 tabs 标记判定（不能用别处推断）"
    );
    // 反例锁定：`tabs_available()` 不得返回常量 true（改坏这条测试应 FAIL）。
    assert!(
        !has_code_line(BAR_RS, "fn tabs_available() -> bool { true }"),
        "tabs_available() 不得无条件返回 true"
    );
}

/// ⑤b 单 webview 页签的端到端骨架必须齐：打开时拉取最新行、页面上报 url/标题、
///     壳侧把四元组 eval 回 setTabs，并且首页固定位次 0。
#[test]
fn mobile_tab_skeleton_is_wired_end_to_end() {
    // 「新建标签」入口已下线（2026-11-20）：新建页签只剩 target=_blank 的 open-tab。
    assert!(
        !bar().contains("w2m-newrow") && !bar().contains("'web2app:new-tab'"),
        "列表面板不得再有「新建标签」行（w2m-newrow / new-tab 一并下线）"
    );
    assert!(
        bar().contains("'web2app:request-tabs'"),
        "打开面板时必须请求壳侧推最新行数据"
    );
    assert!(
        bar().contains("'web2app:mobile-doc'") && bar().contains("watchDoc"),
        "页面必须经 mobile-doc 事件上报 url/标题（壳侧才能维护标签记录）"
    );
    assert!(
        BAR_RS.contains("__web2appMobileBar.setTabs("),
        "壳侧必须经 eval setTabs 把行数据推回面板"
    );
    assert!(
        BAR_RS.contains("HOME_TAB_LABEL") && bar().contains("if (!home)"),
        "首页标签固定不可关闭：Rust 侧挡 0 号位，JS 侧只给非首页画关闭按钮"
    );
}

/// ⑤d **Android UI 线程禁调阻塞型 webview getter**（2026-11-01 真机缺陷的守卫）。
///
/// 缺陷形态（`tabs_enabled=true` + `mobile_bar=true` 的包：安装正常、打开白屏、
/// 约 10 秒后应用自动退出）：
/// ```text
/// RustWebViewClient.onPageFinished（Android UI 线程）
///   → JNI Rust.onPageLoaded（wry binding.rs，同步在该线程执行）
///   → tauri on_page_load 闭包（lib.rs）
///   → mobilebar::on_page_finished → window.url()
///        = MainPipe::send(GetUrl) + recv_timeout(10s)
///        而 MAIN_PIPE 的消费方就是这条 UI 线程（onFirstActivityCreateWry 注册的 looper fd 回调）
///        ⇒ 必然僵 10 秒后超时
///   → 那条 GetUrl 仍留在管道里；UI 线程腾出来后处理它，
///     main_pipe.rs 结尾 `tx.send(url).unwrap()` 撞上已丢弃的接收端 ⇒ panic ⇒ abort ⇒ 进程退出
/// ```
/// 判据成对：① `mobilebar.rs` 的**代码行**里不得出现任何 `.url()` 取地址调用
/// （`window.url()` / `w.url()` / `main_window(app).map(|w| w.url())` 都是同一个缺陷形态，
/// 故针取 `.url()` 而不是特定接收者的写法 —— 第一版只钉 `window.url()`，
/// 换成 `w.url()` 的注入就**假绿**了，见 2026-11-01 反向对照记录）；
/// ② 当前地址必须由 `CurrentUrl` 缓存供给；③ `on_page_finished` 必须收 `url` 参数，
/// 且 `lib.rs` 必须把 `on_page_load` payload 的地址传进去（改回无参 = 缺陷原样复发）。
#[test]
fn mobile_bar_never_queries_webview_on_ui_thread() {
    // ① 禁现问地址（`has_code_line` 先剥注释，避免针被注释满足）。
    assert!(
        !has_code_line(BAR_RS, ".url()"),
        "mobilebar.rs 不得调 `.url()` 取 webview 地址：Android 上它走 MainPipe + \
         recv_timeout(10s)，而调用点就在消费该管道的 UI 线程上 ⇒ 僵 10 秒后触发 wry \
         的 tx.send().unwrap() panic ⇒ 进程 abort（用 CurrentUrl 缓存代替）"
    );
    // ② 替代品必须是缓存（状态 + 两个写入方 + 一个读取方判据）。
    assert!(
        has_code_line(BAR_RS, "pub(crate) struct CurrentUrl(Mutex<String>)"),
        "必须用 CurrentUrl 缓存当前地址（否则只能回头问 webview）"
    );
    assert!(
        has_code_line(BAR_RS, "fn remember_url(app: &AppHandle, url: &str)"),
        "缓存必须有写入函数 remember_url"
    );
    assert!(
        has_code_line(BAR_RS, "remember_url(app, url);"),
        "on_page_finished 必须把调用方传来的 url 写进缓存"
    );
    assert!(
        has_code_line(BAR_RS, "remember_url(app, &p.url);"),
        "doc_reported 也必须写缓存（SPA pushState 不触发文档加载完成事件）"
    );
    assert!(
        has_code_line(
            BAR_RS,
            "fn at_same_target(app: &AppHandle, target: &url::Url) -> bool"
        ),
        "「是否已在同一落点」必须走 at_same_target（读缓存），不得读 webview"
    );
    // ③ 接线：签名收 url + lib.rs 把 payload 地址传进来。
    assert!(
        has_code_line(
            BAR_RS,
            "pub(crate) fn on_page_finished(app: &AppHandle, url: &str)"
        ),
        "on_page_finished 必须收 url 参数（无参就只能回头问 webview）"
    );
    assert!(
        has_code_line(
            LIB_RS,
            "crate::mobilebar::on_page_finished(window.app_handle(), payload.url().as_str());"
        ),
        "lib.rs 的 on_page_load 闭包必须把 payload.url() 传给 on_page_finished"
    );
}

/// ⑤c 窄贴底件（FAB）不得再被「半屏宽」门槛漏掉 —— 真机上被底栏切掉半截的就是它。
///    同时上移后必须保留元素原本的 px 底边距，不能把悬浮按钮顶死在栏边上。
#[test]
fn fixed_bottom_elements_shift_regardless_of_width() {
    assert!(
        bar().contains("var bottomFixed = atBottom && pos === 'fixed'"),
        "position:fixed 的贴底元素必须无宽度门槛上移（FAB / 客服球只有 40–64px 宽）"
    );
    assert!(
        bar().contains("function shiftedBottom(")
            && bar().contains("'calc(' + BAR_H + 'px + ' + SAFE + ' + ' + n + 'px)'"),
        "上移贴底元素时必须把它原本的 px 底边距加回（保留悬浮按钮与栏之间的空隙）"
    );
}

/// ⑤d 加载层必须让出底栏：loading.js 读 __web2appMobileBarH 设置 bottom 且把 height
///     改成 calc（只设 bottom 挡不住 height:100vh 的层继续铺满底部）；底栏脚本
///     负责在挂好（及旋转）后设置该全局并广播 web2app:chrome-ready。
#[test]
fn loading_overlay_reserves_the_mobile_bar() {
    assert!(
        LOADING_JS.contains("__web2appMobileBarH"),
        "loading.js 必须读取底栏预留高度（__web2appMobileBarH）"
    );
    assert!(
        LOADING_JS.contains("setProperty('bottom'") && LOADING_JS.contains("calc(100vh - "),
        "loading.js 必须同时设置 bottom 与 calc 高度，否则加载层继续盖住底栏"
    );
    assert!(
        bar().contains("window.__web2appMobileBarH = reservePx()")
            && bar().contains("new Event('web2app:chrome-ready')"),
        "底栏脚本 挂好后必须广播实测底栏高度（加载层靠它让出底部一格）"
    );
    // 启动页不画底栏 ⇒ 早期近似值只能放在「内置页早退」之后，不能让 splash 加载层
    // 莫名空出一条底栏。
    let early = bar()
        .find("window.__web2appMobileBarH = BAR_H")
        .unwrap_or_else(|| panic!("底栏脚本 必须在 document-start 先给 BAR_H 首帧近似值"));
    let bail = bar()
        .find("isBuiltinUrl(location.href)")
        .unwrap_or_else(|| panic!("底栏脚本 必须保留内置页早退"));
    assert!(
        bail < early,
        "首帧近似高度必须写在内置页早退之后（启动页没有底栏，不能留底部空白）"
    );
}

/// ⑥ 底部空间契约（顶部空间契约的镜像）：底栏占了屏幕底部一格，页面必须让出来，
///    否则页面自己的页脚 / 贴底操作条会被底栏盖住。
#[test]
fn bottom_space_contract_mirrors_the_top_one() {
    assert!(
            bar().contains("html{box-sizing:border-box!important;height:100%!important;overflow:hidden!important;}"),
            "底部空间契约要求 html 不滚（body 当滚动容器），否则滚动条会画到底栏上"
        );
    assert!(
        bar().contains("margin-bottom:"),
        "body 必须让出底部一格（margin-bottom）—— padding 会让滚动条穿过去"
    );
    assert!(
        bar().contains("min-height:0!important;height:"),
        "body 的 min-height 必须归零：页面常写 min-height:100vh，会把 body 撑高一格底栏"
    );
    // 页面自己的贴底固定条 / 100vh 容器也要一起适配
    assert!(
        bar().contains("applyChromeFit") && bar().contains("SHRINK_ATTR"),
        "底栏必须把页面自己的「贴底固定条 / 按视口高度定尺寸的容器」一起适配"
    );
    // 安全区：手势导航条那一截由 env() 补，不能写死
    assert!(
        bar().contains("env(safe-area-inset-bottom, 0px)"),
        "底栏必须让出 env(safe-area-inset-bottom)：手势导航条高度随设备变化，写死必然压到系统条"
    );
}

/// ⑥b 底部遮挡的两条自愈链（2026-11-20 真机缺陷：列表页底部一行被底栏压住且滚不到）：
///    ① 页面自己的「内嵌滚动区 / 子文档」（顶部有自己的头 ⇒ shrink 判据认不出）底边落进
///       底栏压盖带 ⇒ 让位（padding-bottom 一格，滚动到底内容停在底栏上沿）；
///    ② 滚动根契约探测补跑 + 退路可撤销（首探早于页面样式落定会假绿 / 假红）。
#[test]
fn bottom_occlusion_self_heals() {
    assert!(
        bar().contains("data-web2app-mobile-pad"),
        "内嵌滚动区让位必须有独立标记（data-web2app-mobile-pad）"
    );
    assert!(
        bar().contains("paddedBottom(") && bar().contains("'IFRAME'"),
        "iframe / 子文档也要让位，且保留页面原有 px 内边距（paddedBottom）"
    );
    assert!(
        bar().contains("'padding-bottom'") && bar().contains("pads[m].el"),
        "让位必须走 padding-bottom：滚动到底时内容停在底栏上沿（对子文档减高度无效）"
    );
    assert!(
        bar().contains("installScrollRootRecheck") && bar().contains("setTimeout(recheck"),
        "滚动根契约探测必须补跑（load + 两个定时），首探假绿会让底部内容被压住"
    );
    assert!(
        bar().contains("undoScrollFallback"),
        "探测转绿必须撤销退路 inline !important 样式（假红自愈）"
    );
}

/// ⑦ 前进 / 后退的落点守卫：与桌面端标题栏、Android 系统返回键**同一条规则**
///    —— 没有真实相邻历史、或落点是内置启动页时不动作（移动端绝不退回内置页）。
#[test]
fn back_forward_use_the_same_landing_guard_as_desktop() {
    assert!(
        bar().contains("window.navigation"),
        "前进/后退必须经 Navigation API 判定落点（与 titlebar.js 同规则）"
    );
    assert!(
        bar().contains("isBuiltinUrl"),
        "落点是内置启动页时必须拒绝（移动端不允许退回到内置页面）"
    );
    assert!(
        bar().contains("backBtn.disabled") && bar().contains("fwdBtn.disabled"),
        "无历史可走时按钮必须置灰（否则「点了没反应」没有任何解释）"
    );
}

/// ⑧ 启动页（内置页）不画底栏：那时既没有真实历史、也不该给用户一个假的「首页」。
#[test]
fn builtin_splash_page_renders_no_bar() {
    assert!(
        has_code_line(bar(), "isBuiltinUrl(location.href)"),
        "内置启动页必须早退（不画底栏）：那时没有可前进/后退的真实页面"
    );
}

/// ⑨ 移动端交互约定：别把桌面端的鼠标习惯带进来（没有 hover、触摸目标 ≥ 44px）。
#[test]
fn mobile_interaction_rules_are_present() {
    assert!(
        bar().contains("touch-action:manipulation"),
        "必须 touch-action:manipulation（去掉双击缩放带来的点击延迟）"
    );
    assert!(
        bar().contains("min-height:44px"),
        "触摸目标必须 ≥ 44px（Apple HIG / Material 同为 44–48dp）"
    );
    assert!(
        bar().contains("-webkit-tap-highlight-color:transparent"),
        "必须去掉系统点击高亮块（否则整格按钮按下时闪一个色块）"
    );
    assert!(
        bar().contains("w2m-btn:active"),
        "触摸设备没有可靠的 :hover ⇒ 反馈必须走 :active"
    );
    // 反面：不得再画桌面端那种 hover tooltip（触摸设备上要么不出现、要么粘住不走）
    assert!(
        !bar().contains("data-tip"),
        "底栏不该有 tooltip：触摸设备上 :hover 不可靠，语义交给 aria-label"
    );
}

/// ⑩ `mobile_bar` 的**编译期常量与注入点**语义：必须是移动端专用（桌面目标连这个常量
///    都不存在，而不是「存在但恒 false」），且底栏脚本注入点唯一。
///    「字段在各真源里都存在」已迁到 packaging_field_contract.rs（按字段表数据驱动遍历）。
#[test]
fn mobile_bar_constant_is_mobile_only_and_injected_once() {
    assert!(
        has_code_line(CONFIG_RS, "pub(crate) const MOBILE_BAR: bool"),
        "webview-app/src/config.rs 必须定义 MOBILE_BAR 编译期常量"
    );
    // 它必须是**移动端专用**：桌面目标连这个常量都不存在（而不是「存在但恒 false」）。
    assert!(
        has_code_line(CONFIG_RS, "#[cfg(mobile)]"),
        "config.rs 里 MOBILE_BAR 必须是 #[cfg(mobile)]（桌面端无此代码路径）"
    );
    assert!(
        has_code_line(LIB_RS, "#[cfg(mobile)]") && LIB_RS.contains("crate::config::MOBILE_BAR"),
        "lib.rs 必须在移动端按 MOBILE_BAR 决定是否注入底栏脚本"
    );
    // 桌面端（Windows）不得注入底栏脚本：那条块整体 cfg(mobile)，这里按「注入点只有一个
    // 且带 mobile 标记」间接钉住（分片清单只在 lib.rs 出现一次）。
    assert_eq!(
        LIB_RS
            .matches("include_str!(\"mobilebar/core.js\")")
            .count(),
        1,
        "lib.rs 里底栏脚本的注入点必须唯一（core.js 只应出现一次；多一处就会重复挂载底栏）"
    );
}

/// ⑪ #5 打包字段 `android_permissions` 的**语义**：白名单与 Manifest 映射固定，
///    归一函数（白名单三值、去重保序）与 config.json 真数组还原分支双侧齐全，
///    workflow / ci 门禁都要接线 —— 少一处不是「功能缺失」，而是「配置写了却静默不生效」
///    或 CI 模板断言当场红。字段的**存在性**（双侧解析器 / Electron 运行期 / 两份样例）
///    由 packaging_field_contract.rs 按字段表数据驱动巡检。
#[test]
fn android_permissions_semantics_and_manifest_wiring_are_pinned() {
    // Python 唯一真源：白名单三值、归一函数与 config.json 真数组还原分支。
    for needle in [
        "ANDROID_PERMISSION_CHOICES",
        "def normalize_android_permissions",
        "\"camera\"",
        "\"microphone\"",
        "\"location\"",
        // 写 config.json 时必须把 JSON 文本还原成真数组（Electron 侧按真数组读）。
        "elif key == \"android_permissions\":",
    ] {
        assert!(
            PARSER_PY.contains(needle),
            "解析器（Python）必须包含：{needle}"
        );
    }
    // Node 复刻：白名单常量、归一函数、config.json 还原分支，一处都不能少。
    assert!(
        PARSER_MJS.contains("ANDROID_PERMISSION_CHOICES")
            && PARSER_MAIN_MJS.contains("function normalizeAndroidPermissions"),
        "Node 复刻必须有白名单常量与 normalizeAndroidPermissions"
    );
    assert!(
        PARSER_MAIN_MJS.contains("key === 'android_permissions'"),
        "parser.mjs 的 buildMerged / toConfigJson 必须单独处理 android_permissions"
    );
    // Electron 运行期：形态校验（本壳接受但忽略）。
    assert!(
        ELECTRON_CONFIG.contains("function toAndroidPermissions"),
        "electron config.ts 必须有 android_permissions 形态校验"
    );
    // Manifest 补丁脚本：marker + 三个白名单名到四条 Manifest 权限的固定映射。
    for needle in [
        "web2app-android-permissions",
        "\"camera\": (\"android.permission.CAMERA\",)",
        "\"microphone\": (\"android.permission.RECORD_AUDIO\",)",
        "\"location\": (",
        "android.permission.ACCESS_FINE_LOCATION",
        "android.permission.ACCESS_COARSE_LOCATION",
        "ANDROID_PERMISSIONS",
        "--selftest",
    ] {
        assert!(
            PERMISSIONS_PY.contains(needle),
            "patch-android-permissions.py 必须包含：{needle}"
        );
    }
    // 两个打包 workflow 都要接线；ci.yml 的 selftest 名单要收录；解析器 env 名固定。
    for (name, yml) in [
        ("build-custom.yml", BUILD_CUSTOM_YML),
        ("release.yml", RELEASE_YML),
    ] {
        assert!(
            yml.contains("patch-android-permissions.py"),
            "{name} 的 Android job 必须调用 patch-android-permissions.py"
        );
    }
    // ci.yml 的循环按目录发现 ⇒ 覆盖条件是「脚本自带 --selftest」。
    assert!(
        PERMISSIONS_PY.contains("--selftest"),
        "patch-android-permissions.py 必须自带 --selftest，否则 ci.yml 的目录发现循环会跳过它"
    );
    assert!(
            BUILD_CUSTOM_YML.contains("ANDROID_PERMISSIONS_VAL")
                && RELEASE_YML.contains("ANDROID_PERMISSIONS"),
            "两个 workflow 必须把 ANDROID_PERMISSIONS 传给补丁步（custom 走 GITHUB_ENV/模板，release 走 vars）"
        );
}

/// ⑫ #6 H5 视频全屏补丁（patch-android-fullscreen.py）必须真的实现全屏，而不是保留
///    wry 0.57「进来就 onCustomViewHidden」的空桩；两 workflow + ci 门禁必须接线。
#[test]
fn fullscreen_patch_replaces_wry_stub_and_is_wired() {
    for needle in [
        "web2app-android-fullscreen",
        // 替换的是同一个 ChromeClient 的两个回调（签名漂移即报错，不静默跳过）。
        "override fun onShowCustomView(view: View, callback: CustomViewCallback)",
        "override fun onHideCustomView()",
        "ON_SHOW",
        "ON_HIDE",
        // 全屏承载：custom view 铺满 content、WebView 暂时藏起，退出逐项还原。
        "activity.findViewById<ViewGroup>(android.R.id.content)",
        "content.addView(",
        "FrameLayout.LayoutParams(",
        "content?.removeView(customView)",
        "View.GONE",
        "View.VISIBLE",
        // 横屏 + 沉浸式（API30+ 与低版本两套）。
        "ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE",
        "web2appSetImmersive",
        "WindowInsets.Type.systemBars()",
        "BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE",
        "SYSTEM_UI_FLAG_IMMERSIVE_STICKY",
        // 模板漂移保险：只在确认是空桩时才替换。
        "onCustomViewHidden()",
        "立即取消",
        "RustWebChromeClient.kt",
    ] {
        assert!(
            FULLSCREEN_PY.contains(needle),
            "patch-android-fullscreen.py 必须包含：{needle}"
        );
    }
    for (name, yml) in [
        ("build-custom.yml", BUILD_CUSTOM_YML),
        ("release.yml", RELEASE_YML),
    ] {
        assert!(
            yml.contains("patch-android-fullscreen.py"),
            "{name} 的 Android job 必须调用 patch-android-fullscreen.py"
        );
    }
    // ci.yml 的循环按目录发现 ⇒ 覆盖条件是「脚本自带 --selftest」。
    assert!(
        FULLSCREEN_PY.contains("--selftest"),
        "patch-android-fullscreen.py 必须自带 --selftest，否则 ci.yml 的目录发现循环会跳过它"
    );
}

/// ⑬ #7 网页 target=_blank / window.open 必须在页签模式下被拦截成 open-tab 事件：
///    JS 侧（捕获 click + 包 window.open + HAS_TABS 门控 + 只承接 http(s)）与
///    Rust 侧（OPEN_TAB_EVENT 监听 + create_tab_with_url 落到指定 url）缺一不可。
#[test]
fn blank_targets_open_in_new_mobile_tab() {
    // JS 侧要素。
    for needle in [
        "function installBlankGuard()",
        // 门控 + 尽早安装（赶在页面脚本第一次调用 window.open 之前）。
        "if (HAS_TABS) installBlankGuard()",
        // ① 捕获阶段点击拦截 a[target=_blank]。
        "document.addEventListener('click'",
        "node.tagName === 'A'",
        "_blank",
        "ev.preventDefault()",
        "ev.stopPropagation()",
        // ② window.open 包裹。
        "var origOpen = window.open",
        "window.open = function",
        // 两条路径都发同一个事件、同一 payload 形态。
        "emit('web2app:open-tab', { url:",
        // 只承接 http(s)，外跳留给 schemes 补丁。
        "/^https?:\\/\\//i.test(u)",
    ] {
        assert!(
            bar().contains(needle),
            "底栏脚本 的 _blank 拦截必须包含：{needle}"
        );
    }
    // 非页签模式不得安装（没有承接记录；grep HAS_TABS 门控行而非函数体即可）。
    assert!(
        bar().contains("if (HAS_TABS) installBlankGuard();"),
        "_blank 拦截必须由 HAS_TABS 门控（非页签模式不假装支持多窗口）"
    );
    // Rust 侧：监听已由 event_names_match 钉事件名，这里钉落点处理。
    assert!(
        has_code_line(
            BAR_RS,
            "fn create_tab_with_url(app: &AppHandle, raw_url: &str)"
        ),
        "mobilebar.rs 必须实现 create_tab_with_url（承接 open-tab 的 url 落新页签）"
    );
    assert!(
        BAR_RS.contains("create_tab_with_url(&opn, &url)"),
        "OPEN_TAB_EVENT 的监听必须调 create_tab_with_url"
    );
    // http(s) 白名单：非网页外跳交给 schemes 补丁，不能压成页签记录。
    assert!(
        BAR_RS.contains("url.scheme() != \"http\" && url.scheme() != \"https\""),
        "create_tab_with_url 必须只承接 http(s)（tel/mailto 等交给 scheme 拦截）"
    );
    // open-tab 必须带 {url} payload 解析（它是新建页签的唯一入口）。
    assert!(
        has_code_line(BAR_RS, "fn open_tab_url_of(payload: &str)"),
        "mobilebar.rs 必须从 open-tab payload 解析 url"
    );
    // 地址改写：页面给的地址必须经 `nav::resolve_popup_target` 映射到本壳入口，
    // 不能直接导航。Android 只要 proxy_enabled 就恒走本地代理，直接导航页面里写死的
    // 上游绝对外链 ⇒ 无 Authorization 注入（401）/ 仅代理可达时白屏 / 后续链接一起脱管。
    // 规则本体住 `nav`（跨平台，7 条纯逻辑单测在 nav::tests），这条只钉**移动端接线**：
    // 规则被删掉接线也不会报错，只会在真机上表现为「弹窗页签打不开」。
    assert!(
        has_code_line(BAR_RS, "resolve_popup_target(&nav, Some(raw_url))"),
        "create_tab_with_url 必须用 nav::resolve_popup_target 改写落点（与桌面多页签同一条规则）"
    );
}

/// ⑭ 死代码豁免必须停在「移动端模块」这一层（架构审查 P2-2）。
///    crate 级 `#![allow(dead_code)]` 会在桌面目标上掩盖**所有**真死代码（10-25 起一直
///    是兜底）；本模块只在 `cfg(any(mobile, test))` 下编译，桌面/测试构建里它的实现天然
///    没有调用方 —— 所以豁免只允许出现在 mobilebar.rs，且必须带 `not(mobile)` 条件，
///    这样移动端构建仍能报出真死代码。改回全局豁免时本测试应立刻 FAIL。
#[test]
fn dead_code_allowance_stays_scoped_to_mobile_module() {
    assert!(
        !has_code_line(LIB_RS, "#![allow(dead_code)]"),
        "lib.rs 不得再出现 crate 级 `#![allow(dead_code)]`：它会掩盖桌面目标上的真死代码"
    );
    assert!(
        has_code_line(BAR_RS, "#![cfg_attr(not(mobile), allow(dead_code))]"),
        "mobilebar.rs 必须用条件豁免（桌面/测试构建生效，移动端仍受检查）"
    );
}

/// ⑮ 底栏脚本的分片**由 `lib.rs` 唯一权威派生**（不另抄名单）：按序拼成**同一个 IIFE**，
///    core 开 IIFE、mount 必须最后一片并关掉它。改成分片各自独立 IIFE / 调换顺序时立刻红。
#[test]
fn bar_script_parts_are_derived_and_mount_closes_the_iife() {
    let js = bar();
    assert!(
        js.contains("(function () {") && js.contains("'use strict';"),
        "core 分片必须保留 IIFE 与其开头的严格模式（分片不是各自独立的闭包）"
    );
    assert!(
        js.trim_end().ends_with("})();"),
        "最后一片（mount.js）必须以 `}})();` 关闭 IIFE —— 末尾不是它，说明分片顺序被改错了"
    );
    // 防重标记只允许一处：`mount.js` 里多写一次会覆盖掉已挂载的实例。
    assert_eq!(
        js.matches("window.__web2appMobileBar = {").count(),
        1,
        "`window.__web2appMobileBar` 防重标记必须唯一"
    );
}

/// ⑯ #9 系统返回键**先关浮层再后退**：JS 侧暴露 `closeSheet`、Kotlin 侧调同名方法。
///
/// 缺陷形态（缺任一侧）：页签面板 / 更多面板开着时按返回，面板是 DOM 不是 history 条目，
/// 系统返回看不见它 ⇒ 直接离开当前页，用户以为「面板关不掉、还把手滑了」。两侧都静默，
/// 只有真机能发现，所以钉成跨文件契约。
#[test]
fn back_key_closes_sheet_before_navigating() {
    assert!(
        has_code_line(bar(), "function closeSheetIfOpen()"),
        "底栏脚本必须有 closeSheetIfOpen（有面板时关掉并返回 true）"
    );
    assert!(
        bar().contains("closeSheet: closeSheetIfOpen"),
        "必须把它暴露成 window.__web2appMobileBar.closeSheet（Kotlin 侧按这个名字调）"
    );
    assert!(
        bar().contains("if (!activePanel) return false;"),
        "无面板时必须返回 false，让原生继续走后退逻辑"
    );
    // Kotlin 侧：eval 串里必须出现同名方法，且判定排在 web2appBack 之前。
    assert!(
        BACK_PY.contains("b.closeSheet()"),
        "patch-android-back.py 的 eval 串必须调页面侧 closeSheet（与 core.js 同名）"
    );
    let js_at = BACK_PY
        .find("evaluateJavascript(WEB2APP_CLOSE_SHEET_JS)")
        .unwrap_or_else(|| panic!("返回键补丁必须先问页面浮层状态"));
    let back_at = BACK_PY
        .find("web2appBack(webView)")
        .unwrap_or_else(|| panic!("返回键补丁必须有真正的后退动作"));
    assert!(
        js_at < back_at,
        "浮层判定必须排在后退之前（反了 = 面板开着时按返回直接离开页面）"
    );
    // 返回值类型也是契约：数字 1/0（字符串会被 evaluateJavascript 回调再包一层引号）。
    assert!(
        BACK_PY.contains("?1:0"),
        "eval 串必须返回数字 1/0，不能用布尔/字符串"
    );
}

/// ⑰ #10 无真实历史时「再按一次退出」：一按就 finish 会被当成闪退。
#[test]
fn back_key_requires_double_press_to_exit() {
    assert!(
        BACK_PY.contains("Toast.makeText") && BACK_PY.contains("再按一次退出"),
        "第一次返回必须给可见提示（Toast + 文案），否则用户以为按了没反应"
    );
    assert!(
        BACK_PY.contains("web2appLastBackAt") && BACK_PY.contains("<= 2000L"),
        "双按判定必须有时间戳与 2s 窗口"
    );
    // 成对：时间窗**之后**出现的第一个 finish() 才是退出分支 —— 从窗口位置往后找，
    // 否则会命中模块文档字符串里的 `finish()` 说明（那是注释，不是代码）。
    let win = BACK_PY.find("<= 2000L").unwrap();
    let fin = BACK_PY[win..]
        .find("finish()")
        .map(|i| win + i)
        .unwrap_or_else(|| panic!("双按窗口之后必须有 finish() 退出分支"));
    assert!(win < fin, "finish() 必须被双按窗口包住，不能无条件退出");
}

/// ⑲ #12 软键盘让位：只认 visualViewport 的显著差值，且不破坏底部让位契约。
#[test]
fn keyboard_avoidance_uses_visual_viewport() {
    assert!(
        bar().contains("window.visualViewport"),
        "键盘让位必须读 visualViewport（layout viewport 不变时 fixed bottom:0 会被键盘整个盖住）"
    );
    assert!(
        bar().contains("W2M_KB_MIN_GAP"),
        "必须有最小差值阈值：地址栏自动收起也会改 visualViewport，不能当成键盘"
    );
    assert!(
        bar().contains("data-w2m-keyboard"),
        "抬升状态要可观测（断言与真机排查都靠这个属性）"
    );
    // 让位只改底栏 inline bottom；body 的 margin-bottom 契约必须原样保留。
    assert!(
        bar().contains("margin-bottom:"),
        "键盘让位不得顺手改掉 body 的底部让位契约"
    );
    let fit = bar().find("scheduleChromeFit();").unwrap();
    let kb = bar().find("installKeyboardAvoid();").unwrap();
    assert!(
        fit < kb,
        "键盘让位必须在底栏布局（scheduleChromeFit）之后安装"
    );
}

/// ⑳ #16 宽屏（平板 / 横屏）：底栏与面板限宽居中，避免一排按钮被拉成一米宽。
#[test]
fn wide_screen_bar_is_centered_and_capped() {
    assert!(
        bar().contains("@media (min-width:600px)"),
        "必须有宽屏断点（600px 是平板竖屏的最小常见宽度）"
    );
    assert!(
        bar().contains("max-width:560px"),
        "宽屏必须限宽：否则底栏按钮横跨整块平板，拇指够不到"
    );
    assert!(
        bar().contains("left:50%!important") && bar().contains("transform:translateX(-50%)"),
        "限宽后必须居中（left + transform，不改布局流）"
    );
    assert!(
        bar().contains("#__web2app_mobilebar__ .w2m-sheet{max-width:560px"),
        "面板必须与底栏同步限宽，否则底栏窄、面板满宽，视觉断裂"
    );
}

/// ㉑ #18 明文 HTTP **必须可用**：脚本写 network_security_config 全放行，workflow 不再各写 sed。
///
/// 曾被收紧成「仅 127.0.0.1 / localhost」，结果直连 http 站点被系统拦下
///（`ERR_CLEARTEXT_NOT_PERMITTED`）⇒ 产品侧要求回退为全放行，本用例把这条口径钉住：
/// 配置本体一旦重新出现禁明文，用例即失败。
#[test]
fn cleartext_http_stays_allowed() {
    for needle in [
        "web2app-android-cleartext",
        "network_security_config",
        "cleartextTrafficPermitted=\"true\"",
        "android:usesCleartextTraffic",
        "--selftest",
    ] {
        assert!(
            CLEARTEXT_PY.contains(needle),
            "patch-android-cleartext.py 必须包含：{needle}"
        );
    }
    // 判定只看资源文件本体：脚本注释与自测里的「反例 / 历史说明」不参与，否则一改文案就假红。
    const NSC_OPEN: &str = "<network-security-config>";
    const NSC_CLOSE: &str = "</network-security-config>";
    let open = CLEARTEXT_PY
        .find(NSC_OPEN)
        .expect("脚本必须写出 <network-security-config>");
    let close = CLEARTEXT_PY
        .find(NSC_CLOSE)
        .expect("脚本必须闭合 </network-security-config>");
    let xml = &CLEARTEXT_PY[open..close + NSC_CLOSE.len()];
    assert!(
        xml.contains("cleartextTrafficPermitted=\"true\""),
        "明文必须放行：直连 http 站点不允许被系统拦下"
    );
    let banned = concat!("cleartextTrafficPermitted=", "\"false\"");
    assert!(
        !xml.contains(banned),
        "明文不得被禁：直连 http 站点会被系统拦下（ERR_CLEARTEXT_NOT_PERMITTED）"
    );
    for (name, yml) in [
        ("build-custom.yml", BUILD_CUSTOM_YML),
        ("release.yml", RELEASE_YML),
    ] {
        assert!(
            yml.contains("patch-android-cleartext.py"),
            "{name} 的 Android job 必须调用 cleartext 补丁"
        );
        assert!(
            !yml.contains("sed -i -E 's/android:usesCleartextTraffic"),
            "{name} 不得再用 sed 全局放行明文 HTTP（已改为 network_security_config 白名单）"
        );
    }
}
