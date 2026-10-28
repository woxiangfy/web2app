//! 多页签（tab）模式（仅 Windows）。
//!
//! 需求定位：**同账号、共享数据**的多页签——一个宿主窗口 + 顶部标签栏，每个 tab 是
//! 一个独立的 WebviewWindow（共享同一个 WebView2 数据目录，cookie/localStorage 天然互通）。
//! tab 可随时新建 / 切换 / 关闭；`target="_blank"` / `window.open()` 直接新开一个 tab。
//!
//! 技术要点（这是「多窗口伪装成 tab」，而非浏览器原生 tab 抽象）：
//! - Tauri/Wry 无原生 tab，一个 WebviewWindow = 一个 WebView。tab 窗口去边框（`.decorations(false)`）、
//!   坐标吸附在宿主窗口标签栏下方；宿主窗口拖动 / 缩放时，所有 tab 窗口批量跟随。
//! - 标签栏由「宿主窗口」渲染（`shell://localhost/` 自定义协议，前端 HTML），tab 的增删切换通过
//!   IPC 命令回调 Rust；宿主窗口本身不承载网页内容，只画标签栏。
//! - **首页页签**：第一个 tab（打开 app_url 的那一签）固定在第一位且**不允许关闭**，标签栏里
//!   名字前带房子图标作标记。判定规则见 [`is_home_index`]，渲染在 `shell.html::render`。
//! - 本地反向代理保持**单实例单端口**（所有 tab 访问同一上游 + 同一账号），`ProxyTask` 无需改动。
//! - 首个 tab 的域名解析 / 代理启动完成后，结果缓存到 `TabManager`，后续新建 tab 直接复用，
//!   不再重复 DNS TXT 查询。
//!
//! 本模块整体 `#[cfg(windows)]`：多页签能力仅在 Windows 实现（Android 无多窗口叠放 + 共享
//! WebView2 数据目录的对应能力）。

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::{Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::config::{TABS_ENABLED, TITLE};
use crate::nav::Navigation;

/// 宿主窗口（标签栏所在）与 tab 窗口的固定 label 前缀。
/// ⚠️ `HOST_LABEL` 本体已迁 [`crate::nav`]（P2-1：`main_window_label` 的唯一真源在那边，
/// 否则 nav 要回头调 tabs 就成了层级环），这里经 `use` 重导出，
/// 保持既有 `tabs::HOST_LABEL` 引用路径不变。
pub(crate) use crate::nav::HOST_LABEL;
// ===========================================================================
// 子模块（P4 大文件拆分：按功能垂直拆出，零行为变更纯代码搬家，条目见各文件头）
// ===========================================================================

mod ipc;
mod state;
mod window;

use ipc::*;
use state::*;
use window::*;

/// 对外引用路径不变（crate::tabs::*）：子模块中对外的条目在此重导出。
///
/// ⚠️ `#[tauri::command]` 会额外生成 `__cmd__*` / `__tauri_command_name_*` 两个隐藏宏
/// （`generate_handler![tabs::xxx]` 经它们取请求头与命令名），重导出必须连宏一起，
/// 否则 `tabs::xxx` 路径解析不到宏（E0603 / 私有宏导入）。
pub(crate) use ipc::{
    __cmd__close_tab, __cmd__create_tab, __cmd__get_tabs, __cmd__switch_tab,
    __tauri_command_name_close_tab, __tauri_command_name_create_tab, __tauri_command_name_get_tabs,
    __tauri_command_name_switch_tab, close_tab, create_tab, get_tabs, open_new_tab_from_request,
    report_tab_title, retry_tab_bootstrap, switch_tab,
};
pub(crate) use state::{TabManager, SHELL_SCHEME, TAB_PREFIX, TAB_RETRY_EVENT, TAB_TITLE_EVENT};
pub(crate) use window::{
    __cmd__close_host, __cmd__minimize_host, __tauri_command_name_close_host,
    __tauri_command_name_minimize_host, close_host, close_tab_inner, create_host_window,
    destroy_all_tabs, handle_shell, minimize_host, set_tabs_visible, spawn_tab, tabs_enabled,
};

#[cfg(test)]
mod tests;
