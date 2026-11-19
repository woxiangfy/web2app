//! 多页签「IPC 命令门面」子模块 —— P4 大文件拆分自 tabs.rs（零行为变更纯代码搬家）。

//! 条目与拆分前逐字一致；与根模块经 use super::* 互访。原模块内私有的条目一律 pub(super)：
//! 可见范围（tabs 模块树）与拆分前的模块内私有完全相同，只是搬家、不放宽对外可见性。

use super::*;

/// 前端：新建一个 tab。
///
/// ⚠️ 标签栏已移除「+」按钮，当前没有前端调用方。仍保留并注册：`target="_blank"` /
/// `window.open()` 由 WebView2 的 `NewWindowRequested` 钩子在 Rust 侧直接走 [`spawn_tab`]，
/// 本命令留作程序化建 tab 的入口（调试 / 后续扩展）。
#[tauri::command]
pub(crate) fn create_tab(app: tauri::AppHandle) -> Result<String, String> {
    spawn_tab(&app)
}

/// 前端：切换到指定 tab。
#[tauri::command]
pub(crate) fn switch_tab(app: tauri::AppHandle, label: String) {
    activate_tab(&app, &label);
}

/// 前端：关闭指定 tab。
#[tauri::command]
pub(crate) fn close_tab(app: tauri::AppHandle, label: String) {
    close_tab_inner(&app, &label);
}

/// 前端：获取当前 tab 列表（label + title + active + home），用于初始化标签栏。
///
/// ⚠️ 四元组而非三元组：第 4 位 `home` 是首页标记（见 [`is_home_index`]），shell.html 的
/// 初始化映射 `list.map(([label,title,active,home]) => …)` 依赖它。Electron 侧
/// `listForIpc()` 必须给出同形（探针逐字段验）。
#[tauri::command]
pub(crate) fn get_tabs(app: tauri::AppHandle) -> Vec<(String, String, bool, bool)> {
    // 与宿主标签栏（push_tabs_to_shell）共用同一份快照，保证 IPC 与界面永远一致。
    tab_rows(&app)
}

/// tab 列表的统一快照：`(label, 展示标题, 是否激活, 是否首页)`。
///
/// ⚠️ 为什么抽出来：[`get_tabs`]（IPC 命令）与 [`push_tabs_to_shell`]（宿主标签栏 eval）
/// 此前各写了一遍「锁 tabs / active / titles → 标题取 `titles` 缓存、缺失回落 `TabInfo::title`
/// → 拼 active 标志」。两处一旦漂移，现象是**IPC 返回的标题与标签栏显示的不是同一个**
/// —— 静默不一致，没有任何报错。
///
/// ⚠️ 三把锁的顺序必须固定为 `tabs → active → titles`：与[`set_tabs_visible`] 取锁的顺序
/// 一致，避免两处反向加锁形成 AB-BA 死锁。
pub(super) fn tab_rows(app: &tauri::AppHandle) -> Vec<(String, String, bool, bool)> {
    let state = app.state::<TabManager>();
    let tabs = state.tabs.lock().unwrap_or_else(|p| p.into_inner());
    let active = *state.active.lock().unwrap_or_else(|p| p.into_inner());
    let titles = state.titles.lock().unwrap_or_else(|p| p.into_inner());
    tabs.iter()
        .enumerate()
        .map(|(i, t)| {
            // 展示标题：优先用 tab 页面回传的真实网页标题，没有则回落到建窗时的
            // 占位标题（通常是应用显示名）。
            let title = titles
                .get(&t.label)
                .cloned()
                .unwrap_or_else(|| t.title.clone());
            // 首页标记由位次推出（见 is_home_index 的文档：固定第一位 + 不可关 + 自动补齐）。
            (t.label.clone(), title, i == active, is_home_index(i))
        })
        .collect()
}

/// 校验「tab 一次性令牌」：label 与 token 必须同时命中在册记录。
///
/// ⚠️ 为什么必须有：[`report_tab_title`] 与 [`retry_tab_bootstrap`] 都接**核心事件**，
/// 而 Tauri 2.11 的 `app.listen` Event **不携带发送窗口标识**（只有 id / payload），
/// 无法知道事件来自哪个 webview。少了这道校验，任意外部页面都能伪造其它页签的标题或
/// 强制别的 tab 重启 bootstrap。两处此前各写一遍同样判断，抽出来保证不会只改一边。
pub(super) fn tab_token_matches(app: &tauri::AppHandle, label: &str, token: &str) -> bool {
    let state = app.state::<TabManager>();
    let tabs = state.tabs.lock().unwrap_or_else(|p| p.into_inner());
    tabs.iter().any(|t| t.label == label && t.token == token)
}

/// tab 远程页面经核心事件回传真实网页标题的落地函数。
///
/// 历史上它是一条 `#[tauri::command]`，但 tab 里跑的是**远程 origin**（代理 127.0.0.1
/// 或直连上游），Tauri 2 对远程页面的应用自定义命令一律拒绝（框架自带测试
/// `remote_origin_blocked_for_custom_commands_without_app_manifest`），且工程没有 app
/// ACL manifest —— 前端 `.catch` 又静默吞掉，标签栏于是永远显示默认应用名。改走
/// `core:event:allow-emit`（可授权给远程 origin）后由本函数落地。
///
/// ⚠️ Tauri 2.11 的 `app.listen` Event **不携带发送窗口标识**（只有 id/payload），
/// 无法知道事件来自哪个 webview；因此 payload 必须带回建窗时注入该 tab 的一次性
/// [`TabInfo::token`]，与在册记录逐字相等才接受 —— 否则任意外部页面都能伪造其它
/// 页签的标题（钓鱼式欺骗）。title 最终经 shell.html 的 textContent 渲染，无 XSS 面。
pub(crate) fn report_tab_title(
    app: &tauri::AppHandle,
    label: String,
    token: String,
    title: String,
) {
    if !tab_token_matches(app, &label, &token) {
        dlog!("[tabs] 拒绝标题回传：label/token 与在册 tab 不匹配（label={label}）");
        return;
    }
    {
        let state = app.state::<TabManager>();
        state
            .titles
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(label.clone(), title.clone());
    }
    {
        let state = app.state::<TabManager>();
        let mut tabs = state.tabs.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(t) = tabs.iter_mut().find(|t| t.label == label) {
            t.title = title.clone();
        }
    }
    push_tabs_to_shell(app);
}

/// tab 首屏「重试」事件的落地：校验 label/token 后为该 tab 重走完整 bootstrap。
///
/// tab 首文档是本地占位页（不是 splash:// 协议页），loading.js 的重试按钮不能像单窗口
/// 那样导航到 `splash://localhost/retry`；改为发 [`TAB_RETRY_EVENT`]。token 校验与
/// [`report_tab_title`] 同理由（app.listen 的 Event 不带发送窗口标识）。
pub(crate) fn retry_tab_bootstrap(app: &tauri::AppHandle, label: String, token: String) {
    if !tab_token_matches(app, &label, &token) {
        dlog!("[tabs] 拒绝首屏重试：label/token 与在册 tab 不匹配（label={label}）");
        return;
    }
    dlog!("[tabs] 收到首屏重试事件，重新走 bootstrap：{label}");
    crate::nav::bootstrap_for_label(app.clone(), label);
}

/// tab 页面「真实网页文档已提交」事件（`src/tab_doc_ready.js`）的落地：校验令牌后把该 tab
/// 标记为 [`TabInfo::committed`]。只改状态、不动界面（标签栏无需重画）。
///
/// 为什么需要它：下载响应**不产生文档**，所以「只为下载而开的 tab」永远收不到本事件。
/// 下载落盘 / 取消后 `window.rs::close_tab_after_download` 就靠这个状态区分「页面」与
/// 「空页签」。首文档是本地占位页（`tauri.localhost`），脚本对它有早退守卫，不算提交。
///
/// token 校验与 [`report_tab_title`] 同理由（app.listen 的 Event 不带发送窗口标识）。
pub(crate) fn mark_tab_doc_ready(app: &tauri::AppHandle, label: String, token: String) {
    if !tab_token_matches(app, &label, &token) {
        dlog!("[tabs] 拒绝文档就绪回传：label/token 与在册 tab 不匹配（label={label}）");
        return;
    }
    let state = app.state::<TabManager>();
    let mut tabs = state.tabs.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(t) = tabs.iter_mut().find(|t| t.label == label) {
        t.committed = true;
    }
}

/// tab 页面里 Ctrl+W（`src/tab_keys.js`）经核心事件到达后的落地：校验 label/token 后关闭该 tab。
///
/// ⚠️ 关闭统一走 [`close_tab_inner`]：首页那一签由它第 ① 步拒绝（固定第一位、不可关），
/// 本函数**不重复判断首页** —— 「首页判定」全项目只有 [`is_home_index`] 一份实现。
/// 拒绝路径只落日志：界面上首页本来就没有「×」，Ctrl+W 落到它身上应当是「什么都没发生」。
pub(crate) fn request_close_tab(app: &tauri::AppHandle, label: String, token: String) {
    if !tab_token_matches(app, &label, &token) {
        dlog!("[tabs] 拒绝 Ctrl+W 关页签：label/token 与在册 tab 不匹配（label={label}）");
        return;
    }
    dlog!("[tabs] 收到 Ctrl+W，关闭页签：{label}");
    close_tab_inner(app, &label);
}

/// 计算 `target="_blank"` / `window.open()` 弹窗在**新 tab** 中应该落在哪里。
///
/// 规则本体在 [`crate::nav::resolve_popup_target`]（导航原语，跨平台 —— Android 的
/// `mobilebar::create_tab_with_url` 复用同一份）。这里只负责从 `TabManager` 取本壳的
/// `Navigation` 再转交。
pub(super) fn resolve_popup_url(app: &tauri::AppHandle, target: Option<&str>) -> Option<String> {
    let nav = app
        .state::<TabManager>()
        .navigation
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()?;
    crate::nav::resolve_popup_target(&nav, target)
}

/// 处理来自 WebView2 `NewWindowRequested` 的新窗口请求（target="_blank" / window.open()）。
///
/// 仅多页签模式下有意义：不拦截到当前窗，而是新建一个 tab，并导航到弹窗请求的目标地址
/// （落点由 [`resolve_popup_url`] 计算；算不出来时回落到入口地址）。
/// `opener` = 发起这次打开的 tab label：记到新 tab 上，下载后自动关页签时回到它。
pub(crate) fn open_new_tab_from_request(
    app: &tauri::AppHandle,
    target_url: Option<String>,
    opener: Option<String>,
) {
    let start = resolve_popup_url(app, target_url.as_deref());
    // 建 tab 时直接带上目标地址做**一次**导航（历史实现是先建窗导航主地址、再二次导航改地址，
    // 既多一次请求，也与首次导航存在竞态）。
    let _ = spawn_tab_with_target(app, start, opener);
}
