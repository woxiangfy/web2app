# electron-app 架构与组织结构审计（2026-11-07）

> 本次为**重新审视当前代码**，不复用 2026-10-24 审查的结论。10-24 之后仓库发生过多次变更
> （悬浮球改为 canvas 超采样 alpha 位图、`floaticon.html` 转为 Electron 专属、若干跨壳共用件调整），
> 上一轮的部分结论已经失效，本文所有判断均以当时代码为准。
>
> - 审计对象：`electron-app/`（`src/main` 35 个 TS、`src/renderer` 5 个 TS、`test/` 守卫、`scripts/` 打包脚本）
> - 交叉对象：`webview-app/` 的共用注入脚本 / 页面 / 图标 / sidecar 产物
> - 方法：相对 import 全量解析建图（含 type-only 边）、逐文件阅读、跨壳字面量对照、探针/单测覆盖盘点
> - 约束：只读审计，未改动任何代码

---

## 一、总体结论

### 1.1 健康面

- **依赖方向单向、无环**：main 层 77 条同层依赖边、19 条 type-only 边，DFS 全量枚举 **0 条循环依赖**；
  renderer 层 5 个文件层内 0 边、与 main 层 **0 条跨层 import**（页面/脚本靠字符串路径加载）。
- **状态宿主收口较好**：`settings.ts`（入度 8）是唯一持久化文档宿主，`updateDocument` 读-改-写已成为
  设置 / 窗口几何 / 悬浮球几何三个写入方的唯一入口。
- **纯规则下沉有成效**：`tab-order.ts`、`float-layout.ts`、`page-behavior.ts`、`hotkey.ts` 把首页判定、
  吸附几何、右键菜单/下载规则、快捷键文案从 Electron 运行时剥了出来，具备单测条件。
- **跨壳复用主体干净**：`titlebar.js` / `loading.js` / `shell.html` / `splash.html` / `settings.html` /
  `icons` / sidecar 产物均为「Electron 读 webview-app 原件」的单一真源，没有本地第二份拷贝。

### 1.2 主要风险

风险不集中在「模块划得太碎」或「依赖乱飞」，而是集中在三处：

1. **静默失败面**：打包护栏清单滞后、设置兜底页会清空快捷键、IPC 闸门在单窗口模式放行；
2. **入口承载过多**：`index.ts` 954 行、0 导出，同时承担启动装配、窗口创建、悬浮球业务与探针分叉；
   `probe`（约 2300 行）被静态 import 进生产入口并随 `out/**/*` 进 asar；
3. **跨壳契约靠人记**：`isBuiltinUrl`、页签标题 trim、快捷键合法性、`TAB_BAR_HEIGHT` 等同一语义两处实现，
   只有单侧测试，改动一侧不会让另一侧变红。

### 1.3 规模与结构指标

| 指标 | 数值 | 说明 |
|---|---|---|
| main + renderer 源文件 | 40 个 | main 35 / renderer 5 |
| 总行数 / 代码行 | 9882 / 6076 | 注释占 33.8% |
| 依赖环 | 0 | 含 type-only 边一起枚举 |
| 枢纽文件（入度 ≥5） | `settings`(8)、`hotkey`(6)、`tabs`(5)、`sidecar`(5) | 均为合理的基础能力 |
| 入口出度 | `index.ts` = 26 | 装配根，但也是 954 行巨型文件 |
| >80 行函数（非探针） | 6 个 | 见 M-6、M-8 |
| 探针代码占 main 比例 | 约 26% | 且静态进入生产入口 |

---

## 二、严重程度定义

- **高**：会导致「出错也静默」、用户数据丢失或安全边界失效；或结构上阻碍后续改动且当前已产生实际成本。
- **中**：存在明确的漂移/回归风险，或职责边界模糊导致维护成本上升，但当前未表现为线上缺陷。
- **低**：命名、导出面、注释与文档准确性问题，可随批次顺带处理。

---

## 三、高优先级问题

### H-1 打包护栏清单滞后：悬浮球页与球图标不进 asar 校验

- **位置**：`electron-app/scripts/check-asar-config.cjs:30`（`REQUIRED_INJECT` 仅 5 项）、`:98`（注释仍写「5 个」）；
  对照 `electron-app/scripts/stage-assets.cjs:123-132`（实际铺 7 项：新增 `floaticon.html`、`icon.png`）。
- **本质**：stage 的资源清单与 asar 校验清单是两份手写数组，新增悬浮球页/图标时只改了 stage。
- **影响**：若 `electron-builder.yml:35` 的 `files: out/**/*` 规则或构建流程变化导致球页/球图标漏进包，
  构建与 CI 全绿，真机上 `floaticon://` 404、球面空白、托盘图标缺失 —— 正是本项目最忌的「静默退化」。
  当前节点只是「清单没覆盖」，尚未缺件，因此属于**已存在的护栏缺口**而非当前故障。
- **建议**：`stage-assets.cjs` 导出 `STAGED_INJECT` 常量（含源路径/目标名），`check-asar-config.cjs`
  直接引用它生成 `REQUIRED_INJECT`；数量打印改为动态，不再写死「5 项」。
- **工作量**：2 个脚本、约 30 行；无需改业务代码。

### H-2 设置兜底页保存时恒发空快捷键，会清掉用户的唤醒键

- **位置**：`electron-app/src/main/settings-window.ts:76-104`（`FALLBACK_HTML`），关键行 `:98`
  `newSettings:{wake_hotkey:{ctrl:false,alt:false,shift:false,key:''}, ...}`。
- **本质**：兜底页只渲染三个开关，保存时却提交了**整份** `Settings`，其中 `wake_hotkey` 是硬编码空值。
- **影响**：当 `settings.html` 资源缺失（打包漏件、开发路径异常）而进入兜底页时，用户只改托盘开关点保存，
  已配置的唤醒快捷键会被静默清空并落盘；用户不会看到任何提示。
- **建议**：兜底页保存前先 `invoke('get_settings')` 取当前值，仅覆盖三个开关、保留 `wake_hotkey`；
  或在主进程 `normalizeIncoming` 中把「字段缺失」解释为「保留旧值」而不是 `false`/空串（与 H-3 一并处理）。
- **工作量**：1 个文件约 20 行；补 1 条设置归一化单测。

### H-3 IPC 闸门不对称：单窗口模式直接放行，bootstrap 无 sender 校验

- **位置**：
  - `electron-app/src/main/app-ipc.ts:64-74` `assertPrivileged` 开头 `if (!deps.tabsMode) return`；
  - `electron-app/src/main/app-ipc.ts:192-200` `web2app:bootstrap` 的 `ipcMain.on` 无任何 sender/帧校验；
  - 四份同构闸门：`app-ipc.ts:64`、`app-ipc.ts:111`、`settings-window.ts:150`、`float-window.ts:214`。
- **本质**：多页签模式为了区分「壳页」与「内容视图」加了 sender 校验；单窗口模式认为「只有一个窗口」而跳过。
  但安全边界应按「sender 是否可信」定义，而不是按「是否多页签」定义。
- **影响**：
  - 单窗口模式下 `win:minimize` / `win:close` / `win:open-settings` 对任何持有 `ipcRenderer` 的 WebContents 可用；
    若将来误把带 invoke 的 preload 挂到内容视图或子框架（`tabs-probe-preload.ts` 已证明这种 preload 存在），
    远程页面即可关窗/开设置，且无日志、无拒绝。
  - `web2app:bootstrap` 会把约 80KB 注入源码与 `extra` 同步交给任何调用方，属于情报面。
- **建议**：新增 `ipc-guard.ts`，提供 `makeSenderGate(getExpectedId, label)`；单窗口模式把 expected 固定为主窗
  `webContents.id`，多页签为壳 `webContents.id`，两种模式都校验并统一记日志。`web2app:bootstrap` 增加
  「sender 必须是主窗顶层帧」判定（`senderFrame === webContents.mainFrame`）。
- **工作量**：4 个文件、约 120 行；探针 150 条断言不新增不改写，只验证原本通过的命令仍通过。

### H-4 探针静态进入生产入口与发布包，入口文件过载

- **位置**：
  - `electron-app/src/main/index.ts:26` 静态 `import { runProbe, runTabsProbe, startProbeServer } from './probe'`；
    `:68` 静态 `import { runSmokeChecks } from './smoke'`；
  - 探针分支侵入业务路径：`:404`（`backgroundThrottling: !PROBE`）、`:453`（`did-finish-load` 内 `if (PROBE) return`）、
    `:692-702`（直接改写 `cfg.app_url/extra/proxy_enabled`）、`:726`、`:778`；
  - `index.ts` 954 行、0 导出；`createWindow` 373-505（133 行）、装配段 570-950（约 380 行）。
- **本质**：探针是「一次性验证工具」，却以生产代码形态编译、import 并（`electron-builder.yml:35`）随 asar 分发；
  入口文件同时是组合根、窗口工厂和悬浮球业务宿主。
- **影响**：
  - 发布包多带 `probe-single`(1360) + `probe-tabs`(553) + `probe-server`(367) + 夹具，约 2300 行（占 main 26%），
    增加体积与可读攻击面（虽然不运行时不会监听端口）；
  - 业务函数里散落 `if (PROBE)` 分叉，探针改动可能直接改变生产路径，单测无法导入入口验证；
  - 任何入口级改动都要同时照顾三种模式，回归成本高。
- **建议**：
  1. 探针改为**条件加载**：`index.ts` 只在 `PROBE` 为真时 `await import('./probe')`；类型用 `import type` 保留。
  2. 打包时剔除探针：`tsconfig` 分生产/探针两份，或构建后从 `out/` 删除 `probe*.js` 与
     `renderer/tabs-probe-preload.js`（并加 asar 护栏反向断言「不得含 probe 文件」）。
  3. `index.ts` 下沉：`bootstrap.ts`（570-950 的装配）、`main-window.ts`（373-505 的窗口创建 + overrides）；
     入口只保留顺序约束与命令行开关。
- **工作量**：批次级，移动约 500 行、涉及 4-6 个文件；必须保持探针 150 条 / 页签 47 条断言逐字不变。

---

## 四、中优先级问题

### M-1 设置入参归一化与 `parseSettings` 两套语义

- **位置**：`settings.ts:133-141`（`toBool` 宽容接受 `"true"/"1"/"yes"/"on"`）、`:155-164`（`parseSettings` 缺字段取默认）；
  `settings-window.ts:204-219`（`bool = v === true`，缺字段一律 `false`，`key` 才 trim）。
- **本质**：同一份 `Settings` 形状，落盘解析用「宽容 + 默认值」，设置页入参用「严格 + 假值兜底」。
- **影响**：两套语义当前结果碰巧一致（默认值全为 `false`/空串，见 `settings.ts:79-84`），但新增字段若默认 `true`
  或页面回传字符串布尔，设置窗会把它静默写成 `false`；这是 latent contract drift，不是当前可复现故障。
- **建议**：`normalizeIncoming` 改为复用 `parseSettings`，并显式传入「当前设置」作为缺字段回退基准；
  页面侧仍只提交编辑过的字段更稳妥（与 H-2 同批）。
- **工作量**：1 个文件约 30 行；补 3-4 条单测（缺字段保留 / 字符串布尔 / 非法类型）。

### M-2 跨壳同一判据两处实现，只有单侧守卫

- **位置与差异**：

| 语义 | Tauri / 共用件 | Electron | 差异 |
|---|---|---|---|
| 内置页判定 | `webview-app/src/titlebar.js:885-890` 正则 | `page-behavior.ts:145-147` 逐字同正则 | 两份拷贝，单侧改动静默漂移 |
| 页签标题 | `tab_title.js:69-72` `trim()` 且空标题不发送 | `tab-spawn.ts:51-52` 原样传；`tabs.ts:342-347` 原样存 | 页面清空 `<title>` 时 Electron 标签变空白，Tauri 保留旧标题 |
| 快捷键合法性 | `settings.html:421-435`（页面）+ `settings.rs:324-326`（拒绝 Shift+F1） | 仅 `settings.html` 前端校验；`hotkey.ts:88-100` 不校验 | 手改 `settings.json` 可写入裸 `F9`，Electron 主进程不拦，会占满该键 |
| 标签栏高度 | `shell.html:29` + `tabs/state.rs:25` | `tabs.ts:31` | 三处 38，无跨仓对照 |

- **影响**：双壳「同一份页面/同一语义」的前提被破坏；用户在两壳看到不同行为，而 CI 因为只测单侧全绿。
- **建议**：
  1. 在 `electron-app/test/` 新增「跨壳字面量对照」守卫：同时读 `../webview-app/src/titlebar.js` 与本地实现，
     断言正则/常量逐字相同（现有 float-contract 已有读 webview-app 文件的先例）。
  2. Electron `page-title-updated` 回调统一 `title.trim()`，空串时保留旧标题（对齐 `tab_title.js`）。
  3. 把 `hkProblem` 的判据抽成 `hotkey.ts` 纯函数（裸单键 / 系统键 / Shift+F1），设置窗保存前调用；
     与 Tauri 主进程对齐。页面仍保留即时提示，形成前后两道。
- **工作量**：新增 1 个守卫文件约 120 行 + 3 处小改（合计约 40 行）。

### M-3 路径定位两套纪律并存

- **位置**：手写 `app.isPackaged` 三元：`index.ts:135`（`injectDir`）、`:145`（`floatIconPath`）、`:175`（`configRoot`）、
  `:511`、`:516`（两个 preload 路径）；走 `first-existing.ts:17` 的：`settings-window.ts:57`、`tray.ts:30`、`sidecar.ts:97`。
- **本质**：同一件事（开发态读仓库 / 打包态读 asar）有两种实现风格，候选顺序各写各的。
- **影响**：新增资源时容易只改一处；`settings.html` 已有三个候选路径（asar 副本 / `webview-app/dist` / `electron-app/dist`），
  未来再增资源会继续复制三元表达式。属于维护成本，不是行为缺陷。
- **建议**：新增 `paths.ts`，导出 `devOrPackaged(candidates)` / `injectDir()` / `iconPaths()`，全部收口；
  保留 `first-existing` 作为底层实现。
- **工作量**：4-5 个文件、约 60 行机械替换。

### M-4 错误文本与内部 URL 判定重复

- **位置**：`index.ts:437` 与 `tab-spawn.ts:59` 逐字同一模板 `` `无法打开 ${url}（${desc}，${code}）` ``；
  `index.ts:452` 只判 `startsWith('splash:')`，而 `page-behavior.ts:145` 判 `splash:` + `https://splash.localhost` 两形态
  （后者注释正是为了防这种形态差异）；`about:` 判定在 `index.ts:343` 与 `tab-spawn.ts:40` 各一份。
- **影响**：单窗口模式对 `https://splash.localhost` 形态的内置页会误判；错误文案改动需改两处。
- **建议**：`failText.ts` 导出 `navFailText(url, desc, code)`；`page-behavior.ts::isBuiltinUrl` 升级为
  `isInternalUrl` 覆盖 `splash:` / `about:` / `chrome:`，单窗口与页签统一引用。
- **工作量**：3 个文件、约 40 行。

### M-5 纯逻辑与 Electron 运行时绑定，单测盲区明显

- **位置**：
  - `window-state.ts:21` 顶层 `import { screen } from 'electron'`，导致 `parseWindowState` / `planRestore`（`:128`）
    这些纯判定无法在无 Electron 的单测里覆盖；
  - `navigation.ts`（230 行状态机）、`sidecar.ts::locateSidecar/buildSidecarArgs`、`settings-window.ts::normalizeIncoming`、
    `hotkey.ts`、全部 renderer 均无单测（`float-contract` 只做源码文本守卫）。
- **影响**：最易回归的启动顺序、几何恢复、sidecar 参数拼装只能靠 150 条运行期探针兜底，单测是 0。
- **建议**：把 `planRestore(st, displays)` 改为接收显示器矩形数组的纯函数（`screen` 只留在 `capture/…` 适配层）；
  `sidecar` 参数拼装、`normalizeIncoming` 抽纯函数；`navigation` 用注入的 `resolveTarget/loadURL` 替身测状态迁移。
- **工作量**：3 个文件重构约 80 行 + 4 个测试文件约 200 行。

### M-6 探针文件结构：单函数 514 行、4 个段函数 >80 行

- **位置**：`probe-tabs.ts:40-553` 单个 `runTabsProbe`（514 行、18 段 54 断言）；
  `probe-single.ts` 中 sec01 132-253（122 行）、sec10 425-551（127）、sec11 552-818（267）、sec13 854-1189（336）；
  跨段「共享状态」`s/fwd/side`（`:79-88`）实为伪共享：每段自己重新赋值，编排层从未读取。
- **影响**：探针是最主要的回归门禁，却最难定位失败段；伪共享状态让读者误以为存在隐式顺序依赖。
- **建议**：`probe-single/` 子目录按段拆文件（`sec00-splash.ts` …），编排层只留顺序与汇总；
  `probe-tabs` 按 P 段函数化；删除 `s/fwd/side` 参数，改为段函数返回快照。
- **工作量**：机械移动约 1900 行；风险在「断言行必须逐字不变」，需用行多重集比对验收。

### M-7 悬浮球模块级可变状态未完全可重置

- **位置**：`float-window.ts:109-126` 六个模块级变量（`deps/floatWin/dock/moving/press/anchor`）；
  `registerFloatWindow:156-157` 赋值 `deps` 后无清除路径；`destroyFloatWindow:252` 只清 `floatWin/dock`；
  `resetDrag:146-148` 清 `moving/press/anchor`；`:655` 局部 `const anchor = 'r'/'l'` 与 `:126` 的坐标对象 `anchor` 同名遮蔽。
- **影响**：测试或重注册场景会留下上一次的 `deps`；同名遮蔽让 grep 定位失真。当前生产只注册一次，未爆发现象。
- **建议**：`deps` 在 `destroyFloatWindow` 中一并置 `null`（或收进一个 `FloatState` 对象统一重置）；
  局部 `anchor` 改名 `side`。不改 IPC / 窗口生命周期。
- **工作量**：1 个文件约 15 行；`float-contract` 的 anchor 字面量守卫需按「两半」手法同步（判据仍钉在 float-window）。

### M-8 入口/控制器里的重复业务判据与多源状态

- **位置**：`index.ts:610-625` `hideFloatFromIcon` 自己实现「改设置 → 落盘 → 拉齐设置窗 → 销毁球 → 唤回主窗」；
  `index.ts:752-761` 悬浮球单击又实现一遍「可见且非最小化才隐藏」，与 `tray.ts:120-131` 重复；
  `settings-controller.ts:118-131` 在关闭托盘分支无条件 `trayExistedBefore = true`，即使托盘从未建成也会在失败回滚时重建。
- **影响**：托盘、悬浮球、设置窗三处的显隐规则出现多份；回滚会创建「本不该存在」的托盘。
- **建议**：抽 `planToggle(visible, minimized)` 与 `planFloatClick(...)` 纯函数，托盘/球共用；
  `trayExistedBefore` 改为实测 `hasTray()`，`floatExistedBefore` 改为 `isFloatAlive()`。
- **工作量**：3 个文件约 50 行 + 4 条纯函数单测。

### M-9 preload 职责过载与垫片重复

- **位置**：`preload.ts:78-104`（桥）、`:106-150`（注入执行）、`:152-174`（overlay 指令）；
  overlay 的 `fail/slow/hide` 语义与 `loading.js` 的状态机在 `:66-71`、`:164-172` 两处重复描述；
  `preload.ts:108-114` 的 `isInternalPage` 未包含 `settings:` / `shell:` / `floaticon:`；
  `shell-preload.ts:35-42` 与 `settings-preload.ts:31-38` 的 invoke 垫片仅前缀不同；
  `tabs-probe-preload.ts` 随 `out/renderer` 进入发布包。
- **影响**：新增内部 scheme 时若误挂主 preload 会往特权页注入标题栏；探针 preload 进包属多余攻击面。
- **建议**：抽 `tauri-shim.ts`（`makeInvokeShim(prefix, allowed)`）；`preload.ts` 按「桥 / 注入 / overlay」拆三块；
  `isInternalPage` 补齐内部 scheme 白名单；构建时剔除探针 preload（与 H-4 同批）。
- **工作量**：5 个文件约 120 行；探针与单测需回归。

### M-10 `protocols.ts` 注册、资源、兜底与副作用混在一处

- **位置**：`protocols.ts:44-104` 同时做 scheme 注册、注入资源读取、`FALLBACK_HTML` 兜底、
  以及 `/retry` 清状态并触发导航（`:45-56`）；`:59`、`:68` 忽略 pathname，任意路径都返回特权页。
- **本质**：协议处理器是「资源服务 + 控制指令」的混合体，`/retry` 实际上是原始应用命令。
- **影响**：路径穿越已挡住（只读固定文件名），但 `/retry` 未校验方法与来源，任意 GET 都能清掉启动失败状态并重试，
  语义上越权；新增 scheme 时容易继续把业务逻辑写进 handler。
- **建议**：每个 scheme 一个 handler 工厂；`/retry` 限定顶层 GET（校验 `Referer`/`method`）；
  HTML 兜底继续走 `ProtocolDeps.fallbackHtml` 注入。
- **工作量**：1 个文件约 60 行 + 协议探针断言回归。

---

## 五、低优先级问题

### L-1 同名函数 `persistGeometry`

- **位置**：`index.ts:368`（主窗口几何）与 `float-window.ts:673`（悬浮球几何）。
- **建议**：后者改名 `persistFloatGeometry`（导出名已在 index 使用，改动面小）。
- **工作量**：2 个文件、约 5 行。

### L-2 导出面偏大

- **位置**：`float-layout.ts` 30 个导出、`page-behavior.ts` 18 个、`config.ts` 17 个（其中约 10 个仅单测使用）。
- **影响**：读者难以判断公共 API；测试专用导出扩大耦合面。
- **建议**：把仅测试使用的导出标 `/** @internal */` 或在测试侧改为间接覆盖；不需要立即瘦身。
- **工作量**：注释级，约 20 行。

### L-3 无窗口时静默 resolve 与抛错不一致

- **位置**：`app-ipc.ts:78-93`：`minimize` / `close` 在找不到窗口时 `?.` 静默返回，`toggle-maximize`（`:82-88`）却抛错；
  `:88` `maximize()` 后立即返回 `isMaximized()`，窗口状态可能尚未落定。
- **影响**：按钮点击无声失败（违背 `:76-77` 注释的承诺）；极大化返回值偶发旧值。
- **建议**：三处统一抛 `找不到窗口`；返回值改为 `async` + `setImmediate` 后读，或返回 `void` 由页面自行查询。
- **工作量**：1 个文件约 15 行。

### L-4 未使用的页面暴露

- **位置**：`settings-preload.ts:41`、`shell-preload.ts:45` 暴露 `web2appShell`，生产代码无消费方。
- **建议**：确认无外部依赖后删除，或注明保留原因。
- **工作量**：2 个文件约 4 行。

### L-5 文档与代码的「共用件」表述已经失真

- **位置**：`webview-app/AGENTS.md:12` 仍把 `tab_title.js` / `tab_keys.js` / `dnd_polyfill.js` / `floaticon.html`
  列入「双壳单一真源」；根 `AGENTS.md` §3.4.1 已声明 `floaticon.html` 为 Electron 专属例外。
  `floaticon.html:286-330` 仍保留整套 Tauri `tauriEmit` 分支（Tauri v1.81 起不再加载该页）。
- **影响**：按文档执行「改共用件必须双壳验证」会把只需单壳验证的改动扩大化；反之按 AGENTS 把 floaticon 当共用件，
  会让 Tauri 侧误以为仍依赖它。文档错误会直接误导门禁。
- **建议**：修正 `webview-app/AGENTS.md` 的共用件清单与例外；`floaticon.html` 的 Tauri 分支标注为历史死代码
  （是否删除由 Tauri 侧决定，Electron 不单独删共享文件）。
- **工作量**：2 处文档 + 1 处注释，约 15 行。

---

## 六、建议的优化批次顺序

> 排序原则：先堵「静默失败/安全边界」，再收敛跨壳契约，最后做结构性重构与测试补强。
> 每批都必须跑既有门禁：`pnpm run unit`、`pnpm run typecheck`、`pnpm run smoke`、`pnpm run probe`、
> `node scripts/launch.cjs --probe --tabs`。当前基线：unit 115/115、typecheck ✓、smoke ✓、
> 单窗探针 149/150（唯一失败为预存 scroll 环境项，A/B 基线同样失败）、页签 47/47。

### 批次 1：静默失败与安全闸门（最小改动、最高收益）

- **范围**：H-1、H-2、H-3。
- **动作**：stage 清单单一真源 + asar 校验；兜底页保留快捷键；统一 sender 闸门 + bootstrap 帧校验。
- **验收**：新增 3-4 条单测；`check-asar-config` 在故意少铺 `floaticon.html` 时变红；
  探针 149/150 与页签 47/47 不变。

### 批次 2：跨壳契约收敛

- **范围**：M-1、M-2、L-5。
- **动作**：跨壳字面量对照守卫（`isBuiltinUrl` / 标签栏高度 / 标题 trim）；Electron 标题 trim；
  快捷键判据下沉 `hotkey.ts` 并在保存前校验；修正 AGENTS 共用件清单。
- **验收**：新增 Guards 至少 4 组；故意改单侧正则应让另一侧变红；两壳手测标题与快捷键行为一致。

### 批次 3：入口解耦与生命周期收口

- **范围**：H-4（先做条件加载与打包剔除）、M-7、M-8、L-1。
- **动作**：`probe` 改条件 `import`；构建剔除探针文件并加「asar 不得含 probe」护栏；
  `index.ts` 下沉 `bootstrap.ts` / `main-window.ts`；主窗访问器统一并清空悬垂引用；球状态统一重置。
- **验收**：发布包不含 `probe*.js` / `tabs-probe-preload.js`；探针模式仍能完整跑 149+47；
  入口文件行数降到约 500 以内；单测不变。

### 批次 4：探针与测试架构

- **范围**：M-5、M-6。
- **动作**：`probe-single/` 按段拆文件、`probe-tabs` 段函数化、删伪共享；`planRestore` / sidecar 参数 /
  `normalizeIncoming` 抽纯函数并补单测。
- **验收**：探针断言行多重集与拆分前**逐字一致**（PowerShell `Compare-Object` 或等价手段）；
  单测新增 ≥12 条；`probe` 数字不变。

### 批次 5：复用收口与低风险清理

- **范围**：M-3、M-4、M-9、M-10、L-2、L-3、L-4。
- **动作**：`paths.ts`、`failText.ts`、`tauri-shim.ts`、协议 handler 拆分、preload 分块与内部页清单；
  导出面注释、窗口命令统一抛错。
- **验收**：unit / typecheck / smoke / probe / tabs 全绿；新增协议与 preload 守卫。

### 批次 6：守卫升级（可与批次 4 合并）

- **范围**：M-2 的 float-contract 部分、M-10 的探针部分。
- **动作**：把 `float-contract` 中「实现镜像」与「真契约」分文件；统一「先剥注释再匹配」；
  消除 `|| includes(...)` 这类可被注释满足的写法；只保留跨文件接线与跨壳对照。
- **验收**：反向对照（把接线改坏 / 把实现注释掉）必须 FAIL；正常代码全绿。

---

## 七、附：证据摘要（按文件）

| 文件 | 本次关注点 |
|---|---|
| `src/main/index.ts` | 954 行、0 导出；26 个 import；`PROBE` 分叉多处（404/453/692-702/726/778 等）；`mainWindow` 赋值 778 行后从不置 null；`hideFloatFromIcon` 610-625 |
| `src/main/app-ipc.ts` | `assertPrivileged` 单窗口放行 64-74；`assertShell` 111-119；bootstrap 无闸门 192-200 |
| `src/main/settings-window.ts` | 兜底页 76-104（空快捷键 98）；`normalizeIncoming` 204-219；`assertSettingsSender` 150-155 |
| `src/main/settings-controller.ts` | 三组副作用回滚；`trayExistedBefore` 无条件置真 128 |
| `src/main/float-window.ts` | 六元模块状态 109-126；`deps` 永不重置 157；`anchor` 遮蔽 655 |
| `src/main/probe-single.ts` | 1360 行；4 个段 >80 行；伪共享 `s/fwd/side` 79-88 |
| `src/main/probe-tabs.ts` | 40-553 单函数 514 行 |
| `src/main/protocols.ts` | 注册/资源/兜底/`/retry` 混杂 44-104 |
| `src/renderer/preload.ts` | 桥 78-104 / 注入 106-150 / overlay 152-174；`isInternalPage` 108-114 |
| `scripts/stage-assets.cjs` | 7 项注入资源 123-132 |
| `scripts/check-asar-config.cjs` | 仅校验 5 项 30、98 |
| `webview-app/AGENTS.md` | 共用件清单含单壳专属件 12 |
| `webview-app/src/floaticon.html` | 保留 Tauri `tauriEmit` 死分支 286-330 |
| `webview-app/src/tab_title.js` | 标题 trim 且空值不发送 69-72 |
| `webview-app/dist/settings.html` | 快捷键判据仅在页面 421-435 |

---

## 八、未纳入本次判断的项

- 真机五步目视清单（吸附手感 / 拖动跟手 / 悬停缩回 / 双击去抖 / 右键关功能）无法自动化，仍按
  `electron-app/docs/float-ball.md` 人工验证；本次审计不重复其结论。
- `float-contract.test.ts` 的具体假绿清单较长，归入批次 6；本次只确认「实现镜像占比高」这一结构事实。
