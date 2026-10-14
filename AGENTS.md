# 仓库总览与开发约定

> 本文档为仓库全局通用规范。子项目可维护独立规范，但不得与本规范冲突；同一信息仅在一处维护，其他位置保留相对路径引用。

## 一、项目概述

- **项目名称**：**web2app** —— 把指定 Web 站点封装为桌面应用的「网页转桌面」壳（双壳并行）。
- **核心定位**：两个壳共享同一份前端真源与配置语义，对外表现为：自绘标题栏的独立窗口、托盘最小化、全局快捷键、多页签、下载接管、窗口几何持久化与设置页；入口地址由 DNS TXT + Ed25519 签名 / AES-256-GCM 解密下发，并支持本地反向代理统一注入鉴权。
  - `webview-app/`：Rust + Tauri 2 跨平台壳（Windows 为主战场，Android 交叉编译验证）；
  - `electron-app/`：Electron + TypeScript 实现的 Windows 免安装绿色版壳。

## 二、仓库结构

| 路径              | 类型                     | 说明                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | 文档入口                            |
| --------------- | ---------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------- |
| `webview-app/`  | 子项目                    | Rust + Tauri 2 主工程：cargo workspace 四成员（`web2app` 主程序 / `resolve-core` 纯加密 / `resolve-tool` 服务端 CLI / `proxy-server` → Electron 侧 sidecar `w2a-proxy.exe`），含注入脚本、CI 与打包工作流                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | `webview-app/AGENTS.md`         |
| `electron-app/` | 子项目                    | Electron 绿色版工程：第二交付形态（TypeScript 主进程 / 渲染进程、sidecar 复用、打包与探针脚本）；里程碑交付状态与权威值以本目录 `AGENTS.md` 为准                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | `electron-app/AGENTS.md`        |
| `docs/`         | 全局文档                   | 全仓库级专项详细文档（`docs/archive/` 存放已归档历史方案稿；`docs/changes/` 存放按日期归档的详细变更记录）                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | 见「四、文档索引」                       |
| `.github/`      | CI 与打包工作流              | 全仓库统一的 GitHub Actions（2026-09-22 由 `webview-app/` 上移到仓库根；命令经 `defaults.run.working-directory: webview-app` 落到 Rust 工程目录，产物路径则显式带 `webview-app/` 前缀）                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | —                               |
| `scripts/`      | 本地脚本（**不进 CI**）        | 本机侧驱动脚本（均为零第三方依赖的 Node 脚本）：①`init-env.mjs` —— 环境初始化：webview-app 跑 `cargo fetch`（默认仅宿主平台目标，避免拉 Linux/macOS/Android 无关 crate；`--all-targets` 恢复全平台，`--update` 先更新 Cargo.lock），fetch 成功后 tauri-cli 缺失时**自动** `cargo install tauri-cli --version "^2"`（与 CI 同一步，`--no-tauri-cli` 可跳过）、electron-app 跑 `pnpm install`（自动注入 electron / electron-builder 国内镜像环境变量，装完校验 electron 平台二进制并兜底）；子进程输出实时上屏并 tee 到系统临时目录日志，失败时在汇总处重放错误尾部；支持 `--webview` / `--electron` / `--frozen` / `--no-mirror` / `--no-tauri-cli` / `--check`；②`pack-local.mjs`（**入口薄壳**，实现拆在同目录 `packlib/` 的 10 个模块：constants / diagnostic / parser / pycompat / util / paths / driver-electron / driver-tauri / selftest / parity）：读一份配置 JSON，在本机跑出与 CI **同语义**的产物（Tauri MSI/单文件、Electron zip/portable/NSIS 安装版；产物统一落仓库根 `tests/`，不进版本库），⚠️ 它是 CI 解析器的 **Node 复刻**（Python 不是必然存在的依赖），两侧靠 `--parity` 逐项比对钉住等价 | `docs/packaging-config.md` §7.2 |
| `tests/`        | 本地测试工作区（**部分入库**）       | 一次性验证产物（探针日志、截图、打包产物副本）的默认落点 —— `scripts/pack-local.mjs` 的本地产物就落 `tests/pack/{electron,tauri}/`。⚠️ **不是整目录忽略**：根 `.gitignore` 逐条锚定 `/tests/pack/`、`/tests/json/`、`/tests/pw-fixture/`、`/tests/pw-udd-*/`；`tests/ci-sdk-step/`、`tests/build-*.bat` 为**有意入库**的回归夹具/脚本（详见 §3.2）                                                                                                                    | `AGENTS.md` §3.2                |
| `.workbuddy/`   | AI AGENT 记忆目录（**已纳入版本控制**） | 项目记忆：`memory/MEMORY.md`（长期契约索引）、`memory/topics/*.md`（细节与教训全量）、`memory/YYYY-MM-DD.md`（按日工作日志，**只追加**）、`memory/automations/`。⚠️ `.gitignore` 中 `.workbuddy/`、`.trae/` 两行**已注释掉** ⇒ 二者不再是被忽略目录                                                                          | `.workbuddy/memory/MEMORY.md`   |
| `.trae/`        | AI AGENT 规格目录（**已纳入版本控制**） | 历史 spec / 审计稿（`specs/`）。2026-09-22 起归档方案稿已迁 `docs/archive/`                                                                                                          | —                               |
| `signing/`      | 打包签名与发布凭据（**未纳入版本控制**） | keystore、打包配置与发布 secrets（**含敏感信息，不得外发**）；由仓库根 `.gitignore` 忽略。⚠️ 该目录按需在本地创建，**干净检出中本就不存在**                                                                                       | —                               |

> 顶层目录变动时，须同步更新上表及对应项目的 `AGENTS.md`。

## 三、全局约定

### 3.1 开发与交付

- **同步更新**：代码、逻辑变动必须与测试用例、相关文档同步更新。
- **交付门槛**：所有变更必须在本地或构建环境测试通过后方可交付合入。

### 3.2 测试要求

- **目录存放**：测试代码、测试用例统一存放于对应项目的 `tests` 目录下，不得混入业务代码目录。
  **例外（与现状对齐，2026-09-20）**：两壳的自动化断言都**必须在应用进程内执行**，因此不在 `tests/`：
  - `webview-app`：Rust 单元测试用 `#[cfg(test)]` 内联在 `src/*.rs` 内（Rust 惯例，`cargo test` 直接跑）；
  - `electron-app`：契约探针 `src/main/probe.ts`（**入口编排**，断言组拆在 `probe-single.ts` / `probe-tabs.ts` / `probe-infra.ts` / `probe-server.ts`；`pnpm run probe`，命令行加 `--tabs` 走页签断言组）、
    纯函数单测 `test/*.test.ts`（`pnpm run unit`，node:test，零 Electron 依赖直接跑 TS）、
    启动经 `electron-app/scripts/launch.cjs`（`smoke` / `start`）；像素 / 布局取证脚本在 `electron-app/scripts/diag/`（`shot` / `lab` / `diag` / `png` / `tabs-v4` / `tabs-v4-negative`；另有 `splash-fail-card.cjs` —— 启动页失败卡「看不看得见」的真实 Chromium 渲染取证、`float-ring-lab.cjs` —— 悬浮球圆形命中（`setShape`）的渲染/命中对照，两者都直接 `node` 调用，不占 npm script）。
- **环境干净**：`tests/` 目录只装**一次性验证产物**（探针日志、截图、产物副本）——**不是测试代码**；测试过程产生的临时文件、中间产物一律落到这里，不污染业务代码目录。
  **例外（有意入库）**：`tests/ci-sdk-step/`（假 `sdkmanager` + 真仓库清单数据 + `run-tests.sh`，验 `.github/scripts/install-android-sdk.sh`）与 `tests/build-*.bat` 是**回归夹具/脚本**，属测试代码。
  ✅ **已清理（2026-10-12）**：`tests/float-frag-rc/`（v1.79/v1.80 悬浮球「背景异形」的一次性取证工作区：截图、`*.py` 量测脚本、`.ps1`、`.txt`；共 **79** 个文件）曾因**该子目录未被 `.gitignore` 锚定**而被顺手 `git add -A` 收进版本库，与本节「一次性产物不入库」相悖；现工作区已整体删除，但**删除尚未暂存**（`git status` 显示 79 个 ` D`）⇒ 需要一次 `git add -A tests/`（或 `git rm -r --cached tests/float-frag-rc`）把它从索引除名，否则仍留在下一次提交里。根 `.gitignore` 已补 `/tests/float-frag-rc/`。**教训**：往 `tests/` 下写一次性产物前，先确认该子目录已在忽略规则内 —— 光有「一次性产物不入库」的约定，挡不住 `git add -A`。
- **回归补充**：核心逻辑变更或 Bug 修复必须补充对应的测试用例，保障可回归验证。

### 3.3 三级文档体系

| 层级          | 路径                    | 职责范围                  |
| ----------- | --------------------- | --------------------- |
| **L1 根文档**  | `AGENTS.md`           | 仓库全局规则与通用规范，不记录子项目细节。 |
| **L2 项目文档** | `<project>/AGENTS.md` | 子项目专属业务、配置、接口与开发说明。   |
| **L3 专项文档** | `docs/`               | 全局架构、部署流程等跨项目专题文档。    |

- **命名规则**：专项文档采用小写+中划线（如 `docs/electron-plan.md`、`docs/packaging-config.md`）。
- **子文档拆分**：子项目复杂专题可放置于 `<project>/docs/`，但必须在 `<project>/AGENTS.md` 中登记入口。
- **单一维护**：同一信息仅在一处维护，其他位置保留相对路径引用；全局规则变更仅更新 L1 根文档。
- **同步生效**：文档更新与代码变更同步提交、同步生效。
- **登记义务**：新增 L3 文档须在「四、文档索引」登记；子项目内部文档除登记外，还需在其目录 `AGENTS.md` 中交叉引用。

### 3.4 其它全局约定

1. **单一真源**：注入脚本与页面（`titlebar.js`、`loading.js`、`tab_title.js`、`dnd_polyfill.js`、`shell.html`、`splash.html`、`floaticon.html`、`dist/settings.html`、`icons/*`）
   及代理/解析链路（`proxy.rs` → sidecar）为两个壳共用，改动任一均须双壳同步验证，避免契约漂移。
   ⚠️ `mobilebar.js`（移动端底栏）**不在此列**：它是 Tauri 移动端专属（Electron 壳只在 Windows，
   桌面目标连这个模块都不编译），改它只需过 Tauri 侧门禁 + 契约测试。
2. **文本编码**：文档统一 UTF-8（无 BOM）、LF 换行，避免混用导致整文件 diff。
3. **路径引用**：跨目录引用一律相对仓库根目录书写（如 `docs/electron-plan.md`）；同目录内可直接写文件名。
4. **敏感文件**：`signing/` 下的 keystore、`secrets.env` 等凭据不得提交或外发。该目录已由仓库根 `.gitignore` **显式忽略**，提交前请确认 `git status` 内不含 `signing/`。⚠️ 注意 `.workbuddy/` 与 `.trae/` **已改为纳入版本控制**（`.gitignore` 中那两行已注释掉、口径为「AI AGENT 记忆目录」），不再属于忽略项 —— 别再拿它们当「被忽略目录」的例子。
5. **本地打包工作文件（不入库）**：手工打包用的 `test1.json` / `test2.json`（差异仅在 `tabs_enabled`）放在 **`tests/`** 下，与 `scripts/pack-local.mjs` 的本地产物（默认 `tests/pack/electron/` 与 `tests/pack/tauri/`，win-unpacked 整树 + zip/portable）同处本地工作区，不进版本库。⚠️ **实际忽略规则是逐条锚定的，不是整目录**：根 `.gitignore` 只忽略 `/tests/pack/`、`/tests/json/`、`/tests/pw-fixture/`、`/tests/pw-udd-*/`；`tests/` 下**其它**文件仍会被 `git add` 收进去（`tests/ci-sdk-step/` 与 `tests/build-*.bat` 是**有意入库**的回归夹具/脚本）。新建一次性取证目录前先确认它落在忽略规则内，否则用 `git status` 复查。入库的配置样例只有 `webview-app/config.example.json` 与 `electron-app/config.example.json`（CI 契约门禁校验这两份）。
6. **版本控制现状（2026-09-22 起：全项目单仓库）**：仓库根 = 本目录，远端 `git@github.com:woxiangfy/web2app.git`，两个壳同处一仓。
   - **沿革**：在此之前仓库根是 `webview-app/`，根目录与 `electron-app/` 均未纳管（`electron-app/` 长期无版本控制）。2026-09-22 把 `.git` 上移一级完成合并；**`webview-app/`** **目录内部零改动**，其相对路径、构建脚本与 `.gitignore` 全部原样有效。
   - **上移方式**：`.git` 目录直移，**未做历史重写**。因此老文件在新路径下被 git 识别为重命名（40 个），但**逐文件历史需用** **`git log --follow`** **才能穿透**这次搬运。
   - **跨壳路径契约（禁改）**：`electron-app/package.json` 的 sidecar 构建 `--manifest-path ../webview-app/Cargo.toml`、`scripts/stage-assets.cjs` 取 `../webview-app/icons/`、注入脚本 / `settings.html` / `shell.html` / sidecar 产物全部指向 `webview-app/`。**两壳同级是这些相对路径成立的前提**，不得把 `electron-app/` 挪进 `webview-app/`。
   - **CI 位置**：统一在仓库根 `.github/workflows/`（原在 `webview-app/.github/`）。工作目录策略见该文件内注释。
   - **待办**：两壳版本号当前都是 `0.1.0` 且仓库尚无 tag，首次发版前需约定 tag 前缀（如 `tauri-v*` / `electron-v*`）以免撞车。

## 四、文档索引

| 文档                    | 相对路径                                             | 说明                                                                                         |
| --------------------- | ------------------------------------------------ | ------------------------------------------------------------------------------------------ |
| 项目全盘分析与优化建议           | `docs/project-analysis.md`                       | 架构分析、模块职责地图、优化建议与遗留问题                                                                      |
| 架构与组织结构审查（2026-09-29） | `docs/architecture-review-2026-09-29.md`         | 模块边界与重复实现收敛审查：已执行 4 项（零行为变更）、待决策 4 项、刻意不动 3 项及依据                                           |
| Web2App 使用说明          | `webview-app/AGENTS.md`                          | Tauri 版配置、构建、打包与使用                                                                         |
| Electron 绿色版说明        | `electron-app/AGENTS.md`                         | 绿色版配置、构建、验证与使用                                                                             |
| 变更记录                  | `docs/changes/`                                  | 按日期归档的详细变更记录（`YYYY-MM-DD.md`）；根 `AGENTS.md`「五、变更记录」仅保留最近 6 条摘要                             |
| Electron 落地方案         | `docs/electron-plan.md`                          | M0–M7 里程碑与已锁定决策、兼容契约（权威状态以子项目 `AGENTS.md` 为准）                                              |
| Electron 多页签设计方案      | `docs/electron-tabs-design.md`                   | M5 多页签：A′ 架构定案（已实现并真机验收）、落地清单与验收判据                                                         |
| Electron 多页签真机测试步骤    | `docs/electron-tabs-test-steps.md`               | M5 真机验收清单（2026-09-18 完成，V1–V4 已关闭）+ 代理/域名解析待办                                              |
| Electron 方案分析         | `docs/electron-analysis.md`                      | 功能对等矩阵、复用策略、兼容性要点与安全基线                                                                     |
| 验签测试素材说明              | `docs/txttest-verify-materials.md`               | DNS TXT 密文手工验签素材                                                                           |
| Electron 打包瘦身方案       | `docs/electron-size-reduction.md`                | Electron 绿色版打包产物实测基线：语言包/DLL/压缩形态分级瘦身（P0 可直接落地）                                            |
| **Android 端优化清单** | **`docs/android-improvements.md`**               | **Android 壳易用性 / 功能优化的唯一任务清单：20 项映射矩阵、证据基线（Tauri 2.12 / wry 0.57）、补丁实施约定与真机验收清单** |
| **打包配置 JSON 契约**      | **`docs/packaging-config.md`**                   | **打包配置的唯一契约说明**：20 个字段的类型/默认值/含义、多端（Windows / Android / Electron 绿色版）行为对照、校验规则、两条打包入口与迁移指引 |
| （过程稿·溯源）配置字段总表        | `docs/config-fields-2026-09-22.md`               | 重构前 6 层配置面取证，结论已并入 `packaging-config.md`（已精简为溯源指针）                                         |
| （过程稿·溯源）打包 JSON 评审    | `docs/config-json-review-2026-09-22.md`          | 改造前配置 JSON 问题清单，结论已并入 `packaging-config.md`（已精简为溯源指针）                                      |
| （历史）标题栏导航按钮设计         | `docs/archive/add_titlebar_nav_buttons.md`       | 已归档，仅作追溯（2026-09-22 由 `.trae/documents/` 迁入）                                               |
| （历史）诊断与优化             | `docs/archive/web2app-diagnosis-optimization.md` | 已归档，仅作追溯（2026-09-22 由 `.trae/documents/` 迁入）                                               |

> 新增全局专项文档须同步在此登记。子项目内部文档除本表登记外，还需在其目录 `AGENTS.md` 中交叉引用。

## 五、变更记录

> **维护规则**：详细变更写入 `docs/changes/YYYY-MM-DD.md`，下表仅保留最近 6 条摘要；
> 表格只放「版本区间 + 一句话结论 + 详情指针」，逐条过程与证据一律留在 `docs/changes/` 里。

| 日期 | 变更摘要 |
| --- | --- |
| 2026-10-14 | Android 优化清单 P0 三项 + P1 两项落地（清单见 `docs/android-improvements.md`）。**P0**：① **下载接管**——新增 `patch-android-download.py`，经系统 `DownloadManager` 接管附件下载与长按存图（Cookie/UA 透传、API≤28 存储权限 maxSdk=28、API33+ 通知权限、blob 链接 toast 兜底），Manifest 权限随补丁注入；② **外部 scheme 外跳**——新增 `patch-android-schemes.py`，补丁 wry 0.57 的 `RustWebViewClient`，tel/mailto/weixin/alipays/intent/market 等交系统应用打开，支持 `browser_fallback_url` 与 market 网页回落，无应用时 toast 吞掉导航（不再 ERR_UNKNOWN_URL_SCHEME 白页）；③ **打包期硬失败**——解析器新增 `--target android`（Python + Node 复刻双侧）：URL 内嵌账号密码且关代理在 Android 必 401（非 Windows 导航分支无鉴权注入、wry 未覆写 `onReceivedHttpAuthRequest`），该组合打包即红并给两种修法，域名数组模式降级 warning。**P1（#4+#8）**：④ 新增 `patch-android-extras.py`（补丁链固定 insets→back→download→extras→schemes）：`__web2appAndroid` Kotlin 桥四方法 share/openBrowser/clearData/quitApp，全部 `webView.post` 回 UI 线程；清数据补 wry 0.57 漏掉的 Cookie/WebStorage（其 `clearAllBrowsingData` 配方不全），清完回调内 reload；tauri Android `PlatformWebview` 无清数据入口故不走 `web_data.rs`；安全模型 = `RustWebViewClient.onPageFinished` 主框架轮换一次性 UUID 密钥并 evaluateJavascript 注入，跨源 iframe 拿不到、桥方法首参核密钥。⑤ `mobilebar.js` 底栏加无条件第 5 格「更多」面板（刷新/复制链接/分享/浏览器打开/清数据两步确认 3s 撤防/退出）+ toast + 主框架门控，mobilebar.rs 零改动。契约测试新增 4 条、8 脚本 selftest 全入门禁、两 workflow 同步接入。**P1 第二批（#5+#6+#7）**：⑥ **运行时权限按需声明**——打包契约扩到 **21 键**，新增 `android_permissions`（白名单 camera/microphone/location，非敏感、进 Electron config.json 但桌面壳忽略；Python+Node 双侧 parity），新增 `patch-android-permissions.py` 把白名单值映射成 4 条 Manifest 权限（CAMERA / RECORD_AUDIO / FINE+COARSE_LOCATION），空数组零权限跳过；⑦ **H5 视频全屏**——新增 `patch-android-fullscreen.py`，整体替换 wry 0.57「进来就 onCustomViewHidden()」的空实现 onShow/onHide（custom view 铺满 + 传感器横屏 + API30 分代沉浸，退出逆序还原），模板已自带真全屏时显式报错；补丁链固定为 insets→back→download→extras→schemes→**fullscreen→permissions**。⑧ **`target=_blank`/`window.open` 进移动页签**——`mobilebar.js` document-start 装 blank guard（捕获阶段 click 命中 `_blank` 的 http(s) 链接即拦、包 window.open，非 http(s) 回落 schemes 外跳），发**新事件** `web2app:open-tab` `{url}`（区别于无 payload 落首页的 new-tab）；`mobilebar.rs` 只承接 http(s) 复用单 webview 页签记录，非页签模式不装 guard。契约测试再增 3 条（Rust 205 / Electron 101 全绿，parity 55/55）。详见 `docs/changes/2026-10-14.md` §10 |
| 2026-10-13 | 四批改动：① 多页签「首页页签」固定在第一位且不允许关闭（位次 0 方案，双壳四元组）。② 页签外观对齐 Chrome（圆角梯形、激活上浮阴影、短竖线分隔、圆点关闭）。③ 移动端底栏 `mobile_bar`：新增打包配置字段（20 键）、Tauri 移动端渲染 `mobilebar.js`/`mobilebar.rs`，列表按钮按页签实际可用性条件显示，桌面端接受但忽略；新增 10 条契约测试 + 390×844 Electron 渲染层几何取证。④ 移动端底栏真机三缺陷修复：贴底悬浮按钮 FAB 被切半截（fixed 贴底件去掉半屏宽门槛、保留原 px 底边距）；`tabs_enabled=true` 不出列表（移动端落地单 webview 页签记录：新建/切换/关闭 + 文档上报，解除 `TABS_ENABLED` 的 `cfg!(windows)` 钳制）；加载转圈遮住底栏（新增 `__web2appMobileBarH`/`chrome-ready` 协议，loading 层缩到底栏之上）。详见 `docs/changes/2026-10-13.md` |
| 2026-10-09 | v1.59–v1.65：Tauri 壳开启 WebView2 登录密码保存（`IsPasswordAutosaveEnabled`，对齐 Electron 默认行为）；Android 打包后「转圈十几秒 → 白屏」不再猜测根因，改为启动失败卡自带**现场诊断**（v1.61）；v1.62–v1.65 收敛启动期时序（加载层模式标记前置注入、`splash.localhost` 判定与相对路径重试、非 Windows 等首个文档就绪、导航落点校验 + 有限重发）。详见 `docs/changes/2026-10-09.md` |
| 2026-10-10 | v1.67–v1.71：桌面悬浮球从「悬浮小窗」纠正为**悬浮图标**（单击切换主窗显隐）；拖动改屏幕绝对坐标协议、悬停 300ms 延迟缩回、边界软钳制；v1.71 真机取证闭环，定位「球窗静默建不出」根因（builder 漏 `.data_directory()`，wry 回落默认 UDF 且错误被日志后端吞掉）并加契约守卫。详见 `docs/changes/2026-10-10.md` |
| 2026-10-11 | v1.72–v1.77b：悬浮球 P0/P1/P2 全量收口（Tauri 侧新增真机合成指针 harness `--float-probe`；Electron 四角穿透 `setShape` + 双击去抖）；关窗驻留理由纳入 `float_enabled`（只开悬浮球时关窗不再真销毁主窗）；球页不再挂原生提示（`title` → `role`+`aria-label`）；v1.77/1.77b 修「球窗带着任务栏按钮被显示」与「球未铺满活柱导致透明环带漏浅色残片」。详见 `docs/changes/2026-10-11.md` |
| 2026-10-12 | v1.78–v1.80：悬浮球「背景异形」连续四轮收口。v1.78 区域外扩 4px 漏浅色异形、v1.79 回退外加页面自绘黑底仍有锯齿 ⇒ **v1.80 弃掉绕路**：升级 tauri 2.12.1 / wry 0.57 / tao 0.37，球窗同开 `.transparent(true).no_redirection_bitmap(true)` 走**真逐像素 alpha**，并删除全部 `SetWindowRgn`/`CreateEllipticRgn` 改用 `WM_NCHITTEST` 子类化接管圆形命中（视觉与命中彻底分离，与 Electron 同构）。**同日另有一轮纯文档审计**（清理失效 / 过期 / 与实现不符内容，零代码改动：重建本表、字段数 18→19、探针与测试计数按实跑订正、`.gitignore` 与 `/tests/` 口径订正）；**同日第三轮完成 webview-app 代码优化闭环**（架构审查 P1-1：`log_startup` 等四个启动日志函数 `lib.rs` → `debug_log.rs` 并新增归属守卫 `startup_log_lives_in_debug_log_and_not_in_lib`，反向对照已验证；P1-3：三个仅 Android 使用的探测项改挂 `#[cfg(any(not(windows), test))]`，无 blanket 下 Windows 零告警；修复 3 文件 rustfmt 漂移；241 测试全绿，零行为变更）。详见 `docs/changes/2026-10-12.md` |

> 完整历史（含 v1.0–v1.29 仓库搭建与文档体系建立）见 `docs/changes/`。

