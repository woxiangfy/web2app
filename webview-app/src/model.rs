//! 跨壳数据契约：`settings.json` 里「设置」那一段的形状（纯数据，无行为）。
//!
//! 为什么单独成模块（架构审查 2026-11-19，闭环 `docs/architecture-todo.md` P-R1）：
//! 这两个结构原先住在 `settings.rs`，而持久化层 `store.rs` 的 `Document` 又要以
//! `settings: Settings` 作为字段类型 ⇒ 依赖图成环
//! （`settings → store` 调用读写、`store → settings` 引用类型）。Rust 同 crate 内允许这种
//! 环，编译零告警，但语义上「持久化层被迫向上引用业务层的数据模型」是反的。
//!
//! 把纯数据下沉到本模块后方向变成单向：
//!   · `store → model`（持久化认识形状，不认识业务）；
//!   · `settings → model`（业务层再用 re-export 保持 `crate::settings::Settings` 这个旧路径可用，
//!     调用点无需改动）；
//!   · 本模块**不引用任何 crate 内其它模块**（只有 serde），因此不可能再造出新环。
//!
//! ⚠️ 这里只放「会落盘 / 跨壳共享」的类型。命令返回形态（如 `SettingsView`）、校验规则、
//!    窗口与副作用编排都留在 `settings.rs` —— 那些是行为，不是契约。

use serde::{Deserialize, Serialize};

/// 应用设置（持久化到 JSON）。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settings {
    /// 唤醒快捷键（显示 / 隐藏主窗口）。
    pub wake_hotkey: Hotkey,
    /// 是否启用系统托盘图标。
    pub tray_enabled: bool,
    /// 关闭窗口时是否最小化到托盘（需同时启用托盘图标，否则关闭即退出）。
    pub minimize_on_close: bool,
    /// 是否启用桌面悬浮小窗（仅 Windows 桌面端有实际行为；其他平台只持久化该开关）。
    /// `#[serde(default)]`：旧版 settings.json 没有这一字段时按 false 解析，不至于整份失效。
    #[serde(default)]
    pub float_enabled: bool,
    /// 启动时检测到「主进程判为不合法」的旧快捷键后留下的提示（给人看的原因）。
    /// `#[serde(skip)]` ⇒ **不写进 settings.json**，也不从设置页的保存载荷里读；
    /// 只经 `get_settings` 的 `SettingsView` 发给设置页一次，保存成功后自然归空。
    #[serde(skip)]
    pub(crate) hotkey_notice: String,
}

/// 唤醒快捷键结构。key 为 JS KeyboardEvent.code（如 "Digit4" / "F1" / "KeyA"），
/// 可与 Code::from_str 互转；修饰键由三个布尔开关表示。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
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
            // 默认【不启用】桌面悬浮小窗。
            float_enabled: false,
            // 进程内提示：默认没有。
            hotkey_notice: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 旧版 `settings.json` 缺 `float_enabled` 时按 false 解析，其余字段不受影响。
    #[test]
    fn missing_float_enabled_defaults_to_false() {
        let v: serde_json::Value = serde_json::json!({
            "wake_hotkey": {"ctrl": true, "alt": false, "shift": false, "key": "KeyQ"},
            "tray_enabled": true,
            "minimize_on_close": true
        });
        let s: Settings = serde_json::from_value(v).expect("旧文件必须仍可解析");
        assert!(!s.float_enabled);
        assert!(s.tray_enabled);
        assert_eq!(s.wake_hotkey.key, "KeyQ");
    }

    /// `hotkey_notice` 是进程内提示：不落盘、也不从保存载荷里读。
    #[test]
    fn hotkey_notice_is_not_serialized() {
        let s = Settings {
            hotkey_notice: "旧快捷键已失效".to_string(),
            ..Settings::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(
            !json.contains("hotkey_notice"),
            "进程内提示不得写进 settings.json：{json}"
        );
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert!(back.hotkey_notice.is_empty());
    }
}
