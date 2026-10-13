//! 移动端底部导航栏（`mobile_bar`，仅移动端）。
//!
//! 移动端没有系统标题栏可自绘（桌面端那条注入的 `titlebar.js` 只在 Windows 注入），
//! 但纯网页壳里「回到首页 / 后退 / 前进」这三类动作在手机上**没有系统级入口**：
//! Android 只有返回键（且本仓库已把它限制成「不退回内置启动页」，见
//! `.github/scripts/patch-android-back.py`），没有前进、也没有「回到入口」。
//! `mobile_bar=true` 时就在屏幕**底部**常驻一条导航条补上这几个动作。
//!
//! 前端真源是 `src/mobilebar.js`，本模块只负责三件事：
//!   1. 把「底栏高度 / 是否有页签可切」两个值注入给它（**必须在它之前**注册）；
//!   2. 缓存启动期解析出的首页导航信息，供「首页」按钮复用；
//!   3. 监听前端发来的核心事件并落地（远程页面调不了应用命令，只能发事件 —— 见
//!      `src/mobilebar.js` 文件头与 `capabilities/default.json` 的 `core:event:allow-emit`）。
//!
//! ⚠️ 模块按 `#[cfg(any(mobile, test))]` 编译：
//!   · 生产构建里它**只在移动端存在**，桌面目标（Windows Tauri / Electron 绿色版）不编译
//!     这条路径 —— 这与「配了 `mobile_bar=true` 却被忽略」有本质区别：后者是静默行为，
//!     前者是「这条路根本不存在」，不可能在桌面端长出一条底栏；
//!   · `test` 那一半是为了让**本机 `cargo test` 也能跑到本模块的纯逻辑单测**
//!     （移动端是真机才能验的目标，本机跑不到；判据若只能靠真机，就等于没有护栏）。
//!     与 `bootstrap.rs` 里那组 `#[cfg(any(not(windows), test))]` 同一条写法。
//! ⚠️ 因此本模块里**只能**用跨平台 API，不得引用任何 `#[cfg(mobile)]` / `#[cfg(windows)]`
//!    的项（那样会让桌面端的测试构建编译不过）。

use std::sync::Mutex;

use tauri::{AppHandle, Listener, Manager};

use crate::bootstrap::Navigation;
use crate::config::TABS_ENABLED;

/// 底栏内容区高度（**CSS 逻辑像素**，不含手势导航条的安全区）。
///
/// 安全区那一段由页面里的 `env(safe-area-inset-bottom, 0px)` 自行补上 —— 它随设备与
/// 导航方式（三大键 / 手势）变化，编译期写死不了。
/// ⚠️ 这是**唯一真源**：前端 `mobilebar.js` 的底栏 CSS、底部让位高度、页面贴底元素上移的
///    距离全部来自这里（经 [`config_script`] 下发），改高度只改这一处。
pub(crate) const BAR_HEIGHT: i32 = 52;

/// 编译期钉住底栏高度的取值范围（比运行时断言更早一步，也不可能被人跳过）：
///   · ≥ 44：触摸目标的最低要求（Apple HIG / Material 同为 44–48dp），矮于此会「点不准」；
///   · ≤ 72：再高就明显占屏幕（手机竖屏常见 640–900 CSS px，72 已经是上限附近）。
const _: () = assert!(BAR_HEIGHT >= 44 && BAR_HEIGHT <= 72);

/// 当前构建是否真的有「可切换的页签」—— 底栏「列表」按钮的**唯一**显示判据。
///
/// ⚠️⚠️ 为什么要这一层判断：**移动端目前没有多页签**（`mod tabs` 整体 `#[cfg(windows)]`，
/// `TABS_ENABLED` 也在非 Windows 上恒为 false）。若无条件画出「列表」按钮，用户点下去什么
/// 都不会发生 —— 「按钮在、点了没反应」比「没有这个按钮」糟得多，而且排查时毫无线索。
/// 所以宁可少一个按钮，也不要一个死按钮。
///
/// 将来移动端实现多页签时，**只需放宽这个函数的条件**，前端一行都不用改：
/// 按钮由 `mobilebar.js` 按这里下发的值决定是否渲染。
pub(crate) const fn tabs_available() -> bool {
    // `cfg!(windows)` 在本模块的生产目标（mobile）里恒为 false。写出来是为了把「判据是什么」
    // 保留在代码里而不是只留一句注释：哪天移动端有了页签，改的就是这一行。
    cfg!(windows) && TABS_ENABLED
}

/// 注入给页面的配置脚本。
///
/// ⚠️⚠️ 必须排在 `mobilebar.js` **之前**注册（初始化脚本按注册顺序执行）：脚本开头要读
/// `window.__web2appMobileBarCfg`，顺序反了它就会落进「默认值」分支 —— 底栏照画、
/// 但高度与按钮清单不再由壳决定，是最难从表象反推的一类错位。
/// 与 `lib.rs::build_main_window` 里「模式标记必须排在 OVERLAY_SCRIPT 之前」同一条契约。
pub(crate) fn config_script() -> String {
    format!(
        "window.__web2appMobileBarCfg = {{\"h\":{},\"tabs\":{}}};",
        BAR_HEIGHT,
        if tabs_available() { "true" } else { "false" }
    )
}

/// 「首页」按钮的事件名（与 `src/mobilebar.js` 里的字面量逐字一致，契约测试钉着）。
pub(crate) const NAV_HOME_EVENT: &str = "web2app:nav-home";
/// 页签列表里点某一行（切换）的事件名。
pub(crate) const SWITCH_TAB_EVENT: &str = "web2app:switch-tab";
/// 页签列表里点关闭的事件名。
pub(crate) const CLOSE_TAB_EVENT: &str = "web2app:close-tab";

/// 启动期解析出的**首页导航信息**缓存（供底栏「首页」按钮复用）。
///
/// 为什么必须缓存而不是「点一下重新解析一遍」：入口地址可能是【域名数组】（DNS TXT 查询 +
/// 验签解密，实测可达 25s 预算）或走本地反向代理（真实地址是 `127.0.0.1:<随机端口>`）。
/// 页面手里既没有最终地址、也不该每次点首页都重跑一次解析 / 重启代理。
#[derive(Default)]
pub(crate) struct HomeNav(Mutex<Option<Navigation>>);

/// 记下本次解析出的首页导航信息（由 `bootstrap` 在解析成功后调用）。
pub(crate) fn set_home_nav(app: &AppHandle, nav: &Navigation) {
    if let Ok(mut g) = app.state::<HomeNav>().0.lock() {
        *g = Some(nav.clone());
    }
}

/// 注册底栏所需的托管状态与事件监听（在 `setup` 里调用）。
///
/// 三个事件都由前端发**核心事件**（`plugin:event|emit`）到达：主窗加载的是远程页面，
/// Tauri 2 对非本地 origin 的 IPC 强制走 ACL，应用命令无法授权给远程 origin ⇒ 必被拒。
pub(crate) fn setup(app: &AppHandle) {
    app.manage(HomeNav::default());

    let home = app.clone();
    app.listen(NAV_HOME_EVENT, move |_event| {
        navigate_home(&home);
    });

    // 页签相关两个事件：**保留监听**而不是不监听。
    // 理由与 `web_data::clear_web_data` 一致 —— 注册了才有「明确的『当前不支持』」；
    // 不注册的话前端 catch 掉 ACL 拒绝后一片安静，排查要从零开始。
    app.listen(SWITCH_TAB_EVENT, move |event| {
        let label = tab_label_of(event.payload());
        if tabs_available() {
            // 移动端多页签落地后在这里接真正的切换调用（前端无需改动）。
            dlog!("[mobilebar] 切换标签：{label:?}");
        } else {
            unsupported_tab_action("切换", label.as_deref());
        }
    });

    app.listen(CLOSE_TAB_EVENT, move |event| {
        let label = tab_label_of(event.payload());
        if tabs_available() {
            dlog!("[mobilebar] 关闭标签：{label:?}");
        } else {
            unsupported_tab_action("关闭", label.as_deref());
        }
    });
}

/// 事件 payload 里的页签 label（形如 `{"label":"tab-1"}`）；解析不出来就当未知。
fn tab_label_of(payload: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct P {
        label: String,
    }
    serde_json::from_str::<P>(payload).ok().map(|p| p.label)
}

/// 移动端当前没有多页签时，给一条**看得见**的结论（而不是静默无反应）。
fn unsupported_tab_action(kind: &str, label: Option<&str>) {
    dlog!(
        "[mobilebar] {}标签被忽略：当前构建没有移动端多页签能力（label={}）",
        kind,
        label.unwrap_or("未知")
    );
}

/// 底栏「首页」：回到入口地址（已在首页则重新加载）。
///
/// 三条路径：
///   1. 已缓存首页地址 **且** 当前就停在它上面 → `location.reload()`（用户要的是「刷新」，
///      而 `navigate` 到同一个地址在部分内核上是 no-op，点了会像没反应）；
///   2. 已缓存首页地址、当前在别的页面 → 直接导航过去（不重解析、不重启代理）；
///   3. **还没解析过**（用户停在启动页时就点了）→ 重走一次完整 bootstrap。
///      这是唯一会重新解析 / 重启代理的分支，代价可接受：它本来就是「启动尚未成功」的场景。
fn navigate_home(app: &AppHandle) {
    let label = crate::bootstrap::main_window_label();
    let Some(window) = app.get_webview_window(label) else {
        return;
    };

    let cached: Option<Navigation> = app.state::<HomeNav>().0.lock().ok().and_then(|g| g.clone());

    let Some(nav) = cached else {
        // ③ 未就绪：完整重来一次（bootstrap 内部会先终止上一轮残留的代理任务）。
        dlog!("[mobilebar] 首页导航信息尚未就绪，重走一次完整启动流程");
        crate::bootstrap::bootstrap(app.clone());
        return;
    };

    let target = match url::Url::parse(&nav.url) {
        Ok(u) => u,
        Err(e) => {
            dlog!("[mobilebar] 首页地址不合法（{}）：{}", nav.url, e);
            return;
        }
    };

    // ① 已在首页 → 重新加载
    if matches!(window.url(), Ok(cur) if same_target(&cur, &target)) {
        let _ = window.eval("try{location.reload();}catch(e){}");
        return;
    }

    // ② 在别的页面 → 导航回首页
    if let Err(e) = window.navigate(target) {
        dlog!("[mobilebar] 首页导航失败：{e}");
    }
}

/// 两个地址是否指向「同一个落点」：`scheme + host + 端口 + 路径` 全等。
///
/// ⚠️ 刻意**不比较** query / fragment：站点常把一次性参数（如 `?t=`、路由 state）挂在
/// 首页地址后面，逐一比对字符串会把「就在这个页面」误判成「不在」 ⇒ 用户点首页没反应。
/// ⚠️ 也不能只比 host：同一站点下的 `/a` 与 `/b` 是不同页面，点首页必须真的回到入口。
fn same_target(a: &url::Url, b: &url::Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port() == b.port()
        && a.path() == b.path()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_script_reports_height_and_tabs_flag() {
        let s = config_script();
        assert!(
            s.starts_with("window.__web2appMobileBarCfg = {"),
            "cfg 脚本必须是给页面赋值的语句：{s}"
        );
        assert!(
            s.contains(&format!("\"h\":{BAR_HEIGHT}")),
            "cfg 脚本必须带上底栏高度真源：{s}"
        );
        // tabs 段必须是字面量 true/false 之一（前端按 `=== true` 判定）
        assert!(
            s.contains("\"tabs\":true") || s.contains("\"tabs\":false"),
            "cfg 脚本必须带上页签可用性标记：{s}"
        );
    }

    #[test]
    fn mobile_has_no_tabs_so_list_button_is_hidden() {
        // ⚠️ 这条就是「宁可少一个按钮，也不要死按钮」的可观测判据：
        //    移动端恒为 false ⇒ 前端不画「列表」按钮。
        //    （本模块在桌面端只以 `test` 形态编译，那里 TABS_ENABLED 同样是默认值 false。）
        assert!(
            !tabs_available(),
            "移动端当前没有多页签 ⇒ tabs_available() 必须为 false，否则「列表」按钮点了没反应"
        );
    }

    #[test]
    fn same_target_compares_scheme_host_port_path_only() {
        let u = |s: &str| url::Url::parse(s).unwrap();
        assert!(same_target(&u("https://a.com/"), &u("https://a.com/")));
        assert!(same_target(&u("https://a.com/"), &u("https://a.com")));
        // 一次性参数不影响「是不是这个页面」
        assert!(same_target(&u("https://a.com/?t=1"), &u("https://a.com/")));
        assert!(same_target(&u("https://a.com/#x"), &u("https://a.com/")));
        // 端口 / scheme / 路径 / 主机不同 ⇒ 不是同一个落点
        assert!(!same_target(
            &u("https://a.com:8443/"),
            &u("https://a.com/")
        ));
        assert!(!same_target(&u("http://a.com/"), &u("https://a.com/")));
        assert!(!same_target(&u("https://a.com/b"), &u("https://a.com/")));
        assert!(!same_target(&u("https://b.com/"), &u("https://a.com/")));
        // 代理模式下首页是 127.0.0.1:<随机端口>，与上游域名**不**同落点
        assert!(!same_target(
            &u("http://127.0.0.1:51234/"),
            &u("https://a.com/")
        ));
    }

    #[test]
    fn tab_label_of_parses_label_or_gives_up() {
        assert_eq!(
            tab_label_of("{\"label\":\"tab-1\"}").as_deref(),
            Some("tab-1")
        );
        assert_eq!(tab_label_of("not json"), None);
        assert_eq!(tab_label_of("{}"), None);
    }

    #[test]
    fn event_names_are_stable() {
        // 事件名是跨文件契约（前端字面量 ↔ 本模块常量），改这里必须同步改 mobilebar.js。
        assert_eq!(NAV_HOME_EVENT, "web2app:nav-home");
        assert_eq!(SWITCH_TAB_EVENT, "web2app:switch-tab");
        assert_eq!(CLOSE_TAB_EVENT, "web2app:close-tab");
    }
}
