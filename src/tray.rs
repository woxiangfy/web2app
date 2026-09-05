//! 系统托盘与窗口切换（仅桌面端）。
//!
//! 注意：设置相关命令（get_settings / save_settings / close_settings_window）不在此处——
//! 它们是跨平台功能，已移入 [`crate::settings`] 无条件编译；本模块只保留纯桌面端的
//! 托盘与窗口切换逻辑。

use tauri::Manager;

use crate::config::WINDOW_TITLE;

/// 切换主窗口：正常可见则隐藏；最小化或已隐藏则恢复并显示置顶（仅桌面端）。
pub(crate) fn toggle_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let visible = window.is_visible().unwrap_or(false);
        let minimized = window.is_minimized().unwrap_or(false);
        if visible && !minimized {
            let _ = window.hide();
        } else {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

/// 创建系统托盘图标（含右键菜单、左键切换窗口）。仅当设置“启用托盘”时调用。
pub(crate) fn create_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let show_i = MenuItem::with_id(app, "show", "显示窗口", true, None::<&str>)?;
    let settings_i = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let hide_i = MenuItem::with_id(app, "hide", "隐藏到托盘", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_i, &settings_i, &hide_i, &quit_i])?;

    TrayIconBuilder::with_id("main-tray")
        .icon(
            app.default_window_icon()
                .cloned()
                .expect("缺少默认图标，请检查 bundle.icon 配置"),
        )
        .menu(&menu)
        .show_menu_on_left_click(false)
        // 悬停提示文本用"显示名"title（即 WINDOW_TITLE，CI 里由配置 JSON 的 title 导出），
        // 而非"安装名"APP_NAME，与窗口标题保持一致。
        .tooltip(WINDOW_TITLE)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.unminimize();
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            // 托盘右键菜单新增的"设置"项：打开（或聚焦）设置窗口，
            // 与右键网页菜单的"设置"、全局快捷键 Shift+F1 走同一入口。
            "settings" => {
                crate::settings::open_settings_window(app);
            }
            "hide" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
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
