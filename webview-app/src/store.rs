//! 运行期配置文档：`<数据目录>/settings.json` —— **全应用唯一的一份持久化 JSON**。
//!
//! 形态：设置字段**平铺在顶层**（与旧 `settings.json` 逐字节兼容，老文件直接可读），
//! 窗口几何整体放在 `window` 段、悬浮球几何放在 `float_window` 段：
//!
//! ```json
//! {
//!   "wake_hotkey": { "ctrl": false, "alt": true, "shift": false, "key": "Digit4" },
//!   "tray_enabled": true,
//!   "minimize_on_close": false,
//!   "float_enabled": false,
//!   "window": { "x": 100, "y": 200, "width": 1100, "height": 650, "maximized": false },
//!   "float_window": { "x": 12, "y": 240, "width": 56, "height": 56, "edge": "left" }
//! }
//! ```
//!
//! **2026-09-23 合并**：此前「设置」与「窗口几何」各占一个文件（`settings.json` +
//! `window-state.json`），用户打开数据目录看到两个 JSON。现在只剩这一个，
//! Electron 壳同契约（`electron-app/src/main/settings.ts`）。
//! ⚠️ 旧 `window-state.json` **不再读也不再写**（已确认不做迁移）：升级后窗口会回一次
//!    默认尺寸/位置，那个文件就此成为死文件，可手工删除。
//!
//! **2026-09-23 落点改为数据目录**：此前落 `%APPDATA%\<app_name>`（"配置根"），
//! 与 `data_dir` 字段无关 —— 用户选择绿色版（`data_dir = "data"`）时，
//! cookie / 缓存 / 日志都跟着 exe 走，唯独这个 JSON 死活留在 `%APPDATA%`，是同一套
//! 「程序的数据」里唯一不听话的一项。现在**由 [`crate::config::store_dir`] 统一决定**：
//! 它就是 `data_dir`（Windows）/ 应用私有目录（Android），与 WebView2 用户数据目录**同一个**。
//! ⚠️ 旧位置（`%APPDATA%\<app_name>\settings.json`）只作**读取候选**（升级兼容，不迁移、
//!    不删除）：新位置还没写过时读它，第一次写盘就落到新位置，此后旧文件是死文件。
//!    读取候选由 [`candidates`] 统一给出，**读也用它、写只用主位置** —— 否则
//!    「用旧文件的值算出来的新文档」会写到新位置、而旧文件被绕过，用户看到的是设置被重置。
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
use std::io::Write;
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

/// 悬浮球当前吸附的屏幕边（仅 Windows 桌面端产生该段）。序列化为小写字符串
/// （`"none"` / `"left"` / `"right"` / `"top"` / `"bottom"`），与 Electron 侧
/// `float-window.ts` 逐字一致。
///
/// v1.81 起吸附从「左 / 右」扩为**四边**：左 / 右沿 x 半隐藏，上 / 下沿 y 半隐藏
/// （半隐藏的偏移量都是「球边长 − peek」）。旧版落盘文件里没有 `top` / `bottom`，
/// `#[serde(default)]` 缺省按 `none` 解析，向后兼容。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum DockEdge {
    /// 未吸附：窗口是自由摆放的普通矩形，`x/y` 即其左上角。
    #[default]
    None,
    /// 吸附左边（半隐藏时只在屏幕左缘露一条）。
    Left,
    /// 吸附右边。
    Right,
    /// 吸附上边（半隐藏时只在屏幕上缘露一条）。
    Top,
    /// 吸附下边。
    Bottom,
}

/// 悬浮球几何快照。语义同 [`WindowGeom`]（物理像素 outer 矩形），额外记一条吸附边：
/// `edge != none` 时 `x/y` 存的是【展开态】左上角，半隐藏的偏移由宽度 + 所在显示器边界
/// 在运行期算出，无需落盘。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FloatWinGeom {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// 旧版文件没有该字段：缺省按 none 解析（自由摆放）。
    #[serde(default)]
    pub edge: DockEdge,
}

/// 完整文档：设置 + 主窗口几何 + 悬浮球几何。
#[derive(Clone, Debug, Default)]
pub struct Document {
    pub settings: Settings,
    pub window: Option<WindowGeom>,
    pub float_window: Option<FloatWinGeom>,
}

/// 写盘用的视图：`flatten` 让设置字段平铺在顶层，键序由字段声明顺序决定
/// （设置四键 → `window` → `float_window`），与 Electron 侧的规范化输出一致，
/// 两个壳写出来的文件长得一样。
#[derive(Serialize)]
struct DocOut<'a> {
    #[serde(flatten)]
    settings: &'a Settings,
    #[serde(skip_serializing_if = "Option::is_none")]
    window: Option<&'a WindowGeom>,
    #[serde(skip_serializing_if = "Option::is_none")]
    float_window: Option<&'a FloatWinGeom>,
}

/// 文档落点：目录统一走 `crate::config::store_dir`（Windows = 数据目录，即 `data_dir`
/// 字段解析出来的那个目录，与 WebView2 用户数据目录**同一个**；Android = 应用私有配置目录）。
pub fn path(app: &AppHandle) -> Option<PathBuf> {
    Some(crate::config::store_dir(app)?.join(FILE))
}

/// 旧位置（2026-09-23 之前）：`%APPDATA%\<app_name>\settings.json`，**只读候选**。
///
/// Android 上新旧位置是同一个目录（`config_dir == store_dir`），[`candidates`] 会去重。
pub fn legacy_path(app: &AppHandle) -> Option<PathBuf> {
    Some(crate::config::config_dir(app)?.join(FILE))
}

/// 读取候选：**新位置在前、旧位置在后**（去重后返回）。
///
/// ⚠️ [`load`] 与 [`update`] 都走这一份候选 —— 读的位置与写的位置分开是有意的：
/// 读要能兼容升级前留下的文件，写必须只落在新位置（否则「读旧文件算出来的文档」
/// 会写到新位置，而旧文件被绕过 ⇒ 用户看到设置被重置）。
pub fn candidates(app: &AppHandle) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(p) = path(app) {
        out.push(p);
    }
    if let Some(p) = legacy_path(app) {
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// 读取整份文档：按 [`candidates`] 顺序取第一份**存在**的文件；都不存在（首次启动）
/// 或顶层 JSON 损坏时给默认值（不抛错）。
pub fn load(app: &AppHandle) -> Document {
    read_doc(&candidates(app))
}

// 注：曾经这里有一个 pub save（整份覆盖写），全仓无调用方且语义危险（会抹掉另一段），
// 已删除；写盘一律走 [`update`] 的读-改-写（save_at 仍由 update_with 复用）。

/// **读-改-写**：按 [`candidates`] 读盘 → 交给 `f` 改（只该动自己那一段）→ 写回新位置。
///
/// 设置页保存与窗口几何记录**都必须**走这里，这样「挪窗口」和「改设置」互不覆盖。
/// 升级兼容也在这里：新位置还没有文件时读到的是旧位置那份，第一次写盘即落到新位置。
pub fn update(app: &AppHandle, f: impl FnOnce(&mut Document)) -> Result<(), String> {
    let p = path(app).ok_or_else(|| "无法获取配置目录".to_string())?;
    let cands = candidates(app);
    update_with(&p, &cands, f)
}

// ---------- 纯路径版本（单测直接打真文件，不必构造 AppHandle） ----------

/// 按顺序读第一份**存在**的文件；都没有时默认值。
fn read_doc(cands: &[PathBuf]) -> Document {
    for (i, p) in cands.iter().enumerate() {
        if p.exists() {
            if i > 0 {
                dlog!(
                    "[store] 新位置没有 settings.json，改用旧位置（只读兼容）：{}",
                    p.display()
                );
            }
            return load_at(p);
        }
    }
    Document::default()
}

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
    // ⚠️⚠️ **原子替换**：直接覆盖写（`fs::write`）时，写一半进程被杀 / 断电会留下**截断的 JSON**
    // —— 下次启动整份解析失败 ⇒ 设置与窗口几何一起被回默认值（用户视角「设置全丢」，且**不报错**）。
    // 做法与 Electron 侧 `settings.ts::writeDocument` 一致：同目录临时文件 → flush → rename。
    // ⚠️ 临时文件必须与目标**同目录**：跨卷 rename 会退化成非原子的「复制 + 删除」。
    let tmp = atomic_tmp_path(p);
    let r = match write_tmp_and_sync(&tmp, &json) {
        Ok(()) => fs::rename(&tmp, p).map_err(|e| e.to_string()),
        Err(e) => Err(e),
    };
    if r.is_err() {
        // 失败就别把临时文件留在数据目录里（「目录里多了个怪文件」会变成新的排查入口）。
        let _ = fs::remove_file(&tmp);
    }
    r
}

/// 目标文件**同目录**下的临时文件名：`<目标>.tmp-<pid>`。
fn atomic_tmp_path(target: &Path) -> PathBuf {
    let mut s = target.as_os_str().to_owned();
    s.push(format!(".tmp-{}", std::process::id()));
    PathBuf::from(s)
}

/// 写临时文件并 `sync_all` 落盘：rename 之前内容必须真的在磁盘上，
/// 否则「rename 成功但数据在页缓存里」时断电照样丢。
fn write_tmp_and_sync(p: &Path, json: &str) -> Result<(), String> {
    let mut f = fs::File::create(p).map_err(|e| e.to_string())?;
    f.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())
}

/// 读-改-写的共用实现：`cands` = 读取候选（主位置在前），`write_to` = 落盘位置。
fn update_with(
    write_to: &Path,
    cands: &[PathBuf],
    f: impl FnOnce(&mut Document),
) -> Result<(), String> {
    // 必须是「先读再改」：读到的是磁盘上别人的最新改动，`f` 只替换自己那一段。
    let mut doc = read_doc(cands);
    f(&mut doc);
    save_at(write_to, &doc)
}

/// **仅供本模块单测**：直接打真文件，不必构造 `AppHandle`。
///
/// ⚠️ 为什么必须 `#[cfg(test)]`：生产代码里没有任何调用方（写盘一律走 [`update`]，
/// 它才带「读候选 → 写新位置」的升级兼容语义）。放在这里而不加 cfg 就是一个
/// **永不使用的生产函数** —— 只不过被 crate 根的 `#![allow(dead_code)]` 盖住了，
/// 谁也看不见。
#[cfg(test)]
fn update_at(p: &Path, f: impl FnOnce(&mut Document)) -> Result<(), String> {
    update_with(p, &[p.to_path_buf()], f)
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

    // 悬浮球几何段：与 window 段完全独立解析，坏了只等于「悬浮球回默认位置」。
    let float_window = match v.get("float_window") {
        None | Some(serde_json::Value::Null) => None,
        Some(f) => match serde_json::from_value::<FloatWinGeom>(f.clone()) {
            Ok(g) => Some(g),
            Err(e) => {
                dlog!("[store] float_window 段解析失败，本次悬浮球用默认位置（{e}）");
                None
            }
        },
    };

    Ok(Document {
        settings,
        window,
        float_window,
    })
}

/// 序列化为两空格缩进的规范 JSON（与 `serde_json::to_string_pretty` 的 Electron 侧对齐）。
fn serialize_document(doc: &Document) -> Result<String, String> {
    let out = DocOut {
        settings: &doc.settings,
        window: doc.window.as_ref(),
        float_window: doc.float_window.as_ref(),
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
            float_enabled: false,
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
            float_enabled: true,
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

    fn float_geom() -> FloatWinGeom {
        FloatWinGeom {
            x: 12,
            y: 240,
            width: 56,
            height: 56,
            edge: DockEdge::Left,
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
                float_window: Some(float_geom()),
            };
            save_at(p, &doc).unwrap();
            let back = load_at(p);
            assert_eq!(back.settings.wake_hotkey.key, "Digit4");
            assert!(back.settings.tray_enabled);
            assert!(!back.settings.minimize_on_close);
            assert_eq!(back.window, Some(geom()));
            assert_eq!(back.float_window, Some(float_geom()));
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
                    float_window: None,
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
                    float_window: None,
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
        // 旧版没有 float_enabled / float_window：前者由 serde default 兜成 false，后者缺省 None。
        assert!(!doc.settings.float_enabled);
        assert_eq!(doc.float_window, None);
    }

    /// ⚠️⚠️ **双壳互通契约**（electron-app/src/main/settings.ts）。
    ///
    /// 下面的字面量就是 Electron 壳 `writeDocument()` 写出来的**字节形态**
    /// （`JSON.stringify(doc, null, 2)`：设置三键在前、`window` 在后）。两个壳读写
    /// 同一份 `settings.json`（写入落点为各自 data_dir；读取时仍把旧位置
    /// `%APPDATA%\<app_name>\settings.json` 作为候选），这里解析不了就意味着
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
  "float_enabled": true,
  "window": {
    "x": 455,
    "y": 179,
    "width": 1657,
    "height": 986,
    "maximized": false
  },
  "float_window": {
    "x": 12,
    "y": 240,
    "width": 56,
    "height": 56,
    "edge": "left"
  }
}"#;
        let doc = parse_document(json).expect("必须能读 Electron 壳写的 settings.json");
        assert_eq!(doc.settings.wake_hotkey.key, "Digit4");
        assert!(doc.settings.wake_hotkey.alt);
        assert!(doc.settings.tray_enabled);
        assert!(doc.settings.float_enabled);
        assert_eq!(doc.window, Some(geom()));
        assert_eq!(doc.float_window, Some(float_geom()));
    }

    /// 反方向：我们写出来的字节形态必须能被 Electron 侧解析 —— 键序、缩进、布尔类型都钉死。
    #[test]
    fn pretty_output_is_canonical_json() {
        let doc = Document {
            settings: settings_a(),
            window: Some(geom()),
            float_window: Some(float_geom()),
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
  "float_enabled": false,
  "window": {
    "x": 455,
    "y": 179,
    "width": 1657,
    "height": 986,
    "maximized": false
  },
  "float_window": {
    "x": 12,
    "y": 240,
    "width": 56,
    "height": 56,
    "edge": "left"
  }
}"#,
            "写出的字节形态变了：Electron 侧的解析与探针都按这个字面量对齐"
        );

        // 无几何时不该出现 `window` / `float_window` 键（首次启动的文件里没有这两段）。
        let no_geom = Document {
            settings: settings_a(),
            window: None,
            float_window: None,
        };
        let json = serialize_document(&no_geom).unwrap();
        assert!(
            !json.contains("\"window\""),
            "没有几何时不该写 window 段：{json}"
        );
        assert!(
            !json.contains("\"float_window\""),
            "没有悬浮球几何时不该写 float_window 段：{json}"
        );

        // 布尔必须是 JSON 布尔，不能是字符串。
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v["tray_enabled"].is_boolean());
        assert!(v["float_enabled"].is_boolean());
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

    /// **原子写**（2026-09-24，对齐 Electron 侧）：写盘后①内容完整可解析回同一份文档、
    /// ②数据目录里**不留临时文件**。
    ///
    /// 守卫的是一条静默故障链：覆盖写被中途打断 ⇒ 留下截断 JSON ⇒ 下次启动整份解析失败
    /// ⇒ 设置与窗口几何一起回默认，全程不报错。
    #[test]
    fn save_at_writes_complete_json_without_leaving_temp_file() {
        with_temp("atomic", |p| {
            save_at(
                p,
                &Document {
                    settings: settings_a(),
                    window: Some(geom()),
                    float_window: None,
                },
            )
            .expect("写盘应成功");

            // ① 落盘的是完整文档（读回来与原值一致）
            let back = load_at(p);
            assert!(back.settings.tray_enabled, "写盘内容不完整或不一致");
            assert_eq!(back.window, Some(geom()));

            // ② 同目录下没有残留的临时文件
            let dir = p.parent().expect("临时文件必有父目录");
            let leftovers: Vec<String> = fs::read_dir(dir)
                .expect("目录应存在")
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n != FILE)
                .collect();
            assert!(leftovers.is_empty(), "原子写留下了临时文件：{leftovers:?}");

            // ③ 临时文件必须与目标**同目录**：跨卷 rename 会退化成非原子的「复制 + 删除」，
            //    那还不如不绕这一圈。
            let tmp = atomic_tmp_path(p);
            assert_eq!(tmp.parent(), p.parent(), "临时文件必须与目标同目录");
            assert_ne!(tmp, p.to_path_buf(), "临时文件名不能等于目标文件");
        });
    }

    // ---------- 落点 = 数据目录（2026-09-23）：新旧位置的读取顺序与「写只落新位置」 ----------
    // 这一组守卫的是「落点搬家」这个动作本身：搬错了/搬丢了的表象都是
    // 「用户的设置莫名其妙回到默认」，而它不会报任何错。

    /// 用后即删：把一组临时文件的**父目录**清掉。
    fn cleanup(paths: &[&Path]) {
        for p in paths {
            if let Some(d) = p.parent() {
                let _ = fs::remove_dir_all(d);
            }
        }
    }

    /// 新位置没有文件时，读旧位置（升级兼容）—— 不然用户升级后设置全丢。
    #[test]
    fn legacy_file_is_read_when_new_location_is_missing() {
        let primary = temp_file("new-empty");
        let legacy = temp_file("legacy-only");
        cleanup(&[&primary, &legacy]);
        save_at(
            &legacy,
            &Document {
                settings: settings_a(),
                window: Some(geom()),
                float_window: None,
            },
        )
        .unwrap();

        let doc = read_doc(&[primary.clone(), legacy.clone()]);
        assert_eq!(
            doc.settings.wake_hotkey.key, "Digit4",
            "新位置没有文件时必须读旧位置那份（升级兼容）"
        );
        assert_eq!(doc.window, Some(geom()));
        cleanup(&[&primary, &legacy]);
    }

    /// 两边都有时**新位置优先**：旧文件是死文件，不再参与。
    #[test]
    fn new_location_wins_over_legacy() {
        let primary = temp_file("both-new");
        let legacy = temp_file("both-old");
        cleanup(&[&primary, &legacy]);
        save_at(
            &legacy,
            &Document {
                settings: settings_a(),
                window: None,
                float_window: None,
            },
        )
        .unwrap();
        save_at(
            &primary,
            &Document {
                settings: settings_b(),
                window: None,
                float_window: None,
            },
        )
        .unwrap();

        let doc = read_doc(&[primary.clone(), legacy.clone()]);
        assert_eq!(doc.settings.wake_hotkey.key, "F9", "新位置必须优先于旧位置");
        cleanup(&[&primary, &legacy]);
    }

    /// ⚠️⚠️ 落点搬家的**核心回归守卫**：读旧位置、写新位置，且旧位置的值（含 `window` 段）
    /// 在搬过去的文档里必须还在 —— 只改一个字段不该把另一段丢掉。
    #[test]
    fn update_reads_legacy_but_writes_new_location() {
        let primary = temp_file("migrate-new");
        let legacy = temp_file("migrate-old");
        cleanup(&[&primary, &legacy]);
        save_at(
            &legacy,
            &Document {
                settings: settings_a(),
                window: Some(geom()),
                float_window: None,
            },
        )
        .unwrap();
        let legacy_before = fs::read_to_string(&legacy).unwrap();

        update_with(&primary, &[primary.clone(), legacy.clone()], |d| {
            d.settings = settings_b()
        })
        .unwrap();

        assert!(primary.exists(), "改动必须落到新位置（数据目录）");
        let back = load_at(&primary);
        assert_eq!(back.settings.wake_hotkey.key, "F9", "新设置没写进去");
        assert_eq!(
            back.window,
            Some(geom()),
            "从旧位置搬过来的 window 段丢了（读-改-写的「读」没走候选）"
        );
        assert_eq!(
            fs::read_to_string(&legacy).unwrap(),
            legacy_before,
            "旧位置的文件不该被改写（只读兼容，不迁移不删除）"
        );
        cleanup(&[&primary, &legacy]);
    }

    /// 候选全都不存在 = 首次启动：默认值，绝不 panic、也不建任何文件。
    #[test]
    fn no_candidate_exists_yields_defaults_without_touching_disk() {
        let primary = temp_file("none-new");
        let legacy = temp_file("none-old");
        cleanup(&[&primary, &legacy]);

        let doc = read_doc(&[primary.clone(), legacy.clone()]);
        assert!(!doc.settings.tray_enabled);
        assert_eq!(doc.window, None);
        assert!(!primary.exists() && !legacy.exists(), "读盘不该顺手建文件");
        cleanup(&[&primary, &legacy]);
    }
}
