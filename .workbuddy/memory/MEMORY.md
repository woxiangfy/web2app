# web2app（双壳：Tauri + Electron 绿色版）

`webview-app/`=Rust+Tauri2（含 Android）；`electron-app/`=仅 Win 绿色版。**单一真源**：`webview-app/src/{titlebar.js,loading.js,tab_title.js,dnd_polyfill.js,floaticon.html,splash.html,shell.html}`、`dist/settings.html`、`icons/*`、`src/{proxy.rs,resolve.rs}`→sidecar `w2a-proxy.exe`；改任一 ⇒ 必跑 Electron `probe` + `probe --tabs` + Tauri 页面探针（条数以实跑为准）。

> ⚠️ 本文件只留**索引 + 最高频契约**，细节一律外放：`topics/{repo-shape,electron-app,tauri-contracts,lessons,float-ball}.md`（lessons = **通用教训全量**、float-ball = **悬浮球 / 悬浮窗契约全量**，二者排查前必扫）、`docs/packaging-config.md`、`docs/changes/YYYY-MM-DD.md`、本目录 `YYYY-MM-DD.md`、两壳 `AGENTS.md`。凡「计数」**实跑取证再写**。

## 契约

**A 形态 / 门禁**
1. ⚠️⚠️ 两壳**必须同级**（跨壳相对路径禁改）⇒ 禁止把 `electron-app` 挪进 `webview-app`。
2. ⚠️⚠️ 配置字段改必过三处：`parse-packaging-config.py`（唯一真源 **19 键**，`--print-contract-keys` 可复核）+ `electron-app/src/main/config.ts` + Rust 侧；布尔禁 `bool()`；`rejectUnknownKeys` 用 `hasOwnProperty.call`；`build_merged()` 里 `app_url`/`extra` 要在 `str()` 循环**前**取。
3. ⚠️⚠️ 本机门禁逐条：`fmt --check` / `clippy -D warnings` / `test --all` / `typecheck` / `build` / `probe` / `probe --tabs` / `smoke`；Android 本机做不了 ⇒ 靠 CI。⚠️ **`cfg` 分叉只决定「数据」不决定「可变性」**（`let mut` 只在某 cfg 块被改 ⇒ `unused_mut` ⇒ 编译失败）；**非 Win 分支是本机盲区 ⇒ 本地全绿 ≠ CI 绿**。clippy 的 `os error 5` 是 target 竞争**噪声**（判真假 `grep -E "^error"`）。

**B 编辑 / 取证纪律**
4. ⚠️⚠️ **同一文件多处编辑禁同批发出**（后写者基于旧快照 ⇒ 前一条静默丢失却报 success）；改完 grep。`String.replace` 替换串含 `$&` 改 `split/join`。
5. ⚠️⚠️ **「消灭缺陷」≠「满足需求」**：先把需求翻成**成对可观测判据**。**取证方法错 ⇒ 结论错** —— GUI exe 用 `Start-Process -Wait -PassThru` 取 `ExitCode` 且先清 `ELECTRON_RUN_AS_NODE`；**改完 Rust 源码必须 `cargo build` 再取证**。
6. ⚠️⚠️ **新增守卫必须反向对照**（改坏 → 期望 FAILED → 还原比 sha256）；被测字面量用 `concat!` 拆段，否则**自我满足**；⚠️ **针会被注释满足** ⇒ 匹配前先剥注释；CRLF 文件逐字节还原用 Python 写字节。

**C 窗口 / 注入 / 设置**
7. ⚠️⚠️ 顶部空间 = **`html` 不滚、`body` 滚**；`Builder::header` 是 **append** 非 replace ⇒ 不跳过 = 两个 `Origin`（页面能开、WS/POST 全挂）。
8. ⚠️⚠️ 注入脚本必须跑**主世界**（`webFrame.executeJavaScript`），否则标题栏画得出、按钮点不动；严格 CSP 用 `adoptedStyleSheets`。
9. ⚠️⚠️ **`.invoke_handler(...)` 全项目只能一次**（**替换**非追加 ⇒ 写两次静默清空前一批，Win 上报 `Command save_settings not found`）；平台差异写**命令项上的 `#[cfg]`**。守卫 `lib.rs::ipc_command_registry`。
10. ⚠️⚠️ **运行期只有一个 JSON：`settings.json`**（三键平铺 + `window` 段）；**两个写入方**（设置保存 / 关窗记几何）**必须读-改-写**；落点 `<data_dir>\settings.json`，旧位置是**只读候选且「读」也走同一候选**（否则升级后首次保存清零、还不报错）。守卫 `store::tests`(13) + 探针 4。
11. ⚠️⚠️ **设置界面 = 无边框 + 铺满整窗 + `.panel-bd` 唯一滚动容器**（两壳共用 `dist/settings.html`）；无边框 ⇒ 拖动需 capability `start-dragging`（少了不报错、只拖不动）。标题栏「设置」在远程页只能走核心插件事件（`core:event:allow-emit` → `app.listen("web2app:open-settings")`；Electron `win:open-settings` / `tabs:open_settings`），**面板词表 / 白名单 / IPC 名三处同改**。按钮 `[←][→] 标题 [⚙][—][▢][✕]`。守卫 `lib.rs::capability_contract`(3)。

11b. ⚠️⚠️ **多页签「首页页签」= 位次 0**（2026-10-13）：入口 app_url 那一签**固定第一位 + 不可关**，名字前带房子图标、**不渲染 `.close`**。双壳真源 `tabs.rs::is_home_index` ∥ `tab-order.ts::isPinnedHome`（**必须同答案**）；`get_tabs` 是**四元组** `[label,title,active,home]`（三元组 = 首页退化成普通页签）；**拦截必须在摘登记/销毁窗口之前**（先动后判 ⇒ 半具残骸）。守卫 `tab_home_marker_contract` + `tabs::tests::home_tab_*` + 探针 P1h/P7h（**39 → 47 条**）。

11c. ⚠️⚠️ **页签外观 = Chrome 风，只改「形状」不改「配色」**（2026-10-13）：顶部 8px 圆角 + 页签坐落条底（`.tabs{align-items:flex-end}`）+ 相邻非激活页签之间的**短竖线** `.tab + .tab::before`（紧邻激活/悬停侧 `opacity:0`）+ 激活页签白底 + `box-shadow` 上浮 + 关闭按钮圆点（`border-radius:50%`）。⚠️ **`shell.html` 与单窗口 `titlebar.js` 共享的 7 个色值仍由 `top_bar_style_contract::tab_bar_palette_matches_titlebar` 逐字钉住** —— 新增的 `--tab-sep*` / `--tab-shadow*` 是**本文件私有**，别混进那份清单。页签宽度：`flex:1 1 auto` + `max 220 / min 68`（少页签撑到上限即停、多页签收缩后横滚）。

**D 打包 / CI**
12. ⚠️⚠️ **本地打包 = `scripts/pack-local.mjs`**（Node 零依赖 ≥18、**不需 Python**）；解析逻辑**两份实现** ⇒ 改一侧必同步 + 跑 `--parity`。行为全由 JSON 决定；输出目录**不自己清**（删除配额 50 次/轮）⇒ 被 `safe-delete` 拦住才换 `release/local-<时间戳>/`；改写 tracked 的 `tauri.conf.json` / `icons/` / `electron-app/config.json` ⇒ 备份 + `finally` 还原。env **必须剥 `ELECTRON_RUN_AS_NODE`**；`--out` 相对**以 `electron-app/` 为基准**。→ `docs/packaging-config.md` §7.3。
13. ⚠️⚠️ **CI 装 Android SDK = `.github/scripts/install-android-sdk.sh`**（两 workflow 同一份）：平台包不一定叫 `platforms;android-<N>`（16 起带次版本号，实测只有 `36.1/37.0/37.1/37.2`、**无 `android-37`**；新平台**先落 canary**）⇒ 按真实包名挑 + 必要时合并 canary 加 `--channel=3`；补 `platforms/android-N` 软链；回落最高可用并改写 gradle（置 `ANDROID_SDK_STRICT=1` 则回落硬失败）。⚠️ 三个 `|| true` 是实测的（加 `set -euo pipefail` = 加新失败模式）。验证 `bash tests/ci-sdk-step/run-tests.sh`（36 条断言）。
13b. ⚠️⚠️ **`gen/android` 的一切改写 = `.github/scripts/` 下的共享脚本**（2026-10-13，两 workflow 只调用、不再各存内联 python）：`prepare-android-conf.py`（init **前**写 version/productName/identifier）、`apply-android-labels.py`、`patch-android-insets.py`、`set-android-apk-name.py`、`patch-android-back.py`、`install-android-ndk.sh`（NDK 版本单一真源）。工程根解析走 `android_gen.py` —— ⚠️ CI 的 cwd 已是 `webview-app/`，故候选**首选 `gen/android`**（写死 `webview-app/gen/android` = P0：补丁步 FileNotFoundError ⇒ 后续 build/签名/上传全跳过，还被 `continue-on-error` 吞掉）。守卫 `contract_tests::android_pipeline_contract`(5)。各脚本 `--selftest` 由 CI `config-contract` 跑。
⚠️⚠️ **注入进 `build.gradle.kts` 的路径禁「数 `parentFile` 层数」**（2026-11-01）：`rootProject.projectDir` 已是 `gen/android`，到仓库根是**三级** ⇒ 数错一级得到 `webview-app/.github/scripts`，而 `Exec.workingDir` **只在执行期**校验（配置期全绿、白等 5 分钟才报「目录不存在」）⇒ 逐级向上找 `.github/scripts` + 注入时绝对路径兜底 + 配置期 `GradleException`；注：该处**禁写 `java.` 包前缀**（`java` 是扩展访问器 ⇒ `Unresolved reference 'io'`）。→ `docs/changes/2026-11-01.md`

**E 悬浮窗 / 球（v1.67–v1.80）** —— ⚠️ **该段全量契约与教训已迁 `topics/float-ball.md`（球 / 悬浮窗相关排查前必扫）**；逐版过程见 `docs/changes/2026-10-11.md`（v1.67–v1.77b）与 `docs/changes/2026-10-12.md`（v1.78–v1.80）。四条最常踩的：
14. 销毁必须 `destroy()` 不能 `close()`；Electron 的 close-prevent 会取消 `app.quit()`（注入 `isQuitting` 放行）；冷启动副作用用 `applyInitialSettings`（`applySettings(...,{persist:false})` 的 diff 恒 false）；`hide` 要清拖动标志。
15. 球真机 harness = `webview-app/src/floatprobe.rs`（`--float-probe`）：**条数只认结果文件的 `ALL PASS` 行**；**探针模式必须跳过单实例插件与全局快捷键**（否则假绿）；**本机 `build()` 约 50% 挂死是环境抖动、不是产品缺陷**。
16. 球窗**不设任何窗口区域**（v1.80 起）：视觉 = `WS_EX_NOREDIRECTIONBITMAP` + DComp 真逐像素 alpha；命中 = Tauri `WM_NCHITTEST` 子类化 ∥ Electron `win.setShape`（**都只管命中、都不裁视觉**）。
17. 五条不变量：**球 == 活柱 == 命中圆**（禁 `scale()` / 禁 `opacity`，反馈用 `filter: brightness()`）；球页**禁原生 `title`**；球窗唯一显示入口 `show_ball()`，顺序 `show → unminimize → styles → delete_taskbar_tab`；「关窗 = 退出」必须显式写驻留理由；球相关守卫都在 `contract_tests.rs`，动 `floaticon.html` / `floatwin.rs` 前先看守卫。

## 本机环境硬限制（都会伪装成「代码坏了」）
1. **删除走安全垫片**：递归删除被拦，只有**逐文件 unlink** 能过，**配额 ≈50 次/轮** ⇒ 写产物前确认目录在忽略规则内（⚠️ 往 `tests/` 写一次性产物前先确认该子目录**已被 `.gitignore` 锚定**，否则 `git add -A` 会把它收进版本库）；跑构建**换全新输出目录**。
2. **`node -e` 不可靠**（stdout 被吞、退出码 0）⇒ 取证一律写**脚本文件**。
3. **PATH 上 `python` 是 reparse point** ⇒ Node `spawnSync('python')` **恒定 EBUSY**（需绝对路径）；本机 `--parity` 会留「未能验证」（CI 是 Linux，无此限）。
4. **系统临时区会被外部脚本清掉**（`%TEMP%\w2a\` 与 `target/debug/web2app.exe` 一起消失过）⇒ 唯一副本别只放临时区。
5. **改 UI 目视取证**：不能只跑 `cargo test` 就开 exe（不刷新）；WebView2 走 DWM 合成截不到 ⇒ 用 Electron 按真实尺寸渲染同一份共享 HTML 截图。

## 状态
- **仓库 v1.80**（球视觉走真逐像素 alpha + `WM_NCHITTEST` 圆形命中；tauri 2.12.1 / wry 0.57.0 / tao 0.37.1）；Electron M0–M7 全交付；Tauri 持续维护。
- ⚠️ **提交状态（2026-10-12 复核）**：`HEAD = be8b314`（10-12 19:44）**已含 v1.80 全套**；`e19b594` / `686df68`（10-11）已含 v1.72+ 球窗与 `floatprobe.rs` ⇒ **旧说法「v1.69–v1.80 均未提交」已失效**。⚠️ 工作区另有**并行进行中的代码改动**（**非文档轮次所改、未触碰**；核对时看到 `electron-app/src/main/{float-window.ts,sidecar.ts}`、`test/float-contract.test.ts`、Rust 侧 `webview-app/src/{debug_log.rs,bootstrap.rs,lib.rs}` —— **清单随时在变，要准的用 `git status` 现查**，其中 `float-window.ts` 是行尾混用文件）。⚠️ CI 与远端推送状态本机**查不到**（无 `gh`）⇒ 别臆断。⚠️ 本机 Electron 探针/GUI **间歇性可跑**（同一命令曾连续三次在 `whenReady()` 后即死、退出码 127；2026-10-13 又稳定跑出 `--probe` 150/150、`--probe --tabs` 47/47 与 4 张渲染截图）⇒ **别把某次死机当成产品缺陷去改代码**，换一轮重试即可。⚠️ v1.80 **真机目视仍待复验**。
- **测试基数（2026-10-13 实跑）**：Rust 工作区 **248 passed / 6 ignored** = resolve-core 19 + resolve-tool 22 + w2a-proxy 32 + 主 crate 175（10-12 为 240 / 主 crate 167）；Electron `unit` **99/99**、契约探针**单窗 150 / 页签 47**（时点值，以实跑输出为准；页签 39 → 47 = 首页页签那轮）。`--parity` 本机恒 EBUSY ⇒ payload 数只能靠源码统计（**43**）。
- ⚠️ Tauri 侧本地打包未实跑（无 tauri-cli，只有 `--dry-run`）；M4「另存为」未经真机人验；两壳版本均 `0.1.0`、**无 tag**；`core.autocrlf=true` 与根 `AGENTS.md` §3.4.2「统一 LF」冲突，**待决**。⚠️ `tests/float-frag-rc/`（79 个已跟踪文件的悬浮球一次性取证工作区）已由用户于 2026-10-12 从工作区删除，但**删除未暂存**（`git status` 79 个 ` D`）⇒ 需一次 `git add -A tests/` 才从索引除名；根 `.gitignore` 已补 `/tests/float-frag-rc/`。审计见 `docs/changes/`（旧三份 `docs/audit-2026-0*.md` 已于 2026-10-07 删除）。
