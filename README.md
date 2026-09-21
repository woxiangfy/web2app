# 仓库总览与开发约定
> 本文为仓库根目录全局规范，统一所有子项目的协作规则、结构边界与文档体系，是全仓库开发协同的总纲领。各子项目内部规则在对应目录下独立维护，不得与全局规则冲突。

## 一、项目概述
本项目是 **web2app —— 把指定 Web 站点封装为桌面应用的「网页转桌面」壳**，采用双壳并行：

- `webview-app/`：Rust + Tauri 2 跨平台壳（Windows 为主战场，Android 交叉编译验证）；
- `electron-app/`：Electron + TypeScript 实现的 Windows 免安装绿色版壳（M0–M3、M5 多页签、M6 打包已交付；**M4 部分交付**——新窗口策略与加载层随 M1 落地，下载接管 / 右键菜单 / F12 未做；M7 CI 待做）。

两个壳共享同一份前端真源与配置语义，对外表现为：自绘标题栏的独立窗口、托盘最小化、全局快捷键、多页签、下载接管、窗口几何持久化与设置页；入口地址由 DNS TXT + Ed25519 签名 / AES-256-GCM 解密下发，并支持本地反向代理统一注入鉴权。

## 二、仓库结构地图
| 目录路径   | 项目/模块属性 | 核心说明                 | 文档入口               |
| :--------- | :----------- | :----------------------- | :--------------------- |
| `webview-app` | Rust + Tauri 2 主工程 | 跨平台壳主工程：cargo workspace 三 crate（`web2app` 主程序 / `resolve-core` 纯加密 / `resolve-tool` 服务端 CLI）+ `proxy-server` sidecar，含注入脚本、CI 与打包工作流 | `webview-app/README.md` |
| `electron-app` | Electron 绿色版工程 | 第二交付形态：TypeScript 主进程 / 渲染进程、sidecar 复用、打包与探针脚本（M0–M3、M5、M6 已交付；M4 部分交付、M7 待做） | `electron-app/README.md` |
| `docs`     | 文档索引清单   | 存放全仓库级专项详细文档（`docs/archive/` 存放已归档的历史方案稿） | 见下方`四、文档索引清单`       |
| `.github`  | CI 与打包工作流 | 全仓库统一的 GitHub Actions（2026-09-22 由 `webview-app/` 上移到仓库根；命令经 `defaults.run.working-directory: webview-app` 落到 Rust 工程目录，产物路径则显式带 `webview-app/` 前缀） | — |
| `signing`  | 打包签名与发布凭据（**未纳入版本控制**） | keystore、打包配置与发布 secrets（**含敏感信息，不得外发**）；由仓库根 `.gitignore` 忽略 | — |
| `.trae` | IDE 目录（**未纳入版本控制**） | Trae IDE 工作区数据；原归档方案稿已于 2026-09-22 迁至 `docs/archive/` | — |
| `.workbuddy` | 协作记忆（非交付物，**未纳入版本控制**） | 本地工作记忆与经验沉淀，不随代码交付；由仓库根 `.gitignore` 忽略 | — |

> 新增子项目/目录时，**必须在此表补充登记**，并在对应目录下维护独立的项目说明文档；目录废弃也需同步标记更新。

## 三、全局统一约定
### 3.1 开发与交付约定
1. 所有开发任务完成后，必须完成对应测试验证，测试通过后方可交付/合入主干。
2. 任务交付前必须同步更新对应文档，确保代码状态、功能逻辑与文档信息一致。

### 3.2 测试规范约定
1. 测试代码、测试用例统一存放于对应项目的 `tests` 目录下，不得混入业务代码目录。
   **例外（与现状对齐，2026-09-20）**：两壳的自动化断言都**必须在应用进程内执行**，因此不在 `tests/`：
   - `webview-app`：Rust 单元测试用 `#[cfg(test)]` 内联在 `src/*.rs` 内（Rust 惯例，`cargo test` 直接跑）；
   - `electron-app`：契约探针 `src/main/probe.ts`、冒烟与取证脚本 `scripts/*.cjs`（`probe` / `smoke` / `shot` / `lab` / `tabs-v4`）。
   `tests/` 目录用于存放**一次性验证产物**（探针日志、截图、产物副本），见的不是"测试代码"。
2. 测试过程产生的临时文件、中间产物全部放置在 `tests` 目录内，确保不污染实际业务代码。
3. 核心功能变更必须补充对应的测试用例，保障可回归验证。

### 3.3 文档管理约定
1. **三级文档权责划分**：
   - 根目录 `README.md`：仅维护**跨项目公共规则、仓库总览、全局约定**，不涉及子项目业务细节。
   - 子项目目录 `README.md`：维护该项目的业务逻辑、开发步骤、接口规范、配置参数等专属内容。
   - 全局 `docs` 目录：存放流程规范、架构设计、部署手册等更详细的专项文档，需在本文档「文档索引清单」中登记。
2. 全局规则变更仅更新根目录文档。
3. 文档更新与代码变更同步提交、同步生效。

## 四、文档索引清单
| 文档名称 | 文档路径 | 说明 |
| :------- | :------- | :--- |
| 项目全盘分析与优化建议 | `docs/project-analysis.md` | 架构分析、模块职责地图、优化建议与待复验清单 |
| 全流程审计报告（2026-09-20） | `docs/audit-2026-09-20.md` | 本轮「文档 / 实现 / 门禁一致性」审计：问题清单（不足·冗余·缺失）、P0/P1/P2 改动方案与验证证据、遗留建议与本轮局限 |
| Web2App 使用说明 | `webview-app/README.md` | Tauri 版配置、构建、打包与使用 |
| Electron 绿色版说明 | `electron-app/README.md` | 绿色版配置、构建、验证与使用 |
| Electron 落地方案 | `docs/electron-plan.md` | M0–M7 里程碑划分与已锁定决策 |
| Electron 多页签设计方案 | `docs/electron-tabs-design.md` | M5 多页签：选型对比、本地实测依据、落地清单与验收判据 |
| Electron 多页签真机测试步骤 | `docs/electron-tabs-test-steps.md` | M5 T0 之后：打包产物自证 + V1/V2/V4 目视项的真机验收清单 |
| Electron 方案分析 | `docs/electron-analysis.md` | 与 Tauri 版对齐的选型与兼容性分析 |
| 验签测试素材说明 | `docs/txttest-verify-materials.md` | DNS TXT 密文手工验签素材 |
| Electron 打包瘦身方案 | `docs/electron-size-reduction.md` | Electron 绿色版打包产物实测基线：语言包/DLL/压缩形态分级瘦身（P0 可直接落地） |
| （历史）标题栏导航按钮设计 | `docs/archive/add_titlebar_nav_buttons.md` | 已归档，仅作追溯（2026-09-22 由 `.trae/documents/` 迁入） |
| （历史）诊断与优化 | `docs/archive/web2app-diagnosis-optimization.md` | 已归档，仅作追溯（2026-09-22 由 `.trae/documents/` 迁入） |

> 新增文档时，**必须在此表登记**。子项目内部文档除本表登记外，还需在其目录 `README.md` 中交叉引用。

## 五、补充说明
1. **单一真源**：`titlebar.js`、`loading.js`、`shell.html`、`splash.html` 及代理/解析链路（`proxy.rs` → sidecar）为两个壳共用，改动任一均须双壳同步验证，避免契约漂移。
2. **文本编码**：文档统一 UTF-8（无 BOM）、LF 换行，避免混用导致整文件 diff。
3. **路径引用**：跨目录引用一律相对仓库根目录书写（如 `docs/electron-plan.md`）；同目录内可直接写文件名。
4. **敏感文件**：`signing/` 下的 keystore、`secrets.env` 等凭据不得提交或外发。该目录已由仓库根 `.gitignore` **显式忽略**（与 `.workbuddy/`、`.trae/` 同），提交前请确认 `git status` 内不含 `signing/`。
5. **根目录散件**：`test1.json` / `test2.json` 为手工打包工作流的 `config.json` 配置样例（差异仅在 `tabs_enabled`）。二者**已纳入版本控制**（内容仅示例网址，无凭据）。
6. **版本控制现状（2026-09-22 起：全项目单仓库）**：仓库根 = 本目录，远端 `git@github.com:woxiangfy/web2app.git`，两个壳同处一仓。
   - **沿革**：在此之前仓库根是 `webview-app/`，根目录与 `electron-app/` 均未纳管（`electron-app/` 长期无版本控制）。2026-09-22 把 `.git` 上移一级完成合并；**`webview-app/` 目录内部零改动**，其相对路径、构建脚本与 `.gitignore` 全部原样有效。
   - **上移方式**：`.git` 目录直移，**未做历史重写**。因此老文件在新路径下被 git 识别为重命名（40 个），但**逐文件历史需用 `git log --follow` 才能穿透**这次搬运。
   - **跨壳路径契约（禁改）**：`electron-app/package.json` 的 sidecar 构建 `--manifest-path ../webview-app/Cargo.toml`、`scripts/stage-assets.cjs` 取 `../webview-app/icons/`、注入脚本 / `settings.html` / `shell.html` / sidecar 产物全部指向 `webview-app/`。**两壳同级是这些相对路径成立的前提**，不得把 `electron-app/` 挪进 `webview-app/`。
   - **CI 位置**：统一在仓库根 `.github/workflows/`（原在 `webview-app/.github/`）。工作目录策略见该文件内注释。
   - **待办**：两壳版本号当前都是 `0.1.0` 且仓库尚无 tag，首次发版前需约定 tag 前缀（如 `tauri-v*` / `electron-v*`）以免撞车。

## 六、变更记录
| 版本 | 日期 | 变更说明 |
| :--- | :--- | :------- |
| v1.0 | 2026-09-15 | 初始版本 |
| v1.1 | 2026-09-17 | 补全项目概述、仓库结构地图与文档索引清单；新增补充说明（单一真源、编码、敏感文件等）；规则条文未改动 |
| v1.2 | 2026-09-17 | 专项文档归位至 `docs/`：`PLAN.md`→`docs/electron-plan.md`、`ANALYSIS.md`→`docs/electron-analysis.md`、`txttest_verify_materials.md`→`docs/txttest-verify-materials.md`、`docs/project.md`→`docs/project-analysis.md`；清单路径与引用同步 |
| v1.3 | 2026-09-17 | 全部文档与仓库现状逐条核对并优化：`project-analysis.md` 补 13 项建议落地状态与实测数据；`txttest-verify-materials.md` 补 ttl 字段、改用 `resolve-tool`；`electron-app/README.md`、`webview-app/README.md` 勘误命令/路径/配置字段；`electron-plan.md` 同步进度并标注被取代结论 |
| v1.4 | 2026-09-17 | 新增 `docs/electron-tabs-design.md`（M5 多页签设计方案）并登记入索引清单 |
| v1.5 | 2026-09-17 | `electron-tabs-design.md` 升 v1.1：**社区核查后更正选型**——由「`BaseWindow` + 独立壳视图」改为「`BrowserWindow` 当壳 + 子 `WebContentsView`」（新增 §2.3 落地实现核查、§3.5 补充实测 8 项）；`electron-app/README.md` 的 M5 行同步 |
| v1.6 | 2026-09-17 | `electron-tabs-design.md` 升 v1.2：**T0 完成**——新增 `electron-app/scripts/tabs-v4.cjs`，用 `desktopCapturer` 系统级抓屏 + 像素断言把「子视图绘制顺序」（V4）从目视项降级为可自动断言，正反两向对照通过 ⇒ A′ 定案、electron#41044 未回归 |
| v1.7 | 2026-09-17 | 新增 `docs/electron-tabs-test-steps.md`（M5 T0 之后的真机测试步骤：打包产物自证 + V1/V2/V4 目视项）并登记入索引清单 |
| v1.8 | 2026-09-17 | **M5 T1 骨架完成**：`electron-tabs-design.md` 升 v1.3；新增 `electron-app/src/main/tabs.ts`（TabManager）、`src/renderer/shell-preload.ts`（壳命令垫片）；`index.ts` 接入 `createWindow()` 分叉 + `shell://` 协议 + 壳命令 sender 校验 + `resolveInitialTarget()`；`electron-builder.yml` 加回 `shell.html`。单窗口 `probe` 112/112 回归通过、多页签 `smoke` 退出码 0 |
| v1.9 | 2026-09-17 | **M5 T2 页签管理完成**：`electron-tabs-design.md` 升 v1.4；`probe.ts` 新增 `runTabsProbe`（`--tabs` 标志，19 条断言）；修复两处缺陷（`closeTab` 补显式 `removeChildView`、`get_tabs` 契约元组化）；单窗口 112/112 零回归、多页签 19/19 |
| v1.10 | 2026-09-17 | **M5 T3/T4 完成（多页签收尾）**：`electron-tabs-design.md` 升 v1.5；补 P9/P10/P11 断言（window.open 单页签、target path 保留、特权命令 sender 拒绝）+ 内容视图 `did-fail-load` + 探针专用 `tabs-probe-preload.ts`；打包产物（dir/zip/portable）双模式探针退出码 0。多页签探针 **23/23**、单窗口 112/112 零回归 |
| v1.11 | 2026-09-18 | **M6 结清 + 审计修复**：①启动日志落盘（`electron-app/src/main/startup-log.ts` → `data\logs\startup.log`，fail-open / 1 MiB 轮转 / 探针隔离到 `%TEMP%`），`smoke` 增硬断言；②版本信息经 VERSIONINFO 实证**本就已生效**（原「待补」判断有误），`electron-plan.md` §5/§8.8 按实证订正；③SmartScreen 文案补入 `electron-app/README.md`；④审计修复 `renderSplash` 的 `String.replace` 注入模式、`tabs_enabled` 过期注释、重复文案表、`contentBounds` 负高度。复测：单窗口 112/112、多页签 23/23、冒烟 0 回归 |
| v1.12 | 2026-09-20 | **文档索引补登 + webview-app 死代码清理**：①`docs/electron-size-reduction.md`（2026-09-18 生成）补登记入「四、文档索引清单」；②删除 `webview-app/src/config.rs` 中已无人引用的 `HOTKEY_CTRL/ALT/SHIFT/KEY` 四个编译期死常量（快捷键早已改为设置页落盘），同步订正 `webview-app/README.md` 与 `lib.rs` 的过期注释；复测 `cargo test --all` 全绿 |
| v1.13 | 2026-09-20 | **文档/实现一致性 + CI 门禁审计（本轮全流程）**：①修平**既有**的 `cargo fmt --all -- --check` 违规 5 处 / 4 文件（`proxy-server/main.rs`×2、`proxy.rs`×1 尾随空格、`bootstrap.rs`×1、`settings.rs`×1）—— 该命令是 CI 阻断门禁，此前在本机就是红的；②`electron-app/` 源码与 `tsconfig.json` 中 10 处失效引用 `PLAN.md` 归正为 `docs/electron-plan.md`，注释内 `npm run *` 统一为 `pnpm run *`（`electron-builder.yml` 同步）；③`webview-app` 把 `cfg(not(target_os = "android"))` 改用 Tauri 惯用 `cfg(desktop)`，与 `Cargo.toml` 的 `not(any(android, ios))` 门控对齐（原写法在 iOS 目标上会引入「模块在、依赖不在」的编译隐患）；④`electron-app/.gitignore` 补 `tests/*.log`、`tests/*.exe`、`tests/*.zip`（原 `tests/*/` 只忽略目录，~96MB 便携版副本与 10 个探针日志会被提交）；⑤删除 `webview-app/src/loading.rs` 的 `pub fn hide` 死函数；⑥订正过期文档结论：`electron-plan.md` 头部新增「文档状态」表（M4 改「部分交付」、`BaseWindow` 选型已推翻、`.npmrc` 配镜像对 pnpm 无效、体积 320MB→367.67MB、无 GPU 环境必须加 `--no-sandbox --in-process-gpu`），`electron-analysis.md`（「空目录」/「18 字段」）、`electron-tabs-design.md`（V1–V3 已关闭）同步；⑦README 交叉引用补齐（`electron-app/README.md` ↔ `docs/electron-size-reduction.md`、根 README；`webview-app/README.md` → 根 README），并声明「里程碑状态以子项目 README 为准」；⑧新增 `docs/audit-2026-09-20.md`（本轮全流程审计报告）并登记入「四、文档索引清单」 |
| v1.14 | 2026-09-22 | **全项目单仓库化（`.git` 上移一级，未重写历史）**：①仓库根由 `webview-app/` 上移到本目录，`electron-app/`（此前**无任何版本控制**）与根目录 `docs/`、`test1.json`、`test2.json` 首次入库，实现「双壳同处一仓」，且 `webview-app/` **目录内部零改动**；②`.git` 直移 ⇒ 40 个老文件被 git 识别为重命名，历史总量保留，但逐文件历史需 `git log --follow` 穿透；③新增**仓库根 `.gitignore`**（在首次 `git add` 之前落地，显式忽略 `signing/` 明文凭据、`.workbuddy/` 本地记忆、`.trae/` IDE 目录），并用 `git check-ignore` 逐条验证、census 暂存区零敏感项；④`.github/` 迁至仓库根：`ci.yml`/`release.yml`/`build-custom.yml`/`resolve-keys.yml` 统一加 `defaults.run.working-directory: webview-app`；因 `defaults.run` **不作用于 `uses:`**，`action-gh-release` 的 `files:` 与两处 `upload-artifact` 的 `path:` 必须手改加 `webview-app/` 前缀；`generate-signing-key.yml` 无 `checkout`（工作区无 `webview-app/`），**刻意不设**该默认值并注释说明；⑤`.trae/documents/` 两份归档稿迁入 `docs/archive/` 并在索引清单改路径；⑥§二 结构地图、§四 索引、§五.6 同步（新增「跨壳路径契约禁改」与 tag 前缀待办）。 |
