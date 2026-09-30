# 仓库总览与开发约定
> 本文为仓库根目录全局规范，统一所有子项目的协作规则、结构边界与文档体系，是全仓库开发协同的总纲领。各子项目内部规则在对应目录下独立维护，不得与全局规则冲突。

## 一、项目概述
本项目是 **web2app —— 把指定 Web 站点封装为桌面应用的「网页转桌面」壳**，采用双壳并行：

- `webview-app/`：Rust + Tauri 2 跨平台壳（Windows 为主战场，Android 交叉编译验证）；
- `electron-app/`：Electron + TypeScript 实现的 Windows 免安装绿色版壳（**M0–M7 全部交付**：M4 页面行为——下载接管 / 右键菜单 / F12，与 M7 CI 流水线于 2026-09-24 补齐）。

两个壳共享同一份前端真源与配置语义，对外表现为：自绘标题栏的独立窗口、托盘最小化、全局快捷键、多页签、下载接管、窗口几何持久化与设置页；入口地址由 DNS TXT + Ed25519 签名 / AES-256-GCM 解密下发，并支持本地反向代理统一注入鉴权。

## 二、仓库结构地图
| 目录路径   | 项目/模块属性 | 核心说明                 | 文档入口               |
| :--------- | :----------- | :----------------------- | :--------------------- |
| `webview-app` | Rust + Tauri 2 主工程 | 跨平台壳主工程：cargo workspace 四成员（`web2app` 主程序 / `resolve-core` 纯加密 / `resolve-tool` 服务端 CLI / `proxy-server` → Electron 侧 sidecar `w2a-proxy.exe`），含注入脚本、CI 与打包工作流 | `webview-app/README.md` |
| `electron-app` | Electron 绿色版工程 | 第二交付形态：TypeScript 主进程 / 渲染进程、sidecar 复用、打包与探针脚本（里程碑交付状态见 §一，权威值以本目录 README 为准） | `electron-app/README.md` |
| `docs`     | 文档索引清单   | 存放全仓库级专项详细文档（`docs/archive/` 存放已归档的历史方案稿） | 见下方`四、文档索引清单`       |
| `.github`  | CI 与打包工作流 | 全仓库统一的 GitHub Actions（2026-09-22 由 `webview-app/` 上移到仓库根；命令经 `defaults.run.working-directory: webview-app` 落到 Rust 工程目录，产物路径则显式带 `webview-app/` 前缀） | — |
| `scripts`  | 本地脚本（**不进 CI**） | 本机侧驱动脚本。当前只有 `pack-local.mjs`：读一份配置 JSON，在本机跑出与 CI **同语义**的产物（Tauri MSI/单文件、Electron zip/portable/NSIS 安装版；产物统一落仓库根 `tests/`，不进版本库）。⚠️ 它是 CI 解析器的 **Node 复刻**（Python 不是必然存在的依赖），两侧靠 `--parity` 逐项比对钉住等价 | `docs/packaging-config.md` §7.2 |
| `signing`  | 打包签名与发布凭据（**未纳入版本控制**） | keystore、打包配置与发布 secrets（**含敏感信息，不得外发**）；由仓库根 `.gitignore` 忽略 | — |

> 新增子项目/目录时，**必须在此表补充登记**，并在对应目录下维护独立的项目说明文档；目录废弃也需同步标记更新。

## 三、全局统一约定
### 3.1 开发与交付约定
1. 所有开发任务完成后，必须完成对应测试验证，测试通过后方可交付/合入主干。
2. 任务交付前必须同步更新对应文档，确保代码状态、功能逻辑与文档信息一致。

### 3.2 测试规范约定
1. 测试代码、测试用例统一存放于对应项目的 `tests` 目录下，不得混入业务代码目录。
   **例外（与现状对齐，2026-09-20）**：两壳的自动化断言都**必须在应用进程内执行**，因此不在 `tests/`：
   - `webview-app`：Rust 单元测试用 `#[cfg(test)]` 内联在 `src/*.rs` 内（Rust 惯例，`cargo test` 直接跑）；
   - `electron-app`：契约探针 `src/main/probe.ts`（`pnpm run probe`，命令行加 `--tabs` 走页签断言组）、
     纯函数单测 `test/*.test.ts`（`pnpm run unit`，node:test，零 Electron 依赖直接跑 TS）、
     冒烟与取证脚本 `scripts/*.cjs`（`smoke` / `start` 经 `launch.cjs` 拉起；`shot` / `lab` / `diag` / `png` / `tabs-v4` / `tabs-v4-negative`）。
2. `tests/` 目录只装**一次性验证产物**（探针日志、截图、产物副本）——**不是测试代码**；
   测试过程产生的临时文件、中间产物一律落到这里，不污染业务代码目录。
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
| 项目全盘分析与优化建议 | `docs/project-analysis.md` | 架构分析、模块职责地图、优化建议与遗留问题 |
| 全流程审计报告（2026-09-20） | `docs/audit-2026-09-20.md` | 「文档 / 实现 / 门禁一致性」审计：门禁对比、P0/P1 修复项、遗留事项与局限 |
| 全流程审计报告（2026-09-23） | `docs/audit-2026-09-23.md` | 设置界面重做 + 双壳契约同步一轮的审计：目标与验收判据、缺失/过期/冗余三类清单、逐项闭环记录、守卫反向对照与门禁实测证据 |
| 全流程审计报告（2026-09-27） | `docs/audit-2026-09-27.md` | 第五轮三类清单审计：Electron 6 项 P1（启动失败无限自动重试、关后台页签幽灵页签、托盘销毁不回滚、preload 子框架桥梁、public_key PEM 归一化与目录名清洗双壳漂移）+ Rust 3 项 P1（多页签标题 IPC 死路改事件、tab/宿主缺外部代理参数、直连凭据主机白名单）+ P2 健壮性/冗余；含逐项方案、门禁与反向对照证据、遗留项 |
| Web2App 使用说明 | `webview-app/README.md` | Tauri 版配置、构建、打包与使用 |
| Electron 绿色版说明 | `electron-app/README.md` | 绿色版配置、构建、验证与使用 |
| Electron 落地方案 | `docs/electron-plan.md` | M0–M7 里程碑与已锁定决策、兼容契约（权威状态以子项目 README 为准） |
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

> 新增文档时，**必须在此表登记**。子项目内部文档除本表登记外，还需在其目录 `README.md` 中交叉引用。

## 五、补充说明
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

## 六、变更记录
| 版本 | 日期 | 变更说明 |
| :--- | :--- | :------- |
| v1.0–v1.29 | 2026-09-15 ~ 2026-09-24 | 仓库搭建与文档体系建立、双壳（Tauri + Electron 绿色版）并行落地、打包配置 JSON 契约统一（18 字段 / 真 `bool` / `data_dir` 枚举）、设置页重做与单一 `settings.json`、capability 静态守卫、安全审计（本地代理密钥门禁）、本地打包由 Python 迁 Node（`pack-local.mjs`）。逐版细节见 `git log` 与历史审计文档（`docs/audit-2026-09-20.md` / `docs/audit-2026-09-23.md`）。 |
| v1.30 | 2026-09-24 | **M4 页面行为 + M7 CI 流水线交付（Electron 里程碑清零）**：①**M7**——新增 `.github/workflows/electron-ci.yml`（lint / e2e / package 三作业，push+PR 触发、path 过滤 `electron-app/**` 与共享真源 `webview-app/src/**`），过 `check-workflows.py` 校验（7 文件 / 77 run 步骤）；三条 CI 特有坑已取证并写进文件头注释：`--registry` 覆盖 `.npmrc` 的国内镜像、**不生成** `config.json`（实测无配置场景探针仍 130/130，验的正是「fresh clone 能跑」，探针自起 127.0.0.1 夹具站点）、pnpm 固定 v12；②**M4 下载接管**——`will-download` → `pause` → 系统「另存为」（默认落点=系统下载目录、按后缀收敛过滤器、开覆盖确认）→ `setSavePath`+`resume`，取消走 `cancel()` **绝不静默另存**（对标 `save_dialog.rs`）；⚠️ `pause()` 不可省，否则异步的 `showSaveDialog` 返回后 Electron 会再弹它自己的保存对话框；③**M4 右键菜单**——Electron 不暴露 Chromium 原生菜单故自建，取舍对齐 Tauri：不给打印 / 另存为 / 更多工具，带「设置」项（与 Shift+F1、托盘同一入口）；④**M4 F12**——`before-input-event` → `openDevTools({mode:'detach'})`，开关 `devtoolsEnabled()` = `!isPackaged \|\| debug`（对齐 Tauri `DEVTOOLS_ENABLED`；打包版可用 `debug:true` 打开是有意差异）；⑤实现落在新的 `src/main/page-behavior.ts`，**纯规则函数与 Electron 挂载分层**——纯规则由 `test/page-behavior.test.ts` 14 条毫秒级单测钉住（单测 15 → **29**），挂载点由探针 4 条断言验（探针 130 → **134**），并做**反向对照**（`if (false)` 禁掉 3 处挂载 ⇒ 4 条断言全 FAILED，还原后 sha256 与基线逐字节一致）；⚠️ **本轮事故与恢复**：反向对照后用 `git checkout -- src/main/index.ts` 还原，把该文件回退到 **HEAD**，连带丢失上一轮未提交的 v1.28 改动（凭据主机绑定、多页签恢复自愈 `presentMainWindow`、注释订正等）——靠 `out/main/index.js`（改动后编译产物）与「重编当前源码」做 diff 精确定位，再用 Python 原子脚本 11 处替换全部恢复，恢复后 sha256 与基线一致；**教训：还原未提交文件一律用「Python 备份字节 + 写回」，绝不用 `git checkout --`**；⑥门禁复测：Electron `typecheck` / `unit` **29** / 单窗口探针 **134** / 多页签 **26** / 冒烟全绿；Rust 侧本轮未改动 |
| v1.31 | 2026-09-24 | **本地打包入口改为 Node 实现（去掉 Python 依赖）**：①**动机**——`scripts/pack-local.py` 依赖 Python，但 Python **不是**「拿着这个仓库就一定有」的依赖：实测本机 PATH 里的 `python` 来自会话注入（`~/.workbuddy/binaries/python/…`），系统既无 `py.exe` launcher 也无任何 Python 安装 ⇒ 出会话即 `command not found`；而打 Electron 包**必然已有 node**（`electron-builder` 是 node 程序，`pnpm run sidecar/build/stage` 全是 node 脚本）⇒ 新增 **`scripts/pack-local.mjs`**（ESM、零第三方依赖、需 Node ≥ 18），删除 `pack-local.py`；②**代价与护栏**——解析逻辑从此有两份实现 ⇒ 内置 `--selftest`（契约自测，不需要 Python）与 `--parity`（对 **39 个 payload** 与 `.github/scripts/parse-packaging-config.py` 逐项比对：退出码 / `::warning::` 序列 / 错误文案 / `config.json` 字节；缺 Python 时明确报「跳过」而非假装通过），两者均接入 `ci.yml` 的 `config-contract` 作业（Linux 上两侧都是 LF ⇒ 比对逐字节；本机 Windows 的 Python 会把 `\n` 翻成 `\r\n`，`--parity` 把它降级为「仅行尾不同」的汇总提示并说明 CI 不受影响）；③**逐字复刻**——`difflib.SequenceMatcher` 相似度（近似名建议，**不是** Levenshtein）、Python `str()` / `repr()` 语义（`True` / `None` / 容器 repr）、全部 warning 与报错文案、`REMOVED_KEYS` 迁移指引、`data_dir` 空串保留语义；④**天然绕开的三个坑**——不落 env 文件 ⇒ `--env-out` 的 CRLF / heredoc / append 三坑不存在；⑤**新增加固**——传给子进程的环境里剥掉 `ELECTRON_RUN_AS_NODE`（本机 shell 里它是 `1`；对普通 node 无害，但会让子进程里任何 **electron 可执行文件**退化成 node 模式）；⑥**端到端实证**（`--config test2.json --target electron-all --out tests/pack-node`）：产物 `osfipin.test-0.1.0-win-x64.zip`（148M）/ `-portable.exe`（97M）文件名来自 JSON 的 `app_name`、`check-asar-config.cjs` 护栏通过（asar 24 条目）、包内 `out/config.json` **14 字段**（`tabs_enabled` 真 bool、`debug` 自动补默认值、无 `android_*`）、`electron-app/config.json` 打包后**逐字节还原**（sha256 `5b3458db455f`）；⑦文档：`docs/packaging-config.md` §7.3 重写（新增两条护栏对照表）、根 `README.md` §二 的 `scripts` 登记行同步 |
| v1.32 | 2026-09-24 | **本地打包「JSON 定义行为」两处纠正 + `--parity` 诊断补强**：①**图标判据**——`icon_url` 非空即下载并应用（判据与 CI 的 `if [ -z "$ICON_URL" ]` 逐字同源），**删除**原先额外要求的 `--icon` 开关（那等于把「配置说了算」变成「命令行说了算」：同一份 JSON 在 CI 带图标、本地不带），保留 `--no-icon` 作逃逸口；应用图标会**同时**覆盖 `webview-app/icons/icon.png`（Electron 托盘取自 `extraResources`，只设 `-c.win.icon` 对托盘无效），打包后整份还原（实测 sha256 `4cff2467101e` 一致）；②**输出目录不再需要人工介入**——先让 electron-builder 按常规流程清 `release/`，**只有它被删除策略拦住时**才自动换 `release/local-<时间戳>/` 重试一次。⚠️ 中途试过「脚本先自己清一遍」，实测**反噬**：本机删除有配额（`SAFE_DELETE_BULK_CONFIRM_REQUIRED`，50 次/轮），清一次上千个文件把配额烧光 ⇒ electron-builder 连自己的中间产物 `*.nsis.7z` 都删不掉，zip / portable **明明已产出**却以「失败」收尾；改为「不清、被拦才换目录」后同一环境 `PACK_EXIT=0`；③**`--parity` 诊断补强**——`spawnSync` 的 `error` 原先被吞，进程起不来时只报「退出码 null」；现区分「**未能验证**」（spawn 失败＝环境限制，退出码 2）与「语义不一致」（退出码 1）。本机实测：PATH 上那份 `python` 从 Node 里 spawn **恒定 EBUSY**（需兜底到绝对路径），连续 spawn 到第 ~37 次再次 EBUSY ⇒ 本机 `--parity` 可能留少量用例未验证（CI 是 Linux，无此限制）；④**端到端实证**（`--config test1.json --target electron-all`，`app_name=osfipin.test`，`icon_url` 是 256×256 PNG）：第一次尝试被 `safe-delete` 拦住 → **自动**换 `release/local-20260924-215645` 重试 → 产物 `osfipin.test-0.1.0-win-x64.zip`（147.7M）/ `-portable.exe`（96.7M）、`asar 配置护栏通过 ✓`、包内 `out/config.json` 与 test1.json 一致（`tabs_enabled` 为**假**，正好证明吃的是 test1 而非 test2；`debug`/`proxy_bypass` 为自动补的默认值；无 `android_*`）、`config.json` 与 `icons/icon.png` 均逐字节还原、全程 `PACK_EXIT=0`；⑤文档：`docs/packaging-config.md` §7.3 新增「本地行为一律由 JSON 决定」对照表 |
| v1.33 | 2026-09-25 | **本地打包工作区统一收敛到仓库根 `tests/`（不入库）**：①`scripts/pack-local.mjs` 的 Electron 默认输出目录由 `electron-app/release/` 改为**仓库根 `tests/pack/`**（win-unpacked 整树 + zip/portable，单次约 150MB / 上千文件）；根 `.gitignore` 的 `/tests/` 整目录忽略，`git check-ignore` 实测 `tests/pack/...`、`tests/pack-local-*/` 全部命中、**不进版本库**；CI 不受影响（一次性工作区仍走 `electron-builder.yml` 的 `electron-app/release/`）；②清空失败的自动兜底目录同步改为 `tests/pack-local-<时间戳>/`，手工提示改为 `--out ../tests/<名字>`（相对 `--out` 仍锚定 `electron-app/`）；③手工打包工作文件 `test1.json` / `test2.json` 移入 `tests/`（本地专属、不入库），脚本头注释、`docs/packaging-config.md` §7.3 示例命令、README §五.5 同步改为 `tests/test1.json`；④**CI 连带修正**——`ci.yml` 的 config-contract 样例校验循环移除这两个文件（入库后 CI 上不存在，`cat` 必失败），只守仍入库的两壳 `config.example.json`；⑤`--selftest` 的 `--out` 解析断言改为「`../tests/x` 解析到根 tests/ 且放行」，自测全绿、dry-run 实测默认输出绝对路径 `…\tests\pack` |
| v1.34 | 2026-09-25 | **全仓库文档精简与过期内容校正（只动文档，不动代码）**：①**计数一律实跑取证后更新** —— Rust `cargo test --all` **133 通过 / 6 ignored**（13+7+28+85）、Electron `node --test` **29**、探针**单窗口 134/134 与多页签 26/26** ⇒ 同步订正 `project-analysis.md`、`electron-analysis.md`、`electron-plan.md`、`electron-tabs-design.md`、`electron-tabs-test-steps.md`、`electron-size-reduction.md` 里的 112 / 113 / 117 / 126 / 128 / 129 / 130 等旧值；②**过期实现描述校正** —— Electron 配置「exe 同级可改」→ 随 `app.asar` 进包（改配置须**重新打包**）；多页签「点『+』新建」→ 该入口已确认**有意移除**，改由页面内 `window.open` / `target="_blank"` 触发；38px 让位写法 → **「`html` 不滚、`body` 滚」**（旧 `padding-top` 写法已废弃）；配置契约 17 → **18 字段**、布尔**只认真 `bool`**；③**行号 / 路径引用订正**（`proxy.rs`、`config.rs`、`resolve-core`、`lib.rs` 的 `TITLE` 等），改为「**符号名为主 + 行号为辅**」并注明行号会漂移；④历史快照类文档（`config-fields-2026-09-22.md` / `config-json-review-2026-09-22.md` / `audit-2026-09-20.md` / `audit-2026-09-23.md`）**保留原有结构与时点数字**，对已失效处加「已失效 / 已变更」标注而非删除；⑤工作流条数 6 → **7**（补 `electron-ci.yml`）；⑥无法判定的环境态内容（如 `win-unpacked/` 残缺一说、已不存在的 `project_memory.md` 引用）标注**待确认** |
| v1.35 | 2026-09-26 | **注入资源随 app.asar 分发 + 第三轮三类清单审计闭环（无外部输入、模糊项按通用默认补全）**：①**打包形态**（09-26 提交未登记行为，本版补登）：Electron 壳注入资源（titlebar.js / loading.js / splash.html / settings.html）改铺到 `out/inject/` 并随 `app.asar` 分发，asar 外不再有 `resources/inject/`；两壳共用真源 UI 打磨（自绘 CSS tooltip 替代原生 title、按钮 cursor:pointer、设置页文案压缩）；②**不足**：修 Electron 多页签 `closeTab` 邻接选择 bug（删除后才算索引恒 -1 → 关中间/关末尾都跳回第一个幸存页签，与 Tauri `close_tab_inner`「优先右邻」分叉）——纯规则抽到 `tab-order.ts` + 新增 `test/tabs.test.ts`（unit 29 → **35**），探针补 3 页签 P7b 断言（多页签 26 → **31**，含旧公式反向对照）；首个页签创建失败时壳页面注入**可见错误遮罩 + 重试**（`tabs:retry_first_tab`，治空白壳与 debug=false 双静默）；sidecar 握手成功行补 `proxyUrl` 合法性校验、stdin 异步 error 监听防 EPIPE 炸主进程；`applySettings` 失败时回滚本次新建的托盘；`tabs-v4.cjs` 修 node 下必崩（自动转 electron 承载）；③**冗余**：删死代码（`currentWakeAccelerator`、sidecar `upstreamHost/hadAuth` 死字段、托盘永不命中候选）、订正过期注释（settings-preload / settings-window / 探针计数 / proxy-server「825 行」等）；④**缺失/过期文档**：webview README workflow 6→7 并补 electron-ci.yml 与 config-contract 作业；tabs_enabled 示例改回真 bool（字符串示例照抄会被解析器拒）；删「+」按钮过期描述；electron README 修 asar 取证路径、config「重启即生效」自相矛盾句、探针「60 条/11–15 节」→「61 条/11–14 节」及全部计数；electron-plan / electron-size-reduction / electron-analysis 时点数字标注；门禁：Rust **133**（13+7+28+85，6 ignored）、Electron unit **35**、探针 134 与 31 |
| v1.36 | 2026-09-27 | **第四轮三类清单审计闭环（无外部输入，模糊项按通用默认补全）**：①**不足**——修「首个页签失败重试」的空白壳回归：`spawnFirstTab` 原先在**发起重试前**就撤掉失败遮罩，而重试是异步的（sidecar 解析可能数秒）⇒ 用户对着空白壳干等，正是该遮罩要消灭的失败形态；改为**仅成功时撤**，重试期间保留遮罩（按钮已切「重试中…」并置灰）提供持续反馈；②**冗余**——失败遮罩的 `top` 写死 `38px`、文案写死「32 个上限」，与 `TAB_BAR_HEIGHT` / `MAX_TABS` 三处同值可漂移（改栏高会让遮罩盖住标签栏）；改为从 `tabs.ts` 插值同源；③**缺失/过期**——根 README 变更表按版本升序排列，而 `v1.35`(09-26) 被误插在 `v1.34`(09-25) **之前**，读者会把 v1.34 当成更新的一版；两行对调恢复升序；④门禁复测：Electron `typecheck` 通过、`unit` **35/35** 全绿（本轮未触碰 Rust 侧与探针断言） |
| v1.37 | 2026-09-27 | **第五轮三类清单审计闭环（无外部输入，模糊项按通用默认补全）**：①**Electron 不足（6 项 P1）**——修启动失败后的**紧密无限自动重试**循环（失败重载 splash 又自动 bootstrap，sidecar 缺失时死循环；改为自动 bootstrap 一生一次，失败后等用户点重试）；修关闭**后台页签**后标签栏 DOM 残留幽灵页签（`closeTab` 非活动分支漏 `pushToShell`，探针补 P7c 2 条断言并做反向对照：撤推送实测「壳=3 集合=2」FAIL）；设置保存失败时**销毁的托盘也对称回滚**（best-effort 重建）；preload 收窄到**顶层框架**（子框架不注册 sendSync/contextBridge/监听，探针第 16 节新增 4 条 iframe 断言；实测 Electron 44 默认不向子框架注入 preload，该收窄为纵深防御+前向兼容）；`public_key` 补与打包解析器逐字同源的归一化（字面量 `\n`→真换行，PEM 域名模式此前在 Electron 必败）；`sanitizeDirName` 补齐「连续空白压缩 + 首尾句点剥离」对齐 Rust；②**Rust/Tauri 不足（3 项 P1）**——多页签真实标题回传原走应用命令，远程 origin 必被 ACL 拒且 `.catch` 静默（标签栏永远显示默认名）：改 `core:event` 事件 `web2app:tab-title` + 新增 `capabilities/tabs-remote.json`（glob `tab-*` + remote urls + 仅 `event:emit`）+ 一次性 token 防伪 + 4 条静态契约守卫（反向对照：事件名/权限改错均 FAIL）；多页签**宿主与 tab 两个 builder** 都挂 `additional_browser_args`（旧实现外部代理+TABS 组合下全部页签绕过代理，静态守卫钉两处调用点）；直连模式 Authorization 注入与 401 应答加**上游源白名单**（scheme+host+默认端口归一化纯函数，对齐 Electron login 既有规则）；③**P2 健壮性/冗余**——bootstrap scheme 校验对齐 sidecar；tab 首屏占位页空白化 + 加载层 splash 语义（失败态不再被 done 吞）+ 占位页重试事件 `web2app:retry-bootstrap`（token 校验，/retry 改为按请求窗口 label 引导）；缓存未命中的新 tab 改**等待**首屏缓存（消除并发 bootstrap 互杀代理竞态，30s 超时才回落）；`close_tab` 邻接选择抽纯函数穷举单测；代理响应路径剥 `Connection`/`Upgrade`/Connection 点名的逐跳头（101 走 WS 分支不受影响）；bootstrap 线程 `catch_unwind`（CSPRNG 等 panic 不再静默死线程）；托盘图标 expect 改 Err 链；代理 e2e 客户端 `no_proxy()`；删无调用方的 `store::save`、不可达分支/重复条件/过期注释；Electron 设置 IPC 补 sender 校验、`stopSidecar` 幂等（WeakSet 防定时器/监听叠加）、sidecar 握手后异常退出留日志；`WEB2APP_DEBUG` 真值集合两壳对齐（Electron 现接受 1/true/yes，单测钉住）；④**文档**——订正 `packaging-config`（release 触发实为 Publish Release 非 tag；§7 小节重排为 7.1/7.2/7.3）、`electron-size-reduction`（注入资源在 asar 非 extraResources；profile 行号改符号引用）、`project-analysis`（resolve_secret 是告警非 fail-fast；`tungstenite_msg_to_axum` 函数名）、`electron-analysis`（脚本 81KB、编排 ~1.56K 行复核值）、tabs 设计稿 extraResources 时点标注；新增 `docs/audit-2026-09-27.md`；⑤独立审查（R1 pass，1 Minor + 4 Advisory）后补：sidecar 正常停止不再误报「异常退出」（WeakSet 区分主动/意外退出）、目录名清洗空白集精确对齐 Rust White_Space（NEL/BOM 边界单测）、删 tabs.rs 最后一处冗余 cfg、订正 capability 注释里 URLPattern 端口语义（框架实测缺省端口段为通配；显式 `:*` 作为确定意图保留）；Electron login 主机白名单的 scheme 维度（A3）因 AuthInfo 不携带 scheme 且 mixed-content 兜底，列入遗留；⑥门禁：Rust fmt/clippy `-D warnings` 全绿、`cargo test --all` **144 通过 / 6 ignored**（13+7+29+95，较基线 +11）；Electron `typecheck` 通过、unit **40/40**（较基线 +5）、探针单窗口 **138/138**（+4 子框架）、多页签 **33/33**（+2 幽灵页签）；**遗留**：Tauri 多页签改动靠框架源码+静态守卫+Windows 编译，未做真机 GUI 目视（需 TABS_ENABLED=true 打包验证标题/代理/失败态）；非 Windows cfg 已静态核对，待 CI Ubuntu/Android 作业实证；Electron login 白名单 scheme 维度待 Electron API 具备条件后对齐 |
