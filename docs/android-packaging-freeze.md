# Android 打包「冻结契约」

> **为什么有这份文档**：本仓库的 Android 打包链是**下游项目的参照实现**。下游会照抄
> `.github/scripts/patch-android-*.py` 与两份 workflow 的 android job。打包层每多一次"为了某个
> 功能而改"，下游就多一次不得不跟的改动 —— 那等于把"参考实现"变成"漂移源"。
> 所以这里把打包层切成三层，并用契约测试钉住**唯一允许变化的那一面**。
>
> 相关：`android-improvements.md`（能力清单）、`webview-app/docs/android.md`（补丁链与排障）、
> `packaging-config.md`（打包配置字段）。

## 1. 结论先行

1. **打包层只允许三种变化**：① L0 恒需常量的一次性补全；② L1 可选能力（仅 `android_permissions`
   一个字段，且只在产品真的需要时才动）；③ L2 版本适配（只随 wry / tauri 升级而动）。
2. **本次（2026-11-19）缺 `ACCESS_NETWORK_STATE` 暴露的不是"配置面太宽"，而是"恒需常量被写成了
   随配置消失的分支"**：原实现 `if not permissions: return` 让「默认配置」等于「不注入任何权限」。
   修法不是"再加一个开关"，而是把恒需常量改成**无条件**注入（`REQUIRED_PERMISSIONS`，缺失才补）。
3. **产品输入面只有 4 个字段**（`app_name` / `title` / `version` / `android_permissions`），
   已由契约测试 ⑨ `android_packaging_product_inputs_are_frozen` 钉住：任何补丁脚本新增
   `os.environ.get("XXX")` 都会红灯，必须显式决定"它是不是产品输入"。
4. **升级不该产生改动**（见 §6）：① 补丁锚点只认**平台合约**（`WebViewClient` /
   `WebChromeClient` 的 override 签名），上游重排/改缩进/改参数名/重写方法体都不影响；
   ② 锚点真的消失时**降级跳过**（`::warning::` + exit 0），不再让构建红；
   ③ 上游自己实现了就直接**退场**（如 wry 修好全屏时 `fullscreen` 补丁自动变 no-op）。
   这三条已由 `rehearse.py --drill` 的 5 类上游变化演练钉住（含 wry 改文件名）。
5. **L2 仍然可以进一步被吃掉**（见 §7，可选而非必须）：把 3 处 wry Kotlin 胶水改由自身代码
   实现后 `prepare-wry-kotlin.py` 与 `inject-gradle-patch-task.py` 可整体退役（15 → 11 个脚本）。
   §6 落地后它已不再是"升级就疼"的地方，所以只在顺手时做、不夹带在功能改动里。

## 2. 三层模型

| 层 | 内容 | 变化频率 | 谁可以触发变化 | 判据 |
| --- | --- | --- | --- | --- |
| **L0 恒需常量** | Manifest 恒需权限（`INTERNET` / `ACCESS_NETWORK_STATE`）、明文策略、启动底色、包名/版本烘焙、APK 文件名、图标 | ≈ 不变 | 只在「APK 目标 SDK / 系统策略变化」时 | 与打包配置**无关**，缺了就是 bug |
| **L1 可选能力** | `android_permissions`（camera / microphone / location）、`tabs_enabled`、`mobile_bar` | 低 | 产品明确需要某项能力时 | 关掉它应用仍能正常跑，只是少了功能 |
| **L2 版本适配** | wry 生成的 Kotlin 胶水 3 处（`onPageFinished` / `shouldOverrideUrlLoading` / `onShowCustomView`）、`prepare-wry-kotlin.py`、`inject-gradle-patch-task.py` | 只在**锚点消失**时（见 §6） | 只随 wry / tauri 升级，且默认无需动脚本 | 上游修好了就自动退场，不是永久资产 |

判断一个需求该落哪层，只问一句：**关掉它，应用还能正常用吗？** 不能 ⇒ L0（无条件）；
能 ⇒ L1（配置）；只因为上游库没实现 ⇒ L2（且记进"待上游修复即删"清单）。

## 3. 现状盘点（15 个脚本归位）

| 脚本 | 层 | 改哪里 | 随配置变？ |
| --- | --- | --- | --- |
| `prepare-android-conf.py` | L0 | `tauri.conf.json`（version / productName / identifier，init 的输入源） | 是（`VERSION` / `APP_NAME`） |
| `apply-android-labels.py` | L0 | `strings.xml` + Manifest `android:label` | 是（`TITLE`，回落 `APP_NAME`） |
| `set-android-apk-name.py` | L0 | `build.gradle.kts` 的 `base.archivesName` | 是（`APP_NAME`） |
| `patch-android-permissions.py` | L0 + L1 | `AndroidManifest.xml`（恒需块 + 可选块） | 恒需**否** / 可选**是** |
| `patch-android-cleartext.py` | L0 | `res/xml/network_security_config.xml` + Manifest 属性 | 否 |
| `patch-android-splash-theme.py` | L0 | `res/values(-night)/*.xml` + 主题 item | 否 |
| `patch-android-insets.py` | L0 | `MainActivity.kt` + gradle 依赖 | 否 |
| `patch-android-back.py` | L0 | `MainActivity.kt`（返回键 callback） | 否 |
| `patch-android-download.py` | L0 | `MainActivity.kt`（`setDownloadListener`）+ Manifest 2 条权限 | 否 |
| `patch-android-extras.py` | L0 + **L2** | `MainActivity.kt`（JS 桥）+ `RustWebViewClient.onPageFinished`（密钥轮换） | 否 |
| `patch-android-schemes.py` | **L2** | `RustWebViewClient.shouldOverrideUrlLoading` | 否 |
| `patch-android-fullscreen.py` | **L2** | `RustWebChromeClient.onShow/onHideCustomView` | 否 |
| `prepare-wry-kotlin.py` | **L2** | 复刻 wry 的 9 个 Kotlin 模板（只为让 L2 补丁能提前跑） | 仅间接（包名） |
| `inject-gradle-patch-task.py` | **L2** | `build.gradle.kts` 注入 `patchWry*Step0..2` 重放链（wry build.rs 会覆盖 generated/） | 否 |
| `android_gen.py` | 基础设施 | 不写文件，只提供工程根/文件定位的单一真源 | 否 |

要点：**L0 全部与打包配置无关**（只有"烘焙进工程"的 app_name/version 例外，它们是身份不是能力）；
真正会随配置变的只有 4 个字段与 1 个可选权限数组。

## 4. 唯一的产品输入面（契约）

补丁脚本允许读取的产品字段（经 `parse-packaging-config.py` 写进 `GITHUB_ENV`）：

| 变量 | 用途 | 层 |
| --- | --- | --- |
| `APP_NAME` | 包名派生 / 安装名 / APK 文件名 / productName | L0 |
| `TITLE` | 桌面显示名（Android 用 `@string` 覆盖） | L0 |
| `VERSION` | `versionName` / `versionCode` 烘焙 | L0 |
| `ANDROID_PERMISSIONS` | 可选权限数组（camera / microphone / location） | L1 |

基础设施变量（不算产品输入）：`CONFIG_JSON` / `CONFIG_URL` / `GITHUB_ENV` / `GITHUB_WORKSPACE` /
`RUNNER_TEMP` / `WRY_SRC_DIR` / `W2A_SKIP_GRADLE_PATCH` / `W2A_ANDROID_PATCH_STRICT` /
`USERPROFILE` / `PATH`。

> 契约测试 `android_pipeline_contract::android_packaging_product_inputs_are_frozen`（⑨）扫描
> `.github/scripts/*.py` 里的 `os.environ` 读取，白名单外的键直接红灯。**新增一个"随打包配置变化"
> 的输入 = 主动要求下游改一次**，所以必须先进 §5 的判据、再更新本表与白名单。

## 5. 允许改打包逻辑的四种情形（其余一律先讨论）

1. **L0 常量补全**：APK 跑不起来 / 被系统拒（如缺恒需权限、明文被拦、SDK 目标前移）。
2. **L1 新增可选能力**：产品确有需求，且关掉它应用仍正常 —— 同时更新 `packaging-config.md` + 两个解析器 + 示例配置。
3. **L2 跟随依赖升级**：先看 §6 —— 多数升级**不需要动脚本**（漂移降级 / 上游实现即退场）；
   只有锚点真的消失（上游删了 override）才需要改，且同一批次内顺手删掉已修复的补丁。
4. **安全性修复**：会泄露凭据、可被注入的路径（如桥密钥、明文策略）。

反例（**不应**改打包层）：调整 UI 尺寸/颜色（走 `src/` 与 `dist/`）、新增页面能力（走 Rust 命令
或前端）、某一站点的兼容问题（走 `extra` / 配置）、"顺便"重构脚本（无行为变化的重构也要单独批次）。

## 6. 升级免疫：上游升级不产生改动（2026-11-19 落地）

诉求（评审原话）：打包逻辑要固化，**以后升级啥的都别改**。所以这里的目标不是"尽量少改"，
而是把"升级"从"人改脚本"变成"脚本自己处理"：

| # | 机制 | 做法 | 挡住哪些上游变化 | 落点 |
| --- | --- | --- | --- | --- |
| A | 锚点只认**平台合约** | 定位用 `WebViewClient` / `WebChromeClient` 的 override **签名**（`override fun shouldOverrideUrlLoading(` / `onShowCustomView(` …）再做花括号配对，方法体整段由脚本写入；**从不匹配方法体原文** | 重排方法、改缩进、加注释、改参数名、重写方法体 | 7 个补丁脚本的定位器 |
| B | 漂移**降级**、不打断构建 | `android_gen.AnchorDrift` + `run_guarded()`：锚点真的没了 ⇒ 单行 `::warning::` + **exit 0**；环境错误（`gen/android` 没 init、tauri.conf.json 缺失/坏）照旧红 | 上游删 override / 换类 / **改胶水文件名或位置** / 改入口类名 / 模板清单整体失效 | `android_gen.py` 的 `GUARDED_SCRIPTS` |
| C | 上游实现即**退场** | `fullscreen` 发现 `onShowCustomView` 体内已无 `onCustomViewHidden()` ⇒ 判定上游已实现，**原样返回**并提示可删脚本 | wry 修好全屏 | `patch-android-fullscreen.py` |

**B 只管"可降级能力"**：名单 8 个（`prepare-wry-kotlin` / insets / back / download / extras /
schemes / fullscreen / splash-theme）；`patch-android-permissions.py` 与
`patch-android-cleartext.py` **不在**名单 —— 它们缺了 App 直接打不开，宁可构建红也不出废包。
判据同 §2：关掉它，应用还能跑吗？

**默认宽松是给下游的**：照抄这套脚本的项目不该被上游升级打断。本仓库想让 CI 当门禁就设
`W2A_ANDROID_PATCH_STRICT=1`（漂移变 `::error::`）。它是基础设施变量，不改变任何打进包里的内容。

### 证据：`rehearse.py --drill`

在**真 wry 模板**生成出来的 Kotlin 上施加 4 类"升级时真会发生"的变化，再跑完整条链：

| 变异 | 期望 |
| --- | --- |
| `reformat`（缩进 +2、插注释） | 三个补丁照样落盘 |
| `param_rename`（`view`→`w`、`request`→`req`） | 照样落盘（锚点只认类型） |
| `upstream_fullscreen`（上游把全屏实现了） | 补丁**自动退场**（不报错、不覆盖上游实现） |
| `anchor_gone`（删掉 `shouldOverrideUrlLoading`） | 默认降级跳过；`W2A_ANDROID_PATCH_STRICT=1` 时红 |
| `wry_file_renamed`（`RustWebViewClient.kt` 改名） | 改它的两个补丁降级、`fullscreen` 照打；**半成品方向安全**（`MainActivity` 的 JS 桥仍完整，因为桥不反向依赖 client） |

结论：**这 5 类变化下一个 Python 文件都不用改。** 唯一还会让你动手的情形只剩"上游删了
override 又没给替代" —— 那时补丁本身也该退役了。

### 失败分类的边界（别把"环境坏了"也降级）

| 情形 | 抛什么 | 结局 |
| --- | --- | --- |
| `gen/android` 根本没生成 / `java/` 目录不存在 | `FileNotFoundError` | 红（`cargo tauri android init` 没跑） |
| 工程在、`RustWebViewClient.kt` / `MainActivity.kt` 不在 | `AnchorDrift` | 降级（上游改名/挪位置） |
| `tauri.conf.json` 缺失 / `identifier` 为空 | `FileNotFoundError` / `ValueError` | 红（环境配错） |
| wry 的 `src/android/kotlin/` 定位不到、或模板清单全部未命中 | `AnchorDrift` | 降级（build.rs 后面仍会生成） |
| `AndroidManifest.xml` 找不到 `</manifest>` / `<application>` | `ValueError` | 红（L0 必需项，见 §6 名单） |

## 7. 目标形态：把 L2 用自身代码吃掉（迁移路线，可选）

L2 存在的唯一理由：wry 0.57 没做、而 Tauri 又没暴露等价钩子。去掉它带来的收益不只是少改脚本 ——
`prepare-wry-kotlin.py` 与 `inject-gradle-patch-task.py` 这两条"为了重放补丁"的基础设施会一起消失。

| # | 现在（L2 补丁） | 改成（自身代码） | 依赖 / 风险 | 验证 |
| --- | --- | --- | --- | --- |
| ① | `schemes`：`shouldOverrideUrlLoading` 里做 scheme 白名单 + Intent 外跳 | Rust 侧 `on_navigation` 拦下非 http(s) 导航 + 官方 `tauri-plugin-opener` 打开 | 需实测 Android 上 `on_navigation` 的返回值是否阻断 WebView 加载；`opener` 的 Android 实现是否覆盖 `intent://` 的 `browser_fallback_url` 语义 | 真机：`tel:` / `mailto:` / `weixin://` / `intent://` 四类 + 无应用兜底 |
| ② | `extras`：在 `RustWebViewClient.onPageFinished` 里轮换桥密钥 | Rust 侧在 `on_page_load` 里 `webview.eval()` 注入密钥（`eval` 只作用于主框架，语义等价） | 需确认 Android 上 `eval` 可用（init script 已走同一通道） | 真机：跨源 iframe 拿不到密钥、旧密钥失效 |
| ③ | `fullscreen`：整体替换 `onShow/onHideCustomView` | 跟随 wry 升级（上游已实现全屏 custom view 时**删除**本补丁） | 需等 tauri/wry 版本；升级前保留 | 真机：进出全屏、横屏、沉浸还原 |
| ④ | 4 个脚本各自往 `MainActivity.kt` 拼字符串注入（insets/back/download/extras） | 静态覆盖文件 `Web2AppNative.kt`（仓库内**成品**文件）+ MainActivity 只留 1 行 `web2appInstall(this, webView)` | 需把"注入"变成"复制 + 单点 hook"；成品文件随 tauri 模板变化 | 补丁链 `--selftest` + `rehearse.py` 真模板重放 |

③ 完成后 L2 归零 ⇒ 可删 `prepare-wry-kotlin.py`、`inject-gradle-patch-task.py`、以及两份 workflow 里
对应的 2 个步骤与"校验注入结果"步骤（15 → 11 个脚本）。**注意顺序**：③ 必须最后做，
否则 ①② 仍要靠 gradle 重放链在 build.rs 之后落盘。

## 8. 不可替代清单（永远在装配期，别想着用运行期代码绕）

- `AndroidManifest.xml` 的权限与清单属性（安装期确定，运行期无法自授权）；
- `network_security_config.xml`（明文策略是系统读取的静态资源）；
- 主题 `windowBackground`（首帧之前那一层，Rust 无 API —— `tao` 对 Android 的
  `with_background_color` 是空实现）；
- Gradle 的包名 / 版本号 / APK 产物名；
- `tauri.conf.json` 的 `identifier`（Android 包名唯一来源，必须在 `init` 之前写）。

## 9. 本次修复在模型里的位置

- `ACCESS_NETWORK_STATE` 判定为 **L0**（关掉它，域名数组包直接打不开）⇒ 放进 `REQUIRED_PERMISSIONS`，
  **无条件**注入，不经过 `android_permissions`。
- `resolve.rs::fallback_config()` 是**运行期**兜底（读不到系统 DNS ≠ 网络不通），
  属于 `src/` 的自身代码，**不进打包层** —— 这正是"能用自己的代码解决就不要动打包逻辑"的示例。
- 唯一被改动的打包逻辑：`patch-android-permissions.py` 从"空数组整步跳过"改为"恒需块无条件、
  可选块按配置"。这是一次 **L0 补全**，不是 L1 扩张。

## 10. 怎么验证（改打包层时逐项跑）

1. `python3 .github/scripts/<改动的脚本> --selftest`（CI 的目录发现循环也会跑）；
2. `python3 tests/android-template-rehearsal/rehearse.py` —— 在 **tauri 真模板**上重放整条链；
   加 `--drill` 跑升级演练（4 类上游变化，见 §6）；
3. `cargo test --all`（含第 ⑧ 条"步骤集合由目录派生"与第 ⑨ 条"产品输入面冻结"）；
4. 真机按 `android-improvements.md` §4 清单回归，新增项要先写进清单再出包。
