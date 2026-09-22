use std::fs;
use std::path::PathBuf;
#[cfg(desktop)]
use std::str::FromStr;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
#[cfg(desktop)]
use tauri::{WebviewUrl, WebviewWindowBuilder};
#[cfg(desktop)]
use tauri_plugin_global_shortcut::GlobalShortcutExt;
#[cfg(desktop)]
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

use crate::config::config_dir;
// ⚠️ TITLE 只被下方 `#[cfg(desktop)]` 的 open_settings_window 使用（拼设置窗标题）。
// 若与 config_dir 合并成一条无条件 use，Android 目标下该导入即「未使用」，
// 在 `-D warnings`（CI 的 check-android → clippy）下直接编译失败。必须按 cfg 拆开。
#[cfg(desktop)]
use crate::config::TITLE;

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
#[cfg(desktop)]
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
#[cfg(desktop)]
pub fn settings_open_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::SHIFT), Code::F1)
}

/// `settings.json` 的落点（`pub(crate)`：启动日志会把它打出来 —— 「我明明勾了托盘，
/// 重启又没了」这类反馈只能靠这一行看出实际读的是哪个文件）。
pub(crate) fn settings_path(app: &AppHandle) -> Option<PathBuf> {
    // 目录统一走 config::config_dir：桌面端取 app_config_dir 父目录、Android 用自身 dataDir，
    // 再拼自定义 app_name（平台差异见该函数注释）。
    Some(config_dir(app)?.join(SETTINGS_FILE))
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
#[cfg(desktop)]
pub fn open_settings_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        return;
    }
    let url = url::Url::parse("settings://localhost/").expect("构造 settings 协议 URL 失败");
    let _ = WebviewWindowBuilder::new(app, "settings", WebviewUrl::CustomProtocol(url))
        // 标题用【显示名】TITLE 而不是【安装名】APP_NAME —— 这个标题是给用户看的。
        .title(format!("设置 - {}", TITLE))
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
/// 桌面专属逻辑已用 `#[cfg(desktop)]` 剔除，移动端只做持久化。
#[tauri::command]
pub(crate) fn save_settings(
    app: tauri::AppHandle,
    state: State<SettingsState>,
    new_settings: Settings,
) -> Result<(), String> {
    // 1)+2) 桌面端：唤醒快捷键可留空（表示“不设置快捷键”）；若填写则必须可被解析，
    // 且不能与“打开设置(Shift+F1)”冲突。Android 无全局快捷键，跳过此校验
    // （也不引用 tauri_plugin_global_shortcut 的类型）。
    #[cfg(desktop)]
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

    // 1) 锁内一次性克隆旧设置，随即释放锁（临界区极小，只有一次 clone）。
    //    `old` 只用于桌面端的快捷键 / 托盘 diff；Android 无这两项，故一并隔离，避免 unused 警告。
    #[cfg(desktop)]
    let old = state.0.lock().unwrap_or_else(|p| p.into_inner()).clone();

    // 2) 桌面端：应用运行时的快捷键 / 托盘变更（锁外执行——全局快捷键注册、
    //    托盘创建/销毁可能涉及系统调用，不应在持锁期间做）。
    #[cfg(desktop)]
    {
        let old_sc = hotkey_to_shortcut(&old.wake_hotkey);
        if new_sc != old_sc {
            if let Some(os) = old_sc {
                let _ = app.global_shortcut().unregister(os);
            }
            if let Some(ns) = new_sc {
                if let Err(e) = app.global_shortcut().register(ns) {
                    // 新快捷键注册失败（已被其它程序占用等）：回滚到旧快捷键，
                    // 避免“旧的已注销、新的没注册上”的空窗——应用从此失去唤醒能力。
                    if let Some(os) = old_sc {
                        let _ = app.global_shortcut().register(os);
                    }
                    return Err(format!("快捷键注册失败（可能已被其它程序占用）：{e}"));
                }
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

    // 3) 持久化（锁外执行：磁盘 I/O 是最耗时的一步，不应阻塞其它读设置的线程）。
    persist_settings(&app, &new_settings).map_err(|e| e.to_string())?;

    // 4) 重新加锁写入新设置（临界区极小）。副作用 / 持久化任一失败时走 `?` 提前返回，
    //    与旧行为一致：不写内存态，下次 get_settings 仍是旧值。
    let mut guard = state.0.lock().unwrap_or_else(|p| p.into_inner());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_serde_roundtrip() {
        let h = Hotkey {
            ctrl: false,
            alt: true,
            shift: false,
            key: "Digit4".into(),
        };
        let json = serde_json::to_string(&h).unwrap();
        let back: Hotkey = serde_json::from_str(&json).unwrap();
        assert_eq!(back.ctrl, h.ctrl);
        assert_eq!(back.alt, h.alt);
        assert_eq!(back.shift, h.shift);
        assert_eq!(back.key, h.key);
    }

    #[test]
    fn settings_serde_roundtrip_preserves_flags() {
        let s = Settings {
            wake_hotkey: Hotkey {
                ctrl: true,
                alt: false,
                shift: false,
                key: "KeyA".into(),
            },
            tray_enabled: true,
            minimize_on_close: true,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.wake_hotkey.key, "KeyA");
        assert!(back.wake_hotkey.ctrl);
        assert!(back.tray_enabled);
        assert!(back.minimize_on_close);
    }

    /// ⚠️⚠️ **双壳互通契约**（electron-app/src/main/settings.ts）。
    ///
    /// 下面的字面量就是 Electron 壳 `saveSettings()` 写出来的**字节形态**
    /// （`JSON.stringify(s, null, 2)`，2 空格缩进）。两个壳共用同一个
    /// `%APPDATA%\<app_name>\settings.json`，所以这里一旦解析失败，
    /// 表现是「在 Electron 版里勾了托盘，回到 Tauri 版又变回默认」——
    /// 悄无声息，极难定位。故用字面量把字段名 / 嵌套结构 / 类型钉死。
    ///
    /// 反向（Tauri 写、Electron 读）由 Electron 侧的探针断言
    /// 「序列化输出为规范 JSON」守着，两边合起来才是完整的互通保证。
    #[test]
    fn settings_parse_accepts_electron_written_json() {
        let json = r#"{
  "wake_hotkey": {
    "ctrl": false,
    "alt": true,
    "shift": false,
    "key": "Digit4"
  },
  "tray_enabled": true,
  "minimize_on_close": false
}"#;
        let s: Settings =
            serde_json::from_str(json).expect("必须能读 Electron 壳写的 settings.json");
        assert_eq!(s.wake_hotkey.key, "Digit4");
        assert!(s.wake_hotkey.alt);
        assert!(!s.wake_hotkey.ctrl);
        assert!(!s.wake_hotkey.shift);
        assert!(s.tray_enabled);
        assert!(!s.minimize_on_close);
    }

    /// 反方向：Tauri 写出来的两空格缩进 JSON，必须能被 Electron 侧的结构解析接受。
    /// （Electron 侧用的是宽松解析，这里只保证「我们写的是规范 JSON」这个前提成立。）
    #[test]
    fn settings_pretty_output_is_canonical_json() {
        let s = Settings {
            wake_hotkey: Hotkey {
                ctrl: false,
                alt: true,
                shift: false,
                key: "Digit4".into(),
            },
            tray_enabled: true,
            minimize_on_close: false,
        };
        let json = serde_json::to_string_pretty(&s).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["wake_hotkey"]["key"], "Digit4");
        assert_eq!(v["tray_enabled"], serde_json::Value::Bool(true));
        assert_eq!(v["minimize_on_close"], serde_json::Value::Bool(false));
        // 布尔必须是 JSON 布尔，不能是字符串 —— Electron 侧的宽容解析能接受字符串，
        // 但反过来若我们写了字符串，就说明序列化路径被改坏了。
        assert!(v["tray_enabled"].is_boolean());
    }

    #[cfg(desktop)]
    #[test]
    fn hotkey_to_shortcut_valid_key() {
        let h = Hotkey {
            ctrl: false,
            alt: true,
            shift: false,
            key: "Digit4".into(),
        };
        let sc = hotkey_to_shortcut(&h).expect("合法按键应能转换");
        // 该版本 global-shortcut 的 Display 输出修饰键为小写（"alt+Digit4"）。
        assert_eq!(sc.to_string(), "alt+Digit4");
    }

    #[cfg(desktop)]
    #[test]
    fn hotkey_to_shortcut_empty_or_invalid_returns_none() {
        let empty = Hotkey {
            ctrl: false,
            alt: false,
            shift: false,
            key: String::new(),
        };
        assert!(hotkey_to_shortcut(&empty).is_none(), "空 key 表示未启用");
        let bogus = Hotkey {
            ctrl: false,
            alt: false,
            shift: false,
            key: "NotARealKey".into(),
        };
        assert!(
            hotkey_to_shortcut(&bogus).is_none(),
            "无法解析的键名应返回 None"
        );
    }
}
