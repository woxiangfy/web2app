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

1. **单一真源 · 改一必双壳验证（按「是否真被两壳消费」分三类，别混为一谈）**：
   - **真共用**（`electron-app/scripts/stage-assets.cjs` 打包时从 `webview-app/` 复制）：`titlebar.js`、`loading.js`、`shell.html`、`splash.html`、`dist/settings.html`、`icons/*` —— 改动**必须双壳同步验证**。
   - **行为契约对齐、实现不同**：Ctrl+W 关当前页签（Tauri 注入 `tab_keys.js` → 核心事件 `web2app:close-tab`；Electron 主进程 `before-input-event`）、页签真实标题回传（`tab_title.js` vs Electron 原生）—— 改一侧必须对照另一侧行为，**文件本身不共用**。
   - **单壳专属**：`mobilebar/*.js`（Tauri 移动端；5 分片由 `lib.rs` 按序拼成同一 IIFE）、`tab_title.js` / `tab_keys.js` / `dnd_polyfill.js`（WebView2/Tauri 注入，Electron **不消费**）、`floaticon.html`（**Electron 专属** —— Tauri 球 v1.81 起是 Win32 分层窗口 + 位图 alpha，不加载任何页面）。

   代理 / 解析链路（`proxy.rs` / `resolve.rs` → sidecar）走 `#[path]` **同源复用**，改一必双壳验证。
2. **文本编码**：文档统一 UTF-8（无 BOM）、LF 换行，避免混用导致整文件 diff。
3. **路径引用**：跨目录引用一律相对仓库根目录书写（如 `docs/electron-plan.md`）；同目录内可直接写文件名。
4. **敏感文件**：`signing/` 下 keystore、`secrets.env` 等凭据不得提交或外发（已由根 `.gitignore` 忽略）；提交前确认 `git status` 内不含 `signing/`。
5. **本地打包工作文件（不入库）**：手工打包用的 `test1.json` / `test2.json` 放 `tests/`。入库的配置样例只有两个 `config.example.json`（CI 契约门禁校验）；新建一次性目录前先 `git status` 复查忽略规则。
6. **单仓库 · 跨壳路径契约（禁改）**：仓库根 = 本目录（远端 `git@github.com:woxiangfy/web2app.git`），两壳同处一仓（2026-09-22 由 `webview-app/` 上移，**未做历史重写**，逐文件历史需 `git log --follow`）；CI 统一在根 `.github/workflows/`。`electron-app/` 的 sidecar 构建 `--manifest-path ../webview-app/Cargo.toml`、`stage-assets.cjs` 取 `../webview-app/icons/`、注入脚本 / `settings.html` / `shell.html` / sidecar 产物全指向 `webview-app/` —— **两壳同级是前提**，不得把 `electron-app/` 挪进 `webview-app/`。发版待办：两壳版本号均 `0.1.0` 且无 tag，首次发版前需约定 tag 前缀（`tauri-v*` / `electron-v*`）。

## 四、文档索引

| 文档 | 路径 |
| --- | --- |
| 项目全盘分析与优化建议 | `docs/project-analysis.md` |
| electron-app 架构与组织结构审查（2026-11-07，模块边界/依赖/跨壳复用，含分级问题与优化批次） | `docs/electron-app-architecture-review-2026-11-07.md` |
| **本地脚本说明** | **`docs/local-scripts.md`** |
| **打包配置 JSON 契约（21 字段，跨壳唯一权威）** | **`docs/packaging-config.md`** |
| **Android 端优化清单（唯一任务清单）** | **`docs/android-improvements.md`** |
| Electron 方案族：落地 M0–M7 / 分析 / 多页签设计 / 多页签真机 / 打包瘦身 | `docs/electron-plan.md`、`docs/electron-analysis.md`、`docs/electron-tabs-design.md`、`docs/electron-tabs-test-steps.md`、`docs/electron-size-reduction.md` |
| **动态解析本机手动生成步骤（密钥 → TXT 密文 → 打包）** | **`docs/txttest-verify-materials.md`** |
| 溯源稿（配置字段总表、打包 JSON 评审）/ 历史归档 | `docs/config-fields-2026-09-22.md`、`docs/config-json-review-2026-09-22.md`、`docs/archive/` |
| 变更记录（按日归档 `YYYY-MM-DD.md`） | `docs/changes/` |
| Web2App（Tauri）使用说明 / Electron 绿色版说明 | `webview-app/AGENTS.md`、`electron-app/AGENTS.md` |

> 新增全局专项文档须在此登记；子项目内部文档除本表登记外，还需在其目录 `AGENTS.md` 中交叉引用。

## 五、变更记录

> 详细变更写入 `docs/changes/YYYY-MM-DD.md`，此处仅留最近 6 条一句话摘要 + 指针。

| 日期 | 摘要 |
| --- | --- |
| 2026-11-14 | **Android 优化清单重审推进**（`docs/android-improvements.md`）：**#9** 返回键先关底栏浮层、**#10** 无历史时「再按一次退出」（`patch-android-back.py` 改 `evaluateJavascript` 异步续延 + Toast）、**#11** 页签切换「会重新加载」文案、**#12** 键盘让位（`visualViewport`，仅差 ≥60px 生效）、**#15** 代理模式探测真实上游（401/403 单独成类）、**#16** 宽屏底栏限宽居中、**#18** 明文流量策略仍为**全放行**但收成单一落点（新增 `patch-android-cleartext.py` 写 `network_security_config`，删两 workflow 的全局 sed；首版收紧为「仅回环」当日回退 —— 直连 `http://` 外站会被系统拦）。#13/#14/#17/#19 转待决策（含选项与建议，见该文件 §5）。Rust **316 passed / 6 ignored**、14 个 Python 脚本自测全绿、补丁链按序试打通过 → `docs/changes/2026-11-14.md` |
| 2026-11-08 | 唤醒快捷键校验两壳对齐（Tauri 主进程补齐 Electron 五条判据：未设置 / 裸单键 / 系统占用 7 键 / `Shift+F1` / 无法映射，系统占用表三处逐字一致）；旧配置被判非法时启动回退「不设置」+ 设置页提示（进程内、保存后消失、不落盘）；`cargo test --all` 308 通过 / 6 ignored、parity 55/55 → `docs/changes/2026-11-08.md` |
| 2026-11-07 | electron-app 六批架构优化（注入清单单一真源 / 特权 IPC 闸门 / 探针不进包 / 纯逻辑可测 / 路径与协议收口 / 守卫假绿治理）+ webview-app 架构诊断与四批落地（Android 补丁链目录派生校验、dead_code 收窄、settings.json 损坏备份、mobilebar.js 核心+功能拆分）→ `docs/changes/2026-11-07.md` |
| 2026-11-03 | 设置页**「唤醒快捷键」交互重做**（使用者：手动选择分不清组合键还是单键）：旧实现两条入口语义相反 —— 下拉选键**隐式补 Alt**（选 F9 实存 Alt+F9 却不显示），捕获却能产出**裸单键**（按 F9 就是 F9）；现统一为「组合键 = 修饰键（Ctrl/Alt/Shift 至少一个）+ 主键」，`captured` 唯一状态、两个编辑器互相重画，「当前生效」与「待保存」分离，单键 / 系统键（Alt+Tab·Alt+Space·Alt+F4·Ctrl+Esc）/ `Shift+F1` 在**保存前**拦下（一个字节都不写盘）且捕获路径同样不接收；顺带修「先勾修饰键再选键时勾选被清空」与「历史键不在下拉里 ⇒ 显示无、一保存被抹掉」两个真缺陷。Electron 探针 13b 八条（150 → 158）。**复核轮**又修两处静默缺陷：① 捕获**列表之外**的键（`Ctrl+↑`）时下拉 `selectedIndex = -1`（预览说 Ctrl+↑、下拉什么都没选，用户碰一下就把捕获结果丢掉）⇒ 补 option 抽成 `ensureKeyOption`，改由 `renderHotkey` **每次**调用；② `get_settings` 失败后点保存会把快捷键/托盘/最小化**整条覆盖成默认值**还回「已保存」⇒ `loadFailed` + 禁用保存 + 保存处第二道，卡片状态行改说「当前快捷键未知」而不是「未设置」。新增 `contract_tests/settings_hotkey_contract`(5) 静态契约（`has_code_line` 上提 `mod.rs` 共用 —— 第一版用 `contains` 时被**注释**满足而假绿）。Rust **287 passed**、unit 123、`probe` **158/158**、`probe --tabs` 54/54、smoke 通过 → `docs/changes/2026-11-03.md` |
| 2026-11-03 | 多页签支持 **Ctrl+W 关当前页签**（双壳）：Electron 在 `before-input-event` 上挂 `isCloseTabKey/attachCloseTabShortcut`（内容视图 + 壳页面两处），Tauri 新增注入脚本 `src/tab_keys.js` → 核心事件 `web2app:close-tab`（label+一次性 token，防伪与 `tab_title.js` 同一条通路）；首页那一签照旧拒绝（`close_tab_inner` / `isPinnedHome` 单点判定，新入口不重复判）；Ctrl+Shift+W 与长按不算。Rust 282 passed（+2 静态契约）、Electron unit 123、`probe --tabs` 54/54、`probe` 150/150、smoke 通过 → `docs/changes/2026-11-03.md` |
| 2026-11-01 | 修 Android 真机「打开白屏 → 约 10 秒后自动退出」：`mobilebar::on_page_finished` 在 **Android UI 线程**里调 `window.url()`，而 wry 该 getter = `MainPipe::send` + `recv_timeout(10s)`、MAIN_PIPE 消费方正是这条 UI 线程（`onFirstActivityCreateWry` → `ThreadLooper::for_thread()`）⇒ 必然僵 10 秒超时，随后 `main_pipe.rs` 的 `tx.send(url).unwrap()` 撞上已丢弃的接收端 ⇒ panic ⇒ abort。改用缓存 `mobilebar::CurrentUrl`（写入 = `on_page_finished`（payload url）+ `web2app:mobile-doc`，读取 = `at_same_target`），并新增守卫 `mobile_bar_never_queries_webview_on_ui_thread`（⚠️ 初版只钉 `window.url()` 被 `w.url()` 变体绕过=假绿，改钉整个 `.url()` 形态后反向对照如期 FAILED）；Rust 280 passed / 6 ignored → `docs/changes/2026-11-01.md` |

> 完整历史（含 v1.0–v1.29 仓库搭建与文档体系建立）见 `docs/changes/`。
