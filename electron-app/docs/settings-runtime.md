# 设置、托盘、快捷键与窗口几何（electron-app）

> 自 `AGENTS.md` 拆出。悬浮球设置项与实现见 [float-ball.md](float-ball.md)；
> `settings.json` 落点与 `data_dir` 关系见 [runtime-data.md](runtime-data.md)。

### 运行期只有一个 JSON

| 文件 | 内容 | 写在哪 |
|---|---|---|
| `settings.json` | 唤醒快捷键、是否启用托盘、关闭是否最小化到托盘、**是否启用桌面悬浮球**（`float_enabled`），**以及 `window` 段主窗几何与 `float_window` 段悬浮球几何**（坐标 / 尺寸均为**物理像素**；主窗另有是否最大化，悬浮球另有吸附边 `edge: none/left/right`） | **与数据目录同位置**：`<data_dir>\settings.json`（默认 `data\settings.json`，跟着 exe 走）；旧位置 `%APPDATA%\<app_name>\` 仅作**读取兜底**（升级兼容，不迁移不删除） |

⚠️ 2026-09-23 起窗口几何不再单独占一个 `window-state.json`，而是并进 `settings.json` 的
`window` 段（设置字段仍平铺在顶层）—— 用户侧「只需要一个 JSON」。旧的 `window-state.json`
**不再读也不再写**：升级后窗口会回一次默认尺寸 / 位置，那个文件可以手工删掉。
⚠️ 因此**所有写入方都必须走读-改-写**（`saveSettings` / `saveWindowState` / 悬浮球的
`persistGeometry`）：拿内存里的整份文档直接落盘，会在「挪了下窗口 + 改了设置 + 拖了悬浮球」
同时发生时静默吃掉其它段。探针断言（`写几何不吃掉设置` / `写设置不吃掉窗口几何` /
`写设置/写主窗几何不吃掉 float_window 段`）专门钉这件事。

⚠️ **落点 = 数据目录**（2026-09-23 使用者指出）：此前它固定放 `%APPDATA%\<app_name>\`，于是选了绿色版
（`data_dir = "data"`）之后 cookie / 缓存 / 日志都跟着 exe 走，**唯独设置文件留在系统盘** ——
同一套「程序的数据」里唯一不听话的一项。现在写入位置一律是 `<data_dir>\settings.json`，与
`data_dir` 字段的语义完全一致（`../docs/packaging-config.md` §4.3），Tauri 版同规则同位置。

> 升级兼容：新位置还没有文件时会读旧位置那份（`%APPDATA%\<app_name>\`），第一次写盘即落到数据目录；
> 此后旧文件是死文件，可以手工删掉。⚠️ 读-改-写走的是**同一份读取候选**，否则升级后第一次保存
> 会用「新位置（空）算出来的文档」把老设置清零。

### 行为契约（与旧版逐条对齐）

| 项 | 行为 |
|---|---|
| 唤醒快捷键 | 显示 / 隐藏主窗口。默认**未设置**；可在设置窗捕获或从下拉选择。存的是 JS `KeyboardEvent.code`（如 `Digit4`） |
| 设置快捷键 | `Shift + F1`，**固定不可改** |
| 隐藏 / 恢复语义 | 可见且非最小化 → 隐藏；其余（隐藏 / 最小化）→ 恢复并聚焦。⚠️ 不能只看可见性，否则最小化状态下第一次按快捷键会「没反应」 |
| 托盘 | 默认**不启用**。Windows 菜单五项：显示窗口 / 设置 / 悬浮图标 / 隐藏到托盘 / 退出（其它平台四项，无悬浮图标）。左键单击托盘图标 = 切换显隐（右键才弹菜单） |
| 关闭按钮 | **任一「驻留理由」成立即隐藏、不退出**：①勾选「启用托盘」+「关闭时最小化到托盘」→ 隐藏到托盘；②勾选「启用桌面悬浮球」（仅 Windows，v1.75）→ 隐藏到球（点球即唤回）。两者都不满足 → 关闭即退出。⚠️ 理由②不可省：少了它，只启用悬浮球时关窗走 `else` 分支 ⇒ 主窗被**真销毁**、球窗却还活着 ⇒ ①球窗挡住 `window-all-closed`，进程留成既无界面也退不掉的僵尸；②点球时 `presentMainWindow` 的 `isDestroyed()` 早退 ⇒「点了悬浮图标不回窗口」（2026-10-11 真机实测，策略 `tray_enabled:false/minimize_on_close:false/float_enabled:true`） |
| 窗口几何 | 上次最大化 → 直接最大化；位置已在屏幕外 / 尺寸过小 → 丢弃几何并最大化（**保证窗口一定可见**）；否则原位恢复 |
| 桌面悬浮球（v1.67 引入，仅 Windows） | 设置开关 `float_enabled` 创建 / 销毁（含保存失败的成组回滚）；**56×56 透明 / 无边框（`thickFrame:false`）/ 置顶 / 不占任务栏的小球**，`loadFile` 内置本地球页 `floaticon.html`（双壳单一真源，由 `stage-assets.cjs` 拷入 `out/inject/`），**不加载任何网页、不导航、不 bootstrap、不碰 sidecar / session**；**单击球 = 经 `deps.toggleMainWindow()` 切换主窗显隐**（可见且非最小化 → 隐藏，否则显示聚焦）；拖动走**屏幕绝对坐标**协议（页面 rAF 上报、pointerdown 先报 `down` 存按下点、pointercancel 只复位），软钳制 + 吸附展开 / 离开延迟 300ms 缩回；**右键 = 关闭悬浮图标功能 + 唤回主窗口**（v1.76，与 Tauri `floatwin::disable_float_from_icon` 逐条对齐）。实现骨架：`float-preload.ts` 七条白名单 IPC（主进程逐条校验 sender）；几何纯逻辑在 `float-layout.ts`（常量对齐 Tauri `floatwin.rs`：窗 56 / PEEK 28 / SNAP 28 / MARGIN 12 / MIN_VISIBLE 16 / 300ms），落盘经 `screen.dipToScreenRect` 转物理像素写 `float_window` 段；`placeFloat` 是**唯一**定位入口且始终带完整 56×56（v1.71：出屏 / 负坐标后 `getBounds()` 的宽会逐次漂移，其余几何一律不读 `b.width`）；v1.72 起 `win.setShape(circleShapeRects(56))` 四角穿透（包 try/catch 失败只降级）+ 250ms 双击去抖；球页不挂原生提示（v1.73）。v1.68–v1.80 逐轮演进、两壳命中实现分家点与全部守卫见 [float-ball.md](float-ball.md) |

### 设置窗

设置窗是 `settings://localhost/` 协议提供的**无边框窗**（620×600，可缩放，最小 460×400，居中，
`frame:false`），内容直接取自 `../webview-app/dist/settings.html` —— 也就是 Tauri 版的那一份，
**一行都不改**。版式是「面板铺满整窗 + 正文滚动」：标题 / 关闭 / 拖动都由设置页自绘在面板头部
（`-webkit-app-region:drag` 负责拖动）；面板直接贴住窗口四边 —— 不再有「居中面板 + 四周底衬」，
那圈底衬正是使用者指出的「大块空边框」。

打开方式有三条，走的是**同一个** `openSettings()`：托盘菜单「设置」、全局快捷键 **Shift+F1**、
以及标题栏「设置」按钮（位置在**最小化左侧**；单窗口模式经 `win:open-settings`，
多页签模式经标签栏的 `tabs:open_settings`）。

它靠 `src/renderer/settings-preload.ts` 提供的 `window.__TAURI_INTERNALS__.invoke` 垫片工作，
垫片放行六个命令（`get_settings` / `save_settings` / `close_settings_window` / `clear_web_data` /
`show_float_window` / `quit_app`），其余一律拒绝。
⚠️ 命令名是**三处逐字一致**的契约：`dist/settings.html` 的 `invoke` 字面量 / 本壳的
`settings-window.ts` 处理器 / `settings-preload.ts` 的 `ALLOWED` 白名单 —— 漏一处就是
「按钮点了没反应」（`float-contract.test.ts` 有守卫钉这三处）。
`quit_app`（v1.75，设置页「退出程序」）必须接到入口的 `requestQuit`：它是「**先置 `quitting`
再 `app.quit()`**」的唯一实现，直接用裸 `app.quit()` 会被上面的「驻留」分支拦下。
好处是设置页永远不会分叉：改一次两个壳同时生效。

> 找不到 `settings.html` 时会退到一个内置精简页（托盘 / 关闭行为 / 悬浮图标三个复选框），不会出现空白窗。
