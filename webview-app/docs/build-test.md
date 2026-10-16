# 构建、测试与排障（webview-app）

> 自 `AGENTS.md` 拆出的构建 / 测试 / 目录结构 / FAQ 细节。
> 编译期配置常量见 [tauri-config.md](tauri-config.md)；悬浮球真机探针见 [float-ball.md](float-ball.md)；
> Android 构建准备、补丁脚本与补丁链契约见 [android.md](android.md)。

## 目录结构

```
webview-app/
├── Cargo.toml              # workspace 成员 + 依赖与 TLS 后端（Windows=native-tls / 其他=rustls）
├── tauri.conf.json         # Tauri 配置（含 Android bundle 配置）
├── build.rs                # Tauri 构建脚本
├── config.example.json     # 配置模板（含全部字段，供手动打包工作流使用）
├── docs/                   # 本工程专项文档（build-test / tauri-config / proxy-auth / settings-ui / float-ball / workflows / android）
├── src/
│   ├── main.rs             # 桌面入口（薄壳，装配在 lib.rs）
│   ├── lib.rs              # Tauri 应用装配：建窗 / 托盘 / 快捷键 / 命令注册（按平台 cfg 隔离）
│   ├── contract_tests.rs   # 静态契约测试（IPC 命令注册表 / capabilities 与事件名字面量 / 顶栏配色 / 加载层模式标记 / 启动日志归属；仅 cargo test 编译）
│   ├── config.rs           # 编译期 env 注入常量 + 目录名/文件名清洗
│   ├── bootstrap.rs        # 后台解析真实地址 → 就绪后导航
│   ├── proxy.rs            # 本地反向代理（HTTP + WebSocket 鉴权注入）+ 单元测试
│   ├── resolve.rs          # DNS TXT 查询 + 验签解密
│   ├── loading.rs          # 启动过渡（splash + 加载层）状态机
│   ├── debug_log.rs        # 诊断日志（dlog! 宏 / 落盘 / 1 MiB 轮转 / mask_url 去凭据 / log_startup 启动信息格式化）
│   ├── store.rs            # 运行期唯一那份 JSON 的契约（设置 + window 段几何，读-改-写；落点 = 数据目录）
│   ├── settings.rs         # 设置结构、快捷键转换、设置窗口/命令（持久化委托 store）
│   ├── tabs.rs             # 多页签（宿主窗口 + shell 标签栏 + tab 窗口）
│   ├── tray.rs             # 系统托盘
│   ├── window_state.rs     # 窗口几何：抓取 / 恢复策略（落盘在 store 的 window 段）
│   ├── webview_hooks.rs    # WebView2 原生事件（鉴权 / 新窗口 / 右键 / 下载）
│   ├── web_data.rs         # 清除浏览数据（缓存 / Cookie 全清，WebView2 ICoreWebView2Profile2）
│   ├── save_dialog.rs      # 下载「另存为」原生对话框（IFileSaveDialog）
│   ├── floatwin.rs         # 桌面悬浮球窗口（建窗 / 显示入口 show_ball / 拖动吸附 / 圆形命中）
│   ├── floatprobe.rs       # 悬浮球真机合成指针 harness（--float-probe，仅 Windows）
│   ├── splash.html         # 启动页（内联 titlebar.js，解决首文档无初始化脚本）
│   ├── shell.html          # 多页签外壳（标签栏）
│   ├── floaticon.html      # 悬浮球页面（双壳单一真源，透明 + 内置图标）
│   ├── titlebar.js         # 自绘标题栏注入脚本（Windows）
│   ├── mobilebar.js        # 移动端底部导航栏注入脚本（仅移动端；mobile_bar=true 时注入）
│   ├── loading.js          # 加载层注入脚本
│   ├── tab_title.js        # 多页签真实标题回传
│   └── dnd_polyfill.js     # WebView2 拖拽 polyfill
├── crates/
│   ├── resolve-core/       # 纯加密核心（Ed25519 + AES-256-GCM + HKDF），零平台依赖
│   ├── resolve-tool/       # 服务端 CLI：生成密钥对 / 生成 DNS TXT 密文 / 解密验证
│   └── proxy-server/       # Electron 侧 sidecar（bin 名 w2a-proxy），#[path] 复用 src/proxy.rs
├── dist/
│   ├── index.html          # 多页签 tab 窗口的首文档（body 必须空白：loading.js 靠它判定加载中；真实内容由 bootstrap 导航到远程地址）
│   └── settings.html       # 设置界面 UI 源码（编译期被 include_str! 内嵌，经 settings:// 自定义协议加载）
├── tools/                  # 服务端辅助脚本（encrypt_url.py / loading_test.js）
├── examples/               # 仅测试目标的验证示例（probe_dom_ready.rs / probe_new_window.rs / probe_clear_data.rs）
├── capabilities/           # Tauri ACL 能力声明（default.json=主窗；settings-window.json=无边框设置窗；tabs-remote.json=多页签 tab-* 远程页，仅授事件发送；float-icon.json=悬浮球窗，仅授 event emit）
├── icons/                  # 应用图标（icon.ico / icon.png 等）
├── txttest_keys/           # 验签测试密钥对（⚠️ **当前工作区已不存在**：私钥被 .gitignore 忽略、目录整体已移除，见 ../docs/txttest-verify-materials.md 的换钥说明）
└── （CI 与打包工作流已上移至仓库根 `.github/workflows/`，见 [workflows.md](workflows.md)）
```

## 环境要求

- **Rust** 工具链（stable，建议 ≥ 1.70）
- **Windows**
  - Windows 10/11 + WebView2 Runtime（Win11 自带；Win10 一般已预装，否则去微软官网安装）
  - Visual Studio 生成工具（含「使用 C++ 的桌面开发」工作负载）
- **Android**：JDK 17、Android SDK + NDK、已执行过 `cargo tauri android init` —— 版本策略与细节见 [android.md](android.md)

## Windows 构建

```bash
# 开发运行
cargo run

# 发布构建（生成 target/release/ 下的 exe）
cargo build --release

# 打包成 msi / nsis 安装包（产物在 target/release/bundle/）
cargo install tauri-cli --version "^2"
cargo tauri build
```

## Android 构建

```bash
cargo install tauri-cli --version "^2"      # 1) Tauri CLI
cargo tauri android init                    # 2) 仅首次；gen/ 已被 .gitignore 忽略，不要提交
cargo tauri android build --target aarch64 --apk
# ⚠️ --target 只接受架构名（aarch64/armv7/i686/x86_64），不是 rustup 三元组；AAB 去掉 --apk
```

> 环境要求（SDK / NDK 版本策略）、`.github/scripts/` 共享补丁脚本清单与**补丁链固定顺序**
> （insets → back → download → extras → schemes → fullscreen → permissions）、
> 数据目录、包名与 `tauri.conf.json` 字段限制 —— 全文见 [android.md](android.md)。

## 测试

项目内置 Rust 单元测试，覆盖代理（Basic 头生成、hop-by-hop 头识别、Set-Cookie 重写、WS Origin 重写）、
WebView2 钩子与配置清洗、`resolve-core` / `resolve-tool` 的加密往返与篡改检测，
以及 2026-09-23 新增的两类**静态契约守卫**（都是「错了也静默」的那类缺陷）：
运行期文档层 `src/store.rs`（读-改-写 / 落点 = 数据目录 / 双壳字面量互通）、`src/lib.rs` 的 IPC 注册表与
capability / 跨文件事件名字面量（改动 `capabilities/*.json` 或注入脚本的事件名会被它拦下）。

```bash
cargo test --all                # 全量单元测试（2026-10-12 优化闭环后实测 241 通过 / 6 ignored：resolve-core 19 + resolve-tool 22 + w2a-proxy 32 + 主 crate 168；主 crate 合计 171 条，含 `contract_tests` 静态契约守卫）
cargo test --all -- --ignored   # 真 DNS 端到端测试（需联网）：TXT 查询 → 验签 → 解密全链路
```

主 crate 的多数测试跑在 `#[cfg(not(windows))]` 路径上，因此**同时**在验证 Android 侧逻辑；
Windows 侧另有代理与钩子的专属测试，`ci.yml` 的 Windows 作业也会跑它们。

> ⚠️ `src/proxy.rs` 同时被主 crate 与 `crates/proxy-server`（sidecar `w2a-proxy.exe`）经 `#[path]` 复用，
> 因此**它的单元测试会在两个 crate 各跑一遍**（上表 w2a-proxy 的条数里含同一批代理用例）——新增代理
> 测试时两个计数会同时 +N，这是预期而非重复实现。

### 悬浮球真机探针（`--float-probe`，仅 Windows）

`target/debug/web2app.exe --float-probe`（或 `cargo run -- --float-probe`）跑一次**真机合成指针**验证悬浮球全交互链路：
建窗形态（v1.80 起：**窗口无区域** + 真逐像素 alpha）、自由态拖动 / 左右缘吸附、圆形命中（**`WM_NCHITTEST` 三形态**）、
悬停展开 / 抖动吸收 / 300ms 延迟缩回、单击 toggle 主窗、右键 = 关闭悬浮图标功能并唤回主窗口。
退出码：`0` = 全过；`>0` = 失败断言条数；`97` = 窗口环境没起来；`98` = 看门狗总超时。
驱动方式、受限会话 `build()` 挂死的环境抖动与外部重试器自建思路、UDF 落点与断言形态演变 —— 全文见 [float-ball.md](float-ball.md)。

## 常见问题

- **刷新 / 子页面打不开（鉴权丢失）**：确认 `APP_URL` 带 `user:pass`，且 `PROXY_ENABLED` 与
  `PROXY_HTTP/HTTPS` 的搭配符合「本地反向代理」分支（即启用代理时不填外部代理地址）。
- **全局快捷键没反应**：先在设置页（Shift+F1）确认唤醒快捷键**已配置**（默认不启用）；若已配置，
  再排查是否与其他软件冲突（代码只在「按下」那一刻触发）。
- **Android 上访问 http 本地代理被拦**：移动端已配置 `INTERNET` 权限并允许 localhost 明文，正常可用。
- **Android 打开后转圈十几秒 → 白屏 / 失败卡**：**不能直接断定是设备网络不通**。失败卡自带「现场诊断」，分层定位、
  v1.62–v1.65 的启动期排障史与全部判读表见 [android.md](android.md)「启动问题排障史」。
- **Android 端应用的数据存在哪 / 包名规则 / `tauri.conf.json` 字段限制**：见 [android.md](android.md)。
- **换产品名 / 包名**：改 `Cargo.toml` 的 `package.name`（决定 exe / 进程名）、`tauri.conf.json` 的
  `productName`（决定窗口标题 / 托盘提示）即可；Android 包名随打包配置 `app_name` 自动改写，见 [android.md](android.md)。
- **任务管理器里显示的是「WebView2 管理器 / WebView2 渲染器」而不是应用名？**：
  Web2App 的**主进程**已经以 `web2app.exe` 显示（由 `package.name` 决定），窗口标题/托盘提示也已经是
  `Web2App`。但 WebView2 内部会拉起「浏览器管理器 / 渲染器」**子进程**，它们的进程名被写死在微软 WebView2
  运行时二进制里，**没有按应用重命名的官方开关**（除非自行附带一份改名后的 WebView2 运行时副本，代价是更新/许可
  都要自己维护，不推荐）。所以这是 WebView2 运行时的固有限制，应用侧无法改成自身名称；若需更干净的任务管理器
  表现，只能后续考虑改用 Edge WebView2 的「固定版本运行时」并自托管（超出本应用范围）。
