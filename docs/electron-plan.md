# electron-app 落地方案（修订版 · 决策已锁定）

> 修订日期：2026-09-16 ｜ 现状核对：**2026-09-20** ｜ 精简：2026-09-22
> ⚠️ **本文档是「决策与架构记录」，不是当前状态的权威描述。**
> 里程碑状态、命令、契约与判据**一律以 `../electron-app/README.md` 为准**。
> 原稿中的逐日过程记录（各阶段进展、踩坑细节）已精简删除，同类教训均已沉淀到
> `../electron-app/README.md` 与 `.workbuddy/memory/topics/electron-app.md`。

## 决策锁定（2026-09-16）

| 项 | 结论 | 影响 |
|---|---|---|
| 是否做 | **Electron 必做** | 讨论终止，进入实施 |
| 平台 | **仅 Windows** | Electron 不支持移动端的短板消失；Android 继续由 Tauri 版负责 |
| 系统 | **Win10+** | 可用最新 Electron，无 Win7 顾虑 |
| 体积 | **不限** | 不做极致裁剪；asar 保持默认（实测基线见 `electron-size-reduction.md`） |
| 分发 | **只出绿色版**：解压目录版与单文件自解压版**都要** | 打包链简化；放弃自动更新 |
| 数据目录 | **exe 同级 `data\`**（`data_dir="data"`，默认；相对基准 = exe 目录） | 真绿色、整目录可搬走；只读介质自动回退 |
| 代码签名 | **不做** | SmartScreen 引导说明见 `../electron-app/README.md` |
| Tauri 版 | **继续维护，且持续更新** | 双壳长期并行 → 注入脚本单一真源成为硬要求 |
| 多页签 | ~~暂缓~~ → **已完成（M5）** | 选型 A′ = `BrowserWindow` 当壳 + 子 `WebContentsView`，`tabs_enabled` 是生效开关 |

**进度（2026-09-24 核对）**：M0–M7 **全部完成** —— M4（下载接管 / 右键菜单 / F12）与 M7（CI 流水线）于 2026-09-24 交付。（本表是决策记录，**权威状态以 `electron-app/README.md` 为准**。）

---

## 1. 「绿色版」的定义与关键决策

### 1.1 分发形态

- `portable` 目标：自解压单文件 exe，每次启动解压到 `%TEMP%`，数据默认写 **exe 同级
  `data\`**（`data_dir="data"`，2026-09-22 起的默认值；下句「数据默认仍写 `%APPDATA%`」
  是早期方案的历史描述，已失效）。曾因「首次启动慢 / 杀软误报 / 多份 TEMP 残留」犹豫是否
  使用，**最终决定三种形态都出**（`dir` + `zip` + `portable`，M6 已实测）。
- 真绿色的关键：**在 `app.whenReady()` 之前**重定向全部数据路径
  （`userData` / `sessionData` / `logs` / `crashDumps` 四项一起，缺一不可），
  先写文件探测可写性，不可写则回退 `%APPDATA%`（实现：`src/main/portable.ts`，用 `installRoot()` 统一定位）。
- 与 Tauri 版语义对齐：`data_dir=""` 时 Tauri = `%LOCALAPPDATA%\<app_name>`（系统默认程序目录）；
  Electron 版默认 `data_dir="data"` = exe 同级 `data\`（真绿色，**有意不同**，文档需明示）；`""` 时 Electron
  写 `%APPDATA%\<app_name>`。

### 1.2 绿色版依然要处理的非"绿色"残留

- **SmartScreen**：未签名 exe 首次运行弹「Windows 已保护你的电脑」——预期行为，文案见 `electron-app/README.md`；
- **注册表**：Electron 默认不写注册表 → 天然满足；
- **自动更新**：绿色版放弃 `autoUpdater`，人工发新包。

---

## 2. 目标架构

> 设计期草案文件名与实际不符；**实际 `src/main/` 为 16 个文件（2026-09-26 实测）**：
> `config.ts` / `index.ts` / `navigate.ts` / `page-behavior.ts`（M4）/ `portable.ts` / `probe.ts` /
> `settings-window.ts` / `settings.ts` / `shortcuts.ts` / `sidecar.ts` / `startup-log.ts` /
> `tab-order.ts`（页签邻接纯规则）/ `tabs.ts` / `tray.ts` / `web-data.ts`（清缓存）/
> `window-state.ts` —— 以目录实际文件为准（启动参数装配在 `scripts/launch.cjs`，不在 src 下）；
> `src/renderer/` 为 `preload.ts` / `settings-preload.ts` / `shell-preload.ts` /
> `tabs-probe-preload.ts`（4 个）。

- **进程模型**：main（Node，全权）＋ 每个窗口一个 sandboxed renderer ＋ 1 个 Rust sidecar（代理+加密）。
- **技术选型**：TypeScript + `tsc`（TS 7，`module: node16`），设置页/标签栏用原生 HTML（不引框架），与「低依赖」风格一致。

### 2.1 sidecar 是这套方案的核心复用点

- `crates/proxy-server`（bin 名 `w2a-proxy`）：以 `#[path]` 直接引用 `webview-app/src/proxy.rs` 与 `src/resolve.rs`
  —— **零重写、零复制**，两壳永远跑同一份实现。
- 握手：启动后向 **stdout 打一行 JSON 并 flush**（`ok` / `proxyUrl` / `resolvedUrl` / `upstreamHost` / `hadAuth` /
  `authUser` / `authPass` / `pid`）；日志一律走 stderr。
- 生命周期：`will-quit` 里 `kill()`；子进程监视 stdin EOF 自杀（防僵尸三层，见 README「进程间契约」）。
- ⚠️ sidecar **必须走 `extraResources`，不能进 asar**（asar 内 exe 无法 spawn）。

### 2.2 启动序列

```
1. setupPortablePaths()                       ← 必须最先
2. requestSingleInstanceLock()                 ← 第二实例聚焦已有窗口
3. 读 settings.json（失败用默认值）
4. 建无边框窗口 → 立刻加载 splash（本地，零网络）
5. 后台线程：解析真实地址（DNS TXT + Ed25519 验签 + 解密）
6. 后台线程：按 PROXY_ENABLED / PROXY_HTTP / PROXY_HTTPS 决策
      └ 有外部代理 → commandLine.appendSwitch('proxy-server', <复用 build_proxy_args()>)
      └ 否则启动 sidecar 本地反向代理，取回 127.0.0.1:<port>
7. 就绪 → loadURL(代理地址 or 真实地址)
8. 托盘 / 全局快捷键（Alt+4、Shift+F1）/ 页面事件钩子挂载
```

---

## 3. 兼容契约（不逐条对齐，所谓"兼容"就是空话）

| # | 契约项 | 要求 |
|---|---|---|
| 1 | **`config.json` 字段** | 沿用现有 18 字段（2026-09-22 起），不增不改名；`android_*` 四个字段**静默忽略**；⚠️ 布尔**只认真 `bool`**（字符串写法已硬失败，"接受字符串形式"是 2026-09-22 之前的旧行为） |
| 2 | **顶部留白 38px** | 自绘标题栏 + 顶部让位 38px + `applyChromeFit()` 三件事全保留，与 Tauri 版逐像素一致。⚠️ 让位的**现行写法**是「**`html` 不滚、`body` 滚**」：`html{height:100%;overflow:hidden}` + `body{margin-top:38px;height:calc(100vh - 38px);overflow-y:auto}` —— 旧的 `html,body{padding-top:38px}` 会让滚动条越过标题栏，已废弃（见 README「自绘标题栏与页面滚动条」） |
| 3 | **标题栏按钮** | 后退 / 前进 / **设置** / 最小化 / 最大化（双击标题栏最大化）/ 关闭，行为与位置一致（2026-09-23 起「设置」紧挨最小化**左侧**，探针按几何钉住） |
| 4 | **Basic 鉴权** | 覆盖 HTTP **与 WebSocket**；`Set-Cookie` 的 `Domain=真实host` → `127.0.0.1` 重写；`X-Forwarded-Proto` 透传上游 scheme |
| 5 | **新窗口策略** | `target="_blank"` / `window.open()` → 单窗口模式原地导航、多页签模式新开 tab |
| 6 | **快捷键** | `Alt+4` 显隐（可在设置改）、`Shift+F1` 打开设置（固定） |
| 7 | **托盘语义** | 左键显隐；「关闭到托盘」受设置控制 |
| 8 | **单实例** | 第二实例聚焦已有窗口；数据目录在同级 `data\`，锁也跟随 |
| 9 | **数据目录** | 见 §1.1 语义映射；文档需明示两版差异 |
| 10 | **产物命名** | 与 `build-custom.yml` 既有约定一致，让部署脚本不感知壳 |
| 11 | **注入脚本单一真源** | `titlebar.js` / `loading.js` / `shell.html` 从 `webview-app/src`、`splash.html` 同目录、`settings.html` 从 `webview-app/dist` 引用（构建期由 `stage-assets.cjs` 铺到 `out/inject/` 随 asar 分发，2026-09-26 起不再放 asar 外），禁止复制两份 |
| 12 | **页面全局变量** | `window.web2appExtra = <JSON>` 保持同名，供站点脚本读取 |

**必须显式记录的差异**：Electron 原生窗口标题默认跟随 `document.title`（Tauri 版固定编译期常量 `TITLE`，2026-09-22 由 `WINDOW_TITLE` 改名）——
已显式 `win.on('page-title-updated', e => e.preventDefault())` 且建窗时传 `title: cfg.title`（漏传会停在 package.json 的 name）。

---

## 4. 分阶段计划与状态

| 阶段 | 内容 | 状态 |
|---|---|---|
| **M0 骨架** | 目录/TS/electron-builder、`setupPortablePaths()`、`config.json` 读取、无边框空窗 | ✅ 完成 |
| **M1 窗口复刻** | `titlebar.js` 改造、38px 契约、窗口按钮 IPC、双击最大化、拖动 | ✅ 完成 |
| **M2 代理接入** | `w2a-proxy` sidecar、spawn+握手、鉴权端到端 | ✅ 完成 |
| **M3 系统集成** | 托盘、全局快捷键、单实例、设置窗（Shift+F1）、窗口几何持久化 | ✅ 完成 |
| **M4 页面行为** | 新窗口策略、下载另存为、右键菜单、F12、加载层、外链不泄漏凭据 | ✅ **已交付（2026-09-24）**：下载接管 / 右键菜单 / F12 三项补齐（实现在 `electron-app/src/main/page-behavior.ts`）；新窗口策略与加载层随 M1 已交付 |
| **M5 多页签** | `WebContentsView` + `shell.html` 标签栏 + 标题回传 | ✅ 完成（T0–T4；探针**多页签 31/31、单窗口 134/134**，2026-09-26 实测；2026-09-18 真机目视 V1–V4 关闭） |
| **M6 打包** | 三形态出包、版本信息、config 注入、日志目录、SmartScreen 文案 | ✅ 完成 |
| **M7 CI** | Windows workflow：build → lint → E2E → 产物上传 | ✅ **已落库（2026-09-24）**：`.github/workflows/electron-ci.yml`，lint（Ubuntu）+ e2e（Windows）+ package（非 PR，上传 zip / portable）三作业 |

### 4.1 测试能力（Electron 侧比 Tauri 侧强得多）

Tauri 侧断言靠 `<img src="/beacon">` 回传（WebView2 走 DWM 合成，截屏拿不到内容）；Electron 直接绕开：

- `webContents.executeJavaScript()` 直读任意 DOM 状态 → 38px 契约、按钮命中、配色等；
- `webContents.capturePage()` 截图可用 → 视觉回归（注意：**看不到同窗口下的子 `WebContentsView`**，要用 `desktopCapturer`）；
- 可写真正的 E2E（Playwright `_electron.launch()`）。

**探针（`src/main/probe.ts`，`pnpm run probe`）把 §3 的 12 条契约变成可执行断言，退出码即结论** ——
这是双壳长期不跑偏的唯一保障。**注意探针的起窗模式只由命令行决定（`--probe` / `--probe --tabs`），`tabs_enabled` 不参与。**

---

## 5. 风险清单（仍有效的部分）

| # | 风险 | 缓解 |
|---|---|---|
| 1 | **绿色版数据目录不可写** | `try/catch` 回退 `%APPDATA%`；启动日志明示实际数据目录 |
| 2 | **sidecar 僵尸进程** | 子进程监视 stdin EOF 自杀 + `will-quit` kill + 启动时端口探测 |
| 3 | **未签名 exe 的 SmartScreen** | 引导文案（README）；不教用户关闭 SmartScreen |
| 4 | **无自动更新** | 人工发新包 + 应用内「关于」显示版本 |
| 5 | **Chromium 版本滚动** | 固定升级节奏；`npm audit` + Dependabot |
| 6 | **双壳长期漂移** | §3 契约 + 注入脚本单一真源 + 探针双向跑 |
| 7 | **多页签 bounds 同步** | M5 专项测试（V2 跨 DPI 真机目视已关闭） |
| 8 | **国内下载 Electron 二进制** | 走环境变量镜像（`ELECTRON_MIRROR` / `ELECTRON_BUILDER_BINARIES_MIRROR`，`.npmrc` 对 pnpm 无效） |
| 9 | **外链鉴权回退** | sidecar 代理（主）+ `app.on('login')` 兜底 |

---

## 6. 实施过程中已定的小事

1. **`config.json` 随 `app.asar` 进包**（2026-09-22 改造，原方案见下）：由 `scripts/stage-assets.cjs` 生成
   **`out/config.json`**（有则复制项目根的，无则生成全字段默认模板），随 `files: out/**/*` 封进 asar；
   运行期读 `app.getAppPath()/out/config.json`（开发态回落项目根）。
   ⚠️ **原方案**是「`build/config.json` + `extraFiles` 放到 exe 同级」并靠 `loadConfigAny([用户目录, 包内])` 兜单文件形态 ——
   已废弃：配置放 exe 旁边等于**任何拿到包的人都能改掉地址**，配置就失去意义。
   代价是不能再用「放一份 config.json 到 exe 旁边」覆盖，改配置必须重新打包。
   契约与多端行为见 `docs/packaging-config.md`。
2. **窗口/托盘图标**：复用 `webview-app/icons/icon.png`（`extraResources` 拷到 `resources/icons/`，Tray 缩到 16px）；
   `win.icon: build/icon.ico` 由同一脚本从 `../webview-app/icons/icon.ico` 复制，不提交副本。
3. **外部代理**：Electron 侧用 Chromium 原生 `--proxy-server`（M0 已接好）。

---

## 7. 出包过程中的两个关键教训（详见 `electron-app/README.md`）

1. **`createWindow()` 漏传 `title` → 配置 title 静默失效**（拦了 `page-title-updated` 后不会退回页面标题）。
2. **`tsc` 默认「有错也输出」+ 只跑 typecheck 不重跑 build → 半成品 JS 封进 asar**（`noEmitOnError: true` + 打包前必 `build`）。
