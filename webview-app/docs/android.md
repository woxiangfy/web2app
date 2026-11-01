# Android 专项（构建 · 补丁链 · 排障）

> 本文是 webview-app Android 端的唯一详细记录：构建准备、Kotlin 补丁链契约、移动端底栏、启动问题排障史与数据目录。
> 概览见 [../AGENTS.md](../AGENTS.md)；优化任务清单（P0/P1/P2 与真机验收）见
> [../../docs/android-improvements.md](../../docs/android-improvements.md)。

## 构建步骤（一次性准备 + 构建）

```bash
# 1) 安装 Tauri CLI
cargo install tauri-cli --version "^2"

# 2) 初始化安卓工程（仅首次；生成 gen/android 与 Kotlin 胶水）
#    ⚠️ gen/ 已被 .gitignore 忽略，**不要提交**（CI 每次 `cargo tauri android init --ci` 现生成）
cargo tauri android init

# 3) 构建 APK（调试/直接安装）
#    ⚠️ tauri-cli 的 --target 只接受**架构名**（aarch64/armv7/i686/x86_64），
#       不是 rustup 的完整三元组 aarch64-linux-android —— 写成后者会直接报错。
cargo tauri android build --target aarch64 --apk
# 若需上架 Play Store 的 AAB，去掉 --apk 即可
```

> 本地首次构建前需跑一次 `cargo tauri android init`。该目录已被 `.gitignore` 忽略，**无需提交**；
> `release.yml` 的 Android 任务会在 CI 里用 `cargo tauri android init --ci` 现生成。

### 环境要求（Android）

- JDK 17
- Android SDK + NDK。**平台、build-tools 与 NDK 版本都不要写死**：
  Tauri 模板随 tauri-cli 前移（Tauri 2.12 起 `compileSdk = 37`，且不再指定 `buildToolsVersion`），
  CI 由 `.github/scripts/install-android-sdk.sh` 读模板实际值、再从仓库里真实存在的包名中挑
  （注意 Android 16 起平台包带次版本号：`platforms;android-37.0` 而非 `android-37`，且 37.x 只在 canary 通道）；
  ⚠️ 模板要的平台在仓库里**不存在**时，该脚本会回落最高可用平台并改写 gradle（尽力而为，会打
  `::warning::`）—— 要让它硬失败，调用点设 `ANDROID_SDK_STRICT=1`。
  NDK 版本由 `.github/scripts/install-android-ndk.sh` **单一真源**（三个 workflow 共用，不再各写死一遍）。
- 已执行过 `cargo tauri android init`（见上）

## 共享补丁脚本与补丁链契约

安卓工程（`gen/android`）的所有改写都由 `.github/scripts/` 下的**共享脚本**完成，两个打包
workflow 只调用它们，不各自内联（避免漂移）：

| 脚本 | 职责 |
|---|---|
| `prepare-android-conf.py` | init 前写 version / productName / identifier |
| `apply-android-labels.py` | 应用名 |
| `prepare-wry-kotlin.py` | **补丁链前置**：复刻 wry build.rs 预生成 9 个 Kotlin 模板到 `generated/`，否则补丁跑在 build.rs 生成之前必 FileNotFoundError |
| `inject-gradle-patch-task.py` | 往 `app/build.gradle.kts` 注入 `patchWry<变体>` Exec 任务链：wry build.rs 在 Rust 编译期覆盖 generated/ 后、Kotlin 编译前重放 extras → schemes → fullscreen；Gradle 9 内置 Exec + configuration-cache 安全，按 rustBuild/compile 同名变体配对 |
| `patch-android-insets.py` | 系统栏 insets |
| `set-android-apk-name.py` | APK 文件名 |
| `patch-android-back.py` | 返回键守卫 |
| `patch-android-download.py` | 下载接管 + Manifest 权限 |
| `patch-android-extras.py` | 更多面板 JS 原生桥 + 主框架密钥轮换 |
| `patch-android-schemes.py` | 外部 scheme 外跳 |
| `patch-android-fullscreen.py` | H5 视频全屏：整体替换 wry 0.57 空实现的 onShow/onHide |
| `patch-android-permissions.py` | 按打包字段 `android_permissions` 按需声明相机/麦克风/定位权限 |
| `install-android-ndk.sh` | NDK 版本单一真源 |

⚠️ **补丁执行顺序固定**：prepare-wry-kotlin + gradle 任务注入（前置）→
insets → back → download → extras → schemes → fullscreen → permissions；
构建期 gradle 再按 rustBuild<变体> → patchWry<变体>Step0..2 → compile<变体>Kotlin
重放 extras → schemes → fullscreen（MainActivity 系补丁不被 build.rs 覆盖，只打一次；
根因与排障见 `docs/changes/2026-10-15.md`）。
download 复用 back 新建的 `onWebViewCreate` override 只插一行；extras 排在 download
调用之后，改 client 的 onPageFinished，与改 shouldOverride 的 schemes 互不耦合；
fullscreen/permissions 详见 `docs/android-improvements.md` #5/#6。
工程根解析走 `android_gen.py`（CI 的 cwd 是 `webview-app/`，故首选 `gen/android`）。

⚠️ **注入进 `build.gradle.kts` 的 Kotlin 片段必须能通过脚本编译**（它只在 gradle **配置期**才被编译，
本地 `--selftest` 跑不出来——2026-11-01 就是这个盲区炸的，见 `docs/changes/2026-11-01.md`）：
在脚本作用域里 `java` 是 Gradle 生成的**类型安全访问器**（`JavaPluginExtension`）而**不是包名**
⇒ 禁写 `java.io.File(...)` / `java.nio.*` / `java.util.*`（会得到 `Unresolved reference 'io'`），
派生路径一律用 `File#resolve`（`org.gradle.*` 不受影响，`kotlin.io` 等走隐式导入的也没事）。

⚠️ **注入块里的仓库路径不许「数层数」**（同日第二轮，同一条根因的第二种形态）：
`rootProject.projectDir` 是 `<repo>/webview-app/gen/android`，到仓库根是**三级**；早期写
`parentFile.parentFile`（只上两级）⇒ 得到 `webview-app/.github/scripts`。而 Exec 的 `workingDir`
只在**执行期**校验 ⇒ 配置期照样"成功"、白等 5 分钟 Rust 编译才报
`Working directory ... does not exist`（且日志里看不出是路径算错）。
⇒ 规则：从 `projectDir` 逐级向上找第一个含 `patch-android-extras.py` 的 `.github/scripts` →
再用「注入时固化的绝对路径」兜底 → 两者皆无则在**配置期**抛 `GradleException`。
CI 侧另有「校验注入结果」步骤用同款规则**独立复算**一遍（两 workflow 逐字相同）。

⚠️ **注入的 Kotlin 片段还要做「形参 ↔ 实参」类型核对**（同日第五轮）：wry 0.57 的
`shouldOverrideUrlLoading(view, request: WebResourceRequest)` 第二参**不是** `Uri`，
而 `patch-android-schemes.py` 的辅助方法形参是 `uri: Uri` ⇒ 守卫必须传 `request.url`，
直接传 `request` 会得到 `e: RustWebViewClient.kt:115:47 Argument type mismatch`。
字符串拼接式注入不经过类型检查，**selftest 断言若照抄实现就永远绿** ⇒
每个带辅助方法的补丁都要有一条「形参类型 ↔ 实参文本」核对（见
`patch-android-schemes.py::_assert_helper_call_types`）。离线复现：`rehearse.py` 产物的行号与
CI 报错行号一致。

## 移动端底栏（`mobile_bar`，仅移动端）

编译期开关 `MOBILE_BAR=true` 开启（默认关，**仅移动端**）：屏幕底部常驻一条导航条 —— **首页**（回到 `app_url`，已在首页则重新加载；未就绪时重走一次 bootstrap）/ **后退** / **前进** / **列表**（仅多页签）/ **更多**（恒在最后，无条件渲染）。

**「更多」面板（2026-10-14 起）** 从底栏升起，六行：刷新 / 复制链接（Clipboard + execCommand 兜底）/ 分享 / 在浏览器打开 / **清除浏览数据（两步确认，3s 撤防）** / 退出；后四个动作经打包期注入的 Kotlin 桥 `__web2appAndroid` 直达，带主框架一次性密钥、iframe 调不到，操作反馈走面板 toast。后退前进的**落点守卫与桌面端标题栏同一条规则**（没有真实上/下一条、或落点是内置 splash 时不动作并置灰），与 Android 系统返回键的判定同源。

**「列表」按钮只在 `tabs_enabled=true` 时渲染**（2026-10-13 起移动端落地【单 webview 页签记录】：面板可新建 / 切换 / 关闭，首页固定 0 号位；wry 不支持同窗口多子 webview，切换 = 按记录 url 重新加载，页面状态不保留；判定单一真源 `mobilebar.rs::tabs_available`），关闭时只画前三个按钮，绝不出现死按钮。

底栏按**底部空间契约**把 `body` 缩高一格底栏并让出 `env(safe-area-inset-bottom)`，同时把页面自己的贴底固定条上移（**含窄幅悬浮按钮 FAB**：fixed 贴底件无宽度门槛，且保留元素原 px 底边距），否则页面页脚 / FAB 会被底栏切掉半截。加载转圈层经 `__web2appMobileBarH` + `web2app:chrome-ready` 让出底栏，加载期间底栏仍可见可点。桌面端（Windows Tauri / Electron）忽略该开关：常量整体 `#[cfg(mobile)]`，桌面目标无此代码路径。

**`target="_blank"` / `window.open()` 进移动页签**（2026-10-14 起，仅多页签模式）：`mobilebar.js` document-start 装 blank guard —— 捕获阶段 click 向上找最近 `<a>` 命中 `_blank` 且 http(s) 即拦，`window.open` 同拦（非 http(s) 回落原实现交 schemes 外跳），统一发 `web2app:open-tab` `{url}`；`mobilebar.rs` 新事件（与无 payload 落首页的 `web2app:new-tab` 区分）只承接 http(s) 并复用单 webview 页签记录开新页。单页签包不装 guard。

## 启动问题排障史（v1.60–v1.65，改 Android 启动流程前必读）

**Android 打开后转圈十几秒 → 白屏 / 失败卡**：**不能直接断定是设备网络不通**（v1.60 曾这样误判，被真机反馈纠正：手机浏览器访问同一网址完全正常）。v1.61 起失败卡自带「现场诊断」（当前地址 / readyState / HTTP 状态 / 子资源数 / WebView UA / 页面脚本错误），凭一张截图即可分层定位：

1. 「地址」仍是 `http://splash.localhost/`：导航根本没离开启动页 → 壳内导航链问题（不是网络）；已是目标域名：外链文档已开始加载但没渲染出正文 → 网络 / TLS / WebView 内核层。
2. 「HTTP」为 0 / 无记录且手机浏览器能打开同一网址：先看「UA」行的 WebView 版本——无 GMS 的国产 ROM 上 **Android System WebView 可能长期不更新**，过旧内核在新证书链（TLS 1.3 / 现代根证书）或 HTTP/2 上会失败，而手机自带浏览器多为独立更新的内核，二者不矛盾；可到「设置 → 应用 → Android System WebView」更新或启用后重试。
3. 失败卡上「无法连接服务器 / 连接超时 / 安全连接失败」是 v1.60 Rust 侧 reqwest 探测的**独立**结论（与 WebView 不是同一网络栈），仅作参考；浏览器能打开时以诊断卡为准。
4. 仍需逐字网络错误时再抓 `adb logcat | findstr /i "chromium net::"`（或 Android Studio Logcat 过滤 `chromium` / `net::`）；旧打包产物不含诊断卡，必须用当前代码**重新打包**。

### v1.62 修正的两条「必然」路径（与网络无关，出问题时可据此排除）

1. **启动页的加载层语义**：启动页必须走 splash 语义（SLOW 5s / FAIL 40s 的「启动超时 + 重试」、不装就绪监听）。模式标记 `window.__web2appLoadingMode` 由建窗时的初始化脚本在 `OVERLAY_SCRIPT` **之前**注入；漏掉它不会报错，只会让启动页装上 overlay 的空白文档硬上限（CAP 15s；启动页 body 只有 `<script>`，`bodyEmpty()` 恒真）⇒ 一次正常但偏慢的启动被判「页面长时间为空白」。静态守卫 `contract_tests::loading_mode_contract`。
2. **启动页「重试」的落点**：wry 在 Windows/Android 上把 `splash://localhost/` 改写成 `http://splash.localhost/`，页内 `location.protocol` 是 `http:` ⇒ 判定必须看 hostname（`splash.localhost`），跳转必须用**相对路径** `/retry`（绝对 URL 的 scheme 内核不认）。判错时的表象是「点重试只刷新启动页、永远起不来」。契约测试 `loading::tests::retry_uses_relative_path_and_rewritten_hostname`。

### v1.63 让启动期「自己把现场说清」

1. **壳层对主框架加载失败是「必然失明」的**：wry 在 Android 上把 `load_url` / `eval` 实现成「投递 main-pipe 消息后直接 `Ok(())`」⇒ `window.navigate()` / `wv.eval()` **恒返回 Ok、永不报错**（`bootstrap.rs::navigate_main` 里那条 `Err(e) => show_error` 在 Android 上是死代码），且 `RustWebViewClient.onReceivedError` 只对 `ERROR_CONNECT` 做协议重定向特例、**不向壳层透出任何网络错误**。后果：主框架加载失败时 WebView 把启动页留在屏幕上，壳侧一无所知，最后只剩 40s 兜底卡。根因在 wry 上游，本版改为**让卡片自己说清**。
2. **兜底卡现在带现场**：40s 的「启动超时」卡补了 `diagText()`，并在仍停在 splash 时提示「目标地址的导航没有提交」——配合新增的进度行 `正在解析目标地址（N 个域名）…` / `正在打开 <host>…` / `正在启动本地反向代理…` / **`已发起导航，等待页面响应…`**，一眼即可区分「根本没导航」「导航了但没提交」「网络不通」「内核不渲染」四类。⚠️ 判读：卡上**没有**「已发起导航」⇒ `prepare_navigation` 还没走到导航（解析 / 代理阶段）；有这一行却仍停在 splash ⇒ 导航已发出但文档未提交（多半落在 WebView 内核层）。
3. **可达性探测成功时也会上报**：卡上出现「可达性探测：<域名> 已连通（HTTP 200，xxxms），但页面未渲染」时，说明 Rust 侧（reqwest + rustls/webpki-roots）能通而 WebView 不能 ⇒ 问题在**内核**（版本 / TLS 协商 / 渲染），不在网络，优先升级系统「Android System WebView」或换机比对；反过来显示「无法连接服务器 / 连接超时 / 安全连接失败」时才是传输层问题。（此前探测成功时**什么都不推**，最需要的那条判据恰好被丢掉。）
4. **启动期错误不再被静默吃掉**：加载层在挂载完成前是**缓冲队列**（挂载后回放），壳侧在启动早期推来的失败会照常显示，不必再靠「等一下看会不会卡」来判断。另：解析阶段有 25s 总预算（`RESOLVE_BUDGET_SECS`，超时给明确文案而不是一直挂着），解析失败文案保留具体原因（读系统 DNS 配置失败 / 查询某域名失败 / 全部密文解不开 / 预算超时）。

### v1.63.1 让诊断「真的看得见」（v1.63 的补丁）

v1.63 把现场**拼进**了卡片，但「拼进去」≠「看得见」——这类形态本项目已经栽过三次，故补三处并各配静态守卫（`loading::tests::failed_card_must_be_scrollable_and_selectable` / `diag_address_is_redacted_before_display`）：

1. **进度行前置**（实测生效）：`toFail` 里 notes 由「追加到末尾」改为「置于最前」。分诊靠的就是「已发起导航」这类行，地址 / UA 属第二步才看的技术细节；拼在末尾时它们会随长诊断一起沉到滚动区最底下。
2. **失败态可滚动 + 可选中**：失败态改为 `justify-content:flex-start` + `overflow-y:auto`，并放开 `user-select`（根规则是 `none` ⇒ 用户选不中、复制不了，而卡片用途恰恰是「把信息反馈给打包者」）。⚠️ **实测 360×640 / 320×480 下内容并未溢出（`overflowPx=0`）**，故滚动是**防御性**修复（极端小屏 / 更多进度行才触发）；「可选中复制」是当场生效的改进。
3. **地址脱敏**（实测生效）：诊断行的地址经 `redact()` 处理，把 `w2a_key` / `key` / `token` 等查询参数值换成 `***`。启用本地代理时入口形如 `http://127.0.0.1:<port>/auth.html?w2a_key=<门禁密钥>`，而这张卡的用途就是**截图外发**——不脱敏等于把门禁密钥一起发出去。

取证脚本：`electron-app/scripts/diag/splash-fail-card.cjs` —— 按真实窗口尺寸渲染同一份 `splash.html` + `loading.js`（把 40s 兜底卡提前到 1.4s，其余逻辑逐字不动），客观测「进度行是否在最前 / 地址是否脱敏 / 能否滚动 / 关键行在首屏的 y」并出 PNG。

### v1.64 启动期「发得太早」（真机取证定到的根因）

1. **根因不是「导航没发」，而是「导航被覆盖」**：Android 上壳侧对 WebView 的一切操作都走 wry 的 main-pipe（`wry/src/android/main_pipe.rs`），而 `WebViewMessage::LoadUrl` / `WebViewMessage::Eval` 的处理体都是 `if let Some(webview) = get_webview(activity_id) { … }` —— **取不到就静默丢弃**（那个 webview 只在 `CreateWebView` 处理完才写入）。更要命的是**初始 splash 导航是异步排队的**：`CreateWebView` 内走 `load_url(…, main_thread = true)` → Kotlin `loadUrlMainThread`（`RustWebView.kt` 里是 `post { loadUrl(url) }`，排到 UI 队列尾），而之后的 `navigate` 走**同步** `loadUrl`。直连配置下 `prepare_navigation` 只需毫秒级 ⇒ 导航先执行、随后被那条 post 出去的 splash 加载**整体覆盖**；同期那几条进度 `eval` 又落在 splash 文档 document-start **之前**（`window.__web2appLoading` 还没定义）被 `&&` 短路吃掉。⇒ 真机卡上「地址仍是 `splash.localhost`」+「零进度行」是**同一个机制**。
2. **修复：非 Windows 启动期先等首个文档加载完成**，再做解析 / 推送 / 导航（`bootstrap::BootReady` + `wait_first_page_ready`，预算 `SPLASH_READY_BUDGET_MS = 5000`）。就绪信号来自建窗时注册的 `on_page_load`（wry Android 的 `ON_LOAD_HANDLER` ← `RustWebViewClient.onPageStarted / onPageFinished`，`tauri-runtime-wry` 无条件接线）—— 这是本壳在 Android 侧唯一可靠的「文档就绪」判据（轮询 URL 不行，`GetUrl` 同样走 main-pipe）。⚠️ `Started` 必须**清**标记，否则 `/retry` 重载启动页后会沿用旧标记、变成「首启修好、重试仍卡死」；⚠️ 超时**不阻断启动**（退化成旧行为），但会在卡片上留一行「启动页未在 5000ms 内报告加载完成（页面加载事件缺失）」—— 这一行本身即判据。Windows 完全不加（WebView2 时序由 wry 保证），故新增代码全部 `#[cfg(not(windows))]`。
3. **判读下一版真机结果**：卡上出现进度行 ⇒ 修复生效（再按 v1.63 的判读表分层定因）；连进度行都没有、且出现上面那句「页面加载事件缺失」 ⇒ `on_page_load` 在真机没送达，问题回到 wry 上游。
4. 静态守卫：`contract_tests::startup_visibility_contract::android_boot_waits_for_first_page_load` / `page_load_event_is_wired_on_main_window`（均做过反向对照）。⚠️ 本版新增代码全在 `#[cfg(not(windows))]` 下、**本机是盲区**，已用 cfg 互换强制编译验证（8 处翻成 `#[cfg(all())]` 后 `cargo check --all-targets` 退出码 0，随后逐字节还原）。

### v1.65 导航「落点校验 + 有限重发」（v1.64 的检验与自愈，两者互补）

1. **为什么还需要它**：v1.64 做的是「等首个文档就绪再导航」，那是**时序预防**；而 Android 上**没有任何返回值能证明导航生效** —— `navigate()` 恒返回 `Ok(())`（它只是往 main-pipe 投一条消息）。所以真机上仍可能出现「导航确实发起了、地址却停在 `splash.localhost`」。v1.65 改为**直接检验结果**：导航后隔 `NAV_COMMIT_CHECK_MS = 500` 读一次真实地址，仍停在启动页就**重发导航**（最多 `NAV_COMMIT_MAX_RETRY = 2` 次）。`loadUrl` 语义幂等、重发只以最后一次为准，无副作用；正常启动只是一次「读地址 + 返回」。
2. ⚠️ **判据必须同时认两种形态**：`splash://`（`loading::splash_url()` 构造的）与 `http://splash.localhost/` —— wry 的 `custom_protocol_workaround` 会把前者改写掉，**WebView 里真实存在的地址是后者**，只认自定义协议会在真机漏判（v1.62 栽过同类坑）。`is_splash_url()` 两个都认。
3. ⚠️ **必须跑在独立线程**：`navigate_main` 在主线程，而读地址走 main-pipe（webview 未就绪时最长阻塞 `MAIN_PIPE_TIMEOUT` = 10s），放主线程会冻 UI。
4. **下一版真机怎么读这张卡**：出现「重发导航已生效（第 N 次）」⇒ 根因确实是导航被挤掉、重发救回来了；出现「重发导航 2 次后仍未离开启动页（导航未提交，与网络无关）」⇒ 与网络无关，要往 wry 上游走；两条都没有且页面正常 ⇒ 走的正常启动路径。
5. 静态守卫 `android_navigation_is_verified_and_retried`（4 条断言：判据双形态 / 调用点 / 重发上限 / 独立线程）。⚠️ **反向对照抓出守卫自身的盲区**：第一版用裸 `contains("ensure_navigation_committed(app, label, &url);")`，把调用点改成**注释**（`// …`）后守卫**仍然通过** —— 注释里含同样文字，`contains` 被满足；改为 `has_code_line()`（排除 `//`、`*` 开头的行）后，同样的改坏方式**如期 FAILED**。这是本项目「守卫的针被注释满足」的**第四种形态**（前三：注释里写裸 hostname、说明文字含同一字面量、反向断言被自身注释干扰）。

## 数据存在哪 / 包名 / tauri.conf 字段

**Android 端应用的数据存在哪？**（下文中 `<pkg>` = 构建时的应用包名，即打包工作流在 `android init` 前
由 `app_name` 清洗后写入 `tauri.conf.json` 的 `identifier` = **`com.web2app.<app_name 清洗值>`**
（仓库里 `tauri.conf.json` 的静态值只是占位；本地不跑工作流直接 `cargo tauri android` 构建时才会用到它）；
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

**`cargo run` 报 `unknown field package` / `targetSdkVersion` / `permissions`（build script 阶段）**：
`tauri-build` 对 `bundle.android` 只识别少量字段，本仓的 `tauri.conf.json` 因此**只写 `minSdkVersion`**（实测当前依赖 `tauri-utils 2.10.1` 的 `AndroidConfig` 只认 `minSdkVersion` / `versionCode` / `autoIncrementVersionCode` / `debugApplicationIdSuffix` 四个键，且 `deny_unknown_fields`；**应用包名只能取自 `identifier`**）。
`package` / `targetSdkVersion` / `useKotlin` / `permissions` 写进去会在**桌面**构建时同样报错（`tauri-build`
始终解析整份 `tauri.conf.json`）。处理方式是**保持精简**，其余在 `cargo tauri android init` 生成
`gen/android` 后于原生工程里配置 —— **不要为了凑这些字段去升级依赖**。

> ⚠️ 但这不等于「Tauri 版本不许动」：**v1.80 已把 tauri 升到 2.12.1**（wry 0.57 / tao 0.37），
> 目的是让 `tauri-runtime-wry` 透传 `no_redirection_bitmap`（球窗真逐像素 alpha，见 [float-ball.md](float-ball.md)）。
> 升级本身不是禁区，代价是必须逐项复验 WebView2 钩子（`webview_hooks.rs` 的初始化脚本、新窗口、下载、
> 右键）与多页签链路；本仓 v1.80 的升级经 `cargo check` 零涟漪 + 全量测试通过，尚未在真机复验。

**换产品名 / 包名**：改 `Cargo.toml` 的 `package.name`（决定 exe / 进程名）、`tauri.conf.json` 的
`productName`（决定窗口标题 / 托盘提示）即可。**Android 应用包名不再手改 `identifier`**：打包工作流会在
`android init` 之前按打包配置里的 `app_name` 自动把 `identifier` 改写成 `com.web2app.<清洗值>`
（清洗规则见 `docs/packaging-config.md` §4.2）。若直接跑 `cargo tauri android`（不经工作流），才会用到
`tauri.conf.json` 里的静态 `identifier`——此时请自行确保它已是你要的包名（Tauri 2 的 `bundle.android`
尚不支持 `package` 字段，包名只能取自 `identifier`）。不同 `app_name` 的 APK 包名不同、可以在同一台手机共存（否则全都叫 `com.example.web2app` 会互相覆盖安装）。
