# 仓库总览与开发约定

> **本文件每次任务都会加载** ⇒ 只放「不知道就会踩坑的硬约束」+ 索引指针；专项细节一律下沉 `docs/`（由文档目录承担索引职责）。
> 子项目规范不得与本文件冲突；同一信息仅在一处维护，其余位置只留相对路径引用。

## 一、项目概述

**web2app** 把指定 Web 站点封装为桌面应用的「网页转桌面」双壳仓库，两壳共享同一份前端真源与配置语义；对外表现为自绘标题栏窗口、托盘最小化、全局快捷键、多页签、下载接管、窗口几何持久化与设置页；入口地址由 DNS TXT + Ed25519 签名 / AES-256-GCM 解密下发，并支持本地反向代理统一注入鉴权。

## 二、仓库结构

| 路径 | 说明 | 入口 |
| --- | --- | --- |
| `webview-app/` | Rust + Tauri 2 主工程（Windows + Android），cargo workspace 四成员 | [AGENTS.md](webview-app/AGENTS.md) |
| `electron-app/` | Electron + TS 的 Windows 免安装绿色版壳 | [AGENTS.md](electron-app/AGENTS.md) |
| `docs/` | 全仓库级专项文档（`archive/` 归档稿、`changes/` 按日变更记录） | 见「四」 |
| `.github/` | 统一 CI / 打包 workflow（工作目录落 `webview-app`，产物带 `webview-app/` 前缀） | — |
| `scripts/` | 零依赖 Node 本地脚本：`init-env.mjs` 环境初始化、`pack-local.mjs` 本地出包 | [local-scripts.md](docs/local-scripts.md) |
| `tests/` | 一次性产物落点（**非整目录忽略**；`ci-sdk-step/`、`android-build-step/`、`android-template-rehearsal/`、`build-*.bat` 有意入库） | §3.2 |
| `.workbuddy/`、`.trae/`、`signing/` | AI 记忆目录 / 历史 spec（均已入库）/ 签名凭据（**未入库**，本地按需创建） | — |

> 顶层目录变动时须同步更新本表及对应项目 `AGENTS.md`。

## 三、全局约定

### 3.1 开发与交付

代码 / 逻辑变动必须同步测试用例与文档；变更须在本地或构建环境测试通过后方可交付合入。

### 3.2 测试要求

- 测试代码放各项目规定位置（两壳自动化断言须在应用进程内跑，故不在 `tests/`，见各项目 `AGENTS.md`）；核心逻辑变更或 Bug 修复必须补测试用例。
- `tests/` **只装一次性产物**（探针日志 / 截图 / 产物副本）。⚠️ 根 `.gitignore` 是**逐条锚定**而非整目录忽略 —— 往 `tests/` 写一次性产物前先 `git status` 确认落在忽略规则内（教训见 `docs/changes/2026-10-12.md`）。

### 3.3 三级文档体系

L1 根 `AGENTS.md`（全局规则，不记子项目细节）→ L2 `<project>/AGENTS.md`（子项目业务 / 配置 / 接口 / 开发说明）→ L3 `docs/`、`<project>/docs/`（专题，命名小写 + 中划线）。
- 单一维护：同一信息仅一处维护，其余位置留相对路径引用；全局规则变更仅更新 L1。
- **登记义务**：全局 L3 文档须在「四」登记；子项目专题放 `<project>/docs/` 并在该项目 `AGENTS.md` 登记 + 交叉引用。

### 3.4 其它全局约定（硬约束，不知道就会踩坑）

1. **单一真源 · 改一必双壳验证**：注入脚本与页面（`titlebar.js`、`loading.js`、`tab_title.js`、`dnd_polyfill.js`、`shell.html`、`splash.html`、`floaticon.html`、`dist/settings.html`、`icons/*`）及代理 / 解析链路（`proxy.rs` → sidecar）为两壳共用。⚠️ 两个**例外**（单壳专属，改它只需过对应壳门禁 + 契约测试）：`mobilebar.js`（移动端底栏，Tauri 移动端专属）；`floaticon.html`（悬浮球页，**Electron 专属** —— Tauri 球 v1.81 起是 Win32 分层窗口 + 位图 alpha，不再加载任何页面）。
2. **文本编码**：文档统一 UTF-8（无 BOM）、LF 换行，避免混用导致整文件 diff。
3. **路径引用**：跨目录引用一律相对仓库根目录书写（如 `docs/electron-plan.md`）；同目录内可直接写文件名。
4. **敏感文件**：`signing/` 下 keystore、`secrets.env` 等凭据不得提交或外发（已由根 `.gitignore` 忽略）；提交前确认 `git status` 内不含 `signing/`。
5. **本地打包工作文件（不入库）**：手工打包用的 `test1.json` / `test2.json` 放 `tests/`。入库的配置样例只有两个 `config.example.json`（CI 契约门禁校验）；新建一次性目录前先 `git status` 复查忽略规则。
6. **单仓库 · 跨壳路径契约（禁改）**：仓库根 = 本目录（远端 `git@github.com:woxiangfy/web2app.git`），两壳同处一仓（2026-09-22 由 `webview-app/` 上移，**未做历史重写**，逐文件历史需 `git log --follow`）；CI 统一在根 `.github/workflows/`。`electron-app/` 的 sidecar 构建 `--manifest-path ../webview-app/Cargo.toml`、`stage-assets.cjs` 取 `../webview-app/icons/`、注入脚本 / `settings.html` / `shell.html` / sidecar 产物全指向 `webview-app/` —— **两壳同级是前提**，不得把 `electron-app/` 挪进 `webview-app/`。发版待办：两壳版本号均 `0.1.0` 且无 tag，首次发版前需约定 tag 前缀（`tauri-v*` / `electron-v*`）。

## 四、文档索引

| 文档 | 路径 |
| --- | --- |
| 项目全盘分析与优化建议 / 架构与组织结构审查（2026-09-29） | `docs/project-analysis.md`、`docs/architecture-review-2026-09-29.md` |
| webview-app 架构与组织结构审查（2026-10-25，模块/依赖/复用） | `docs/architecture-review-2026-10-25.md` |
| **本地脚本说明** | **`docs/local-scripts.md`** |
| **打包配置 JSON 契约（21 字段，跨壳唯一权威）** | **`docs/packaging-config.md`** |
| **Android 端优化清单（唯一任务清单）** | **`docs/android-improvements.md`** |
| Electron 方案族：落地 M0–M7 / 分析 / 多页签设计 / 多页签真机 / 打包瘦身 | `docs/electron-plan.md`、`docs/electron-analysis.md`、`docs/electron-tabs-design.md`、`docs/electron-tabs-test-steps.md`、`docs/electron-size-reduction.md` |
| 验签测试素材 / 溯源稿（配置字段总表、打包 JSON 评审）/ 历史归档 | `docs/txttest-verify-materials.md`、`docs/config-fields-2026-09-22.md`、`docs/config-json-review-2026-09-22.md`、`docs/archive/` |
| 变更记录（按日归档 `YYYY-MM-DD.md`） | `docs/changes/` |
| Web2App（Tauri）使用说明 / Electron 绿色版说明 | `webview-app/AGENTS.md`、`electron-app/AGENTS.md` |

> 新增全局专项文档须在此登记；子项目内部文档除本表登记外，还需在其目录 `AGENTS.md` 中交叉引用。

## 五、变更记录

> 详细变更写入 `docs/changes/YYYY-MM-DD.md`，此处仅留最近 6 条一句话摘要 + 指针。

| 日期 | 摘要 |
| --- | --- |
| 2026-11-01 | 修 CI「Build Android APK」零产物：注入模板里的 `val w2aScriptsDir = java.io.File(...)` 在 `build.gradle.kts` 里解析成**扩展访问器 `java` 的成员**（不是包名）⇒ `Unresolved reference 'io'`（`108:26`），gradle **配置期**脚本编译失败 ⇒ 任务图建不起来。改 `w2aRepoRoot.resolve(".github/scripts")`，自测补「禁 `java.io.*`」守卫（匹配前先剥 `//` 注释，否则针被注释满足）→ `docs/changes/2026-11-01.md` |
| 2026-10-28 | Android 打包三连：① `config_url` 误填 JSON 的 `URLError`（解析器改「先判形态再取内容」）；② `cargo tauri android build \| tee` 失败被**吞成绿色**（缺 pipefail）⇒ 下游假报「未找到 *-unsigned.apk」，现补 `shell: bash` + `set -o pipefail` + 根因摘录 + 产物校验；③ 定性「零产物 = gradle 未启动」，加**环境指纹**步骤（tauri-cli/rustc/JDK/模板 AGP·Gradle·Kotlin/SDK/NDK）+ `W2A_SKIP_GRADLE_PATCH` 二分开关，并用**真模板**离线重放补丁链 11/11；连带 `cargo fmt --all`（v1.81 遗留）→ `docs/changes/2026-10-28.md` |
| 2026-10-27 | 悬浮球 v1.81 根治边缘毛刺：Tauri 球弃 WebView2 改 **Win32 分层窗口**（`UpdateLayeredWindow` 预乘 ARGB + 4×4 超采样圆盘，像素 alpha 穿透），Electron 球改 canvas 超采样 alpha 位图；盘面弃蓝色渐变改跟随系统深浅色白/黑；吸附扩为**四边**就近择一（`DockEdge` 增 top/bottom），`floaticon.html` 转 Electron 专属、`capabilities/float-icon.json` 退役 → `docs/changes/2026-10-27.md` |
| 2026-10-25 | webview-app 架构审查落盘 + 优化批次①②③④（P1–P4 全落地）：contract_tests.rs 拆 13 个领域子模块、打包配置字段表单源化（`scripts/packaging-fields.json`）、新建 `nav.rs` 切断 bootstrap 双向耦合（六模块归零）、electron 样例 17 键差异定性、debug_log/floatprobe 口径钉死、floatwin/tabs/proxy 三大文件垂直拆分（子模块化，proxy `#[path]` 双壳同源不破），Rust 282 全绿 / parity 55/55 → `docs/changes/2026-10-25.md` |
| 2026-10-24 | electron-app 架构审查优化批次①–⑤ 落地（updateDocument 统一读-改-写 / index.ts 装配化 / probe-single 19 段函数化 / 设置页命令名三方守卫 / util.ts 与 protocols.ts 依赖收口 / 悬浮球决策纯函数化），unit 115/115、probe 149/150（预存环境项）、tabs 47/47 → `docs/changes/2026-10-24.md` |
| 2026-10-15 | 修 Android 打包 `patch-android-extras.py` 的 `FileNotFoundError`（补丁链前置 `prepare-wry-kotlin.py` + build 后重放 task），连带修 Gradle API / 变体配对 / cargo metadata 缺陷；selftest 12/12 → `docs/changes/2026-10-15.md` |

> 完整历史（含 v1.0–v1.29 仓库搭建与文档体系建立）见 `docs/changes/`。
