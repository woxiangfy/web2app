/// 启动信息实现的**归属**契约（架构审查 2026-09-29 P1-1，2026-10-12 落地）。
///
/// 背景：`log_startup` + 三个 `describe_*` 是约 120 行纯格式化职责，原本混在装配中枢
/// `lib.rs`（装配 + IPC 命令 + 启动日志三合一），而 `debug_log` 才是「诊断」的职责所有者。
/// P1-1 把它们整体迁走后，`lib.rs` 只留一行 `debug_log::log_startup(...)`。本守卫钉死搬迁
/// 结果，防止以后有人在 `lib.rs` 再抄出第二份启动日志 —— 两份真源漂移时，两壳日志字段会
/// 静默对不上，且编译期零告警、clippy 全绿。
const LIB_RS: &str = include_str!("../lib.rs");
const DEBUG_LOG_RS: &str = include_str!("../debug_log.rs");

/// 同其它模块：只认代码行，排除注释（针写在注释里不该让守卫变绿）。
fn has_code_line(src: &str, needle: &str) -> bool {
    src.lines().any(|line| {
        let t = line.trim();
        t.contains(needle) && !t.starts_with("//") && !t.starts_with('*')
    })
}

#[test]
fn startup_log_lives_in_debug_log_and_not_in_lib() {
    // ① 唯一实现必须在 debug_log.rs，且对模块外可见（lib.rs 只能经限定路径调用）。
    assert!(
        has_code_line(DEBUG_LOG_RS, "pub(crate) fn log_startup("),
        "log_startup 必须定义在 debug_log.rs（诊断职责的所有者，架构审查 P1-1）"
    );

    // ② lib.rs 不得再持有四个函数中的任何一个定义（抄回第二份 = 真源分裂）。
    for banned in [
        "fn log_startup(",
        "fn describe_target(",
        "fn describe_proxy(",
        "fn describe_tabs(",
    ] {
        assert!(
            !has_code_line(LIB_RS, banned),
            "lib.rs 不得再定义 {banned}：启动日志的唯一真源已迁到 debug_log.rs（P1-1），\
                 抄第二份会让日志字段静默漂移"
        );
    }

    // ③ 装配处只允许经限定路径调这一次（不靠 use 把名字重新拉回 lib.rs）。
    assert!(
        has_code_line(LIB_RS, "debug_log::log_startup("),
        "lib.rs 必须以 debug_log::log_startup(...) 调用启动信息（装配处只留一行）"
    );
}
