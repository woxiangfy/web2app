# 数据目录、日志与配置（electron-app）

> 自 `AGENTS.md` 拆出：`data_dir` 语义、日志开关与落点、`config.json` 随包进 asar 与严格校验、
> `app_name` / `title` 取舍。配置字段契约见 `../docs/packaging-config.md`。

## 数据目录

`config.json` 的 `data_dir`（字符串，与 Tauri 版**同名同语义**）：

| `data_dir` 取值 | 数据位置 |
|---|---|
| `"data"`（默认） | **exe 同级 `data\`**（相对路径基准 = exe 所在目录，非 cwd）— 整个目录可搬走，不写系统盘、不写注册表 |
| `""`（空字符串） | `%APPDATA%\<app_name>`（如 `app_name=Baidu` 即 `%APPDATA%\Baidu`）— 跟随系统，不受程序位置影响 |
| 其它相对路径（如 `"profile"`） | exe 同级 `\profile\` |
| 绝对路径（如 `D:\w2a`） | 原样使用 |

⚠️ **空字符串是有意义的值**（= 系统默认目录），不是「未设置」：读取时不能写成
`data_dir || 'data'`，否则 `""` 会被 `'data'` 吃掉、静默退回 exe 同级。

目录不可写时（写保护的 U 盘、`Program Files`、只读网络盘）会**自动回退**到系统默认目录并打日志，
保证「能启动」优先于「够绿色」。四个路径一起重定向：`userData` / `sessionData` / `logs` / `crashDumps`
（只改 `userData` 会留下 Cookies 和日志在系统盘）。

> 与 Tauri 版的差异：`data_dir=""` 时 Tauri 写 `%LOCALAPPDATA%\<app_name>`，Electron 写 `%APPDATA%\<app_name>`。
> 这是有意的——Electron 版定位为真绿色版（默认 `"data"` 根本不落系统盘）。部署文档需明示这一点。

## 日志目录（`data\logs\startup.log`，**`debug` 打开时才有**）

⚠️ **Windows 上 `electron.exe` 是 GUI 子系统程序**：用户双击运行时 `console.log` 没有任何去处
（没有控制台可挂）；portable 单文件形态更彻底——自解压包装器会吞掉内层进程的 stdout。
于是「双击没反应 / 打开的是 example.com / 标题不对」这类反馈在用户侧**零证据**，只能远程猜。

**但「默认就落盘」同样不是期望行为**：它会在用户的交付目录里凭空多出 `data\logs\`，
而绝大多数使用者根本不需要它。所以 2026-09-22 起日志改成**配置开关**：

| 开关 | 效果 |
|---|---|
| `"debug": false`（**默认**） | 不建 `logs\`、不写任何文件，**控制台三路输出也一并静默** |
| `"debug": true` | 写 `<数据目录>\logs\startup.log`，同时照常打印到 stdout / stderr |

**运行时逃生口**（不改配置、不重新打包，用于现场排障）：命令行加 `--debug`，或设环境变量
`WEB2APP_DEBUG=1`。两者任一生效即等同 `debug=true`。

> 「完全静默」是有意选的：GUI 程序本来就没有控制台，`debug=false` 时把 `console.log` / `warn` /
> `error` 换成空函数，避免这些输出被 portable 包装器或父进程捕获后又落到**别处**（CI 日志、
> 用户的重定向文件……）。**关掉就要关干净。**
> **唯一例外**：探针（`--probe`）与冒烟（`--smoke`）**内部强制打开** debug —— 它们的结论全靠
> stdout，绝不能因为「默认关日志」而自检失声。

开日志时，启动后立刻把 console 的三路输出同时落一份盘（`src/main/startup-log.ts`）：

| 形态 | 日志位置 |
|---|---|
| 解压目录版（`data_dir="data"`，默认） | 「用户放置 exe 的目录」`\data\logs\startup.log` |
| `data_dir=""`（系统默认目录） | `%APPDATA%\<app_name>\logs\startup.log` |
| 探针（`--probe`） | `%TEMP%\w2a-probe-store\startup.log` ← **不写真实目录**（与 `settings.json` 同一套隔离） |

⚠️ 探针那条不是洁癖：在打包产物上跑一次 `--probe` 就会在用户 exe 旁边留下 `data\logs\`，
污染交付物（`data\session` 已经踩过一次，见 [build-package.md](build-package.md)「打包」一节）。

契约（改这个模块时别破坏）：

* **fail-open**：目录建不出来 / 文件不可写 ⇒ 只在控制台告警**一次**并停止后续写入尝试，
  应用照常启动。为了记日志让应用起不来是本末倒置；但**必须说清楚**，不许静默。
* **不碰 stdout**：包装 `console` 时先调用原始实现，宿主捕获到的输出一字不变
  —— 探针与冒烟的结论全靠 stdout，绝不能因为加日志而变哑。
* **同步追加**（`appendFileSync`）：崩溃或被强杀时，最后几行已经落盘。
* **1 MiB 轮转**成 `startup.prev.log`（只留一代），且**轮转失败不影响继续写**——
  轮转与写入是两个独立失败域，不能互相拖垮。
* 行格式 `<ISO 时间戳> <原文>`；启动块以 `===== web2app 启动 =====` 分隔，便于从长日志里切段。

取证（拿到用户的一段日志就能定位，不必再问「你机器上是什么情况」）：

```powershell
Get-Content .\data\logs\startup.log -Tail 40
```

`pnpm run smoke` 有一条硬断言：日志文件必须**已经包含 `[web2app] 启动信息`**，不满足即退出码非 0。
只判「文件存在」是不够的——一个只写了表头的空文件同样会骗过断言。
（冒烟内部强制 `debug=true`，所以这条断言与「默认关闭」不冲突；若 `fileLog` 拿到 `null`，
断言会直接报「未启用（冒烟模式本应强制打开 debug，出现即 bug）」——把「开关接错」与
「写盘失败」两类原因分开。）

> 负向对照（2026-09-18 实测）：把日志目录的父级设成文件 ⇒ `ok=false / ENOTDIR`；
> 把 `startup.log` 建成目录 ⇒ `ok=false / EISDIR`。两次调用**都不抛异常**，
> 只打一条 `已跳过文件日志（不影响使用）` 的告警。即「日志坏了 ≠ 应用坏了」。

同处取版本信息（M6 的另一半，实测已由 electron-builder 自动写入）：

```powershell
(Get-Item .\release\win-unpacked\Web2App.exe).VersionInfo |
  Format-List FileVersion,ProductVersion,ProductName,CompanyName,FileDescription,LegalCopyright
```

> ⚠️ `CompanyName` 来自 `package.json` 的 `author`。写成 `"author": "Web2App"`（字符串）也**能**生效
> —— electron-builder 内部会先经 `normalizePackageData` 把字符串归一化成 `{name, …}`；
> 但别把它误解成「随便写都行」：`appInfo.companyName` 取的是 `author.name`，
> 一旦换成 `{ email: … }` 这种没有 `name` 的对象，公司名就会**静默留空**。改动后请跑一次上面的命令。

## 配置文件

**配置随包进 `app.asar`，不放 exe 同级。** 启动时读 `app.getAppPath()/out/config.json`：

| 形态 | 读取位置 |
|---|---|
| 打包后（绿色版） | `resources\app.asar` 内的 `out/config.json` |
| 开发态 | 项目根 `electron-app/config.json`（`configRoot()` 的回退分支） |

> ⚠️ **为什么不再放 exe 同级**：早期用 `extraFiles` 把配置放在 exe 旁边，结果**任何拿到包的人
> 都能用记事本改掉地址**（改成自己的站再转发）—— 配置就失去了意义。现在改动必须重新打包。
> 代价是**不能**再用「放一份 config.json 到 exe 旁边」来覆盖，改配置 = 重跑 `build` + `stage` + 打包。

> ⚠️ **护栏**：`scripts/check-asar-config.cjs` 断言 `out/config.json` 确实在 `app.asar` 内。
> 这条防的是**静默失败** —— 配置没进包时 electron-builder 照样出包、双击照样打开，
> 只是静默退回内置默认地址（`example.com`），构建日志全绿。

字段与 `webview-app` 的配置契约**同名同义**，唯一实现见
[`.github/scripts/parse-packaging-config.py`](../../.github/scripts/parse-packaging-config.py)，
完整字段表与多端行为对照见 [`docs/packaging-config.md`](../../docs/packaging-config.md)。
`config.example.json` 按契约列出本壳的 **17 个字段**（省略仅 Android 使用的签名 `android_*`；
其中 `mobile_bar`、`android_permissions` 是移动端 / 仅 Android 专用字段，本壳按契约接收但**不消费**），
复制成 `config.json` 再改即可用。

**校验是严格的**（与解析器逐条对齐）：

- **布尔只接受真 `bool`**（`true` / `false`，不带引号）。字符串形式一律报错并给出改法 ——
  曾经这里宽容地接受 `"1"` / `"yes"` / `"on"`，而 Tauri 侧规则不同，于是同一份配置里写 `"on"`
  两个壳判定**相反**。
- **未知字段直接失败**（`android_*` 除外，它们属同一份契约、只是本壳不实现，会提示后忽略）。
- **`app_url` 接受 `string` 或 `string[]`**（真 JSON 类型），数组模式必须带 `public_key`。
- **`version` 只接受三段数字字符串**（`"1.2.3"`，默认 `"0.0.1"`；主次 ≤ 255、补丁 ≤ 65535）。
  打包期经 `electron-builder -c.extraMetadata.version=` 覆盖进打包用 package.json，决定
  zip/portable/setup 文件名与 exe 版本资源；运行期 `toVersion()` 再校验一次。
- **已移除的字段出现即报错**：`window_title` → 改用 `title`；`data_dir_default`（布尔）→ 改用 `data_dir`（字符串，`false` ⇒ `"data"` / `true` ⇒ `""`）。
- 文件存在但 JSON 非法时**直接抛错**（不静默回落）—— 否则「改坏一个字符」的表现是「配置像没生效」，最难查。

`config.json` 与 `data/` 都在 `.gitignore` 里 —— 它们是运行期/本地产物，不是源码。
打包脚本 `scripts/stage-assets.cjs` 把项目根的 `config.json`（缺失则生成一份全字段默认模板）
复制到 **`out/config.json`**（随 `files: out/**/*` 进 asar），并在复制前做一次浅层契约校验
（`window_title`、字符串布尔会在这里就失败，而不是等用户双击）。

### `app_name` 与 `title`

| 字段 | 作用 | 本壳取值 |
|---|---|---|
| `app_name` | 安装名 / 数据目录名（Android 侧为应用名） | `data_dir=""` 时系统默认目录的末段（`%APPDATA%\<app_name>\`）；`settings.json` 随数据目录走，不再固定在这里 |
| `title` | **显示名**：窗口标题、托盘提示 | 缺省回落 `app_name` |

⚠️ 窗口标题在创建 `BrowserWindow` 时就固定为 `cfg.title`，并拦截 `page-title-updated`
（对齐 Tauri 版：不跟随 `document.title`）。**漏传 `title` 不会退回成「显示页面标题」，
而是永远停在 Electron 默认值**（`package.json` 的 `name`）—— 见 `smoke` 的标题断言。

> 与 Tauri 版的差异：Tauri 版把配置固化成**编译期常量**（改配置必须重新编译）；
> Electron 版把配置**打进 `app.asar`**（打包期读取，改配置必须重新 `build` + `stage` + 打包）。
> 两者都不是「交付后运行期改文件」—— Electron 侧改完**源码工程里的** `config.json`
> 后要重新打包并替换 `app.asar`，仅重启应用不会生效。
