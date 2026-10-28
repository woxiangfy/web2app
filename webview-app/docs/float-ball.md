# 桌面悬浮球（悬浮图标）专题

> 本文是 webview-app「桌面悬浮图标（悬浮球）」的唯一详细记录：功能行为、设置项、v1.67–v1.81 演进与真机取证。
> 概览见 [../AGENTS.md](../AGENTS.md)「功能特性」表对应行；⚠️ v1.81 起 Tauri 球是 **Win32 分层窗口**（无球页），
> 共享页 `src/floaticon.html` 转为 **Electron 专属**（例外登记见根 `AGENTS.md` §3.4.1），
> Electron 壳同源实现见 [../../electron-app/docs/float-ball.md](../../electron-app/docs/float-ball.md)。

## 功能行为与演进（v1.67–v1.81）

设置开关 `float_enabled`（v1.67 引入、v1.69 纠正形态、v1.70 修复交互、v1.71 真机闭环、**v1.81 渲染根治**）：一个 56×56 **无边框置顶小球**（不占任务栏）。**v1.81 起球窗是 Win32 分层窗口**（`WS_EX_LAYERED` + `UpdateLayeredWindow` 提交 32bpp 预乘 ARGB 位图，圆盘 = 位图 alpha，无页面 / 无 IPC / 无令牌；图标即软件自身 `icons/icon.png` 编译期内嵌、面积平均叠加）；此前（v1.67–v1.80）为 WebView2 球窗加载内置本地页 `floaticon://localhost/`（`src/floaticon.html`，**不加载任何网页**，经同协议 `/icon.png` 取图标，pointer 事件走 8 事件 + 一次性令牌）；**单击球 = 切换原来的主窗口显隐**（v1.71 起 `tray::toggle_window`：可见非最小化→隐藏并同步 tab 可见性，否则显示 / 取消最小化 / 聚焦；此前只「显示」）；球可拖动（屏幕绝对坐标 + 抓取偏移协议；拖动中软钳制至少留 16 逻辑像素、松手自由态钳回显示器）、**右键 = 关闭悬浮图标功能并唤回主窗口**（v1.76：`float_enabled` 置 false 并落盘 + 销毁球窗 + 显示主窗 —— 只把球藏起来会留下「进程还在跑、桌面上却再也唤不出窗口」的死局）；拖到屏幕**四边**（上 / 下 / 左 / 右，就近择一）自动**半隐藏吸附**（左 / 右沿 x、上 / 下沿 y 半隐藏，只露半球 28 逻辑像素；v1.70 起悬停展开 / 离开延迟 300ms 缩回 + 真实光标位置校验，防抖）；v1.71 起曾用 `SetWindowRgn`+`CreateEllipticRgn` 把可视 / 命中裁成 56 逻辑「活柱」内圆形（WebView2 强制宿主最小宽约 131 逻辑，区域外系统级穿透）—— ⚠️ **v1.80 已把这套区域 API 全部删除**：该 1bit 区域会切过 CSS 圆边的抗锯齿、且区域圆与球之间会留出透明环带（真机「背景异形」的两个源头），改为**真逐像素 alpha + `WM_NCHITTEST` 子类化**（见下节 v1.78–v1.80）；几何与吸附边持久化在 `settings.json` 的 `float_window` 段（展开态物理像素，`edge` 取 `none/left/right/top/bottom`）；托盘菜单「悬浮图标」可显隐切换。

### v1.72–v1.81 逐轮收口

- **v1.72**：单击加 **250ms 双击去抖**（`floaticon.html` 的 `CLICK_DEBOUNCE_MS`，窗口期内的第二次点击丢弃 —— 系统双击会在 ~100ms 内送来两次 `pointerup`，旧实现两次都发 `click` ⇒ toggle 相互抵消、主窗「闪现」），Tauri 侧交互链路另有真机探针 `--float-probe`（见下节）。
- **v1.73：球不挂原生提示**（原 `title` 改 `role="button"` + `aria-label`）—— 球窗 always-on-top，原生提示条（独立非置顶窗口）只会被画在球窗**之后**、从透明环带漏出一块浅色残片（真机现象「图标背景有异样图形」），守卫 `float_ball_has_no_native_tooltip`。
- **v1.77：球窗「真不进任务栏」** —— `.skip_taskbar(true)` **不够**（tao 的 `ON_TASKBAR` 按「窗口无父」置位 ⇒ 创建时写死 `WS_EX_APPWINDOW`，`DeleteTab` 又在窗口还没显示时调用 ⇒ 球实测是**带着任务栏按钮**被显示出来的，`ex=0x40118`），任务栏按钮的原生提示条 / 任务视图浮层被 always-on-top 球窗压到身后、只从透明环带漏出残片（真机现象「图标背景异样」，仅 Tauri 侧 —— Electron 的 `skipTaskbar` 是真移除）⇒ 修法：显示球窗**只能**经 `show_ball()`（`show → unminimize → apply_ball_window_styles → delete_taskbar_tab`，顺序硬要求 —— tao `apply_diff` 在显示路径用缓存标志**整体重写** `GWL_EXSTYLE`，写在 show 之前等于没改），置 `WS_EX_TOOLWINDOW|WS_EX_NOACTIVATE`、**按位清** `WS_EX_APPWINDOW`，再 `ITaskbarList::DeleteTab` 回收已建出的按钮，球窗 `.title("")`；读回实测 `0x40118` → `0x8000198`。
- **v1.77b：球必须铺满活柱**（球径 `46px → **56px**` = `ICON_SIZE_LOGICAL`）—— 窗口区域是「内接于活柱的圆」（现场只读取证 `GetRgnBox=(0,0)..(55,56)`、`PtInRegion` 网格两端递减），球比它小 ⇒ 中间那圈透明环带把窗口后面的东西透出来（Tauri/WebView2 非真透明 ⇒ 浅色残片；Electron 真透明 ⇒ 无感），这是「图标背景异样」的第三个源头（前两个：v1.73 球元素 `title`、v1.77 任务栏按钮）；连带 hover / grabbing 的 `scale()` 改 `filter: brightness()`、collapsed 的 `opacity` 改 `filter: brightness(.92)`、图标 `30 → 36px`，守卫 `float_page_ball_fills_the_live_pillar`，守卫 ⑥b / ⑥c。
- **v1.78–v1.80：给球窗换「透明地基」**（详见 `docs/changes/2026-10-12.md`）—— v1.78 把区域外扩 4px 仍漏浅色异形、v1.79 回退外扩并让页面自绘不透明黑底仍留锯齿：三案都困在「wry 0.55 + tao 0.35 的透明链路会把 border-radius 抗锯齿这类**半透明像素压平到白底**」这条旧链路里，几何与底色都救不了。**v1.80 弃掉绕路**：升级 **tauri 2.12.1 / wry 0.57 / tao 0.37**（`no_redirection_bitmap` 自 tauri-runtime-wry 2.12 起才透传），球窗 builder 同开 `.transparent(true).no_redirection_bitmap(true)` ⇒ `WS_EX_NOREDIRECTIONBITMAP` + DirectComposition 拿到**真逐像素 alpha**；**删除全部 `SetWindowRgn` / `CreateEllipticRgn`**，新增 `install_circle_hit_test`（`EnumChildWindows` 递归枚举父窗 + 全部 WebView2 子窗 → `SetWindowLongPtrW(GWLP_WNDPROC)` 幂等子类化，进程只拦 `WM_NCHITTEST`：圆内转 `HTCLIENT`、圆外回 `HTTRANSPARENT`，圆心/半径存静态原子量、未初始化 fail-open）⇒ **视觉与命中彻底分离**，与 Electron（`win.setShape(circleShapeRects(…))`）同构；页面恢复真透明、初始化脚本不再注入壳标记。⚠️ v1.80 的真机目视（浅/深桌面软边、四角穿透、拖动/悬停/单击/右键四类交互）**仍待复验**。
- **v1.81：球窗原生化，毛刺链整体退役**（详见 `docs/changes/2026-10-27.md`）—— v1.77b–v1.80 四轮证伪钉死了根因：**圆的边缘只要经过浏览器 CSS 裁剪 / 合成管线，就一定有毛刺**（border-radius 抗锯齿被压平白底、`filter` 强制合成层、`translate(-50%,-50%)` 落到分数物理像素）。v1.81 弃 WebView2 球窗，改 **Win32 分层窗口**：`CreateWindowExW` + `WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST` + `WS_POPUP`（样式创建时一次定死，v1.77 的 `GWL_EXSTYLE` / `DeleteTab` 补丁连同任务栏问题一并消失），`UpdateLayeredWindow`（`ULW_ALPHA` + `AC_SRC_ALPHA`）提交 **32bpp 预乘 ARGB** 位图，圆盘由 **4×4 超采样**覆盖率画出（平涂 + 1px 半透明描边 + `icons/icon.png` 面积平均叠加）—— **圆轮廓就是位图 alpha**，系统合成器逐像素合成。命中由分层窗口的**像素 alpha 穿透**天然承担（alpha=0 自动透传点击）⇒ `WM_NCHITTEST` 子类化 / 命中圆 / 活柱 / 窗口区域 / WebView2 最小宽 hack **全部退役**（连同 `SetWindowRgn` 失败链）。盘面弃蓝色渐变，改读 `HKCU\…\Themes\Personalize\AppsUseLightTheme` 的**平涂白 / 黑底盘**（`WM_SETTINGCHANGE` / `WM_DWMCOLORIZATIONCHANGED` 重绘）。吸附从左 / 右扩为**四边**（上 / 下 / 左 / 右就近择一，`DockEdge` 五值 `none/left/right/top/bottom`，`decide_edge` 平局按 left→right→top→bottom 严格 `<`）。交互全部进 WndProc（`end_press` 分派 `on_drag_end`/`on_cancel`/`on_click`），球页 → Rust 的令牌 + 8 事件链连同 `capabilities/float-icon.json` **整体删除**（无球页 = 无 IPC 攻击面）；`src/floaticon.html` 转 **Electron 专属**（改 canvas 超采样 alpha 位图 + 同套自适应 / 四边吸附）。

## 设置页对应项（Shift+F1 第 4 项）

**启用桌面悬浮图标（悬浮球）**（v1.67 引入，v1.69 纠正为悬浮球，v1.70 修复拖动 / 悬停 / 边界交互，v1.71 单击改 toggle + 数据目录根因修复；v1.81 起球窗原生化、命中改像素 alpha 穿透）：勾选后立即创建小球，取消立即销毁；另有「立即显示悬浮图标」按钮（命令 `show_float_window`，跨平台注册、非 Windows 体内空操作）。**单击球切换原来的主窗口显隐**（可见则隐藏、否则显示聚焦）；球可拖动（屏幕绝对坐标协议，拖出屏会被钳回）、拖到屏幕**四边**（上 / 下 / 左 / 右就近择一）半隐藏吸附（悬停展开、移开延迟 300ms 缩回）、**右键 = 关闭悬浮图标功能 + 唤回主窗口**（v1.76：写 `float_enabled=false` 落盘 + 销毁球窗 + 显示主窗，且同步已打开设置页的复选框）；几何 / 吸附边写入 `settings.json` 的 `float_window` 段（`edge` 取 `none/left/right/top/bottom`）。实现见 `src/floatwin.rs`（+ 子模块 `native.rs` 原生球窗 / `events.rs` 状态机 / `tests.rs` 单测）——v1.81 起**无球页 / 无协议注册 / 无能力文件**（`floaticon.html` 与 `capabilities/float-icon.json` 均已从 Tauri 侧退役）。

- **v1.72**：单击带 **250ms 双击去抖**（`CLICK_DEBOUNCE_MS`，窗口期内第二次点击丢弃，防主窗「闪现」）；整条交互链路已由**真机合成指针 harness** `--float-probe` 取证（见下节）。
- **v1.81**：球窗原生化后上列 v1.77 / v1.77b / v1.78–v1.80 的机制**全部退役**（任务栏补丁 `DeleteTab`、活柱铺满、`WM_NCHITTEST` 子类化、页面 `filter` 反馈）—— 原生样式创建时一次定死（`WS_EX_TOOLWINDOW` 不进任务栏）、窗口 == 球 == 位图（无环带）、命中 = 像素 alpha 穿透；悬停 / 拖动 / 半隐藏的视觉差异只改**平涂色**（白 / 黑 + 亮度系数）。历史条目仅作演进存档，勿再按其描述维护代码。

## 真机探针（`--float-probe`，仅 Windows）

`target/debug/web2app.exe --float-probe`（或 `cargo run -- --float-probe`）跑一次**真机合成指针**验证：
建窗形态（`phase_ball_frame`：原生 HWND 存在 / 可见、`get_webview_window("float")` 必须为 `None`（球窗不再是 WebView）、
球窗 rect == `ball_px(ball_scale())`、扩展样式四项 `WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST`、
`WindowFromPoint` 像素 alpha 穿透**环境诊断**（球心命中球窗 / 角点穿透给下层，**不计入失败**））、自由态拖动（1:1 跟手 / 软钳制 / 松手钳回，物理坐标口径）、
**四边吸附**（`phase_top_dock` / `phase_bottom_dock` 及左右缘：y / x 轴 `expanded_*` / `collapsed_*` 几何跟随 + 悬停展开）、
单击 toggle 主窗、右键 = **关闭悬浮图标功能并唤回主窗口**
（球窗**被销毁** + `float_enabled` 置 false + 主窗被唤回；复原后吸附态保留）
（断言条数随版本演进，确切条数以真机 `ALL PASS` 行为准 —— v1.81 改的是 `floatprobe.rs` 自身，须重新打包后在真机复验）。

- **驱动方式**：`floatwin::inject_page_event` 直接驱动**真实状态机**（`on_down` / `on_move` / `on_drag_end` /
  `on_click` / `on_cancel` / `on_peek_enter` / `on_peek_leave` + `dock_state`），参数即**物理屏幕坐标** ——
  v1.81 起没有球页 / 令牌 / 8 事件链，探针也不再绕过任何环节：分派与真实鼠标消息走**同一个** WndProc 处理路径。
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
  > ⚠️ 历史取值：v1.77b–v1.79 用窗口椭圆区域裁圆时，`GetWindowRgnBox` 对椭圆区域**含端**（`[0,70)×[0,70)` 上报 `(0,0,69,69)`，宽高各 −1；100% / 125% 缩放一致）。**v1.80 起球窗不再设任何窗口区域（v1.81 原生球窗同样无区域）**，探针不再查 `GetWindowRgnBox`，这条「含端」事实只在读旧版本证据时才有意义。
- ⚠️ 本机删除走安全垫片（递归删除一律被拦、有配额）⇒ 探针的临时运行目录**不要自己清**，换新目录重来即可；
  WebView2 的 UDF 落点是 `%LOCALAPPDATA%\com.example.web2app\EBWebView`，**与 exe 所在目录无关**
  （「换个全新目录跑 exe」不会换掉脏 UDF；要禁用就**重命名备份**）。
