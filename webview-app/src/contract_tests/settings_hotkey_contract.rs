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
