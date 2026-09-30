# MEMORY.md 瘦身前的完整存档（2026-09-17）

> 存档原因：`MEMORY.md` 超出注入上限、被截断；按「压缩主文件 + 全文留档」处理，内容一字未丢。
> 本文件是 2026-09-17 13:33 时 `MEMORY.md` 的**逐字原文**（14472 字节 / 44 行）。
> 现行精简版 `MEMORY.md` 里的 A/B/C 契约均源自此，长推导请回查本文件。

---

# web2app（双壳：Tauri 版 + Electron 绿色版）

`webview-app/` = Rust+Tauri2（含 Android，仓库 `woxiangfy/web2app`）；`electron-app/` = 仅 Win 绿色版。
**共享单一真源**：`webview-app/src/{titlebar.js,loading.js,splash.html,shell.html}`、`proxy.rs`、`crates/resolve-core`→sidecar `w2a-proxy.exe`。
> ⚠️ 本文件只留「不可违反的契约」。**动手前先查详版**：`topics/archive-2026-09-16-thinbefore.md`（Tauri 侧逐条含实测）、`topics/electron-app.md`（Electron 侧逐条）。

## A. Tauri 侧铁律
1. **远程 origin 的 IPC 必过 ACL**（Win 只有 `http://<scheme>.localhost` 算 local；`127.0.0.1:port`/公网域名都是 remote）。`remote.urls` **必须端口通配** `["http://*:*","https://*:*"]`，否则走本地代理(随机端口)时窗口按钮全被拒，且**静默**（禁空 `.catch`，必须 `console.error` 原因）。注入脚本**禁止 invoke 自定义命令**（前进/后退用 `window.history`）。
2. **注入脚本**（titlebar/loading）：注入时机 = 全局对象已建、HTML 未解析 ⇒ `whenHostReady` 等宿主，硬兜底**重试到 20s**（失败置 `__web2appTitlebar=false` 供补挂）。顶部空间契约 = **`html` 不滚、`body` 滚**（见 A.3）。底部锚定被挤出下沿 ⇒ `applyChromeFit()`：贴顶 **`fixed` 下移**（正常模式下**不动 `sticky`**）、底部锚定**只改 top**、高度≈视口(±1px)的缩为 `calc(100vh-38px)`。
3. ⚠️⚠️ **「滚动条越过标题栏」的正解 = 换滚动根（`html` 不滚、`body` 滚），不是调宽度、更不是把滚动条藏掉**。Chromium 把**视口（根）滚动条绘制在所有 DOM 内容之上**（`z-index:2147483647` 压不住），其几何恒为「视口右缘 × **整个**视口高」⇒ 只要 `html` 是滚动容器，滚动条必然从 y=0 横穿标题栏，并盖住最右的关闭按钮（`elementFromPoint(innerWidth-1,20)` 返回 `null` = 点不动）。已逐一实测否掉：`width:100vw`/px 钉宽（`rect.width` 全绿而**可见**只到 `clientWidth`）、`::-webkit-scrollbar{border-top:38px}` 与 `-track{border-top:38px}`（border 不参与零件布局，thumb 仍从 y=0 起）、`OverlayScrollbar`/`FluentOverlayScrollbar`（Electron 44 已失效）、`scrollbar-width:none`（缝没了但滚动条也没了 —— 使用者否掉：「不应该强制隐藏，只是不要超过菜单栏」）。**正解**：`html{box-sizing:border-box;height:100%;overflow:hidden}` + `body{margin-top:38px;min-height:0;height:calc(100vh-38px);overflow-y:auto;scroll-behavior:inherit}`（`scroll-behavior:inherit` 为镜像 `html` 的平滑滚动）。⚠️ 必须 `margin-top` 不能 `padding-top`（padding 在盒内、滚动条仍从盒上沿画）；`min-height:0` 防页面 `min-height:100vh` 把滚动口撑高 38px（最后 38px 滚不到）。三条副作用：① `window` 级滚动 API 失效 ⇒ `shimWindowScroll()` 转发 `scrollY/pageYOffset/scrollX/pageXOffset`(getter)、`scrollTo/scroll/scrollBy`、body 的 `scroll` 事件补发到 `window`+`document`、`Document.prototype.scrollingElement→body`、`<html>` **实例**上的 `scrollTop/scrollLeft` 遮蔽转发；② `applyChromeFit` 正常模式下**不动 sticky**（`top:0` 的 sticky 天然停在 38，再加就成 76）；③ 页面用 `!important` 压掉契约 ⇒ 「html 不滚 + body 也不滚 = 页面钉死滚不动」（比原缺陷严重）⇒ `ensureScrollRoot(style)` 探测（html hidden + body auto + `body.height≈视口-38`，第三条专抓 `body{height:auto!important}`）失败即退回 html 留白旧模式，且**必须在样式表挂上之后**探测（早一步 computed style 还是页面原样 ⇒ 必然误判并把退路写进 inline style 永久压住新契约）。**判据**：`intrudeRows`（x=`innerWidth-1` 逐点 y=0..37 必须全命中标题栏）、`bodyScrollbarW>0`、`bodyScrollable`、`window.scrollTo(0,200)→scrollY≈200`+scroll 事件触发（**异步，要等一帧**）。**禁用 `rect.width==innerWidth` 或「滚动条宽度归零」当判据**（前者测不出覆盖 = 把缺陷锁死）。
4. **新窗口**：禁 `SetNewWindow(&当前 core)`（复用的文档不跑初始化脚本 ⇒ 标题栏整片消失）⇒ `SetHandled(true)` + 主线程 `window.navigate(url)`；`about:blank` 忽略。兜底：主窗 `page_load(Finished)` 再 eval titlebar.js（自带防重）。
5. **多页签**（仅 Win，`TABS_ENABLED` 默认 false）：tab 必须是宿主的 **owned window**（否则点宿主任意处都会把宿主抬到 tab 上）；关子窗用 `destroy()`**禁** `close()`（→ 无限重入卡死）；隐藏/最小化/退出分别 `set_tabs_visible(false)` / `sync_tabs_with_host` / `destroy_all_tabs()`；几何靠闭环校准 `place_tab`（set→读回→补偿 ≤3 轮）。
6. **WebView2 钩子**：`with_webview` 在主线程**内联同步**执行（闭包 sleep = 死锁+白卡）；必须后台线程 + `static HOOKS_REGISTERED` **一次性登记**（重试会让同一窗口挂多个 `NewWindowRequested` ⇒ 一次点击开 N 窗）；`done` 标志全函数共用且在**轮次开头**检查。下载：`Handled=true` 才隐藏默认 UI；另存对话框**只能主线程同步弹**，取消用 `SetCancel(true)`。
7. ⚠️⚠️ **`Builder::header` 是 append 不是 replace**（`http`/`reqwest` 皆然）：要覆盖请求头**必须在透传循环里跳过它**，否则发出两个同名 `Origin`（逗号拼接）⇒ 后端拒连，「页面能开、WS/POST 全挂」。回归测试 `proxy_sends_exactly_one_rewritten_origin`。
8. **sidecar 协议**：上游地址经 **stdin** 传（命令行对同用户进程可见 = 泄密码）；stdout **只一行握手 JSON 且必 flush**（日志走 stderr）；防僵尸三层 = `will-quit` stop → 关 stdin → 1s 强杀 → sidecar 自监视 stdin EOF（只有第三层能兜住被任务管理器结束）。
9. **配置**：`app_name`=安装名、`title`=显示名（缺省回落，编译期变量仍叫 `WINDOW_TITLE`）；`parse_domains` 只认 JSON 数组；`RESOLVE_SECRET` 两端必须同步；私钥须 48B PKCS#8 DER。
10. **构建/CI**：`env_bool!` 在 const 上下文用 `const fn str_eq`；`#[cfg]` 调用点**下沉到平台专属那行**；rustc 增量 ICE 删 `target/debug/incremental`**别 clean**；`setup-android@v4` 必须显式 `packages: 'platform-tools'`。
11. ⚠️ **Tauri 侧别指望截屏**（DWM 合成：`CopyFromScreen` 拿下层窗、`PrintWindow` 全黑）⇒ 用页面回传探针 `%TEMP%\w2a_probe2\`（`run_probe.sh <fixed|sticky|vh|bottom|inset|diag>`）。被遮挡窗口定时器被节流 ⇒ 上报要在 `load` 里同步发。

## B. Electron 绿色版（仅 Win10+ · zip + 单文件 portable · 数据写 exe 同级 `data\` · 不签名）
进度 **M0–M3 ✅ / M6 🟡 出包已通 / M4 ⬜ / M5 暂缓**；Tauri 版继续维护 ⇒ 双壳并行、契约对齐是硬要求。
1. ⚠️⚠️ **所有 electron 命令经 `scripts/launch.cjs`**：宿主注入 `ELECTRON_RUN_AS_NODE=1` 会让 Electron 以**纯 Node** 启动（`require('electron')` 返回路径字符串，报错位置却落在我们代码里）。**禁用 `env -u`**（切断宿主 stdout 管道 ⇒ 退出码 0 却零输出），**也别清 `NODE_OPTIONS`**。
2. ⚠️⚠️ **注入脚本必须跑主世界**（`webFrame.executeJavaScript`）：隔离世界看不到 `contextBridge` 的 `window.web2app` ⇒ 「标题栏画得出来、按钮全点不动」，而 DOM 跨世界共享、表象看着是对的。
3. ⚠️ 严格 CSP 站点拦 `<style>` ⇒ 用 `document.adoptedStyleSheets` 挂样式（不受 CSP 约束），保留 `<style>` 回退。
4. ⚠️⚠️ **「exe 同级」一律走 `installRoot()`**（portable 用 `PORTABLE_EXECUTABLE_DIR`；开发态 `getAppPath()`）；userData/sessionData/logs/crashDumps **四项一起改**且在 ready 之前；同级不可写要**真写文件探测**后回退 `%APPDATA%`。
5. ⚠️ **窗口标题必须显式传** `new BrowserWindow({ title: cfg.title })`：拦了 `page-title-updated` 之后漏传**不会**退回页面标题，而是永远停在 package.json 的 name（`smoke` 已加断言钉住）。
6. ⚠️ **打包前必须 `npm run build`**：tsc 默认「有错也输出」会把半成品 JS 留在 `out/` 封进 asar（现象：双击弹 `xxx is not defined` 且**进程不退出**）⇒ 已开 `noEmitOnError: true`。
7. `config.json` 经 `stage-assets.cjs`→`build/config.json`→`extraFiles` 发到 **exe 同级**（绿色版预置一份）；`loadConfigAny([用户目录, resourcesPath/..])` 兜住单文件形态。**electron-builder 26 无顶层 `zip:` 块**（写了拒绝启动）。
8. `settings.json`/`window-state.json` 写 **`%APPDATA%\<app_name>\`**（为与 Tauri 版互通；`data\` 有同名则以它为准）；设置页 HTML **不重写**，靠设置窗 preload 垫片提供 `__TAURI_INTERNALS__.invoke`；几何**物理像素存盘 / DIP 恢复**（否则跨 DPI 错位）；探针把这两个 JSON 重定向到 `%TEMP%\w2a-probe-store\`。
9. ⚠️ 静默陷阱：托盘「退出」必须先置 `quitting=true`（否则被「关闭到托盘」拦下）；`globalShortcut.register()` 失败**必须把旧键注册回去**；显隐语义要 `visible && !minimized`。
10. **验证**：`npm run probe`（**112 条**，动过注入脚本/代理/设置链路**必跑**）· `smoke` · `shot`（布局取证：四条判据 + PNG）· `lab`（滚动条多模式对照）· `cargo test`+clippy。⚠️ 打包产物**没有 stdout**（portable 连内层进程 stdout 都被吞）⇒ 只能靠旁证：窗口标题、exe 同级 `data\`、缓存里 grep 目标域名。
11. ⚠️ 本沙箱**禁止一切删除**（safe-delete）、**父目录 rename 因子目录被锁必失败** ⇒ 出包绕法 = `-c.directories.output=<新目录>` + copy 归位；别设计「构建→rename」流水线。

## C. 反复应验的教训
1. **断言的期望值若取自「实现当前的表现」，就等于把缺陷锁死。** 例：探针曾断言「标题栏宽 == clientWidth」并注释「差一个滚动条宽是正常行为」—— 那条 15px 的缝因此被保护起来，直到使用者截图反馈「滚动条显示不合理」。写断言前先问一句：**这个数字是「我想要的」，还是「现在是这样的」？**
2. ⚠️ **`getBoundingClientRect()` 只证明布局，不证明可见。** 同一个坑踩了第二次：改成断言「`rect.width == innerWidth`」后全绿，但那 15px 是被滚动条在**绘制**层盖掉的 —— rect 测不出来。**判断「有没有被遮住」只能用命中测试（`elementFromPoint`）或像素。** 另有配套陷阱：`Read` 看大图会缩略（1656px 宽看起来像 1064px），一眼「好像铺满了」；要看像素就用 `scripts/png-inspect.cjs`。
3. **沙箱里 `Add-Type` 被安全策略拦、没有 PIL** ⇒ PNG 取证走 `node:zlib` 手解（`scripts/png-decode.cjs`）。另：`capturePage()` 在无 GPU 环境会抛 `UnknownVizError`，**必须包 try/catch**，否则调用方进程直接挂住不返回（截图只作附加信息，不得参与退出码）。
4. **一跑就「卡住几分钟不返回」的排查顺序**：先看输出文件是否已写完（常是判定已出、进程没退）→ 再查是否有 `await` 抛错后没人 `app.exit()`（`capturePage` 的 `UnknownVizError` 是惯犯）。
5. ⚠️⚠️ **验证打包产物（GUI 子系统 exe）时，`& "app.exe" --probe` 是假绿**：PowerShell 对 GUI 程序**不等待**，`$LASTEXITCODE` 立刻为 0、日志 0 字节 —— 每层证据都在说「成功」。必须 `Start-Process -Wait -PassThru` 取 `$p.ExitCode`；再加独立旁证：**跑前删掉探针的重定向目录，跑后确认它被重新创建**（同时排掉「单实例锁静默 quit」和「进程没起来」）。
6. **受限网络里 electron-builder 会卡在下载 electron 运行时**（`%LOCALAPPDATA%\electron-builder\Cache` 下只有 `7zip`/`nsis-*`/`downloads`、无 `electron` 就是没下过）⇒ 绕过：`"-c.electronDist=node_modules/electron/dist"`，几十秒出包。副作用：会把该目录里的历史 `debug.log` 一并带进包（无害）。**另：命令默认 120s 超时会掐掉 zip/portable 这类长构建**，表现为 `exit=1`、日志停在 `building target=portable` ⇒ 显式给 600s（`zip` 常已产出，先 `ls` 再决定要不要重跑；`portable` 若因 safe-delete 拦 `emptyDir(locales)` 就换全新输出目录）。
7. ⚠️⚠️ **「问题依旧」先查跑的是哪一版包，别急着改代码。** 一次 `dist` 同时产出 `dir`/`zip`/`portable` 三份且旧的不清，仓库里还并存 `dist/`、`dist-fix/`、`dist-v2/`… ⇒「源码已修好」与「现象依旧」可同时为真（2026-09-17 实测：探针 103/103 全绿，用户仍在报同一现象，一查他跑的那份包注入脚本还是修复前的）。判据：①`<包>\resources\inject\*.js` 与 `webview-app/src/*.js` 的 sha256；②`<包>\data\` 与 `%APPDATA%\<app_name>\window-state.json` 的 mtime（早于修复时刻＝旧包）。对齐用**覆盖式 copy**（不删除）。⚠️ **只换注入脚本不换 `app.asar` 会让包内旧探针报假红**（旧断言「该页确有滚动条」与新行为冲突 ⇒ 退出码 1）。成套装法：`Web2App.exe` + `resources\app.asar` + `resources\inject\*.js` + `config.json` 四件一起覆盖（`data\` 保留）。
8. ⚠️⚠️ **「消灭了缺陷」不等于「满足了需求」——用更差的替代品换掉症状同样是失败。** 2026-09-17 为了让滚动条别越过标题栏，先上了「把滚动条宽度归零」：缝没了、断言全绿、验收指标也对，但使用者的原话是「**滚动条不应该强制隐藏，只是不要超过菜单栏而已**」—— 他要的是「仍在 + 位置不越界」，我只满足了「不越界」。**动手前先把需求翻译成一句可观测的判据**（此处 = 滚动条可见 ∧ 最高点 ≤ 38px），任何方案必须同时满足两条；只满足一条 = 换了个缺陷。
9. **修「侵入类」缺陷时，判据要成对：位置 + 存在性。** 只断言「不许侵入」（`intrudeRows == []`）会被「干脆把那东西删掉」满足；只断言「存在」（`bodyScrollbarW > 0`）会被「存在但压在上面」满足。两条一起才是完整的验收 —— 这一对断言正好就是 C.8 那句需求的可执行形式。
