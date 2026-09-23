//! 运行期配置文档：`<配置目录>/settings.json` —— **全应用唯一的一份持久化 JSON**。
//!
//! 形态：设置字段**平铺在顶层**（与旧 `settings.json` 逐字节兼容，老文件直接可读），
//! 窗口几何整体放在 `window` 段：
//!
//! ```json
//! {
//!   "wake_hotkey": { "ctrl": false, "alt": true, "shift": false, "key": "Digit4" },
//!   "tray_enabled": true,
//!   "minimize_on_close": false,
//!   "window": { "x": 100, "y": 200, "width": 1100, "height": 650, "maximized": false }
//! }
//! ```
//!
//! **2026-09-23 合并**：此前「设置」与「窗口几何」各占一个文件（`settings.json` +
//! `window-state.json`），用户打开数据目录看到两个 JSON。现在只剩这一个，
//! Electron 壳同契约（`electron-app/src/main/settings.ts`）。
//! ⚠️ 旧 `window-state.json` **不再读也不再写**（已确认不做迁移）：升级后窗口会回一次
//!    默认尺寸/位置，那个文件就此成为死文件，可手工删除。
//!
//! ⚠️⚠️ 合并后的唯一真风险是**两段互相覆盖**：写入方有两个（设置页保存 vs 窗口关闭时记几何），
//! 从前各写各的文件天然隔离，现在共用一个文件。所以本模块对外**只提供 [`update`]**
//! （以及配套的 [`load`]）：读盘 → 只替换自己那一段 → 写回。任何「拿内存里的整份文档
//! 直接 `fs::write`」的写法都会在「挪了下窗口 + 改了设置」同时发生时静默吃掉其中一方。
//! （`electron-app/src/main/window-state.ts` 的文件头正是用这个理由论证过「不该合并」；
//! 既然合并了，就用读-改-写把风险挡掉，并由本模块的单测钉住。）
//!
//! ⚠️ 第二个风险是**一段坏掉拖垮另一段**：所以两段各自独立解析 —— `window` 段坏只等于
//! 「这次没有几何可恢复」，绝不会把用户的三个开关一起打回默认值。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::AppHandle;

use crate::settings::Settings;

/// 唯一的运行期配置文件（名字保留 `settings.json`：老设置文件路径不变，零迁移即可读）。
pub const FILE: &str = "settings.json";

/// 窗口几何快照。x/y = 窗口左上角在屏幕上的坐标（outer），width/height = 外框尺寸（outer），
/// 均为【物理像素】，以保证保存与 `set_position`/`set_size` 一致、跨会话不因 DPI 缩放而错位。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowGeom {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// 旧版配置文件可能没有该字段：缺省按 false 解析（窗口不被误判为最大化）。
    #[serde(default)]
    pub maximized: bool,
}

/// 完整文档：设置 + 窗口几何。
#[derive(Clone, Debug, Default)]
pub struct Document {
    pub settings: Settings,
    pub window: Option<WindowGeom>,
}

/// 写盘用的视图：`flatten` 让设置字段平铺在顶层，键序由字段声明顺序决定
/// （设置三键 → `window`），与 Electron 侧的规范化输出一致，两个壳写出来的文件长得一样。
#[derive(Serialize)]
struct DocOut<'a> {
    #[serde(flatten)]
    settings: &'a Settings,
    #[serde(skip_serializing_if = "Option::is_none")]
    window: Option<&'a WindowGeom>,
}

/// 文档落点：目录统一走 `crate::config::config_dir`（桌面端取 app_config_dir 父目录、
/// Android 用自身 dataDir，再拼自定义 app_name），与设置、窗口几何**同一个文件**。
pub fn path(app: &AppHandle) -> Option<PathBuf> {
    Some(crate::config::config_dir(app)?.join(FILE))
}

/// 读取整份文档；文件不存在或顶层 JSON 损坏时返回默认值（不抛错）。
pub fn load(app: &AppHandle) -> Document {
    match path(app) {
        Some(p) => load_at(&p),
        None => Document::default(),
    }
}

/// 写入整份文档。⚠️ 调用方几乎总是想要 [`update`] 而不是这个 —— 直接用它会整体覆盖另一段。
pub fn save(app: &AppHandle, doc: &Document) -> Result<(), String> {
    let p = path(app).ok_or_else(|| "无法获取配置目录".to_string())?;
    save_at(&p, doc)
}

/// **读-改-写**：读盘 → 交给 `f` 改（只该动自己那一段）→ 写回。
///
/// 设置页保存与窗口几何记录**都必须**走这里，这样「挪窗口」和「改设置」互不覆盖。
pub fn update(app: &AppHandle, f: impl FnOnce(&mut Document)) -> Result<(), String> {
    let p = path(app).ok_or_else(|| "无法获取配置目录".to_string())?;
    update_at(&p, f)
}

// ---------- 纯路径版本（单测直接打真文件，不必构造 AppHandle） ----------

fn load_at(p: &Path) -> Document {
    let text = match fs::read_to_string(p) {
        Ok(t) => t,
        // 文件不存在 = 首次启动还没写过，正常，忽略。
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Document::default(),
        Err(e) => {
            dlog!("[store] 读取失败，已回退默认值（{e}）：{}", p.display());
            return Document::default();
        }
    };
    match parse_document(&text) {
        Ok(doc) => doc,
        Err(e) => {
            dlog!("[store] 解析失败，已回退默认值（{e}）：{}", p.display());
            Document::default()
        }
    }
}

fn save_at(p: &Path, doc: &Document) -> Result<(), String> {
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serialize_document(doc)?;
    fs::write(p, json).map_err(|e| e.to_string())
}

fn update_at(p: &Path, f: impl FnOnce(&mut Document)) -> Result<(), String> {
    // 必须是「先读再改」：读到的是磁盘上别人的最新改动，`f` 只替换自己那一段。
    let mut doc = load_at(p);
    f(&mut doc);
    save_at(p, &doc)
}

/// 解析文档。`Err` = 顶层 JSON 坏了（整份不可用，调用方回退默认值）；
/// 单段坏只影响那一段（见模块头「一段坏掉拖垮另一段」）。
fn parse_document(text: &str) -> Result<Document, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;

    // 设置段：整份对象交给 Settings（多余的 `window` 键会被忽略）。
    let settings = match serde_json::from_value::<Settings>(v.clone()) {
        Ok(s) => s,
        Err(e) => {
            dlog!("[store] 设置段解析失败，三个开关回默认值（{e}）；窗口几何不受影响");
            Settings::default()
        }
    };

    // 窗口几何段：缺省 / null 都表示「没有几何可恢复」。
    let window = match v.get("window") {
        None | Some(serde_json::Value::Null) => None,
        Some(w) => match serde_json::from_value::<WindowGeom>(w.clone()) {
            Ok(g) => Some(g),
            Err(e) => {
                dlog!("[store] window 段解析失败，本次不恢复窗口几何（{e}）；设置不受影响");
                None
            }
        },
    };

    Ok(Document { settings, window })
}

/// 序列化为两空格缩进的规范 JSON（与 `serde_json::to_string_pretty` 的 Electron 侧对齐）。
fn serialize_document(doc: &Document) -> Result<String, String> {
    let out = DocOut {
        settings: &doc.settings,
        window: doc.window.as_ref(),
    };
    serde_json::to_string_pretty(&out).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Hotkey;

    fn settings_a() -> Settings {
        Settings {
            wake_hotkey: Hotkey {
                ctrl: false,
                alt: true,
                shift: false,
                key: "Digit4".into(),
            },
            tray_enabled: true,
            minimize_on_close: false,
        }
    }

    fn settings_b() -> Settings {
        Settings {
            wake_hotkey: Hotkey {
                ctrl: true,
                alt: false,
                shift: true,
                key: "F9".into(),
            },
            tray_enabled: false,
            minimize_on_close: true,
        }
    }

    fn geom() -> WindowGeom {
        WindowGeom {
            x: 455,
            y: 179,
            width: 1657,
            height: 986,
            maximized: false,
        }
    }

    /// 每个用例一个独立的临时文件（路径结构与线上一致：`<dir>/settings.json`）。
    fn temp_file(tag: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("w2a-store-test-{}-{tag}", std::process::id()));
        dir.join(FILE)
    }

    fn with_temp<F: FnOnce(&Path)>(tag: &str, f: F) {
        let p = temp_file(tag);
        let _ = fs::remove_dir_all(p.parent().unwrap());
        f(&p);
        let _ = fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn document_roundtrip_keeps_both_sections() {
        with_temp("roundtrip", |p| {
            let doc = Document {
                settings: settings_a(),
                window: Some(geom()),
            };
            save_at(p, &doc).unwrap();
            let back = load_at(p);
            assert_eq!(back.settings.wake_hotkey.key, "Digit4");
            assert!(back.settings.tray_enabled);
            assert!(!back.settings.minimize_on_close);
            assert_eq!(back.window, Some(geom()));
        });
    }

    /// ⚠️⚠️ 合并后的核心回归守卫：设置页保存**不得**吃掉窗口几何。
    #[test]
    fn writing_settings_keeps_window_geometry_on_disk() {
        with_temp("settings-keeps-window", |p| {
            save_at(
                p,
                &Document {
                    settings: settings_a(),
                    window: Some(geom()),
                },
            )
            .unwrap();

            update_at(p, |d| d.settings = settings_b()).unwrap();

            let back = load_at(p);
            assert_eq!(back.settings.wake_hotkey.key, "F9", "新设置没写进去");
            assert_eq!(
                back.window,
                Some(geom()),
                "写设置把窗口几何吃掉了（读-改-写被绕过？）"
            );
        });
    }

    /// ⚠️⚠️ 反向：记窗口几何**不得**吃掉三个开关。
    #[test]
    fn writing_window_keeps_settings_on_disk() {
        with_temp("window-keeps-settings", |p| {
            save_at(
                p,
                &Document {
                    settings: settings_a(),
                    window: None,
                },
            )
            .unwrap();

            let moved = WindowGeom {
                x: 10,
                y: 20,
                ..geom()
            };
            update_at(p, |d| d.window = Some(moved)).unwrap();

            let back = load_at(p);
            assert_eq!(back.window, Some(moved));
            assert_eq!(
                back.settings.wake_hotkey.key, "Digit4",
                "记几何把设置吃掉了（读-改-写被绕过？）"
            );
            assert!(back.settings.tray_enabled);
        });
    }

    /// 真实的「文件不存在」首次运行路径：读回默认值，而不是报错。
    #[test]
    fn missing_file_yields_defaults() {
        with_temp("missing", |p| {
            let doc = load_at(p);
            assert!(!doc.settings.tray_enabled);
            assert_eq!(doc.settings.wake_hotkey.key, "");
            assert_eq!(doc.window, None);
        });
    }

    /// 旧版 `settings.json`（只有三个设置键，没有 `window`）必须能原样读出来 ——
    /// 「沿用 settings.json」的前提就是这个。
    #[test]
    fn legacy_settings_only_file_still_parses() {
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
        let doc = parse_document(json).expect("旧版 settings.json 必须能解析");
        assert_eq!(doc.settings.wake_hotkey.key, "Digit4");
        assert!(doc.settings.tray_enabled);
        assert_eq!(doc.window, None);
    }

    /// ⚠️⚠️ **双壳互通契约**（electron-app/src/main/settings.ts）。
    ///
    /// 下面的字面量就是 Electron 壳 `writeDocument()` 写出来的**字节形态**
    /// （`JSON.stringify(doc, null, 2)`：设置三键在前、`window` 在后）。两个壳共用
    /// 同一个 `%APPDATA%\<app_name>\settings.json`，这里解析不了就意味着
    /// 「在 Electron 版里改了设置/挪了窗口，回到 Tauri 版变回默认」——悄无声息。
    #[test]
    fn parse_accepts_electron_written_document() {
        let json = r#"{
  "wake_hotkey": {
    "ctrl": false,
    "alt": true,
    "shift": false,
    "key": "Digit4"
  },
  "tray_enabled": true,
  "minimize_on_close": false,
  "window": {
    "x": 455,
    "y": 179,
    "width": 1657,
    "height": 986,
    "maximized": false
  }
}"#;
        let doc = parse_document(json).expect("必须能读 Electron 壳写的 settings.json");
        assert_eq!(doc.settings.wake_hotkey.key, "Digit4");
        assert!(doc.settings.wake_hotkey.alt);
        assert!(doc.settings.tray_enabled);
        assert_eq!(doc.window, Some(geom()));
    }

    /// 反方向：我们写出来的字节形态必须能被 Electron 侧解析 —— 键序、缩进、布尔类型都钉死。
    #[test]
    fn pretty_output_is_canonical_json() {
        let doc = Document {
            settings: settings_a(),
            window: Some(geom()),
        };
        let json = serialize_document(&doc).unwrap();
        assert_eq!(
            json,
            r#"{
  "wake_hotkey": {
    "ctrl": false,
    "alt": true,
    "shift": false,
    "key": "Digit4"
  },
  "tray_enabled": true,
  "minimize_on_close": false,
  "window": {
    "x": 455,
    "y": 179,
    "width": 1657,
    "height": 986,
    "maximized": false
  }
}"#,
            "写出的字节形态变了：Electron 侧的解析与探针都按这个字面量对齐"
        );

        // 无几何时不该出现 `window` 键（首次启动的文件里没有这一段）。
        let no_geom = Document {
            settings: settings_a(),
            window: None,
        };
        let json = serialize_document(&no_geom).unwrap();
        assert!(
            !json.contains("\"window\""),
            "没有几何时不该写 window 段：{json}"
        );

        // 布尔必须是 JSON 布尔，不能是字符串。
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v["tray_enabled"].is_boolean());
    }

    /// 一段坏掉不该拖垮另一段：`window` 段是垃圾时，三个开关照常读出。
    #[test]
    fn corrupt_window_section_keeps_settings() {
        let json = r#"{
  "wake_hotkey": { "ctrl": false, "alt": true, "shift": false, "key": "Digit4" },
  "tray_enabled": true,
  "minimize_on_close": false,
  "window": "这不是对象"
}"#;
        let doc = parse_document(json).expect("顶层 JSON 是好的，就不该整份失败");
        assert!(doc.settings.tray_enabled, "window 段坏了把设置也带崩了");
        assert_eq!(doc.window, None);
    }

    /// 顶层 JSON 损坏 → 整份回默认值（与旧 `load_settings` 的行为一致，不抛错）。
    #[test]
    fn broken_top_level_json_falls_back_to_defaults() {
        assert!(parse_document("{ 不是 JSON").is_err());
        with_temp("broken", |p| {
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, "{ 不是 JSON").unwrap();
            let doc = load_at(p);
            assert!(!doc.settings.tray_enabled);
            assert_eq!(doc.window, None);
        });
    }
}
