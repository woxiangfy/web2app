# Web2App

> 把一个网址打包成桌面 / 手机原生应用的轻量工具。
> **Rust + Tauri 2** 编写，支持 **Windows（WebView2）** 与 **Android**。

Web2App 启动后直接内嵌并打开你预设的网址，可最小化到系统托盘、用全局快捷键一键显隐；
当网址带 `https://用户名:密码@host` 形式的 Basic 鉴权时，会自动给**每一个请求（含 WebSocket）**注入凭据，
无需在浏览器里手动输入账号密码，也不会出现「首次能开、刷新后空白」的问题。

---

## 功能特性

| 功能 | 说明 | 平台 |
| --- | --- | --- |
| 内嵌网址 | 默认打开编译时写死的网址，全屏/可缩放窗口 | Windows / Android |
| 设置界面 | 按 **Shift+F1** 打开；可配置唤醒快捷键、托盘开关、关闭行为，设置持久化到 `settings.json` | Windows |
| 系统托盘 | 可开关；最小化到托盘，左键显隐、菜单可退出（受设置控制） | Windows |
| 全局快捷键 | 一键切换窗口显示 / 隐藏；按键与修饰符可在设置里修改（默认 **Alt+4**） | Windows |
| Basic 鉴权 | 支持 `https://user:pass@host`，自动注入 `Authorization` 头 | Windows / Android |
| WebSocket 鉴权 | 本地反向代理对 WS 握手统一注入凭据，覆盖实时同步场景 | Windows / Android |
| 外部 HTTP/HTTPS 代理 | 可配置系统级代理（仅 Windows WebView2 生效） | Windows |
| 新窗口拦截 | `target="_blank"` / `window.open()` 强制在当前窗口打开 | Windows |
| 单实例 | 避免多开：第二个实例启动会把已存在窗口带到前台，而非再开一个 | Windows |
| 开发者工具 | WebView2 下按 **F12** 打开 DevTools 排查问题（debug 开 / release 关） | Windows |
| 优雅关闭 | 关闭行为由设置决定：启用托盘且开启“最小化到托盘”时隐藏到托盘；否则退出应用 | Windows |
| Cookie 会话保持 | 自动把上游 `Set-Cookie` 的 `Domain=真实host` 重写为 `127.0.0.1`，避免 localhost 下丢 cookie | Windows / Android |

---

## 支持平台

- **Windows**：原生 WebView2 渲染，支持托盘、全局快捷键、F12 调试、外部代理。
- **Android**：复用同一份 Rust 代码，鉴权走纯 Rust 本地反向代理（零平台依赖）；无系统托盘/快捷键（移动端由系统任务管理替代）。`tauri.conf.json` 的 `bundle.android` 目前仅设 `minSdkVersion: 24`（当前 Tauri 2 版本尚未支持 `package` / `targetSdkVersion` / `useKotlin` / `permissions` 字段，应用包名默认取自 `identifier`）；更细的 SDK 版本与权限在 `cargo tauri android init` 生成 `gen/android` 后于原生工程中调整。

---

## 设置界面（Shift+F1）

在 Windows 下按 **Shift + F1** 随时打开设置窗口（该快捷键固定，不可在界面内修改）。

可配置项：

1. **唤醒快捷键**：显示 / 隐藏主窗口的全局快捷键。支持“点击捕获”（按下组合键即可，按 Esc 取消）或从下拉框选择常用键。默认 **Alt+4**。
   - 保存后立即生效；不能与「打开设置(Shift+F1)」冲突。
2. **是否启用托盘图标**：关闭后启动不创建托盘图标（关闭窗口即退出）。
3. **关闭窗口后最小化到托盘**：勾选且同时启用托盘时，点关闭按钮 = 最小化到托盘；否则关闭窗口即退出应用。

> 设置保存在应用配置目录下（例如 `%LOCALAPPDATA%\com.example.web2app\settings.json`），为 JSON 文件，下次启动自动读取。

界面后续可在 `dist/settings.html` 上继续扩展更多能力（同一份 Rust 命令通道 `get_settings` / `save_settings` 即可复用）。

---

## 工作原理（为什么需要本地反向代理）

现代 Chromium / WebView2 **已废弃**「URL 内嵌 userinfo 自动发送 Basic 鉴权」的行为，且 WebView2 的
`BasicAuthenticationRequested` 事件对 **WebSocket 握手不生效**（这是微软官方确认的限制）。

Web2App 的解法是内置一个**本地反向代理**（`src/proxy.rs`）：

1. 程序启动时监听 `127.0.0.1:随机端口`；
2. WebView 只访问这个本地代理地址；
3. 代理对**每一个 HTTP 请求和 WebSocket 握手**统一注入 `Authorization: Basic ...` 头，再转发到真实服务器；
4. 代理对 `Host` / `X-Forwarded-*` 头做透传，让上游收到的请求与「浏览器直连」时一致。

该代理是纯 Rust、零平台依赖，因此 **Windows 与 Android 共用同一份代码**。

> 当 `PROXY_ENABLED = false` 或「启用了外部 HTTP 代理」时，WebView 直连上游，由 WebView2 原生事件兜底鉴权。

---

## 目录结构

```
web2app/
├── Cargo.toml              # 依赖与 TLS 后端（Windows=native-tls / 其他=rustls）
├── tauri.conf.json         # Tauri 配置（含 Android bundle 配置）
├── build.rs                # Tauri 构建脚本
├── src/
│   ├── main.rs             # 入口：窗口/托盘/快捷键/代理决策（按平台 cfg 隔离）
│   ├── settings.rs         # 设置结构、持久化(settings.json)、快捷键转换、设置窗口/命令
│   └── proxy.rs            # 本地反向代理（HTTP + WebSocket 鉴权注入）+ 单元测试
├── dist/
│   ├── index.html          # 占位前端（纯 Rust 工程，实际内容来自远程 URL）
│   └── settings.html       # 设置界面 UI 源码（编译期被 include_str! 内嵌，经 settings:// 自定义协议加载）
├── icons/icon.ico          # 应用图标
└── .github/workflows/
    ├── ci.yml              # 提交即自动测试（lint + test + 多平台编译检查）
    └── release.yml         # 发版即自动编译（Windows 安装包 + Android APK）
```

---

## 编译时配置（改这里，然后重新编译）

所有运行期参数都在 `src/main.rs` 顶部的**编译期常量**里写死（不读配置文件 / 环境变量）：

```rust
// 默认打开的网址，支持带账号密码：
//   "https://用户名:密码@www.example.com"
const APP_URL: &str = "https://用户名:密码@www.example.com";

const WINDOW_TITLE: &str = "Web2App";

// F12 开发者工具（仅 Windows WebView2）。已按构建模式自动判定：debug 开、release 关，无需手动改
#[cfg(debug_assertions)]
const DEVTOOLS_ENABLED: bool = true;
#[cfg(not(debug_assertions))]
const DEVTOOLS_ENABLED: bool = false;

// WebView2 用户数据目录：true=系统默认（紧挨 exe，安装版可能因无写权限而失败）；
// false=固定到 %LOCALAPPDATA%\com.example.web2app（始终可写，安装版推荐）
const DATA_DIR_DEFAULT: bool = false;

// —— 代理 —— 不需要就保持 PROXY_ENABLED = false
const PROXY_ENABLED: bool = false;
const PROXY_HTTP: &str  = "";   // 留空 = 走自实现本地代理（覆盖 WebSocket）
const PROXY_HTTPS: &str = "";   // 留空 = 同上
const PROXY_BYPASS: &str = "localhost;127.0.0.1;<-loopback>";

// —— 全局快捷键（仅桌面端）：Alt + 4 ——
const HOTKEY_CTRL:  bool = false;
const HOTKEY_ALT:   bool = true;
const HOTKEY_SHIFT: bool = false;
const HOTKEY_KEY:   Code = Code::Digit4;
```

改完直接 `cargo build`（见下文）。

---

## 本地编译与运行

### 环境要求

- **Rust** 工具链（stable，建议 ≥ 1.70）
- **Windows**
  - Windows 10/11 + WebView2 Runtime（Win11 自带；Win10 一般已预装，否则去微软官网安装）
  - Visual Studio 生成工具（含「使用 C++ 的桌面开发」工作负载）
- **Android**
  - JDK 17
  - Android SDK + NDK（r27 及以上）、`build-tools 34`、`platforms;android-34`
  - 已执行过 `cargo tauri android init`（见下）

### Windows

```bash
# 开发运行
cargo run

# 发布构建（生成 target/release/ 下的 exe）
cargo build --release

# 打包成 msi / nsis 安装包（产物在 target/release/bundle/）
cargo install tauri-cli --version "^2"
cargo tauri build
```

### Android（一次性准备 + 构建）

```bash
# 1) 安装 Tauri CLI
cargo install tauri-cli --version "^2"

# 2) 初始化安卓工程（仅首次；会生成 gen/android 与 Kotlin 胶水，请提交到仓库）
cargo tauri android init

# 3) 构建 APK（调试/直接安装）
cargo tauri android build --target aarch64-linux-android --apk
# 若需上架 Play Store 的 AAB，去掉 --apk 即可
```

> 首次 `android init` 后务必把生成的 `gen/android/` 目录提交进仓库，`release.yml` 的 Android 任务依赖它。

---

## 测试

项目内置 Rust 单元测试（Basic 头生成、hop-by-hop 头识别等）：

```bash
cargo test --all
```

这些测试编译/运行的是与 **Android 完全一致**的 `#[cfg(not(windows))]` 代码路径，因此同时在验证移动端逻辑。

---

## 自动构建（GitHub Actions）

仓库已内置两条流水线，**无需人为触发**：

### 1. 提交即测试 — `.github/workflows/ci.yml`

在 `push` 到 `main`/`master` 或针对它们的 PR 时运行：

- **Lint & Test (Ubuntu)**：安装 WebKit2GTK 等依赖 → `cargo fmt --check`（仅提示）→
  `cargo clippy -D warnings` → `cargo test --all`（同时验证 Android 代码路径）。
- **Check (Windows)**：`cargo check --all-targets` 验证 WebView2 那套代码能编译。
- **Check (Android)**（`continue-on-error`，不阻断）：用 `cargo ndk` 交叉编译 `aarch64-linux-android`，
  验证 Rust 侧可编译（不产出 APK）。

### 2. 发版即编译 — `.github/workflows/release.yml`

在 GitHub 上 **Publish 一个 Release** 时运行：

- **Build Windows installers**：`cargo tauri build` 产出 `msi` / `nsis`，自动上传到本次 Release 的资源附件。
- **Build Android APK**（尽力而为）：构建 `aarch64` APK 并作为 workflow artifact 上传。
  若仓库里还没有 `gen/android`，该任务会失败但**不影响 Windows 包发布**。

> 想正式签名 Android 包时，在 `release.yml` 的 Android 任务里配置
> `KEY_ALIAS` / `KEY_PASSWORD` / `KEYSTORE` 等密钥（仓库 Secrets），再启用签名步骤即可。

### 3. 手动打包 — `.github/workflows/build-custom.yml`

在 GitHub Actions 页手动 **Run workflow**，填一份配置 JSON（或填一个返回 JSON 的 URL），
一次触发同时产出：**Windows MSI 安装包** + **Windows 免安装绿色版 zip** + **已签名 Android APK**。
无需改源码，就能为不同的网址/名字/图标生成各自独立的安装包。

配置模板见仓库根目录的 **`config.example.json`**（复制到本地改好再粘贴即可）。

两个名字字段的区分（很容易混）：

| 字段 | 含义 | 影响范围 |
| --- | --- | --- |
| `app_name` | **安装名**（给系统/安装器看） | Windows 安装包名、开始菜单与桌面快捷方式、安装目录、"程序和功能"显示名；同时也是两端**数据存储目录名** |
| `title` | **显示名**（给用户看） | **Windows 窗口标题**、**Android 应用名称**（桌面图标下的名字） |

`title` 省略时自动回落为 `app_name`。二者分开后即可实现「安装包叫英文名 `MyApp`、界面叫中文名 `我的应用`」。
旧字段 `window_title` 等价于 `title`，仅为兼容老配置保留，新配置请直接用 `title`。

> 粘贴到 `config_json` 输入框时必须是**单行 JSON**；用 `config_url` 指向 RAW 文件（如 GitHub raw 链接）则无此限制。

#### Android 正式签名（与上面配置 JSON 的字段对应）

手动打包产出的 release APK 默认**未签名**（无法直接安装），CI 会用你提供的签名信息对 APK 做
zipalign + apksigner 签名。签名密钥沿三层策略**依次取第一份可用的**：

| 优先级 | 签名来源 | 适合场景 |
| --- | --- | --- |
| 1 | 配置 JSON 里的 `android_keystore_base64` 等 4 个字段 | 每个应用各持一把正式密钥，随配置一起分发 |
| 2 | 仓库 Secrets（`ANDROID_KEYSTORE_BASE64` / `ANDROID_KEY_ALIAS` / `ANDROID_KEYSTORE_PASSWORD` / `ANDROID_KEY_PASSWORD`） | 长期使用 / 准备上架，各次构建可互相覆盖升级 |
| 3 | 都没有 → CI 现场用 keytool 生成**一次性** keystore 签名 | 临时试装；但每次构建签名都不同，**新版无法覆盖安装旧版**（需先卸载） |

**正式签名请务必用方式 1 或 2**，并保留好 keystore 文件与密码——Android 应用升级必须用**同一把密钥**签名，
密钥一旦丢失，已装应用将永远无法覆盖升级。

生成并填到 JSON 的完整步骤：

**① 在本机生成一把 keystore**（需 JDK；装过 Android Studio 的话可直接用其自带 keytool，
例如 `"C:\Program Files\Android\Android Studio\jbr\bin\keytool.exe"`，或先 `export PATH="$PATH:.../jbr/bin"`）：

```bash
keytool -genkeypair -v \
  -keystore release.keystore \
  -alias my-app-alias \
  -keyalg RSA -keysize 2048 -validity 10000 \
  -storepass YourStorePass -keypass YourKeyPass \
  -dname "CN=MyApp, OU=Dev, O=MyCompany, L=City, ST=State, C=CN"
```

- `-storepass` 是 keystore 存储密码 → 填 JSON 的 `android_keystore_password`
- `-keypass` 是条目 key 密码，通常与 storepass 相同 → 填 `android_key_password`（可省略，默认同 storepass）
- `-alias` 填 `android_key_alias`
- 有效性 `-validity 10000` 天 ≈ 27 年，够用；正式上架建议更长

**② 把 keystore 编码成 base64 单行字符串**（CI 无法直接收文件，只能收 JSON 里的 base64）：
PowerShell：
```powershell
[Convert]::ToBase64String([IO.File]::ReadAllBytes("C:\path\to\release.keystore"))
```
Git Bash / macOS / Linux：
```bash
base64 -w0 release.keystore   # 输出即一整行 base64
```

**③ 把 base64 与别名/密码填入配置 JSON**（字段名严格对应 `config.example.json`）：
```json
{
  "app_url": "https://user:pass@example.com",
  "app_name": "MyApp",
  "title": "我的应用",
  "android_keystore_base64": "<第②步输出的整行 base64>",
  "android_key_alias": "my-app-alias",
  "android_keystore_password": "YourStorePass",
  "android_key_password": "YourKeyPass"
}
```

> ⚠️ `android_keystore_base64` 是 keystore 的**明文等价物**（任何人拿到都能用它给同包名应用签名冒充你），
> 等于一把私钥。**不要**把它提交进公开仓库的 JSON / README / 历史记录；正式长期使用更推荐放仓库
> **Secrets**（方式 2），配置 JSON 里只留方式 1 用于「每个应用各自一把密钥、随配置分发」的场景。
> 想验证 JSON 里的 keystore 是否有效，可用 `keytool -list -keystore <解码后的.jks> -storepass <密码>` 查看别名。

---

## 常见问题

- **刷新 / 子页面打不开（鉴权丢失）**：确认 `APP_URL` 带 `user:pass`，且 `PROXY_ENABLED` 与
  `PROXY_HTTP/HTTPS` 的搭配符合「本地反向代理」分支（即启用代理时不填外部代理地址）。
- **全局快捷键没反应**：检查 `HOTKEY_*` 组合是否与其他软件冲突；代码已严格只在「按下」那一刻触发。
- **Android 上访问 http 本地代理被拦**：移动端已配置 `INTERNET` 权限并允许 localhost 明文，正常可用。
- **Android 端应用的数据存在哪？**（下文中 `<pkg>` = `tauri.conf.json` 的 `identifier`，当前为 `com.example.web2app`；
  `<app_name>` = 打包配置里的 `app_name`，清洗掉非法字符后的值）
  - **设置文件**：`/data/data/<pkg>/<app_name>/settings.json`
  - **WebView 数据**（Cookie / localStorage / IndexedDB）：`/data/data/<pkg>/app_webview/`
  - **缓存**：`/data/data/<pkg>/cache/`
  - **日志**：`/data/data/<pkg>/logs/`

  注意 Tauri 移动端的 `app_config_dir()` **不会再拼 identifier**，它直接返回应用私有根目录
  `/data/data/<pkg>`（见 `tauri/src/path/android.rs` → `PathPlugin.kt` 的 `getConfigDir` =
  `activity.dataDir`），与桌面端「基础目录 + identifier」的行为不同——所以取值后**不能再取 parent**，
  否则会跳到 `/data/data`（不可写，且与其它包名冲突）。代码已按平台分别处理（见 `src/settings.rs`）。

  这些目录都是**应用私有目录**：未 root / 非 debug 包在文件管理器里看不到，**卸载应用即被系统清空**。
  排查时可用 `adb shell run-as <pkg> ls files/` 查看。
  （构建产物 APK 不在手机上，而在构建机的 `gen/android/app/build/outputs/apk/`，CI 签名后上传为 artifact。）
- **`cargo run` 报 `unknown field package` / `targetSdkVersion` / `permissions`（build script 阶段）**：
  这是 **当前 Tauri 2 版本的 `tauri-build` 对 `bundle.android` 仅识别 `minSdkVersion` 等少量字段**所致
  （`package` / `targetSdkVersion` / `useKotlin` / `permissions` 是较新版本才支持的）。该错误会在桌面构建时
  同样报出（因为 `tauri-build` 始终解析整份 `tauri.conf.json`）。**不要升级 Tauri 版本**（会破坏已调通的
  WebView2 钩子），直接把 `bundle.android` 精简为只保留 `minSdkVersion`，其余在 `cargo tauri android init`
  生成 `gen/android` 后再于原生工程里配置即可。
- **换产品名 / 包名**：改 `Cargo.toml` 的 `package.name`、`tauri.conf.json` 的
  `productName` / `identifier` 即可（当前分别为 `web2app` / `Web2App` / `com.example.web2app`；Android 应用包名默认由 `identifier` 派生，当前 Tauri 2 版本暂不支持在 `tauri.conf.json` 里用 `bundle.android.package` 覆盖）。
- **任务管理器里显示的是「WebView2 管理器 / WebView2 渲染器」而不是应用名？**：
  Web2App 的**主进程**已经以 `web2app.exe` 显示（由 `package.name` 决定），窗口标题/托盘提示也已经是
  `Web2App`。但 WebView2 内部会拉起「浏览器管理器 / 渲染器」**子进程**，它们的进程名被写死在微软 WebView2
  运行时二进制里，**没有按应用重命名的官方开关**（除非自行附带一份改名后的 WebView2 运行时副本，代价是更新/许可
  都要自己维护，不推荐）。所以这是 WebView2 运行时的固有限制，应用侧无法改成自身名称；若需更干净的任务管理器
  表现，只能后续考虑改用 Edge WebView2 的「固定版本运行时」并自托管（超出本应用范围）。
