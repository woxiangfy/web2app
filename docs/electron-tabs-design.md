# Electron 多页签（M5）设计方案

> **文档定位**：把 `docs/electron-plan.md` §5 的 M5 多页签细化为可直接施工的方案。
> **当前状态**：**已实现并完成真机验收（T0–T4 全部 ✅；V1–V4 目视项 2026-09-18 全部关闭）**。
> `tabs_enabled=true` 走多页签模式（壳页面 + 子 `WebContentsView`）；`tabs_enabled=false`（默认）
> 单窗口模式行为逐条不变。真机清单与实测结论见 `electron-tabs-test-steps.md`；
> 里程碑状态与命令以 `../electron-app/README.md` 为准。本文是**决策与架构记录**，不再保留逐日过程
> （原 §13 变更记录已精简为一行要点）。

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
| A3 | **打包产物**自带探针在两种模式下均退出码 0 | `Start-Process -Wait -PassThru` 取 `ExitCode`。⚠️ 前提：**探针的起窗模式必须由探针标志决定、不受 `tabs_enabled` 影响**（v1.6 修复）——否则 `tabs_enabled=true` 的包里 `--probe` 会以页签模式起窗而失败 |
| A4 | 标签栏与内容区**逐像素**不重叠、不留缝 | §8 的「位置 + 存在性」成对判据 |

【默认补全】平台 Win10+ x64 绿色版、不做代码签名；页签上限 32（对齐 Tauri `MAX_TABS` 熔断）；
关闭最后一个页签 = 关闭宿主窗口（对齐 Tauri `close_tab_inner`）；不做页签拖拽排序 / 标签栏溢出菜单 /
页签恢复（Tauri 版同样没有）；不新增 `config.json` 字段，上限写成代码常量而非配置项。

---

## 2. 调研结论（仅保留影响设计的结论）

- **视图 API**：`BrowserView` 自 Electron 30 废弃 → 用 `WebContentsView`（现行一等 API，
  `setBounds` / `setVisible` / `setBackgroundColor` 齐备）；`<webview>` 被官方劝退且需开 `webviewTag`
  （与 `preload.ts` 的安全基线冲突）。
- **关键差异**：`WebContentsView` 默认白底（`BrowserView` 透明）需 `setBackgroundColor` 顶首帧；
  无 `setAutoResize`，监听窗口 `resize` 后自行 `setBounds`；视图**不随窗口关闭自动销毁**，必须自己 `close()`；
  `BaseWindow` **自身没有 `webContents`**，而 `BrowserWindow` 有——这一条是选型分水岭。
- **Tauri 侧**：单窗口多 webview 仍是不稳定特性（需 `unstable` feature，官方 PR 自述「API isn't
  finished yet」，另有 wry#583 Windows 同步命令死锁）⇒ 两壳**必然不同构**，**不要移植 `tabs.rs` 形态**；
  Electron 侧「一个窗口、一个壳、N 个内容视图」实现量比 Tauri 侧少一个量级。
- **社区落地形态（2026-09-17 联网核查）**：真正落地的多页签应用（electron-browser-shell / aka-browser）
  **都用 `BrowserWindow` 当壳**（窗口自身 renderer 画标签栏）+ 子 `WebContentsView` 承载内容；
  `BaseWindow` 只见于官方/教学示例，且普遍没有可拖动的自绘 chrome。曾经的默认库 `electron-tabs` 已停更。
- **上游限制（三条都指向同一设计约束：chrome 与内容永不重叠）**：
  - electron#43320：视图**重叠**时拖拽区判定是**并集**，维护者明确上游不修 ⇒ 平铺布局天然规避；
  - electron#41044（Windows 子视图可能被画在窗口内容之下）：**已关闭**，A′ 依赖它，已真机验证未回归（§8.3 V4）；
  - macOS 原生页签 `addTabbedWindow()` **仅 macOS**，Windows 无捷径。

---

## 3. 前置实测结论（本地 Electron 44.4.1，主机 DPI=200%，均为实测非文档推断）

| # | 结论 | 对设计的影响 |
| :-- | :--- | :--- |
| 1 | `BrowserWindow.contentView` 可挂子 `WebContentsView`（`children: 0 → 1`，引用一致） | **A′ 根基成立**。v1.0 曾由「children=0」推出「必须换 `BaseWindow`」，**推理过度、已推翻**（v1.1 更正） |
| 2 | `BrowserWindow.fromWebContents()` **不能区分壳与内容**（壳与子视图的 `webContents` 都解析到宿主窗口） | 窗口按钮等特权命令必须**校验 `e.sender.id === hostWin.webContents.id`**（§7-3） |
| 3 | `getBounds()` 与 `getContentBounds()` 在 Windows 无边框窗口差 1px | 内容视图 bounds 必须用 **`getContentBounds()`** 推算，否则右/上边留缝或压 1px |
| 4 | `setBounds` / `setVisible` 均为 **DIP 坐标**、双向可读；重复 `addChildView` = 抬到最上且 children 数不增；数组顺序从下到上（末尾最上）；`View` 无 `setZIndex` | 切换首选 `setVisible`（不动 z-order）；z-order 可显式控制，**不需要**像 Tauri 那样乘 DPI + `place_tab` 三轮回环校准 |
| 5 | `ready-to-show` 在视图上**不触发**（属 `BrowserWindow` 专有；同一窗口自身页面正常触发） | 内容视图首帧信号用 `did-finish-load` / `dom-ready` + `setBackgroundColor` 顶白帧；窗口级首帧时序保留 |
| 6 | `webContents.close()` **不会自动摘除视图**（T2 探针暴露：children 不减少、关闭视图仍可见） | 必须显式 `removeChildView(view)`（对非子视图是 no-op，安全） |
| 7 | `page-title-updated` 可直接取到标题（`getTitle()` 一致） | 页签标题**不需要注入脚本**（Tauri 侧要 `tab_title.js` + IPC 回传），一条事件监听即可 |
| 8 | `setWindowOpenHandler` / `session` / `executeJavaScript` 等能力逐视图齐备 | 新窗口拦截、下载、脚本注入均为**逐视图**挂载 |

---

## 4. 架构选型（A′ 定案，v1.1 由 A 改为 A′）

| 维度 | **方案 A′：`BrowserWindow` 壳 + N 子 `WebContentsView`**（采用） | 方案 A：`BaseWindow` + 独立壳视图（v1.0 原选型） | 方案 B：N 个 `parent` 子窗口（镜像 Tauri） |
| :--- | :--- | :--- | :--- |
| 壳承载 | 窗口自身 `webContents`（`shell.html` 现状即此形态） | 独立壳视图 | 窗口自身 |
| 窗口类 | **保持 `BrowserWindow`** | 换 `BaseWindow` | 保持 |
| 拖拽标签栏 | 沿用现状已跑通的路径（窗口自身页面 `app-region: drag`） | 无社区先例（示例甚至没设 drag） | 同 A′ |
| 窗口按钮 IPC | 现有语义可用 + `sender.id` 校验（§3-2） | 必须自建「sender → 宿主」映射 | 同 A′ |
| 既有设施（几何/托盘/快捷键/标题拦截） | **零改动** | 全部需泛化为 `BaseWindow` | 零改动 |
| 渲染进程数 | N（壳复用窗口自身进程） | N+1 | N+1 |
| 几何 / z-order / 关页签 | DIP 一次到位；无遮挡问题；`close()` + `removeChildView` | 同左 | 自己算 move/resize/DPI + 闭环校准；owner + 自愈清理；`destroy()` 防重入 |
| 依赖的前提 | 子视图绘制在窗口自身内容之上（#41044，已关闭，**已真机验证**） | 壳视图内 `drag` 生效（无社区先例） | 复刻全部 Tauri 痛点 |
| 社区先例 | 两个落地实现（electron-browser-shell / aka-browser） | 仅官方/教学示例，均无自绘可拖 chrome | 少 |

**采用 A′ 的三条硬理由**：

1. **与 Tauri 侧同构**：Tauri `tabs.rs` 本就是「宿主窗口自身 webview 画标签栏 + 内容另开窗口」，
   `shell.html` 按「整页 + 顶部 38px bar」写，A′ 下 `shell.html` **一行不改**；A 要求同一页面在
   「38px 视口」里渲染，反而制造新分叉。
2. **风险落在已跑通的路径上**：拖拽标签栏、窗口按钮 IPC、几何/托盘/快捷键全部沿用单窗口模式已验证
   路径，v1.0 的 go/no-go（「视图内 drag 能否拖动窗口」）不再是卡点。
3. **成本更低**：少一个渲染进程；窗口类不变 ⇒ `window-state.ts` / `tray.ts` / `shortcuts.ts` /
   `index.ts` 的窗口级逻辑零改动；`shell-preload.ts` 白名单垫片也不是必需了。

**保留 A 作为回退路径**：若 Windows 上子视图被画在窗口内容之下（#41044 回归），回退到 A；两者 TS 层
差异**仅**「窗口类 + 壳承载方式」，建窗动作收敛到**单个 `createHostWindow()`** 便于回退（§7-2）。

否决 **方案 B**：把 `tabs.rs` 里最难的三件事（z-order、几何校准、关闭重入）原样搬到 TS 重写，而
换来的「窗口类不用换」A′ 同样成立；且 Electron 官方明确 Windows 上**子窗口不跟随父窗口移动**，
同步逻辑一个都躲不掉。否决 **方案 C**（`<webview>` / iframe）：webview 被官方劝退且要开 `webviewTag`
（给远程页面开新攻击面，与 `preload.ts` 安全基线冲突）；iframe 被站点普遍的 `frame-ancestors` CSP 拒绝。

---

## 5. 目标结构

```
BrowserWindow  (frame:false, 同现有无边框外观)
├─ win.webContents  ← 壳页面：加载 shell://localhost/（= webview-app/src/shell.html，零改动）
│    preload: renderer/preload.ts（现有）+ 壳命令白名单
│    职责：标签栏渲染 / 切换 / 关闭 / 前进后退 / 自绘窗口按钮
│    ⚠️ 硬约束 1：整页必须 overflow:hidden（shell.html 已是）——否则壳页面滚动条会从 0–38 露出来
│    ⚠️ 硬约束 2：尺寸用 getContentBounds()（不是 getBounds()，见 §3-3）
└─ contentView[i]     bounds = { x:0, y:38, width:W, height:H-38 }（从 getContentBounds 推）
     ├─ 不注入 titlebar.js，也不挂 preload（见下注记）
     ├─ 每个视图独立：page-title-updated / setWindowOpenHandler / did-fail-load / context-menu
     └─ 切换：setVisible(true/false)；抬层：remove+add
```

> **实现注记（v1.5）——「只注入 loading.js」实际退化为「什么都不注入」**：内容视图连 preload 都没挂，
> 首帧白闪改由 `WebContentsView.setBackgroundColor('#f7f8fa')` 顶住（`tabs.ts`）。理由：`loading.js`
> 的 overlay 模式依赖 preload 把它经 `webFrame.executeJavaScript` 注入**主世界**，而内容视图为安全
> 刻意不挂 preload（远程页面连 `ipcRenderer` 都碰不到）。为「加载层」单独引入第三个 preload 的成本大于
> 收益（纯白底 → 转圈动画）。**这是有意的简化**；失败态靠 `did-fail-load` → 页签标题「加载失败」兜底。
> 若后续要补「切页签无白闪 + 失败重试」的完整过渡，再评估引入轻量 `tab-preload.ts`。

**坐标契约**：`38` 同时是 `shell.html` 的 `--bar-h` 与内容视图的 `y` 起点，两者必须来自**同一个常量**
（`TAB_BAR_HEIGHT`）。这是 Tauri 侧 `tabs.rs` 已有的约定，沿用。

**为什么内容视图不注入 `titlebar.js`**：这不只是「避免双标题栏」。实测链路是——
`titlebar.js` 会设置 `window.__web2appChromeH = 38`，`loading.js` 读它决定加载层顶部让出多少；
内容视图在 `y=38` 起渲染，**不应该再让出 38px**。于是「不注入 titlebar.js」⇒ `__web2appChromeH`
自动为 `undefined` ⇒ 加载层按 `h=0` 满铺，**`loading.js` 一行都不用改**。

**同样白拿的**：单窗口模式下那套 `applyChromeFit` / `ensureScrollRoot` / `shimWindowScroll`
（顶部空间契约、滚动根探测、滚动条侵入）在内容视图里**整体不需要**——视图自身从 `y=38` 开始，
页面滚动条天然画不到标签栏上。项目里最密集的一类历史 bug 在此模式下**结构性消失**。

---

## 6. 双模式差异矩阵（必须显式记录，避免被当成 bug）

| 维度 | 单窗口模式（现状，默认） | 多页签模式（A′） |
| :--- | :--- | :--- |
| 窗口类 | `BrowserWindow` | `BrowserWindow`（**不变**） |
| 壳 / 顶部 38px | `titlebar.js` 注入**远程页面**自绘，页面让出 38px | 壳 = 窗口自身页面 `shell.html` 独占 `y∈[0,38)`；远程页面在子视图，从 `y=38` 起 |
| `window.__web2appChromeH` | titlebar.js 置 38 | 内容视图不注入 ⇒ `undefined` ⇒ 加载层满铺 |
| 滚动根 / 滚动条侵入 | 需 `ensureScrollRoot` 等一整套机制 | **不存在**（滚动条在视图内；壳页面 `overflow:hidden`） |
| 窗口标题 | `page-title-updated` 被 `preventDefault()`，固定 `cfg.title` | 壳行为不变；内容视图标题事件只用于刷新标签栏文本 |
| 新窗口请求 | `deny` + 当前窗口原地导航 | 壳页面仍 `deny`；内容视图 → 新建页签并落到目标 path |
| 窗口按钮 IPC | `BrowserWindow.fromWebContents(sender)` | 内容视图也会命中（§3-2）⇒ 必须加 `sender.id` 校验 |
| 页签标题 | — | 直接监听事件，**无需注入脚本** |
| 进程模型 | 1 渲染进程 | N 渲染进程（内存随页签数线性增长） |
| 几何持久化 | `window-state.ts` | **同一套代码**（对象类型未变） |

---

## 7. 落地清单（已全部实施）

> 全部为**新增或最小改动**，不触碰 `webview-app/` 的 Rust 代码（单一真源侧零改动）。

**P0（不做则功能不可用）**

| # | 位置 | 改动 | 依据 |
| :-- | :--- | :--- | :--- |
| 1 | `src/main/tabs.ts`（新） | `TabManager`：`spawnTab` / `activateTab` / `closeTab` / `destroyAllTabs` / `relayoutTabs` / `pushTabsToShell` | 对齐 `tabs.rs` 职责划分，但去掉几何校准与幽灵清理 |
| 2 | `src/main/index.ts` | `tabs_enabled` 真正生效：启动序列**最外层**分叉；建窗动作收敛到单个 `createHostWindow()`（A/A′ 差异只在这一处，便于回退） | `config.ts` 现仅读取该字段打日志 |
| 3 | `src/main/index.ts` | **特权命令必须校验 sender**：窗口按钮 / 页签命令一律先判 `e.sender.id === hostWin.webContents.id`，不匹配**记日志并拒绝**（不许静默） | §3-2；与 MEMORY 教训「静默失效最贵」同类 |
| 4 | `src/main/index.ts` | **新页签禁止调用 `bootstrap()`**（开头 `stopActiveSidecar()` 会停掉服务其它页签的代理）；只做 `view.loadURL(target.url)` 复用模块级 target 缓存 | `bootstrap()` 第 327 行 |
| 5 | `src/main/index.ts` | `shell://` 协议注册（与既有 `splash:` / `settings:` 同处），指向 `shell.html`；壳页面复用现有 `preload.ts` + 壳命令白名单 | 现有 `protocol.handle('settings', …)` 同型 |
| 6 | `src/main/tabs.ts` | 内容视图 bounds 一律用 **`getContentBounds()`** 推算；窗口 `resize` → 重算全部视图 | §3-3（1px 差） |
| 7 | 各内容视图 | `setBackgroundColor(SPLASH_BG)` 顶住首帧（视图默认白底）；首帧信号用 `did-finish-load` / `dom-ready`（`ready-to-show` 在视图上不触发，§3-5） | §2 官方文档 |
| 8 | `electron-builder.yml` | `extraResources` **加回 `shell.html`** | 上一轮 P1 清理时已注明「实现 tabs 时加回该行」 |

**P1（对齐既有语义 / 防回归）**

| # | 位置 | 改动 |
| :-- | :--- | :--- |
| 9 | 各内容视图 | `page-title-updated` / `setWindowOpenHandler` / `did-fail-load` 逐视图挂载；`will-download` 挂在共享 `defaultSession`（主窗口与内容视图都没设 `partition` ⇒ 挂一次全局生效；若未来设独立 partition 必须改回逐视图）；`setWindowOpenHandler` 页签模式返回 `deny` + `spawnTab(url)`；`did-fail-load` 记日志 + 页签标题置「加载失败」 |
| 10 | `tabs.ts` | 页签上限 32 熔断 + 超限拒绝（对齐 `MAX_TABS`，含一条可排查日志） |
| 11 | `tabs.ts` | 关闭最后一个页签 → 关闭宿主窗口（复用既有托盘/退出语义）；激活态落到相邻页签（优先右侧） |
| 12 | `tabs.ts` | `destroyAllTabs()`：宿主确定退出时显式 `close()` 所有视图 webContents（官方明示视图不随窗口销毁自动回收）+ 清 Map 引用；探针加「引用归零」断言 |
| 13 | 壳页面 | 前进/后退用 `webContents.navigationHistory.goBack()`（`webContents.goBack()` 在 Electron 44 已标记 Deprecated）；历史导航作用于**当前激活的内容视图** |
| 14 | 托盘 / 快捷键 / 设置窗 / 几何 | 作用对象仍是 `BrowserWindow` ⇒ **零改动**；页签模式下逐项回归（probe 断言 + 人工确认） |
| 15 | `probe.ts` | 新增 tabs 断言组（§8）+ `--tabs` 以页签模式起窗 |

**P2（可选增强，需跨壳联动，均未做）**

| # | 改动 | 说明 |
| :-- | :--- | :--- |
| 16 | `shell.html` 补最大化/还原按钮 + 双击标签栏空白最大化 | 动双壳单一真源 `shell.html`：Tauri 侧需同步注册命令，否则静默失败；且 Tauri 宿主 `resizable(false)` 下能否最大化需另测。见 §11-D2 |
| 17 | 空闲页签启用 `backgroundThrottling` | 省电；**默认不改**（Tauri 隐藏窗口同样被系统节流，保持两壳行为一致）；探针模式必须保持 `false` |
| 18 | 页签级「重新加载 / 复制链接」右键菜单 | Tauri 版没有，属新增能力，需先定需求 |
| 19 | 页签内容圆角（`setBorderRadius`） | 默认不用，避免圆角遮罩在高 DPI / 无 GPU 下引入合成问题（§3-8 旁注） |

---

## 8. 探针与验证清单

### 8.1 判据必须**成对**（位置 + 存在性，沿用项目既有铁律）

对「侵入类」缺陷，只断言「不许侵入」会被「干脆删掉」满足；只断言「存在」会被「存在但压在上面」满足。

| # | 成对判据 | 说明 |
| :-- | :--- | :--- |
| P1 | `contentView[i].getBounds() === {x:0,y:38,w:W,h:H-38}`（W/H 取自 `getContentBounds()`） | 位置 + 尺寸 |
| P2 | `win.contentView.children.length === tabCount` | 存在性（不许靠「删掉视图」满足 P1） |
| P3 | **命中测试**：壳页面内 `elementFromPoint(5,5)` 命中标签栏元素；内容视图内命中的是页面自身元素 | 被遮住只能用命中测试或像素证明，`getBoundingClientRect()` 不够（项目同一坑踩过两次） |
| P4 | 切换后 `active.getVisible() === true` ∧ 其余**全部** `=== false` | 缺后半句会被「全都可见、靠 z-order 盖住」蒙混 |
| P5 | 内容视图内 `scrollTo(0,200)` 后 `scrollY ≈ 200`，且页面上边界仍在视图内（不越过标签栏） | 顶部空间契约在页签模式下的等价断言 |
| P6 | 页面 `<title>X</title>` → 标签栏文本 `=== X` | 事件链路 |
| P7 | 关闭一个页签：children 减 1 **且**激活态落到相邻页签（不悬空、不全隐） | 位置 + 存在性 |
| P8 | 熔断：连开至上限后第 N+1 次返回失败，且 children **不再增长** | 防空转建窗 |
| P9 | 一次 `window.open()` 只新增 **1** 个页签 | 对齐 Tauri 侧「一次点击只开 1 个」回归判据 |
| P10 | `target="_blank"` 的目标 path 被保留（未被降级成入口地址） | 对齐 Tauri `resolve_popup_url` 回归判据 |
| P11 | **特权命令 sender 校验**：从内容视图发 `close_host` 类消息必须被拒绝，且日志有拒绝记录 | §3-2 安全硬化；只断言「壳能关窗」会被「谁都能关窗」满足 |
| P12 | 重复 `addChildView(view)` 后 children **不增**且该视图在数组末尾；`setVisible(false)` 后重复 add 仍为 `false` | §3-4 实测语义，防后续 Electron 升级改变行为后无人察觉 |
| P13 | 单窗口模式跑全量断言 = 112 条不变、退出码 0 | 零回归闸门 |

### 8.2 工具与命令

```bash
# 无 GPU 沙箱必须带这两个参数（历史教训：只靠 disableHardwareAcceleration 不够）
pnpm run probe --no-sandbox --in-process-gpu                 # 单窗口 112 条
pnpm run probe --tabs --no-sandbox --in-process-gpu          # 页签断言组 23 条
pnpm run tabs-v4 / tabs-v4-negative                          # V4 绘制顺序 正/负向对照（退出码即结论）

# 打包产物自证（必须 Start-Process -Wait 取 ExitCode；直接 & 调用 GUI exe 是假绿）
Start-Process -FilePath "...\Web2App.exe" -ArgumentList '--probe','--tabs' -Wait -PassThru |
  Select-Object ExitCode
```

### 8.3 目视项（程序断言不了，真机人工确认；2026-09-18 全部关闭）

| # | 项目 | 为什么不能自动断言 | 真机结果 |
| :-- | :--- | :--- | :--- |
| V1 | 拖动标签栏空白处能移动窗口 | 拖拽由 OS 命中测试接管（`WM_NCHITTEST`），无公开 API 可读 | ✅ 正常（A′ 下走单窗口已跑通的同一条路径，风险已大幅下降） |
| V2 | 最大化 / 还原 / 跨 DPI 拖动后标签栏与内容不错位 | 需 100% / 150% / 200% 三种缩放各看一眼（开发机 DPI=200% 只覆盖一档） | ✅ 正常 |
| V3 | 切页签无白闪 | 依赖首帧时序，属观感 | ✅ 正常 |
| V4 | **子视图绘制在窗口自身内容之上**（内容不被壳页面盖住） | 涉及合成顺序（electron#41044，已关闭） | ✅ 2026-09-17 程序化 + 09-18 真机目视，结论一致 |

> **真机验收结论（2026-09-18）**：多页签功能目视 **8/8 正常**、跨 DPI 目视**正常**、
> 单窗口模式回归 **7/8 正常**（仅「代理 / 域名解析」一项**留待后续单独测试**——它属 sidecar 链路，
> 与页签改造无交集）。**M5 的验收项至此全部关闭**。仍留待真机的 V4 子项：`resize` 时子视图与标签栏之间
> 「无 1px 缝」（属 V2 的跨 DPI 目视，本机只覆盖 200% 一档）。

### 8.4 V4 的程序化验证（2026-09-17，`scripts/tabs-v4.cjs`）

壳页面（窗口自身 `webContents`）填「标签栏红 + 内容区蓝」，子 `WebContentsView` 填「纯绿」、
`bounds={x:0,y:38,...}`；`desktopCapturer` 系统级抓屏后逐像素断言 `y=20` 是红、`y=60` 是绿且绝非蓝。
负向对照（`--negative`，把内容视图盖到 `y=0`）应观察到 `y=20` 变绿，证明断言真的在测合成顺序而非恒绿。

**两个关键取证发现（写进 MEMORY）**：

1. ⚠️⚠️ **`webContents.capturePage()` 看不到同窗口下的子 `WebContentsView`**（只从单个 guest webContents
   的合成器 surface 拷贝）。实测 capturePage 拿到 `y=60` 是**蓝**（壳页面），误判成 #41044 回归；
   换 desktopCapturer 后 `y=60` 是**绿**（子视图），真相才出来。**取证方法错 ⇒ 结论错**。
2. `desktopCapturer` 在无 GPU 沙箱**可用**，但 `thumbnailSize` 必须显式给非零值（`{width:0,height:0}`
   会返回空 thumbnail，PNG 解码报 `unexpected end of file`）。

---

## 9. 风险与对策

| # | 风险 | 对策 |
| :-- | :--- | :--- |
| R1 | 子视图被画在窗口自身内容之下（#41044 回归，Windows） | **A′ 唯一硬前提**。上游 issue 已关闭且参考实现在 Windows CI 跑同一形态；已真机验证未回归（§8.3 V4）。若回归回退到方案 A（差异收敛在 `createHostWindow()` 一处） |
| R2 | 内容视图无 `ready-to-show`（实测确认） | `did-finish-load` / `dom-ready` + `setBackgroundColor(SPLASH_BG)` 顶住白帧；窗口级 `ready-to-show` 仍可用（壳是窗口自身页面） |
| R3 | 视图不自动销毁 ⇒ 内存泄漏（官方明示） | `closeTab` 时 `webContents.close()` + 清 Map 引用；`destroyAllTabs()` 兜底；探针加「关掉后引用为 0」断言 |
| R4 | 每页签独立渲染进程，内存线性增长 | 32 上限熔断；P2-17 可选开节流 |
| R5 | 双模式代码路径分叉，回归成本上升 | `tabs_enabled` **只在最外层分叉一次**，禁止在既有函数里散落 `if`；单窗口路径保持逐字节不变（A1 闸门守住） |
| R6 | 能力差异被当成 bug（如 Electron 有最大化、Tauri 没有） | §6 差异矩阵写进 `electron-app/README.md`；§11-D2 显式决策 |
| R7 | 沙箱内 GPU 抖动导致假失败 | 断言容错加载、但**不允许容错断言**（前置探针已加重试） |
| R8 | chrome 与内容一旦重叠，拖拽区判定变并集且上游不修（#43320） | **架构级规避**：平铺布局，chrome 与内容**永不重叠**；壳页面只给 `.bar` / `.tabs` 设 drag，不给 `body` 设 |

---

## 10. 分阶段计划（T0–T4 全部完成）

| 阶段 | 内容 | 出口判据 | 状态 |
| :--- | :--- | :--- | :--- |
| T0 前置校验 | V4 + V1/V2 | V4 程序化验证 `V4_PASS`（A′ 定案，无需回退方案 A）；V1/V2/V3 真机目视通过 | ✅ 2026-09-17/18 |
| T1 骨架 | `createWindow()` 分叉 + 1 个内容视图 + 壳命令 sender 校验（§7-3）；窗口按钮可用 | 按钮四动作生效，且非法 sender 被拒绝并留日志（不许静默） | ✅ 2026-09-17 |
| T2 页签管理 | 新建 / 切换 / 关闭 / 相邻激活 / 熔断 / 标题回传 | §8 的 P2–P8、P12 全绿 | ✅ 2026-09-17 |
| T3 链路对齐 | bootstrap 复用（禁止新页签重启代理）、新窗口策略、下载/右键逐视图挂载、托盘/快捷键/设置窗/几何回归 | P9–P11 全绿；单窗口模式 A1 仍全绿 | ✅ 2026-09-17 |
| T4 探针与出包 | 新增断言组 + `--tabs`；`extraResources` 加回 `shell.html`；打包产物自证 | A1–A4 全绿；`dist` 出 zip/portable | ✅ 2026-09-17 |

合计约 2 天，与 `electron-plan.md` §5 对 M5 的估量一致。
**go/no-go 卡点**是 T0 的 V4（子视图绘制顺序）——它决定 A′ 还是 A，不再是「功能能否做」的卡点。

---

## 11. 决策点（历史记录，已被实施结果取代）

| # | 决策 | 最终选择 |
| :-- | :--- | :--- |
| D1 | 本轮是否启动 M5 | ② 按 §10 实施（已全部完成，T0–T4 ✅、V1–V4 关闭） |
| D2 | 宿主窗口的缩放/最大化能力 | ① 保守：等价 Tauri 现状（宿主不可缩放、无最大化按钮，保持 `shell.html` 不动）。**注意**：①会让多页签模式**丢掉**单窗口模式已有的最大化能力，属能力回退；如需可做 P2-16（要动双壳单一真源，Tauri 侧需同步注册命令） |
| D3 | 关闭最后一个页签 | ① 关闭宿主窗口（对齐 Tauri `close_tab_inner`） |
| D4 | 隐藏页签是否保活（节流） | ① 默认节流（对齐 Tauri：隐藏窗口同样被系统节流）。若业务页有长连接/SSE 需后台运行应改②——属业务假设，需产品侧确认 |

## 12. 参考来源

（外部链接清单已删除——结论已固化进正文与代码注释。关键上游 issue：electron#41044（已关闭）、
#42061、#43320，均已在本文件正文引用。）

## 13. 变更记录

（v1.0–v1.7 逐日过程已精简删除。要点：v1.1 社区核查后选型 A→A′（推翻 BaseWindow 结论，诚实记录
「推理过度」教训）；v1.2 T0 完成、V4 程序化验证通过；v1.3–v1.5 T1–T4 完成、探针单窗口 112/112 +
页签 23/23；v1.6 修复「探针起窗模式受 `tabs_enabled` 影响」缺陷 + 正式出包（config `tabs_enabled=true`）；
v1.7 2026-09-18 真机验收 V1–V4 全部关闭、多页签 8/8 正常。）
