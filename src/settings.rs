use std::fs;
use std::path::PathBuf;
#[cfg(not(target_os = "android"))]
use std::str::FromStr;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
#[cfg(not(target_os = "android"))]
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

use crate::APP_NAME;

const SETTINGS_FILE: &str = "settings.json";

/// 应用设置（持久化到 JSON）。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settings {
    /// 唤醒快捷键（显示 / 隐藏主窗口）。
    pub wake_hotkey: Hotkey,
    /// 是否启用系统托盘图标。
    pub tray_enabled: bool,
    /// 关闭窗口时是否最小化到托盘（需同时启用托盘图标，否则关闭即退出）。
    pub minimize_on_close: bool,
}

/// 唤醒快捷键结构。key 为 JS KeyboardEvent.code（如 "Digit4" / "F1" / "KeyA"），
/// 可与 Code::from_str 互转；修饰键由三个布尔开关表示。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            wake_hotkey: Hotkey {
                ctrl: false,
                alt: true,
                shift: false,
                key: "Digit4".to_string(), // 默认 Alt+4
            },
            tray_enabled: true,
            minimize_on_close: true,
        }
    }
}

/// 托管状态：运行时当前生效的设置。
pub struct SettingsState(pub Mutex<Settings>);

impl Default for SettingsState {
    fn default() -> Self {
        SettingsState(Mutex::new(Settings::default()))
    }
}

/// 把 Hotkey 结构转换为 Shortcut（用于注册全局快捷键）。
/// key 必须是可被 Code::from_str 解析的合法键名（JS event.code 即符合）。
#[cfg(not(target_os = "android"))]
pub fn hotkey_to_shortcut(h: &Hotkey) -> Option<Shortcut> {
    let code = Code::from_str(&h.key).ok()?;
    let mut mods = Modifiers::empty();
    if h.ctrl {
        mods |= Modifiers::CONTROL;
    }
    if h.alt {
        mods |= Modifiers::ALT;
    }
    if h.shift {
        mods |= Modifiers::SHIFT;
    }
    Some(Shortcut::new(Some(mods), code))
}

/// 打开设置界面的固定快捷键：Shift+F1（不可在设置里修改）。
#[cfg(not(target_os = "android"))]
pub fn settings_open_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::SHIFT), Code::F1)
}

fn settings_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|p| p.join(SETTINGS_FILE))
}

/// 读取设置；文件不存在或解析失败时使用默认值。
pub fn load_settings(app: &AppHandle) -> Settings {
    if let Some(path) = settings_path(app) {
        if let Ok(s) = fs::read_to_string(&path) {
            if let Ok(s) = serde_json::from_str::<Settings>(&s) {
                return s;
            }
        }
    }
    Settings::default()
}

/// 写入设置到 app config 目录下的 settings.json。
pub fn save_settings(app: &AppHandle, s: &Settings) -> Result<(), String> {
    let path = settings_path(app).ok_or_else(|| "无法获取配置目录".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}

/// 打开（或聚焦已存在的）设置窗口。仅桌面端使用。
/// 设置页通过 `settings://localhost/` 自定义协议提供（协议在 Builder 上注册，
/// HTML 由 Rust 侧编译期内嵌），因此不依赖 devUrl，dev / release 行为一致。
#[cfg(not(target_os = "android"))]
pub fn open_settings_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        return;
    }
    let url = url::Url::parse("settings://localhost/").expect("构造 settings 协议 URL 失败");
    let _ = WebviewWindowBuilder::new(app, "settings", WebviewUrl::CustomProtocol(url))
        .title(format!("设置 - {}", APP_NAME))
        .inner_size(480.0, 470.0)
        .resizable(false)
        .position(420.0, 180.0)
        .build();
}
