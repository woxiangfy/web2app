/// 桌面悬浮球（仅 Windows）的静态契约（**v1.81：Win32 分层窗口 + 位图 alpha 根治毛刺**）。
///
/// 与多页签同一类「全是静默失败」的问题域：
///   · 球面若退回浏览器 CSS 画圆（border-radius / filter / transform）⇒ 边缘毛刺复发，
///     而编译 / 单测 / CI 全绿（v1.77b–v1.80 四轮真机证伪，失败链存档见 `floatwin.rs`）；
///   · 命中若退回 `WM_NCHITTEST` 子类化 / 窗口区域 / 命中圆 ⇒ 圆外点击被误收、区域 1bit
///     硬掩码切掉超采样软边；
///   · 球页 → Rust 事件链（令牌 / 8 事件 / floaticon 协议）若复活 ⇒ 无谓的 IPC 攻击面回来，
///     而原生 WndProc 里根本没有对应的监听。
/// 这里用源码断言把这些前提钉死。
///
/// ⚠️ `floaticon.html` 的页面侧断言**移交 Electron 契约**（`electron-app/test/float-contract.test.ts`）：
///    v1.81 起该文件是 **Electron 专属**（Tauri 侧既不加载也不注入它）。
const FLOAT_RS: &str = concat!(
    include_str!("../floatwin.rs"),
    include_str!("../floatwin/events.rs"),
    include_str!("../floatwin/native.rs"),
    include_str!("../floatwin/tests.rs"),
);
/// app 接线处。
const LIB_RS: &str = include_str!("../lib.rs");
/// 设置模块（右键关功能要写设置 + 同步设置页复选框）。
const SETTINGS_RS: &str = include_str!("../settings.rs");
/// 设置页（双壳单一真源；复选框 id 与 Rust 推送脚本必须一致）。
const SETTINGS_HTML: &str = include_str!("../../dist/settings.html");
/// 托盘（单击球复用其 toggle_window）。
const TRAY_RS: &str = include_str!("../tray.rs");
/// bootstrap（NavCache 类断言的「别再加回来」钉住这两个文件）。
const BOOTSTRAP_RS: &str = include_str!("../bootstrap.rs");
/// 导航 / 窗口会话登记数据枢纽（P2-1 从 bootstrap.rs 抽出）。
const NAV_RS: &str = include_str!("../nav.rs");
/// 运行期配置文档（`DockEdge` 四边枚举 + `FloatWinGeom.edge` 向后兼容的落点）。
const STORE_RS: &str = include_str!("../store.rs");
/// 悬浮球真机探针（只钉它自己那处假主窗的 UDF 契约）。
const FLOATPROBE_RS: &str = include_str!("../floatprobe.rs");

/// 只认真正的代码行（排除 `//` / `*` 注释），防止守卫被注释里的同名文字满足。
fn has_code_line(src: &str, needle: &str) -> bool {
    src.lines().any(|line| {
        let t = line.trim();
        t.contains(needle) && !t.starts_with("//") && !t.starts_with('*')
    })
}

/// [`has_code_line`] 的反向版：代码行（剥纯注释行）里是否出现过 `needle`。用于
/// 「某 API 不得复活」类反向钉死 —— 模块文档 / 存档注释里允许提到名字。
fn code_has_occurrence(src: &str, needle: &str) -> bool {
    src.lines().any(|line| {
        let t = line.trim();
        t.contains(needle) && !t.starts_with("//") && !t.starts_with('*')
    })
}

/// 取某个函数 / 类型签名的**函数体**（签名行起，到第一个顶格 `}` 行为止）。
///
/// 为什么需要它：只断言「文件里出现过某个调用」是**自我满足**的假守卫 —— 文件别处（别的
/// 函数、甚至注释）有同名调用照样能绿。把断言限定进被测函数的函数体内，才真正钉住
/// 「这一条路径做了那件事」。
fn fn_body(src: &str, signature: &str) -> String {
    let Some(start) = src.find(signature) else {
        return String::new();
    };
    let mut out = String::new();
    for line in src[start..].lines() {
        out.push_str(line);
        out.push('\n');
        if line == "}" {
            break;
        }
    }
    out
}

/// ① 球窗 = **Win32 分层窗口**：样式创建时一次定死，不再有 tao / WebView2 建窗链。
///
/// 这是 v1.81 根治毛刺的地基：`WS_EX_LAYERED` 使 `UpdateLayeredWindow` 的**逐像素 alpha**
/// 成为窗口外观本身（圆轮廓 = 位图 alpha，不经任何 CSS / 裁剪 / 窗口区域）；
/// `WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST` + `WS_POPUP` 一次写进
/// `CreateWindowExW`，没有框架会再改它（v1.77 的 `GWL_EXSTYLE` 补丁因此退役）。
#[test]
fn float_ball_is_a_native_layered_window() {
    assert!(
        has_code_line(FLOAT_RS, "CreateWindowExW("),
        "悬浮球必须经 CreateWindowExW 建原生窗口（不再是 WebviewWindowBuilder）"
    );
    assert!(
        has_code_line(
            FLOAT_RS,
            "WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST"
        ),
        "球窗扩展样式必须创建时一次定死：WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST"
    );
    assert!(
        has_code_line(FLOAT_RS, "WS_POPUP,"),
        "球窗必须是 WS_POPUP（无边框 / 无标题栏，窗口 == 球）"
    );
    assert!(
        has_code_line(FLOAT_RS, "w!(\"web2app_float_ball\")"),
        "球窗必须注册独立窗口类 web2app_float_ball（WndProc 挂在它上面）"
    );
    // 反向钉死：WebView2 球窗整条链不得复活（注释存档不算）。
    for banned in [
        "WebviewWindowBuilder",
        "WebviewUrl::",
        ".transparent(true)",
        ".no_redirection_bitmap(true)",
        ".decorations(false)",
        ".skip_taskbar(true)",
        ".always_on_top(true)",
        ".resizable(false)",
    ] {
        assert!(
            !code_has_occurrence(FLOAT_RS, banned),
            "floatwin 不得再出现 {banned}：球窗已原生化，WebView2 建窗链（含其毛刺根因）已整体退役"
        );
    }
}

/// ② 球面 = **4×4 超采样圆盘 + 预乘 ARGB 位图**，经 `UpdateLayeredWindow` 提交。
///
/// 毛刺的根治点全在这一条里：
///   · 圆盘按 4×4 子样本算**覆盖率**作 alpha（`cnt / (SS*SS)`）⇒ 边缘是单级覆盖率渐变，
///     没有中间白底、没有 CSS 裁剪；
///   · 位图 32bpp **预乘**（`AC_SRC_ALPHA` 的硬要求，`premultiply_bgra` 最后统一做）；
///   · 图标按**面积平均**下采样叠加（最近邻会出噪点），`icons/icon.png` 编译期内嵌。
#[test]
fn float_ball_renders_supersampled_premultiplied_bitmap() {
    assert!(
        has_code_line(FLOAT_RS, "const SS: i32 = 4;"),
        "圆盘必须 4×4 超采样（SS=4）：覆盖率 alpha 是软边的来源，去掉就是整像素阶梯"
    );
    assert!(
        has_code_line(FLOAT_RS, "let alpha = cnt / (SS * SS) as f64;"),
        "边缘像素的 alpha 必须是子样本覆盖率（不是二值的圆内/圆外）"
    );
    assert!(
        has_code_line(FLOAT_RS, "fn premultiply_bgra(buf: &mut [u8])"),
        "必须有 premultiply_bgra：UpdateLayeredWindow + AC_SRC_ALPHA 只认预乘 ARGB"
    );
    assert!(
        has_code_line(FLOAT_RS, "fn composite_icon("),
        "图标必须走 composite_icon 面积平均叠加（最近邻下采样会出噪点 / 锯齿）"
    );
    assert!(
        has_code_line(FLOAT_RS, "include_bytes!(\"../../icons/icon.png\")"),
        "球面图标必须编译期内嵌 icons/icon.png（打包替换图标后天然跟随）"
    );
    assert!(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("icons/icon.png")
            .exists(),
        "icons/icon.png 必须存在（悬浮球盘面图标 + bundle 图标的同源文件）"
    );
    // 提交链：DIB 32bpp 顶向下 + UpdateLayeredWindow + ULW_ALPHA + 预乘 blend。
    assert!(
        has_code_line(FLOAT_RS, "biBitCount: 32,") && has_code_line(FLOAT_RS, "biHeight: -h,"),
        "球面位图必须是 32bpp 顶向下 DIB（与像素缓冲区同序，别做行翻转）"
    );
    for needle in [
        "UpdateLayeredWindow(",
        "ULW_ALPHA",
        "BlendOp: AC_SRC_OVER as u8,",
        "AlphaFormat: AC_SRC_ALPHA as u8,",
    ] {
        assert!(
            has_code_line(FLOAT_RS, needle),
            "球面提交链缺少关键代码：{needle}（逐像素 alpha 合成的前提）"
        );
    }
    // 状态差异（hover 提亮 / 半隐藏压暗）只许改平涂色，不得引入任何合成层副作用。
    for banned in ["filter:", "opacity", "radial-gradient", "linear-gradient", "border-radius"] {
        assert!(
            !code_has_occurrence(FLOAT_RS, banned),
            "球面渲染不得再出现 {banned}：那正是 v1.77b–v1.80 毛刺的来源（状态只改平涂色）"
        );
    }
}

/// ③ 命中 = **像素 alpha 自动穿透**，旧的整条命中链不得复活。
///
/// 分层窗口里 alpha=0 的像素把点击原样透传给下层窗口（含跨进程）—— 不需要
/// `WM_NCHITTEST` 子类化、命中圆原子量、「活柱」、窗口区域、WebView2 最小宽 hack。
/// 这些退役 API 一旦回来，圆外点击会被误收、1bit 窗口区域会把超采样软边重新切成阶梯
/// （v1.77b / v1.78 真机证伪），故逐名反向钉死。
#[test]
fn float_hit_testing_is_per_pixel_alpha_only() {
    assert!(
        has_code_line(FLOAT_RS, "WS_EX_LAYERED"),
        "WS_EX_LAYERED 必须在：像素 alpha 穿透是分层窗口的属性，别处没有"
    );
    for banned in [
        // 窗口区域（1bit 硬掩码，切 AA）。
        "SetWindowRgn",
        "CreateEllipticRgn",
        "CreateRectRgn",
        "REGION_PAD",
        "apply_live_region",
        // WM_NCHITTEST 子类化 / 命中圆 / 穿透回执。
        "WM_NCHITTEST",
        "HTTRANSPARENT",
        "EnumChildWindows",
        "SetWindowLongPtrW",
        "update_hit_circle",
        "install_circle_hit_test",
        "hit_circle",
        // 活柱（WebView2 最小宽 hack）。
        "live_offset",
        "live_px",
        // 任务栏补丁（原生样式一次定死后无的放矢）。
        "ITaskbarList",
        "DeleteTab",
        "GWL_EXSTYLE",
        "apply_ball_window_styles",
        // v1.80 的 DComp 路线（已被 UpdateLayeredWindow 取代）。
        "WS_EX_NOREDIRECTIONBITMAP",
    ] {
        assert!(
            !code_has_occurrence(FLOAT_RS, banned),
            "floatwin 代码里不得再出现 {banned}：命中已由像素 alpha 穿透承担，旧命中链复活 = \
                 圆外点击被误收 / 软边被切阶梯（v1.77b–v1.80 失败链，见 floatwin.rs 模块文档）"
        );
    }
}

/// ④ 无球页 / 无 IPC / 无一次性令牌 / 无 floaticon 协议 —— 交互全部在 WndProc 内。
///
/// v1.81 起球窗不是 WebView：没有页面可注入、没有事件可伪造，令牌与 8 事件整条链失去
/// 存在前提。谁把它加回来，就等于凭空恢复一个 IPC 攻击面 + 一套没有调用方的监听。
#[test]
fn float_has_no_page_no_ipc_no_token() {
    for banned in [
        "FLOATICON_SCHEME",
        "handle_floaticon",
        "FLOAT_LABEL",
        "register_float_listeners",
        "token_matches",
        "__web2appFloatToken",
        "push_state",
        "EventKind",
        "FloatMsg",
        "plugin:event|emit",
        "listen(FLOAT_",
        "initialization_script",
        "on_page_load",
    ] {
        assert!(
            !code_has_occurrence(FLOAT_RS, banned),
            "floatwin 代码里不得再出现 {banned}：v1.81 起交互在 WndProc 内，球页 → Rust 事件链已退役"
        );
    }
    // 八个旧事件名一个都不许再定义（常量拆开写，避免本文件的注释自我满足）。
    for ev in [
        concat!("web2app:float-", "down"),
        concat!("web2app:float-", "move"),
        concat!("web2app:float-", "cancel"),
        concat!("web2app:float-", "drag-end"),
        concat!("web2app:float-", "peek-enter"),
        concat!("web2app:float-", "peek-leave"),
        concat!("web2app:float-", "hide"),
        concat!("web2app:float-", "click"),
    ] {
        assert!(
            !FLOAT_RS.contains(ev),
            "floatwin 不得再定义事件名 {ev}：没有球页，也没有对应的监听者"
        );
    }
    // lib.rs 接线处同样不得残留协议注册 / 监听注册 / 球窗标签分支。
    for banned in [
        "FLOATICON_SCHEME",
        "handle_floaticon",
        "floatwin::register_float_listeners",
        "floatwin::FLOAT_LABEL",
    ] {
        assert!(
            !LIB_RS.contains(banned),
            "lib.rs 不得再出现 {banned}（球页 / 协议 / 监听三者整体退役）"
        );
    }
    // NavCache 的「别再加回来」照旧（与球页无关，但同属这条历史线）。
    assert!(
        !BOOTSTRAP_RS.contains("NavCache") && !NAV_RS.contains("NavCache"),
        "bootstrap.rs / nav.rs 的进程级 NavCache 必须保持删除（已无读取方）"
    );
    assert!(
        !LIB_RS.contains("bootstrap::NavCache") && !LIB_RS.contains("nav::NavCache"),
        "lib.rs 不得再 manage NavCache"
    );
}

/// ⑤ capability 文件随球页一起退役：`capabilities/float-icon.json` **必须不存在**。
///
/// 它是「球页要发核心事件」时代的授权文件；没有球页就没有要授权的调用方，留着只会
/// 让权限表与现实漂移。include_str! 已随之移除 —— 这里反向钉住「不得再加回来」。
#[test]
fn float_capability_files_are_gone() {
    let cap_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
    for name in ["float-icon.json", "float-remote.json"] {
        assert!(
            !cap_dir.join(name).exists(),
            "capabilities/{name} 必须删除：球页已退役，capability 文件没有授权对象，留着就是漂移"
        );
    }
}

/// ⑥ 单击球 = **切换**【原来的主窗口】显隐（去抖）。
///
/// 分派在 WndProc 的一次按压收尾 `end_press`：越过拖动阈值 = 拖动（吸附 / 落盘）；
/// 未越过且非系统打断 = 单击；系统打断（`WM_CANCELMODE`）只复位。
/// ⚠️ 单击分支不得退回「只显示」：主窗已可见时点球必须把它藏回去（v1.71 真机缺陷）。
#[test]
fn float_click_toggles_the_original_main_window() {
    let end = fn_body(FLOAT_RS, "fn end_press(canceled: bool)");
    assert!(!end.is_empty(), "找不到 end_press：一次按压的收口没了");
    assert!(
        end.contains("events::on_drag_end(&app)"),
        "end_press 必须把越过阈值的松手派给 on_drag_end（吸附 / 落盘）"
    );
    assert!(
        end.contains("events::on_cancel(&app)"),
        "end_press 必须把系统打断（canceled）派给 on_cancel（只复位，不弹主窗）"
    );
    assert!(
        end.contains("events::on_click(&app)"),
        "end_press 必须把未移动的松手派给 on_click（单击）"
    );
    let click = fn_body(FLOAT_RS, "pub(super) fn on_click(app: &tauri::AppHandle)");
    assert!(!click.is_empty(), "找不到 on_click");
    assert!(
        click.contains("crate::tray::toggle_window(app)"),
        "on_click 必须调 tray::toggle_window 切换主窗显隐"
    );
    assert!(
        !click.contains("crate::tray::show_window(app)"),
        "on_click 不得调 tray::show_window：单击是 toggle（主窗已可见时点球毫无反应是 v1.71 真机缺陷）"
    );
    // 双击去抖：两次 toggle 相互抵消会让主窗「闪现」。
    assert!(
        has_code_line(FLOAT_RS, "const CLICK_DEBOUNCE_MS: u64 = 250;"),
        "必须有单击去抖窗口 CLICK_DEBOUNCE_MS = 250ms（系统双击会送来两次松手）"
    );
    assert!(
        click.contains("CLICK_DEBOUNCE_MS"),
        "on_click 必须真的用上去抖（常量摆着不用 = 双击闪现照旧）"
    );
    // 托盘侧共用的 toggle 必须真切换（hide + 同步 tab 窗 + show）。
    assert!(
        TRAY_RS.contains("pub(crate) fn toggle_window"),
        "tray.rs 必须提供 toggle_window（托盘左键 / 唤醒快捷键 / 单击球共用）"
    );
    for needle in ["window.hide()", "sync_tabs_visibility(app, false)", ".show()"] {
        assert!(
            TRAY_RS.contains(needle),
            "tray::toggle_window 必须真正切换主窗口（找不到 {needle}）"
        );
    }
}

/// ⑦ 右键球 = **关闭悬浮图标功能 + 唤回主窗口**（不是「只把球藏起来」）。
///
/// 真机缺陷（2026-10-11 使用者实测，策略 `tray_enabled:false / minimize_on_close:false /
/// float_enabled:true`）：主窗「关闭」只等于隐藏到球，球是桌面上**唯一**可见入口；
/// 此刻右键把球一藏 —— 进程继续驻留却再也唤不出任何窗口。三步缺一不可：
///   ① `float_enabled` 置 false 并落盘（含同步已打开设置页的复选框）；
///   ② **销毁**球窗；③ 显示主窗口。
#[test]
fn float_right_click_disables_feature_and_restores_main_window() {
    // ① 事件分派：WM_RBUTTONDOWN → on_right_click → disable_float_from_icon。
    let wndproc = fn_body(FLOAT_RS, "unsafe extern \"system\" fn ball_wndproc");
    assert!(!wndproc.is_empty(), "找不到 ball_wndproc：球窗没有窗口过程");
    assert!(
        wndproc.contains("events::on_right_click(&app)"),
        "WM_RBUTTONDOWN 分支必须派给 events::on_right_click"
    );
    let right = fn_body(FLOAT_RS, "pub(super) fn on_right_click(app: &tauri::AppHandle)");
    assert!(
        right.contains("disable_float_from_icon(app)"),
        "on_right_click 必须走 disable_float_from_icon"
    );
    assert!(
        !right.contains("hide_float_window(app)"),
        "on_right_click 不得退回 hide_float_window：只隐藏会留下「进程在跑、桌面再无入口」的死局"
    );

    // ② 三件事必须真的在函数体里（各自单独限定，缺哪件就红哪条）。
    let body = fn_body(FLOAT_RS, "pub(crate) fn disable_float_from_icon");
    assert!(!body.is_empty(), "floatwin.rs 必须有 disable_float_from_icon 函数");
    assert!(
        body.contains("crate::settings::set_float_enabled(app, false)"),
        "右键必须把 float_enabled 置 false 并落盘（否则进程继续驻留、重启后球又回来）"
    );
    assert!(
        body.contains("destroy_float_window(app)"),
        "右键必须**销毁**球窗：只 hide 会继续挡住「所有窗口关闭 ⇒ 退出」，也留下无入口的隐患"
    );
    assert!(
        body.contains("crate::tray::show_window(app)"),
        "右键必须把主窗口显示出来（用户点完右键要看到窗口，而不是一片空桌面）"
    );

    // ③ 设置写入纪律：只翻转这一位 + 落盘成功才写内存态（与 save_settings 同）。
    let setter = fn_body(SETTINGS_RS, "pub(crate) fn set_float_enabled");
    assert!(!setter.is_empty(), "settings.rs 必须有进程内 set_float_enabled（右键路径没有前端载荷）");
    assert!(
        setter.contains("persist_settings(app, &next)?") && setter.contains("*guard = next;"),
        "set_float_enabled 必须先落盘、成功后才写内存态（失败不写内存态）"
    );

    // ④ 设置页复选框必须同步：不同步的话它仍打着勾，用户随手「保存」就把球推回来。
    assert!(
        SETTINGS_RS.contains("fn push_float_enabled_to_settings_window"),
        "settings.rs 必须把新值推给已打开的设置页"
    );
    assert!(
        SETTINGS_RS.contains("getElementById('floatEnabled')")
            && SETTINGS_HTML.contains("id=\"floatEnabled\""),
        "设置页复选框 id（floatEnabled）在 Rust 推送脚本与 dist/settings.html 之间必须逐字一致"
    );
}

/// ⑧ 「销毁」必须真销毁，「关闭」必须只隐藏 —— 两条路径不许互换。
///
/// `WM_CLOSE`（Alt+F4 / 系统关闭）被 [`ball_wndproc`] 拦成隐藏（可从设置页 / 托盘再显示）；
/// 而设置里关掉功能 / 右键关功能走的 `destroy_ball` 必须 `DestroyWindow` 真销毁 ——
/// 用「隐藏」代替销毁是静默降级（留一个隐藏球窗继续挡住「所有窗口关闭 ⇒ 退出」）。
#[test]
fn float_destroy_is_real_and_close_hides() {
    let destroy = fn_body(FLOAT_RS, "pub(super) fn destroy_ball()");
    assert!(!destroy.is_empty(), "找不到 destroy_ball");
    assert!(
        destroy.contains("let _ = DestroyWindow(h);"),
        "销毁悬浮球必须调 Win32 DestroyWindow（真销毁）"
    );
    assert!(
        !destroy.contains("hide_ball()"),
        "destroy_ball 不得退化成 hide_ball：功能关闭时只藏起来是静默降级"
    );
    // WM_CLOSE → 隐藏（Alt+F4 语义），且隐藏走 SW_HIDE。
    let wndproc = fn_body(FLOAT_RS, "unsafe extern \"system\" fn ball_wndproc");
    assert!(
        wndproc.contains("super::hide_float_window(&app)"),
        "WM_CLOSE 分支必须转成 hide_float_window（Alt+F4 不该把球销毁）"
    );
    let hide = fn_body(FLOAT_RS, "pub(super) fn hide_ball()");
    assert!(
        hide.contains("ShowWindow(h, SW_HIDE)"),
        "hide_ball 必须经 ShowWindow(SW_HIDE) 隐藏"
    );
    // 销毁后进程级状态必须清干净（下次建窗不得拿到陈旧 HWND）。
    assert!(
        destroy.contains("BALL_HWND.store(0, Ordering::SeqCst)"),
        "destroy_ball 必须清零 BALL_HWND（陈旧 HWND 会让后续操作打到已销毁窗口）"
    );
    assert!(
        wndproc.contains("WM_NCDESTROY"),
        "WndProc 必须处理 WM_NCDESTROY（外部销毁路径同样要清态）"
    );
}

/// ⑨ lib.rs 接线：托管状态 / 启动按开关建球 / 退出落盘；球窗的关闭语义已下沉 WndProc。
#[test]
fn lib_wires_float_state_lifecycle_and_persistence() {
    assert!(
        has_code_line(LIB_RS, ".manage(floatwin::FloatState::default())"),
        "lib.rs 必须 manage floatwin::FloatState"
    );
    assert!(
        has_code_line(LIB_RS, "if initial_settings.float_enabled {")
            && has_code_line(LIB_RS, "floatwin::show_float_window(app.handle());"),
        "启动时必须按设置 float_enabled 建悬浮球"
    );
    assert!(
        has_code_line(LIB_RS, "floatwin::persist_float_geometry(app_handle);"),
        "退出时必须持久化悬浮球几何"
    );
    // 反向：球窗的 CloseRequested 拦截不该再出现在 lib.rs（WM_CLOSE 在 WndProc 内拦成隐藏）。
    assert!(
        !LIB_RS.contains("floatwin::hide_float_window(app_handle);"),
        "lib.rs 不得再按窗口标签拦 CloseRequested：球窗是原生窗口，关闭语义在 ball_wndproc 里"
    );
}

/// ⑩ 吸附 = **四边就近择一**（上 / 下 / 左 / 右），几何纯函数与落盘枚举两头钉死。
///
/// 左 / 右沿 x 半隐藏，上 / 下沿 y 半隐藏（peek = 球边长一半）；多条边同时入容差取
/// `|gap|` 最小者（治屏幕**角**上的歧义），平局按 left → right → top → bottom 声明顺序。
/// `DockEdge` 序列化小写（`none/left/right/top/bottom`）与 Electron 侧逐字一致。
#[test]
fn float_docks_to_all_four_edges() {
    for sig in [
        "fn decide_edge(",
        "fn expanded_x(",
        "fn collapsed_x(",
        "fn expanded_y(",
        "fn collapsed_y(",
        "fn clamp_x(",
        "fn clamp_y(",
        "fn dock_target_pos(",
    ] {
        assert!(has_code_line(FLOAT_RS, sig), "四边吸附几何缺少纯函数：{sig}");
    }
    // decide_edge 必须真的考虑四条边（不是只判左右）。
    let decide = fn_body(FLOAT_RS, "fn decide_edge(");
    for edge in ["DockEdge::Left", "DockEdge::Right", "DockEdge::Top", "DockEdge::Bottom"] {
        assert!(
            decide.contains(edge),
            "decide_edge 必须考虑 {edge}：吸附已扩为四边就近择一"
        );
    }
    // 松手决策必须走 decide_edge（不是别处另写一份判定）。
    let drag_end = fn_body(FLOAT_RS, "pub(super) fn on_drag_end(app: &tauri::AppHandle)");
    assert!(
        drag_end.contains("decide_edge("),
        "on_drag_end 必须经 decide_edge 判吸附边"
    );
    assert!(
        drag_end.contains("persist_geometry(app)"),
        "on_drag_end 必须落盘几何（含吸附边）"
    );
    // 落盘枚举：五变体小写序列化 + 旧数据缺省 none。
    let en = fn_body(STORE_RS, "pub enum DockEdge {");
    assert!(!en.is_empty(), "store.rs 必须有 DockEdge 枚举");
    for variant in ["None,", "Left,", "Right,", "Top,", "Bottom,"] {
        assert!(en.contains(variant), "DockEdge 必须含变体 {variant}（四边 + 自由态）");
    }
    assert!(
        has_code_line(STORE_RS, "#[serde(rename_all = \"lowercase\")]"),
        "DockEdge 必须小写序列化（none/left/right/top/bottom，与 Electron 侧 float-window.ts 逐字一致）"
    );
    let geom = fn_body(STORE_RS, "pub struct FloatWinGeom {");
    assert!(
        geom.contains("#[serde(default)]") && geom.contains("pub edge: DockEdge,"),
        "FloatWinGeom.edge 必须 #[serde(default)]：旧版落盘文件没有该字段，缺省按 none 解析"
    );
    // 四边用例本身也是契约的一部分（改写断言不是删测试）。
    for case in [
        "fn decide_edge_detects_all_four_edges",
        "fn decide_edge_picks_nearest_edge_and_breaks_corner_ties",
        "fn collapsed_and_expanded_y_mirror_the_x_axis",
        "fn dock_target_pos_moves_only_the_docked_axis",
        "fn dock_edge_serde_round_trips_all_variants",
    ] {
        assert!(
            FLOAT_RS.contains(case),
            "floatwin/tests.rs 必须保留用例 {case}（四边吸附的单测覆盖面不许缩水）"
        );
    }
}

/// ⑪ 盘面底色 **跟随系统深浅色**（白 / 近黑平涂），不再有蓝色渐变。
///
/// 悬浮球是系统级小工具，观感应跟着系统走：读
/// `HKCU\…\Themes\Personalize\AppsUseLightTheme`，`WM_SETTINGCHANGE` /
/// `WM_DWMCOLORIZATIONCHANGED` 时重绘。旧的蓝色渐变（`#6a9bff → #274db8`）是硬编码观感，
/// 与深色桌面格格不入（2026-10-27 需求②）。
#[test]
fn float_base_color_follows_system_theme() {
    assert!(
        has_code_line(FLOAT_RS, "fn apps_use_light_theme() -> bool"),
        "必须有 apps_use_light_theme 读系统深浅色开关"
    );
    assert!(
        has_code_line(FLOAT_RS, "w!(\"AppsUseLightTheme\")"),
        "主题开关必须读注册表 AppsUseLightTheme"
    );
    assert!(
        has_code_line(FLOAT_RS, "WM_SETTINGCHANGE | WM_DWMCOLORIZATIONCHANGED"),
        "主题切换（浅色 ⇄ 深色）必须触发重绘"
    );
    assert!(
        has_code_line(FLOAT_RS, "let (br, bg, bb): (f64, f64, f64) = if dark {"),
        "盘面底色必须按深浅色分叉取平涂色（白 / 近黑）"
    );
    for banned in ["#6a9bff", "#274db8", "radial-gradient", "linear-gradient"] {
        assert!(
            !code_has_occurrence(FLOAT_RS, banned),
            "球面不得再出现 {banned}：蓝色渐变已废弃，底色跟随系统深浅色"
        );
    }
}

/// ⑫ 悬停展开 / 延迟缩回：防边缘抖动的三件套（延迟 + 代际 + 到期时光标校验）。
#[test]
fn float_peek_delay_cursor_check_and_soft_clamp() {
    assert!(
        has_code_line(FLOAT_RS, "const COLLAPSE_DELAY_MS: u64 = 300;"),
        "缩回必须延迟 300ms（边缘进出事件连环触发时防抖）"
    );
    assert!(
        has_code_line(FLOAT_RS, "fn finish_peek_leave(app: &tauri::AppHandle, gen: u64)"),
        "必须有延迟到期的统一收口 finish_peek_leave"
    );
    let finish = fn_body(FLOAT_RS, "fn finish_peek_leave(app: &tauri::AppHandle, gen: u64)");
    assert!(
        finish.contains("cursor_screen_pos()") && finish.contains("point_in_rect("),
        "finish_peek_leave 到期必须校验真实光标位置：光标已在缩回后矩形内则保持展开"
    );
    assert!(
        FLOAT_RS.contains("peek_gen: AtomicU64"),
        "必须有悬停代际计数器，使排队中的旧缩回任务作废"
    );
    assert!(
        has_code_line(FLOAT_RS, "fn point_in_rect(") && has_code_line(FLOAT_RS, "fn clamp_visible("),
        "点包含 / 拖动软钳制必须是纯函数（有单测）"
    );
}

/// ⑬ 拖动 = **物理屏幕绝对坐标 + 抓取偏移**（增量协议会让球半速跟手 + 来回抖）。
///
/// WndProc 的 lparam 是客户区坐标，必须换算成物理屏幕坐标再进状态机；抓取偏移源自
/// **按下点**（不是越过阈值那帧的光标，否则首帧纹丝不动 —— 合成指针真机取证实锤）。
#[test]
fn float_drag_tracks_by_absolute_screen_coords() {
    assert!(
        has_code_line(FLOAT_RS, "fn on_down(app: &tauri::AppHandle, sx: i32, sy: i32)"),
        "必须有 on_down：只暂存按下点，不展开、不置 moving"
    );
    assert!(
        has_code_line(FLOAT_RS, "fn on_move(app: &tauri::AppHandle, cx: i32, cy: i32)"),
        "on_move 必须接收屏幕绝对坐标（物理像素），而不是增量"
    );
    assert!(
        FLOAT_RS.contains("anchor: Mutex<Option<(i32, i32)>>")
            && FLOAT_RS.contains("press: Mutex<Option<(i32, i32)>>"),
        "FloatState 必须暂存按下点 press 与抓取偏移 anchor"
    );
    assert!(
        has_code_line(FLOAT_RS, "fn clear_drag_runtime(app: &tauri::AppHandle)"),
        "必须有 clear_drag_runtime 供 click / cancel / dragend 统一清按压 / 拖动态"
    );
    // lparam（客户区）→ 物理屏幕坐标换算 + 阈值判定。
    let wndproc_helpers = FLOAT_RS;
    assert!(
        has_code_line(wndproc_helpers, "fn screen_point(hwnd: HWND, lparam: LPARAM)")
            && has_code_line(wndproc_helpers, "ClientToScreen(hwnd, &mut p)"),
        "WndProc 坐标必须经 ClientToScreen 换算成物理屏幕坐标（拖动 / 吸附同一坐标系）"
    );
    assert!(
        has_code_line(FLOAT_RS, "const DRAG_THRESHOLD_LOGICAL: f64 = 5.0;"),
        "必须有拖动阈值 DRAG_THRESHOLD_LOGICAL = 5（未越过阈值的松手算单击）"
    );
    // 按下捕获 / 松手释放：拖出窗口外也能收到后续 move / up。
    assert!(
        has_code_line(FLOAT_RS, "let _ = SetCapture(hwnd);")
            && has_code_line(FLOAT_RS, "let _ = ReleaseCapture();"),
        "按压必须 SetCapture、收尾必须 ReleaseCapture（否则拖出球外就丢事件）"
    );
}

/// ⑭ 真机探针入口保留：`inject_page_event` 逐事件驱动**真实**状态机，
///    `dock_state` 把内部吸附态交给探针与读回几何对照断言。
#[test]
fn float_probe_entry_drives_the_real_state_machine() {
    assert!(
        has_code_line(FLOAT_RS, "pub(crate) fn inject_page_event("),
        "必须保留 floatprobe 的事件注入入口 inject_page_event"
    );
    let inject = fn_body(FLOAT_RS, "pub(crate) fn inject_page_event(");
    for ev in [
        "\"down\" =>", "\"move\" =>", "\"cancel\" =>", "\"dragend\" =>",
        "\"peekenter\" =>", "\"peekleave\" =>", "\"hide\" =>", "\"click\" =>",
    ] {
        assert!(inject.contains(ev), "inject_page_event 必须分派事件 {ev}");
    }
    assert!(
        has_code_line(FLOAT_RS, "pub(crate) fn dock_state(app: &tauri::AppHandle)"),
        "必须有 dock_state 只读访问器（探针对照内部状态与真实几何）"
    );
}

/// ⑮ 悬浮球真机探针的「假主窗」也必须设 WebView2 用户数据目录。
///
/// 漏设的后果**不是**「换了个缓存目录」这么轻：真机实测 `WebviewWindowBuilder::build()`
/// **永不返回** —— HWND 已经建出来、主线程此后不再执行任何任务（连排队的 `app.exit`
/// 都送不到），整轮探针只能等看门狗。症状伪装成「WebView2 初始化偶发停摆」。
#[test]
fn float_probe_fake_main_window_sets_data_directory() {
    const NEEDLE: &str = concat!("Some(dir) => b.", "data_directory(dir),");
    assert!(
        has_code_line(FLOATPROBE_RS, NEEDLE),
        "floatprobe 的假主窗必须 data_directory(dir)（与生产主窗同一个 UDF；漏设 = build() 卡死）"
    );
    // 反向对照：把调用挖掉后同一判据必须不再成立，否则这条守卫咬不住人。
    let broken = FLOATPROBE_RS.replace(NEEDLE, "Some(dir) => b,");
    assert_ne!(broken, FLOATPROBE_RS, "反向对照：被测 needle 必须真的存在于源码里");
    assert!(
        !has_code_line(&broken, NEEDLE),
        "反向对照失败：挖掉 data_directory 后判据仍成立 ⇒ 守卫是自欺的"
    );
}

/// ⑯ v1.75：**只启用悬浮图标**（未启用托盘）时，关闭主窗也必须【驻留】而不是真销毁。
///
/// 真机缺陷（2026-10-11 使用者实测，策略 `tray_enabled:false / minimize_on_close:false /
/// float_enabled:true`）：关窗走 `else` 分支 ⇒ 主窗被真销毁，而球窗还活着 ——
///   ① 球窗挡住「所有窗口关闭 ⇒ 退出」，进程留成既无界面也退不掉的僵尸；
///   ② 点球时 `tray::toggle_window` 的 `get_webview_window("main")` 返回 `None`，
///      一个动作都不做 ⇒ 用户看到「点了悬浮图标不回窗口」。
/// 球的产品语义就是「唤回主窗的入口」，故「球在 ⇒ 应用驻留」必须成立。
#[test]
fn main_close_stays_resident_when_float_icon_enabled() {
    // 空白归一化后再匹配：本守卫要钉的是**语义**，不该被换行 / 缩进调整搞红。
    let flat = LIB_RS.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains("let stay_resident = (tray && min) || (cfg!(windows) && float_icon);"),
        "handle_main_close 的驻留判据必须含「Windows 且 float_enabled」：\
             少了它，只启用悬浮图标时关闭窗口会真销毁主窗 —— 球变成点不动的孤儿，进程还退不掉"
    );
    // ⚠️ 托盘那一路仍然必须**两个开关都要**（这条极易被「顺手放宽成 ||」弄坏：
    //    只勾「启用托盘」而不勾「关闭时最小化到托盘」时，关窗应当正常退出）。
    assert!(
        !flat.contains("let stay_resident = tray ||"),
        "托盘驻留不得放宽成只看 tray_enabled：必须 tray_enabled && minimize_on_close"
    );
    // 平台差异必须是 `cfg!` 布尔（只在**值**上分叉），不得用 `#[cfg]` 分叉语句：
    // 后者会让 `float_icon` 在非 Windows 上变成「未使用的变量」⇒ -D warnings 编译失败。
    assert!(
        has_code_line(LIB_RS, "let (tray, min, float_icon) = {"),
        "必须一次性读出三个开关（含 float_enabled）再统一判定驻留"
    );
    assert!(
        !has_code_line(LIB_RS, "#[cfg(windows)] let stay_resident"),
        "驻留判据不得用 #[cfg] 分叉语句（非 Windows 上 float_icon 会变成未使用变量）"
    );
}

/// ⑰ v1.75：设置页「退出程序」必须与托盘「退出」走**同一条** `app.exit()` 路径。
///
/// 启用托盘 / 悬浮图标后「关闭窗口」只等于隐藏 ⇒ 没启用托盘的用户原本**没有任何退出入口**。
/// 这条按钮是那种状态下唯一的出口：命令体必须 `app.exit(0)`（→ `RunEvent::ExitRequested`，
/// **不经过窗口 CloseRequested**，驻留分支不会把退出吞掉）；写成 `window.close()` 就会重现
/// 「点了退出，窗口只是藏起来」的形态。
#[test]
fn settings_page_quit_button_uses_exit_not_window_close() {
    assert!(
        has_code_line(LIB_RS, "app.exit(0);"),
        "设置页「退出程序」必须调 app.exit(0)（与托盘 quit 同路径；window.close() 会被驻留分支吞掉）"
    );
    assert!(
        has_code_line(LIB_RS, "fn quit_app(app: tauri::AppHandle) {"),
        "quit_app 命令体必须存在（settings.html → 垫片白名单 → 注册表 → 命令体 是一条链）"
    );
    assert!(
        SETTINGS_HTML.contains("invoke(\"quit_app\")"),
        "dist/settings.html 必须调用 quit_app（否则「退出程序」按钮点了没反应）"
    );
    assert!(
        SETTINGS_HTML.contains("id=\"quitBtn\""),
        "dist/settings.html 必须有 #quitBtn（元素 id 是与脚本 / 探针的接口）"
    );
}

/// 多页签宿主窗口也必须显式设 WebView2 数据目录。
///
/// ⚠️ 这是一处**真实修掉的偏差**（v1.72）：`create_host_window` 原来漏了这一句，而它恰是
/// 多页签模式下**最先创建 WebView2 环境**的窗口 —— 后果是①宿主的 WebView2 数据落到系统
/// 默认目录（绿色版「不写程序目录之外」被破坏）；②`tabs.rs` 里「宿主是同 UDF 下第一个创建
/// 环境的窗口」的注释前提不再成立。两处都静默，编译 / 单测 / 探针全绿。
///
/// 判据为什么用 `builder.` 作接收者：`create_host_window` 的局部变量正是 `builder`，
/// 而 tab 窗那处是 `window.`、主窗是 `wb.` —— 接收者互不相同，needle 只能来自宿主窗那行。
#[test]
fn tabs_host_window_sets_data_directory() {
    const TABS_RS: &str = concat!(
        include_str!("../tabs.rs"),
        include_str!("../tabs/state.rs"),
        include_str!("../tabs/window.rs"),
        include_str!("../tabs/ipc.rs"),
        include_str!("../tabs/tests.rs"),
    );
    const NEEDLE: &str = concat!("Some(dir) => build", "er.data_directory(dir),");
    assert!(
        has_code_line(TABS_RS, NEEDLE)
            && has_code_line(TABS_RS, "let builder = match crate::config::data_dir(app) {"),
        "create_host_window 必须与主窗 / tab 窗同构显式设置 WebView2 数据目录"
    );
    // 反向对照：挖掉后同判据必须不再成立。
    let broken = TABS_RS.replace(NEEDLE, "Some(dir) => builder,");
    assert_ne!(broken, TABS_RS, "反向对照：被测 needle 必须真的存在于 tabs.rs");
    assert!(
        !has_code_line(&broken, NEEDLE),
        "反向对照失败：挖掉 data_directory 后判据仍成立 ⇒ 守卫是自欺的"
    );
}
