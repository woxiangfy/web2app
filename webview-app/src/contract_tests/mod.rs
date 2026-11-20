//! 静态契约测试（只在 `cargo test` 下编译，由 lib.rs 的 `#[cfg(test)] mod contract_tests;` 挂入）。
//! 2026-10-25 按领域拆分为独立子模块（架构审查 2026-10-25 P1）：
//!   · `ipc_command_registry` —— IPC 命令注册表唯一调用点 + 前端命令 ⊆ 注册表；
//!   · `capability_contract` —— capabilities 授权 + Rust/JS 两侧事件名字面量；
//!   · `top_bar_style_contract` —— 多页签标签栏与单窗口标题栏的关键配色逐字一致；
//!   · `tab_home_marker_contract` —— 「首页」页签固定第一位、不可关闭；
//!   · `webview2_password_autosave` —— WebView2 密码自动保存开关（Windows）；
//!   · `loading_mode_contract` —— 加载层模式标记必须在 OVERLAY_SCRIPT 之前注册（跨平台）；
//!   · `diagnostics_visibility_contract` —— 屏幕上的技术细节由打包参数 `debug` 决定：
//!     loading.js 的诊断总闸 / notes 短路 / 两壳注入标记 / Electron 文案与 Chromium 日志；
//!   · `startup_visibility_contract` —— 启动可见性 / Android 首屏等待与重试；
//!   · `back_navigation_guard_contract` —— 标题栏后退落点守卫（含 Android 补丁链）；
//!   · `android_pipeline_contract` —— CI workflow 与 Android 补丁脚本管线契约（含打包输入面冻结）；
//!   · `float_window_contract` —— 悬浮球全套行为与逐像素 alpha（Windows）；
//!   · `startup_log_placement_contract` —— 启动信息实现只允许住在 debug_log.rs（架构审查 P1-1）；
//!   · `mobile_bar_contract` —— 移动端底栏 + 打包配置字段语义（含 Android 补丁接线）；
//!   · `packaging_field_contract` —— 打包配置 21 字段的**数据驱动**存在性守卫（字段表
//!     `scripts/packaging-fields.json` 驱动，2026-10-25 P1-b 取代逐字段手写存在性断言）；
//!   · `settings_hotkey_contract` —— 设置页「唤醒快捷键」= 组合键（修饰键 + 主键），
//!     单键/系统键保存前拦截、当前生效与待保存分离、读失败禁止保存（跨壳共用页面，跨平台）；
//!   · `module_visibility_contract` —— 跨模块引用（`crate::a::b`）的目标必须是 pub：
//!     平台专属模块（`probe` 等）的私有 fn 在 Windows 上「定义与调用一起被 cfg 摘掉」，
//!     只有 Android 目标才会 `error[E0603]`（2026-11-19 打包 red 的回归守卫）。
//!
//! 2026-10-03 从 lib.rs 整体迁出：入口文件不该混装数百行测试（既有架构评审 P1-1）。
//! ⚠️ 子模块位于 `src/contract_tests/`，比被测文件低一级目录 ⇒ 各子模块内 `include_str!`
//!    路径统一带一层 `../`（`"../lib.rs"`、`"../../capabilities/..."` 等）；
//!    include 进来的就是被测文件本体，将来若再移动这些文件，必须同步调整这些路径。

/// 只看「代码行」：把 `//` 之后的内容剥掉再匹配。
///
/// ⚠️ 为什么必须剥：本仓库踩过 —— 把一行代码**注释掉**来模拟删除时，`contains` 仍然
///    匹配到注释里的那段文本，于是守卫「如期通过」，实际上功能已经没了（假绿）。
///    2026-11-03 的设置页快捷键守卫就是这么被绕过的（`// reverse-check: btn.disabled = true;`）。
///
/// 只做行内切分（`split("//").next()`），不处理跨行 `/* */` 与字符串里的 `//`：
/// 对本仓库的使用场景（查单个 JS 语句 / 事件名）足够，且不会误伤 `http://` 之外的文本。
pub(crate) fn has_code_line(src: &str, needle: &str) -> bool {
    strip_line_comments(src).contains(needle)
}

/// 同上，返回命中行数（用于「至少出现在 N 处」这类断言）。
pub(crate) fn count_code_lines(src: &str, needle: &str) -> usize {
    strip_line_comments(src)
        .lines()
        .filter(|l| l.contains(needle))
        .count()
}

/// 去掉行注释后的整车文本 —— 供「A 必须出现在 B 之前」这类位置断言使用。
///
/// 位置断言如果直接 `find` 原始文本，注释里写一遍同样的代码就能伪造顺序（同 `contains` 的坑）。
pub(crate) fn strip_line_comments(src: &str) -> String {
    src.lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

/// IPC 命令注册表的静态契约测试。
///
/// 存在理由：`Builder::invoke_handler` 是**替换**语义，多调用一次就会静默丢掉前一次的
/// 全部命令 —— 编译期无警告、CI 全绿，只在用户点下按钮那一刻变成「Command … not found」
/// （真实事故：Windows 设置页「保存失败：Command save_settings not found」）。
/// 这里用两条静态断言把它钉死：① 调用点唯一；② 前端调用的命令名 ⊆ 注册表里的名字。
#[cfg(test)]
mod ipc_command_registry;

#[cfg(test)]
mod capability_contract;

#[cfg(test)]
mod top_bar_style_contract;

#[cfg(test)]
mod tab_home_marker_contract;

#[cfg(test)]
mod webview2_password_autosave;

#[cfg(test)]
mod loading_mode_contract;

#[cfg(test)]
mod diagnostics_visibility_contract;

#[cfg(test)]
mod startup_visibility_contract;

#[cfg(test)]
mod back_navigation_guard_contract;

#[cfg(test)]
mod android_pipeline_contract;

#[cfg(test)]
#[cfg(windows)]
mod float_window_contract;

#[cfg(test)]
mod startup_log_placement_contract;

#[cfg(test)]
mod mobile_bar_contract;

#[cfg(test)]
mod packaging_field_contract;

#[cfg(test)]
mod settings_hotkey_contract;

#[cfg(test)]
mod module_visibility_contract;
