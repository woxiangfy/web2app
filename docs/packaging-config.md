# 打包配置 JSON 契约（多端行为对照）

> **这份文档是打包配置的契约说明。** 字段名、类型、默认值、校验规则的**唯一实现**是
> [`.github/scripts/parse-packaging-config.py`](../.github/scripts/parse-packaging-config.py)（含内置自测）。
> 文档与之冲突时**以脚本为准**；改脚本 = 改契约，必须同步本文件。
>
> 另有一个运行期对等实现：[`electron-app/src/main/config.ts`](../electron-app/src/main/config.ts)
> （Electron 绿色版读 asar 内的 `config.json`），校验规则逐条对齐。

---

## 1. 一句话模型

**一份配置 JSON → 三个产物。** 同一份 JSON，在三条流水线上得到**同样语义**的产物：

| 产物 | 入口 workflow | 配置什么时候被读 |
|---|---|---|
| Windows 安装版（MSI）/ 单文件绿色版（zip） | `.github/workflows/build-custom.yml` | **编译期**：值被写成二进制里的常量 |
| Android APK（已签名） | `.github/workflows/build-custom.yml`（勾选 `android`） | **编译期**：同上 |
| Windows 绿色版 / 安装版（Electron，zip / portable exe / NSIS setup） | `.github/workflows/electron-build-custom.yml` | **打包期**：规范化后写进 `app.asar` |

> ⚠️ **两个壳都是「改配置必须重新打包」**，区别只是 Tauri 侧需要重新**编译**、Electron 侧只需重新**打包**。
> 两者都**不支持**交付后改配置——这正是配置被放进程序本体的原因（见 §6）。
>
> 另有一条**不打自定义包**的发版路径：`release.yml`（在 Release 页 **Publish 一个 Release**
> 时触发；光 push tag 不触发），配置来自**仓库变量 / Secrets**，字段名不同
>（`TITLE` / `PROXY_ENABLED` …）。见 §7.3。
>
> 还有一条**本机**路径：`scripts/pack-local.mjs`（不用 Actions，直接在本机出包，只依赖 Node）。
> ⚠️ 它是本解析器的 **Node 复刻**而不是复用（Python 不是必然存在的依赖）⇒ 两侧靠
> `--parity` 逐项比对钉住等价。见 §7.2。

---

## 2. 字段总表

共 **18 个字段**。除 `app_url` 必填外，其余全部可省略并自动合并默认值。

| 字段 | 类型 | 默认值 | 平台 | 含义 |
|---|---|---|---|---|
| `app_url` | **`string` \| `string[]`** | 必填 | 全平台 | 要嵌入的地址。两种形态见 §3.1 |
| `app_name` | string | `"Web2App"` | 全平台 | **安装名**：安装包名 / 快捷方式 / 安装目录 / "程序和功能"显示名；**同时是数据目录名**，并派生出 **Android 包名**（见 §4.2） |
| `title` | string | `""` → 回落 `app_name` | 全平台 | **显示名**：窗口标题 / 托盘悬停 / 设置窗标题（Win）；Android 应用名（桌面图标下） |
| `extra` | 任意 JSON | `{}` | 全平台 | 注入被包裹网页的附加参数，前端读 `window.web2appExtra` |
| `icon_url` | string | `""` | 全平台（**构建期消费**） | 应用图标 PNG 的 URL；空 = 沿用仓库默认图标 |
| `proxy_enabled` | **`bool`** | `false` | 仅 Windows | 启用本地反向代理（统一注入 Basic 鉴权，覆盖 WebSocket） |
| `proxy_http` | string | `""` | 仅 Windows | 外部 HTTP 代理地址 |
| `proxy_https` | string | `""` | 仅 Windows | 外部 HTTPS 代理地址（留空 = 复用 `proxy_http`） |
| `proxy_bypass` | string | `"localhost;127.0.0.1;<-loopback>"` | 仅 Windows | 不走代理的地址，**分号分隔** |
| `data_dir` | **string** | `"data"` | Win + Electron | 应用数据目录。`"data"` = 程序所在目录下的 `data\`（跟着程序走）；`""` = 系统默认程序目录（由 `app_name` 命名）。运行期文件（`settings.json` / `logs\`）都落在这个目录下。见 §4.3 |
| `tabs_enabled` | **`bool`** | `false` | 仅 Windows | 多页签模式（Electron 亦可）；Android 忽略 |
| `debug` | **`bool`** | `false` | 全平台 | 诊断日志开关：`true` = 写 `<数据目录>\logs\startup.log` 并同时打印；`false` = **不建目录、不写文件，控制台输出也一并静默**。见 §3.5 |
| `public_key` | string | `""` | 全平台 | Ed25519 公钥（裸 base64 或 SPKI PEM）。**`app_url` 为数组时必填** |
| `resolve_secret` | string | `""` → 代码内置默认 | 全平台 | AES 对称 Secret，须与服务端生成密文时一致。⚠️ **编译期**注入 |
| `android_keystore_base64` | string | `""` | 仅 Android | keystore(.jks/.keystore) 的 base64；优先于仓库 Secrets |
| `android_key_alias` | string | `""` | 仅 Android | keystore 别名（给了 keystore 则必填） |
| `android_keystore_password` | string | `""` | 仅 Android | keystore 密码（给了 keystore 则必填） |
| `android_key_password` | string | `""` | 仅 Android | key 密码（可空 = 同 keystore 密码） |

### 已移除的字段

| 字段 | 状态 | 改用 |
|---|---|---|
| `window_title` | **已移除**，配置里出现即**报错** | `title`（语义完全相同） |
| `data_dir_default` | **已移除**（旧为布尔），配置里出现即**报错** | `data_dir`（字符串）：`false` → `"data"`、`true` → `""`（详见 §3.3 / §4.3 / §8） |

移除原因：同一个「显示名」概念曾有三套名字（JSON 的 `title` / `window_title`、仓库变量的
`APP_TITLE` / `WINDOW_TITLE`）。名字越多，越容易出现「同一份配置在两个壳里标题不同」这种
不报错的缺陷。`data_dir_default` 是布尔，表达不了「相对路径 / 绝对路径 / 系统目录」三种取值，
改成字符串 `data_dir` 后两壳同名同义（§3.3 / §4.3）。

### 类型约定（三条硬规则）

1. **布尔一律写真 `bool`，不带引号。** `"proxy_enabled": "false"` 这种字符串写法**已不再接受**，
   会直接失败并给出改法。
2. **`app_url` 支持真 JSON 类型 `string` 或 `string[]`**，不再是「字符串里套 JSON」。
3. **未知字段硬失败**（含近似名建议）。历史上未知字段只打 `::warning::` 就跳过，
   于是「字段名拼错」和「字段不生效」表现完全一样，只能靠猜。

---

## 3. 重点字段详解

### 3.1 `app_url` —— 直连地址 或 跳板域名数组

**形态一：字符串（直连）**

```json
{ "app_url": "https://user:pass@example.com" }
```

URL 里的 `user:pass@`（userinfo）由**本地代理**转换成 `Authorization: Basic` 注入上游，
这样凭据不出现在页面的 JS 里。

**形态二：字符串数组（DNS TXT 跳板）**

```json
{
  "app_url": ["a.com", "b.com"],
  "public_key": "HkCpkH1tMpNIxWoWAkvlX8vm93BOc5PtIKrQxl8RUDo="
}
```

运行时**按序**对每个域名做 DNS TXT 查询，TXT 值是一段 base64 密文，用 `public_key` 验签 +
AES-GCM 解密得到真实 URL。任一域名解析 / 解密 / 验签失败自动尝试下一个，全部失败则报
"服务器连接失败"。

> ⚠️ **数组形态必须提供 `public_key`** —— 没有公钥就无法验签。解析器在打包期就会强制校验。
>
> **兼容**：老配置里 `"app_url": "[\"a.com\"]"`（字符串形态的 JSON 数组文本）仍可用，
> 但会打一条 warning 提示改成真数组。

### 3.2 `app_name` vs `title` —— 安装名 / 显示名分离

两者刻意分开，为了支持「**安装包叫英文名、界面叫中文名**」：

| | `app_name`（安装名） | `title`（显示名） |
|---|---|---|
| 给谁看 | 系统 / 安装器 | 用户 |
| Windows | 安装包名、开始菜单与桌面快捷方式、安装目录、"程序和功能" | 窗口标题、托盘悬停、右键菜单"设置"标题 |
| Android | **包名（applicationId）由它派生** = `com.web2app.<清洗后的值>` | 桌面图标下的名字 |
| Electron | `productName`（zip/portable 文件名）、`%APPDATA%` 子目录名（`data_dir` 为空串时） | 窗口标题、托盘悬停 |
| 缺省 | `Web2App` | 回落 `app_name` |

> ⚠️ **`app_name` 语义过载**：它同时是「安装名」「数据目录名」和「Android 包名来源」。
> 应用发布后**改 `app_name` 等于丢掉全部登录态**（数据目录换了名字），而且会换掉 Android 包名
> —— 在手机上那就是**另一个应用**（不能覆盖升级）。改之前请想清楚。

### 3.3 布尔字段：为什么不再接受 `"true"`

旧模板写的是 `"proxy_enabled": "false"`。字符串 `"false"` 在**真值判断下是 true**，
所以两个壳各自用不同的宽容规则去"猜"它：

| 写法 | Tauri（`env_bool!`） | Electron（旧 `toBool`） |
|---|---|---|
| `"true"` / `"1"` / `"yes"` | `true` | `true` |
| `"false"` / `"0"` / `"no"` | `false` | `false` |
| `"on"` | **`false`**（不在白名单） | **`true`** |

同一份配置里写 `"on"`，两个壳的判定**相反**。这类缺陷不报错、只在运行时表现为行为差异，
是最难查的一类。**现在字符串一律拒绝**，歧义在打包期就消灭。

哪些字段是布尔：`proxy_enabled`、`tabs_enabled`、`debug`。

> 注：真 bool 经打包解析器写入环境变量时是 Python 风格的 `True` / `False`（首字母大写，
> CI 的 `write_env` 与本地 Node 复刻同源），Tauri 的 `env_bool!` 按 **ASCII 大小写不敏感**
> 识别（`True` / `TRUE` 等价于 `true`）；`"on"` 无论如何都不认。

> ⚠️ 数据目录**曾经也是布尔**（旧字段 `data_dir_default`），现已改成**字符串** `data_dir`——
> 布尔只能表达「二选一」，而这个概念需要表达「哪个相对路径 / 哪个绝对路径 / 用系统目录」。
> 见 §4.3。

### 3.4 `resolve_secret` —— 编译期注入，改完必须重新打包

它是 `resolve-core` 里 `option_env!("RESOLVE_SECRET")` 的**编译期常量**：

- **Tauri 侧**：编进主程序；
- **Electron 侧**：编进 sidecar `w2a-proxy.exe`（它是**单独编译**的）。

⚠️ 因此 Electron 手动打包 workflow 里，「解析配置」必须**排在「编译 sidecar」之前**——
保证编译 sidecar 时 `RESOLVE_SECRET` 已在环境里。顺序错了，sidecar 内置的是公开默认
Secret，用户配的 `resolve_secret` 会**静默失效**（域名解析一律失败，且不报错）。

未配置时的行为**取决于 `app_url` 形态**（判据与 `resolve-core::parse_domains` 同源：文本以 `[` 开头即域名数组）：

- **域名数组模式**：打 warning —— 所有产物共享同一个固定密钥串，安全门槛较低；
- **单 URL 字符串模式**：**不打 warning**，只输出一行「该字段当前不参与」的说明。
  理由：客户端解密的唯一入口是 DNS TXT 那条路（Tauri `bootstrap.rs` 里 `parse_domains(APP_URL)`
  命中才走，Electron 是 `navigate.ts` 的 `needsDnsResolve` = `Array.isArray(app_url)`），
  字符串模式只走 direct / proxy 两条路，`SECRET`（`resolve-core::derive_key` 的唯一消费者）
  **没有任何调用点** ⇒ 此时告警属于「狼来了」，会把真该修的配置问题淹掉。

### 3.5 `debug` —— 诊断日志开关（不是「日志级别」）

两态开关，管的是「有没有诊断输出」这件事本身：

| 取值 | 行为 |
|---|---|
| `false`（**默认**） | 不建 `logs\`、不写文件；**控制台输出也一并静默**（Electron 侧 `console.log/warn/error` 被换成空函数） |
| `true` | 写 `<数据目录>\logs\startup.log`，**并且**照常打印到 stdout / stderr |

- **为什么是布尔而不是 `off/error/info/debug` 那种级别**：分级会让默认值变成「有一条输出但不告诉你」。
  排查时最先要回答的问题是「**到底有没有日志**」，四态里没有哪一态能一眼回答，两态可以。
- **默认必须是 `false`**：默认开会在**用户的数据目录里留下交付物以外的文件**（`data\logs\`）。
  用户拿到的应该是「一个程序」，不是「一个程序 + 一份他从没要过的日志」。
  ⚠️ 两壳都曾踩过这个坑：Electron 侧此前**无条件**落盘（2026-09-18 引入），Tauri 侧则根本
  没有出口（GUI 子系统下 `eprintln!` 等于没有输出）。现在统一到同一个开关。
- **运行时逃生口**（不改配置、不重新打包，用于现场排障）：
  - 命令行加 `--debug`；
  - 或设环境变量 `WEB2APP_DEBUG=1`（真值白名单与 `env_bool!` 逐字一致：`1` / `true` / `yes`）。
- ⚠️ **两个环境变量名不是一回事，别混**：

  | 名字 | 谁在用 | 什么时候读 |
  |---|---|---|
  | `DEBUG` | **打包期**注入（`write_env` 写一行 `DEBUG=True`），对应 `config.rs` 的编译期常量 | 编译期（`option_env!`） |
  | `WEB2APP_DEBUG` | **运行期**逃生口 | 每次启动（`debug_log::enabled()` / Electron 的 `process.env`） |

  打包期的名字沿用「字段名大写」的既有规则（`TABS_ENABLED` / `PROXY_ENABLED` 同理），
  不为此搞特例；运行期那个特意加了 `WEB2APP_` 前缀 —— `DEBUG` 是 shell / Node / Python / 大量库
  都在用的通用名，外层环境里随便一个 `DEBUG=1` 就会让交付物开始写日志，属于「莫名其妙多个文件」。
- **两壳必须同义**：Tauri 侧实现是 `webview-app/src/debug_log.rs`（`dlog!` 宏），
  Electron 侧是 `electron-app/src/main/startup-log.ts`；落点规则都遵循 §4.3 的 `data_dir` 规则
  ⇒ **`data_dir` 一改，两壳的日志位置一起改**。
- **唯一例外**：Electron 的探针（`--probe`）与冒烟（`--smoke`）**内部强制打开** debug。
  这两个模式的全部结论来自 stdout，不能因为「默认关日志」而自检失声。
- **sidecar（`w2a-proxy.exe`）自己不做门控**：它是**控制台进程**，`stderr` 由宿主逐行读走
  （`sidecar.ts` 的回调 → `index.ts` 的 `log: (line) => console.log(line)`），再按**宿主的**
  `debug` 决定去向 —— 关闭时宿主的 `console.log` 是空操作，这些行不落到任何地方；打开时才进
  stdout 与 `startup.log`。所以那边共享源码里的 `dlog!` 直接展开成裸 `eprintln!`：**门控归宿主**，
  在 sidecar 里再复制一份判定只会多出第二个可能与宿主不一致的真值来源（§3.3 的 `"on"` 就是这么来的）。
  > 实现坑：`macro_rules!` 不 `#[macro_export]` 时**只在定义它的 crate 内可见**，而共享源文件被
  > sidecar 用 `#[path]` 引进来算 sidecar 自己的模块 ⇒ 主 crate 那份 `dlog!` 在 sidecar 里
  > **根本看不见**（`cannot find macro dlog`，加 `crate::` 全限定也无效 —— 找不到的是宏名本身）。
  > 因此 `crates/proxy-server/src/main.rs` 里自带一个同名宏，且必须声明在 `mod proxy;` **之前**。

### 3.6 日志文件契约（两壳逐字相同）

| 项 | 约定 |
|---|---|
| 文件名 | `startup.log`（轮转后的旧文件 `startup.prev.log`，**只留一代**） |
| 轮转阈值 | 1 MiB，且在**启动时**检查（运行期不 stat，避免每行一次系统调用） |
| 写失败 | **fail-open**：目录建不出 / 文件打不开 ⇒ 本次静默跳过，**不影响启动**（为了记日志让应用起不来是本末倒置） |
| 凭据 | **绝不记凭据**：地址一律去掉 `user:pass@`（`mask_url`），Secret 只报「已配置 / 未配置」 |

> 文件名、轮转阈值两处常量与 Electron 侧刻意**同值**（`debug_log.rs::LOG_FILE_NAME` /
> `MAX_BYTES` ↔ `startup-log.ts::LOG_FILE_NAME` / `MAX_BYTES`）——「同一份契约」不能只写在文档里。

---

## 4. 多端行为对照

### 4.1 总表

| 维度 | Windows（Tauri） | Android（Tauri） | Windows 绿色版（Electron） |
|---|---|---|---|
| 配置读取时机 | 编译期常量 | 编译期常量 | 打包期写进 `app.asar` |
| 改配置 | 重新编译 | 重新编译 | 重新打包 |
| `app_name` | 安装包名 / 数据目录名 / **Android 包名**（派生） | 同左 | `productName` / `%APPDATA%` 子目录名 |
| `title` | 窗口标题 + 托盘提示 + 「设置」窗标题 | **应用名称**（桌面 label） | 窗口标题 + 托盘提示 |
| `tabs_enabled` | ✅ 有效 | ❌ 忽略 | ✅ 有效 |
| `debug` | ✅ 写 `<数据目录>\logs\startup.log` | ✅ 写应用私有目录下的 `logs\startup.log` | ✅ 写 `<数据目录>\logs\startup.log`；探针 / 冒烟**强制开** |
| `proxy_*` | ✅ 有效 | ❌ 忽略 | ✅ 有效 |
| `data_dir` | 见 §4.3 | ❌ 忽略（始终私有目录） | 见 §4.3 |
| `icon_url` | 构建期 `cargo tauri icon` | 同左 | 构建期交给 electron-builder 转 ico |
| `android_*` | ❌ 忽略 | ✅ 用于签名 | ❌ 忽略（且不写进包内 config.json） |
| `public_key` | ✅ 数组模式必填 | ✅ 同左 | ✅ 同左 |
| `resolve_secret` | 编进主程序 | 编进主程序 | 编进 sidecar `w2a-proxy.exe` |

### 4.2 Android 侧的三个额外动作

1. **应用名称（桌面图标下的 label）取 `title`** —— `init` 之后打补丁覆盖 `strings.xml` 与
   Manifest 内联 label（未配 `title` 时回落 `app_name`）；
2. **包名（applicationId）由 `app_name` 派生** = `com.web2app.<清洗后的 app_name>`。
   必须在 `init` **之前**改写 `tauri.conf.json` 的 `identifier` —— Tauri 2 **没有**
   `bundle.android.package` 字段，安卓包名只能来自 `identifier`。
   清洗规则：只留 ASCII 字母/数字/下划线并小写；首字符是数字则加 `app` 前缀；
   一个 ASCII 字符都没有（如纯中文名）则用名字指纹兜底；命中 Java 关键字则补下划线。
   ⇒ 不同 `app_name` 得到不同包名，**可在同一台手机上共存**（此前固定 `com.example.web2app`，
   不同配置的 APK 会互相覆盖安装）。要换成自己公司的域前缀（如 `com.yourcorp.*`），
   改 workflow 里那一行 `pkg = "com.web2app." + seg`。
3. `android_*` 4 个字段落盘成 `release.keystore` + `ANDROID_KEYSTORE_*` 环境变量，
   并 `::add-mask::` 掩码密码。keystore 由 **CI 一次性生成的专用 keystore** 兜底（未提供时）。

### 4.3 `data_dir`：四种取值，两壳同一套规则

| 取值 | Windows（Tauri） | Windows 绿色版（Electron） |
|---|---|---|
| `"data"`（**默认**） | `<exe 目录>\data\` | `<exe 目录>\data\` |
| `""`（空串） | `%LOCALAPPDATA%\<app_name>` | `%APPDATA%\<app_name>` |
| 其它相对路径 | `<exe 目录>\<该路径>` | `<exe 目录>\<该路径>` |
| 绝对路径 | 原样使用 | 原样使用 |

**四条要点**：

1. **相对路径的基准是 exe 所在目录，不是 `cwd`。** 进程工作目录会随启动方式变化（快捷方式的
   「起始位置」、命令行 `cd` 后启动、从资源管理器拖拽启动），用它会让同一个包在不同启动方式下
   把数据落到不同地方 —— 用户表现为「设置丢了 / 又要重新登录」。两壳都取「程序自身所在目录」
   （Tauri：`std::env::current_exe()` 的父目录；Electron：`installRoot()`）。
2. **空串是「有含义的取值」，不是「未设置」。** 它表达「用系统默认程序目录，别跟着程序跑」。
   ⚠️ 这条在意料之外的地方埋了两个坑，两处都已按「保留空串」实现：
   - Rust 侧**不能**用 `env_str!` —— 它把空串当未设置并回落到默认值，必须用裸 `option_env!`；
   - shell / GitHub 变量侧**不能**用带冒号的 `${DATA_DIR:-data}` 或 `vars.DATA_DIR || 'data'`
     —— 它们把「已设置但为空」也替换掉，必须用 `${DATA_DIR-data}`。
3. **不可写会自动回退**，保证「能启动」优先于「够绿色」。安装版（MSI）默认装进
   `Program Files`，标准用户在那里没有写权限；若不回退，WebView2 初始化会失败、
   **窗口直接打不开**。两壳都用**真写文件探测**（建目录 → 写探针文件 → 删除），
   失败则退回系统默认目录并打印一行原因。
   > 只调 `create_dir_all` / `access(W_OK)` 是不够的：写保护 U 盘、只读网络盘、
   > 企业策略下的 `Program Files` 都可能「建目录成功但写文件失败」。
4. **两壳的「系统默认」不是同一个磁盘位置**（Tauri 在 `%LOCALAPPDATA%`、Electron 在
   `%APPDATA%`），这是各自框架的惯例位置；但**语义完全相同**：由 `app_name` 命名、
   与程序位置无关。⚠️ **两壳的日志目录 `<dataDir>\logs` 都随之移动** —— 这不再是 Electron 独有
   （见 §3.5）：`data_dir` 一改，两边的日志位置**一起**改。Android 不走这套规则，用应用私有目录。
5. **运行期的 `settings.json`（设置 + 窗口几何）也在这个目录里**（2026-09-23 起）：
   `<data_dir>\settings.json` —— 由 `config::store_dir`（Tauri）/ `storeLocation`（Electron）
   统一决定，与 WebView2 用户数据目录、日志目录**同一个**。此前它固定放在
   `%APPDATA%\<app_name>\`，于是选了绿色版（`data_dir = "data"`）之后，cookie / 缓存 / 日志
   都跟着 exe 走，**唯独设置文件留在系统盘** —— 同一套「程序的数据」里唯一不听话的一项。
   ⚠️ 旧位置保留为**读取候选**（升级兼容）：新位置还没有文件时读它、第一次写盘即落到新位置，
   此后旧文件是死文件（不迁移、不删除，可手工删）。
   ⚠️ `data_dir = ""` 时两壳的系统默认目录不同（见上一条）⇒ 那种模式下这个 JSON 各自落在
   自己那份数据目录里，属数据目录既有差异的自然结果。
   ⚠️ Android 不适用：它是应用私有目录，文件在 `/data/data/<pkg>/<app_name>/settings.json`。

> **与旧字段的对照**：旧的 `data_dir_default`（布尔）两壳**同名反义** —— Tauri `false` =
> `%LOCALAPPDATA%`、Electron `false` = exe 同级 `data\`，仅凭 JSON 看不出来。
> 新方案把「跟着程序走」变成两壳的**同一个默认值且同名同义**：
> `false` ⇒ `"data"`、`true` ⇒ `""`。

### 4.4 `proxy_bypass` 的默认值十处必须一致（已被自测钉住）

同一个串 `localhost;127.0.0.1;<-loopback>` 是跨壳、跨阶段的**单一字面量契约**，同时出现在
以下 10 个入库文件里。`node scripts/pack-local.mjs --selftest` 会逐个读这些文件校验，
改掉一个其余没跟上 ⇒ 本地与 CI 的自测直接失败（CI 见 `.github/workflows/ci.yml`）。

**① 运行期默认（程序读取，2 处）**

1. `webview-app/src/config.rs` 的 `env_str!("PROXY_BYPASS", …)`（Tauri 编译期默认）；
2. `electron-app/src/main/config.ts` 的 `DEFAULTS.proxy_bypass`（Electron 运行期默认）。

**② 打包期默认（脚本读取，3 处）**

3. `.github/scripts/parse-packaging-config.py` 的 `DEFAULTS["proxy_bypass"]`（CI 真源；
   同文件顶部字段表与文件尾部内置自测还各出现一次）；
4. `scripts/packlib/constants.mjs` 的 `DEFAULTS.proxy_bypass`（Node 复刻；
   断言在 `scripts/packlib/selftest.mjs`，`--parity` 另钉住它与第 3 项逐字等价）；
5. `electron-app/scripts/stage-assets.cjs` —— 没有 `config.json` 时生成的全字段兜底模板。

**③ 示例 / 工作流 / 文档片段（5 处）**

6. `webview-app/config.example.json`；
7. `electron-app/config.example.json`；
8. `.github/workflows/generate-signing-key.yml`（签名密钥向导生成给用户的配置模板）；
9. `.github/workflows/build-custom.yml`（手工触发工作流输入框的注释说明）；
10. `webview-app/AGENTS.md`（Rust 配置示例里的 `const PROXY_BYPASS` 片段）。

⚠️ `electron-app/config.json`（**未入库**、本地调试用）由 pack-local 打包时按第 4 项规范化
生成，是产物不是真源，不在此清单内。

**留空 = 不下发 bypass 参数**（即空值不覆盖 Chromium 默认）。任一真源不一致都会导致
「没配 bypass 时两个壳的直连范围不同」。

---

## 5. 校验与报错行为

解析器（打包期）与 `config.ts`（运行期）**逐条对齐**，任何一边放宽都会让
「CI 通过、双击却起不来」或反过来。

| 情况 | 行为 |
|---|---|
| 未知字段 | **硬失败**，附近似名建议（`proxy_enable` → 是否想写 `proxy_enabled`？） |
| `window_title` / `data_dir_default` | **硬失败**，提示改用 `title` / `data_dir`，并把新旧值对应关系讲清楚 |
| 布尔写成字符串 | **硬失败**，给出正确/错误写法对照 |
| `debug` 为 `true` | **不打 warning**，但要清楚：产物会在用户数据目录里留下 `logs\`（§3.5/§3.6）。发布前确认这是有意为之 |
| `data_dir` 不是字符串 | **硬失败**（`true` 若被宽容地转成字符串，会静默建出一个名叫 `True` 的目录） |
| `data_dir` 含 NUL | **硬失败** |
| `data_dir` 含 `<>"\|?*` | warning（Windows 上建目录会失败，届时自动回退系统默认目录） |
| `app_url` 数组但无 `public_key` | **硬失败** |
| `app_url` 空数组 / 空串 | **硬失败** |
| `app_url` 以 `[` 开头但不是合法数组 | **硬失败** |
| `app_url` 不以 `http(s)://` 开头 | warning（本地夹具可能用别的 scheme） |
| `app_url` 是「JSON 数组文本」写法 | warning（建议改真数组） |
| `icon_url` 非 http(s) | warning（打包时下载失败会回落默认图标） |
| `resolve_secret` 未配置 | ⚠️ **仅当 `app_url` 是域名数组**才 warning（共享默认密钥，安全门槛较低）；单 URL 字符串模式只打一行说明、**不告警**——该模式运行期不解密 TXT，`SECRET` 没有任何调用点 |
| `android_*` 出现在 Electron 侧 | 忽略并提示（同一份契约，本壳不实现） |
| 字段值为 `null` | 视为**用默认值** |

> **为什么未知字段要硬失败？** 手误与"字段不生效"在宽容模式下表现完全一样。
> 硬失败 + 近似名建议是唯一能让手误在打包时就现形的做法。

**自测与门禁**：解析器内置契约自测，CI 的 **`config-contract` 作业**（`ci.yml`）里会跑，防止契约悄悄漂移。
该作业同时做另外两件同样只能靠 CI 兜住的事 —— 静态检查所有 workflow 的 YAML 与内联 `python`/`bash` 语法
（这类错「只在 CI 上才炸」，本地不跑就等于没有护栏），以及**逐个校验仓库内仍纳管的配置样例**
（`webview-app/config.example.json` 与 `electron-app/config.example.json` 是否符合契约，
曾停留在字符串布尔，靠这一步在提交时就现形）。

```bash
python3 .github/scripts/parse-packaging-config.py --selftest
python3 .github/scripts/parse-packaging-config.py --print-contract-keys   # 打印字段名，供模板断言
python3 .github/scripts/check-workflows.py                                # workflow YAML + 内联脚本语法 + .github/scripts/*.sh
```

---

## 6. 配置存在哪里（为什么写不进 exe 同级）

| 壳 | 位置 | 可读性 |
|---|---|---|
| Tauri | 值已编进 `web2app.exe`（常量），交付物里**没有配置文件** | 需反编译才可见 |
| Electron | `app.asar` 内的 `out/config.json` | **不可直接读改**；解开 asar 才能看到 |

**Electron 侧为什么不再把 `config.json` 放在 exe 同级？**

早期实现用 electron-builder 的 `extraFiles` 把配置放在 exe 旁边，结果是**任何拿到包的人
都能直接改地址**（记事本打开改成自己的站，再发给别人）——配置就失去了意义。
现在配置随 asar 进包，改动必须重新打包。

> 实现要点：`src/main/index.ts` 里 `configRoot()` 指向 `app.getAppPath()/out`（打包态 = asar 内）；
> 开发态回落到项目根，方便本地调试。
>
> ⚠️ **护栏**：`electron-build-custom.yml` 出包后跑 `scripts/check-asar-config.cjs`，
> 断言 `out/config.json` **确实在 `app.asar` 内**。这条防的是**静默失败**——配置没进包时
> electron-builder 照样出包、双击照样打开，只是静默退回内置默认地址，构建日志全绿。

---

## 7. 打包入口怎么选

### 7.1 手动打包（推荐为每个应用出一份包）

| | Tauri 版 | Electron 绿色版 / 安装版 |
|---|---|---|
| workflow | `build-custom.yml` | `electron-build-custom.yml` |
| 勾选项 | `windows_installer` / `windows_portable` / `android` | `electron_zip` / `electron_portable` / `electron_nsis` |
| 配置来源 | `config_json` 或 `config_url`（二选一，前者优先） | 同左 |
| 产物 | `.msi` / `-portable.zip` / `.apk` | `.zip`（解压即用）/ `-portable.exe`（单文件）/ `-setup.exe`（NSIS 安装向导） |
| 产物体积 | 单文件绿色版 **< 20MB**（自有 exe 11.06MB） | zip ≈147.6MB / portable ≈96.5MB / setup ≈107.3MB（Chromium 载荷） |

**配置怎么给**：

- `config_json`：直接粘贴单行 JSON。JSON 标准不支持注释，**粘贴时请去掉注释**。
- `config_url`：填一个公开可访问、返回该 JSON 的 URL（可指向仓库内 `.json` 的 raw 链接）。
  「想一键生成某个固定应用」就把 `config_url` 固定指向仓库里那份 `.json`。

> `config_json` 与 `config_url` 都填时，**以 `config_json` 为准**。

### 7.2 本地打包（`scripts/pack-local.mjs`）

不经过 GitHub Actions，在本机按同一份 JSON 出包。

⚠️ 这里有一个**刻意的取舍**：本脚本是 `.github/scripts/parse-packaging-config.py` 的 **Node 复刻**，
而**不是复用**它。理由是 Python **不是**「拿着这个仓库就一定有」的依赖 —— 2026-09-24 实测：
Windows 上 PATH 里的 `python` 来自会话注入（`~/.workbuddy/binaries/python/…`），系统里既没有
`py.exe` launcher 也没有任何 Python 安装 ⇒ 出了那个会话就 `command not found`。
而打 Electron 包**必然已有 node**（`electron-builder` 本身就是 node 程序，
`pnpm run sidecar/build/stage` 也全是 node 脚本）。

⇒ 代价是解析逻辑**有两份实现**，因此配了**两条护栏**（都已接进 `.github/workflows/ci.yml`
的 `config-contract` 作业）：

| 命令 | 作用 | 需要 Python |
|---|---|---|
| `node scripts/pack-local.mjs --selftest` | 契约自测（字段/默认值/废弃字段报错/类型拒绝/近似名建议） | 否 |
| `node scripts/pack-local.mjs --parity` | 与 Python 解析器**逐项比对**：退出码 / `::warning::` 序列 / 错误文案 / `config.json` 字节 | **是**（缺失时明确报「跳过」，不假装通过） |

**⚠️ 改任何一侧的解析语义，都必须同步另一侧，并跑 `--parity`。** 它比对的是 39 个 payload
（含全部错误分支与边界），任一项不同即失败。

```bash
# 先看将执行什么命令（不动任何文件）
node scripts/pack-local.mjs --config tests/test1.json --target electron-all --dry-run

# Electron 绿色版（zip + single-file portable exe）
node scripts/pack-local.mjs --config tests/test1.json --target electron-all

# Electron Windows 安装版（NSIS setup exe；与绿色版分开跑，理由见下）
node scripts/pack-local.mjs --config tests/test1.json --target electron-nsis

# Tauri Windows 安装版 / 单文件版（需先 cargo install tauri-cli --version "^2"）
node scripts/pack-local.mjs --config tests/test1.json --target tauri-msi
node scripts/pack-local.mjs --config tests/test1.json --target tauri-portable

# 两条护栏
node scripts/pack-local.mjs --selftest
node scripts/pack-local.mjs --parity
```

要求 **Node ≥ 18**（用到内置 `fetch`），**零第三方依赖**。

`--config` / `--json` / `--url` 三选一；`--target` ∈
`electron-{zip,portable,nsis,all}` / `tauri-{msi,portable,all}`。

> ⚠️ **`electron-nsis` 刻意不并入 `electron-all`**：打安装版必须带
> `-c.extraMetadata.distribution=nsis` 标记（写进 app.asar 内 package.json，运行时据此
> 强制使用系统默认数据目录 `%APPDATA%`，忽略 `data_dir`）；若与 zip/portable 同次构建，
> 绿色产物会被一起误标成安装版。三种形态都要时，绿色版与安装版分别跑一次。

**⚠️⚠️ 本地行为一律由 JSON 决定 —— 不允许有「JSON 之外还得有人再做一步」的隐式前提。**
这条踩过两次，都已修（本地判据与 CI 逐字同源）：

| 事项 | 判据（**只看 JSON**） | CI 里的同一条 | 曾经的错法 |
|---|---|---|---|
| 自定义图标 | `icon_url` **非空** | `if [ -z "$ICON_URL" ] … exit 0`，否则下载 + 应用 | 本地额外要求 `--icon` 开关 ⇒ 同一份 JSON 在 CI 带图标、在本地不带 |
| 产物输出目录 | 不需要人参与 | CI 工作区是一次性的，`electron-app/release/` 由 electron-builder.yml 指定 | 让人**手工删掉**输出目录 |

- `icon_url` 非空时会下载 PNG 并应用：Electron 侧**同时**覆盖 `webview-app/icons/icon.png`
  （托盘图标走 `extraResources`，只设 `-c.win.icon` 对托盘无效），Tauri 侧跑 `cargo tauri icon`
  重新生成整套图标。要**刻意**跳过用 `--no-icon`（逃逸口，不是默认路径）。
- 输出目录：本地打包的**交付产物一律落到仓库根 `tests/` 下** —— 那是根 `AGENTS.md` §3.2 约定的本地测试
  产物目录，根 `.gitignore` 的 `/tests/` 整目录忽略，**不会进版本库**；CI 不受影响（工作区一次性，
  仍走各自默认路径）。两壳默认**分子目录**（绿色版都叫 `<app_name>.exe`，混放会互相覆盖）：
  - **Electron → `tests/pack/`**：win-unpacked/ 整树、zip/portable 与 NSIS setup exe（单次约 150MB）
    由 electron-builder 直接输出到这里；CI 里仍走 `electron-builder.yml` 指定的 `electron-app/release/`。
    也可在 `electron-app/` 下直接跑 `pnpm run dist:installer`（等价于
    `--target electron-nsis`，输出同样重定向到 `tests/pack/`）。
    脚本**不自己清空**输出目录，先让 electron-builder 按常规流程清；**只有它因「删不掉」而失败时**，
    脚本才自动换 `tests/pack-local-<时间戳>/` 重试一次（全新目录没东西可清 ⇒ 必然能出包）。
    ⚠️ 不要改成「脚本先自己清一遍」：本机的删除有配额，清一次上千个文件会把配额烧光，害得
    electron-builder 连自己的中间产物都删不掉 ⇒「产物已出、构建却报失败」（2026-09-24 实测）。
  - **Tauri → `tests/pack/tauri/`**：Tauri 无法配置 cargo 的输出目录，脚本在构建成功后**归集**：
    单文件绿色版由 `target/release/web2app.exe`（cargo 增量编译产物，**复制不搬走**）复制为
    `<app_name>.exe`；`bundle/msi/*.msi` 与 `bundle/nsis/*` 安装包（`tauri.conf.json` 的
    `bundle.targets` 同时含两种）**移动**到该目录。默认目录每次构建整目录重建（清掉升版前的旧安装包）；
    自定义 `--out` 目录则只覆盖同名文件、不清空。
  - 可用 `--out <DIR>` 覆盖；相对路径以**各自壳工程目录**为基准（Electron 锚定 `electron-app/`、
    Tauri 锚定 `webview-app/`），在根 tests/ 下换目录写 `--out ../tests/<名字>`，绝对路径原样使用。

**为什么两壳的本地步骤不一样**——配置在哪一刻被读，决定本地要做什么：

| 壳 | 配置被读的时刻 | 本地要做的 | 对应本地命令 |
|---|---|---|---|
| Tauri | **编译期**（`option_env!` → 二进制常量） | 在 `cargo tauri build` **之前**把环境变量设好 | `cargo tauri build` |
| Electron | **打包期**（读 `config.json` → 进 `app.asar`） | 先落一份**规范化**的 `config.json` | `pnpm run dist`（sidecar → tsc → stage → electron-builder） |

**⚠️ 「写 env 文件」那条路的三个坑**（Node 版完全不走这条路 ⇒ 天然不存在；手工用解析器的
`--env-out` 或读 CI 的 `$GITHUB_ENV` 时会踩）：

1. **不要 `source` 解析器写出的 env 文件。** Windows 上 `--env-out` 落盘是 **CRLF**
   （解析器里 `open(path, "a", encoding="utf-8")` 按 `os.linesep` 把 `\n` 翻成 `\r\n`）⇒
   `source` 后每个值尾部粘一个 `\r`，`APP_URL` 直接变成带 `\r` 的地址；
   `PROXY_BYPASS=localhost;127.0.0.1;<-loopback>` 含 `;`（bash 当命令分隔符拆断）；
   `public_key` 是多行 PEM 时写的是 **GitHub heredoc**（`KEY<<EOF…EOF`，bash 不认）。
   ⇒ Node 版把 `env` 对象**直接传给子进程**，既不落也不读 env 文件。
2. **`--env-out` 不传时解析器静默跳过**（`main()` 里是 `if args.env_out:`）——
   本地忘传就「解析成功但一个变量都没设」。
3. **`--env-out` 是 append 模式**（`open(path, "a")`）⇒ 反复跑会累积、新值盖不住旧值。

**⚠️ 本地打包会改写文件，脚本一律备份 + `finally` 还原**（`--keep-config` 可保留）：

| 文件 | 为什么被改写 | 是否纳管 |
|---|---|---|
| `webview-app/tauri.conf.json` | `app_name` → `productName`（安装包/快捷方式/安装目录名）+ 固定 `mainBinaryName` | **tracked** |
| `webview-app/icons/*` | `icon_url` 非空时 `cargo tauri icon` 重新生成整套图标（Electron 侧同步覆盖其中 `icon.png` 供托盘用） | **tracked** |
| `electron-app/config.json` | 落规范化配置 | 未纳管，但通常是开发者本地调试那一份 ⇒ **同样还原** |

> CI 不还原是因为工作区一次性；本地会脏工作区 / 覆盖本地调试配置。

**本地做不了 Android**：`ring` 的 build script 需要 `aarch64-linux-android-clang`（NDK），
脚本会明确拒绝而不是产半成品 ⇒ Android 只能走 CI。

### 7.3 `release.yml`（Publish Release 触发，打 tag 不触发）

> 触发条件（2026-09-27 订正）：workflow 监听 `release.types:[published]` —— 必须在
> GitHub 的 Release 页**发布一个 Release**；只 `git push tag` 不会触发。

配置来自**仓库变量 / Secrets**，字段名与 JSON 不同：

| JSON 字段 | 仓库变量 / Secret |
|---|---|
| `app_url` | `APP_URL`（Secret 优先，回落 Variable） |
| `app_name` | `APP_NAME` |
| `title` | **`TITLE`**（旧名 `APP_TITLE` / `WINDOW_TITLE` 仅作回落） |
| `extra` | `EXTRA` |
| `icon_url` | `ICON_URL` |
| `public_key` | `PUBLIC_KEY`（Secret 优先） |
| `resolve_secret` | `RESOLVE_SECRET`（Secret 优先） |
| `proxy_enabled` / `proxy_http` / `proxy_https` / `proxy_bypass` | 同名大写 |
| `data_dir` / `tabs_enabled` / `debug` | 同名大写（`DEBUG` 未配置 = 关闭） |
| `android_*` | `ANDROID_KEYSTORE_BASE64` / `ANDROID_KEY_ALIAS` / `ANDROID_KEYSTORE_PASSWORD` / `ANDROID_KEY_PASSWORD` |

> ⚠️ `EXTRA` 与 `ICON_URL` 历史上在 `release.yml` 里**缺失**，等于 Release 路径下
> `window.web2appExtra` 恒为 `{}`、且不能用自定义图标。现已补齐（两条 job 都有）。
>
> ⚠️ **`release.yml` 的 `DATA_DIR` 默认是「空串」（= 系统默认目录），与 JSON 侧的默认 `"data"` 不同。**
> 这是有意的：`release.yml` 产出的是 MSI 安装版，装进 `Program Files` 后 exe 同级不可写，
> 系统默认目录对它才是最稳的默认值。要让它也「跟着程序走」，设仓库变量 `DATA_DIR=data`。
> ⚠️ 该行**不能**写成 `vars.DATA_DIR || 'data'`：`||` 会把「已配置但为空」也替换成 `data`，
> 于是仓库变量里再也表达不出「用系统默认目录」这个取值。

---

## 8. 迁移指引

**从旧配置升级到本契约，只需四步：**

1. **删掉 `window_title`** —— 若与 `title` 并存，确认保留哪个；只留 `title`。
2. **`data_dir_default` 改成 `data_dir`（字符串）** —— `false` → `"data"`、`true` → `""`。
   旧字段出现会**直接报错**并给出这个对应关系，照报错改即可。
3. **布尔去掉引号** —— `"false"` → `false`、`"true"` → `true`
   （现在有三个布尔字段：`proxy_enabled` / `tabs_enabled` / `debug`）。
4. **`app_url` 数组改成真数组** —— `"[\"a.com\",\"b.com\"]"` → `["a.com","b.com"]`。

**模板**：

- [`webview-app/config.example.json`](../webview-app/config.example.json) —— Tauri / Android 侧（含全部字段）
- [`electron-app/config.example.json`](../electron-app/config.example.json) —— Electron 绿色版侧（省略仅 Android 的 `android_*`）

---

## 9. 已知缺口（待办，不在本次改造范围）

| 编号 | 字段 | 现状 | 影响 |
|---|---|---|---|
| P0 | `resolve_secret` | Electron 侧须在编译 sidecar 前注入 | 顺序错则**静默失效**；workflow 已按正确顺序编排 |
| P1 | `app_name` | 语义过载（安装名 + 数据目录名 + Android 包名前缀来源） | 发布后改名 = 丢登录态；同时换掉 Android 包名（在手机上等于另一个应用） |
| P1 | `android_*` 4 键 | 与「产物参数」混在同一个 JSON 里 | 一个 base64 keystore + 明文密码塞进配置 JSON |
| P2 | Android 包名前缀 | 固定 `com.web2app.`，只能用改 workflow 源码的方式更换 | 上架应用市场若要换成自己公司域名，需改代码而非配置 |

> ✅ **本次已解决**：`data_dir_default` 两壳同名反义（改为 `data_dir`，两壳同一套规则）；
> Android 包名固定为 `com.example.web2app`（改为由 `app_name` 派生）。

---

## 附：契约的取证方式（可复核）

| 内容 | 位置 |
|---|---|
| 字段 / 类型 / 默认值 / 校验 | `.github/scripts/parse-packaging-config.py`（`DEFAULTS` / `BOOL_KEYS` / `REMOVED_KEYS` / `selftest`） |
| Electron 运行期对等实现 | `electron-app/src/main/config.ts`（`DEFAULTS` / `mustBool` / `toAppUrl` / `rejectUnknownKeys`） |
| 编译期常量 | `webview-app/src/config.rs` |
| 落盘日志（Tauri 侧） | `webview-app/src/debug_log.rs`；`dlog!` 宏定义在 `src/lib.rs`，sidecar 侧的同名宏在 `crates/proxy-server/src/main.rs` |
| 落盘日志（Electron 侧） | `electron-app/src/main/startup-log.ts`；装配（`DEBUG` 判定 / `silenceConsole()`）在 `src/main/index.ts` |
| 打包入口（Tauri / Android） | `.github/workflows/build-custom.yml` |
| 打包入口（Electron 绿色版 / 安装版） | `.github/workflows/electron-build-custom.yml` |
| 打 tag 自动发布 | `.github/workflows/release.yml` |
| asar 配置护栏 | `electron-app/scripts/check-asar-config.cjs` |
| 静态检查所有 workflow（YAML + 内联脚本 + `.github/scripts/*.sh`） | `.github/scripts/check-workflows.py` |
