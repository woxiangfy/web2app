#[cfg(desktop)]
use std::str::FromStr;
use std::sync::Mutex;

use serde::Serialize;
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

/// 设置的数据模型已下沉到 [`crate::model`]（架构审查 2026-11-19，闭环 `docs/architecture-todo.md`
/// P-R1 的「store ↔ settings 类型环」）：那里是纯数据、不引用任何模块，`store` 直接以它为字段
/// 类型，方向变成单向 `settings → store`、`store → model`。
///
/// 这里用 `pub use` 保留原路径 `crate::settings::{Settings, Hotkey}`：调用点（`lib.rs` 的
/// `SettingsState`、`debug_log` 的启动信息、契约测试）无需改动，且语义上仍是「设置业务
/// 拥有这套模型」。
pub use crate::model::{Hotkey, Settings};

/// `get_settings` 的返回形态：设置本体 + 只给界面看的进程内提示（不落盘）。
///
/// `#[serde(flatten)]` 让前端拿到的字段形状与原来的 `Settings` 完全一致，
/// 只多一个可选的 `hotkey_notice`（Electron 侧不发这个字段，共用设置页按空处理）。
#[derive(Serialize)]
pub(crate) struct SettingsView {
    #[serde(flatten)]
    settings: Settings,
    hotkey_notice: String,
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

/// 系统占用的组合（`hotkey_label` 的输出 → 原因）。
///
/// ⚠️⚠️ 跨壳**三处逐字一致**：Electron `hotkey.ts::RESERVED_HOTKEYS`、共用设置页
/// `dist/settings.html::HK_RESERVED`、本表。改一处必须三处同改 ——
/// `contract_tests/settings_hotkey_contract.rs` 逐条对照（含原因文案）。
pub(crate) const RESERVED_HOTKEYS: &[(&str, &str)] = &[
    ("Alt+Tab", "系统切换窗口"),
    ("Alt+Space", "系统窗口菜单"),
    ("Alt+F4", "关闭当前窗口"),
    ("Alt+Escape", "系统切换窗口"),
    ("Ctrl+Escape", "打开开始菜单"),
    // 中文 Windows 上这是输入法中英文切换：注册成全局键会**打断用户打字**，比不生效更糟
    ("Ctrl+Space", "输入法中英文切换"),
    ("Ctrl+Alt+Delete", "系统安全界面"),
];

/// Electron `hotkey.ts::CODE_TO_KEY` 里**逐个列名**的那部分（字母 / 主键数字 / 功能键 /
/// 小键盘数字由前缀区间覆盖，见 [`is_supported_key`]）。
///
/// 两侧必须同一套键集：否则会出现「设置页能存、某一壳却注册不上」的静默差异
/// （Electron 的 accelerator 表是手写的，认不出的键 `register()` 会静默失败）。
pub(crate) const SUPPORTED_EXTRA_KEYS: &[&str] = &[
    "NumpadDecimal",
    "NumpadAdd",
    "NumpadSubtract",
    "NumpadMultiply",
    "NumpadDivide",
    "Space",
    "Enter",
    "Escape",
    "Tab",
    "Backspace",
    "Delete",
    "Insert",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
    "Minus",
    "Equal",
    "BracketLeft",
    "BracketRight",
    "Backslash",
    "Semicolon",
    "Quote",
    "Comma",
    "Period",
    "Slash",
    "Backquote",
];

/// 这个 `KeyboardEvent.code` 能不能当全局快捷键的主键（与 Electron `CODE_TO_KEY` 同一套键集）。
fn is_supported_key(code: &str) -> bool {
    if SUPPORTED_EXTRA_KEYS.contains(&code) {
        return true;
    }
    let single = |s: &str| s.len() == 1;
    if let Some(rest) = code.strip_prefix("Key") {
        return single(rest) && rest.as_bytes()[0].is_ascii_uppercase();
    }
    if let Some(rest) = code.strip_prefix("Digit") {
        return single(rest) && rest.as_bytes()[0].is_ascii_digit();
    }
    if let Some(rest) = code.strip_prefix("Numpad") {
        return single(rest) && rest.as_bytes()[0].is_ascii_digit();
    }
    if let Some(rest) = code.strip_prefix('F') {
        return rest
            .parse::<u8>()
            .map(|n| (1..=24).contains(&n))
            .unwrap_or(false);
    }
    false
}

/// 校验一个唤醒快捷键：**空串 = 可保存**，否则是给人看的原因。
///
/// 判据与顺序都对齐 Electron 主进程 `hotkey.ts::hotkeyProblem`（跨壳同一套，别各写近似版）：
/// 未设置 → 裸单键 → 系统占用 → `Shift+F1` → 无法映射的按键。
/// 同一函数被两处使用：① `save_settings` 保存前（页面之外的第二道）
/// ② `load_settings` 启动回退（旧配置被判非法时不注册、并在设置页提示）。
pub(crate) fn hotkey_problem(h: &Hotkey) -> String {
    let key = h.key.trim();
    if key.is_empty() {
        // 「不设置快捷键」永远合法
        return String::new();
    }
    let label = hotkey_label(h);
    if !h.ctrl && !h.alt && !h.shift {
        return format!(
            "只有单个按键（{label}）：全局快捷键会占满这个键的输入（在别处打不了字），\
             请至少勾选一个修饰键（Ctrl / Alt / Shift）；不想要快捷键就把主键选成「无」"
        );
    }
    if let Some((_, why)) = RESERVED_HOTKEYS.iter().find(|(k, _)| *k == label) {
        return format!("{label} 已被占用（{why}），注册了也会被抢走，请换一个");
    }
    if label == "Shift+F1" {
        return "Shift+F1 是「打开设置」的固定快捷键，不能占用".to_string();
    }
    if !is_supported_key(key) {
        return format!("不支持的按键（{label}），请换一个");
    }
    String::new()
}

/// 读取设置；文件不存在或解析失败时使用默认值。
///
/// 整份文件（设置 + 窗口几何）的读写与「读-改-写」都在 [`crate::store`]，这里只取设置那一段。
/// ⚠️ 诊断：文件**不存在**（首次启动、很常见）是正常态，不打日志、不告警；但**读盘失败**
/// （权限 / 磁盘错误）与**内容解析失败**（文件损坏 / 被第三方改写）是真实的「配置没生效」
/// 隐患 —— 用户反馈「我明明勾了托盘、重启又没了」正是这条路径，故那两类失败必须在 `store`
/// 里经 `dlog!` 现形（`debug` 开启时能定位到具体文件与原因），否则永远查不到根因。
pub fn load_settings(app: &AppHandle) -> Settings {
    let mut s = crate::store::load(app).settings;
    fallback_invalid_hotkey(&mut s);
    s
}

/// 启动回退：磁盘上留着的旧快捷键若已不被接受（裸单键 / 系统占用 / `Shift+F1` / 无法映射），
/// 本次运行按「不设置」处理 —— **不注册**，也不改写文件；只在内存里留一条提示给设置页。
///
/// 不这样做的话，旧配置的表现是「按下去没反应」或「占满某个键」，用户会以为程序坏了。
/// 提示只活在进程内：用户在设置页重新保存一次（`new_settings` 的该字段为默认空）即清空。
fn fallback_invalid_hotkey(s: &mut Settings) -> bool {
    let problem = hotkey_problem(&s.wake_hotkey);
    if problem.is_empty() {
        return false;
    }
    let label = hotkey_label(&s.wake_hotkey);
    dlog!("[settings] 唤醒快捷键 {label} 不再被接受（{problem}），本次运行回退为不设置");
    s.hotkey_notice = format!(
        "旧配置里的唤醒快捷键 {label} 不再被接受：{problem}。\
         已回退为「不设置」（配置文件未改动）；重新设置并保存后本条提示消失。"
    );
    s.wake_hotkey = Hotkey::default();
    true
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

/// 进程内翻转「启用桌面悬浮图标」这一个开关（v1.76，球上**右键**调用）。
///
/// 为什么不是复用 `save_settings` 命令：右键发生在 Rust 的事件回调里，那里没有前端调用栈、
/// 也就没有整份 `Settings` 载荷可传；而自行拼一份新 `Settings` 会把用户刚在设置页改的
/// 快捷键 / 托盘开关一起覆盖掉。所以这里只翻转**一个**布尔：
/// 读当前内存态 → 改这一位 → 落盘 → 写回内存态 → 同步已打开的设置页。
///
/// ⚠️ 顺序照搬 `save_settings` 的纪律：**落盘成功才写内存态**（任一步失败都不写内存态），
///    否则会出现「界面 / 运行期已改、文件没写成、重启又回来」的自相矛盾。
///
/// ⚠️ 必须同步设置页 UI（[`push_float_enabled_to_settings_window`]）：设置窗若正开着，
///    它的复选框还打着勾，用户随手一点「保存」就会把 `float_enabled` 推回 true（球自己跑回来）。
///
/// 仅 Windows 需要（悬浮球只在 Windows 存在 ⇒ 唯一调用方 `floatwin`）；
/// 其它平台没有调用方，写成无条件函数会因 `dead_code` 在 `-D warnings` 下编译失败。
#[cfg(windows)]
pub(crate) fn set_float_enabled(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let mut next = app
        .state::<SettingsState>()
        .0
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    if next.float_enabled != enabled {
        next.float_enabled = enabled;
        persist_settings(app, &next)?;
        // ⚠️ 先把 State 绑到局部变量再取锁：`app.state::<T>().0.lock()` 一行写会因
        //    临时值在语句末尾被 drop 而借不到（E0716）。
        let st = app.state::<SettingsState>();
        let mut guard = st.0.lock().unwrap_or_else(|p| p.into_inner());
        *guard = next;
    }
    push_float_enabled_to_settings_window(app, enabled);
    Ok(())
}

/// 把「启用桌面悬浮图标」的最新值直接推给**已打开的**设置页（没有设置窗时静默）。
///
/// 为什么必须推：设置页只在**加载时**读一次 `get_settings`，而球上右键关功能发生在页面
/// 之外 —— 不同步的话复选框还打着勾，用户随手一点「保存」就会把 `float_enabled` 推回 true。
/// `#floatEnabled` 这个 id 与 dist/settings.html、探针三处同源，改名要一起改。
#[cfg(windows)]
fn push_float_enabled_to_settings_window(app: &AppHandle, enabled: bool) {
    if let Some(w) = app.get_webview_window("settings") {
        let js = format!(
            "try{{var c=document.getElementById('floatEnabled');if(c)c.checked={enabled};}}catch(e){{}}"
        );
        let _ = w.eval(&js);
    }
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
        .background_color(crate::nav::SPLASH_BG)
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
pub(crate) fn get_settings(state: State<SettingsState>) -> SettingsView {
    let s = state.0.lock().unwrap_or_else(|p| p.into_inner()).clone();
    let hotkey_notice = s.hotkey_notice.clone();
    SettingsView {
        settings: s,
        hotkey_notice,
    }
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
    // 1)+2) 桌面端：唤醒快捷键可留空（表示“不设置快捷键”）；若填写则必须过
    // `hotkey_problem` 的**全部判据**（与 Electron 主进程同一套：裸单键 / 系统占用 /
    // Shift+F1 / 无法映射）。页面已经拦过一次，这里是「绕过页面直写 settings.json」
    // 路径的第二道。Android 无全局快捷键，跳过此校验
    // （也不引用 tauri_plugin_global_shortcut 的类型）。
    #[cfg(desktop)]
    let new_sc = {
        let key = new_settings.wake_hotkey.key.trim();
        if key.is_empty() {
            // 空 key = 未启用唤醒快捷键
            None
        } else {
            let problem = hotkey_problem(&new_settings.wake_hotkey);
            if !problem.is_empty() {
                return Err(problem);
            }
            // 判据里已含「可映射」，这里只剩类型转换；仍用 `ok_or_else` 兜底，
            // 避免两处键集定义万一漂移时变成 panic。
            let sc = hotkey_to_shortcut(&new_settings.wake_hotkey).ok_or_else(|| {
                format!(
                    "不支持的按键：{}，请换一个按键",
                    new_settings.wake_hotkey.key
                )
            })?;
            Some(sc)
        }
    };

    // 1) 锁内一次性克隆旧设置，随即释放锁（临界区极小，只有一次 clone）。
    //    `old` 只用于桌面端的快捷键 / 托盘 diff；Android 无这两项，故一并隔离，避免 unused 警告。
    #[cfg(desktop)]
    let old = state.0.lock().unwrap_or_else(|p| p.into_inner()).clone();

    // 2) 先落盘（磁盘优先），再动运行期。
    //
    //    顺序很关键：旧实现是「先应用副作用、再持久化」，一旦写盘失败（磁盘满 / 只读 /
    //    文件被占）就出现一种极难自查的错位 —— 新快捷键已注册、旧快捷键已注销，而内存态
    //    与文件仍是旧值；快捷键回调比对的是**内存**旧值 ⇒ 新键无反应、旧键失效。用户重试
    //    保存时 diff 仍是「旧≠新」，`register(新)` 返回「已注册」⇒ 走回滚 ⇒ 该键从此再也
    //    存不进去（直到重启）。
    //
    //    先落盘则相反：写失败时运行期一个副作用都没动。剩下的失败面只有「已落盘、但运行时
    //    副作用失败」（键被别的程序占用 / 托盘创建失败），那条路径用 [`restore_persisted`]
    //    把磁盘退回旧值。
    //
    //    锁外执行：磁盘 I/O 是耗时最长的一步，不应阻塞其它读设置的线程。
    persist_settings(&app, &new_settings).map_err(|e| e.to_string())?;

    // 3) 桌面端：应用运行时的快捷键 / 托盘变更（锁外执行——全局快捷键注册、
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
                    restore_persisted(&app, &old);
                    return Err(format!("快捷键注册失败（可能已被其它程序占用）：{e}"));
                }
            }
        }
        if new_settings.tray_enabled != old.tray_enabled {
            if new_settings.tray_enabled {
                if let Err(e) = crate::tray::create_tray(&app) {
                    restore_persisted(&app, &old);
                    return Err(e.to_string());
                }
            } else {
                app.remove_tray_by_id("main-tray");
            }
        }
        // 桌面悬浮小窗开关：勾选 → 立即建窗显示；取消勾选 → 立即销毁（几何保留在 settings.json，
        // 下次勾选时恢复）。仅 Windows 有实现；其他桌面平台该开关只是一个被持久化的布尔值。
        #[cfg(windows)]
        if new_settings.float_enabled != old.float_enabled {
            if new_settings.float_enabled {
                crate::floatwin::show_float_window(&app);
            } else {
                crate::floatwin::destroy_float_window(&app);
            }
        }
    }

    // 4) 最后写内存态（临界区极小）。走到这里说明磁盘与运行期都已是新设置；
    //    任一失败都在上面 `?` 提前返回，内存态仍是旧值。
    let mut guard = state.0.lock().unwrap_or_else(|p| p.into_inner());
    *guard = new_settings;
    Ok(())
}

/// 副作用应用失败后，把已落盘的新设置退回旧值（`save_settings` 的补偿步骤）。
///
/// 只在「已写盘、但运行时副作用失败」这一条路径上调用：内存态此时仍是旧值，不回写磁盘的话，
/// 下次启动会载入一份从未生效过的设置（例如一个注册不上的快捷键）。
///
/// 双失败（写盘 + 回写都失败）只能留日志：内存态与运行期仍是一致的旧值，代价是磁盘上留着
/// 未生效的新值，下次启动以它为准 —— 比「运行期与内存/磁盘三方各执一词」更容易解释。
#[cfg(desktop)]
fn restore_persisted(app: &tauri::AppHandle, old: &Settings) {
    if let Err(e) = persist_settings(app, old) {
        dlog!("[settings] 副作用失败后回写旧设置也失败：{e}");
    }
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
            float_enabled: true,
            hotkey_notice: String::new(),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.wake_hotkey.key, "KeyA");
        assert!(back.wake_hotkey.ctrl);
        assert!(back.tray_enabled);
        assert!(back.minimize_on_close);
        assert!(back.float_enabled);
    }

    // 旧版 settings.json 没有 float_enabled 字段：#[serde(default)] 必须兜成 false，
    // 否则升级后老用户的整份配置会因为缺字段而解析失败。
    #[test]
    fn settings_without_float_enabled_defaults_false() {
        let json = r#"{"wake_hotkey":{"ctrl":false,"alt":false,"shift":false,"key":""},"tray_enabled":true,"minimize_on_close":false}"#;
        let back: Settings = serde_json::from_str(json).unwrap();
        assert!(back.tray_enabled);
        assert!(!back.float_enabled);
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

    // ------------------------------------------------------------------
    // 唤醒快捷键判据（与 Electron hotkey.ts::hotkeyProblem 同一套）
    // ------------------------------------------------------------------

    fn hk(ctrl: bool, alt: bool, shift: bool, key: &str) -> Hotkey {
        Hotkey {
            ctrl,
            alt,
            shift,
            key: key.into(),
        }
    }

    #[test]
    fn hotkey_problem_accepts_legal_combinations() {
        // 未设置永远合法
        assert_eq!(hotkey_problem(&hk(false, false, false, "")), "");
        assert_eq!(hotkey_problem(&hk(true, true, true, "   ")), "");
        for h in [
            hk(false, true, false, "Digit4"),
            hk(true, false, false, "KeyA"),
            hk(true, false, true, "F9"),
            hk(false, false, true, "Numpad3"),
            hk(true, false, false, "ArrowUp"),
            hk(false, true, false, "BracketLeft"),
            hk(true, false, false, "F24"),
            hk(true, false, false, "Enter"),
            hk(false, true, false, "NumpadDecimal"),
        ] {
            assert_eq!(hotkey_problem(&h), "", "{} 应被判为合法", hotkey_label(&h));
        }
    }

    #[test]
    fn hotkey_problem_rejects_bare_single_key() {
        for key in ["F9", "F1", "Digit0", "KeyA", "Escape", "Space"] {
            let h = hk(false, false, false, key);
            let p = hotkey_problem(&h);
            assert!(
                p.starts_with("只有单个按键"),
                "{key} 无修饰键必须被拒，实际：{p}"
            );
        }
    }

    #[test]
    fn hotkey_problem_rejects_reserved_system_keys() {
        // 与 RESERVED_HOTKEYS 一一对应；若表漏项这里会直接编译/断言失败
        for h in [
            hk(false, true, false, "Tab"),
            hk(false, true, false, "Space"),
            hk(false, true, false, "F4"),
            hk(false, true, false, "Escape"),
            hk(true, false, false, "Escape"),
            hk(true, false, false, "Space"),
            hk(true, true, false, "Delete"),
        ] {
            let p = hotkey_problem(&h);
            assert!(
                p.contains("已被占用"),
                "{} 是系统占用键，必须被拒，实际：{p}",
                hotkey_label(&h)
            );
        }
        assert_eq!(RESERVED_HOTKEYS.len(), 7, "表项数与上面的对应用例须同步");
    }

    #[test]
    fn hotkey_problem_rejects_settings_shortcut() {
        let p = hotkey_problem(&hk(false, false, true, "F1"));
        assert!(p.contains("Shift+F1"), "Shift+F1 是打开设置键，实际：{p}");
    }

    #[test]
    fn hotkey_problem_rejects_unmappable_keys() {
        // Electron CODE_TO_KEY 里没有的键：Tauri 侧同样不接受（两侧键集必须一致）
        for key in ["CapsLock", "IntlRo", "MediaPlayPause", "F25", "MetaLeft"] {
            let h = hk(true, false, false, key);
            let p = hotkey_problem(&h);
            assert!(
                p.starts_with("不支持的按键"),
                "{key} 不可映射，必须被拒，实际：{p}"
            );
        }
    }

    #[test]
    fn fallback_invalid_hotkey_clears_key_and_leaves_notice() {
        let mut s = Settings {
            wake_hotkey: hk(false, false, false, "F9"),
            tray_enabled: false,
            minimize_on_close: false,
            float_enabled: false,
            hotkey_notice: String::new(),
        };
        assert!(fallback_invalid_hotkey(&mut s), "裸 F9 应触发回退");
        assert!(
            s.wake_hotkey.key.is_empty(),
            "回退后应变成「不设置」，否则启动时仍会注册"
        );
        assert!(
            s.hotkey_notice.contains("F9") && s.hotkey_notice.contains("不再被接受"),
            "设置页提示须说明是哪个键、为什么，实际：{}",
            s.hotkey_notice
        );
    }

    #[test]
    fn fallback_invalid_hotkey_leaves_legal_config_alone() {
        let mut s = Settings {
            wake_hotkey: hk(false, true, false, "Digit4"),
            tray_enabled: true,
            minimize_on_close: false,
            float_enabled: true,
            hotkey_notice: String::new(),
        };
        assert!(!fallback_invalid_hotkey(&mut s));
        assert_eq!(s.wake_hotkey.key, "Digit4", "合法配置不得被改动");
        assert!(s.hotkey_notice.is_empty(), "合法配置不应留提示");
    }

    // 提示是「进程内」的：`#[serde(skip)]` 必须保证它不进 settings.json。
    // 否则用户看到提示后重启，提示会从磁盘复活（而键盘配置早已回退）。
    #[test]
    fn hotkey_notice_is_not_persisted() {
        let s = Settings {
            hotkey_notice: "测试提示".to_string(),
            ..Settings::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(
            !json.contains("hotkey_notice"),
            "提示不得写进 settings.json：{json}"
        );
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert!(back.hotkey_notice.is_empty(), "反序列化应为空（默认）");
    }
}
