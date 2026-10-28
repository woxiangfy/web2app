/// 首页页签标记（`src/shell.html`）的静态契约。
///
/// 背景：多页签模式下打开默认入口地址（app_url）的那一签是「首页」—— 它**固定在第一位**且
/// **不允许关闭**，页签名字前带一个房子图标作标记。判定真源在宿主侧（Rust
/// `tabs.rs::is_home_index` / Electron `tab-order.ts::isPinnedHome`），本模块钉住**渲染那一半**。
///
/// 为什么必须钉：三处漏一处都是**静默降级** —— 编译期零警告、clippy 与 CI 全绿：
///   · 不画图标 ⇒ 用户看不出哪个是首页（需求要的「特定标记」没了）；
///   · 给首页也画 × ⇒ 用户点了没反应（后端已拒绝），本项目最忌讳的失败形态；
///   · 解不出 / 不下发 `home` ⇒ 首页退化成普通页签，上面两条同时发生。
///
/// ⚠️ 位置类断言（「关闭按钮必须被 home 判断包住」）必须在**只含代码行**的文本上做，
/// 否则注释里的同名字面量会把位置带偏（本项目已栽过「针被注释满足」）。
/// 被测真源：双壳共用的标签栏页面（Electron 经 `shell://` 协议读同一份文件）。
const SHELL: &str = include_str!("../shell.html");

/// 只认真正的代码行（排除 `//` 注释行与 `*` 续行）。
fn code_lines(src: &str) -> String {
    src.lines()
        .filter(|line| {
            let t = line.trim();
            !t.starts_with("//") && !t.starts_with('*')
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_code_line(src: &str, needle: &str) -> bool {
    code_lines(src).contains(needle)
}

#[test]
fn home_tab_renders_icon_and_never_a_close_button() {
    // ① 样式：首页专属类名 + 图标盒子（图标颜色走 currentColor，深浅色主题自动跟随）。
    assert!(
        has_code_line(SHELL, ".tab.home .label"),
        "shell.html 缺少首页页签样式 `.tab.home .label`（首页应当在名字上加粗/标记）"
    );
    assert!(
        has_code_line(SHELL, ".tab .home-ico {"),
        "shell.html 缺少 `.tab .home-ico` 图标盒子样式"
    );

    // ② 渲染：home 进类名 + 图标挂在 label **之前**（名字前的小图标）。
    assert!(
        has_code_line(
            SHELL,
            r#"el.className = "tab" + (t.active ? " active" : "") + (t.home ? " home" : "")"#
        ),
        "render() 必须按 t.home 给页签加 `home` 类（改坏 ⇒ 首页在标签栏上看不出区别）"
    );
    assert!(
        has_code_line(SHELL, "if (t.home) el.appendChild(homeIcon());"),
        "render() 必须在首页上插入主页图标"
    );
    // 图标本体：钉住房子路径数据本身，避免「插一个空盒子」也算过。
    assert!(
        SHELL.contains("M10 20v-6h4v6h5v-8h3L12 3 2 12h3v8z"),
        "主页图标必须是真的房子图形（路径数据被改掉了？）"
    );

    // ③ 关闭按钮：`.close` 的构造必须在 `if (!t.home)` 之内 —— 位置断言（成对判据的
    //    「不该有」那一半；「该有」那一半由 Electron 探针在真实 DOM 上验）。
    let code = code_lines(SHELL);
    let guard = code
        .find("if (!t.home) {")
        .expect("render() 必须有 `if (!t.home) {` 守卫：首页不渲染关闭按钮");
    let close = code
        .find(r#"close.className = "close";"#)
        .expect("render() 必须构造关闭按钮（非首页页签仍要能关）");
    assert!(
        guard < close,
        "关闭按钮必须被 `if (!t.home)` 包住 —— 顺序反了等于给首页也画了 ×"
    );

    // ④ 初始化链路：get_tabs 是**四元组**，解构必须带 home（漏一个 ⇒ 首页退化成普通页签）。
    assert!(
        has_code_line(
            SHELL,
            "render(list.map(([label, title, active, home]) => ({ label, title, active, home })));"
        ),
        "shell.html 的 get_tabs 初始化必须解出第 4 位 home（对齐 tabs.rs::get_tabs 的四元组）"
    );
}
