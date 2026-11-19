# web2app（双壳：Tauri + Electron 绿色版）

`webview-app/`=Rust+Tauri2（含 Android）；`electron-app/`=仅 Win 绿色版。单一真源：`webview-app/src/{titlebar,loading,tab_title,tab_keys,dnd_polyfill}.js`、`shell.html`、`splash.html`、`floaticon.html`、`dist/settings.html`、`icons/*`、`src/{proxy,resolve}.rs`→sidecar `w2a-proxy.exe`；改任一 ⇒ 双壳验证（Electron `probe` / `probe --tabs` / `smoke`；Tauri 页面探针）。

> ⚠️ 本文件 = 索引 + 最高频契约。**全量原文**（2026-11-14 瘦身前，逐条未改）→ `topics/archive-2026-11-14-memory-full.md`；专题 `topics/{repo-shape,electron-app,tauri-contracts,lessons,float-ball}.md`（lessons = 通用教训、float-ball = 球契约，排查前必扫）、`docs/packaging-config.md`、`docs/changes/`、本目录日志、两壳 `AGENTS.md`。计数一律实跑取证。

## 最容易踩（细节见归档 / topics）
1. 两壳必须同级；跨壳相对路径禁改。
2. 配置字段改必过三处（解析器唯一真源 **19 键** / `config.ts` / Rust），布尔禁 `bool()`，`app_url`+`extra` 在 `str()` 循环**前**取。
3. 同一文件多处编辑**禁同批**发出（后写基于旧快照 ⇒ 静默丢改动）；改完 grep。
4. 新增守卫必须反向对照（改坏→FAILED→还原比 sha256）；匹配前**剥注释**，字面量用 `concat!` 拆段；源码文本守卫判定范围**贴目标节点**（大回看窗口会吃邻近标注假绿，2026-11-19 `@internal` 守卫实翻车）。
5. `.invoke_handler(...)` 全项目只能一次（**替换**非追加）；平台差异写命令项上的 `#[cfg]`。
6. 运行期只有一个 `settings.json`，两个写入方**必须读-改-写**。
7. Android **UI 线程禁调阻塞型 webview getter**（`url()` 等 ⇒ 僵 10s 后 abort），改用 payload / 缓存。
8. Electron 出包入口**三处**：`package.json` 的 `pack`/`dist*`、`tests/build-electron-all.bat`、`electron-build-custom.yml`；顺序 `sidecar→build→stage→prune→electron-builder`。
9. `gen/android` 的一切改写只走 `.github/scripts/` 共享脚本；CI 的 cwd 已是 `webview-app/` ⇒ 候选首选 `gen/android`。
10. 球：销毁用 `destroy()`；球窗不设任何窗口区域（命中靠 `WM_NCHITTEST` ∥ `setShape`）；`show_ball()` 是唯一显示入口。

## 本机环境硬限制（都会伪装成「代码坏了」）
1. 递归删除走安全垫片（逐文件 unlink，配额 ≈50 次/轮）⇒ 写产物前确认已被 `.gitignore` 锚定，构建换全新输出目录。
2. `node -e` 不可靠 ⇒ 取证一律写脚本文件。
3. PATH 上 `python` 是 reparse point ⇒ Node `spawnSync` 恒 EBUSY，本机 `--parity` 只能留「未能验证」。
4. 系统临时区会被外部脚本清掉 ⇒ 唯一副本别只放临时区。
5. 改 UI 目视取证：WebView2 走 DWM 合成截不到 ⇒ 用 Electron 渲染同一份共享 HTML 截图。

## 状态
- 仓库 v1.80（tauri 2.12.1 / wry 0.57.0 / tao 0.37.1）；Electron M0–M7 全交付。
- 测试基数 = 时点值，以实跑为准（2026-11-03：Rust 287 passed / 6 ignored、unit 123、探针 158 / 54）。提交 / CI 状态一律 `git status` 现查。
- 未决：Tauri 本地打包未实跑（无 tauri-cli）、M4「另存为」无人验、两壳 `0.1.0` 无 tag、`core.autocrlf` 与 LF 约定冲突、本机 Electron 探针间歇死机（重试即可）。
- ⚠️ 动态解析两类失败别混：**解析失败**（「服务器连接失败」⇒ 查 sidecar stderr）vs **导航失败**（`chrome-error://` ⇒ 密文 URL 不可用，重签 TXT 不必重打包）。`Name::from_str_relaxed` 等细节见归档。
- 2026-11-14：Android 清单 #9–#18 已落地，`#14/#17/#19` 与「切换快照」定为不做 → `docs/android-improvements.md` §5。
- 2026-11-18：Electron 自研「网页账号密码保存」已交付，**同日复审又修 4 处**（观察器提前 return ⇒ SPA 同文档登录永不落盘、新文档 1.2s 确认探针误丢暂存、重加密回写用旧快照、清浏览数据清不掉暂存）→ `docs/changes/2026-11-18.md`；Electron unit **215**、单窗口探针 **178**、页签 **69**。
- 2026-11-19：Tauri 架构审查（`docs/architecture-review-2026-11-19.md`）+ 两项零行为变更：Android 可达性探测 `bootstrap.rs`→**`src/probe.rs`**（挂 `any(not(windows), test)`）、`Settings`/`Hotkey`→**`src/model.rs`**（`store`/`settings` 共同引用，类型环已断；`crate::settings::Settings` 经 `pub use` 仍可用）。**同日第二轮**：复核 6 项待决策（P-R6「抽 apply.rs 断环」/ P-R7「floatprobe feature」**改判不做**）并落地 P-R3/P-R4/P-R5 —— ①弹窗落点改写规则上移 **`nav::resolve_popup_target`**（桌面 tabs 与 Android mobilebar 共用；此前移动端完全没做 ⇒ 代理+鉴权下 `target=_blank` 落 401，**缺口已闭**）；②`save_settings` 改**先落盘后副作用**（失败 `restore_persisted` 回写，钉住「存不下快捷键」失败链）；③`mobilebar.rs` 测试外置 `src/mobilebar/tests.rs`（生产 1005→789 行）。两条新契约均已反向对照。Rust `cargo test --all` **334 passed / 3 ignored**。
- 2026-11-19：**安卓真机「加载失败」根因 = Manifest 缺 `ACCESS_NETWORK_STATE`**（hickory 0.26 的 Android `system_conf` 用 JNI 调 `ConnectivityManager.getActiveNetwork()` 取系统 DNS）。修法两层：① `patch-android-permissions.py` 增**恒需权限块**（INTERNET + ACCESS_NETWORK_STATE，**缺失才补**、marker `web2app-android-required`、`android_permissions` 只管可选项；空数组**不再整步跳过**）；② `resolve.rs::build_resolver()` 读系统配置失败**不再判死** ⇒ `fallback_config()`（223.5.5.5 / 119.29.29.29）。**契约**：Rust 侧读系统资源 ⇒ 必须先核对 Android Manifest 权限；失败卡出 `JNI call error` + `permission.` ⇒ 清单缺权限，不是网络问题。`cargo test --all` **336 passed / 6 ignored**（19+22+38+257）⇒ `docs/changes/2026-11-19.md`、`docs/android-improvements.md` #21、`webview-app/docs/android.md` 排障史新节。
- 2026-11-19：**Android 打包冻结契约** → `docs/android-packaging-freeze.md`（评审：打包层是下游参照实现，不该随功能变；默认「能用自己的代码解决就不动打包」）。三层：L0 恒需常量（无条件）/ L1 可选能力（仅 `android_permissions`）/ L2 版本适配（wry 胶水 3 处 + prepare-wry-kotlin + gradle 重放链）。**唯一产品输入面** = `APP_NAME`/`TITLE`/`VERSION`/`ANDROID_PERMISSIONS`，由契约测试 ⑨ `android_packaging_product_inputs_are_frozen` 冻结（扫 environ、剥 `#` 注释、白名单外红灯）。判据：关掉它应用还能跑吗？能 ⇒ L1，不能 ⇒ L0，只因上游没实现 ⇒ L2。L2 归零路线（schemes→on_navigation+opener、extras 密钥→webview.eval、fullscreen→随 wry 升级）完成后可删 2 个基础脚本（15→11）。`test` **337**。
- 2026-11-19：**Android 打包「升级免疫」**（评审：升级也别改脚本）→ `docs/android-packaging-freeze.md` §6 三条机制：①锚点只认平台 override 签名（+花括号配对，不匹配方法体原文）；②`android_gen.AnchorDrift`+`run_guarded()`：漂移默认 `::warning::`+exit 0（`W2A_ANDROID_PATCH_STRICT=1` 才红），可降级名单 `GUARDED_SCRIPTS` 7 个，permissions/cleartext 保持硬失败；③上游实现即退场（fullscreen 见体内无 `onCustomViewHidden()` ⇒ 原样返回）。证据 `rehearse.py --drill` 4 类变异全绿 + 反向对照 4 组 sha256 一致。`test` **337**。**边界**：上游删 override 又无替代仍需动手（补丁该退役）。
- 2026-11-19（复核）：**定位失败也要分「环境错误 vs 布局漂移」** —— `gen/android`/`java` 不存在 ⇒ `FileNotFoundError`（红）；工程在但 `RustWebViewClient.kt`/`MainActivity.kt` 不在、`<style>` 找不到、wry 模板清单全未命中 ⇒ `AnchorDrift`（降级）。`GUARDED_SCRIPTS` 8 个。演练第 5 类 `wry_file_renamed` 证明半成品方向安全（MainActivity 桥不反向依赖 client）。`patch-android-insets.py` 的 `write_text` 曾缺 `newline="\n"`（Windows 写 CRLF，CI Linux 掩盖）。
- 2026-11-19（第四轮）：**Android 打包 `error[E0603]`** —— `probe` 抽模块后 `probe_target`/`spawn_reachability_probe` 仍私有；桌面看不见是因为调用点与定义**都被 `cfg(not(windows))` 摘掉**。修：两函数 `pub(crate)`、`show_float_window` 非 Windows 消费 `app`。新守卫 `contract_tests/module_visibility_contract.rs`（扫 `crate::<模块>::<项>`、带 10 处下限自校验、反向对照已做）。**教训：抽模块 = 可见性变更**；平台专属模块（probe/mobilebar/floatwin）的错误桌面 `cargo test` 看不见。门禁真相：`ci.yml` 的 `check-android` 才是 Android-cfg 权威，手工 `workflow_dispatch` 出包不等它。`test` **338**。
