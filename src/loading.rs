//! 启动过渡体验：本地启动页 + 注入式加载层。
//! ---------------------------------------------------------------------------
//! 目标：消除「软件启动 → 首个页面渲染出内容」之间的无反馈空白等待。
//!
//! 分两段接力，两段共用 `src/loading.js` 里的同一套视觉与状态机，所以用户看到的是
//! 一条连续的「加载中…」，而不是两次互相独立的转圈：
//!
//!   ① splash 阶段：主窗口创建时即加载 `splash://localhost/`（自定义协议，纯本地、
//!      零网络请求，首帧可见）。此后才在后台线程做耗时工作（域名解析 / 启动本地反向
//!      代理），避免这些操作把"窗口都还没出来"的空白期拉长。
//!   ② overlay 阶段：真实地址就绪后导航过去；`initialization_script` 注入的加载层会在
//!      每个新文档的 document-created 阶段（早于页面脚本与首次绘制）铺满视口，
//!      检测到页面渲染出内容后淡出移除。
//!
//! 容错：
//!   · 慢 / 超时：由 JS 侧计时器自行降级（慢提示 → 超时错误 + 重试）。
//!   · 解析失败 / 代理启动失败：Rust 侧通过 `show_error` 把错误态推给当前页面。
//!   · 重试：splash 阶段跳 `splash://localhost/retry`（重新走一遍完整启动流程），
//!     overlay 阶段直接刷新当前地址。

use std::borrow::Cow;
use tauri::{http::Request, http::Response, AppHandle, Manager, UriSchemeContext};
use url::Url;

/// 自定义协议名。URL 形如 `splash://localhost/`（Tauri 内部映射为 `http://splash.localhost/`）。
pub const SPLASH_SCHEME: &str = "splash";

/// 启动页里「重试」按钮跳转的路径：命中即重新执行一次完整的启动流程。
const RETRY_PATH: &str = "/retry";

const SPLASH_TEMPLATE: &str = include_str!("splash.html");
const LOADING_JS: &str = include_str!("loading.js");

/// 注入到每一次导航目标文档的加载层脚本。
///
/// 通过 `WebviewWindowBuilder::initialization_script` 注册，在 document-created 阶段执行，
/// 早于页面自身脚本与首次绘制 —— 这是它能盖住白屏、又不会让页面"闪一下再被遮住"的关键。
pub const OVERLAY_SCRIPT: &str = LOADING_JS;

/// 启动页地址。
pub fn splash_url() -> Url {
    Url::parse("splash://localhost/").expect("构造 splash 协议 URL 失败")
}

/// 渲染启动页 HTML：把窗口标题与加载层脚本填进模板占位符。
///
/// 先替换标题再插入脚本，避免脚本内容被后续替换误伤。
pub fn splash_html(title: &str) -> String {
    SPLASH_TEMPLATE
        .replace("__WEB2APP_TITLE__", &html_escape(title))
        .replace("/*__WEB2APP_LOADING_JS__*/", LOADING_JS)
}

/// splash 自定义协议的处理器。
///
/// `/retry` 会在返回新页面之前触发一次完整的重新启动（重新解析地址 / 重启本地代理 /
/// 重新导航），这样"重试"才真的能重试，而不是只刷新一张永远停在原地的启动页。
/// 其余路径一律返回启动页 HTML（每次都重新生成，便于 `/retry` 拿到干净状态）。
pub fn handle_splash(
    ctx: UriSchemeContext<'_, tauri::Wry>,
    req: Request<Vec<u8>>,
) -> Response<Cow<'static, [u8]>> {
    if req.uri().path().trim_end_matches('/') == RETRY_PATH {
        crate::bootstrap::bootstrap(ctx.app_handle().clone());
    }

    let html = splash_html(crate::config::WINDOW_TITLE);
    Response::builder()
        .header("Content-Type", "text/html; charset=utf-8")
        // 启动页是即时生成的，禁止缓存，确保"重试"拿到的是干净状态。
        .header("Cache-Control", "no-store")
        .body(html.into_bytes().into())
        .unwrap()
}

/// 把加载层切到失败态（解析真实地址失败 / 本地代理启动失败等启动期错误）。
///
/// 用 `eval` 而不是 IPC：加载层可能被注入到任意第三方页面，走 eval 不依赖任何权限配置。
/// 整个表达式包在 try/catch 里，页面尚未就绪时静默失败即可。
///
/// `label`：目标窗口 label（单窗口为 `"main"`，多页签为 `tab-N`）。
pub fn show_error(app: &AppHandle, label: &str, title: &str, detail: &str) {
    let Some(w) = app.get_webview_window(label) else {
        return;
    };
    let js = format!(
        "try{{window.__web2appLoading&&window.__web2appLoading.fail({t},{d});}}catch(e){{}}",
        t = js_str(title),
        d = js_str(detail)
    );
    let _ = w.eval(js.as_str());
}

/// 通知加载层页面已就绪，可以淡出了（正常路径由 JS 自行判定，这里给 Rust 侧一个兜底入口）。
///
/// `label`：目标窗口 label（单窗口为 `"main"`，多页签为 `tab-N`）。
pub fn hide(app: &AppHandle, label: &str) {
    let Some(w) = app.get_webview_window(label) else {
        return;
    };
    let _ = w.eval("try{window.__web2appLoading&&window.__web2appLoading.hide();}catch(e){}");
}

/// 把字符串转成可安全嵌入 JS 源码的双引号字符串字面量（含两侧引号）。
pub(crate) fn js_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            // 行分隔符在旧引擎里不是合法的字符串字面量内容
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            // 转义 '<' 以防宿主页面把这段 JS 当成 HTML 片段解析时提前闭合
            '<' => out.push_str("\\u003c"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// HTML 文本/属性转义（只用于模板里的标题）。
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splash_html_inlines_script_and_title() {
        let html = splash_html(r"Tom's <App> & \Co");
        assert!(!html.contains("__WEB2APP_TITLE__"), "标题占位符应被替换");
        assert!(
            !html.contains("/*__WEB2APP_LOADING_JS__*/"),
            "脚本占位符应被替换"
        );
        assert!(html.contains("Tom&#39;s &lt;App&gt; &amp; \\Co"));
        assert!(html.contains("__web2appLoadingMode = 'splash'"));
        assert!(html.contains("w2l-ring"), "应内联加载层脚本");
    }

    #[test]
    fn js_str_escapes_quotes_and_angle_brackets() {
        assert_eq!(js_str("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(js_str("<script>"), "\"\\u003cscript>\"");
        assert_eq!(js_str("line\nbreak"), "\"line\\nbreak\"");
    }

    #[test]
    fn overlay_script_is_valid_js_shape() {
        // 注入脚本以 IIFE 形式自执行，且不带任何外部依赖
        let s = OVERLAY_SCRIPT.trim();
        assert!(s.starts_with("/*"), "脚本应以注释开头");
        assert!(s.ends_with("})();"), "脚本应以 IIFE 闭合结束");
    }
}
