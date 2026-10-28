# Web2App（Rust + Tauri 2）

> 把一个网址打包成桌面 / 手机原生应用的轻量工具，支持 **Windows（WebView2）** 与 **Android**。
> 内嵌打开预设网址、托盘最小化、全局快捷键、多页签、自绘标题栏、下载接管与设置页；
> `https://user:pass@host` 形式的 Basic 鉴权由**本地反向代理**给每个请求（含 WebSocket）注入凭据。
>
> 仓库级规则、目录地图与文档索引见根 `AGENTS.md`。Windows 免安装绿色版由另一个壳
> `electron-app/`（Electron + TS）交付，两者共用同一份注入脚本与 `config.json` 契约。

## 硬约束速览（不知道就会踩坑）

1. **单一真源**：`src/{titlebar.js,loading.js,tab_title.js,dnd_polyfill.js,shell.html,splash.html,floaticon.html}`、
   `src/{proxy.rs,resolve.rs}`、`dist/settings.html` 与 Electron 壳共用 —— 改动任一**必须双壳同步验证**。
   ⚠️ `mobilebar.js` **不在**此列（仅 Tauri 移动端），改它只需过本侧门禁。
2. **测试门禁**：改动后 `cargo test --all` 必须全绿（当前 282 通过 / 6 ignored）；`src/contract_tests/`（按领域 13 个子模块）
   是静态契约守卫 —— 改 `capabilities/*.json` 或注入脚本的事件名会被它拦下。
3. **配置是编译期常量**：所有运行期参数在 `src/config.rs` 由 `option_env!` 注入，**改完必须重新编译**；
   打包 workflow 通过环境变量注入。全文见 [docs/tauri-config.md](docs/tauri-config.md)。
   ⚠️ 例：`const PROXY_BYPASS: &str = env_str!("PROXY_BYPASS", "localhost;127.0.0.1;<-loopback>")`
   的默认串是跨壳**单一字面量契约**，10 个入库文件必须一致（清单见 `../docs/packaging-config.md` §4.4，
   `scripts/pack-local.mjs --selftest` 逐文件钉住）。
4. **Android 补丁链顺序固定**：`prepare-wry-kotlin` → insets → back → download → extras → schemes → fullscreen → permissions。
5. **悬浮球建窗必须显式 `.data_directory(config::data_dir(app))`**（v1.71 真机根因，有契约守卫）。
6. **设置页命令名三处逐字一致**：`settings.html` 字面量 / `settings-preload.ts` 白名单 / 主进程 IPC 名 —— 漏一处即「按钮点了没反应」。
7. **`settings.json` 三个写入方一律读-改-写**（设置页保存 / 主窗几何 / 悬浮球几何），否则互相覆盖；写盘为原子写。
8. **改完注入脚本必须重新打包**：脚本随资源进包，跑旧产物永远看不到修复。

## 功能特性

| 功能 | 平台 |
| --- | --- |
| 内嵌网址 · 设置界面（Shift+F1）· 托盘 · 全局快捷键（默认不启用）· 下载接管 · 登录密码保存 · 窗口几何记忆 · 拖拽 polyfill · F12 DevTools | Windows |
| Basic 鉴权 + WebSocket 鉴权（本地反向代理统一注入）· Cookie 会话保持 | Windows / Android |
| 自绘标题栏（38px 通栏；前进/后退带落点守卫 v1.66）· 单实例 · 外部 HTTP/HTTPS 代理 · 新窗口拦截 | Windows |
| 多页签 `TABS_ENABLED`（默认关；首页页签固定第一位、不可关闭） | Windows（Android 为底栏页签记录形态） |
| 移动端底栏 `MOBILE_BAR`（默认关，仅移动端；含「更多」面板） | Android |
| 桌面悬浮图标 `float_enabled`（56×56 透明置顶小球；单击切换主窗显隐、右键关闭并唤回） | Windows |
| 启动过渡（启动页 + 加载层 + 失败卡） | Windows / Android |
| Android 原生补丁：返回键 · 下载接管 · 外部 scheme 外跳 · 更多面板 Kotlin 桥 · H5 视频全屏 · 运行时权限 · `target=_blank` 进移动页签 | Android |

- 鉴权链路原理与凭据主机白名单：[docs/proxy-auth.md](docs/proxy-auth.md)
- 设置窗口版式、6 个配置项与 `settings.json` 落点/原子写：[docs/settings-ui.md](docs/settings-ui.md)
- 悬浮球（形态 / 单击 toggle / 拖动吸附 / 圆形命中 / 逐轮演进 / `--float-probe`）：[docs/float-ball.md](docs/float-ball.md)
- Android（构建准备 / 补丁脚本与补丁链契约 / 底栏 / 启动排障史 / 数据目录）：[docs/android.md](docs/android.md)
- 7 条 CI workflow / 打包字段 / 动态解析与安全模型 / Android 签名：[docs/workflows.md](docs/workflows.md)
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
| [docs/workflows.md](docs/workflows.md) | 7 条 workflow 完整用法、打包字段说明、动态解析与安全模型、Android 签名 |
| [docs/android.md](docs/android.md) | Android 构建准备、共享补丁脚本与补丁链契约、移动端底栏、启动排障史（v1.60–v1.65）、数据目录与包名 |
