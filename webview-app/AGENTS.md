# Web2App（Rust + Tauri 2）

> 把一个网址打包成桌面 / 手机原生应用的轻量工具，支持 **Windows（WebView2）** 与 **Android**。
> 内嵌打开预设网址、托盘最小化、全局快捷键、多页签、自绘标题栏、下载接管与设置页；
> `https://user:pass@host` 形式的 Basic 鉴权由**本地反向代理**给每个请求（含 WebSocket）注入凭据。
>
> 仓库级规则、目录地图与文档索引见根 `AGENTS.md`。Windows 免安装绿色版由另一个壳
> `electron-app/`（Electron + TS）交付，两者共用同一份注入脚本（清单与例外见根 `AGENTS.md` §3.4.1）与 `config.json` 契约。

## 硬约束速览（不知道就会踩坑）

1. **单一真源**：分三类（详见 `../AGENTS.md` §3.4.1）——
   **真共用**（Electron 打包时从本工程复制）：`src/{titlebar.js,loading.js,shell.html,splash.html}`、`dist/settings.html`、
   `icons/*`；**行为对齐但实现不同**：Ctrl+W 关页签（`tab_keys.js` vs Electron `before-input-event`）、页签标题回传（`tab_title.js`）；
   **单壳专属**：`tab_title.js` / `tab_keys.js` / `tab_doc_ready.js` / `dnd_polyfill.js`（WebView2/Tauri 注入，Electron 不消费）、
   `mobilebar/*.js`（仅 Tauri 移动端；5 个分片由 `lib.rs` 按序拼成同一个 IIFE）、`floaticon.html`（仅 Electron）。
   `src/{proxy.rs,resolve.rs}` 经 sidecar `#[path]` 同源复用 —— 改一必双壳验证。
2. **测试门禁**：改动后 `cargo test --all` 必须全绿（当前 324 通过 / 6 ignored）；`src/contract_tests/`（按领域 14 个子模块）
   是静态契约守卫 —— 改 `capabilities/*.json` 或注入脚本的事件名会被它拦下。
3. **配置是编译期常量**：所有运行期参数在 `src/config.rs` 由 `option_env!` 注入，**改完必须重新编译**；
   打包 workflow 通过环境变量注入。全文见 [docs/tauri-config.md](docs/tauri-config.md)。
   ⚠️ 例：`const PROXY_BYPASS: &str = env_str!("PROXY_BYPASS", "localhost;127.0.0.1;<-loopback>")`
   的默认串是跨壳**单一字面量契约**，10 个入库文件必须一致（清单见 `../docs/packaging-config.md` §4.4，
   `scripts/pack-local.mjs --selftest` 逐文件钉住）。
4. **Android 补丁链以打包 workflow 为唯一权威**：`build-custom.yml` 与 `release.yml` 的调用顺序必须
   **同序**（当前 prepare-wry-kotlin → insets → back → download → extras → schemes → fullscreen →
   permissions）。「哪个脚本有没有被接线」由 `contract_tests/android_pipeline_contract.rs` ⑧
   **按目录派生**（不用手写名单）：新增/删除补丁脚本不必改测试，漏接线会自动红灯。
5. **悬浮球建窗必须显式 `.data_directory(config::data_dir(app))`**（v1.71 真机根因，有契约守卫）。
6. **设置页命令名三处逐字一致**：`settings.html` 字面量 / `settings-preload.ts` 白名单 / 主进程 IPC 名 —— 漏一处即「按钮点了没反应」。
7. **`settings.json` 三个写入方一律读-改-写**（设置页保存 / 主窗几何 / 悬浮球几何），否则互相覆盖；写盘为原子写。
8. **改完注入脚本必须重新打包**：脚本随资源进包，跑旧产物永远看不到修复。
9. **唤醒快捷键判据两壳同一套**：`settings.rs::hotkey_problem` ↔ Electron `hotkey.ts::hotkeyProblem`
   （未设置 / 裸单键 / 系统占用 7 键 / `Shift+F1` / 无法映射），系统占用表在 Rust、Electron、
   共用设置页**三处逐字一致**；`contract_tests/settings_hotkey_contract.rs` ⑥⑦ 钉住漂移。
   同一判据必须同时用于「保存」和「启动回退」—— 旧配置被判非法时按「不设置」处理并在设置页提示，
   别只校验保存路径（否则旧裸键会照旧注册、占满用户输入）。
10. **下载页签自动关闭（多页签）判据只有「是否提交过真实文档」**：`committed` 由
   `tab_doc_ready.js` 回传（本地占位页 `tauri.localhost` 必须早退），`DownloadStarting` 在
   取消时立即关、确认保存时等 `StateChanged` 终态再关 —— **销毁发起下载的 webview 会中断
   下载且不可续传（UserShutdown）**。首页与已提交文档的页签不关；关闭走
   `close_tab_after_download`（后台线程 + `run_on_main_thread` 延后）。任一环被改坏都是
   静默失效，`contract_tests/capability_contract.rs` ⑨ 与 `tabs/tests.rs` 单测守着。
   **另：`window.open()` 在本壳返回 null**（`NewWindowRequested` 被 `SetHandled` 收走），
   站点常据此走 `location.href` 同地址兜底 ⇒ 一次点击两次下载、两个「另存为」。必须在
   `NewWindowRequested` **同步**登记目标 URL + 打开者（晚了就认不出），并在
   `DownloadStarting` **弹框前** `should_drop_popup_fallback` 取消第二条（同地址 + 同打开者
   + 3s）。⚠️ 这是**事后判重**（兜底地址带新签名时不命中），只作兜底：Electron 侧已从根上
   改为 `allow` + 隐藏窗口让 `window.open()` 返回非 null，且该窗口**活到下载结算**
   （`tab-spawn.ts` 的 `did-create-window` + `tabs.ts::attachHiddenPopupToTab`），本壳若要
   等价需 `put_NewWindow` 另建一个 WebView2，**尚未做**。

## 功能特性

| 功能 | 平台 |
| --- | --- |
| 内嵌网址 · 设置界面（Shift+F1）· 托盘 · 全局快捷键（默认不启用）· 下载接管 · 登录密码保存 · 窗口几何记忆 · 拖拽 polyfill · F12 DevTools | Windows |
| Basic 鉴权 + WebSocket 鉴权（本地反向代理统一注入）· Cookie 会话保持 | Windows / Android |
| 自绘标题栏（38px 通栏；前进/后退带落点守卫 v1.66）· 单实例 · 外部 HTTP/HTTPS 代理 · 新窗口拦截 | Windows |
| 多页签 `TABS_ENABLED`（默认关；首页页签固定第一位、不可关闭；下载页签自动关闭） | Windows（Android 为底栏页签记录形态） |
| 移动端底栏 `MOBILE_BAR`（默认关，仅移动端；含「更多」面板） | Android |
| 桌面悬浮图标 `float_enabled`（56×56 透明置顶小球；单击切换主窗显隐、右键关闭并唤回） | Windows |
| 启动过渡（启动页 + 加载层 + 失败卡） | Windows / Android |
| Android 原生补丁：返回键 · 下载接管 · 外部 scheme 外跳 · 更多面板 Kotlin 桥 · H5 视频全屏 · 运行时权限 · `target=_blank` 进移动页签 | Android |

> 「登录密码保存」两壳**机制不同**：本壳走 WebView2 原生（`ICoreWebView2Settings4::IsPasswordAutosaveEnabled`，`src/webview_hooks.rs`），
> Electron 壳为自研（preload 采集 / 填充 + `safeStorage`，见 `../electron-app/AGENTS.md` 硬约束 18）。
> **不得在 Tauri 侧再注入 Electron 那段** —— WebView2 已原生弹一次，会双弹窗。

- 鉴权链路原理与凭据主机白名单：[docs/proxy-auth.md](docs/proxy-auth.md)
- 设置窗口版式、6 个配置项与 `settings.json` 落点/原子写：[docs/settings-ui.md](docs/settings-ui.md)
- 悬浮球（形态 / 单击 toggle / 拖动吸附 / 圆形命中 / 逐轮演进 / `--float-probe`）：[docs/float-ball.md](docs/float-ball.md)
- Android（构建准备 / 补丁脚本与补丁链契约 / 底栏 / 启动排障史 / 数据目录）：[docs/android.md](docs/android.md)
- 8 条 CI workflow / 打包字段 / 动态解析与安全模型 / Android 签名：[docs/workflows.md](docs/workflows.md)
- 打包配置 JSON 的 21 个字段契约（跨壳唯一权威）：`../docs/packaging-config.md`

## 编译与运行

```bash
cargo run                                                      # 开发运行（Windows）
cargo build --release                                          # 发布构建（target/release/）
cargo install tauri-cli --version "^2" && cargo tauri build     # 打包 MSI / NSIS
cargo test --all                                               # 单元测试 + 静态契约守卫
```

Windows / Android 的完整环境要求、Android 构建命令、测试分组计数、`--float-probe` 探针与常见问题，
以及 `src/` 目录结构树（含 `src/nav.rs` 导航 / 窗口会话登记数据枢纽、`src/bootstrap.rs` 启动编排
的职责划分：功能模块一律 `→ nav`，`bootstrap` 只留编排并经 nav 的 boot 回调对外；`floatwin` / `tabs` /
`proxy` 三个大模块已按功能垂直拆为根文件 + `src/xxx/` 子模块）—— 见 [docs/build-test.md](docs/build-test.md)。

## 专项文档

| 文档 | 内容 |
| --- | --- |
| [docs/build-test.md](docs/build-test.md) | 环境要求、Windows / Android 构建命令、测试与计数、悬浮球探针、常见问题、目录结构 |
| [docs/tauri-config.md](docs/tauri-config.md) | `src/config.rs` 编译期常量全文 + 诊断日志（`debug` 开关）契约 |
| [docs/proxy-auth.md](docs/proxy-auth.md) | 本地反向代理工作原理、入口密钥门禁、直连模式凭据主机白名单 |
| [docs/settings-ui.md](docs/settings-ui.md) | 设置窗口版式与拖动/事件权限、6 个配置项、`settings.json` 落点与写入契约 |
| [docs/float-ball.md](docs/float-ball.md) | 桌面悬浮球：功能行为、设置项、v1.67–v1.80 演进与真机取证、`--float-probe` |
| [docs/workflows.md](docs/workflows.md) | 8 条 workflow 完整用法、打包字段说明、动态解析与安全模型、Android 签名 |
| [docs/android.md](docs/android.md) | Android 构建准备、共享补丁脚本与补丁链契约、移动端底栏、启动排障史（v1.60–v1.65）、数据目录与包名 |
