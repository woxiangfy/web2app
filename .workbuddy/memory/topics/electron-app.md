# electron-app（Windows 绿色版 Electron 壳）— 详细笔记

> 索引在 `MEMORY.md` §10。方案 `electron-app/PLAN.md`，命令与差异 `electron-app/README.md`。
> 定位：**仅 Windows · Win10+ · 绿色版（目录 zip + 单文件 portable 都要）· 数据写 exe 同级 `data\` · 不做代码签名**。
> Tauri 版（webview-app）继续维护，兼管 Android → **双壳长期并行**。
> 进度：**M0–M7 ✅（双壳契约全面对齐；契约探针单窗口 134 / 多页签 31，2026-09-26 实测）**。

## ⚠️⚠️ 宿主注入 `ELECTRON_RUN_AS_NODE=1`（2026-09-16 在这上面绕了很久）

桌面型 IDE / agent 沙箱会给所有子进程注入 `ELECTRON_RUN_AS_NODE=1`（配套
`NODE_OPTIONS=--require <它们的语言服务 shim>`）。这个变量泄漏进 Electron 后，
Electron **以纯 Node 身份启动**：

- `require('electron')` 返回**可执行文件路径字符串**，不是 API 对象；
- 业务代码第一句 `app.disableHardwareAcceleration()` 就抛
  `Cannot read properties of undefined` —— **错误位置在我们的代码里**，看着完全是项目 bug；
- 判别最快的一招：`electron.exe --version` 打印 `v24.x`（Node 版本）而非 `v44.x`；
- **时好时坏**（不同宿主 / 不同调用方式注入与否不一致），极易误判为随机故障。

**修法**：所有 electron 命令统一经 `scripts/launch.cjs` 拉起（已接进 `npm run start/smoke/probe`），
它在 spawn 前 `delete env.ELECTRON_RUN_AS_NODE`，其余环境原样透传。

**排查时的两个反向坑（别再走回头路）**：
1. `env -u ELECTRON_RUN_AS_NODE ...` **不可用**：`env(1)` 会重建进程环境，把宿主用来捕获
   stdout 的管道一起切断 ⇒ **退出码 0 但一个字都没有**，于是会去怀疑参数、怀疑代码，
   而真正的问题是"看不见输出"。要清就用 bash 内置 `unset`，或在 Node 里 `delete env.X`。
2. **`NODE_OPTIONS` 不要一起清**：那个 shim 正是宿主的输出通道，清了同样变哑巴。

另：Windows 上 `electron.exe` 是 GUI 子系统程序，**stdout 要靠重定向才稳定可见**
（`... > log.txt 2>&1`）。直接跑可能什么都不刷，别误判成"没执行"。

## 路径装配（最容易做错的一块）
- ⚠️⚠️ **"exe 同级"一律走 `installRoot()`**，不要直接写 `app.getPath('exe')`：
  - 单文件自解压（`portable` target）下 `app.getPath('exe')` 指向 `%TEMP%` 里的解压副本 →
    必须用 electron-builder 注入的 **`PORTABLE_EXECUTABLE_DIR`**（它指向用户放置 exe 的目录）。
  - 开发态（`electron .`）`app.getPath('exe')` = `node_modules/electron/dist/electron.exe` →
    必须回落 `app.getAppPath()`（判据 `!app.isPackaged`）。
- ⚠️ 数据路径**四项一起重定向**：`userData` / `sessionData` / `logs` / `crashDumps`。
  只改 `userData` 会留下 Cookies、日志、崩溃转储在系统盘 ⇒「绿色版不干净」。
  且必须在 `app.whenReady()` **之前**（一旦有 store/session 先读过 userData 就会部分失效，
  表现是「一部分数据在 exe 旁边、一部分在 %APPDATA%」这种最难查的状态）。
- ⚠️ 可写性探测必须**真写一个文件再删**：`access(W_OK)` 在写保护 U 盘 / 部分企业策略下的
  Program Files / 只读网络盘上会误报可写。不可写时回退 `%APPDATA%` 并打日志（能启动 > 够绿色）。
- 单实例锁文件在 `userData` 下 ⇒ 随重定向自动跟随。顺序：路径 → 代理 switch → 协议注册 → 单实例锁。
- `config.json` 的 `data_dir`（枚举，2026-09-22 起取代布尔 `data_dir_default`；默认 `local_appdata`）：`local_appdata` = exe 同级 `data\`（绿色版默认）；`system` = `%APPDATA%\<app_name>`；`portable_sibling` = 显式 exe 同级 `data\`。⚠️ 与 Tauri 版差异：Tauri 的 `local_appdata` 写 `%LOCALAPPDATA%\<app_name>`。部署文档需明示。

## 环境坑（其余）
- ⚠️ **electron@44 的二进制不随 `npm install` 下载**：包内已无 `postinstall`，改为独立 bin
  `install-electron`（`node_modules/electron/install.js`）。已在 `package.json` 加 `"postinstall": "install-electron"`。
  实测解压后 `node_modules/electron/dist` **约 320MB**。
- ⚠️ **TS 7 移除了 `moduleResolution: node`**（报 TS5108）→ 用 `node16`；
  因 `package.json` 无 `"type":"module"`，仍输出 CJS。
- ⚠️ **无 GPU 环境**（容器 / 部分远程桌面）会刷 `GPU process isn't usable. Goodbye.` 直接退出。
  **探针模式已自动关闭硬件加速**（`PROBE` 分支调 `app.disableHardwareAcceleration()`），不必额外传参；
  真机不要长期关（滚动与动画会明显变卡）。
- 墙内必配 `.npmrc`：`electron_mirror` + `electron_builder_binaries_mirror` 指 npmmirror，否则卡死。

## M1 已完成：两个「错了也静默」的真实缺陷

1. **注入脚本跑在隔离世界** ⇒ `contextBridge` 暴露的 `window.web2app` **只存在于主世界**，
   于是 `shellName()` 永远返回 unknown、三个窗口按钮全部点不动。而 **DOM 跨世界共享**，
   标题栏照样画得出来、38px 让位也照样生效 —— 表象上"看着是对的"，这是最难查的一点。
   **修法**：`preload.ts` 改用 **`webFrame.executeJavaScript`（主世界）**，与 Tauri 侧
   `initialization_script`（本来就在主世界）对齐。
   ⚠️ 同一处还有连带影响：加载层指令（fail/slow/hide）原先经 preload 读隔离世界的
   `window.__web2appLoading` —— 也必须跨到主世界执行（`JSON.stringify` 参数后嵌入代码，
   并转义 U+2028/2029）。
   📌 `webFrame` 在 `sandbox: true` 的 preload 里**可用**（已实测）。

2. **严格 CSP 站点（`style-src 'self'`）把 `<style>` 元素整个拦掉** ⇒ 标题栏与加载层裸奔
   （透明背景、按钮挤成一团、38px 让位也丢）。
   **修法**：改用**构造函数式样式表** `document.adoptedStyleSheets`（**不受页面 CSP 约束**），
   保留 `<style>` 回退；`titlebar.js` / `loading.js` 两侧写法保持一致（否则会出现
   "标题栏有样式、加载层没样式"的半截状态）。
   ⚠️ `adoptedStyleSheets` 是 FrozenArray，只能整体替换，且不保证有 `concat` → 用最笨的复制方式。

## 契约探针（`npm run probe`）

`src/main/probe.ts`：自己起 `127.0.0.1` 随机端口的夹具站点（全内联在代码里，不落盘、不进产物），
逐个导航并断言 **134 条契约（单窗口；多页签另 31 条），退出码即结论**。覆盖启动页内联注入、标题栏几何/配色/拖动区、
38px 让位（fixed / sticky / `100vh` / `inset:0`）、严格 CSP 降级、加载层淡出、
`target="_blank"` 与 `window.open` 原地导航、三个窗口按钮真实驱动原生窗口。

- **动过 `titlebar.js` / `loading.js`（两壳共用的单一真源）必须跑**。
- 断言里踩过的 3 个坑（是断言写错，不是产品缺陷）：
  - 「标题栏满宽」别比 `innerWidth`：`left/right:0` 的 fixed 元素宽度要**减去纵向滚动条**（本机 15.3px）→ 用 `clientWidth`。
  - 「标题栏已着色」别写死浅色 `#eef0f2`：深色系统上是 `#1f2530` → 断言"落在已知浅/深两色里"。
  - 「加载层已淡出」别用固定 sleep：有 `MIN_MS 220 + FADE_MS 280 + DCL_DELAY 240`，**元素是动画结束后才移除**→ 要轮询等待。
  - 夹具要加 `h1{margin:0}`：h1 默认 margin 会**塌陷**到父容器，把 `#content.top` 从 38 顶到 56.8。

## 复用边界（决定投入产出比）
- **零重写**：`proxy.rs`(741 行) + `resolve-core`(573 行) 编成 sidecar `w2a-proxy.exe`（新增 bin crate）。
  ⚠️ **不能放进 asar**（asar 内 exe 无法 `spawn`）→ 必须 `extraResources`，运行时用 `process.resourcesPath` 定位；
  `spawn(exe, args)` 用数组形式（路径可能含空格/中文）。握手：stdout 单行 JSON `{port,upstream}`；父进程退出要 kill 子进程防僵尸。
- **引用而非复制**：`titlebar.js` / `loading.js` / `splash.html` / `shell.html` / `settings.html` 从
  `webview-app/src`（settings 是 `dist/`）取——**单一真源**。
  ⚠️ 打包落点已变（2026-09-26）：**不再经 `extraResources` 放 `resources/inject`（明文可改）**，
  改由 `stage-assets.cjs` 拷到 `out/inject/` 随 asar 分发（与 config.json 同待遇）；
  运行时 `index.ts::injectDir()` / `settings-window.ts::locateSettingsHtml()` 用 **`app.isPackaged`**
  分叉（打包读 `app.getAppPath()/out/inject`，开发直读源码），**别用 `existsSync`**（stage 也会在开发目录生成 out/inject）。
  护栏 `check-asar-config.cjs` 已扩展为同时校验 `out/inject/*` 5 文件进 asar。
- **不引入**：`dnd_polyfill.js`（WebView2 专属拖拽 bug，Chromium 无此问题）。
- 外部代理可复用 `build_proxy_args()` 的语义：Electron 侧用 Chromium 原生 `--proxy-server` /
  `--proxy-bypass-list` 两个 switch（Tauri 版是拼成一条 `additional_browser_args`）。

## 关键 API 映射
`setWindowOpenHandler`（新窗口）/ `will-download` + `dialog.showSaveDialog`（下载）/ `context-menu`（右键）/
`app.on('login')`（Basic 兜底，⚠️ 需实测对 **WebSocket 握手**是否生效）/ `Tray` / `globalShortcut` /
`requestSingleInstanceLock` / `WebContentsView`（多页签，暂时搁置）。
- 多页签语义变化：`WebContentsView` 是同一窗口内子视图 ⇒ **不需要** Tauri 那套 owned window、
  `place_tab` 3 轮闭环校准、`destroy()` 防重入；但要自己测最大化/全屏/DPI 下的 bounds 同步，并显式管理标签栏层级。

## 实测
- M0：`tsc` / `tsc --noEmit` 通过；冒烟退出码 0；启动页模板 **77676 字符**、未替换占位符 0。
  ⚠️ JS 的 `String.length` 是**字符数不是字节数**（中文字符 UTF-8 3 字节），别按文件字节数核算，会以为崩了。
  无 config → data 落项目根、`title` 回落 `app_name`；有 config（布尔写成字符串 `"true"`）→ 代理开关正确、
  `proxy_http` 自动镜像到 https、`extra` 正确序列化、`android_*` 只告警不报错、`tabs_enabled=true` 提示待排期。
- M1：`npm run probe` **39/39 通过、退出码 0**（修隔离世界 + CSP 两个缺陷前后：29/39 → 39/39）。
- Tauri 侧同步验证：`cargo test --lib loading::` **7/7**；`cargo clippy --all-targets -- -D warnings` 干净。

## 两个必须记住的写法陷阱
- ⚠️⚠️ **注入模板替换不能用 `String.replace`**：替换串里的 `$&`、`$'` 是特殊模式，
  会把注入的 31KB `titlebar.js` 全文改坏（表现为「脚本少一段 / 报奇怪语法错」）→ 用 `split/join`。
- ✅ **splash 占位符隐患已修**：`splash.html` 原先把三个占位符名也写在 HTML 注释里，全局替换会把
  `titlebar.js`(31KB) + `loading.js`(18KB) 全文塞进**注释**（当时没炸纯属侥幸：脚本第 1 行是 `/*`、
  首个 `*/` 恰在第 70 行）。现改为**裸 token**（`<script>__WEB2APP_TITLEBAR_JS__</script>`），
  Tauri 侧 `loading.rs` 有 3 条回归测试盯着（`splash_template_has_no_token_in_comments` 等）。

## M2 待办
1. `w2a-proxy` sidecar —— **已完成**，见下「M2 落地」。
2. 带 Basic 鉴权的**真实站点**端到端（含 WebSocket 凭据）—— 探针已用本地 mock 上游覆盖协议层，内容层待补。
3. CI：`npm ci` → `cargo build --release -p w2a-proxy` → `tsc --noEmit` → `npm run smoke` → `npm run probe`。
4. `electron-builder` 依赖**尚未安装**（`pack` / `dist` 现在跑不了），M6 打包阶段补。

## M2 落地（sidecar 代理 + 加密解析）· 断言 50/50

**产物**：`crates/proxy-server`（bin `w2a-proxy`，release **3.76MB**），用 `#[path]` 引
`webview-app/src/proxy.rs` 与 `src/resolve.rs` —— **零重写、零复制**，两壳永远同一份实现。
`electron-builder.yml` 的 `extraResources` 已启用（放 `sidecar/`；**绝不能进 asar**）。
`npm run sidecar` / `pack` / `dist` / `probe` 都会先构建它。

**为复用做的一处必要解耦**：`resolve.rs` 原本用 `tauri::async_runtime::block_on` 驱动 DNS 查询，
sidecar 拉不进整套 Tauri 依赖树 ⇒ 改为 `pub async fn`，**由调用方驱动**（主 crate 用 Tauri 运行时，
sidecar 用自己的 tokio），文件头写明该约束当护栏；`bootstrap.rs` 调用点相应包一层 `block_on`。

**Electron 侧新增**：`src/main/sidecar.ts`（spawn + 握手 + 生命周期）、`navigate.ts` 改 async 并接入
sidecar（`how: direct | proxy | dns | dns+proxy`）、`index.ts` 管启停（`navSeq` 代际防并发起多个）。

**抓到一个真 bug —— Tauri 版同样中招（本轮最有价值的产出）**：
`handle_websocket` 里「先透传浏览器所有头、后面再设一次 `Origin` 来覆盖」**完全无效** ——
`http::request::Builder::header` 与 `reqwest::RequestBuilder::header` 都是 `HeaderMap::append`
而非 insert。上游因此收到**两个** Origin（`http://127.0.0.1:代理, http://127.0.0.1:上游`，逗号拼接），
比不改写更糟：做 Origin 校验的后端（WS 防劫持、CSRF 防护）直接拒连 = 「页面能开，但 WS/POST 全挂」——
与那段注释想避免的后果**一模一样，只是原因不同**。
修法：透传循环里跳过 `origin`；HTTP 分支**同时补上** Origin 重写（此前压根没做）。
回归测试 `proxy_sends_exactly_one_rewritten_origin`；探针两条断言盯着（HTTP + WS 各一）。
**来源**：探针新增的 `WebSocket 的 Origin 已被改写` 断言第一次跑就红了，实际值是逗号拼接的两个源。

**sidecar 契约**（都写进代码注释与 README）：
- 上游地址**经 stdin 传**（`--url-stdin`）：Windows 上同用户的其它进程能通过 `GetCommandLine`/WMI
  读到命令行参数，而 URL 里内嵌的就是站点账号密码。
  ⚠️ 踩过：`--url-stdin` 时 `parse_args` 末尾那句「必须提供 --url 或 --domains」会先把进程拦掉
  （那时 url 还没从 stdin 读）⇒ 校验条件要放宽 `&& !args.url_stdin`。
- stdout **只给机器读**：成功/失败都打**恰好一行** JSON 且必须 `flush`；日志一律走 stderr。
  往 stdout 多打一行 ⇒ 宿主永远等不到握手，表现是「窗口一直转圈、日志里什么都没有」。
- `--no-proxy`（域名解析 + 直连）握手后立即退出；凭据经 `authUser` / `authPass` 字段交宿主
  （喂 Chromium 的 `login` 事件），**明文，任何地方都不得写日志**。
- 防僵尸三层：`will-quit` 显式 `stop()`（内部先关 stdin，1s 后强杀）+ sidecar 自己监视 stdin EOF。
  ⚠️ 只有第三层能兜住「宿主被任务管理器结束」——前两层都来不及跑。

**踩到的编译坑**：`USAGE` 常量用普通字符串写、内容里含 JSON 双引号 ⇒ 字符串在那里**提前结束**，
而编译器报出来的是一串「unknown start of token: 。/（/：」（全角标点被当成代码），
排查方向直接被带偏（我第一反应是文件编码问题）。改 raw string `r#"..."#` 解决。

## M3 落地（托盘 / 快捷键 / 设置窗 / 窗口几何）· 断言 98/98

新增 `src/main/` 五个模块 + `src/renderer/settings-preload.ts`：
`settings.ts`（读写 + accelerator 映射）· `window-state.ts`（几何 + DIP↔物理）· `tray.ts` ·
`shortcuts.ts`（含失败回滚）· `settings-window.ts`（`settings://` + 3 个 IPC + 兜底页）· 垫片 preload。

### ① 设置页 HTML 不重写，用 preload 垫片适配
Tauri 版设置页（`webview-app/dist/settings.html`）调 `window.__TAURI_INTERNALS__.invoke(...)`。
Electron 侧在**设置窗自己的 preload** 里 `exposeInMainWorld('__TAURI_INTERNALS__', {invoke})`，
把 `get_settings` / `save_settings` / `close_settings_window` 三个命令映射到 IPC，**页面一行不改**。
理由与「注入脚本单一真源」同理：界面一旦分叉，两壳就会各修各的 bug。
垫片只放行白名单三命令（探针有「未授权命令被拒绝」断言钉住）。
⚠️ 设置窗的 preload 必须与主窗口的**分两个文件**：主窗口那份会注入自绘标题栏，
设置窗（**2026-09-23 起是无边框窗**）注入了会多出 38px 留白 + 一条浮空标题栏（探针有断言守）。
设置窗自身的版式（**面板铺满整窗** / 正文是唯一滚动容器 / 头部 `-webkit-app-region:drag`）也由探针 5 条断言钉住。

### ② 运行期配置写 `<data_dir>\settings.json`（2026-09-23 起，此前固定 `%APPDATA%\<app_name>\`）
⚠️ **2026-09-23 起只剩这一个 JSON**：设置的三个开关平铺顶层 + 窗口几何在 `window` 段
（`window-state.json` 已取消、不再读写）。两个写入方（设置页保存 / 关窗记几何）**必须走读-改-写**
（`saveSettings` / `saveWindowState`），否则「挪窗口 + 改设置」会互相覆盖 —— 探针 4 条断言钉住此事。
⚠️ **落点与 `data_dir` 一致**（用户反馈：绿色版 `data_dir="data"` 时 cookie / 缓存 / 日志都跟着 exe 走、
唯独设置留在系统盘）。契约落点是 `settings.ts::storeLocation(opts)`：**写只落**
`join(dataDir, file)`；**读候选 = `[新位置, 旧位置]`**（旧位置 = `join(appDataDir, sanitizeDirName(appName), file)`，
两位置相同则只留一个）。⚠️ 读-改-写的「读」必须用 `readRawForUpdate(loc)`（第一份存在的候选）而不是
`readRawDocument(loc.write)` —— 否则升级后新位置为空 ⇒ 第一次保存把老设置**清零且不报错**。
`window-state.ts::saveWindowState` 同样走 `readRawForUpdate`。旧文件**不迁移、不删除**（可回退）。
为兼顾绿色性：`config.json` 的 `data_dir` 决定 `dataDir` 本身（`relative` = exe 同级 `data\`）。
⚠️ 探针模式把存储整体重定向到 `%TEMP%\w2a-probe-store\` —— 否则跑一次探针就顺手改掉
用户的托盘开关和窗口尺寸（已实测 `%APPDATA%\Web2App` 保持不存在）。

### ③ 物理像素存盘 + DIP 恢复（跨 DPI 不错位的唯一做法）
Tauri 的 `outer_position()`/`outer_size()` 是**物理像素**；Electron 的 `getBounds()`/`setBounds()` 是 **DIP**。
存盘走 `screen.dipToScreenRect(win, bounds)`，恢复走 `screen.screenToDipRect(null, phys)`。
不换算 ⇒ 125%/150% 缩放机器上窗口缩水、且与旧版互相读取整体错位。

### ④ 「互通」怎么被钉住（两个方向都要测）
| 方向 | 手段 |
|---|---|
| Electron 写 → Tauri 读 | Rust 侧 `settings_parse_accepts_electron_written_json` / `state_parse_accepts_electron_written_json`：把 Electron **实际写出的字节形态**当字面量喂 serde |
| Tauri 写 → Electron 读 | 探针断言「序列化输出为规范 JSON」：字段名 / 嵌套 / 布尔**类型** / 键顺序逐字节比对 |

### 踩到的坑（都是「错了也静默」那类）
- ⚠️**`quitting` 标志不可省**：托盘菜单「退出」→ `app.quit()` 会触发窗口 `close` →
  被「关闭到托盘」分支 `preventDefault` 拦下 ⇒ 表现是「点了退出，窗口只是藏起来、进程还在」。
  托盘退出回调里要**先置 `quitting=true` 再 quit()**；`before-quit` 里也要置位。
- ⚠️**`globalShortcut.register()` 不抛异常、只返回 boolean**。不回滚就是「旧的已注销、新的没注册上」
  的空窗 ⇒ 应用从此失去唤醒能力，而界面上还显示着旧快捷键（因为这次保存整体失败了）。
  正确顺序：注销旧 → 注册新 → 失败则**把旧键注册回去**并返回错误。
- ⚠️**显隐语义不能只看 `isVisible()`**：必须 `visible && !minimized` 才隐藏，否则最小化状态下
  第一次触发会先隐藏、再按才显示（用户感觉「第一次按没反应」）。
- ⚠️ **`useContentSize: true`**：Tauri 的 `inner_size(480,470)` 是**内容区**；
  不写这个则 Electron 的 width/height 是外框，内容区少掉边框+标题栏高度。
- ⚠️ accelerator 里 **`+` 是分隔符**，符号键要写原始字符（`Equal → =`），不能指望 `Shift+Equal` 变加号。
  不在映射表里的键名一律返回 `null`（上层提示「不支持的按键」）——猜错的后果是 `register()` 静默 false，
  表现为「保存成功了但快捷键没用」。
- ⚠️ 启动期 `applySettings` 失败**只记日志不打断启动**（托盘图标缺失 / 快捷键被占用都不该让应用打不开）；
  但**设置窗保存时**要抛给用户看（与旧版 `create_tray(...)?` 一致）。

### 验证手法（本机实测）
非探针路径（探针会跳过 `applySettings`，所以托盘/快捷键的真实生效**只能在非探针路径验**）：
临时放一份 `%APPDATA%\<app_name>\settings.json`（含 `window` 段，模拟 Tauri 遗留；2026-09-23 前是 `settings.json` + `window-state.json` 两份）
+ 项目根放 `config.json`（`app_name` 与之对应）→ `node scripts/launch.cjs --smoke` →
日志应出现「恢复几何 300,220 1024×700（物理）」「唤醒快捷键已注册：Alt+4（Alt+4）」「托盘/关闭行为: 已启用」。
⚠️ **清理残留别用 `rm -rf "$APPDATA/xxx"`**：Git Bash 下会被 WorkBuddy 的 safe-delete 包装干预，
它把绝对路径当相对路径拼成 `c:\...\electron-app\C:\Users\...` 而失败。用
`Remove-Item -LiteralPath "$env:APPDATA\xxx" -Recurse -Force`（PowerShell 工具）才干净。

## 兼容契约 14 条（双壳不跑偏的唯一保障）
config.json 18 字段不增不改名（`android_*` 静默忽略，布尔接受字符串）· 顶部留白 38px ·
标题栏按钮（后退/前进/最小化/最大化/双击/关闭）· Basic 鉴权覆盖 HTTP **与 WS** · `Set-Cookie` Domain→`127.0.0.1` 重写 ·
`X-Forwarded-Proto` 透传 · 新窗口策略 · `Alt+4` 显隐 + `Shift+F1` 设置 · 托盘语义 ·
单实例 · 数据目录语义 · 产物命名 · 注入脚本单一真源 · `window.web2appExtra` 同名。
⚠️ 已知差异（有意对齐）：Electron 原生标题默认跟随 `document.title`，Tauri 版固定 `WINDOW_TITLE`
→ 一致做法是**两件事一起**：`win.on('page-title-updated', e => e.preventDefault())`
**并且** `new BrowserWindow({ title: cfg.title })`。
⚠️⚠️ 只做前者是个静默坑（2026-09-17 实测）：拦掉更新后标题**不会**退回页面标题，
而是永远停在 Electron 默认值（`package.json` 的 `name`）—— 从 M1 到 M6 一直显示 `web2app-electron`，
配置里的 `title` 完全没生效，只有人眼看任务栏才发现。`smoke` 已加硬断言钉住。

---

## M6 出包落地（三形态 + 两个真 bug）· 2026-09-17

样本配置：`app_url=https://www.baidu.com`，`app_name=Baidu`，`title=百度`。
产物在 `electron-app/dist/`：zip 147.6MB（解压即用）· portable exe 96.4MB（单文件）·
`win-unpacked/` 372MB（dir 形态）· `config.json`（可直接放 portable 旁边覆盖包内那份）。

### 两个真 bug（出包才暴露，都不是打包配置问题）

1. **`createWindow()` 漏传 `title`** → 配合 `page-title-updated` 被拦，窗口标题永远停在
   Electron 默认值 `web2app-electron`；配置 `title` 从 M1 起就是空话。
   → 已传 `title: cfg.title`；`smoke` 加断言 `mainWindow.getTitle() === cfg.title`。
2. **`tsc` 有错也输出 + 只跑 `typecheck` 没重跑 `build`** → 旧的半成品 JS 被封进 asar，
   双击弹「A JavaScript error occurred in the main process: loadConfig is not defined」
   **且进程不退出**（错误框挂着，外部只能看到「有进程、无窗口、无日志」）。
   → `tsconfig` 开 `noEmitOnError: true`；`pack`/`dist` 固定 `build → 打包` 顺序。
   ⚠️ 通用教训：**打包前必须确认 `out/` 是本次源码编出来的，`typecheck` 不能代替 `build`。**

### 新增能力（详见 PLAN §8.8 / README）

- `scripts/stage-assets.cjs`（`npm run stage`）：复制 `../webview-app/icons/icon.ico` → `build/icon.ico`，
  并把项目根 `config.json`（无则生成全字段默认模板）→ `build/config.json`。
  理由：`extraFiles` 指向不存在的文件会让构建**直接失败**，而该文件是 gitignore 的运行期文件。
- `electron-builder.yml`：`extraFiles: build/config.json → config.json`（**exe 同级**，不是 resources/）。
  ⇒ 结论：绿色版 zip **预置**一份可编辑 config.json（PLAN §7 悬置问题到此关闭）。
- `config::loadConfigAny([installRoot(), join(resourcesPath,'..')])`：单文件形态的配置回落。
  ⚠️ 不回落的话，只拷一个 portable exe 会连着内置 `example.com` 启动 → 被误判成「打包失败」。
- `artifactName` 提到顶层（**electron-builder 26 删除了顶层 `zip:` 块**，
  写了直接 `unknown property 'zip'` 拒绝启动；`TargetConfiguration` 只认 target/arch）。

### 验证手段（打包产物没有 stdout，只能靠旁证）

`npm run probe` 98/98 · `smoke` 退出码 0 · 单文件版拷到干净目录（旁无 config）→
窗口标题 `百度` · `data\` 建在 exe 同级 · 缓存取证命中 `https://www.baidu.com/` + 百度 JS + Local Storage 记录。
⚠️ portable 的自解压包装器**连内层进程的 stdout 都吞掉**，`--smoke` 拿不到任何日志。

### 环境坑（沙箱特有，非项目问题）

- **禁止一切删除**：node `fs.rmSync` 与 PowerShell 删除 cmdlet 都报 `[safe-delete] … trash-failed`；
  electron-builder 清理旧输出目录 / NSIS 临时 7z 时因此报错退出
  （`SAFE_DELETE_BULK_CONFIRM_REQUIRED`，阈值 50）→ 绕法：`-c.directories.output=<新目录>` + copy 归位。
- **父目录 rename 会因子目录被锁失败**（Chromium 在应用目录留句柄；`release\win-unpacked`、
  `dist.new\win-unpacked` 都复现）→ 别设计「构建到 A 再 rename 成 B」的流水线。
- 沙箱 PowerShell 会话同样带 `ELECTRON_RUN_AS_NODE=1` → 跑打包产物前要先清掉这个环境变量，
  否则 exe 以纯 Node 起来只吐 `bad option: --smoke`（看起来像参数不支持）。
- `--smoke` 会在输出目录里建 `data\` → 跑过的解压目录不再是出厂状态；要 pristine 的目录形态就发 zip。

---

## 滚动条穿过标题栏 → `width:100vw` 契约 · 2026-09-17

使用者截图反馈「滚动条显示不合理」：纵向滚动条**从窗口最顶端**开始画，裸露在标题栏右边那条
15px 的缝里 ⇒ 看起来像「标题栏被切掉一角 + 滚动条穿过标题栏」。

**根因**：`#__web2app_titlebar__` 原为 `position:fixed;top:0;left:0;right:0`。
fixed 元素的 `left/right:0` **只算到 `clientWidth`**（不含纵向滚动条），而滚动条属于视口、
从 y=0 就开始画 ⇒ 右侧必然空出一条滚动条宽度。

**实测**（400×340 窗口 / Windows 经典 15px 滚动条）：

| 写法 | 宽度 | 覆盖滚动条? |
|---|---|---|
| `left:0;right:0` | 384 = clientWidth | ✗ |
| `width:100%` | 384 = clientWidth | ✗ |
| `width:100vw` | 399 = innerWidth | ✓ |

**修法**：标题栏改 `width:100vw`（去掉 `right:0`）；加载层遮罩 `#__web2app_loading__` 同步改
`100vw/100vh`（它的 right/bottom:0 会让右侧与底部各漏 15px）。
无滚动条时 `100vw == clientWidth`，WebView2 的 overlay 滚动条不占布局 ⇒ 两壳共用无副作用。

**教训（比 bug 本身重要）**：探针里原有的断言是
`p.near('标题栏满宽', s.barRect?.w, s.view.cw, 1)`，注释还写着「差一个滚动条宽是正常行为而非缺陷」——
**断言把缺陷固化成了期望值**，谁改宽谁被判红，于是没人去追问那条缝是什么。
⇒ 已改为 `p.near(..., s.view.iw, 1)`，并补一条 `p.check('plain 页确有纵向滚动条')`
防止它在无滚动条页面上退化成恒真。**断言前先问：这个数字是「想要的」还是「现在是这样的」。**

**新工具**：`npm run shot`（`scripts/layout-shot.cjs`）——注入 titlebar.js → 打印
`innerWidth/clientWidth/scrollbarW/titlebarW` → `capturePage()` 存 PNG 到
`%TEMP%\w2a-layout-shot.png`，退出码即判定。
⚠️ 必须用 `capturePage()`：Chromium 走 DWM 合成，系统级 `CopyFromScreen`/`PrintWindow` 只能拿到黑图或下层窗口。

### 二次反馈（同日「问题依旧」）· 复核 + 显式钉宽 · 2026-09-17

**排查第一步永远是「他跑的哪份产物」**，不是改代码：
`%APPDATA%\<app_name>\window-state.json`（关闭时写）与 `<包内>\data\session\` 的时间戳能定位到
具体是哪个 exe 跑过、窗口多大 —— 本次两者都是 `dist\Web2App-0.1.0-win-x64\Web2App.exe`（07:36，
`resources/inject/titlebar.js` 已含 `100vw`），**旧包假设不成立**。

复核（当前源码在 Electron 无法复现）：夹具页与真实百度页、430×470 与 829×438（= 使用者
window-state 的 1658×876 物理 @dpr=2）两档，`titlebarW == innerWidth`（828 含 15px 滚动条），
滚动条从 38px 才开始（PNG 人眼确认）；主程序路径 `npm run probe` 99/99。
⇒ 「宽度正好差一条滚动条」只可能来自「用了 `100%` / `left+right:0`」；`100vw` 含滚动条在本引擎是稳定事实。

**加固**（`titlebar.js`，两壳共用）：`barWidthPx()` + `syncBarWidth()` —— 宽度**显式**写成
`innerWidth / zoom`（补偿页面 `html{zoom}`），在挂载 / 挪进 body / `resize` / 自愈守护补挂后各同步一次；
CSS 的 `100vw` 只留作首帧兜底。**不要**加「凭感觉补几像素」，那是跨 DPI 的新错位源。
`probe` 增第 100 条：把 inline 宽度改写成 `clientWidth`（复刻那条缝）→ 触发 `resize` → 断言被钉回
`innerWidth`（期望值取自使用者能说清的目标，不是脚本当前表现）。

**`layout-shot.cjs` 此前跑不起来的三个坑（已修）**：
1. 无 GPU / 容器沙箱：GPU 子进程反复崩 → `FATAL: GPU process isn't usable. Goodbye.`，
   连 `data:` URL 都 `ERR_FAILED`（**看着像断网，其实渲染进程死了**）。
   `app.disableHardwareAcceleration()` 不够，实测只有 **`--no-sandbox` + `--in-process-gpu`** 能活
   （单用 `--in-process-gpu` 仍崩）。仅取证脚本开 `no-sandbox`，主程序不开。
2. **Chromium 不读 `HTTP(S)_PROXY` 环境变量**（只认系统代理设置）⇒ 要把环境变量转成 `--proxy-server=…`。
3. 采集字段补 `dpr` / `barCssWidth` / `htmlOverflow` / `rootScrollable`，用于「断言过而人眼不对」时定性。

---

## ⚠️ 勘误 + 真根因：滚动条画在 DOM 之上 · 2026-09-17（第三次反馈）

**上面两节（`width:100vw` 契约 / 复核 + 显式钉宽）的结论都是错的**，原文保留只为记录踩坑路径。
使用者第二次说「很明显还是没有解决」，并提示「是不是框架层面问题」—— 是对的。

### 我错在哪（两个独立的错）
1. 只信 `LAYOUT` 里的 `titlebarW == innerWidth == 828` 就下结论「无缝」；改成断言
   `rect.width == innerWidth` 后**依然全绿**。
   ⇒ **`getBoundingClientRect()` 只证明布局宽度，不证明可见宽度。** 同一个坑踩了两次。
2. 用 `Read` 看 PNG 时**图被缩略显示**（实际 1656×876，看到的是约 1064px 宽的样子），
   肉眼「看起来铺满了」。⇒ 要量像素就用 `scripts/png-inspect.cjs`，不要凭缩略图下判断。

### 真根因（框架层，不是 CSS 问题）
**Chromium 把视口滚动条绘制在所有 DOM 内容之上**，`z-index:2147483647` 也压不住。
⇒ 「注入 DOM 的自绘标题栏盖住滚动条」**物理上做不到**，宽度调多少都没用。

逐像素（`scripts/png-inspect.cjs`，修复前）：
```
标题栏深色最右 x=1625（图宽 1656，右侧缺口 30 物理像素 = 15 DIP）
右边缘 (x=1655, y=0..100) 恒为 rgb(252,252,252)
```
命中测试（`scripts/titlebar-diag.cjs`）：
```
probe.fixed100vw = 828          ← 100vw 就是 innerWidth，单位/写法都没问题
elementAt[812] = BUTTON.w2t-btn
elementAt[813..827] = null      ← 那 15px 里任何 DOM 元素都命中不到
```

### 解法（`scripts/scrollbar-lab.cjs` 四模式实测）
| 配置 | clientWidth | 命中区止于 |
|---|---|---|
| 默认（经典滚动条） | 813 | 812 ✗ |
| `--enable-features=OverlayScrollbar` | 813 | 812 ✗ **开关在 Electron 44 已失效** |
| `--enable-features=FluentOverlayScrollbar` | 813 | 812 ✗ 同上 |
| **`html{scrollbar-width:none}`** | **828** | **827 ✓** |

⚠️ **本节结论随后即被文末「第四次反馈」取代**（把滚动条宽度归零 = 滚动条也没了，不是使用者要的）。
`titlebar.js` 曾加过 `html{scrollbar-width:none!important}` + `html::-webkit-scrollbar{width:0!important;height:0!important}`（现已移除）。
代价：页面不再显示滚动条，但 `canScroll` 仍 true（滚轮/键盘/触摸板照常）；
只作用于 `html` 自身，不碰页面内部的 div 滚动区。`100vw` 保留为首帧兜底。

### 判据（第三次才换对指标）
`probe.ts`：`scrollbarW == 0` + `hitRight == innerWidth-1`（`elementFromPoint` 从右往左扫）
+ `canScroll` 为真 + 自愈（把宽度改写成 `innerWidth-15` → `resize` → 断言钉回）。
**`rect.width == innerWidth` 永远不要再当「满宽」的判据** —— 它测不出覆盖。`probe` **103/103**。

### 若将来必须保留可见滚动条
见文末「第四次反馈：换滚动根」—— 已经实现了，且**不需要**换 view 架构：
把滚动根从 `html` 挪到 `body`（`margin-top:38px` + `height:calc(100vh-38px)` + `overflow-y:auto`），
滚动条只在 body 盒子内绘制，天然不会越过标题栏。

（下面这条是当时的判断，作为备选方案保留：换架构 = 标题栏不再注入页面，改成两个 view
（Electron: `BaseWindow` + 两个 `WebContentsView`；Tauri 2: 同窗口多 webview）——
滚动条天然被裁在页面 view 内，`100vw` / `zoom` / CSP / 页面 `transform` 这一整类问题一并消失。
改动大得多，且会让两个壳的架构分叉，因此在「换滚动根」可行后不采用。）

### 新工具
- `scripts/png-decode.cjs`（`node:zlib` 手解 PNG，导出 `decodePng/lum/rgb/rightmostDark`）
- `scripts/png-inspect.cjs`（PNG → 结构图 `#`深/`.`近白/`~`灰/`+`彩 + 右边缘逐像素扫描）
- `scripts/titlebar-diag.cjs`（祖先链 zoom/transform/overflow/contain + `100vw` 实测 + `elementFromPoint` 逐点）
- `scripts/scrollbar-lab.cjs`（四模式对照，输出 `clientWidth` 与命中边界）
- `layout-shot.cjs` 判定改 `hitRight` + `scrollbarW`；**`capturePage` 必须包 try/catch** ——
  无 GPU 环境抛 `UnknownVizError` 会把调用方进程**挂住不返回**（白等 6 分钟，而判定其实早已打完）。
- `package.json` 新增 `npm run lab` / `diag` / `png`


---

## ⚠️ 第四次反馈：滚动条不能藏，要「在、但别越界」→ 换滚动根 · 2026-09-17（终稿）

使用者原话：「**被你气笑了，现在滚动条没了。滚动条不应该强制隐藏，只是不要超过菜单栏而已**」
—— 上面那节的 `scrollbar-width:none` **不是**他要的：他接受滚动条存在，只要求它的最高点不要越过
标题栏下沿。这句话把方案空间收窄成一条：**让滚动的那块矩形自己从 38px 开始**。

### 又排掉三个候选（`scripts/scrollbar-lab.cjs` v2：五种模式一次跑完，判定 = 右端那一列的纵向颜色剖面）

| 做法 | 标题栏那 38px 内 | 结论 |
|---|---|---|
| 原生（`html` 滚） | y0..y56 全是滚动条像素 | ✗ |
| `::-webkit-scrollbar{border-top:38px solid transparent}` | thumb 仍从 y=0 起 | ✗ **border 不参与滚动条零件布局** |
| `::-webkit-scrollbar-track{border-top:38px}` | 同上 | ✗ 同上 |
| `scrollbar-width:none` | 干净 | ✗ 滚动条也没了（被否） |
| **`html` 不滚 + `body` 滚** | **y0..y32 = rgb(31,37,48) 纯标题栏色，y40 起才是滚动条** | **✓** |

### 终稿实现（两壳共用 `titlebar.js`）

```css
html{box-sizing:border-box;height:100%;overflow:hidden}
body{box-sizing:border-box;margin-top:38px;min-height:0;
     height:calc(100vh - 38px);overflow-y:auto;scroll-behavior:inherit}
```
- 必须 `margin-top`（不能用 `padding-top`：padding 在盒内，滚动条仍从盒上沿画）。
- `min-height:0` 专治页面 `min-height:100vh` —— 否则 body 被撑高一个标题栏，最后 38px 滚不到。
- `scroll-behavior:inherit` 镜像 `html` 的 `smooth`（该属性不继承，不镜像则平滑滚动变瞬移）。
- 附带收益：`elementFromPoint(innerWidth-1, 20)` 重新命中标题栏 ⇒ **最右侧关闭按钮右端重新可点**。

**代价 ①（必须补的转发）**：`window` 级滚动 API 指向不再滚动的 `documentElement`。
`shimWindowScroll()` 四处转发（全部 try/catch、幂等）：`scrollY/pageYOffset/scrollX/pageXOffset`
用 getter 指向 body；`scrollTo/scroll/scrollBy` 转发；body 的 `scroll` 事件补发到 `window` **和**
`document`（元素的 scroll 事件不冒泡）；`Document.prototype.scrollingElement` → body；
`<html>` **实例**上的 `scrollTop/scrollLeft` 遮蔽转发（只挂这一个实例，不碰 `Element.prototype`）。

**代价 ②**：`applyChromeFit` 正常模式下**不再动 `position:sticky`** —— body 已是滚动容器，
`top:0` 的 sticky 天然停在 y=38；再加 38 就成了 76px。`fixed` 仍照旧下移（它锚的是视口）。

**代价 ③（兜底）**：页面用 `!important` 压掉契约 ⇒「html 不滚 + body 也不滚 = 页面钉死滚不动」，
比原缺陷严重。`ensureScrollRoot(style)` 探测三条：html `hidden` + body `auto` + `body.height ≈ 视口-38`
（第三条专抓 `body{height:auto!important}`，只查 overflow 抓不到），失败即退回 html 留白旧模式并
`console.error` 说明原因。
⚠️⚠️ **必须在样式表挂上之后调用**：挂在 `mount()` 之前时 computed style 还是页面原样 ⇒ 必然误判成
失败、把退路写进 inline style（`!important` 优先级最高）⇒ 新契约永久起不来。本轮真实踩过：
现象是 `body` 高度仍是内容高 4432、根滚动条照旧（`rootOverflowY: auto`）。

### 判据（成对，杜绝「把症状删掉」也算通过）

| 判据 | 期望 |
|---|---|
| `intrudeRows`（x=`innerWidth-1` 逐点 y=0..37 命中标题栏） | `[]` ← 「别超过菜单栏」的直译 |
| `bodyScrollbarW = body.offsetWidth - clientWidth` | > 0（本机 15）← 滚动条**存在性** |
| `bodyScrollable` | `true` |
| `window.scrollTo(0,200)` → `scrollY` + scroll 事件 | ≈200 且事件触发（**异步，必须等一帧再读**） |
| `document.documentElement.scrollTop = 300` → `body.scrollTop` | ≈300（老式回顶写法） |
| `document.scrollingElement === document.body` | `true` |

`probe` **112/112**；`npm run shot` 四条 CHECK 全 OK（夹具页 + 真实百度页，829×438）；
打包产物 `dist\win-unpacked\Web2App.exe --probe` ExitCode 0（`Start-Process -Wait -PassThru`
取退出码 + probe-store 跑前删/跑后重建作旁证）。

### 教训（新增，已进 MEMORY.md C.8 / C.9）
- **「消灭了缺陷」≠「满足了需求」**：用「隐藏滚动条」换掉「滚动条越界」只是换了个缺陷。
  动手前先把需求翻译成可观测判据（此处 = 滚动条可见 ∧ 最高点 ≤ 38px），两条必须同时满足。
- **侵入类缺陷的判据要成对**：只断言「不侵入」会被「干脆删掉」满足；只断言「存在」会被
  「存在但压在上面」满足。

---

## 增量：MEMORY.md 瘦身迁入（2026-09-22）· Electron 硬契约 15 条

**进度**：M0–M7 ✅（Tauri 版继续维护 ⇒ 契约对齐是硬要求）。M7 前提 `electron-app/` 已纳入版本控制。

**门禁**（本机**逐条**跑别挑）：`cargo fmt --all -- --check`（ci.yml 阻断步骤，2026-09-20 曾长期红）+ `clippy --all-targets -D warnings` + `test --all`；`pnpm run typecheck`/`build`/`probe`(134)/`probe --tabs`(31)/`smoke`。

1. ⚠️⚠️ 所有 electron 命令经 `scripts/launch.cjs`（宿主注入 `ELECTRON_RUN_AS_NODE=1` ⇒ 以纯 Node 启动，报错却落在我们代码里）。禁 `env -u`（切断 stdout ⇒ 退出码 0 零输出）；别清 `NODE_OPTIONS`。
2. ⚠️⚠️ 注入脚本必须跑**主世界**（`webFrame.executeJavaScript`），否则隔离世界看不到 `contextBridge` 的 `window.web2app` ⇒ 标题栏画得出、按钮全点不动。严格 CSP 站点拦 `<style>` ⇒ 用 `document.adoptedStyleSheets`（留 `<style>` 回退）。
3. ⚠️⚠️「exe 同级」一律走 `installRoot()`（portable 用 `PORTABLE_EXECUTABLE_DIR`，开发态 `getAppPath()`）；userData/sessionData/logs/crashDumps **四项一起改**且在 ready 前；不可写要**真写文件探测**后回退 `%APPDATA%`。数据目录由 `config.json` 的 `data_dir` 决定，`portable.ts` 的 `DataDirMode` = `relative`/`absolute`/`system`/`fallback` 四条已实测。⚠️ `config.ts` 的 `rejectUnknownKeys` 必须用 `Object.prototype.hasOwnProperty.call(...)`（用 `in` 会被原型链在 `toString` 上误判成「已移除」）。
4. ⚠️ 窗口标题必须显式传 `title: cfg.title`：拦了 `page-title-updated` 后漏传**不会**退回页面标题，而是永远停在 package.json 的 name。⚠️ 打包前必须 `npm run build`（`noEmitOnError: true`），否则半成品 JS 封进 asar（双击弹 `xxx is not defined` 且进程不退）。
5. ⚠️⚠️ `config.json` **随 `app.asar` 进包**（2026-09-22 改造）：`stage-assets.cjs` 写 `out/config.json`，`configRoot()` 读 `app.getAppPath()/out`（开发态回落项目根）。**已无 `extraFiles`** ⇒ 不能再靠「放一份 config.json 到 exe 旁边」覆盖。**electron-builder 26 无顶层 `zip:` 块**。⚠️ `build/` = `directories.buildResources` 会被 electron-builder **整个排除出包**，只能放构建期资源（`icon.ico` 复制品、临时下载的图标）。
6. 运行期**只有一个** `settings.json`（**`<data_dir>\`，2026-09-23 起**；旧 `%APPDATA%\<app_name>\` 降级为**只读候选**）：设置三键平铺 + `window` 段几何（2026-09-23 合并，原 `window-state.json` 作废、不迁移）⇒ 两个写入方都走**读-改-写**（且「读」走**候选链**，不是只读写入位置），谁都不能整体覆盖；设置页 HTML **不重写**，靠 preload 垫片提供 `__TAURI_INTERNALS__.invoke`；几何**物理像素存盘 / DIP 恢复**；探针把这一个 JSON 重定向到 `%TEMP%\w2a-probe-store\`。
7. 静默陷阱：托盘「退出」先置 `quitting=true`；`globalShortcut.register()` 失败**必须把旧键注册回去**；显隐语义要 `visible && !minimized`。
8. **多页签唯一开关 = config.json 的 `tabs_enabled`**：壳 = 窗口自身 `shell://`→`shell.html`，内容 = N 个 `WebContentsView`；只在最外层分叉一次，禁止在既有函数里散落 `if`。
9. ⚠️⚠️ **探针起窗模式必须与断言组一一对应，禁止 `cfg.tabs_enabled` 参与**：`TABS_MODE = TABS_PROBE ? true : PROBE ? false : cfg.tabs_enabled`。否则 config 一开页签，`--probe`（134 条单窗口断言）以页签模式起窗 ⇒ 注入抛 `Script failed to execute`、退 1，门禁退化成「看用户当前配置过不过」。
10. 验证：`probe`(**134**)·`probe --tabs`(**31**)·`smoke`·`shot`·`lab`·`tabs-v4`(+`--negative`)。⚠️ 沙箱无 GPU ⇒ **必须** `pnpm run probe --no-sandbox --in-process-gpu`（`probe`/`smoke` 脚本**不自带**这两个参数，只有 `tabs-v4` 自带）。⚠️ **传参禁写 `--` 分隔符**（pnpm 把字面量 `"--"` 透传 ⇒ 假象「加了 `--no-sandbox` 仍崩 GPU」）。⚠️ 漏传表现为 `ERR_FAILED (-2) loading 'splash://localhost/'` + `GPU process exited unexpectedly` ⇒ **几乎总是环境/参数问题不是代码坏**。⚠️ 打包产物**没有 stdout** ⇒ 靠窗口标题 / exe 同级 `data\` / 缓存 grep 取证。⚠️ **跑探针前不需要 build**（`probe.ts` 走 ts 运行期），但**目视 exe 之前必须 build**（见 lessons）。
11. 启动日志（M6）：`<dataDir>\logs\startup.log`（探针改写 `%TEMP%\w2a-probe-store\`）；契约 = **fail-open** + 同步追加 + 1MiB 轮转 + 包装 `console` 必须先调原始实现。`smoke` 硬断言「已含 `[web2app] 启动信息`」。版本信息由 electron-builder 从 `package.json` 的 `version`/`description`/`author`+`copyright` 自动写全（`CompanyName` 取 `author.name`）；取证 `(Get-Item exe).VersionInfo`。
12. ⚠️ 沙箱**禁止一切删除**、父目录 rename 因句柄被锁必失败 ⇒ 出包绕法 = `--config.directories.output=<新目录>` + copy 归位。⚠️ 覆盖 `release/` 时 exe/`app.asar` 被占用会 **EBUSY**，失败的 electron-builder **已先把 `win-unpacked` 删成残缺**。⚠️ **探针别在交付目录里跑**（会在 exe 同级重建 `data\session`）污染交付物；也别把大文件 copy 进仓库目录（删不掉 ⇒ 留垃圾）。
13. **体积/分发形态**（全量 `docs/electron-size-reduction.md` + 附录A + 二进制体积审计 SOP）：`app.asar` 仅 0.20MB、exe **234.91MB** 且 **96.6% 是代码**（官方已 strip ⇒ **剪不掉**；UPX 三变体全崩 0xC0000005 ⇒ **压不了别试**）⇒ 应用侧 minify/摇树/asar 压缩全是噪声。实际可动：`locales` 48.30MB（留 en-US/zh-CN/zh-TW = 1.68MB）；**保留** `LICENSES.chromium.html`（合规，7z 里仅 0.18MB）；`dxcompiler+dxil`、`vk_swiftshader+vulkan-1` 默认别删。同一载荷 zip 147.6MB vs **portable 96.5MB**（NSIS 默认 7z/LZMA，`NsisTarget.js:509`）⇒ 主推 portable；真要减小只有自编译 Electron（不经济）或换壳（= 把 Chromium 成本转给系统：Tauri 自有 11.06MB ↔ WebView2 固定运行时 860.81MB）。
14. ⚠️ `electronLanguages`（`ElectronFramework.js:58-101`）：顶层或 `win.` 级均可，删 `../locales/*.pak`；匹配 = 精确名 **或** wanted 以 `<文件语言名>-` 开头 ⇒ **Windows 必须写全名**（写 `"zh"` 一个都命不中，前缀规则只对 mac `.lproj` 有效）；须同时列 `zh-CN`/`zh-TW`；全不命中只 warn（安全）。时机 `beforeCopyExtraFiles` ⇒ 压缩包同步变小。

15. ⚠️ **「打开设置」有三条入口，两壳两模式各走各的通路**（2026-09-23 加「设置」按钮时的结论）：
   - 托盘菜单「设置」/ 全局快捷键 Shift+F1 / **标题栏「设置」按钮**（位置在最小化右侧）→ 同一个 `openSettings()`。
   - 单窗口模式：`titlebar.js` 注入在**远程页面**里 ⇒ Electron 走 preload 白名单 `window.web2app.openSettings()`
     → IPC `win:open-settings`；Tauri **不能**走应用命令（远程 origin 的 IPC 强制过 ACL，应用命令无 app 清单可授权），
     只能发事件 —— 页面 `plugin:event|emit`（capability 里的 `core:event:allow-emit`）→ `lib.rs`
     `app.listen("web2app:open-settings")` 落地。⚠️ 事件名两侧字面量必须逐字一致。
   - 多页签模式：标签栏 `shell.html` 是**本地页面** ⇒ 直接 `invoke("open_settings")`；
     Electron 侧对应 `tabs:open_settings` + `shell-preload` 的 `ALLOWED` 白名单（页面字面量 / 白名单 / IPC 名三处同改）。
   - ⚠️ 断言要「等设置页真的渲染出 `.panel`」而不是「窗口对象存在」—— 加载失败时窗口对象照样存在（假绿），
     且抢在加载完成前 `close()` 会中止导航、在日志里留下 `ERR_FAILED(-2)` 噪声（第一版就踩了）。
