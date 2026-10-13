# Web2App

> 把一个网址打包成桌面 / 手机原生应用的轻量工具。
> **Rust + Tauri 2** 编写，支持 **Windows（WebView2）** 与 **Android**。
>
> 仓库级规则、目录地图与文档索引见根目录 `AGENTS.md`。Windows 免安装绿色版由另一个壳
> `electron-app/`（Electron + TS）交付，两者共用同一份注入脚本与 `config.json` 契约 ——
> 改动**单一真源**（`src/{titlebar.js,loading.js,tab_title.js,dnd_polyfill.js,shell.html,splash.html,floaticon.html}`、`src/{proxy.rs,resolve.rs}`、
> `dist/settings.html`）后必须双壳同步验证，详见根 `AGENTS.md`「3.4 其它全局约定」。

Web2App 启动后直接内嵌并打开你预设的网址，可最小化到系统托盘、用全局快捷键一键显隐；
当网址带 `https://用户名:密码@host` 形式的 Basic 鉴权时，会自动给**每一个请求（含 WebSocket）**注入凭据。

---

## 功能特性

| 功能 | 说明 | 平台 |
| --- | --- | --- |
| 内嵌网址 | 默认打开编译时写死的网址，全屏/可缩放窗口 | Windows / Android |
| 设置界面 | 按 **Shift+F1** 打开；可配置唤醒快捷键、托盘开关、关闭行为（持久化到 `settings.json`），并可一键**清除缓存 / Cookie**（全清） | Windows |
| 系统托盘 | 可开关；最小化到托盘，左键显隐、菜单可退出（受设置控制） | Windows |
| 全局快捷键 | 一键切换窗口显示 / 隐藏；按键与修饰符在设置里配置（**默认不启用**） | Windows |
| Basic 鉴权 | 支持 `https://user:pass@host`，自动注入 `Authorization` 头 | Windows / Android |
| WebSocket 鉴权 | 本地反向代理对 WS 握手统一注入凭据，覆盖实时同步场景 | Windows / Android |
| 外部 HTTP/HTTPS 代理 | 可配置系统级代理（仅 Windows WebView2 生效；2026-09-27 起多页签的宿主与 tab 窗口同样挂载，避免整组页签绕过代理） | Windows |
| 新窗口拦截 | `target="_blank"` / `window.open()` 强制在当前窗口打开 | Windows |
| 多页签（tab） | 编译期开关 `TABS_ENABLED=true` 开启（默认关）：宿主窗口 + 顶部标签栏，每个 tab 独立 WebView、共享同一份数据目录（同账号），可切换/关闭；**首个 tab（入口 app_url）是「首页」：固定在第一位、不允许关闭，名字前带主页图标**（判定单一真源 `tabs.rs::is_home_index` ∥ Electron `tab-order.ts::isPinnedHome`，两者必须同答案；`get_tabs` / `listForIpc` 下发四元组 `[label,title,active,home]`，`shell.html` 据此画图标 / 不画 ×；守卫 `contract_tests::tab_home_marker_contract` + 探针 P1h/P7h。见 `docs/changes/2026-10-13.md`）；`target="_blank"` / `window.open()` 直接新开 tab（标签栏无「+」按钮，有意移除）。新 tab 落点：与入口同源（含上游真实主机）的链接映射到入口地址的对应路径，跨域外链**直接打开目标地址**（与 Electron 壳一致，v1.52）。真实网页标题由远程页经核心事件 `web2app:tab-title`（建窗时注入一次性 token 防伪）回传——远程 origin 无法调用应用命令；标题回传与 token 注入**只在主框架生效**（v1.55：WebView2 初始化脚本会注入所有 iframe，子框架（如 bilibili 页内广告 frame，其标题为 "Leader Iframe" 等）曾顶掉标签栏真实标题；契约测试 `tab_title_script_runs_in_top_frame_only` 钉住）；首屏失败有错误态与「重试」事件（`web2app:retry-bootstrap`）。**顶部标签栏与单窗口标题栏为同一套配色/按钮样式**（v1.54：`shell.html` 复用 `titlebar.js` 的浅/深底色、下沿分隔线、按钮盒模型与 hover 色；静态契约测试 `tab_bar_palette_matches_titlebar` 钉住），且 `shell.html` 是双壳共用的单一真源，Tauri / Electron 表现一致 | Windows |
| 单实例 | 避免多开：第二个实例启动会把已存在窗口带到前台，而非再开一个 | Windows |
| 开发者工具 | WebView2 下按 **F12** 打开 DevTools 排查问题（debug 开 / release 关） | Windows |
| 优雅关闭 | 关闭行为由设置决定（两个**驻留理由**并列，任一成立即隐藏、不退出）：①启用托盘且开启“最小化到托盘”→ 隐藏到托盘；②启用桌面悬浮图标（v1.75）→ 隐藏到球（点球即唤回）。两者都不成立则退出应用。⚠️ 理由②不可省：少了它，只启用悬浮球时关窗走 `else` 分支 ⇒ 主窗被**真销毁**、球窗却还活着 ⇒ ①球窗挡住「所有窗口关闭 ⇒ 退出」，进程留成既无界面也退不掉的僵尸；②点球时 `toggle_window` 的 `get_webview_window("main")` 返回 `None`，一个动作都不做 ⇒「点了悬浮图标不回窗口」（2026-10-11 真机实测） | Windows |
| Cookie 会话保持 | 自动把上游 `Set-Cookie` 的 `Domain=真实host` 重写为 `127.0.0.1`，避免 localhost 下丢 cookie | Windows / Android |
| 自绘标题栏 | 无系统装饰：前进 / 后退 / **设置** / 最小化 / 最大化 / 关闭内嵌于 38px 标题栏，随页面深浅色切换（「设置」紧挨最小化**左侧**）。**前进/后退带落点守卫（v1.66）**：经 Chromium Navigation API 判定相邻条目，没有真实历史或落点是内置 splash 启动页时按钮置灰、不触发导航。**顶部通栏为 Windows 专属**：Android 端（有系统标题栏）不注入、启动页也不内联，避免叠成双通栏 | Windows |
| 移动端底栏 | 编译期开关 `MOBILE_BAR=true` 开启（默认关，**仅移动端**）：屏幕底部常驻一条导航条 —— **首页**（回到 `app_url`，已在首页则重新加载；未就绪时重走一次 bootstrap）/ **后退** / **前进** / **列表**。后退前进的**落点守卫与桌面端标题栏同一条规则**（没有真实上/下一条、或落点是内置 splash 时不动作并置灰），与 Android 系统返回键的判定同源。**「列表」按钮只在当前构建真的有页签可切时才渲染**（移动端目前无多页签 ⇒ 只画前三个按钮，绝不出现点了没反应的死按钮；判定单一真源 `mobilebar.rs::tabs_available`，将来移动端有了页签只需放宽它的 cfg，前端一行不用改）。底栏按**底部空间契约**把 `body` 缩高一格底栏并让出 `env(safe-area-inset-bottom)`，同时把页面自己的贴底固定条上移，否则页面页脚会被底栏盖住。桌面端（Windows Tauri / Electron）忽略该开关：常量整体 `#[cfg(mobile)]`，桌面目标无此代码路径 | Android |
| Android 返回键 | **系统返回键不允许回到内置 splash 启动页（v1.66）**：CI 在 `tauri android init` 后经 `.github/scripts/patch-android-back.py` 给 MainActivity 注入回调，用 `WebView.copyBackForwardList()` 判定上一条历史 —— 是真实网页才 `goBack()`，落点内置或无历史时走 `finish()` 回桌面 | Android |
| 启动过渡 | 解析真实地址期间显示启动页 + 加载层，避免白屏；目标页加载失败时加载层转失败卡（含「重试」）。**Android 导航后另发一次 Rust 侧可达性探测**（v1.60），把 DNS/连接/超时/TLS 的具体原因提前显示在失败卡上；**v1.61 起失败卡自带「现场诊断」**（当前地址 / readyState / HTTP 状态 / 资源数 / WebView UA / 页面脚本错误），截图即可区分「没离开启动页」与「外链没渲染」，无需 adb；**v1.62 修正启动页的加载层语义与重试落点**：①模式标记改由建窗时的初始化脚本在 `OVERLAY_SCRIPT` **之前**注入（`lib.rs::build_main_window`，此前只有 `tabs.rs` 有）——启动页不再被当成目标页跑，正常偏慢的启动不会被 overlay 的 15s 空白硬上限误报成失败；②启动页判定加改写后的 hostname（`splash.localhost`），重试改用**相对路径** `/retry`（此前 `location.protocol === 'splash:'` 在 wry 把协议改写成 `http://splash.localhost/` 后恒不成立，重试退化为只刷新 `/` 的死循环）；**v1.64 修启动期「发得太早」**：Android 上壳侧操作走 wry main-pipe，而 `LoadUrl` / `Eval` 在 webview 未就绪时**静默丢弃**，且初始 splash 导航是 `post{}` 异步、后续 `navigate` 是同步 ⇒ 直连配置下导航先发、随后**被那条 post 出去的 splash 加载覆盖**，同期进度 `eval` 又落在 document-start 之前被短路（真机表象：地址仍是 `splash.localhost` **且**零进度行，同一机制）；改为非 Windows 启动期**先等首个文档加载完成**（`bootstrap::BootReady`，就绪信号来自建窗时注册的 `on_page_load`）再解析 / 推送 / 导航；**v1.65 再加「落点校验 + 有限重发」**：`navigate()` 恒返回 `Ok(())`、返回值不能证明导航生效，故导航后隔 500ms 读一次真实地址（`is_splash_url` 同时认 `splash://` 与改写后的 `http://splash.localhost/`），仍停在启动页就重发（最多 2 次）—— v1.64 是预防、v1.65 是检验与自愈 | Windows / Android |
| 下载接管 | 网页发起的下载走 Windows 原生「另存为」对话框，可取消 | Windows |
| 登录密码保存 | 网页登录表单提交后弹出「保存密码？」气泡，保存后下次访问自动填充（v1.59 起显式开启 WebView2 `IsPasswordAutosaveEnabled`——该开关默认 **false**，与 Electron 壳的 Chromium 默认行为对齐；老运行时不支持 `ICoreWebView2Settings4` 时静默跳过，不影响其它钩子）；已保存密码随设置页「清除全部浏览数据」一并清除 | Windows |
| 窗口几何记忆 | 记住上次窗口位置 / 尺寸（物理像素），几何失效时自动最大化 | Windows |
| 桌面悬浮图标（悬浮球） | 设置开关 `float_enabled`（v1.67 引入、v1.69 纠正形态、v1.70 修复交互、v1.71 真机闭环）：一个 56×56 **透明无边框置顶小球**（不占任务栏），加载内置本地页 `floaticon://localhost/`（`src/floaticon.html`，**不加载任何网页**；球内图标即软件自身 `icons/icon.png`，经同协议 `/icon.png` 编译期内嵌）；**单击球 = 切换原来的主窗口显隐**（v1.71 起 `tray::toggle_window`：可见非最小化→隐藏并同步 tab 可见性，否则显示 / 取消最小化 / 聚焦；此前只「显示」）；球可拖动（v1.70 起页面上报**屏幕绝对坐标**、pointerdown 发 `down` 存按下点，8 个事件；拖动中软钳制至少留 16 逻辑像素、松手自由态钳回显示器）、**右键 = 关闭悬浮图标功能并唤回主窗口**（v1.76：`float_enabled` 置 false 并落盘 + 销毁球窗 + 显示主窗 —— 只把球藏起来会留下「进程还在跑、桌面上却再也唤不出窗口」的死局）；拖到屏幕左 / 右缘自动**半隐藏吸附**（只露半球 28 逻辑像素；v1.70 起悬停展开 / 离开延迟 300ms 缩回 + 真实光标位置校验，防抖）；v1.71 起曾用 `SetWindowRgn`+`CreateEllipticRgn` 把可视 / 命中裁成 56 逻辑「活柱」内圆形（WebView2 强制宿主最小宽约 131 逻辑，区域外系统级穿透）—— ⚠️ **v1.80 已把这套区域 API 全部删除**：该 1bit 区域会切过 CSS 圆边的抗锯齿、且区域圆与球之间会留出透明环带（真机「背景异形」的两个源头），改为**真逐像素 alpha + `WM_NCHITTEST` 子类化**（见本行末尾 v1.78–v1.80）；几何与吸附边持久化在 `settings.json` 的 `float_window` 段（展开态物理像素）；托盘菜单「悬浮图标」可显隐切换。⚠️ 建窗必须显式 `.data_directory(config::data_dir(app))`（v1.71 真机根因：省略时 wry 回落默认 UDF，受控终端不可写致球窗静默建不出，有契约守卫）；**v1.72**：单击加 **250ms 双击去抖**（`floaticon.html` 的 `CLICK_DEBOUNCE_MS`，窗口期内的第二次点击丢弃 —— 系统双击会在 ~100ms 内送来两次 `pointerup`，旧实现两次都发 `click` ⇒ toggle 相互抵消、主窗「闪现」），Tauri 侧交互链路另有真机探针 `--float-probe`（27 项断言，见「测试」章）；**v1.73：球不挂原生提示**（原 `title` 改 `role="button"` + `aria-label`）—— 球窗 always-on-top，原生提示条（独立非置顶窗口）只会被画在球窗**之后**、从透明环带漏出一块浅色残片（真机现象「图标背景有异样图形」），守卫 `float_ball_has_no_native_tooltip`；**v1.77：球窗「真不进任务栏」** —— `.skip_taskbar(true)` **不够**（tao 的 `ON_TASKBAR` 按「窗口无父」置位 ⇒ 创建时写死 `WS_EX_APPWINDOW`，`DeleteTab` 又在窗口还没显示时调用 ⇒ 球实测是**带着任务栏按钮**被显示出来的，`ex=0x40118`），任务栏按钮的原生提示条 / 任务视图浮层被 always-on-top 球窗压到身后、只从透明环带漏出残片（真机现象「图标背景异样」，仅 Tauri 侧 —— Electron 的 `skipTaskbar` 是真移除）⇒ 修法：显示球窗**只能**经 `show_ball()`（`show → unminimize → apply_ball_window_styles → delete_taskbar_tab`，顺序硬要求 —— tao `apply_diff` 在显示路径用缓存标志**整体重写** `GWL_EXSTYLE`，写在 show 之前等于没改），置 `WS_EX_TOOLWINDOW|WS_EX_NOACTIVATE`、**按位清** `WS_EX_APPWINDOW`，再 `ITaskbarList::DeleteTab` 回收已建出的按钮，球窗 `.title("")`；读回实测 `0x40118` → `0x8000198`；**v1.77b：球必须铺满活柱**（球径 `46px → **56px**` = `ICON_SIZE_LOGICAL`）—— 窗口区域是「内接于活柱的圆」（现场只读取证 `GetRgnBox=(0,0)..(55,56)`、`PtInRegion` 网格两端递减），球比它小 ⇒ 中间那圈透明环带把窗口后面的东西透出来（Tauri/WebView2 非真透明 ⇒ 浅色残片；Electron 真透明 ⇒ 无感），这是「图标背景异样」的第三个源头（前两个：v1.73 球元素 `title`、v1.77 任务栏按钮）；连带 hover / grabbing 的 `scale()` 改 `filter: brightness()`、collapsed 的 `opacity` 改 `filter: brightness(.92)`、图标 `30 → 36px`，守卫 `float_page_ball_fills_the_live_pillar`，守卫 ⑥b / ⑥c；**v1.78–v1.80：给球窗换「透明地基」**（详见 `docs/changes/2026-10-12.md`）—— v1.78 把区域外扩 4px 仍漏浅色异形、v1.79 回退外扩并让页面自绘不透明黑底仍留锯齿：三案都困在「wry 0.55 + tao 0.35 的透明链路会把 border-radius 抗锯齿这类**半透明像素压平到白底**」这条旧链路里，几何与底色都救不了。**v1.80 弃掉绕路**：升级 **tauri 2.12.1 / wry 0.57 / tao 0.37**（`no_redirection_bitmap` 自 tauri-runtime-wry 2.12 起才透传），球窗 builder 同开 `.transparent(true).no_redirection_bitmap(true)` ⇒ `WS_EX_NOREDIRECTIONBITMAP` + DirectComposition 拿到**真逐像素 alpha**；**删除全部 `SetWindowRgn` / `CreateEllipticRgn`**，新增 `install_circle_hit_test`（`EnumChildWindows` 递归枚举父窗 + 全部 WebView2 子窗 → `SetWindowLongPtrW(GWLP_WNDPROC)` 幂等子类化，进程只拦 `WM_NCHITTEST`：圆内转 `HTCLIENT`、圆外回 `HTTRANSPARENT`，圆心/半径存静态原子量、未初始化 fail-open）⇒ **视觉与命中彻底分离**，与 Electron（`win.setShape(circleShapeRects(…))`）同构；页面恢复真透明、初始化脚本不再注入壳标记。⚠️ v1.80 的真机目视（浅/深桌面软边、四角穿透、拖动/悬停/单击/右键四类交互）**仍待复验** | Windows |
| 拖拽修复 | WebView2 拖拽 bug 的 polyfill（用鼠标事件合成完整 HTML5 DnD 序列） | Windows |

---

## 支持平台

- **Windows**：原生 WebView2 渲染，支持托盘、全局快捷键、F12 调试、外部代理。
- **Android**：复用同一份 Rust 代码，鉴权走纯 Rust 本地反向代理（零平台依赖）；无系统托盘/快捷键（移动端由系统任务管理替代）。`tauri.conf.json` 的 `bundle.android` 目前仅设 `minSdkVersion: 24`（实测当前依赖 `tauri-utils 2.10.1` 的 `AndroidConfig` **只认** `minSdkVersion` / `versionCode` / `autoIncrementVersionCode` / `debugApplicationIdSuffix` 四个键，且 `deny_unknown_fields` ⇒ 写 `package` / `targetSdkVersion` / `useKotlin` / `permissions` 一律在**桌面构建阶段也报错**；**应用包名只能取自 `identifier`**）；更细的 SDK 版本与权限在 `cargo tauri android init` 生成 `gen/android` 后于原生工程中调整。
  > ⚠️ 打包工作流会在 `android init` **之前**把 `identifier` 改写成 **`com.web2app.<由 app_name 清洗得到>`**，
  > 于是不同 `app_name` 的 APK 包名不同、可以在同一台手机共存（否则全都叫 `com.example.web2app` 会互相覆盖安装）。
  > 清洗规则与「上架要换自己域名前缀」的说明见 `docs/packaging-config.md` §4.2。

---

## 设置界面（Shift+F1）

在 Windows 下按 **Shift + F1** 随时打开设置窗口（该快捷键固定，不可在界面内修改）；
标题栏的**「设置」按钮**（位置在最小化左侧，单窗口与多页签两种模式都有）和托盘菜单的「设置」
打开的是同一个窗口。

窗口本身**没有系统标题栏**：整页就是一块面板，**铺满整个窗口**（无底衬 / 无圆角 / 无外边框 ——
旧版「居中面板 + 四周底衬」在 620 宽的窗里各留下 30px 空档，看着像「大块空边框」，已去掉），
标题 / 关闭 / 拖动都由 `dist/settings.html` 自绘在面板头部，**正文是唯一可滚动的区域**（内容多了只滚中间，
底部「保存」按钮始终可见）。窗口可缩放（最小 460×400）、打开时居中于屏幕。

> ⚠️ 无边框 ⇒ 拖动靠 `data-tauri-drag-region`，它需要 capability 里的
> `core:window:allow-start-dragging`（见 `capabilities/settings-window.json`）。
> 少了这条权限的表现是「窗口拖不动，且只有控制台里一条被拒绝的调用」，不会报错弹窗。
> ⚠️ 标题栏那个「设置」按钮走的是**事件**而不是命令：主窗加载的是远程页面，Tauri 2 对非本地
> origin 的 IPC 强制走 ACL，而应用命令没有 app 级清单可授权 ⇒ 只能发核心插件的
> `plugin:event|emit`（`capabilities/default.json` 里的 `core:event:allow-emit`），
> 由 `lib.rs` 的 `web2app:open-settings` 监听落地成开窗。多页签标签栏是本页面（`shell://`），
> 可以直接 `invoke("open_settings")`。
> ⚠️ v1.55：三条开窗路径（IPC 命令 / 事件 / Shift+F1 回调）都在主线程事件回调栈内，
> `settings::open_settings_window` 不能在调用栈里直接 `WebviewWindowBuilder::build()`
> （嵌套事件循环 → 新 WebView2 永不初始化 → 纯白窗）；函数内统一先切后台线程再
> `run_on_main_thread` 排队到下一事件循环轮次（与 NewWindowRequested 延迟建 tab 同模式）。

可配置项：

1. **唤醒快捷键**：显示 / 隐藏主窗口的全局快捷键。支持“点击捕获”（按下组合键即可，按 Esc 取消）或从下拉框选择常用键。**默认不启用**（未设置时不注册任何全局快捷键）。
   - 保存后立即生效；不能与「打开设置(Shift+F1)」冲突。
2. **是否启用托盘图标**：关闭后启动不创建托盘图标（关闭窗口即退出）。
3. **关闭窗口后最小化到托盘**：勾选且同时启用托盘时，点关闭按钮 = 最小化到托盘；**只启用悬浮图标（v1.75）时点关闭 = 隐藏（点球唤回）**；两者都没有才「关闭窗口即退出应用」。
4. **启用桌面悬浮图标（悬浮球）**（v1.67 引入，v1.69 纠正为悬浮球，v1.70 修复拖动 / 悬停 / 边界交互，v1.71 单击改 toggle + 活柱裁剪 + 数据目录根因修复；v1.80 起「活柱裁剪」改为真逐像素 alpha + `WM_NCHITTEST` 圆形命中）：勾选后立即创建小球，取消立即销毁；另有「立即显示悬浮图标」按钮（命令 `show_float_window`，跨平台注册、非 Windows 体内空操作）。**单击球切换原来的主窗口显隐**（可见则隐藏、否则显示聚焦）；球可拖动（屏幕绝对坐标协议，拖出屏会被钳回）、拖到屏幕左 / 右缘半隐藏吸附（悬停展开、移开延迟 300ms 缩回）、**右键 = 关闭悬浮图标功能 + 唤回主窗口**（v1.76：写 `float_enabled=false` 落盘 + 销毁球窗 + 显示主窗，且同步已打开设置页的复选框）；几何 / 吸附边写入 `settings.json` 的 `float_window` 段。详见功能特性表与 `src/floatwin.rs` / `src/floaticon.html`（协议注册见 `lib.rs`，能力见 `capabilities/float-icon.json`）。**v1.72**：单击带 **250ms 双击去抖**（`CLICK_DEBOUNCE_MS`，窗口期内第二次点击丢弃，防主窗「闪现」）；整条交互链路已由**真机合成指针 harness** `--float-probe` 取证（27 项断言，见「测试」章）。**v1.77**：球窗此前其实**带着任务栏按钮**（`.skip_taskbar(true)` 挡不住，实测 `GWL_EXSTYLE=0x40118` 含 `WS_EX_APPWINDOW`）⇒ 任务栏按钮的原生提示条浮层被置顶球窗压在身后、从透明环带漏出残片（真机「图标背景异样」）⇒ 显示球窗一律经 `show_ball()`（顺序 `show → unminimize → 补样式 → DeleteTab` 是硬要求，见 `src/floatwin.rs` 模块头），读回实测 `0x8000198`。 **v1.77b**：球径必须 = 活柱边长（`46px → 56px`）—— 窗口圆形区域与球之间那圈环带会透出窗口后面的东西（Tauri 侧成浅色残片）；悬停 / 拖动反馈从 `transform: scale()` 改 `filter: brightness()`，半隐藏从 `opacity` 改 `filter: brightness(.92)`（`opacity` 让整颗球半透明 = 自毁这层保护）；另补上 `ITaskbarList::HrInit()`（v1.77 漏了它 ⇒ `DeleteTab` 静默无效）。**v1.78–v1.80**：透明链路换地基（见功能特性表同项）—— v1.80 起球窗走**真逐像素 alpha**（`.no_redirection_bitmap(true)`），`SetWindowRgn` / `CreateEllipticRgn` **已全部删除**，圆形命中改由 `WM_NCHITTEST` 子类化（`install_circle_hit_test`）接管、视觉不再经过窗口区域；因此「球 == 活柱 == 命中圆」这条约束仍成立，但**不再是靠窗口区域裁出来的**，`Resized` / `ScaleFactorChanged` / `on_page_load` 触发的是 `update_hit_circle` 而非旧 `apply_live_region`。
5. **清除缓存与 Cookie**：一个按钮，**只有全清**（刻意不做分项选择）。一次清空本应用的全部本地数据
   —— 缓存、Cookie（含登录状态）、Local Storage / IndexedDB、Service Worker 等，随后重新加载内容页，
   需要重新登录。⚠️ 后端命令 `clear_web_data` **仅 Windows 桌面版有实现**，其它平台返回明确的
   「不支持」错误（宁可报错，也不做「显示已清除、其实什么都没清」的假成功）。
   实现见 `src/web_data.rs`（WebView2 原生 `ICoreWebView2Profile2::ClearBrowsingDataAll`）。
   ⚠️ 两处**已真机实测**的坑（取证见 `examples/probe_clear_data.rs`）：
   ①WebView2 的**完成回调不可靠** —— 探针 10+ 次运行里**一次都没执行过**，而数据**照样被清掉**
   ⇒ 本命令**只在回调带回错误时报失败**，回调不来只如实说明、照常重载；
   ②清除刚结束时该 WebView 的**导航会被静默丢弃、甚至直接阻塞** ⇒ 重载前刻意留 1.2s 落定
   （`RELOAD_SETTLE`）。⚠️ 「清完重载一定生效」这条**本机未取证**，待真机目视。

6. **退出程序**（v1.75）：面板里的一个按钮，二次点击确认后彻底结束应用（连托盘图标与悬浮图标一并收掉）。
   ⚠️ 它不是「锦上添花的第二个关闭按钮」：启用了托盘或悬浮图标后「关闭窗口」只等于隐藏，而**没有托盘时
   托盘的「退出」菜单项根本不存在** ⇒ 常驻后台的用户唯一的出口就是这里（Shift+F1 随时能打开本面板）。
   后端命令 `quit_app` = 托盘「退出」同一条 `app.exit(0)` 路径。

> 设置保存在**数据目录**下（`<data_dir>\settings.json` —— `data_dir` 默认 `"data"` ⇒ `<exe 同级>\data\settings.json`，与 cookie / 缓存 / 日志同一个目录；`app_name` 默认 `Web2App`），为 JSON 文件，下次启动自动读取。
> ⚠️ 2026-09-23 前它固定在 `%APPDATA%\<app_name>\`；那个旧位置现在只作**读取兜底**（升级兼容，不迁移不删除）。
> ⚠️ 只有上面 1–4 项落在 `settings.json` 里；**第 5 项不落盘** —— 它是一次性动作，不是配置项。
> ⚠️ **窗口几何（上表「窗口几何记忆」）也在这同一个 `settings.json` 的 `window` 段里**（2026-09-23 起；
> 此前单独占一个 `window-state.json`，该文件已不再读写，可手工删除）；**悬浮球几何在 `float_window` 段**
> （v1.67 起：`{x,y,width,height,edge}`，物理像素，`edge` 为 `none`/`left`/`right`，无几何时整段不出现）。
> 三个写入方（设置页保存 / 关窗记主窗几何 / 悬浮球拖动与退出）一律走 `src/store.rs` 的**读-改-写**，互不覆盖。
> 写盘是**原子的**（同目录临时文件 → `sync_all` → rename，与 Electron 侧 `writeDocument` 同策略）：
> 写一半被杀 / 断电不会留下截断的 `settings.json` —— 否则下次启动整份解析失败，
> 四个设置项与主窗 / 悬浮球几何会一起被回默认值，**且不报错**。

界面后续可在 `dist/settings.html` 上继续扩展更多能力（Rust 命令通道 `get_settings` / `save_settings` /
`close_settings_window` / `clear_web_data` / `show_float_window` / `quit_app` 即可复用）。
`quit_app`（v1.75，面板「退出程序」按钮）= 托盘菜单「退出」同一条路径 `app.exit(0)`
（→ `RunEvent::ExitRequested`，**不经过窗口 CloseRequested**，因此上面那个「驻留」分支
不会把退出吞掉）；它是**常驻后台时唯一的退出入口** —— 没启用托盘的用户原本根本退不掉程序。
⚠️ 该 HTML 是**双壳共用的单一真源**：Electron 壳靠 `settings-preload.ts` 的命令白名单 +
`settings-window.ts` 的 IPC 名对接，新增命令时**「页面字面量 / 白名单 / IPC 名」三处必须逐字一致**，
否则表现是「按钮点了没反应」——探针与冒烟各有一条断言守着这个契约。

---

## 工作原理（为什么需要本地反向代理）

现代 Chromium / WebView2 **已废弃**「URL 内嵌 userinfo 自动发送 Basic 鉴权」的行为，且 WebView2 的
`BasicAuthenticationRequested` 事件对 **WebSocket 握手不生效**（这是微软官方确认的限制）。

Web2App 的解法是内置一个**本地反向代理**（`src/proxy.rs`）：

1. 程序启动时监听 `127.0.0.1:随机端口`；
2. WebView 只访问这个本地代理地址（入口 URL 保留上游路径并自带一次性随机密钥 `?w2a_key=...`）；
3. 代理对**每一个 HTTP 请求和 WebSocket 握手**统一注入 `Authorization: Basic ...` 头，再转发到真实服务器；
4. 代理对 `Host` / `X-Forwarded-*` 头做透传，让上游收到的请求与「浏览器直连」时一致；
5. **入口门禁**：每个请求（含 WS 握手）必须在 query 或 `w2a_key` HttpOnly cookie 带上本次启动的
   256-bit 随机密钥，否则一律回 **404**（不透露代理存在，防同机进程扫描借道带凭据访问目标站）。
   首次带 query 访问时代理回种 cookie，页面里的根绝对路径资源与 WS 即靠 cookie 通过；
   转发上游前密钥会从 query 与 Cookie 中剥除，密钥不离开本机。

该代理是纯 Rust、零平台依赖，因此 **Windows 与 Android 共用同一份代码**。

> 当 `PROXY_ENABLED = false` 或「启用了外部 HTTP 代理」时，WebView 直连上游，由 WebView2 原生事件兜底鉴权。
> **凭据主机白名单（2026-09-27）**：直连模式下，Authorization 注入与 401 自动应答都只对
> **入口上游源**（scheme + host + 默认端口归一化，`bootstrap::is_upstream_request`）生效；
> 页面里的第三方子资源 / 外站 401 质询拿不到入口账号密码（与 Electron 壳 `login` 事件的
> host 校验是同一条规则）。URI 不可解析或源未知时失败关闭（不注入）。

---

## 目录结构

```
webview-app/
├── Cargo.toml              # workspace 成员 + 依赖与 TLS 后端（Windows=native-tls / 其他=rustls）
├── tauri.conf.json         # Tauri 配置（含 Android bundle 配置）
├── build.rs                # Tauri 构建脚本
├── config.example.json     # 配置模板（含全部字段，供手动打包工作流使用）
├── src/
│   ├── main.rs             # 桌面入口（薄壳，装配在 lib.rs）
│   ├── lib.rs              # Tauri 应用装配：建窗 / 托盘 / 快捷键 / 命令注册（按平台 cfg 隔离）
│   ├── contract_tests.rs   # 静态契约测试（IPC 命令注册表 / capabilities 与事件名字面量 / 顶栏配色 / 加载层模式标记 / 启动日志归属；仅 cargo test 编译）
│   ├── config.rs           # 编译期 env 注入常量 + 目录名/文件名清洗
│   ├── bootstrap.rs        # 后台解析真实地址 → 就绪后导航
│   ├── proxy.rs            # 本地反向代理（HTTP + WebSocket 鉴权注入）+ 单元测试
│   ├── resolve.rs          # DNS TXT 查询 + 验签解密
│   ├── loading.rs          # 启动过渡（splash + 加载层）状态机
│   ├── debug_log.rs        # 诊断日志（dlog! 宏 / 落盘 / 1 MiB 轮转 / mask_url 去凭据 / log_startup 启动信息格式化）
│   ├── store.rs            # 运行期唯一那份 JSON 的契约（设置 + window 段几何，读-改-写；落点 = 数据目录）
│   ├── settings.rs         # 设置结构、快捷键转换、设置窗口/命令（持久化委托 store）
│   ├── tabs.rs             # 多页签（宿主窗口 + shell 标签栏 + tab 窗口）
│   ├── tray.rs             # 系统托盘
│   ├── window_state.rs     # 窗口几何：抓取 / 恢复策略（落盘在 store 的 window 段）
│   ├── webview_hooks.rs    # WebView2 原生事件（鉴权 / 新窗口 / 右键 / 下载）
│   ├── web_data.rs         # 清除浏览数据（缓存 / Cookie 全清，WebView2 ICoreWebView2Profile2）
│   ├── save_dialog.rs      # 下载「另存为」原生对话框（IFileSaveDialog）
│   ├── floatwin.rs         # 桌面悬浮球窗口（建窗 / 显示入口 show_ball / 拖动吸附 / 圆形命中）
│   ├── floatprobe.rs       # 悬浮球真机合成指针 harness（--float-probe，仅 Windows）
│   ├── splash.html         # 启动页（内联 titlebar.js，解决首文档无初始化脚本）
│   ├── shell.html          # 多页签外壳（标签栏）
│   ├── floaticon.html      # 悬浮球页面（双壳单一真源，透明 + 内置图标）
│   ├── titlebar.js         # 自绘标题栏注入脚本（Windows）
│   ├── mobilebar.js        # 移动端底部导航栏注入脚本（仅移动端；mobile_bar=true 时注入）
│   ├── loading.js          # 加载层注入脚本
│   ├── tab_title.js        # 多页签真实标题回传
│   └── dnd_polyfill.js     # WebView2 拖拽 polyfill
├── crates/
│   ├── resolve-core/       # 纯加密核心（Ed25519 + AES-256-GCM + HKDF），零平台依赖
│   ├── resolve-tool/       # 服务端 CLI：生成密钥对 / 生成 DNS TXT 密文 / 解密验证
│   └── proxy-server/       # Electron 侧 sidecar（bin 名 w2a-proxy），#[path] 复用 src/proxy.rs
├── dist/
│   ├── index.html          # 多页签 tab 窗口的首文档（body 必须空白：loading.js 靠它判定加载中；真实内容由 bootstrap 导航到远程地址）
│   └── settings.html       # 设置界面 UI 源码（编译期被 include_str! 内嵌，经 settings:// 自定义协议加载）
├── tools/                  # 服务端辅助脚本（encrypt_url.py / loading_test.js）
├── examples/               # 仅测试目标的验证示例（probe_dom_ready.rs / probe_new_window.rs / probe_clear_data.rs）
├── capabilities/           # Tauri ACL 能力声明（default.json=主窗；settings-window.json=无边框设置窗；tabs-remote.json=多页签 tab-* 远程页，仅授事件发送；float-icon.json=悬浮球窗，仅授 event emit）
├── icons/                  # 应用图标（icon.ico / icon.png 等）
├── txttest_keys/           # 验签测试密钥对（⚠️ **当前工作区已不存在**：私钥被 .gitignore 忽略、目录整体已移除，见 docs/txttest-verify-materials.md 的换钥说明）
└── （CI 与打包工作流已上移至仓库根 `.github/workflows/`，见下文「自动构建」）
```

---

## 编译时配置（改这里，然后重新编译）

所有运行期参数都是 **`src/config.rs` 的编译期常量**，由 `option_env!` 在编译时注入（不读运行期配置文件）。
手动打包工作流（`build-custom.yml`）就是通过环境变量把这些值传进来的：

```rust
// 默认打开的网址，支持带账号密码；写成域名数组时走 DNS TXT 验签解密（见下文「动态解析」）
const APP_URL: &str = "https://用户名:密码@www.example.com";

const APP_NAME: &str = "Web2App";     // 安装名 / 数据目录名（Android 侧为应用 ID 派生）
const TITLE: &str = APP_NAME;         // 显示名 / 窗口标题，缺省回落 APP_NAME（旧名 WINDOW_TITLE 已移除）

// 额外参数：编译期内联成 window.web2appExtra = {...}，页面 JS 可直接读；非法 JSON 自动回落 {}
const EXTRA: &str = "{}";

// 多页签（仅 Windows）：默认关闭
const TABS_ENABLED: bool = false;

// 诊断日志（全平台）：**默认关闭**。true = 写 <数据目录>\logs\startup.log 并同时打印；
// false = 不建目录、不落文件，控制台输出也一并静默。运行期逃生口：`--debug` / WEB2APP_DEBUG=1
const DEBUG: bool = false;

// 动态解析（app_url 为域名数组时用）：Ed25519 公钥，裸 base64 或 SPKI PEM
const PUBLIC_KEY: &str = "";

// F12 开发者工具（仅 Windows WebView2）。已按构建模式自动判定：debug 开、release 关，无需手动改
#[cfg(debug_assertions)]
const DEVTOOLS_ENABLED: bool = true;
#[cfg(not(debug_assertions))]
const DEVTOOLS_ENABLED: bool = false;

// WebView2 用户数据目录（"app_name" 派生的目录名）。
// ⚠️ 空串是**有含义的取值**：表示「用系统默认程序目录 %LOCALAPPDATA%\<app_name>，不跟着程序跑」，
//    因此这里必须用裸 option_env! 读取，**不能**用把空串当未设置的 env_str!。
// 默认 "data" = <exe 所在目录>\data\（跟着程序走）。也可填其它相对路径或绝对路径。
// 目录不可写会自动回退到系统默认目录（安装版装进 Program Files 时不至于打不开窗）。
const DATA_DIR: &str = "data";

// —— 代理 —— 不需要就保持 PROXY_ENABLED = false
const PROXY_ENABLED: bool = false;
const PROXY_HTTP: &str  = "";   // 留空 = 走自实现本地代理（覆盖 WebSocket）
const PROXY_HTTPS: &str = "";   // 留空 = 同上
const PROXY_BYPASS: &str = "localhost;127.0.0.1;<-loopback>";
```

另有 `RESOLVE_SECRET`（在 `resolve-core` 侧编译期注入，未设置时回落内置默认值；改它必须与服务端同步）。

⚠️ 全局快捷键**不走编译期常量**：快捷键现在在**设置页（Shift+F1）**中配置并落盘到 `settings.json`，
**默认不启用**。

改完直接 `cargo build`（见下文）。

### 诊断日志（`debug`，**默认关闭**）

⚠️ **Windows 上它是 GUI 子系统程序**（`src/main.rs` 的 `windows_subsystem = "windows"`）：双击运行时
没有控制台可挂，`eprintln!` 的输出**没有任何去处**。于是「双击没反应 / 打开的是 example.com /
标题不对」这类反馈在用户侧**零证据**，只能远程猜。

`debug = true` 时把这些诊断同时落一份盘（`src/debug_log.rs`）：

| 平台 | 位置 |
|---|---|
| Windows | `<数据目录>\logs\startup.log`（目录规则同 `DATA_DIR`，默认 `<exe 所在目录>\data\logs\`） |
| Android | `/data/data/<pkg>/logs/startup.log`（应用私有目录，见「常见问题」） |

**为什么不默认开**：每次启动都在用户的数据目录里留下交付物以外的文件，不是期望行为。代价要写清楚：
上面那条「用户侧第一现场」在默认配置下**不存在**。所以留了**运行时逃生口**（交付出去的包也能临时
取证，不必重新编译）：

```powershell
.\web2app.exe --debug                # 或
$env:WEB2APP_DEBUG = "1"; .\web2app.exe
```

> ⚠️ 逃生口的环境变量是 **`WEB2APP_DEBUG`**（运行期），**不是** `DEBUG`。后者是**打包期**注入用的
> 名字（沿用「字段名大写」规则，同 `TABS_ENABLED`）；`DEBUG` 在开发机上是常见变量名，把它当逃生口
> 会让本地 release 构建莫名其妙开始写日志。

契约（改这个模块时别破坏）：

* **文件名 / 轮转与 Electron 侧逐字相同**：`startup.log`、1 MiB 轮转、只留 `startup.prev.log` 一代
  —— 双壳同一份契约（`docs/packaging-config.md` §3.5 / §3.6）。
* **fail-open**：目录建不出 / 文件不可写 ⇒ 本次静默跳过，**绝不影响启动**（为了记日志让应用起不来
  是本末倒置）。
* **绝不记凭据**：地址一律经 `mask_url` 去掉 `user:pass@`；Secret 只报「已配置 / 未配置」。
* **零外部依赖**：只用 std（本仓库不引 `log` / `env_logger`），时间戳自己格式化成 ISO-8601 UTC。
* 关闭时 `dlog!` **直接短路**：连 `format!` 都不做 —— 代理是热路径（每条响应都会走一遍），
  不能为了「反正不输出」而白拼字符串。

取证：

```powershell
Get-Content .\data\logs\startup.log -Tail 40
```

---

## 本地编译与运行

### 环境要求

- **Rust** 工具链（stable，建议 ≥ 1.70）
- **Windows**
  - Windows 10/11 + WebView2 Runtime（Win11 自带；Win10 一般已预装，否则去微软官网安装）
  - Visual Studio 生成工具（含「使用 C++ 的桌面开发」工作负载）
- **Android**
  - JDK 17
  - Android SDK + NDK。**平台、build-tools 与 NDK 版本都不要写死**：
    Tauri 模板随 tauri-cli 前移（Tauri 2.12 起 `compileSdk = 37`，且不再指定 `buildToolsVersion`），
    CI 由 `.github/scripts/install-android-sdk.sh` 读模板实际值、再从仓库里真实存在的包名中挑
    （注意 Android 16 起平台包带次版本号：`platforms;android-37.0` 而非 `android-37`，且 37.x 只在 canary 通道）；
    ⚠️ 模板要的平台在仓库里**不存在**时，该脚本会回落最高可用平台并改写 gradle（尽力而为，会打
    `::warning::`）—— 要让它硬失败，调用点设 `ANDROID_SDK_STRICT=1`。
    NDK 版本由 `.github/scripts/install-android-ndk.sh` **单一真源**（三个 workflow 共用，不再各写死一遍）。
  - 已执行过 `cargo tauri android init`（见下）

### Windows

```bash
# 开发运行
cargo run

# 发布构建（生成 target/release/ 下的 exe）
cargo build --release

# 打包成 msi / nsis 安装包（产物在 target/release/bundle/）
cargo install tauri-cli --version "^2"
cargo tauri build
```

### Android（一次性准备 + 构建）

```bash
# 1) 安装 Tauri CLI
cargo install tauri-cli --version "^2"

# 2) 初始化安卓工程（仅首次；生成 gen/android 与 Kotlin 胶水）
#    ⚠️ gen/ 已被 .gitignore 忽略，**不要提交**（CI 每次 `cargo tauri android init --ci` 现生成）
cargo tauri android init

# 3) 构建 APK（调试/直接安装）
#    ⚠️ tauri-cli 的 --target 只接受**架构名**（aarch64/armv7/i686/x86_64），
#       不是 rustup 的完整三元组 aarch64-linux-android —— 写成后者会直接报错。
cargo tauri android build --target aarch64 --apk
# 若需上架 Play Store 的 AAB，去掉 --apk 即可
```

> 本地首次构建前需跑一次 `cargo tauri android init`。该目录已被 `.gitignore` 忽略，**无需提交**；
> `release.yml` 的 Android 任务会在 CI 里用 `cargo tauri android init --ci` 现生成。
>
> 安卓工程（`gen/android`）的所有改写都由 `.github/scripts/` 下的**共享脚本**完成，两个打包
> workflow 只调用它们，不各自内联（避免漂移）：`prepare-android-conf.py`（init 前写
> version / productName / identifier）、`apply-android-labels.py`（应用名）、
> `patch-android-insets.py`（系统栏 insets）、`set-android-apk-name.py`（APK 文件名）、
> `patch-android-back.py`（返回键守卫）、`install-android-ndk.sh`（NDK 版本单一真源）。
> 工程根解析走 `android_gen.py`（CI 的 cwd 是 `webview-app/`，故首选 `gen/android`）。

---

## 测试

项目内置 Rust 单元测试，覆盖代理（Basic 头生成、hop-by-hop 头识别、Set-Cookie 重写、WS Origin 重写）、
WebView2 钩子与配置清洗、`resolve-core` / `resolve-tool` 的加密往返与篡改检测，
以及 2026-09-23 新增的两类**静态契约守卫**（都是「错了也静默」的那类缺陷）：
运行期文档层 `src/store.rs`（读-改-写 / 落点 = 数据目录 / 双壳字面量互通）、`src/lib.rs` 的 IPC 注册表与
capability / 跨文件事件名字面量（改动 `capabilities/*.json` 或注入脚本的事件名会被它拦下）。

```bash
cargo test --all                # 全量单元测试（2026-10-12 优化闭环后实测 241 通过 / 6 ignored：resolve-core 19 + resolve-tool 22 + w2a-proxy 32 + 主 crate 168；主 crate 合计 171 条，含 `contract_tests` 静态契约守卫）
cargo test --all -- --ignored   # 真 DNS 端到端测试（需联网）：TXT 查询 → 验签 → 解密全链路
```

### 悬浮球真机探针（`--float-probe`，仅 Windows）

`target/debug/web2app.exe --float-probe`（或 `cargo run -- --float-probe`）跑一次**真机合成指针**验证：
建窗形态（v1.80 起：**窗口无区域** + 真逐像素 alpha）、自由态拖动（1:1 跟手 / 软钳制 / 松手钳回）、左右缘吸附（含活柱贴边与几何跟随）、圆形命中（**`WM_NCHITTEST` 三形态**：建窗 / 右吸附缩回 / 左吸附缩回 —— 圆心与 WebView2 子窗都必须 `HTCLIENT`，四角与活柱外的撑宽区必须 `HTTRANSPARENT`）、
悬停展开 / 抖动吸收 / 300ms 延迟缩回、单击 toggle 主窗、右键 = **关闭悬浮图标功能并唤回主窗口**
（球窗**被销毁** + `float_enabled` 置 false + 主窗被唤回；复原后吸附态保留）
（断言项数随版本演进：v1.72 = 24、v1.76 = 27 **+ 6 项环境诊断**（诊断不计入失败）；**v1.80 把区域断言改为「窗口无区域」并新增 NCHITTEST 三形态校验（建窗 / 右吸附 / 左吸附），确切条数以真机 `ALL PASS` 行为准**——v1.80 改的是 `floatprobe.rs` 自身，须重新打包后在真机复验）。

- **驱动方式**：`floatwin::inject_page_event` 用**真实令牌**构造合法载荷，走真实 `token_matches` →
  `handle_event` → **真实窗口操作**；唯一跳过的是「球页 JS 把事件发出去」那一段
  （因此**页面侧 JS** 由 Electron 侧 harness「真发 `PointerEvent`、驱动同一份共享球页」与 Rust 静态契约守卫
  `float_page_click_is_debounced_against_double_click` 覆盖，见 `electron-app/AGENTS.md`）。
- **退出码**：`0` = 全过；`>0` = 失败断言条数；**`97` = 窗口环境没起来**（主线程往返 60s 内不回来，
  与环境有关、与代码无关）；**`98` = 看门狗总超时**（探针**绝不允许**挂死）。
- ⚠️ **本机（受限桌面会话）`WebviewWindowBuilder::build()` 约 50% 概率永久挂死**，同一 exe 同样代码时好时坏；
  该调用同步阻塞、进程内无法给自己设超时，且换 exe 目录 / 换 WebView2 UDF / 清残留 `msedgewebview2` 全都无效。
  **这是环境抖动，不是产品缺陷** ⇒ 用外部重试器跑。⚠️ 重试器是**一次性本机工作区产物**，跑完往往已被清掉、目录名随批次变化 ⇒ **本文不指向具体文件**（历史用过 `tests/run-float-probe-*.ps1`、`tests/float-*-rc/`，**现均已不在工作区**），按「输出停滞判卡 → 杀进程 → 换全新目录重来」的思路自建即可。⚠️ 注意根 `.gitignore` 对 `tests/` 是**逐条锚定忽略、不是整目录**（只忽略 `/tests/pack/`、`/tests/json/`、`/tests/pw-fixture/`、`/tests/pw-udd-*/`）⇒ 新建一次性目录前先确认它落在忽略规则内。⚠️ 选型教训：**判断结论要看结果文件里的 `ALL PASS` 行，不要只看退出码** —— 旧版 `.ps1` 重试器在本机**读不到退出码**（打印 `EXIT=` 空值 ⇒ 即便跑出 0 也不认，一路重试到假失败），新版改用 .NET `WaitForExit(ms)` 并以结果文件为准。
- ⚠️ 探针模式会**跳过单实例插件与全局快捷键** —— 否则本机已有实例在跑时，单实例插件让进程**直接退出**，
  表现为「零输出、退出码 0」的**假绿**。
- ⚠️ **探针内手写建窗必须与生产同构**：假主窗漏设 `data_directory` 时建窗会从 7~9ms 退化成 60s+ 甚至挂死
  （契约守卫 `float_probe_fake_main_window_sets_data_directory` 已钉住）。
- 真机实测事实（写进断言，勿再当成「失败」）：`monitor_from_point` 用 **`MONITOR_DEFAULTTONULL`**（屏外的点查不到显示器 ⇒ 软钳制整段被跳过，测试点必须落在显示器**内**）。
  > ⚠️ 历史取值：v1.77b–v1.79 用窗口椭圆区域裁圆时，`GetWindowRgnBox` 对椭圆区域**含端**（`[0,70)×[0,70)` 上报 `(0,0,69,69)`，宽高各 −1；100% / 125% 缩放一致）。**v1.80 起球窗不再设任何窗口区域**，该断言已反转为「`GetWindowRgnBox` 必须为空」，这条「含端」事实只在读旧版本证据时才有意义。
- ⚠️ 本机删除走安全垫片（递归删除一律被拦、有配额）⇒ 探针的临时运行目录**不要自己清**，换新目录重来即可；
  WebView2 的 UDF 落点是 `%LOCALAPPDATA%\com.example.web2app\EBWebView`，**与 exe 所在目录无关**
  （「换个全新目录跑 exe」不会换掉脏 UDF；要禁用就**重命名备份**）。

主 crate 的多数测试跑在 `#[cfg(not(windows))]` 路径上，因此**同时**在验证 Android 侧逻辑；
Windows 侧另有代理与钩子的专属测试，`ci.yml` 的 Windows 作业也会跑它们。

> ⚠️ `src/proxy.rs` 同时被主 crate 与 `crates/proxy-server`（sidecar `w2a-proxy.exe`）经 `#[path]` 复用，
> 因此**它的单元测试会在两个 crate 各跑一遍**（上表 w2a-proxy 的条数里含同一批代理用例）——新增代理
> 测试时两个计数会同时 +N，这是预期而非重复实现。

---

## 自动构建（GitHub Actions）

共 **7 条** workflow，统一存放在**仓库根 `.github/workflows/`**（2026-09-22 由本目录上移）：
其中 **5 条服务本工程**（命令经 `defaults.run.working-directory: webview-app` 落到本工程目录，
产物路径则显式带 `webview-app/` 前缀）——2 条**自动**（`ci.yml` / `release.yml`）+
3 条**手动**（`generate-signing-key.yml` / `resolve-keys.yml` / `build-custom.yml`，
在 Actions 页手动 Run workflow）；另外 2 条服务 **Electron 绿色版壳**（工作目录是 `electron-app/`，
用法见 `electron-app/AGENTS.md`）：自动的 `electron-ci.yml`（lint / 探针 e2e / 出包）与
手动的 `electron-build-custom.yml`（自定义打包）。

> 打包一个可安装、可覆盖升级的应用，推荐顺序：**先跑 `generate-signing-key.yml` 生成并保存签名 → 再跑 `build-custom.yml` 打包**。详见下文第 3、4 小节。

### 1. 手动触发 — `.github/workflows/ci.yml`

⚠️ 该 workflow **只有 `workflow_dispatch`**（在 Actions 页点「Run workflow」），**已不再随 push / PR 自动运行**：

- **Lint & Test (Ubuntu)**：安装 WebKit2GTK 等依赖 → `cargo fmt --all -- --check`（**阻断**）→
  `cargo clippy -D warnings` → `cargo test --all`（同时验证 Android 代码路径）。
- **Check (Windows)**：`cargo check --all-targets` 验证 WebView2 那套代码能编译，并跑 `clippy` / `test`。
- **Check (Android)**：用 `cargo ndk` 交叉编译 `aarch64-linux-android`，并跑 **Android 目标 clippy**
  （**阻断**，不产出 APK）。NDK 由 `.github/scripts/install-android-ndk.sh` 安装（版本单一真源）。
- **config-contract**：跑打包配置契约门禁 —— 解析器 `--selftest`、**六个安卓共享脚本各自的
  `--selftest`**、校验全仓库 workflow YAML 与 `.github/scripts/*.sh` 语法（`check-workflows.py`）、
  逐个校验两份 `config.example.json` 与 `scripts/pack-local.mjs`
  的 `--parity`（Node 复刻与 Python 解析器的 43 组用例必须逐条一致）。详见
  `docs/packaging-config.md` §5。

### 2. 发版即编译 — `.github/workflows/release.yml`

两种触发方式：**Publish 一个 Release**（跑 Windows + Android），或在 Actions 页手动 Run workflow
（手动触发只跑 Android）。

- **Build Windows installers**（仅 Release 事件）：`cargo tauri build` 产出 `msi` / `nsis`，自动上传到本次 Release 的资源附件。
- **Build Android APK**：在 CI 里现跑 `cargo tauri android init --ci` 生成 `gen/android`（该目录被
  `.gitignore` 忽略，无需提交），再构建 `aarch64` APK 并作为 workflow artifact 上传。
- **产物版本号**：`VERSION`（仓库变量；为空则回落本次 Release 的 tag，自动剥前导 `v`；再为空则
  保留 `tauri.conf.json` 的值）—— 与 `build-custom.yml` 的能力对齐（历史上 release 侧不写 version，
  发版产物版本恒为 `tauri.conf.json` 的 `0.1.0`）。

> Android 正式签名所需仓库 Secrets：`ANDROID_KEYSTORE_BASE64` / `ANDROID_KEY_ALIAS` /
> `ANDROID_KEYSTORE_PASSWORD` / `ANDROID_KEY_PASSWORD`。流水线里由 `build-custom.yml` / `release.yml`
> 的对应步骤读取（与手动打包用的是同一套名字）。

### 3. 手动打包 — `.github/workflows/build-custom.yml`

在 GitHub Actions 页手动 **Run workflow**，先勾选本次要打哪些产物（三个开关互不依赖），
再填一份配置 JSON（或填一个返回 JSON 的 URL）。无需改源码，就能为不同的网址/名字/图标
生成各自独立的安装包。

> 三个开关的默认值：`windows_portable=true`（打单文件版），`windows_installer=false`、`android=false`。

手动运行时有 **3 个独立复选框**，想打哪个就勾哪个，产物分别上传到各自的 artifact：

| 选项 | 含义 | 对应产物 |
| --- | --- | --- |
| `windows_installer` | 打 Windows **安装版(MSI)** | `windows-msi` → `<app_name>_<版本>_x64_zh-CN.msi` |
| `windows_portable` | 打 Windows **单文件版**（免安装 zip） | `<app_name>-portable` → `<app_name>-portable.zip`（内含单个 `<app_name>.exe`） |
| `android` | 打已签名的 Android APK | `android-apk` → 签名 APK |

全部勾选＝一次触发同时产出上面三样；只勾 Windows 安装版时跳过 zip、只勾单文件版时用
`--no-bundle` 只编 exe（不打 MSI，更快）；三个都不勾则工作流无 job 运行。

> 说明：这里的「单文件版」即 **Windows 免安装绿色版**——单个 `<app_name>.exe`，双击即用、
> 不写注册表，依赖系统自带 WebView2 运行时。该产物不再手工多压一层 zip，而是把
> `<app_name>.exe` 直接上传，由 GitHub 打成 **`<app_name>-portable.zip`**（文件名含应用名，
> 且只有一层压缩，不会出现 "zip 套 zip"）。

配置模板见 **`config.example.json`**（复制到本地改好再粘贴即可）。

两个名字字段的区分（很容易混）：

| 字段 | 含义 | 影响范围 |
| --- | --- | --- |
| `app_name` | **安装名**（给系统/安装器看） | Windows 安装包名、开始菜单与桌面快捷方式、安装目录、"程序和功能"显示名；同时也是两端**数据存储目录名** |
| `title` | **显示名**（给用户看） | **Windows 窗口标题**、**Android 应用名称**（桌面图标下的名字） |

`title` 省略时自动回落为 `app_name`。二者分开后即可实现「安装包叫英文名 `MyApp`、界面叫中文名 `我的应用`」。
⚠️ 旧字段 `window_title` **已移除**（写了会在打包时直接报错）——它和 `title` 曾是同一个概念的两个名字，
现已只保留 `title`；编译期变量名也从 `WINDOW_TITLE` 改为 `TITLE`，与字段名一致。
完整字段表与两壳行为对照见 `docs/packaging-config.md`。

`extra`（可选，**对象**）—— 随打包一起注入到被包裹网页里的自定义配置，前端 JS 可直接读取：

- 会被序列化成 `window.web2appExtra = <你的对象>;` 注入到页面，例如上面的示例会被前端读到：

  ```js
  // 在你的网页里（被本应用打开后）
  window.web2appExtra            // => { build: "example", deploy_env: "prod" }
  window.web2appExtra.deploy_env // => "prod"
  ```

- 支持嵌套对象、数组、数字、布尔与中文字符串，最终都以合法 JS 对象字面量注入；
  不填时前端读到 `window.web2appExtra = {}`（空对象）。
- 典型用途：同一个网页被多个 app 复用，用不同的 `extra` 区分部署环境 / 渠道 / 站点皮肤等。

> 粘贴到 `config_json` 输入框时必须是**单行 JSON**；用 `config_url` 指向 RAW 文件（如 GitHub raw 链接）则无此限制。

`tabs_enabled`（可选，**真布尔**，默认 `false`，不带引号）—— 多页签（tab）模式开关，**仅 Windows 生效**：

- 设为 `true` 后，应用不再是一个单一窗口，而是「一个宿主窗口 + 顶部标签栏」，每个 tab 是独立的 WebView、**共享同一份数据目录**（同账号，cookie / localStorage 天然互通）。
- tab 可切换、可关闭；新 tab 由网页里的 `target="_blank"` / `window.open()` **直接新开**（标签栏的「+」按钮已有意移除，见 `src/shell.html` 注释；程序化建窗仍可走 `create_tab` 命令）。
- **新 tab 落点**（v1.52）：链接与入口**同源**（含上游真实主机）时，取目标的 path/query/fragment 拼到入口地址（本地反向代理会把所有 path 转发到同一上游）；**跨域外链直接打开目标地址**，不再回落首页。首个 tab 就绪后导航信息缓存到 `TabManager`、后续新 tab 复用；首屏未就绪时由首个到达者启动 bootstrap（`bootstrap_started` 标志判定），不再空等 30s 超时。
- Android 端忽略此字段（移动端无多页签能力）：`mod tabs` 整体 `#[cfg(windows)]`，且编译期常量 `TABS_ENABLED` 以 `cfg!(windows) && …` 在非 Windows **恒为 false**（v1.53 起），打包链路即使误注入 `TABS_ENABLED=true` 也不会在 Android 上启用多页签。
- 该开关等价于编译期环境变量 `TABS_ENABLED`；`release.yml` 模式下对应仓库变量 `TABS_ENABLED`。打包解析器写入环境变量的是 Python 风格 `True` / `False`（首字母大写），Rust 侧 `env_bool!` 按 ASCII 大小写不敏感识别（v1.50 起；此前只认小写，`True` 会静默回落 `false` —— 配置写了多页签、产物仍是单窗口）。
- ⚠️ 字符串形式（`"true"`）会被打包解析器直接拒绝（布尔字段只认真 `bool`，见 `docs/packaging-config.md`）。

```json
{ "app_url": "https://u:p@host", "tabs_enabled": true }
```

#### 动态解析真实 URL（`app_url` 数组 + `public_key` 公钥解密）

当后端地址可能变动、又不想频繁重新打包时，可以把 `app_url` 配成**一组域名（JSON 数组）**，
并同时提供 `public_key`（Ed25519 公钥）。应用启动时会：

1. **依次**对数组里每个域名做 **DNS TXT 记录查询**；
2. 拿到 TXT 值（一段 base64 密文）后，用 `public_key` 做 **Ed25519 验签 + AES-GCM 解密**得到真实 URL；
3. 某个域名查询/解密/验签失败则**自动尝试下一个**；全部失败时报错 **「服务器连接失败」**。

```json
{
  "app_url": ["node1.example.com", "node2.example.com"],
  "public_key": "HkCpkH1tMpNIxWoWAkvlX8vm93BOc5PtIKrQxl8RUDo=",
  "app_name": "MyApp"
}
```

##### 安全模型

| 诉求 | 实现方式 | 安全边界 |
| --- | --- | --- |
| **防劫持（硬保证）** | 服务端 Ed25519 **私钥签名**，客户端**公钥验签** | 攻击者即使拿到公钥 + 完全控制 DNS TXT，没有私钥就无法伪造出能通过验签的密文，最多破坏记录导致解密失败（DoS），**无法劫持到恶意网址** |
| **隐藏真实网址（软保证）** | AES-256-GCM 认证加密包装 | 外部查 TXT 只能看到密文。但对称密钥由客户端内置 Secret 派生，**攻击者反编译客户端可还原**，故此层只防"公网批量爬取/随手 dig"，不防针对性反编译 |
| **每次加密不同** | Nonce(12B) 每次随机（同时充当 HKDF salt，派生密钥也每次不同） | 同一 URL 多次加密密文完全不同，防重放/防密文比对猜测 |
| **密文尽可能短** | 固定开销 97B（Nonce12 + 分隔/签发日/有效期 5 + 签名64 + tag16）+ URL 本体 | v4 唯一格式 base64 后通常 160~180 字符，**单条 DNS TXT 可放下**（单串上限 255 字节）；v1/v2/v3 旧格式不再兼容 |

要点：

- **数据格式（v4 唯一）**（服务端 `tools/encrypt_url.py` / `resolve-tool` 与客户端
  `resolve-core`（`crates/resolve-core`）一致）：
  `TXT = base64( Nonce(12B) ‖ AES-256-GCM(HKDF(Secret,Nonce), Payload ‖ Ed25519签名) )`，
  其中 `Payload = URL ‖ 0x00 ‖ 签发日d(2B) ‖ 有效期ttl_d(2B)`（分隔符后尾部固定 4B）。
  **解密端只认这一种结构**：v3（秒级 16B 尾）/ v2（带盐 25B）/ v1（无 ttl 17B）
  记录一律报「Payload 尾部结构异常」拒绝，线上旧 TXT 需用当前工具重新签发（详见
  `docs/txttest-verify-materials.md`）。
- **Secret：客户端编译期注入不变，服务端加密支持 `--secret` 逐次指定**：
  - 客户端侧，AES 密钥源头仍是 `resolve-core` 编译期的 `option_env!("RESOLVE_SECRET")`：
    构建前在 shell 设好自定义 Secret（cargo 自动追踪该 env，改值会重编 `resolve-core`），
    不设则用内置默认串 `web2app-fixed-secret-2026`。
  - 服务端侧，`resolve-tool encrypt/all` 与 Python 工具自 v1.46 起支持 `--secret <值>`
    逐次调用显式传入，取值优先级 **参数 > 环境变量 `RESOLVE_SECRET` > 内置默认**，同一份
    工具二进制不必为不同 Secret 分别编译；PowerShell/cmd 的 env 老用法继续有效（设了 env
    就省略 `--secret`）。两端 Secret 不一致时 HKDF 派生密钥不同、客户端必然解密失败。
- **签发日与有效期（ttl_days）由服务端生成时决定**：时间以 **UTC 天**为单位，
  `d` = 签发日距固定基准日 **2026-01-01** 的天数（u16，可覆盖至约 2205 年），
  `ttl_d` 是随密文下发的有效期天数：
  - `ttl_d > 0`：客户端校验 `今日 - d <= ttl_d`，超出即拒绝（最小粒度 1 天）；
  - `ttl_d = 0`：**永不失效**，客户端跳过日期校验（适合后端地址长期不变的场景）。
  这样是否设有效期、设多长，完全由生成密文的一方决定，客户端无需重新编译。
- **`public_key` 格式**：Ed25519 公钥的**裸 base64（32 字节）**，或 SPKI PEM（`BEGIN PUBLIC KEY`）。
  推荐直接用 `genkey` 生成的 `public_key.b64` 内容。
- **查询通道**：走系统默认 DNS（读 `/etc/resolv.conf` 或 Windows 系统配置）。
- **降级**：`app_url` 为普通字符串时行为与原来完全一致（忽略 `public_key`）。

生成密钥对 + 加密 URL，有两种等价方式（私钥格式与密文格式完全互通）：

1. **纯 Rust 工具 `resolve-tool`**（推荐，无需 Python 依赖；独立 crate，不编译 Tauri）：
   ```bash
   cargo run -p resolve-tool -- genkey --out keys   # 生成私钥(服务端持有) + 公钥(客户端)
   cargo run -p resolve-tool -- genkey --stdout     # 不落盘：私钥 PEM/公钥直接打印到控制台（等价于双击 scripts/gen-resolve-keys.bat）
   cargo run -p resolve-tool -- encrypt keys/private_key.pem "https://user:pass@host" --ttl-days 0
   cargo run -p resolve-tool -- encrypt keys/private_key.pem "https://user:pass@host" --ttl-days 7 --count 3
   # 客户端构建期注入了自定义 Secret 时，用 --secret 逐次显式传入（值必须与客户端一致）：
   cargo run -p resolve-tool -- encrypt keys/private_key.pem "https://user:pass@host" --ttl-days 0 --secret "与客户端一致的 Secret"
   # encrypt 第一个参数也接受内联 PEM 字符串（以 -----BEGIN 开头，可折叠成单行、勿带 # 注释行），配合 genkey --stdout 全程不落盘：
   cargo run -p resolve-tool -- encrypt "-----BEGIN PRIVATE KEY-----<PEM 本体拼成一行>-----END PRIVATE KEY-----" "https://user:pass@host" --ttl-days 0
   ```
   `--ttl-days 天`：默认 **0 = 永不失效**；正数 = 生成后 N 天内有效（最小粒度 1 天，上限 65535），到期客户端拒绝（防重放/防过期需显式开启）。
   `--count N`：每个 URL 生成 N 份密文（随机 Nonce，密文不同但解密结果相同），可把同一个
   地址分发到多个域名 / 多条 DNS TXT 记录。
   `--secret <值>`：显式指定本次加密的 Secret（优先级高于环境变量；空串报错），不传则回落
   编译期/环境变量 `RESOLVE_SECRET` → 内置默认。
   也可用 `all` 一键生成密钥对 + 密文：`cargo run -p resolve-tool -- all "https://..." --out keys --count 3`（同样支持 `--secret`）。
   **解密验证**一条 TXT 密文（AES-GCM 认证 + Ed25519 验签，通过后打印真实 URL、签发日期与 TTL 天数；公钥为文件路径或直接内联的裸 base64/SPKI PEM；密文省略时走 stdin）：
   ```bash
   cargo run -p resolve-tool -- decrypt "<公钥裸 base64>" "<TXT 密文>" --secret "与客户端一致的 Secret"
   echo "<TXT 密文>" | cargo run -p resolve-tool -- decrypt "<公钥裸 base64>"
   ```
   过期记录（ttl_days>0 且已超时）仍展示解密内容，但退出码为 1，与客户端「过期即拒绝」一致。

2. **Python 脚本 `tools/encrypt_url.py`**（需 `pip install cryptography`）：
   ```bash
   python tools/encrypt_url.py genkey keys          # 生成私钥 + 公钥
   python tools/encrypt_url.py keys/private_key.pem "https://user:pass@host" --ttl-days 0
   python tools/encrypt_url.py keys/private_key.pem "https://user:pass@host" --ttl-days 0 --secret "与客户端一致的 Secret"
   # 与 resolve-tool 相同，第一个参数也支持直接传内联 PEM 字符串（多行完整 PEM，勿带 # 注释行）；--secret 同名同义（参数 > 环境变量 > 默认）
   ```

把密文作为某个域名的 TXT 记录值，把 `keys/public_key.b64` 的内容作为配置里的 `public_key`。

> 也可以直接在 GitHub Actions 里跑 **`resolve-keys.yml`** 这个 workflow（手动触发），
> 它提供「生成密钥对」「生成 TXT 密文」「一键生成密钥对 + 密文」三个操作，全程在 CI 上完成，
> 产出打包成 artifact 下载。

#### Android 签名（打包时从三层来源取第一份可用的）

手动打包产出的 release APK **必须签名**才能安装。签名密钥沿三层策略**依次取第一份可用的**：

| 优先级 | 签名来源 | 适合场景 |
| --- | --- | --- |
| 1 | 配置 JSON 里的 `android_keystore_base64` 等 4 个字段 | 每个应用各持一把正式密钥，随配置一起分发 |
| 2 | 仓库 Secrets（`ANDROID_KEYSTORE_BASE64` / `ANDROID_KEY_ALIAS` / `ANDROID_KEYSTORE_PASSWORD` / `ANDROID_KEY_PASSWORD`） | 长期使用 / 准备上架，各次构建可互相覆盖升级 |
| 3 | 都没有 → CI 现场用 keytool 生成**一次性** keystore 签名 | 临时试装；但每次构建签名都不同，**新版无法覆盖安装旧版**（需先卸载） |

> ⚠️ **正式使用请务必用方式 1 或 2**，并长期保存 keystore 文件与密码。Android 应用升级必须用**同一把密钥**签名，密钥一旦丢失，已装应用将永远无法覆盖升级。

### 4. 生成签名密钥（推荐）— `.github/workflows/generate-signing-key.yml`

> 这是最省心的签名方式：**云端全自动生成一把固定 keystore，零输入、随机密码**，跑一次就把「密钥文件 + 密码 + 已签好名的配置 JSON」打包成一个 artifact 给你下载。

**为什么用它**：手动打包要"签名一致、可覆盖升级"，就需要一把**固定**的 keystore 反复用。
本 workflow 在云端用 JDK 的 `keytool` 生成这把密钥（随机强密码），并自动填进一份打包配置 JSON，你无需在本机装 JDK、也无需手敲 `keytool` / `base64` 命令。

**用法（3 步）**：

1. Actions → **「生成 Android 签名密钥 (keystore)」** → Run workflow（**无需填任何东西，全自动**）。
2. 下载 artifact **`android-signing-key`**，里面有 4 个文件：
   - `release.keystore` —— 密钥库文件本身，请**长期妥善保存**（丢了无法找回）；
   - `keystore-info.txt` —— alias、keystore 密码、base64（单行）+ 使用提醒；
   - `packaging-config.json` —— 已把签名 4 项填好的打包配置模板（美化版）；
   - `packaging-config.single.json` —— 同一配置的单行版，直接粘给 `build-custom.yml` 的 `config_json`。
3. 打开 `packaging-config.json`，把 `app_url` / `app_name` / `title` 改成你的真实应用（**签名与网址无关**，改了仍有效），
   然后把 single 版粘进 **build-custom.yml** 打包。

> ⚠️ **每次运行本 workflow 都会生成一把新密钥**。因此只运行一次、保存好这把；不要为了"再拿一把"而重复运行，
> 否则新旧密钥签的 APK 之间无法覆盖升级（要先卸载）。若希望以后打包**自动**用同一把（不用每次粘 base64），
> 把 `keystore-info.txt` 里的 base64 / alias / storepass 配成仓库 Secrets（`ANDROID_KEYSTORE_BASE64` /
> `ANDROID_KEY_ALIAS` / `ANDROID_KEYSTORE_PASSWORD`）即可，`build-custom.yml` 会自动读取。

### 5. 本机手敲 keytool 生成（备选）

不想用云端 workflow 时可在本机生成（需 JDK；装了 Android Studio 可直接用它自带的 `jbr\bin\keytool.exe`）：

**① 生成一把 keystore**：

```bash
keytool -genkeypair -v \
  -keystore release.keystore \
  -alias my-app-alias \
  -keyalg RSA -keysize 2048 -validity 10000 \
  -storepass YourStorePass -keypass YourKeyPass \
  -dname "CN=MyApp, OU=Dev, O=MyCompany, L=City, ST=State, C=CN"
```

字段对应关系：`-alias` → `android_key_alias`，`-storepass` → `android_keystore_password`，
`-keypass` → `android_key_password`（可省略，默认同 storepass）；`-validity 10000` 天 ≈ 27 年，够用。

**② 编码成单行 base64**（CI 只收 JSON 里的 base64，收不了文件）：
```bash
base64 -w0 release.keystore     # Git Bash / macOS / Linux
```
```powershell
[Convert]::ToBase64String([IO.File]::ReadAllBytes("C:\path\to\release.keystore"))
```

**③ 填入配置 JSON**：即 `config.example.json` 的字段，再加上 ② 的输出 → `android_keystore_base64`，
字段名严格一致（可直接参考 `generate-signing-key.yml` 产出的 `packaging-config.single.json`）。

> ⚠️ `android_keystore_base64` 是 keystore 的**明文等价物**（任何人拿到都能用它给同包名应用签名冒充你），
> 等于一把私钥。**不要**把它提交进公开仓库的 JSON / 文档 / 历史记录；正式长期使用更推荐放仓库
> **Secrets**（方式 2），配置 JSON 里只留方式 1 用于「每个应用各自一把密钥、随配置分发」的场景。
> 想验证 JSON 里的 keystore 是否有效，可用 `keytool -list -keystore <解码后的.jks> -storepass <密码>` 查看别名。

---

## 常见问题

- **刷新 / 子页面打不开（鉴权丢失）**：确认 `APP_URL` 带 `user:pass`，且 `PROXY_ENABLED` 与
  `PROXY_HTTP/HTTPS` 的搭配符合「本地反向代理」分支（即启用代理时不填外部代理地址）。
- **全局快捷键没反应**：先在设置页（Shift+F1）确认唤醒快捷键**已配置**（默认不启用）；若已配置，
  再排查是否与其他软件冲突（代码只在「按下」那一刻触发）。
- **Android 上访问 http 本地代理被拦**：移动端已配置 `INTERNET` 权限并允许 localhost 明文，正常可用。
- **Android 打开后转圈十几秒 → 白屏 / 失败卡**：**不能直接断定是设备网络不通**（v1.60 曾这样误判，被真机反馈纠正：手机浏览器访问同一网址完全正常）。v1.61 起失败卡自带「现场诊断」（当前地址 / readyState / HTTP 状态 / 子资源数 / WebView UA / 页面脚本错误），凭一张截图即可分层定位：
  1. 「地址」仍是 `http://splash.localhost/`：导航根本没离开启动页 → 壳内导航链问题（不是网络）；已是目标域名：外链文档已开始加载但没渲染出正文 → 网络 / TLS / WebView 内核层。
  2. 「HTTP」为 0 / 无记录且手机浏览器能打开同一网址：先看「UA」行的 WebView 版本——无 GMS 的国产 ROM 上 **Android System WebView 可能长期不更新**，过旧内核在新证书链（TLS 1.3 / 现代根证书）或 HTTP/2 上会失败，而手机自带浏览器多为独立更新的内核，二者不矛盾；可到「设置 → 应用 → Android System WebView」更新或启用后重试。
  3. 失败卡上「无法连接服务器 / 连接超时 / 安全连接失败」是 v1.60 Rust 侧 reqwest 探测的**独立**结论（与 WebView 不是同一网络栈），仅作参考；浏览器能打开时以诊断卡为准。
  4. 仍需逐字网络错误时再抓 `adb logcat | findstr /i "chromium net::"`（或 Android Studio Logcat 过滤 `chromium` / `net::`）；旧打包产物不含诊断卡，必须用当前代码**重新打包**。
- **v1.62 修正的两条「必然」路径（与网络无关，出问题时可据此排除）**：
  1. **启动页的加载层语义**：启动页必须走 splash 语义（SLOW 5s / FAIL 40s 的「启动超时 + 重试」、不装就绪监听）。模式标记 `window.__web2appLoadingMode` 由建窗时的初始化脚本在 `OVERLAY_SCRIPT` **之前**注入；漏掉它不会报错，只会让启动页装上 overlay 的空白文档硬上限（CAP 15s；启动页 body 只有 `<script>`，`bodyEmpty()` 恒真）⇒ 一次正常但偏慢的启动被判「页面长时间为空白」。静态守卫 `contract_tests::loading_mode_contract`。
  2. **启动页「重试」的落点**：wry 在 Windows/Android 上把 `splash://localhost/` 改写成 `http://splash.localhost/`，页内 `location.protocol` 是 `http:` ⇒ 判定必须看 hostname（`splash.localhost`），跳转必须用**相对路径** `/retry`（绝对 URL 的 scheme 内核不认）。判错时的表象是「点重试只刷新启动页、永远起不来」。契约测试 `loading::tests::retry_uses_relative_path_and_rewritten_hostname`。
- **v1.63 让启动期「自己把现场说清」（排查 Android 启动问题时先看这里）**：
  1. **壳层对主框架加载失败是「必然失明」的**：wry 在 Android 上把 `load_url` / `eval` 实现成「投递 main-pipe 消息后直接 `Ok(())`」⇒ `window.navigate()` / `wv.eval()` **恒返回 Ok、永不报错**（`bootstrap.rs::navigate_main` 里那条 `Err(e) => show_error` 在 Android 上是死代码），且 `RustWebViewClient.onReceivedError` 只对 `ERROR_CONNECT` 做协议重定向特例、**不向壳层透出任何网络错误**。后果：主框架加载失败时 WebView 把启动页留在屏幕上，壳侧一无所知，最后只剩 40s 兜底卡。根因在 wry 上游，本版改为**让卡片自己说清**。
  2. **兜底卡现在带现场**：40s 的「启动超时」卡补了 `diagText()`，并在仍停在 splash 时提示「目标地址的导航没有提交」——配合新增的进度行 `正在解析目标地址（N 个域名）…` / `正在打开 <host>…` / `正在启动本地反向代理…` / **`已发起导航，等待页面响应…`**，一眼即可区分「根本没导航」「导航了但没提交」「网络不通」「内核不渲染」四类。⚠️ 判读：卡上**没有**「已发起导航」⇒ `prepare_navigation` 还没走到导航（解析 / 代理阶段）；有这一行却仍停在 splash ⇒ 导航已发出但文档未提交（多半落在 WebView 内核层）。
  3. **可达性探测成功时也会上报**：卡上出现「可达性探测：<域名> 已连通（HTTP 200，xxxms），但页面未渲染」时，说明 Rust 侧（reqwest + rustls/webpki-roots）能通而 WebView 不能 ⇒ 问题在**内核**（版本 / TLS 协商 / 渲染），不在网络，优先升级系统「Android System WebView」或换机比对；反过来显示「无法连接服务器 / 连接超时 / 安全连接失败」时才是传输层问题。（此前探测成功时**什么都不推**，最需要的那条判据恰好被丢掉。）
  4. **启动期错误不再被静默吃掉**：加载层在挂载完成前是**缓冲队列**（挂载后回放），壳侧在启动早期推来的失败会照常显示，不必再靠「等一下看会不会卡」来判断。另：解析阶段有 25s 总预算（`RESOLVE_BUDGET_SECS`，超时给明确文案而不是一直挂着），解析失败文案保留具体原因（读系统 DNS 配置失败 / 查询某域名失败 / 全部密文解不开 / 预算超时）。
- **v1.63.1 让诊断「真的看得见」（v1.63 的补丁，同样是排查前必读）**：v1.63 把现场**拼进**了卡片，但「拼进去」≠「看得见」——这类形态本项目已经栽过三次，故补三处并各配静态守卫（`loading::tests::failed_card_must_be_scrollable_and_selectable` / `diag_address_is_redacted_before_display`）：
  1. **进度行前置**（实测生效）：`toFail` 里 notes 由「追加到末尾」改为「置于最前」。分诊靠的就是「已发起导航」这类行，地址 / UA 属第二步才看的技术细节；拼在末尾时它们会随长诊断一起沉到滚动区最底下。
  2. **失败态可滚动 + 可选中**：失败态改为 `justify-content:flex-start` + `overflow-y:auto`，并放开 `user-select`（根规则是 `none` ⇒ 用户选不中、复制不了，而卡片用途恰恰是「把信息反馈给打包者」）。⚠️ **实测 360×640 / 320×480 下内容并未溢出（`overflowPx=0`）**，故滚动是**防御性**修复（极端小屏 / 更多进度行才触发）；「可选中复制」是当场生效的改进。
  3. **地址脱敏**（实测生效）：诊断行的地址经 `redact()` 处理，把 `w2a_key` / `key` / `token` 等查询参数值换成 `***`。启用本地代理时入口形如 `http://127.0.0.1:<port>/auth.html?w2a_key=<门禁密钥>`，而这张卡的用途就是**截图外发**——不脱敏等于把门禁密钥一起发出去。
  取证脚本：`electron-app/scripts/diag/splash-fail-card.cjs` —— 按真实窗口尺寸渲染同一份 `splash.html` + `loading.js`（把 40s 兜底卡提前到 1.4s，其余逻辑逐字不动），客观测「进度行是否在最前 / 地址是否脱敏 / 能否滚动 / 关键行在首屏的 y」并出 PNG。
- **v1.64 启动期「发得太早」（真机取证定到的根因，改 Android 启动流程前必读）**：
  1. **根因不是「导航没发」，而是「导航被覆盖」**：Android 上壳侧对 WebView 的一切操作都走 wry 的 main-pipe（`wry/src/android/main_pipe.rs`），而 `WebViewMessage::LoadUrl` / `WebViewMessage::Eval` 的处理体都是 `if let Some(webview) = get_webview(activity_id) { … }` —— **取不到就静默丢弃**（那个 webview 只在 `CreateWebView` 处理完才写入）。更要命的是**初始 splash 导航是异步排队的**：`CreateWebView` 内走 `load_url(…, main_thread = true)` → Kotlin `loadUrlMainThread`（`RustWebView.kt` 里是 `post { loadUrl(url) }`，排到 UI 队列尾），而之后的 `navigate` 走**同步** `loadUrl`。直连配置下 `prepare_navigation` 只需毫秒级 ⇒ 导航先执行、随后被那条 post 出去的 splash 加载**整体覆盖**；同期那几条进度 `eval` 又落在 splash 文档 document-start **之前**（`window.__web2appLoading` 还没定义）被 `&&` 短路吃掉。⇒ 真机卡上「地址仍是 `splash.localhost`」+「零进度行」是**同一个机制**。
  2. **修复：非 Windows 启动期先等首个文档加载完成**，再做解析 / 推送 / 导航（`bootstrap::BootReady` + `wait_first_page_ready`，预算 `SPLASH_READY_BUDGET_MS = 5000`）。就绪信号来自建窗时注册的 `on_page_load`（wry Android 的 `ON_LOAD_HANDLER` ← `RustWebViewClient.onPageStarted / onPageFinished`，`tauri-runtime-wry` 无条件接线）—— 这是本壳在 Android 侧唯一可靠的「文档就绪」判据（轮询 URL 不行，`GetUrl` 同样走 main-pipe）。⚠️ `Started` 必须**清**标记，否则 `/retry` 重载启动页后会沿用旧标记、变成「首启修好、重试仍卡死」；⚠️ 超时**不阻断启动**（退化成旧行为），但会在卡片上留一行「启动页未在 5000ms 内报告加载完成（页面加载事件缺失）」—— 这一行本身即判据。Windows 完全不加（WebView2 时序由 wry 保证），故新增代码全部 `#[cfg(not(windows))]`。
  3. **判读下一版真机结果**：卡上出现进度行 ⇒ 修复生效（再按 v1.63 的判读表分层定因）；连进度行都没有、且出现上面那句「页面加载事件缺失」 ⇒ `on_page_load` 在真机没送达，问题回到 wry 上游。
  4. 静态守卫：`contract_tests::startup_visibility_contract::android_boot_waits_for_first_page_load` / `page_load_event_is_wired_on_main_window`（均做过反向对照）。⚠️ 本版新增代码全在 `#[cfg(not(windows))]` 下、**本机是盲区**，已用 cfg 互换强制编译验证（8 处翻成 `#[cfg(all())]` 后 `cargo check --all-targets` 退出码 0，随后逐字节还原）。
- **v1.65 导航「落点校验 + 有限重发」（v1.64 的检验与自愈，两者互补）**：
  1. **为什么还需要它**：v1.64 做的是「等首个文档就绪再导航」，那是**时序预防**；而 Android 上**没有任何返回值能证明导航生效** —— `navigate()` 恒返回 `Ok(())`（它只是往 main-pipe 投一条消息）。所以真机上仍可能出现「导航确实发起了、地址却停在 `splash.localhost`」。v1.65 改为**直接检验结果**：导航后隔 `NAV_COMMIT_CHECK_MS = 500` 读一次真实地址，仍停在启动页就**重发导航**（最多 `NAV_COMMIT_MAX_RETRY = 2` 次）。`loadUrl` 语义幂等、重发只以最后一次为准，无副作用；正常启动只是一次「读地址 + 返回」。
  2. ⚠️ **判据必须同时认两种形态**：`splash://`（`loading::splash_url()` 构造的）与 `http://splash.localhost/` —— wry 的 `custom_protocol_workaround` 会把前者改写掉，**WebView 里真实存在的地址是后者**，只认自定义协议会在真机漏判（v1.62 栽过同类坑）。`is_splash_url()` 两个都认。
  3. ⚠️ **必须跑在独立线程**：`navigate_main` 在主线程，而读地址走 main-pipe（webview 未就绪时最长阻塞 `MAIN_PIPE_TIMEOUT` = 10s），放主线程会冻 UI。
  4. **下一版真机怎么读这张卡**：出现「重发导航已生效（第 N 次）」⇒ 根因确实是导航被挤掉、重发救回来了；出现「重发导航 2 次后仍未离开启动页（导航未提交，与网络无关）」⇒ 与网络无关，要往 wry 上游走；两条都没有且页面正常 ⇒ 走的正常启动路径。
  5. 静态守卫 `android_navigation_is_verified_and_retried`（4 条断言：判据双形态 / 调用点 / 重发上限 / 独立线程）。⚠️ **反向对照抓出守卫自身的盲区**：第一版用裸 `contains("ensure_navigation_committed(app, label, &url);")`，把调用点改成**注释**（`// …`）后守卫**仍然通过** —— 注释里含同样文字，`contains` 被满足；改为 `has_code_line()`（排除 `//`、`*` 开头的行）后，同样的改坏方式**如期 FAILED**。这是本项目「守卫的针被注释满足」的**第四种形态**（前三：注释里写裸 hostname、说明文字含同一字面量、反向断言被自身注释干扰）。
- **Android 端应用的数据存在哪？**（下文中 `<pkg>` = 构建时的应用包名，即打包工作流在 `android init` 前
  由 `app_name` 清洗后写入 `tauri.conf.json` 的 `identifier` = **`com.web2app.<app_name 清洗值>`**
  （仓库里 `tauri.conf.json` 的静态值只是占位；本地不跑工作流直接 `cargo tauri android` 构建时才会用到它）；
  `<app_name>` = 打包配置里的 `app_name`，清洗掉非法字符后的值）
  - **设置文件**：`/data/data/<pkg>/<app_name>/settings.json`
  - **WebView 数据**（Cookie / localStorage / IndexedDB）：`/data/data/<pkg>/app_webview/`
  - **缓存**：`/data/data/<pkg>/cache/`
  - **日志**：`/data/data/<pkg>/logs/`

  注意 Tauri 移动端的 `app_config_dir()` **不会再拼 identifier**，它直接返回应用私有根目录
  `/data/data/<pkg>`（见 `tauri/src/path/android.rs` → `PathPlugin.kt` 的 `getConfigDir` =
  `activity.dataDir`），与桌面端「基础目录 + identifier」的行为不同——所以取值后**不能再取 parent**，
  否则会跳到 `/data/data`（不可写，且与其它包名冲突）。代码已按平台分别处理（见 `src/settings.rs`）。

  这些目录都是**应用私有目录**：未 root / 非 debug 包在文件管理器里看不到，**卸载应用即被系统清空**。
  排查时可用 `adb shell run-as <pkg> ls files/` 查看。
  （构建产物 APK 不在手机上，而在构建机的 `gen/android/app/build/outputs/apk/`，CI 签名后上传为 artifact。）
- **`cargo run` 报 `unknown field package` / `targetSdkVersion` / `permissions`（build script 阶段）**：
  `tauri-build` 对 `bundle.android` 只识别少量字段，本仓的 `tauri.conf.json` 因此**只写 `minSdkVersion`**。
  `package` / `targetSdkVersion` / `useKotlin` / `permissions` 写进去会在**桌面**构建时同样报错（`tauri-build`
  始终解析整份 `tauri.conf.json`）。处理方式是**保持精简**，其余在 `cargo tauri android init` 生成
  `gen/android` 后于原生工程里配置 —— **不要为了凑这些字段去升级依赖**。
  > ⚠️ 但这不等于「Tauri 版本不许动」：**v1.80 已把 tauri 升到 2.12.1**（wry 0.57 / tao 0.37），
  > 目的是让 `tauri-runtime-wry` 透传 `no_redirection_bitmap`（球窗真逐像素 alpha，见「测试」章）。
  > 升级本身不是禁区，代价是必须逐项复验 WebView2 钩子（`webview_hooks.rs` 的初始化脚本、新窗口、下载、
  > 右键）与多页签链路；本仓 v1.80 的升级经 `cargo check` 零涟漪 + 全量测试通过，尚未在真机复验。
- **换产品名 / 包名**：改 `Cargo.toml` 的 `package.name`（决定 exe / 进程名）、`tauri.conf.json` 的
  `productName`（决定窗口标题 / 托盘提示）即可。**Android 应用包名不再手改 `identifier`**：打包工作流会在
  `android init` 之前按打包配置里的 `app_name` 自动把 `identifier` 改写成 `com.web2app.<清洗值>`
  （清洗规则见 `docs/packaging-config.md` §4.2）。若直接跑 `cargo tauri android`（不经工作流），才会用到
  `tauri.conf.json` 里的静态 `identifier`——此时请自行确保它已是你要的包名（Tauri 2 的 `bundle.android`
  尚不支持 `package` 字段，包名只能取自 `identifier`）。
- **任务管理器里显示的是「WebView2 管理器 / WebView2 渲染器」而不是应用名？**：
  Web2App 的**主进程**已经以 `web2app.exe` 显示（由 `package.name` 决定），窗口标题/托盘提示也已经是
  `Web2App`。但 WebView2 内部会拉起「浏览器管理器 / 渲染器」**子进程**，它们的进程名被写死在微软 WebView2
  运行时二进制里，**没有按应用重命名的官方开关**（除非自行附带一份改名后的 WebView2 运行时副本，代价是更新/许可
  都要自己维护，不推荐）。所以这是 WebView2 运行时的固有限制，应用侧无法改成自身名称；若需更干净的任务管理器
  表现，只能后续考虑改用 Edge WebView2 的「固定版本运行时」并自托管（超出本应用范围）。
