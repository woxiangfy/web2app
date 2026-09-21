# electron-app 方案分析：用 Electron 与 webview-app 对齐兼容

> ⚠️ **2026-09-16 更新：决策已定 —— Electron 必做、仅 Windows、Win10+、体积不限、只出绿色版。**  
> 本文件 §0/§1/§7/§9 中「是否该做 Electron」的论证**已被该决策覆盖**，仅作历史记录；  
> §2 功能对等矩阵、§3 复用策略、§5 安全基线仍然有效。  
> **落地方案请以同目录的 `electron-plan.md` 为准。**



> 分析日期：2026-09-16 ｜ 对象：`electron-app/`（**分析时为空的目录；后已完整交付 M0–M3、M5、M6**）＋ 已有的 `webview-app/`（Rust + Tauri 2，5600 行 Rust / 70KB 注入脚本）  
> 待确认前提：**「以满足依赖环境的需求」** —— 本报告按最可能的一种解释展开（见 §0），另附其它解释的分支判断。

---

## 0. 先把「依赖环境」拆开

这句话至少有三种读法，指向完全不同的结论：

| #     | 读法                                                               | 现状证据                                                                                                                                      | 是否该上 Electron                           |
| ----- | ---------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------- |
| **A** | 目标机器**没有 / 装不上 WebView2 Runtime**（内网离线、精简版 LTSC、非管理员、云桌面/服务器核心版） | `tauri.conf.json` **没有配 `webviewInstallMode`** → 默认 `downloadBootstrapper`，安装时要联网下载运行时；`build-custom.yml` 的绿色版 zip 注释也写着「依赖系统自带 WebView2」 | ❌ **不必**，Tauri 有三档现成解法（§1），零代码改动        |
| **B** | 目标机器 WebView2 **版本太旧**，站点前端特性/媒体能力跑不起来                           | 系统运行时随 Edge 更新，不可控                                                                                                                        | ❌ **不必**，`fixedRuntime` 锁版本即可           |
| **C** | 要求**完全自包含 / 合规不许依赖系统组件**；或目标含 **Win7/8**                         | —                                                                                                                                         | ⚠️ 只有 C 的前半段是 Electron 的正解；Win7 段见 §1.4 |

> **我的判断**：如果是 A 或 B，Electron 是「用一次重写去解决一个 3 行配置就能解决的问题」。请先确认是哪一种，再决定是否进入 §2。

---

## 1. 如果动机是 WebView2 依赖：先看这三档零代码解法

Tauri 2 的 `bundle.windows.webviewInstallMode`：

| 模式                     | 需要联网  | 安装包增量       | 说明                              |
| ---------------------- | ----- | ----------- | ------------------------------- |
| `downloadBootstrapper` | 是     | 0 MB        | **当前默认**（未显式配置）                 |
| `embedBootstrapper`    | 是     | ~1.8 MB     | 引导器内嵌，安装体验更稳                    |
| **`offlineInstaller`** | **否** | **~127 MB** | 内嵌 WebView2 完整离线安装器，**推荐给离线环境** |
| **`fixedRuntime`**     | **否** | **~180 MB** | 内嵌一份**固定版本**运行时目录，版本可控、可随绿色版一起发 |
| `skip`                 | 否     | 0 MB        | ⚠️ 运行时缺失就不工作                    |

```jsonc
// tauri.conf.json —— 只有这一处改动
"bundle": { "windows": { "webviewInstallMode": { "type": "fixedRuntime",
  "path": "./Microsoft.WebView2.FixedVersionRuntime.<ver>.x64/" } } }
```

### 1.1 一个反直觉但关键的结论

**「Electron 体积小」这个常见论据，在「必须自包含」这个前提下不成立。**

- 系统已有运行时 / 允许联网：Tauri 绿色版 exe ≈ **10–15 MB**（见 `Cargo.toml` 的 `opt-level=3` 注释），Electron 空壳 nsis ≈ **80–100 MB**，差 5–10 倍。
- 必须自包含：Tauri `fixedRuntime` **+180 MB** vs Electron 自带 Chromium ≈ **150–200 MB** —— **基本抹平**。

所以一旦动机是 A/C（自包含），真正的差异不再是体积，而是下方这几项：

| 维度            | WebView2 + fixedRuntime         | Electron 44（Chromium 150 / Node 24）       |
| ------------- | ------------------------------- | ----------------------------------------- |
| 安装包量级         | ~180 MB                         | ~150–200 MB                               |
| 空闲内存          | 单实例增量 ~60–120 MB（与 Edge 共享运行时）  | ~150–300 MB（主进程 + 渲染 + GPU + utility 多进程） |
| 冷启动           | 更快（但本项目还要等 DNS 解析 + 代理就绪，差异被掩盖） | 慢 0.5–1.5 s                               |
| **安全补丁责任**    | **由 Windows/Edge 更新**，你不必跟进     | **你自己**，Chromium 每约 4 周一个大版本，要持续升级并重测     |
| 依赖矩阵          | 系统 WebView2（可用 fixedRuntime 顶掉） | 无系统依赖，但自带一整套运行时                           |
| Android / iOS | ✅ 一套 Rust 代码同时出 APK             | ❌ **完全不支持**（见 §6）                         |

### 1.2 Win7/8 是死路（两个方向都是）

- WebView2 / Chromium 109 之后不再支持 Win7/8/8.1。
- **Electron 从 v23 起也移除了 Win7/8/8.1 支持**；能跑 Win7 的最后一版是 Electron 22，已停止维护、无安全补丁。

结论：若目标含 Win7，正确动作是推系统升级或找替代终端，而不是换壳。

---

## 2. 若确定要 Electron：功能对等矩阵

以 `webview-app/src/` 实际代码为基准逐模块盘点。

| webview-app 模块                                            | 规模        | Electron 对应                                                                                                        | 迁移难度           | 关键说明                                                                                                                                              |
| --------------------------------------------------------- | --------- | ------------------------------------------------------------------------------------------------------------------ | -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `proxy.rs` 本地反向代理（HTTP+WS 注入 Basic、Cookie Domain 重写、流式转发） | 741 行     | **原样复用**                                                                                                           | ★ 极低           | 纯 Rust、零平台依赖。编成**独立 sidecar exe**，主进程 spawn、从 stdout 取端口即可。**这是整个迁移里最值钱的一步**——Chromium 同样有 localhost cookie 问题，代理不可省                              |
| `crates/resolve-core`（Ed25519 + AES-GCM + HKDF）           | 573 行     | **原样复用**                                                                                                           | ★ 极低           | sidecar 或 napi-rs 原生模块（甚至可编 WASM）                                                                                                                 |
| `resolve.rs` DNS TXT 查询                                   | 208 行     | Node `dns.resolveTxt` + 侧车验签                                                                                       | ★ 低            | 拆两半：查询用 Node，验签解密回 Rust                                                                                                                           |
| 注入脚本 `titlebar.js` / `loading.js` / `dnd_polyfill.js`     | ~70 KB    | preload + `executeJavaScript`                                                                                      | ★★ 低，**可直接复用** | Electron 的 `dom-ready` / `did-finish-load` 时机远好于 WebView2 的 document-created，`whenHostReady`+MutationObserver 那套兜底基本可删；**且不存在 ACL/capability 概念** |
| 无边框窗口 + 自绘标题栏                                             | —         | `frame:false` + `-webkit-app-region: drag`，或 `titleBarStyle:'hidden'`+`titleBarOverlay`（Win11 拿到原生按钮+Snap Layouts） | ★ 大幅简化         | 不需要 `shadow(false)` + DPI inset 闭环校准那套                                                                                                            |
| 窗口按钮 IPC                                                  | —         | `ipcMain` / `ipcRenderer`                                                                                          | ★ 大幅简化         | **Tauri 那条「远程 origin 强制 ACL、`remote.urls` 必须写 `:*`」的坑整条消失**                                                                                       |
| 多页签 `tabs.rs`                                             | 872 行     | `WebContentsView`（同窗口内子视图）                                                                                         | ★★★ 中          | 几何自动跟随窗口，**不需要** owned window + `place_tab` 3 轮补偿 + `destroy()` 防重入；但标签栏逻辑要重写（`shell.html` 可复用）                                                   |
| `webview_hooks.rs`（新窗口/右键/下载/鉴权）                          | 757 行     | `setWindowOpenHandler` / `context-menu` / `will-download` / `webRequest`                                           | ★★ 大幅简化        | 全是一等 API，**不用** COM、不用后台线程轮询 `with_webview`、不用 `HOOKS_REGISTERED` 去重闸门                                                                            |
| 下载 + 另存为 `save_dialog.rs`                                 | 127 行     | `session.on('will-download')` + `dialog.showSaveDialog`                                                            | ★ 低            | 不用自己调 `IFileSaveDialog`、不用管 STA/COM 非 Send                                                                                                        |
| `tray.rs` / 全局快捷键 / 单实例                                   | 115 行     | `Tray` / `globalShortcut` / `requestSingleInstanceLock`                                                            | ★ 低            | 标准 API                                                                                                                                            |
| `settings.rs` + `dist/settings.html`                      | 328 行     | BrowserWindow + JSON                                                                                               | ★ 低            | HTML 直接复用                                                                                                                                         |
| `window_state.rs`                                         | 143 行     | 手写或 `electron-window-state`                                                                                        | ★ 低            | —                                                                                                                                                 |
| `bootstrap.rs` / `loading.rs` / `lib.rs` 编排               | ~1000 行   | 主进程重写                                                                                                              | ★★ 中           | 逻辑本身不复杂，但要重新组织启动时序                                                                                                                                |
| `.github/workflows/build-custom.yml`                      | **62 KB** | electron-builder / electron-forge 重写                                                                               | ★★★★ **高**     | **最大的隐藏成本**：内含 config JSON 注入、MSI + 绿色版 zip、Android 签名三层取密钥。Electron 侧要重做一份，Android 那半没有对应物                                                       |

**净账**：约 **3000 行 Rust 需重写为 TS**，约 **1500 行 Rust + 70 KB 注入脚本 + 2 个 HTML 可直接复用**，外加**一条新的打包流水线**。

---

## 3. 复用策略（决定这个方案值不值）

### 方案 B（推荐）：Electron 壳 + Rust sidecar

```
electron-app/
├── src/main/            # 主进程：窗口/托盘/快捷键/下载/多页签
├── src/renderer/        # preload + 复用 webview-app/src 下的注入脚本
├── sidecar/             # 由 webview-app 的 Rust 代码编出：w2a-proxy.exe（含 resolve-core）
└── shared/              # config.json schema、行为契约测试
```

- 代理与加密**零重写**，同时**保住 Android 与 CLI（`resolve-tool`）的既有资产**；
- 代价：多一个进程、要处理 sidecar 生命周期与端口握手（stdout 单行 JSON 即可）；
- 打包时 `extraResources` 带上 sidecar，签名时一起签。

### 方案 C：napi-rs 原生模块

把 `resolve-core`（+ 代理内核）编成 `.node`，无子进程、调用开销低；但升级 Electron 时要**重编 ABI**，CI 矩阵变复杂（各平台 × 各架构）。

### 方案 A：全 JS/TS 重写

**不推荐**。代理的流式转发/WS 握手/Cookie 重写用 Node 重做一遍，等于把 741 行已测代码换成一个未测实现，还要重做端到端测试；加密层重写更会引入密钥/协议风险。

> 明确建议：**无论走哪条，`proxy` + `resolve-core` 不做 JS 重写。**

---

## 4. 兼容性设计要点

要让两个壳对「部署脚本 / 运维 / 用户」表现一致：

1. **配置契约单一真源**：沿用现有 `config.example.json` 的 17 个字段（`app_url`/`app_name`/`title`/`extra`/`proxy_*`/`tabs_enabled`/`public_key`/`resolve_secret`/…），抽一个壳无关的解析层（可放 Rust sidecar 或共享 JSON Schema + 测试）。
2. **注入脚本物理复用**：`titlebar.js` / `loading.js` / `dnd_polyfill.js` / `shell.html` / `settings.html` **从 `webview-app/src/` 引用**（软链或构建期拷贝），避免两份分叉——这是保持「兼容」最容易被忽略也最容易腐烂的地方。
3. **行为契约清单（要写成断言测试）**：顶部留白 38px；标题栏按钮行为（最小化/最大化/关闭/前进/后退）；Basic 鉴权注入覆盖 HTTP+WS；`Set-Cookie` Domain 重写；`target="_blank"` 策略；默认快捷键 `Alt+4`；`Shift+F1` 打开设置；托盘左键显隐、关闭行为受设置控制；单实例；绿色版免安装。
4. **产物命名对齐**：安装包 / 绿色版 zip 的文件名规则与 `build-custom.yml` 现有约定一致，让部署脚本不感知壳的差异。
5. **两处已知语义落差**（必须显式处理，不能想当然）：
   - 「外部 HTTP/HTTPS 代理」现在走 WebView2 的系统代理设置 → Electron 侧是 `session.setProxy({proxyRules})`，**作用域与生效时机不同**；
   - 新窗口拦截：Tauri 侧是注入脚本 + `SetHandled(true)`，Electron 侧是 `setWindowOpenHandler` **主进程决策**，注入脚本那段要删掉而不是照搬。

---

## 5. 安全基线（Electron 侧必须从零做一遍）

Tauri 的 ACL/`capability` 是**默认拒绝**的；Electron 没有等价物，默认配置对「加载远程页面」这件事**并不安全**。必须自查：

- `contextIsolation: true`、`nodeIntegration: false`、`sandbox: true`、`webSecurity: true`（全部保持默认，不许为了省事打开）；
- **禁止** `enableRemoteModule` / `@electron/remote`；
- 远程页面只能通过 preload 用 `contextBridge` 暴露**最小 API 面**（白名单方法，不暴露 `ipcRenderer` 本体）；
- `setWindowOpenHandler` 一律 `{action:'deny'}` 后自行决策；`will-navigate` / `webRequest` 做域名白名单；
- **禁用** `webPreferences.webviewTag`，不要用 `<webview>` 标签；
- CSP 现在是 `null`（`tauri.conf.json` 的 `app.security.csp`），Electron 侧建议给出显式 CSP；
- 把「远程页面 + 本地代理」的信任边界重新画一遍：代理只绑 `127.0.0.1`（沿用现有实现），凭据不得跨域外链泄漏（沿用 `strip_userinfo` / `no_proxy()` 的既有基线）。

---

## 6. 最大的战略成本：Android 与三 crate 结构

`webview-app` 现在最大的架构优势是：**同一份 Rust 代码同时产出 Windows 应用和 Android APK**，鉴权代理零平台依赖所以在两端共用。

- Electron **不支持移动端**，这条路直接断掉；
- 若 Android 仍是交付物，就变成「Tauri-Android 壳 + Electron-Windows 壳」双壳维护，且 Rust sidecar 在 Android 上要编 `.so` + JNI 才能在 Electron 里……而 Android 上没有 Electron，等于 sidecar 在移动端反而要另一套接入方式；
- `resolve-tool`（服务端 CLI）不受影响，这条资产安全。

**这一项如果成立，就足以否掉全量迁移，只保留「Windows 专用壳」定位。**

---

## 7. 决策树

```
「依赖环境」具体卡在哪？
├── 目标机没装 WebView2 / 离线装不上 …… 改 webviewInstallMode = offlineInstaller  →  收工（3 行）
├── WebView2 版本太旧 …………………… fixedRuntime 锁版本                          →  收工（3 行）
├── 目标是 Win7 / 8 …………………………… Electron ≤22 已 EOL、最新版也不支持        →  推系统升级
├── 要求完全自包含 / 合规不许用系统组件
│     ├── 先试 fixedRuntime 绿色版（+180MB，免管理员、不写系统）
│     └── 仍不满足 …………………………… Electron 正解 → 按 §3「方案 B」做，Windows 专用壳
└── 只是觉得 WebView2 坑多 / 想换栈 …… 「依赖环境」不成立，先量化迁移账再谈
```

---

## 8. 建议路线

**第一步（半天）**：在**目标机**上确认到底是不是 A/B。检测 WebView2 是否安装与版本：

```powershell
# WebView2 Runtime 的 pv 值；不存在 = 未安装
Get-ItemProperty 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}' -EA SilentlyContinue | Select pv
Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}' -EA SilentlyContinue | Select pv
```

**第二步**：A/B → 直接改 `webviewInstallMode`（+§1 的绿色版固定运行时），一天内可交付，**不动一行业务代码**。

**第三步**：只有「合规要求自包含」或「后续明确要放弃 Android」时，才启动 Electron。启动方式不是全量重写，而是一个 **2 天 spike**：

> spike 范围（其余一律不做）：无边框窗口 + 自绘标题栏（复用 `titlebar.js`）→ 启动 Rust sidecar 并接入代理（复用 `proxy.rs`）→ Basic 鉴权注入生效 → 托盘 + 全局快捷键 + 单实例 → 下载另存为 → 量测 **安装包体积 / 空闲内存 / 冷启动 / 独立窗口内存**。
>
> spike 验收：能用 `config.json` 打开一个带 Basic 鉴权的真实站点，且窗口按钮、托盘、快捷键行为与 webview-app 一致。**不碰多页签、不碰 Android、不碰打包流水线。**

**第四步**：拿 spike 的实测数据（体积/内存/工时）与「改 3 行配置」的方案做对比，再决定是否全量。若全量，按 §3 方案 B + §4 契约 + §5 安全清单推进，并把 `build-custom.yml` 的 Electron 版本作为独立工作项排期。

---

## 9. 结论

1. **技术可行性**：高。Electron 能覆盖 webview-app 的全部 Windows 功能，且有一半以上模块在 Electron 下**更简单**（无 ACL、无 COM 钩子、无 owned window、无 DPI 闭环校准、无注入时机竞态）。
2. **但「以满足依赖环境的需求」这个理由，大概率不成立**：Tauri 自带 `offlineInstaller` / `fixedRuntime` 两档，零代码改动即可覆盖「离线 / 运行时缺失 / 版本不可控」，正是 A、B 两类问题的正解。
3. **一旦自包含，体积优势抹平**；真正的差异转移到**内存占用、安全补丁维护责任、以及 Android 能力的丧失** —— 这三项里没有一项是 Electron 占优的。
4. **不建议全量重写**。若确需 Electron，请限定为「Windows 专用壳」，并**务必用 sidecar 复用 `proxy.rs` + `resolve-core`**，否则会把 741 行已测代理和整条加密链路换成未测实现，得不偿失。
5. **`build-custom.yml`（62 KB）是本次评估里最容易被低估的成本项**，任何 Electron 方案都必须为它单列排期。

---

## 10. 待你确认

1. 「依赖环境」具体是 §0 的 **A / B / C** 哪一种？目标是 Win10+ 还是含 Win7？
2. Android APK 还继续交付吗？（这一条直接决定 Electron 是全量替代还是仅 Windows 分支）
3. 是否接受「安装包 +127/+180 MB」换取零代码改动？还是体积硬性上限更优先？
