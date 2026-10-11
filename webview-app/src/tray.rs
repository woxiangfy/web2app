//! 系统托盘与窗口切换（仅桌面端）。
//!
//! 注意：设置相关命令（get_settings / save_settings / close_settings_window）不在此处——
//! 它们是跨平台功能，已移入 [`crate::settings`] 无条件编译；本模块只保留纯桌面端的
//! 托盘与窗口切换逻辑。

use tauri::Manager;

use crate::config::TITLE;

// ⚠️ 「主窗口 label」与「tab 窗口显隐同步」两个原语已统一搬到 [`crate::bootstrap`]
//    （`main_window_label` / `sync_tabs_visibility`）—— 它们不是桌面端专属
//    （主窗关闭语义在 Android 上同样要走），而本模块是 `cfg(desktop)`，放在这里会让
//    非桌面目标拿不到。这里改为直接调用，消除第 2、3 份重复实现。
use crate::bootstrap::{main_window_label, sync_tabs_visibility};

/// 切换主窗口：正常可见则隐藏；最小化或已隐藏则恢复并显示置顶（仅桌面端）。
pub(crate) fn toggle_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(main_window_label()) {
        let visible = window.is_visible().unwrap_or(false);
        let minimized = window.is_minimized().unwrap_or(false);
        if visible && !minimized {
            let _ = window.hide();
            sync_tabs_visibility(app, false);
        } else {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
            sync_tabs_visibility(app, true);
        }
    }
}

/// 显示并聚焦主窗口（托盘菜单「显示窗口」/ 第二实例 共用）：
/// 取消最小化、显现、置顶聚焦；不做 toggle——调用方意图就是「把窗口调出来」。
///
/// ⚠️ **单击悬浮球不走这里**（v1.71 起改走 [`toggle_window`]：可见且非最小化 → 隐藏）——
/// 别再把它当成悬浮球的入口写进注释或调用点。
pub(crate) fn show_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(main_window_label()) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    sync_tabs_visibility(app, true);
}

/// 创建系统托盘图标（含右键菜单、左键切换窗口）。仅当设置“启用托盘”时调用。
pub(crate) fn create_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let show_i = MenuItem::with_id(app, "show", "显示窗口", true, None::<&str>)?;
    let settings_i = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    // 悬浮图标菜单项仅 Windows（floatwin 模块整体 cfg(windows)）；两壳托盘项保持一致。
    #[cfg(windows)]
    let float_i = MenuItem::with_id(app, "float", "悬浮图标", true, None::<&str>)?;
    let hide_i = MenuItem::with_id(app, "hide", "隐藏到托盘", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    #[cfg(not(windows))]
    let menu = Menu::with_items(app, &[&show_i, &settings_i, &hide_i, &quit_i])?;
    #[cfg(windows)]
    let menu = Menu::with_items(app, &[&show_i, &settings_i, &float_i, &hide_i, &quit_i])?;

    // 图标缺失返回 Err 而不是 expect：本函数在「设置里勾选启用托盘」的运行期路径上也会被
    // 调用（settings.rs），expect 会让进程因配置问题直接 panic；交给上层弹可读错误。
    let icon = app.default_window_icon().cloned().ok_or_else(|| {
        tauri::Error::InvalidIcon(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "缺少默认图标，请检查 bundle.icon 配置",
        ))
    })?;

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        // 悬停提示文本用"显示名"title（即 TITLE，CI 里由配置 JSON 的 title 导出），
        // 而非"安装名"APP_NAME，与窗口标题保持一致。
        .tooltip(TITLE)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                show_window(app);
            }
            // 托盘右键菜单新增的"设置"项：打开（或聚焦）设置窗口，
            // 与右键网页菜单的"设置"、全局快捷键 Shift+F1 走同一入口。
            "settings" => {
                crate::settings::open_settings_window(app);
            }
            // 悬浮窗显示 / 隐藏切换（仅 Windows 有此菜单项）。
            #[cfg(windows)]
            "float" => {
                crate::floatwin::toggle_float_window(app);
            }
            "hide" => {
                if let Some(w) = app.get_webview_window(main_window_label()) {
                    let _ = w.hide();
                }
                sync_tabs_visibility(app, false);
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // 左键单击托盘图标 = 显示/隐藏
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}
