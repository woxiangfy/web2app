# 编译期配置与诊断日志（webview-app）

> 自 `AGENTS.md` 拆出。所有运行期参数都是 **`src/config.rs` 的编译期常量**，由 `option_env!`
> 在编译时注入（不读运行期配置文件）。手动打包工作流（`build.` 系 workflow）就是通过环境变量
> 把这些值传进来的 —— 打包字段契约见 `../docs/packaging-config.md`，workflow 用法见 [workflows.md](workflows.md)。

```rust
// 默认打开的网址，支持带账号密码；写成域名数组时走 DNS TXT 验签解密（见「工作原理」）
const APP_URL: &str = "https://用户名:密码@www.example.com";

const APP_NAME: &str = "Web2App";     // 安装名 / 数据目录名（Android 侧为应用 ID 派生）
const TITLE: &str = APP_NAME;         // 显示名 / 窗口标题，缺省回落 APP_NAME（旧名 WINDOW_TITLE 已移除）

// 额外参数：编译期内联成 window.web2appExtra = {...}，页面 JS 可直接读；非法 JSON 自动回落 {}
const EXTRA: &str = "{}";

// 多页签：默认关闭。Windows = 多 WebView 标签栏；Android = 底栏内单 webview 页签记录（需同时开 MOBILE_BAR）
const TABS_ENABLED: bool = false;

// 诊断日志（全平台）：**默认关闭**。true = 写 <数据目录>\logs\startup.log 并同时打印；
// false = 不建目录、不落文件，控制台输出也一并静默。运行期逃生口：`--debug` / WEB2APP_DEBUG=1
const DEBUG: bool = false;

// 动态解析（app_url 为域名数组时用）：Ed25519 公钥，裸 base64 或 SPKI PEM
const PUBLIC_KEY: &str = "";

// F12 开发者工具（仅 Windows WebView2）。已按构建模式自动判定：debug 开、release 关，无需手动改
#[cfg(debug_assertions)]
const DEVTOOLS_ENABLED: bool = true;
#[cfg(not(debug_assertions))]
const DEVTOOLS_ENABLED: bool = false;

// WebView2 用户数据目录（"app_name" 派生的目录名）。
// ⚠️ 空串是**有含义的取值**：表示「用系统默认程序目录 %LOCALAPPDATA%\<app_name>，不跟着程序跑」，
//    因此这里必须用裸 option_env! 读取，**不能**用把空串当未设置的 env_str!。
// 默认 "data" = <exe 所在目录>\data\（跟着程序走）。也可填其它相对路径或绝对路径。
// 目录不可写会自动回退到系统默认目录（安装版装进 Program Files 时不至于打不开窗）。
const DATA_DIR: &str = "data";

// —— 代理 —— 不需要就保持 PROXY_ENABLED = false
const PROXY_ENABLED: bool = false;
const PROXY_HTTP: &str  = "";   // 留空 = 走自实现本地代理（覆盖 WebSocket）
const PROXY_HTTPS: &str = "";   // 留空 = 同上
const PROXY_BYPASS: &str = "localhost;127.0.0.1;<-loopback>";
```

另有 `RESOLVE_SECRET`（在 `resolve-core` 侧编译期注入，未设置时回落内置默认值；改它必须与服务端同步）。

⚠️ 全局快捷键**不走编译期常量**：快捷键现在在**设置页（Shift+F1）**中配置并落盘到 `settings.json`，
**默认不启用**。

改完直接 `cargo build`（见 [build-test.md](build-test.md)）。

## 诊断日志（`debug`，默认关闭）

⚠️ **Windows 上它是 GUI 子系统程序**（`src/main.rs` 的 `windows_subsystem = "windows"`）：双击运行时
没有控制台可挂，`eprintln!` 的输出**没有任何去处**。于是「双击没反应 / 打开的是 example.com /
标题不对」这类反馈在用户侧**零证据**，只能远程猜。

`debug = true` 时把这些诊断同时落一份盘（`src/debug_log.rs`）：

| 平台 | 位置 |
|---|---|
| Windows | `<数据目录>\logs\startup.log`（目录规则同 `DATA_DIR`，默认 `<exe 所在目录>\data\logs\`） |
| Android | `/data/data/<pkg>/logs/startup.log`（应用私有目录，见 [android.md](android.md)） |

**为什么不默认开**：每次启动都在用户的数据目录里留下交付物以外的文件，不是期望行为。代价要写清楚：
上面那条「用户侧第一现场」在默认配置下**不存在**。所以留了**运行时逃生口**（交付出去的包也能临时
取证，不必重新编译）：

```powershell
.\web2app.exe --debug                # 或
$env:WEB2APP_DEBUG = "1"; .\web2app.exe
```

> ⚠️ 逃生口的环境变量是 **`WEB2APP_DEBUG`**（运行期），**不是** `DEBUG`。后者是**打包期**注入用的
> 名字（沿用「字段名大写」规则，同 `TABS_ENABLED`）；`DEBUG` 在开发机上是常见变量名，把它当逃生口
> 会让本地 release 构建莫名其妙开始写日志。

契约（改这个模块时别破坏）：

* **文件名 / 轮转与 Electron 侧逐字相同**：`startup.log`、1 MiB 轮转、只留 `startup.prev.log` 一代
  —— 双壳同一份契约（`../docs/packaging-config.md` §3.5 / §3.6）。
* **fail-open**：目录建不出 / 文件不可写 ⇒ 本次静默跳过，**绝不影响启动**（为了记日志让应用起不来
  是本末倒置）。
* **绝不记凭据**：地址一律经 `mask_url` 去掉 `user:pass@`；Secret 只报「已配置 / 未配置」。
* **零外部依赖**：只用 std（本仓库不引 `log` / `env_logger`），时间戳自己格式化成 ISO-8601 UTC。
* 关闭时 `dlog!` **直接短路**：连 `format!` 都不做 —— 代理是热路径（每条响应都会走一遍），
  不能为了「反正不输出」而白拼字符串。

取证：

```powershell
Get-Content .\data\logs\startup.log -Tail 40
```
