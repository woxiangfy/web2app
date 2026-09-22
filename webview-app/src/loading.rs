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
/// 自绘标题栏脚本（仅单窗口模式有意义）。启动页需要内联它，原因见 `splash_html`。
const TITLEBAR_JS: &str = include_str!("titlebar.js");

/// 启动页模板里的三个占位符。
///
/// ⚠️ 默认 `&str` 字面量写在这里是**故意**的：它是唯一的真源清单，而模板里刻意不写它们的
/// 字面量（写出来会让全局替换把近似 50KB 脚本正文灌进 HTML 注释）。见 `splash.html` 注释。
pub const TOKEN_TITLE: &str = "__WEB2APP_TITLE__";
pub const TOKEN_TITLEBAR_JS: &str = "__WEB2APP_TITLEBAR_JS__";
pub const TOKEN_LOADING_JS: &str = "__WEB2APP_LOADING_JS__";

/// 注入到每一次导航目标文档的加载层脚本。
///
/// 通过 `WebviewWindowBuilder::initialization_script` 注册，在 document-created 阶段执行，
/// 早于页面自身脚本与首次绘制 —— 这是它能盖住白屏、又不会让页面"闪一下再被遮住"的关键。
pub const OVERLAY_SCRIPT: &str = LOADING_JS;

/// 启动页地址。
pub fn splash_url() -> Url {
    Url::parse("splash://localhost/").expect("构造 splash 协议 URL 失败")
}

/// 渲染启动页 HTML：把窗口标题、加载层脚本、自绘标题栏脚本填进模板占位符。
///
/// 先替换标题再插入脚本，避免标题内容被后续替换误伤（脚本里不含任何占位符字面量，
/// 由 `injected_scripts_contain_no_placeholder_tokens` 测试盯着）。
///
/// ⚠️ 模板里两个 `<script>` **直接**放占位符本体，不能包 `/* … */`：
/// 包装写法只有在「脚本第一个 `*/` 之前没有第二个闭合符」时才碰巧成立，而两个脚本里
/// 各有十几处 `*/`（头注释一改成行注释就会炸，表现为整条顶部栏消失）。
///
/// ⚠️ 标题栏脚本内联进启动页的真实理由：`initialization_script` 是 WebView 建好**之后**
/// 才注册的，内联能让「窗口一出现就有顶部栏」，不依赖注入时机；真实页面上两条路都会走，
/// 靠 `titlebar.js` 的 `__web2appTitlebar` 防重。（旧注释声称「首个文档拿不到
/// initialization_script」——该说法已被 `examples/probe_new_window.rs` 实测推翻，别再引用。）
pub fn splash_html(title: &str) -> String {
    SPLASH_TEMPLATE
        .replace(TOKEN_TITLE, &html_escape(title))
        .replace(TOKEN_TITLEBAR_JS, TITLEBAR_JS)
        .replace(TOKEN_LOADING_JS, LOADING_JS)
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

    let html = splash_html(crate::config::TITLE);
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
        assert!(!html.contains(TOKEN_TITLE), "标题占位符应被替换");
        assert!(!html.contains(TOKEN_LOADING_JS), "脚本占位符应被替换");
        assert!(!html.contains(TOKEN_TITLEBAR_JS), "标题栏占位符也应被替换");
        assert!(html.contains("Tom&#39;s &lt;App&gt; &amp; \\Co"));
        assert!(html.contains("__web2appLoadingMode = 'splash'"));
        assert!(html.contains("w2l-ring"), "应内联加载层脚本");
        assert!(
            html.contains("__web2app_titlebar__"),
            "启动页应内联自绘标题栏（首个文档拿不到 initialization_script）"
        );
    }

    /// 启动页模板里，占位符**只能**出现在预期的两个 `<script>` 与 `<title>` 里。
    ///
    /// 背景（2026-09-16 修的 bug，两壳共有）：模板头部注释里原本也写了这三个占位符名，
    /// 于是全局替换把约 50KB 脚本正文灌进了 HTML 注释 —— 注释里一旦出现两个连续连字符
    /// 就会提前闭合，脚本正文随之变成页面正文。
    #[test]
    fn splash_template_has_no_token_in_comments() {
        let comment = SPLASH_TEMPLATE
            .split_once("-->")
            .map(|(head, _)| head)
            .unwrap_or("");
        for tok in [TOKEN_TITLE, TOKEN_TITLEBAR_JS, TOKEN_LOADING_JS] {
            // 整份模板里每个占位符恰好出现一次（就是它该出现的那处）
            assert_eq!(
                SPLASH_TEMPLATE.matches(tok).count(),
                1,
                "占位符 {tok} 在模板里应恰好出现一次"
            );
            assert!(
                !comment.contains(tok),
                "占位符 {tok} 不该出现在 HTML 注释里（会把脚本正文灌进注释）"
            );
        }
    }

    /// 两个 `<script>` 里必须是**裸**占位符，不能包在 `/* … */` 里。
    ///
    /// 包装写法只有在「脚本第一个 `*/` 之前没有第二个闭合符」时才碰巧成立：旧版依赖的是
    /// titlebar.js 第 1 行是 `/*`、第 70 行才是它自己头注释的 `*/`，外层注释被那句顶掉后
    /// 正文照常解析 —— 纯属巧合。头注释一改成行注释（或脚本被压缩）立刻变成语法错误。
    #[test]
    fn splash_script_tokens_are_not_comment_wrapped() {
        assert!(SPLASH_TEMPLATE.contains("<script>__WEB2APP_TITLEBAR_JS__</script>"));
        assert!(SPLASH_TEMPLATE.contains("<script>__WEB2APP_LOADING_JS__</script>"));
        assert!(!SPLASH_TEMPLATE.contains("/*__WEB2APP_"));
    }

    /// 注入的脚本里不能含任何占位符字面量（否则会级联替换/自替换），
    /// 也不能含 `</script`（内联进 `<script>` 时会提前闭合标签）。
    #[test]
    fn injected_scripts_contain_no_placeholder_tokens() {
        for (name, js) in [("titlebar.js", TITLEBAR_JS), ("loading.js", LOADING_JS)] {
            for tok in [TOKEN_TITLE, TOKEN_TITLEBAR_JS, TOKEN_LOADING_JS] {
                assert!(!js.contains(tok), "{name} 不应包含占位符字面量 {tok}");
            }
            let lower = js.to_ascii_lowercase();
            assert!(
                !lower.contains("</script"),
                "{name} 含 </script，内联进 <script> 会提前闭合标签"
            );
        }
    }

    /// 注入脚本运行在「文档尚未解析」的时刻，此时 `document.documentElement` / `head` /
    /// `body` 都可能为 null。历史上两个脚本直接 appendChild → 静默抛错 → 顶部栏与加载层
    /// 双双不出现。这里锁死「必须等 DOM 宿主就绪」这条约束，避免以后被改回去。
    #[test]
    fn injected_scripts_wait_for_dom_host() {
        for (name, js) in [("loading.js", OVERLAY_SCRIPT), ("titlebar.js", TITLEBAR_JS)] {
            assert!(js.contains("whenHostReady"), "{name} 必须等 DOM 宿主就绪");
            assert!(
                js.contains("MutationObserver"),
                "{name} 应使用 MutationObserver 尽早挂载（隐藏窗口下定时器会被节流）"
            );
        }
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
