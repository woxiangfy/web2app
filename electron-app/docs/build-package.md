# 打包与包管理（electron-app）

> 自 `AGENTS.md` 拆出：pnpm 与镜像、打包坑、手动打包 workflow、产物核对、绿色版形态、SmartScreen。
> 命令速查表见 `AGENTS.md`「命令」；配置字段契约见 `../docs/packaging-config.md`。

## 包管理器：pnpm（2026-09-17 迁移）

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

⚠️ `start` / `smoke` / `probe` 三个命令都经 `scripts/launch.cjs` 拉起，**不要直接 `npx electron .`** ——
原因见 [probe-sidecar.md](probe-sidecar.md)。
（`shot` / `lab` / `diag` 各自 `spawn` Electron，脚本内自行清理 `ELECTRON_RUN_AS_NODE`。）

## 打包（`pack` / `dist`）常见故障

| 现象 | 原因 | 处理 |
|---|---|---|
| `Invalid configuration object … unknown property 'zip'` | electron-builder **26 起移除了顶层 `zip:` 配置块**（`TargetConfiguration` 只认 `target`/`arch`） | 产物名改用顶层 `artifactName` 模板；已在 `electron-builder.yml` 注明 |
| 双击 exe 弹「A JavaScript error occurred in the main process: xxx is not defined」 | tsc **默认「有错也照样输出」**，`out/` 里留下改了一半的 JS，之后只跑 `typecheck` 没重跑 `build`，打包就把坏产物封进 asar | `tsconfig` 已开 `noEmitOnError: true`（编译失败时 `out/` 保持原样），`pack`/`dist` 里也是先 `pnpm run build` 再打包 |
| 目录里已有 `release/` 时构建报删除失败 | 环境可能拦截删除（如 agent 沙箱的 safe-delete 策略） | 换输出目录：`npx electron-builder --win -c.directories.output=<新目录>` |
| 单文件版双击后是空白官网/`example.com` | 包内 **`app.asar` 里没有 `out/config.json`**（打包前漏了 `pnpm run stage` / 漏了 `pnpm run build`） | 配置已随 asar 进包，**不再**支持放 exe 旁边覆盖；重跑 `build` + `stage` 再打包。CI 侧由 `scripts/check-asar-config.cjs` 硬断言兜底 |
| **改了注入脚本、重新打包后「现象依旧」** | 双击的是**旧产物**：`release/` 下 zip / portable / 解压目录多份并存，旧文件不会被自动清理 | 见下节「先确认跑的是哪一版包」——先核对 sha256 与运行痕迹 mtime，再动代码 |

### 手动打包（推荐为每个应用出一份包）

本地 `pnpm run pack` / `pnpm run dist` 出的是**当前 `config.json`**的包，适合开发调试。
要为某个具体应用出一份正式包，走 GitHub Actions 的
**`自定义打包（Electron 绿色版 / 安装版）`**（`.github/workflows/electron-build-custom.yml`）：

1. Actions → 该 workflow → **Run workflow**；
2. 粘贴一份配置 JSON（或填一个返回 JSON 的 URL）—— 与 Tauri 版**同一份契约、同一个解析器**；
3. 勾选要打的形态：`electron_zip`（解压即用）/ `electron_portable`（单文件 exe）/
   `electron_nsis`（Windows 安装向导 setup exe）。

**本地出安装版**：`pnpm run dist:installer`（与
`node ../scripts/pack-local.mjs --config <配置> --target electron-nsis` 语义等价；两条入口产物
目录不同——前者落 `tests/pack/`，后者按 pack-local 约定落 `tests/pack/electron/`，两处都被根 `.gitignore` 锚定忽略）。
⚠️ 安装版必须独立一次构建：它带 `distribution=nsis` 标记，与 zip/portable 同次构建会把绿色产物
一起误标成安装版。安装版运行时**强制使用系统默认数据目录** `%APPDATA%\<app_name>`，
忽略配置的 `data_dir`（由 `src/main/distribution.ts` 读取标记、`portable.ts` 执行强制）。

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

## 两种绿色版形态（同一份代码，自动区分）

| 形态 | 产物 | `installRoot()` 依据 | 特点 |
|---|---|---|---|
| 解压目录版 | `Web2App-<ver>-win-x64.zip` | `dirname(app.getPath('exe'))` | 启动快，体积最小（压缩后 ~110–150MB） |
| 单文件自解压版 | `Web2App-<ver>-win-x64-portable.exe` | `PORTABLE_EXECUTABLE_DIR`（electron-builder 注入） | 一个文件带走；⚠️ 启动时解压运行时到 `%TEMP%`，冷启动明显变慢 |

⚠️ 单文件模式下 **`app.getPath('exe')` 指向 `%TEMP%` 里的解压副本**，不是用户放置 exe 的位置。
所有「exe 同级」的定位都必须走 `installRoot()`，不要直接写 `app.getPath('exe')`。

## 首次运行与 SmartScreen（产物未签名）

产物**没有代码签名**。从浏览器/IM 拿到的 zip 与 exe 会被标记「来自 Internet」，
首次双击时 Windows 会弹**「Windows 已保护你的电脑」**（蓝色 SmartScreen 提示），
必须点「更多信息」→「仍要运行」才能启动。

* 这是**预期行为**，既不是打包失败也不是病毒：`signing/` 下那份证书只服务 Tauri 侧。
* 想根治只有两条路——买代码签名证书并配 `win.certificateFile`，或用组织内的白名单 / 域策略放行。
  ⛔ **不要**教用户关掉 SmartScreen（那是全局降安全，代价远大于收益）。
* 交付话术建议照抄上一段，并附上日志的位置（⚠️ `data\logs\startup.log` **只在 `debug` 打开时才有**；
  默认关闭时让用户临时用 `--debug` 复现一次再收日志 —— 见 [runtime-data.md](runtime-data.md)）：
  「点了没反应」十有八九是被 SmartScreen 拦在门外，而不是程序本身有问题。
