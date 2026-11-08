//! 设置页「唤醒快捷键」交互契约（`dist/settings.html`，两壳共用）。
//!
//! 存在理由：这一块出过的事故全是**静默**的 —— 界面自洽地显示着一个错误说法，
//! 既有测试全绿，只有用户按下去、或者保存之后才发现不对：
//!   ① 「手动选择」偷偷补 `alt:true`（选 F9 实存 Alt+F9，界面只写「F9」），
//!      而「点击捕获」允许**裸单键**（按 F9 就是 F9）—— 两条入口语义相反、显示文案相同；
//!   ② 主键为空时顺手清掉修饰键 —— 用户最自然的「先勾修饰键、再选主键」等于白勾；
//!   ③ 列表之外的键（捕获可得到 `ArrowUp` / `Numpad3`）没补进下拉 ⇒ `sel.value` 落空，
//!      下拉显示「什么都没选」而预览写着 `Ctrl+↑`，用户碰一下下拉就把捕获结果丢掉；
//!   ④ `get_settings` 失败后界面全是默认值，此时放行保存 = 把用户真实配置静默覆盖成默认。
//!
//! 现行规则只有一条：**组合键 = 修饰键（Ctrl / Alt / Shift 至少一个）+ 主键**，
//! 主键留空 = 不设置；`captured` 是唯一状态，「手动组合」与「捕获」都只是它的编辑器，
//! 改任一处都必须把另一处一起重画。
//!
//! ⚠️ 为什么在 Rust 侧做静态断言：`settings.html` 是双壳共用资源，Electron 有探针
//! （真实点击 / 合成按键，见 `probe-single.ts` 13b 段），但 **Tauri 侧本机跑不了 GUI 探针**
//! ⇒ 这些「少一行就静默」的接线必须在静态层也钉一遍。行为断言仍以探针为准。
//!
//! ⚠️ 所有断言一律走 `has_code_line` / `strip_line_comments`（剥掉 `//` 之后的内容）：
//!    直接 `contains` 会被**注释**满足 —— 本文件第一版就这么假绿过一次
//!    （`// reverse-check: btn.disabled = true;` 照样命中，测试全过而功能已删）。

use super::{count_code_lines, has_code_line, strip_line_comments};

const SETTINGS_HTML: &str = include_str!("../../dist/settings.html");
/// 本 crate 的设置实现（判据 `hotkey_problem` / 启动回退都在这里）。
const SETTINGS_RS: &str = include_str!("../settings.rs");
/// 跨壳基准：Electron 主进程的判据与 accelerator 键表。**只读**它作为参照，
/// 本仓库不改 electron-app（那边的未提交改动属于同事）。
const ELECTRON_HOTKEY_TS: &str = include_str!("../../../electron-app/src/main/hotkey.ts");

/// 从 `start_marker` 起、到 `end_marker` 止的块里，逐行抽键值字面量（键可带引号也可裸写）：
/// `'Alt+Tab': '系统切换窗口',` / `"Alt+Tab": "系统切换窗口",` / `NumpadDecimal: 'numdec',`。
/// 注释先被剥掉，避免把说明文字里的引号当成数据；`Array.from({ length: 26 }, …)`
/// 这类代码行（键里带括号 / 空格 / 点）会被跳过。
fn literal_pairs(src: &str, start_marker: &str, end_marker: &str) -> Vec<(String, String)> {
    let code = strip_line_comments(src);
    let start = code
        .find(start_marker)
        .unwrap_or_else(|| panic!("找不到 {start_marker}"));
    let rest = &code[start..];
    let end = rest
        .find(end_marker)
        .unwrap_or_else(|| panic!("{start_marker} 后找不到结束标记"));
    let mut out = Vec::new();
    for line in rest[..end].lines() {
        let t = line.trim();
        let Some(colon) = t.find(':') else {
            continue;
        };
        let mut key = t[..colon].trim().to_string();
        if key.len() >= 2 {
            let quoted = (key.starts_with('\'') && key.ends_with('\''))
                || (key.starts_with('"') && key.ends_with('"'));
            if quoted {
                key = key[1..key.len() - 1].to_string();
            }
        }
        if key.is_empty()
            || key
                .chars()
                .any(|c| matches!(c, '(' | ')' | '{' | '}' | '.' | ' ' | '='))
        {
            continue;
        }
        // 值 = 冒号之后第一段引号内容（两侧都写成字符串字面量）
        let tail = &t[colon + 1..];
        let mut buf = String::new();
        let mut in_quote = false;
        let mut quote = ' ';
        let mut value = None;
        for ch in tail.chars() {
            if !in_quote {
                if ch == '\'' || ch == '"' {
                    in_quote = true;
                    quote = ch;
                }
            } else if ch == quote {
                value = Some(std::mem::take(&mut buf));
                break;
            } else {
                buf.push(ch);
            }
        }
        if let Some(v) = value {
            out.push((key, v));
        }
    }
    out
}

/// ⑥ 系统占用表**三处逐字一致**：Rust（主进程）/ Electron（主进程）/ 共用设置页。
///
/// 存在理由：这张表是「注册了也会被系统抢走」的清单，任何一处落后都会出现
/// 「Tauri 拦了、Electron 放行」或「页面说能存、主进程拒绝」—— 两侧行为差异，
/// 而各自的测试都绿。
#[test]
fn reserved_hotkey_table_is_identical_in_three_places() {
    let rust: Vec<(String, String)> = crate::settings::RESERVED_HOTKEYS
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let electron = literal_pairs(ELECTRON_HOTKEY_TS, "const RESERVED_HOTKEYS", "\n}");
    let page = literal_pairs(SETTINGS_HTML, "const HK_RESERVED = {", "\n  };");
    assert!(
        !rust.is_empty() && rust.len() == electron.len() && rust.len() == page.len(),
        "三处表项数必须一致：Rust {} / Electron {} / 页面 {}",
        rust.len(),
        electron.len(),
        page.len()
    );
    assert_eq!(
        rust, electron,
        "Rust 与 Electron 的系统占用表（含原因文案）已漂移"
    );
    assert_eq!(
        rust, page,
        "Rust 与共用设置页的系统占用表（含原因文案）已漂移"
    );
}

/// ⑦ 五条判据 + accelerator 键集两侧同一套（Tauri 主进程对齐 Electron 主进程）。
///
/// 判据顺序：未设置 → 裸单键 → 系统占用 → `Shift+F1` → 无法映射。
/// 注：页面（两壳共用）只做前三类 + `Shift+F1` 的即时提示，「无法映射」只有主进程能做到
/// （Electron `CODE_TO_KEY` / Tauri `is_supported_key`）。
#[test]
fn hotkey_criteria_and_key_set_do_not_drift_between_shells() {
    // Electron 侧的五条判据（原文锚点）
    for needle in [
        "if (!h.ctrl && !h.alt && !h.shift) {",
        "const takenBy = RESERVED_HOTKEYS[label]",
        "if (label === SETTINGS_SHORTCUT)",
        "if (!CODE_TO_KEY[key])",
    ] {
        assert!(
            has_code_line(ELECTRON_HOTKEY_TS, needle),
            "Electron 判据锚点变了：{needle} —— 先确认它是否仍等价，再同步本文件与 Tauri 侧"
        );
    }
    // Tauri 侧的同一套判据（顺序即执行顺序）
    for needle in [
        "if !h.ctrl && !h.alt && !h.shift {",
        "RESERVED_HOTKEYS.iter().find",
        "if label == \"Shift+F1\" {",
        "if !is_supported_key(key) {",
    ] {
        assert!(
            has_code_line(SETTINGS_RS, needle),
            "Tauri 判据缺一条：{needle}（判据必须与 Electron hotkeyProblem 同一套、同一顺序）"
        );
    }
    // 同一判据必须同时用于「保存」和「启动回退」：只写一处 ⇒ 另一条路径静默放行旧配置
    let uses = count_code_lines(SETTINGS_RS, "hotkey_problem(");
    assert!(
        uses >= 3,
        "hotkey_problem 至少 3 处使用（定义 + save_settings + fallback_invalid_hotkey），实际 {uses}"
    );

    // accelerator 键集：Electron 的静态列名项必须与 Tauri 的 SUPPORTED_EXTRA_KEYS 完全一致
    let electron_static: Vec<String> =
        literal_pairs(ELECTRON_HOTKEY_TS, "const CODE_TO_KEY", "\n}")
            .into_iter()
            .map(|(k, _)| k)
            .collect();
    let mut rust_extra: Vec<String> = crate::settings::SUPPORTED_EXTRA_KEYS
        .iter()
        .map(|k| k.to_string())
        .collect();
    let mut electron_sorted = electron_static.clone();
    electron_sorted.sort();
    rust_extra.sort();
    assert_eq!(
        rust_extra, electron_sorted,
        "键集漂移：SUPPORTED_EXTRA_KEYS 与 Electron CODE_TO_KEY 的列名项必须逐项相同"
    );
    // 由前缀区间生成的几族（26 字母 / 10 主键数字 / 10 小键盘数字 / F1–F24）也要两侧对得上
    for marker in ["length: 26", "length: 24"] {
        assert!(
            has_code_line(ELECTRON_HOTKEY_TS, marker),
            "Electron 键集族标记消失：{marker}"
        );
    }
    let digits = count_code_lines(ELECTRON_HOTKEY_TS, "length: 10");
    assert!(
        digits >= 2,
        "主键数字与小键盘数字两族都应是 length: 10，实际 {digits} 处"
    );

    let prob = |key: &str| {
        crate::settings::hotkey_problem(&crate::settings::Hotkey {
            ctrl: true,
            alt: false,
            shift: false,
            key: key.to_string(),
        })
    };
    for key in ["KeyZ", "Digit9", "Numpad9", "F24", "ArrowUp", "Backquote"] {
        assert_eq!(prob(key), "", "{key} 在两侧都应可映射");
    }
    for key in ["CapsLock", "F25", "IntlRo", "MediaPlayPause"] {
        assert!(
            prob(key).contains("不支持的按键"),
            "{key} 在两侧都应被拒（Electron CODE_TO_KEY 里没有它）"
        );
    }
}

/// ⑧ 旧配置被判非法时：启动回退成「不设置」+ 设置页可见提示 + 保存后提示消失。
///
/// 三种静默失效都要堵住：① 不回退 ⇒ 旧裸键继续占满输入；② 回退但无提示 ⇒
/// 用户以为程序坏了；③ 提示落盘或永不清 ⇒ 改好之后还在报旧问题。
#[test]
fn invalid_old_hotkey_falls_back_with_visible_notice() {
    assert!(
        has_code_line(
            SETTINGS_RS,
            "fn fallback_invalid_hotkey(s: &mut Settings) -> bool {"
        ),
        "缺少启动回退：旧配置不合法时必须按「不设置」处理（不注册），而不是硬注册"
    );
    assert!(
        has_code_line(SETTINGS_RS, "fallback_invalid_hotkey(&mut s);"),
        "load_settings 必须调用回退（只定义不调用 = 旧配置照旧注册）"
    );
    assert!(
        has_code_line(SETTINGS_RS, "#[serde(skip)]"),
        "提示必须 `#[serde(skip)]`：写进 settings.json 后重启会「复活」一条旧提示"
    );
    assert!(
        has_code_line(SETTINGS_RS, "let hotkey_notice = s.hotkey_notice.clone();"),
        "get_settings 必须把提示发给页面（SettingsView），否则界面无从展示"
    );
    // 页面：读取 → 渲染 → 保存成功归零
    assert!(
        has_code_line(SETTINGS_HTML, "loadNotice = s.hotkey_notice || \"\";"),
        "设置页必须读取 hotkey_notice"
    );
    assert!(
        has_code_line(SETTINGS_HTML, "st.textContent = loadNotice;"),
        "提示必须出现在快捷键卡片的状态行（只存不发 = 静默）"
    );
    let resets = count_code_lines(SETTINGS_HTML, "loadNotice = \"\";");
    assert!(
        resets >= 2,
        "loadNotice 至少两处归零（初始化 + 保存成功），实际 {resets}"
    );
}

/// ① 界面必须把「组合键」显式构造出来（修饰键 + 主键 + 预览），而不是一个裸下拉。
///
/// 漏掉的后果：退回「下拉选主键」那种形态 —— 用户看不出选出来的到底是不是组合键，
/// 而这正是本次返工的起因（「手动选择存在歧义，是组合快捷键还是单个按键？」）。
#[test]
fn hotkey_ui_is_an_explicit_combo_builder() {
    for id in ["hkCtrl", "hkAlt", "hkShift", "keySelect", "hkPreview"] {
        let needle = format!("id=\"{id}\"");
        assert!(
            has_code_line(SETTINGS_HTML, &needle),
            "设置页必须有 {needle}：三个修饰键 + 主键下拉 + 预览框是「组合键」这个词表的全部零件"
        );
    }
    // 三个修饰键必须由同一张表驱动（新增/改名只改一处）
    assert!(
        has_code_line(SETTINGS_HTML, "const HKMODS = ["),
        "修饰键必须集中在 HKMODS 表里：分散成三段硬编码后，加一个修饰键要改三处、必漏"
    );
}

/// ② 单键与系统占用组合必须在**保存之前**拦下，且拦截点只有一个 `hkProblem`。
///
/// 漏掉的后果：
///   · 裸单键 → `globalShortcut.register('F9')` 把系统里每次 F9 都变成「切窗口」，
///     用户在别处再也打不出这个键；
///   · `Ctrl+Space`（中文 Windows 的中英切换）→ 注册成功后**打断用户打字**，比不生效更糟。
/// 两个入口（手动 / 捕获）与保存必须共用同一个判据，否则出现「手动能存、捕获被拒」。
#[test]
fn hotkey_invalid_combos_are_rejected_before_saving() {
    assert!(
        has_code_line(SETTINGS_HTML, "function hkProblem(hk)"),
        "必须有单一判据 hkProblem：校验散在多处时，两条入口迟早给出不同答案"
    );
    assert!(
        has_code_line(SETTINGS_HTML, "if (!hk.ctrl && !hk.alt && !hk.shift)"),
        "hkProblem 必须拒绝裸单键（缺这条 ⇒ 单键会被当成合法快捷键存下去）"
    );
    for combo in ["\"Ctrl+Space\"", "\"Alt+Tab\"", "\"Alt+F4\""] {
        assert!(
            has_code_line(SETTINGS_HTML, combo),
            "HK_RESERVED 必须包含 {combo}（系统/输入法占用，注册了也会被抢走）"
        );
    }
    // 两条入口 + 保存处都必须调它（少一处 = 那条路径静默收下非法值）
    let uses = count_code_lines(SETTINGS_HTML, "hkProblem(");
    assert!(
        uses >= 4,
        "hkProblem 至少要有 4 处使用（定义 + 捕获 + 渲染 + 保存），实际 {uses} 处"
    );
}

/// ③ 「当前生效」与「待保存」必须是两件事：左边 kbd 只在保存成功后变。
///
/// 漏掉的后果：`hkCurrent` 跟着勾选一起跳 ⇒ 它一边写着「当前生效」，一边报的是草稿，
/// 等于在说谎；用户以为已经生效，其实还得点保存。
#[test]
fn hotkey_current_and_pending_are_separate() {
    assert!(
        has_code_line(SETTINGS_HTML, "let savedHotkey = {"),
        "必须有独立的 savedHotkey：它才是「当前生效」的真源"
    );
    assert!(
        has_code_line(SETTINGS_HTML, "cur.textContent = hkLabel(savedHotkey);"),
        "「当前生效」必须读 savedHotkey，不能读 captured（读 captured = 报草稿当现状）"
    );
    assert!(
        has_code_line(SETTINGS_HTML, "savedHotkey = {"),
        "保存成功路径必须把 captured 落到 savedHotkey（否则保存完「当前生效」还是旧值）"
    );
    // 手动组合的四个输入必须收敛到同一个处理函数
    assert!(
        has_code_line(SETTINGS_HTML, "function readManual()"),
        "手动组合的四个输入必须收敛到 readManual（分散处理 ⇒ 勾选与下拉各说各话）"
    );
}

/// ④ 列表之外的键必须能补进下拉 —— 且补的时机在**每次渲染**，不是初始化一次。
///
/// 漏掉的后果：捕获到 `ArrowUp` 之后 `sel.value` 落空 ⇒ `selectedIndex = -1`
/// （下拉看上去「什么都没选」），而预览写着 `Ctrl+↑`；用户随后碰一下下拉，
/// `readManual` 就把捕获结果整条丢掉，且看起来像是自己选了「无」。
#[test]
fn hotkey_out_of_list_key_is_selectable() {
    assert!(
        has_code_line(SETTINGS_HTML, "function ensureKeyOption(code)"),
        "必须有 ensureKeyOption：捕获 / 历史配置都可能给出常用列表之外的键"
    );
    assert!(
        has_code_line(SETTINGS_HTML, "ensureKeyOption(hk.key);"),
        "ensureKeyOption 必须在 renderHotkey 里调用 —— 只在初始化补一次会漏掉「运行中捕获」这条路径"
    );
    // 位置：必须在 renderHotkey 内、且排在 sel.value 之前（先补选项再选中）
    let code = strip_line_comments(SETTINGS_HTML);
    let render = code
        .find("function renderHotkey()")
        .expect("renderHotkey 定义丢了");
    let ensure = code
        .find("ensureKeyOption(hk.key);")
        .expect("renderHotkey 里没有 ensureKeyOption(hk.key)");
    let assign = code
        .find("sel.value = hk.key || \"\";")
        .expect("renderHotkey 里没有 sel.value 赋值");
    assert!(
        render < ensure && ensure < assign,
        "ensureKeyOption 必须夹在 renderHotkey 内、且在 sel.value 之前（先补选项再选中）"
    );
}

/// ⑤ `get_settings` 失败时**禁止保存**：此时界面全是默认值，写回去就是静默清零。
///
/// 漏掉的后果（实测过）：配置文件被手工改坏 → 设置窗读到错误 → 用户只想改个托盘开关
/// → 点保存 → `wake_hotkey` 与托盘开关一起被写成默认值，界面还回一句「已保存」。
/// 这与「升级后首次保存把配置清零」是同一类事故：读失败被当成了「用户想清空」。
#[test]
fn hotkey_save_is_blocked_when_settings_failed_to_load() {
    assert!(
        has_code_line(SETTINGS_HTML, "let loadFailed = false;"),
        "必须有 loadFailed 标志（读失败与「用户真的想清空」必须能区分）"
    );
    assert!(
        has_code_line(SETTINGS_HTML, "loadFailed = true;"),
        "get_settings 的 catch 必须置 loadFailed（否则失败与成功的界面长得一样）"
    );
    // catch 块里必须**立刻**禁用保存按钮（只是提示而不禁用，用户照样会点）
    let code = strip_line_comments(SETTINGS_HTML);
    let catch_at = code
        .find("loadFailed = true;")
        .expect("catch 里没有 loadFailed = true");
    let tail = &code[catch_at..(catch_at + 400).min(code.len())];
    assert!(
        tail.contains("btn.disabled = true;"),
        "加载失败必须立刻禁用保存按钮（catch 块内 400 字符内应出现 btn.disabled = true）"
    );
    // 保存处理函数里还要有第二道（按钮被重新启用的那天）
    assert!(
        has_code_line(SETTINGS_HTML, "if (loadFailed) {"),
        "保存处理函数里必须有 loadFailed 守卫：按钮 disabled 只是第一道，别让它成为唯一一道"
    );
    // 卡片内状态行也不能报假情报：读失败 ≠ 未设置，真实值只是「未知」
    assert!(
        has_code_line(SETTINGS_HTML, "当前快捷键未知"),
        "读失败时状态行必须说「未知」而不是「未设置」：后者会让用户以为自己的快捷键丢了"
    );
}
