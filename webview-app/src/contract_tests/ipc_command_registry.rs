/// 注册表的唯一真源：本文件源码。
const LIB_RS: &str = include_str!("../lib.rs");

/// 计数用的「针」拆成两段拼接：避免本模块自己的注释 / 断言文案被当成调用点或注册表。
const INVOKE_HANDLER: &str = concat!(".invoke", "_handler(");
const GENERATE_HANDLER: &str = concat!("tauri::generate", "_handler![");

/// 从 `invoke("cmd"` / `invoke('cmd'` 里抽出命令名（前端调用点）。
fn invoked_commands(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(pos) = rest.find("invoke(") {
        // ⚠️ 先推进游标再判断，任何分支都不会造成死循环。
        rest = &rest[pos + "invoke(".len()..];
        let t = rest.trim_start();
        let Some(quote) = t.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            continue;
        };
        let body = &t[1..];
        let Some(end) = body.find(quote) else {
            continue;
        };
        let name = &body[..end];
        // 只收合法的命令名字面量：`invoke(cmd, …)`（变量）之类自然被排除。
        if !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':' || c == '-')
        {
            out.push(name.to_string());
        }
    }
    out
}

/// 从唯一那处 `generate_handler![…]` 里抽出已注册的命令名（取路径末段）。
fn registered_commands() -> Vec<String> {
    let start = LIB_RS
        .find(GENERATE_HANDLER)
        .expect("lib.rs 里必须有 generate_handler! 注册表");
    let body = &LIB_RS[start..];
    let end = body
        .find("]);")
        .expect("注册表必须以 `]);` 收尾（否则本测试扫不到尾）");
    body[..end]
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//") && !l.starts_with("#["))
        .map(|l| l.trim_end_matches(','))
        // 只留「就是一个 Rust 路径」的行 —— 排掉 `tauri::generate_handler![` 自身之类的噪声。
        .filter(|l| {
            l.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
        })
        .map(|p| p.rsplit("::").next().unwrap_or(p).to_string())
        .collect()
}

/// 平台专属的前端页面：多页签标签栏 —— 它们调用的命令本身也是 `#[cfg(windows)]` 注册的。
///
/// ⚠️ `tab_title.js` **不在**这里：它注入在远程 tab 页面里，2026-09-27 起只发核心
/// 事件（`plugin:event|emit`）、不调任何应用命令（远程 origin 调应用命令必被 ACL 拒）。
/// 它的契约在 `capability_contract` 模块里按「事件名 + capability + token」另立守卫。
///
/// ⚠️⚠️ 平台差异只允许落在**数据**上，绝不能落在**可变性**上。早先这里写的是
/// `let mut sources = vec![跨平台页面];` + `#[cfg(windows)] { sources.push(页签页面) }`，
/// 而那句 `push` 在**非 Windows 目标**（CI 的 Ubuntu `lint-and-test` / Android 作业）
/// 上根本不参与编译 ⇒ `mut` 变成未使用 ⇒ `-D warnings` 下直接编译失败
/// （2026-09-23 CI 报 `error: variable does not need to be mutable`）。
/// 改写成「cfg 分叉只决定数组内容」后，任何目标上都不可能再出现 unused_mut。
#[cfg(windows)]
const PLATFORM_SOURCES: &[(&str, &str)] = &[("src/shell.html", include_str!("../shell.html"))];
/// 非 Windows 目标没有页签页面（命令未注册，验了必然假失败）⇒ 空表。
#[cfg(not(windows))]
const PLATFORM_SOURCES: &[(&str, &str)] = &[];

/// 待校验的全部前端页面：跨平台页面（设置页）+ 平台专属页面（页签）。
fn frontend_sources() -> Vec<(&'static str, &'static str)> {
    std::iter::once((
        "dist/settings.html",
        include_str!("../../dist/settings.html"),
    ))
    .chain(PLATFORM_SOURCES.iter().copied())
    .collect()
}

/// ① 调用点唯一：`Builder::invoke_handler` 调第二次就是「命令集体消失」。
#[test]
fn exactly_one_invoke_handler_call_site() {
    let n = LIB_RS.matches(INVOKE_HANDLER).count();
    assert_eq!(
        n, 1,
        "lib.rs 里出现了 {n} 次 invoke_handler 调用。它是【替换】不是【追加】——\
             第二次调用会静默丢掉前一次注册的全部命令（Windows 上表现为设置页保存报\
             「Command … not found」）。平台差异请写成命令项上的 #[cfg(...)] 属性。"
    );
}

/// ② 前端真的会调的每个命令，都必须能在注册表里找到；否则运行时就是一条「命令不存在」。
#[test]
fn every_frontend_invoked_command_is_registered() {
    let registered = registered_commands();
    // 页面清单见 `frontend_sources()`（平台差异只影响清单内容，不影响可变性）。
    let sources = frontend_sources();

    let mut missing: Vec<String> = Vec::new();
    for (file, src) in sources {
        let found = invoked_commands(src);
        // ⚠️ 先堵「假绿」：抽不出任何命令时下面的子集断言会无条件通过。
        assert!(
            !found.is_empty(),
            "{file} 里一个 invoke(\"…\") 都没抽到 —— 要么前端改了调用姿势，\
                 要么提取器坏了；两种情况都必须人工确认，不能让本条断言静默通过"
        );
        for cmd in found {
            if !registered.contains(&cmd) {
                missing.push(format!("{file} → {cmd}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "前端调用了未注册的 IPC 命令（运行时报「Command … not found」）：{missing:?}；\
             已注册：{registered:?}"
    );
}

/// ③ 注册表本身：基础命令跨平台必须在；多页签命令在 Windows 上必须在。
#[test]
fn required_commands_are_registered() {
    let registered = registered_commands();
    let base = [
        "get_settings",
        "save_settings",
        "close_settings_window",
        "clear_web_data",
        "history_nav",
        // 设置页「立即显示悬浮窗」按钮：跨平台注册（settings.html 是跨平台资源），
        // 命令体在非 Windows 是空操作，见 lib.rs 的 show_float_window。
        "show_float_window",
        // 设置页「退出程序」按钮（v1.75）：常驻后台（托盘 / 悬浮图标）时**唯一**的退出入口，
        // 同样必须跨平台注册（settings.html 是跨平台资源）。
        "quit_app",
    ];
    for name in base {
        assert!(
            registered.iter().any(|r| r == name),
            "IPC 命令 {name} 未注册（已注册：{registered:?}）"
        );
    }

    // 桌面端：打开设置窗（多页签标签栏 shell.html 会调它；Android 没有设置窗）。
    #[cfg(desktop)]
    {
        let name = "open_settings";
        assert!(
            registered.iter().any(|r| r == name),
            "IPC 命令 {name} 未注册（已注册：{registered:?}）"
        );
    }

    #[cfg(windows)]
    for name in [
        "create_tab",
        "switch_tab",
        "close_tab",
        "get_tabs",
        "minimize_host",
        "close_host",
    ] {
        assert!(
            registered.iter().any(|r| r == name),
            "多页签 IPC 命令 {name} 未注册（已注册：{registered:?}）"
        );
    }
}
