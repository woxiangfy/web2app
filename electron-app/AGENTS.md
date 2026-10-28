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
pnpm run smoke       # 冒烟自检（内部强制 debug=true），退出码即结论
pnpm run probe       # 兼容契约探针：150 条断言，退出码即结论
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
6. **`settings.json` 三个写入方一律读-改-写**（`saveSettings` / `saveWindowState` / `persistGeometry`），否则互相覆盖。
7. **「exe 同级」定位一律走 `installRoot()`**，不要写 `app.getPath('exe')`（单文件模式下它指向 `%TEMP%` 解压副本）。
8. **`pnpm run` 传参不要写 `--` 分隔符**；装依赖前必须设 `ELECTRON_MIRROR` / `ELECTRON_BUILDER_BINARIES_MIRROR` 两个镜像环境变量。
9. **`data_dir=""` 是有意义的值**（系统默认目录），不能写成 `data_dir || 'data'`。

## 验证清单（改了什么就必跑什么）

| 你改了什么 | 必跑 |
|---|---|
| `titlebar.js` / `loading.js` / `splash.html`（两壳单一真源） | `probe` |
| `proxy.rs` / `resolve.rs`、sidecar 契约 | `probe` + `cargo test` |
| 设置 / 托盘 / 快捷键 / 窗口几何链路 | `probe` |
| `float-window.ts` / `float-layout.ts` / 设置开关接线 | `unit` + `probe` + **真机目视**（五步清单见 [float-ball.md](docs/float-ball.md)） |
| `config.ts`、路径装配（`portable.ts`） | `smoke` |
| 启动日志（`startup-log.ts`）、`debug` 开关与 `silenceConsole()` | `smoke` |
| 打包配置 / 出包前后 | `probe` + 真机双击产物 |
| 任何提交前 | `typecheck` |

> 逐条原因见各专项文档。产物侧的 `--probe` 必须用 `Start-Process -Wait -PassThru` 取 `$p.ExitCode`
> —— 直接 `& app.exe --probe` 对 GUI 子系统 exe 是**假绿**（PowerShell 不等待，退出码立刻为 0）。

## 变更记录

> 全量变更记录（逐条保留原文，含门禁数字与踩坑细节）见 [docs/changelog.md](docs/changelog.md)。此处仅保留最近 3 条。

| 日期 | 摘要 |
|---|---|
| 2026-10-24 | 架构审查优化批次①–⑤：`updateDocument` 读-改-写统一入口 / index.ts 装配化 / `probe-single` 19 段函数化 / 设置页命令名三方守卫 / `util.ts` + `protocols.ts` 依赖收口 + 悬浮球 `planDrop` / `planPeekCollapse` 纯函数化（unit 115/115、probe 149/150 预存环境项、tabs 47/47） |
| 2026-10-11 | v1.75 修「只启用悬浮球时关窗 → 点球不回窗口」：驻留判据纳入 `float_enabled` + 入口统一 `requestQuit` + 设置页新增「退出程序」 |
| 2026-10-11 | v1.72 悬浮球四角穿透（`circleShapeRects` + `win.setShape` 包 try/catch）+ 250ms 双击去抖；真机 harness 17 项 |

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
