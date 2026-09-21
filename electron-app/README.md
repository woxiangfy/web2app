# electron-app

web2app 的 **Windows 绿色版**壳（Electron），与 `../webview-app`（Rust + Tauri 2）行为兼容。
Tauri 版继续维护（同时负责 Android），两者共用同一份 `config.json` 契约与同一套注入脚本。

> 完整方案见 `docs/electron-plan.md`（⚠️ 那是**设计方案 + 过程记录**，里程碑状态与命令**以本 README 为准**）；
> 前期可行性论证见 `docs/electron-analysis.md`；打包体积与分发形态见 `docs/electron-size-reduction.md`；
> 全仓库规则与目录地图见根目录 `README.md`。

## 现状与未实现能力（先看这张表再动手）

| 里程碑 | 状态 | 说明 |
|---|---|---|
| M0 工程脚手架 / M1 窗口 + 注入 + 探针 | ✅ | 无边框窗口 + 注入脚本 + 探针骨架（探针 39 条起家，现已 112 条） |
| M2 sidecar（本地反向代理 + 加密解析） | ✅ | 复用 Rust `proxy.rs` / `resolve.rs`，零重写 |
| M3 托盘 / 全局快捷键 / 设置窗 / 窗口几何 | ✅ | 与 Tauri 版文件级互通 |
| M6 出包（dir / zip / portable） | ✅ | `pack` / `dist`。三形态实测出包；**启动日志**已落地 `data\logs\startup.log`（见「日志目录」一节）；**版本信息**（`VERSIONINFO`）由 electron-builder 从 `package.json` 的 `version` / `description` / `author` 与 `electron-builder.yml` 的 `copyright` 自动写入，实测已落地（取证命令见「日志目录」一节）；首次运行的 SmartScreen 提示见「首次运行与 SmartScreen」 |
| **M4 下载接管** | ⬜ **未实现** | `will-download` 目前只记日志、下载走 Chromium 默认「另存为」；对标 Tauri `save_dialog.rs` 的自绘对话框待做 |
| **M5 多页签** | ✅ **已完成（T0–T4）** | `tabs_enabled=true` → 多页签模式（`BrowserWindow` 当壳 + 子 `WebContentsView`，与 Tauri 侧同构，窗口类不变）。**选型依据与逐版过程**见 `docs/electron-tabs-design.md`（含 §13 变更记录）；**真机验收清单与实测结论**见 `docs/electron-tabs-test-steps.md`。收口状态：探针 **23/23**、单窗口 **112/112** 零回归；2026-09-18 真机目视 **V1–V4 全部关闭**（多页签 8/8 正常、跨 DPI 正常；仅「代理 / 域名解析」留待后续单独测试，属 sidecar 链路）。⚠️ 两条别踩：①**探针的起窗模式只由命令行决定**，`tabs_enabled` 不参与——否则页签配置下跑 `--probe`，112 条单窗口断言会往壳页面注入脚本而必失败，报错只是 `Script failed to execute`，完全不指向根因；②新页签**不得**调 `bootstrap()`（它会掐掉在服页签的代理并把壳页面导航走），走 `resolveInitialTarget()` 复用 target 缓存 |
| **M7 CI 流水线** | ⬜ **未落库** | 本目录没有 CI 配置（`webview-app/.github/workflows/` 那几条只服务 Tauri 版），目前只能本地跑 |

> 除上表三项外，后续「新增/改动会影响哪些验证」见文末「验证清单」——
> 这是本壳唯一能防「改了一个壳、把另一个壳带坏」的手段。

## 命令

```bash
# ⚠️ 首次安装前先设两个镜像环境变量，否则会卡在下载（见下节 pnpm）
pnpm install         # 装依赖；装完必须确认 node_modules/electron/dist/electron.exe 存在

pnpm start           # 编译 + 启动（开发态）
pnpm run typecheck   # 只做类型检查（不能代替 build，见「打包」表）
pnpm run build       # 只编译到 out/

pnpm run sidecar     # 编出 ../webview-app/target/release/w2a-proxy.exe（本地代理 + 加密解析）
pnpm run stage       # 打包前置：铺 build/icon.ico 与 build/config.json（pack/dist 会自动调用）
pnpm run smoke       # 冒烟自检：打印生效路径/配置 + 断言启动页模板/设置页/托盘图标/窗口标题/启动日志落盘，退出码即结论
pnpm run probe       # 兼容契约探针：112 条断言，退出码即结论 —— 动过注入脚本/代理/设置链路必跑
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
| 单文件版双击后是空白官网/`example.com` | `config.json` 没放在 portable exe 旁边，且包内也没有预置副本 | 现在已预置（`stage` + `extraFiles`）；想覆盖就把 `config.json` 放到 exe 旁边 |
| **改了注入脚本、重新打包后「现象依旧」** | 双击的是**旧产物**：`release/` 下 zip / portable / 解压目录多份并存，旧文件不会被自动清理 | 见下节「先确认跑的是哪一版包」——先核对 sha256 与运行痕迹 mtime，再动代码 |

### ⚠️ 先确认「跑的是哪一版包」（30 秒，省一轮返工）

「源码已修好」与「现象依旧」可以同时为真 —— 用户双击的常常是几天前那份产物（`release/` 里多份
产物并存，旧文件不会被自动清）。2026-09-17 实测过一次：探针全绿，而包内 `resources\inject\titlebar.js`
还停在修复前。

```bat
:: 1) 注入脚本对不对版：比这两个 sha256
certutil -hashfile release\win-unpacked\resources\inject\titlebar.js SHA256
certutil -hashfile ..\webview-app\src\titlebar.js                        SHA256

:: 2) 最近跑的到底是哪份包：看运行痕迹 mtime（早于修复时刻 ⇒ 就是旧包）
dir /tc release\win-unpacked\data
dir /tc "%APPDATA%\Baidu\window-state.json"
```

对齐办法（**覆盖式**，不用删除）：把源码的注入脚本 + 新包的 `app.asar` copy 覆盖进用户实际在跑的
那份包，再重出 zip / portable 覆盖旧产物。
⚠️ **必须成套换**：`Web2App.exe` + `resources\app.asar` + `resources\inject\*.js` + `config.json`
四件一起覆盖（`data\` 保留）。只换注入脚本不换 `app.asar` 时，包内**旧探针**会因旧断言与新行为
冲突而失败（`--probe` 退出码 1），看起来像「修复把东西改坏了」。

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

| `config.json` 的 `data_dir_default` | 数据位置 |
|---|---|
| `false`（默认） | **exe 同级 `data\`** — 整个目录可搬走，不写系统盘、不写注册表 |
| `true` | `%APPDATA%\<app_name>`（如 `app_name=Baidu` 即 `%APPDATA%\Baidu`）— 跟随系统 |

同级目录不可写时（写保护的 U 盘、`Program Files`、只读网络盘）会**自动回退**到用户目录并打日志，
保证「能启动」优先于「够绿色」。四个路径一起重定向：`userData` / `sessionData` / `logs` / `crashDumps`
（只改 `userData` 会留下 Cookies 和日志在系统盘）。

> 与 Tauri 版的差异：Tauri 的 `data_dir_default=false` 写的是 `%LOCALAPPDATA%\<app_name>`。
> 这是有意的——Electron 版定位为真绿色版。部署文档需明示这一点。

## 日志目录（`data\logs\startup.log`）

⚠️ **Windows 上 `electron.exe` 是 GUI 子系统程序**：用户双击运行时 `console.log` 没有任何去处
（没有控制台可挂）；portable 单文件形态更彻底——自解压包装器会吞掉内层进程的 stdout。
于是「双击没反应 / 打开的是 example.com / 标题不对」这类反馈在用户侧**零证据**，只能远程猜。
所以启动后立刻把 console 的三路输出同时落一份盘（`src/main/startup-log.ts`）：

| 形态 | 日志位置 |
|---|---|
| 解压目录版（`data_dir_default=false`，默认） | 「用户放置 exe 的目录」`\data\logs\startup.log` |
| `data_dir_default=true` | `%APPDATA%\<app_name>\logs\startup.log` |
| 探针（`--probe`） | `%TEMP%\w2a-probe-store\startup.log` ← **不写真实目录**（与 `settings.json` / `window-state.json` 同一套隔离） |

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
* 交付话术建议照抄上一段，并附上 `data\logs\startup.log` 的位置：
  「点了没反应」十有八九是被 SmartScreen 拦在门外，而不是程序本身有问题。

## 配置文件

启动时按顺序找 `config.json`（`config::loadConfigAny`）：

1. **`installRoot()\config.json`** —— 开发态为项目根目录；绿色版为 exe 所在目录（用户可直接改）；
2. **包内预置的那份** —— 仅单文件自解压形态会用到：`extraFiles` 把 `config.json` 放到
   `resources\` 的上一级，若用户没在 portable exe 旁边放配置，就回落到它，而不是内置默认值。

> 没有第 2 条时，portable 单文件版会连着内置默认的 `example.com` 启动 ——
> 「双击就打不开目标站点」会被误判成打包失败。两个位置都没有时才用内置默认值，并在启动日志里打印查找过的路径。

字段与 `webview-app` 的配置契约**同名同义、不增不改名**；`android_*` 四个字段在本壳中静默忽略。
`config.example.json` 已按契约**列全 17 个字段**（13 个通用 + 4 个 Android），复制成 `config.json`
再改即可用；其中布尔值故意写成字符串（`"true"` / `"false"`），用来演示下面这条宽容解析规则。
解析是**宽容**的：布尔值接受 `true` / `"true"` / `"1"` / `"yes"`（因为上游 `build-custom.yml` 的模板里是字符串）。
文件存在但 JSON 非法时**直接抛错**（不静默回落）—— 否则「改坏一个字符」的表现是「配置像没生效」，最难查。

`config.json` 随包分发：打包脚本 `scripts/stage-assets.cjs` 把项目根的 `config.json`
（缺失则生成一份全字段默认模板）复制到 `build/config.json`，再由 `extraFiles` 放到 exe 同级。
`config.json` 与 `data/` 都在 `.gitignore` 里 —— 它们是运行期产物，不是源码。

### `app_name` 与 `title`

| 字段 | 作用 | 本壳取值 |
|---|---|---|
| `app_name` | 安装名 / 数据目录名（Android 侧为应用名） | 数据目录与 `settings.json` 所在的 `%APPDATA%\<app_name>\` |
| `title` | **显示名**：窗口标题、托盘提示 | 缺省回落 `app_name` |

⚠️ 窗口标题在创建 `BrowserWindow` 时就固定为 `cfg.title`，并拦截 `page-title-updated`
（对齐 Tauri 版：不跟随 `document.title`）。**漏传 `title` 不会退回成「显示页面标题」，
而是永远停在 Electron 默认值**（`package.json` 的 `name`）—— 见 `smoke` 的标题断言。

> 与 Tauri 版的差异：Tauri 版把配置编译成**编译期常量**（改配置必须重编译）；
> Electron 版是**运行期读文件**，改完重启即生效。

## 设置 / 托盘 / 全局快捷键 / 窗口几何（M3）

### 两个小 JSON 的位置（有意与「数据写同级 `data\`」不同）

| 文件 | 内容 | 写在哪 |
|---|---|---|
| `settings.json` | 唤醒快捷键、是否启用托盘、关闭是否最小化到托盘 | 优先 `%APPDATA%\<app_name>\`（**与旧版同路径**），若 `data\` 下已有同名文件则以它为准 |
| `window-state.json` | 窗口坐标 / 尺寸（**物理像素**）、是否最大化 | 同上 |

⚠️ 这两个文件放 `%APPDATA%` 而不是 `data\`，是为了满足「**与 Tauri 版可互相读取**」这条契约：
用户在两个壳之间来回切换时，勾过的托盘开关、设过的快捷键不该丢。
真正体量大的 session / 缓存 / 日志仍然全在 `data\`，绿色性不受影响（详见 `src/main/settings.ts` 文件头）。

> 想让它彻底绿色？把 `settings.json` / `window-state.json` 手工放进 `data\` 即可 ——
> 解析规则是「`data\` 优先，且读哪写哪」。

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

设置窗是 `settings://localhost/` 协议提供的**普通窗口**（480×470，带系统标题栏，不可缩放），
内容直接取自 `../webview-app/dist/settings.html` —— 也就是 Tauri 版的那一份，**一行都不改**。

它靠 `src/renderer/settings-preload.ts` 提供的 `window.__TAURI_INTERNALS__.invoke` 垫片工作，
垫片只放行三个命令（`get_settings` / `save_settings` / `close_settings_window`），其余一律拒绝。
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

`pnpm run probe`（112 条）与 `pnpm run shot` 都是这套判据；`pnpm run shot` 另给 PNG 与 `INFO` 明细。
⚠️ **不要**再用 `rect.width == innerWidth` 或「滚动条宽度归零」当判据 —— 前者测不出覆盖，
后者把缺陷锁死成期望值。**改完必须重新打包**：脚本在 `resources/inject/titlebar.js` 里，
跑旧的 exe 永远看不到修复。

> 排查这类「断言过、人眼不对」的问题请用 `scripts/scrollbar-lab.cjs`（多模式对照，几种做法一次跑完）
> 与 `scripts/png-inspect.cjs`（把 PNG 读成结构图 + 逐像素扫描）。两者都零依赖，
> 专治沙箱里没有图片库、`Add-Type` 被安全策略拦的情况。

## 本地代理 sidecar（`w2a-proxy.exe`）

Electron 侧有两块能力**不复用就会重开一遍验证**：

| 能力 | 为什么不能（也不该）用 JS 重写 |
|---|---|
| 本地反向代理 | 给**每一个**请求（含 WebSocket 握手）注入 `Authorization: Basic`。741 行已测 Rust：流式转发、Cookie Domain 重写、WS 双向隧道 |
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

手动调试：

```bash
cd ../webview-app && cargo build --release -p w2a-proxy
./target/release/w2a-proxy.exe --help
echo "https://example.com" | ./target/release/w2a-proxy.exe --url-stdin   # 看握手 JSON
```

## 兼容契约探针（`pnpm run probe`）

`src/main/probe.ts` 自己起一个 `127.0.0.1` 随机端口的夹具站点，逐个导航并断言 **112 条**契约，
**退出码即结论**（不通过则非 0，可以直接挂 CI）。夹具全部内联在代码里，不落盘、不进打包产物。

覆盖范围：启动页内联注入、标题栏几何与配色、38px 让位（fixed / sticky / `100vh` / `inset:0` 四种页面形态）、
严格 CSP 站点的样式降级、加载层淡出、`target="_blank"` 与 `window.open` 的原地导航、
最小化 / 最大化 / 关闭三个按钮真的驱动了原生窗口，以及 **sidecar 的代理契约**：

* 直连夹具（不带凭据）必须 **401** —— 先证明夹具真在鉴权，否则后面的断言是假通过；
* 经代理必须 200，且**上游**收到的 `Authorization` 与 `proxy.rs::build_basic_auth` 的格式逐字节一致；
* **WebSocket 握手也带上了 `Authorization`** —— 这是本地代理当初存在的全部理由
  （WebView2 的原生鉴权事件对 WS 不生效），也是「Electron 复用同一份 Rust 代理」值不值的唯一依据；
* WebSocket **双向**通信打通（上游收到的客户端消息 + 页面收到的服务端消息）；
* `Origin` 被改写为**上游源**（且只有一个）—— 详见下面那条已修的坑；
* `stop()` 之后 sidecar 进程**真的退出**（`kill(pid, 0)` 探活），不留孤儿。

M3 之后还覆盖**系统集成**（48 条，即第 11–15 节）：

* `Hotkey`（旧版存 JS `event.code`）→ Electron accelerator 的映射，含「不认识的键名返回 null，绝不猜」；
* `settings.json` 的**序列化字节形态**必须与 `serde_json` 写出来的一致（字段名 / 嵌套 / 布尔类型）——
  这是「与 Tauri 版可互相读取」的硬保证，反向由 `webview-app` 侧的
  `settings_parse_accepts_electron_written_json` 盯着；
* 存储位置规则：`data\` 无同名文件时写向 `%APPDATA%\<app_name>\`，读取候选同时覆盖两处；
* 窗口几何的**DIP ↔ 物理像素**换算（跨 DPI 不错位）与四种恢复策略（含「位置在屏外 → 最大化」）；
* 托盘：启用后图标出现、关闭后图标销毁；
* 全局快捷键：注册 / 不支持的键名明确报错 / 清空注销 三条路径；
* 设置窗：`settings://localhost/` 能打开、垫片就位、**点保存的完整链路**（页面 → IPC → 副作用 → 落盘）、
  未授权命令被拒、能关闭；
* 「关闭到托盘」：关窗后窗口未销毁且被隐藏，再切换能恢复。

> ⚠️ 探针**不碰用户真实设置**：它把 `settings.json` / `window-state.json` 整体重定向到
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
1. 占位符被包在 `/* … */` 注释里，而 `titlebar.js` 内含 29 处 `*/` —— 注释会**提前闭合**，
   靠脚本第 1 行是 `/*`、首个 `*/` 恰好出现在第 70 行才侥幸没炸。现已改为**裸 token**
  （`<script>__WEB2APP_TITLEBAR_JS__</script>`），并有 `splash_script_tokens_are_not_comment_wrapped` 等测试。
2. 替换必须用 `split/join`，**绝不能用 `String.replace`** —— 替换串（即整个 JS 源码）里的
   `$&`、`$'` 会被当作替换模式，直接把脚本改坏。

## 验证清单（改了什么就必跑什么）

| 你改了什么 | 必跑 | 为什么 |
|---|---|---|
| `titlebar.js` / `loading.js` / `splash.html`（**两壳共用的单一真源**） | `probe` | 这两个脚本是「错了也静默」的重灾区：顶部栏没挂上、38px 没让位，界面看起来都「差不多」 |
| `proxy.rs` / `resolve.rs`、sidecar 契约 | `probe`（内含 sidecar 三连断言）+ `cargo test` | 代理是两壳共用的 Rust 源码，改一处两个壳一起变 |
| 设置 / 托盘 / 快捷键 / 窗口几何链路 | `probe` | 探针走的是与设置窗**完全相同**的那批函数，不是复刻实现 |
| `config.ts`、路径装配（`portable.ts`） | `smoke` | 绿色版路径与配置解析全是「错了也静默」，`smoke` 会把生效路径打出来 |
| 启动日志（`startup-log.ts`）、`data\logs` 落点 | `smoke` | 「日志没写出来」同样是静默故障，而它是用户侧唯一的证据来源；`smoke` 断言文件已含 `[web2app] 启动信息` |
| 打包配置 / 出包前后 | `probe` + 在**真机**双击产物 | 打包产物没有 stdout，只能靠窗口标题、`data\` 与缓存取证 |
| 任何提交前 | `typecheck`（`build` 由 `pack`/`dist` 自动跑） | `typecheck` **不能**代替 `build`，见「打包」表第 2 行 |

产物侧的 `--probe` 也可以跑（包内自带探针），但**必须**是
`Start-Process -Wait -PassThru` 取 `$p.ExitCode` —— 直接 `& app.exe --probe` 对 GUI 子系统 exe
是**假绿**（PowerShell 不等待，退出码立刻为 0）。

## 变更记录

| 版本 | 日期 | 变更说明 |
|---|---|---|
| 0.1.0 | 2026-09-17 | M6 出包打通（dir / zip / portable 三形态）；修两个真 bug：`createWindow()` 漏传 `title`、`tsc` 有错也输出导致半成品封进 asar |
| 0.1.0 | 2026-09-17 | 滚动条越界：从 `width:100vw` → `scrollbar-width:none` → 最终**换滚动根**（`html` 不滚、`body` 滚）；探针判据改成成对的「不侵入 + 仍存在」 |
| 0.1.0 | 2026-09-17 | 文档与配置订正：①`config.example.json` 补全到契约的 17 个字段（原先缺 `icon_url` / `proxy_https` / `resolve_secret` 及两个 Android 密码字段）；②移除进包的死资源 `shell.html`；③`.gitignore` 补 `dist/`、`dist-*/`；④「无 GPU 环境」说明改为与实测一致（必须加 `--no-sandbox --in-process-gpu`） |
| 0.1.0 | 2026-09-17 | 健壮性补强：①`whenReady` 之后加顶层兜底，任何 await 抛错都收敛成**非 0 退出码**（此前只是 `UnhandledPromiseRejectionWarning`，既不报结论也不退出，CI 里表现为挂死）；②保存设置失败时**回滚已生效的唤醒快捷键**，兑现「任一步失败都不写内存态」 |
| 0.1.0 | 2026-09-18 | **M6 收尾：启动日志落盘** —— 新增 `src/main/startup-log.ts`（零 Electron 依赖、fail-open、1 MiB 轮转），装配到 `data\logs\startup.log`，探针模式下重定向到 `%TEMP%\w2a-probe-store\` 以免污染真实目录；`smoke` 增加「日志已含启动信息」硬断言；`logStartup()` 打印日志路径。负向对照：`ENOTDIR` / `EISDIR` 两种写失败均 `ok=false` 且不抛异常 |
| 0.1.0 | 2026-09-18 | 审计修复：①`renderSplash()` 的失败态注入由 `String.replace` 改 `split/join` —— 原写法把含失败 URL 的文案当**替换串**解释，`$&` / `` $` `` / `$'` 会截断启动页脚本（该约束本就写在同文件注释里，属自食其言）；②`config.ts` 里「`tabs_enabled` M5 实现前仅记录、不生效」的过期注释订正；③`index.ts` 两处逐字重复的解析路径文案表抽成 `howLabel()`；④`tabs.ts` 的 `contentBounds()` 高度钳零（窗口被压到比标签栏还矮时会让 `setBounds` 收到负高度） |
| 0.1.0 | 2026-09-20 | 文档与仓库卫生审计：①源码/构建配置里 10 处失效引用 `PLAN.md` → `docs/electron-plan.md`（该文件 2026-09-17 已归位）；②注释里 `npm run *` 统一为 `pnpm run *`（包管理器已迁移，`electron-builder.yml` 同步）；③`.gitignore` 补 `tests/*.log`、`tests/*.exe`、`tests/*.zip` —— 原 `tests/*/` 只忽略目录，`tests/` 下一份 ~96MB 便携版副本与 10 个探针日志会被随仓库提交（M7 纳入版本控制前的必要前置）；④README 补 `docs/electron-size-reduction.md` 与根 `README.md` 的交叉引用，并声明「里程碑状态以本 README 为准」 |

