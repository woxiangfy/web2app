# 仓库总览与开发约定

> 本文档为仓库全局通用规范。子项目可维护独立规范，但不得与本规范冲突；同一信息仅在一处维护，其他位置保留相对路径引用。

## 一、项目概述

* **项目名称**：**web2app** —— 把指定 Web 站点封装为桌面应用的「网页转桌面」壳（双壳并行）。
* **核心定位**：两个壳共享同一份前端真源与配置语义，对外表现为：自绘标题栏的独立窗口、托盘最小化、全局快捷键、多页签、下载接管、窗口几何持久化与设置页；入口地址由 DNS TXT + Ed25519 签名 / AES-256-GCM 解密下发，并支持本地反向代理统一注入鉴权。

  - `webview-app/`：Rust + Tauri 2 跨平台壳（Windows 为主战场，Android 交叉编译验证）；
  - `electron-app/`：Electron + TypeScript 实现的 Windows 免安装绿色版壳。

## 二、仓库结构

| 路径 | 类型 | 说明 | 文档入口 |
| --- | --- | --- | --- |
| `webview-app/` | 子项目 | Rust + Tauri 2 主工程：cargo workspace 四成员（`web2app` 主程序 / `resolve-core` 纯加密 / `resolve-tool` 服务端 CLI / `proxy-server` → Electron 侧 sidecar `w2a-proxy.exe`），含注入脚本、CI 与打包工作流 | `webview-app/AGENTS.md` |
| `electron-app/` | 子项目 | Electron 绿色版工程：第二交付形态（TypeScript 主进程 / 渲染进程、sidecar 复用、打包与探针脚本）；里程碑交付状态与权威值以本目录 `AGENTS.md` 为准 | `electron-app/AGENTS.md` |
| `docs/` | 全局文档 | 全仓库级专项详细文档（`docs/archive/` 存放已归档历史方案稿；`docs/changes/` 存放按日期归档的详细变更记录） | 见「四、文档索引」 |
| `.github/` | CI 与打包工作流 | 全仓库统一的 GitHub Actions（2026-09-22 由 `webview-app/` 上移到仓库根；命令经 `defaults.run.working-directory: webview-app` 落到 Rust 工程目录，产物路径则显式带 `webview-app/` 前缀） | — |
| `scripts/` | 本地脚本（**不进 CI**） | 本机侧驱动脚本（均为零第三方依赖的 Node 脚本）：①`init-env.mjs` —— 环境初始化：webview-app 跑 `cargo fetch`（默认仅宿主平台目标，避免拉 Linux/macOS/Android 无关 crate；`--all-targets` 恢复全平台，`--update` 先更新 Cargo.lock）、electron-app 跑 `pnpm install`（自动注入 electron / electron-builder 国内镜像环境变量，装完校验 electron 平台二进制并兜底），支持 `--webview` / `--electron` / `--frozen` / `--no-mirror` / `--check`；②`pack-local.mjs`（**入口薄壳**，实现拆在同目录 `packlib/` 的 10 个模块：constants / diagnostic / parser / pycompat / util / paths / driver-electron / driver-tauri / selftest / parity）：读一份配置 JSON，在本机跑出与 CI **同语义**的产物（Tauri MSI/单文件、Electron zip/portable/NSIS 安装版；产物统一落仓库根 `tests/`，不进版本库），⚠️ 它是 CI 解析器的 **Node 复刻**（Python 不是必然存在的依赖），两侧靠 `--parity` 逐项比对钉住等价 | `docs/packaging-config.md` §7.2 |
| `signing/` | 打包签名与发布凭据（**未纳入版本控制**） | keystore、打包配置与发布 secrets（**含敏感信息，不得外发**）；由仓库根 `.gitignore` 忽略 | — |

> 顶层目录变动时，须同步更新上表及对应项目的 `AGENTS.md`。

## 三、全局约定

### 3.1 开发与交付

* **同步更新**：代码、逻辑变动必须与测试用例、相关文档同步更新。
* **交付门槛**：所有变更必须在本地或构建环境测试通过后方可交付合入。

### 3.2 测试要求

* **目录存放**：测试代码、测试用例统一存放于对应项目的 `tests` 目录下，不得混入业务代码目录。
  **例外（与现状对齐，2026-09-20）**：两壳的自动化断言都**必须在应用进程内执行**，因此不在 `tests/`：
  - `webview-app`：Rust 单元测试用 `#[cfg(test)]` 内联在 `src/*.rs` 内（Rust 惯例，`cargo test` 直接跑）；
  - `electron-app`：契约探针 `src/main/probe.ts`（**入口编排**，断言组拆在 `probe-single.ts` / `probe-tabs.ts` / `probe-infra.ts` / `probe-server.ts`；`pnpm run probe`，命令行加 `--tabs` 走页签断言组）、
    纯函数单测 `test/*.test.ts`（`pnpm run unit`，node:test，零 Electron 依赖直接跑 TS）、
    启动经 `electron-app/scripts/launch.cjs`（`smoke` / `start`）；像素 / 布局取证脚本在 `electron-app/scripts/diag/`（`shot` / `lab` / `diag` / `png` / `tabs-v4` / `tabs-v4-negative`）。
* **环境干净**：`tests/` 目录只装**一次性验证产物**（探针日志、截图、产物副本）——**不是测试代码**；测试过程产生的临时文件、中间产物一律落到这里，不污染业务代码目录。
* **回归补充**：核心逻辑变更或 Bug 修复必须补充对应的测试用例，保障可回归验证。

### 3.3 三级文档体系

| 层级 | 路径 | 职责范围 |
| --- | --- | --- |
| **L1 根文档** | `AGENTS.md` | 仓库全局规则与通用规范，不记录子项目细节。 |
| **L2 项目文档** | `<project>/AGENTS.md` | 子项目专属业务、配置、接口与开发说明。 |
| **L3 专项文档** | `docs/` | 全局架构、部署流程等跨项目专题文档。 |

* **命名规则**：专项文档采用小写+中划线（如 `docs/architecture-design.md`）。
* **子文档拆分**：子项目复杂专题可放置于 `<project>/docs/`，但必须在 `<project>/AGENTS.md` 中登记入口。
* **单一维护**：同一信息仅在一处维护，其他位置保留相对路径引用；全局规则变更仅更新 L1 根文档。
* **同步生效**：文档更新与代码变更同步提交、同步生效。
* **登记义务**：新增 L3 文档须在「四、文档索引」登记；子项目内部文档除登记外，还需在其目录 `AGENTS.md` 中交叉引用。

### 3.4 其它全局约定

1. **单一真源**：注入脚本与页面（`titlebar.js`、`loading.js`、`shell.html`、`splash.html`、`dist/settings.html`、`icons/*`）
   及代理/解析链路（`proxy.rs` → sidecar）为两个壳共用，改动任一均须双壳同步验证，避免契约漂移。
2. **文本编码**：文档统一 UTF-8（无 BOM）、LF 换行，避免混用导致整文件 diff。
3. **路径引用**：跨目录引用一律相对仓库根目录书写（如 `docs/electron-plan.md`）；同目录内可直接写文件名。
4. **敏感文件**：`signing/` 下的 keystore、`secrets.env` 等凭据不得提交或外发。该目录已由仓库根 `.gitignore` **显式忽略**（与 `.workbuddy/`、`.trae/` 同），提交前请确认 `git status` 内不含 `signing/`。
5. **本地打包工作文件（不入库）**：手工打包用的 `test1.json` / `test2.json`（差异仅在 `tabs_enabled`）放在 **`tests/`** 下，与 `scripts/pack-local.mjs` 的本地产物（默认 `tests/pack/`，win-unpacked 整树 + zip/portable）同处一棵**被根 `.gitignore` 的 `/tests/` 整目录忽略**的本地工作区，不进版本库。入库的配置样例只有 `webview-app/config.example.json` 与 `electron-app/config.example.json`（CI 契约门禁校验这两份）。
6. **版本控制现状（2026-09-22 起：全项目单仓库）**：仓库根 = 本目录，远端 `git@github.com:woxiangfy/web2app.git`，两个壳同处一仓。
   - **沿革**：在此之前仓库根是 `webview-app/`，根目录与 `electron-app/` 均未纳管（`electron-app/` 长期无版本控制）。2026-09-22 把 `.git` 上移一级完成合并；**`webview-app/` 目录内部零改动**，其相对路径、构建脚本与 `.gitignore` 全部原样有效。
   - **上移方式**：`.git` 目录直移，**未做历史重写**。因此老文件在新路径下被 git 识别为重命名（40 个），但**逐文件历史需用 `git log --follow` 才能穿透**这次搬运。
   - **跨壳路径契约（禁改）**：`electron-app/package.json` 的 sidecar 构建 `--manifest-path ../webview-app/Cargo.toml`、`scripts/stage-assets.cjs` 取 `../webview-app/icons/`、注入脚本 / `settings.html` / `shell.html` / sidecar 产物全部指向 `webview-app/`。**两壳同级是这些相对路径成立的前提**，不得把 `electron-app/` 挪进 `webview-app/`。
   - **CI 位置**：统一在仓库根 `.github/workflows/`（原在 `webview-app/.github/`）。工作目录策略见该文件内注释。
   - **待办**：两壳版本号当前都是 `0.1.0` 且仓库尚无 tag，首次发版前需约定 tag 前缀（如 `tauri-v*` / `electron-v*`）以免撞车。

## 四、文档索引

| 文档 | 相对路径 | 说明 |
| --- | --- | --- |
| 项目全盘分析与优化建议 | `docs/project-analysis.md` | 架构分析、模块职责地图、优化建议与遗留问题 |
| 架构与组织结构审查（2026-09-29） | `docs/architecture-review-2026-09-29.md` | 模块边界与重复实现收敛审查：已执行 4 项（零行为变更）、待决策 4 项、刻意不动 3 项及依据 |
| Web2App 使用说明 | `webview-app/AGENTS.md` | Tauri 版配置、构建、打包与使用 |
| Electron 绿色版说明 | `electron-app/AGENTS.md` | 绿色版配置、构建、验证与使用 |
| 变更记录 | `docs/changes/` | 按日期归档的详细变更记录（`YYYY-MM-DD.md`）；根 `AGENTS.md`「五、变更记录」仅保留最近 6 条摘要 |
| Electron 落地方案 | `docs/electron-plan.md` | M0–M7 里程碑与已锁定决策、兼容契约（权威状态以子项目 `AGENTS.md` 为准） |
| Electron 多页签设计方案 | `docs/electron-tabs-design.md` | M5 多页签：A′ 架构定案（已实现并真机验收）、落地清单与验收判据 |
| Electron 多页签真机测试步骤 | `docs/electron-tabs-test-steps.md` | M5 真机验收清单（2026-09-18 完成，V1–V4 已关闭）+ 代理/域名解析待办 |
| Electron 方案分析 | `docs/electron-analysis.md` | 功能对等矩阵、复用策略、兼容性要点与安全基线 |
| 验签测试素材说明 | `docs/txttest-verify-materials.md` | DNS TXT 密文手工验签素材 |
| Electron 打包瘦身方案 | `docs/electron-size-reduction.md` | Electron 绿色版打包产物实测基线：语言包/DLL/压缩形态分级瘦身（P0 可直接落地） |
| **打包配置 JSON 契约** | **`docs/packaging-config.md`** | **打包配置的唯一契约说明**：18 个字段的类型/默认值/含义、多端（Windows / Android / Electron 绿色版）行为对照、校验规则、两条打包入口与迁移指引 |
| （过程稿·溯源）配置字段总表 | `docs/config-fields-2026-09-22.md` | 重构前 6 层配置面取证，结论已并入 `packaging-config.md`（已精简为溯源指针） |
| （过程稿·溯源）打包 JSON 评审 | `docs/config-json-review-2026-09-22.md` | 改造前配置 JSON 问题清单，结论已并入 `packaging-config.md`（已精简为溯源指针） |
| （历史）标题栏导航按钮设计 | `docs/archive/add_titlebar_nav_buttons.md` | 已归档，仅作追溯（2026-09-22 由 `.trae/documents/` 迁入） |
| （历史）诊断与优化 | `docs/archive/web2app-diagnosis-optimization.md` | 已归档，仅作追溯（2026-09-22 由 `.trae/documents/` 迁入） |

> 新增全局专项文档须同步在此登记。子项目内部文档除本表登记外，还需在其目录 `AGENTS.md` 中交叉引用。

## 五、变更记录

> **维护规则**：详细变更写入 `docs/changes/YYYY-MM-DD.md`，下表仅保留最近 6 条记录。

| 日期 | 变更摘要 |
| --- | --- |
| 2026-09-24 | v1.30–v1.32：M4 页面行为 + M7 CI 流水线交付（Electron 里程碑清零）、本地打包入口改为 Node 实现（去 Python 依赖）、本地打包「JSON 定义行为」纠正 + `--parity` 诊断补强。详见 `docs/changes/2026-09-24.md` |
| 2026-09-25 | v1.33–v1.34：本地打包工作区统一收敛到仓库根 `tests/`（不入库）、全仓库文档精简与过期内容校正（只动文档）。详见 `docs/changes/2026-09-25.md` |
| 2026-09-26 | v1.35：注入资源随 `app.asar` 分发 + 第三轮三类清单审计闭环。详见 `docs/changes/2026-09-26.md` |
| 2026-09-27 | v1.36–v1.37：第四、五轮三类清单审计闭环（Electron 6 项 P1 + Rust/Tauri 3 项 P1 + P2 健壮性/冗余）。详见 `docs/changes/2026-09-27.md` |
| 2026-09-29 | v1.38：结构重构（不改行为）+ 环境初始化脚本 `init-env.mjs`。详见 `docs/changes/2026-09-29.md` |
| 2026-10-07 | v1.39–v1.49：第六轮三类清单审计闭环（修复本地代理入口 URL 丢弃上游路径，含 2 条单测与反向对照 + 文档计数订正）；文档体系更名（三份 `README.md` → `AGENTS.md`）与全局引用/注释收尾、过期计数与示例订正；删除已过时的三份「全流程审计报告」（`docs/audit-*.md`）；密钥生成入口改为纯控制台输出、不落盘（`resolve-tool genkey --stdout` + 重写 `gen-resolve-keys.bat`，新增 2 条单测）；RESOLVE_SECRET 呈现由 64 字符 hex 改为 44 字符 base64（32 字节熵不变）；`encrypt` 支持内联 PEM 字符串入参（Rust/Python 双工具同步，新增 5 条单测）；TXT 密文紧凑化为 v3 格式（删冗余随机盐，实测 188→176 字符）；v1.46 撤销 v1.45 的三代兼容、解密端只认 v3（v1/v2 旧 TXT 需重签），`encrypt`/Python 双工具新增 `--secret` 逐次指定 Secret（客户端仍编译期注入），顺带纠正 txttest_keys 9/22 密钥不同步并重签 v3 固定向量；v1.47 `--ttl` 默认值 48h→0（默认永不失效，防重放需显式正整数）；v1.48 `resolve-tool` 新增 `decrypt` 解密验证子命令（GCM 认证 + Ed25519 验签，输出 URL/签发时间/TTL，支持 stdin 与公钥三态入参；resolve-core 同步新增结构化解密 API，客户端零行为变化）；**v1.49 TXT 升级为 v4「按天」格式：签发日 = 距 2026-01-01 的 UTC 天数、TTL 单位也是天，各占 u16（2B，今日已为第 279 天故 1B 不够；可覆盖至约 2205 年），尾部 16B→4B、固定开销 109B→97B，22 字符 URL 密文 176→160 字符；v4-only 不兼容 v3/v2/v1，固定向量已重签，线上旧 TXT（含 webnote2026.lassl.vip）需重新部署；172 测试通过 / 6 ignored**；txttest 真机 DNS 夹具已失效（域名 NXDOMAIN/旧记录+旧密钥作废，需重新部署）。详见 `docs/changes/2026-10-07.md` |

> 完整历史（含 v1.0–v1.29 仓库搭建与文档体系建立）见 `docs/changes/`。
