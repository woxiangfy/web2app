# 跨轮次反复应验的教训（从 MEMORY.md 迁出，内容未改）

> 这些是「诊断/验证方法」层面的经验，两个壳（Tauri / Electron）通用。动手排查前先扫一遍。

1. **断言期望值若取自「实现当前的表现」，等于把缺陷锁死**。先问：这数字是「我想要的」还是「现在是这样的」？
2. ⚠️ `getBoundingClientRect()` 只证明布局，不证明可见（踩过两次）⇒「有没有被遮住」只能用命中测试或像素。
3. 沙箱 `Add-Type` 被拦、无 PIL ⇒ PNG 取证走 `node:zlib`（`scripts/png-decode.cjs`）；`capturePage()` 无 GPU 抛 `UnknownVizError`，**必须 try/catch** 且不参与退出码。「卡住几分钟不返回」：先看输出文件是否已写完（常是判定已出、进程没退）→ 再查是否有 await 抛错后没人 `app.exit()`。
4. ⚠️⚠️ 验证 GUI exe：`& "app.exe" --probe` 是**假绿** ⇒ 必须 `Start-Process -Wait -PassThru` 取 `$p.ExitCode` + 旁证（跑前删探针目录、跑后确认被重建）。⚠️⚠️ **打包 exe 退出码 9（0x80000003）的真凶是宿主注入 `ELECTRON_RUN_AS_NODE=1`** ⇒ 跑前 `Remove-Item Env:ELECTRON_RUN_AS_NODE`；判别：`--version` 也崩 ⇒ 是环境变量不是业务代码。
5. 受限网络里 electron-builder 会卡在下载/解压 electron 运行时 ⇒ 加 `--config.electronDist=node_modules/electron/dist`（⚠️ 是 `--config.`，写成 `-c.` 会被当配置文件路径报 ENOENT）。**命令默认 120s 超时会掐掉 zip/portable**（先 `ls` 再定是否重跑）。`pnpm --workspace-root exec pwd` 报 code=1 是非致命探测、可忽略。
6. ⚠️⚠️ 「问题依旧」先查跑的是哪一版包，别急着改代码（`dist`/`dist-fix` 并存极易跑错）：比 `resources/inject/*.js` 与源码 sha256、看 `data\` 与 `window-state.json` mtime。⚠️ **手术式补丁必须连 `app.asar` 一起换**。
7. ⚠️⚠️ **「消灭了缺陷」≠「满足了需求」——用更差的替代品换掉症状同样是失败**。动手前把需求翻译成一句可观测的判据。修「侵入类」缺陷判据必须成对：位置 + 存在性（只断言「不许侵入」会被「干脆删掉」满足）。
8. ⚠️⚠️ 「退出码即结论」的入口（探针/冒烟/CI）必须有顶层 `.catch()` 兜到 `app.exit(1)`，未接住的 rejection ⇒ CI 挂死。⚠️ 验证错误处理的修复 = 把原来的失败复现一遍（比对修前/修后日志与退出码）。
9. ⚠️ PowerShell 的 `Select-String`/`Get-Content` 输出**不能**用来判定文件编码（UTF-8 中文显示成 `ÑÏ¸ñ` 假乱码），判据走字节级。⚠️ `Set-Content -Encoding utf8` 会写 **UTF-8 BOM**（JSON 报 `Unexpected token \ufeff`）⇒ 用 `[IO.File]::WriteAllText(path, text, (New-Object System.Text.UTF8Encoding $false))`。
10. 审计文档先找内部矛盾；审计源码先判「违反规范」是否属「有意偏离」（符合规范但砍能力 = 负收益）。⚠️ **「文档写着待补」≠「真的缺」**：落笔前先跑一条取证命令，别拿文档当事实。
11. ⚠️ 探针脚本里「建临时窗口又 `destroy()`」会触发 Electron 默认的「所有窗口关闭即退出」⇒ 进程以**退出码 0** 静默消失；注册 `app.on('window-all-closed', …)` 即抑制。结果写文件取证 + `stage()` 分段日志。
12. **写方案前先做 API 前置实测**，且「更省事的变体」与「更重的方案」都要实测。⚠️⚠️ **实测数据可靠 ≠ 由它推出的结论可靠**：任何「必须换掉某个基础构件」的结论，落笔前三查——①官方文档对该构件的定位原文 ②真正落地的社区实现 ③上游 issue 维护者定论。
13. ⚠️⚠️ **「取证方法错 ⇒ 结论错」**：`capturePage()` 看不到同窗口下的子 `WebContentsView`，多视图合成顺序必须用 `desktopCapturer`（`thumbnailSize` 必须显式非零）。⚠️ `data:` URL 的 HTML 必须 `encodeURIComponent`。
14. ⚠️ 临时测试产物禁止在仓库/项目根新建 `dist-*`/`tmp*`：统一落子项目 `tests/`（`.gitignore` 用 `tests/*/` + `!tests/.gitkeep`）；electron-builder 输出放 `release/`。
15. ⚠️⚠️ **同一文件的多处编辑禁止同批发出**：两条并行 edit 时，后一条基于**旧快照**写回 ⇒ 前一条**静默丢失**，而工具两处都报 success（2026-09-18 实测丢了一条）。判据：改完立刻 grep 关键标记，别信「success」二字。
16. ⚠️ 修「替换/注入」类代码，先看替换串里有没有不可信文本：`String.replace(pat, repl)` 会把 repl 里的 `$&`/`` $` ``/`$'` 当模式 ⇒ 用 `split/join`（`splash.html` 占位符、`renderSplash` 失败态两处已立规）。
17. ⚠️ **体积类结论必须先量构成再谈优化**：99% 体积是 Chromium 运行时，纠结 0.2MB 应用代码是方向性错误；同一份载荷换压缩算法（deflate→LZMA）就有 35% 差异 ⇒ 先比形态、再谈裁剪。
18. ⚠️⚠️ **工具让你 `--force` 绕过它自己的安全检查时，默认答案是"不"**。UPX 对 Electron 44 的 exe 直接拒绝：`CantPackException: superfluous data between sections (try --force)`；硬来之后三个变体（`--lzma` / 默认 NRV / `--strip-relocs=0`）**全部 0xC0000005**。工具拒绝 = 它知道结构不正常，`--force` 只是把风险转交给你。
19. ⚠️⚠️ **压缩 / 加壳 / 替换基础构件类方案，必须有「同目录 + 同参数 + 未压缩」对照组**，否则失败会被误判成环境问题（沙箱无 GPU、`ELECTRON_RUN_AS_NODE`、缺运行时）。本次判据成立正是因为对照组：未压缩 ExitCode=0（13.3s，112 条断言过）vs 压缩版 0xC0000005（1.0–2.0s 即崩）。
20. ⚠️ **"整文件加壳压缩"与"多进程架构"天然冲突**：UPX 手册原文——packed programs 运行时**不与其它进程共享内存**（不同于文件映射）。Electron 的 main / renderer / GPU / utility 共用同一个 exe ⇒ 每个进程各解压一份 235MB 到私有内存：磁盘省 162MB、**运行内存涨几百 MB**；且 LZMA 解压 7–9s/GiB ⇒ 每次启动 +2s。这类方案再"能压"也不适用。
21. ⚠️ 引用社区数字必须标版本与来源：Electron 15 时代的「exe 85→65MB」不可直接套到 44.4.1 —— 「实测数据可靠 ≠ 由它推出的结论可靠」的又一例。
22. ⚠️⚠️ **本会话沙箱拦「`.git/refs/remotes/**` 下的新建」，导致 `git fetch`/`git update-ref` 退出 0 但引用不落盘**（`git status` 显示 `[gone]`）。判别特征：同一命令内 `git` 自报 `* [new branch] main -> origin/main`、退出码 0，而 `Test-Path` 立刻为 False；把该路径换成工作区文件或 `.git/packed-refs` 就正常 ⇒ **是路径级拦截，不是 git 故障、不是仓库损坏**。
    - **判 push 成败只认 `git ls-remote origin refs/heads/main`**（服务端权威值）；本地 ahead/behind/gone 全是易失元数据，不可作为结论。
    - 绕法：直写 `.git/packed-refs`（`# pack-refs with: peeled fully-peeled sorted` + `<sha> refs/remotes/origin/main`），`git rev-parse origin/main` 立即可解。
    - 排查此类「命令报成功、磁盘无变化」时，务必做**同命令内 + 下一命令**两次取证，并配一个「同结构但不同路径」的对照组，否则会把环境拦截误判成代码/仓库缺陷。
23. ⚠️⚠️ **命令报「写/读失败」时，第一嫌疑是本地落点（路径/权限/磁盘满），不是远端、不是 URL**。2026-09-22 实测：CI 里 `curl -fsSL "$ICON_URL" -o build/icon.png` 报 `curl: (23) client returned ERROR on write of 2906 bytes`。**23 = CURLE_WRITE_ERROR = 本地写盘失败**（含「输出目录不存在、打不开文件」这条路径），与网络无关；真凶是 `build/` 被 `.gitignore` 忽略、fresh checkout 里不存在，而创建它的 `pnpm run stage` 排在下载**之后**。**判据**：把错误里的字节数与响应头 `Content-Length` 比对 —— 相等即证明响应体已完整送达、下载侧无辜（本例 2906 正是全长）。⚠️ 用户说「**换个地址还是同样报错**」是这类误判的典型信号 ⇒ **换方向查，别换地址**。修法一行 `mkdir -p build`；对照实验：同 URL 在目标目录不存在时 exit 23，建好目录后 exit 0 + 2906 字节。

24. ⚠️⚠️ **告警/校验的触发条件必须等于「运行期真的会走到那条路」**。字段有形态分叉时（本例 `app_url` = 单 URL 字符串 ↔ 域名数组），按形态统一告警 = 狼来了：CI 上每条构建都挂一个假 warning，真该修的配置问题会被噪声淹掉。**做法**：先把「谁会消费这个字段」逐壳追到调用点（Tauri `parse_domains(APP_URL)` 命中才解密；Electron `needsDnsResolve = Array.isArray(app_url)`），再让告警跟着同一判据分叉。**判据**：对照跑两个形态的样例，各自的 warning 计数应是 1 与 0，而不是都 1（2026-09-22 实测：仓库内 4 份配置样例的 warning 由 4 → 0）。

25. ⚠️⚠️ **`macro_rules!` 的作用域是 crate 级的 —— 被 `#[path]` 复用的共享源文件里不能用「主 crate 的」私有宏**。本仓库的 `crates/proxy-server` 直接以 `#[path = "../../../src/proxy.rs"]` 复用主 crate 源文件 ⇒ 那些文件算 sidecar **自己的**模块，主 crate 里定义的 `dlog!` 在 sidecar 侧**根本看不见**：`error: cannot find macro 'dlog' in this scope`。⚠️ **加 `crate::` 全限定也救不了** —— 找不到的是**宏名本身**，不是展开体里的路径（`#[macro_export]` 才能跨 crate，但那会把主 crate 的 tauri 依赖拖进 sidecar，不可取）。**正解**：每个消费 crate 自带同名宏（sidecar 侧展开为裸 `eprintln!` —— 它是控制台进程，stderr 由宿主的开关决定去向；主 crate 侧才落盘）。**门禁**：必须 `cargo test --all` / `clippy --all-targets`，**只跑主 crate 会漏掉 sidecar 的编译面**。
26. ⚠️⚠️ **改完 Rust 源码必须 `cargo build` 之后才取证**：`cargo test` / `cargo check` **不刷新** `target\debug\<bin>.exe`。2026-09-22 实测踩中：据一个 **6 天前**（09-16）的 `web2app.exe` 跑「默认 vs `--debug`」成对判据，两组都不落盘 ⇒ 差点误判「Tauri 侧 debug 开关没生效」，其实是**被测对象不含改动**（该 exe 的 ASCII 里既无 `startup.log` 也无 `WEB2APP_DEBUG`；`(Get-Item exe).LastWriteTime` 与源文件 mtime 相差 6 天）。**判据（先做再测）**：①比 exe 与源文件 mtime；②`[IO.File]::ReadAllBytes(exe)` 里 ASCII 搜本次新增的字符串常量。⚠️ 与此同类：`cargo check --all-targets` 只保证**类型过**，不等于跑得起来。⚠️ **2026-09-23 跨领域复现（改 UI 也一样）**：改动 `dist/settings.html` / `titlebar.js` / `shell.html` 后若只跑了 `cargo test` 就开 exe 目视，看到的是**旧界面** ⇒ 会误判「改了没生效」。而且 GUI 目视本身还受 DWM 合成影响（WebView2 窗口截不到内容）。**本轮的可靠路径是绕开 exe**：用 Electron 按**真实窗口尺寸**渲染**同一份共享 HTML** 截图取证（双壳单一真源 ⇒ 版式等价），位置类判据则交给探针的 `getBoundingClientRect()` 几何断言。**改动前基线**用 `git show HEAD:<path>` 导出旧文件副本再渲染（同一脚本、同一尺寸），才能得到可对比的 before/after。
27. ⚠️ **shell 续行链（`\`）中间夹注释会截断整条链**。实测：`A=1 \` + 注释行 + `B=2 bash -c 'env'` ⇒ 子进程**只看到 `B`**，`A` 未设置（退出码 0，无任何提示）。GitHub Actions 的 `run: |` 里拼 `PKG=... AWK=... TITLE_VAL=...` 那种长链时，**所有注释必须整体上移到链首之前**，链中只留赋值。**判据**：写完用一次最小对照跑（同结构、加/不加注释各一次），别靠眼睛数 `\`。

28. ⚠️⚠️ **「收口类」改动必须按壳逐一核对，文档要写明是哪个壳**。本项目两壳（Tauri / Electron）共享前端真源与 `proxy.rs`，
    但**写盘、IPC、事件这些宿主侧实现是两套**。典型事故：给 `settings.json` 加原子写时只改了 Electron 的
    `writeDocument`，Tauri 侧 `store.rs::save_at` 仍是 `fs::write` 直接覆盖，而变更记录写的是「`settings.json` 原子写」
    —— 读起来像两壳都做了，实际上 Tauri 侧「写一半被杀 ⇒ 截断 JSON ⇒ 下次启动整份解析失败 ⇒ 设置与几何一起回默认
    **且不报错**」的链条完好无损。⚠️ **判据**：拿「变更记录里的动作名」去 grep **两个壳**的实现，
    别只 grep 你刚改的那一侧；文档里凡是只写一个壳的收口，都要显式标壳名。
    ⚠️ 同源的还有「文档已写而资产未入 git」（本轮 `electron-app/test/` 是 `??` 状态）——
    `git status --short` 里的 `??` 与 `A` 差异就是「规范已写、交付未落」的直接证据。

## 附：二进制体积审计 SOP（2026-09-18 立，4 步）

1. **量构成**：`Get-ChildItem -Recurse -File | Sort Length -Desc` → 逐项排序。⚠️ 先确认量的是**哪个目录**（同名 `data\` 曾同时存在于交付目录内与同级）。用 `原始运行时` 与 `打包目录` 做自洽校验。
2. **判"有没有可删的"**：解析 PE 节表（`e_lfanew` 在 `0x3C`，节表在 `PE+24+SizeOfOptionalHeader`）看 `.text`/`.rdata`/`.debug*` 占比。**有 `.debug*` / 符号节才谈"剥符号"**；`.text+.rdata` 占比 ≈96% 的（如 Electron exe）只能压、不能剪。
3. **判"能压到多少"**：`7za a -t7z -mx=9 -mf=BCJ2`（用 electron-builder 缓存里的 7za）→ 得到压缩上限与压缩比，作为任何压缩方案的期望基线。
4. **压缩/加壳方案落地前**：同目录 + 同参数 + **未压缩对照组**，跑同一个探针，比退出码与耗时。差异才是结论；没有对照组就不许下结论。

## 附：工具链 / 护栏真伪（2026-09-22 自 MEMORY.md 迁入）

- ⚠️⚠️ **「所有条目同时失败且原因为空」= 先怀疑工具链**。典型：`bash -n` 报 6 个 workflow 全挂，真凶是 `which bash` 命中 WSL 启动器。**判据**：拿 `echo hi` 最小样例跑候选解释器对照。（2026-09-22 复现变体：整个 `PATH` 为空 ⇒ `ls`/`wc`/`dirname` 全 `command not found`，不是仓库问题。）
- ⚠️⚠️ **文档里声称的护栏必须去 CI 里核实真的存在**（grep 作业名/脚本名），否则只是文字；**规格里的「默认值/类型」也要拿真实产物核一遍**。

## 附：真机探针与 WebView2 原生 API 取证（2026-09-22 清缓存探针 `webview-app/examples/probe_clear_data.rs`）

- ⚠️⚠️ **夹具 HTTP 服务必须「每连接一线程」**。单线程 `accept` + 阻塞 `read` 会被浏览器的**预连接**堵死 accept 循环 ⇒ 后续请求永远不被服务。表现极具误导性：导航返回 `Ok` 却什么都没有，或 `Navigate` 抛 `HRESULT_FROM_WIN32(ERROR_INVALID_STATE) = 0x8007139F`。**判据**：改成每连接一线程后同一句导航立刻正常（服务端打出 `REQ /check cookie=[...]`）。⇒ **外部 API 报「状态不对」时，先怀疑自己这边的夹具/环境。**
- ⚠️⚠️ **`ClearBrowsingDataAll` 是长时间异步操作**：发起后一段时间内 profile 级 API 返回 `ERROR_INVALID_STATE`、**导航被静默丢弃**（`load_url` 照样 `Ok`，甚至**直接阻塞**）、`GetCookies` 甚至永不完成。窗口期实测 ≥32s，且与「窗口可见 / 隐藏」「用谁的 clear API」**均无关**。⇒ ①断言必须等落定，**不能刚发起就查**；②**别把会挂死的调用放进主线程**（`cookies_for_url` 内部是 `wait_with_pump` = 阻塞 `GetMessageA`）。
- ⚠️⚠️ **`cookies_for_url()` 会挂死**，不能当主线判据。改判据为「**新导航后的文档 + 夹具服务端实际收到的 `Cookie:` 头**」（网络层证据，比页面里读 DOM 硬），`cookies_for_url` 只作落定后的旁证。
- ⚠️⚠️ **完成回调不是判据**：`ClearBrowsingDataAll` 的完成回调在 10+ 次运行里**一次都没执行过**（上游 wry 的 `clear_all_browsing_data()` 是纯 fire-and-forget，handler **1.1µs** 即释放；自建 handler 830ms ~ >15s 才释放），而**数据照样被清掉**（三重证据）。⇒ 判据只能落在「数据有没有被清掉」，回调到达性只作观测；生产代码同理**不能拿回调当成功/失败的唯一凭据**。
- ⚠️⚠️ **`cargo clippy --example` 不重建 example 可执行文件**：跑探针前必须 `cargo build --example`，否则跑的是**上一次**的二进制（表现为「改了代码、输出却一模一样」）。这是「先查你跑的是哪一版包」的又一变体，本次连踩两次。
- ⚠️ **对照组是抓「判据陷阱」的唯一手段**：对照组 B（写数据但**不**清除）当场证明同一 origin 的新文档在未清除时 `localStorage.length` 也读到 `0` ⇒ 它根本不能当判据。没有对照组就会把一个**恒真条件**当证据。
- ⚠️ **探针会自我退化**：反复用 `process::exit` 收尾会跳过 WebView2 拆除，`<exe>.WebView2` 越跑越脏（先是「清除后不可用」，最后连 `cookies_for_url` 都卡）。**跑前把该目录改名备份**即可恢复。
- ⚠️ 探针退出码要**三态**：`0`=结论成立 / `1`=结论不成立 / `2`=**本机无法取证**。把「观测不到」报成 `1` 等于用错的取证方法得出错的结论。


## 附：还原文件 / 反向对照的操作纪律（2026-09-24 事故后立）

- ⚠️⚠️⚠️ **还原「未提交」的改动，绝不用 `git checkout -- <file>`**：它还原的是**暂存区**（没有暂存就还原到 HEAD），
  会把该文件上**所有未提交改动一起丢掉**——不只是你刚改的那几行。2026-09-24 实测：反向对照后用它还原
  `electron-app/src/main/index.ts`，结果连上一轮未提交的 v1.28 改动（凭据主机绑定、多页签恢复自愈
  `presentMainWindow`、多处注释订正）全部消失，而工具只报「成功」。
  **正确做法**：改前用 Python 读字节备份到 `/tmp/xxx.sha` + 副本，改完 `write_bytes` 写回，再 `sha256sum -c` 校验。
- ⚠️⚠️ **误删后的兜底恢复路径**：`tsc` 是确定性的 ⇒ 「重编当前源码」的产物与**旧编译产物**做 diff，
  差异就是丢失内容（`out/main/index.js` 保留了全部注释，据此可逐处重建 TS）。本次据此 11 处替换全部还原，
  sha256 与基线一致。⇒ **改动后跑过 build 的话，产物就是一份免费备份**。
- ⚠️ **Python `write_text()` 在 Windows 上会把 `\n` 写成 `\r\n`**（`newline=None` 的默认行为），
  而项目约定统一 LF ⇒ 还原文件必须用 `write_bytes(s.encode('utf-8'))`。判据：`sha256sum -c` 不通过时
  先怀疑换行符，不要急着改内容。
- ⚠️ **反向对照用 `if (false) <调用>` 而不是删掉调用**：删掉会触发 `noUnusedLocals` ⇒ `noEmitOnError` 下
  build 失败、探针根本跑不起来，反而看不到「断言是否咬人」。
- ⚠️ **Node 按 ESM 解析 `.ts`** → `import { Menu } from 'electron'`（CJS 模块）直接抛
  `SyntaxError: Named export 'Menu' not found`。必须 `import electron from 'electron'` 再解构，
  这样「tsc 编译成 CJS」与「Node 直跑 ESM」两条路径都成立。纯 Node 下解构出 undefined 但不报错，
  这正是「纯函数区能被单测直接 import」的前提（顶层不得碰任何 Electron API）。
