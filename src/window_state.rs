//! 窗口几何持久化（仅桌面端）：记住上次窗口的大小/位置/最大化状态，
//! 下次启动自动恢复。数据目录与 settings 一致（`<app_config>/<app_name>/`），
//! 独立文件 window-state.json，不与“设置”混在一起（窗口几何随移动频繁变）。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, WebviewWindow};

use crate::config::{sanitize_dir_name, APP_NAME};

const FILE: &str = "window-state.json";

/// 窗口几何快照。x/y = 窗口左上角在屏幕上的坐标（outer），
/// width/height = 窗口外框尺寸（outer），均为【物理像素】，
/// 以保证保存与 set_position/set_size 一致、跨会话不因 DPI 缩放而错位。
#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct State {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// 旧版配置文件可能没有该字段：缺省按 false 解析（窗口不被误判为最大化）。
    #[serde(default)]
    pub maximized: bool,
}

fn path(app: &AppHandle) -> Option<PathBuf> {
    // 目录与 settings_path 保持一致：桌面端 app_config_dir 末段是 identifier，
    // 取父目录后再拼上清洗后的自定义 app_name（不同 app 配置互相隔离）。
    let base = app.path().app_config_dir().ok()?;
    let root = base.parent().unwrap_or(&base).to_path_buf();
    Some(root.join(sanitize_dir_name(APP_NAME)).join(FILE))
}

/// 读取上次保存的几何；文件不存在 / 解析失败返回 None（用默认窗口尺寸）。
pub fn load(app: &AppHandle) -> Option<State> {
    let p = path(app)?;
    let s = fs::read_to_string(&p).ok()?;
    serde_json::from_str::<State>(&s).ok()
}

/// 写入窗口几何。
pub fn save(app: &AppHandle, st: &State) {
    let Some(p) = path(app) else {
        return;
    };
    if let Some(parent) = p.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(st) {
        let _ = fs::write(&p, json);
    }
}

/// 读取指定 label 窗口当前几何（若该窗口存在）。
///
/// 单窗口模式主窗口为 `"main"`；多页签模式主窗口是宿主窗口（label 为 `tabs::HOST_LABEL`），
/// 由调用方传入实际 label。
pub fn persist_label(app: &AppHandle, label: &str) {
    let Some(win) = app.get_webview_window(label) else {
        return;
    };
    if let Some(st) = capture(&win) {
        save(app, &st);
    }
}

/// 从给定窗口抓取当前几何。
fn capture(win: &WebviewWindow) -> Option<State> {
    let pos = win.outer_position().ok()?;
    let size = win.outer_size().ok()?;
    let maximized = win.is_maximized().unwrap_or(false);
    Some(State {
        x: pos.x,
        y: pos.y,
        width: size.width,
        height: size.height,
        maximized,
    })
}

/// 创建窗口后应用已保存的几何；保存的几何若已失效（上次最大化、尺寸过小、
/// 或窗口中心落在屏外——显示器被拔掉/分辨率/DPI 变化所致）则**丢弃几何并直接
/// 最大化**，保证窗口始终可见可用。无保存状态时不做任何事（保留窗口默认尺寸）。
pub fn apply(win: &WebviewWindow, st: &State) {
    // 1) 上次是最大化 → 直接最大化，必然铺满且可见。
    if st.maximized {
        let _ = win.maximize();
        return;
    }

    // 2) 校验保存的位置/尺寸是否还“够用”：
    //    - 窗口中心点不在任何当前显示器上 → 窗口被留在屏外，用户看不到。
    //    - 尺寸小于最小窗口约束(创建时 min_inner_size=400x300，此处 outer 略留余量)。
    let center_on_screen = win
        .monitor_from_point(
            (st.x as f64) + (st.width as f64) / 2.0,
            (st.y as f64) + (st.height as f64) / 2.0,
        )
        .ok()
        .flatten()
        .is_some();
    let too_small = st.width < 420 || st.height < 320;
    if !center_on_screen || too_small {
        let _ = win.maximize();
        return;
    }

    // 3) 几何有效：恢复位置与尺寸。
    let _ = win.set_position(tauri::PhysicalPosition::new(st.x, st.y));
    let _ = win.set_size(tauri::PhysicalSize::new(st.width, st.height));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_serde_roundtrip_preserves_all_fields() {
        let st = State {
            x: 100,
            y: 200,
            width: 800,
            height: 600,
            maximized: true,
        };
        let json = serde_json::to_string(&st).unwrap();
        let back: State = serde_json::from_str(&json).unwrap();
        assert_eq!(back.x, 100);
        assert_eq!(back.y, 200);
        assert_eq!(back.width, 800);
        assert_eq!(back.height, 600);
        assert!(back.maximized);
    }

    #[test]
    fn state_parse_accepts_missing_maximized_as_false() {
        // 兼容旧版配置文件（无 maximized 字段时 serde 默认补 false）。
        let back: State = serde_json::from_str(r#"{"x":1,"y":2,"width":3,"height":4}"#).unwrap();
        assert!(!back.maximized);
    }
}
