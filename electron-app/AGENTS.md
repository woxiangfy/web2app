# web2app Electron 绿色版壳（electron-app）

> web2app 的 **Windows 绿色版**壳（Electron），与 `../webview-app`（Rust + Tauri 2）行为兼容。
> Tauri 版继续维护（同时负责 Android），两者共用同一份 `config.json` 契约与同一套注入脚本。
>
> 完整方案见 `../docs/electron-plan.md`（那是**决策与架构记录**，里程碑状态与命令**以本文件为准**）；
> 可行性论证见 `../docs/electron-analysis.md`；体积与分发形态见 `../docs/electron-size-reduction.md`；
> 全仓库规则与目录地图见根 `AGENTS.md`。

## 现状（先看这张表再动手）

| 里程碑 | 状态 | 一句话 |
|---|---|---|
| M0 / M1 脚手架 + 窗口 + 注入 + 探针 | ✅ | 无边框窗 + 注入脚本 + 探针骨架（39 → 150 条） |
| M2 sidecar（本地代理 + 加密解析） | ✅ | 复用 Rust `proxy.rs` / `resolve.rs`，零重写 → [probe-sidecar.md](docs/probe-sidecar.md) |
| M3 托盘 / 快捷键 / 设置窗 / 窗口几何 / 悬浮球 | ✅ | 与 Tauri 版文件级互通 → [settings-runtime.md](docs/settings-runtime.md) |
| M4 页面行为（下载接管 / 右键菜单 / F12） | ✅ | `src/main/page-behavior.ts`；F12 由 `devtoolsEnabled()` = `!isPackaged \|\| debug` 控制 |
| M5 多页签 | ✅ | `tabs_enabled=true`；选型见 `../docs/electron-tabs-design.md`、真机验收见 `../docs/electron-tabs-test-steps.md`（V1–V4 已关闭） |
| M6 出包（dir / zip / portable / NSIS） | ✅ | → [build-package.md](docs/build-package.md) |
| M7 CI 流水线 | ✅ | `.github/workflows/electron-ci.yml`（lint / e2e / package） |

## 命令

```bash
# ⚠️ 首次安装前先设两个镜像环境变量，否则会卡在下载（见 build-package.md）
pnpm install         # 装依赖；装完必须确认 node_modules/electron/dist/electron.exe 存在

pnpm start           # 编译 + 启动（开发态）
pnpm run typecheck   # 只做类型检查（不能代替 build）
pnpm run build       # 只编译到 out/
pnpm run unit        # 纯函数单测（node:test，毫秒级）

pnpm run sidecar     # 编出 ../webview-app/target/release/w2a-proxy.exe
pnpm run stage       # 打包前置：铺 build/icon.ico 与 out/config.json
pnpm run prune       # 打包前置：剔除 out/ 下的探针产物（probe*、tabs-probe-preload）
pnpm run smoke       # 冒烟自检（内部强制 debug=true），退出码即结论
pnpm run probe       # 兼容契约探针：158 条断言，退出码即结论
pnpm run shot        # 布局取证：四条判据 + PNG

pnpm run pack        # 只出解压目录 release/win-unpacked/
pnpm run dist        # 出 zip + portable exe（→ release/）
pnpm run dist:installer  # 出 NSIS 安装版（→ ../tests/pack）
pnpm run dist:all        # 绿色版 + 安装版一次出齐
```

一次性排查工具（怀疑「断言过、人眼不对」时才用）：`pnpm run lab` 滚动条多模式对照 · `diag` 标题栏遮挡诊断 ·
`png` 逐像素取证 · `tabs-v4` / `tabs-v4-negative` 子视图绘制顺序。用法见 [docs/titlebar-scrollbar.md](docs/titlebar-scrollbar.md)。

## 硬约束速览（不知道就会踩坑）

1. **必须经 `scripts/launch.cjs` 启动**（`start` / `smoke` / `probe`），**不要直接 `npx electron .`** —— 宿主注入的
   `ELECTRON_RUN_AS_NODE` 会让 Electron 以纯 Node 身份启动，报错位置却在本项目代码里（见 [probe-sidecar.md](docs/probe-sidecar.md)）。
2. **改配置 / 注入脚本后必须重新打包**：`out/config.json` 与 `out/inject/*` 都随 `app.asar` 分发，没有「只换注入脚本」的路径。
3. **`typecheck` 不能代替 `build`**：tsc 默认「有错也照样输出」，打包会把半成品封进 asar。
4. **单一真源与本壳共用**：`titlebar.js` / `loading.js` / `splash.html` / `settings.html` / `proxy.rs` / `resolve.rs` ——
   改动后跑 `probe`（唯一能发现「另一个壳被带坏」的手段）。
5. **设置页命令名三处逐字一致**：`settings.html` 字面量 / `settings-preload.ts` 白名单 / 主进程 IPC 名。
6. **`settings.json` 三个写入方一律读-改-写**（`saveSettings` / `saveWindowState` / `persistGeometry`（主窗）/
   `persistFloatWinGeometry`（球窗）），否则互相覆盖。
7. **「exe 同级」定位一律走 `installRoot()`**，不要写 `app.getPath('exe')`（单文件模式下它指向 `%TEMP%` 解压副本）。
8. **`pnpm run` 传参不要写 `--` 分隔符**；装依赖前必须设 `ELECTRON_MIRROR` / `ELECTRON_BUILDER_BINARIES_MIRROR` 两个镜像环境变量。
9. **`data_dir=""` 是有意义的值**（系统默认目录），不能写成 `data_dir || 'data'`。
10. **特权 IPC 一律走 `src/main/ipc-guard.ts::assertTrustedSender`**：窗口 / 页签 / 设置 / 悬浮球四类命令
   **两种模式都校验** `sender.id === 宿主 webContents.id`；`web2app:bootstrap` 还要求顶层框架。
   不要按「是否多页签」决定是否校验（2026-11-07 架构审查 H-3）。
11. **注入资源清单只在 `scripts/staged-inject.cjs` 维护**：`stage-assets.cjs`（铺 `out/inject/`）与
   `check-asar-config.cjs`（出包后护栏）共用这一份，别在任一侧再写一份数组（否则新增文件时护栏会静默漏项）。
12. **唤醒快捷键主进程也要校验**：保存前 `hotkey.ts::hotkeyProblem`（裸单键 / 系统占用 / `Shift+F1` /
   无法映射的按键），页面提示之外的第二道。跨壳字面量（内置页正则、`TAB_BAR_HEIGHT`/`MAX_TABS`、
   占用快捷键表、页签标题 trim）改一侧必须改另一侧 —— `test/cross-shell-contract.test.ts` 同时读两侧源码对照。
13. **探针 / 冒烟只能运行期动态加载**：`src/main/index.ts` 顶部只允许 `import type { ProbeServer }`，
   调用点一律 `await import('./probe.js')` / `await import('./smoke.js')`（静态 import 会把约 2300 行
   探针连同 `out/` 一起打进 `app.asar`）。打包前 `scripts/prune-probe.cjs` 剔除 `out/main/probe*` 与
   `out/renderer/tabs-probe-preload.js(.map)`，`check-asar-config.cjs` 反向断言包内不含它们 —— 新增打包
   命令时必须保留 `pnpm run prune` 这一步，这是 `test/probe-isolation.test.ts` 钉的顺序（build→stage→prune→builder）。
   ⚠️ **出包入口有三个，每个都要有 prune**：① 本目录 `package.json` 的 `pack` / `dist*`；
   ② **本地打包**（根 `scripts/packlib/driver-electron.mjs`，被 `tests/build-electron-all.bat` 调起）；
   ③ **`.github/workflows/` 里自己拼步骤的**（`electron-build-custom.yml` 按勾选组合出包，
   不调 `pnpm run dist`）。2026-11-07 首次整改只改了① ⇒ 2026-11-14 本地与 CI 两条路都被实测出
   混进探针产物。三者的顺序现由 `probe-isolation.test.ts` 分别钉住：① 查 `package.json` 脚本、
   ② 查驱动源码、③ 扫描 `.github/workflows/*.yml`（自己拼步骤的必须显式 prune 且位置在前）。
14. **主窗显隐判据只有一条**：`src/main/window-toggle.ts::planToggle(visible, minimized)`（可见且非最小化才隐藏），
   托盘左键 / 悬浮球单击都走它，不要各自写内联表达式。
15. **窗口几何的纯判定在 `src/main/window-plan.ts`**：`parseWindowState` / `planRestore` 不得再依赖
   `electron`（`screen` 只允许出现在 `window-state.ts` 适配层），否则单测 import 不进来（M-5）。
   同理：新增纯逻辑优先放零 Electron 依赖的模块，`node:test` 才能覆盖。
16. **资源路径只有 `src/main/paths.ts` 一个出口**：`injectDir` / `injectFile` / `floatIconPath` /
   `configRoot` / `preloadPath`。新增「打包态读 asar、开发态读仓库」的资源时加在这里，不得在入口
   再写 `app.isPackaged ? … : …` 三元；判据用 `app.isPackaged`，**不是**「文件是否存在」（`stage`
   会在开发目录生成 `out/inject/`）。导航失败文案 / 内部页判定同理：`nav-text.ts` 单一真源。
17. **preload 不得相对 require**：`preload.ts` / `shell-preload.ts` / `settings-preload.ts` /
   `tab-preload.ts`（页签内容页最小 preload，注入 `window.web2appExtra` + 网页登录账号密码管理）都跑在
   `sandbox: true` 渲染进程里（Electron 45 起 `require` **只放行 `electron`**，`events` / `timers` / `url`
   三个 shim 与 `Buffer` / `setImmediate` / `clearImmediate` 全局一并移除）。需要共享的桥逻辑
   只能在**文件内**分块，跨文件的形状一致性由 `test/preload-contract.test.ts` 锁（不抽共享模块）；
   `tab-preload` 的装配（tabs.ts 真传 / index.ts 真装配 / 通道真注册）由 `test/tab-preload.test.ts` 锁，
   运行期由 `probe --tabs` 的 P2b 两条成对断言负责。
   ⚠️ **页签内容页不得挂 `preload.js`**：它会把自绘标题栏 / 加载层注入内容页，而页签模式的标题栏
   属于壳页面 `shell.html` —— 挂上去就是两条标题栏。内容页需要新数据时，往 `tab-preload.ts` 加。
18. **网页登录账号密码保存是 Electron 自研**（Tauri 走 WebView2 原生 `IsPasswordAutosaveEnabled`）：
   Electron 只含 Chromium content 层，**没有** Chrome 的密码管理器 / 自动填充 UI，也没有任何开关能打开。
   实现约定：`preload.ts` 与 `tab-preload.ts` 各有一段**逐字一致**的 `installPasswordManager()`（沙箱不能
   共享模块，由 `test/password-contract.test.ts` 直接比对两段源码；运行期 `probe` 第 18 节 / `P13` 各自
   覆盖单窗口与页签）；只认 `isTrusted` + `navigator.userActivation.isActive` 的 submit，**不得**
   `preventDefault`（DOM 气泡活不过导航，阻止提交会改站点行为）；不向页面主世界暴露任何桥；
   存储只放 `credentials.json` 的 `safeStorage` 密文，加密不可用时拒绝保存；`clearWebData` 必须一并
   清理；**不得在 Tauri 侧也注入这段**（WebView2 已原生弹一次，会双弹窗）。

## 刻意不动（已定，勿再提拆分）

- **探针大结构维持现状**：`probe-single.ts` 不按段拆目录、`probe-tabs.ts` 的 514 行编排不按 P 段函数化 —— 探针改动的唯一验收手段是「断言行与改动前逐字一致」，纯机械搬迁约 1900 行的回归风险大于可读性收益。
- **`index.ts`（约 950 行）不继续大拆**：它的装配顺序与副作用时序（谁先注册协议、谁先建窗、探针何时动态加载）本身就是硬约束，拆成多文件只会把它们变成跨文件的隐式契约，回归面大于收益。新增逻辑仍按硬约束 15 下沉到零依赖模块。

## 验证清单（改了什么就必跑什么）

| 你改了什么 | 必跑 |
|---|---|
| `titlebar.js` / `loading.js` / `splash.html`（两壳单一真源） | `probe` |
| `proxy.rs` / `resolve.rs`、sidecar 契约 | `probe` + `cargo test` |
| 设置 / 托盘 / 快捷键 / 窗口几何链路 | `probe` |
| 特权 IPC / sender 闸门（`ipc-guard.ts`、`app-ipc.ts`） | `unit` + `probe`（单窗口）+ `probe`（`--tabs`，P11 拒绝面） |
| 跨壳字面量 / 契约（内置页 URL、标签栏高度、标题 trim、快捷键表） | `unit`（`cross-shell-contract.test.ts` 同时读两侧源码） |
| `float-window.ts` / `float-layout.ts` / 设置开关接线 | `unit` + `probe` + **真机目视**（五步清单见 [float-ball.md](docs/float-ball.md)） |
| 资源路径（`paths.ts`）、导航失败文案 / 内部页判定（`nav-text.ts`） | `unit` + `smoke` + `probe` |
| 自定义协议（`protocols.ts`，含 `/retry` 闸门） | `unit` + `probe` + `真机点启动页重试` |
| 任何 preload（`src/renderer/*preload.ts`） | `unit`（`preload-contract`）+ `probe` |
| 账号密码保存（`credentials*.ts`、`w2a:pwd-*`、两 preload 密码段） | `unit`（`credentials` + `password-contract`）+ `probe`（第 18 节）+ `probe --tabs`（P13）+ 真机登录一次 |
| sidecar 参数拼装 / 可执行文件定位（`sidecar.ts`） | `unit` + `probe` |
| 导航状态机（`navigation.ts`：target 缓存 / 自动 bootstrap 名额 / 重试重置） | `unit` + `probe` |
| 启动日志（`startup-log.ts`）、`debug` 开关与 `silenceConsole()` | `smoke` |
| 打包配置 / 出包前后 | `probe` + `真机双击产物` + 核对包内无 `out/main/probe*`（`prune` 与 asar 反向断言） |
| 任何提交前 | `typecheck` |

> 逐条原因见各专项文档。产物侧的 `--probe` 必须用 `Start-Process -Wait -PassThru` 取 `$p.ExitCode`
> —— 直接 `& app.exe --probe` 对 GUI 子系统 exe 是**假绿**（PowerShell 不等待，退出码立刻为 0）。

## 变更记录

> 全量变更记录（逐条保留原文，含门禁数字与踩坑细节）见 [docs/changelog.md](docs/changelog.md)。此处仅保留最近 3 条。

| 日期 | 摘要 |
|---|---|
| 2026-11-15 | **Electron 44.4.1 → 45.3.2**：45 唯一的硬破坏面（sandbox preload 移除 `events/timers/url` shim 与 `Buffer/setImmediate/clearImmediate` 全局）本壳不受影响 —— preload 只 `require('electron')`；`setShape` / `WebContentsView` / `setWindowOpenHandler` 在 45 typings 中均在（unit 188/188、probe 156/158 既有 F12 项、tabs 58/58、smoke ✅、asar 护栏 52 项通过） |
| 2026-11-15 | 多页签下载页签自动关闭：`window.open` 附件（未提交文档）在落盘/取消后自动关页签并回到打开者，已加载页签不关；`DownloadHooks`（`onSaveChoice` / `pickSavePath`）+ 页签探针 P14 两条成对断言。**另修「一次点击两次下载 / 两个另存为」（unit 188/188、tabs 59/59）**：根因是站点用 `if (!w || w.closed)` 判断「弹窗被拦截」—— `deny` 让 `window.open()` 拿到 null，而「建了立刻销毁」让 `w.closed` 变 true，两条都会触发 `location.href` 兜底。改 `allow` + `overrideBrowserWindowOptions{show:false,skipTaskbar:true}` 建隐藏窗口，并让它**活到这次下载结算**（`registerHiddenPopup` / `attachHiddenPopupToTab`：绑到承载它的新页签，页签关闭才销毁；`onDownloadStart` 取消兜底超时），新页签承接导航与下载；隐藏弹窗自己抢跑的那条下载由 `isHiddenPopup` 丢弃，URL 判重（`shouldDropPopupFallback`）降为纵深防御。P15 断言 `window.open()` 非 null、兜底未执行、**下载中 `w.closed === false`**、附件完整落盘。**同日发现 `--debug` 不可用**：Electron 44→45 升 Node 后该参数被 Node 直接拒绝（`DEP0062`），进程秒退、日志一行不写 ⇒ 诊断开关改名 **`--w2a-debug`**（环境变量 `WEB2APP_DEBUG=1` 不变） |
| 2026-10-24 | 架构审查优化批次①–⑤：`updateDocument` 读-改-写统一入口 / index.ts 装配化 / `probe-single` 19 段函数化 / 设置页命令名三方守卫 / `util.ts` + `protocols.ts` 依赖收口 + 悬浮球 `planDrop` / `planPeekCollapse` 纯函数化（unit 115/115、probe 149/150 预存环境项、tabs 47/47） |
| 2026-10-11 | v1.75 修「只启用悬浮球时关窗 → 点球不回窗口」：驻留判据纳入 `float_enabled` + 入口统一 `requestQuit` + 设置页新增「退出程序」 |

## 专项文档

| 文档 | 内容 |
|---|---|
| [docs/build-package.md](docs/build-package.md) | pnpm 与镜像、打包坑表、手动打包 workflow、产物核对、绿色版形态、SmartScreen |
| [docs/runtime-data.md](docs/runtime-data.md) | `data_dir` 语义、日志开关与落点、`config.json` 随 asar 与严格校验、`app_name` / `title` |
| [docs/settings-runtime.md](docs/settings-runtime.md) | 运行期 `settings.json`、行为契约表（托盘 / 关闭 / 几何）、设置窗与命令白名单 |
| [docs/probe-sidecar.md](docs/probe-sidecar.md) | `launch.cjs` 必要性、sidecar 进程间契约、探针覆盖明细、安全基线、占位符契约 |
| [docs/float-ball.md](docs/float-ball.md) | 桌面悬浮球：IPC / 布局常量 / `placeFloat`、v1.68–v1.80 演进、守卫与真机验证 |
| [docs/titlebar-scrollbar.md](docs/titlebar-scrollbar.md) | 自绘标题栏与页面滚动条：换滚动根契约、对照实验、判据表与 diag 工具 |
| [docs/changelog.md](docs/changelog.md) | 本壳全量变更记录 |
| [../docs/electron-app-architecture-review-2026-11-07.md](../docs/electron-app-architecture-review-2026-11-07.md) | 本壳架构与组织结构审查（2026-11-07）：分级问题、跨壳漂移、优化批次顺序（批次①–⑥ 已落地，逐批改动见 [../docs/changes/2026-11-07.md](../docs/changes/2026-11-07.md)） |
