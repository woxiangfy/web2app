# 设置界面与 settings.json（webview-app）

> 自 `AGENTS.md` 拆出。`dist/settings.html` 是**双壳共用的单一真源**（Electron 侧经 `settings-preload.ts`
> 命令白名单 + `settings-window.ts` IPC 名对接）。悬浮球设置项的实现全文见 [float-ball.md](float-ball.md)。

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
4. **启用桌面悬浮图标（悬浮球）**（v1.67 引入）：勾选后立即创建小球，取消立即销毁；另有「立即显示悬浮图标」按钮（命令 `show_float_window`）。**单击球切换原来的主窗口显隐**；球可拖动、左右缘半隐藏吸附（悬停展开、移开延迟 300ms 缩回）、**右键 = 关闭悬浮图标功能 + 唤回主窗口**（v1.76）；几何 / 吸附边写入 `settings.json` 的 `float_window` 段。实现见 `src/floatwin.rs` / `src/floaticon.html`（协议注册见 `lib.rs`，能力见 `capabilities/float-icon.json`）；v1.69–v1.80 逐轮演进与真机取证全文见 [float-ball.md](float-ball.md)。
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
> ⚠️ **窗口几何也在这同一个 `settings.json` 的 `window` 段里**（2026-09-23 起；
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
