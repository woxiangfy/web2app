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
   - **单壳专属**：`mobilebar/*.js`（Tauri 移动端；6 分片由 `lib.rs` 按序拼成同一 IIFE）、`tab_title.js` / `tab_keys.js` / `tab_doc_ready.js` / `dnd_polyfill.js`（WebView2/Tauri 注入，Electron **不消费**）、`floaticon.html`（**Electron 专属** —— Tauri 球 v1.81 起是 Win32 分层窗口 + 位图 alpha，不加载任何页面）。

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
| **架构待决策清单（Rust / Electron 两侧，处理完划掉并回填结论；含 2026-11-19 第二轮复核改判）** / Tauri 侧架构审查（2026-11-19，重点 Android：模块划分/依赖/复用/过度设计） | **`docs/architecture-todo.md`**、`docs/architecture-review-2026-11-19.md` |
| **本地脚本说明** | **`docs/local-scripts.md`** |
| **打包配置 JSON 契约（21 字段，跨壳唯一权威）** | **`docs/packaging-config.md`** |
| **Android 端优化清单（唯一任务清单）** | **`docs/android-improvements.md`** |
| **Android 打包「冻结契约」（三层模型 / 产品输入面白名单 / L2 用自身代码替代的迁移路线）** | **`docs/android-packaging-freeze.md`** |
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
| 2026-11-20 | **Android 底栏三项（真机反馈）**：① 「更多」面板精简为**清除浏览数据 / 退出应用**两行（地址头、刷新 / 复制链接 / 分享 / 在浏览器打开连同 `copyCurrentLink` 与四个图标一并下线，Kotlin 桥 `share`/`openBrowser` 保留为预留能力）；② 「列表」面板去掉「＋ 新建标签」与「切换标签会重新加载页面」提示，**`web2app:new-tab` 事件链一并移除**（JS emit / `NEW_TAB_EVENT` / 监听 / `create_tab` / `home_url`，新建页签只剩 `web2app:open-tab`）；③ **底部遮挡修复**：页面自己的「带头 + 自滚动内容区」底边落进底栏压盖带时让位（`padding-bottom` 一格，iframe/子文档同样生效，标记 `data-web2app-mobile-pad`），并给滚动根契约探测加**补跑自愈**（`load`+1.5s+3s，退路 inline 样式可撤销，假绿/假红两条错路都能回头）。契约测试同步（反例锁定已下线行不得回归；新增 ⑥b `bottom_occlusion_self_heals`，删旧 ⑱），`cargo test --all` **338 passed / 6 ignored**、fmt/clippy ✅；**待真机复验 android.md 清单 #4–#7/#12/#16** ⇒ `docs/changes/2026-11-20.md` |
| 2026-11-20 | **顶 / 底配色跟随页面（真机截图反馈：顶部白条、底栏灰条）**：根因都是壳自己的硬编码 —— 顶部那格是 `enableEdgeToEdge` + insets 内缩后露出的主题 `windowBackground`（CSS 改不到窗口层），底栏是写死的 `#eef0f2` 且 body 已让位、后面没有页面内容。做法：新增第 6 分片 `src/mobilebar/chrome.js`（接缝取色：顶部 = 视口 y=0、底部 = 底栏上沿往上 2px；顺序 theme-color → 背景色 → 渐变色标近似 → 放弃保持默认；`CHROME_TINT=15%` 轻混色；首帧/300ms/1200ms/滚动/视口/日夜/DOM 变动统一 120ms 去抖）＋ `core.js` 配色变量化（`--w2m-bg/fg/border`，兜底值与改动前逐字一致）＋ `patch-android-extras.py` 桥新增 `setChrome`（刷窗口背景 + 图标明暗；**Android 15 / targetSdk 35 起 `setStatusBarColor` 是 no-op**，必须刷背景）。分片数 5→6（`lib.rs` 仍是唯一权威），新增契约 `chrome_colors_follow_the_page`（`b.setChrome(` 漂移反向对照已做、`backdrop-filter` 反例锁定），`cargo test --all` 全绿、fmt ✅；不做 `backdrop-filter`、不动底部空间契约；跨域图片/视频背景取不到像素属已知边界；**待真机复验 android.md「自适应配色」6 项** ⇒ `docs/changes/2026-11-20.md` §5 |
| 2026-11-19 | **安卓端真机「加载失败」：Manifest 缺 `ACCESS_NETWORK_STATE`**（用户截图：域名数组包一打开就停在失败卡，`解析目标地址失败：读取系统 DNS 配置失败：JNI call error … Neither user 10358 nor current process has android.permission.ACCESS_NETWORK_STATE`）。根因在依赖源码：`hickory-resolver 0.26.1` 的 Android `system_conf` 是 JNI 调 `ConnectivityManager.getActiveNetwork()` / `getLinkProperties()` / `getDnsServers()` 取系统 DNS，受该权限保护；而 `patch-android-permissions.py` 当时「`ANDROID_PERMISSIONS` 为空就整步跳过」⇒ 默认包从未声明。修法两层：① 打包侧恒需权限块（`INTERNET` + `ACCESS_NETWORK_STATE`，**缺失才补**、与打包配置无关、独立 marker `web2app-android-required`，`android_permissions` 恢复「只表达可选能力」），`main()` 不再因空数组提前返回；② 运行期 `build_resolver()` 读系统配置失败**不再判死**，新增 `fallback_config()`（223.5.5.5 / 119.29.29.29）继续查并留 `dlog!`（读不到配置 ≠ 网络不通）；新增单测 `fallback_config_registers_all_builtin_dns` 钉「兜底不是空配置」。自测 7 → 8 组（新增空数组仍须注入恒需 / 模板自带 INTERNET 不重复），**反向对照已做**（改坏 ⇒ 红灯，还原 sha256 一致）；`rehearse.py` 在 tauri 真模板重放全链 `permissions=5 条` ✓；`cargo test --all` **336 passed / 6 ignored**、fmt/clippy ✅ ⇒ `docs/changes/2026-11-19.md`、`docs/android-improvements.md` #21 + §4 第 15 条、`webview-app/docs/android.md` 排障史新节。**同日追加**：按评审意见定下 **Android 打包冻结契约**（`docs/android-packaging-freeze.md`）—— 三层模型（L0 恒需常量 / L1 可选能力 / L2 版本适配）、唯一产品输入面（`APP_NAME`/`TITLE`/`VERSION`/`ANDROID_PERMISSIONS`，新增契约测试 ⑨ 冻结，含两组反向对照）、以及把 L2 改由自身代码实现的迁移路线（L2 归零后可删 `prepare-wry-kotlin.py` + `inject-gradle-patch-task.py`，15 → 11 脚本）。**第三轮（评审要求「升级也别改脚本」）**：把「升级免疫」做成机制 —— ① 锚点只认平台 override 签名（`android_gen` 里写成纪律）；② 新增 `AnchorDrift` + `run_guarded()`：锚点漂移默认 `::warning::` + exit 0（`W2A_ANDROID_PATCH_STRICT=1` 才红），7 个可降级脚本接入、permissions/cleartext 保持硬失败；③ `fullscreen` 遇上游已实现改为**自动退场**（此前是 raise ⇒ 升级必改）；④ 新增 `rehearse.py --drill`：真 wry 模板上跑 5 类上游变化（reformat / param_rename / upstream_fullscreen / anchor_gone / wry_file_renamed）全绿，反向对照 6 组已做、逐文件 sha256 还原一致；**复核补漏**（定位失败分「环境错误 vs 布局漂移」：`gen/android` 没 init 才红，胶水文件改名/入口类改名/wry 模板清单失效一律降级）。**第四轮（Android 打包 `error[E0603]`）**：`probe` 抽模块后 `probe_target` / `spawn_reachability_probe` 仍是私有 `fn`，而调用点与定义在 Windows 上**一起被 `cfg(not(windows))` 摘掉** ⇒ 桌面 `cargo test`/`clippy`/契约测试全绿、只有 Android 目标报错（`ci.yml` 的 `check-android` 本可拦下，本次是手工出包单跑未等它绿）。修法：两函数 `pub(crate)`、`show_float_window` 非 Windows 分支消费掉 `app` 参数；新增契约测试 `module_visibility_contract`（扫 `crate::<模块>::<项>`，目标有定义行却非 `pub` 即红灯，含 10 处下限自校验 + 反向对照）⇒ `test` **338** |
| 2026-11-19 | **Tauri 侧架构审查（重点 Android）+ 两项零行为变更整理**：依赖方向健康（`bootstrap` 不再被下行依赖、跨 crate 无重复实现），但发现 1 处实质复用缺口 —— 移动端 `open-tab` 未复用桌面「上游 → 入口」地址改写（本地代理 + Basic 鉴权下页面绝对外链落 401），已登记待真机复验；另 3 项主链路改造（`save_settings` 副作用非事务 / `mobilebar` 991 行单文件 / 设置簇三条环）定为待决策。本轮执行：① `bootstrap` 尾部 273 行 Android 可达性探测抽成 `src/probe.rs`（873 → 504 行，cfg 与「Windows 发布产物不携带」的纪律不变）；② `Settings`/`Hotkey` 下沉 `src/model.rs`，闭环待办 P-R1 的 `store ↔ settings` 类型环（`store → model`、`settings → model`），契约断言随字段改扫。`cargo test --all` **333 passed / 3 ignored**、fmt/clippy ✅（Android 目标本机无 SDK/NDK 未编译）⇒ `docs/architecture-review-2026-11-19.md`、`docs/changes/2026-11-19.md`；**第二轮**：复核 6 项待决策（P-R6「抽 apply.rs 断环」与 P-R7「floatprobe feature gate」**改判不做**，前者方案本身断不了环）并落地 P-R3/P-R4/P-R5 —— ①「上游→入口」改写规则上移 `nav::resolve_popup_target`，移动端 `create_tab_with_url` 复用（代理+鉴权下 `target=_blank` 再也不落 401）；② `save_settings` 改「先落盘 → 副作用 → 内存态」，副作用失败 `restore_persisted` 回写（钉住「存不下键」的失败链）；③ `mobilebar.rs` 测试外置到 `src/mobilebar/tests.rs`。两项新契约均已反向对照；`test` **334** ⇒ `docs/changes/2026-11-19.md` |
| 2026-11-18 | **electron-app 架构全面复查（第二轮）+ 两处小修**：值边 0 环 / renderer↔main 0 跨层 import / 11-07 审查全部闭环 / 过度设计面很轻；修 `sidecar-ship.ts` 重复的「平台→可执行名」规则（re-export `sidecar.ts::sidecarExeName`）、`settings-controller.ts` 副作用注入化使「成组回滚」矩阵可单测（新增 9 条，此前零覆盖）。unit **233/233**、typecheck/build ✅、probe 三轮 m3 全绿（第 18 节时序项本机间歇抖动）；待决策 3 项（TabManager 隐藏弹窗子系统 / page-behavior 归类 / float-layout 导出面）⇒ `docs/changes/2026-11-18.md` |
| 2026-11-18 | **域名数组解析改为并发竞速**（明确不做缓存）：原 `resolve.rs` 串行遍历域名，3 个域名时最坏耗时是三者之和，启动页长时间停在「正在解析目标地址」。新增纯调度 `race_first_ok`（`FuturesUnordered`）：所有域名**同时**发起「TXT 查询 → 逐条解密验签」，**第一个成功立即返回**，其余 future 随函数返回被丢弃（查询取消、不在后台空转）；`Resolver` 构建一次并 `clone` 共享连接池；逐域失败原因仍完整 `dlog!`（stdout 协议不受影响）。语义变化：由「按配置顺序优先」变为「最快成功者胜出」（3 个域名互为镜像时无差异）。总预算 `RESOLVE_BUDGET_SECS=25s` 保留（单域名 hickory 重试仍可能 10s+）。新增 3 条竞速单测 + 1 条静态契约（必须含 `FuturesUnordered` / `race_first_ok`，防悄悄退回串行）；`cargo test --all` **331 passed / 3 ignored**、fmt/clippy ✅；两壳共用（Tauri 进程内 / Electron sidecar `#[path]` 同源）⇒ `docs/changes/2026-11-18.md` |
| 2026-11-18 | **Electron 包内 sidecar 按配置裁剪**（使用者关注点不是体积，而是装完提示「这个程序要联网」）：`proxy_enabled=false` + 字符串 `app_url` 时运行期 `navigate.ts::needsSidecar` 恒假 ⇒ `w2a-proxy.exe` 一次都不会被 spawn，但 `extraResources` 仍无条件把它拷进 `resources/sidecar/`。新增 `src/main/sidecar-ship.ts`（判定**复用**运行期 `needsSidecar`；落盘四态 kept / removed / absent / 需要却缺失即抛错）+ `scripts/after-pack.cjs`（electron-builder `afterPack` 钩子，`electron-builder.yml` 单点接线 ⇒ 三个出包入口共享，不重蹈 2026-11-14 `prune` 三漏二）+ `check-asar-config.cjs` **双向**护栏（该有却没有 / 不该有却还在都红灯）；`test/sidecar-ship.test.ts` 9 条。⚠️ **「不启用代理」≠「不需要 sidecar」**：`app_url` 是域名数组时无论 `proxy_enabled` 如何都必须保留（DNS TXT + Ed25519 验签 + AES-GCM 解密只有 Rust 侧实现）。`unit` **224/224**、`typecheck`/`build` ✅；钩子实跑：字符串 ⇒ `removed`、域名数组 ⇒ `kept`；`--dir` 实包日志 `[after-pack] sidecar 剔除 ⇒ removed`、`resources/sidecar/` 不存在、护栏 `sidecar: 不需要且未随包 ✓`，并做了双向反向对照（各退 1）⇒ `docs/changes/2026-11-18.md` |
| 2026-11-18 | **Electron 壳补上「网页登录账号密码保存 + 自动填充」**（用户真机验证发现缺失；`docs/changes/2026-10-09.md` 原记「Electron 默认已支持」被订正为错误前提）：Electron 只含 Chromium **content 层**，Chrome 浏览器层的密码管理器 / 自动填充 UI 未编译进产物、无 API / 开关可开 ⇒ **只能自研**。新增 `src/main/credentials-logic.ts`（纯规则：origin 归一化，回环地址丢端口以适配本地代理随机端口；条目上限 64）+ `credentials.ts`（`<data_dir>/credentials.json`，`safeStorage` 加密、加密不可用拒绝保存、解密失败不删文件、原子写）+ `w2a:pwd-lookup/-save/-forget` 三条 IPC（顶层框架 + 内容页 sender 闸门，origin 由主进程从 `senderFrame.url` 推导）；`preload.ts` / `tab-preload.ts` 各加一段**逐字一致**的 `installPasswordManager()`（隔离世界、不暴露页面桥；只认 `isTrusted` + `userActivation` 的 submit，**不 preventDefault**；**提交只暂存到主进程内存（60s TTL），确认登录成功（新文档无可见密码框 / 本页表单消失）才落盘，失败不覆盖旧密码**，成功后右上角「已保存/删除」气泡（删除即直接删掉该站点凭据）；自动填充成功后提示「密码已自动填充」；自动填充走原型 value setter + input/change；**触发时机 = 首屏 / DOM 变动（MutationObserver）/ URL 变化（主世界 history 钩子，覆盖 pushState / replaceState；无任何常驻轮询）/ focusin —— 有空的密码框就填，不需要先点输入框**；`safeStorage` **只用异步 API**（`isAsyncEncryptionAvailable` / `encryptStringAsync` / `decryptStringAsync`，同步版在 Electron 45 已废弃））；`clearWebData` 一并清凭据；`tabs.ts` 增 `ownsWebContents`。测试：`unit` **213/213**（新增纯逻辑 8 条 + 装配契约 11 条）、`typecheck`/`build` ✅、`probe` 单窗口 **176/176**（第 18 节 18/18）、`probe --tabs` **69/69**（P13 含同项）；探针踩坑记录：`requestSubmit()` 的 submit `isTrusted=true`、`executeJavaScript(...,true)` 附带 user activation、隐藏窗口的子 `WebContentsView` 收不到 DOM 键鼠 ⇒ **同日复审又修 4 处**：观察器回调无条件提前 `return` 导致「SPA 同一文档登录成功」永不落盘（只有跨文档跳转能存）、新文档 1.2s 确认探针会把本页刚提交的暂存判成失败丢弃、重加密回写用旧文档快照覆盖并发保存、清除全部浏览数据清不掉待落盘暂存（会把文件写回来）（新增夹具 `/login-spa.html` + 探针 18i/18j；单窗口 **178/178**、页签 **69/69**、`unit` **215/215**）⇒ `docs/changes/2026-11-18.md` |
> 完整历史（含 v1.0–v1.29 仓库搭建与文档体系建立）见 `docs/changes/`。
