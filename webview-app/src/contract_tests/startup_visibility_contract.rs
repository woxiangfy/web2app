/// 启动期「壳侧信息必须真的落到卡片上」的静态契约（跨平台）。
///
/// 背景（2026-10-09 真机取证第二轮）：真机只拿到一张「启动超时」兜底卡，卡上**零现场信息**。
/// 追下来是两条独立的信息黑洞，且都不报错、clippy 与 CI 全绿：
///   ① `loading.js` 在挂载完成前挂的是**纯 no-op 桩** ⇒ 而直连配置的启动只需毫秒级完成
///      （无域名解析、无本地代理）⇒ 它的失败推送**必然**落在挂载之前 ⇒ 被静默吃掉；
///   ② 壳侧根本没有「阶段进度」这条通路 ⇒ 40s 兜底卡出现时，没人知道卡在解析、代理还是导航。
/// 更狠的一层：Android 上 `navigate()` 与 `eval()` **恒返回 Ok**（wry 只是把消息投给
/// main pipe），且 wry **不向壳层透出 `onReceivedError`** ⇒ 主框架加载失败时壳侧完全无从
/// 得知，WebView 就把启动页一直留在屏幕上（`bootstrap.rs` 的 `Err(e) => show_error` 分支在
/// Android 上是**死代码**）。所以卡片上的信息是唯一的现场，这里把两侧的落点钉住。
const LOADING_RS: &str = include_str!("../loading.rs");
const BOOTSTRAP_RS: &str = include_str!("../bootstrap.rs");
/// 导航 / 窗口会话登记数据枢纽（P2-1 从 bootstrap.rs 抽出）：页面加载事件落点 /
/// BootReady 登记的 needle 已随代码迁到这里，断言语义不变。
const NAV_RS: &str = include_str!("../nav.rs");
const LIB_RS: &str = include_str!("../lib.rs");

/// `loading.rs` 必须提供往卡片追加信息的通路（`note`），且与 JS 侧 API 同名。
#[test]
fn loading_rs_exposes_note_channel() {
    assert!(
        LOADING_RS.contains("pub fn show_note(app: &AppHandle, label: &str, line: &str)"),
        "loading.rs 必须提供 show_note —— 没有它，壳侧无法把阶段进度 / 探测结论送到卡片上"
    );
    assert!(
        LOADING_RS.contains("window.__web2appLoading.note({l})"),
        "show_note 必须调用加载层对外暴露的 note API（改名会让通路静默失效）"
    );
}

/// 启动流程必须在关键节点留下进度，否则兜底卡无法判读。
#[test]
fn bootstrap_pushes_phase_progress() {
    for (needle, why) in [
        (
            "正在解析目标地址（{} 个域名）…",
            "域名数组配置必须报出「正在解析」并带域名个数",
        ),
        ("正在打开 {}…", "导航前必须报出「正在打开 <host>」"),
        (
            "正在启动本地反向代理…",
            "启用本地代理时必须报出「正在启动代理」",
        ),
        (
            "已发起导航，等待页面响应…",
            "导航发起后必须留痕 —— 这是区分「根本没导航」与「导航了但没提交」的唯一线索",
        ),
    ] {
        assert!(
            BOOTSTRAP_RS.contains(needle),
            "{why}（bootstrap.rs 缺 {needle}）"
        );
    }
    assert!(
        BOOTSTRAP_RS.contains("crate::loading::show_note(&a, &l, &s)"),
        "阶段进度必须经 loading::show_note 在【主线程】推送（eval 不允许从后台线程直接调）"
    );
}

/// 解析失败必须保留具体原因（旧实现统一吞成「服务器连接失败」，把唯一线索丢了）。
#[test]
fn resolve_failure_keeps_reason() {
    assert!(
        BOOTSTRAP_RS.contains("解析目标地址失败：{e}"),
        "解析失败必须把具体原因带进用户可见文案（读系统 DNS 配置失败 / 查询失败 / 预算超时…）"
    );
    assert!(
        BOOTSTRAP_RS.contains("RESOLVE_BUDGET_SECS"),
        "解析阶段必须设总预算：域名是串行查的，最坏可超过启动页 40s 的兜底卡"
    );
}

/// Android 启动期必须先等**首个文档加载完成**，再做解析 / 推送 / 导航。
///
/// 背景（2026-10-09 第二次真机取证定到的根因）：wry Android 的 `main_pipe.rs` 里
/// `WebViewMessage::LoadUrl` / `WebViewMessage::Eval` 两条消息的处理体都是
/// `if let Some(webview) = get_webview(activity_id) { … }` —— 目标未就绪时**静默丢弃**；
/// 而初始 splash 导航走的是 `post{}` 异步排队的 `loadUrlMainThread`，后续 `navigate` 走的
/// 是同步 `loadUrl`。直连配置毫秒级完成 bootstrap ⇒ 导航先发、随后被那条 post 出去的
/// splash 加载**覆盖**；同期的进度 `eval` 落在 document-start 之前被 `&&` 短路吃掉。
/// 真机 40s 卡上因此**只有页面自产的四行**、四条阶段进度一条不见。
#[test]
fn android_boot_waits_for_first_page_load() {
    assert!(
        BOOTSTRAP_RS.contains("fn wait_first_page_ready"),
        "bootstrap.rs 必须提供「等首个文档加载完成」的实现"
    );
    assert!(
        BOOTSTRAP_RS.contains("if !wait_first_page_ready(&app, &label)"),
        "启动流程必须真的调用它 —— 少了这一句，导航会被 post 出去的 splash 加载覆盖、\
             进度 eval 会被静默吃掉（真机取证 2026-10-09）"
    );
    assert!(
        NAV_RS.contains(
            "pub(crate) fn on_page_load(app: &tauri::AppHandle, label: &str, finished: bool)"
        ),
        "必须有页面加载事件的落点（等待靠它置位，改名/改签名会让等待恒超时；\
             落点已随 P2-1 迁往 nav.rs）"
    );
    assert!(
        NAV_RS.contains("ready.clear(label)"),
        "Started 必须清标记 —— 否则 /retry 重载启动页后沿用旧标记，重试仍会卡死"
    );
    assert!(
        BOOTSTRAP_RS.contains("SPLASH_READY_BUDGET_MS"),
        "等待必须有预算上限：拿不到页面加载事件时要退化成旧行为，而不是永远不启动"
    );
}

/// `on_page_load` 必须在**建主窗时**接到 `nav::on_page_load`，否则等待恒超时。
#[test]
fn page_load_event_is_wired_on_main_window() {
    assert!(
        LIB_RS.contains("nav::on_page_load("),
        "lib.rs 建主窗时必须把 on_page_load 接到 nav::on_page_load"
    );
    assert!(
        LIB_RS.contains("nav::BootReady::default()"),
        "BootReady 状态必须被 manage，否则 nav 取状态时会 panic"
    );
}

/// 在源码里找一段**位于代码行上**的文字（排除注释行）。
///
/// 为什么不用裸 `contains`：本项目已多次踩到「守卫的针被注释满足」—— 把调用点注释掉
/// 之后（`// xxx(...)`），`contains` 依然为真、守卫形同虚设。本次反向对照已实测到这一点。
fn has_code_line(src: &str, needle: &str) -> bool {
    src.lines().any(|line| {
        let t = line.trim();
        t.contains(needle) && !t.starts_with("//") && !t.starts_with('*')
    })
}

/// 导航后必须做「落点校验 + 有限重发」—— 这是「导航到底有没有生效」的终局检验。
///
/// 存在理由：`navigate()` 在 Android 上恒返回 `Ok(())`（只是往 main-pipe 投一条消息），
/// 所以**没有任何返回值能证明导航生效**；真机出现过「导航发起了、地址却停在
/// `splash.localhost`」。只靠「等首个文档就绪」是时序推断，这条是结果检验，两者互补。
#[test]
fn android_navigation_is_verified_and_retried() {
    // ① 判据必须同时认两种启动页形态：自定义协议 + wry 改写后的 http://splash.localhost/。
    //    只认自定义协议会在真机漏判 —— v1.62 已经栽过一次同类坑。
    assert!(
        has_code_line(
            BOOTSTRAP_RS,
            r#"url.scheme() == "splash" || url.host_str() == Some("splash.localhost")"#
        ),
        "启动页判据必须同时认 splash:// 与 http://splash.localhost/ 两种形态"
    );
    // ② 调用点必须真的接上（取语句本体，不取裸函数名 —— 否则会被注释满足）。
    assert!(
        has_code_line(
            BOOTSTRAP_RS,
            "ensure_navigation_committed(app, label, &url);"
        ),
        "导航成功后必须挂上落点校验：navigate() 恒返回 Ok，返回值不能作为成功判据"
    );
    // ③ 重发次数必须是有限常量（不允许无上限重发）。
    assert!(
        has_code_line(BOOTSTRAP_RS, "const NAV_COMMIT_MAX_RETRY: u32 = 2;"),
        "重发次数必须是有限常量，否则异常时无限重发"
    );
    // ④ 校验必须跑在独立线程：主线程里 sleep + 读地址（Android 走 main-pipe，
    //    未就绪时最长阻塞 10s）会把 UI 冻住。
    let start = BOOTSTRAP_RS
        .find("fn ensure_navigation_committed")
        .expect("落点校验函数缺失");
    let seg = &BOOTSTRAP_RS[start..(start + 900).min(BOOTSTRAP_RS.len())];
    assert!(
        has_code_line(seg, "std::thread::spawn"),
        "落点校验必须跑在独立线程里（主线程会冻 UI）"
    );
}
