# web2app（双壳：Tauri + Electron 绿色版）

`webview-app/`=Rust+Tauri2（含 Android）；`electron-app/`=仅 Win 绿色版。**单一真源**：`webview-app/src/{titlebar.js,loading.js,splash.html,shell.html}`、`dist/settings.html`、`icons/*`、`src/{proxy.rs,resolve.rs}`→sidecar `w2a-proxy.exe`；改任一 ⇒ 必跑 Electron `probe`+`probe --tabs`+Tauri 页面探针。

> ⚠️ 只留**索引 + 最高频契约**；细节写进 `topics/{repo-shape,electron-app,tauri-contracts,lessons}.md`（lessons=**教训全量，排查前必扫**）、`docs/packaging-config.md`、`docs/changes/`、`YYYY-MM-DD.md`。凡「计数」**实跑取证再写**。

## 契约
**A 形态/门禁**
1. ⚠️⚠️ 两壳**必须同级**（跨壳相对路径禁改）⇒ 禁止把 `electron-app` 挪进 `webview-app`。
2. ⚠️⚠️ 配置字段改必过三处：`parse-packaging-config.py`（唯一真源 18 键）+ `electron-app/src/main/config.ts` + Rust 侧；布尔禁 `bool()`；`rejectUnknownKeys` 用 `hasOwnProperty.call`；`build_merged()` 里 `app_url`/`extra` 在 `str()` 循环**前**取。
3. ⚠️⚠️ 本机门禁逐条：`fmt --check`/`clippy -D warnings`/`test --all`/`typecheck`/`build`/`probe`/`probe --tabs`/`smoke`；Android 本机做不了 ⇒ 靠 CI。⚠️⚠️ **`cfg` 分叉只决定「数据」不决定「可变性」**（`let mut` 只在某 cfg 块被改 ⇒ `unused_mut` ⇒ 编译失败）；**非 Win cfg 分支是本机盲区 ⇒ 本地全绿 ≠ CI 绿**。clippy 的 `os error 5` 是 target 竞争**噪声**，判真假用 `grep -E "^error"`。
**B 编辑/取证纪律**
4. ⚠️⚠️ **同一文件多处编辑禁同批发出**（后写者基于旧快照 ⇒ 前一条静默丢失却报 success）；改完 grep。`String.replace` 替换串含 `$&` 改 `split/join`。
5. ⚠️⚠️ **「消灭缺陷」≠「满足需求」**：先翻成**成对可观测判据**。**取证方法错 ⇒ 结论错** —— GUI exe 用 `Start-Process -Wait -PassThru` 取 `ExitCode`，先清 `ELECTRON_RUN_AS_NODE`；**改完 Rust 源码必须 `cargo build` 再取证**。
6. ⚠️⚠️ **新增守卫必须反向对照**（改坏→期望 FAILED→还原比 sha256）；被测字面量用 `concat!` 拆段，否则**自我满足**；⚠️ **针会被注释满足** ⇒ 匹配前**先剥注释**。文件是 CRLF ⇒ 逐字节还原用 Python 写字节。
**C 窗口/注入/设置**
7. ⚠️⚠️ 顶部空间 = **`html` 不滚、`body` 滚**；`Builder::header` 是 **append** 非 replace ⇒ 不跳过 = 两个 `Origin`（页面能开、WS/POST 全挂）。
8. ⚠️⚠️ 注入脚本必须跑**主世界**（`webFrame.executeJavaScript`），否则标题栏画得出、按钮点不动；严格 CSP 用 `adoptedStyleSheets`。
9. ⚠️⚠️ **`.invoke_handler(...)` 全项目只能一次**（**替换**非追加 ⇒ 写两次静默清空前一批，Win 上 `Command save_settings not found`）；平台差异写**命令项上的 `#[cfg]`**。守卫 `lib.rs::ipc_command_registry`。→ §8。
10. ⚠️⚠️ **运行期只有一个 JSON：`settings.json`**（三键平铺 + `window` 段）。**两个写入方（设置保存/关窗记几何）必须读-改-写**。落点＝`<data_dir>\settings.json`；旧位置＝**只读候选且「读」也走同一候选**（否则升级后首次保存清零不报错）。守卫 `store::tests`(13)+探针 4。→ §10。
11. ⚠️⚠️ **设置界面 = 无边框 + 铺满整窗 + `.panel-bd` 唯一滚动容器**（两壳共用 `dist/settings.html`）；无边框 ⇒ 拖动需 capability `start-dragging`（少了不报错、只拖不动）。标题栏「设置」在远程页只能用核心插件事件（`core:event:allow-emit`→`app.listen("web2app:open-settings")`；Electron `win:open-settings`/`tabs:open_settings` —— **面板词表/白名单/IPC 名三处同改**）。按钮 `[←][→] 标题 [⚙][—][▢][✕]`。capability 守卫 `lib.rs::capability_contract`(3)。→ §11。
**D 打包/CI**
12. ⚠️⚠️ **本地打包 = `scripts/pack-local.mjs`**（Node 零依赖 ≥18，**不需 Python**）。解析逻辑**两份实现**（`parse-packaging-config.py` 的复刻）⇒ 改一侧必同步 + 跑 `--parity`。行为全由 JSON 决定；输出目录**不自己清**（删除配额 50 次/轮）⇒ 被 `safe-delete` 拦住才换 `release/local-<时间戳>/`。改写 tracked 的 `tauri.conf.json`/`icons/`/`electron-app/config.json` ⇒ 备份 + `finally` 还原。env **必须剥 `ELECTRON_RUN_AS_NODE`**；`--out` 相对**以 `electron-app/` 为基准**。→ packaging-config §7.3。
13. ⚠️⚠️ **CI 装 Android SDK = `.github/scripts/install-android-sdk.sh`**（两 workflow 同一份）。平台包不一定叫 `platforms;android-<N>`（16 起带次版本号：实测只有 `36.1/37.0/37.1/37.2`，**无 `android-37`**；新平台**先落 canary**）⇒ 按真实包名挑 + 必要时合并 canary 加 `--channel=3`；补 `platforms/android-N` 软链；回落最高可用并改写 gradle。⚠️⚠️ 三个 `|| true` 是实测的（加 `set -euo pipefail` = 加新失败模式）。验证 `bash tests/ci-sdk-step/run-tests.sh`。→ lessons 末节。
**E 悬浮窗/球（v1.67–1.72）**
14. ⚠️⚠️ 悬浮窗三条**静默失败**修法：①Tauri 销毁必须 `destroy()` 不能 `close()`（Close 与 Alt+F4 同入口，被 `prevent_close+hide` 吞）；②Electron close 默认 `preventDefault` 转隐藏，而**「close 被 prevent」会取消 `app.quit()`** ⇒ deps 注入 `isQuitting` 放行；③冷启动副作用**不能复用** `applySettings(next,{persist:false})`（diff 恒 false）⇒ 用 `applyInitialSettings`（prev 固定 `DEFAULT_SETTINGS`），**对托盘图标同样成立**；④`hide` 必须清拖动标志。⚠️ 探针覆盖不到 ⇒ 真机四步见 `electron-app/AGENTS.md`。
15. ⚠️⚠️ **球真机 harness（v1.72）= `src/floatprobe.rs`**（仅 Win，`--float-probe`）：`inject_page_event` 注入 8 事件（**唯一跳过「球页 JS 把事件发出去」**），**27** 断言 + 6 诊断（v1.72=24；v1.76 右键语义升级 +3）。⚠️⚠️ **本机 `build()` 约 50% 永久挂死**（换目录/换 UDF/清 `msedgewebview2` 全无效）⇒ **环境抖动非产品缺陷**；⚠️⚠️ 用 `tests/run-float-probe-v176.ps1`（既有 `float-v172-rc/run-float-probe.ps1` 在本机**读不到退出码**、打印 `EXIT=` 空值 ⇒ 跑出 0 也不认、一路重试到假失败；新版用 .NET `WaitForExit(ms)` + 以结果文件 `ALL PASS` 为判据）。退出码 `0`/`>0`/`97` 环境/`98` 看门狗。⚠️⚠️ **失败收尾的 `labels` 是分诊金钥匙**：`["float","main"]` = build 其实成功、只是没等到可见 ⇒ 「窗口不存在」≠「build 失败」；且**判「是不是我的改动」先看失败点相对改动点的位置**。⚠️⚠️ **某步从「复用」改成「重建」⇒ 所有下游等待都要重论证**（v1.76 右键改销毁 ⇒ 复原要重建，固定 `sleep(700)` 必然赶不上 ⇒ 改轮询到可见；连带 `WATCHDOG_SECS` 240→330、`BUILD_WAIT` 120→**180**（本机下午球窗「建到可见」稳定 >120s；等待上限放宽是对「环境多慢」的容忍、不是对产品行为的断言）。⚠️ 探针模式**必须跳过单实例插件与全局快捷键**（否则已有实例在跑 ⇒ 零输出 + 退出码 0 的**假绿**）。⚠️ 假主窗漏 `data_directory` ⇒ 建窗 9ms 退化到 60s+ 挂死。
16. ⚠️ 球实测：`GetWindowRgnBox` **含端**（`[0,70)²`→`(0,0,69,69)`）；`monitor_from_point` 用 **`MONITOR_DEFAULTTONULL`**（测试点须放显示器**内**）；**WebView2 UDF = `%LOCALAPPDATA%\com.example.web2app\EBWebView`，与 exe 目录无关**。⚠️ **`tabs.rs::create_host_window` 曾漏 `data_directory`（v1.72 修）** ⇒ 宿主数据落默认目录、破坏绿色版承诺；**同一不变量在多处实现必须逐处核对**。
17. ⚠️ Tauri `CreateEllipticRgn` ↔ Electron `win.setShape(circleShapeRects(56))`（`@experimental` ⇒ **必须 try/catch 降级**；区域只决定命中，视觉仍由 CSS 圆 + transparent）；页面侧 **250ms 双击去抖**。→ `docs/changes/2026-10-11.md`。
18. ⚠️⚠️ **本机 50% 卡死 ⇒ 探针只能「取证」不能当 CI 门禁；且即便全绿也不覆盖渲染** —— 渲染需**普通桌面会话**（受限会话 WebView2 不渲染，`msedgewebview2.exe` 进程数恒 0）。
19. ⚠️⚠️ **「关窗 = 退出」被任何主窗之外的存活窗口改写**（球的独立窗/页签窗都会挡住 `window-all-closed`）⇒ 必须显式写「驻留理由」：`(tray && min) || (win && float_enabled)`（v1.75，两壳同语义；Tauri 用 `cfg!` **布尔**，`#[cfg]` 分叉会让非 Win 的 `float_icon` 变未使用变量 ⇒ `-D warnings` 编译失败）。⚠️ **没启用托盘时托盘的「退出」不存在** ⇒ 设置页必须有「退出程序」= `quit_app`（Tauri `app.exit(0)` / Electron `requestQuit`＝先置 `quitting` 再 `app.quit()`；四处同改：HTML 字面量/ALLOWED/`tauri:` 处理器/入口接线，`smoke.ts` 命令清单也要有）。⚠️ 守卫**必须限定在被测块内**：`runtimeSettings.float_enabled` 在启动日志里也出现 ⇒ 全文 `includes` 是自我满足的假守卫。

19. ⚠️⚠️ **球页不得挂原生提示（`title`）**：球窗 always-on-top，而原生提示条是**独立非置顶窗口** ⇒ 提示条只会被画在球窗**之后**、从透明环带漏出浅色残片（真机症状「图标背景有异样图形」）；球窗仅 56 逻辑宽、提示条必然与之重叠 ⇒ **结构性不可用**，改 `role="button"` + `aria-label`（操作说明另有设置页承载）。守卫 `contract_tests::float_ball_has_no_native_tooltip` + Electron `float-contract.test.ts`（都只判「球元素那一个标签」）。⚠️ **取证范式**：Electron 复刻**同一份** `floaticon.html`（透明 + alwaysOnTop + `setShape` 圆）+ 真实鼠标 `SetCursorPos` + `PIL.ImageGrab` 的「hover / 移开 / 再 hover」三态对照 ⇒ **差异 bbox 即 hover 专属元素**（脚本留档 `tests/float-tooltip-rc/`）。→ `docs/changes/2026-10-11.md` v1.73。

20. ⚠️⚠️ **球窗「真不进任务栏」= `show_ball()` 是唯一显示入口**（v1.77）：`.skip_taskbar(true)` **不够** —— tao 的 `ON_TASKBAR` 按「窗口无父」置位 ⇒ 建窗写死 `WS_EX_APPWINDOW`，`DeleteTab` 又在 `visible(false)` 时调 ⇒ 球是**带着任务栏按钮**被显示的（读回 `ex=0x40118`），按钮的原生提示条被置顶球窗压在身后、从透明环带漏残片（真机「背景异样」，仅 Tauri；Electron `skipTaskbar` 是真移除）。⚠️⚠️ **顺序硬要求：`show() → unminimize() → apply_ball_window_styles() → delete_taskbar_tab()`** —— tao `apply_diff` 在显示路径用**缓存标志整体重写 `GWL_EXSTYLE`**，写在 `show()` 之前**等于没改**（`APPWINDOW` 被写回、另两位被抹；v1.72 的 `NOACTIVATE` 因此一直未生效）。⇒ **通用范式：手改 `GWL_EXSTYLE` 必须在每次 `show()`/`unminimize()` 之后重补 + 读回校验**（`SetWindowLongPtrW` 失败只回报 0，而 0 也是合法旧值）。球窗另置 `.title("")`。读回 `0x8000198`。守卫 ⑥b/⑥c。→ `docs/changes/2026-10-11.md` v1.77。

21. ⚠️⚠️ **球页 `#w2f-ball` 必须铺满活柱**（球径 = `ICON_SIZE_LOGICAL` = 区域圆直径，v1.77b）：窗口的**区域是「内接于活柱的圆」**（现场只读探针：`GetRgnBox=(0,0)..(55,56)`、`PtInRegion` 网格两端递减 ⇒ 直径 56），球只要比它小，两者之间就留一圈**透明环带** ⇒ 把窗口后面的东西透出来：**Tauri/WebView2 底层非真透明 ⇒ 浅色残片**（真机「背景异样」第三次报告）；Electron 真透明 ⇒ 只透桌面（「仅限 Tauri」的答案）。⇒ **推论：凡「球内元素」都不得靠 `transform: scale()` 放大**（球已顶到区域边界，`scale>1` 立刻被裁 = 没有反馈、放大那圈又是环带）⇒ 反馈改用 `filter: brightness()`；**半隐藏态也不得用 `opacity`**（整颗球半透明 = 自毁这层保护）。守卫 `contract_tests::float_page_ball_fills_the_live_pillar`（球宽高从常量抠值，禁 `scale(`/`opacity`）+ Electron 同源。⚠️ `ITaskbarList` **任何方法前必须先 `HrInit()`**（否则 `DeleteTab` 返 `E_FAIL` 且什么都不做 —— v1.77 漏了、还用 `let _ =` 吞掉）。→ `docs/changes/2026-10-11.md` v1.77b。

## 本机环境硬限制（都会伪装成「代码坏了」）
1. **删除走安全垫片**：递归删除被拦，只有**逐文件 unlink** 能过，**配额 ≈50 次/轮** ⇒ 写产物前确认目录在忽略规则内；跑构建**换全新输出目录**。
2. **`node -e` 不可靠**（stdout 被吞、退出码 0）⇒ 取证一律写**脚本文件**。
3. **PATH 上 `python` 是 reparse point** ⇒ Node `spawnSync('python')` **恒定 EBUSY**（需绝对路径）；本机 `--parity` 可能留「未能验证」（CI 是 Linux 无此限）。
4. **系统临时区会被外部脚本清掉**（`%TEMP%\w2a\` 与 `target/debug/web2app.exe` 一起消失过）⇒ 唯一副本别只放临时区。
5. **改 UI 目视取证**：不能只跑 `cargo test` 就开 exe（不刷新）；WebView2 走 DWM 合成截不到 ⇒ 用 Electron 按真实尺寸渲染同一份共享 HTML 截图。

## 状态
- **仓库 v1.77b**；Electron M0–M7 全交付；Tauri 持续维护。⚠️ **v1.51 / v1.68 / v1.69–v1.77b 均未提交、未跑 CI**（本机已跑 Rust 三件套 + Electron 六项 + 两侧真机 harness）。⚠️ **本机 Electron GUI 起不来**（`app.whenReady()` 后进程即死、退出码 127）⇒ 渲染型 A/B 取证本机做不了。
- ⚠️ Tauri 侧本地打包未实跑（无 tauri-cli，只有 `--dry-run`）；M4「另存为」未经真机人验；两壳版本均 `0.1.0` 无 tag。`core.autocrlf=true` 与 README「统一 LF」冲突，**待决**。审计见 `docs/audit-2026-09-23.md`。
