# webview-app（Rust + Tauri 2 跨平台 WebView 壳）

嵌网址+托盘+快捷键+代理/Basic 鉴权。仓库 `woxiangfy/web2app`（main/SSH）。多页签 `src/tabs.rs`，加密 `crates/resolve-core`。

## 远程 origin 与 capability（2026-09-14 / 2026-09-16 补坑）
- ⚠️⚠️ Tauri 2 对**非 local origin** 的 IPC **强制 ACL**（`Webview::on_message`：`plugin_command.is_some() || has_app_acl_manifest || !is_local` 时校验，未命中 capability 一律拒）。Windows 上只有 `http://<scheme>.localhost` 算 local（`is_local_url` 是「scheme==tauri 协议 scheme && domain == `<已注册协议名>.localhost`」），**`http://127.0.0.1:port` 与公网域名都算 remote** → 单窗口模式主窗（远程页）里所有 IPC 都需 capability 放行，否则前端 `.catch()` 静默＝「点了没反应」。
- ⚠️⚠️⚠️ `remote.urls` **必须写端口通配 `:*`**（2026-09-16 真机 bug）：tauri 用 **URLPattern** 匹配（`tauri-utils::acl::RemoteUrlPattern`），而 URLPattern 里**「不写端口」＝只匹配该 scheme 的默认端口**（`Component::compile(None)` 给 `*` 的前提是 init.port 为 None；构造函数字符串 `http://*` 解析出的 port 是 `""`，于是只匹配空端口）。实测：`http://*` 匹配 `http://127.0.0.1:18200/` = **false**，`http://*:*` = true；`https://*:*` 也能匹配 8443。⇒ 旧配 `["http://*","https://*"]` 只在**直连 https://域名（443 默认端口）**时按钮可用，一旦走**本地反向代理（端口随机）**或自定义端口站点，最小化/最大化/关闭/拖动/双击最大化（`plugin:window|*`，本地 origin 也要过 ACL）**全部被拒**，用户看到的就是「按钮又失效了」。现配 `["http://*:*","https://*:*"]`。ACL 拒绝的原文形如：`window.toggle_maximize not allowed on window "main", webview "main", URL: http://127.0.0.1:18200/ ... allowed on: [windows: "main", URL: local], [URL: http://*], [URL: https://*]`。
- ⚠️ 反过来：**remote 页里的 IPC 失败是静默的**，所以注入脚本必须 `console.error('[web2app] 窗口命令 X 被拒绝：', err)`，**禁止** `.catch(function(){})`（否则排查只能从零开始）。
- `capabilities/default.json`：`windows:["main"]` + `remote.urls: ["http://*:*","https://*:*"]` + 5 个 core:window 权限（minimize/toggle-maximize/close/start-dragging/internal-toggle-maximize）。目录＝`CARGO_MANIFEST_DIR/capabilities/**/*`（tauri-build）。多页签模式宿主窗口 label 是 `host`，但宿主/标签栏走的是 **app 自定义命令**且 origin 是 local ⇒ 不需要 cap。
- ⚠️ app 自定义命令（history_nav/*_host 等）**无法**授权给远程 origin（要 build.rs 的 app_manifest）→ 注入脚本里别 invoke 它们；前进/后退直接用 `window.history`。


## 注入脚本（loading.js / titlebar.js / splash.html）
- ⚠️⚠️ 注入时机＝「全局对象已创建、**HTML 尚未解析**」，实测 `documentElement`/`head`/`body` **全为 null**；直接 `appendChild` 抛 TypeError 被 try/catch 吞掉 → **顶部栏与加载动画双双不出现**（「无多标签时没有顶部菜单栏」+「首次打开没加载动画」）。必须 `whenHostReady` 等宿主：MutationObserver 盯 `document` childList + interval/readystatechange 兜底（实测只等 6ms）。
- ⚠️ 首个文档是否注入：**wry 0.55.1 实测「先 `AddScriptToExecuteOnDocumentCreated` 完成、再 Navigate」→ 首文档也有脚本**（`examples/probe_new_window.rs`，探针里首个文档打印出 `[INJ]`）。旧注释「首文档拿不到脚本」不成立；`splash.html` 内联标题栏无害可留，但别再据此推断其它问题。
- `bodyEmpty()` 须跳过 `#__web2app_titlebar__`；标题栏占顶 38px → `window.__web2appChromeH`+`web2app:chrome-ready` 让加载层让出。
- ⚠️⚠️ **顶部空间契约**（2026-09-14 重写，**别改回 `html,body{padding-top:38px}`**）：只在 `html` 上加 `padding-top:38px!important` + `box-sizing:border-box!important`（+`min-height:100%`）。旧写法两边各加一次、body 又在 html 内容框内 ⇒ 实际留白 **76px**＝用户截图里的「顶部一整条空白」；border-box 是为了让页面 `height:100%` 链仍等于「视口−38」，不多出 38px 溢出。
- ⚠️⚠️ **底部空间契约**（2026-09-16 追加）：`100vh`/`min-height:100vh` 量的是**视口**高度，而视口并没变小（我们只是拿掉了 38px 布局高度）⇒ 底部锚定的容器整条被挤出视口下沿 38px＝「页面底部存在部分被下面覆盖」。实测 `/vh.html`：`#root{min-height:100vh}` top=38 **bottom=688**（viewH=650），页脚 632..688 整条在屏幕外。
- ⚠️⚠️ 解法＝`applyChromeFit()`（原 `shiftPageTopBars`，一个扫描干三件事；两阶段先只读收集再统一写，避免逐元素重排）：
  ①**贴顶**的 fixed/sticky（`rect.top ∈ [-1,46]`，声明 top<38；absolute 只认 `rect.top ≤ 1`＝包含块是初始包含块）→ 打 `data-web2app-chrome-shift` + inline **`top:38px!important`**；
  ②**底部锚定**（computed `bottom ≠ auto`）的贴顶元素**只改 top**：元素被【缩短】而非下移，底边仍钉视口底 ⇒ 整屏外壳（`fixed;inset:0`）既不被压头也不被切尾（旧版把这类整类排除，正是「顶部 38px 一直压着」的老边界，现在解决了）；
  ③**高度≈视口高度**（±1px）且底边已越过视口底的容器 → 打 `data-web2app-chrome-shrink` + inline `height/min-height: calc(100vh - c)`，`c`＝38（元素会被下移时）或它离文档顶的实际距离（容忍 body 默认 8px 边距），**夹在 [38,54] 内**（只补偿标题栏这一档，不做大改写）。
  识别条件全部基于实测几何、认不出就什么都不做；多轮扫描（40 次/300ms 间隔 + MutationObserver）能让嵌套容器逐层收敛。
  复测（2026-09-16）：`/vh.html` root 38..**650**、页脚 594..650；`/inset.html` 外壳 38..650（页头 38..82、页脚 606..650）；`/fixed.html` 页头仍 38；`/sticky.html` 滚到 700 时 sticky 头仍 38；`/plain.html` 正常流从 38 起。仍存边界：`position:absolute` 且包含块不是初始包含块的整屏容器不处理；真正的「原生内缩」（把 WebView2 bounds 下移 38px）能让 `100vh` 天然正确，但那要标题栏改由原生绘制＝另一套架构。
- ⚠️ 窗口按钮句柄**不要在挂载时缓存**（2026-09-16）：`wire()` 里旧写法把 `getCurrentWindow()` 结果缓存在闭包，拿不到就整段不绑定 ⇒ 按钮永久失效且毫无日志。现在 `currentWindow()` 每次点击现取（并兜底 `__TAURI__.webviewWindow`）。
- 复测：(a) `cargo run --example probe_dom_ready`（不可见窗口）；(b) 布局/IPC 用 `%TEMP%\w2a_probe2\`：`bash run_probe.sh <fixed|sticky|plain|vh|bottom|inset|diag>` 一键跑（起站点→跑 app→汇总几何/诊断）。页面用 `<img src="/beacon?d=<json>">` 回传 `marks`（任何 `data-mark` 元素的 rect/pos）/`barRect/hdrRect/htmlPad/scrollY/innerH/docH`；`diag` 场景回报 `typeof __TAURI__`、close 按钮 `elementFromPoint` 命中测试、`toggleMaximize()` 的 ok/拒绝原因。**不要指望截屏**：WebView2 走 DWM 合成，GDI `CopyFromScreen` 拿到的是下层窗口、`PrintWindow(PW_RENDERFULLCONTENT)` 返回全黑（实测 99.4% 同色），`--disable-gpu` 也无效。
- ⚠️ 探针坑：被遮挡窗口的 `setTimeout/setInterval` 被节流 ⇒ 上报必须在 `load`/`DOMContentLoaded` 里**同步**发（`setTimeout(…,0)` 不行）；app 是单实例，连跑要等上一个进程退干净，偶发「no beacon.log」重跑一次即好；exe 是编译期配置，跑探针必须用 `APP_URL=http://127.0.0.1:18200/start.html PROXY_ENABLED=false APP_NAME=探针` 重新 build。
- ⚠️ `whenHostReady` 的硬兜底**别写「setTimeout 3s 后只试一次」**：那一次若仍拿不到宿主节点就直接 return，而防重标记已置位 ⇒ **永久放弃挂载**。正确写法：持续重试到 20s + 监听 `load`/`DOMContentLoaded`；挂载失败时把 `__web2appTitlebar` 置回 false 供后续补挂（loading.js 同款）。
- 复测：`cargo run --example probe_dom_ready`（不可见窗口）。

## 新窗口请求 / 单窗口模式（2026-09-14 现场实验定案）
- ⚠️⚠️ **禁用 `SetNewWindow(&当前 core)`**：WebView2 会复用当前 webview 承载目标内容，但**该文档不执行** `AddScriptToExecuteOnDocumentCreated` 注册的初始化脚本 ⇒ 注入的标题栏与加载层整片消失（＝「点开其他页面后顶部菜单栏全部消失」的根因）。MSDN 要求 `NewWindow` 是「另建 webview，同 environment 且**未被导航过**」；传自己的 core 属滥用，页面里 `w.document.write()` 还会改写主页面。
- 正确做法：`args.SetHandled(true)` + 后台线程 → `run_on_main_thread` → `window.navigate(url)`；`about:blank` 忽略并打日志（单窗口模式给不了独立窗口）。代价：`window.open()` 返回值不再是可用窗口对象。
- 兜底：主窗 `.on_page_load(Finished)` 里再 `eval(titlebar.js)`（自带防重 ⇒ 正常路径 no-op）。

## 定位「注入脚本没生效」的手段（wry 探针会骗人，别再用）
- 判据别用 `document.title` 通道：初始化脚本在 document-created 跑时 `<title>` 还没解析，`[INJ] ` 与随后解析出的 `page B` 是**同一文档**的两次 title 变化 → 极易误判成「新文档也有注入脚本」（本轮就因此得出过错误结论）。
- 可靠做法：真实 app + 自建本地站点（`APP_URL=http://127.0.0.1:<port>/x.html PROXY_ENABLED=false APP_NAME=探针`，`parse_domains` 只认 JSON 数组 → 普通 URL 直连），页面每 400ms 用 `<img src="/beacon?d=<json>">` 回传 `#__web2app_titlebar__` 是否存在 / rect / computedStyle / **`elementFromPoint(60,20)`**（是否被盖住）/ `body.children.length` / `performance.timeOrigin`（区分文档）。
- 判据：`bar:false` 且 `body.children.length` == 页面自身节点数 ⇒ 该文档**根本没执行注入脚本**。

## 配置语义
- `app_name`=安装名（Win productName+数据目录）；`title`=显示名（Win 标题/Android `strings.xml`），缺省回落 `app_name`。⚠️ 编译期变量仍叫 `WINDOW_TITLE`。
- ⚠️ `RESOLVE_SECRET` 缺省 `web2app-fixed-secret-2026`，改 Secret **两端必须同步**；私钥须 48B 纯 PKCS#8 DER（dalek 默认 83B）。

## 多页签（仅 Win，`TABS_ENABLED` 默认 false）
- 宿主窗口 + `shell://localhost/` 标签栏（`shell.html`）；tab 是独立 WebviewWindow，共享 WebView2 data_directory → 代理单实例单端口。新 tab **只**由 `target="_blank"`/`window.open()` 触发。
- ⚠️⚠️ tab 必须设成宿主的 **owned window**（`.owner(&host)`）：否则点宿主**任何**位置（含标签栏空白拖动区）都会把宿主抬到 tab 之上 → 空客户区盖住网页。
- ⚠️ 关闭子窗口一律 `destroy()`，**禁用 `close()`**（`close()`→`CloseRequested`→`prevent_close()`+`close_tab_inner` → 无限重入＝卡死）。
- ⚠️ tab 不随宿主隐藏/最小化/销毁：隐藏→`set_tabs_visible(false)`；最小化→`sync_tabs_with_host`（`Resized`+`TABS_SUSPENDED`）；退出→`destroy_all_tabs()`。

## 无边框窗口几何（Win）
- ⚠️ 客户区内缩**只与 DPI 有关、与 `resizable` 无关**（`MARKER_UNDECORATED_SHADOW`→`calculate_insets_for_dpi` 只看 dpi，每边 `SM_CXSIZEFRAME+SM_CXPADDEDBORDER`，150%≈11px）。宿主与 tab 都 `.shadow(false)` → `outer == inner`（无系统投影/Win11 圆角）。
- ⚠️ 对齐用**闭环校准** `place_tab`（set→读回 `inner_*`→补偿 ≤3 轮），显示后再补一次；标签栏 38 逻辑px×scale 后 **floor**。

## WebView2 钩子（`webview_hooks.rs`）
- ⚠️ `with_webview` 在主线程**内联同步执行** → 闭包 sleep 等 controller ＝**死锁 + 白卡 5s**；须**后台线程**循环、每次试一次、5s 上限。
- ⚠️⚠️ 非主线程调用走 `proxy.send_event` **异步投递** → 重试使多个闭包同时在途、每个都注册 → 同窗口挂多个 `NewWindowRequested` → **一次点击开 N 个窗口、正反馈卡死整机**。去重只能在 `register_webview_hooks` 开头用 `static HOOKS_REGISTERED` 一次性登记。
- ⚠️⚠️ 判成功的 `done` 标志**必须全函数共用、且在【轮次开头】检查**。旧实现每轮新建 `done` 并在 `with_webview` 返回后立刻检查 → 异步投递时闭包尚未执行，必然读到 `false`，而闭包置位的是**上一轮已丢弃**的标志 ⇒ **哪怕第一次就注册成功也必然空转到 5s**，打出「CoreWebView2 在 5s 内未就绪…未注册」的**假告警** + 白排 ~100 个闭包。修后实测：`窗口 main 的钩子已注册（尝试 1 次，耗时 50ms）`。判断这条日志真假的方法是看**同窗口是否出现了业务日志**（如「拦截到新窗口请求」）——出现即证明钩子已注册。
- `resolve_popup_url` 同源须比**入口主机**或 `upstream_host`；`bootstrap_inner(cache_navigation=true)` 才写 `TabManager.navigation`，两处入口都必须 true。
- ⚠️⚠️ 下载（`DownloadStarting` + `src/save_dialog.rs`，2026-09-14）：**光设 `ResultFilePath` 不会静默**——MSDN：`Handled=true` 才隐藏默认下载 UI 且「下载照常进行」，默认 `false` ＝弹默认下载 UI；`Cancel=true` 则任何情况下都不弹。路径已存在会被**直接覆盖**（旧实现因此既无交互又会覆盖同名文件 ⇒ 用户「点下载没反应」）。现在同步弹系统 `IFileSaveDialog`：确认→`SetResultFilePath`+`SetHandled(true)`；取消→`SetCancel(true)`；建不出对话框→**什么都不设**。
- ⚠️ 保存对话框**必须主线程同步弹**（tao `window.rs:1450` / wry `webview2/mod.rs:115` 都已 `CoInitializeEx(APARTMENTTHREADED)` ⇒ 主线程 STA）：换后台线程会丢真模态，且 windows-rs 的 COM 接口**不是 Send**（`IUnknown(NonNull)`），`args`/`deferral` 送不回主线程；但 `webview2_com::EventClosure` **无 Send 约束** → 回调闭包可捕获 `HWND`。加 feature `Win32_UI_Shell`/`Win32_UI_Shell_Common` 即可，无需新 crate。

## 构建 / CI / cfg
- `env_bool!` 在 const 上下文不能用 `==`/`match`/`&&`/`||`（E0658）→ `const fn str_eq`+if-else。`generate_context!()` 需 PNG 图标；`productName`/`mainBinaryName` 在 `tauri.conf.json` **顶层**。
- workflow 内嵌 Python f-string 不能顶格；`build-custom.yml` 模板须纯 JSON。rustc 1.95 增量编译偶发 ICE：删 `target/debug/incremental`，**别 clean**（5.9G）。
- Android：`strings.xml` app_name 转义单引号；MainActivity **只注入不重写**；`app_config_dir()` **不拼 identifier**；不能本地交叉编译 → `--cfg fake_android` 验两分支。
- ⚠️ `#[cfg(...)]` 调用点必须同 cfg、尽量**下沉到平台专属那一行**；`#[cfg(windows)]` 在 Ubuntu clippy 不编译 → 每平台都跑 clippy。
- ⚠️ `android-actions/setup-android@v4` 的 `packages` **默认值＝`tools platform-tools`**，而独立 `tools` 包已被 Google 下架 ⇒ sdkmanager 报 `Failed to find package 'tools'` 并 **exit 1**，失败在 action 内部（报错栈落在 `_actions/.../dist/index.js`）⇒ 三条 workflow 全中招（2026-09-15）。必须在 `with:` 里显式 `packages: 'platform-tools'`。同 action 的 `cmdline-tools-version` 默认（现 14742923＝20.0）也会随 tag 移动；要免疫 tag 移动就钉 SHA（`v4` = `40fd30fb8d7440372e1316f5d1809ec01dcd3699`）。

## 待办
- 真机复验（单窗口模式）：① 顶部栏 + 加载动画 ② 最小化/最大化/关闭按钮（2026-09-16 定位到 ACL 端口通配并修好，探针 `ipc phase=ok`，**待真机在真实站点确认**）③ 拖动 + 双击最大化（同上，同一条 `plugin:window|*` ACL，需一并复验）④ 点开其他页面后顶部栏仍在（本地已复验通过，待真机确认）⑤ **顶部占位只留 38px、页面自己的头部不被切**（本地用 fixed/sticky/普通流三种页面复验通过，待真机在真实站点上确认）⑥ 多页签：点标签栏空白处网页仍在 / 一次点击只开 1 个窗口 / 新 tab 落目标 path / 几何严丝合缝 / 连续关 tab。
- ⑦（2026-09-16 新）**页面底部：`100vh`/`min-height:100vh` 容器的页脚不再被窗口下沿吞掉** —— 本地用 vh / 100vh+fixed 底栏 / fixed 外壳三种页面复验通过，待真机在真实站点确认（识别靠几何启发式，真实站点若仍被切，先看该元素有没有被 `data-web2app-chrome-shrink` 标记，再调识别条件）。

