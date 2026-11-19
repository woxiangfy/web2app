//! `mobilebar` 的单元测试 —— 2026-11-19 P-R5 从 `mobilebar.rs` 末尾外置（纯搬家）。
//!
//! 条目与拆分前逐字一致，`use super::*` 的可见范围不变（仍是 `mobilebar` 模块内私有项）。
//! 外置理由：生产侧 772 行已按「状态机 / 事件入口 / 导航原语」分节，218 行测试混在同一个
//! 文件里只会让阅读与契约扫描（`mobile_bar_contract::BAR_RS`）都更费劲 —— 拆分前契约
//! 扫描的文本里混着断言自身的字面量，必须靠剥注释才能不自满足。

use super::*;

#[test]
fn config_script_reports_height_and_tabs_flag() {
    let s = config_script();
    assert!(
        s.starts_with("window.__web2appMobileBarCfg = {"),
        "cfg 脚本必须是给页面赋值的语句：{s}"
    );
    assert!(
        s.contains(&format!("\"h\":{BAR_HEIGHT}")),
        "cfg 脚本必须带上底栏高度真源：{s}"
    );
    // tabs 段必须是字面量 true/false 之一（前端按 `=== true` 判定）
    assert!(
        s.contains("\"tabs\":true") || s.contains("\"tabs\":false"),
        "cfg 脚本必须带上页签可用性标记：{s}"
    );
}

#[test]
fn tabs_available_follows_compile_flag_in_test_build() {
    // 本机 `cargo test` 默认不注入 TABS_ENABLED ⇒ 这里必须是 false（cfg 脚本因此
    // 下发 "tabs":false，前端不画列表按钮）。CI 的 `TABS_ENABLED=true cargo check`
    // 只编译不执行，不会跑到这条断言。
    assert!(
        !tabs_available(),
        "测试构建未注入 TABS_ENABLED，tabs_available() 应为 false"
    );
}

#[test]
fn same_target_compares_scheme_host_port_path_only() {
    let u = |s: &str| url::Url::parse(s).unwrap();
    assert!(same_target(&u("https://a.com/"), &u("https://a.com/")));
    assert!(same_target(&u("https://a.com/"), &u("https://a.com")));
    // 一次性参数不影响「是不是这个页面」
    assert!(same_target(&u("https://a.com/?t=1"), &u("https://a.com/")));
    assert!(same_target(&u("https://a.com/#x"), &u("https://a.com/")));
    // 端口 / scheme / 路径 / 主机不同 ⇒ 不是同一个落点
    assert!(!same_target(
        &u("https://a.com:8443/"),
        &u("https://a.com/")
    ));
    assert!(!same_target(&u("http://a.com/"), &u("https://a.com/")));
    assert!(!same_target(&u("https://a.com/b"), &u("https://a.com/")));
    assert!(!same_target(&u("https://b.com/"), &u("https://a.com/")));
    // 代理模式下首页是 127.0.0.1:<随机端口>，与上游域名**不**同落点
    assert!(!same_target(
        &u("http://127.0.0.1:51234/"),
        &u("https://a.com/")
    ));
}

#[test]
fn tab_label_of_parses_label_or_gives_up() {
    assert_eq!(
        tab_label_of("{\"label\":\"tab-1\"}").as_deref(),
        Some("tab-1")
    );
    assert_eq!(tab_label_of("not json"), None);
    assert_eq!(tab_label_of("{}"), None);
}

#[test]
fn event_names_are_stable() {
    // 事件名是跨文件契约（前端字面量 ↔ 本模块常量），改这里必须同步改 底栏脚本。
    assert_eq!(NAV_HOME_EVENT, "web2app:nav-home");
    assert_eq!(SWITCH_TAB_EVENT, "web2app:switch-tab");
    assert_eq!(CLOSE_TAB_EVENT, "web2app:close-tab");
    assert_eq!(NEW_TAB_EVENT, "web2app:new-tab");
    assert_eq!(OPEN_TAB_EVENT, "web2app:open-tab");
    assert_eq!(REQUEST_TABS_EVENT, "web2app:request-tabs");
    assert_eq!(DOC_EVENT, "web2app:mobile-doc");
}

#[test]
fn open_tab_url_of_parses_url_or_gives_up() {
    assert_eq!(
        open_tab_url_of("{\"url\":\"https://a.com/x\"}").as_deref(),
        Some("https://a.com/x")
    );
    assert_eq!(open_tab_url_of("not json"), None);
    assert_eq!(open_tab_url_of("{}"), None);
    // 数字 / null 等非字符串 url：反序列化失败 = 未知（不当成合法落点）。
    assert_eq!(open_tab_url_of("{\"url\":123}"), None);
}

/// 页签记录状态机：与桌面端同一条「首页固定 0 号位」契约，加上单 webview 模型特有的
/// 「切换/新建前由接线层把当前 url 记进活动标签」前提（observe）。
#[test]
fn mobile_tabs_lifecycle_and_home_pin() {
    let mut t = MobileTabs::default();
    t.reset_home("https://a.com/");

    // 初始：只有首页，位次 0，活动。
    assert_eq!(t.tabs.len(), 1);
    assert_eq!(t.tabs[0].label, HOME_TAB_LABEL);
    assert_eq!(t.active, 0);

    // 首页关不掉。
    assert!(matches!(t.close(HOME_TAB_LABEL), CloseOutcome::Blocked));
    assert_eq!(t.tabs.len(), 1);

    // 用户在首页内浏览，url 被 observe 记进活动标签。
    assert!(t.observe("https://a.com/news", Some("新闻")));
    assert_eq!(t.tabs[0].url, "https://a.com/news");
    assert!(!t.observe("https://a.com/news", Some("新闻"))); // 幂等
    assert!(!t.observe("https://a.com/news", Some(""))); // 空标题不覆盖
    assert_eq!(t.tabs[0].title, "新闻");

    // 新建两个标签：m1 / m2，活动位次随新建推进，序号只增不复用。
    assert_eq!(t.new_tab("https://a.com/"), 1);
    t.observe("https://a.com/m1", Some("页面1"));
    assert_eq!(t.new_tab("https://a.com/"), 2);
    t.observe("https://a.com/m2", Some("页面2"));
    assert_eq!(t.tabs.len(), 3);
    assert_eq!(t.tabs[1].label, "m1");
    assert_eq!(t.tabs[2].label, "m2");

    // 点当前标签不切换。
    assert!(t.switch("m2").is_none());
    // 切回首页：活动位次 0。
    assert_eq!(t.switch(HOME_TAB_LABEL), Some(0));
    // 在首页标签上继续浏览（接线层会 observe）。
    t.observe("https://a.com/again", None);
    assert_eq!(t.tabs[0].url, "https://a.com/again");

    // 关掉后台标签 m1：不导航（navigate_to=None），活动位次仍为 0。
    assert!(matches!(
        t.close("m1"),
        CloseOutcome::Removed { navigate_to: None }
    ));
    assert_eq!(t.tabs.len(), 2);
    assert_eq!(t.active, 0);
    // m2 因 m1 被删而前移到位次 1。
    assert_eq!(t.tabs[1].label, "m2");

    // 关掉当前标签（首页不可能；构造活动在 m2 后关闭它）：导航到左邻（首页）。
    assert_eq!(t.switch("m2"), Some(1));
    match t.close("m2") {
        CloseOutcome::Removed {
            navigate_to: Some(i),
        } => assert_eq!(i, 0),
        other => panic!("关活动标签应导航到左邻，实际 {other:?}"),
    }
    assert_eq!(t.tabs.len(), 1);
    assert_eq!(t.active, 0);

    // 再新建：序号继续增长（m3，不与已关的 m1/m2 撞）。
    assert_eq!(t.new_tab("https://a.com/"), 1);
    assert_eq!(t.tabs[1].label, "m3");
}

#[test]
fn observe_ignores_builtin_and_empty_state() {
    let mut t = MobileTabs::default();
    // 还没有任何标签时 observe 不改东西。
    assert!(!t.observe("https://a.com/", None));
    t.reset_home("https://a.com/");
    // 两种启动页形态都不认。
    assert!(!t.observe("splash://localhost/retry", Some("启动页")));
    assert!(!t.observe("http://splash.localhost/", Some("启动页")));
    assert!(t.observe("https://a.com/x", None));
}

#[test]
fn rows_json_is_four_tuple_with_home_and_active_flags() {
    let mut t = MobileTabs::default();
    t.reset_home("https://a.com/");
    t.new_tab("https://a.com/");
    t.observe("https://a.com/p", Some("内页"));
    // 切到首页，让两个 active 标记都好验。
    t.switch(HOME_TAB_LABEL);
    let s = t.rows_json();
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    let rows = v.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    // [label,title,active,home]
    assert_eq!(rows[0][0], HOME_TAB_LABEL);
    assert_eq!(rows[0][1], "首页"); // 首页无标题时回退「首页」
    assert_eq!(rows[0][2], true); // 当前活动
    assert_eq!(rows[0][3], true); // home 标记
    assert_eq!(rows[1][0], "m1");
    assert_eq!(rows[1][1], "内页");
    assert_eq!(rows[1][2], false);
    assert_eq!(rows[1][3], false);
}

#[test]
fn display_title_fallbacks_and_builtin_detection() {
    assert!(is_builtin_url("splash://localhost/"));
    assert!(is_builtin_url("http://splash.localhost/retry"));
    assert!(is_builtin_url("https://splash.localhost/"));
    assert!(!is_builtin_url("https://a.com/"));
    assert!(!is_builtin_url("not a url"));

    let home = MTab {
        label: HOME_TAB_LABEL.into(),
        url: "https://a.com/".into(),
        title: String::new(),
    };
    assert_eq!(display_title(&home, 0), "首页");
    let other = MTab {
        label: "m1".into(),
        url: "https://shop.example.com/p".into(),
        title: String::new(),
    };
    assert_eq!(display_title(&other, 1), "shop.example.com");
    let titled = MTab {
        label: "m1".into(),
        url: "https://a.com/".into(),
        title: "  我的页面  ".into(),
    };
    assert_eq!(display_title(&titled, 1), "我的页面");
}
