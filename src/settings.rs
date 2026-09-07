use std::fs;
use std::path::PathBuf;
#[cfg(not(target_os = "android"))]
use std::str::FromStr;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
#[cfg(not(target_os = "android"))]
use tauri::{WebviewUrl, WebviewWindowBuilder};
#[cfg(not(target_os = "android"))]
use tauri_plugin_global_shortcut::GlobalShortcutExt;
#[cfg(not(target_os = "android"))]
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

use crate::config::{sanitize_dir_name, APP_NAME};

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
            // 默认【不设置】唤醒快捷键：key 为空字符串表示“未启用”。
            // 用户可在设置页自行捕获一个组合键，保存后立即生效。
            wake_hotkey: Hotkey {
                ctrl: false,
                alt: false,
                shift: false,
                key: String::new(),
            },
            // 默认【不启用】系统托盘。
            tray_enabled: false,
            // 默认【关闭窗口即退出程序】，而非最小化到托盘（无托盘时本也无从最小化）。
            minimize_on_close: false,
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
    // Tauri 默认按 tauri.conf.json 的 identifier（com.example.web2app）生成目录末段，
    // 这里改用清洗后的自定义 app_name，与 WebView2 数据目录保持一致（见 lib.rs）。
    let base = app.path().app_config_dir().ok()?;

    // 桌面端：app_config_dir 形如
    //   Windows  %APPDATA%\<identifier>
    //   Linux    ~/.config/<identifier>
    //   macOS    ~/Library/Application Support/<identifier>
    // 末段是 identifier，取父目录后再拼上自定义 app_name，让不同 app 的配置互相隔离。
    #[cfg(not(target_os = "android"))]
    let root = base.parent().unwrap_or(&base).to_path_buf();

    // Android：Tauri 移动端的 app_config_dir 【不会】再拼 identifier，它直接返回
    // 本应用私有根目录 /data/data/<package>
    // （tauri/src/path/android.rs 的 app_config_dir → PathPlugin.kt 的 getConfigDir
    //  = activity.dataDir）。所以这里【绝不能取 parent】——那会跳到 /data/data，
    // 既不是本应用可写目录（selinux/权限拒绝），又会和其它包重名冲突。
    // 正确做法是直接在自身私有目录内建子目录：<dataDir>/<app_name>/settings.json
    #[cfg(target_os = "android")]
    let root = base;

    let dir = root.join(sanitize_dir_name(APP_NAME));
    Some(dir.join(SETTINGS_FILE))
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

/// 写入设置到 app config 目录下的 settings.json（纯持久化，不含运行时副作用）。
pub fn persist_settings(app: &AppHandle, s: &Settings) -> Result<(), String> {
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

/// 读取当前设置（供设置页 / 前端调用）。
#[tauri::command]
pub(crate) fn get_settings(state: State<SettingsState>) -> Settings {
    state.0.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// 保存设置：校验 + 应用运行时变更（快捷键/托盘）+ 持久化。
///
/// 注意：此命令在 Android 端同样注册（设置功能跨平台），但全局快捷键 / 托盘等
/// 桌面专属逻辑已用 `#[cfg(not(target_os = "android"))]` 剔除，移动端只做持久化。
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
            let sc = hotkey_to_shortcut(&new_settings.wake_hotkey).ok_or_else(|| {
                format!(
                    "不支持的按键：{}，请换一个按键",
                    new_settings.wake_hotkey.key
                )
            })?;
            if sc == settings_open_shortcut() {
                return Err("唤醒快捷键不能与“打开设置(Shift+F1)”冲突，请换一个".into());
            }
            Some(sc)
        }
    };

    let mut guard = state.0.lock().unwrap_or_else(|p| p.into_inner());
    // `old` 只用于桌面端的快捷键 / 托盘 diff；Android 无这两项，故一并隔离，避免 unused 警告。
    #[cfg(not(target_os = "android"))]
    let old = guard.clone();

    // 3) 桌面端：应用运行时的快捷键 / 托盘变更
    #[cfg(not(target_os = "android"))]
    {
        let old_sc = hotkey_to_shortcut(&old.wake_hotkey);
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
                crate::tray::create_tray(&app).map_err(|e| e.to_string())?;
            } else {
                app.remove_tray_by_id("main-tray");
            }
        }
    }

    // 4) 持久化 + 更新托管状态
    persist_settings(&app, &new_settings).map_err(|e| e.to_string())?;
    *guard = new_settings;
    Ok(())
}

/// 关闭设置窗口。
#[tauri::command]
pub(crate) fn close_settings_window(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.close();
    }
}
