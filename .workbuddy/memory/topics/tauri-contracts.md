# Tauri 侧硬契约（从 MEMORY.md 迁出，内容未改）

> 全量细节另见 `archive-2026-09-16-thinbefore.md`。本文件是「动手前必读」的浓缩版。
> ⚠️ 本文属**单一真源清单的一部分**：`webview-app/src/{titlebar.js,loading.js,splash.html,shell.html}`、`dist/settings.html`、`icons/*`、`src/{proxy.rs,resolve.rs}`→sidecar `w2a-proxy.exe`；改任一 ⇒ 必跑 Electron `probe`(134) + `probe --tabs`(31) + Tauri 页面探针。

1. 远程 origin 走 IPC 必过 ACL；`remote.urls` 必为 `["http://*:*","https://*:*"]`（缺端口通配 ⇒ 窗口按钮**静默**全失效）。注入 = `whenHostReady` + 重试到 20s，失败置 `__web2appTitlebar=false` 供补挂。禁空 `.catch`；注入脚本禁 invoke 自定义命令 —— 想在远程页面里触发「应用级动作」只能发**核心插件的事件**（如标题栏「设置」按钮的 `web2app:open-settings`，配 `core:event:allow-emit`），见 §11。
2. ⚠️⚠️ 顶部空间 = **`html` 不滚、`body` 滚**：`html{height:100%;overflow:hidden}` + `body{margin-top:38px;min-height:0;height:calc(100vh-38px);overflow-y:auto}`（必须 `margin-top` 非 padding；`min-height:0` 防 `min-height:100vh` 撑高）。配 `shimWindowScroll()` 转发 scrollY/pageYOffset/scrollTo/scroll 及 `Document.scrollingElement`→body，`ensureScrollRoot()` 三段探测须在样式表挂载之后，失败退回 html 留白旧模式。判据成对：`intrudeRows==[]` ∧ `bodyScrollbarW>0` ∧ `bodyScrollable` ∧ `scrollTo(0,200)→scrollY≈200`；**禁用** `rect.width==innerWidth`。
3. ⚠️⚠️ `Builder::header` 是 **append** 非 replace ⇒ 须在透传循环里跳过它，否则两个 `Origin`（逗号拼接）= 页面能开、WS/POST 全挂。回归 `proxy_sends_exactly_one_rewritten_origin`。
4. `with_webview` 主线程内联同步 = 死锁+白卡 ⇒ 后台线程 + `static HOOKS_REGISTERED` 一次性登记；另存为**只能主线程同步弹**。新窗口禁 `SetNewWindow(&当前 core)`（复用文档不跑初始化脚本 ⇒ 标题栏消失）⇒ `SetHandled(true)` + 主线程 `window.navigate(url)`；`about:blank` 忽略。多页签（仅 Win，`TABS_ENABLED` 默认 false）：tab 须宿主 **owned window**，关子窗用 `destroy()` **禁** `close()`，`place_tab` 闭环 ≤3 轮。
5. sidecar：上游地址走 **stdin**；stdout 只一行握手 JSON 且必 flush（日志走 stderr）；防僵尸三层 = will-quit stop → 关 stdin → 1s 强杀 → 自监视 EOF。
6. 配置/构建：`app_name`=安装名、`title`=显示名（缺省回落）；`parse_domains` 只认 JSON 数组；私钥须 48B PKCS#8 DER；`RESOLVE_SECRET` 两端同步。`env_bool!` 用 `const fn str_eq`；增量 ICE 删 `target/debug/incremental` **别 clean**；Tauri 侧**别指望截屏**（DWM 合成）⇒ 页面回传探针 `%TEMP%\w2a_probe2\`（`run_probe.sh <fixed|sticky|vh|bottom|inset|diag>`），上报在 `load` 里同步发。
7. ⚠️⚠️ **`use` 的 cfg 必须与使用点的 cfg 一致**（2026-09-22 踩，`settings.rs` 的 `APP_NAME`）：本项目 `lib.rs` 有全量 `#![allow(dead_code)]` ⇒ **Android 目标下唯一会拦的 lint 就是 unused import**。只要某个名字的**全部使用点**都在 `#[cfg(desktop)]`（或 `#[cfg(windows)]`）里，它的导入就必须同样加 cfg，否则桌面/Linux 恒不报、Android（`desktop` 为假）直接 `-D warnings` 失败。给 item 加 cfg 时**同步检查它的导入**。把 `cfg(not(target_os="android"))` 这类写法批量换成 `cfg(desktop)` 属于语义替换，**必须连带复核导入**。
   - 本机**做不了** Android 门禁：`cargo clippy --target aarch64-linux-android` 会在依赖层的 `ring` build script 处失败（需 `aarch64-linux-android-clang`，本机无 NDK），只有 CI 有 NDK ⇒ 改动后要确保 CI 能跑。
   - 本机能跑的是等价 check-windows 门禁：`fmt --check` / `clippy --all-targets -D warnings` / `test --all` / `TABS_ENABLED=true check --all-targets`。

8. ⚠️⚠️ **IPC 命令注册表只能有一次 `.invoke_handler(...)` 调用**（2026-09-23 踩，用户报「保存失败：Command save_settings not found」）：
   - `Builder::invoke_handler` 是**替换**不是追加（tauri-2.11.5 `src/app.rs:1662` = `self.invoke_handler = Box::new(…)`；AppManager 只持有这一个 handler）。`lib.rs` 曾按模块写成两次调用（多页签那次在 `#[cfg(windows)]` 块里）⇒ 后一次**整体覆盖**前一次，`get_settings` / `save_settings` / `close_settings_window` / `clear_web_data` / `history_nav` 在 **Windows 上全部消失**（与 `tabs_enabled` 无关）。
   - 现象：前端报 `Command {name} not found` —— 由 handler 对未知命令返回 `false`、tauri `src/webview/mod.rs:1911` 据此 `reject`。**编译期零警告、CI 全绿**，只在用户点按钮时现形。连带受害：设置页 4 个命令全挂、标题栏前进/后退（`history_nav`）全挂。
   - 平台差异一律写成**命令项上的 `#[cfg(...)]`**（`generate_handler!` 支持逐项属性：宏用 `Attribute::parse_outer` 解析每个 path，并把 attrs 重新发到对应 match 分支上），**不要**为分平台再开第二次调用。
   - 回归守卫：`src/lib.rs` 的 `ipc_command_registry` 测试（① 调用点计数唯一 ② 前端 `invoke("…")` 名 ⊆ 注册名 ③ 必需命令齐备 ④ 抽不出命令即失败，堵假绿）。计数「针」用 `concat!(".invoke","_handler(")` 拼接 ⇒ 本文件自己的注释/断言文案不会被算成调用点（写注释时别写出完整字面量）。
   - 另一条**不同**的错法（本次已排除，但仍要会分辨）：tauri 2.5+ 会把 ACL 未列出的应用命令**编译掉**（`tauri-macros` `filter_unused_commands` → 打印「Removed unused commands from application: …」），前提是存在 app 级 ACL 清单（`has_app_acl`）。本项目 `capabilities/default.json` 只引用 core 权限、`gen/schemas/acl-manifests.json` 无 application 段 ⇒ `has_app_acl=false` ⇒ 不过滤。若哪天新增 app 级 `permissions/*.toml`，这条就会变成新的「命令不存在」来源。
9. ⚠️⚠️ **「清除全部浏览数据」`clear_web_data`（`src/web_data.rs`，2026-09-22 加，真机探针取证过）**：
   - 命令**必须 `async`**：同步命令在主线程**内联**执行 ⇒ 等完成回调时把主线程消息泵堵死，而那个回调正靠这个泵派发 = 自锁。`with_webview` 也必须从**后台线程**投递（异步投递）。实际形态 = `spawn_blocking` → 后台线程反复投递直到 controller 就绪 → 发起 `ClearBrowsingDataAll` → 另一处 `recv_timeout` 等结果。
   - **完成回调不能当成功/失败的判据**：真机探针 10+ 次运行里回调**一次都没执行过**（上游 wry 的 `clear_all_browsing_data()` 干脆是 fire-and-forget、handler 1.1µs 即释放），而**数据确实被清掉**。⇒ 只在**回调带回 `Err`** 时返回错误；回调不来只如实说明并**照常重载页面**（⚠️ 别用 `?` 提前 return —— 那会跳过重载，用户看到的就是「按钮白点」）。
   - 清除刚结束时该 WebView 的**导航会被静默丢弃、甚至直接阻塞**（`load_url` 可能既不 `Err` 也不生效）⇒ 重载前留 `RELOAD_SETTLE`=1.2s。
   - ⚠️ `cookies_for_url()`（wry 现成 API）在清除未落定时会**挂死**（内部 `wait_with_pump` = 阻塞 `GetMessageA`），别放进主线程当判据。
   - 平台：模块与命令**不按平台 gate**（只在实现体内 `#[cfg(windows)]` 分叉），否则 Android 上会退化成「命令不存在」而不是明确报错。
   - 取证探针：`examples/probe_clear_data.rs`，三态退出码（0/1/2），对照组开关 `PROBE_CHECK_ONLY` / `PROBE_SKIP_CLEAR` / `PROBE_USE_WRY_CLEAR` / `PROBE_VISIBLE`。⚠️ 跑前先 `cargo build --example`（`cargo clippy` 不重建 example 二进制）；反复跑会让 `<exe>.WebView2` 变脏，改名备份即可恢复。
   - 双壳对齐：Electron 侧命令名逐字一致（`tauri:clear_web_data`），设置页字面量 ↔ preload 白名单 `ALLOWED` ↔ IPC 注册名三处必须同改。

10. ⚠️⚠️ **运行期只有一个 JSON：`settings.json`（设置平铺顶层 + `window` 段几何）**（2026-09-23 合并，用户要求「只需要一个 JSON」）：
   - 契约落在 `src/store.rs`：`WindowGeom` + `Document` + `load`/`save`/**`update`**。`settings.rs::persist_settings` 与 `window_state::save` 都只是薄封装。
   - **合并的唯一真风险 = 两个写入方互相覆盖**（设置页保存 vs 关窗记几何；`window-state.ts` 文件头当初正是用这个理由论证不该合并）⇒ 所有写入走 `store::update()`（读盘 → 只替换自己那一段 → 写回）。**禁止**拿内存里的整份 `Document` 直接 `fs::write`。回归守卫：`store::tests::{writing_settings_keeps_window_geometry_on_disk, writing_window_keeps_settings_on_disk}`（打真临时文件）。
   - 第二个风险 = **一段坏掉拖垮另一段** ⇒ `parse_document` 两段独立解释：`window` 段坏只是「这次没几何」，三个开关照读；顶层 JSON 坏才整份回默认。
   - 字节形态用 `DocOut{#[serde(flatten)] settings, window}` 固定键序（设置三键 → `window`），与 Electron `writeDocument()` 一致；`store::tests::pretty_output_is_canonical_json` 断言**逐字节**字面量。
   - ⚠️ `WindowGeom` 必须定义在**非 desktop-gated** 的模块里（`window_state.rs` 是 `#[cfg(desktop)]`，Android 编译得不到它）—— 见本文件 §7 的 cfg 教训。
   - 旧 `window-state.json` **不迁移**（用户已确认）：不再读也不再写，升级后窗口回一次默认尺寸/位置，文件可手工删。
   - ⚠️⚠️ **落点 = 数据目录**（2026-09-23 追加，用户反馈「应与 `data_dir` 定义的行为一致」）：
     `config.rs::store_dir`（`#[cfg(windows)]` = `data_dir`；非 Windows = `config_dir`）决定**写入位置**，
     一律 `<data_dir>\settings.json`（与 WebView2 数据目录、`logs\` 同一个 ⇒ 绿色版搬目录不再漏掉设置）。
     **读取候选 = 新位置优先、旧位置兜底**（`store::candidates` = `store_dir` → `config_dir`，去重），旧文件**不迁移不删除**。
     ⚠️ **读-改-写的「读」也必须走同一份候选**（`store::update_with` / Electron `readRawForUpdate`）——
     若「读」只读写入位置（新位置为空）而「写」写新位置，**升级后第一次保存会把老设置清零**，且不报错。
     守卫：`store::tests::update_reads_legacy_but_writes_new_location`（断言旧文件**字节不变**）
     + Electron 探针「落点搬家」2 条。`config_dir` 现在**只用于读旧位置**。

11. ⚠️ **无边框设置窗 + 「打开设置」的三条通路**（2026-09-23 重做设置界面时定下）：
   - 版式契约（`dist/settings.html`，两壳共用）：窗口**无系统标题栏**（Tauri `decorations(false)` / Electron `frame:false`），
     整页只有一块**铺满整窗**的面板（`html/body` 都 `overflow:hidden`，**唯一滚动容器是 `.panel-bd`**
     （`flex:1; min-height:0; overflow-y:auto`；少了 `min-height:0` 子项不收缩 ⇒ 永远不出滚动条），
     底部操作区固定（「保存」不被滚出视野）。面板头部带 `data-tauri-drag-region`（**标题那一格也要带** ——
     bare 值语义是「只有点在带属性的那个元素本身上才拖动」）。
   - ⚠️ **2026-09-23 由「居中 + 四周底衬」改为「铺满整窗」**（用户报「大块空边框」）：620 宽窗里
     `body` 的 12px padding 与 `.panel` 的 `max-width:560px` 各留 **30px** 空档（实测面板 rect `[30,12,560,577]`）。
     改法：去掉 padding / `max-width` / border / 圆角 / 外阴影，**头部补一条 `border-bottom`** 作为窗口边界的视觉替代
     （不然满铺面板与窗口边缘无分界）。改动后实测 `[0,0,620,601]`、`border:0px`、`radius:0px`、`shadow:none`、头部下边框 `0.667px`。
     ⚠️ 元素 id 与脚本**一字未改** ⇒ 两壳同时受益，无需改 preload / IPC。
   - ⚠️ 无边框 ⇒ 拖动需要 capability：`capabilities/settings-window.json`（`windows:["settings"]` +
     `core:window:allow-start-dragging` / `allow-internal-toggle-maximize`）。**少了它窗口拖不动、且只有控制台一条被拒绝的调用**，
     不报错、不弹窗。设置窗与主窗的权限**刻意分开**（设置窗不给 minimize/close：关窗走应用命令 `close_settings_window`，本地 origin 不需要 ACL）。
   - 打开设置的三条通路（都落到同一个 `open_settings_window()`）：托盘菜单 / Shift+F1 / **标题栏「设置」按钮**（**最小化左侧**，2026-09-23 起）。
     ⚠️ 按钮顺序 = `[← 后退] [→ 前进] 标题 [⚙ 设置] [— 最小化] [▢ 最大化] [✕ 关闭]`（原为「最小化右侧」）；
     多页签标签栏 `shell.html` 同步，但 `data-page-node-id` **保持不变**（改了会破坏 Builder.io 映射）。
     ⚠️ 探针的位置判据只比 `getBoundingClientRect()` **几何**、不看 DOM 顺序（防「DOM 对但 CSS 挪走」）；
     改按钮位置必须同步改 `probe.ts` 两条断言（标题栏 + 标签栏）与 `shell.html`。
     · 单窗口（远程页面）：Electron = preload 白名单 `openSettings()` → IPC `win:open-settings`；
       Tauri = `plugin:event|emit`（`core:event:allow-emit`，见 `capabilities/default.json`）→ `lib.rs` `app.listen("web2app:open-settings")`。
       **不能用应用命令**（远程 origin 强制过 ACL，应用命令没有 app 级清单可授权 —— 同 §8 第二条）。
     · 多页签（本地 `shell://` 页面）：直接 `invoke("open_settings")`（新注册的 `#[cfg(desktop)]` 命令）。
     ⚠️ 新增这条命令后 `ipc_command_registry` 的「必需命令」多了一条桌面端断言；前端字面量集合也因此多一个 `open_settings`（`shell.html`）。
   - ⚠️ 别再为了「让远程页面也能用应用命令」去加 app 级 `permissions/*.toml`：那会让 `has_app_acl=true`，
     进而触发 §8 那条 `filter_unused_commands` 连带风险（命令被编译掉）。
