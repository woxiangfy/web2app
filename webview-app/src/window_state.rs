//! 窗口几何：记住上次窗口的大小 / 位置 / 最大化状态，下次启动自动恢复（仅桌面端）。
//!
//! ⚠️ **落盘位置（2026-09-23 起）**：与「设置」同住**同一个** `settings.json` 的 `window` 段，
//! 不再有独立的 `window-state.json`（旧的 `window-state.json` 就此成为死文件，不再读也不再写）。
//! 类型 `WindowGeom`、文件落点与「读-改-写」工具都在 [`crate::store`] —— 本模块只管
//! 「抓几何 / 恢复几何」这件事本身；**绝不能**自己去写整个文件，那会把用户刚保存的设置抹掉。

use tauri::{AppHandle, Manager, WebviewWindow};

use crate::store::WindowGeom;

/// 读取上次保存的几何；没有（首次启动 / 文件里没有 `window` 段）返回 None（用默认窗口尺寸）。
pub fn load(app: &AppHandle) -> Option<WindowGeom> {
    crate::store::load(app).window
}

/// 写入窗口几何（读-改-写，只替换 `window` 段）。
///
/// 失败只在诊断日志里留痕：为了记一个窗口尺寸让用户看到报错不值得。
pub fn save(app: &AppHandle, st: &WindowGeom) {
    if let Err(e) = crate::store::update(app, |doc| doc.window = Some(*st)) {
        dlog!("[window_state] 写入窗口几何失败：{e}");
    }
}

/// 读取指定 label 窗口当前几何并落盘（若该窗口存在）。
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
fn capture(win: &WebviewWindow) -> Option<WindowGeom> {
    let pos = win.outer_position().ok()?;
    let size = win.outer_size().ok()?;
    let maximized = win.is_maximized().unwrap_or(false);
    Some(WindowGeom {
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
pub fn apply(win: &WebviewWindow, st: &WindowGeom) {
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
    fn geometry_serde_roundtrip_preserves_all_fields() {
        let g = WindowGeom {
            x: 100,
            y: 200,
            width: 800,
            height: 600,
            maximized: true,
        };
        let json = serde_json::to_string(&g).unwrap();
        let back: WindowGeom = serde_json::from_str(&json).unwrap();
        assert_eq!(back, g);
    }

    #[test]
    fn geometry_parse_accepts_missing_maximized_as_false() {
        // 兼容更早的配置文件（无 maximized 字段时 serde 默认补 false）。
        let back: WindowGeom =
            serde_json::from_str(r#"{"x":1,"y":2,"width":3,"height":4}"#).unwrap();
        assert!(!back.maximized);
    }

    // ⚠️ 几何「写进 / 读出同一个 JSON 文件」的互通契约（含与 Electron 壳的字面量比对）
    // 已挪到 `crate::store::tests` —— 那边测的是**文档中的 window 段**，
    // 因为落盘单位已经从「几何独占一个文件」变成「文档里的一段」。
}
