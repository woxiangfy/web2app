# web2app（双壳：Tauri + Electron 绿色版）

`webview-app/`=Rust+Tauri2（含 Android）；`electron-app/`=仅 Win 绿色版。**单一真源**：`webview-app/src/{titlebar.js,loading.js,tab_title.js,dnd_polyfill.js,floaticon.html,splash.html,shell.html}`、`dist/settings.html`、`icons/*`、`src/{proxy.rs,resolve.rs}`→sidecar `w2a-proxy.exe`；改任一 ⇒ 必跑 Electron `probe` + `probe --tabs` + Tauri 页面探针（条数以实跑为准）。

> ⚠️ 本文件只留**索引 + 最高频契约**；细节外放：`topics/{repo-shape,electron-app,tauri-contracts,lessons,float-ball}.md`（lessons = 通用教训全量、float-ball = 悬浮球契约全量，排查前必扫）、`docs/packaging-config.md`、`docs/changes/YYYY-MM-DD.md`、本目录 `YYYY-MM-DD.md`、两壳 `AGENTS.md`。凡「计数」实跑取证再写。

## 契约

**A 形态 / 门禁**
1. ⚠️⚠️ 两壳**必须同级**（跨壳相对路径禁改）⇒ 禁止把 `electron-app` 挪进 `webview-app`。
2. ⚠️⚠️ 配置字段改必过三处：`parse-packaging-config.py`（唯一真源 **19 键**，`--print-contract-keys` 复核）+ `electron-app/src/main/config.ts` + Rust 侧；布尔禁 `bool()`；`rejectUnknownKeys` 用 `hasOwnProperty.call`；`build_merged()` 里 `app_url`/`extra` 要在 `str()` 循环**前**取。
3. ⚠️⚠️ 本机门禁逐条：`fmt --check` / `clippy -D warnings` / `test --all` / `typecheck` / `build` / `probe` / `probe --tabs` / `smoke`；Android 本机做不了 ⇒ 靠 CI。⚠️ **`cfg` 分叉只决定「数据」不决定「可变性」**（`let mut` 只在某 cfg 块被改 ⇒ `unused_mut` ⇒ 编译失败）；**非 Win 分支是本机盲区 ⇒ 本地全绿 ≠ CI 绿**。clippy 的 `os error 5` 是 target 竞争**噪声**（判真假 `grep -E "^error"`）。

**B 编辑 / 取证纪律**
4. ⚠️⚠️ **同一文件多处编辑禁同批发出**（后写者基于旧快照 ⇒ 前一条静默丢失却报 success）；改完 grep。`String.replace` 替换串含 `$&` 改 `split/join`。
5. ⚠️⚠️ **「消灭缺陷」≠「满足需求」**：先把需求翻成**成对可观测判据**。**取证方法错 ⇒ 结论错** —— GUI exe 用 `Start-Process -Wait -PassThru` 取 `ExitCode` 且先清 `ELECTRON_RUN_AS_NODE`；**改完 Rust 源码必须 `cargo build` 再取证**。
6. ⚠️⚠️ **新增守卫必须反向对照**（改坏 → 期望 FAILED → 还原比 sha256）；被测字面量用 `concat!` 拆段，否则**自我满足**；⚠️ **针会被注释满足** ⇒ 匹配前先剥注释（`has_code_line` / `strip_line_comments` 已上提 `contract_tests/mod.rs` 共用，位置断言也用剥注释后文本 `find`）；CRLF 文件逐字节还原用 Python 写字节。

**C 窗口 / 注入 / 设置**
7. ⚠️⚠️ 顶部空间 = **`html` 不滚、`body` 滚**；`Builder::header` 是 **append** 非 replace ⇒ 不跳过 = 两个 `Origin`（页面能开、WS/POST 全挂）。
8. ⚠️⚠️ 注入脚本必须跑**主世界**（`webFrame.executeJavaScript`），否则标题栏画得出、按钮点不动；严格 CSP 用 `adoptedStyleSheets`。
9. ⚠️⚠️ **`.invoke_handler(...)` 全项目只能一次**（**替换**非追加 ⇒ 写两次静默清空前一批）；平台差异写**命令项上的 `#[cfg]`**。守卫 `lib.rs::ipc_command_registry`。
10. ⚠️⚠️ **运行期只有一个 JSON：`settings.json`**（三键平铺 + `window` 段）；**两个写入方**（设置保存 / 关窗记几何）**必须读-改-写**；落点 `<data_dir>\settings.json`，旧位置是**只读候选且「读」也走同一候选**。守卫 `store::tests`(13) + 探针 4。
11. ⚠️⚠️ **设置界面 = 无边框 + 铺满整窗 + `.panel-bd` 唯一滚动容器**（两壳共用 `dist/settings.html`）；无边框 ⇒ 拖动需 capability `start-dragging`。标题栏「设置」在远程页只能走核心插件事件（`core:event:allow-emit` → `app.listen("web2app:open-settings")`；Electron `win:open-settings` / `tabs:open_settings`），**面板词表 / 白名单 / IPC 名三处同改**。守卫 `lib.rs::capability_contract`(3)。
11b. ⚠️⚠️ **多页签「首页页签」= 位次 0**：固定第一位 + 不可关 + 房子图标 + 不渲染 `.close`。双壳真源 `tabs.rs::is_home_index` ∥ `tab-order.ts::isPinnedHome` **必须同答案**；`get_tabs` 是**四元组** `[label,title,active,home]`；**拦截必须在摘登记/销毁窗口之前**。守卫 `tab_home_marker_contract` + `tabs::tests::home_tab_*` + 探针 P1h/P7h。→ `docs/changes/2026-10-13.md`
11c. ⚠️⚠️ **页签外观 = Chrome 风，只改「形状」不改「配色」**：顶部 8px 圆角 + `.tabs{align-items:flex-end}` + 相邻非激活间短竖线 `.tab + .tab::before` + 激活白底 + `box-shadow` + 关闭按钮圆点；`shell.html` 与 `titlebar.js` 共享的 **7 个色值**仍由 `top_bar_style_contract::tab_bar_palette_matches_titlebar` 逐字钉住。→ 同上
11d. ⚠️⚠️ **Android UI 线程禁调阻塞型 webview getter**（真机「白屏约 10 秒后自动退出」根因）：`url()` / `cookies_for_url()` / `can_go_back()` 全是 `MainPipe::send` + `recv_timeout(10s)`，消费方正是建 Activity 的 UI 线程 ⇒ 僵 10s 后 `tx.send().unwrap()` panic abort。`on_page_load` 闭包 / 移动端回调一律用 **payload 或缓存**（`mobilebar::CurrentUrl`）。守卫 `mobile_bar_never_queries_webview_on_ui_thread`（钉整个 `.url()` 形态，只钉 `window.url()` 会被 `w.url()` 绕过）。→ `webview-app/docs/android.md`
11e. ⚠️⚠️ **Ctrl+W 关当前页签，双壳各有两条焦点通路**：网页里 → Electron `before-input-event` ∥ Tauri 注入 `tab_keys.js` → 核心事件 `web2app:close-tab`（label + 一次性 token）；标签栏 → `shell.html` keydown（+ Electron 宿主 `webContents`）。首页仍拒绝，Ctrl+Shift+W / Alt / Meta / 长按不算。⚠️ 禁 WebView2 `AcceleratorKeyPressed`（写了永不触发 = 假绿）；Electron 内容视图那条必须 `setImmediate`；**只认逻辑键位 `key`，禁 `code` 兜底**（AZERTY 上「生出 w 的物理键」是 `KeyZ` ⇒ Ctrl+Z 会变关页签）。守卫 `capability_contract::tab_close_*`(2) + `page-behavior.test.ts`(3) + 探针 P13。→ `docs/changes/2026-11-03.md`
11f. ⚠️⚠️ **设置页「唤醒快捷键」= 组合键（修饰键至少一个 + 主键），禁裸单键**：`captured` 唯一状态、手动组合与捕获两个编辑器互相重画；「当前生效」`savedHotkey` 与「待保存预览」`hkPreview` **必须分开**。`hkProblem()` 保存前拦裸单键 / `Alt+Tab·Alt+Space·Alt+F4·Alt+Esc·Ctrl+Esc·Ctrl+Space` / `Shift+F1`，捕获路径同样拒收。四个易回归点：① key 为空**不得**清修饰键；② 历史键不在下拉列表要补 option；③ 捕获到列表外的键（`ArrowUp`）必须 `ensureKeyOption()` 在 **`renderHotkey` 每次**调（否则 `selectedIndex=-1`、碰下拉就丢结果）；④ `get_settings` 失败 ⇒ `loadFailed` + 禁用保存 + 保存处第二道，状态行说「当前快捷键**未知**」。后端保持**宽容解析**。静态存在性断言**必须剥注释**。守卫 探针 `probe` 13b 八条 + `settings_hotkey_contract`(5)。→ `docs/changes/2026-11-03.md`

**D 打包 / CI**
12. ⚠️⚠️ **本地打包 = `scripts/pack-local.mjs`**（Node 零依赖 ≥18、**不需 Python**）；解析逻辑**两份实现** ⇒ 改一侧必同步 + 跑 `--parity`。行为全由 JSON 决定；输出目录**不自己清**（删除配额 50 次/轮）⇒ 被 `safe-delete` 拦住才换 `release/local-<时间戳>/`；改写 tracked 的 `tauri.conf.json` / `icons/` / `electron-app/config.json` ⇒ 备份 + `finally` 还原。env **必须剥 `ELECTRON_RUN_AS_NODE`**；`--out` 相对**以 `electron-app/` 为基准**。→ `docs/packaging-config.md` §7.3
12b. ⚠️⚠️ **Electron 出包入口有三个，加步骤必须三边都看**：① `electron-app/package.json` 的 `pack`/`dist*`；② **本地打包** `tests/build-electron-all.bat` → 根 `scripts/packlib/driver-electron.mjs`；③ **`.github/workflows/` 里自拼步骤的** `electron-build-custom.yml`（按勾选组合出包，故意不调 `dist`）。顺序一律 `sidecar → build → stage → prune → electron-builder`（② 另加 `check-asar-config.cjs`；`electron-ci.yml` 走 `pnpm run dist` 天然含 prune）。2026-11-07 加 `prune` 时只改① ⇒ ②③ 都漏，2026-11-14 本地实测报红、CI 侧同型缺陷一并补。守卫 `probe-isolation.test.ts` 三条分别钉 ①②③，⚠️ builder 锚点用 `runTee(builderCmd(`（`builderCmd(` 首次出现是函数定义 ⇒ 恒真假绿）。→ `docs/changes/2026-11-14.md`
13. ⚠️⚠️ **CI 装 Android SDK = `.github/scripts/install-android-sdk.sh`**（两 workflow 同一份）：平台包不一定叫 `platforms;android-<N>`（16 起带次版本号，实测只有 `36.1/37.0/37.1/37.2`、**无 `android-37`**；新平台**先落 canary**）⇒ 按真实包名挑 + 必要时 `--channel=3`；补 `platforms/android-N` 软链；回落最高可用并改写 gradle（`ANDROID_SDK_STRICT=1` 则硬失败）。⚠️ 三个 `|| true` 是实测的。验证 `bash tests/ci-sdk-step/run-tests.sh`。
13b. ⚠️⚠️ **`gen/android` 的一切改写 = `.github/scripts/` 下的共享脚本**（两 workflow 只调用）：`prepare-android-conf.py`（init **前**写 version/productName/identifier）、`apply-android-labels.py`、`patch-android-insets.py`、`set-android-apk-name.py`、`patch-android-back.py`、`install-android-ndk.sh`。工程根解析走 `android_gen.py` —— ⚠️ CI 的 cwd 已是 `webview-app/` ⇒ 候选**首选 `gen/android`**（写死 `webview-app/gen/android` = P0）。守卫 `contract_tests::android_pipeline_contract`(5)，各脚本 `--selftest`。⚠️ **注入 `build.gradle.kts` 的路径禁数 `parentFile` 层数**（`rootProject.projectDir` 到仓库根是三级），须逐级向上找 `.github/scripts` + 绝对路径兜底 + 配置期 `GradleException`；**禁写 `java.` 包前缀**（`java` 是扩展访问器 ⇒ `Unresolved reference 'io'`）。→ `docs/changes/2026-11-01.md`

**E 悬浮窗 / 球（v1.67–v1.80）** —— ⚠️ **全量契约与教训已迁 `topics/float-ball.md`（排查前必扫）**；逐版过程 `docs/changes/2026-10-11.md` / `2026-10-12.md`。四条最常踩的：
14. 销毁必须 `destroy()` 不能 `close()`；Electron 的 close-prevent 会取消 `app.quit()`（注入 `isQuitting` 放行）；冷启动副作用用 `applyInitialSettings`；`hide` 要清拖动标志。
15. 球真机 harness = `webview-app/src/floatprobe.rs`（`--float-probe`）：**条数只认结果文件的 `ALL PASS` 行**；**探针模式必须跳过单实例插件与全局快捷键**（否则假绿）；**本机 `build()` 约 50% 挂死是环境抖动**。
16. 球窗**不设任何窗口区域**（v1.80 起）：视觉 = `WS_EX_NOREDIRECTIONBITMAP` + DComp 真逐像素 alpha；命中 = Tauri `WM_NCHITTEST` 子类化 ∥ Electron `win.setShape`（**都只管命中**）。
17. 五条不变量：**球 == 活柱 == 命中圆**（禁 `scale()` / `opacity`，反馈用 `filter: brightness()`）；球页**禁原生 `title`**；唯一显示入口 `show_ball()`，顺序 `show → unminimize → styles → delete_taskbar_tab`；「关窗 = 退出」必须显式写驻留理由；守卫都在 `contract_tests.rs`。

## 本机环境硬限制（都会伪装成「代码坏了」）
1. **删除走安全垫片**：递归删除被拦，只有**逐文件 unlink** 能过，**配额 ≈50 次/轮** ⇒ 写产物前确认目录已被 `.gitignore` 锚定（⚠️ 往 `tests/` 写一次性产物前先确认该子目录已锚定）；跑构建**换全新输出目录**。
2. **`node -e` 不可靠**（stdout 被吞、退出码 0）⇒ 取证一律写**脚本文件**。
3. **PATH 上 `python` 是 reparse point** ⇒ Node `spawnSync('python')` **恒定 EBUSY**（需绝对路径）；本机 `--parity` 会留「未能验证」（CI 是 Linux，无此限）。
4. **系统临时区会被外部脚本清掉** ⇒ 唯一副本别只放临时区。
5. **改 UI 目视取证**：不能只跑 `cargo test` 就开 exe（不刷新）；WebView2 走 DWM 合成截不到 ⇒ 用 Electron 按真实尺寸渲染同一份共享 HTML 截图。

## 状态
- **仓库 v1.80**（球视觉真逐像素 alpha + `WM_NCHITTEST` 圆形命中；tauri 2.12.1 / wry 0.57.0 / tao 0.37.1）；Electron M0–M7 全交付；Tauri 持续维护。
- **测试基数（2026-11-03 实跑，时点值，以实跑输出为准）**：Rust **287 passed / 6 ignored**、Electron `unit` **123/123**、契约探针**单窗 158 / 页签 54**。`--parity` 本机恒 EBUSY ⇒ payload 数靠源码统计（**43**）。⚠️ 提交 / CI / 远端 / 并行改动状态一律 `git status` 现查，别据旧快照下结论。
- ⚠️ Tauri 侧本地打包未实跑（无 tauri-cli，只有 `--dry-run`）；M4「另存为」未经真机人验；两壳版本均 `0.1.0`、**无 tag**；`core.autocrlf=true` 与根 `AGENTS.md` §3.4.2「统一 LF」冲突，**待决**。本机 Electron 探针 / GUI **间歇性可跑**（同一命令曾连续三次 `whenReady()` 后即死、退出码 127）⇒ 别把某次死机当产品缺陷，换一轮重试即可。
- `docs/txttest-verify-materials.md` 自 2026-11-14 起改为「**本机手动生成 TXT 密文 + 打包**」操作手册（历史测试素材、旧密文表与跨语言对照已清除；主路径 = 双击 `scripts/gen-resolve-keys.bat` → `resolve-tool encrypt` → DNS TXT → `pack-local.mjs`）。
- ⚠️⚠️ **动态解析的两个「看起来像网络问题」的失败形态，别混**（2026-11-14 实测）：① **解析失败** = 界面「服务器连接失败，请检查网络后重试」⇒ 查 `w2a-proxy.exe --domains … --public-key …` 的 stderr；② **导航失败** = 「网页加载失败」/`chrome-error://` ⇒ 密文里的 URL 本身不可用（`nslookup <主机> 1.1.1.1`），要重新签发 TXT（**不必重打包**）。⚠️ **TXT 记录名里的下划线只能当 label 首字符**：hickory 的 `txt_lookup(&str)` 走 IDNA/STD3（`txt_w2a_abc.*` 直接 `Label contains invalid characters`、请求都没发出），`resolve.rs::lookup_name` 已改用 `Name::from_str_relaxed` 放宽；另 `resolve.rs` 的查询失败必须 `dlog!` 带 `{e}`（曾 `Err(_)` 吞掉唯一线索）。
- 2026-11-14：Android 清单 **#9/#10/#11/#12/#13/#15/#16/#18 已落地**（返回键浮层优先 + 双按退出 / 键盘让位 / 探真实上游 / 宽屏限宽 / 启动底色走主题 / 明文单一落点）；**#14/#17/#19 与 #11 后半「切换快照」全部定为不做**（产品侧判断无必要，清单无悬空项；理由与重开门槛见 `docs/android-improvements.md` §5，该节已从「待决策」改为「决策留档」）。⚠️ **#14 安全上本可行**（v4 载荷自带 `sign_day`/`ttl_days` ⇒ 缓存有效期可钉死），不做纯因收益不抵代价 —— 日后重开别再从「安全语义」论证一遍。⚠️ **Android 首帧底色只能靠资源主题**：tao 0.37.1 的 `background_color` 在 Android 是空实现（文档写明 Unsupported）⇒ `nav::SPLASH_BG` 在安卓从未生效，改它 = 打空靶；正解 = `patch-android-splash-theme.py`（主题 `windowBackground` → `values`/`values-night` 颜色，色值须与 `splash.html` 逐字一致，契约测试跨文件钉住）。⚠️ **Android 明文 = 全放行**（`patch-android-cleartext.py` 写 `network_security_config` 的 `base-config=true`，取代 workflow 里的全局 `usesCleartextTraffic=true`）；曾收紧为「仅 `127.0.0.1`/`localhost`」⇒ 直连 `http://` 外站被系统拦，产品侧要求回退，**别再收紧**（真要收紧必须保留回环例外）。
