/// 加载层「模式标记」的静态契约（跨平台）。
///
/// 背景（2026-10-09 排查 Android 打包后「转圈十几秒 → 白屏」时定位的**第一条必然路径**，
/// 与网络无关）：启动页由自定义协议提供（`splash://localhost/`，被 wry 改写成
/// `http://splash.localhost/`），它必须让注入的加载层以 **splash** 语义运行 —— 模式由
/// `window.__web2appLoadingMode` 决定，而该标记必须在 `OVERLAY_SCRIPT` **之前**注入
/// （初始化脚本按注册顺序执行）。
///
/// 漏掉标记 / 顺序写反的代价：**编译期零警告、clippy 与 CI 全绿**，只在运行时变成
/// 「启动页被当成目标页跑」⇒ 装上了 overlay 的空白文档硬上限（CAP_MS=15s；启动页 body 里
/// 只有 `<script>`，`bodyEmpty()` 恒真）⇒ 一次正常但偏慢的启动（DNS/TXT 往返 + 代理起停
/// 可能十几秒）被判「页面长时间为空白」。之所以能骗过所有人：`splash.html` 里内联的
/// `__web2appLoadingMode = 'splash'` 在 `<body>` 里，被 document-start 注入抢先；
/// 而 Electron 侧 preload 对内部页面整体跳过注入（只跑内联脚本），顺序恰好成立 ——
/// 于是同一个缺陷只在 Tauri 壳上现形。
/// 被测真源：主窗口建窗处（`run()` 内的闭包）。
const LIB_RS: &str = include_str!("../lib.rs");

/// ⚠️ 字面量必须**拆成两段拼接**：直接写全量会让断言在「标记被删掉」时仍然自我满足
/// （`ipc_command_registry` / `capability_contract` 都踩过这个坑）。
/// 取的是**注入语句本身**而不是裸 hostname —— 后者会命中紧邻的说明注释，守卫就咬不动了。
const MODE_MARKER: &str = concat!(
    "window.__web2appLoadingMode = (location.hostname === '",
    "splash.localhost'"
);
/// 注册加载层那一句（`loading::` 限定符把它与 tabs.rs 的 `crate::loading::` 版本区分开）。
const OVERLAY_REG: &str = concat!(".initialization_script(loading::", "OVERLAY_SCRIPT)");

/// 主窗口：模式标记必须存在，且排在 OVERLAY_SCRIPT 之前。
#[test]
fn main_window_registers_mode_marker_before_overlay() {
    let marker = LIB_RS.find(MODE_MARKER).expect(
        "lib.rs 主窗口缺加载层模式标记 —— 启动页会被当成目标页跑，\
             overlay 的 15s 空白硬上限会把正常启动误报成「页面长时间为空白」",
    );
    let overlay = LIB_RS
        .find(OVERLAY_REG)
        .expect("lib.rs 主窗口应注册 loading::OVERLAY_SCRIPT");
    assert!(
        marker < overlay,
        "模式标记必须注册在 OVERLAY_SCRIPT【之前】：初始化脚本按注册顺序执行，\
             顺序反了标记来不及生效（等价于没写）"
    );
}
