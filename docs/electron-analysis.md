# electron-app 方案分析：用 Electron 与 webview-app 对齐兼容

> 分析日期：2026-09-16 ｜ 精简：2026-09-22
> ⚠️ **决策已锁定（2026-09-16）：Electron 必做、仅 Windows、Win10+、体积不限、只出绿色版。**
> 原文「是否该做 Electron」的论证（§0/§1/§6/§7/§8/§9/§10）已被该决策覆盖，此处删除；
> 仅保留仍有效的**功能对等矩阵、复用策略、兼容性要点与安全基线**。
> **落地方案与里程碑状态以 `docs/electron-plan.md` 与 `electron-app/AGENTS.md` 为准。**

---

## 1. 功能对等矩阵（以 `webview-app/src/` 实际代码为基准）

| webview-app 模块                                            | 规模（2026-09-25 实测） | Electron 对应                                                                                                        | 迁移难度           | 关键说明                                                                                                                                              |
| --------------------------------------------------------- | --------- | ------------------------------------------------------------------------------------------------------------------ | -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `proxy.rs` 本地反向代理（HTTP+WS 注入 Basic、Cookie Domain 重写、流式转发） | 1247 行    | **原样复用（sidecar）**                                                                                                | ★ 极低           | 纯 Rust、零平台依赖。编成独立 sidecar exe，主进程 spawn、从 stdout 取端口。Chromium 同样有 localhost cookie 问题，代理不可省                              |
| `crates/resolve-core`（Ed25519 + AES-GCM + HKDF）           | 582 行     | **原样复用（sidecar）**                                                                                                | ★ 极低           | 随 sidecar 编译，无需 Node 侧重写                                                                                                                 |
| `resolve.rs` DNS TXT 查询                                   | 227 行     | 侧车承担                                                                                                             | ★ 低            | 挪进 sidecar 最省事（实际实现）                                                                                                                          |
| 注入脚本 `titlebar.js` / `loading.js` / `dnd_polyfill.js`     | ~70 KB（2026-09-27 复核实测：titlebar 60,236B + loading 22,653B ≈ **81KB**，dnd 另计；量级判断不变）    | preload + `executeJavaScript`（主世界）                                                                                | ★★ 低，**可直接复用** | `whenHostReady` 兜底可留可删；不存在 ACL/capability 概念；`dnd_polyfill.js` 是 WebView2 专属 bug，**不引入** |
| 无边框窗口 + 自绘标题栏                                             | —         | `frame:false` + 38px 契约 + `applyChromeFit()` 全套保留                                                                | ★ 低            | 不重写 CSS 契约，逐像素对齐 Tauri 版                                                                                                                    |
| 窗口按钮 IPC                                                  | —         | `ipcMain` / `ipcRenderer`                                                                                          | ★ 低            | **「remote.urls 必须写 `:*`」的 ACL 坑整条消失**                                                                                                          |
| 多页签 `tabs.rs`                                             | 956 行     | `WebContentsView`（`BrowserWindow` 当壳 + 子视图）                                                                       | ★★ 中           | 几何自动跟随窗口，无需 owned window 校准；落地选型 A′（见 `electron-tabs-design.md`）                                                        |
| `webview_hooks.rs`（新窗口/右键/下载/鉴权）                          | 763 行     | `setWindowOpenHandler` / `context-menu` / `will-download` / `app.on('login')`                                      | ★★ 低          | 全是一等 API，无 COM、无后台线程轮询                                                                                                                  |
| 下载 + 另存为 `save_dialog.rs`                                 | 127 行     | `session.on('will-download')` + `dialog.showSaveDialog`                                                            | ★ 低            | 不用碰 `IFileSaveDialog` / STA / COM（**M4 已于 2026-09-24 实现**：本壳直接用 `dialog.showSaveDialog`，同样不碰 COM）                                                                                       |
| `tray.rs` / 全局快捷键 / 单实例                                   | 115 行     | `Tray` / `globalShortcut` / `requestSingleInstanceLock`                                                            | ★ 低            | 标准 API                                                                                                                                            |
| `settings.rs` + `dist/settings.html`                      | 328 行     | BrowserWindow + JSON（设置页 HTML **一行不改**，用 `__TAURI_INTERNALS__` 垫片）                                          | ★ 低            | 设置页单一真源，改一次两壳同时生效                                                                                                                        |
| `window_state.rs`                                         | 114 行     | 手写（物理像素存盘 / DIP 恢复）                                                                                         | ★ 低            | 与 Tauri 版可互相读取（探针逐字节比对）。几何已并入 `store.rs` 的 `window` 段（`store.rs` 681 行）                                                 |
| `bootstrap.rs` / `loading.rs` / `lib.rs` 编排               | ~1000 行（2026-09-27 复核实测三文件合计约 1,560 行，旧估计偏小；不影响「★★中」结论）   | 主进程重写（`index.ts` / `navigate.ts`）                                                                              | ★★ 中           | 逻辑本身不复杂，但要重新组织启动时序                                                                                                                                |
| `.github/workflows/build-custom.yml`                      | **约 57 KB**（2026-10-12 实测 57.4 KB；2026-09-26 为 52.9 KB、决策时为 62 KB，随迭代浮动） | electron-builder 打包链重做                                                                                          | ★★★★ **高**     | 最大的隐藏成本：config JSON 注入、多形态产物、签名取密钥；Electron 侧已由 `stage-assets.cjs` + `electron-build-custom.yml` 承担，**Android 那半没有对应物** |

**净账（决策时估算）**：约 3000 行 Rust 重写为 TS，约 1500 行 Rust + 注入脚本 + HTML 直接复用，外加一条打包流水线。
实际落地：sidecar（`crates/proxy-server`）以 `#[path]` 零重写复用 `proxy.rs` / `resolve.rs`，见 `electron-app/AGENTS.md`「本地代理 sidecar」。

> ⚠️「规模」列是 2026-09-25 `wc -l` 实测值，仅供量级参考，会随版本演进；不要拿它当契约。

---

## 2. 复用策略

### 方案 B（已采纳）：Electron 壳 + Rust sidecar

- 代理与加密**零重写**，同时保住 Android 与 CLI（`resolve-tool`）既有资产；
- 代价：多一个进程，要处理 sidecar 生命周期与端口握手（stdout 单行 JSON）；
- 打包时 `extraResources` 带上 sidecar（**不能进 asar**，asar 内 exe 无法 spawn）。

### 其他方案（已否决）

| 方案 | 否决原因 |
|---|---|
| 方案 A：全 JS/TS 重写 | 把 741 行已测代理与整条加密链路换成未测实现，还要重做端到端测试 |
| 方案 C：napi-rs 原生模块 | 升级 Electron 要重编 ABI，CI 矩阵变复杂（各平台 × 各架构） |

---

## 3. 兼容性设计要点（让两个壳对外表现一致）

1. **配置契约单一真源**：沿用现有 **21 个字段**（`app_url` / `app_name` / `title` / `version` / `extra` / `icon_url` / `proxy_*` / `data_dir` / `tabs_enabled` / `mobile_bar` / `debug` / `public_key` / `resolve_secret` / `android_*`×4 签名字段 / `android_permissions`（仅 Android，本壳接受但忽略），完整清单与计数以 `packaging-config.md` 为准）。⚠️ 布尔**只认真 `bool`**，字符串写法已拒绝（旧文「Electron 侧宽容接受字符串」已失效）。
2. **注入脚本物理复用**：`titlebar.js` / `loading.js` / `shell.html` / `settings.html` 从 `webview-app/` 引用（构建期拷贝），避免两份分叉。
3. **行为契约写成断言测试**：38px 顶部留白、标题栏按钮、Basic 鉴权覆盖 HTTP+WS、`Set-Cookie` Domain 重写、`target="_blank"` 策略、快捷键、托盘语义、单实例、绿色版免安装 —— 全部收敛进 `pnpm run probe`：**单窗口 138 条（2026-09-27 实测；v1.76 起 150 条）**；多页签另 **33 条**（v1.58 起 39 条）。
4. **产物命名对齐**：与 `build-custom.yml` 既有约定一致，让部署脚本不感知壳的差异。
5. **两处已知语义落差**（必须显式处理）：
   - 外部 HTTP/HTTPS 代理：Electron 侧用 Chromium `--proxy-server` switch（已接好）；
   - 新窗口拦截：Tauri 侧注入脚本 + `SetHandled(true)`，Electron 侧 `setWindowOpenHandler` **主进程决策**，注入脚本那段要删掉而不是照搬。

---

## 4. 安全基线（Electron 侧必须保持）

Tauri 的 ACL/`capability` 默认拒绝；Electron 没有等价物，默认配置对「加载远程页面」并不安全。已落实：

- `contextIsolation: true`、`nodeIntegration: false`、`sandbox: true`、`webSecurity: true`（保持默认）；
- 禁用 `enableRemoteModule` / `@electron/remote`；
- 远程页面只能经 preload 用 `contextBridge` 暴露**最小 API 面**（`window.web2app` 白名单，不暴露 `ipcRenderer` 本体）；
- `setWindowOpenHandler` 一律 `deny` 后自行决策（单窗口原地导航 / 多页签新开 tab）；
- 禁用 `webPreferences.webviewTag`，不用 `<webview>` 标签；
- 注入脚本经 `webFrame.executeJavaScript` 在**主世界**执行（隔离世界看不到 contextBridge，按钮点不动）；
- 信任边界：代理只绑 `127.0.0.1`，凭据不得跨域外链泄漏。
