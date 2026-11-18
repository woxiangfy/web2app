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
   - **单壳专属**：`mobilebar/*.js`（Tauri 移动端；5 分片由 `lib.rs` 按序拼成同一 IIFE）、`tab_title.js` / `tab_keys.js` / `tab_doc_ready.js` / `dnd_polyfill.js`（WebView2/Tauri 注入，Electron **不消费**）、`floaticon.html`（**Electron 专属** —— Tauri 球 v1.81 起是 Win32 分层窗口 + 位图 alpha，不加载任何页面）。

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
| 2026-11-18 | **Electron 壳补上「网页登录账号密码保存 + 自动填充」**（用户真机验证发现缺失；`docs/changes/2026-10-09.md` 原记「Electron 默认已支持」被订正为错误前提）：Electron 只含 Chromium **content 层**，Chrome 浏览器层的密码管理器 / 自动填充 UI 未编译进产物、无 API / 开关可开 ⇒ **只能自研**。新增 `src/main/credentials-logic.ts`（纯规则：origin 归一化，回环地址丢端口以适配本地代理随机端口；条目上限 64）+ `credentials.ts`（`<data_dir>/credentials.json`，`safeStorage` 加密、加密不可用拒绝保存、解密失败不删文件、原子写）+ `w2a:pwd-lookup/-save/-forget` 三条 IPC（顶层框架 + 内容页 sender 闸门，origin 由主进程从 `senderFrame.url` 推导）；`preload.ts` / `tab-preload.ts` 各加一段**逐字一致**的 `installPasswordManager()`（隔离世界、不暴露页面桥；只认 `isTrusted` + `userActivation` 的 submit，**不 preventDefault**；**提交只暂存到主进程内存（60s TTL），确认登录成功（新文档无可见密码框 / 本页表单消失）才落盘，失败不覆盖旧密码**，成功后右上角「已保存/删除」气泡（删除即直接删掉该站点凭据）；自动填充成功后提示「密码已自动填充」；自动填充走原型 value setter + input/change；**触发时机 = 首屏 / DOM 变动（MutationObserver）/ URL 变化（主世界 history 钩子，覆盖 pushState / replaceState；无任何常驻轮询）/ focusin —— 有空的密码框就填，不需要先点输入框**；`safeStorage` **只用异步 API**（`isAsyncEncryptionAvailable` / `encryptStringAsync` / `decryptStringAsync`，同步版在 Electron 45 已废弃））；`clearWebData` 一并清凭据；`tabs.ts` 增 `ownsWebContents`。测试：`unit` **213/213**（新增纯逻辑 8 条 + 装配契约 11 条）、`typecheck`/`build` ✅、`probe` 单窗口 **176/176**（第 18 节 18/18）、`probe --tabs` **69/69**（P13 含同项）；探针踩坑记录：`requestSubmit()` 的 submit `isTrusted=true`、`executeJavaScript(...,true)` 附带 user activation、隐藏窗口的子 `WebContentsView` 收不到 DOM 键鼠 ⇒ `docs/changes/2026-11-18.md` |
| 2026-11-16 | **修「页签模式下 `window.web2appExtra` 恒为 undefined」**（用户真机截图 + 解 asar 取证）：`tabs_enabled: true` 的产物里内容页是独立 `WebContentsView`，`tabs.ts::spawnTab` 的 `webPreferences` **没有 `preload` 字段** ⇒ extra 注入无从执行（单窗口模式正常，Tauri 页签 `tabs/window.rs` 一直有），而包内 `out/config.json` 的 `extra` 非空（`{"version":"w2a-0.0.1"}`）。修法：新增 `renderer/tab-preload.ts`（**只**暴露 `web2appExtra`，空对象不暴露，无任何窗口桥；**不许挂 `preload.js`** —— 那会往内容页塞自绘标题栏，与壳页面 `shell.html` 的标签栏叠成两条）+ 同步通道 `w2a:tab-extra`（`app-ipc.ts::registerTabExtraIpc`，闸门只到顶层框架：页签内容页 webContents 不是主窗，`expectedSenderId` 不适用；extra 本就是要注入页面的公开数据）+ `tabs.ts` 的 `tabPreload` 与 `index.ts` 装配；`test/tab-preload.test.ts` 锁「装配」六条、`probe --tabs` 新增 P2b 两条**成对**断言（读得到 extra ‖ 内容页仍无 `web2app` 桥）。unit **194/194**、`--tabs` **62/62**、单窗口 **157/158**（既有 `Shift+F1` 被占用项）、typecheck/build ✅；反向对照：改坏 `tabPreload` ⇒ unit `✖ 1/6` + 探针 `FAIL P2b（实际 null，退出码 1）`，还原后三文件 sha256 逐一相同 → `docs/changes/2026-11-16.md` |
| 2026-11-16 | **Electron 下载「两个另存为 / 保存后没有文件」三连修**（全有真机证据）：① `item.pause()` **挡不住** Electron 自带的保存框 —— 它只看 item 返回时有没有 `savePath`，于是同一次下载弹两个框（第二个标题栏是**下载地址**）；改为 `pause()` 后**同步**给占位落点 `%TEMP%\w2a-pending\<时间戳>-<名字>`，用 `EnumWindows` 数 `#32770` 做 A/B：修前 2 个（`另存为` + 下载地址）、修后 1 个；② 保存后立刻关下载页签会把下载一起掐掉（真机 `下载（另存为 #2）` 之后 19s `下载结束（cancelled）`）⇒ 关页签移到 `done`（**结算后**才关，与 Tauri `UserShutdown` 同一坑）；③ 搬移以 `item.getSavePath()` 判来源不可靠（结束后可能为空，文件永远留在 pending）⇒ 改以 `existsSync` 为准 + `moved` 幂等 + 启动 `cleanPendingDownloads()` 清残留；探针 P15 的 `w.closed` 样本改为 `window.open` 返回即采（原 300ms 定时器会被关页签截断，偶发 59/60）；探针修掉后 `--tabs` **60/60**（连跑稳定）、`unit` **188/188**、单窗口 155/158（3 条既有环境项）、typecheck/build ✅ → `docs/changes/2026-11-16.md` |
| 2026-11-15 | **双壳修「下载页签下载后空白页签不关」**：站点点击 → `window.open` 新页签指向附件（`Content-Disposition: attachment`），浏览器下载开始后自动关掉该空页签并回到点击时的页面。判据收敛为一条：**页签是否提交过文档**（未提交 → 关；已提交 → 不关，页面里点下载时保留当前页签）。**Electron**：`did-navigate` 置 `committed`，`will-download` 落盘/取消后 `closeTabAfterDownload()` 按 `opener` 回到打开者；`attachDownload` 增 `DownloadHooks`（`onSaveChoice` 在 `setSavePath`+`resume`/`cancel` **之后**回调，`pickSavePath` 供探针免弹模态框）；新增 `/dl.txt` 夹具 + 页签探针 P14 两条成对断言（空页签自动关且附件完整落盘 / 已加载页签不关），unit **188/188**、页签探针 **58/58**、单窗口 **156/158**（两条 F12 为既有 `app.isPackaged` 环境项）。**另修「一次点击两次下载 / 两个另存为」**：页签模式 `setWindowOpenHandler` 返回 `deny` ⇒ 页面 `window.open()` 得到 **null**，站点据此走「弹窗被拦截」的 `location.href` 同地址兜底，同一次点击产生两条下载（点掉第一个保存框后第二个还在）。**Electron 从根上修**：站点判据是 `if (!w || w.closed)`，`deny` 让 `w` 为 null、「建了立刻销毁」让 `w.closed` 为 true —— 两条都会兜底二次下载。故改 `allow` + `overrideBrowserWindowOptions{show:false,skipTaskbar:true}` 建**隐藏窗口**并让它**活到这次下载结算**（`registerHiddenPopup` / `attachHiddenPopupToTab` 绑到承载它的新页签，页签关闭才销毁；`onDownloadStart` 取消兜底超时），新页签承接导航与下载，隐藏弹窗自己抢跑的那条由 `isHiddenPopup` 丢弃；`noteExpectedPopupDownload`/`shouldDropPopupFallback`（同地址 + 同打开者 + 3s）降为纵深防御，页签探针 P15 断言 `window.open()` 非 null、兜底未执行、下载中 `w.closed === false`、附件完整落盘（tabs **59/59**）。**Tauri 仍只能按 URL 判重**（WebView2 `SetHandled` 后同样返回 null，要返回非 null 须 `put_NewWindow` 另建一个 WebView2，未做）：`NewWindowRequested` **同步**登记「目标 URL + 打开者」，`DownloadStarting` **弹框前**判重取消第二条（同地址 + 同打开者 + 3s；局限是兜底地址带新签名时失效）。**Tauri**：新增 `tab_doc_ready.js`（占位 `tauri.localhost` 页早退；附件下载不产生文档 ⇒ 不发事件）+ `TAB_DOC_READY_EVENT` → `mark_tab_doc_ready` 置 `committed`；`NewWindowRequested` 记 `opener`；`DownloadStarting` 取消分支立即关、确认保存/对话框不可用则等 `DownloadOperation::StateChanged` 到终态再 `close_tab_after_download()`（**不能选完就关**：销毁发起下载的 webview 会以 `UserShutdown` 中断下载且不可续传；关闭本身切后台线程 + `run_on_main_thread` 延后，避免在自身 COM 回调里销毁自己；首页/已提交不关）；纯规则 `should_close_tab_after_download` 单测 + `capability_contract` ⑨ 静态守卫（含「登记早于建窗、判重早于弹框」的顺序断言）；`cargo test --all` **324 passed / 6 ignored**（lib 249 / proxy 34）、fmt/clippy ✅；**同日 Electron 44.4.1 → 45.3.2**：45.0 唯一硬破坏面（sandbox preload 移除 `events`/`timers`/`url` shim 与 `Buffer`/`setImmediate` 全局）本壳不命中（preload 只 `require('electron')`），其余变更逐条无关（无权限处理器 / `window.open` 只建一个随即销毁的隐藏窗口 / 无 native addon 传 fd / 未用 `safeStorage`），`electron.d.ts` 核对 `setShape`、`WebContentsView`、`setWindowOpenHandler` 均在；`package.json` + lockfile 更新，unit **188/188**、单窗口 **156/158**（既有 F12）、页签 **58/58**、smoke ✅、`--dir` 出包 + asar 护栏 52 项通过 → `docs/changes/2026-11-15.md` |
| 2026-11-14 | **修「打包后双击打不开，提示服务器连接失败」**：配置的 TXT 记录名 `txt_w2a_lcjm26.*` 把下划线放在 label **中间**，而 `Resolver::txt_lookup(&str)` 走 `Name::from_utf8` → IDNA/STD3（只许字母/数字/连字符，下划线仅首字符特判）⇒ **构造查询名就失败、请求根本没发出**，三个候选域名全挂，界面固定报「服务器连接失败，请检查网络后重试」（`nslookup` 一切正常 ⇒ 极易误判成网络）。修法：`resolve.rs` 改 `Name::from_str_relaxed`（两壳共用，无需改 DNS）+ `Err(_)`→`Err(e)` 保留真实原因 + 2 条单测；`cargo test --all` 246/34 通过、fmt/clippy ✅、重打包后包内 sidecar 直接解出 `https://api2.xwamp.com/user-0408/`。⚠️ 同时发现**第二层问题**：该目标主机在 1.1.1.1 / 119.29.29.29 / 8.8.8.8 全是 NXDOMAIN，需重新签发 TXT 密文（不用重打包）→ `docs/changes/2026-11-14.md` |
| 2026-11-14 | **修「探针被打进 `app.asar`」**：Electron 出包入口有**三个** —— `electron-app/package.json` 的 `pack`/`dist*`（2026-11-07 已插 `pnpm run prune`）、**本地打包** `tests/build-electron-all.bat` → 根 `scripts/packlib/driver-electron.mjs`、**CI 自拼步骤**的 `.github/workflows/electron-build-custom.yml`（后两者都漏了）。`files: out/**` 于是把 `probe*.js` 与 `tabs-probe-preload.js` 打进包，出包仍成功、只在 `check-asar-config.cjs` 反向断言处报红。两处均补 `prune` + `probe-isolation.test.ts` 新增两条入口守卫（🔴 各自反向对照：删掉该行如期 FAILED，还原 sha256 一致）+ 文档同步；本地重跑 electron-all 退出码 0、asar 条目 **58 → 52**；`unit` **187/187**、`check-workflows.py` 通过 → `docs/changes/2026-11-14.md` |
> 完整历史（含 v1.0–v1.29 仓库搭建与文档体系建立）见 `docs/changes/`。
