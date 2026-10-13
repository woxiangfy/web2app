# Electron 打包产物瘦身方案（web2app Electron 绿色版实测基线）

> 基线日期：2026-09-18 · electron 44.4.1 · electron-builder 26.15.3 · win x64
> 所有「实测」数字取自本机 `electron-app/release/` 与 `node_modules/electron/dist/`；标「估」的为按实测压缩比推算。
> ⚠️ 量体积前必须在**干净产物**上量：本仓 `release/Web2App-0.1.0-win-x64/` 内嵌了 `data\`（**18.22MB**，跑过 exe 的会话缓存），同级 `release\data\` 另有一份 44.99MB 残留；不清掉会误判基线。
> 自洽校验：干净目录 **371.66MB** ≈ 原始运行时 367.67MB + 自有载荷（`app.asar` 0.20 + sidecar 3.58 + inject 0.09）≈ 371.5MB ✅

## 1. 先量体积：谁才是真正的大头

| 对象 | 体积 | 占比 | 性质 |
|---|---|---|---|
| 原始运行时 `node_modules/electron/dist/` | 367.67 MB | — | 参考值（未打包） |
| └ `electron.exe` | **234.91 MB** | 全量 64% | 96.6% 是代码+只读数据，**不可裁**；压缩路线经实测不可用 ⇒ 见 **附录 A** |
| └ `locales/*.pak`（55 个） | 48.30 MB | 13% | ✅ 可裁 |
| └ `dxcompiler.dll` + `dxil.dll` | 25.99 MB | 7% | ⚠️ 有条件下可裁（WebGPU） |
| └ `LICENSES.chromium.html` | 19.52 MB | 5% | ❌ 不建议裁（合规） |
| └ `resources.pak` | 11.86 MB | 3% | ❌ 不可裁 |
| └ `icudtl.dat` | 10.37 MB | 3% | ❌ 不可裁 |
| └ `vk_swiftshader.dll` + `vulkan-1.dll` | 6.16 MB | 2% | ⚠️ 有条件下可裁（软件渲染） |
| └ `d3dcompiler_47.dll` | 4.52 MB | 1% | ⚠️ ANGLE D3D11 依赖，删前须实测 |
| └ `ffmpeg.dll` / `chrome_200_percent.pak` | 2.97 / 1.21 MB | 1% | 前者不可裁；后者可裁 |
| 自有载荷 `app.asar` | **0.20 MB** | ≈0 | 已无可压 |
| 自有载荷 `resources/sidecar/w2a-proxy.exe` | 3.58 MB | 1% | 已 `lto+strip`（见 §5.3） |

**结论：本项目的应用代码只有 0.20MB，体积 99% 是 Chromium 运行时。** 因此：
- 对 `app.asar` 做任何压缩/minify/摇树，收益是「0.2MB 的百分之几」= 噪声；
- `asar: true` 是**保护机制不是压缩器**（asar 不做压缩、还有对齐开销），别指望它瘦身；
- 瘦身的主战场只有三处：**语言包、几个可选 DLL、压缩算法/产物形态**。

## 2. 三种分发形态实测对比（同一份载荷）

| 形态 | 实测 | 压缩方式 | 用户成本 | 评价 |
|---|---|---|---|---|
| `dir` 解压目录 | 389.9 MB（含内嵌 `data\` 18.22MB）／净 **371.66 MB** | 无 | 需整目录分发 | 只适合内测 |
| `zip` | **147.6 MB** | deflate（`-mx=7`） | 系统原生可解 | ⚠️ 明显偏胖 |
| `7z`（目标） | 估 **~96–100 MB** | LZMA2（`-mx=9`，可加 BCJ2） | Win11 原生支持；Win10 需装 7-Zip | 下载体积最优但增加门槛 |
| `portable` | **96.5 MB** | NSIS 载荷默认 7z/LZMA | 双击即用，无需解压器 | ✅ **综合最优** |

**关键发现（零成本、零风险）**：`portable` 与 `zip` 是**同一份 appOutDir** 压出来的，`zip` 却胖了 51MB —— 差距全在压缩算法（deflate vs LZMA），不在打包内容。
依据：`NsisTarget.js:509` → `COMPRESSION_METHOD = options.useZip ? "zip" : "7z"`（默认 `7z`），而 archive 目标的 zip 默认只给 `-mx=7`（`targets/archive.js:66-71`）。

⇒ **如果只想要一个下载包，`portable` 已经是压缩最优解，别再花力气"瘦身 zip"；要么承认 zip 是兼容包，要么把 zip 目标换成 `7z` 目标。**

## 3. 方案分级（按「收益/风险」排序）

### P0 — 零风险，建议直接做

| 方案 | 实测/估算收益 | 落地方式 |
|---|---|---|
| ① 裁语言包 | dir **−46.07 MB**；portable **−11.1 MB**（估）；zip −13.6 MB（估） | `electronLanguages: ["en-US","zh-CN","zh-TW"]`（见 §4 语义陷阱） |
| ② 出包卫生：清掉交付目录里内嵌的 `data\` | dir **−18.22 MB**（同级 `release\data\` 另有 −44.99MB 残留） | 出包前删除；探针/冒烟**禁止**在交付目录跑（会在 exe 同级重建 `data\session`） |
| ③ 清掉残缺的 `release/win-unpacked`（只剩 0.3MB） | 避免误把残缺目录当产物 | electron-builder 中途 EBUSY 失败会留残缺目录，重跑前先确认 |
| ④ zip 目标 → 7z 目标（或改推 portable） | **−48 MB 左右**（估） | `win.target` 加 `7z`；`targets/targetFactory.js:10` 明确支持 `7z` |
| ⑤ `compression: maximum` | zip 路径可再省几个百分点（未实测） | 顶层 `compression: maximum`（zip 会加 `-mfb=258 -mpass=15`，7z 本来就 `-mx=9`） |

`electronLanguages` 的实测压缩收益推导：`en-US.pak + zh-CN.pak` 原始 1.12MB → 7z 后 0.27MB（**4.15:1**），故 46.07MB 待裁语言包 ≈ 11.1MB 压缩后体积。

### P1 — 有收益，但**必须先实测目标环境**再删

| 文件 | 未压缩 | 删掉的代价 | 判据（删完必须过） |
|---|---|---|---|
| `dxcompiler.dll` + `dxil.dll` | 25.99 MB | WebGPU/HLSL shader 编译失效（Dawn 用 DXC） | 站点若用 WebGPU 就**别删**；本项目是通用 WebView 壳 ⇒ 默认不删 |
| `vk_swiftshader.dll` + `vulkan-1.dll` | 6.16 MB | 无 GPU 环境（虚拟机/远程桌面/沙箱）失去软件 Vulkan 回退 ⇒ 可能白屏 | ⚠️ 本项目探针就在**无 GPU 沙箱**里跑（`--in-process-gpu`），删前先在目标机实测 |
| `d3dcompiler_47.dll` | 4.52 MB | ANGLE D3D11 后端运行时编译 HLSL 需要 | 别删（省 4.5MB 换渲染崩，不划算） |
| `chrome_200_percent.pak` | 1.21 MB | 200% DPI 下的图片资源缺失（高 DPI 机器图标模糊/缺失） | 收益太小，建议保留 |
| `default_app.asar` / `elevate.exe` | 0.21 MB | 无 | 收益可忽略，不值得做钩子 |

落地方式（electron-builder 无对应配置项，需要用钩子）：顶层加 `afterPack: scripts/after-pack.cjs`，在钩子里对 `context.appOutDir` 做白名单删除。**钩子必须 fail-open 且打印日志**（删失败只告警、不阻断出包），并只删**明确列在白名单里**的文件名，禁止通配删除。

### P2 — 明确不做

| 方案 | 为什么不做 |
|---|---|
| 删 `LICENSES.chromium.html`（19.52MB） | ① **实测它在 7z 里只有 0.18MB（108:1）**——文本压缩率极高，删它对**下载包几乎零收益**，只在解压目录上好看；② Chromium 许可要求随分发提供许可文本，删了有合规风险。**结论：保留。** |
| 用 UPX 压 `Web2App.exe` | 234.91MB 里主要是 V8 快照与 Chromium 代码，UPX 后启动变慢且**极易被杀软误报**，绿色版口碑代价太大 |
| 删 `icudtl.dat` / `resources.pak` / `ffmpeg.dll` | 运行时强依赖（i18n / 资源 / 媒体解码），删了直接起不来或媒体全废 |
| 指望 `asar` 压缩 / 压 minify 自研代码 | 见 §1：自有载荷 0.20MB，收益为噪声 |

### P3 — 想再往下走，只能换地基

| 方案 | 量级 | 代价 |
|---|---|---|
| 自定义编译 Electron（GN flags 裁 WebRTC/PDF/打印/非必要 codec） | 可再省 30–50MB | 维护成本高，每次升 Electron 都要重来；本项目已在维护 Tauri 双壳，不建议 |
| 换壳：Tauri 2（本仓已有 `webview-app/`） | 产物 5–15MB 量级 | 依赖系统 WebView2；本项目双壳并存正是为此 |
| `sidecar` 用 UPX 压 | 3.58 → ~1.2MB（估） | ⚠️ 它是被 `spawn` 的子进程，UPX 后杀软误报风险同样存在 |

## 4. ⚠️ `electronLanguages` 的语义陷阱（已读源码确认）

出处：`app-builder-lib/out/electron/ElectronFramework.js:58-101`

1. **生效位置**：平台级优先，顶层兜底 —— `platformSpecificBuildOptions.electronLanguages || config.electronLanguages`。electron-builder ≥24.2.0 起支持 Windows/Linux（早期版本仅 mac）。
2. **作用对象**：非 mac 平台删的是 `getResourcesDir(appOutDir)/../locales/*.pak`。
3. **匹配规则**（**这里最容易写错**）：
   ```js
   wanted === language || wanted.startsWith(`${language}-`) || wanted.startsWith(`${language}_`)
   ```
   `language` 取的是**文件名去掉扩展名后的小写全名**（Windows 上是 `en-us`、`zh-cn`）。
   - ✅ 写全名：`"en-US"` → 命中 `en-US.pak`
   - ❌ 写 `"zh"` → **一个都命中不了**（`"zh".startsWith("zh-cn-") === false`），
   - ⚠️ 前缀规则只对 mac 的 `en.lproj` 这类短目录名（wanted=`en-US`）有效。
   - **必须显式列出 `zh-CN` 与 `zh-TW`**（只有 `zh-CN` 时繁体界面会回落英文）。
4. **失败模式是安全的**：一条都没删成功时会打 `warn: no locales found matching wanted languages, skipping cleanup` 然后跳过 —— 不会把语言包删空。
5. **时机**：该清理挂在 `beforeCopyExtraFiles` 里，**早于 archive/target 产出** ⇒ 压缩包同步变小（不只是解压目录变小）。

推荐值（简中为主 + 英文兜底 + 繁体）：
```yaml
electronLanguages:
  - en-US
  - zh-CN
  - zh-TW
```
实测保留这 3 个 = 1.68 MB（若再加 `en-GB` 为 2.23 MB），其余 46.07 MB 全部可省。

## 5. 本项目建议落地的 yml 组合

### 5.1 直接抄（P0 组合）

```yaml
# electron-builder.yml
compression: maximum          # zip 路径：-mx=9 + -mfb=258 -mpass=15（7z 本来就 -mx=9）

electronLanguages:            # 顶层；win.electronLanguages 亦可
  - en-US
  - zh-CN
  - zh-TW

win:
  target:
    - target: dir
      arch: [x64]
    - target: 7z              # zip → 7z；如不想要 7-Zip 门槛，可保留 zip 只作兼容包
      arch: [x64]
    - target: portable
      arch: [x64]

7z:
  artifactName: ${productName}-${version}-win-${arch}.${ext}
```

可选环境变量（`targets/archive.js`）：
- `ELECTRON_BUILDER_7Z_FILTER=BCJ2` —— 允许的过滤器：`BCJ/BCJ2/ARM/ARMT/IA64/PPC/SPARC/DELTA`；x86 二进制用 BCJ2 通常再省几个百分点。
- `ELECTRON_BUILDER_COMPRESSION_LEVEL=<0-9>` —— 覆盖配置里的压缩级别。

### 5.2 预期效果汇总

| 产物 | 现在（实测） | 只做 P0（估） | P0 + P1（激进，估） |
|---|---|---|---|
| `dir` | 371.66 MB（净） | ~325.6 MB | ~292 MB |
| `zip` | 147.6 MB | ~134 MB | — |
| `7z` / `portable` | 96.5 MB | ~85.4 MB | ~75 MB |

**天花板提醒**：`Web2App.exe` 单个 234.91MB，其中 96.6% 是机器码与只读数据、且官方已 strip ⇒ **剪不掉**；压缩路线（UPX）经三变体实测全部崩溃 ⇒ **压不了**（详见 **附录 A**）。干净解压目录是 371.66MB（运行时占 367.67MB），再怎么优化也只是「把 Chromium 之外那几 MB 自有载荷 + 可选 DLL 剪掉」。**任何宣称「Electron 绿色版压到几十 MB」的说法，都是换了壳（把 Chromium 成本转给操作系统）或没算运行时。**

### 5.3 已做过的（不用再重复劳动）

- `files: [out/**/*, package.json]` 白名单已收紧；`package.json` 无 `dependencies` ⇒ 整个 `node_modules` 不进包（`app.asar` 仅 0.20MB 即证据）。
- `asar: true`：注入脚本（titlebar.js / loading.js / shell.html）与 `dist/settings.html`
  自 v1.35 起铺到 **`out/inject/` 并随 `app.asar` 分发**（2026-09-27 订正：旧写法「走
  resources」已失效，asar 外不再有注入资源）；图标与 sidecar 仍是 `extraResources` ——
  sidecar 必须在 asar 外，因为 asar 内 exe 无法 `spawn`。
- sidecar `w2a-proxy.exe` 已 `opt-level=3 + lto + codegen-units=1 + strip`（见
  `webview-app/Cargo.toml` 的 `[profile.release]`，符号名为主、行号会漂移；2026-09-27
  实测位于 94–101 行）⇒ 3.58MB 已是偏优结果，不再调 profile。
- 只出 x64 单架构、无多余 target。

## 6. 验证清单（改完必跑）

1. **尺寸判据**（量之前先确认没有 `data\` 污染）：
   `release/Web2App-<ver>-win-x64/locales/` 应只剩 3 个 `.pak`；干净目录应从 371.66MB 降到 ≈325.6MB。
2. **功能判据（回归门禁）**：`pnpm run probe`（设计基线 **138** 条 @ 2026-09-27，**v1.76 起 150** 条；条数随探针演进，以实测输出为准）与 `pnpm run smoke` 必须全绿 —— 裁语言包会影响 Chromium 的 i18n 初始化，**必须实测而不是"应该没事"**。
3. ⚠️ **探针必须在 `tests/` 下的副本里跑**，不要在交付目录里跑（会在 exe 同级重建 `data\session`，把刚瘦下来的目录又撑胖 18MB）。
4. ⚠️ 打包 exe 走 `--probe` 前记得 `Remove-Item Env:ELECTRON_RUN_AS_NODE`，否则退出码 9（0x80000003）会被误判成业务崩溃。
5. 出包后交付目录里除保留的 3 个 `.pak` 语言包外，`data\`、`logs\` 等运行时残留都不应存在。

## 7. 一句话优先级

**先做 P0①（语言包）+ P0②（清 `data\` 卫生）+ P0④（zip→7z 或主推 portable）**，这三项零风险且覆盖了 90% 的可达收益；DLL 类（P1）除非目标环境实测通过，否则一律不动；`LICENSES.chromium.html` 保留。**exe 本体（234.91MB）不要在这上面花时间** —— 三条路线都实测/论证过了，见附录 A。

## 附录 A：`Web2App.exe`（234.91MB）到底能不能动？（2026-09-18 实测）

### A.1 先看它是什么：PE 节表实测

| 节 | 文件内占用 | 说明 | 能否删 |
|---|---|---|---|
| `.text` | 183.66 MB | x86-64 机器码（V8 / Blink / Skia / Node） | ❌ |
| `.rdata` | 43.32 MB | 只读数据（字符串、常量、vtable） | ❌ |
| `.pdata` | 5.33 MB | 异常/栈展开表，加载器必需 | ❌ |
| `.reloc` | 1.17 MB | 重定位表（ASLR 必需） | ❌ |
| `.data` | 1.32 MB | 可写数据 | ❌ |
| `.rsrc` | 0.10 MB | 图标 / VERSIONINFO | ❌ |
| 其余 9 个近空节 | ≈0.01 MB | 上游构建产物（`CPADinfo`/`LZMADEC`/`.eh_fram`/`prot`/`malloc_h` 等） | ❌ |

⇒ **96.6% 是代码 + 只读数据，且官方构建已 `strip`**（无 `.debug*`/符号节，`.pdb` 只剩一个路径字符串）⇒ 没有任何「删掉就变小」的东西：**剪不掉，只能压，或重新编译。**
（对照组已验证：`node_modules/electron/dist/electron.exe` 与打包后的 `Web2App.exe` 节表完全一致 ⇒ 这不是本项目配置造成的。）

### A.2 路线 1：UPX 压缩 —— 三变体全崩，**不可用**

| 变体 | 压缩结果 | 运行 `--probe` 结果 |
|---|---|---|
| `upx --lzma -9`（默认参数） | ❌ **被 UPX 拒绝**：`CantPackException: superfluous data between sections (try --force)` | — |
| `--force --lzma -9`（strip-relocs 默认开） | 234.91 → **72.95 MB**（31.06%，耗时 115s） | ❌ **ExitCode 0xC0000005**（访问违例，2.0s 即崩） |
| `--force -9`（默认 NRV/UCL 算法） | 234.91 → **83.56 MB**（35.57%） | ❌ 0xC0000005（1.0s） |
| `--force --lzma -9 --strip-relocs=0`（保留重定位表） | 234.91 → 72.95 MB | ❌ 0xC0000005（2.0s） |
| **对照组**：同目录、同参数、**未压缩** | — | ✅ **ExitCode=0**，13.3s，112 条断言通过（**2026-09-24 实验当时的条数**，探针现 134 条；此处是时点记录，勿回改） |

对照组排除了环境因素（不是沙箱/缺 GPU/`ELECTRON_RUN_AS_NODE` 的问题）⇒ **责任在 UPX**。UPX 那句 `(try --force)` 本身就是"我知道这文件结构不正常，你坚持我就硬来"，硬来的结果就是镜像起不来。同类已知问题：upx#141（Electron exe 压缩后 Win7 `APPCRASH`）、upx#83（压缩后 `0xc0000005`）。

**即使压成功也不该用**（UPX 官方手册 + Electron 架构冲突）：
1. ⚠️ **进程间不再共享内存**：UPX 手册原文「packed programs do not share RAM at runtime in the way that executables mapped from a file system do」⇒ Electron 是**多进程**架构（主进程 / 渲染 / GPU / utility 共用同一个 exe），压缩后每个进程各自解压一份 235MB 到私有内存：磁盘省 162MB，**运行内存可能涨几百 MB**。
2. ⚠️ **启动变慢**：UPX 官方口径 LZMA 解压「7–9 秒/GiB」⇒ 235MB ≈ **每次启动 +2 秒**（与实测崩溃前 1.0–2.0s 吻合）。
3. ⚠️ **杀软误报**：UPX 是典型加壳特征，绿色版"干净"的卖点会被误报抵消。
4. **工具链不支持**：electron-builder 26.15.3 **没有** `upx` 配置项（已 grep 全库确认），只能自己写 `afterPack` 钩子。

### A.3 路线 2：自编译 Electron —— 官方支持，但工程上不经济

- 官方入口：`@electron/build-tools`（`e init --root=… --bootstrap testing` → `e sync` → `e build`），详见 Electron 官方 Build Instructions；适用于「为了产出定制 Electron 二进制」。
- 原理：用 Chromium 的 GN 参数把不要的模块**根本不编进去**（`is_debug=false`、`symbol_level=0`、`is_component_build`、`enable_pdf` / `enable_printing` / `proprietary_codecs` / `enable_webrtc` / `v8_enable_i18n_support` 等——**具体开关以目标版本 args 文件为准，本仓未实测**）。
- 代价：depot_tools + 完整 Chromium 源码、数百 GB 磁盘、以小时计的构建；**每次升 Electron 都要重做一遍**，并且由自己承担 triage。
- 社区量级参考（**非本仓实测**，来自社区文章、Electron 15 时代估算）：exe 85MB → 65MB 一档（约 −24%）。换算到 234.91MB 的 44.4.1 会是多少，**没有可靠的公开数据**，只能自己编一次才知道。
- ⇒ 判断：**技术可行、工程不经济**。本仓已在维护 Tauri 双壳，不建议再背一套自编译 Chromium 的长期维护成本。

### A.4 路线 3：换壳 —— 唯一"成熟"的体积方案，但换的是地基

| 方案 | 自有产物体积 | 前提 / 代价 |
|---|---|---|
| **Tauri 2（本仓已有 `webview-app/`）** | 实测 `web2app.exe` **11.06 MB**（另有 sidecar `w2a-proxy.exe` 3.58 MB） | 依赖系统 WebView2：本机固定运行时实测 **860.81 MB**（其中 `msedge.dll` 332.14 MB）；`tauri.conf.json` 未配 `webviewInstallMode` ⇒ 走 Tauri 默认 `downloadBootstrapper`，老机器需联网安装 |
| mini-electron / miniblink | 几 MB（`mini-electron.exe` 几百 KB + `nodejs.dll`） | 官方自称实现 ~90% Electron API；**Windows only、Blink 版本落后、单进程多线程架构、session 相关模块缺失、商业支持模式**。本项目依赖 `WebContentsView` 多页签 + session 隔离 + 现代 Web 标准 ⇒ **不适用** |
| Neutralino / Sciter / 直连 WebView2 | 1–10 MB | 都要重写壳层，Web 标准覆盖面比 Chromium 窄 |

### A.5 结论（修正本文件早期"不可动"的表述）

- 精确表述：**在「官方 Electron 运行时 + 标准 electron-builder 管线」内，exe 本体不可裁**（96.6% 是代码，无符号可剥）；**压缩路线（UPX）经三变体实测全部崩溃，不可用**。
- 要真正减小它只有两条：**自编译 Electron**（可行但不经济）或**换掉 Chromium 依赖本身**——而"换壳"的实质是**把 Chromium 的成本转移给操作系统**（Tauri 自有 11MB ↔ 系统 WebView2 860MB），不是消灭它。
- **对当前决策无影响**：P0 三项仍是收益最大、风险最低的动作；下载体积的收益由形态与压缩层承担（§2：zip 147.6MB → portable/7z ≈96MB）。
