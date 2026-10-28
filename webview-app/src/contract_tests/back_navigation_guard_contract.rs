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
/// 自绘标题栏注入脚本（两个壳单一真源）。
const TITLEBAR_JS: &str = include_str!("../titlebar.js");
/// history_nav 命令实现（守卫脚本常量与调用点）。
const LIB_RS: &str = include_str!("../lib.rs");
/// Android MainActivity 补丁脚本（CI 在 android init 之后调用）。
const PATCH_PY: &str = include_str!("../../../.github/scripts/patch-android-back.py");
/// 两个会打出 Android 包的 workflow，必须接上补丁步骤。
const BUILD_CUSTOM_YML: &str = include_str!("../../../.github/workflows/build-custom.yml");
const RELEASE_YML: &str = include_str!("../../../.github/workflows/release.yml");

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
