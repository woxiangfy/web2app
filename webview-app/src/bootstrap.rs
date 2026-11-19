//! 启动编排：后台准备真实地址 → 就绪后导航。
//!
//! `run()`（lib.rs）只负责「建窗 + 加载本地启动页」，真正的耗时操作（域名解析、启动本地反向代理）
//! 放到这里的后台线程：这样"加载中…"从窗口出现的第一帧就可见，而不是等准备全部做完。
//! 准备完成后回到主线程发起导航；失败则在启动页上展示错误与重试入口。
//!
//! ⚠️ 本模块**只留编排职责**（架构审查 2026-10-25 P2-1）：导航会话数据（[`crate::nav`]：
//! Navigation / ProxyTask / BootReady / label 登记 / origin 白名单等）已抽成数据枢纽，
//! 功能模块一律走 `→ nav`，不再上行依赖本模块；本模块的对外入口 [`boot_entry`] 也反向经
//! `nav` 的 boot 回调转发（接线在 `lib.rs`），依赖方向保持
//! `bootstrap → nav / 功能模块 / config` 的单向形态。
//!
//! 架构审查 2026-11-19：Android 可达性探测（原住在本文件尾部的 banner 段，约 270 行）已抽到
//! [`crate::probe`] —— 那是「导航之后为什么还是白屏」的诊断，与「何时导航」的编排是两件事。
//! 本文件现在只做：等首文档就绪 → 准备（解析 / 代理）→ 导航 → 失败上报。

use tauri::Manager;
use url::Url;

use crate::config::{APP_URL, PUBLIC_KEY};
#[cfg(not(windows))]
use crate::nav::BootReady;
use crate::nav::{local_proxy_enabled, origin_of, Navigation, ProxyTask};

/// 启动期等待「首个文档加载完成」的预算（毫秒）。
///
/// 启动页是本地自定义协议页（真机实测 HTTP 200 / 125ms），正常几十毫秒即完成；5s 是给冷启动的
/// WebView 留余量。**超时不致命**：继续按原流程走（退化成「不等待」的旧行为），只在卡片上留一行
/// 说明 —— 绝不因为拿不到页面加载事件就把启动彻底卡死。
///
/// 分支口径沿用本模块既有约定（`not(windows)`，与可达性探测那部分一致）：本项目实际发布的
/// 非 Windows 目标就是 Android；若将来跑桌面 Linux 而 wry 不上报 page load，最坏也只是每次启动
/// 多等这个预算，仍然能正常启动。
#[cfg(not(windows))]
const SPLASH_READY_BUDGET_MS: u64 = 5000;

/// 在后台线程等待首个文档加载完成（仅非 Windows；理由见 [`BootReady`] 的长注释）。
///
/// 返回 `true` = 拿到了加载完成事件；`false` = 预算内没拿到（页面加载事件缺失）。
#[cfg(not(windows))]
fn wait_first_page_ready(app: &tauri::AppHandle, label: &str) -> bool {
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_millis(SPLASH_READY_BUDGET_MS);
    while std::time::Instant::now() < deadline {
        if app.state::<BootReady>().is_ready(label) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    false
}

// ===========================================================================
// 非 Windows（Android 等）：导航后的「落点校验 + 有限重试」
// ===========================================================================
//
// 为什么需要：即使启动期已经等了首个文档就绪（见 [`BootReady`]），Android 上「导航是否
// 真的落到目标地址」**仍然没有可靠信号** —— wry 的 `navigate()` 恒返回 `Ok(())`（它只是
// 往 main-pipe 投一条消息），`onReceivedError` 也不向壳层透出任何网络错误。真机因此出现过
// 「导航确实发起了、地址却永远停在 `splash.localhost`」。
//
// 与其继续推断「启动页的异步加载与导航谁先谁后」，这里直接看**结果**：导航后隔一小段
// 读一次 WebView 的真实地址，若仍停在启动页就重发导航。这条判据不依赖任何时序假设，
// 也不依赖页面加载事件是否送达 —— 它是「导航到底有没有生效」的终局检验。
//
// 重发是安全的：`loadUrl` 语义上是「加载这个地址」的幂等操作，重复发起只会以最后一次为准，
// 不会叠加副作用；而真出问题时（导航被启动页挤掉）重发正是所需的那一次。

/// 导航发起后隔多久校验一次落点（毫秒）。
///
/// 取值理由：启动页是本地页面（真机实测 125ms 完成），而目标地址的 `loadUrl` 一旦被 WebView
/// 接受，`getUrl()` 会**立即**返回新地址（不等加载完成）。500ms 足够区分「已提交」与「没提交」，
/// 又不会让正常启动多等。
#[cfg(not(windows))]
const NAV_COMMIT_CHECK_MS: u64 = 500;

/// 落点仍在启动页时最多重发几次导航。
///
/// 只重发 2 次：第一次命中「导航被启动页挤掉」这一根因；第二次兜住「重发时启动页又抢了一次」
/// 的极端情形。再多没有新增覆盖面，只会拖长失败时的等待。
#[cfg(not(windows))]
const NAV_COMMIT_MAX_RETRY: u32 = 2;

/// 该地址是否仍是本地启动页。
///
/// ⚠️ **两种形态都要认**：`splash://localhost/`（`loading::splash_url()` 构造的）与
/// `http://splash.localhost/` —— wry 的 `custom_protocol_workaround` 在 Windows/Android 都会
/// 把前者改写成后者（`use_https_scheme` 默认 false，本项目未开），所以 WebView 里**真实存在**的
/// 地址是后者。只认自定义协议会在真机上漏判（这正是 v1.62 修过的那类判据坑）。
#[cfg(not(windows))]
fn is_splash_url(url: &Url) -> bool {
    url.scheme() == "splash" || url.host_str() == Some("splash.localhost")
}

/// 导航后校验落点；仍停在启动页则重发导航（仅非 Windows）。
///
/// ⚠️ **必须跑在独立线程**：调用方 `navigate_main` 在主线程执行，而这里要 sleep 且读地址 ——
/// `WebviewWindow::url()` 在 Android 上走 main-pipe（`GetUrl` 消息由主线程 Looper 消费），
/// 放在主线程会把 UI 冻住。
///
/// 正常启动（导航生效）时它只是一次 500ms 后的读地址 + 返回，零副作用；只有异常路径才会重发。
#[cfg(not(windows))]
fn ensure_navigation_committed(app: &tauri::AppHandle, label: &str, target: &Url) {
    let app = app.clone();
    let label = label.to_string();
    let target = target.clone();
    std::thread::spawn(move || {
        for attempt in 1..=NAV_COMMIT_MAX_RETRY {
            std::thread::sleep(std::time::Duration::from_millis(NAV_COMMIT_CHECK_MS));
            let Some(window) = app.get_webview_window(&label) else {
                return;
            };
            match window.url() {
                // 已离开启动页 = 导航提交成功，正常启动走这条分支。
                Ok(cur) if !is_splash_url(&cur) => {
                    if attempt > 1 {
                        note(&app, &label, &format!("重发导航已生效（第 {attempt} 次）"));
                    }
                    return;
                }
                Ok(_) => note(
                    &app,
                    &label,
                    &format!(
                        "导航未落到目标地址（仍停在启动页），正在重发（第 {attempt}/{NAV_COMMIT_MAX_RETRY} 次）…"
                    ),
                ),
                // 读不到地址不是重发的理由，但也不该吞掉 —— 写进卡片供真机判读。
                Err(e) => note(
                    &app,
                    &label,
                    &format!("读取当前地址失败（{e}），仍尝试重发导航"),
                ),
            }
            let a = app.clone();
            let l = label.clone();
            let u = target.clone();
            let _ = app.run_on_main_thread(move || {
                if let Some(w) = a.get_webview_window(&l) {
                    let _ = w.navigate(u);
                }
            });
        }
        // 走到这里 = 每一轮校验都还停在启动页。把「重发也没用」说清楚 —— 它的分诊含义
        // 与「导航根本没发出去」不同：地址是对的、导航也发了，是 WebView 没提交。
        note(
            &app,
            &label,
            &format!(
                "重发导航 {NAV_COMMIT_MAX_RETRY} 次后仍未离开启动页（导航未提交，与网络无关）"
            ),
        );
    });
}

/// 去掉 URL 中的 userinfo（用户名:密码），返回“干净”地址。
/// 用于直连 / 外部代理场景下 WebView 的导航地址（鉴权由 WebView2 原生事件或本地代理注入，
/// 不应把密码写进浏览器地址栏 / 交给外部代理字符串）。
/// ⚠️ 复用 [`crate::proxy::strip_userinfo`]（sidecar 共享的单一真源），不再本地重复实现。
fn strip_userinfo(url: &Url) -> String {
    crate::proxy::strip_userinfo(url)
}

/// 启动编排入口（`nav::bootstrap` / `nav::bootstrap_for_label` 经登记的回调转发到这里，
/// 接线在 `lib.rs` 的 setup —— 控制反转的理由见 [`crate::nav`] 模块头）。
///
/// 允许被重复调用（启动页上的"重试"即 splash://localhost/retry → 这里），
/// 因此进入时先终止上一轮残留的本地反向代理，避免端口泄漏。
///
/// `label = None` = 单窗口模式（目标窗 `"main"`）；`Some(label)` = 导航到指定窗口
///（多页签首个 tab / splash `/retry` 传入）。
///
/// ⚠️ 一律**必须缓存**解析结果（`cache_navigation = true`）。多页签模式下
/// 首个 tab 恰恰是由 `tabs::spawn_tab` 走这条路径创建的（单窗口模式走 `label = None`），
/// 曾经这里传 `false` 导致 `TabManager.navigation` **永远为 None**，连锁后果三连：
///   1) [`crate::tabs::resolve_popup_url`] 拿不到入口/上游主机 → 一律返回 None →
///      `target="_blank"` 新 tab 全部落在主地址（用户反馈的“弹窗打开的网址一直是 app_url”）；
///   2) 每新建一个 tab 都重新 DNS 解析 + 重新起一个本地反向代理（新端口），
///      而 `bootstrap_inner` 开头又会 abort 掉 ProxyTask 里的旧代理 → 已在用的旧 tab 断连；
///   3) 新建 tab 明显变慢。
pub(crate) fn boot_entry(app: tauri::AppHandle, label: Option<String>) {
    bootstrap_inner(app, label.unwrap_or_else(|| "main".to_string()), true);
}

/// 启动流程核心：后台准备 → 主线程导航 / 报错。
///
/// `cache_navigation`：是否把本次解析结果写入 `TabManager.navigation` 供后续新 tab 复用。
/// 只有 Windows 多页签模式用到（该字段在非 Windows 下不引用），故用 cfg_attr 局部放宽
/// unused_variables，避免 Android / iOS 编译告警。
#[cfg_attr(not(windows), allow(unused_variables))]
fn bootstrap_inner(app: tauri::AppHandle, label: String, cache_navigation: bool) {
    let use_local_proxy = local_proxy_enabled();
    std::thread::spawn(move || {
        // ⚠️⚠️ Android：先等首个文档（启动页）加载完成，再解析 / 推送 / 导航。
        // 不等它就会「导航被 post 出去的 splash 加载覆盖 + 进度 eval 落在 document-start 之前
        // 被静默丢弃」——根因、上游代码位置与真机证据见 [`BootReady`] 的长注释。
        // Windows 不加此等待（WebView2 的注入/导航时序由 wry 自己保证）。
        #[cfg(not(windows))]
        if !wait_first_page_ready(&app, &label) {
            // 拿不到加载事件**本身就是一个可判读的事实**，但绝不阻断启动：继续按原流程走
            //（退化成旧的「不等待」行为），只是把这件事写在卡片上。
            note(
                &app,
                &label,
                &format!(
                    "启动页未在 {}ms 内报告加载完成（页面加载事件缺失），按原流程继续",
                    SPLASH_READY_BUDGET_MS
                ),
            );
        }

        // 重试场景：清掉上一轮可能仍在监听的代理任务。
        let stale = app
            .state::<ProxyTask>()
            .0
            .lock()
            .ok()
            .and_then(|mut g| g.take());
        if let Some(abort) = stale {
            abort.abort();
        }

        // catch_unwind：prepare_navigation 里仍有极少数 expect（如 CSPRNG 取密钥失败），
        // panic 在这个 detached 线程里只会让线程静默死亡——启动页永远转圈、用户零入口，
        // 比显式报错更糟（项目约定：环境级故障也要可见）。AssertUnwindSafe：这里没有
        // 跨 catch 的借用不变量需要维护。
        let prepared = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            prepare_navigation(&app, &label, use_local_proxy)
        }));
        let outcome = match prepared {
            Ok(r) => r,
            Err(pa) => {
                let msg = if let Some(s) = pa.downcast_ref::<&str>() {
                    format!("启动过程发生内部错误：{s}")
                } else if let Some(s) = pa.downcast_ref::<String>() {
                    format!("启动过程发生内部错误：{s}")
                } else {
                    "启动过程发生内部错误（未知 panic）".to_string()
                };
                Err(msg)
            }
        };

        match outcome {
            Ok(nav) => {
                // 解析成功：缓存导航信息供后续复用。重试场景覆盖旧缓存也是正确的——
                // 重试后端口/地址可能已变，缓存必须跟着更新。多页签模式额外写一份
                // TabManager.navigation（页签新建路径复用）；单窗口模式无后续复用方。
                #[cfg(windows)]
                if cache_navigation && crate::tabs::tabs_enabled() {
                    if let Ok(mut cache) = app.state::<crate::tabs::TabManager>().navigation.lock()
                    {
                        *cache = Some(nav.clone());
                    }
                }
                // 移动端底栏的「首页」按钮：把解析结果缓存下来，点首页时直接复用
                //（不重跑 DNS 解析、不重启本地代理）。见 `crate::mobilebar::HomeNav`。
                #[cfg(mobile)]
                crate::mobilebar::set_home_nav(&app, &nav);
                let a = app.clone();
                let l = label.clone();
                let _ = app.run_on_main_thread(move || navigate_main(&a, &l, nav));
            }
            Err(msg) => {
                dlog!("[bootstrap] {msg}");
                let a = app.clone();
                let l = label.clone();
                // 启动页仍然显示着，直接把错误态推上去（含重试按钮），
                // 不再像早期版本那样 std::process::exit —— 用户至少能看到发生了什么。
                let _ = app.run_on_main_thread(move || {
                    crate::loading::show_error(&a, &l, "加载失败", &msg)
                });
            }
        }
    });
}

/// 启动期「解析目标地址」阶段的**总预算**（秒）。
///
/// 背景（2026-10-09 真机取证）：启动页的兜底卡是 40s（`loading.js` 的 `FAIL_MS`）。域名数组
/// 配置下解析是**并发竞速**的（所有域名同时查，第一个「查询 + 解密验签」成功即返回，不等待其余），
/// 最坏耗时不再随域名个数线性叠加；但单个域名的 hickory 默认 `attempts=2 × timeout=5s ×
/// 每个 nameserver`，某个 nameserver 不可达时单条仍可能拖到 10s+，启动会长时间停在
/// "卡住"状态（既无进度也无报错）。取值需**明显小于** 40s（给渲染与推送留时间），同时给弱网
/// 留足余量：整个解析阶段（含单域名内的重试）超过它即判定失败，把「静默卡住」变成明确报错。
const RESOLVE_BUDGET_SECS: u64 = 25;

/// 从启动期后台线程往加载层推一行进度。
///
/// ⚠️ `eval` 必须回主线程（Tauri 的 WebView 操作都要求主线程），所以这里统一转一手 ——
/// 调用方（`prepare_navigation` 跑在 `std::thread::spawn` 里）不能直接调 `loading::show_note`。
fn note(app: &tauri::AppHandle, label: &str, line: &str) {
    let a = app.clone();
    let l = label.to_string();
    let s = line.to_string();
    let _ = app.run_on_main_thread(move || crate::loading::show_note(&a, &l, &s));
}

/// 解析真实地址并按需启动本地反向代理（在后台线程执行，可能耗时）。
///
/// `app` / `label` 只用于把各阶段进度推给启动页上的加载层（[`crate::loading::show_note`]）——
/// 启动期一旦偏慢或卡住，卡片上就留有「卡在哪一步」的记录，不必依赖 adb / logcat。
fn prepare_navigation(
    app: &tauri::AppHandle,
    label: &str,
    use_local_proxy: bool,
) -> Result<Navigation, String> {
    // 若 APP_URL 是【域名数组】，则并发对这些域名做 DNS TXT 查询 + Ed25519 验签
    // + AES-GCM 解密得到真实 URL（第一个成功即用，不等待其余；全部失败时返回可展示的错误文案）。
    // ⚠️ `resolve_url_from_domains` 是 async（该模块刻意不依赖 Tauri，见 resolve.rs 头注释），
    // 由这里用 Tauri 的异步运行时阻塞驱动 —— 本函数跑在后台线程，不在 async 上下文内。
    let resolved_url: String = if let Some(domains) = crate::resolve::parse_domains(APP_URL) {
        let domains: Vec<String> = domains
            .into_iter()
            .map(|d| crate::resolve::normalize_domain(&d))
            .collect();
        note(
            app,
            label,
            &format!("正在解析目标地址（{} 个域名）…", domains.len()),
        );
        // ⚠️⚠️ 必须设**总预算**：解析是**并发竞速**的（不再随域名个数叠加），但单条 hickory 查询
        // 默认 `attempts=2 × timeout=5s × 每个 nameserver`，某个 nameserver 不可达时仍可能拖到 10s+；
        // 用户只看到启动页那张 40s 兜底卡。超时即给出可看懂的失败，而不是一直挂着。
        tauri::async_runtime::block_on(async {
            match tokio::time::timeout(
                std::time::Duration::from_secs(RESOLVE_BUDGET_SECS),
                crate::resolve::resolve_url_from_domains(&domains, PUBLIC_KEY),
            )
            .await
            {
                Ok(r) => r,
                Err(_) => Err(format!(
                    "解析目标地址超过 {} 秒仍未完成（网络受限或 DNS 不可达），请切换网络后重试",
                    RESOLVE_BUDGET_SECS
                )),
            }
        })
        // ⚠️ 不要把具体原因（`_`）丢掉只留一句「服务器连接失败」：启动期故障的具体成因
        //（读系统 DNS 配置失败 / 查询某域名失败 / 全部密文解不开 / 预算超时）是唯一能指导
        // 下一步的信息（项目约定：环境级故障也要可见）。
        .map_err(|e| format!("解析目标地址失败：{e}"))?
    } else {
        APP_URL.to_string()
    };

    let parsed = Url::parse(&resolved_url)
        .map_err(|e| format!("应用配置的网址不合法（{e}）：{resolved_url}"))?;

    // scheme 白名单：与 sidecar（proxy-server/main.rs）同一语义。非 http(s) 配置
    // （或 DNS 解出异常 scheme）下继续启动只会让代理每条请求 502、启动页既不报错也无法
    // 定位，直接在这里给出可看懂的失败。
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(format!(
                "只支持 http / https 上游，当前是 {other}：{resolved_url}"
            ))
        }
    }

    // 地址已被接受：下面每一步都推给卡片 —— 启动期一旦偏慢或卡住，卡片上就留有「卡在哪一步」。
    note(
        app,
        label,
        &format!("正在打开 {}…", parsed.host_str().unwrap_or("目标站点")),
    );

    let (url, proxy_abort) = if use_local_proxy {
        note(app, label, "正在启动本地反向代理…");
        // 自身实现的本地反向代理：给每个请求（含 WebSocket 握手）注入 Authorization。
        let (u, abort) = tauri::async_runtime::block_on(crate::proxy::start_proxy(&parsed))
            .map_err(|e| format!("本地代理启动失败：{e}"))?;
        (u, Some(abort))
    } else {
        // 直连 / 外部代理：导航到去掉 userinfo 的上游地址（鉴权由原生事件或外部代理注入）。
        (strip_userinfo(&parsed), None)
    };

    Ok(Navigation {
        url,
        user: parsed.username().to_string(),
        pass: parsed.password().unwrap_or("").to_string(),
        use_local_proxy,
        proxy_abort,
        upstream_origin: origin_of(&parsed),
    })
}

/// 在主线程发起首次导航（准备工作已在后台线程完成）。
///
/// `label`：目标窗口 label（单窗口为 `"main"`，多页签为 `tab-N`）。
fn navigate_main(app: &tauri::AppHandle, label: &str, nav: Navigation) {
    // ⚠️ 探测目标必须在 `nav.proxy_abort` 被 move 之前算出来 —— 部分移动之后就不能再
    //    整体借用 `nav` 了（清单 #15：代理模式要探上游，不能探 127.0.0.1）。
    #[cfg(not(windows))]
    let (probe_url, via_proxy) = crate::probe::probe_target(&nav);

    // 代理句柄先入库，保证退出 / 重试时能优雅终止。
    // 多页签模式下：本地代理是单实例共享，句柄只由首次启动入库一次；后续 tab 复用的
    // Navigation 里的 proxy_abort 是同一个句柄的克隆，重复入库会覆盖，属无害但应避免。
    // 这里用「仅当 ProxyTask 当前为空才入库」保证只入一次。
    if let Some(abort) = nav.proxy_abort {
        if let Ok(mut g) = app.state::<ProxyTask>().0.lock() {
            if g.is_none() {
                *g = Some(abort);
            }
        }
    }

    let Some(window) = app.get_webview_window(label) else {
        return;
    };

    // Windows：必须先注册 WebView2 钩子（新窗口拦截 + Basic 鉴权）再导航，否则窗口首批
    // 请求会漏掉 Authorization 注入；钩子注册由 attach_webview_hooks 完成（不负责 Navigate），
    // 真正的导航在下面用 `window.navigate(url)` 触发（wry 内部会等 CoreWebView2 controller
    // 就绪再触发 WebView2 的 Navigation，避免 with_webview 回调时机太早导致整个钩子+导航被
    // 跳过、tab 停在 builder 默认入口的「app_url」问题）。
    #[cfg(windows)]
    {
        crate::webview_hooks::attach_webview_hooks(
            app.clone(),
            &window,
            nav.user,
            nav.pass,
            nav.use_local_proxy,
            nav.upstream_origin,
        );
        if let Ok(url) = Url::parse(&nav.url) {
            let _ = window.navigate(url);
        }
    }

    // 其它平台（Android 等）：没有 WebView2，直接导航；鉴权统一由本地反向代理注入。
    #[cfg(not(windows))]
    {
        let _ = (&nav.user, &nav.pass, &nav.use_local_proxy);
        match Url::parse(&nav.url) {
            Ok(url) => {
                // ⚠️ `clone()`：`navigate` 拿走所有权，而下面的落点校验还要用同一份目标地址。
                match window.navigate(url.clone()) {
                    Ok(()) => {
                        // ⚠️⚠️ `Ok(())` 在这里**不代表导航成功**：wry 在 Android 上把
                        // `load_url` 实现成「投一条 main-pipe 消息后直接 `Ok(())`」
                        //（`wry/src/android/mod.rs`），且 `RustWebViewClient.onReceivedError`
                        // 只对 `ERROR_CONNECT` 做协议重定向特例、**不向壳层透出任何网络错误**
                        // ⇒ 主框架加载失败时壳侧完全无从得知，WebView 会把启动页留在屏幕上，
                        // 最终只剩加载层那张 40s 兜底卡（真机取证：2026-10-09）。所以这里
                        // ①必须留下「已发起导航」的痕迹 —— 这是区分「根本没导航」与
                        // 「导航了但没提交」的唯一线索；②并发一次可达性探测，把结论写回卡片。
                        note(app, label, "已发起导航，等待页面响应…");
                        // 探测本身：Android WebView 网络失败时停在「空白错误文档」（桌面端浏览器
                        // 那种带错误码的页面在 WebView 上不渲染），仅靠加载层固定超时只能给通用
                        // 文案；探测可在数秒内把 DNS / 连接 / 超时 / TLS 的具体原因推给失败卡。
                        // ⚠️ 探的是 `crate::probe::probe_target` 算出的**上游**：代理模式下 nav.url 是
                        //    `127.0.0.1:<port>`，探它只能证明本地代理进程活着（清单 #15）。
                        crate::probe::spawn_reachability_probe(
                            app.clone(),
                            label,
                            probe_url,
                            via_proxy,
                        );
                        // 落点校验：探测管「网络通不通」，这里管「导航到底有没有生效」。
                        // 两者互补 —— 真机上这两件事曾经都只能靠猜。见 `ensure_navigation_committed`。
                        ensure_navigation_committed(app, label, &url);
                    }
                    Err(e) => crate::loading::show_error(
                        app,
                        label,
                        "加载失败",
                        &format!("无法打开页面：{e}"),
                    ),
                }
            }
            Err(_) => crate::loading::show_error(app, label, "加载失败", "应用配置的地址不合法"),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_userinfo_removes_credentials_keeps_rest() {
        let u = Url::parse("https://alice:secret@example.com/path?q=1#frag").unwrap();
        assert_eq!(strip_userinfo(&u), "https://example.com/path?q=1#frag");
    }

    #[test]
    fn strip_userinfo_without_userinfo_unchanged() {
        let u = Url::parse("https://example.com:8443/a").unwrap();
        assert_eq!(strip_userinfo(&u), "https://example.com:8443/a");
    }

    #[test]
    fn strip_userinfo_keeps_ipv6_authority() {
        let u = Url::parse("http://user:pw@[::1]:8080/x").unwrap();
        assert_eq!(strip_userinfo(&u), "http://[::1]:8080/x");
    }

    // ⚠️ origin 归一化 / 上游主机白名单 / 非 http(s) 拒绝的用例已随 `origin_of` /
    // `is_upstream_request` 迁往 `src/nav.rs`（P2-1，needle 跟随搬家，断言未改）。
}
