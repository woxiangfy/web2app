# Android 端优化清单与实施跟踪

> 本文件是 Android 壳（`webview-app/` 的 `#[cfg(mobile)]` 部分 + CI 安卓补丁链路）
> 易用性 / 功能优化的**唯一任务清单**。每条建议保留原始编号可追溯；实施状态在本文件维护，
> 详细变更过程写入 `docs/changes/YYYY-MM-DD.md`。
>
| 基线（取证于 2026-10-14） | 版本 |
| --- | --- |
| Tauri | 2.12.1 |
| wry | 0.57.0 |
| tao | 0.37.x |
| minSdk / target | 24 / 模板默认（compileSdk 随模板） |
| 产物 | aarch64 APK（CI 现签名） |

## 0. 实施约定（与现有架构同构，不引入新插件）

1. **补丁一律走 CI Python 脚本**：`gen/android` 由 `cargo tauri android init` 每次重新生成，
   任何对生成工程（`MainActivity.kt`、wry 的 `RustWebViewClient.kt`、`AndroidManifest.xml`）的
   修改都必须落在 `.github/scripts/patch-android-*.py`，并被 **两个** workflow
   （`build-custom.yml`、`release.yml`）的 android job 在 init 后依次调用。
   **固定顺序（2026-10-15 起）**：
   **prepare-wry-kotlin → 注入 gradle 补丁任务链**（两个前置环节，2026-10-15 新增，见下）→
   insets → back → download → **extras** → schemes →
   **fullscreen → permissions → cleartext → splash-theme**（extras 复用 download 建好的
   `onWebViewCreate` 且调用必须排在其后；它改的是 `onPageFinished`，与改
   `shouldOverrideUrlLoading` 的 schemes 互不耦合；fullscreen 整体替换
   `RustWebChromeClient` 的 onShow/onHide 两方法；permissions 只动 AndroidManifest 且与
   download 各用各的 marker；cleartext 写 `res/xml` 并改 Manifest 的明文开关，与 permissions
   同文件但属性不重叠，2026-11-14 起取代原先 CI 里的全局 `usesCleartextTraffic=true` sed；
   splash-theme 只写 `res/values` 与主题 item，与 cleartext 的 `res/xml` 不重叠）。
   - **为什么需要两个前置环节**：wry 的 Kotlin 文件（RustWebViewClient 等）不是 init 产物，
     而是 wry `build.rs` 在 **Rust 编译期**才生成到 `java/<包名>/generated/`，补丁脚本在
     build 之前跑会 `FileNotFoundError`；且 build.rs 编译时还会覆盖提前打的补丁。
     `prepare-wry-kotlin.py` 先复刻 build.rs 把 9 个模板生成到 `generated/`（让首轮补丁可跑），
     `inject-gradle-patch-task.py` 往 `app/build.gradle.kts` 注入任务链，在 build.rs 覆盖后、
     Kotlin 编译前用内置 `Exec` 任务（Gradle 9 已移除 `Project#exec`，且模板开了
     configuration-cache）按 `rustBuild<变体> → patchWry<变体>Step0..2 → compile<变体>Kotlin`
     的同名变体配对重放 extras → schemes → fullscreen（MainActivity 系补丁不被覆盖，无需重放）。
     细节与排障全过程见 `docs/changes/2026-10-15.md`。
2. **每个补丁脚本必须带 `--selftest`**（合成源码夹具 + 幂等断言），并登记进
   `ci.yml` 的 `config-contract` 作业门禁。
3. 工程根解析只允许用 `.github/scripts/android_gen.py` 的 `resolve_gen_root()`，
   不再写死 `webview-app/gen/android`（2026-10-13 的静默中断教训见该模块头注释）。
4. 纯 Rust / 注入脚本的改动走双壳契约测试；Android 专属（`mobilebar.*`）只需 Tauri 门禁。
5. 打包配置语义变更必须 Python 解析器与 `scripts/packlib/parser.mjs`（Node 复刻）双侧同步，
   并跑 `node scripts/pack-local.mjs --parity`。

## 1. 总映射矩阵

| # | 优先级 | 事项 | 状态 | 落地位置 |
| --- | --- | --- | --- | --- |
| 1 | P0 | 文件下载接管（DownloadManager + 长按存图） | ✅ 已实施（2026-10-14） | `patch-android-download.py` |
| 2 | P0 | 外部 scheme 外跳（tel/mailto/weixin/intent…） | ✅ 已实施（2026-10-14） | `patch-android-schemes.py` |
| 3 | P0 | Android + URL 内嵌账号密码 + 直连 = 打包期硬失败 | ✅ 已实施（2026-10-14） | `parse-packaging-config.py --target android` + Node 复刻 |
| 4 | P1 | Android 清除浏览数据入口（桥内补全配方，详见 §2 #4/#8） | ✅ 已实施（2026-10-14） | `patch-android-extras.py`（JS 原生桥，**非** `web_data.rs`）+ 更多面板 |
| 5 | P1 | 相机 / 麦克风 / 定位权限声明 | ✅ 已实施（2026-10-14） | 打包字段 `android_permissions`（21 键）+ `patch-android-permissions.py` |
| 6 | P1 | 网页视频全屏（wry 0.57 的 onShowCustomView 是空实现） | ✅ 已实施（2026-10-14） | `patch-android-fullscreen.py`（整体替换 onShow/onHide） |
| 7 | P1 | `target=_blank` / `window.open` 拦截进移动页签 | ✅ 已实施（2026-10-14） | `src/mobilebar/tabs.js` blank guard + `mobilebar.rs` 新事件 `web2app:open-tab` |
| 8 | P1 | 底栏「更多」面板（刷新 / 复制链接 / 分享 / 浏览器打开 / 清数据 / 退出） | ✅ 已实施（2026-10-14） | `src/mobilebar/more.js` + `patch-android-extras.py`（Kotlin 桥；`mobilebar.rs` 零改动） |
| 9 | P2 | 返回键优先关闭页签列表面板等浮层 | ✅ 已实施（2026-11-14） | `patch-android-back.py`（`evaluateJavascript` 续延）+ `core.js::closeSheetIfOpen` |
| 10 | P2 | 无历史时「再按一次退出」而不是立即 finish | ✅ 已实施（2026-11-14） | `patch-android-back.py`（Toast + 2s 窗口） |
| 11 | P2 | 页签切换重载丢状态：先文案明示，后评估快照 | ✅ 文案已实施（2026-11-14）；❌ 快照不做（与 #17 同判据） | `src/mobilebar/tabs.js` 面板 `.w2m-hint` |
| 12 | P2 | 软键盘弹出时底栏让位（visualViewport） | ✅ 已实施（2026-11-14） | `src/mobilebar/bottom_space.js::installKeyboardAvoid` |
| 13 | P2 | 暗色启动底色（原生主题 `windowBackground` 随系统日夜） | ✅ 已实施（2026-11-14） | `patch-android-splash-theme.py`（纯资源；Rust 侧一行未改） |
| 14 | P2 | DNS TXT 解析结果持久化（冷启动秒开 + 后台静默更新） | ❌ 不做（2026-11-14 决策，理由见 §5） | — |
| 15 | P2 | 代理模式可达性探测穿到真实上游（而非只探 127.0.0.1） | ✅ 已实施（2026-11-14） | `bootstrap.rs::probe_target` + `probe_auth_message` |
| 16 | P2 | 平板 / 横屏底栏限宽居中 | ✅ 已实施（2026-11-14） | `src/mobilebar/core.js` 宽屏断点 |
| 17 | P2 | 移动页签记录跨启动持久化 | ❌ 不做（2026-11-14 决策，理由见 §5） | — |
| 18 | 兼容 | 明文流量放行：本地代理与 http 站点直连均可用（networkSecurityConfig） | ✅ 已实施（2026-11-14；收紧后当日回退） | `patch-android-cleartext.py`（替代 workflow 内 sed） |
| 19 | 分发 | 增加 x86_64 / universal APK；评估 AAB | ❌ 不做（2026-11-14 决策，理由见 §5） | — |
| 20 | 流程 | 真机能力验收清单入回归（见 §4） | 进行中（本文件 §4 即清单） | 每次打包后人工执行 |

状态取值：待办 / 进行中 / ✅ 已实施（日期）/ ❌ 不做（日期 + 理由见 §5）。

---

## 2. 已实施项的验收要点（真机）

### #1 文件下载接管

- 证据：wry 0.57 的 `RustWebViewClient.kt` / `android/mod.rs` 未挂 `DownloadListener`，
  Rust 层 `download_started_handler` 在 Android attributes 解构中被丢弃；
  Windows 侧的下载接管（`save_dialog.rs` / `webview_hooks.rs`）在 Android 无对应物。
- 方案：`onWebViewCreate` 里 `setDownloadListener` → 系统 `DownloadManager`
  落公共 Download 目录（Cookie / UA 头透传、文件名取 `URLUtil.guessFileName`），
  API ≤ 28 运行时申请 `WRITE_EXTERNAL_STORAGE`（Manifest `maxSdkVersion=28`），
  API 33+ 申请 `POST_NOTIFICATIONS`（不阻断下载本身）。
- 验收：①普通附件下载，通知栏有进度、Download 目录有文件；②需登录站点的下载带对
  Cookie 成功；③长按网页图片「保存图片」成功；④API ≤ 28 首次下载弹存储权限，拒绝时
  有 toast、不崩溃；⑤`blob:` 链接给明确 toast 而非静默。

### #2 外部 scheme 外跳

- 证据：wry 默认在 WebView 内 load 所有 scheme，非 http(s) 落 `ERR_UNKNOWN_URL_SCHEME` 白页。
- 方案：补丁 `RustWebViewClient.shouldOverrideUrlLoading`，
  http/https/about/data/blob/javascript/file/content 放行，其余用
  `Intent(ACTION_VIEW)` 外跳；`intent://` 走 `Intent.parseUri(..., URI_INTENT_SCHEME)`
  并支持 `browser_fallback_url`；`market://` 无应用市场时回落 Play Store 网页；
  无应用可处理时 toast 提示并**吞掉**该次导航（不留错误白页）。
- 验收：`tel:` / `mailto:` / `weixin://` / `alipays://` 能拉起对应应用；
  未安装微信时点微信支付链接有 toast、不出现 ERR_UNKNOWN_URL_SCHEME；
  正常 http(s) 跳转、splash 启动页、页面内 iframe 不受影响。

### #3 直连内嵌凭据在 Android 打包期硬失败

- 证据：`bootstrap.rs` 的 `#[cfg(not(windows))]` 导航分支不做任何鉴权注入
  （`nav.user/nav.pass` 仅 `let _ =` 压掉）；wry 0.57 `RustWebChromeClient`
  未覆写 `onReceivedHttpAuthRequest`，默认取消 ⇒ Basic Auth 站点在 Android 直连必 401。
  鉴权的唯一跨端机制是本地反向代理（`proxy_enabled=true`）。
- 方案：解析器新增 `--target android`（仅 CI android job 传）：
  - 单 URL **含 `user:pass@` 且 `proxy_enabled=false`** → 硬失败，文案给出两种修法；
  - 域名数组 + 关代理 → **不输出任何东西**（打包期解不出真实地址，无条件告警只会在每个
    默认组合的 Android 构建里刷噪声；2026-11-14 起，理由见 `docs/packaging-config.md` §5）。
  - Node 复刻侧加同名纯函数与自测，`--parity` 覆盖无 target 主路径。
- 验收：CI android job 对该组合直接红；Windows / Electron 打包不受影响；
  `parse-packaging-config.py --selftest` 与 `pack-local.mjs --selftest` 全绿。

### #4 / #8 清除浏览数据 + 底栏「更多」面板（P1）

- 证据与路径修正（取证结论，勿走回头路）：
  - tauri 在 Android 的 `PlatformWebview` **只暴露 `jni_handle()`**，没有 Windows 侧的
    controller / 清数据入口 ⇒ 原计划的 `web_data.rs` 加 `#[cfg(target_os = "android")]`
    分支走不通；Rust 侧本批**零改动**。
  - wry 0.57 自带的 Kotlin `WebView.clearAllBrowsingData()`（经 JNI / main-pipe 暴露）
    **配方不全**：只做 `deleteDatabase` + `clearCache` + `clearHistory` + `clearFormData`，
    **漏清 Cookie 与 HTML5 存储（`WebStorage`）** ⇒ 直接调它登录态不会掉。
- 方案（`patch-android-extras.py`，一份桥同时承载 #4 与 #8 的原生四动作）：
  - `MainActivity.web2appSetupExtras()` 在 `onWebViewCreate`（download 调用之后）里
    `addJavascriptInterface(object { … }, "__web2appAndroid")`，四个 `@JavascriptInterface`
    方法：`share(url,title)`（ACTION_SEND 系统分享面板）、`openBrowser(url)`
    （ACTION_VIEW，仅 http/https，无浏览器 toast）、`clearData()`、`quitApp()`
    （`finishAndRemoveTask()` + 300ms 后 `killProcess`）；方法体一律 `webView.post{}`
    回 UI 线程（接口回调在 binder 线程）。
  - `clearData()` 完整配方：`clearCache/clearHistory/clearFormData/deleteDatabase`
    + `WebStorage.getInstance().deleteAllData()` + `CookieManager.removeAllCookies{ flush() }`，
    Cookie 回调里 `evaluateJavascript("location.reload()")` —— 清完立即重载，用户直接看到登录态失效。
  - **安全模型（防第三方 iframe 冒调危险动作）**：文件尾注入顶层 `object Web2appBridge`
    （`@Volatile key` + `rotate()` = 新 UUID）；补丁 wry `RustWebViewClient.onPageFinished`
    老签名（只可能是主框架回调），方法体首行轮换密钥并经 `evaluateJavascript` 注入
    `window.__web2appAndroidKey='<uuid>'`。evaluateJavascript 只作用于主框架，
    跨源 iframe 读不到顶层变量；每个桥方法第一参 `key`，`web2appBridgeOk` 不符即静默 return。
- 页面侧（`src/mobilebar/more.js`，Tauri 移动端专属）：底栏**无条件**新增第 5 格「更多」
  （顺序 home → back → fwd → [list] → more），面板从底栏上沿升起（复用 w2m-sheet 样式语言），
  六行：刷新 / 复制链接（Clipboard API + execCommand 兜底）/ 分享 / 在浏览器打开 /
  **清除浏览数据（两步确认，3s 自动撤防）** / 退出应用；toast 浮层做反馈；
  调桥前判 `window.top === window.self` 与密钥存在，桥缺失 toast「当前版本不支持该操作」。
- 验收（§4 第 12 项细化）：
  ①面板六行齐全、顺序与文案一致；②复制链接在真实站点成功且有 toast；
  ③分享弹出系统分享面板、URL/标题正确；④浏览器打开拉起系统浏览器且仅 http(s) 可走；
  ⑤需要登录的站点：清数据 → 自动 reload → 登录态确实消失（Cookie + localStorage 都没了），
  首次点击只进入「再点一次确认清除」红字态，3s 不操作自动复原；
  ⑥退出后面板/进程不在最近任务残留；⑦第三方页面内嵌的跨源 iframe 调
  `__web2appAndroid.clearData(任意值)` 必须无效（无密钥）；⑧不打包 extras 的旧包上点原生动作
  只出 toast，不崩不静默。

### #5 相机 / 麦克风 / 定位权限声明（P1）

- 证据：wry 0.57 的授权策略是「Manifest 没声明就直接 deny」—— H5 `getUserMedia`、
  `navigator.geolocation` 不弹任何系统授权框直接失败；而全量声明又会让 Play 审核要求
  解释每项用途，不能一刀切。
- 方案（按需声明，默认零权限）：
  - 打包配置新增第 **21** 个字段 `android_permissions`（字符串数组，白名单三值
    `camera` / `microphone` / `location`；缺省 `[]`；去重保序；非法元素打包期硬失败）。
    Python 解析器与 Node 复刻双侧同语义，`--parity` 覆盖成功 4 例 / 失败 7 例；
    字段**非敏感**，会写进 Electron `config.json`（Electron 壳接受但忽略，形态校验照做）。
  - 新增 `patch-android-permissions.py`：读 `--permissions` 或环境变量
    `ANDROID_PERMISSIONS`（紧凑 JSON；空数组打印跳过、return 0），在 `</manifest>` 前
    插入带 marker 的 `<uses-permission>` 块。固定映射：
    `camera` → `CAMERA`；`microphone` → `RECORD_AUDIO`；
    `location` → `ACCESS_FINE_LOCATION` + `ACCESS_COARSE_LOCATION`。
    marker 幂等、缺 `</manifest>` 显式 raise。
  - 两个 workflow（`build-custom.yml` 手工输入 / `release.yml` repository variable
    `ANDROID_PERMISSIONS`，缺省 `'[]'`）在 fullscreen 步之后调用；两份入库 example
    均带 `"android_permissions": []`；两处内联 cfg 的契约键断言同步扩到 21 键。
- 验收：①配置 `["camera","microphone","location"]` 出包后，网页首次调摄像头 / 麦克风 /
  定位弹系统授权框，拒绝后 H5 拿到的是权限错误而非静默失败；②配置为空（默认）的包
  Manifest 不含这四条权限，系统设置里看不到对应授权项；③非法值（如 `"gps"`）打包期红。

### #6 网页视频全屏（P1）

- 证据：wry 0.57 的 `RustWebChromeClient.kt` 里 `onShowCustomView` 的实现是进来就
  `callback.onCustomViewHidden()` + super —— 等于立刻通知 Chromium「已退出全屏」，
  H5 `<video>` / `requestFullscreen()` 的全屏按钮点了没反应。
- 方案（`patch-android-fullscreen.py`，**整体替换** onShow + onHide 两个方法；
  只改首行不行，残留的立即取消回调仍会撤销全屏）：
  - onShow：重复全屏请求直接拒绝并回调；`android.R.id.content` 下第 0 个子 view
    （webview）置 GONE，新建 `FrameLayout` 以 MATCH_PARENT 承载 custom view；
    朝向锁 `SCREEN_ORIENTATION_SENSOR_LANDSCAPE`（用户横/竖拿都跟随传感器，不硬钉正向横屏）；
    进沉浸式。
  - onHide：移除承载容器、webView 恢复 VISIBLE、还原保存的朝向、退沉浸式、
    **最后**才 `callback?.onCustomViewHidden()` 通知 Chromium；任一步缺对象都安全降级。
  - 沉浸：API 30+ 走 `window.insetsController` + `WindowInsets.Type.systemBars()` +
    `BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE`；低版本走已废弃的 `systemUiVisibility`
    标志位（`@Suppress("DEPRECATION")`），minSdk 24 两套都要。
  - 漂移守卫：正则定位后要求 onShow 体内必须含 `onCustomViewHidden()`（即仍是 wry 0.57
    的空实现），否则显式报错「上游已实现真全屏」，不静默替换；新增四条 import 后
    **重算** regex 下标再做区间替换；marker 幂等。
- 验收：①横屏 H5 视频点全屏按钮：画面铺满、系统栏隐藏、横屏；②退出（视频控件 /
  系统返回 / Esc 等价操作）后还原竖屏与 webview，连续进出不黑屏不残影；
  ③`requestFullscreen()` 的非视频元素同样可全屏；④全屏下来电 / 切后台回来状态正常。

### #7 `target=_blank` / `window.open` 进移动页签（P1）

- 证据：wry 多窗口策略下页面 `target="_blank"` 链接与 `window.open()` 默认不产生新窗口，
  表现为点击无反应或被丢在原页；而移动底栏已有页签记录（#8 同批落地的单 webview 多页签），
  正好承接「打开新页」。
- 方案（纯注入层 + Rust 新事件，不动 Kotlin）：
  - `src/mobilebar/tabs.js` 新增 `installBlankGuard()`，仅 `HAS_TABS`（多页签开启）且非内置页时
    在 document-start 安装：①document **捕获阶段** click 监听，从点击目标向上找最近
    `<a>`，命中 `target="_blank"`（词边界匹配，防 `x_blank` 误伤）且 href 是 http(s)
    则 preventDefault + stopPropagation，发 `web2app:open-tab` 事件 `{url}`；
    ②包 `window.open`：http(s) URL 改发同一事件并返回 `null`（页面侧新窗口引用本就不可用），
    非 http(s) 回落原 `window.open`（交给 #2 schemes 链路）。
  - `mobilebar.rs` 新增事件常量 `OPEN_TAB_EVENT = "web2app:open-tab"`（与无 payload、
    落首页的 `web2app:new-tab` 区分）：`open_tab_url_of` 解 payload `{url}`，
    `create_tab_with_url` 复用现有页签记录（trim、空 / 非法 URL / 非 http(s) 一律记日志
    放弃——tel/mailto 等留给 schemes 补丁），绑定 current_url 后 `new_tab(url)` 并导航、
    刷新底栏列表。
  - 非页签模式（`tabs_enabled=false`）不装 guard，行为与旧版一致。
- 验收：①多页签包内点 `target="_blank"` 外链：底栏新增页签并落在该 URL，原页签保留；
  ②页面脚本 `window.open('https://…')` 同样开新页签；③`_self` / `target` 缺省链接
  仍在当前页签导航，不被拦截；④`window.open('tel:10086')` 走外跳而非吞掉；
  ⑤单页签包（tabs_enabled=false）不拦截、不报错。

### P2 批次（2026-11-14，本机可验证部分已闭环，真机项见 §4）

先说明判定口径：本机**没有 Android SDK/NDK**，这批改动只做到「静态契约 + 纯逻辑单测 +
补丁链按序试打」；凡需真机才能判定的行为一律列进 §4，不拿「测试绿」当「功能已验」。

**#9 返回键优先关浮层 + #10 再按一次退出**（同一脚本 `patch-android-back.py`）

- 形态：`handleOnBackPressed` 先 `evaluateJavascript` 问页面
  `window.__web2appMobileBar.closeSheet()`，返回 `1` 即消费本次返回；否则进 `web2appBack()`
  （真实历史 → `goBack()`；没有 → 2s 窗口内双按才 `finish()`，首次只 Toast「再按一次退出」）。
- 两个易回退点：① eval 串必须返回**数字**（布尔/字符串会被回调再包一层引号，`r == "1"`
  永远不成立）；② 浮层判定必须在后退**之前**（反了 = 面板开着时按返回直接离开页面）。
- 本机取证：`--selftest` 7 组 + 3 条反向对照（数字返回值 / 顺序 / 双按窗口各自改坏均 FAIL）；
  契约测试 `back_key_closes_sheet_before_navigating`、`back_key_requires_double_press_to_exit`；
  补丁链实测 back → download → extras 共用**唯一** `onWebViewCreate` 且叠加正确。
- 真机验收：§4 第 7 项。

**#11 页签切换 = 重新加载（只做文案）**

- 面板「新建标签」下方固定一行「切换标签会重新加载页面（移动端同时只保留一个页面）」。
- 不做状态快照：单 webview 下要序列化 `history.state` + 表单 + 滚动位置，收益与风险不对等
  （§5 记为「不建议做」）。
- 真机验收：§4 第 6 项顺带看一眼。

**#12 软键盘让位**（`bottom_space.js::installKeyboardAvoid`）

- 判据是 `visualViewport` 与 `innerHeight` 差值 ≥ 60px（地址栏伸缩远小于此）；`adjustResize`
  模式下 layout viewport 本身变矮、差值为 0，天然无操作 —— 不需要按 `windowSoftInputMode` 分支。
- 抬升只改底栏 inline `bottom`，**不动** body 的底部让位（键盘覆盖的是 layout 视口下沿，
  那是页面滚动的事）。
- 真机验收：§4 第 8 项。

**#15 代理模式探测真实上游**（`bootstrap.rs::probe_target`）

- 旧实现探 `nav.url`，代理模式下是 `127.0.0.1:<port>` ⇒ 只能证明本地代理活着。现在：代理 +
  上游 origin 已知 → 探上游并标记 `via_proxy`；否则退回导航地址。
- 401/403 在代理模式下单独成类（`probe_auth_message`）：「上游可达但鉴权未通过」——
  不能再把用户引去查 WebView 内核。
- 本机取证：`probe_target_prefers_upstream_origin_when_proxied`（4 例）、
  `probe_auth_message_separates_unauthorized_from_render_failure`、`probe_ok_message` 的
  via_proxy 变体。
- 真机验收：§4 第 11 项（失败卡指向真实上游主机名；密码错时给 401 结论）。

**#16 宽屏底栏限宽居中**（`core.js` 的 `@media (min-width:600px)`）

- 底栏与面板 `max-width:560px` + `left:50%` + `translateX(-50%)`。`syncBarWidth` 仍钉
  `innerWidth`，max-width 会把它收在上限内 —— 因此**不用改 JS**（少一处状态就少一类不同步）。
- 真机验收：§4 第 9 项（平板 / 横屏下底栏居中不拉满，面板与底栏同宽）。

**#18 明文流量放行**（`patch-android-cleartext.py`）

- 生成 `res/xml/network_security_config.xml`：`base-config cleartextTrafficPermitted="true"`
  （**全放行**）；Manifest 的 `android:usesCleartextTraffic="…"` 换成
  `android:networkSecurityConfig="@xml/…"`。
- **为什么不直接用 `usesCleartextTraffic="true"`**：只为把策略收成**单一落点** —— 两个 workflow
  共用同一份脚本、不再各写一份 sed，日后按域收紧也只改一处常量。
- **曾收紧、当日回退**：首版为「仅 `127.0.0.1` / `localhost`」，导致直连 `http://` 目标站点被系统
  拦下（`ERR_CLEARTEXT_NOT_PERMITTED`），而目标站点大量只有 http ⇒ 产品侧判定不可接受，改回全放行。
  ⚠️ **若日后重新收紧，`domain-config` 必须保留回环例外**，否则本地代理链路先断。
- 本机取证：`--selftest` 9 组（含「两属性并存只留一个」「自闭合 application」「缺标签报错」），
  并含一条反例断言（XML 本体出现 `cleartextTrafficPermitted="false"` 即失败）；契约测试
  `cleartext_http_stays_allowed` 只取 `<network-security-config>` 本体判定，避免脚本注释 / 自测
  字面量把用例顶成假红（该用例已做反向对照：改回 `false` 必 FAILED）。
- 真机验收：§4 第 11 项 + 直接输入 `http://` 外站应能正常打开（不得出现 `ERR_CLEARTEXT_NOT_PERMITTED`）。

**#13 暗色启动底色**（`patch-android-splash-theme.py`，纯资源）

- 写 `res/values/web2app_splash.xml`（`#f7f8fa`）与 `res/values-night/web2app_splash.xml`
  （`#14181f`）；再往 Manifest 引用的主题（`@style/...`）里插
  `<item name="android:windowBackground">@color/web2app_splash</item>`，已有该项则只换值。
- ⚠️ **Rust 侧一行未改**：tao 0.37.1 的 `background_color` 在 Android 是空实现（文档亦写明
  Unsupported）⇒ `nav::SPLASH_BG` 那几处调用在安卓上从未生效，闪的是主题底色。改常量属于
  打空靶，所以这一项只剩资源补丁。
- 本机取证：`--selftest` 8 组（插入 / 替换已有值 / 幂等 / 空 style 缩进 / 缺同名 style 报错 /
  框架主题报错 / application 与 activity 两种主题位置 / 文件级集成）；契约测试
  `splash_theme_colors_match_splash_html` 跨文件比色值（已做反向对照：改暗色值 → FAILED，
  还原后 sha256 一致）；`tests/android-template-rehearsal/rehearse.py` 按真模板顺序重放整条链。
- 真机验收：§4 第 14 项（暗色系统下冷启动不得先闪白）。

### P0 / P1 落地后的静态验证记录（2026-10-14，真机验收待出包）

- 补丁脚本在临时目录对**真实 wry 0.57 / tauri-cli 2.12.2 模板**按序试打
  （insets → back → download → extras → schemes → fullscreen → permissions），
  **七个**脚本各跑两遍全部通过且幂等；extras 后 `onWebViewCreate` 内调用顺序
  super → downloads → extras，顶层 `object Web2appBridge` 确认在 class 之外；
  fullscreen 后 onShow/onHide 的「立即取消」stub 已整体替换；全权限时 Manifest 同时含
  download（maxSdk=28 / POST_NOTIFICATIONS）与 permissions 两个独立 marker 块；
  模板方法签名漂移时补丁显式报错而非静默跳过。
- `config-contract` 门禁全绿：**7** 个安卓补丁脚本 selftest（download / extras / schemes
  / fullscreen / permissions 等）、解析器 selftest、`pack-local.mjs --selftest`、
  `check-workflows.py`（7 workflow / 107 脚本片段 / 13 个 .py）；Rust 侧契约测试新增
  3 条本批断言（`android_permissions` 全真源 / fullscreen 接线 / blank guard 双侧要素），
  `cargo test --workspace` **205** 测试全绿、rustfmt 干净；Electron 侧
  `pnpm run unit` **101** 全绿（含 `toAndroidPermissions` 新用例）；parity 55/55。
- `--target android` CLI 端到端四组合验证：内嵌凭据+直连硬失败（URL 经 mask 脱敏）、
  内嵌凭据+代理通过、无 target（Windows 路径）不受影响、域名数组+直连**零注解**（2026-11-14 起）。
- 真机行为以上面各条「验收」为准，列入 §4 第 1、2、4、5、6、12 项。

---

## 3. P1 / P2 待办的方案要点（实施时展开）

- **#4 + #8 已实施（2026-10-14，见 §2 同名小节）**。实施中两处推翻原假设，后续不要回退：
  ① Android 清数据**不经过** `web_data.rs`（tauri Android `PlatformWebview` 无对应入口），
  走 Kotlin JS 桥；② wry 0.57 的 `clearAllBrowsingData()` **不能直接用**（漏 Cookie/WebStorage），
  桥内自行补全配方。后续若升级 wry，先验证该方法配方是否补齐，再考虑收敛。
- **#5/#6/#7 已实施（2026-10-14，见 §2 同名小节）**。三处实施时的关键决策，勿回退：
  ① #5 权限按需声明，`android_permissions` 是非敏感字段（进 Electron config.json，
  Electron 壳接受但忽略），默认 `[]` 零权限；② #6 必须**整体替换** onShow+onHide
  两方法且加 import 后重算匹配下标，模板已自带真全屏时补丁显式报错；
  ③ #7 用**新事件** `web2app:open-tab`（带 `{url}`）而非复用无 payload 落首页的
  `web2app:new-tab`，Rust 与 JS 双侧只承接 http(s)，非页签模式不装 guard。
- **#14/#17 持久化**：❌ **不做**（2026-11-14 决策，理由见 §5）。两者都要往 settings.json 加段落，
  且无法用「少写点代码」换掉它们的固有代价：#14 是「为省几秒启动而引入一份本机地址缓存」，
  #17 是「为记住几个临时页签而明文存 URL 并承担失效白屏」。收益与代价不匹配，保持现状。
- **#18 明文流量**：✅ 已实施（2026-11-14）—— 见 §2.7。原计划「`res/xml` + Manifest 引用 + 删 sed」
  已落地（sed 换成独立补丁脚本 `patch-android-cleartext.py`）；策略由首版「仅回环」**回退为全放行**。

## 4. 真机能力验收清单（#20，每次出包人工回归）

wry 升级会改变下列行为（项目已有升级后行为漂移的先例），不能只靠静态/单测：

1. 普通下载、登录态下载、长按保存图片；
2. `tel:` / `mailto:` / `weixin://` / `alipays://` / `intent://` 唤起与无应用兜底；
3. 拍照上传（`<input type=file capture>`）、普通文件上传；
4. 相机 / 麦克风 / 定位授权弹窗（#5，对照打包配置 `android_permissions` 的取值逐项验，
   默认空数组的包系统设置里不应出现这三项授权）；
5. H5 视频全屏与退出（#6，横屏铺满 + 沉浸 + 退出还原朝向，连续进出不黑屏）；
6. `target=_blank` 弹窗型登录 / 预览（#7，多页签包开新页签；单页签包不拦截）；
7. 系统返回键：页面内跳转后退、浮层关闭（#9）、再按退出（#10）、绝不退回 splash；
8. 软键盘弹起 / 收起时底栏位置（#12）；
9. 旋转屏 / 折叠屏 / 平板横屏底栏布局（#16）；
10. 弱网：首启 25s 预算内失败卡文案与重试；杀进程冷启动仍在预算内解析（#14 不做缓存，行为同前）；
11. 代理模式：带鉴权站点经 `127.0.0.1` 加载成功；失败卡诊断指向上游真实结论（#15）；
12. 「更多」面板（#4/#8，逐条见 §2 验收 ①–⑧）：六行齐全；复制链接成功有 toast；
    分享 / 浏览器打开拉起正确；清数据两步确认 + 自动 reload 后 Cookie 与登录态消失需重新登录；
    退出无最近任务残留；跨源 iframe 无密钥调桥无效；旧包（无桥）点原生动作只 toast 不崩；
13. 明文直连（#18）：直接打开 `http://` 目标站点能正常渲染，**不得**出现
    `ERR_CLEARTEXT_NOT_PERMITTED`；同一次运行里代理模式（`127.0.0.1`）仍可用；
14. 启动底色（#13）：系统切深色后冷启动，**首帧就应是深色**（`#14181f`），不得先闪一下浅色；
    切回浅色同样（`#f7f8fa`）；启动页自身颜色与之一致（无撕裂边界）。

---

## 5. 结项记录：#14 / #17 / #19 与 #11 快照均定为不做（2026-11-14）

这几项已全部**结项为不做**（#13 已于同日实施，见 §2）。本节不再是「待决策」，而是**决策留档**：
保留方案与取舍，供以后有人重提时直接看「为什么不做」，不必再论证一遍。

判据统一为一条：**收益不足以支付它引入的复杂度或语义代价**。#14 是「为省几秒启动，换来一份
需要失效回退与可见性说明的本机地址缓存」；#17 是「为记住几个临时页签，换来明文 URL 落盘 +
点开白屏」；#19 是「为解决不存在的分发问题，换来体积或产物互撞」。三者都不是技术障碍。

**一处仍然有效的事实纠正**（针对上一版基于推测的判断）：#13 **不需要动 Rust** —— tao 0.37.1 的
`WindowBuilder::with_background_color` 文档写明「iOS / Android: Unsupported」，Android 侧
`platform_impl/android/mod.rs:874` 是**空实现** ⇒ 桌面那几处 `.background_color(nav::SPLASH_BG)`
在 Android 上完全无效，闪的是 Activity 主题的 `windowBackground`。这条也是 #13 最终走「纯资源
补丁」的原因。


### #13 暗色启动底色（Android 原生层）— ✅ 已实施（2026-11-14）

实现与取证见 §2「#13 暗色启动底色」（`patch-android-splash-theme.py`，纯资源、Rust 未改）。
下面保留当时的决策过程（A 采纳、B 出局），供以后改口径时查依据：

**现状（已核实）**：`splash.html:39/43` 与 `loading.js` 都已随 `prefers-color-scheme` 在
`#f7f8fa` / `#14181f` 之间切换。首帧之前那一层在 Android 上**与 `nav::SPLASH_BG` 无关**
（见 §5 开头的事实纠正），闪的是 **Activity 主题的 `windowBackground`**（模板默认浅色）。

**A（推荐）纯资源补丁**：新脚本 `patch-android-splash-theme.py`
- 写 `res/values/web2app_splash.xml`（`#f7f8fa`）与 `res/values-night/web2app_splash.xml`
  （`#14181f`）两个颜色资源；从 Manifest 取 `android:theme="@style/X"`，在
  `themes.xml` 的 `<style name="X">` 内插 `<item name="android:windowBackground">
  @color/web2app_splash</item>`（已有该项则改写其值）。
- 优：**零 Rust 改动**（桌面与测试构建完全不受影响）；日夜切换由系统资源机制完成，
  不存在「判断当前主题」的时机问题；色值与 Web 层共用一套，可静态对齐并钉进契约测试。
- 劣：只能靠 CI / 真机看最终效果（本机 `gen/` 是空的，没有可读的 tauri 模板）；
  模板若改 `<style>` 形态，脚本要跟 —— 所以「找不到主题/样式」必须**显式失败**而不是静默跳过。
- 残留待真机确认：WebView 自己首帧是否还会先铺一层白（wry 侧有
  `webview_attributes.background_color`，`tauri-runtime-wry:4779`）—— 若会，再补一行。
- 工作量 **S**（1 个脚本 + 2 个 workflow 各一行 + 1 条契约测试把色值与 `splash.html` 逐字比对）。

**B 改 `SPLASH_BG` 常量**：Android 上无效（纠正 1），改了也不会变；反而可能让桌面亮色用户
多一次深闪。**放弃**（上一版把它当「用一个缺陷换另一个」不够狠 —— 它压根没碰到问题）。


### #14 DNS TXT 解析结果缓存 — ❌ 不做（2026-11-14 决策）

**结论**：不做。启动期解析保持现状（每次冷启动都走 DNS + 验签，预算 25s）。

**为什么不做**（按当下产品形态判断）：

1. **它解决的不是「坏」而是「慢一点」**。正常网络下解析 <1s，与加入缓存后的差距只有几秒；
   缓存真正生效的场景只有弱网/断网，而这类用户本次能进、下次地址可能已过期。
2. **代价是一份要长期维护的本机旁路**：落 settings.json、失效回退、换域名/换公钥后的作废、
   启动页状态行、Electron 侧透传——每一项都要测试与真机验证，换来的只是「省几秒」。
3. **与 #17 是同一笔账**：两者都要往唯一运行期 JSON 里加段落，都只服务一次冷启动的体验，
   而项目当前没有「冷启动秒开」这条需求（真机包启动即用，入口由 DNS 下发本身就有一次网络往返）。

**若日后重开，前置条件**：出现明确的「弱网/断网可用」诉求（如外勤、地下场景），且接受
「地址缓存有效期严格取签名 `sign_day + ttl_days`」——v4 载荷自带这两个字段
（`resolve-core/src/lib.rs:19`：`URL || 0x00 || 签发日(2B) || ttl_days(2B)`，
`DecryptedRecord{sign_day, ttl_days}` 已导出，`check_ttl()` 与运行期同一判定），
因此**不存在「越过下发方授权窗口」的问题**，安全上不需要额外决策。

**当时的候选与取舍**（留档）

**现状（已核实）**：每次冷启动都要 DNS + 解密验签，总预算 `RESOLVE_BUDGET_SECS = 25`
（`bootstrap.rs:281`），弱网下这段是纯粹的空等。入口 `resolve_url_from_domains` 只吐
`String`（`resolve.rs:110`）；要拿 TTL 还得加一个返回 `DecryptedRecord` 的兄弟函数
（内部把 `decrypt_txt` 换成 `decrypt_record` 即可，`resolve-core` 已导出）。

**A（推荐）网络优先 + 短预算回退**
- 预算 25s → **4s**：先照常解析；超时或失败才用缓存（缓存必须未过签名 TTL）；成功后刷新缓存。
- 优：**URL 只在导航前决定一次**，不存在「后台刷出新地址后要不要重导航」这个死结
  （重导航会丢表单/打断操作，不重导航则缓存与远端脱节）；正常网络与今天等价（<1s）；
  弱网最坏 4s 而不是 25s；断网也能进（受 TTL 约束）。
- 劣：仍比纯缓存慢 0–4s；要加一段持久化 + 一条启动页状态行。
- 缓存字段建议 `{url, sign_day, ttl_days, domains_hash, key_hash}`（`domains_hash` /
  `key_hash` 用来在换域名或换公钥后**自动作废**旧缓存，否则会拿旧授权开新配置）。
- 工作量 **M**。

**B 缓存优先 + 后台静默刷**：0.1s 出画面。
- 劣：刷出**不同** URL 时无从下手（见 A 的优），只能「本次不换、下次启动生效」——
  于是缓存与远端最长脱节一个会话，且要在设置页解释这件事。工作量 **M+**，收益只差几秒。

**C 不做**：零风险，弱网首启仍是 25s 空等 —— **本项采纳 C**。

**D 只存 URL、不设有效期**：**不可取** —— 会越过签名 TTL，等于给本机开一条不受下发方
约束的旁路。

**当时的建议是 A（未采纳）**：A 的唯一「需拍板点」是「是否允许断网也能进应用」。产品侧
判断该能力不必要，故连同 A 一并放弃。

**跨壳注意**（仅重开时相关）：新段落要落 `settings.json`（仓库唯一运行期 JSON 契约，不新开文件）。
Electron 侧 `settings.ts` 只对 `window` / `float_window` 做原样透传（`LoadedDocument` 注释，
`settings.ts:212-224`）⇒ 两壳若共用同一数据目录，Electron 保存会把新段落丢掉。要么给它的
透传加一行，要么明确「两壳目录不同、无需透传」。


### #17 移动页签记录跨启动持久化 — ❌ 不做（2026-11-14 决策）

**结论**：不做。移动页签继续是纯内存状态，重启后从首页重新开始。

**为什么不做**：

1. **它恢复的不是「状态」，只是「一串地址」**。单 webview 模型下移动页签本就是临时记录，
   点开即重新加载；恢复出来后用户会以为「后台还在跑」，比不恢复更误导（与 #11「切换会重新
   加载」的文案预期直接冲突）。
2. **要付明文落盘的账**：记录里的 `url` 可能带一次性 token，只能写进 settings.json（与
   Cookie 同处），还要在「清除数据」里连带清、在 `tabs_enabled=false` 的构建里清掉旧记录 ——
   为一个重启不丢列表的小便利，引入一个需要长期维护的敏感数据落点。
3. **失效白屏**：地址可能早已过期，点开是白屏，还得再补一套错误态。收益是「列表还在」，
   代价是三条边界 + 一个易漏的清理路径。

**若日后重开，前置条件**：必须先有「重启后接着看」的真实诉求，且明确接受「非首页 URL 明文
落盘 + 清数据连带清」；实现上只允许存非首页、最多 10 条、不自动加载。

**连带结项**：**#11 后半的「切换快照」同判据一并定为不做**（状态表已同步）。快照要保留的是
每个页签的 DOM/表单/滚动状态，成本远高于「记住地址」，而移动端页签的使用方式就是「点开看一
眼就走」——为它维持多份活动状态不划算。文案明示（已实施）足够消除误解。

**当时的候选与取舍**（留档）

**现状（已核实）**：`MobileTabs { tabs: Vec<MTab>, active, seq }` 是**纯内存**状态
（`mobilebar.rs:169`），bootstrap 解析成功即 `reset_home`。移动端不存在「后台还活着的页面」，
记录里的 `url` 是切换时**重新加载**的地址。

**A（推荐）只持久化非首页记录**：最多 10 条 `{label, url, title}`；启动后 `active` 固定 0、
**不自动加载**；`seq` 取已有最大号 +1 续；首页 url 每次用新解析结果覆盖（不存旧首页地址）。
- 优：语义诚实，与 #11 的「切换会重新加载」文案一致；冷启动不被拖慢（与 #14 不打架）。
- 劣：恢复出来的 url 可能早已失效，点开是白屏 —— 需要沿用现有失败卡/错误页，不能白屏无提示。
- 两个必须一起处理的点：① `url` 可能带一次性 token ⇒ 明文落盘（不进日志、且**清数据要连带清**，
  `web_data.rs` 现在只清 WebView 数据）；② `tabs_enabled=false` 的构建要忽略并**清掉**旧记录。
- 工作量 **S**（存/读两处 + 3 条纯逻辑单测）。

**B 全量恢复 + 自动加载上次 active**：最像桌面习惯。
- 劣：单 webview 模型下「恢复」其实是**重载**，用户会以为状态还在（比不恢复更误导）；
  且把冷启动变成「等上次那个页面加载完」，与 #14 的收益互相抵消。

**C 不做**：零风险；移动端页签本来就只是临时记录，代价是每次重启从零开始 —— **本项采纳 C**。

**当时的建议是 A（未采纳）**：A 的「需拍板点」是「是否接受历史 URL 明文写盘」。产品侧判断
该能力不必要，故连同 A 一并放弃；B 本就因「恢复 = 重载、比不恢复更误导」被排除。


### #19 x86_64 / universal APK 与 AAB — ❌ 不做（2026-11-14 决策）

**结论**：不做。Release 继续只发 aarch64 单 ABI APK。

**现状（已核实）**：只装 `aarch64-linux-android` 一个 target（`build-custom.yml:342`），
`cargo tauri android build --target aarch64 --apk`（:626）；APK 名由
`set-android-apk-name.py` 按 `app_name` 统一（`archivesName`）。

**A 加 x86_64（只为模拟器）**：只应存在于 **CI 临时构建**，不进 Release。
- 代价：第二套 native 库；Rust 依赖**没有跨 target 增量**，Android job 的编译段近似翻倍。
- 陷阱：两个 ABI 的产物**同名**（`archivesName` 相同、分在不同 abi 目录）⇒ upload-artifact
  里会互相覆盖，必须给文件名加 ABI 后缀。

**B universal（arm64 + armv7 + x86_64）**：一个包通吃。
- 劣：每个用户都多带两套用不到的 `.so`（native 体积近似 ×2）；armv7 只覆盖很老的机型，
  本项目没有「老设备」诉求。与「绿色小包」定位直接冲突。

**C AAB**：本项目靠 GitHub Release 直发 APK，AAB **不能直接安装**（要经 Play 拆分下发）。
只有「上架 Play」才成立，且要额外维护 upload key —— 现在做等于多一个用不上的产物。

**为什么不做**：判据很简单 —— 三个方案解决的都不是现存问题（真机全是 aarch64）。
若某天需要模拟器回归，按 A 的「CI-only + 文件名加 ABI 后缀」形态加，Release 仍只发 aarch64。

### 汇总

| 项 | 结论 | 工作量 | 备注 |
| --- | --- | --- | --- |
| #13 暗色启动底色 | ✅ **已实施**：纯资源补丁（`values` + `values-night` 颜色 + 主题 `windowBackground`） | 已完成 | Rust 一行未改（Android 上 `background_color` 是空实现） |
| #14 解析结果缓存 | ❌ **不做** | — | 只省几秒启动，换一份需失效回退 + 可见性说明的本机缓存 |
| #17 页签持久化 | ❌ **不做** | — | 恢复的只是地址（点开才加载），还要明文落盘 + 失效白屏 |
| #19 ABI / AAB | ❌ **不做** | — | 需要模拟器回归再按 CI-only 加 x86_64 |
| #11 后半「切换快照」 | ❌ **不做**（文案已实施） | — | 同 #17 判据：为「点开看一眼就走」维持多份活动状态不划算 |

四项「不做」的共同点：**都不是技术做不到，而是收益与代价不匹配**。若日后重开，各自小节的
「为什么不做」与「前置条件」就是验收门槛 —— 不满足门槛就别重开。


