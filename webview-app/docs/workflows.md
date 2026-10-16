# CI 与打包工作流

> 本文是 webview-app 相关 GitHub Actions 的唯一详细说明（共 7 条 workflow 的完整用法）。
> 摘要与入口见 [../AGENTS.md](../AGENTS.md)「自动构建」；打包配置 JSON 字段契约见
> [../../docs/packaging-config.md](../../docs/packaging-config.md)。

共 **7 条** workflow，统一存放在**仓库根 `.github/workflows/`**（2026-09-22 由本目录上移）：
其中 **5 条服务本工程**（命令经 `defaults.run.working-directory: webview-app` 落到本工程目录，
产物路径则显式带 `webview-app/` 前缀）——2 条**自动**（`ci.yml` / `release.yml`）+
3 条**手动**（`generate-signing-key.yml` / `resolve-keys.yml` / `build-custom.yml`，
在 Actions 页手动 Run workflow）；另外 2 条服务 **Electron 绿色版壳**（工作目录是 `electron-app/`，
用法见 `electron-app/AGENTS.md`）：自动的 `electron-ci.yml`（lint / 探针 e2e / 出包）与
手动的 `electron-build-custom.yml`（自定义打包）。

> 打包一个可安装、可覆盖升级的应用，推荐顺序：**先跑 `generate-signing-key.yml` 生成并保存签名 → 再跑 `build-custom.yml` 打包**。详见下文第 3、4 小节。

## 1. 手动触发 — `.github/workflows/ci.yml`

⚠️ 该 workflow **只有 `workflow_dispatch`**（在 Actions 页点「Run workflow」），**已不再随 push / PR 自动运行**：

- **Lint & Test (Ubuntu)**：安装 WebKit2GTK 等依赖 → `cargo fmt --all -- --check`（**阻断**）→
  `cargo clippy -D warnings` → `cargo test --all`（同时验证 Android 代码路径）。
- **Check (Windows)**：`cargo check --all-targets` 验证 WebView2 那套代码能编译，并跑 `clippy` / `test`。
- **Check (Android)**：用 `cargo ndk` 交叉编译 `aarch64-linux-android`，并跑 **Android 目标 clippy**
  （**阻断**，不产出 APK）。NDK 由 `.github/scripts/install-android-ndk.sh` 安装（版本单一真源）。
- **config-contract**：跑打包配置契约门禁 —— 解析器 `--selftest`、**六个安卓共享脚本各自的
  `--selftest`**、校验全仓库 workflow YAML 与 `.github/scripts/*.sh` 语法（`check-workflows.py`）、
  逐个校验两份 `config.example.json` 与 `scripts/pack-local.mjs`
  的 `--parity`（Node 复刻与 Python 解析器的 43 组用例必须逐条一致）。详见
  `docs/packaging-config.md` §5。

## 2. 发版即编译 — `.github/workflows/release.yml`

两种触发方式：**Publish 一个 Release**（跑 Windows + Android），或在 Actions 页手动 Run workflow
（手动触发只跑 Android）。

- **Build Windows installers**（仅 Release 事件）：`cargo tauri build` 产出 `msi` / `nsis`，自动上传到本次 Release 的资源附件。
- **Build Android APK**：在 CI 里现跑 `cargo tauri android init --ci` 生成 `gen/android`（该目录被
  `.gitignore` 忽略，无需提交），再构建 `aarch64` APK 并作为 workflow artifact 上传。
- **产物版本号**：`VERSION`（仓库变量；为空则回落本次 Release 的 tag，自动剥前导 `v`；再为空则
  保留 `tauri.conf.json` 的值）—— 与 `build-custom.yml` 的能力对齐（历史上 release 侧不写 version，
  发版产物版本恒为 `tauri.conf.json` 的 `0.1.0`）。

> Android 正式签名所需仓库 Secrets：`ANDROID_KEYSTORE_BASE64` / `ANDROID_KEY_ALIAS` /
> `ANDROID_KEYSTORE_PASSWORD` / `ANDROID_KEY_PASSWORD`。流水线里由 `build-custom.yml` / `release.yml`
> 的对应步骤读取（与手动打包用的是同一套名字）。

## 3. 手动打包 — `.github/workflows/build-custom.yml`

在 GitHub Actions 页手动 **Run workflow**，先勾选本次要打哪些产物（三个开关互不依赖），
再填一份配置 JSON（或填一个返回 JSON 的 URL）。无需改源码，就能为不同的网址/名字/图标
生成各自独立的安装包。

> 三个开关的默认值：`windows_portable=true`（打单文件版），`windows_installer=false`、`android=false`。

手动运行时有 **3 个独立复选框**，想打哪个就勾哪个，产物分别上传到各自的 artifact：

| 选项 | 含义 | 对应产物 |
| --- | --- | --- |
| `windows_installer` | 打 Windows **安装版(MSI)** | `windows-msi` → `<app_name>_<版本>_x64_zh-CN.msi` |
| `windows_portable` | 打 Windows **单文件版**（免安装 zip） | `<app_name>-portable` → `<app_name>-portable.zip`（内含单个 `<app_name>.exe`） |
| `android` | 打已签名的 Android APK | `android-apk` → 签名 APK |

全部勾选＝一次触发同时产出上面三样；只勾 Windows 安装版时跳过 zip、只勾单文件版时用
`--no-bundle` 只编 exe（不打 MSI，更快）；三个都不勾则工作流无 job 运行。

> 说明：这里的「单文件版」即 **Windows 免安装绿色版**——单个 `<app_name>.exe`，双击即用、
> 不写注册表，依赖系统自带 WebView2 运行时。该产物不再手工多压一层 zip，而是把
> `<app_name>.exe` 直接上传，由 GitHub 打成 **`<app_name>-portable.zip`**（文件名含应用名，
> 且只有一层压缩，不会出现 "zip 套 zip"）。

配置模板见 **`config.example.json`**（复制到本地改好再粘贴即可）。

两个名字字段的区分（很容易混）：

| 字段 | 含义 | 影响范围 |
| --- | --- | --- |
| `app_name` | **安装名**（给系统/安装器看） | Windows 安装包名、开始菜单与桌面快捷方式、安装目录、"程序和功能"显示名；同时也是两端**数据存储目录名** |
| `title` | **显示名**（给用户看） | **Windows 窗口标题**、**Android 应用名称**（桌面图标下的名字） |

`title` 省略时自动回落为 `app_name`。二者分开后即可实现「安装包叫英文名 `MyApp`、界面叫中文名 `我的应用`」。
⚠️ 旧字段 `window_title` **已移除**（写了会在打包时直接报错）——它和 `title` 曾是同一个概念的两个名字，
现已只保留 `title`；编译期变量名也从 `WINDOW_TITLE` 改为 `TITLE`，与字段名一致。
完整字段表与两壳行为对照见 `docs/packaging-config.md`。

`extra`（可选，**对象**）—— 随打包一起注入到被包裹网页里的自定义配置，前端 JS 可直接读取：

- 会被序列化成 `window.web2appExtra = <你的对象>;` 注入到页面，例如上面的示例会被前端读到：

  ```js
  // 在你的网页里（被本应用打开后）
  window.web2appExtra            // => { build: "example", deploy_env: "prod" }
  window.web2appExtra.deploy_env // => "prod"
  ```

- 支持嵌套对象、数组、数字、布尔与中文字符串，最终都以合法 JS 对象字面量注入；
  不填时前端读到 `window.web2appExtra = {}`（空对象）。
- 典型用途：同一个网页被多个 app 复用，用不同的 `extra` 区分部署环境 / 渠道 / 站点皮肤等。

> 粘贴到 `config_json` 输入框时必须是**单行 JSON**；用 `config_url` 指向 RAW 文件（如 GitHub raw 链接）则无此限制。

`tabs_enabled`（可选，**真布尔**，默认 `false`，不带引号）—— 多页签（tab）模式开关，**Windows 与 Android 均生效但形态不同**：

- 设为 `true` 后，应用不再是一个单一窗口，而是「一个宿主窗口 + 顶部标签栏」，每个 tab 是独立的 WebView、**共享同一份数据目录**（同账号，cookie / localStorage 天然互通）。
- tab 可切换、可关闭；新 tab 由网页里的 `target="_blank"` / `window.open()` **直接新开**（标签栏的「+」按钮已有意移除，见 `src/shell.html` 注释；程序化建窗仍可走 `create_tab` 命令）。
- **新 tab 落点**（v1.52）：链接与入口**同源**（含上游真实主机）时，取目标的 path/query/fragment 拼到入口地址（本地反向代理会把所有 path 转发到同一上游）；**跨域外链直接打开目标地址**，不再回落首页。首个 tab 就绪后导航信息缓存到 `TabManager`、后续新 tab 复用；首屏未就绪时由首个到达者启动 bootstrap（`bootstrap_started` 标志判定），不再空等 30s 超时。
- **Windows**：即上面这套多 WebView 完整形态（`mod tabs` 整体 `#[cfg(windows)]`）。
- **Android**（2026-10-13 起生效）：wry 在 Android 不支持同窗口多子 webview，落地为 `mobilebar.rs` 的【单 webview 页签记录】—— 底栏「列表」面板里可新建 / 切换 / 关闭标签，首页固定 0 号位不可关；切换 = 按记录的 url **重新加载**（页面状态不保留）。入口画在底栏里，**必须同时开 `mobile_bar=true`**；只开 `tabs_enabled` 不会有任何界面变化。
- 该开关等价于编译期环境变量 `TABS_ENABLED`（v1.53–v1.81 曾以 `cfg!(windows) && …` 在非 Windows 恒为 false，移动端页签落地后该钳制已移除）；`release.yml` 模式下对应仓库变量 `TABS_ENABLED`。打包解析器写入环境变量的是 Python 风格 `True` / `False`（首字母大写），Rust 侧 `env_bool!` 按 ASCII 大小写不敏感识别（v1.50 起；此前只认小写，`True` 会静默回落 `false` —— 配置写了多页签、产物仍是单窗口）。
- ⚠️ 字符串形式（`"true"`）会被打包解析器直接拒绝（布尔字段只认真 `bool`，见 `docs/packaging-config.md`）。

```json
{ "app_url": "https://u:p@host", "tabs_enabled": true }
```

### 动态解析真实 URL（`app_url` 数组 + `public_key` 公钥解密）

当后端地址可能变动、又不想频繁重新打包时，可以把 `app_url` 配成**一组域名（JSON 数组）**，
并同时提供 `public_key`（Ed25519 公钥）。应用启动时会：

1. **依次**对数组里每个域名做 **DNS TXT 记录查询**；
2. 拿到 TXT 值（一段 base64 密文）后，用 `public_key` 做 **Ed25519 验签 + AES-GCM 解密**得到真实 URL；
3. 某个域名查询/解密/验签失败则**自动尝试下一个**；全部失败时报错 **「服务器连接失败」**。

```json
{
  "app_url": ["node1.example.com", "node2.example.com"],
  "public_key": "HkCpkH1tMpNIxWoWAkvlX8vm93BOc5PtIKrQxl8RUDo=",
  "app_name": "MyApp"
}
```

#### 安全模型

| 诉求 | 实现方式 | 安全边界 |
| --- | --- | --- |
| **防劫持（硬保证）** | 服务端 Ed25519 **私钥签名**，客户端**公钥验签** | 攻击者即使拿到公钥 + 完全控制 DNS TXT，没有私钥就无法伪造出能通过验签的密文，最多破坏记录导致解密失败（DoS），**无法劫持到恶意网址** |
| **隐藏真实网址（软保证）** | AES-256-GCM 认证加密包装 | 外部查 TXT 只能看到密文。但对称密钥由客户端内置 Secret 派生，**攻击者反编译客户端可还原**，故此层只防"公网批量爬取/随手 dig"，不防针对性反编译 |
| **每次加密不同** | Nonce(12B) 每次随机（同时充当 HKDF salt，派生密钥也每次不同） | 同一 URL 多次加密密文完全不同，防重放/防密文比对猜测 |
| **密文尽可能短** | 固定开销 97B（Nonce12 + 分隔/签发日/有效期 5 + 签名64 + tag16）+ URL 本体 | v4 唯一格式 base64 后通常 160~180 字符，**单条 DNS TXT 可放下**（单串上限 255 字节）；v1/v2/v3 旧格式不再兼容 |

要点：

- **数据格式（v4 唯一）**（服务端 `tools/encrypt_url.py` / `resolve-tool` 与客户端
  `resolve-core`（`crates/resolve-core`）一致）：
  `TXT = base64( Nonce(12B) ‖ AES-256-GCM(HKDF(Secret,Nonce), Payload ‖ Ed25519签名) )`，
  其中 `Payload = URL ‖ 0x00 ‖ 签发日d(2B) ‖ 有效期ttl_d(2B)`（分隔符后尾部固定 4B）。
  **解密端只认这一种结构**：v3（秒级 16B 尾）/ v2（带盐 25B）/ v1（无 ttl 17B）
  记录一律报「Payload 尾部结构异常」拒绝，线上旧 TXT 需用当前工具重新签发（详见
  `docs/txttest-verify-materials.md`）。
- **Secret：客户端编译期注入不变，服务端加密支持 `--secret` 逐次指定**：
  - 客户端侧，AES 密钥源头仍是 `resolve-core` 编译期的 `option_env!("RESOLVE_SECRET")`：
    构建前在 shell 设好自定义 Secret（cargo 自动追踪该 env，改值会重编 `resolve-core`），
    不设则用内置默认串 `web2app-fixed-secret-2026`。
  - 服务端侧，`resolve-tool encrypt/all` 与 Python 工具自 v1.46 起支持 `--secret <值>`
    逐次调用显式传入，取值优先级 **参数 > 环境变量 `RESOLVE_SECRET` > 内置默认**，同一份
    工具二进制不必为不同 Secret 分别编译；PowerShell/cmd 的 env 老用法继续有效（设了 env
    就省略 `--secret`）。两端 Secret 不一致时 HKDF 派生密钥不同、客户端必然解密失败。
- **签发日与有效期（ttl_days）由服务端生成时决定**：时间以 **UTC 天**为单位，
  `d` = 签发日距固定基准日 **2026-01-01** 的天数（u16，可覆盖至约 2205 年），
  `ttl_d` 是随密文下发的有效期天数：
  - `ttl_d > 0`：客户端校验 `今日 - d <= ttl_d`，超出即拒绝（最小粒度 1 天）；
  - `ttl_d = 0`：**永不失效**，客户端跳过日期校验（适合后端地址长期不变的场景）。
  这样是否设有效期、设多长，完全由生成密文的一方决定，客户端无需重新编译。
- **`public_key` 格式**：Ed25519 公钥的**裸 base64（32 字节）**，或 SPKI PEM（`BEGIN PUBLIC KEY`）。
  推荐直接用 `genkey` 生成的 `public_key.b64` 内容。
- **查询通道**：走系统默认 DNS（读 `/etc/resolv.conf` 或 Windows 系统配置）。
- **降级**：`app_url` 为普通字符串时行为与原来完全一致（忽略 `public_key`）。

生成密钥对 + 加密 URL，有两种等价方式（私钥格式与密文格式完全互通）：

1. **纯 Rust 工具 `resolve-tool`**（推荐，无需 Python 依赖；独立 crate，不编译 Tauri）：
   ```bash
   cargo run -p resolve-tool -- genkey --out keys   # 生成私钥(服务端持有) + 公钥(客户端)
   cargo run -p resolve-tool -- genkey --stdout     # 不落盘：私钥 PEM/公钥直接打印到控制台（等价于双击 scripts/gen-resolve-keys.bat）
   cargo run -p resolve-tool -- encrypt keys/private_key.pem "https://user:pass@host" --ttl-days 0
   cargo run -p resolve-tool -- encrypt keys/private_key.pem "https://user:pass@host" --ttl-days 7 --count 3
   # 客户端构建期注入了自定义 Secret 时，用 --secret 逐次显式传入（值必须与客户端一致）：
   cargo run -p resolve-tool -- encrypt keys/private_key.pem "https://user:pass@host" --ttl-days 0 --secret "与客户端一致的 Secret"
   # encrypt 第一个参数也接受内联 PEM 字符串（以 -----BEGIN 开头，可折叠成单行、勿带 # 注释行），配合 genkey --stdout 全程不落盘：
   cargo run -p resolve-tool -- encrypt "-----BEGIN PRIVATE KEY-----<PEM 本体拼成一行>-----END PRIVATE KEY-----" "https://user:pass@host" --ttl-days 0
   ```
   `--ttl-days 天`：默认 **0 = 永不失效**；正数 = 生成后 N 天内有效（最小粒度 1 天，上限 65535），到期客户端拒绝（防重放/防过期需显式开启）。
   `--count N`：每个 URL 生成 N 份密文（随机 Nonce，密文不同但解密结果相同），可把同一个
   地址分发到多个域名 / 多条 DNS TXT 记录。
   `--secret <值>`：显式指定本次加密的 Secret（优先级高于环境变量；空串报错），不传则回落
   编译期/环境变量 `RESOLVE_SECRET` → 内置默认。
   也可用 `all` 一键生成密钥对 + 密文：`cargo run -p resolve-tool -- all "https://..." --out keys --count 3`（同样支持 `--secret`）。
   **解密验证**一条 TXT 密文（AES-GCM 认证 + Ed25519 验签，通过后打印真实 URL、签发日期与 TTL 天数；公钥为文件路径或直接内联的裸 base64/SPKI PEM；密文省略时走 stdin）：
   ```bash
   cargo run -p resolve-tool -- decrypt "<公钥裸 base64>" "<TXT 密文>" --secret "与客户端一致的 Secret"
   echo "<TXT 密文>" | cargo run -p resolve-tool -- decrypt "<公钥裸 base64>"
   ```
   过期记录（ttl_days>0 且已超时）仍展示解密内容，但退出码为 1，与客户端「过期即拒绝」一致。

2. **Python 脚本 `tools/encrypt_url.py`**（需 `pip install cryptography`）：
   ```bash
   python tools/encrypt_url.py genkey keys          # 生成私钥 + 公钥
   python tools/encrypt_url.py keys/private_key.pem "https://user:pass@host" --ttl-days 0
   python tools/encrypt_url.py keys/private_key.pem "https://user:pass@host" --ttl-days 0 --secret "与客户端一致的 Secret"
   # 与 resolve-tool 相同，第一个参数也支持直接传内联 PEM 字符串（多行完整 PEM，勿带 # 注释行）；--secret 同名同义（参数 > 环境变量 > 默认）
   ```

把密文作为某个域名的 TXT 记录值，把 `keys/public_key.b64` 的内容作为配置里的 `public_key`。

> 也可以直接在 GitHub Actions 里跑 **`resolve-keys.yml`** 这个 workflow（手动触发），
> 它提供「生成密钥对」「生成 TXT 密文」「一键生成密钥对 + 密文」三个操作，全程在 CI 上完成，
> 产出打包成 artifact 下载。

### Android 签名（打包时从三层来源取第一份可用的）

手动打包产出的 release APK **必须签名**才能安装。签名密钥沿三层策略**依次取第一份可用的**：

| 优先级 | 签名来源 | 适合场景 |
| --- | --- | --- |
| 1 | 配置 JSON 里的 `android_keystore_base64` 等 4 个字段 | 每个应用各持一把正式密钥，随配置一起分发 |
| 2 | 仓库 Secrets（`ANDROID_KEYSTORE_BASE64` / `ANDROID_KEY_ALIAS` / `ANDROID_KEYSTORE_PASSWORD` / `ANDROID_KEY_PASSWORD`） | 长期使用 / 准备上架，各次构建可互相覆盖升级 |
| 3 | 都没有 → CI 现场用 keytool 生成**一次性** keystore 签名 | 临时试装；但每次构建签名都不同，**新版无法覆盖安装旧版**（需先卸载） |

> ⚠️ **正式使用请务必用方式 1 或 2**，并长期保存 keystore 文件与密码。Android 应用升级必须用**同一把密钥**签名，密钥一旦丢失，已装应用将永远无法覆盖升级。

## 4. 生成签名密钥（推荐）— `.github/workflows/generate-signing-key.yml`

> 这是最省心的签名方式：**云端全自动生成一把固定 keystore，零输入、随机密码**，跑一次就把「密钥文件 + 密码 + 已签好名的配置 JSON」打包成一个 artifact 给你下载。

**为什么用它**：手动打包要"签名一致、可覆盖升级"，就需要一把**固定**的 keystore 反复用。
本 workflow 在云端用 JDK 的 `keytool` 生成这把密钥（随机强密码），并自动填进一份打包配置 JSON，你无需在本机装 JDK、也无需手敲 `keytool` / `base64` 命令。

**用法（3 步）**：

1. Actions → **「生成 Android 签名密钥 (keystore)」** → Run workflow（**无需填任何东西，全自动**）。
2. 下载 artifact **`android-signing-key`**，里面有 4 个文件：
   - `release.keystore` —— 密钥库文件本身，请**长期妥善保存**（丢了无法找回）；
   - `keystore-info.txt` —— alias、keystore 密码、base64（单行）+ 使用提醒；
   - `packaging-config.json` —— 已把签名 4 项填好的打包配置模板（美化版）；
   - `packaging-config.single.json` —— 同一配置的单行版，直接粘给 `build-custom.yml` 的 `config_json`。
3. 打开 `packaging-config.json`，把 `app_url` / `app_name` / `title` 改成你的真实应用（**签名与网址无关**，改了仍有效），
   然后把 single 版粘进 **build-custom.yml** 打包。

> ⚠️ **每次运行本 workflow 都会生成一把新密钥**。因此只运行一次、保存好这把；不要为了"再拿一把"而重复运行，
> 否则新旧密钥签的 APK 之间无法覆盖升级（要先卸载）。若希望以后打包**自动**用同一把（不用每次粘 base64），
> 把 `keystore-info.txt` 里的 base64 / alias / storepass 配成仓库 Secrets（`ANDROID_KEYSTORE_BASE64` /
> `ANDROID_KEY_ALIAS` / `ANDROID_KEYSTORE_PASSWORD`）即可，`build-custom.yml` 会自动读取。

## 5. 本机手敲 keytool 生成（备选）

不想用云端 workflow 时可在本机生成（需 JDK；装了 Android Studio 可直接用它自带的 `jbr\bin\keytool.exe`）：

**① 生成一把 keystore**：

```bash
keytool -genkeypair -v \
  -keystore release.keystore \
  -alias my-app-alias \
  -keyalg RSA -keysize 2048 -validity 10000 \
  -storepass YourStorePass -keypass YourKeyPass \
  -dname "CN=MyApp, OU=Dev, O=MyCompany, L=City, ST=State, C=CN"
```

字段对应关系：`-alias` → `android_key_alias`，`-storepass` → `android_keystore_password`，
`-keypass` → `android_key_password`（可省略，默认同 storepass）；`-validity 10000` 天 ≈ 27 年，够用。

**② 编码成单行 base64**（CI 只收 JSON 里的 base64，收不了文件）：
```bash
base64 -w0 release.keystore     # Git Bash / macOS / Linux
```
```powershell
[Convert]::ToBase64String([IO.File]::ReadAllBytes("C:\path\to\release.keystore"))
```

**③ 填入配置 JSON**：即 `config.example.json` 的字段，再加上 ② 的输出 → `android_keystore_base64`，
字段名严格一致（可直接参考 `generate-signing-key.yml` 产出的 `packaging-config.single.json`）。

> ⚠️ `android_keystore_base64` 是 keystore 的**明文等价物**（任何人拿到都能用它给同包名应用签名冒充你），
> 等于一把私钥。**不要**把它提交进公开仓库的 JSON / 文档 / 历史记录；正式长期使用更推荐放仓库
> **Secrets**（方式 2），配置 JSON 里只留方式 1 用于「每个应用各自一把密钥、随配置分发」的场景。
> 想验证 JSON 里的 keystore 是否有效，可用 `keytool -list -keystore <解码后的.jks> -storepass <密码>` 查看别名。
