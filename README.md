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
| `webview-app` | Rust + Tauri 2 主工程 | 跨平台壳主工程：cargo workspace 四成员（`web2app` 主程序 / `resolve-core` 纯加密 / `resolve-tool` 服务端 CLI / `proxy-server` → Electron 侧 sidecar `w2a-proxy.exe`），含注入脚本、CI 与打包工作流 | `webview-app/README.md` |
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
   - `electron-app`：契约探针 `src/main/probe.ts`（`pnpm run probe`，命令行加 `--tabs` 走页签断言组）、
     冒烟与取证脚本 `scripts/*.cjs`（`smoke` / `start` 经 `launch.cjs` 拉起；`shot` / `lab` / `diag` / `png` / `tabs-v4` / `tabs-v4-negative`）。
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
| 项目全盘分析与优化建议 | `docs/project-analysis.md` | 架构分析、模块职责地图、优化建议与遗留问题 |
| 全流程审计报告（2026-09-20） | `docs/audit-2026-09-20.md` | 「文档 / 实现 / 门禁一致性」审计：门禁对比、P0/P1 修复项、遗留事项与局限 |
| 全流程审计报告（2026-09-23） | `docs/audit-2026-09-23.md` | 设置界面重做 + 双壳契约同步一轮的审计：目标与验收判据、缺失/过期/冗余三类清单、逐项闭环记录、守卫反向对照与门禁实测证据 |
| Web2App 使用说明 | `webview-app/README.md` | Tauri 版配置、构建、打包与使用 |
| Electron 绿色版说明 | `electron-app/README.md` | 绿色版配置、构建、验证与使用 |
| Electron 落地方案 | `docs/electron-plan.md` | M0–M7 里程碑与已锁定决策、兼容契约（权威状态以子项目 README 为准） |
| Electron 多页签设计方案 | `docs/electron-tabs-design.md` | M5 多页签：A′ 架构定案（已实现并真机验收）、落地清单与验收判据 |
| Electron 多页签真机测试步骤 | `docs/electron-tabs-test-steps.md` | M5 真机验收清单（2026-09-18 完成，V1–V4 已关闭）+ 代理/域名解析待办 |
| Electron 方案分析 | `docs/electron-analysis.md` | 功能对等矩阵、复用策略、兼容性要点与安全基线 |
| 验签测试素材说明 | `docs/txttest-verify-materials.md` | DNS TXT 密文手工验签素材 |
| Electron 打包瘦身方案 | `docs/electron-size-reduction.md` | Electron 绿色版打包产物实测基线：语言包/DLL/压缩形态分级瘦身（P0 可直接落地） |
| **打包配置 JSON 契约** | **`docs/packaging-config.md`** | **打包配置的唯一契约说明**：18 个字段的类型/默认值/含义、多端（Windows / Android / Electron 绿色版）行为对照、校验规则、两条打包入口与迁移指引 |
| （过程稿）配置字段总表 | `docs/config-fields-2026-09-22.md` | Step 1 取证：6 个配置面的字段清单与已确认缺口（结论已并入 `packaging-config.md`） |
| （过程稿）打包 JSON 评审 | `docs/config-json-review-2026-09-22.md` | 改造前的配置 JSON 问题清单与优化建议 |
| （历史）标题栏导航按钮设计 | `docs/archive/add_titlebar_nav_buttons.md` | 已归档，仅作追溯（2026-09-22 由 `.trae/documents/` 迁入） |
| （历史）诊断与优化 | `docs/archive/web2app-diagnosis-optimization.md` | 已归档，仅作追溯（2026-09-22 由 `.trae/documents/` 迁入） |

> 新增文档时，**必须在此表登记**。子项目内部文档除本表登记外，还需在其目录 `README.md` 中交叉引用。

## 五、补充说明
1. **单一真源**：注入脚本与页面（`titlebar.js`、`loading.js`、`shell.html`、`splash.html`、`dist/settings.html`、`icons/*`）
   及代理/解析链路（`proxy.rs` → sidecar）为两个壳共用，改动任一均须双壳同步验证，避免契约漂移。
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
| v1.1 | 2026-09-17 | 补全项目概述、仓库结构地图、文档索引与补充说明 |
| v1.2 | 2026-09-17 | 专项文档归位至 `docs/`，清单与引用同步 |
| v1.3 | 2026-09-17 | 全部文档与仓库现状逐条核对优化 |
| v1.4 | 2026-09-17 | 新增 `docs/electron-tabs-design.md` 并登记索引 |
| v1.5 | 2026-09-17 | M5 选型更正为 A′（`BrowserWindow` 壳 + 子 `WebContentsView`） |
| v1.6 | 2026-09-17 | M5 T0 完成：`tabs-v4.cjs` 程序化验证 V4（A′ 定案） |
| v1.7 | 2026-09-17 | 新增 `docs/electron-tabs-test-steps.md` 并登记索引 |
| v1.8 | 2026-09-17 | M5 T1 骨架完成（tabs.ts / shell:// 协议 / sender 校验） |
| v1.9 | 2026-09-17 | M5 T2 页签管理完成（19 条断言；修 removeChildView、get_tabs 契约） |
| v1.10 | 2026-09-17 | M5 T3/T4 收尾：探针单窗口 112/112、页签 23/23、打包产物双模式退出码 0 |
| v1.11 | 2026-09-18 | M6 结清 + 审计修复（启动日志落盘、VERSIONINFO 实证订正） |
| v1.12 | 2026-09-20 | 文档索引补登 + webview-app 死代码清理 |
| v1.13 | 2026-09-20 | 文档/实现一致性 + CI 门禁审计（fmt 归零、失效引用归正、cfg(desktop) 等） |
| v1.14 | 2026-09-22 | 全项目单仓库化（`.git` 上移、workflow 迁至仓库根、归档稿入 `docs/archive/`） |
| v1.15 | 2026-09-22 | 全仓库文档精简：删除过期结论与逐日过程记录，保留决策、契约与已验证结论 |
| v1.16 | 2026-09-22 | 打包配置契约统一：移除 `window_title`、布尔规范为真 `bool`、`app_url` 支持 `string \| string[]`、两壳行为对齐；Electron `config.json` 收入 `app.asar`；新增 Electron 手动打包 workflow；新增 `docs/packaging-config.md` 并登记索引 |
| v1.17 | 2026-09-22 | `data_dir_default` 废弃为 **`data_dir`（字符串，两壳同名同义）**：默认 `"data"` = 程序目录下 `data\`（相对基准 = exe 目录，非 cwd）、`""` = 系统默认程序目录（由 `app_name` 定义）；⚠️ 空串是有意义的值，禁止被 `env_str!` / `${VAR:-}` / `\|\|` 吞掉；Android 应用包名改由 `app_name` 派生（工作流在 `android init` 前改写 `identifier` 为 `com.web2app.<清洗值>`），`docs/packaging-config.md` §4.2/§4.3 同步 |
| v1.18 | 2026-09-22 | `ci.yml` 新增 **`config-contract` 作业**（此前文档声称「CI 里会跑」但并无该作业）：解析器 `--selftest` + 全 workflow YAML/内联脚本静态检查 + 逐个校验仓库内配置样例；作业级 `defaults.run.working-directory: .` 覆盖 workflow 级（契约脚本在仓库根，不在 `webview-app/`）。修 `check-workflows.py` 的 `find_bash()` 误选中 WSL 启动器（`C:\Windows\System32\bash.exe` 退出码 -1，导致**每个** `bash -n` 都假失败） |
| v1.19 | 2026-09-22 | 设置页新增**清除缓存 / Cookie（只有全清，无分项）**：Tauri 侧走 WebView2 原生 `ICoreWebView2Profile2::ClearBrowsingDataAll`（见 `webview-app/src/web_data.rs`，命令 `clear_web_data`），Electron 侧走 `session.clearData + clearAuthCache + clearHostResolverCache`（见 `electron-app/src/main/web-data.ts`）；两壳清完都重新加载内容页。⚠️ 该命令必须 **async**（同步命令在主线程内联执行会堵死消息泵 → WebView2 完成回调永远派发不出来）；探针新增 1 条断言（**113/113**），且探针下走桩分支不清真实数据 |
| v1.21 | 2026-09-22 | **诊断日志改为 `debug` 开关（默认关）+ Electron 托盘图标对齐 `icon_url`**：①契约新增第 18 个字段 `debug`（**bool，默认 `false`**）—— Tauri 侧新增 `webview-app/src/debug_log.rs`（`dlog!` 宏 + 落盘 + 1 MiB 轮转 + `mask_url` 去凭据），Electron 侧 `startup-log.ts` 由「无条件落盘」改为按开关，且 `debug=false` 时**控制台三路输出一并静默**（`silenceConsole()`）；运行期逃生口 `--debug` / `WEB2APP_DEBUG=1`（⚠️ **不是**打包期注入用的 `DEBUG`，后者沿用「字段名大写」规则）；探针 / 冒烟内部强制开，避免自检失声；②`release.yml` 两个 job 补 `DEBUG` 仓库变量（未配置 = 空串 = 关闭）；③**托盘图标修复**：`electron-build-custom.yml` 下载的 `icon_url` 此前只喂 `-c.win.icon`（exe / 任务栏图标），托盘取的是 `extraResources` 从 `webview-app/icons/` 拷入的**仓库默认图标** ⇒ 下载后同时覆盖 `../webview-app/icons/icon.png`，两处同一张图；④顺手修掉 `build-custom.yml` 一次性 keystore 分支的**续行链断链**（注释夹在 `\` 续行链中间会截断赋值：实测 `A=1 \` + 注释行 + `B=2 cmd` 时外部进程只看到 `B`） |
| v1.22 | 2026-09-23 | **闭环 `docs/audit-2026-09-20.md` 遗留项**：①**测试缺口补齐**（B5）—— `proxy-server` 握手 JSON 契约（`parse_args` / `strip_userinfo` / 单行 JSON 契约）与 `tabs.rs::resolve_popup_url` 补单测；为使其可单测，把「弹窗落点」抽成纯逻辑 `resolve_popup_url_for`（`AppHandle` 只负责取缓存的 `Navigation`）。②**B4 静默失败消费点接完**—— `settings::load_settings` 读盘 / 解析失败、`webview_hooks::attach_webview_hooks` 的 `with_webview` 投递失败，两处接入 `dlog!`（`settings.json` 不存在 = 首次首启的正常态，不打日志） |
| v1.23 | 2026-09-23 | **运行期只剩一个 JSON（`settings.json` = 设置三键 + `window` 段几何）**：窗口几何从 `window-state.json` 并入 `settings.json` 的 `window` 段（设置字段仍平铺顶层，与 Tauri 版同契约）；旧文件**不迁移、不再读写**（升级后窗口回一次默认尺寸/位置）；因「两个写入方会互相覆盖」是当初分文件的唯一理由，两壳写盘一律改为**读-改-写**（Rust `src/store.rs::update`、Electron `saveSettings` / `saveWindowState`），且两段**各自独立解析**（`window` 段坏掉不拖垮三个开关）；单测 65 → 71（含两条互斥守卫：写设置保几何 / 写几何保设置，打真临时文件）、探针 113 → 117（含键序 `设置三键 → window`） |
| v1.24 | 2026-09-23 | **设置界面重做（无边框 + 居中面板 + 正文滚动）+ 标题栏新增「设置」按钮**：①版式（两壳共用 `dist/settings.html`）——窗口无系统标题栏，整页一块居中圆角面板，标题 / 关闭 / 拖动自绘在面板头部；`html/body` 都不滚，**唯一滚动容器是 `.panel-bd`**，底部操作区固定；元素 id 与脚本逻辑一字未改（三处契约不受影响）；②新增 capability **`webview-app/capabilities/settings-window.json`**（`windows:["settings"]`，只给 `start-dragging` + `internal-toggle-maximize`）—— 无边框窗少了它**拖不动且不报错**，只有一条被拒的控制台日志；③「设置」按钮必须绕开 ACL：主窗是远程页面，只能发核心插件事件（`core:event:allow-emit` → `app.listen("web2app:open-settings")`），多页签标签栏是本地页面才直接 `invoke("open_settings")`（**页面字面量 / 白名单 / IPC 名三处逐字一致**）；④设置窗参数 480×470 / 带系统标题栏 → 620×600 / 无边框 / 可缩放（最小 460×400）/ 居中；探针 117 → **126**、多页签 23 → **26**；⑤新增 `src/lib.rs` 的 `capability_contract` 静态守卫（3 条：主窗权限齐备、设置窗标签与 `settings.rs` 建窗标签一致且是无边框窗、事件名字面量两侧一致）⇒ 主 crate 单测 71 → 74，「能力声明漏一条 / 事件名不同步」这类静默失败不再靠人眼，且该 capability 文件一旦漏纳版本控制**直接编译失败** |
| v1.25 | 2026-09-23 | **文档/实现一致性审计（报告见 `docs/audit-2026-09-23.md`）+ capability 守卫**：①订正三处**计数漂移**（打包字段 17 → **18**、Electron `config.example.json` 13 → **14**、单测 104 → **116**）；②`M3 之后系统集成` 52 → **60** 条、设置页垫片白名单「三个命令」→ **四个**（补 `clear_web_data`）；③§五.1 单一真源清单补 `dist/settings.html` / `icons/*`；④`index.ts` / `config.ts` 注释与三个 `docs` 里的**操作型**旧计数（112 / 23）同步为 126 / 26；⑤新增 `src/lib.rs::capability_contract` 静态守卫（3 条，配合 v1.24 的 capability），并做**反向对照**确认三条都会真的失败；⑥`webview-app/capabilities/settings-window.json` 由未纳管改为入库（`include_str!` 使其缺失即编译失败） |

> 注：编号 `v1.20` 为早期笔误跳号（无对应版本），历史编号保持原样不改写。
