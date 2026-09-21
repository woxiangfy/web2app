# Electron 多页签（M5）设计方案

> **文档定位**：把 `docs/electron-plan.md` §5 的 M5 多页签细化为可直接施工的方案
> （该处原文写的是「⏸ 暂缓」，属**历史表述**；M5 现已完成）。
> 该文档 §3.1 给出的方向是 `WebContentsView`——本文用**本地实测 + 社区落地形态核查**把它收敛成设计，
> 并纠正/补充四处原判断（见 §3.4、§2.3）。
>
> **当前状态**：**已实现并完成真机验收（T0–T4 全部 ✅；V1–V4 目视项 2026-09-18 全部关闭）**。
> `tabs_enabled=true` 走多页签模式（壳页面 + 子 `WebContentsView`）；`tabs_enabled=false`（默认）
> 单窗口模式行为逐条不变。真机清单与实测结论见 `electron-tabs-test-steps.md`。
>
> **v1.1 的重要更正**：v1.0 结论是「必须把窗口类换成 `BaseWindow`」。经社区核查 + 补充实测
> （§2.3 / §3.5），该结论**过强、已推翻**：真正落地的多页签应用都用 `BrowserWindow` 当壳。
> 选型相应改为**方案 A′**（见 §4）。

---

## 1. 业务目标与验收标准

### 1.1 目标

`config.json` 的 `tabs_enabled=true` 时，Electron 绿色版呈现与 Tauri 版**观感一致**的多页签
（顶部标签栏 + 多个内容页，共享同一份 cookie/localStorage 与同一套代理链路）；
`tabs_enabled=false`（默认）时，行为与当前单窗口模式**逐条不变**。

### 1.2 验收标准（可观测、有退出码）

| # | 判据 | 手段 |
| :-- | :--- | :--- |
| A1 | 单窗口模式**零回归**：现有 `probe` 112 条全绿 | `pnpm run probe --no-sandbox --in-process-gpu`，退出码 0 |
| A2 | 多页签模式：新增断言组全绿 | 同上，加 `--tabs`（本文 §8 清单） |
| A3 | **打包产物**自带探针在两种模式下均退出码 0 | `Start-Process -Wait -PassThru` 取 `ExitCode`。⚠️ 前提：**探针的起窗模式必须由探针标志决定、不受 `tabs_enabled` 影响**（v1.6 修复，见 §13）——否则 `tabs_enabled=true` 的包里 `--probe` 会以页签模式起窗而失败 |
| A4 | 标签栏与内容区**逐像素**不重叠、不留缝 | §8 的「位置 + 存在性」成对判据 |

【默认补全】以下按行业通用默认值处理，未另行确认：
- **平台**：Win10+ x64 绿色版，不做代码签名；
- **页签上限**：32（对齐 Tauri `MAX_TABS` 熔断值，理由见 `tabs.rs` 的「异常建窗风暴」注释）；
- **关闭最后一个页签** = 关闭宿主窗口（走既有托盘/退出统一语义，对齐 Tauri `close_tab_inner`）；
- **不做**页签拖拽排序、不做标签栏溢出菜单、不做页签恢复（Tauri 版同样没有）；
- **不新增 `config.json` 字段**：复用既有 `tabs_enabled`，上限写成代码常量而非配置项。

---

## 2. 调研结论

### 2.1 Electron 侧：视图 API 已换代，直接用新的

| 形态 | 状态 | 说明 |
| :--- | :--- | :--- |
| `BrowserView` | **Electron 30 起废弃** | 官方迁移指引明确「replaced by `WebContentsView`」；仍是 SIM 形态 |
| `WebContentsView` | **现行一等 API** | 直接挂在 Chromium 的 Views 渲染管线，`setBounds` / `setVisible` / `setBackgroundColor` 齐备 |
| `BaseWindow` | **多视图窗口的基类之一** | 「flexible way to compose multiple web views in a single window」；`BrowserWindow` 是它「带一个全尺寸视图」的子类 |
| `<webview>` 标签 | **官方劝退** | 官方明示「我们不建议使用 WebView……不能保证该 API 在将来版本仍可用」，且需显式打开 `webviewTag` |

关键差异（官方文档原文要点，直接影响设计）：

- `WebContentsView` **默认白底**（`BrowserView` 是透明），需要 `setBackgroundColor` 顶住首帧；
- `resize` 不再有 `setAutoResize`，改为监听窗口 `resize` 后自行 `setBounds`；
- 视图**不会随窗口关闭自动销毁** ⇒ 不管就是内存泄漏，必须自己 `close()`；
- `BaseWindow` **自身没有 `webContents`**；而 `BrowserWindow` 有——这一条是 §2.3/§4 的分水岭。

### 2.2 Tauri 侧：同类能力仍是不稳定特性 ⇒ 两壳**必然不同构**

Tauri 2 确实有单窗口多 webview 的入口 `Window::add_child(WebviewBuilder, pos, size)`，但：

- 需开启 `features = ["unstable"]`；
- 官方 PR 自述「The API isn't finished yet… the auto resize thing needs more work」；
- 子 webview 的定位/尺寸问题仍在（tauri#10420：Linux 下子视图被纵向平铺、`set_bounds` 变空操作）；
- 另有 Windows 下在同步命令里建 webview 会死锁的已知问题（wry#583）。

因此本项目 `tabs.rs` 选择「**多窗口伪装 tab**」（一个宿主窗口画标签栏 + N 个 owned 顶层窗口承载内容），
代价是必须自己解决 z-order 遮挡、`place_tab` 闭环校准、`destroy()` 防重入三件事——
`tabs.rs` 顶部那三处历史 bug 注释就是这笔账。

> **结论**：**不要试图把 `tabs.rs` 的形态移植到 Electron。** Electron 侧有稳定的一等视图嵌入能力，
> 正确做法是「一个窗口、一个壳、N 个内容视图」，实现量比 Tauri 侧**少一个量级**
> （对照：`tabs.rs` 872 行；本方案核心逻辑预计 300–400 行 TS）。

### 2.3 社区落地形态核查（2026-09-17 联网）

「官方文档推荐什么」与「社区真正在跑什么」是两件事，本节把已落地的多页签 Electron 应用逐个对照，
用来回答一个具体问题：**壳（标签栏）该由「窗口自身页面」承载，还是由「独立视图」承载？**

| 参考实现 | 窗口类 | 壳（标签栏）承载 | 内容承载 |
| :--- | :--- | :--- | :--- |
| **electron-browser-shell**（500+★，Chrome 扩展支持的权威参考实现，GitHub Actions 含 Windows 作业） | `BrowserWindow`，`frame:false` + `titleBarOverlay` | **窗口自身 webContents**（加载 `webui.html`） | 子 `WebContentsView`，`y = toolbarHeight(64)` 起，切换用 `setVisible` |
| **aka-browser**（DRM 浏览器，React UI + `TabManager`） | `BrowserWindow`（窗口自身 renderer 画全部 chrome） | **窗口自身 renderer** | 子 `WebContentsView`（`TabManager` 逐页签管理） |
| **mamezou-tech/electron-example-browserview**（示例级演示仓库） | `BaseWindow` | 独立 `WebContentsView`（`tabbar.html`，**无 drag 区域**；窗口带系统边框） | 子 `WebContentsView`（`y=40/60` 起） |
| `gaoyujin/electron-multi-tabs`、`electron-tabs` 库 | `BrowserWindow` + `<webview>` | 渲染进程 DOM | `<webview>` 标签 |
| 官方文档 | 明示「只有一个全尺寸视图时，`BrowserWindow` 是更简单的选择」；`BaseWindow` 的定位是**组合多个视图** | — | — |

**结论分两层：**

1. **视图 API 层 → 原判断正确**：社区一致收敛到 `WebContentsView`。曾经的默认库 `electron-tabs`
   **已停更**（README 首行「discontinued」，要求 `webviewTag: true`），`<webview>` 被官方劝退。
2. **窗口类层 → 原判断过强，需更正**：**真正落地的多页签应用都用 `BrowserWindow` 当壳**
   （壳 = 窗口自身 renderer），把内容放进子 `WebContentsView`；`BaseWindow` 主要出现在
   官方文档与教学示例里，且这些示例普遍**没有可拖动的自绘 chrome**（窗口用系统标题栏）。

**上游已知限制（三条都指向同一个设计约束：chrome 与内容不要重叠）**

| 上游问题 | 内容 | 对本方案的意义 |
| :--- | :--- | :--- |
| electron#43320（`component/draggable-regions`，30/31/32/34 均复现） | 视图**重叠**时拖拽区判定是**并集**：`no-drag` 对上层视图无效。维护者明确「Chromium 不区分『未设置』与『显式 no-drag』……除非重构拖拽区范式，否则无法修复」 | 我们的**平铺**布局（壳 `0–38` / 内容 `38–H`）天然规避；**硬约束：chrome 与内容永不重叠** |
| electron#41044（Windows 子视图被画在窗口自身内容**之下**） | 该 issue **已关闭**，维护者认可「让 API 子视图保持在原生视图之上」的修复方向 | A′ 依赖它。参考实现在 Windows CI 上跑同一形态，但仍列为**真机复验项**（§8.3 V4） |
| electron#42061 | 没有 `setTopBrowserView` 的对应 API；抬层靠 remove + add | 已在 §3.5 实测出确切语义，见探针 P12 |
| macOS 原生页签（`win.addTabbedWindow()` / `new-window-for-tab`） | **仅 macOS** | 目标平台是 Windows ⇒ 没有「直接用系统页签」这条捷径 |

---

## 3. 前置实测（本地 Electron 44.4.1，主机 DPI = 200%）

探针脚本与结果：`%TEMP%\w2a-tabs-spike\`（`main.cjs` / `result.json` / `stages.log`）。
所有结论均为**实测**，非文档推断。

| # | 问题 | 实测结果 | 对设计的影响 |
| :-- | :--- | :--- | :--- |
| 1 | `BaseWindow.webContents` 存在吗 | `typeof === 'undefined'` | 若走 `BaseWindow`：挂在 `win.webContents` 的逻辑必须下移到每个视图。**A′ 下这条不适用**（`BrowserWindow` 有 `webContents`） |
| 2 | `BrowserWindow.contentView.children` 含自身视图吗 | `childCount = 0` | ⚠️ 该事实本身成立（自身视图不在 JS 子视图列表里），但 v1.0 由它推出的「**因此不能用 `BrowserWindow` 当壳**」**是错的**——见 §3.5 第 1 条 |
| 3 | 窗口几何 API 是否齐全（供 `window-state.ts` 复用） | `getBounds` / `isMaximized` / `isFullScreen` / `setPosition` 全部为 `function` | ✅几何持久化与恢复**零改动**复用 |
| 4 | `setBounds` / `getBounds` 的坐标系 | 设 `{0,38,800,562}` 读回完全一致 | ✅**DIP 坐标**，不需要像 Tauri 那样乘 DPI、更不需要 `place_tab` 三轮回环校准 |
| 5 | `setVisible` / `getVisible` | 双向生效，读回准确 | ✅页签切换首选 `setVisible`（不动 z-order、不触发重排） |
| 6 | `addChildView(view, index)` 与重复添加 | `index` 形式可用；重复添加会把该视图**抬到最上层**，children 数不增 | ✅z-order 可显式控制，作为兜底切换手段（确切语义见 §3.5 末条） |
| 7 | `ready-to-show` 在视图上是否触发 | **未触发**（`readyToShowFired = false`） | ⚠️§3.5 第 5 条已给出确定结论：这是 `BrowserWindow` 专有事件 |
| 8 | 页签标题能否直接取到 | `page-title-updated` 收到 `"Renamed"`，`getTitle()` 一致 | ✅**不需要注入脚本**（Tauri 侧要 `tab_title.js` + IPC 回传），Electron 侧一条事件监听即可 |
| 9 | `webContents.close()` 后视图是否自动摘除 | children 由 3 变 2 | ⚠️**结论已更正（见 v1.4）**：T2 探针实测 `closeTab` 只调 `webContents.close()` 时 `contentView.children` **不减少**、关闭的视图仍可见（18/18 断言里两条 P7 失败）。**必须显式 `removeChildView(view)`**（对非子视图是 no-op，安全）。「webContents.close() 自动摘除」要么有时序依赖、要么仅限特定路径，不可靠 |
| 10 | 每个视图的 `webContents` 能力 | `setWindowOpenHandler` / `session` / `getTitle` / `executeJavaScript` 齐备 | ✅新窗口拦截、下载、`session`、脚本注入均为**逐视图**挂载 |

### 3.4 与 `electron-plan.md` §3.1 的差异（含 v1.0 自我更正）

1. **§3.1 说「几何无需校准」——成立，但原因比预想更强**：不仅是「同窗口内子视图」，
   且实测 `setBounds/getBounds` 同为 DIP 坐标系 ⇒ **完全不需要 DPI 换算**。
   Tauri 侧那段 `TAB_BAR_HEIGHT * scale` + `place_tab` 收敛循环，在 Electron 侧**一行都不用写**。
2. **【v1.1 更正】v1.0 第 2 条作废**：v1.0 由「`contentView.children` 不含自身视图（=0）」
   推出「必须换成 `BaseWindow`」——**推理过度**。该数字只说明「自身视图不登记在 JS 子视图列表里」，
   完全不妨碍往 `BrowserWindow` 的 `contentView` 里 `addChildView`（§3.5 实测 `0 → 1`）。
   **代价**：若不更正，会白白引入「窗口类换代 + 壳视图 + 壳 preload + IPC 重映射」四笔成本，
   并把窗口拖拽押在一个无社区先例的做法上。
3. **§3.1 把「层级顺序需显式管理」列为风险，实测反而可控**：
   `addChildView` 顺序即层级、`setVisible` 双向可读 ⇒ 切换页签有两条干净路径，
   不需要「靠反复 remove/add 调 z-order」这种脆弱写法。

### 3.5 补充实测：A′ 形态（`BrowserWindow` 壳 + 子内容视图）

探针：`%TEMP%\w2a-tabs-spike2\`（`main.cjs` → `result.json`；`order.cjs` → `order.json`）。
两个脚本都遵循项目既有教训：结果**写文件**取证（GUI 子系统 stdout 不可靠）、注册
`window-all-closed` 抑制「全部窗口关闭即退出」、用 `Start-Process -Wait -PassThru` 取退出码。

| # | 问题 | 实测结果 | 对设计的影响 |
| :-- | :--- | :--- | :--- |
| 1 | `BrowserWindow` 的 `contentView` 能否挂子 `WebContentsView` | `children: 0 → 1`，引用一致 | ✅**A′ 的根基成立**（推翻 v1.0 的推论） |
| 2 | `BrowserWindow.fromWebContents()` 能否区分壳与内容 | 壳的 `webContents` → 窗口；**子视图的 `webContents` 也 → 窗口**（两者 `getType()` 同为 `'window'`） | ⚠️**不能靠它区分**。窗口按钮等特权命令必须**校验 `e.sender.id === hostWin.webContents.id`**，否则内容页一旦能发同类消息即可操作窗口（§7-3） |
| 3 | `getBounds()` 与 `getContentBounds()` 是否一致 | `getBounds` = 900×600 @x316；`getContentBounds` = **899**×600 @x317 | ⚠️无边框窗口在 Windows 上有 1px 差 ⇒ 内容视图 bounds **必须用 `getContentBounds()`**，否则右/上边留缝或压 1px |
| 4 | 视图 bounds 坐标系 | 设 `{x:0,y:38,w:900,h:562}` 读回一致 | ✅DIP，与 §3-4 一致 |
| 5 | 子视图的 `ready-to-show` | **未触发**（窗口 `show:false`）；同一窗口**自身**页面的 `ready-to-show` **正常触发** | ✅该事件属 `BrowserWindow`（自带 `webContents`）⇒ **A′ 保留现有首帧时序**；内容视图另找信号（`did-finish-load` / `dom-ready` + `setBackgroundColor`） |
| 6 | `setVisible` / `getVisible` | 双向生效；`setVisible(false)` 后再 `addChildView` **不会**重置为可见 | ✅切换用 `setVisible`；抬层不会把隐藏页签意外显示出来 |
| 7 | 层级语义（children 顺序含义） | 依次 `add A,B,C` → `A,B,C`；重复 `add B` → `A,C,B`；`remove A` 后 `add A` → `C,B,A`；`addChildView(c, 0)` → `C,B,A`（不变） | ✅数组顺序 = **从下到上**（末尾在最上）；重复 add 已存在视图 = **抬到顶且不增条目**；`View` 上**没有 `setZIndex`** |
| 8 | `setBorderRadius` 是否可用 | `function` | 可选（参考实现用它给页签内容做 8px 圆角）；本项目**不用**，避免圆角遮罩在高 DPI / 无 GPU 下引入合成问题 |

---

## 4. 架构选型

| 维度 | **方案 A′：`BrowserWindow`（壳）+ N 子 `WebContentsView`（内容）**（推荐） | 方案 A：`BaseWindow` + 独立壳视图 + N 内容视图（v1.0 原选型） | 方案 B：`BrowserWindow` + N 个 `parent` 子窗口（镜像 Tauri） | 方案 C：`<webview>` / iframe |
| :--- | :--- | :--- | :--- | :--- |
| 壳承载 | **窗口自身 `webContents`**（`shell.html` 现状即此形态） | 独立壳视图 | 窗口自身 | 渲染进程 DOM / 嵌套页面 |
| 视图承载 | 子 `WebContentsView` 同窗口内 | 同左 | N 个顶层 HWND | — |
| 窗口类 | **保持 `BrowserWindow`** | 换 `BaseWindow` | 保持 | 保持 |
| 拖拽标签栏 | **沿用现状已跑通的路径**（窗口自身页面上的 `app-region: drag`） | 无社区先例（示例里的 `tabbar.html` 甚至没设 drag） | 同 A′ | — |
| `shell.html` | **零改动**（它本就是「全窗口页面 + 顶部 38px 的 bar」，与 Tauri 宿主形态一致） | 需在 38px 视口里渲染，布局前提变了 | 零改动 | 零改动 |
| 窗口按钮 IPC | **现有 `fromWebContents` 语义可用**，但需加 `sender.id` 校验（§3.5-2） | 必须自建「sender → 宿主」映射（原 §7-3 陷阱） | 同 A′ | — |
| 既有设施（几何 / 托盘 / 快捷键 / 标题拦截） | **零改动**（对象仍是 `BrowserWindow`） | 全部需泛化为 `BaseWindow` | 零改动 | 零改动 |
| 渲染进程数 | **N**（壳复用窗口自身进程） | N+1 | N+1 | 1 |
| 几何同步 | `setBounds` 一次到位（DIP） | 同左 | 自己算 move/resize/DPI + 闭环校准 | 不需要 |
| z-order 遮挡 / 幽灵窗口 | 不存在 | 不存在 | 必须设 owner + 自愈清理 | 不存在 |
| 关页签 | `view.webContents.close()` | 同左 | 必须 `destroy()` 防重入 | — |
| 依赖的前提 | 子视图绘制在窗口自身内容**之上**（electron#41044，已关闭，**需真机复验**） | 壳视图内 `drag` 生效（无社区先例，无法用 API 断言） | 复刻全部 Tauri 痛点 | — |
| 社区先例 | **两个落地实现（electron-browser-shell / aka-browser）** | 仅官方/教学示例，且均无自绘可拖 chrome | 少 | 多但官方劝退 |

**采用方案 A′。**（v1.1 由 A 改为 A′）

选择 A′ 的三条硬理由：

1. **与 Tauri 侧同构**：Tauri 的 `tabs.rs` 本就是「宿主窗口自身 webview 画标签栏 + 内容另开窗口」，
   `shell.html` 也是按「整页 + 顶部 38px bar」写的。A′ 下**两个壳的形态一致**，
   `shell.html` 一行不改；而 A 要求同一个页面在「38px 视口」里渲染，反而制造了新的分叉。
2. **风险落在已跑通的路径上**：拖拽标签栏、窗口按钮 IPC、几何/托盘/快捷键，A′ 全部沿用现状
   （单窗口模式已验证），v1.0 的 T0 go/no-go（「视图内 drag 能否拖动窗口」）**不再是卡点**。
3. **成本更低**：少一个渲染进程；窗口类不变 ⇒ `window-state.ts` / `tray.ts` / `shortcuts.ts` /
   `index.ts` 的窗口级逻辑零改动；`shell-preload.ts` 的白名单垫片也不是必需了（壳是窗口自身页面，
   用现有 `preload.ts` 即可，仍需按 §7-3 收紧 sender 校验）。

**保留 A 作为回退路径**：若真机复验发现 Windows 上子视图被画在窗口内容之下（#41044 回归），
则回退到 A。两者在 TS 层的差异**仅**「窗口类 + 壳承载方式」，因此实现时把建窗动作收敛到
**单个 `createHostWindow()`**，避免分叉散落（§7-2）。

否决 **方案 B** 的理由：它把 `tabs.rs` 里最难的三件事（z-order、几何校准、关闭重入）
原样搬到 TS 层重写一遍，而换来的唯一好处是「窗口类不用换」——**A′ 同样不用换**，且不用重写这三件事。
另：Electron 官方文档明确在 Windows 上**子窗口不跟随父窗口移动**，同步逻辑一个都躲不掉。

否决 **方案 C** 的理由：`<webview>` 被官方劝退且要开 `webviewTag`（本壳现有
`webPreferences.webviewTag = false`，开它等于给远程页面开新攻击面，与 `preload.ts` 的安全基线冲突）；
曾经的事实默认库 `electron-tabs` 已停更；iframe 则被站点普遍的 `frame-ancestors` CSP 拒绝。

---

## 5. 目标结构

```
BrowserWindow  (frame:false, 同现有无边框外观；resizable 见 §11-D2)
├─ win.webContents  ← 壳页面：加载 shell://localhost/（= webview-app/src/shell.html，零改动）
│    preload: renderer/preload.ts（现有）+ 壳命令白名单
│    职责：标签栏渲染 / 切换 / 关闭 / 前进后退 / 自绘窗口按钮
│    ⚠️ 硬约束 1：整页必须 overflow:hidden（shell.html 已是）——壳页面的滚动条画在
│       内容视图「下面」，但会从 0–38 这一条露出来，正是本项目反复踩的那类缝
│    ⚠️ 硬约束 2：尺寸用 getContentBounds()（不是 getBounds()，见 §3.5-3）
└─ contentView[i]     bounds = { x:0, y:38, width:W, height:H-38 }（从 getContentBounds 推）
     ├─ 只注入 loading.js（**不注入 titlebar.js**）
     ├─ 每个视图独立：page-title-updated / setWindowOpenHandler / did-fail-load
     │                / session(will-download) / context-menu
     └─ 切换：setVisible(true/false)；抬层：remove+add（§3.5-7）
```

> **实现注记（v1.5）——「只注入 loading.js」实际上退化为「什么都不注入」**：T1/T2 实现时内容视图
> 连 preload 都没挂，首帧白闪改由 `WebContentsView.setBackgroundColor('#f7f8fa')` 顶住（`tabs.ts`）。
> 理由：`loading.js` 的 overlay 模式依赖一个 preload 把它经 `webFrame.executeJavaScript` 注入**主世界**，
> 而内容视图为安全刻意不挂 preload（远程页面连 `ipcRenderer` 都碰不到）。为了「加载层」单独引入
> 第三个 preload 的成本，大于它带来的观感收益（纯白底 → 转圈动画）。**这是一个有意的简化**，
> 若后续要补「切页签无白闪 + 失败重试」的完整过渡（§8.3 V3 目视项），再评估引入轻量
> `tab-preload.ts`（只注入 loading.js、不注入 titlebar.js）。当前失败态靠 `did-fail-load` → 页签标题「加载失败」兜底。

**坐标契约**：`38` 同时是 `shell.html` 的 `--bar-h` 与内容视图的 `y` 起点，
两者必须来自**同一个常量**（`TAB_BAR_HEIGHT`）。这是 Tauri 侧 `tabs.rs` 已有的约定，沿用。

**为什么内容视图不注入 `titlebar.js`**：这不只是「避免双标题栏」。实测链路是——

- `titlebar.js:713` 会设置 `window.__web2appChromeH = 38`；
- `loading.js:191` 读它来决定加载层顶部让出多少（`h ? h+'px' : '0px'`）；
- 内容视图在 `y=38` 起渲染，**不应该再让出 38px**。

于是「**不注入 titlebar.js**」⇒ `__web2appChromeH` 自动为 `undefined` ⇒ 加载层按 `h=0` 满铺，
**`loading.js` 一行都不用改、也不需要为 tabs 模式加分支**。这是 A/A′ 共同的白拿好处。

**同样白拿的**：单窗口模式下那套 `applyChromeFit` / `ensureScrollRoot` / `shimWindowScroll`
（顶部空间契约、滚动根探测、滚动条侵入）在内容视图里**整体不需要**——
视图自身从 `y=38` 开始，页面滚动条天然画不到标签栏上。项目里最密集的一类历史 bug 在此模式下**结构性消失**。

---

## 6. 双模式差异矩阵（必须显式记录，避免被当成 bug）

| 维度 | 单窗口模式（现状，默认） | 多页签模式（A′） |
| :--- | :--- | :--- |
| 窗口类 | `BrowserWindow` | `BrowserWindow`（**不变**） |
| 壳 / 顶部 38px | `titlebar.js` 注入**远程页面**自绘，页面让出 38px | 壳 = **窗口自身页面** `shell.html` 独占 `y∈[0,38)`；远程页面在子视图里，从 `y=38` 起 |
| `window.__web2appChromeH` | `titlebar.js` 置 38 | 内容视图不注入 ⇒ 自动 `undefined` ⇒ 加载层满铺 |
| 滚动根 / 滚动条侵入 | 需 `ensureScrollRoot` 等一整套机制 | **不存在**（滚动条在视图内；且壳页面 `overflow:hidden`） |
| 窗口标题 | `page-title-updated` 被 `preventDefault()`，固定 `cfg.title` | 同上（壳是自身页面，行为不变）；**内容视图各自的标题事件只用于刷新标签栏文本** |
| 新窗口请求 | `deny` + 当前窗口原地导航 | 壳页面仍 `deny`；内容视图 → 新建页签并落到目标 path |
| 窗口按钮 IPC | `BrowserWindow.fromWebContents(sender)` | 同一个窗口对象可拿到，但**内容视图也会命中**（§3.5-2）⇒ 必须加 `sender.id` 校验 |
| 页签标题 | — | 直接监听事件，**无需注入脚本** |
| 进程模型 | 1 渲染进程 | N 渲染进程（内存随页签数线性增长） |
| 几何持久化 | `window-state.ts` | **同一套代码**（对象类型未变） |

---

## 7. 落地清单（含已定位的陷阱）

> 全部为**新增或最小改动**，不触碰 `webview-app/` 的 Rust 代码（单一真源侧本轮零改动）。

**P0（不做则功能不可用）**

| # | 位置 | 改动 | 依据 |
| :-- | :--- | :--- | :--- |
| 1 | `src/main/tabs.ts`（新） | `TabManager`：`spawnTab` / `activateTab` / `closeTab` / `destroyAllTabs` / `relayoutTabs` / `pushTabsToShell` | 对齐 `tabs.rs` 的职责划分，但去掉几何校准与幽灵清理 |
| 2 | `src/main/index.ts` | `tabs_enabled` 真正生效：启动序列在**最外层**分叉。**建窗动作收敛到单个 `createHostWindow()`**，返回 `BrowserWindow`，A/A′ 差异只在这一处（便于回退，见 §4） | `config.ts` 现已读取该字段，仅打日志 |
| 3 | `src/main/index.ts` | **特权命令必须校验 sender**：`winOf()` 现为 `BrowserWindow.fromWebContents(e.sender)`；实测内容视图的 `webContents` **同样解析到宿主窗口**（§3.5-2）⇒ 仅靠它无法区分。窗口按钮 / 页签命令一律先判 `e.sender.id === hostWin.webContents.id`，不匹配**记日志并拒绝**（不许静默） | 本文件 `registerWindowIpc()` 现有实现；与 `MEMORY` 教训「静默失效最贵」同类 |
| 4 | `src/main/index.ts` | **新页签禁止调用 `bootstrap()`**：其开头即 `stopActiveSidecar()`，会把正在服务其它页签的代理停掉。新页签只做 `view.loadURL(target.url)`，复用模块级 `target` 缓存 | 本文件 `bootstrap()` 第 327 行 |
| 5 | `src/main/index.ts` | `shell://` 协议注册（与既有 `splash:` / `settings:` 同处），指向 `shell.html`；壳页面**复用现有 `preload.ts`** + 壳命令白名单 | 现有 `protocol.handle('settings', …)` 同型 |
| 6 | `src/main/tabs.ts` | 内容视图 bounds 一律用 **`getContentBounds()`** 推算；窗口 `resize` → 重算全部视图 bounds | §3.5-3（1px 差） |
| 7 | 各内容视图 | `setBackgroundColor(SPLASH_BG)` 顶住首帧（`WebContentsView` 默认白底）；首帧信号用 `did-finish-load` / `dom-ready`（`ready-to-show` 在视图上不触发，§3.5-5） | §2.1 官方文档 |
| 8 | `electron-builder.yml` | `extraResources` **加回 `shell.html`** | 上一轮 P1 清理时已注明「实现 tabs 时加回该行」 |

**P1（对齐既有语义 / 防回归）**

| # | 位置 | 改动 |
| :-- | :--- | :--- |
| 9 | 各内容视图 | `page-title-updated` / `setWindowOpenHandler` / `did-fail-load` / `session.on('will-download')` **逐视图**挂载；`setWindowOpenHandler` 在页签模式返回 `{ action:'deny' }` 并 `spawnTab(url)`。**实现注记（v1.5）**：`will-download` 挂在共享的 `defaultSession` 上（主窗口与所有内容视图都没设 `partition`），挂一次即覆盖全部视图——「逐视图」在共享 session 前提下等价于「挂一次全局生效」，若未来给内容视图设独立 partition 必须改回逐视图挂载；`did-fail-load` 在内容视图上记日志 + 把页签标题置「加载失败」（内容视图无加载层，失败态只能体现在标题上） |
| 10 | `tabs.ts` | 页签上限 32 熔断 + 超限拒绝（对齐 `MAX_TABS`，含一条可排查日志） |
| 11 | `tabs.ts` | 关闭最后一个页签 → 关闭宿主窗口（复用既有托盘/退出语义）；激活态落到相邻页签（优先右侧） |
| 12 | `tabs.ts` | `destroyAllTabs()`：宿主确定退出时显式 `close()` 所有视图的 `webContents`（官方明示视图不随窗口销毁自动回收；本项目未实测该路径 ⇒ 按官方结论处理，并在探针里加「引用归零」断言） |
| 13 | 壳页面 | 前进/后退改用 `webContents.navigationHistory.goBack()`——`webContents.goBack()` 在 Electron 44 已标记 **Deprecated**；历史导航作用于**当前激活的内容视图** |
| 14 | 托盘 / 快捷键 / 设置窗 / 几何 | 作用对象仍是 `BrowserWindow` ⇒ **预计零改动**；但需在页签模式下逐项跑一遍回归（`probe` 断言 + 人工确认） |
| 15 | `probe.ts` | 新增 tabs 断言组（§8），并支持 `--tabs` 以页签模式起窗 |

**P2（可选增强，需跨壳联动）**

| # | 改动 | 说明 |
| :-- | :--- | :--- |
| 16 | `shell.html` 补「最大化/还原」按钮 + 双击标签栏空白最大化 | 会动到**双壳单一真源**：Tauri 侧需同步注册对应命令，否则 Tauri 版点了无反应（静默失败）。且 Tauri 宿主现为 `resizable(false)`，能否最大化需另测。见 §11-D2 |
| 17 | 空闲页签启用 `backgroundThrottling` | 省电；注意探针模式必须保持 `false`（现有 `!PROBE` 写法），否则断言失真。**默认不改**（Tauri 侧隐藏窗口同样被系统节流，保持两壳行为一致） |
| 18 | 页签级「重新加载 / 复制链接」右键菜单 | Tauri 版没有，属新增能力，需先定需求 |
| 19 | 页签内容圆角（`setBorderRadius`） | 参考实现用它做 8px 圆角；本项目**默认不用**，避免圆角遮罩在高 DPI / 无 GPU 下引入合成问题（§3.5-8） |

---

## 8. 探针与验证清单

### 8.1 判据必须**成对**（沿用项目已有铁律）

对「侵入类」缺陷，只断言「不许侵入」会被「干脆删掉」满足；只断言「存在」会被
「存在但压在上面」满足。因此每条都要**位置 + 存在性**成对：

| # | 成对判据 | 说明 |
| :-- | :--- | :--- |
| P1 | `contentView[i].getBounds() === {x:0,y:38,w:W,h:H-38}`（`W/H` 取自 `getContentBounds()`） | 位置 + 尺寸 |
| P2 | `win.contentView.children.length === tabCount` | 存在性（不许靠「删掉视图」满足 P1） |
| P3 | **命中测试**：壳页面内 `document.elementFromPoint(5,5)` 命中标签栏元素；内容视图内 `elementFromPoint(5,5)` 命中的是页面自身元素 | 「有没有被遮住」只能用命中测试或像素证明，`getBoundingClientRect()` 不够（项目里同一个坑踩过两次） |
| P4 | 切换后 `active.getVisible() === true` ∧ 其余**全部** `=== false` | 缺后半句会被「全都可见、靠 z-order 盖住」蒙混 |
| P5 | 内容视图内 `window.scrollTo(0,200)` 后 `scrollY ≈ 200`，且**页面上边界仍在视图内**（不越过标签栏） | 顶部空间契约在页签模式下的等价断言 |
| P6 | 标题：页面 `<title>X</title>` → 标签栏文本 `=== X` | 事件链路 |
| P7 | 关闭一个页签：children 减 1 **且** 激活态落到相邻页签（不悬空、不全隐） | 位置 + 存在性 |
| P8 | 熔断：连开至上限后第 N+1 次返回失败，且 children **不再增长** | 防空转建窗 |
| P9 | 一次 `window.open()` 只新增 **1** 个页签 | 对齐 Tauri 侧「一次点击只开 1 个」的回归判据 |
| P10 | `target="_blank"` 的目标 path 被保留（未被降级成入口地址） | 对齐 Tauri `resolve_popup_url` 的回归判据 |
| P11 | **特权命令 sender 校验**：从内容视图 `webContents` 发 `close_host` 类消息必须被拒绝，且日志有一条拒绝记录 | §3.5-2 的安全硬化；只断言「壳能关窗」会被「谁都能关窗」满足 |
| P12 | 层级语义：重复 `addChildView(view)` 后 `children.length` **不增**且该视图在数组末尾；`setVisible(false)` 后重复 add，`getVisible()` 仍为 `false` | §3.5-6/7 实测语义，防止后续 Electron 版本改变行为后无人察觉 |
| P13 | 单窗口模式跑全量断言 = 112 条不变、退出码 0 | 零回归闸门 |

### 8.2 工具与命令

```bash
# 无 GPU 沙箱必须带这两个参数（历史教训：只靠 disableHardwareAcceleration 不够）
pnpm run probe --no-sandbox --in-process-gpu
pnpm run probe --tabs --no-sandbox --in-process-gpu      # 新增：页签模式断言组
pnpm run smoke --no-sandbox --in-process-gpu

# V4（子视图绘制顺序，A′ 唯一硬前提）已程序化验证 —— 见 §8.4
pnpm run tabs-v4           # 正向：V4_PASS（退出码 0）
pnpm run tabs-v4-negative  # 负向对照：验证断言有效性（NEGATIVE_CONTROL_OK）

# 打包产物自证（必须用 Start-Process -Wait 取 ExitCode；
# 直接 & 调用 GUI 子系统 exe 是「假绿」：立刻返回 0、日志 0 字节）
Start-Process -FilePath "...\Web2App.exe" -ArgumentList '--probe','--tabs' -Wait -PassThru |
  Select-Object ExitCode
```

### 8.3 目视项（程序断言不了，必须在可见窗口上人工确认）

| # | 项目 | 为什么不能自动断言 | 真机结果（2026-09-18） |
| :-- | :--- | :--- | :--- |
| V1 | 拖动标签栏空白处能移动窗口 | 拖拽由 OS 命中测试接管（`WM_NCHITTEST`），无公开 API 可读。**A′ 下走的是单窗口模式已跑通的同一条路径**（窗口自身页面），风险已大幅下降，但仍是人工项 | ✅ 正常 |
| V2 | 最大化 / 还原 / 跨 DPI 拖动后标签栏与内容不错位 | 需要在 100% / 150% / 200% 三种缩放下各看一眼（开发机 DPI=200%，只覆盖一档） | ✅ 正常（跨 DPI 目视正常） |
| V3 | 切页签无白闪 | 依赖首帧时序，属观感 | ✅ 正常 |
| V4 | **子视图确实绘制在窗口自身内容之上**（内容不被壳页面盖住） | 涉及合成顺序。对应 electron#41044（已关闭）。**2026-09-17 已程序化验证通过**：见 §8.4 `tabs-v4.cjs`——用 `desktopCapturer` 系统级抓屏 + 像素断言（红壳蓝底、绿内容视图），正反两向对照，`V4_PASS` | ✅ 2026-09-17 程序化 + 2026-09-18 真机目视，结论一致 |

> **真机验收结论（2026-09-18）**：多页签功能目视 **8/8 正常**、跨 DPI 目视**正常**、
> 单窗口模式回归 **7/8 正常**（仅「代理 / 域名解析」一项**留待后续单独测试**，
> 它属 sidecar 链路，与页签改造无交集）。**M5 的验收项至此全部关闭**。

### 8.4 V4 的程序化验证（2026-09-17 实测完成）

`scripts/tabs-v4.cjs` 把 V4 从「人眼目视」降级为「可自动断言」，结论：**V4 通过，electron#41044 未回归**。

**方法**：壳页面（窗口自身 `webContents`）填「标签栏红 + 内容区蓝」，子 `WebContentsView` 填「纯绿」、
`bounds={x:0,y:38,...}`；系统级抓屏后逐像素断言 `y=20` 是红、`y=60` 是绿、`y=60` 绝非蓝。
负向对照（`--negative`，把内容视图盖到 `y=0`）应观察到 `y=20` 变绿，证明断言真的在测合成顺序而非恒绿。

**两个关键取证发现（写进 MEMORY）**：

1. ⚠️⚠️ **`webContents.capturePage()` 看不到同窗口下的子 `WebContentsView`**。它只从「单个 guest
   webContents 的合成器 surface」拷贝（paseo 项目 `browser-capture-harness.md` 原文）。实测用 capturePage
   拿到 `y=60` 是**蓝色**（壳页面），误判成 #41044 回归；换 desktopCapturer 后 `y=60` 是**绿色**（子视图），
   真相才出来。**这是「取证方法错 ⇒ 结论错」的实例，与 §3.4-2 的教训同类**。
2. ✅ `desktopCapturer` 在无 GPU 沙箱**可用**，但 `thumbnailSize` 必须显式给非零值（`{width:0,height:0}`
   会返回空 thumbnail，PNG 解码报 `unexpected end of file`）。

> 仍留待真机的 V4 子项：`resize` 时子视图与标签栏之间「无 1px 缝」（属 §8.3 V2 的跨 DPI 目视，
> 本机只覆盖 200% 一档）。

---

## 9. 风险与对策

| # | 风险 | 影响 | 对策 |
| :-- | :--- | :--- | :--- |
| **R1** | **子视图被画在窗口自身内容之下**（electron#41044 回归，Windows） | 内容区显示成壳页面（白/灰），功能不可用 | **A′ 的唯一硬前提**。上游 issue 已关闭且参考实现在 Windows CI 跑同一形态 ⇒ 风险中低。对策：① 真机复验（§8.3 V4）置于 T0；② 回退到方案 A（仅换窗口类 + 壳承载，差异收敛在 `createHostWindow()` 一处） |
| R2 | 内容视图无 `ready-to-show`（实测确认） | 首帧可能白闪；「先最大化再显示」的现有做法对内容视图不适用 | 用 `did-finish-load` / `dom-ready` + `setBackgroundColor(SPLASH_BG)` 顶住白帧；窗口级 `ready-to-show` 仍可用（壳是窗口自身页面） |
| R3 | 视图不自动销毁 ⇒ 内存泄漏（官方明示） | 长会话内存增长 | `closeTab` 时 `webContents.close()` + **清掉 Map 引用**；`destroyAllTabs()` 兜底；探针加「关掉后引用为 0」断言 |
| R4 | 每页签独立渲染进程 | 内存线性增长 | 沿用 32 上限熔断；P2 项 17 可选开节流 |
| R5 | 双模式代码路径分叉 | 断言「看配置才知道过不过」，回归成本上升 | `tabs_enabled` **只在最外层分叉一次**，禁止在既有函数里散落 `if`；单窗口路径保持逐字节不变（A1 闸门守住） |
| R6 | 能力差异被当成 bug（如 Electron 有最大化、Tauri 没有） | 用户侧困惑 | §6 差异矩阵写进 `electron-app/README.md`；§11-D2 显式决策 |
| R7 | 沙箱内 GPU 抖动导致假失败 | 白跑一轮 | 已知：`ERR_FAILED (-2)` 偶发（前置探针即踩到，故加了重试）；断言要容错加载、但**不允许容错断言** |
| R8 | chrome 与内容一旦重叠，拖拽区判定变成并集且**上游不会修**（electron#43320） | 拖动页面内容会连带拖动窗口（交互诡异） | **架构级规避**：平铺布局，chrome 与内容**永不重叠**；壳页面只给 `.bar` / `.tabs` 设 `drag`，不给 `body` 设 |

---

## 10. 分阶段计划

| 阶段 | 内容 | 出口判据 | 量级 |
| :--- | :--- | :--- | :--- |
| **T0 前置校验** | ~~可见窗口上验证 §8.3 的 V4 + V1/V2~~ **已完成**：V4 经 `tabs-v4.cjs` 程序化验证 `V4_PASS`（§8.4）；V1/V2/V3 于 **2026-09-18 真机目视全部通过** | V4 通过 ⇒ 继续（A′ 定案，无需回退方案 A）；V1/V2/V3 已关闭 | ✅ 0.5 h |
| **T1 骨架** | `createWindow()` 分叉 + 1 个内容视图 + 壳命令 sender 校验（§7-3）；窗口按钮可用 | 按钮四个动作全部生效，且**非法 sender 被拒绝并留日志**（不许静默） | ✅ 0.4 d（2026-09-17 完成，见 §13 v1.3） |
| **T2 页签管理** | 新建 / 切换 / 关闭 / 相邻激活 / 熔断 / 标题回传 | §8 的 P2–P8、P12 全绿 | ✅ 0.6 d（2026-09-17 完成，见 §13 v1.4） |
| **T3 链路对齐** | `bootstrap` 复用（禁止新页签重启代理）、新窗口策略、下载/右键钩子逐视图挂载、托盘/快捷键/设置窗/几何回归 | P9–P11 全绿；单窗口模式 A1 仍全绿 | ✅ 0.5 d（2026-09-17 完成，见 §13 v1.5） |
| **T4 探针与出包** | 新增断言组 + `--tabs`；`extraResources` 加回 `shell.html`；打包产物自证 | A1–A4 全绿；`dist` 出 zip/portable | ✅ 0.5 d（2026-09-17 完成，见 §13 v1.5） |

合计约 **2 天**，与 `electron-plan.md` §5 对 M5 的估量一致。

> **go/no-go 卡点**：T0 的 V4（子视图绘制顺序）。它决定 A′ 还是 A，不再是「功能能否做」的卡点。

---

## 11. 决策点（需确认后再开工）

> ⚠️ **本节为开工前的历史决策记录，已被取代**：D1 最终选了**②**（M5 已按 §10 实施完毕，T0–T4 全部完成，真机目视 V1–V4 亦已关闭）。保留原文是为了留住当时的判断依据；**当前状态以 §13 变更记录与 `electron-plan.md` §5 为准**。

| # | 决策 | 选项 | 建议 |
| :-- | :--- | :--- | :--- |
| **D1** | ~~本轮是否启动 M5~~（已决：②） | ① 仅交付本方案，M5 仍暂缓；② 按 §10 实施（2 天，T0 为卡点） | ~~【默认补全】**①**~~ → 实际选 **②** |
| **D2** | 宿主窗口的缩放/最大化能力 | ① **保守**：等价 Tauri 现状（宿主不可缩放、无最大化按钮，保持 `shell.html` 不动）；② **增强**：补最大化/还原按钮 + 双击标签栏最大化 | 【默认补全】**①**。选②要动双壳单一真源 `shell.html`，并要求 Tauri 侧同步注册命令，否则 Tauri 版点了无反应（静默失败）；且 Tauri 宿主 `resizable(false)` 下能否最大化需另测。**注意**：①会让 Electron 版在多页签模式下**丢掉**单窗口模式已有的最大化能力，属能力回退，若不可接受请选② |
| **D3** | 关闭最后一个页签 | ① 关闭宿主窗口（对齐 Tauri）；② 保留一个空白页签 | 【默认补全】**①**，与 `close_tab_inner` 语义一致 |
| **D4** | 隐藏页签是否保活（节流） | ① **默认节流**（对齐 Tauri：隐藏窗口同样被系统节流）；② 隐藏但保活（`backgroundThrottling:false` 或「移出视口」写法） | 【默认补全】**①**。若业务页有长连接/SSE 需要在后台持续运行，应改选②——这属**业务假设**，需产品侧确认 |

---

## 12. 参考来源

| 主题 | 来源 |
| :--- | :--- |
| `BaseWindow`（多视图窗口、资源管理、`contentView`） | Electron 官方文档 `api/base-window` |
| `WebContentsView`（`setBounds` / `setVisible` / `setBackgroundColor`） | Electron 官方文档 `api/web-contents-view`、`api/view` |
| `BrowserView` 已废弃 → `WebContentsView` | Electron 官方博客 *Migrating from BrowserView to WebContentsView*（「deprecated since Electron 30」） |
| `Web Embeds` 选型（iframe / webview / WebContentsView 三者边界） | Electron 官方文档 `tutorial/web-embeds` |
| 多视图层级、`addChildView` 顺序即 Z 序 | Electron 官方迁移指南 + 本仓库实测（§3.5） |
| 视图重叠时拖拽区判定为并集、上游明确不修 | electron#43320（`component/draggable-regions`，30/31/32/34 复现，维护者结论） |
| Windows 子视图绘制顺序回归（已关闭） | electron#41044 |
| 无 `setTopBrowserView` 对应 API（抬层靠 remove+add） | electron#42061 |
| 原生页签 API 仅 macOS | Electron 官方文档 `api/browser-window`（`addTabbedWindow` / `new-window-for-tab`） |
| **落地参考实现**：`BrowserWindow` 当壳 + 子 `WebContentsView` | samuelmaddock/electron-browser-shell（`packages/shell/browser/main.js`、`tabs.js`；500+★，Windows CI） |
| **落地参考实现**：同上形态 | hmmhmmhm/aka-browser（`src/main/TabManager.ts`、`WindowManager.ts`） |
| 官方/教学示例：`BaseWindow` + 独立壳视图（壳无 drag 区域） | mamezou-tech/electron-example-browserview（`main.mjs` / `tabbar.html`）及其架构博客 |
| `electron-tabs` 库已停更、`<webview>` 方案被劝退 | brrd/electron-tabs README（「discontinued」）；Electron 官方文档 `api/webview-tag` 警告 |
| Tauri 单窗口多 webview 仍为不稳定特性 | tauri PR #8280（「API isn't finished yet」）、tauri#10420（子视图定位 bug）、wry#583（Windows 同步命令死锁） |
| 同类应用（Electron→Tauri 迁移）因 CSP 无法 iframe 化而放弃标签内嵌 | mamezou-tech 开发者博客 *Tried a PoC to Port an Electron App to Tauri 2.0* |
| 本项目既有约定 | `webview-app/src/tabs.rs`、`webview-app/src/shell.html`、`electron-app/src/main/index.ts`、`docs/electron-plan.md` §3.1/§5 |

---

## 13. 变更记录

| 版本 | 日期 | 变更 |
| :--- | :--- | :--- |
| v1.0 | 2026-09-17 | 首版：调研 + 本地实测 + 方案 A（`BaseWindow` + 壳视图）选型 + 落地清单 + 探针清单 + 风险与决策点 |
| v1.1 | 2026-09-17 | **社区核查后更正选型**：新增 §2.3（4 个落地/示例实现 + 4 条上游限制核查）与 §3.5（A′ 形态补充实测 8 项）；选型由 A 改为 **A′（`BrowserWindow` 当壳 + 子 `WebContentsView`）**，A 降为回退路径；作废 v1.0 §3.4-2 的推论；新增风险 R1/R8、探针 P11/P12、目视项 V4、决策点 D4；诚实记录「推理过度」这一教训 |
| v1.2 | 2026-09-17 | **T0 完成（V4 程序化验证通过）**：新增 §8.4，`scripts/tabs-v4.cjs` 用 `desktopCapturer` 系统级抓屏 + 像素断言把 V4 从「目视项」降级为「可自动断言」，正反两向对照均通过 ⇒ **A′ 定案，electron#41044 未回归**；记录取证发现「`capturePage` 看不到子视图、必须用 `desktopCapturer`（且 `thumbnailSize` 非零）」。V4 目视部分仅剩「resize 无 1px 缝」留待真机 |
| v1.3 | 2026-09-17 | **T1 骨架完成**：新增 `src/main/tabs.ts`（`TabManager`：spawn/activate/close/relayout/pushToShell/destroyAllTabs，32 上限熔断，bounds 用 `getContentBounds()`）、`src/renderer/shell-preload.ts`（壳命令白名单垫片）；`index.ts` 接入——`createWindow()` 分叉（`TABS_MODE` 单点判定）、`shell://` 协议、`registerTabsIpc`（6 条壳命令，全部 `assertShell` 校验 sender.id）、`resolveInitialTarget()`（只解析不导航，供页签复用 target 缓存且不污染壳页面）、窗口按钮 IPC 加 `assertPrivileged` 校验；`config.ts` 告警改为「已实现」日志；`electron-builder.yml` 加回 `shell.html`。验证：单窗口 `probe` 112/112 回归通过、`smoke` 通过、`tabs_enabled=true` 多页签 `smoke` 退出码 0、打包产物 `shell.html` 已进包 |
| v1.4 | 2026-09-17 | **T2 页签管理完成**：`probe.ts` 新增 `runTabsProbe`（`--tabs` 标志，§8 P1/P2/P4/P6/P7/P8/P12 断言组，**19/19 通过**）；`index.ts` 加 `TABS_PROBE` 分叉。**修复两处缺陷**：① `closeTab`/`destroyAllTabs`/`spawnTab 失败回收` 只调 `webContents.close()` 导致 `contentView.children` 不减少、关闭视图仍可见（探针 P7 两条失败暴露）——补显式 `removeChildView(view)`，并更正 §3.5-9 的「不必手动 removeChildView」结论；② `get_tabs` 契约错位——`tabs.ts` 的 `list()` 返回对象数组，但 `shell.html` 用 `list.map(([label,title,active]) => …)` 元组解构（对齐 Tauri `Vec<(String,String,bool)>`），拆分出 `listForIpc()` 返回元组数组。验证：单窗口 `probe` 112/112 零回归、多页签探针 19/19 |
| v1.5 | 2026-09-17 | **T3/T4 完成（M5 收尾）**：① **T3 链路对齐**——`probe.ts` 补 P9（一次 `window.open` 只新增 1 页签）/P10（`target="_blank"` 目标 path 保留，不被降级成入口 `/plain.html`）/P11（内容视图发特权命令 `close_host` 被 `assertShell` 的 sender.id 校验拒绝且窗口未误关）三组断言，新增探针专用 `src/renderer/tabs-probe-preload.ts`（把 IPC 通道硬塞给独立内容视图模拟「未来误给内容视图加 preload」的最坏情形）；`index.ts` 内容视图补 `did-fail-load`（记日志 + 页签标题置「加载失败」）。**澄清两处实现注记**：`will-download` 挂在共享 `defaultSession`（主窗口与内容视图都没设 partition）⇒「逐视图」等价于「挂一次全局生效」；内容视图「只注入 loading.js」退化为「什么都不注入」（安全起见不挂 preload，白闪靠 `setBackgroundColor` 顶住）。② **T4 出包**——`extraResources` 的 `shell.html` 已随包（10914B）；打包产物（dir/zip/portable）经 `Start-Process -Wait -PassThru` 取退出码，`--probe` 与 `--probe --tabs` 均 **0**。**踩坑记录**：打包 exe 直接跑退出码 9 是宿主机注入了 `ELECTRON_RUN_AS_NODE=1` 导致（清掉该变量后退出码 0）；electron-builder 打包须 `--config.electronDist=node_modules/electron/dist`（否则走「解压 cache zip」慢路径卡死）。验证：单窗口 `probe` 112/112、多页签探针 **23/23**、打包产物双模式退出码 0 |
| v1.6 | 2026-09-18 | **多页签版正式出包 + 修一处「回归门禁依赖用户当前配置」的缺陷**。① **缺陷**：`TABS_MODE = cfg.tabs_enabled \|\| TABS_PROBE` 让 `config.json` 的页签开关也能决定**探针**的起窗模式 —— `tabs_enabled=true` 的包上跑 `--probe`（112 条单窗口断言）会以页签模式起窗，单窗口断言往壳页面（`shell://`）注入脚本直接抛 `Error: Script failed to execute`、退出码 1，且报错完全不指向根因（回归门禁退化成「看用户当前配置过不过」）。**修法**：`TABS_MODE = TABS_PROBE ? true : PROBE ? false : cfg.tabs_enabled` —— 探针模式下起窗模式与断言组一一对应；非探针模式仍由 `tabs_enabled` 决定（行为不变）。**验证**：同一份包（`tabs_enabled=true`）修前 `--probe`=1、修后=0。② **出包**：`config.json` 的 `tabs_enabled` 置 **true**（多页签交付形态），`electron-builder --win zip portable --config.electronDist=node_modules/electron/dist` 产出 zip/portable；**解压目录版与便携版**上 `--probe` / `--probe --tabs` / `--smoke` 三模式退出码均 **0**。③ 新增两条流程坑：**覆盖已有 `release/` 时若 `app.asar`·`Web2App.exe` 被句柄占用会 `EBUSY`**，而**失败的 electron-builder 已先把 `win-unpacked` 删成残缺目录**（只剩 `resources/`）⇒ 出包走 `--config.directories.output=<新目录>` + copy 归位；**在产物目录里直接跑探针会留下 `data/session`（Chromium 会话缓存）污染交付物** ⇒ 探针应在 `tests/` 下的副本上跑 |
| v1.7 | 2026-09-18 | **真机验收完成（M5 验收项全部关闭）**：在有 GPU 的真机上按 `docs/electron-tabs-test-steps.md`（v1.6 清单）执行 —— **多页签功能目视 8/8 正常**（标签栏出现/内容在栏下/新建与 `window.open` 单页签/切换/关闭与相邻激活/V1 拖标签栏移窗/V3 切页签无白闪/滚动条不侵入）、**跨 DPI 目视正常**（V2）、**单窗口模式回归 7/8 正常**（仅「代理 / 域名解析」一项**留待后续单独测试**：它走 sidecar 链路，与页签改造无交集，探针 M2 断言已在源码态与打包态跑通）。§8.3 的 V1/V2/V3/V4 目视项至此**全部关闭** |
