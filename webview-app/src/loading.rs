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
//!   · 重试：splash 阶段跳**相对路径** `/retry`（重新走一遍完整启动流程），
//!     overlay 阶段直接刷新当前地址。
//!     ⚠️ 必须是相对路径：页面里的协议已被 wry 改写成 `http://splash.localhost/`，
//!     绝对 URL `splash://localhost/retry` 在内核里是不认识的 scheme，导航会直接失败。

use std::borrow::Cow;
use tauri::{http::Request, http::Response, AppHandle, Manager, UriSchemeContext};
use url::Url;

/// 自定义协议名。URL 形如 `splash://localhost/`（Tauri 内部映射为 `http://splash.localhost/`）。
pub const SPLASH_SCHEME: &str = "splash";

/// 启动页里「重试」按钮跳转的路径：命中即重新执行一次完整的启动流程。
const RETRY_PATH: &str = "/retry";

const SPLASH_TEMPLATE: &str = include_str!("splash.html");
const LOADING_JS: &str = include_str!("loading.js");
/// 自绘标题栏脚本（**Windows 专属**，仅单窗口模式有意义）。启动页需要内联它，原因见 `splash_html`。
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
///
/// ⚠️⚠️ 注册方**必须**在本脚本【之前】再注册一条模式标记脚本（初始化脚本按注册顺序执行），
/// 否则启动页会被当成目标页跑（overlay 语义），命中 15s 空白文档硬上限：
/// ```text
/// .initialization_script("window.__web2appLoadingMode = (location.hostname === \
///     'splash.localhost' || location.protocol === 'splash:') ? 'splash' : 'overlay';")
/// .initialization_script(loading::OVERLAY_SCRIPT)
/// ```
/// 单窗口见 `lib.rs::build_main_window`（实为 `run()` 内的闭包），多页签见
/// `tabs.rs::create_tab_window`（那里钉的是 `tauri.localhost`）。
/// 静态守卫：`contract_tests.rs::loading_mode_contract`。
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
    // 自绘标题栏（顶部通栏）是 **Windows 专属**：Android 有系统标题栏，再内联一条自绘通栏就会
    // 叠成「双标题栏」，且加载层会依 `__web2appChromeH` 在顶部空出 38px。非 Windows 一律替换为
    // 空串 —— 占位符仍照常消费（模板与 `splash_template_has_no_token_in_comments` 都按「恰好出现
    // 一次」约束，不能直接删掉占位符）。
    let titlebar_js = if cfg!(windows) { TITLEBAR_JS } else { "" };
    SPLASH_TEMPLATE
        .replace(TOKEN_TITLE, &html_escape(title))
        .replace(TOKEN_TITLEBAR_JS, titlebar_js)
        .replace(TOKEN_LOADING_JS, LOADING_JS)
}

/// splash 自定义协议的处理器。
///
/// `/retry` 会在返回新页面之前触发一次完整的重新启动（重新解析地址 / 重启本地代理 /
/// 重新导航），这样"重试"才真的能重试，而不是只刷新一张永远停在原地的启动页。
/// 其余路径一律返回启动页 HTML（每次都重新生成，便于 `/retry` 拿到干净状态）。
///
/// ⚠️ 重试目标取**发起请求的 webview label**：单窗口是 `"main"`（与历史硬编码一致）；
/// 参数化后同一个处理器在多页签窗口里被命中时也能把 bootstrap 导航回正确的 tab。
pub fn handle_splash(
    ctx: UriSchemeContext<'_, tauri::Wry>,
    req: Request<Vec<u8>>,
) -> Response<Cow<'static, [u8]>> {
    if req.uri().path().trim_end_matches('/') == RETRY_PATH {
        let app = ctx.app_handle().clone();
        let label = ctx.webview_label().to_string();
        crate::nav::bootstrap_for_label(app, label);
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
/// ⚠️⚠️ 注意「页面尚未就绪」这条：加载层在**挂载完成前**装的是**缓冲桩**（`loading.js` 的
/// `pending`），挂载后回放 —— 所以启动期过早推来的错误**不会**再被静默吃掉。直连配置的启动
/// 流程只需毫秒级（无域名解析、无本地代理），失败推送必然落在挂载之前，这条通路是 Tauri 侧
/// 唯一的错误出口；旧实现挂的是纯 no-op 桩，真机上表现为「只有 40s 兜底卡、看不到真因」。
///
/// `label`：目标窗口 label（单窗口为 `"main"`，多页签为 `tab-N`）。
pub fn show_error(app: &AppHandle, label: &str, title: &str, detail: &str) {
    let call = format!(
        "window.__web2appLoading.fail({t},{d})",
        t = js_str(title),
        d = js_str(detail)
    );
    eval_on_loading(app, label, &call);
}

/// 往当前卡片追加一行补充信息（启动各阶段进度 / 可达性探测结论）。
///
/// 与 [`show_error`] 同一条通路，但**不改变卡片状态**：这些行会被失败卡一并渲染
/// （`loading.js::pushNote` 的 notes 会拼进 `toFail` 的详情），是 40s 兜底卡的唯一现场。
///
/// 为什么要单独一条 API 而不是复用 `fail`：`fail` 会把卡片切成失败态并覆盖详情，
/// 而「导航已发起」「服务器已连通」这类信息是**正向**进度，不该被当成失败。
pub fn show_note(app: &AppHandle, label: &str, line: &str) {
    let call = format!("window.__web2appLoading.note({l})", l = js_str(line));
    eval_on_loading(app, label, &call);
}

/// 在加载层上 eval 一句调用（统一包 try/catch，并容忍窗口已关闭）。
fn eval_on_loading(app: &AppHandle, label: &str, call: &str) {
    let Some(w) = app.get_webview_window(label) else {
        return;
    };
    let js = format!("try{{window.__web2appLoading&&{call};}}catch(e){{}}");
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
        // 自绘标题栏（顶部通栏）是 Windows 专属：Windows 内联；非 Windows（Android）替换为空串，
        // 避免与系统标题栏叠成双通栏（`__web2app_titlebar__` 是 titlebar.js 里的 DOM id / 选择器）。
        if cfg!(windows) {
            assert!(
                html.contains("__web2app_titlebar__"),
                "Windows 启动页应内联自绘标题栏（让窗口一出现就有顶部栏）"
            );
        } else {
            assert!(
                !html.contains("__web2app_titlebar__"),
                "非 Windows（Android）启动页不应内联自绘标题栏"
            );
        }
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

    /// overlay 空白文档超时：必须进入失败卡，而不是无条件淡出露出空白错误文档。
    ///
    /// 背景（Android 真机问题）：WebView 网络失败停在空白错误文档，旧实现在 CAP_MS
    /// 到点时无条件 `done()`，淡出后只剩纯白，30s 的失败卡又被 `state==='done'` 吞掉。
    #[test]
    fn overlay_empty_body_cap_shows_failure_card_not_blank() {
        let js = OVERLAY_SCRIPT;
        assert!(
            !js.contains("> CAP_MS) return done()"),
            "空白文档到点不能再无条件 done()，否则 Android 网络失败时淡出露出纯白页"
        );
        // 新实现：CAP 分支调用 toFail 给出失败态（含重试按钮）。
        assert!(
            js.contains("页面长时间为空白"),
            "CAP 空白文档应给出可识别的失败文案"
        );
    }

    /// Rust 侧可达性探测以 soft 方式调用 fail：页面已有正文时探测假阴性必须让位。
    #[test]
    fn overlay_fail_api_supports_soft_mode() {
        let js = OVERLAY_SCRIPT;
        assert!(
            js.contains("fail: function (t, d, soft)"),
            "fail API 必须支持第三个 soft 参数"
        );
        assert!(
            js.contains("if (!bodyEmpty()) return"),
            "soft 模式下页面已有正文时应忽略探测失败，避免盖住真实页面"
        );
    }

    /// 失败卡必须携带「现场诊断」：用户凭一张截图即可区分故障层级，无需 adb/logcat。
    ///
    /// 真机问题（浏览器能开、壳内白屏）排查中确认：仅靠笼统的「请检查网络」文案会把
    /// 排查方向带偏。诊断至少要能回答三件事：①当前文档地址（是否仍停在 splash）；
    /// ②导航的 HTTP 状态 / 资源数；③WebView 真实 UA（系统内置 WebView 过旧是
    /// 「浏览器正常、壳内 TLS/渲染失败」的高发根因）。
    #[test]
    fn overlay_failure_card_carries_on_screen_diagnostics() {
        let js = OVERLAY_SCRIPT;
        // 地址 + readyState/元素数
        // ⚠️ 地址必须**经 redact()**：本地代理入口形如
        // `http://127.0.0.1:<port>/auth.html?w2a_key=<门禁密钥>`，而这张卡的用途就是截图外发。
        //（脱敏本身另有专门守卫 diag_address_is_redacted_before_display，这里只钉「地址行存在」。）
        assert!(
            js.contains("'地址: ' + redact(location.href)"),
            "诊断应包含当前文档地址，且必须经 redact 抹掉 query 里的凭据"
        );
        // 导航时序：HTTP 状态（Chromium 123+ 才有 responseStatus，老 WebView 显示 ?）
        assert!(
            js.contains("getEntriesByType('navigation')"),
            "诊断应读取 PerformanceNavigationTiming"
        );
        assert!(js.contains("responseStatus"), "诊断应包含导航 HTTP 状态");
        // WebView 真实版本
        assert!(
            js.contains("UA: ' + navigator.userAgent"),
            "诊断应包含 navigator.userAgent（识别系统 WebView 版本）"
        );
        // 多行文本必须能换行显示，否则诊断挤成一行不可读
        assert!(
            js.contains("white-space:pre-wrap"),
            "失败卡详情样式必须保留换行"
        );
        // CAP 分支与壳侧 fail API 两条路径都要附上诊断
        assert!(js.contains("diagText()"), "失败卡文案应拼接 diagText()");
    }

    /// 壳侧调用在加载层**挂载前**到达时不得被静默丢弃。
    ///
    /// 背景（2026-10-09 真机取证）：挂载前挂的是纯 no-op 桩（`fail: function () {}`），而
    /// **直连配置**的启动流程只需毫秒级就完成（无域名解析、无本地代理）⇒ 它的失败推送必然
    /// 落在挂载之前 ⇒ 被静默吃掉，用户只看到 40s 的「启动超时」兜底卡。现在改为缓冲 + 回放。
    #[test]
    fn loading_layer_buffers_shell_calls_before_mount() {
        let js = OVERLAY_SCRIPT;
        assert!(js.contains("var pending = []"), "必须有挂载前的缓冲队列");
        assert!(
            js.contains("function drainPending()"),
            "挂载后必须回放缓冲调用"
        );
        assert!(
            js.contains("drainPending();"),
            "安装完成后必须调用 drainPending()"
        );
        assert!(
            js.contains("if (c[0] === 'fail') failCard(c[1], c[2], c[3])"),
            "回放必须真的把失败态推上去，不能只回放慢提示"
        );
        // 用**正向**断言而不是「不得出现某个字符串」：空实现（旧桩）会让它直接失败，而反向断言
        // 容易被文件里任何一句说明注释里的字面量干扰（本项目在守卫上已踩过两次这个坑）。
        assert!(
            js.contains("fail: function (t, d, soft) { pending.push(['fail', t, d, soft]); }"),
            "挂载前的桩必须把调用**缓冲**进 pending —— 退回空实现会静默吞掉壳侧的错误"
        );
    }

    /// 壳侧补充信息（阶段进度 / 可达性探测结论）不得被兜底卡覆盖。
    ///
    /// 兜底卡 40s 才渲染，而探测结论常在几秒内到达；若只写进 detail，会被后来的 `toFail`
    /// 整体覆盖 ⇒ 卡片上只剩通用文案。故必须单独存进 `notes` 并在 `toFail` 里拼出来。
    #[test]
    fn shell_notes_survive_fallback_card() {
        let js = OVERLAY_SCRIPT;
        assert!(js.contains("var notes = []"), "必须有 notes 容器");
        // ⚠️ 必须**前置**（`d = notes…` 而不是 `d += notes…`）：这几行是「走到哪一步」的分诊
        // 线索（尤其「已发起导航」——区分「根本没导航」与「导航了但没提交」的唯一线索），而
        // diagText 的地址 / UA 属技术细节。拼在末尾时它们会随长诊断一起沉到滚动区最底部。
        assert!(
            js.contains(r"if (notes.length) d = notes.join('\n') + '\n' + d"),
            "toFail 必须把 notes **前置**拼进详情，否则分诊线索会沉到长诊断底下"
        );
        assert!(
            js.contains("note: function (l) { pushNote(l); }"),
            "对外 API 必须暴露 note（壳侧阶段进度 / 探测结论走它）"
        );
        assert!(
            js.contains("pushNote('【'"),
            "已有卡片时，后到的失败结论应追加而不是丢弃"
        );
    }

    /// 启动页 40s 兜底卡必须带现场诊断 + 「仍停在启动页」的判读提示。
    ///
    /// 它以前是**唯一不带诊断**的失败卡（CAP 卡与 fail API 卡都带 `diagText()`），真机上就
    /// 只能给出一句「应用初始化耗时过长」——无从判断卡在哪一步。
    #[test]
    fn splash_fallback_card_carries_diagnostics() {
        let js = OVERLAY_SCRIPT;
        let i = js.find("'启动超时'").expect("splash 兜底卡文案应存在");
        let seg = &js[i..(i + 420).min(js.len())];
        assert!(seg.contains("diagText()"), "启动超时卡必须带现场诊断");
        assert!(
            seg.contains("仍停在启动页"),
            "地址仍是 splash 时应给出「导航没有提交」的判读提示"
        );
    }

    /// 失败卡必须**可滚动**，否则长诊断在窄屏上既看不见、也滚不到。
    ///
    /// 背景（2026-10-09，第三次踩到「拼进去了但用户看不见」）：失败卡内容含四条阶段进度
    /// ＋ 地址 / 状态 / HTTP / 整条 UA / 脚本错误，窄屏手机上总高度轻易超过视口；而本层是
    /// `display:flex; justify-content:center` 且启动页 `html,body{overflow:hidden}` —— 内容
    /// 一超高，**顶部（「地址」行所在处）就被推出屏幕上方，且滚不回来**。
    /// 本守卫钉住「失败态顶部对齐 + 整层可滚动 + 放开文本选择」这组修复不被回退。
    #[test]
    fn failed_card_must_be_scrollable_and_selectable() {
        let js = OVERLAY_SCRIPT;
        assert!(
            js.contains("justify-content:flex-start!important;overflow-y:auto!important;"),
            "失败态必须顶部对齐且整层可滚动 —— 否则长诊断会被推出屏幕且滚不回来"
        );
        assert!(
            js.contains("#__web2app_loading__.w2l-failed{"),
            "上述布局必须只挂在失败态选择器上（写到常态会让加载中的居中布局变形）"
        );
        assert!(
            js.contains("-webkit-user-select:text!important;user-select:text!important;}"),
            "失败态必须放开文本选择 —— 卡片用途就是「把信息反馈给打包者」，选不中等于没给"
        );
    }

    /// 诊断卡上的地址必须**先脱敏**再显示。
    ///
    /// 背景：启用本地反向代理时 WebView 入口是
    /// `http://127.0.0.1:<port>/auth.html?w2a_key=<门禁密钥>`，而这张卡的用途就是
    /// **截图外发给打包者** —— 不脱敏等于把门禁密钥随截图泄漏出去。
    #[test]
    fn diag_address_is_redacted_before_display() {
        let js = OVERLAY_SCRIPT;
        assert!(
            js.contains("'地址: ' + redact(location.href)"),
            "诊断行的地址必须经 redact() 处理（直接拼 location.href 会把 w2a_key 显示出来）"
        );
        assert!(
            js.contains("function redact(u)"),
            "redact 实现必须存在，不得被内联成空操作"
        );
        assert!(
            js.contains("w2a_key|key|token|access_token"),
            "redact 至少要覆盖 w2a_key —— 它是本壳唯一会出现在 URL 里的凭据"
        );
    }

    /// 启动页「重试」必须走**相对路径** + 改写后的 hostname 判定。
    ///
    /// 背景（2026-10-09 排查 Android 白色启动页时定位的第二条必然路径）：wry 的
    /// `custom_protocol_workaround` 在 Windows 与 Android 上把 `splash://localhost/` 改写成
    /// `http://splash.localhost/`（`use_https_scheme` 默认 false），页面里 `location.protocol`
    /// 是 `http:` ⇒ 旧写法 `location.protocol === 'splash:'` **恒不成立**，重试落到
    /// `location.reload()`（只重载 `/`）；而 `/retry` 才是唯一触发 bootstrap 的路径
    /// （`handle_splash` / Electron `protocols.ts` 都按 pathname 判）⇒ 「停在启动页时点重试
    /// 永远起不来」。Windows 同样中招；Electron 侧 scheme 未被改写才一直看似正常。
    #[test]
    fn retry_uses_relative_path_and_rewritten_hostname() {
        let js = OVERLAY_SCRIPT;
        assert!(
            js.contains("var RETRY_URL = '/retry'"),
            "重试落点必须是相对路径 '/retry'（协议被改写后绝对 URL 导航必失败）"
        );
        assert!(
            !js.contains("splash://localhost/retry"),
            "不得再出现绝对 URL splash://localhost/retry —— 内核不认识该 scheme"
        );
        assert!(
            js.contains("location.hostname === 'splash.localhost'"),
            "启动页判定必须包含改写后的 hostname（http://splash.localhost/）"
        );
        assert!(
            js.contains("function isSplashDoc()") && js.contains("if (IS_SPLASH && isSplashDoc())"),
            "onRetry 必须走 isSplashDoc()，不得退回 `location.protocol === 'splash:'` 单一判据"
        );
    }
}
