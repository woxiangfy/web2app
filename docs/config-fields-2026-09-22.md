# web2app 配置字段总表（Step 1 交付物，2026-09-22）

> ⚠️ **历史快照（重构前）**：本文记录的是**契约统一之前**的字段现状与问题清单，请**不要**当现行规范阅读。
> 其中列出的问题（`window_title` 冗余、`data_dir_default` 两壳同名反义、模板缺键、契约多实现）**均已解决**，
> 现行唯一真源是 [`packaging-config.md`](./packaging-config.md)（解析器 `.github/scripts/parse-packaging-config.py`）。
> 保留本文仅为「问题从哪来」的溯源。

**目的**：在讨论类型、默认值、校验规则之前，先把**有多少个可配置字段、每个字段是什么含义、在哪一层、谁读它**定死。
上一份 `docs/config-json-review-2026-09-22.md` 只看了"打包 JSON"这一层；本表覆盖全部 **6 层配置面**。

**唯一真源原则**：真正被源码读取的只有 **C 层（编译期环境变量，12 个）**。A/B 两层是它的**打包期投影**，
D 层是**运行期另一套**（与打包无关），E/F 是工具与构建参数。任何字段的"含义"必须能在 C 层或 D 层找到落点，
否则就是**空字段**（本表已标出 2 个）。

---

## 0. 六个配置层概览

| 层 | 载体 | 字段数 | 何时生效 | 谁写 | 谁读 |
|---|---|---|---|---|---|
| **A** | 打包期配置 JSON（`config_json` / `config_url`） | 18 键 + 5 个 workflow 输入 | 触发工作流时 | 人（粘贴/URL/仓库内 .json） | `build-custom.yml` 解析步骤 |
| **B** | `release.yml` 的仓库变量 / Secrets | 12 vars/secrets + Android 4 + 1 token | 打 Release 时 | 人在 GitHub Settings 配 | `release.yml` 的 `env:` 块 |
| **C** | 编译期环境变量（`option_env!`） | **12** | `cargo build` 时烘进二进制 | A/B 层注入 | `src/config.rs` / `resolve-core` |
| **D** | 运行期配置文件 | `settings.json` 4 + `window-state.json` 5 | 运行时读写 | 程序/用户（设置页） | `settings.rs` / `window_state.rs`；Electron 侧同契约 |
| **E** | 命令行参数（工具） | `w2a-proxy` 6 / `resolve-tool` 4 + 3 子命令 / `encrypt_url.py` | 手动或子进程调用 | 人 / 宿主进程 | 各 `main.rs` |
| **F** | 构建配置（仓库内，非"提供给打包的 JSON"） | `tauri.conf.json` 11 / `electron-builder.yml` 13 / `Cargo.toml` profile / `capabilities/default.json` 5 | 构建时 | 人（改仓库） | cargo / tauri / electron-builder |

> 另有 **2 个 `_github` 侧的"一次性 artifact 模板"**：`signing/packaging-config.json`（14 键）与 CI 在
> `build-custom.yml:1031-1068` 生成的那份（同 14 键）——它们**属于 A 层**，但内容不全（见 §4）。

---

## 1. A 层：打包期配置 JSON（`build-custom.yml`）

### 1.1 workflow 输入（`workflow_dispatch.inputs`）

| 输入 | 类型 | 默认 | 含义 | 位置 |
|---|---|---|---|---|
| `config_json` | string | — | 配置 JSON 字符串（**单行**），优先级高于 `config_url` | `:66-68` |
| `config_url` | string | — | 返回上述 JSON 的公开 URL（可指向仓库内 .json 的 raw 链接） | `:69-71` |
| `windows_installer` | boolean | `false` | 打 MSI 安装版 | `:72-76` |
| `windows_portable` | boolean | `true` | 打单文件绿色版（zip） | `:77-81` |
| `android` | boolean | `false` | 打已签名 APK | `:82-86` |

### 1.2 配置字段（18 键；`DEFAULTS` 在 `:128-154`）

**类型说明**：`str-bool` = JSON 里可写原生 `true` 或字符串 `"true"`（解析器都接受，`build-custom.yml:188-189`）；
实际落到源码时**一律是字符串**（写进 `GITHUB_ENV`），由 `env_bool!` 再解释。

| # | 字段 | 类型 | 默认 | 含义（唯一语义） | 落点 |
|---|---|---|---|---|---|
| 1 | `app_url` | string | `""`（**必填**） | 要嵌入的网址。**形态二选一**：① `https://user:pass@host`（userinfo 由本地代理转成 `Authorization: Basic`）；② **以 `[` 开头的字符串**（内容是 JSON 字符串数组，如 `["a.com","b.com"]`）⇒ 运行时按序做 DNS TXT 查询 + Ed25519 验签 + AES-GCM 解密取真实 URL。⚠️ **不是 JSON 数组类型**，是"字符串里套 JSON"（`resolve-core/src/lib.rs:378-385`） | `APP_URL` |
| 2 | `app_name` | string | `Web2App` | **安装名**：MSI 包名 / 快捷方式 / 安装目录 / "程序和功能"显示名；**同时是两端数据存储目录名**（设置文件所在目录） | `APP_NAME` |
| 3 | `title` | string | `""` → 回落 `app_name` | **显示名**：Windows 窗口标题、托盘悬停、右键菜单"设置"标题、Android 应用名（桌面图标下） | `WINDOW_TITLE` |
| 4 | `window_title` | string | `""` | **⚠️ 已废弃**，等价于 `title`，仅兼容老配置。取值链 `title → window_title → app_name`（`:238-247`） | `WINDOW_TITLE` |
| 5 | `extra` | object/array/任意 JSON | `{}` | 注入被包裹网页的附加参数，前端读 `window.web2appExtra`。序列化成**单行 JSON 文本**存进 env | `EXTRA` |
| 6 | `icon_url` | string | `""` | 应用图标 PNG 的 URL（`.png`，`cargo tauri icon` 重新生成全套图标）。**只在构建期消费，不进二进制** | 无 env |
| 7 | `proxy_enabled` | str-bool | `"false"` | 启用本地反向代理：**仅当 `proxy_http`/`proxy_https` 均为空**时 = 自实现代理（统一注入 Basic 鉴权，覆盖 WebSocket）；填了外部代理则走 Chromium 原生 `--proxy-server` | `PROXY_ENABLED` |
| 8 | `proxy_http` | string | `""` | 外部 HTTP 代理地址（`http://host:port`）。⚠️ 留空 ≠ "不代理"，而是"走自实现代理" | `PROXY_HTTP` |
| 9 | `proxy_https` | string | `""` | 外部 HTTPS 代理；留空 = 同 `proxy_http` | `PROXY_HTTPS` |
| 10 | `proxy_bypass` | string | `""` → 代码默认 `localhost;127.0.0.1;<-loopback>` | 不走代理的地址，**分号分隔** | `PROXY_BYPASS` |
| 11 | `tabs_enabled` | str-bool | `"false"` | 多页签模式（**仅 Windows**；Android 忽略）：宿主窗口 + 顶部标签栏，每个 tab 独立 WebView 共享同一数据目录，`target="_blank"` 直接新开 tab | `TABS_ENABLED` |
| 12 | `data_dir_default` | str-bool | `"false"` | WebView2 数据目录：`false`=`%LOCALAPPDATA%\<app_name>`（推荐）；`true`=系统默认（**紧挨 exe**，安装版下通常不可写）。⚠️ **Electron 壳同名反义**，见 §4 | `DATA_DIR_DEFAULT` |
| 13 | `public_key` | string | `""` | Ed25519 公钥（裸 base64 / SPKI PEM）。**仅当 `app_url` 为域名数组形态时必填**，解析步骤强制校验（`:200-207`）；支持手写 `\n`，会被归一成真实换行 | `PUBLIC_KEY` |
| 14 | `resolve_secret` | string | `""` → 代码默认 `web2app-fixed-secret-2026` | AES 对称 Secret，**必须与服务端生成 TXT 密文时一致**（`encrypt_url.py` / `resolve-tool`）。填了 = 本次产物独立密钥，提高反编译门槛 | `RESOLVE_SECRET`（由 `resolve-core` 读取） |
| 15 | `android_keystore_base64` | string | `""` | Android keystore(.jks/.keystore) 的 base64；**优先于仓库 Secrets**。仅在 `android` 勾选时用 | 不写 env（`:160-161`） |
| 16 | `android_key_alias` | string | `""` | keystore 别名（给了 keystore 则必填） | 不写 env |
| 17 | `android_keystore_password` | string | `""` | keystore 密码（给了 keystore 则必填） | 不写 env |
| 18 | `android_key_password` | string | `""` | key 密码（可空 = 同 keystore 密码） | 不写 env |

**解析器行为**（Step 2 要改的地方，先记事实）：未知字段只 `::warning::` 后忽略（`:183-185`、`:473`）；
`bool` 会被翻转成字符串（`:188-189`）；`extra` 单独序列化（`:190-192`）；三个 `str-bool` 只做 `strip().lower()`
不校验取值（`:229-230`）；`app_url`/`icon_url`/`resolve_secret` 做 `strip()`；域名数组形态会重新序列化成字符串（`:200-207`）。

---

## 2. B 层：`release.yml` 的仓库变量 / Secrets

**这是与 A 层并行、字段名不同的第二套配置面**（`release.yml:42-57` Windows、`:135-147` Android）。

| 概念 | 变量名 | 取值来源 | 与 A 层的差异 |
|---|---|---|---|
| 目标地址 | `APP_URL` | **`secrets.APP_URL` 优先**，回落 `vars.APP_URL` | 同名；可写域名数组字面量（靠字符串判定） |
| 安装名 | `APP_NAME` | `vars` | 同名 |
| 显示名 | **`APP_TITLE`** | `vars.APP_TITLE` | ⚠️ **A 层叫 `title`** |
| 显示名（旧） | `WINDOW_TITLE` | `${{ vars.APP_TITLE \|\| vars.WINDOW_TITLE }}` | A 层同名但由 `title` 导出 |
| 数据目录 | `DATA_DIR_DEFAULT` | `vars` | 同名 |
| 公钥 | `PUBLIC_KEY` | **`secrets` 优先**，回落 `vars` | 同名 |
| Secret | `RESOLVE_SECRET` | **`secrets` 优先**，回落 `vars` | 同名 |
| 本地代理开关 | `PROXY_ENABLED` | `vars` | 同名 |
| 外部代理 | `PROXY_HTTP` / `PROXY_HTTPS` / `PROXY_BYPASS` | `vars` | 同名 |
| 多页签 | `TABS_ENABLED` | `vars` | 同名 |
| 附加参数 | — | — | ⚠️ **缺失**：全文件无 `EXTRA` ⇒ Release 路径下 `window.web2appExtra` 恒为 `{}` |
| 图标 | — | — | ⚠️ **缺失**：无 `ICON_URL` ⇒ Release 路径不能用自定义图标 |
| Android 签名 | `ANDROID_KEYSTORE_BASE64` / `ANDROID_KEY_ALIAS` / `ANDROID_KEYSTORE_PASSWORD` / `ANDROID_KEY_PASSWORD` | `secrets`（`:447-450`） | A 层是 JSON 内联 4 键，B 层是 4 个 Secret |
| 发布 | `GITHUB_TOKEN` | `secrets` | 仅 `release.yml` 用 |

---

## 3. C 层：编译期环境变量（**源码唯一真源，12 个**）

| env | 类型 | 默认（代码内） | 读取点 |
|---|---|---|---|
| `APP_URL` | 字符串 | `https://用户名:密码@www.example.com` | `src/config.rs:161` |
| `PUBLIC_KEY` | 字符串 | `""` | `src/config.rs:166` |
| `APP_NAME` | 字符串 | `Web2App` | `src/config.rs:169` |
| `WINDOW_TITLE` | 字符串 | `= APP_NAME` | `src/config.rs:171` |
| `EXTRA` | JSON 文本 | `{}`（非法则回落 `{}`，`extra_json()`） | `src/config.rs:176` |
| `DATA_DIR_DEFAULT` | 布尔 | `false` | `src/config.rs:208` |
| `PROXY_ENABLED` | 布尔 | `false` | `src/config.rs:212` |
| `PROXY_HTTP` / `PROXY_HTTPS` | 字符串 | `""` | `src/config.rs:216/218` |
| `PROXY_BYPASS` | 字符串 | `localhost;127.0.0.1;<-loopback>` | `src/config.rs:220` |
| `TABS_ENABLED` | 布尔 | `false` | `src/config.rs:228` |
| `RESOLVE_SECRET` | 字节串 | `web2app-fixed-secret-2026` | `crates/resolve-core/src/lib.rs:69` |

**布尔取值白名单（A/B 层必须遵守，否则静默回落默认）**：`env_bool!` 认 `true/1/yes` → true，`false/0/no` → false，
**其余一律回落默认**（`src/config.rs:60-83`）。⚠️ Electron 的 `toBool` 多认一个 `'on'`（`config.ts:80`）⇒ 两壳判定不同。

**未消费字段**：`icon_url`（仅构建期用）与 A 层的 4 个 `android_*`（仅打包期用）**不进 C 层**，属正常；
另有 1 个"进了 D 层但没人读"的字段，见 §4 P0-1。

---

## 4. D 层：运行期配置文件（与打包无关，可随时改）

### 4.1 `settings.json`（用户设置，设置页可改）

存放于 `<配置根>\<app_name>\settings.json`（Tauri）；Electron 写 `%APPDATA%\<app_name>\` 或 `data\`（同名优先）。
> ⚠️ **2026-09-23 起已改**：落点 = **数据目录**（`<data_dir>\settings.json`，由 `data_dir` 字段决定），旧位置 `%APPDATA%\<app_name>\` 只作读取兜底。现行规则见 `packaging-config.md` §4.3 要点 5。

| 字段 | 类型 | 默认 | 含义 | UI |
|---|---|---|---|---|
| `wake_hotkey.ctrl/alt/shift` | bool ×3 | `false` | 唤醒（显示/隐藏主窗）快捷键的修饰键 | 设置页捕获 |
| `wake_hotkey.key` | string | `""` | JS `KeyboardEvent.code`（如 `Digit4`/`F1`）；**空串 = 未启用** | 设置页捕获 |
| `tray_enabled` | bool | `false` | 是否显示系统托盘图标 | `#trayEnabled` |
| `minimize_on_close` | bool | `false` | 关闭窗口时最小化到托盘（需同时开托盘，否则关闭即退出） | `#minimizeOnClose` |

读取点：`src/settings.rs:27-64`（默认值）、`:105-129`（读写）；设置页 `dist/settings.html`（仅 3 个可编辑项，
IPC 命令 `get_settings` / `save_settings` / `close_settings_window`）。

### 4.2 `window-state.json`（窗口几何，程序自动维护）

| 字段 | 类型 | 含义 |
|---|---|---|
| `x` / `y` | i32 | 窗口位置（**物理像素存盘**，恢复时换算成 DIP） |
| `width` / `height` | u32 | 尺寸 |
| `maximized` | bool（`#[serde(default)]`） | 是否最大化（兼容无此字段的旧文件） |

读取点：`src/window_state.rs:16-23`。

### 4.3 Electron 壳的 `config.json`（运行期，**与 A 层同契约但可随时改**）

`loadConfigAny([installRoot(), 包内])`（`electron-app/src/main/config.ts:133-147`）⇒ 改完重启即生效，无需重打包。
字段集 = A 层的前 13 个（忽略 4 个 `android_*`，并有提示）；**但 `window_title` 未实现回落**（见 §5）。

---

## 5. E 层：命令行参数

| 工具 | 参数 | 默认 | 含义 |
|---|---|---|---|
| `w2a-proxy`（sidecar） | `--url <URL>` | — | 直连上游地址，含 `user:pass@` 则强制注入 Basic（**手动调试用**） |
| | `--url-stdin` | — | 从 stdin 读第一行作为上游地址（**宿主走这条**，避免凭据出现在命令行） |
| | `--domains <a.com,b.com>` | — | 逗号分隔域名，按序尝试（DNS TXT 解析模式） |
| | `--public-key <B64>` | — | Ed25519 公钥（域名模式必填） |
| | `--no-proxy` | false | 只解析地址、不起代理（凭据回到宿主，由 login 事件注入） |
| | `--handshake-file <路径>` | — | 额外把握手 JSON 写入文件 |
| | ⚠️ **无 `--secret`** | — | `resolve_secret` **没有任何命令行通道** ⇒ Electron 侧无从传入（P0-1） |
| `resolve-tool` | `genkey [--out DIR]` | `--out .` | 生成密钥对：`private_key.pem` + `public_key.b64` |
| | `encrypt <key.pem> <url> [--ttl 秒] [--count N]` | `--ttl 172800`、`--count 1` | 输出 base64 密文；`--count` = 同一 URL 生成 N 份不同密文（不同 Nonce/盐，解密结果相同） |
| | `encrypt --batch <key.pem>` | 同上 | 从 stdin 逐行读 URL |
| | `all <url> [--out DIR] [--ttl] [--count]` | 同上 | 一键生成密钥对 + 密文 |
| | `--ttl` | 172800（48h） | 密文有效期秒数，**`0` = 永不失效** |
| `tools/encrypt_url.py` | `RESOLVE_SECRET` 环境变量 | `web2app-fixed-secret-2026` | Python 版服务端工具，**与 `resolve_secret` 必须一致** |
| `resolve-keys.yml` 输入 | `action` | 必填（选择） | `genkey` / `encrypt` / `all` |
| | `private_key` / `url` / `count`(1) / `ttl`(172800) | 见描述 | 工作流版的同参数（与 `resolve-tool` 等价） |

---

## 6. F 层：构建配置（仓库内文件，非"提供给打包的 JSON"）

| 文件 | 字段 | 当前值 | 含义/影响 |
|---|---|---|---|
| `tauri.conf.json` | `productName` | `Web2App` | 安装名（CI 会用 `app_name` 覆盖） |
| | `version` | `0.1.0` | 产物版本号 |
| | `identifier` | `com.example.web2app` | **Android 包名由此派生**（无 `bundle.android.package` 覆盖，README:496 记为上游限制） |
| | `mainBinaryName` | CI 写死 `web2app` | 主程序 exe 名，保证产物路径稳定 |
| | `build.frontendDist` / `devUrl` / `beforeBuildCommand` | `dist` / example / 空 | 前端资源目录（仅 `dist/settings.html` 等） |
| | `app.security.csp` | `null` | **未设 CSP**（远程站点由站点自身控制） |
| | `app.withGlobalTauri` | `true` | 暴露 `window.__TAURI__`（`titlebar.js` 依赖） |
| | `bundle.targets` / `icon` / `android.minSdkVersion` | `msi,nsis` / icon.png+ico / `24` | 打包目标与图标、Android 最低 SDK |
| `electron-builder.yml` | `appId` / `productName` / `copyright` | `com.example.web2app` / `Web2App` / … | Windows 应用标识与版本信息（`CompanyName` 取 `package.json` 的 `author.name`） |
| | `artifactName` | `${productName}-${version}-win-${arch}.${ext}` | zip 产物名；`portable.artifactName` 单独覆盖加 `-portable` |
| | `asar` / `files` / `win.target` / `win.icon` | `true` / `out/**` / dir+zip+portable / build/icon.ico | 打包形态与文件集 |
| | `portable.requestExecutionLevel` | `user` | **绿色版绝不要 UAC** |
| | `extraFiles` | `build/config.json → config.json` | 把运行期配置放到 exe 同级 |
| | `extraResources` | `inject/{splash,titlebar,loading,shell}.html(.js)`、`inject/settings.html`、`icons/icon.png`、`sidecar/w2a-proxy.exe` | 注入脚本/设置页/图标/sidecar（**sidecar 必须走 resources，asar 内无法 spawn**） |
| `Cargo.toml` | `[workspace] members` | resolve-core / resolve-tool / proxy-server | 三 crate 边界 |
| | `[profile.release]` | `lto = true`、`strip = true` | 体积与符号 |
| `capabilities/default.json` | `identifier` / `windows` / `remote.urls` / `permissions` | `main-window-chrome` / `["main"]` / `["http://*:*","https://*:*"]` / 5 个 window 权限 | **远程 origin 的 IPC 必须命中 ACL**；`remote.urls` 缺端口通配会让标题栏按钮静默失效 |

---

## 7. 跨层命名映射（同一概念在 6 层的名字）

| 概念 | A 打包 JSON | B release vars/secrets | C 编译期 env | D 运行期文件 | E CLI |
|---|---|---|---|---|---|
| 目标地址 | `app_url` | `APP_URL` | `APP_URL` | `config.json.app_url` | `--url` / `--url-stdin` / `--domains` |
| 跳板域名列表 | `app_url`（`[` 开头字符串） | `APP_URL`（同左，界面未说明） | `APP_URL` | `config.json.app_url` | `--domains` |
| 安装名 | `app_name` | `APP_NAME` | `APP_NAME` | `config.json.app_name` | — |
| 显示名 | `title` | **`APP_TITLE`** | `WINDOW_TITLE` | `config.json.title` | — |
| 显示名（旧） | `window_title` | `WINDOW_TITLE` | `WINDOW_TITLE` | `config.json.window_title`（**未实现**） | — |
| 附加参数 | `extra` | **缺** | `EXTRA` | `config.json.extra` | — |
| 图标 | `icon_url` | **缺** | —（构建期消费） | `config.json.icon_url` | — |
| 本地代理开关 | `proxy_enabled` | `PROXY_ENABLED` | `PROXY_ENABLED` | `config.json.proxy_enabled` | 有参数即启用 |
| 外部代理 | `proxy_http` / `proxy_https` | 同名 | 同名 | 同名 | — |
| 直连白名单 | `proxy_bypass` | `PROXY_BYPASS` | `PROXY_BYPASS` | `config.json.proxy_bypass` | — |
| 多页签 | `tabs_enabled` | `TABS_ENABLED` | `TABS_ENABLED` | `config.json.tabs_enabled` | — |
| WebView2 数据目录 | `data_dir_default` | `DATA_DIR_DEFAULT` | `DATA_DIR_DEFAULT` | `config.json.data_dir_default`（**反义**） | — |
| 公钥 | `public_key` | `PUBLIC_KEY`（secrets 优先） | `PUBLIC_KEY` | `config.json.public_key` | `--public-key` |
| 加解密 Secret | `resolve_secret` | `RESOLVE_SECRET`（secrets 优先） | `RESOLVE_SECRET` | `config.json.resolve_secret`（**未消费**） | ⚠️ **无通道** |
| Android 签名 | `android_*` 4 键 | `ANDROID_*` 4 Secrets | — | — | — |
| 密文有效期/份数 | — | — | — | — | `--ttl` / `--count` |
| 运行期：快捷键/托盘/关闭行为 | — | — | — | `settings.json` 4 键 | — |
| 运行期：窗口几何 | — | — | — | `window-state.json` 5 键 | — |

**结论**：概念 17 个，但**名字最多有 4 个变体**（显示名：`title` / `APP_TITLE` / `WINDOW_TITLE` / `config.json.title`）；
A 层 18 键里有 **4 个不进 C 层**（`icon_url` + 3 个 `android_*`... 准确说 4 个 `android_*` + `icon_url`），
B 层比 A 层**少 2 个概念**（`extra`、`icon_url`）。

---

## 8. 已确认的字段级缺口（本表的直接产出）

| 编号 | 字段 | 状态 | 一句话 |
|---|---|---|---|
| P0-1 | `resolve_secret` | **D 层读了不生效** | Electron `config.json` 读入后无任何消费点，`w2a-proxy` 也没有 `--secret` 通道 ⇒ 绿色版解密必失败 |
| P0-2 | `data_dir_default` | **同名反义** | Tauri `false`=`%LOCALAPPDATA%` / Electron `false`=exe 同级 `data\`；`true` 在 Tauri 安装版可能直接开不了窗 |
| P0-3 | `extra` / `resolve_secret` / `window_title` | **模板缺** | `packaging-config.json` 只 14 键，复用即静默丢这 3 个 |
| P1-1 | `window_title` | **B/C 层有、D 层缺** | Electron 完全忽略 ⇒ 同一老配置两壳标题不同 |
| P1-2 | `extra` / `icon_url` | **B 层缺** | Release 路径拿不到 `extra` 与自定义图标 |
| P1-3 | `proxy_bypass` | **默认值三层不一致** | 代码有 `localhost;127.0.0.1;<-loopback>`，Electron 默认空且空则不下发 |
| P1-4 | `app_name` | **语义过载** | 同时是"安装名"与"数据目录名" ⇒ 发布后改名 = 丢全部登录态 |
| P1-5 | Android 包名 | **无字段** | 固定 `com.example.web2app`，不同配置的 APK 互相覆盖安装 |
| P1-6 | 4 个 `android_*` | **放错层** | 一个 ~10KB 的 base64 + 明文密码塞进"配置 JSON"，与"产物参数"混在同一个文件里 |

---

## 9. 待你拍板的开放问题（Step 1 收口）

1. **`window_title` 现在就能删吗？** 是否还有老配置/老 Release 依赖它？可删则 A/B/C 三层一起去掉，语义唯一化。
2. **`extra` 与 `icon_url` 要不要补进 B 层（release vars）？** 补的话 `extra` 在 vars 里是 JSON 字符串，需要同样的"合法性兜底"。
3. **`data_dir_default` 是否接受改名**（如 `data_dir: "local_appdata" | "system" | "portable_sibling"`）？改名会影响已有配置与 Release 变量，需要一次兼容窗口。
4. **4 个 `android_*` 是否从"配置 JSON"里拆出去**（单独文件/仅 Secrets），让配置 JSON 只描述产物行为？
5. **要不要新增两个字段**：`android_package`（解 P1-5）、`data_dir_name`（解 P1-4，默认 = `app_name`）？
6. **`proxy_bypass` 的默认值以哪一层为准**（代码那串 vs 空 = 不下发）？

确认以上 6 条后，Step 2 就可以把它们写成一份 `config.schema.json`：**字段名 + 类型 + 默认值 + 平台作用域 + 校验规则 + 是否废弃**，
再让 A/B/C/D/F 各处从它派生或由 CI 断言一致——那是唯一能永久止住漂移的做法。

---

## 附：本表的取证方式（可复核）

- A 层：`build-custom.yml:11-62`（字段说明注释）、`:128-247`（Windows 解析）、`:418-536`（Android 解析）、`:1031-1068`（模板生成）
- B 层：`release.yml:42-57`、`:135-147`、`:447-450`
- C 层：`webview-app/src/config.rs:40-83`（宏）、`:161-228`（常量）、`crates/resolve-core/src/lib.rs:69`
- D 层：`src/settings.rs:23-129`、`src/window_state.rs:10-46`、`dist/settings.html`、`electron-app/src/main/config.ts:15-203`
- E 层：`crates/proxy-server/src/main.rs:47-96`、`crates/resolve-tool/src/main.rs:13-54`、`tools/encrypt_url.py:41-43`、`resolve-keys.yml:32-47`
- F 层：`webview-app/tauri.conf.json`、`electron-app/electron-builder.yml`、`webview-app/Cargo.toml`、`webview-app/capabilities/default.json`
