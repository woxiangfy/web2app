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

// ⚠️ TITLE 只被下方 `#[cfg(desktop)]` 的 open_settings_window 使用（拼设置窗标题）。
// 若写成无条件 use，Android 目标下该导入即「未使用」，在 `-D warnings`
// （CI 的 check-android → clippy）下直接编译失败。必须按 cfg 拆开。
#[cfg(desktop)]
use crate::config::TITLE;

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

/// 唤醒快捷键的可读文案（空 key = 未启用），供启动诊断日志使用（`lib.rs::log_startup`）。
/// 本函数跨平台（Hotkey 本身跨平台）。
///
/// ⚠️⚠️ 跨壳契约：与 Electron 侧 `hotkey.ts::hotkeyLabel()` **逐字同形**（modifier 顺序、
/// 连接符、空值文案的半角括号都要一致），这样两侧启动日志可直接对照。美化规则：
///   · 空 key（含纯空白）→ `(未设置)`（半角括号，与 Electron 一致）；
///   · `Digit` 前缀去掉（`Digit4` → `4`）、`Key` 前缀去掉（`KeyA` → `A`）；
///   · `Numpad` 前缀换成 `Num` 后接剩余部分（`Numpad0` → `Num0`、`NumpadDecimal` → `NumDecimal`）；
///   · 四个方向键转箭头（`ArrowUp` → `↑` 等）；其余键名（`F1` / `Space` / `Backquote`…）原样。
pub(crate) fn hotkey_label(h: &Hotkey) -> String {
    let key = h.key.trim();
    if key.is_empty() {
        return "(未设置)".to_string();
    }
    let pretty = if let Some(rest) = key.strip_prefix("Digit") {
        rest.to_string()
    } else if let Some(rest) = key.strip_prefix("Key") {
        rest.to_string()
    } else if let Some(rest) = key.strip_prefix("Numpad") {
        format!("Num{rest}")
    } else {
        match key {
            "ArrowUp" => "↑".to_string(),
            "ArrowDown" => "↓".to_string(),
            "ArrowLeft" => "←".to_string(),
            "ArrowRight" => "→".to_string(),
            other => other.to_string(),
        }
    };
    let mut parts: Vec<String> = Vec::new();
    if h.ctrl {
        parts.push("Ctrl".to_string());
    }
    if h.alt {
        parts.push("Alt".to_string());
    }
    if h.shift {
        parts.push("Shift".to_string());
    }
    parts.push(pretty);
    parts.join("+")
}

/// 读取设置；文件不存在或解析失败时使用默认值。
///
/// 整份文件（设置 + 窗口几何）的读写与「读-改-写」都在 [`crate::store`]，这里只取设置那一段。
/// ⚠️ 诊断：文件**不存在**（首次启动、很常见）是正常态，不打日志、不告警；但**读盘失败**
/// （权限 / 磁盘错误）与**内容解析失败**（文件损坏 / 被第三方改写）是真实的「配置没生效」
/// 隐患 —— 用户反馈「我明明勾了托盘、重启又没了」正是这条路径，故那两类失败必须在 `store`
/// 里经 `dlog!` 现形（`debug` 开启时能定位到具体文件与原因），否则永远查不到根因。
pub fn load_settings(app: &AppHandle) -> Settings {
    crate::store::load(app).settings
}

/// 写入设置（纯持久化，不含运行时副作用）。
///
/// ⚠️⚠️ 必须是**读-改-写**：设置与窗口几何现在同住一个 `settings.json`，
/// 拿内存里的整份文档直接落盘会把「挪窗口」那一方刚写进去的几何一起抹掉
/// （原 `window-state.ts` 文件头正是用这个理由论证过不该合并；见 `crate::store` 模块头）。
pub fn persist_settings(app: &AppHandle, s: &Settings) -> Result<(), String> {
    let s = s.clone();
    crate::store::update(app, |doc| doc.settings = s)
}

/// 打开（或聚焦已存在的）设置窗口。仅桌面端使用。
/// 设置页通过 `settings://localhost/` 自定义协议提供（协议在 Builder 上注册，
/// HTML 由 Rust 侧编译期内嵌），因此不依赖 devUrl，dev / release 行为一致。
///
/// ⚠️⚠️ 本函数【不能】在调用栈内直接建窗：三条调用路径全部发生在**主线程事件回调栈内**——
/// ① 标签栏齿轮 → IPC 命令 `open_settings`（tauri 在主线程 `on_message` 里**同步**派发
///    invoke）；② 自绘标题栏 → `web2app:open-settings` 事件监听；③ 全局快捷键 Shift+F1。
/// 在回调栈内联 `WebviewWindowBuilder::build()` 等于「事件派发期间再创建一个窗口 + 新的
/// WebView2 环境」→ 嵌套事件循环，新 WebView2 controller 的初始化消息永远得不到分发：
/// Win32 窗口建出来了（620×600、任务栏有「设置 - xxx」），但 settings:// 协议从不被请求、
/// 页面永不渲染 → 用户看到一个纯白无边框窗。
///
/// 修法与 [`crate::webview_hooks`] 的 NewWindowRequested 处理同模式：先切到后台线程，
/// 再 `run_on_main_thread` 把真正的建窗排队到**下一个事件循环轮次**（当前回调退栈后）。
/// ⚠️ 不能省掉后台线程直接 `run_on_main_thread`——在主线程上调用它会被**同步内联执行**，
/// 等于没 defer（webview_hooks 里已有同一条踩坑注释）。
#[cfg(desktop)]
pub fn open_settings_window(app: &AppHandle) {
    let app_deferred = app.clone();
    std::thread::spawn(move || {
        let app_main = app_deferred.clone();
        let _ = app_deferred.run_on_main_thread(move || {
            open_settings_window_on_main(&app_main);
        });
    });
}

/// [`open_settings_window`] 的实际实现：运行在主线程的一个【独立】事件循环轮次
///（已脱离 IPC / 事件 / 快捷键回调栈），可以安全地创建设置窗口。
///
/// ⚠️ 设置窗是**无边框窗**（`decorations(false)`）：用户要求的版式是「没有顶部的窗体、
/// 面板居中、内容多可滚动」——标题与关闭按钮由 `dist/settings.html` 自绘在面板头部
/// （拖动靠 `data-tauri-drag-region`，需要 capabilities/settings-window.json 里的
/// `core:window:allow-start-dragging`；少了它窗口会拖不动且**不会报错**）。
/// 窗口 `center()` 到屏幕中央，底色取加载层同一个浅灰，盖住 WebView 初始化瞬间的白闪。
#[cfg(desktop)]
fn open_settings_window_on_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        return;
    }
    let url = url::Url::parse("settings://localhost/").expect("构造 settings 协议 URL 失败");
    // build 失败必须留痕：这类错误此前被 `let _` 静默吞掉，用户侧只表现为「点齿轮没反应 /
    // 空白窗」，没有任何线索能区分「命令没调到」与「建窗失败」。
    if let Err(e) = WebviewWindowBuilder::new(app, "settings", WebviewUrl::CustomProtocol(url))
        // 标题用【显示名】TITLE 而不是【安装名】APP_NAME —— 这个标题是给用户看的
        //（无边框窗里系统不画它，但仍会出现在任务栏 / 任务视图与无障碍工具里）。
        .title(format!("设置 - {}", TITLE))
        .inner_size(620.0, 600.0)
        .min_inner_size(460.0, 400.0)
        .resizable(true)
        .decorations(false)
        .background_color(crate::bootstrap::SPLASH_BG)
        .center()
        .build()
    {
        dlog!("[settings] 设置窗创建失败：{e}");
    }
}

/// settings 自定义协议处理器：编译期把 dist/settings.html 内嵌后经协议返回设置页。
///
/// 与 tabs::handle_shell / loading::handle_splash 同属「内嵌前端页」处理器：
/// 不依赖 devUrl（tauri dev 会把 App 相对路径代理到 devUrl，导致设置页被解析到
/// example.com），dev / release 行为完全一致。跨平台（协议在 lib.rs 无条件注册）。
pub(crate) fn handle_settings(
    _ctx: tauri::UriSchemeContext<'_, tauri::Wry>,
    _req: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    let html = include_str!("../dist/settings.html");
    tauri::http::Response::builder()
        .header("Content-Type", "text/html; charset=utf-8")
        .body(html.as_bytes().to_vec())
        .unwrap()
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

/// 打开设置窗口（IPC 命令）。
///
/// ⚠️ 谁能调：
///   · **本地页面**（多页签模式的标签栏 shell://localhost/）—— 直接 invoke 本命令，不需要 ACL；
///   · **远程页面**（单窗口模式的主窗）—— **调不了**：Tauri 2 对非本地 origin 的 IPC 强制走
///     ACL，而应用命令没有 app 级 ACL 清单可授权（与 `history_nav` 同一条约束）。自绘标题栏
///     里那个「设置」按钮只能发核心插件的 `plugin:event|emit`，由 lib.rs 的事件监听转发到这里。
#[cfg(desktop)]
#[tauri::command]
pub(crate) fn open_settings(app: tauri::AppHandle) {
    open_settings_window(&app);
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

    // ⚠️ 「双壳互通契约」（字段名 / 嵌套 / 字节形态）的断言已随文件合并挪到
    // `crate::store::tests` —— 那边测的是**整份文档**（设置 + `window` 段），
    // 这里再留一份 `Settings` 单体的等价断言就会出现两个真源。

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

    // hotkey_label 跨平台（不依赖 tauri Shortcut），且与 Electron 侧 hotkey.ts::hotkeyLabel()
    // 逐字同形 —— 下面的期望值就是双壳契约，改动任一侧都必须同步另一侧。
    #[test]
    fn hotkey_label_aligns_with_electron() {
        fn hk(ctrl: bool, alt: bool, shift: bool, key: &str) -> Hotkey {
            Hotkey {
                ctrl,
                alt,
                shift,
                key: key.into(),
            }
        }
        // 空 key（含纯空白）：半角括号文案，修饰键也被忽略
        assert_eq!(hotkey_label(&hk(false, false, false, "")), "(未设置)");
        assert_eq!(hotkey_label(&hk(true, false, false, "   ")), "(未设置)");
        // Digit / Key 前缀美化（Alt+4 与 Electron 探针 probe-single.ts 钉的是同一条）
        assert_eq!(hotkey_label(&hk(false, true, false, "Digit4")), "Alt+4");
        assert_eq!(hotkey_label(&hk(true, false, true, "KeyA")), "Ctrl+Shift+A");
        assert_eq!(hotkey_label(&hk(false, false, false, "Digit0")), "0");
        // Numpad：前缀换 Num 后接剩余部分（注意不是 accelerator 用的 num0 / numdec）
        assert_eq!(hotkey_label(&hk(false, false, false, "Numpad0")), "Num0");
        assert_eq!(
            hotkey_label(&hk(false, false, false, "NumpadDecimal")),
            "NumDecimal"
        );
        // 方向键转箭头
        assert_eq!(hotkey_label(&hk(false, false, false, "ArrowUp")), "↑");
        assert_eq!(hotkey_label(&hk(false, false, false, "ArrowDown")), "↓");
        assert_eq!(hotkey_label(&hk(false, false, false, "ArrowLeft")), "←");
        assert_eq!(hotkey_label(&hk(false, false, false, "ArrowRight")), "→");
        // 其余键名原样；修饰键顺序固定 Ctrl → Alt → Shift
        assert_eq!(hotkey_label(&hk(false, false, false, "F1")), "F1");
        assert_eq!(
            hotkey_label(&hk(false, false, false, "Backquote")),
            "Backquote"
        );
        assert_eq!(
            hotkey_label(&hk(true, true, true, "Digit0")),
            "Ctrl+Alt+Shift+0"
        );
    }
}
