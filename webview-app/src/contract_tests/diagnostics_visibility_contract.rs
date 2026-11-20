/// 「用户屏幕上的技术信息由打包参数 `debug` 决定」的静态契约（跨平台 + 跨壳）。
///
/// 背景（2026-11-21）：`debug` 此前只关掉了**日志**（`startup.log` / stdout），屏幕上该显示的
/// 技术细节照样显示 —— 失败卡上的 `地址 / readyState / HTTP / 资源数 / 整条 UA / 脚本错误`、
/// 启动阶段进度、Android 可达性探测结论、`无法打开 http://127.0.0.1:PORT/…（ERR_…，-102）`。
/// 对最终用户这些字既不能操作、又把内部实现（本地代理入口 / WebView 版本 / net error code）
/// 暴露在屏幕上；而 debug=true 的排障包正需要它们。
///
/// 这些断言全部是**静态**的：编译期零警告、clippy 与 TS 检查全绿，只有真实启动失败时才看得出
/// 「诊断又漏给用户了」—— 所以必须钉在源码上。
///
/// ⚠️ 被测真源（跨壳）：
///   · `webview-app/src/loading.js`（两壳共用的加载层）
///   · `webview-app/src/loading.rs`（Tauri 的注入拼装）
///   · `webview-app/src/{lib.rs,tabs/window.rs}`（两处注入点）
///   · `electron-app/src/main/{index.ts,nav-text.ts,navigate.ts}`（Electron 的注入与文案）
const LOADING_JS: &str = include_str!("../loading.js");
const LOADING_RS: &str = include_str!("../loading.rs");
const LIB_RS: &str = include_str!("../lib.rs");
const TABS_WINDOW_RS: &str = include_str!("../tabs/window.rs");
const ELECTRON_INDEX_TS: &str = include_str!("../../../electron-app/src/main/index.ts");
const ELECTRON_NAV_TEXT_TS: &str = include_str!("../../../electron-app/src/main/nav-text.ts");
const ELECTRON_NAVIGATE_TS: &str = include_str!("../../../electron-app/src/main/navigate.ts");
const BOOTSTRAP_RS: &str = include_str!("../bootstrap.rs");
const PROBE_RS: &str = include_str!("../probe.rs");

/// 取一个函数体片段（从签名起 `len` 字符）—— 断言必须贴目标节点，
/// 整文件 `contains` 会被邻近的注释 / 别的函数满足（2026-11-19 已翻车一次）。
fn body_after<'a>(src: &'a str, anchor: &str, len: usize) -> &'a str {
    let start = src
        .find(anchor)
        .unwrap_or_else(|| panic!("找不到锚点：{anchor}"));
    // 按字节截断时必须退回字符边界：源码里大量中文（3 字节/字），直接切片会 panic。
    let mut end = (start + len).min(src.len());
    while !src.is_char_boundary(end) {
        end -= 1;
    }
    &src[start..end]
}

/// 加载层的诊断总闸：缺省关闭，且只认严格 `true`。
///
/// ⚠️ 缺省必须是 false：老壳 / 第三方注入 / 标记没送到时都按「不显示」处理，
///    这样「忘记传标记」的失败模式是「少显示诊断」而不是「把 UA 泄漏给用户」。
#[test]
fn loading_js_defaults_diagnostics_off() {
    assert!(
        LOADING_JS.contains("var DIAG = false;"),
        "loading.js 必须有诊断总闸且缺省关闭（default false）"
    );
    assert!(
        LOADING_JS.contains("DIAG = window.__web2appDiag === true;"),
        "诊断总闸只认严格 true —— 认 truthy 会让字符串 'false' 也打开诊断"
    );
}

/// `pushNote`（阶段进度 / 探测结论）在诊断关闭时**直接丢弃**。
#[test]
fn notes_are_dropped_when_diagnostics_off() {
    let body = body_after(LOADING_JS, "function pushNote(line)", 900);
    assert!(
        body.contains("if (!DIAG) return;"),
        "pushNote 必须在诊断关闭时短路 —— 否则「正在启动本地反向代理」「可达性探测：…内核…」\
         这些打包者视角的进度仍会进卡片"
    );
}

/// 失败卡的现场诊断（`diagText()`）只能在诊断打开时拼接。
#[test]
fn diag_text_is_gated_in_every_failure_card() {
    // ① failCard（壳侧推来的解析 / 代理 / 探测失败）
    let fail_card = body_after(LOADING_JS, "function failCard(title, detail, soft)", 600);
    assert!(
        fail_card.contains("(DIAG ? '\\n' + diagText() : '')"),
        "failCard 必须按 DIAG 决定是否拼 diagText()"
    );

    // ② 40s 兜底卡（splash / overlay 各一条文案，共用同一处拼接）
    let fallback = body_after(LOADING_JS, "IS_SPLASH ? '启动超时' : '加载超时'", 900);
    assert!(
        fallback.contains("(DIAG ? '\\n' + diagText() : '')"),
        "超时兜底卡必须按 DIAG 决定是否拼 diagText()"
    );
    assert!(
        fallback.contains("DIAG && isSplashDoc()"),
        "「仍停在启动页：导航没有提交」属分诊结论，也必须随 DIAG 一起隐藏"
    );

    // ③ overlay 的空白文档硬上限卡（锚点取分支条件：DIAG 三元在文案之前）
    let cap = body_after(LOADING_JS, "if (Date.now() - t0 > CAP_MS)", 1200);
    assert!(
        cap.contains("DIAG") && cap.contains("页面长时间为空白"),
        "空白文档卡必须按 DIAG 切换「反馈给打包者 + 诊断」与普通用户文案"
    );
}

/// Tauri：注入脚本必须带诊断标记，且两处注入点都用它。
#[test]
fn tauri_injects_diag_marker_through_overlay_script() {
    assert!(
        LOADING_RS.contains("pub fn overlay_script() -> String"),
        "loading.rs 必须提供 overlay_script()（带诊断标记的注入入口）"
    );
    let body = body_after(LOADING_RS, "fn diag_prefix()", 700);
    assert!(
        body.contains("window.__web2appDiag = true;")
            && body.contains("window.__web2appDiag = false;"),
        "diag_prefix 必须给出 true / false 两种字面量"
    );
    assert!(
        body.contains("if crate::debug_log::enabled()"),
        "诊断标记必须走 debug_log::enabled()（编译期 DEBUG **或** 运行时 --debug / \
         WEB2APP_DEBUG=1）；只看 config::DEBUG 会让交付包失去唯一逃生口，且与 Electron 侧分叉"
    );

    // 启动页与两处初始化脚本注入都必须走 overlay_script()
    // 片段要跨过 splash_html 上方的整段文档注释（约 1.2KB 中文），故给足窗口。
    let splash = body_after(
        LOADING_RS,
        "pub fn splash_html(title: &str) -> String",
        2000,
    );
    assert!(
        splash.contains("let loading_js = overlay_script();"),
        "splash_html 必须用 overlay_script() —— 否则「启动页上的失败卡」永远不显示诊断"
    );
    assert!(
        LIB_RS.contains(".initialization_script(loading::overlay_script())"),
        "lib.rs 主窗口必须注入 loading::overlay_script()"
    );
    assert!(
        TABS_WINDOW_RS.contains(".initialization_script(crate::loading::overlay_script())"),
        "tabs/window.rs 必须注入 crate::loading::overlay_script()"
    );

    // 反例：直接注入源码常量 = 诊断永远关闭（debug 包也看不到）
    for (name, src) in [("lib.rs", LIB_RS), ("tabs/window.rs", TABS_WINDOW_RS)] {
        assert!(
            !src.contains(".initialization_script(loading::OVERLAY_SCRIPT)")
                && !src.contains(".initialization_script(crate::loading::OVERLAY_SCRIPT)"),
            "{name} 不得直接注入 OVERLAY_SCRIPT：它不带诊断标记，debug 包也会退化成「永不显示」"
        );
    }
}

/// Electron：`readInject('loading.js')` 必须前置同一枚标记。
#[test]
fn electron_prepends_diag_marker_on_loading_js() {
    assert!(
        ELECTRON_INDEX_TS
            .contains("const DIAG_MARKER = `window.__web2appDiag = ${DEBUG ? 'true' : 'false'};"),
        "index.ts 必须由 DEBUG 生成诊断标记"
    );
    let body = body_after(ELECTRON_INDEX_TS, "function readInject(name: string)", 700);
    assert!(
        body.contains("name === 'loading.js'") && body.contains("DIAG_MARKER + out"),
        "readInject 必须只给 loading.js 前置标记（titlebar.js 不需要，也别污染其它资源）"
    );
}

/// Electron：Chromium 自身的 stderr 也要随 debug 静音。
#[test]
fn electron_silences_chromium_logs_when_debug_off() {
    assert!(
        ELECTRON_INDEX_TS.contains("if (!DEBUG) {"),
        "Chromium 日志静音必须挂在 !DEBUG 上"
    );
    assert!(
        ELECTRON_INDEX_TS.contains("appendSwitch('log-level', '3')")
            && ELECTRON_INDEX_TS.contains("appendSwitch('disable-logging')"),
        "必须同时使用 log-level=3 与 disable-logging（只关一项仍会漏 GPU / 证书告警）"
    );
}

/// Electron：导航失败文案的 URL + net error code 受 debug 门控。
#[test]
fn electron_nav_failure_text_hides_url_when_debug_off() {
    let body = body_after(ELECTRON_NAV_TEXT_TS, "export function navFailText(", 500);
    assert!(
        body.contains("if (debug) return `无法打开 ${url}（${desc}，${code}）`"),
        "navFailText 的完整文案必须在 debug 分支里（默认包不得带 URL / 错误码）"
    );
    assert!(
        body.contains("页面无法打开，请检查网络连接后重试。"),
        "navFailText 的非 debug 分支必须给用户可执行的文案"
    );

    // 配置类 / sidecar 类失败同样分级（原始异常与地址只在 debug 包出现）
    assert!(
        ELECTRON_NAVIGATE_TS.contains("function parseAppUrl(raw: string, debug = false)"),
        "parseAppUrl 必须接受 debug 并按其分级"
    );
    assert!(
        ELECTRON_NAVIGATE_TS
            .contains("export function sidecarFailureText(e: unknown, debug = false)"),
        "sidecarFailureText 必须接受 debug 并按其分级"
    );
    assert!(
        ELECTRON_NAVIGATE_TS.contains("sidecarFailureText(e, ctx.debug)"),
        "resolveTarget 必须把 ctx.debug 传下去（否则分级形同虚设）"
    );
}

/// Tauri：启动期错误串的分级判定也必须认运行时逃生口（`--debug` / `WEB2APP_DEBUG=1`）。
///
/// ⚠️ 只看 `config::DEBUG`（编译期常量）的后果：交付包双击报「无法连接服务器」，用户按
/// 文档加 `--debug` 重启后**日志有了、屏幕还是那句通用文案** —— 而屏幕上的失败卡恰恰是
/// 手机上唯一能取到的现场（没有控制台、没有 adb）。
#[test]
fn tauri_error_texts_use_runtime_diagnostics_switch() {
    assert!(
        !BOOTSTRAP_RS.contains("crate::config::DEBUG"),
        "bootstrap.rs 的文案分级必须走 debug_log::enabled()，不得直接用编译期 config::DEBUG"
    );
    assert!(
        BOOTSTRAP_RS.contains("crate::debug_log::enabled()"),
        "bootstrap.rs 应至少有若干处 debug_log::enabled() 分级（原始错误串只在诊断开启时上屏）"
    );
}

/// 可达性探测的失败详情必须在 Rust 侧分好级。
///
/// 它走 `fail(t,d,soft)` 直达卡片，**不经过** `pushNote` ⇒ `loading.js` 的 `DIAG` 总闸管不到。
/// 不分级就会出现「探测成功的结论被隐藏、探测失败的主机名 + DNS/TLS 术语反而显示」的分叉。
#[test]
fn probe_failure_detail_is_gated_before_reaching_overlay() {
    assert!(
        PROBE_RS.contains("probe_failure_detail(detail, crate::debug_log::enabled())"),
        "探测失败详情必须按 debug_log::enabled() 分级后再推给加载层"
    );
    let body = body_after(PROBE_RS, "fn probe_failure_detail(", 400);
    assert!(
        body.contains("PROBE_FAIL_USER_DETAIL"),
        "关闭诊断时必须回落到统一的用户文案常量"
    );
    assert!(
        PROBE_RS.contains("const PROBE_FAIL_USER_DETAIL: &str"),
        "用户文案必须是常量（便于断言它不含主机名 / 分类术语）"
    );
}
