/// 顶部通栏配色的静态契约测试。
///
/// 多页签标签栏（`src/shell.html`）与单窗口标题栏（`src/titlebar.js`）是**同一块 UI** 的两种
/// 形态：前者铺在宿主窗口顶部、后者叠在主窗口顶部。配色/分隔线一旦漂移，两种模式看起来就不像
/// 同一个应用（使用者 2026-10-08 反馈「多页签顶部通栏风格要与单页签的一致」）。这里把两处共用的
/// 关键色值钉死 —— 改配色必须两边一起改，否则本测试报错。
/// 多页签顶栏（宿主窗口 decorations(false) + 自绘标签栏）。
const SHELL: &str = include_str!("../shell.html");
/// 单窗口 / 多页签 tab 页的自绘标题栏。
const TITLEBAR: &str = include_str!("../titlebar.js");

/// 两处顶栏必须共用同一套关键色：浅底 / 深底 / 正文 / 深色正文 / 浅色分隔线 / 深色分隔线 /
/// 按钮 hover 底色。字面量在两份文件里都写成小写、无多余空格，便于逐字比对。
#[test]
fn tab_bar_palette_matches_titlebar() {
    const SHARED: &[&str] = &[
        "#eef0f2",               // 浅色底
        "#1f2530",               // 深色底
        "#2c3440",               // 浅色正文
        "#e6ebf2",               // 深色正文
        "rgba(0,0,0,.08)",       // 浅色下沿分隔线
        "rgba(255,255,255,.07)", // 深色下沿分隔线
        "rgba(120,133,153,.16)", // 按钮 hover 底色
    ];
    for lit in SHARED {
        assert!(
            TITLEBAR.contains(lit),
            "titlebar.js 缺少顶栏配色 {lit}（本测试的真源之一）"
        );
        assert!(
            SHELL.contains(lit),
            "shell.html 缺少与 titlebar.js 一致的顶栏配色 {lit}：\
                 多页签标签栏与单页签标题栏必须同色（改配色请两份文件一起改）"
        );
    }
}
