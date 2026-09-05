//! 系统托盘、窗口切换与设置命令（仅桌面端）。

use tauri::{Manager, State};

#[cfg(not(target_os = "android"))]
use tauri_plugin_global_shortcut::GlobalShortcutExt;

use crate::config::WINDOW_TITLE;
use crate::settings::{Settings, SettingsState};

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

#[tauri::command]
pub(crate) fn get_settings(state: State<SettingsState>) -> Settings {
    state.0.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

#[tauri::command]
pub(crate) fn save_settings(
    app: tauri::AppHandle,
    state: State<SettingsState>,
    new_settings: Settings,
) -> Result<(), String> {
    // 1)+2) 桌面端：唤醒快捷键可留空（表示“不设置快捷键”）；若填写则必须可被解析，
    // 且不能与“打开设置(Shift+F1)”冲突。Android 无全局快捷键，跳过此校验
    // （也不引用 tauri_plugin_global_shortcut 的类型）。
    #[cfg(not(target_os = "android"))]
    let new_sc = {
        let key = new_settings.wake_hotkey.key.trim();
        if key.is_empty() {
            // 空 key = 未启用唤醒快捷键
            None
        } else {
            let sc = crate::settings::hotkey_to_shortcut(&new_settings.wake_hotkey).ok_or_else(|| {
                format!(
                    "不支持的按键：{}，请换一个按键",
                    new_settings.wake_hotkey.key
                )
            })?;
            if sc == crate::settings::settings_open_shortcut() {
                return Err("唤醒快捷键不能与“打开设置(Shift+F1)”冲突，请换一个".into());
            }
            Some(sc)
        }
    };

    let mut guard = state.0.lock().unwrap_or_else(|p| p.into_inner());
    let old = guard.clone();

    // 3) 桌面端：应用运行时的快捷键 / 托盘变更
    #[cfg(not(target_os = "android"))]
    {
        let old_sc = crate::settings::hotkey_to_shortcut(&old.wake_hotkey);
        if new_sc != old_sc {
            if let Some(os) = old_sc {
                let _ = app.global_shortcut().unregister(os);
            }
            if let Some(ns) = new_sc {
                app.global_shortcut()
                    .register(ns)
                    .map_err(|e| e.to_string())?;
            }
        }
        if new_settings.tray_enabled != old.tray_enabled {
            if new_settings.tray_enabled {
                create_tray(&app).map_err(|e| e.to_string())?;
            } else {
                app.remove_tray_by_id("main-tray");
            }
        }
    }

    // 4) 持久化 + 更新托管状态
    crate::settings::save_settings(&app, &new_settings).map_err(|e| e.to_string())?;
    *guard = new_settings;
    Ok(())
}

#[tauri::command]
pub(crate) fn close_settings_window(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.close();
    }
}
