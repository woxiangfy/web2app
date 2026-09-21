# electron-app 落地方案（修订版 · 决策已锁定）

> 修订日期：2026-09-16 ｜ 现状核对：**2026-09-20** ｜ 本文件取代 `electron-analysis.md` 中「是否该做」的讨论，只保留落地方案。

> ⚠️ **本文档是「设计方案 + 过程记录」，不是当前状态的权威描述。**
> 里程碑状态、命令、契约与判据**一律以 `../electron-app/README.md` 为准**。
> 2026-09-20 核对确认下列内容属**历史陈述，勿据此施工**（原文保留以留痕）：
>
> | 位置 | 已过期 | 现状 |
> |---|---|---|
> | §2 目录树 | 设计期草案文件名（`bootstrap.ts` / `proxy-sidecar.ts` / `window.ts` / `hotkey.ts` / `ipc.ts` / `renderer/inject/`） | 实际文件清单见 §2 的「实际 `src/main/` 为」一段 |
> | §3.1 表 | 多页签承载写 `BaseWindow` | 已推翻：选型 A′ = `BrowserWindow` 当壳 + 子 `WebContentsView`（`electron-tabs-design.md` §2.3） |
> | §6 风险 9、§8.1 | 教用 `.npmrc` 配 `ELECTRON_MIRROR` | 对 pnpm **无效**，必须走环境变量（`../electron-app/README.md`「包管理器：pnpm」） |
> | §8.2 | `tabs_enabled=true` 提示「待 M5」；Electron 体积「约 320MB」 | M5 已完成，`tabs_enabled` 是生效开关；体积实测 **367.67MB** |
> | §8.3 | 无 GPU 环境「无需额外传参」 | **不够**：必须再加 `--no-sandbox --in-process-gpu` |
> | §8.6 / §8.8 | CI 用 `npm ci`；portable 96.4MB；win-unpacked 372MB | 已迁 pnpm；实测 **96.5MB** / **371.66MB**（口径见 `electron-size-reduction.md`） |
> | §5 M2/M3 行 | 验收数字 50/50、98/98 | 当时快照；当前 **112**（单窗口）/ **23**（多页签） |
> `electron-analysis.md` 仍可作为功能盘点与迁移映射的参考。

## 决策锁定（2026-09-16）

| 项 | 结论 | 影响 |
|---|---|---|
| 是否做 | **Electron 必做** | 讨论终止，进入实施 |
| 平台 | **仅 Windows** | Electron 不支持移动端的短板消失；Android 继续由 Tauri 版负责 |
| 系统 | **Win10+** | 可用最新 Electron（44.x / Chromium 150 / Node 24），无 Win7 顾虑 |
| 体积 | **不限** | 不做极致裁剪；asar 保持默认 |
| 分发 | **只出绿色版**，且**解压目录版与单文件自解压版都要** | 打包链简化（无 MSI/NSIS/WiX）；但放弃自动更新 |
| 数据目录 | **exe 同级 `data\`**（`data_dir_default=false`） | 真绿色、整目录可搬走；只读介质自动回退 |
| 代码签名 | **不做** | 需为 SmartScreen 写引导说明；版本号/SHA256 供核对 |
| Tauri 版 | **继续维护，且持续更新** | 双壳长期并行 → §4 契约与「注入脚本单一真源」成为硬要求 |
| 多页签 | ~~**暂缓**（复杂则以后再说）~~ **已完成（M5）** | 原计划 M5 降为待排期；实际已落地：选型 A′（`BrowserWindow` 当壳 + 子 `WebContentsView`），`tabs_enabled` 现在是**生效开关** |

**进度（2026-09-20 核对）：M0–M3、M5、M6 已完成并实测通过（M6 含启动日志与版本信息）；M4 已交付一部分（新窗口策略见 M1、加载层/失败态见 M1），仅剩「下载接管」等页面行为未做；M7（CI）未落库**，见 §5 与 §8。


---

## 0. 约束变化带来的结论重算

| 维度 | 上一版前提 | 本版（已定） | 影响 |
|---|---|---|---|
| 动机 | 待确认（可能是依赖环境） | 直接实施，不再论证 | 省掉 `webviewInstallMode` / `fixedRuntime` 那整条支线 |
| 平台 | Win + Android | **仅 Windows** | Electron 不支持移动端的最大短板消失 |
| 系统 | 未定（可能含 Win7） | **Win10+** | 可用最新 Electron（无 Win7 顾虑）；建议实测锁 Win10 1809+ |
| 体积 | 敏感 | **不限** | 不做极致裁剪；asar 保持默认，可读性优先 |
| 分发 | MSI + NSIS + 绿色版 zip | **只绿色版** | 打包链大幅简化；但自动更新基本放弃（见 §6.4） |
| 旧版 Tauri | — | 保留（继续负责 Android 与备用） | 双壳维护，靠 §4 契约约束一致性 |

**结论：可行性从「有条件」变为「直接可做」。** 主要工作量集中在主进程编排、多页签与打包链三块。

---

## 1. 「绿色版」的准确定义（本节是最容易做错的地方）

### 1.1 ⚠️ 不要用 electron-builder 的 `portable` target

> ⚠️ **本节结论已被决策锁推翻（2026-09-16）**：最终决定「解压目录版与单文件自解压版**都要**」，
> `electron-builder.yml` 现配置 `dir` + `zip` + `portable` 三个 target，M6 已实测三形态出包（见 §8.8）。
> 以下内容保留作「当初为什么犹豫」的记录，**不要**据此删掉 `portable` target。

`portable` 产出的是一个**自解压单文件 exe**：每次启动都把自己解压到 `%TEMP%`，数据默认仍写 `%APPDATA%`。后果：

- 首次启动明显变慢（要解压 ~250–300MB）；
- 杀软/企业策略对「运行时向 `%TEMP%` 落 exe」敏感，误报概率更高；
- 磁盘上会残留多份 `%TEMP%` 副本；
- 它**不是**免安装即用的绿色版，只是「无需安装包」。

### 1.2 正确形态：`dir` 工作目录 + `zip` 分发

```yaml
# electron-builder.yml
appId: com.example.web2app
productName: Web2App
directories: { output: release }
files: ['out/**', 'package.json']
asarUnpack: ['**/*.node']          # 原生模块（若用 napi-rs 方案）
extraResources:                    # ⚠️ sidecar 必须放这里，asar 内的 exe 无法 spawn
  - from: ../webview-app/target/release/w2a-proxy.exe
    to: sidecar/w2a-proxy.exe
win:
  target: [{ target: dir, arch: [x64] }]   # 先出目录，再用 zip/7z 打包
  icon: build/icon.ico
portable: null                     # 明确不用
```

- 产物：`Web2App-<ver>-win-x64/` 目录 → 压成 `Web2App-<ver>-win-x64.zip`（预计压缩后 **110–150MB**，解压 **250–300MB**）。
- 分发：解压到任意目录，双击 `Web2App.exe` 即用。
- 若你更想要**单文件**，那就是 `portable` target，需接受上面那串代价——**建议先用目录形态**，这是两条路的分叉点之一（见 §7）。

### 1.3 真绿色的关键：重定向全部数据路径

必须在 `app.whenReady()` **之前**（早于任何 `electron-store` / `session` 使用）执行：

```ts
// src/main/portable.ts —— 在 main 入口第一行调用
import { app } from 'electron'
import { dirname, join } from 'node:path'

export function setupPortablePaths() {
  const exeDir = dirname(app.getPath('exe'))
  const dataDir = join(exeDir, 'data')          // 与 webview-app 的 data_dir_default=false 语义对齐
  try {
    mkdirSync(dataDir, { recursive: true })
    writeFileSync(join(dataDir, '.writable'), '')   // 可写性探测
    app.setPath('userData', dataDir)
    app.setPath('sessionData', join(dataDir, 'session'))
    app.setPath('logs', join(dataDir, 'logs'))
    app.setPath('crashDumps', join(dataDir, 'crash'))
  } catch {
    // 只读介质（U 盘写保护 / Program Files / 网络盘）→ 退回 %APPDATA%，保证能启动
    console.error('[web2app] 绿色数据目录不可写，回退用户目录')
  }
}
```

必须一并覆盖的路径：`userData`（Cache / GPUCache / Cookies / Local Storage / Network 都在其下）、`sessionData`、`logs`、`crashDumps`。
单实例锁（`requestSingleInstanceLock()`）的锁文件在 `userData` 下 → 随重定向**自动**落在同目录，无需额外处理。

### 1.4 与现有 Tauri 绿色版的语义对齐

`webview-app` 的 `data_dir_default` 字段（`false` = `%LOCALAPPDATA%\{app_name}`）在 Electron 侧应映射为：

| `data_dir_default` | Tauri 现状 | Electron 侧 |
|---|---|---|
| `false` | `%LOCALAPPDATA%\<app_name>` | exe 同级 `data\`（真绿色，**推荐**） |
| `true` | WebView2 系统默认目录 | `%APPDATA%\Web2App`（跟随系统） |

> 语义差异需写进 §4 契约并在文档里说明，避免运维以为两版行为一致。

### 1.5 绿色版依然要处理的非"绿色"残留

- **SmartScreen**：未签名 exe 首次运行会弹「Windows 已保护你的电脑」。绿色版通常不签名 → 需提前告知使用者操作路径，或为 exe 补时间戳签名（`signing/` 里现有的是 Android keystore，**Windows 代码签名证书是另一套**，属待确认项）。
- **注册表**：Electron 默认不写注册表（不装、不注册协议处理器、不加开机自启）→ 天然满足。
- **自动更新**：绿色版基本放弃 `autoUpdater`（它需要安装目录可写 + 更新源），改为人工发新包（§6.4）。

---

## 2. 目标架构

> ⚠️ **本节是设计阶段的草案，实际实现的文件名与下表并不一致。**实际 `src/main/` 为：
> `config.ts` / `index.ts` / `navigate.ts` / `portable.ts` / `probe.ts` / `settings-window.ts` /
> `settings.ts` / `shortcuts.ts` / `sidecar.ts` / `tray.ts` / `window-state.ts`；
> `src/renderer/` 为 `preload.ts` / `settings-preload.ts`。

```
electron-app/
├── package.json
├── electron-builder.yml
├── tsconfig.json
├── src/
│   ├── main/
│   │   ├── index.ts            # 入口：setupPortablePaths() → 单实例 → 建窗 → 后台 bootstrap
│   │   ├── portable.ts         # §1.3 路径重定向
│   │   ├── config.ts           # 17 字段配置（与 webview-app 同一份 config.json）
│   │   ├── bootstrap.ts        # 后台解析真实地址（DNS TXT + 验签）→ 就绪后导航
│   │   ├── proxy-sidecar.ts    # 拉起 w2a-proxy.exe、握手取端口、生命周期管理
│   │   ├── window.ts           # 无边框窗口 + 窗口按钮 IPC + 几何持久化
│   │   ├── tabs.ts             # WebContentsView 多页签（tabs_enabled 时启用）
│   │   ├── tray.ts / hotkey.ts / settings.ts
│   │   └── ipc.ts              # contextBridge 暴露面的主进程侧
│   └── renderer/
│       ├── preload.ts          # 最小 API 面（不暴露 ipcRenderer 本体）
│       └── inject/             # 直接引用 webview-app/src 下的注入脚本（单一真源）
│           ├── titlebar.js     # ⚠️ 唯一需要改造的：去掉 __TAURI__
│           ├── loading.js      # 基本原样复用
│           ├── splash.html     # 原样
│           └── shell.html      # 标签栏，原样
└── sidecar/ → webview-app/target/release/w2a-proxy.exe
```

**进程模型**：main（Node，全权）＋ 每个窗口一个 sandboxed renderer ＋ 1 个 Rust sidecar（代理+加密）。
**技术选型建议**：TypeScript + `tsc`，设置页/标签栏用原生 HTML（不引 React/Vue）——与项目「低依赖」的一贯风格一致。

### 2.1 sidecar 是这套方案的核心复用点

需要在 `webview-app` 侧新增一个 bin crate（不改现有逻辑）：

```toml
# crates/proxy-server/Cargo.toml  —— bin 名 w2a-proxy；只依赖 proxy.rs + resolve-core，不拉 Tauri
[[bin]]
name = "w2a-proxy"
```

- 握手：启动后向 **stdout 打一行 JSON** 并 **flush**（字段：`ok` / `proxyUrl` / `resolvedUrl` /
  `upstreamHost` / `hadAuth` / `authUser` / `authPass` / `pid`；日志一律走 stderr），主进程读到即完成握手；
  子进程退出/超时 → 主进程报错并显示可读错误页（对齐现有 bootstrap 的失败文案）。
- 生命周期：`app.on('will-quit')` 里 `kill()`；子进程自己也要在父进程断开时退出（避免僵尸进程占端口）。
- ⚠️ **asar 内的 exe 无法 `spawn`** → 必须走 `extraResources`，运行时用 `process.resourcesPath` 定位。
- ⚠️ 路径可能含空格/中文 → `spawn(exe, args)` 用数组形式，别拼字符串。

---

## 3. 迁移映射（仅 Windows 必需部分）

| 现有模块 | 规模 | Electron 对应 | 工作量 | 备注 |
|---|---|---|---|---|
| `proxy.rs` + `resolve-core` | 741 + 573 行 | **原样复用**（sidecar） | 极低 | 见 §2.1，**绝不重写** |
| `resolve.rs` DNS TXT | 208 行 | 侧车承担 | 极低 | 挪进 sidecar 更省事 |
| `titlebar.js` | 31 KB | 改造 `__TAURI__` → `window.web2app` | 低 | ⚠️ `applyChromeFit()` 那套 38px 契约**必须整体保留** |
| `loading.js` / `splash.html` | 18 KB | 原样 | 极低 | 注入时机更宽裕，`whenHostReady` 兜底可留可删 |
| `shell.html` / `settings.html` | 11 + dist | 原样 | 低 | 只换 IPC 调用名 |
| `dnd_polyfill.js` | 22 KB | **不引入** | — | WebView2 专属 bug，Chromium 无此问题 |
| `webview_hooks.rs` | 757 行 | `setWindowOpenHandler` / `will-download` / `context-menu` / `app.on('login')` | 中 | 全是一等 API，无 COM、无后台线程轮询 |
| `tabs.rs` | 872 行 | `WebContentsView` | **高** | 见 §3.1 |
| `lib.rs` 编排 | 502 行 | `src/main/index.ts` | 中 | 启动序列按 §3.2 重排 |
| `save_dialog.rs` | 127 行 | `dialog.showSaveDialog` | 低 | 不用碰 `IFileSaveDialog` / STA / COM |
| `tray.rs` `settings.rs` `window_state.rs` | 586 行 | `Tray` / 自绘设置窗 / 手写 | 低 | — |
| `build-custom.yml` | 62 KB | `electron-builder.yml` + 一条 workflow | **中** | installer 那半可砍，但 config 注入/产物命名要保留 |
| capability / ACL 层 | — | **整体消失** | — | 那条 `remote.urls` 必须带 `:*` 的坑不再存在 |

> 净重写量约 **2500–3000 行 TS**；可直接复用约 **1500 行 Rust + 50KB 前端资源**。

### 3.1 多页签的语义变化（最需要提前想清楚）

| | Tauri（现状） | Electron（建议） |
|---|---|---|
| 承载 | 每个 tab 一个独立 `WebviewWindow`（顶层 HWND） | `WebContentsView` 挂在**壳窗口**内 —— ⚠️ 原写 `BaseWindow`，**该判断已推翻**：落地用 `BrowserWindow` 当壳（见 `electron-tabs-design.md` §2.3 与 §4 方案 A′） |
| 几何 | owned window + `place_tab` 闭环校准（≤3 轮） | 视图 bounds 由主进程设，窗口 resize 时同步，**无需校准** |
| 遮挡问题 | 必须 `.owner(&host)` 否则空客户区盖住网页 | 不存在（同窗口内子视图） |
| 关闭 | 必须 `destroy()` 防重入 | `view.webContents.close()`，无重入风险 |
| 隐藏/最小化 | 需 `set_tabs_visible` / `sync_tabs_with_host` | 随窗口自然跟随 |
| ⚠️ 新风险 | — | `WebContentsView` 与窗口最大化/全屏/DPI 变化的 bounds 同步要自己测；标签栏（`shell.html`）改为同一窗口内的 HTML 视图，**层级顺序**需显式管理 |

### 3.2 启动序列（对齐现有语义）

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

两条代理路径与现有注释里的三态决策一致；`--proxy-server` 是 Chromium 原生 switch，**现有 `build_proxy_args()` 的产物可几乎直接复用**。

---

## 4. 兼容契约（不逐条对齐，所谓"兼容"就是空话）

| # | 契约项 | 要求 |
|---|---|---|
| 1 | **`config.json` 字段** | 沿用现有 17 字段，不增不改名；`android_*` 四个字段**静默忽略**而非报错（向后兼容）。布尔值需接受字符串形式（上游模板是 `"false"`）。建议补一份 JSON Schema 做校验 |
| 2 | **顶部留白 38px** | 自绘标题栏 + `html { padding-top: 38px }` + `applyChromeFit()` 三件事全保留，与 Tauri 版逐像素一致 |
| 3 | **标题栏按钮** | 后退 / 前进 / 最小化 / 最大化（双击标题栏最大化）/ 关闭，行为与位置一致 |
| 4 | **Basic 鉴权** | 覆盖 HTTP **与 WebSocket**；`Set-Cookie` 的 `Domain=真实host` → `127.0.0.1` 重写；`X-Forwarded-Proto` 透传上游 scheme |
| 5 | **新窗口策略** | `target="_blank"` / `window.open()` → 单窗口模式原地导航、多页签模式新开 tab |
| 6 | **快捷键** | `Alt+4` 显隐（可在设置改）、`Shift+F1` 打开设置（固定） |
| 7 | **托盘语义** | 左键显隐；「关闭到托盘」受设置控制 |
| 8 | **单实例** | 第二实例聚焦已有窗口 → **注意数据目录在同级 `data\`，锁也跟随** |
| 9 | **数据目录** | 见 §1.4 的语义映射表；文档需明示两版差异 |
| 10 | **产物命名** | zip 名规则与 `build-custom.yml` 现有约定一致，让部署脚本不感知壳 |
| 11 | **注入脚本单一真源** | `titlebar.js` / `loading.js` / `shell.html` **从 `webview-app/src` 引用**（构建期拷贝），禁止复制两份 |
| 12 | **页面全局变量** | `window.web2appExtra = <JSON>` 保持同名，供站点脚本读取 |

**必须显式记录的差异**（易被误认为 bug）：Electron 原生窗口标题默认跟随 `document.title`（Tauri 版是固定的 `WINDOW_TITLE`）——若要与旧版一致需显式 `win.on('page-title-updated', e => e.preventDefault())`。

---

## 5. 分阶段计划

| 阶段 | 内容 | 验收标准 | 量级 | 状态 |
|---|---|---|---|---|
| **M0 骨架** | 目录/TS/electron-builder `dir` target、`setupPortablePaths()`、`config.json` 读取、无边框空窗 | 解压到任意目录双击可启动；`data\` 出现在同级；只读目录能回退启动 | 0.5 天 | ✅ 完成 |
| **M1 窗口复刻** | `titlebar.js` 改造、38px 契约、窗口按钮 IPC、双击最大化、拖动 | 与 Tauri 版逐像素对比一致；无 ACL 类静默失败 | 1 天 | ✅ 完成 |
| **M2 代理接入** | 新增 `w2a-proxy` bin crate、spawn+握手、鉴权端到端 | 带 Basic 鉴权的真实站点可打开、刷新不空白、**WS 也带上凭据** | 1 天 | ✅ 完成（断言 50/50；真实站点待补） |
| **M3 系统集成** | 托盘、全局快捷键、单实例、设置窗（Shift+F1）、窗口几何持久化 | 与旧版行为一致；`settings.json` 与旧版可互相读取 | 1 天 | ✅ 完成（断言 98/98；双向互通已由两侧测试钉住） |
| **M4 页面行为** | 新窗口策略、下载另存为、右键菜单、F12、加载层、外链不泄漏凭据 | 与旧版行为一致 | 1 天 | 🟡 **部分交付**：新窗口策略（`setWindowOpenHandler`）与加载层 / 失败态随 M1 已交付；**未实现**：下载另存为（`will-download` 现仅记日志）、右键菜单、F12、外链凭据隔离的专项验证 |
| **M5 多页签** | `WebContentsView` + `shell.html` 标签栏 + 标题回传 | 点标签栏空白处网页不被盖；一次点击只开 1 个；新 tab 落目标 path；连续关不卡 | 2 天 | ✅ 完成（T0–T4；探针 23/23，单窗口 112/112 零回归；2026-09-18 真机目视 V1–V4 全部关闭） |
| **M6 打包** | zip 绿色版、版本信息、config 注入、日志目录、SmartScreen 提示文案 | 产出可分发 zip；目标干净机器上解压即用 | 1 天 | ✅ 完成（出包三形态见 §8.8；版本信息经 VERSIONINFO 实证已落地；启动日志已落 `data\logs\startup.log`；SmartScreen 文案见 `../electron-app/README.md`） |
| **M7 CI** | 一条 Windows workflow：build → lint → E2E → 产物上传 | 提交即跑通 | 1 天 | ⬜ 未落库（前提：`electron-app/` 尚未纳入版本控制） |

**合计约 8.5 人日**（不含 M5 多页签可选的额外打磨）。

> 建议顺序：**M0→M1→M2 连续做完再评估**。这三步就能回答「Electron 版能不能真的替代」这个核心问题，成本 2.5 天。

### 5.1 测试能力反而比 Tauri 版强得多

Tauri 侧做断言要靠 `%TEMP%\w2a_probe2` 那套 `<img src="/beacon?d=...">` 回传（因为 WebView2 走 DWM 合成，截屏拿到的是下层窗口、`PrintWindow` 全黑）。

Electron 直接绕开这些：

- 主进程可 `webContents.executeJavaScript()` 直读任意 DOM 状态 → 断言 38px 契约、按钮 `elementFromPoint` 命中、`applyChromeFit` 是否打标；
- `webContents.capturePage()` **截图可用** → 做视觉回归；
- 可用 **Playwright 的 Electron 支持**（`_electron.launch()`）写真正的 E2E；
- 无需节流规避、无需单实例等待、无需重新 build 换配置（配置可运行时读）。

**建议 M1 起就同步搭这套断言**，把 §4 的 12 条契约变成可执行用例——这也是双壳长期不跑偏的唯一保障。

---

## 6. 风险清单（按严重度）

| # | 风险 | 说明 | 缓解 |
|---|---|---|---|
| 1 | **绿色版数据目录不可写** | 解压到 Program Files / 只读盘 / 网络盘 → 无法写 `data\` | §1.3 的 `try/catch` 回退 `%APPDATA%`；启动日志明示实际数据目录 |
| 2 | **sidebar 僵尸进程** | 主进程崩了 sidecar 还占着端口 | 子进程监听父进程断开自杀 + `will-quit` kill + 启动时端口探测 |
| 3 | **未签名 exe 的 SmartScreen** | 绿色版通常不签名，首次运行被拦 | 明确告知操作步骤；或采购 Windows 代码签名证书（**现有 `signing/` 是 Android keystore，不通用**） |
| 4 | **无自动更新** | 绿色版放弃 `autoUpdater` | 人工发新包 + 应用内「关于」显示版本；如需更新能力，改为「检查版本 → 提示手动下载」 |
| 5 | **Chromium 版本滚动** | 每约 8 周一个 major，安全补丁要自己跟 | 固定升级节奏；`npm audit` + Dependabot；把 Electron 版本写进 CI 矩阵 |
| 6 | **Win10 早期版本** | Electron 官方口径「Windows 10 and up」，未承诺 1507/1607 | 在目标机实测；建议对外声明 **Win10 1809+** |
| 7 | **双壳长期漂移** | Android 仍在 Tauri，两套实现会各改各的 | §4 契约 + 注入脚本单一真源 + 共享 JSON Schema；契约用例双向跑 |
| 8 | **多页签 bounds 同步** | `WebContentsView` 在最大化/全屏/DPI 切换时的几何 | M5 专项测试；必要时监听 `resize` / `display-metrics-changed` |
| 9 | **国内下载 Electron 二进制** | `npm i electron` 要拉 ~120MB，易超时 | `.npmrc` 配 `ELECTRON_MIRROR=https://npmmirror.com/mirrors/electron/`；CI 缓存 |
| 10 | **外链鉴权回退** | 页面里指向自身域名的绝对 URL 不走本地代理 → 无凭据 | 双路径：sidecar 代理（主）+ `app.on('login')`（兜底，对齐 WebView2 的 `BasicAuthenticationRequested`）。⚠️ 需实测 `login` 事件对 **WebSocket 握手**是否生效 |

---

## 7. 未决事项（不再阻塞实施）

已确认：绿色版形态两种都出、数据目录写 exe 同级 `data\`、不做代码签名、Tauri 版长期并行。
（原文此处还有「多页签暂缓」——**该条已被取代**：M5 多页签已实施完毕，见 §5 与 §8。）

仍需在实施过程中定的小事：

1. ~~**绿色版 zip 里要不要预置一份 `config.json` 模板**？~~ → **已定（M6）**：**预置**。
   由 `scripts/stage-assets.cjs` 生成 `build/config.json`（有则复制项目根的，无则生成全字段默认模板），
   再用 `extraFiles` 放到 **exe 同级**；单文件形态下这份同时充当回落副本（`config::loadConfigAny`）。
2. ~~**窗口图标 / 托盘图标**从哪来~~ → **已定（M3）**：复用 `webview-app/icons/icon.png`
   （由 `electron-builder.yml` 的 `extraResources` 拷到 `resources/icons/`，Tray 里再缩到 16px）。
   `win.icon: build/icon.ico` 也已落实（M6）：同一脚本从 `../webview-app/icons/icon.ico` 复制，
   不再要求人工补文件。
3. **外部代理与 `proxy_enabled`**：Electron 侧用 Chromium 原生 `--proxy-server`（已在 M0 接好），
   而 Tauri 版另一条路径是 `additional_browser_args`；两条路径的 bypass 列表语法需实测一致。

---

## 8. 各阶段进展（M0–M3 已完成、M6 出包已通）

### 8.1 已落地

| 文件 | 作用 |
|---|---|
| `package.json` | 依赖 + `postinstall: install-electron`（⚠️ electron@44 不再自动下载二进制）+ 冒烟/打包脚本 |
| `tsconfig.json` | TS 7，`module: node16`（⚠️ TS 7 已移除 `moduleResolution: node`） |
| `.npmrc` | electron 与 electron-builder 二进制走 npmmirror，否则墙内必卡 |
| `electron-builder.yml` | 三个 target：`dir` + `zip` + `portable`（`requestExecutionLevel: user`，绿色版绝不 UAC）；`extraResources` 从 `../webview-app/src` 拷注入脚本（单一真源）；sidecar 条目已启用（M2 已完成，见 §8.6） |
| `src/main/portable.ts` | 绿色路径装配：`installRoot()` 同时支持目录版与单文件自解压版；四项路径一起重定向；写探测 + 只读介质回退 |
| `src/main/config.ts` | 17 字段解析（宽容布尔、`android_*` 静默忽略、`extra` 合法性校验、`title`→`app_name` 回落）；`buildProxySwitches()` 对齐 `build_proxy_args()` 语义 |
| `src/main/index.ts` | 启动序列（路径→代理 switch→协议注册→单实例→建窗）、`splash://` 协议、无边框窗口、窗口按钮 IPC、启动日志、冒烟自检 |
| `src/renderer/preload.ts` | 只暴露白名单方法（`contextBridge`，不交出 `ipcRenderer`） |
| `README.md` | 命令、两种形态、数据目录语义、与 Tauri 版的差异、已知隐患 |

### 8.2 实测数据（本机）

- `electron@44.4.1` 二进制解压后 **约 320MB**（`node_modules/electron/dist`）——印证 §0 的体积判断。
- `tsc --noEmit` / `tsc` 均通过；产物为 CJS（`out/main/*.js`、`out/renderer/preload.js`）。
- 冒烟自检（无 `config.json`）：数据目录 = `<项目根>\data`，`title` 正确回落为 `app_name`。
- 冒烟自检（有 `config.json`，布尔值为字符串）：`"proxy_enabled":"true"` 正确解析为代理开关、
  `proxy_http` 自动镜像到 https、`extra` 序列化正确、`android_*` 只告警不报错、
  `tabs_enabled=true` 提示「待 M5」。
- 启动页模板替换：**77676 字符**，**未替换占位符 0 个** ✅（该断言现在会进 CI 退出码）

### 8.3 环境坑（踩到了，记下来）

1. **electron@44 的二进制不随 `npm install` 下载**（包内无 `postinstall`，改为独立的 `install-electron` bin）→ 已在 `package.json` 加 `"postinstall": "install-electron"`。
2. **TS 7 移除了 `moduleResolution: node`**（报 TS5108）→ 改 `node16`（因 `package.json` 无 `"type":"module"`，仍输出 CJS）。
3. **无 GPU 环境**（容器 / 部分远程桌面）启动会刷 `GPU process isn't usable. Goodbye.` 并退出 →
   探针模式已**自动**关闭硬件加速（`PROBE` 分支调 `app.disableHardwareAcceleration()`），但
   ⚠️ **这还不够**：GPU 子进程仍会反复崩，最后一条 `FATAL` 把整个进程带走，退出码是
   `-2147483645`（异常码，不是 1）。这种环境**必须**再显式加 `--no-sandbox --in-process-gpu`。
   真机不要长期关（滚动与动画会明显变卡）。
4. 开发态 `app.getPath('exe')` 指向 `node_modules/electron/dist/electron.exe`，
   故 `installRoot()` 在 `!app.isPackaged` 时回落 `app.getAppPath()`。
5. ⚠️⚠️ **宿主注入 `ELECTRON_RUN_AS_NODE=1` 会让 Electron 以纯 Node 身份启动**。
   桌面型 IDE / agent 沙箱会给所有子进程注入它（配套 `NODE_OPTIONS=--require <语言服务 shim>`）。
   症状：`app` 为 undefined，业务代码第一句就抛 `Cannot read properties of undefined`，
   **错误位置落在我们自己的代码里**，看上去完全是项目 bug；更麻烦的是**时好时坏**
   （不同宿主、不同调用方式注入与否不一致），极易被误判为随机故障。
   判别：`electron.exe --version` 打印 `v24.x`（Node 版本）而不是 `v44.x`。
   修法：所有 electron 命令统一经 `scripts/launch.cjs` 拉起（已接进 `pnpm run start/smoke/probe`），
   它在 spawn 前 `delete env.ELECTRON_RUN_AS_NODE`。
   ⚠️ 排查时**别用 `env -u`**：`env(1)` 会重建进程环境，连宿主用来捕获 stdout 的管道一起切断 ⇒
   「退出码 0 但一个字都没有」，于是你会去怀疑参数、怀疑代码，而真正的问题是**看不见输出**
   （2026-09-16 在这上面绕了很久）。也别顺手清 `NODE_OPTIONS` —— 那个 shim 正是输出通道。

### 8.4 曾发现的隐患（**已修复，两壳同步**）

`splash.html` 顶部注释（第 10–12 行）曾把三个占位符名也写了一遍，而替换是全局的 ⇒
`titlebar.js`(31KB) 与 `loading.js`(18KB) 的全文会被写进那个 HTML **注释**里。
当时"无害"纯属侥幸（脚本第 1 行是 `/*`、首个 `*/` 恰好落在第 70 行，才没让注释提前闭合）。

**已修**，两端一致：
- `splash.html` 改为**裸 token**（`<script>__WEB2APP_TITLEBAR_JS__</script>`），注释里不再出现占位符名；
- Tauri 侧 `loading.rs` 的回归测试（`splash_template_has_no_token_in_comments`、
  `splash_script_tokens_are_not_comment_wrapped`、`injected_scripts_contain_no_placeholder_tokens`）
  → `cargo test --lib loading::` **7/7 通过**；
- Electron 侧冒烟自检会断言"未替换占位符 = 0"。

### 8.5 M1 进展（已完成：窗口复刻 + 兼容契约探针）

契约探针 `src/main/probe.ts`（`npm run probe`）一次跑完全部断言、**退出码即结论**。
夹具站点内联在代码里（`127.0.0.1` 随机端口），不落盘、不进打包产物。
覆盖：启动页内联注入、标题栏几何/配色/拖动区、38px 让位（fixed / sticky / `100vh` / `inset:0` 四种形态）、
严格 CSP 站点降级、加载层淡出、`target="_blank"` 与 `window.open` 原地导航、
三个窗口按钮真实驱动原生窗口，以及 M2 的 sidecar 代理契约（见 §8.6）。

M1 修掉的两个**真实缺陷**——都不是"配置问题"，是代码问题，且都属于"错了也静默"那一类：

1. **注入脚本跑在隔离世界** ⇒ contextBridge 暴露的 `window.web2app` 只存在于主世界，于是
   `shellName()` 永远返回 unknown、三个窗口按钮全部点不动；而 DOM 跨世界共享，标题栏照样画得出来，
   表象上"看着是对的"，最难查。
   修法：`preload.ts` 改用 `webFrame.executeJavaScript`（主世界），与 Tauri 侧
   initialization_script 所在的世界对齐。
   （教训：preload 顶部注释里那条"隔离世界能正常握手"的推论是错的，已改正并写明原因。）
2. **严格 CSP 站点（`style-src 'self'`）把 `<style>` 元素整个拦掉** ⇒ 标题栏与加载层裸奔
   （透明背景、按钮挤成一团、38px 让位也丢）。
   修法：改用构造函数式样式表（`document.adoptedStyleSheets`，**不受页面 CSP 约束**），
   保留 `<style>` 回退。脚本两壳共用，Tauri 版同步受益。

另修掉探针自身的 3 处期望值错误（不是产品缺陷，是断言写错）：

- 「标题栏满宽」比的是 `innerWidth`，而 `left/right:0` 的 fixed 元素宽度要减去纵向滚动条（本机 15.3px）→ 改用 `clientWidth`；
- 「标题栏已着色」写死了浅色 `#eef0f2`，深色系统上必然误报 → 改为「落在已知浅/深两套配色里」；
- 「页面正文未被压住」受 h1 默认 `margin` 塌陷影响（`#content.top` 38→56.8）→ 夹具加 `h1{margin:0}`。

### 8.6 M2 进展（已完成：sidecar 代理 + 加密解析）

**50/50 断言通过**（M1 的 39 条 + M2 新增 11 条），`npm run probe` 退出码 0。

新增 `crates/proxy-server`（bin 名 `w2a-proxy`），以 `#[path]` 直接引用 `webview-app/src/proxy.rs`
与 `src/resolve.rs` —— **零重写、零复制**，两壳永远跑同一份实现。release 产物 3.76MB。
`electron-builder.yml` 的 `extraResources` 已启用（放 `sidecar/`，**不能进 asar**）。

为此做了一处**必要的解耦**：`resolve.rs` 原本用 `tauri::async_runtime::block_on` 驱动 DNS 查询，
sidecar 拉不进 Tauri 整套依赖树。已改为 `pub async fn`，由调用方驱动
（主 crate 用 Tauri 运行时，sidecar 用自己的 tokio），文件头写明了这个约束。

Electron 侧新增 `src/main/sidecar.ts`（spawn + 握手 + 生命周期）、`navigate.ts` 改为 async 并接入
sidecar、`index.ts` 管 sidecar 的启停（navSeq 代际防并发起多个 + `will-quit` 回收）。

**M2 抓到了一个真 bug，Tauri 版同样中招**（这是这轮最有价值的产出）：

> `handle_websocket` 里「先透传浏览器所有头、后面再设一次 `Origin` 来覆盖」的写法**无效** ——
> `http::request::Builder::header` 与 `reqwest::RequestBuilder::header` 都是 `HeaderMap::append`
> 而不是 insert。上游因此收到**两个** Origin（`http://127.0.0.1:代理端口, http://127.0.0.1:上游端口`），
> 比不改写更糟：做 Origin 校验的后端（WS 防跨站劫持、CSRF 防护）会直接拒连 —— 与那段注释想避免的
> 后果**一模一样**，只是原因不同。
>
> 修法：透传循环里跳过 `origin`（HTTP 分支同时补上 Origin 重写，此前它压根没重写）。
> 回归测试 `proxy_sends_exactly_one_rewritten_origin`；探针两条断言盯着（HTTP + WS 各一）。
> 来源：探针新增的 `WebSocket 的 Origin 已被改写` 断言第一次跑就红了，实际值是逗号拼接的两个源。

其余在 M2 里确定的契约（都写进了代码注释与 README）：

- 上游地址**经 stdin 传递**（`--url-stdin`）而不是命令行：Windows 上同用户的其它进程能通过
  `GetCommandLine` / WMI 读到命令行参数，而 URL 里内嵌的就是站点账号密码；
- sidecar 的 stdout **只给机器读**（恰好一行握手 JSON，且必须 flush），日志一律走 stderr；
- `--no-proxy`（域名解析 + 直连）时 sidecar 握手后立即退出，凭据经 `authUser` / `authPass` 字段
  交给宿主喂 Chromium 的 `login` 事件（**明文，禁止写日志**）；
- 防僵尸三层：`will-quit` 显式 `stop()` + `stop()` 内先关 stdin 后 1s 强杀 + sidecar 自己监视
  stdin EOF（宿主被任务管理器结束时只有第三层能兜住）。

M2 剩余（不阻塞验收）：
1. 带 Basic 鉴权的**真实站点**端到端（探针用的是本地 mock 上游，覆盖协议层；真实站点验内容层）。
2. CI：`npm ci` → `cargo build --release -p w2a-proxy` → `tsc --noEmit` → `npm run smoke` → `npm run probe`
   （归入 M7，当前未开始）。
3. ~~`electron-builder` 依赖尚未安装~~ → **已完成**：`electron-builder ^26.15.3` 已入 devDependencies，
   M6 出包已打通（见 §8.8）。

### 8.7 M3 进展（已完成：托盘 / 快捷键 / 设置窗 / 窗口几何）

**98/98 断言通过**（M2 的 50 条 + M3 新增 48 条），`npm run probe` 退出码 0；
`cargo test` 46/46（新增 3 条互通契约测试），`cargo clippy --all-targets -D warnings` 干净。
（上表为 M3 当时的快照；探针此后又扩到 **112 条**，见 `README.md` 的「兼容契约探针」。）

新增 `src/main/` 五个模块 + 一个 renderer preload：

| 文件 | 职责 |
|---|---|
| `settings.ts` | `settings.json` 读写（含路径解析）、Hotkey ↔ Electron accelerator、宽容解析 |
| `window-state.ts` | `window-state.json` 读写、DIP↔物理像素换算、`planRestore()` 四分支决策 |
| `tray.ts` | 托盘创建/销毁、菜单四项、左键显隐、图标定位 |
| `shortcuts.ts` | 唤醒快捷键注册（**含失败回滚**）+ 固定 `Shift+F1` |
| `settings-window.ts` | 设置窗（`settings://` 协议）、三个 IPC 命令、兜底精简页 |
| `renderer/settings-preload.ts` | `window.__TAURI_INTERNALS__.invoke` 垫片（白名单三命令） |

#### 三个关键决策

**① 设置页 HTML 不重写，用垫片适配（而不是改 HTML 让两壳各认一套 API）。**
Tauri 版设置页调用 `window.__TAURI_INTERNALS__.invoke(...)`。Electron 侧在设置窗的 preload 里
提供同名对象、把三个命令映射到 IPC —— 页面一行不改。理由与「注入脚本单一真源」完全一致：
**界面一旦分叉，两个壳就会各修各的 bug**。垫片只放行白名单（探针有断言钉住「未授权命令被拒」）。

**② `settings.json` / `window-state.json` 写 `%APPDATA%\<app_name>\`，不写 `data\`。**
这与「绿色版数据写 exe 同级」的总原则**有意不一致**，因为契约 #9 要求「与旧版可互相读取」：
Tauri 侧这两个文件走 `app_config_dir()` 的父目录（`%APPDATA%`）+ 清洗后的 `app_name`，
不含 bundle identifier。为兼顾绿色性，解析规则是「`data\` 下若有同名文件则以它为准，且读哪写哪」——
用户手工把两个 JSON 放进 `data\` 就能恢复完全绿色。
体量大的 session / 缓存 / 日志仍然全在 `data\`，绿色性不受影响。

**③ 物理像素存盘 + DIP 恢复。**
旧版 `outer_position()` / `outer_size()` 返回**物理像素**；Electron 的 `getBounds()` / `setBounds()`
走 **DIP**。不换算的话，在 125%/150% 缩放的机器上恢复出来的窗口会缩水一截、且与旧版互相读取会整体错位。
故存盘走 `screen.dipToScreenRect()`，恢复走 `screen.screenToDipRect()`，探针有断言比对换算结果。

#### 「与旧版可互相读取」是怎么被钉住的

| 方向 | 手段 |
|---|---|
| Electron 写 → Tauri 读 | Rust 侧新增 `settings_parse_accepts_electron_written_json` / `state_parse_accepts_electron_written_json`，用**字面量**喂给 serde（= Electron 实际写出的字节形态） |
| Tauri 写 → Electron 读 | Electron 侧探针断言「序列化输出为规范 JSON」——字段名、嵌套结构、布尔**类型**、键顺序全部逐字节比对 |

两个方向合起来才是完整的互通保证；任何一处改名/改类型都会立刻红。

#### 其他值得记的点

- **探针不碰用户真实文件**：探针模式把两个 JSON 整体重定向到 `%TEMP%\w2a-probe-store\`，
  跑 `npm run probe` 不会改掉你机器上的托盘开关和窗口尺寸（已实测 `%APPDATA%\Web2App` 未被创建）。
- **`quitting` 标志不可省**：托盘菜单「退出」走 `app.quit()`，它会触发窗口的 `close` 事件 ——
  没有这个标志就会被「关闭到托盘」分支拦下，表现为「点了退出，窗口只是藏起来了、进程还在」。
- **快捷键注册失败必须回滚**：Electron 的 `globalShortcut.register()` 不抛异常、只返回 boolean。
  不回滚的话会留下「旧的已注销、新的没生效」的空窗 —— 应用从此失去唤醒能力，
  而界面上还显示着旧快捷键（因为这次保存整体失败了）。
- **显隐语义不能只看可见性**：必须 `visible && !minimized` 才隐藏，否则最小化状态下第一次按
  快捷键会「没反应」（先隐藏、再按才显示）。
- 设置窗用 `useContentSize: true`：Tauri 的 `inner_size(480,470)` 说的是**内容区**，
  不写这个则窗口外框是 480×470、内容区少掉边框与标题栏的高度。

M3 剩余（不阻塞验收）：
1. **页面右键菜单里的「设置」入口**（`webview_hooks.rs:467` 有对应实现）—— 归入 M4 的「右键菜单」。
2. 托盘菜单项「退出」尚未有自动化断言（触发即退出进程，无法在同一进程内断言）；
   `before-quit` 置位的 `quitting` 逻辑已覆盖两条退出路径，靠人工验收。




---

### 8.8 M6 进展（打包已打通：三形态出包 + 两个真 bug 被出包过程逼出来）

2026-09-17：以「配置 `app_url=https://www.baidu.com`」为样本，端到端跑通
`build → stage → electron-builder → 实机启动 → 确认目标站点已加载`。

### 产物（输出目录 `electron-app/release/`）

| 文件 | 体积 | 说明 |
|---|---|---|
| `Web2App-0.1.0-win-x64.zip` | 147.6 MB | 解压即用（顶层含 `config.json`，随包一份） |
| `Web2App-0.1.0-win-x64-portable.exe` | 96.4 MB | 单文件自解压；运行时解压到 `%TEMP%`，数据仍写用户放置目录 |
| `win-unpacked/` | 372 MB | `dir` 形态（= zip 解压后的样子） |

### 出包过程暴露的两个真 bug（都不是打包配置问题）

**① `createWindow()` 漏传 `title` → 配置里的 title 静默失效。**
`page-title-updated` 被 `preventDefault()` 拦掉之后，窗口标题不会「退回成页面标题」，
而是**永远停在 Electron 默认值**（`package.json` 的 `name` = `web2app-electron`）。
任务栏、Alt+Tab、任务管理器里看到的都是这个值，而配置里的 `title`（如「百度」）完全没生效。
注释一直写着「窗口标题固定为配置里的 title」，但从 M1 起就只是**愿望**。
修法：`new BrowserWindow({ title: cfg.title, … })`，并在 `smoke` 里加一条硬断言
（`mainWindow.getTitle() === cfg.title`，不一致就让冒烟退出码非 0）。

**② tsc「有错也输出」+ 只跑 `typecheck` 不重跑 `build` → 把半成品 JS 封进 asar。**
现象是打包出来的 exe 一点就弹
`A JavaScript error occurred in the main process: ReferenceError: loadConfig is not defined`，
且**进程不退出**（错误框留着），只看到一个窗口都没有的僵尸进程。
根因：`tsc` 默认 `noEmitOnError: false`，编译报错时仍把改了一半的 JS 写进 `out/`；
之后只跑 `npm run typecheck`（通过）没跑 `build`，`electron-builder` 就把这份陈旧产物打进了 asar。
修法：`tsconfig` 开 `noEmitOnError: true`；`pack`/`dist` 脚本保持「先 build 再打包」的顺序。

> 教训（值得推广到所有 Electron 项目）：**打包前必须确认 `out/` 是本次源码编出来的**。
> 仅凭 `typecheck` 通过就认为产物是好的，是这次绕的最大一个弯。

### 打包相关的三个工程决策

**① `config.json` 随包分发（`extraFiles`，放 exe 同级而非 `resources/`）。**
由新增的 `scripts/stage-assets.cjs` 生成：项目根有 `config.json` 就原样复制，
没有就生成一份全字段默认模板（`extraFiles` 指向不存在的文件会让 electron-builder 直接报错退出，
而 `config.json` 是 gitignore 的运行期文件，干净仓库里本来就没有 —— 所以「一定有」必须由脚本保证）。
顺带解决 PLAN 里悬着的那个问题「绿色版 zip 要不要预置一份 config.json 模板」→ **要预置**。

**② 单文件形态必须能把包内预置的那份当回落。**
portable 的 `installRoot()` 指向**用户放置 exe 的目录**，而 exe 本身在 `%TEMP%` 解压副本里；
用户只拷一个 exe、没在旁边放 config 时，若不回落就会拿内置默认的 `example.com` 启动 ——
表现是「打包失败」，实际是少了份配置。故新增 `config::loadConfigAny([用户目录, 包内目录])`。
实测：`%TEMP%\w2a-portable-test2\Web2App.exe` 旁边**没有** `config.json`，
窗口标题仍是「百度」，`data\` 仍建在 exe 同级 → 回落与绿色性同时成立。

**③ 图标走脚本复制，不提交副本。**
`build/icon.ico` ← `../webview-app/icons/icon.ico`（单一真源，与注入脚本同理），
`build/` 整体 gitignore。

### electron-builder 26 配置变更（踩坑）

顶层 `zip:` 配置块**已被移除**，写了会直接
`Invalid configuration object … unknown property 'zip'` 而拒绝启动
（`TargetConfiguration` 只接受 `target` / `arch`）。zip 产物名改用顶层 `artifactName` 模板；
`portable.artifactName` 仍可用且优先。

### 实测证据链（不是「跑通了」三个字）

| 环节 | 证据 |
|---|---|
| 解压目录形态 | `--smoke` 打印 `入口目录=dist\win-unpacked`、`数据目录=…\data (mode=sibling)`、`配置文件=…\config.json`、`窗口标题='百度'`，退出码 0，无残留进程 |
| 单文件形态 | 拷到干净目录（旁无 config）→ 窗口标题 `'百度'`（连续 8 次采样 32s 稳定）、`data\` 建在 exe 同级 |
| **真的打开了百度** | `data\session` 里 Cache 命中 `https://www.baidu.com/`（含 logo、`sugrec` 接口）、`Code Cache/js` 里是百度脚本、`Local Storage\leveldb` 有 baidu 记录 |
| zip 结构 | 顶层条目含 `config.json`；`resources/{inject/{splash,titlebar,loading}.js,sidecar/w2a-proxy.exe,icons/icon.png}` 齐全 |

### M6 剩余（2026-09-18 结清：①实为已生效、②已落地、③已补文案）

1. ✅ **版本信息**（`VERSIONINFO`）—— 此前「待补版本信息」的判断**有误**：electron-builder
   本就会从 `package.json` 的 `version` / `description` / `author` 与 `electron-builder.yml`
   的 `copyright` 写全，无需额外配置。实测取证（`release-tabs\win-unpacked\Web2App.exe`）：
   `FileVersion=0.1.0`、`ProductVersion=0.1.0.0`、`ProductName=Web2App`、
   `FileDescription=Web2App`、`CompanyName=Web2App`、`LegalCopyright=Copyright © 2026 Web2App`、
   `InternalName=Web2App`。
   ⚠️ `CompanyName` 取自 `author.name`：`"author": "Web2App"`（字符串）能生效，是因为
   app-builder-lib 内部先经 `normalizePackageData` 把字符串归一化成 `{name, …}`；
   一旦改成 `{ "email": … }` 这类**没有 `name`** 的对象，公司名会**静默留空**（`!== null` 才写入）。
   改完请复跑一次取证命令。
   （`build/icon.ico` 之外的图标规格不做：两个 target 都只用到这一处图标。）
2. ✅ **日志目录落地** —— 新增 `electron-app/src/main/startup-log.ts`，落 `<dataDir>\logs\startup.log`
   （`data_dir_default=true` 时为 `%APPDATA%\<app_name>\logs\`）。契约见
   `electron-app/README.md`「日志目录」：fail-open、同步追加、1 MiB 轮转、不改变 stdout 行为。
   `smoke` 增加硬断言「日志已含 `[web2app] 启动信息`」（只判「文件存在」会被只写了表头的空文件骗过）。
   探针模式改写 `%TEMP%\w2a-probe-store\startup.log`，避免在交付产物旁留下 `data\logs\`
   （`data\session` 污染交付物已经踩过一次）。
   📌 **实证的深层动因**：文件日志不是"锦上添花"——用户双击 GUI 子系统 exe 时 `console.log`
   本就无处可去，没有这份文件，所有现场反馈都只能靠远程猜。
3. ✅ **SmartScreen 提示文案** —— 落 `electron-app/README.md` 的
   「首次运行与 SmartScreen（产物未签名）」一节：说明那是**预期行为**而非打包失败、
   给出两条根治路径（签名证书 / 组织白名单）、明确 ⛔ 不得引导用户关闭 SmartScreen。

---

### 8.9 自绘标题栏 × 页面滚动条（⚠️ 本节结论已更新）

2026-09-17，来自使用者的截图反馈：「滚动条显示不合理」——窗口右侧的纵向滚动条**从窗口最顶端**开始画，
裸露在标题栏右边那条 15px 的缝里，看上去像标题栏被切掉一角、滚动条穿过标题栏。

第一版把根因判为「fixed 元素的 `left:0;right:0` 只算到 `clientWidth`」，对应修法是标题栏改 `width:100vw`。
**该结论已被推翻**：`width:100vw` 能让 `rect.width` 全绿，但**可见**宽度仍只到 `clientWidth` ——
因为 Chromium 把**视口（根）滚动条绘制在所有 DOM 内容之上**（`z-index:2147483647` 也压不住），
其几何恒为「视口右缘 × **整个**视口高」；只要 `html` 是滚动容器，滚动条必然从 `y=0` 横穿标题栏。
同批被实测否掉的还有：px 钉宽、`::-webkit-scrollbar{border-top:38px}`（border 不参与零件布局）、
`scrollbar-width:none`（缝没了但滚动条也没了，使用者明确否掉「强制隐藏」）。

> ✅ **最终契约与实现见 `README.md` 的「自绘标题栏与页面滚动条」一节**：**`html` 不滚、`body` 滚**
> （`html{height:100%;overflow:hidden}` + `body{margin-top:38px;height:calc(100vh-38px);overflow-y:auto}`），
> 配套 `shimWindowScroll()` 把 window 级滚动 API 转发到 body、`ensureScrollRoot()` 三段探测失败则退回旧模式。
> 判据必须**成对**：`intrudeRows==[]` ∧ `bodyScrollbarW>0` ∧ `bodyScrollable` ∧ `window.scrollTo(0,200)→scrollY≈200`；
> **禁用** `rect.width==innerWidth` 当判据（它测不出「被绘制层盖住」）。

### 一个更值得记的教训：**断言把缺陷固化成了期望值**

探针里原本有这么一条：

```ts
// 标题栏是 position:fixed + left/right:0 → 宽度 = 视口宽度【减去纵向滚动条】。
// 拿 innerWidth 比会差一个滚动条宽（本机 15.3px），那是 WebView 的正常行为而非缺陷。
p.near('标题栏满宽（铺满可视区）', s.barRect?.w, s.view.cw, 1)
```

注释写得很笃定 ——「那是正常行为而非缺陷」，于是**缺陷被断言保护了起来**：
任何人把标题栏改宽都会被这条断言判红，反而没人去问「那条缝到底是什么」。
**当一条断言的期望值来自「实现当前的表现」而不是「使用者的观感」时，它就只是在锁死现状。**

改后的写法把判据换成真正想要的东西，并补一条前置条件防止它退化成恒真：

```ts
const sbw = s.view.iw - s.view.cw
p.check('plain 页确有纵向滚动条', sbw > 4, ...)   // 没有滚动条时这条断言本来也没意义
p.near('标题栏铺满整窗宽（含滚动条区域，不留缝）', s.barRect?.w, s.view.iw, 1)
```

### 新增取证工具 `npm run shot`（`scripts/layout-shot.cjs`）

这类「断言能过、人眼觉得不对」（或反过来）的布局问题，正好是纯数字断言的盲区。
该脚本：注入 titlebar.js → 打印 `innerWidth / clientWidth / scrollbarW / titlebarW` →
`webContents.capturePage()` 存一张 PNG 到 `%TEMP%\w2a-layout-shot.png`，退出码即判定。

⚠️ 用 `capturePage()` 而不是系统截屏：Chromium 走 DWM 合成，`CopyFromScreen` / `PrintWindow`
只会拿到黑图或下层窗口（Tauri 版那边早已踩过）。
