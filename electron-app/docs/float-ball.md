# 桌面悬浮球（Electron 侧实现）

> 本文是 electron-app「桌面悬浮球」的唯一详细记录：实现结构与 v1.68–v1.81 逐轮演进。
> 行为契约速览（创建/销毁、关闭驻留、设置页）见 [../AGENTS.md](../AGENTS.md)「设置 / 托盘 / 全局快捷键 / 窗口几何」；
> 共用球页 `floaticon.html` 与 Tauri 侧实现对位见 [../../webview-app/docs/float-ball.md](../../webview-app/docs/float-ball.md)。

## 实现结构与交互协议

设置开关 `float_enabled` 创建 / 销毁（含保存失败的成组回滚）；一个 **56×56 透明 / 无边框
（`thickFrame:false`，否则隐形缩放边框使外框实测 59×58）/ 置顶 / 不占任务栏的小球**，
`loadFile` 内置本地球页 `floaticon.html`（双壳单一真源，由 `stage-assets.cjs` 拷入 `out/inject/`；
球内图标即软件自身 `icon.png`，同目录 stage），**不加载任何网页、不导航、不 bootstrap、不碰 sidecar / session**。

- **单击球 = 经 `deps.toggleMainWindow()` 切换原来的主窗口显隐**（v1.71：可见且非最小化 → 隐藏，否则显示聚焦；
  此前调 `showMainWindow` 只显示，主窗可见时点击无反应）。
- **拖动协议（v1.70）**：页面 rAF 上报**屏幕绝对坐标**、pointerdown 先报 `down` 存按下点
  （首帧 move 先展开吸附态再算锚点，1:1 跟手）、pointercancel 未拖动只复位；拖动中软钳制（至少留 16 逻辑像素）、
  自由态松手钳回显示器；悬停展开 / 离开延迟 300ms 缩回（到期真实光标校验）。
- **右键 = 关闭悬浮图标功能 + 唤回主窗口**（v1.76：入口 `hideFloatFromIcon` 把 `float_enabled` 置 false 并落盘 +
  销毁球窗 + `presentMainWindow()`（含多页签空壳自愈），且同步已打开设置页的 `#floatEnabled`；
  与 Tauri `floatwin::disable_float_from_icon` 逐条对齐 —— 只把球藏起来会留下「进程还在跑、桌面上却再也唤不出窗口」的死局）。
- **IPC**：经 `float-preload.ts` 的七条白名单 IPC（`web2app:float-click/-down/-move/-drag-end/-cancel/-peek/-hide`）驱动，
  主进程逐条校验 sender。
- **几何**：纯逻辑在 `float-layout.ts`（DIP 空间，常量与 Tauri `floatwin.rs` 对齐：窗 56 / PEEK 28 半球 / SNAP 28 /
  MARGIN 12 / MIN_VISIBLE 16 / 延迟 300ms），落盘经 `screen.dipToScreenRect` 转**物理像素**写 `float_window` 段。
  吸附 / 拖动的**决策编排**也在该文件（2026-10-24 P2-4 纯函数化、v1.81 四边化）：`decideEdge(b, mon)` 四边
  就近择一（平局 left→right→top→bottom 严格 `<`，左 / 右须纵向重叠、上 / 下须横向重叠）、`planDrop(b, mon)`
  松手落点（吸附贴边按轴分派 / 自由态钳回）、`planPeekCollapse(edge, mon, winY, cursor)` 悬停延迟到期缩回判定
  （光标在缩回后矩形内则不缩）；`float-window.ts` 只做摆放（`placeFloat`）/ 登记 dock / 落盘 / 通知前端，
  IPC 与窗口生命周期一行未动。
- **v1.71 几何漂移**：出屏 / 负坐标移动后 `getBounds()` 的宽会逐次漂移（实测 56→58→60），
  `placeFloat` 是**唯一**定位入口且始终带完整固定 56×56，其余几何一律不读 `b.width`；
  隐藏建窗、`ready-to-show` 后显现（+1s 兜底）；close 默认转隐藏，`isQuitting()` 放行退出（否则 app.quit 被取消）。

## 逐轮演进（v1.68–v1.81）

- **v1.68 两条「静默失败」缺陷**：①**开着悬浮窗退不掉程序**——悬浮窗 `close` 默认 `e.preventDefault()` 转隐藏，
  而 Electron 里「close 被 prevent」会**取消正在进行的 `app.quit()`**（实测：主窗已关、悬浮窗只剩隐藏、进程仍活着）
  ⇒ `FloatWindowDeps` 新增 `isQuitting`（入口接 `quitting`，由 `before-quit` / `requestQuit()` 置位），退出路径放行；
  ②**冷启动不建立进程级副作用**——入口冷启动调 `applySettings(loadedSettings.settings, …)`，而 `runtimeSettings`
  初始化时已等于**同一份** `loadedSettings.settings`，托盘 / 悬浮窗的「变化才动」diff **恒为 false**
  ⇒ 控制器新增 `applyInitialSettings`（prev 固定 `DEFAULT_SETTINGS`），重启后悬浮窗（与托盘图标）才会照常出现；
  另 `hideFloatWindow()` 补清 `moving`（拖动途中被隐藏会让悬停展开永久失效）。
- **v1.69 悬浮形态纠正**：悬浮「小窗」→ 悬浮图标（悬浮球）。使用者明确否定 v1.67「320×460 小窗加载同一站点」的
  方向——悬浮的应是**图标**，**单击图标显示原来的主窗口**。重写：56×56 透明无边框置顶窗 + 内置本地球页，
  **删除** target 缓存轮询 / `loadURL` / `float-bootstrap` 同步取脚本 / webFrame 注入 / session 共享；
  托盘菜单「悬浮窗」→「悬浮图标」。
- **v1.70 拖动协议重写**：v1.69 上报 clientX 增量被窗口每帧自身位移抵消（半速跟手），改屏幕绝对坐标 +
  pointerdown 发 `down` 暂存按下点，新增 `cancel`（IPC 五→七、`clearDragRuntime` 统一清按压残留）；
  悬停抖动——离开延迟 300ms（`peekGen` 代际 + 到期 `screen.getCursorScreenPoint` 真实光标校验）；
  边界——`clampVisible` 软钳制留 16 逻辑像素、`clampFreeInside` 自由态钳回；`thickFrame:false` 修隐形缩放边框。
- **v1.72 两项**：①**四角「看得见但点得着」**——`transparent:true` 只让四角**看不见**，四角在系统看来仍属窗口、
  仍吃鼠标（layered window 的完全透明像素**不会**自动穿透）⇒ `float-layout.ts` 新增纯函数 `circleShapeRects(d)`
  （`setShape` 只接受 `Rectangle[]`、无椭圆入口，故用扫描线逐行 1 像素高矩形逼近圆；按「像素中心是否落在圆内」
  定左右端，阶梯与真圆偏差 <1px），`float-window.ts` 建窗后 `win.setShape(circleShapeRects(DEFAULT_FLOAT_W))`
  并**包 try/catch**（`setShape` 在 Electron 里标记 `@experimental` win32/linux ⇒ 失败只降级成「四角挡点击」的旧行为，
  **绝不让球不可用**；区域只决定**命中**，视觉仍由 CSS 圆 + `transparent` 负责）；②**双击去抖**（共享球页）——
  `floaticon.html` 加 `CLICK_DEBOUNCE_MS = 250` + `lastClickAt` + 两段式 `endPointer`
  （系统双击 ~100ms 内送来两次 `pointerup`、旧实现两次都发 `click` ⇒ toggle 相互抵消、主窗「闪现」），
  真机 harness 第 ⑥ 组随之改三态 + 钉住 `pointercancel` 不触发。
- **v1.73**：球页**不再挂原生提示**（原 `title` 改 `role="button"` + `aria-label`）—— 球窗 `alwaysOnTop` 会把
  原生提示条（独立非置顶窗口）压到球窗之后、只从透明环带漏出浅色残片，`float-contract.test.ts` 有同款守卫。
- **v1.75**：关窗驻留判据纳入 `float_enabled`（「球在 ⇒ 应用驻留」，见 AGENTS.md 行为契约表「关闭按钮」行），
  设置页新增「退出程序」并接到入口的 `requestQuit`（常驻后台时唯一的退出入口）。
- **v1.77：本壳无需改动，但它是「仅限 Tauri」症状的对照组** —— Tauri 侧实测球窗是**带着任务栏按钮**被显示的
  （`WS_EX_APPWINDOW`，`.skip_taskbar(true)` 挡不住）⇒ 任务栏按钮的原生提示条被置顶球窗压在身后、
  从透明环带漏出残片；而本壳 `skipTaskbar: true` 是**真移除**（自有窗口类写 `WS_EX_TOOLWINDOW`，
  球窗根本不在任务栏里 ⇒ 没有承载提示条的载体）。⚠️ 注意本壳球窗**是带标题的**（`title: deps.title`），
  照样没症状 ⇒ 反证根因是 `APPWINDOW` 而不是标题文本，别照搬 Tauri 侧「置空标题」那条。
- **v1.77b**：本壳与 Tauri 共用同一份 `floaticon.html`，故同源守卫一并钉死「球径 == `DEFAULT_FLOAT_W`
  （= 区域圆直径）」「球块不得 `scale(`」「collapsed 块不得 `opacity`」—— Tauri 侧把球从 46 改成 56 之后，
  本壳由 `circleShapeRects(56)` 裁出的圆与球严格重合，环带在两侧都不再存在。
- **v1.78–v1.80：本壳再一次「无需改动」** —— Tauri 侧那四轮「背景异形」收口全发生在它的旧透明链路上
  （wry 0.55 + tao 0.35），本壳一直走真 DirectComposition alpha，代码只改了注释。
  但两壳的**命中实现自此分家**：Tauri 删除 `SetWindowRgn` / `CreateEllipticRgn`，改用 `WM_NCHITTEST` 子类化
  （`install_circle_hit_test`）；本壳仍用 `win.setShape(circleShapeRects(56))`。
  共同语义不变——**命中只管命中，都不裁视觉**（`docs/changes/2026-10-12.md`）。
- **v1.81：球面 canvas 超采样位图 + 四边吸附**（详见 `../../docs/changes/2026-10-27.md`）——
  本壳虽无 Tauri 那条「Chromium 合成压平半透明像素」的毛刺链，但同源球页里的 CSS 画圆
  （`border-radius` + 渐变 + `filter` / `transform` 状态反馈）仍是两壳共同的软边毛刺源，
  且 `translate(-50%,-50%)` 在非 1 / 4 整数倍 DPI 下落到分数物理像素。`floaticon.html` 弃 CSS 画圆：
  **canvas 超采样**（`SUPERSAMPLE = 8`）把「平涂底盘 + 1px 半透明描边 + 图标（drawImage 烘入）」
  栅格化成**一张带 alpha 的位图**（`background-image` data URL），边缘 alpha = 子样本覆盖率；
  预渲染 idle / hover / collapsed 三变体写 `--w2f-disc*`，状态切换**只换图**（不再有合成层副作用）；
  位图边长 = **整数物理像素**（`sizeDev = round(56 × dpr)` 算完回写 CSS px），跟随深浅色
  （`matchMedia('(prefers-color-scheme: dark)')` + change 重挂）与 dpr（resolution 监听重渲）。
  吸附从左 / 右扩为**四边**（`FloatEdge` 五值、`decideEdge` 就近择一、`expandedY` / `collapsedY` /
  `clampX`、`planDrop` / `planPeekCollapse` 按轴分派，`float-window.ts` 六处坐标同步按轴取值）。
  ⚠️ 命中仍归 `win.setShape(circleShapeRects(56))`（扫描线圆），与位图圆盘严格等大 —— **视觉 = 命中**不变。

## 守卫与验证

- 静态契约 `test/float-contract.test.ts`：floaticon.html↔preload↔主进程七通道 IPC 名一致（v1.70 增 down / cancel）、
  float-click 接 showMainWindow、sender 校验、主框架门控、`thickFrame:false` 与透明窗属性、
  屏幕绝对坐标拖动协议 + 悬停延迟、只 loadFile 本地页（无 loadURL / bootstrap / sidecar / target 缓存）、
  stage 登记 floaticon.html、**`win.setShape(circleShapeRects(...))` 必须存在且必须包 try/catch**（⚠️ 反向对照时发现裸 `includes`
  会被注释行骗过 ⇒ 改为先剥注释再匹配）；v1.81 旧断言**逐条改写**（非删除）：补 canvas 超采样位图
  （`SUPERSAMPLE` / 三变体 `--w2f-disc*` / 整数物理像素对齐）、禁 CSS 画圆合成（border-radius / 渐变 /
  filter / transform 反向钉死）；Tauri capability 两条随**球页退役**同口径改写（`capabilities/float-icon.json`
  已删除，断言改为「不得存在」）。
- 纯几何单测 `test/float-layout.test.ts`：吸附边判定 / 展开缩回 x / y clamp / 多显示器负原点 /
  物理像素最小尺寸（常量 56 / PEEK 28）+ `circleShapeRects` 用例；P2-4 起补
  `planDrop` / `planPeekCollapse` 两个决策编排用例（吸附 / 自由 / 屏外钳回 / snap 容差；
  光标半开区间 / 左右镜像 / y 越界钳回）；v1.81 扩**四边**（`decideEdge` 贴上 / 下判 top/bottom、
  `expandedY` / `collapsedY` / `clampX`、按轴分派）。
- 真机合成指针 harness：v1.70 交付时 15 项、v1.72 起第 ⑥ 组扩为双击去抖三态共 **17** 项；⚠️ 它是
  **一次性本机工作区产物**（`tests/float-*-harness/` 等，目录名随批次变化），本文不指向具体路径，
  按「带 `--no-sandbox --in-process-gpu` 并剥掉 `ELECTRON_RUN_AS_NODE`」的既有禁令自建即可。
- 真机目视四步（探针覆盖不到的冷启动副作用 / 退出放行 / 销毁语义 / 单击调出主窗）见
  [../AGENTS.md](../AGENTS.md)「验证清单」悬浮球行。⚠️ 该清单与吸附手感 / 拖动跟手 /
  悬停展开缩回时机 / 双击去抖等交互项**无法自动化**（只有真机 + 合成指针能验）——
  2026-10-24 P2-4 只把几何决策纳入单测，真机项仍按本清单人工验证，不做自动化伪装。
