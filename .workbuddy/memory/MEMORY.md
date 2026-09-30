# web2app（双壳：Tauri 版 + Electron 绿色版）

`webview-app/`=Rust+Tauri2（含 Android）；`electron-app/`=仅 Win 绿色版。**单一真源**：`webview-app/src/{titlebar.js,loading.js,splash.html,shell.html}`、`dist/settings.html`、`icons/*`、`src/{proxy.rs,resolve.rs}`→sidecar `w2a-proxy.exe`；改任一 ⇒ 必跑 Electron `probe` + `probe --tabs` + Tauri 页面探针。

> ⚠️ 本文件只留**索引 + 最高频契约**：**细节不在这里扩写**，一律写进下面这些文件并在此留一行指针
> （它一旦膨胀就会被注入截断，等于全丢）。全量：`topics/repo-shape.md`（仓库/Git/CI 形态）·
> `topics/electron-app.md`（Electron 全量 + 15 条硬契约）· `topics/tauri-contracts.md` ·
> `topics/lessons.md`（**教训全量，排查前必扫**）· `docs/packaging-config.md`（配置契约唯一文档）·
> `docs/electron-size-reduction.md` · `topics/archive-*.md` · `YYYY-MM-DD.md`（逐日过程）。
> ⚠️ 凡「计数」（字段数 / 单测数 / 断言条数）**一律实跑取证再写**，或改写「随版本演进，实跑为准」
> —— 历史上抓到 10+ 处漂移，根因都是「加了代码没回头改文档」。

## 最高频 / 代价最大的 15 条

1. ⚠️⚠️ 两壳**必须同级**：跨壳相对路径（`--manifest-path ../webview-app/Cargo.toml`、`stage-assets.cjs` 取 `../webview-app/icons/`）禁改 ⇒ **禁止把 `electron-app` 挪进 `webview-app`**。
2. ⚠️⚠️ 配置字段改动必过三处对等实现：`.github/scripts/parse-packaging-config.py`（**唯一真源，18 键**）+ `electron-app/src/main/config.ts` + Rust 侧；布尔禁 `bool()`；`rejectUnknownKeys` 必须 `Object.prototype.hasOwnProperty.call`；`build_merged()` 里 `app_url`/`extra` 要在 `str()` 循环**之前**单独取。
3. ⚠️⚠️ 顶部空间 = **`html` 不滚、`body` 滚**；`Builder::header` 是 **append** 非 replace ⇒ 不跳过就是两个 `Origin`（页面能开、WS/POST 全挂）。
4. ⚠️⚠️ 注入脚本必须跑**主世界**（`webFrame.executeJavaScript`），否则标题栏画得出、按钮全点不动；严格 CSP 站点用 `adoptedStyleSheets`。
5. ⚠️⚠️ 探针起窗模式必须与断言组一一对应：`TABS_MODE = TABS_PROBE ? true : PROBE ? false : cfg.tabs_enabled`；**多页签唯一开关 = `tabs_enabled`**，只在最外层分叉一次。
6. ⚠️⚠️ 跑探针**必须**带 `--no-sandbox --in-process-gpu`，且**禁写 `--` 分隔符**；报 `ERR_FAILED(-2)` / GPU 崩 ⇒ 先怀疑参数与环境，不是代码。
7. ⚠️⚠️ 本机门禁**逐条**跑：`fmt --check` + `clippy --all-targets -D warnings` + `test --all` + `typecheck`/`build`/`probe`/`probe --tabs`/`smoke`（条数实跑取证）。Android 门禁本机**做不了**（无 NDK，`ring` build script 要 `aarch64-linux-android-clang`）⇒ 只能靠 CI。⚠️⚠️ **`cfg` 分叉只允许决定「数据」，不允许决定「可变性」**：`let mut` 若只在某个 `#[cfg]` 块里被改 ⇒ 那句静默不参与编译 ⇒ `unused_mut` ⇒ `-D warnings` 下编译失败。⚠️⚠️ **非 Windows 的 cfg 分支在本机是盲区** ⇒ **本地全绿 ≠ CI 会绿**；改了带 `#[cfg(...)]` 的代码必须显式说明「未在本机覆盖的分支」。代跑土办法与细节见 `topics/lessons.md`。
8. ⚠️⚠️「exe 同级」一律走 `installRoot()`；`config.json` **随 `app.asar` 进包**（`out/config.json`），**已无 `extraFiles`**。userData/sessionData/logs/crashDumps 四项一起改且在 ready 前。
9. ⚠️⚠️ **同一文件的多处编辑禁止同批发出**（后写者基于旧快照 ⇒ 前一条静默丢失，工具却报 success）；改完立刻 grep 关键标记。`String.replace` 的替换串含 `$&` 之类必须改 `split/join`。
10. ⚠️⚠️ **「消灭了缺陷」≠「满足了需求」**：动手前把需求翻成**成对的可观测判据**。**取证方法错 ⇒ 结论错** —— GUI exe 用 `Start-Process -Wait -PassThru` 取 `ExitCode`（`& "app.exe" --probe` 是假绿），跑前清 `ELECTRON_RUN_AS_NODE`。⚠️ **改完 Rust 源码必须 `cargo build` 再取证**：`cargo test`/`check` **不刷新** `target\<profile>\<bin>.exe`。
11. ⚠️⚠️ **IPC 注册表 `.invoke_handler(...)` 全项目只能有一次** —— 它是**替换**不是追加 ⇒ 分模块写两次会静默清空前一批命令（Windows 上表现为 `Command save_settings not found`）。平台差异写**命令项上的 `#[cfg(...)]`**。守卫 = `src/lib.rs::ipc_command_registry`。详见 `topics/tauri-contracts.md` §8。
12. ⚠️⚠️ **运行期只有一个 JSON：`settings.json`**（设置三键平铺顶层 + `window` 段几何；旧 `window-state.json` **不迁移**）。**两个写入方（设置页保存 / 关窗记几何）必须读-改-写** —— 整体覆盖会在「挪窗口 + 改设置」时静默吃掉一方。⚠️ **落点 = 数据目录**（`config.rs::store_dir` / Electron `storeLocation`）：一律 `<data_dir>\settings.json`；旧位置降级为**只读候选**（新位置优先、不迁移不删除），且**读-改-写的「读」也走同一份候选** —— 否则升级后第一次保存把老设置**清零且不报错**。守卫 = `store::tests`（13 条）+ 探针 4 条。详见 `topics/tauri-contracts.md` §10。
13. ⚠️⚠️ **设置界面 = 无边框窗 + 铺满整窗面板 + 正文滚动**（两壳共用 `dist/settings.html`；v1.27 由「居中 + 四周底衬」改**满铺**，元素 id 与脚本一字未改 ⇒ 两壳同时生效）：`html/body` 都不滚，**唯一滚动容器是 `.panel-bd`**（`flex:1;min-height:0;overflow-y:auto`）。⚠️ **无边框 ⇒ 拖动必须有 capability**（`start-dragging`；少了它不报错、只是拖不动）。⚠️ **标题栏「设置」按钮在远程页面里只能用核心插件事件**（`core:event:allow-emit` → `app.listen("web2app:open-settings")`；多页签标签栏是本地页面才可直接 `invoke`；Electron 侧 `win:open-settings`/`tabs:open_settings` —— **面板词表 / 白名单 / IPC 名三处同改**）。按钮顺序 `[←][→] 标题 [⚙][—][▢][✕]`（设置键在**最小化左侧**）；位置判据**只比几何**、不看 DOM 顺序。详见 `topics/tauri-contracts.md` §11。
14. ⚠️⚠️ **capability 与跨文件字面量有静态守卫**（`lib.rs::capability_contract` 3 条；用 `include_str!` ⇒ **文件缺失即编译失败**）。⚠️ **新增守卫必须做反向对照**（故意改坏 → 期望 FAILED → 还原后比 sha256），否则不知道它是否真咬人；守卫里的被测字面量要 `concat!` 拆段写，否则**自我满足**。⚠️ clippy 输出里 `error deleting lock file … 拒绝访问 (os error 5)` 是 Windows 上 target 目录竞争的**环境噪声**，判真假用 `grep -E "^error"`。
15. ⚠️⚠️ **本地按 JSON 打包 = `scripts/pack-local.mjs`**（Node，零第三方依赖，需 ≥18；**不再需要 Python** —— 打 Electron 包必然已有 node，而本机系统里根本没有 Python）。⚠️⚠️ **代价 = 解析逻辑两份实现**（它是 `parse-packaging-config.py` 的**复刻**而非复用）⇒ **改任一侧必须同步另一侧并跑 `--parity`**；两条护栏 `--selftest`/`--parity` 已接入 `ci.yml`。⚠️⚠️ **本地行为一律由 JSON 决定 —— 不允许「JSON 之外还得有人再做一步」**：`icon_url` 非空即自动下载并应用（判据与 CI 同源；`--no-icon` 只是逃逸口）；输出目录**不自己清**（本机删除有 50 次/轮的配额，自己清一次上千文件会把配额烧光，反噬 electron-builder 的收尾删除）—— 先让它自己清，**被 `safe-delete` 拦住才自动换 `release/local-<时间戳>/` 重试**。⚠️ 会改写 tracked 的 `tauri.conf.json` / `icons/` 与未纳管的 `electron-app/config.json` ⇒ 一律备份 + `finally` 还原（**`finally` 要求进程活着走到**；启动即打印备份目录）。⚠️ 环境坑：受限网络加 `--config.electronDist`；`--out` 相对路径**以 `electron-app/` 为基准**（按仓库根解析会落到未被忽略的根 `tests/`）；传给子进程的 env **必须剥掉 `ELECTRON_RUN_AS_NODE`**（否则子进程里任何 **electron 可执行文件**退化成 node 模式）。Android 本机**做不了**，脚本明确拒绝。契约见 `docs/packaging-config.md` §7.3。

## 状态

- **仓库已到 v1.32**（变更记录在根 `README.md`，逐版快照 + 门禁实测数）。Electron **M0–M7 全部交付**；Tauri 版持续维护。
- ⚠️ **本轮改动尚未提交**：`scripts/pack-local.mjs`（新增）+ `scripts/pack-local.py`（已删）+ `ci.yml` / `README.md` / `docs/packaging-config.md`。提交前人工复核。
- ⚠️ **Tauri 侧本地打包仍未实跑**：本机没装 tauri-cli（`cargo install tauri-cli --version "^2"`）；只有 `--dry-run` 证据。
- 全流程审计（文档/实现一致性 + 计数漂移 + capability 守卫）见 `docs/audit-2026-09-23.md`（§一–§七为**当日时点快照，按纪律不回改**）。
- ⚠️ M4 下载「另存为」对话框**未经真机人工验证**（探针只验挂载），首次实跑需人工确认。两壳版本号都 `0.1.0` 且无 tag ⇒ 首次发版前先定前缀（`tauri-v*` / `electron-v*`）。`core.autocrlf=true` 与 README「统一 LF」冲突，**待决**。
- ⚠️ **本机环境三条硬限制**（都会伪装成「代码坏了」）：
  1. **删除走安全垫片**：递归删除一律被拦、`.asar` 上恒定失败，只有**逐文件 unlink** 能过，且**有配额
     （≈50 次/轮，超了静默开始抛错）** ⇒ ①往某目录写产物前先确认它**在忽略规则内**；②**别指望事后能删**；
     ③受限环境里跑构建要**换全新输出目录**，不要删旧的（删了会反噬宿主进程自己的收尾删除）。
  2. **`node -e` 不可靠**：stdout 被吞（退出码 0、零输出），`env -u … node …` 同样吞 ⇒ 取证一律写**脚本文件**再跑。
  3. **PATH 上那份 `python` 是会话注入的 reparse point**：从 Node 里 `spawnSync('python', …)` **恒定 `EBUSY`**
     （需兜底到绝对路径），连续 spawn 到第 ~37 次再次 `EBUSY` ⇒ 本机 `--parity` 可能留少量「未能验证」的用例
     （**CI 是 Linux，无此限制**）。
- ⚠️ **写出去的文件是 CRLF**：要逐字节还原某个文件时不能靠 Write 工具，得用 Python 直接写字节（据此按 `LF + 尾换行` 还原过 `electron-app/config.json`，sha256 与基线吻合）。
- ⚠️ **改 UI（共享 HTML / 注入脚本）后的目视取证**：不能只跑 `cargo test` 就开 exe（**不刷新 exe ⇒ 看到旧界面**），且 WebView2 走 DWM 合成截不到内容 ⇒ 可靠路径是**用 Electron 按真实窗口尺寸渲染同一份共享 HTML** 截图；改动前基线用 `git show HEAD:<path>` 导出旧副本，同脚本同尺寸才有可比性。
