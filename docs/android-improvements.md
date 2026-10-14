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
   **固定顺序（2026-10-14 起）**：insets → back → download → **extras** → schemes →
   **fullscreen → permissions**（extras 复用 download 建好的 `onWebViewCreate` 且调用必须排在其后；
   它改的是 `onPageFinished`，与改 `shouldOverrideUrlLoading` 的 schemes 互不耦合；
   fullscreen 整体替换 `RustWebChromeClient` 的 onShow/onHide 两方法；permissions 只动
   AndroidManifest 且与 download 各用各的 marker，先后仅以脚本假设为准）。
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
| 7 | P1 | `target=_blank` / `window.open` 拦截进移动页签 | ✅ 已实施（2026-10-14） | `mobilebar.js` blank guard + `mobilebar.rs` 新事件 `web2app:open-tab` |
| 8 | P1 | 底栏「更多」面板（刷新 / 复制链接 / 分享 / 浏览器打开 / 清数据 / 退出） | ✅ 已实施（2026-10-14） | `mobilebar.js` + `patch-android-extras.py`（Kotlin 桥；`mobilebar.rs` 零改动） |
| 9 | P2 | 返回键优先关闭页签列表面板等浮层 | 待办 | `patch-android-back.py` 增强 |
| 10 | P2 | 无历史时「再按一次退出」而不是立即 finish | 待办 | `patch-android-back.py` 增强 |
| 11 | P2 | 页签切换重载丢状态：先文案明示，后评估快照 | 待办 | 空状态文案先行 |
| 12 | P2 | 软键盘弹出时底栏让位（visualViewport） | 待办 | `mobilebar.js` |
| 13 | P2 | 暗色启动底色（SPLASH_BG 随 prefers-color-scheme + 原生主题） | 待办 | `bootstrap.rs` / `splash.html` / res 主题 |
| 14 | P2 | DNS TXT 解析结果持久化（冷启动秒开 + 后台静默更新） | 待办 | `bootstrap.rs` + settings.json |
| 15 | P2 | 代理模式可达性探测穿到真实上游（而非只探 127.0.0.1） | 待办 | `bootstrap.rs::spawn_reachability_probe` |
| 16 | P2 | 平板 / 横屏底栏限宽居中 | 待办 | `mobilebar.js` |
| 17 | P2 | 移动页签记录跨启动持久化 | 待办 | `mobilebar.rs` + settings.json |
| 18 | 安全 | 明文流量收敛：仅放行 localhost（networkSecurityConfig） | 待办 | 替换两个 workflow 里的 sed 步骤 |
| 19 | 分发 | 增加 x86_64 / universal APK；评估 AAB | 待办 | 两个 workflow 的 android build 步骤 |
| 20 | 流程 | 真机能力验收清单入回归（见 §4） | 进行中（本文件 §4 即清单） | 每次打包后人工执行 |

状态取值：待办 / 进行中 / ✅ 已实施（日期）。

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
  - 域名数组 + 关代理 → warning（解密结果静态不可知，提示「若真实地址需要 Basic Auth 必须开代理」）。
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
- 页面侧（`mobilebar.js`，Tauri 移动端专属）：底栏**无条件**新增第 5 格「更多」
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
  - `mobilebar.js` 新增 `installBlankGuard()`，仅 `HAS_TABS`（多页签开启）且非内置页时
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
  内嵌凭据+代理通过、无 target（Windows 路径）不受影响、域名数组+直连降级 warning。
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
- **#14/#17 持久化**：都落 settings.json，注意「缓存地址失效时的回退 + 弱网竞态」。
- **#18 安全收敛**：新增 `res/xml/network_security_config.xml`（cleartext 仅
  `127.0.0.1`/`localhost`），Manifest 改引用它；删掉两个 workflow 里全局置 true 的 sed。

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
10. 弱网：首启 25s 预算内失败卡文案与重试；杀进程冷启动（#14 后应秒开缓存地址）；
11. 代理模式：带鉴权站点经 `127.0.0.1` 加载成功；失败卡诊断指向上游真实结论（#15）；
12. 「更多」面板（#4/#8，逐条见 §2 验收 ①–⑧）：六行齐全；复制链接成功有 toast；
    分享 / 浏览器打开拉起正确；清数据两步确认 + 自动 reload 后 Cookie 与登录态消失需重新登录；
    退出无最近任务残留；跨源 iframe 无密钥调桥无效；旧包（无桥）点原生动作只 toast 不崩。
