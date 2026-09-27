# electron-app

web2app 的 **Windows 绿色版**壳（Electron），与 `../webview-app`（Rust + Tauri 2）行为兼容。
Tauri 版继续维护（同时负责 Android），两者共用同一份 `config.json` 契约与同一套注入脚本。

> 完整方案见 `docs/electron-plan.md`（⚠️ 那是**决策与架构记录**，里程碑状态与命令**以本 README 为准**）；
> 前期可行性论证见 `docs/electron-analysis.md`；打包体积与分发形态见 `docs/electron-size-reduction.md`；
> 全仓库规则与目录地图见根目录 `README.md`。

## 现状与未实现能力（先看这张表再动手）

| 里程碑 | 状态 | 说明 |
|---|---|---|
| M0 工程脚手架 / M1 窗口 + 注入 + 探针 | ✅ | 无边框窗口 + 注入脚本 + 探针骨架（探针 39 条起家，现已 138 条） |
| M2 sidecar（本地反向代理 + 加密解析） | ✅ | 复用 Rust `proxy.rs` / `resolve.rs`，零重写 |
| M3 托盘 / 全局快捷键 / 设置窗 / 窗口几何 | ✅ | 与 Tauri 版文件级互通 |
| M6 出包（dir / zip / portable） | ✅ | `pack` / `dist`。三形态实测出包；**启动日志**已落地 `data\logs\startup.log`（**仅 `debug` 打开时才有**，见「日志目录」一节）；**版本信息**（`VERSIONINFO`）由 electron-builder 从 `package.json` 的 `version` / `description` / `author` 与 `electron-builder.yml` 的 `copyright` 自动写入，实测已落地（取证命令见「日志目录」一节）；首次运行的 SmartScreen 提示见「首次运行与 SmartScreen」 |
| **M4 页面行为（下载接管 / 右键菜单 / F12）** | ✅ **已完成** | 实现落在 `src/main/page-behavior.ts`：①**下载接管** `will-download` → `pause` → 系统「另存为」（`dialog.showSaveDialog`，默认落点=系统下载目录、按后缀收敛过滤器、开覆盖确认）→ `setSavePath` + `resume`；用户取消走 `cancel()`，**绝不**静默存到别处（对标 `save_dialog.rs`）；②**右键菜单**自建（Electron 不暴露 Chromium 原生菜单），取舍与 Tauri 一致——不给打印 / 另存为 / 更多工具，**带「设置」项**（与 Shift+F1 / 托盘同一入口）；③**F12** `before-input-event` → `openDevTools({ mode: 'detach' })`，开关为 `devtoolsEnabled()` = `!isPackaged \|\| debug`（对齐 Tauri 的 `DEVTOOLS_ENABLED`；打包版可用配置 `debug: true` 打开，这是本壳与 Tauri 的**有意**差异——绿色版没有 debug 构建分发）。⚠️ 两个坑：①`item.pause()` 不可省，否则 `showSaveDialog` 异步返回后 Electron 会再弹**它自己**的保存对话框（两个抢注意力）；②多页签模式只给内容视图挂右键菜单（挂到壳窗口会在标签栏上弹出「返回 / 前进」这类错位项） |
| **M5 多页签** | ✅ **已完成（T0–T4）** | `tabs_enabled=true` → 多页签模式（`BrowserWindow` 当壳 + 子 `WebContentsView`，与 Tauri 侧同构，窗口类不变）。**选型依据与决策记录**见 `docs/electron-tabs-design.md`；**真机验收清单与实测结论**见 `docs/electron-tabs-test-steps.md`。收口状态：探针 **33/33**（2026-09-27 补 P7c 关后台页签壳 DOM 即时刷新 2 条，旧实现在非活动分支漏 `pushToShell` 会留幽灵页签）、单窗口 **138/138**（2026-09-27 第 16 节新增子框架桥梁隔离 4 条）零回归；2026-09-18 真机目视 **V1–V4 全部关闭**（多页签 8/8 正常、跨 DPI 正常；仅「代理 / 域名解析」留待后续单独测试，属 sidecar 链路）。⚠️ 两条别踩：①**探针的起窗模式只由命令行决定**，`tabs_enabled` 不参与——否则页签配置下跑 `--probe`，单窗口那批断言会往壳页面注入脚本而必失败，报错只是 `Script failed to execute`，完全不指向根因；②新页签**不得**调 `bootstrap()`（它会掐掉在服页签的代理并把壳页面导航走），走 `resolveInitialTarget()` 复用 target 缓存 |
| **M7 CI 流水线** | ✅ **已落库** | `.github/workflows/electron-ci.yml`（push / PR 到主分支自动触发，path 过滤 `electron-app/**` + 共享真源 `webview-app/src/**`）：①`lint`（Ubuntu）typecheck + 单测；②`e2e`（Windows）单窗口探针 + 多页签探针 + 启动冒烟；③`package`（非 PR）出 zip / portable 并上传产物。⚠️ 三个 CI 特有的坑已写进文件头注释：`.npmrc` 的国内镜像必须用 `--registry` 覆盖掉、`config.json` 本就不在仓库里（探针自起 127.0.0.1 夹具站点，无配置也能跑完）、pnpm 固定 v12 |

> 除上表三项外，后续「新增/改动会影响哪些验证」见文末「验证清单」——
> 这是本壳唯一能防「改了一个壳、把另一个壳带坏」的手段。

## 命令

```bash
# ⚠️ 首次安装前先设两个镜像环境变量，否则会卡在下载（见下节 pnpm）
pnpm install         # 装依赖；装完必须确认 node_modules/electron/dist/electron.exe 存在

pnpm start           # 编译 + 启动（开发态）
pnpm run typecheck   # 只做类型检查（不能代替 build，见「打包」表）
pnpm run build       # 只编译到 out/
pnpm run unit        # 纯函数单测（node:test，毫秒级）：改 config.ts 等零 Electron 依赖模块必跑

pnpm run sidecar     # 编出 ../webview-app/target/release/w2a-proxy.exe（本地代理 + 加密解析）
pnpm run stage       # 打包前置：铺 build/icon.ico 与 out/config.json（pack/dist 会自动调用）
pnpm run smoke       # 冒烟自检：打印生效路径/配置 + 断言启动页模板/设置页/托盘图标/窗口标题/启动日志落盘（内部强制 debug=true），退出码即结论
pnpm run probe       # 兼容契约探针：138 条断言，退出码即结论 —— 动过注入脚本/代理/设置链路必跑
pnpm run shot        # 布局取证：注入 titlebar.js 后打印四条判据 + 截一张 PNG（改标题栏/滚动条/几何后看它）

pnpm run pack        # 只出解压目录 release/win-unpacked/
pnpm run dist        # 出 zip + 单文件 portable exe（→ release/）；要解压目录形态用 pack
```

**一次性排查工具**（怀疑「断言过、人眼不对」时才用，与产品使用无关）：
`pnpm run lab` 滚动条多模式对照 · `pnpm run diag` 标题栏遮挡诊断 · `pnpm run png` 截图逐像素取证 ·
`pnpm run tabs-v4` / `tabs-v4-negative` 多页签子视图绘制顺序（`desktopCapturer` 系统级抓屏 + 像素断言）。
前三个都零依赖，专治沙箱里没有图片库、`Add-Type` 被安全策略拦的情况；用法见文末「自绘标题栏与页面滚动条」。


### 包管理器：pnpm（2026-09-17 迁移）

**装依赖前必须设两个环境变量**（这是 pnpm 与 npm 唯一的操作差异，不设会卡住）：

```powershell
$env:ELECTRON_MIRROR="https://npmmirror.com/mirrors/electron/"
$env:ELECTRON_BUILDER_BINARIES_MIRROR="https://npmmirror.com/mirrors/electron-builder-binaries/"
pnpm install
```

⚠️ **这两行必须走环境变量，`.npmrc` 里那两行同名键对 pnpm 无效** ——
pnpm 不会把 `.npmrc` 的自定义键透传给依赖的 install 脚本（脚本里能看到的 `npm_config_*` 只有
`npm_config_user_agent`），而 `electron/install.js` 与 electron-builder 的下载器都只认环境变量。
不设的后果是 install.js 去连 GitHub Releases：国内表现为**卡住不返回**（不是报错，很难看出来）。

三条与 pnpm 有关的坑，都固化在 `pnpm-workspace.yaml` 里（该文件对 npm 无影响）：

| 配置 | 为什么 |
|---|---|
| `allowBuilds: electron: true` | pnpm 10 起默认不跑依赖的 postinstall，而 electron 靠它下载解压 ~158MB 二进制；pnpm 12 且把"被忽略"从警告升级为**报错** `ERR_PNPM_IGNORED_BUILDS`，直接中断安装。不放行的症状是"装完了但 `dist/electron.exe` 不存在"。 |
| `electron-winstaller: false` | electron-builder 的传递依赖，只服务 Squirrel.Windows 目标（本项目不出）。⛔ 必须**显式**拒绝：既没允许也没拒绝同样会触发上面的报错。 |
| `nodeLinker: hoisted` | 产出扁平 `node_modules`（接近 npm），而不是默认的符号链接虚拟 store。这是 electron-builder 官方 README 对 pnpm 的既有指引；且本机实测默认 isolated 布局会在"提升符号链接"那步失败（`Failed to create symlink … os error 2`，两次分别停在 `color-name` / `source-map`，包名会变 ⇒ 像竞态）。代价是失去严格隔离，但省磁盘（store 硬链接）保留。 |

⚠️ **验证装好了没有，只看一个东西**：`node_modules/electron/dist/electron.exe` 是否存在。
`pnpm install` 返回 0 不代表二进制装上了。补救：设好 `ELECTRON_MIRROR` 后
`node node_modules/electron/install.js`（实测 3 秒完成）。

⚠️ **给脚本传参数时不要写 `--` 分隔符**（与 npm 不同）：pnpm 会把字面量 `"--"` 一起传给脚本。
实测 `pnpm run build -- --version` 打出的是 `$ tsc "--" "--version"`，tsc 把 `--` 当成「选项结束」标记，
版本号打不出来 —— 这个坑在本机造成过两次「明明加了 `--no-sandbox` 探针却仍崩在 GPU」的假象。
正确写法 `pnpm run probe --no-sandbox --in-process-gpu`；校验透传是否正常用
`pnpm run build --version`（应打印 `Version 7.0.2` 之类）。

`package-lock.json` 已改名为 `package-lock.json.npm-bak`（两个 lockfile 并存会让
electron-builder 的包管理器检测落空、回退到环境变量判断）。要退回 npm 就改回来。

⚠️ `pack` / `dist` / `probe` 都会先跑 `pnpm run sidecar`：sidecar 是**必需组件**，
缺了它「本地反向代理」与「域名解析」两种配置都起不来。

⚠️ `start` / `smoke` / `probe` 三个命令都经 `scripts/launch.cjs` 拉起，**不要直接 `npx electron .`** —— 原因见下节。
（`shot` / `lab` / `diag` 各自 `spawn` Electron，脚本内自行清理 `ELECTRON_RUN_AS_NODE`。）

### 打包（`pack` / `dist`）

| 现象 | 原因 | 处理 |
|---|---|---|
| `Invalid configuration object … unknown property 'zip'` | electron-builder **26 起移除了顶层 `zip:` 配置块**（`TargetConfiguration` 只认 `target`/`arch`） | 产物名改用顶层 `artifactName` 模板；已在 `electron-builder.yml` 注明 |
| 双击 exe 弹「A JavaScript error occurred in the main process: xxx is not defined」 | tsc **默认「有错也照样输出」**，`out/` 里留下改了一半的 JS，之后只跑 `typecheck` 没重跑 `build`，打包就把坏产物封进 asar | `tsconfig` 已开 `noEmitOnError: true`（编译失败时 `out/` 保持原样），`pack`/`dist` 里也是先 `pnpm run build` 再打包 |
| 目录里已有 `release/` 时构建报删除失败 | 环境可能拦截删除（如 agent 沙箱的 safe-delete 策略） | 换输出目录：`npx electron-builder --win -c.directories.output=<新目录>` |
| 单文件版双击后是空白官网/`example.com` | 包内 **`app.asar` 里没有 `out/config.json`**（打包前漏了 `pnpm run stage` / 漏了 `pnpm run build`） | 配置已随 asar 进包，**不再**支持放 exe 旁边覆盖；重跑 `build` + `stage` 再打包。CI 侧由 `scripts/check-asar-config.cjs` 硬断言兜底 |
| **改了注入脚本、重新打包后「现象依旧」** | 双击的是**旧产物**：`release/` 下 zip / portable / 解压目录多份并存，旧文件不会被自动清理 | 见下节「先确认跑的是哪一版包」——先核对 sha256 与运行痕迹 mtime，再动代码 |

#### 手动打包（推荐为每个应用出一份包）

本地 `pnpm run pack` / `pnpm run dist` 出的是**当前 `config.json`**的包，适合开发调试。
要为某个具体应用出一份正式绿色版，走 GitHub Actions 的
**`自定义打包（Electron 绿色版）`**（`.github/workflows/electron-build-custom.yml`）：

1. Actions → 该 workflow → **Run workflow**；
2. 粘贴一份配置 JSON（或填一个返回 JSON 的 URL）—— 与 Tauri 版**同一份契约、同一个解析器**；
3. 勾选要打的形态：`electron_zip`（解压即用）/ `electron_portable`（单文件 exe）。

它比本地打包多做了三件事：用共享解析器**规范化**配置（真 bool / 真数组）并写进 `out/config.json`；
在 `RESOLVE_SECRET` 存在的环境下**编译 sidecar**（否则 `resolve_secret` 会静默失效）；
出包后断言 `config.json` 确实在 `app.asar` 内。

> ⚠️ **顺序关键**：解析配置必须排在编译 sidecar **之前**。`resolve-core` 的 Secret 是
> `option_env!("RESOLVE_SECRET")` 的编译期常量，而 `w2a-proxy.exe` 是单独编译的 ——
> 顺序错了，sidecar 内置的是公开默认 Secret，域名解析会一律失败且不报错。

### ⚠️ 先确认「跑的是哪一版包」（30 秒，省一轮返工）

「源码已修好」与「现象依旧」可以同时为真 —— 用户双击的常常是几天前那份产物（`release/` 里多份
产物并存，旧文件不会被自动清）。2026-09-17 实测过一次：探针全绿，而包里的 `titlebar.js`
还停在修复前。

```bat
:: 1) 最近跑的到底是哪份包：看运行痕迹 mtime（早于修复时刻 ⇒ 就是旧包）
dir /tc release\win-unpacked\data
dir /tc release\win-unpacked\data\settings.json

:: 2) 包内注入脚本对不对版：2026-09-26 起注入资源随 app.asar 分发
::    （resources\app.asar\out\inject\），certutil 不能直接读 asar 内文件，
::    先解包再比 sha256（或直接看下一条「成套换」）：
npx asar extract release\win-unpacked\resources\app.asar %TEMP%\w2a-asar
certutil -hashfile %TEMP%\w2a-asar\out\inject\titlebar.js SHA256
certutil -hashfile ..\webview-app\src\titlebar.js                   SHA256
```

对齐办法（**覆盖式**，不用删除）：把**新出的整份包**覆盖进用户实际在跑的那份，再重出
zip / portable 覆盖旧产物。
⚠️ **必须成套换**：`Web2App.exe` + `resources\app.asar` **两件**一起覆盖（`data\` 保留）。
2026-09-26 起注入脚本（`out/inject/*.js`、`settings.html`）与 `out/config.json` **都在
`app.asar` 内**（不再有 asar 外的 `resources\inject\`），所以改注入脚本 / 改配置都必须
**重新打包**，不存在「只换注入脚本」的路径；拿旧包跑新探针（`--probe`）会因旧断言与新行为
冲突而退出码 1，看起来像「修复把东西改坏了」，其实是包没换。

## ⚠️ 为什么必须经 `scripts/launch.cjs`

部分宿主环境（桌面型 IDE、agent 沙箱等）会给所有子进程注入 `ELECTRON_RUN_AS_NODE=1`
（配套还注入 `NODE_OPTIONS=--require <它们的语言服务 shim>`）。这个变量一旦泄漏进 Electron，
Electron 会**以纯 Node 身份启动**：

- `require('electron')` 返回的是**可执行文件路径字符串**，不是 API 对象；
- 于是业务代码第一句 `app.disableHardwareAcceleration()` 就抛
  `TypeError: Cannot read properties of undefined`，错误位置在**我们自己的代码里**，
  看起来完全像是本项目的 bug；
- `electron.exe --version` 会打印 Node 版本（`v24.x`）而不是 `v44.x` —— 这是判别「是否掉进
  Node 模式」最快的一招。

更麻烦的是它**时好时坏**（不同宿主 / 不同调用方式注入与否不一致），极易被误判为随机故障。

`scripts/launch.cjs` 在 spawn 前 `delete env.ELECTRON_RUN_AS_NODE`，其余环境原样透传。

> 排查时踩过的两个反向坑，别再走回头路：
> 1. **别用 `env -u ELECTRON_RUN_AS_NODE ...`**：`env(1)` 会重建进程环境，把宿主用来捕获
>    stdout 的管道一起切断，表现为「退出码 0 但一个字都没有」——于是你会去怀疑参数、怀疑代码，
>    而真正的问题是**看不见输出**。用 bash 内置 `unset`，或像 `launch.cjs` 那样在 Node 里处理。
> 2. **`NODE_OPTIONS` 不要一起清**：那个 shim 正是宿主捕获输出的通道，清了同样变哑巴。

另：Windows 上 `electron.exe` 是 GUI 子系统程序，**stdout 需要重定向才稳定可见**
（`... > log.txt 2>&1`）。直接跑可能什么都刷不出来，别误判成没执行。

## 两种绿色版形态（同一份代码，自动区分）

| 形态 | 产物 | `installRoot()` 依据 | 特点 |
|---|---|---|---|
| 解压目录版 | `Web2App-<ver>-win-x64.zip` | `dirname(app.getPath('exe'))` | 启动快，体积最小（压缩后 ~110–150MB） |
| 单文件自解压版 | `Web2App-<ver>-win-x64-portable.exe` | `PORTABLE_EXECUTABLE_DIR`（electron-builder 注入） | 一个文件带走；⚠️ 启动时解压运行时到 `%TEMP%`，冷启动明显变慢 |

⚠️ 单文件模式下 **`app.getPath('exe')` 指向 `%TEMP%` 里的解压副本**，不是用户放置 exe 的位置。
所有「exe 同级」的定位都必须走 `installRoot()`，不要直接写 `app.getPath('exe')`。

## 数据目录

`config.json` 的 `data_dir`（字符串，与 Tauri 版**同名同语义**）：

| `data_dir` 取值 | 数据位置 |
|---|---|
| `"data"`（默认） | **exe 同级 `data\`**（相对路径基准 = exe 所在目录，非 cwd）— 整个目录可搬走，不写系统盘、不写注册表 |
| `""`（空字符串） | `%APPDATA%\<app_name>`（如 `app_name=Baidu` 即 `%APPDATA%\Baidu`）— 跟随系统，不受程序位置影响 |
| 其它相对路径（如 `"profile"`） | exe 同级 `\profile\` |
| 绝对路径（如 `D:\w2a`） | 原样使用 |

⚠️ **空字符串是有意义的值**（= 系统默认目录），不是「未设置」：读取时不能写成
`data_dir || 'data'`，否则 `""` 会被 `'data'` 吃掉、静默退回 exe 同级。

目录不可写时（写保护的 U 盘、`Program Files`、只读网络盘）会**自动回退**到系统默认目录并打日志，
保证「能启动」优先于「够绿色」。四个路径一起重定向：`userData` / `sessionData` / `logs` / `crashDumps`
（只改 `userData` 会留下 Cookies 和日志在系统盘）。

> 与 Tauri 版的差异：`data_dir=""` 时 Tauri 写 `%LOCALAPPDATA%\<app_name>`，Electron 写 `%APPDATA%\<app_name>`。
> 这是有意的——Electron 版定位为真绿色版（默认 `"data"` 根本不落系统盘）。部署文档需明示这一点。

## 日志目录（`data\logs\startup.log`，**`debug` 打开时才有**）

⚠️ **Windows 上 `electron.exe` 是 GUI 子系统程序**：用户双击运行时 `console.log` 没有任何去处
（没有控制台可挂）；portable 单文件形态更彻底——自解压包装器会吞掉内层进程的 stdout。
于是「双击没反应 / 打开的是 example.com / 标题不对」这类反馈在用户侧**零证据**，只能远程猜。

**但「默认就落盘」同样不是期望行为**：它会在用户的交付目录里凭空多出 `data\logs\`，
而绝大多数使用者根本不需要它。所以 2026-09-22 起日志改成**配置开关**：

| 开关 | 效果 |
|---|---|
| `"debug": false`（**默认**） | 不建 `logs\`、不写任何文件，**控制台三路输出也一并静默** |
| `"debug": true` | 写 `<数据目录>\logs\startup.log`，同时照常打印到 stdout / stderr |

**运行时逃生口**（不改配置、不重新打包，用于现场排障）：命令行加 `--debug`，或设环境变量
`WEB2APP_DEBUG=1`。两者任一生效即等同 `debug=true`。

> 「完全静默」是有意选的：GUI 程序本来就没有控制台，`debug=false` 时把 `console.log` / `warn` /
> `error` 换成空函数，避免这些输出被 portable 包装器或父进程捕获后又落到**别处**（CI 日志、
> 用户的重定向文件……）。**关掉就要关干净。**
> **唯一例外**：探针（`--probe`）与冒烟（`--smoke`）**内部强制打开** debug —— 它们的结论全靠
> stdout，绝不能因为「默认关日志」而自检失声。

开日志时，启动后立刻把 console 的三路输出同时落一份盘（`src/main/startup-log.ts`）：

| 形态 | 日志位置 |
|---|---|
| 解压目录版（`data_dir="data"`，默认） | 「用户放置 exe 的目录」`\data\logs\startup.log` |
| `data_dir=""`（系统默认目录） | `%APPDATA%\<app_name>\logs\startup.log` |
| 探针（`--probe`） | `%TEMP%\w2a-probe-store\startup.log` ← **不写真实目录**（与 `settings.json` 同一套隔离） |

⚠️ 探针那条不是洁癖：在打包产物上跑一次 `--probe` 就会在用户 exe 旁边留下 `data\logs\`，
污染交付物（`data\session` 已经踩过一次，见「打包」一节）。

契约（改这个模块时别破坏）：

* **fail-open**：目录建不出来 / 文件不可写 ⇒ 只在控制台告警**一次**并停止后续写入尝试，
  应用照常启动。为了记日志让应用起不来是本末倒置；但**必须说清楚**，不许静默。
* **不碰 stdout**：包装 `console` 时先调用原始实现，宿主捕获到的输出一字不变
  —— 探针与冒烟的结论全靠 stdout，绝不能因为加日志而变哑。
* **同步追加**（`appendFileSync`）：崩溃或被强杀时，最后几行已经落盘。
* **1 MiB 轮转**成 `startup.prev.log`（只留一代），且**轮转失败不影响继续写**——
  轮转与写入是两个独立失败域，不能互相拖垮。
* 行格式 `<ISO 时间戳> <原文>`；启动块以 `===== web2app 启动 =====` 分隔，便于从长日志里切段。

取证（拿到用户的一段日志就能定位，不必再问「你机器上是什么情况」）：

```powershell
Get-Content .\data\logs\startup.log -Tail 40
```

`pnpm run smoke` 有一条硬断言：日志文件必须**已经包含 `[web2app] 启动信息`**，不满足即退出码非 0。
只判「文件存在」是不够的——一个只写了表头的空文件同样会骗过断言。
（冒烟内部强制 `debug=true`，所以这条断言与「默认关闭」不冲突；若 `fileLog` 拿到 `null`，
断言会直接报「未启用（冒烟模式本应强制打开 debug，出现即 bug）」——把「开关接错」与
「写盘失败」两类原因分开。）

> 负向对照（2026-09-18 实测）：把日志目录的父级设成文件 ⇒ `ok=false / ENOTDIR`；
> 把 `startup.log` 建成目录 ⇒ `ok=false / EISDIR`。两次调用**都不抛异常**，
> 只打一条 `已跳过文件日志（不影响使用）` 的告警。即「日志坏了 ≠ 应用坏了」。

同处取版本信息（M6 的另一半，实测已由 electron-builder 自动写入）：

```powershell
(Get-Item .\release\win-unpacked\Web2App.exe).VersionInfo |
  Format-List FileVersion,ProductVersion,ProductName,CompanyName,FileDescription,LegalCopyright
```

> ⚠️ `CompanyName` 来自 `package.json` 的 `author`。写成 `"author": "Web2App"`（字符串）也**能**生效
> —— electron-builder 内部会先经 `normalizePackageData` 把字符串归一化成 `{name, …}`；
> 但别把它误解成「随便写都行」：`appInfo.companyName` 取的是 `author.name`，
> 一旦换成 `{ email: … }` 这种没有 `name` 的对象，公司名就会**静默留空**。改动后请跑一次上面的命令。

## 首次运行与 SmartScreen（产物未签名）

产物**没有代码签名**。从浏览器/IM 拿到的 zip 与 exe 会被标记「来自 Internet」，
首次双击时 Windows 会弹**「Windows 已保护你的电脑」**（蓝色 SmartScreen 提示），
必须点「更多信息」→「仍要运行」才能启动。

* 这是**预期行为**，既不是打包失败也不是病毒：`signing/` 下那份证书只服务 Tauri 侧。
* 想根治只有两条路——买代码签名证书并配 `win.certificateFile`，或用组织内的白名单 / 域策略放行。
  ⛔ **不要**教用户关掉 SmartScreen（那是全局降安全，代价远大于收益）。
* 交付话术建议照抄上一段，并附上日志的位置（⚠️ `data\logs\startup.log` **只在 `debug` 打开时才有**；
  默认关闭时让用户临时用 `--debug` 复现一次再收日志）：
  「点了没反应」十有八九是被 SmartScreen 拦在门外，而不是程序本身有问题。

## 配置文件

**配置随包进 `app.asar`，不放 exe 同级。** 启动时读 `app.getAppPath()/out/config.json`：

| 形态 | 读取位置 |
|---|---|
| 打包后（绿色版） | `resources\app.asar` 内的 `out/config.json` |
| 开发态 | 项目根 `electron-app/config.json`（`configRoot()` 的回退分支） |

> ⚠️ **为什么不再放 exe 同级**：早期用 `extraFiles` 把配置放在 exe 旁边，结果**任何拿到包的人
> 都能用记事本改掉地址**（改成自己的站再转发）—— 配置就失去了意义。现在改动必须重新打包。
> 代价是**不能**再用「放一份 config.json 到 exe 旁边」来覆盖，改配置 = 重跑 `build` + `stage` + 打包。

> ⚠️ **护栏**：`scripts/check-asar-config.cjs` 断言 `out/config.json` 确实在 `app.asar` 内。
> 这条防的是**静默失败** —— 配置没进包时 electron-builder 照样出包、双击照样打开，
> 只是静默退回内置默认地址（`example.com`），构建日志全绿。

字段与 `webview-app` 的配置契约**同名同义**，唯一实现见
[`.github/scripts/parse-packaging-config.py`](../.github/scripts/parse-packaging-config.py)，
完整字段表与多端行为对照见 [`docs/packaging-config.md`](../docs/packaging-config.md)。

`config.example.json` 按契约列出本壳可用的 **14 个字段**（省略仅 Android 使用的 `android_*`），
复制成 `config.json` 再改即可用。

**校验是严格的**（与解析器逐条对齐）：

- **布尔只接受真 `bool`**（`true` / `false`，不带引号）。字符串形式一律报错并给出改法 ——
  曾经这里宽容地接受 `"1"` / `"yes"` / `"on"`，而 Tauri 侧规则不同，于是同一份配置里写 `"on"`
  两个壳判定**相反**。
- **未知字段直接失败**（`android_*` 除外，它们属同一份契约、只是本壳不实现，会提示后忽略）。
- **`app_url` 接受 `string` 或 `string[]`**（真 JSON 类型），数组模式必须带 `public_key`。
- **已移除的字段出现即报错**：`window_title` → 改用 `title`；`data_dir_default`（布尔）→ 改用 `data_dir`（字符串，`false` ⇒ `"data"` / `true` ⇒ `""`）。
- 文件存在但 JSON 非法时**直接抛错**（不静默回落）—— 否则「改坏一个字符」的表现是「配置像没生效」，最难查。

`config.json` 与 `data/` 都在 `.gitignore` 里 —— 它们是运行期/本地产物，不是源码。
打包脚本 `scripts/stage-assets.cjs` 把项目根的 `config.json`（缺失则生成一份全字段默认模板）
复制到 **`out/config.json`**（随 `files: out/**/*` 进 asar），并在复制前做一次浅层契约校验
（`window_title`、字符串布尔会在这里就失败，而不是等用户双击）。

### `app_name` 与 `title`

| 字段 | 作用 | 本壳取值 |
|---|---|---|
| `app_name` | 安装名 / 数据目录名（Android 侧为应用名） | `data_dir=""` 时系统默认目录的末段（`%APPDATA%\<app_name>\`）；`settings.json` 随数据目录走，不再固定在这里 |
| `title` | **显示名**：窗口标题、托盘提示 | 缺省回落 `app_name` |

⚠️ 窗口标题在创建 `BrowserWindow` 时就固定为 `cfg.title`，并拦截 `page-title-updated`
（对齐 Tauri 版：不跟随 `document.title`）。**漏传 `title` 不会退回成「显示页面标题」，
而是永远停在 Electron 默认值**（`package.json` 的 `name`）—— 见 `smoke` 的标题断言。

> 与 Tauri 版的差异：Tauri 版把配置固化成**编译期常量**（改配置必须重新编译）；
> Electron 版把配置**打进 `app.asar`**（打包期读取，改配置必须重新 `build` + `stage` + 打包）。
> 两者都不是「交付后运行期改文件」—— Electron 侧改完**源码工程里的** `config.json`
> 后要重新打包并替换 `app.asar`，仅重启应用不会生效。

## 设置 / 托盘 / 全局快捷键 / 窗口几何（M3）

### 运行期只有一个 JSON

| 文件 | 内容 | 写在哪 |
|---|---|---|
| `settings.json` | 唤醒快捷键、是否启用托盘、关闭是否最小化到托盘，**以及 `window` 段的窗口几何**（坐标 / 尺寸为**物理像素** + 是否最大化） | **与数据目录同位置**：`<data_dir>\settings.json`（默认 `data\settings.json`，跟着 exe 走）；旧位置 `%APPDATA%\<app_name>\` 仅作**读取兜底**（升级兼容，不迁移不删除） |

⚠️ 2026-09-23 起窗口几何不再单独占一个 `window-state.json`，而是并进 `settings.json` 的
`window` 段（设置字段仍平铺在顶层）—— 用户侧「只需要一个 JSON」。旧的 `window-state.json`
**不再读也不再写**：升级后窗口会回一次默认尺寸 / 位置，那个文件可以手工删掉。
⚠️ 因此**两个写入方都必须走读-改-写**（`saveSettings` / `saveWindowState`）：拿内存里的整份
文档直接落盘，会在「挪了下窗口 + 改了设置」同时发生时静默吃掉其中一方。探针两条断言
（`写几何不吃掉设置` / `写设置不吃掉窗口几何`）专门钉这件事。

⚠️ **落点 = 数据目录**（2026-09-23 使用者指出）：此前它固定放 `%APPDATA%\<app_name>\`，于是选了绿色版
（`data_dir = "data"`）之后 cookie / 缓存 / 日志都跟着 exe 走，**唯独设置文件留在系统盘** ——
同一套「程序的数据」里唯一不听话的一项。现在写入位置一律是 `<data_dir>\settings.json`，与
`data_dir` 字段的语义完全一致（`docs/packaging-config.md` §4.3），Tauri 版同规则同位置。

> 升级兼容：新位置还没有文件时会读旧位置那份（`%APPDATA%\<app_name>\`），第一次写盘即落到数据目录；
> 此后旧文件是死文件，可以手工删掉。⚠️ 读-改-写走的是**同一份读取候选**，否则升级后第一次保存
> 会用「新位置（空）算出来的文档」把老设置清零。

### 行为契约（与旧版逐条对齐）

| 项 | 行为 |
|---|---|
| 唤醒快捷键 | 显示 / 隐藏主窗口。默认**未设置**；可在设置窗捕获或从下拉选择。存的是 JS `KeyboardEvent.code`（如 `Digit4`） |
| 设置快捷键 | `Shift + F1`，**固定不可改** |
| 隐藏 / 恢复语义 | 可见且非最小化 → 隐藏；其余（隐藏 / 最小化）→ 恢复并聚焦。⚠️ 不能只看可见性，否则最小化状态下第一次按快捷键会「没反应」 |
| 托盘 | 默认**不启用**。菜单四项：显示窗口 / 设置 / 隐藏到托盘 / 退出。左键单击托盘图标 = 切换显隐（右键才弹菜单） |
| 关闭按钮 | 需**同时**勾选「启用托盘」+「关闭时最小化到托盘」才隐藏到托盘；否则关闭即退出 |
| 窗口几何 | 上次最大化 → 直接最大化；位置已在屏幕外 / 尺寸过小 → 丢弃几何并最大化（**保证窗口一定可见**）；否则原位恢复 |

### 设置窗

设置窗是 `settings://localhost/` 协议提供的**无边框窗**（620×600，可缩放，最小 460×400，居中，
`frame:false`），内容直接取自 `../webview-app/dist/settings.html` —— 也就是 Tauri 版的那一份，
**一行都不改**。版式是「面板铺满整窗 + 正文滚动」：标题 / 关闭 / 拖动都由设置页自绘在面板头部
（`-webkit-app-region:drag` 负责拖动）；面板直接贴住窗口四边 —— 不再有「居中面板 + 四周底衬」，
那圈底衬正是使用者指出的「大块空边框」。

打开方式有三条，走的是**同一个** `openSettings()`：托盘菜单「设置」、全局快捷键 **Shift+F1**、
以及标题栏「设置」按钮（位置在**最小化左侧**；单窗口模式经 `win:open-settings`，
多页签模式经标签栏的 `tabs:open_settings`）。

它靠 `src/renderer/settings-preload.ts` 提供的 `window.__TAURI_INTERNALS__.invoke` 垫片工作，
垫片只放行四个命令（`get_settings` / `save_settings` / `close_settings_window` / `clear_web_data`），其余一律拒绝。
好处是设置页永远不会分叉：改一次两个壳同时生效。

> 找不到 `settings.html` 时会退到一个内置精简页（只保留两个复选框），不会出现空白窗。

## 自绘标题栏与页面滚动条（`html` 不滚、**`body` 滚**）

**一句话结论**：注入的标题栏不改滚动条宽度，而是**换滚动根** —— `html{height:100%;overflow:hidden}`
不滚，`body{margin-top:38px;height:calc(100vh - 38px);overflow-y:auto}` 滚。
页面滚动的矩形从 38px 开始，滚动条只在 body 盒子内绘制，**物理上不可能越到标题栏上**。
（使用者原话：「滚动条不应该强制隐藏，只是不要超过菜单栏而已」。）

### 为什么只能这么做（2026-09-17 逐像素 + 对照实验查明）

**Chromium 把视口（根）滚动条绘制在所有 DOM 内容之上**，`z-index:2147483647` 也压不住；
而且它的几何恒为「视口右缘 × **整个**视口高」，没有任何 CSS 能把它往下缩。于是只要由 `html`
来滚，滚动条必然从 y=0 开始画、横穿标题栏，还顺带盖住最右侧的关闭按钮
（`elementFromPoint(innerWidth-1, 20)` 返回 `null` = 那里点不动）。

标题栏的 `getBoundingClientRect().width` 一直是 `innerWidth`（828，断言全绿），但右端那 15px 在
**绘制**阶段被滚动条覆盖 —— **rect 只证明布局，不证明可见**。此前两轮误判都源于把「现状」当期望值：
第一版断言写 `barRect.w == clientWidth` 并把那条缝注释成「WebView 的正常行为」，第二版写
`rect.width == innerWidth`。

实测（Electron 44 / 828×438 窗口 / 经典 15px 滚动条 / dpr=2，`pnpm run lab` 四模式对照）：

| 配置 | 标题栏那 38px 内 | 结论 |
|---|---|---|
| 原生（`html` 滚） | 全是滚动条像素 | ✗ 横穿标题栏 |
| `--enable-features=OverlayScrollbar` | 同左 | ✗ 该开关在 Electron 44 **已失效** |
| `--enable-features=FluentOverlayScrollbar` | 同左 | ✗ 同上 |
| `::-webkit-scrollbar{border-top:38px}` | thumb 仍从 y=0 起 | ✗ border 不参与滚动条零件布局 |
| `::-webkit-scrollbar-track{border-top:38px}` | 同上 | ✗ 同上 |
| `scrollbar-width:none` | 干净 | ✗ 滚动条也没了 —— 使用者明确否掉 |
| **`html` 不滚 + `body` 滚** | **y0..y32 = 纯标题栏色** | **✓ 唯一可行** |

### 代价与边界（都有断言或兜底）

- **`window` 级滚动 API** 会指向一个不再滚动的 `documentElement` ⇒ `shimWindowScroll()` 转发：
  `scrollY/pageYOffset/scrollX/pageXOffset`（getter → body）、`scrollTo/scroll/scrollBy`、
  body 的 `scroll` 事件补发到 `window` + `document`、`Document.prototype.scrollingElement` → body、
  `<html>` 实例上的 `scrollTop/scrollLeft` 遮蔽转发（只给这一个实例挂，不碰 `Element.prototype`）。
  懒加载 / 吸顶导航 / 回到顶部 / 阅读进度全靠这组 API。
- **`applyChromeFit` 不再动页面自己的 `position:sticky`**：body 已是滚动容器，`top:0` 的 sticky
  天然停在 y=38；再给它加 38 就成了 76px（吸顶导航凭空多一条空白）。`fixed` 仍照旧下移
  —— 它锚的是视口，不是滚动盒。
- **页面用 `!important` 压掉契约**会变成「html 不滚 + body 也不滚 = 页面钉死滚不动」，
  比原缺陷严重得多 ⇒ `ensureScrollRoot()` 探测（html `hidden` + body `auto` + body 高度 ≈ 视口-38，
  第三条专门抓 `body{height:auto!important}` 这种只查 overflow 抓不到的），失败即退回
  「html 顶部留白」旧模式并 `console.error` 说明原因。
  ⚠️ 它**必须在样式表挂上之后**跑：早一步时 computed style 还是页面原样，必然误判成失败、
  把退路写进 inline style（`!important` 优先级最高）—— 新契约就永久起不来了。
- `width:100vw` 保留：标题栏铺满整窗宽（含滚动条本该占的那一条），JS 再 `syncBarWidth()` 钉一次 px。
  加载层遮罩 `100vw/100vh` 同理。

### 盯住它的一组判据

| 判据 | 期望 | 说明 |
|---|---|---|
| `intrudeRows` | `[]` | x=`innerWidth-1` 逐点 y=0..37 必须**全部命中标题栏** ← 直接对应「滚动条别超过菜单栏」 |
| `bodyScrollbarW` | > 0（本机 15） | 滚动条**仍然存在**，且是占宽的那种，归 body 管 |
| `bodyScrollable` | `true` | 页面照常能滚 |
| `window.scrollTo(0,200)` | `scrollY≈200` **且** scroll 事件触发 | 转发没坏（事件异步，要等一帧再读） |

`pnpm run probe`（138 条）与 `pnpm run shot` 都是这套判据；`pnpm run shot` 另给 PNG 与 `INFO` 明细。
⚠️ **不要**再用 `rect.width == innerWidth` 或「滚动条宽度归零」当判据 —— 前者测不出覆盖，
后者把缺陷锁死成期望值。**改完必须重新打包**：脚本随 asar 分发在 `out/inject/titlebar.js` 里
（2026-09-26 起不再放 `resources/inject/`），跑旧的 exe 永远看不到修复。

> 排查这类「断言过、人眼不对」的问题请用 `scripts/scrollbar-lab.cjs`（多模式对照，几种做法一次跑完）
> 与 `scripts/png-inspect.cjs`（把 PNG 读成结构图 + 逐像素扫描）。两者都零依赖，
> 专治沙箱里没有图片库、`Add-Type` 被安全策略拦的情况。

## 本地代理 sidecar（`w2a-proxy.exe`）

Electron 侧有两块能力**不复用就会重开一遍验证**：

| 能力 | 为什么不能（也不该）用 JS 重写 |
|---|---|
| 本地反向代理 | 给**每一个**请求（含 WebSocket 握手）注入 `Authorization: Basic`。经过充分测试的 Rust：流式转发、Cookie Domain 重写、WS 双向隧道、一次性随机密钥门禁（无密钥一律 404） |
| 动态域名解析 | DNS TXT → Ed25519 验签 → AES-GCM 解密。Node 侧要引 Ed25519 + HKDF + AES-GCM 一整套，且必须与 `resolve-core` **字节级**一致 |

所以 `crates/proxy-server` 用 `#[path]` **直接引用** `webview-app/src/proxy.rs` 与 `src/resolve.rs`
（零重写、零复制），编出 `w2a-proxy.exe` 作为 sidecar。定位顺序见 `src/main/sidecar.ts::locateSidecar()`：

1. `process.resourcesPath/sidecar/w2a-proxy.exe`（打包后，由 `electron-builder.yml` 的 `extraResources` 放置）
2. `../webview-app/target/release/w2a-proxy.exe`（开发态，优先 release —— 代理是 CPU 敏感路径）
3. `../webview-app/target/debug/w2a-proxy.exe`

⚠️ **必须放 `extraResources`，绝不能进 asar**：asar 内的可执行文件无法被 `spawn`。

### 进程间契约

```
宿主 spawn → 传参 → sidecar 起代理 → stdout 打一行 JSON（宿主据此导航）→ 常驻转发
                                                    ↓
                              stdin EOF = 宿主已退出 ⇒ sidecar 自杀（防僵尸）
```

* **stdout 只给机器读**：成功/失败都打**且仅打一行** JSON；日志一律走 stderr（宿主并进自己的日志）。
  往 stdout 多打一行，宿主就会一直等握手 —— 表现是「窗口一直转圈、日志里什么都看不到」。
* **握手行必须 flush**（管道上是块缓冲，不 flush 宿主永远等不到）。
* **凭据走 stdin**（`--url-stdin`），不走命令行：Windows 上同用户的其它进程能通过
  `GetCommandLine` / WMI 读到命令行参数，而 URL 里内嵌的就是站点账号密码。
* **`--no-proxy`**（域名解析 + 直连）时 sidecar 只解析地址、不起代理，并在握手后立即退出；
  此时它会把凭据放在 `authUser` / `authPass` 字段里交给宿主（宿主用 Chromium 的 `login` 事件注入）。
  ⚠️ 这两个字段是**明文凭据**，任何地方都不得写日志。
* **握手成功行的 `proxyUrl` 自带一次性随机门禁密钥**（`http://127.0.0.1:端口/?w2a_key=<43 字符>`）：
  宿主只把该入口 URL 交给窗口；无密钥 / 错密钥访问代理一律 **404**（防同机进程扫描借道），
  首访回种的 HttpOnly cookie 让页面根绝对路径资源与 WS 自动通过，密钥不会转发给上游。

手动调试：

```bash
cd ../webview-app && cargo build --release -p w2a-proxy
./target/release/w2a-proxy.exe --help
echo "https://example.com" | ./target/release/w2a-proxy.exe --url-stdin   # 看握手 JSON
```

## 兼容契约探针（`pnpm run probe`）

`src/main/probe.ts` 自己起一个 `127.0.0.1` 随机端口的夹具站点，逐个导航并断言 **138 条**契约，
**退出码即结论**（不通过则非 0，可以直接挂 CI）。夹具全部内联在代码里，不落盘、不进打包产物。

覆盖范围：启动页内联注入、标题栏几何与配色、38px 让位（fixed / sticky / `100vh` / `inset:0` 四种页面形态）、
严格 CSP 站点的样式降级、加载层淡出、`target="_blank"` 与 `window.open` 的原地导航、
最小化 / 最大化 / 关闭三个按钮真的驱动了原生窗口，以及 **sidecar 的代理契约**：

* 直连夹具（不带凭据）必须 **401** —— 先证明夹具真在鉴权，否则后面的断言是假通过；
* **门禁**：无密钥、错密钥访问代理都必须 **404**（不透露代理存在），带入口 query 密钥才放行；
* 经代理必须 200，且**上游**收到的 `Authorization` 与 `proxy.rs::build_basic_auth` 的格式逐字节一致；
* **WebSocket 握手也带上了 `Authorization`** —— 这是本地代理当初存在的全部理由
  （WebView2 的原生鉴权事件对 WS 不生效），也是「Electron 复用同一份 Rust 代理」值不值的唯一依据；
* WebSocket **双向**通信打通（上游收到的客户端消息 + 页面收到的服务端消息）；
* `Origin` 被改写为**上游源**（且只有一个）—— 详见下面那条已修的坑；
* `stop()` 之后 sidecar 进程**真的退出**（`kill(pid, 0)` 探活），不留孤儿。

M3 之后还覆盖**系统集成**（**61 条，即第 11–14 节**；第 15 节是 M4 页面行为 4 条；
第 16 节是**子框架桥梁隔离** 4 条——同源 iframe 拿不到 `web2app` / `web2appExtra`，
顶框仍正常暴露；第 17 节关闭按钮 1 条）：

* `Hotkey`（旧版存 JS `event.code`）→ Electron accelerator 的映射，含「不认识的键名返回 null，绝不猜」；
* `settings.json` 的**序列化字节形态**必须与 `serde_json` 写出来的一致（字段名 / 嵌套 / 布尔类型 /
  **键序**）—— 这是「与 Tauri 版可互相读取」的硬保证，反向由 `webview-app` 侧的
  `store::tests::parse_accepts_electron_written_document` 盯着；
* **设置与窗口几何同住一个文件**后，两个写入方互不吃掉对方（`写几何不吃掉设置` /
  `写设置不吃掉窗口几何`）—— 合并的唯一真风险，两条方向都钉住；
* 存储位置规则：写入一律 `<data_dir>\settings.json`（= 数据目录，与 `data_dir` 字段一致），
  读取候选为「数据目录 → `%APPDATA%\<app_name>\`」（后者的 `window` 段也会被一并搬到新位置）；
* 窗口几何的**DIP ↔ 物理像素**换算（跨 DPI 不错位）与四种恢复策略（含「位置在屏外 → 最大化」）；
* 托盘：启用后图标出现、关闭后图标销毁；
* 全局快捷键：注册 / 不支持的键名明确报错 / 清空注销 三条路径；
* 设置窗：`settings://localhost/` 能打开、垫片就位、**点保存的完整链路**（页面 → IPC → 副作用 → 落盘）、
  未授权命令被拒、能关闭；
* 「关闭到托盘」：关窗后窗口未销毁且被隐藏，再切换能恢复。

> ⚠️ 探针**不碰用户真实配置**：它把 `settings.json`（唯一那份，设置 + 窗口几何）整体重定向到
> `%TEMP%\w2a-probe-store\`，跑 `pnpm run probe` 不会改掉你机器上的托盘开关和窗口尺寸。

**注入了 `titlebar.js` / `loading.js`、改过 `proxy.rs`、或动过设置/托盘/快捷键链路之后必须跑这个** ——
前两个是本壳与 Tauri 版共用的单一真源，`proxy.rs` 现在也是，设置页 HTML 同理。
探针是唯一能在改动后立刻发现「另一个壳被带坏」的手段。

> ⚠️ 探针抓到的第一个**真 bug（Tauri 版同样中招）**：代理里「先透传所有头、后面再设一次 Origin
> 来覆盖」的写法是无效的 —— `http::request::Builder::header` 与 `reqwest::RequestBuilder::header`
> 都是 **append**（`HeaderMap::append`）而不是 insert。结果是上游收到**两个** Origin
> （`http://127.0.0.1:代理端口, http://127.0.0.1:上游端口`），比不改写更糟：
> 任何做 Origin 校验的后端都会拒连，表现为「页面能开，但 WS / POST 全挂」。
> 修法是在透传时就跳过 `origin`，并有 `proxy_sends_exactly_one_rewritten_origin` 盯着。


> 无 GPU 的环境（容器 / 部分远程桌面）里 Chromium 会以 `GPU process isn't usable. Goodbye.` 退出。
> 探针模式**已自动**关闭硬件加速，但**这还不够** —— 2026-09-17 实测：GPU 子进程仍会反复崩
> （`exit_code=1`），最后一条 `FATAL` 直接把整个进程带走，退出码是 `-2147483645`（异常码，不是 1）。
> 这种环境必须再显式加 `--no-sandbox --in-process-gpu`（写法见上节「给脚本传参数时不要写 `--`」）。
> 真机不要长期关硬件加速（滚动与动画会明显变卡）。

## 安全基线

渲染进程加载的是**远程页面**，`webPreferences` 保持全默认值：`contextIsolation: true` /
`nodeIntegration: false` / `sandbox: true` / `webviewTag: false`。
页面只能通过 `contextBridge` 暴露的白名单 `window.web2app`（`isElectron` / `chromeH` /
`minimize` / `toggleMaximize` / `close`）与壳交互，拿不到 `ipcRenderer`、`require`、`process`。

注入脚本经 **`webFrame.executeJavaScript` 在主世界**执行（不是隔离世界）—— 原因与踩坑记录见
`src/renderer/preload.ts` 顶部注释。

## 占位符契约（已加固）

`splash.html` 用三个占位符承载运行时数据：`__WEB2APP_TITLE__`、`__WEB2APP_TITLEBAR_JS__`、
`__WEB2APP_LOADING_JS__`，由 `loading.rs`（Tauri）与 `main/index.ts`（Electron）各自替换。

⚠️ 曾经的两个隐患，都已有回归测试盯着：
1. 占位符曾被包在 `/* … */` 注释里，而 `titlebar.js` 的头注释本身就含数十处 `*/`
   （脚本持续在增长，具体数字/行号会漂移，勿再写死）—— 外层注释随时会**提前闭合**。
   现已改为**裸 token**（`<script>__WEB2APP_TITLEBAR_JS__</script>`），并有
   `splash_script_tokens_are_not_comment_wrapped` 等测试。
2. 替换必须用 `split/join`，**绝不能用 `String.replace`** —— 替换串（即整个 JS 源码）里的
   `$&`、`$'` 会被当作替换模式，直接把脚本改坏。

## 验证清单（改了什么就必跑什么）

| 你改了什么 | 必跑 | 为什么 |
|---|---|---|
| `titlebar.js` / `loading.js` / `splash.html`（**两壳共用的单一真源**） | `probe` | 这两个脚本是「错了也静默」的重灾区：顶部栏没挂上、38px 没让位，界面看起来都「差不多」 |
| `proxy.rs` / `resolve.rs`、sidecar 契约 | `probe`（内含 sidecar 三连断言）+ `cargo test` | 代理是两壳共用的 Rust 源码，改一处两个壳一起变 |
| 设置 / 托盘 / 快捷键 / 窗口几何链路 | `probe` | 探针走的是与设置窗**完全相同**的那批函数，不是复刻实现 |
| `config.ts`、路径装配（`portable.ts`） | `smoke` | 绿色版路径与配置解析全是「错了也静默」，`smoke` 会把生效路径打出来 |
| 启动日志（`startup-log.ts`）、`data\logs` 落点、`debug` 开关与 `silenceConsole()` | `smoke` | 「日志没写出来」同样是静默故障，而它是用户侧唯一的证据来源；`smoke` 断言文件已含 `[web2app] 启动信息`（冒烟内部强制 `debug=true`）。⚠️ 改 `debug` 默认值或静默逻辑时必须连带看探针 —— `--probe` 同样强制开，**一旦把 `console` 永久静默就会让所有自检一起变哑** |
| 打包配置 / 出包前后 | `probe` + 在**真机**双击产物 | 打包产物没有 stdout，只能靠窗口标题、`data\` 与缓存取证 |
| 任何提交前 | `typecheck`（`build` 由 `pack`/`dist` 自动跑） | `typecheck` **不能**代替 `build`，见「打包」表第 2 行 |

产物侧的 `--probe` 也可以跑（包内自带探针），但**必须**是
`Start-Process -Wait -PassThru` 取 `$p.ExitCode` —— 直接 `& app.exe --probe` 对 GUI 子系统 exe
是**假绿**（PowerShell 不等待，退出码立刻为 0）。

## 变更记录

| 版本 | 日期 | 变更说明 |
|---|---|---|
| 0.1.0 | 2026-09-27 | **第五轮审计闭环（6 项 P1 + P2，对应根 README v1.37）**：①修启动失败后的**无限自动重试**循环（失败重载 splash 会再次自动 bootstrap；新增 `splashAutoBootConsumed`，自动启动一生一次，失败只等用户点重试）；②修关闭**后台页签**后标签栏残留幽灵页签（`closeTab` 非活动分支补 `pushToShell`，探针新增 P7c 2 条，31 → **33**，含反向对照「撤推送 → 壳=3 集合=2 FAIL」）；③设置保存失败时**被销毁的托盘对称回滚**（best-effort 重建，旧实现只回滚新建侧）；④preload 整个运行时主体收进 `window.self === window.top` 分支——子框架不 `sendSync` bootstrap、不暴露 `web2app` / `web2appExtra`、不注册 overlay 监听（探针新增第 16 节 4 条，134 → **138**；实测 Electron 44 默认不向子框架注入 preload，本改动为纵深防御+前向兼容）；⑤`config.ts` 新增 `normalizePublicKey`（字面量 `\n`/`\r` 归一，与打包解析器同源——PEM 公钥域名模式此前在本壳必败）；`sanitizeDirName` 补齐连续空白压缩与首尾句点剥离，与 Rust `sanitize_dir_name` 完全对齐；`debugFlagTruthy` 让 `WEB2APP_DEBUG` 接受 1/true/yes（与 Rust 逐字一致，旧实现只认 `'1'`）；⑥P2：设置 IPC 四个 handler 补 sender 校验（纵深防御）；`stopSidecar` 用 WeakSet 幂等（重复调用不再叠加定时器/监听器）；sidecar 握手成功后异常退出写进日志（旧实现静默）；删不可达的「同地址短路」分支；订正探针头「12 条契约」、分段 0–15、注入脚本 49KB、backgroundThrottling 等过期注释；⑦独立审查（R1）后补：正常 `stop()` 不再误报「异常退出」（WeakSet 区分主动停止）、目录名清洗空白集精确对齐 Rust White_Space（NEL 压空格 / BOM 保留，新增 1 条单测）。门禁：`typecheck` 通过、单测 **40/40**（+5：清洗 2、public_key 2、debug 真值 1）、探针 **138/138** 与 **33/33** 全绿 |
| 0.1.0 | 2026-09-26 | **注入资源随 app.asar 分发 + 三类清单审计修复**：①打包形态调整 —— 注入资源（`titlebar.js` / `loading.js` / `splash.html` / `settings.html`）改由 `stage-assets.cjs` 铺到 `out/inject/`，随 `files: out/**/*` **进 `app.asar`**，删除 asar 外的 `resources/inject/`（`electron-builder.yml`、`index.ts`、`settings-window.ts`、`check-asar-config.cjs` 同步）；②两壳共用真源的 UI 打磨：`titlebar.js` 与 `shell.html` 的按钮悬停提示从原生 `title` 换成自绘 CSS tooltip（随深浅色主题、无系统延迟），按钮光标 `default → pointer`；`settings.html` 压缩两行低价值文案；③**审计修复**：修多页签 `closeTab` 邻接选择 bug（删除后才算索引恒为 -1，关中间 / 关末尾都跳回第一个幸存页签，与 Tauri `close_tab_inner` 分叉）——纯规则抽到无 Electron 依赖的 `tab-order.ts` 并新增 `test/tabs.test.ts` 6 条单测，探针补 3 页签邻接断言（多页签 26 → **31**，单测 29 → **35**）；首个页签创建失败时壳页面注入**可见错误遮罩 + 重试按钮**（新增 `tabs:retry_first_tab`，治空白壳 + debug=false 静默）；sidecar 握手成功行校验 `proxyUrl` 合法、stdin 异步 error 加监听；设置保存失败时回滚本次新建的托盘；`tabs-v4.cjs` 修 node 下必崩（改为自动 spawn electron 承载）；清理死代码（`currentWakeAccelerator`、sidecar 死字段、托盘死候选）与一批过期注释；④文档：三个 README 与 docs 计数 / 路径 / workflow 条数全面对齐（根 README v1.35） |
| 0.1.0 | 2026-09-24 | **M4 页面行为交付（下载接管 / 右键菜单 / F12）**：新增 `src/main/page-behavior.ts`，「纯规则函数 + Electron 挂载」分层——纯规则（`devtoolsEnabled` / `isDevtoolsKey` / `contextMenuTemplate` / `saveDialogOptions`）由 `test/page-behavior.test.ts` 14 条毫秒级单测钉住，挂载点由探针 4 条断言验（探针 130 → **134**）。下载：暂停 → 系统另存为 → 落盘 / 取消，取消**不**静默另存；右键菜单取舍对齐 Tauri（不给打印/另存为/更多工具，带「设置」项）；F12 对齐 `DEVTOOLS_ENABLED`（未打包恒开、打包版由 `debug` 决定）。⚠️ 实现时踩到：Node 按 **ESM** 解析 `.ts`，`import { Menu } from 'electron'`（CJS）直接抛 `Named export not found` ⇒ 必须默认导入再解构 |
| 0.1.0 | 2026-09-24 | **M7 CI 流水线落库**：新增 `.github/workflows/electron-ci.yml`（lint / e2e / package 三作业）。取证三件事：①`pnpm install` 必须显式 `--registry=https://registry.npmjs.org` 覆盖 `.npmrc` 的国内镜像；②**不生成** `config.json` —— 它本就不在仓库里，实测「无配置」场景下探针仍 130/130（探针自起 127.0.0.1 夹具站点），这样 CI 验的正是「fresh clone 能跑」；③Node 22 可直接 `node --test` 跑 `.ts`（实测 15/15），故与既有 workflow 同取 22 |
| 0.1.0 | 2026-09-24 | **与 Tauri 侧对齐 + 测试资产纳管**：①`settings.json` 的**原子写补齐到 Tauri 侧**（`store.rs::save_at` 由 `fs::write` 直接覆盖改为「同目录临时文件 + flush + rename」，失败清理临时文件）——此前只 Electron 侧做了，Tauri 侧「写一半被杀 ⇒ 截断 JSON ⇒ 下次启动解析失败、设置与几何一起回默认且不报错」的链条仍然成立；两壳写盘策略自此一致；②`test/`（纯函数单测，`pnpm run unit`）此前未纳入版本控制，已入库 |
| 0.1.0 | 2026-09-24 | **安全审计与健壮性收口**：①本地代理入口加**一次性随机密钥门禁**（入口 URL 带 `?w2a_key=`，query 或 `HttpOnly; SameSite=Lax` cookie 必须带对，无/错一律 **404** 而非 401/502 —— 不透露代理存在），**WS 握手同样受门禁**，转发上游前剥除 query 与 cookie 中的密钥；②`login` 事件回填凭据**绑定入口主机:端口**（外域 401 子资源骗不到入口账号密码）；③多页签托盘恢复自愈（零页签时自动补建，此前恢复后是空壳）；④直连 userinfo 非法百分号编码兜底（不再 unhandled rejection、splash 不再永久转圈）；⑤新增 `test/config.test.ts`（node:test 15 条）；⑥`settings.json` 写盘改原子写；⑦sidecar 握手 `kind` 白名单收敛、`pid` 必须有限数、清理死代码与过期注释；探针 128 → **130** |
| 0.1.0 | 2026-09-23 | **设置面板铺满整窗 + 「设置」按钮移到最小化左侧 + `settings.json` 落点改为数据目录**：①`settings.html` 去掉「居中面板 + 四周底衬」（`body` 的 12px padding 与 `.panel` 的 `max-width:560px` 在 620 宽的窗里各留下 30px 空档，使用者指为「大块空边框」）⇒ 面板铺满整窗、无 border / 圆角 / 外阴影，头部加一条分隔线补回边界感；②两壳共用的 `titlebar.js` 与多页签标签栏 `shell.html` 把 ⚙ 从「最小化右侧」移到**最小化左侧**（`[←][→] 标题 [⚙][—][▢][✕]`），探针的位置判据同步反过来（比几何、不看 DOM 顺序）；③**`settings.json` 落点 = `data_dir`**（此前固定 `%APPDATA%\<app_name>\`）：写入一律 `<data_dir>\settings.json`（与 WebView2 数据目录、`logs\` 同一个，绿色版搬目录不再漏掉设置），旧位置保留为**读取候选**（升级兼容，不迁移不删除），读-改-写与 load 走同一份候选，避免「升级后第一次保存把老设置清零」；④探针 126 → **128** 条（新增「落点搬家：读旧位置写数据目录且 `window` 段跟着搬」「旧位置一个字节不动」），单窗口 128/128、多页签 26/26、冒烟自检通过全绿 |
| 0.1.0 | 2026-09-23 | **设置界面改为「无边框窗 + 居中面板 + 正文滚动」+ 标题栏新增「设置」按钮**：①`settings.html` 重做版式 —— 窗口无系统标题栏，整页只有一块居中的圆角面板（四周留底衬），标题 / 关闭 / 拖动由面板头部自绘；`html/body` 都不滚，**唯一滚动容器是 `.panel-bd`**（否则滚动条会横穿面板圆角），底部操作区固定（内容再多「保存」也不会被滚出视野）；元素 id 与脚本逻辑一字未改；②设置窗参数 480×470 / 带系统标题栏 / 不可缩放 → **620×600 / `frame:false` / 可缩放（最小 460×400）/ 居中**，Tauri 侧同步改 `decorations(false)` + `.center()`；③标题栏在**最小化右侧**新增「设置」按钮，两壳两模式都接上：单窗口走 `window.web2app.openSettings` → IPC `win:open-settings`，多页签走标签栏 `invoke("open_settings")` → `tabs:open_settings`（`shell.html` 字面量 ↔ `shell-preload` 白名单 ↔ 主进程三处逐字一致）；④探针 117 → **126** 条（新增「按钮位置」「点按钮真能开设置窗且页面已渲染」「设置窗无边框」「面板四周留底衬」「面板居中」「正文是唯一滚动容器」「头部可拖动」等），多页签 23 → **26** 条。⚠️ 反例教训：第一版「点按钮开设置窗」只断言了窗口对象存在 —— 页面加载失败时它也成立（假绿），且探针抢在加载完成前 `close()` 会中止导航、在日志里留下 `ERR_FAILED(-2)` 噪声；改成等 `document.querySelector('.panel')` 出现后两条一起消失 |
| 0.1.0 | 2026-09-23 | **窗口几何并入 `settings.json`（运行期只剩一个 JSON）**：①`window-state.json` 取消，几何改为 `settings.json` 的 `window` 段（设置字段仍平铺顶层），与 Tauri 版同契约；旧文件**不迁移**、不再读写（升级后窗口回一次默认尺寸/位置）；②合并的唯一风险是「两个写入方互相覆盖」（`window-state.ts` 文件头当初正是用这个理由论证不该合并）⇒ 两个方向都改成**读-改-写**（`saveSettings` / `saveWindowState`，只替换自己那一段），探针新增 4 条断言钉住（含键序 `设置三键 → window`），113 → **117** 条；③Rust 侧新增 `src/store.rs` 承载文档契约（`WindowGeom` + 读-改-写 + 「一段坏掉不拖垮另一段」的独立解析），`settings.rs` / `window_state.rs` 退化为薄封装；单测 65 → 71（新增 9 条含双壳字面量互通） |
| 0.1.0 | 2026-09-22 | **诊断日志改为 `debug` 开关 + 托盘图标对齐 `icon_url`**：①新增契约字段 `debug`（**bool，默认 `false`**）—— `false` 时**不建 `logs\`、不写文件，控制台三路输出也一并静默**（此前是无条件落盘，会在用户的交付目录里凭空多出 `data\logs\`）；`true` 时写 `<数据目录>\logs\startup.log` 并照常打印；运行期逃生口 `--debug` / `WEB2APP_DEBUG=1`；**探针与冒烟内部强制打开**（结论全靠 stdout，不能自检失声）；②`logStartup()` 增印「诊断日志：已开启 / 未开启」，冒烟断言在 `fileLog === null` 时报「未启用（冒烟模式本应强制打开 debug，出现即 bug）」—— 把「开关接错」与「写盘失败」两类原因分开；③**托盘图标修复**：`electron-build-custom.yml` 下载的 `icon_url` 此前只喂给 `-c.win.icon`（exe / 任务栏图标），托盘取的是 `resources/icons/icon.png`（由 `extraResources` 从 `webview-app/icons/` 拷入的**仓库默认图标**）⇒ 下载后**同时覆盖 `../webview-app/icons/icon.png`**，使托盘与 exe 用同一张 `icon_url`；复用既有单一真源，不改 `tray.ts` |
| 0.1.0 | 2026-09-17 | M6 出包打通（dir / zip / portable 三形态）；修两个真 bug：`createWindow()` 漏传 `title`、`tsc` 有错也输出导致半成品封进 asar |
| 0.1.0 | 2026-09-17 | 滚动条越界：从 `width:100vw` → `scrollbar-width:none` → 最终**换滚动根**（`html` 不滚、`body` 滚）；探针判据改成成对的「不侵入 + 仍存在」 |
| 0.1.0 | 2026-09-17 | 文档与配置订正：①`config.example.json` 补全到契约的 17 个字段（原先缺 `icon_url` / `proxy_https` / `resolve_secret` 及两个 Android 密码字段）；②移除进包的死资源 `shell.html`；③`.gitignore` 补 `dist/`、`dist-*/`；④「无 GPU 环境」说明改为与实测一致（必须加 `--no-sandbox --in-process-gpu`） |
| 0.1.0 | 2026-09-17 | 健壮性补强：①`whenReady` 之后加顶层兜底，任何 await 抛错都收敛成**非 0 退出码**（此前只是 `UnhandledPromiseRejectionWarning`，既不报结论也不退出，CI 里表现为挂死）；②保存设置失败时**回滚已生效的唤醒快捷键**，兑现「任一步失败都不写内存态」 |
| 0.1.0 | 2026-09-18 | **M6 收尾：启动日志落盘** —— 新增 `src/main/startup-log.ts`（零 Electron 依赖、fail-open、1 MiB 轮转），装配到 `data\logs\startup.log`，探针模式下重定向到 `%TEMP%\w2a-probe-store\` 以免污染真实目录；`smoke` 增加「日志已含启动信息」硬断言；`logStartup()` 打印日志路径。负向对照：`ENOTDIR` / `EISDIR` 两种写失败均 `ok=false` 且不抛异常 |
| 0.1.0 | 2026-09-18 | 审计修复：①`renderSplash()` 的失败态注入由 `String.replace` 改 `split/join` —— 原写法把含失败 URL 的文案当**替换串**解释，`$&` / `` $` `` / `$'` 会截断启动页脚本（该约束本就写在同文件注释里，属自食其言）；②`config.ts` 里「`tabs_enabled` M5 实现前仅记录、不生效」的过期注释订正；③`index.ts` 两处逐字重复的解析路径文案表抽成 `howLabel()`；④`tabs.ts` 的 `contentBounds()` 高度钳零（窗口被压到比标签栏还矮时会让 `setBounds` 收到负高度） |
| 0.1.0 | 2026-09-22 | **`data_dir` 取代 `data_dir_default` + Android 包名随 `app_name`**：①`data_dir_default`（布尔，两壳同名反义）废弃为 `data_dir`（**字符串，两壳同名同义**）—— `"data"`（默认）= exe 同级 `data\`、`""` = 系统默认目录、其它相对路径 = exe 同级、绝对路径 = 原样；⚠️ 空串是**有意义的值**，读取处禁止 `data_dir \|\| 'data'`（会被 `'data'` 吃掉）；②相对路径基准明确为 **exe 所在目录**（`installRoot()`）而非 cwd；③`portable.ts` 的 `DataDirMode` 扩为 `relative \| absolute \| system \| fallback`，三模式均经 `launch.cjs --smoke` 实测；④`config.ts` 的 `rejectUnknownKeys` 改用 `hasOwnProperty`（`in` 会在 `toString` 这类键上被原型链误判） |
| 0.1.0 | 2026-09-22 | **配置契约统一 + 手动打包 workflow**：①`config.json` 从「exe 同级（`extraFiles`）」改为**随 `app.asar` 进包**，杜绝拿到包就能改地址；`configRoot()` 读 `app.getAppPath()/out`，新增 `scripts/check-asar-config.cjs` 断言配置确在 asar 内（防静默退回 `example.com`）；②`config.ts` 重写为与 `.github/scripts/parse-packaging-config.py` 逐条对齐的严格解析：布尔只认真 `bool`（字符串报错并给改法）、未知字段硬失败、`app_url` 接受真 `string \| string[]`、`window_title` 报错；③`DEFAULTS.proxy_bypass` 与 Tauri/解析器三处对齐；④新增手动打包 workflow `自定义打包（Electron 绿色版）`，与 Tauri 版共用同一解析器，并在 `RESOLVE_SECRET` 环境下编译 sidecar |
| 0.1.0 | 2026-09-20 | 文档与仓库卫生审计：①源码/构建配置里 10 处失效引用 `PLAN.md` → `docs/electron-plan.md`（该文件 2026-09-17 已归位）；②注释里 `npm run *` 统一为 `pnpm run *`（包管理器已迁移，`electron-builder.yml` 同步）；③`.gitignore` 补 `tests/*.log`、`tests/*.exe`、`tests/*.zip` —— 原 `tests/*/` 只忽略目录，`tests/` 下一份 ~96MB 便携版副本与 10 个探针日志会被随仓库提交（M7 纳入版本控制前的必要前置）；④README 补 `docs/electron-size-reduction.md` 与根 `README.md` 的交叉引用，并声明「里程碑状态以本 README 为准」 |
