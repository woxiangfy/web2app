# 本地脚本（scripts/）说明

> `scripts/` 下的脚本均为**零第三方依赖**的 Node 脚本（Node ≥ 18），**不进 CI** ——
> CI 侧对应能力由 `.github/workflows/` 与 `.github/scripts/` 承担。本文是两个入口脚本的权威说明；
> 打包配置 JSON 契约的唯一文档是 [packaging-config.md](packaging-config.md)（pack-local 详见其 §7.2）。

## init-env.mjs —— 环境初始化

一条命令把两个壳的依赖拉齐。用法：

```bash
node scripts/init-env.mjs                 # 两侧都初始化（cargo fetch + pnpm install + tauri-cli）
node scripts/init-env.mjs --webview       # 只初始化 webview-app（cargo fetch + tauri-cli）
node scripts/init-env.mjs --electron      # 只初始化 electron-app（pnpm）
node scripts/init-env.mjs --update        # cargo 先 update 再 fetch（会改写 Cargo.lock）
node scripts/init-env.mjs --all-targets   # cargo 拉全平台依赖（默认只拉宿主平台；Android 交叉编译前用）
node scripts/init-env.mjs --frozen        # pnpm install --frozen-lockfile（CI 语义，lockfile 不符即失败）
node scripts/init-env.mjs --no-mirror     # 不注入国内镜像（境外网络 / 直连 GitHub 更快时）
node scripts/init-env.mjs --no-tauri-cli  # 不自动装 tauri-cli（默认缺失自动装，首次约几分钟）
node scripts/init-env.mjs --check         # 只检查工具链与当前状态，不下载、不安装
```

退出码：`0` = 选中的全部成功（`--check` 下无缺失）；`1` = 有失败/缺失；`2` = 参数错误。

### webview-app 侧（cargo）

- 默认 `cargo fetch --manifest-path webview-app/Cargo.toml --target <host>`：按 Cargo.lock 下载 crate，
  但**只拉当前宿主平台**（target triple 由 `rustc -vV` 的 `host:` 行解析）。
  ⚠️ `cargo fetch` 不带 `--target` 时会拉**全平台**依赖 —— 本工程依赖树里含 gtk / webkit2gtk（Linux）、
  objc2（macOS）、ndk（Android）等大量与本机无关的 crate，墙内网络下纯属浪费；
  要准备 Android 交叉编译时用 `--all-targets`（或 `cargo fetch --target <android-triple>`）。
- `--update` 先跑 `cargo update`（Cargo.lock 是受版本控制的文件，会改动它，git diff 可见，自行决定是否提交）。
- tauri-cli 在 fetch 成功后**默认自动安装**：`cargo install tauri-cli --version "^2"`（与 CI 同一步）。
  它是本地打 Tauri 包（`pack-local.mjs --target tauri-*`）的硬依赖，普通 `cargo build` / sidecar
  编译不需要；`--no-tauri-cli` 可跳过（打包前再手动装），`--check` 永远只检查不安装。
  装完用 `cargo tauri --version` 复验（判据与 pack-local 一致）。

### electron-app 侧（pnpm）

- `pnpm install`，并**显式注入两个镜像环境变量**（默认开，`--no-mirror` 关）：
  `ELECTRON_MIRROR`（装 electron ~158MB 平台二进制）与
  `ELECTRON_BUILDER_BINARIES_MIRROR`（electron-builder 的 winCodeSign / nsis 等工具包）。
  ⚠️ 原因：pnpm **不会**把 `.npmrc` 的自定义键透传给依赖的 install 脚本，install.js 只认环境变量；
  不设就去连 GitHub Releases，国内表现为**卡住不返回**（不是报错）。细节见
  `electron-app/.npmrc` 与 `pnpm-workspace.yaml` 的文件头注释。
- 装完只认一个结果：`node_modules/electron/dist/electron.exe` 是否存在（退出码 0 不代表二进制装上了）。
  缺失时自动跑 `node node_modules/electron/install.js` 兜底（实测约 3 秒）。

### 输出与失败排查（两侧共用）

每个子进程的输出实时上屏，同时 tee 一份完整原文到系统临时目录日志（路径开头打印）；
失败时在最终汇总处**重放该命令输出的尾部** —— cargo/pnpm 的安装进度有几百行，真正的错误
（下载失败 / TLS / 编译错误）会被顶出可视区，曾导致「只知道失败、看不到原因」。

### 为什么是 Node

与 `pack-local.mjs` 同理：弄 electron-app 必然已有 node（electron-builder、`pnpm run *` 全是 node
程序），node 是「拿这个仓库做开发」的必要条件；且在 PowerShell 之外保留一份 macOS/Linux 也能跑的入口。

## pack-local.mjs —— 本地打包（与 CI 同语义）

**入口薄壳**，实现拆在同目录 `packlib/` 的 10 个模块：
`constants` / `diagnostic` / `parser` / `pycompat` / `util` / `paths` / `driver-electron` /
`driver-tauri` / `selftest` / `parity`。

读一份配置 JSON，在本机跑出与 CI **同语义**的产物（Tauri MSI/单文件；Electron zip/portable/NSIS
安装版），产物统一落仓库根 `tests/pack/{tauri,electron}/`（不进版本库，根 `.gitignore` 锚定）。

⚠️ 它是 CI 解析器（`.github/scripts/parse-packaging-config.py`）的 **Node 复刻** ——
Python 不是必然存在的依赖；两侧靠 `--parity` 逐项比对钉住等价。

> 用法、目标矩阵、参数与配置字段语义的**唯一权威文档**见
> [packaging-config.md](packaging-config.md) §7.2（及全文的字段契约），本文不重复维护。
