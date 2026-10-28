/// WebView2 网页密码自动保存开关的静态契约（Windows）。
///
/// 背景：WebView2 的 `IsPasswordAutosaveEnabled` **默认 false** —— 不弹「保存/更新密码？」
/// 气泡、不保存新密码（一般表单自动填充是另一个独立开关、默认 true），与 Electron 壳的
/// Chromium 默认开启不对齐。`webview_hooks.rs` 必须在每个 CoreWebView2 的钩子注册阶段把它
/// 显式置 true。COM 调用无法在单测里执行，这里用源码断言把「开关确实接上、且置的是 true」
/// 钉死，防止误删或误改成 false。
/// 被测真源：Windows WebView2 钩子注册代码本体。
const HOOKS_RS: &str = include_str!("../webview_hooks.rs");

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
