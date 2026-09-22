# 打包配置 JSON 评审（2026-09-22）

> ⚠️ **历史快照（重构前）**：本文是**契约统一之前**的评审记录，请**不要**当现行规范阅读。
> 文中的 P0/P1 问题（含 `data_dir_default` 同名反义、字符串布尔、模板缺键、Android 包名冲突）
> **均已解决**，现行唯一真源是 [`packaging-config.md`](./packaging-config.md)。
> 保留本文仅为「问题从哪来」的溯源。

**对象**：手动打包工作流消费的那份配置 JSON —— 即 `build-custom.yml` 的 `config_json` / `config_url`
输入、`webview-app/config.example.json` / `electron-app/config.example.json` 模板、仓库根 `test1.json` /
`test2.json`、以及 `signing/packaging-config.json`（和 CI 自动生成的同名文件）。

**结论一句话**：字段本身设计合理（`app_name` / `title` 分离、`app_url` 双形态 + 公钥强校验、`extra`
注入都做得干净），**问题全在"契约有多份实现、且没有一致性门禁"**——同一份 JSON 交给两个壳会得到
不同行为，其中 3 处会造成静默失效或直接打不开。

---

## 1. 配置入口盘点：同一契约，7 份独立实现

| # | 位置 | 键数 | 谁读 | 角色 |
|---|---|---|---|---|
| 1 | `build-custom.yml:128-154`（Windows job `DEFAULTS`） | 18 | 解析步骤 | 打包期默认值/白名单 |
| 2 | `build-custom.yml:418-443`（Android job `DEFAULTS`） | 18 | 解析步骤 | 与 #1 **逐字节重复** |
| 3 | `webview-app/src/config.rs`（`env_str!` / `env_bool!`） | 13 常量 | `web2app` 编译期 | 运行期默认值 |
| 4 | `electron-app/src/main/config.ts:58-72` | 13 | Electron 启动期 | 运行期默认值 |
| 5 | `electron-app/scripts/stage-assets.cjs:55-69` | 13 | 打包前置 | 无 config.json 时的随包模板 |
| 6 | `build-custom.yml:1031-1046` + `signing/packaging-config.json` | **14** | 用户复用 | ⚠️ 缺 3 键 |
| 7 | `webview-app/config.example.json` / `electron-app/config.example.json` | 17 | 用户复制 | 两份内容互相冲突 |

- `ci.yml` 只有 `fmt / clippy / test / check`，**没有任何配置契约校验**，`electron-app` 的
  `typecheck / probe` 也不在 CI 里 ⇒ 漂移没有任何拦截点。
- 契约键数本身就有两个版本：CI 认 18 键（含 `window_title`），`electron-app/README.md:251` 与
  `config.ts:15` 宣称 17 键。

---

## 2. P0：会让产物行为错误/静默失效的 3 条

### P0-1 `resolve_secret` 在 Electron 绿色版**完全不生效**

| 侧 | 事实 | 证据 |
|---|---|---|
| 解析 | `resolve-core` 的 Secret 是**编译期常量** `option_env!("RESOLVE_SECRET")`，默认 `web2app-fixed-secret-2026` | `crates/resolve-core/src/lib.rs:58-71` |
| Tauri | CI 把 `RESOLVE_SECRET` 写进 `GITHUB_ENV` 后再 `cargo tauri build` ⇒ 主程序内置正确 Secret | `build-custom.yml:262-271`、`src/resolve.rs:110` |
| Electron | `w2a-proxy.exe` 由 **`pnpm run sidecar` 在本机单独编译**（`package.json:12`），编译时环境里没有 `RESOLVE_SECRET` ⇒ 内置的是**公开默认 Secret** | — |
| 传参 | `buildSidecarArgs()` 只发 `--domains` / `--public-key` / `--url-stdin` / `--no-proxy`；`w2a-proxy` 的 `--help` 里**根本没有 `--secret`** | `sidecar.ts:100-114`、`crates/proxy-server/src/main.rs:74-82` |
| 字段 | `config.ts:37/70/184` 读入 `resolve_secret` 后**再无任何使用点** | grep 全 `electron-app/src` 仅命中 config.ts |

⇒ 用户按文档"为本次产物配独立 Secret 并同步服务端"后：Tauri 版正常，**Electron 绿色版必然报
"服务器连接失败"**，且失败文案笼统、无法归因。`config.ts:36` 那句"⚠️ 改这里两端必须同步"承认了
这个字段却没有落地动作。

**建议**（保留"提高反编译门槛"的初衷）：
1. `w2a-proxy` 增加 `--secret-stdin`（**不要**用 `--secret`：Windows 上命令行对同用户进程可见，
   项目已经为 URL 用了 stdin 规避，Secret 同理）；
2. `resolve-core` 增 `resolve_url_from_domains_with_secret(domains, pk, secret)`，编译期常量降级为默认值；
3. `buildSidecarArgs` 在 `dns` 且 `cfg.resolve_secret` 非空时把 Secret 经 stdin 传入；
4. 过渡期：CI 在「`app_url` 为数组 + `resolve_secret` 非空」时打 `::warning::`，明示该组合当前
   **仅 Tauri 生效**，并同步修 README 字段表。

### P0-2 `data_dir_default` 同名反义，且 `true` 在安装版可能直接开不了窗

| 取值 | Tauri（`src/config.rs:202-208`） | Electron（`portable.ts:80-129`） |
|---|---|---|
| `false` | `%LOCALAPPDATA%\<app_name>` | **exe 同级 `data\`** |
| `true` | WebView2 系统默认（**紧挨 exe** = 安装后在 Program Files） | `%APPDATA%\<app_name>` |

Tauri 侧代码注释自己写着：安装版下 exe 在 Program Files、标准用户无写权限 ⇒ **WebView2 初始化失败、
窗口直接打不开**，所以"安装版强烈建议 false"。而 Electron 侧对不可写目录会**自动回退**（`portable.ts:111-119`），
所以同一个 `true` 在一边是"换个目录"、在另一边是"打不开"。JSON 里完全看不出来。

**建议**：JSON 层引入显式枚举，`data_dir_default` 降级为兼容别名：

```json
"data_dir": "local_appdata" | "system" | "portable_sibling"   // 默认 local_appdata
```

映射：`false → local_appdata`（Tauri 行为不变）；`true → system`（Tauri 行为不变，但 CI 对
`windows_installer=true` + `system` 组合直接 `::error::`）；Electron 绿色版把 `portable_sibling`
作为默认值并在文档里显式说明"绿色版默认同级 `data\`"。

### P0-3 复用 CI 生成的 `packaging-config.json` 会**静默丢 3 个字段**

`build-custom.yml:1031-1046` 生成的 cfg 只有 14 个键，**缺 `extra`、`resolve_secret`、`window_title`**。
用户拿它当下一次打包的输入（这是模板的用途）⇒

- `extra` 丢失 → 前端 `window.web2appExtra` 变 `{}`，按渠道/环境区分的逻辑全部失效；
- `resolve_secret` 丢失 → 回落到公开默认 Secret → 数组模式解不出 URL；
- 且解析器对未知字段只 warning、对缺失字段**一句话都不说**（见 P1-1）。

**建议**：① 生成模板时从一份共享契约（见 §4）派生，**列全契约键**，缺的留 `""` / `{}`；
② 签名四项拆到独立文件（`signing/keystore-*.json`），打包 JSON 里只留指针；
③ `signing/packaging-config.json` 现在把 keystore(base64) 与 `android_keystore_password` **明文**
写在一个会被复制转发的文件里，务必在生成时打印"该文件含密钥，勿随包分发"。

---

## 3. P1：契约漂移与静默陷阱

| # | 问题 | 证据 | 影响 | 建议 |
|---|---|---|---|---|
| P1-1 | 未知字段只 `::warning::` 后 `continue` | `build-custom.yml:183-185`、`:473` | `proxy_bypas` / `tab_enabled` / `app_ur` 拼错 ⇒ 产物静默用默认值 | 用 `difflib.get_close_matches` 给"你是不是想写 X？"并 `sys.exit`；**未知字段=打包失败** |
| P1-2 | 布尔值不校验取值，两壳判定不同 | `build-custom.yml:229-230` 只 `strip().lower()`；`config.rs:60-83` 认 `true/1/yes/false/0/no`；`config.ts:80` **多认 `on`** | 写 `"on"` ⇒ Tauri=false、Electron=true；写 `"ture"` ⇒ 两边静默回落 | 解析期白名单校验并归一（非法即失败）；删掉 Electron 的 `'on'` 分支 |
| P1-3 | `window_title` 只在 CI/Tauri 生效 | `build-custom.yml:242-247` 有回落链；`config.ts:167-186` 完全忽略（连 `MOBILE_ONLY_KEYS` 那类提示都没有） | 老配置：Tauri 窗口标题=X，Electron 窗口标题=app_name | Electron 补兼容，或解析期为它打 deprecation warning 并强制归一 |
| P1-4 | `proxy_bypass` 默认值漂移 | `config.rs:220` 默认 `localhost;127.0.0.1;<-loopback>`；`config.ts:67/230-231` 默认空且**空则不下发 bypass** | 同一份 `proxy_enabled:true`+外部代理配置，回环/内网直连行为两壳不同 | 把默认串落到两壳 `DEFAULTS` 与模板，或统一改空；补一条契约测试 |
| P1-5 | 两份 `config.example.json` 语义冲突 | `electron-app/` 那份 `proxy_enabled:"true"` + `http://10.0.0.1:8080` + `public_key:"AAAA"` + keystore `"xx"/"yy"/"zz"` | 用户"复制改好再粘贴"时拿错文件 ⇒ 产物**开着代理指向 10.0.0.1，直接断网** | 收成**唯一真源模板**（其余由它派生/校验）；占位值一律改成明显无效的空值 |
| P1-6 | 文档路径漂移 | `build-custom.yml:58` 写"仓库根目录的 config.example.json"，单仓库化后实际在 `webview-app/`；README 只写文件名不写目录 | 用户找不到模板 | 注释与 README 用带目录的相对路径 |
| P1-7 | `app_url` 无 scheme 校验 | `build-custom.yml:208-211` 只判非空 | `www.example.com` 能过 ⇒ 运行期白屏 | 字符串形态必须 `^https?://`；数组形态必须纯域名 |
| P1-8 | `resolve_secret` 未配置仅 warning | `build-custom.yml:217-219` | 静默共享源码里明文的 `web2app-fixed-secret-2026` | 数组模式下**必填**（单 URL 模式仍可空）；或生成模板时自动写入随机 Secret |
| P1-9 | `app_name` 承担"数据目录名" | `config.rs:120-143`、`config.ts:90-93` | **发布后改 `app_name` = 数据目录迁移 = 用户全部登录态丢失** | 增 `data_dir_name`（默认=`app_name`）解耦；在 README 标为"发布后不可变更字段" |
| P1-10 | Android 包名无法按配置区分 | `tauri.conf.json:5` 固定 `identifier: com.example.web2app`，job 只改 `productName`；`webview-app/README.md:496` 已记为上游限制 | 不同 `config.json` 产出的 APK **互相覆盖安装** | 增 `android_package` 字段并在 `android init` 后改写 `build.gradle.kts` 的 `applicationId/namespace`；至少先给 warning |
| P1-11 | 一次性 keystore 的 warning 太轻 | `build-custom.yml:998` 的 `::warning::`，而 android job 是 `continue-on-error: true`；`config.ts` 侧无感 | 用户以为签名成功，之后**无法覆盖升级** | 升级为 `::error::`，除非显式给 `allow_ephemeral_keystore: "true"` |
| P1-12 | `icon_url` 只校验 PNG magic | `build-custom.yml:328-331`、`:676-679` | 非正方形/过小图片会变形或报错 | 解析 PNG IHDR 校验正方形且 ≥256×256 |
| P1-13 | `test1.json` / `test2.json` 近乎重复 | 唯一差异 `tabs_enabled`（README:70 亦自述） | 改 `app_url` 只改一个 ⇒ 版本漂移 | 合成一份模板 + 开关生成变体；或加 CI 校验"除 tabs_enabled 外逐字节一致" |

---

## 4. 建议的整改路径（按风险递增，可分三步落地）

**第 1 步（低风险，纯加固，不动现有 JSON 语义）**
1. 未知字段 → 硬失败 + 拼写建议；布尔值白名单校验；
2. `app_url` scheme 校验、`icon_url` 尺寸校验、数组模式 `resolve_secret` 必填；
3. 生成的 `packaging-config.json` 列全契约键；一次性 keystore 的 warning 升级；
4. 修掉 `build-custom.yml:58` 的路径漂移与两份 example 的冲突占位值。

**第 2 步（建立门禁：把"契约"变成一个可执行的断言）**
5. 抽一份**唯一真源** `config.schema.json`（键名 + 类型 + 默认值 + 平台作用域 + 是否 deprecated），
   由它生成/校验：CI 两处 `DEFAULTS`、`config.rs` 的常量表、`config.ts` 的 `DEFAULTS`、
   `stage-assets.cjs` 的 `tpl`、两份 `config.example.json`、`packaging-config*`；
6. CI 增一个 `config-contract` job：断言上述各处键集合与默认值**完全一致**（这是唯一能永久
   止住本次评审这 13 条漂移的办法）；
7. 顺手把 `electron-app` 的 `typecheck`（`probe` 可选）纳入 CI —— 现在 Electron 侧改配置解析
   没有任何 PR 阶段拦截。

**第 3 步（语义重构，需要一次兼容窗口）**
8. `data_dir` 枚举替代 `data_dir_default`（旧名保留为别名 + deprecation warning）；
9. 新增 `data_dir_name` / `android_package` / `allow_ephemeral_keystore`；
10. Secret 走 stdin 贯通两壳（P0-1）。

---

## 5. 验收判据（每条都可观测，建议进测试或 CI）

| 判据 | 期望 |
|---|---|
| 故意把 JSON 写成 `proxy_bypas` | 工作流**失败**并提示"是不是想写 proxy_bypass" |
| `{"app_url":"www.example.com"}` | 工作流**失败** |
| `{"window_title":"X","app_name":"Y"}` | Tauri 与 Electron 窗口标题**都是 X** |
| `{"app_url":["a.com","b.com"],"public_key":"…","resolve_secret":"s"}` | Electron 绿色版**能解出 URL**（当前必失败） |
| `{"proxy_enabled":"true","proxy_http":"http://p:8080"}` | 两壳浏览器参数里的 `proxy-bypass-list` **一致** |
| `{"tabs_enabled":"on"}` | 两壳判定**一致**（当前相反） |
| 复用 CI 生成的 `packaging-config.json` 打包 | `extra` 与 `resolve_secret` **不丢** |
| 7 份实现两两比对 | 键集合与默认值**完全相同** |

---

## 6. 已排查、确认**不是**问题的疑点（避免以后重复怀疑）

- **编译期常量 + 缓存的陈旧配置风险**：`config.rs` 用 `option_env!` 且全仓库没有
  `cargo:rerun-if-env-changed`，看起来"改了 env 不重编译"。实测 rustc 会把 env 依赖写进 dep-info：
  `target/release/deps/web2app_lib.d` 里有 `# env-dep:APP_URL`、`# env-dep:TABS_ENABLED=true`、
  `# env-dep:WINDOW_TITLE` 等，`resolve_core-*.d` 里有 `# env-dep:RESOLVE_SECRET` ⇒ **cargo 会据此
  判定指纹失效并重编译**，`Swatinem/rust-cache` 恢复 `target/` 也不会复用旧配置产物。**无需处理。**
- **`extra` 的注入逃逸面**：走 `initialization_script(format!("window.web2appExtra = {};", extra_json()))`
  （`src/lib.rs:218`），不拼 HTML ⇒ 不存在 `</script>` 逃逸；`extra_json()` 已有合法性兜底，
  且 CI 侧 `json.dumps(separators=…)` 保证单行写入 `GITHUB_ENV`。**设计良好。**
- **`extra` / `public_key` 写 `GITHUB_ENV` 的编码**：`PYTHONUTF8` + 多行 PEM 用 heredoc 语法写入，
  处理得当。**无需处理。**

## 7. 做得好的部分（保持）

`app_name` / `title` 语义分离清楚；`app_url` 数组 + `public_key` 的强校验与失败回落链；
`resolve_secret` 未配置时的告警；日志里对 `app_url` / 密码字段的 `mask()`；`icon_url` 下载做
magic 校验且失败即退出；`packaging-config.single.json` 的单行形态（专为粘贴输入框设计，
README:318 有说明）；`stage-assets.cjs` 在构建期就校验 JSON 合法性并硬失败。
