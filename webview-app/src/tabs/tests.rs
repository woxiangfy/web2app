//! tabs 单元测试 —— P4 拆分自 tabs.rs 末尾的 mod tests（条目逐字搬家）。

/// 生产代码全文（拆分后的 tabs 根 + state / window / ipc 子模块，不含测试自身）。
/// 断言仍按「#[cfg(test)] 之前」收口生产段；测试不在拼接里，断言不会被自己的字面量满足。
const TABS_PROD_SRC: &str = concat!(
    include_str!("../tabs/state.rs"),
    include_str!("../tabs/window.rs"),
    include_str!("../tabs/ipc.rs"),
    include_str!("../tabs.rs"),
);

use super::*;

/// 首页页签（固定第一位 + 不可关）的规则与下发链路守卫。
///
/// 三处任何一处漏改都会**静默降级** —— 编译期零警告、clippy 与 CI 全绿，只有真机上才现形：
///   · `is_home_index` 只在位次 0 为真 —— 「固定在第一位」这条不变式的全部依据；
///   · `close_tab_inner` 必须**在摘登记 / 销毁窗口之前**拦下首页（否则留下半个状态：
///     登记没了、窗口还在，而标签栏本就不给关闭按钮 ⇒ 一具打不开也关不掉的残骸）；
///   · `tab_rows` / `push_tabs_to_shell` 必须把 home 下发给标签栏（否则 shell.html 画不出
///     房子图标，还会给首页画上「×」）。
///
/// ⚠️ 只统计 `#[cfg(test)]` 之前的生产代码段：否则断言自己的字面量会把自己满足掉（自引用）。
/// ⚠️ 用 `has_code_line` 而不是裸 `contains`：本项目的针被注释满足过不止一次。
#[test]
fn home_tab_is_the_first_index_and_cannot_be_closed() {
    assert!(is_home_index(0), "位次 0 必须是首页（固定第一位）");
    assert!(
        !is_home_index(1) && !is_home_index(2) && !is_home_index(MAX_TABS - 1),
        "首页只有位次 0 一个，其余位次都不是"
    );

    let full = TABS_PROD_SRC;
    let src = full.split("#[cfg(test)]").next().unwrap_or(full);

    // 下发链路：快照四元组 + push 里的 home 字段。
    assert!(
        has_code_line(src, "Vec<(String, String, bool, bool)>"),
        "get_tabs / tab_rows 必须返回四元组（第 4 位 = home）"
    );
    assert!(
        has_code_line(src, "active:{active_js},home:{home_js}"),
        "push_tabs_to_shell 必须把 home 下发给 shell.html（漏了 ⇒ 首页没有图标、还带 ×）"
    );

    // 拒绝链路：拦截语句必须真的在 close_tab_inner 的函数体里，且排在销毁窗口【之前】。
    // ⚠️ 位置断言一律在「只含代码行」的文本上做 —— 裸 `find` 会被注释里的同名字面量带偏。
    let code = code_lines(src);
    let body = {
        let at = code
            .find("pub(crate) fn close_tab_inner")
            .expect("close_tab_inner 必须存在");
        &code[at..]
    };
    let refuse = body
        .find("is_some_and(is_home_index)")
        .expect("close_tab_inner 必须拦截首页（is_some_and(is_home_index)）");
    let destroy = body
        .find("window.destroy()")
        .expect("close_tab_inner 必须销毁 tab 窗口（destroy 而非 close，见函数文档）");
    assert!(
        refuse < destroy,
        "首页拦截必须排在「摘登记 / 销毁窗口」之前：先动后判会留下半个状态"
    );
    let unregister = body
        .find("unregister_tab(app, label);")
        .expect("close_tab_inner 必须摘除登记");
    assert!(
        refuse < unregister,
        "首页拦截必须排在 unregister_tab 之前（登记被摘掉 = 首页从列表里消失）"
    );
}

/// 逐行取「代码行」（排除 `//` 注释行与 `*` 续行）拼回的文本，与 contract_tests.rs 的
/// `has_code_line` 同一姿势 —— 防「守卫的针被注释满足」。
fn code_lines(src: &str) -> String {
    src.lines()
        .filter(|line| {
            let t = line.trim();
            !t.starts_with("//") && !t.starts_with('*')
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 见 [`code_lines`]：源码里是否存在「位于代码行上」的这段文字。
fn has_code_line(src: &str, needle: &str) -> bool {
    code_lines(src).contains(needle)
}

/// 构造一个可单测的 Navigation（本地反向代理模式）。
/// `entry`：代理入口（形如 http://127.0.0.1:PORT/）；`upstream`：上游真实**源**
/// （scheme://host[:非默认端口]，与 nav::origin_of 同形态）。
fn proxy_nav(entry: &str, upstream: Option<&str>) -> Navigation {
    Navigation {
        url: entry.to_string(),
        user: String::new(),
        pass: String::new(),
        use_local_proxy: true,
        proxy_abort: None,
        upstream_origin: upstream.map(str::to_string),
    }
}

#[test]
fn popup_same_as_entry_host_joins_path_query_fragment() {
    let nav = proxy_nav("http://127.0.0.1:4567/", None);
    let out =
        resolve_popup_url_for(&nav, Some("http://127.0.0.1:4567/notes?from=popup#x")).unwrap();
    assert_eq!(out, "http://127.0.0.1:4567/notes?from=popup#x");
}

#[test]
fn popup_same_as_entry_replaces_root_path() {
    let nav = proxy_nav("http://127.0.0.1:4567/app", None);
    let out = resolve_popup_url_for(&nav, Some("http://127.0.0.1:4567/other")).unwrap();
    assert_eq!(out, "http://127.0.0.1:4567/other");
}

#[test]
fn popup_same_as_upstream_remaps_through_entry() {
    // 页面里写死的绝对链接（指向上游真实主机）→ 映射到代理入口。
    let nav = proxy_nav("http://127.0.0.1:4567/", Some("https://real.example.com"));
    let out = resolve_popup_url_for(&nav, Some("https://real.example.com/path/a")).unwrap();
    assert_eq!(out, "http://127.0.0.1:4567/path/a");
    // 显式默认端口与省略端口同源
    let out = resolve_popup_url_for(&nav, Some("https://real.example.com:443/path/b")).unwrap();
    assert_eq!(out, "http://127.0.0.1:4567/path/b");
}

#[test]
fn popup_non_default_port_is_not_same_origin() {
    // 旧实现只比 host：同主机的非默认端口会被误判同源、抹掉端口映射到错误服务。
    let nav = proxy_nav("http://127.0.0.1:4567/", Some("https://real.example.com"));
    assert_eq!(
        resolve_popup_url_for(&nav, Some("https://real.example.com:8443/path")),
        Some("https://real.example.com:8443/path".into()),
        "非默认端口不算上游源：代理模式下外链原样打开，不抹端口"
    );
    // scheme 不同也不同源（http 对 https）
    assert_eq!(
        resolve_popup_url_for(&nav, Some("http://real.example.com/path")),
        Some("http://real.example.com/path".into())
    );
}

#[test]
fn popup_cross_domain_proxy_mode_opens_external() {
    // 代理只对固定上游注入凭据，跨域外链直接打开不会泄漏账号。
    let nav = proxy_nav("http://127.0.0.1:4567/", None);
    assert_eq!(
        resolve_popup_url_for(&nav, Some("https://other.com/x")),
        Some("https://other.com/x".into())
    );
}

#[test]
fn popup_cross_domain_plain_mode_opens_external() {
    // 非代理（直连）模式下，跨域外链同样直接打开目标地址（与 Electron 壳一致）：
    // 凭据由 `is_upstream_request` 主机白名单把关，外域请求不会拿到入口账号密码。
    // 历史实现在这里返回 None，导致书签管理器点开外站书签时新 tab 落到入口首页
    // （用户反馈的「新标签页都是首页，不是指定的页面」）。
    let mut nav = proxy_nav("https://entry.example.com", None);
    nav.use_local_proxy = false;
    assert_eq!(
        resolve_popup_url_for(&nav, Some("https://other.com/x")),
        Some("https://other.com/x".into())
    );
    // 同源路径照常拼回入口。
    assert_eq!(
        resolve_popup_url_for(&nav, Some("https://entry.example.com/notes")),
        Some("https://entry.example.com/notes".into())
    );
}

#[test]
fn popup_missing_or_unsupported_target_returns_none() {
    let nav = proxy_nav("http://127.0.0.1:4567/", None);
    assert_eq!(resolve_popup_url_for(&nav, None), None);
    assert_eq!(resolve_popup_url_for(&nav, Some("about:blank")), None);
    assert_eq!(
        resolve_popup_url_for(&nav, Some("javascript:void(0)")),
        None
    );
    assert_eq!(resolve_popup_url_for(&nav, Some("not a url")), None);
}

#[test]
fn close_adjacency_plan_matches_tab_order_rule() {
    // 4 个页签 [0,1,2,3]，删除后幸存 3 个。
    // 关激活本身：右邻优先（删末位时夹到左邻）。
    assert_eq!(plan_active_after_close(0, 0, 3), (0, true));
    assert_eq!(plan_active_after_close(1, 1, 3), (1, true));
    assert_eq!(plan_active_after_close(2, 2, 3), (2, true));
    assert_eq!(plan_active_after_close(3, 3, 3), (2, true), "删末位 → 左邻");
    // 关激活【之前】的后台 tab：索引前移，不需要显式激活。
    assert_eq!(plan_active_after_close(2, 0, 3), (1, false));
    assert_eq!(plan_active_after_close(3, 1, 3), (2, false));
    // 关激活【之后】的 tab：索引不变。
    assert_eq!(plan_active_after_close(0, 2, 3), (0, false));
    assert_eq!(plan_active_after_close(1, 2, 3), (1, false));
    // 2 页签关 1（幸存 1）：左右邻都是它。
    assert_eq!(plan_active_after_close(0, 0, 1), (0, true));
    assert_eq!(plan_active_after_close(1, 1, 1), (0, true));
    assert_eq!(plan_active_after_close(1, 0, 1), (0, false));
}

/// 外部代理开关必须同时挂在宿主与 tab 两个 builder 上（少一个 → 多页签绕过外部代理，
/// 见 create_host_window 的注释）。只统计 #[cfg(test)] 之前的生产代码段，避免本断言
/// 自己的字符串被计入（自引用）。
#[test]
fn external_proxy_args_on_both_host_and_tab_builders() {
    let full = TABS_PROD_SRC;
    let src = full.split("#[cfg(test)]").next().unwrap_or(full);
    assert_eq!(
        src.matches(".additional_browser_args(").count(),
        2,
        "宿主窗口与 tab 窗口必须各有一处 additional_browser_args"
    );
    assert!(src.contains("crate::config::build_proxy_args()"));
}

/// 首屏 bootstrap 触发点必须存在，且导航缓存未就绪时的轮询回调不得把弹窗落点丢成 None。
///
/// 两个都是「静默降级、只在真机上才现形」的缺陷，各配一条静态断言钉住：
/// · 缺 bootstrap 触发点 → 首个 tab 空等 30s 超时才回落（首次打开极慢）；
/// · 轮询回调硬编码 None → 首屏就绪前开出的 tab 全落回入口地址（新 tab 都是首页）。
///
/// 只统计 #[cfg(test)] 之前的生产代码段（避免与测试里的注释互相引用）。
#[test]
fn first_tab_triggers_bootstrap_and_poll_keeps_start() {
    let full = TABS_PROD_SRC;
    let src = full.split("#[cfg(test)]").next().unwrap_or(full);
    assert!(
        src.contains("bootstrap_for_label(app.clone(), label.to_string())"),
        "首个 tab 必须直接启动完整 bootstrap（否则首次打开要空等 30s）"
    );
    assert!(
        !src.contains("navigate_tab(&a, &l, None)"),
        "轮询回调必须原样保留弹窗落点 start，不能硬编码 None"
    );
}
