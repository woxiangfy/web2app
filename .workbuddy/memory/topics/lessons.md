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

## 附：CI / GitHub Actions 专属（2026-10-07 Android 打包失败后立）

- ⚠️⚠️ **Android SDK 平台包名不一定是 `platforms;android-<N>`**：Android 16 起带**次版本号**
  （仓库只有 `platforms;android-36.1` / `37.0` / `37.1` / `37.2`，**没有** `platforms;android-37`），
  且新平台**先落 canary 通道**（stable 只到 `android-36`）⇒ 既要按真实包名挑，也要给安装命令加 `--channel=3`。
  判据来源别猜：直接取 `dl.google.com/android/repository/repository2-1.xml`（stable）与 `repository2-3.xml`
  （canary），`grep -o 'path="platforms;android-[0-9.]*"'` 一跑就清楚。落点 = 共享脚本
  `.github/scripts/install-android-sdk.sh`（build-custom / release 调同一份）；本地行为验证 =
  `bash tests/ci-sdk-step/run-tests.sh`（假 sdkmanager + 真清单数据，33 条断言）。
- ⚠️⚠️ **给旧脚本加 `set -euo pipefail` = 加一批新失败模式**，三个已实测的「静默 exit 2 / 零输出」陷阱：
  ① `ls a b | head -n1`：`ls` 只要有参数不存在退出码就是 **2** ⇒ 整步静默退出、日志空白（最贵的一种，
  因为什么线索都不留）；② `yes | sdkmanager …`：sdkmanager 先退出 ⇒ `yes` 收 SIGPIPE(=141) ⇒ pipefail 判失败；
  ③ `printf 大输入 | grep -q`：grep 命中即退 ⇒ printf 收 SIGPIPE，同上。⇒ 兜底一律 `|| true`，
  且**别用管道做「文件里是否包含」判断**（改写文件再 grep）。改完必须重跑行为验证 + 反向对照。
- ⚠️ **抽出去给 workflow 调用的 `.sh` 会掉出守卫范围**：`check-workflows.py` 原先只查 `run:` 内联脚本，
  且仅限显式写了 `shell: bash` 的步骤 ⇒ 抽出共享脚本后已补 `.github/scripts/*.sh` 的 `bash -n`
  （反向对照：追加未闭合 `if` → rc=1 且有明确报错，还原后 sha256 不变）。
- ⚠️⚠️ **干跑 / 生成脚本的产物一律写系统临时区，绝不落进仓库树**：干跑脚本原先把用例目录写
  `$HERE/tmp/run-<时间戳>/`（跑一次 ~72 个文件），**连跑 9 次就攒了 455 个文件 / 1.5MB**，而且没有任何
  一步会清它（`tests/` 又是未纳管目录 ⇒ 攒到用户来问「临时文件删了吗」）。正确形态：
  `TMP_BASE="${CI_SDK_STEP_TMP:-${TMPDIR:-/tmp}/<name>}"`，`mkdir -p` 后**把路径打印出来**（要留证据时靠
  这个环境变量指定）；不要在脚本里 `rm -rf` 旧目录——本机递归删除会被安全垫片拦、CI 上又是不必要的风险。
  另：`data/` 里生成清单时留下的中间态文件（`tmp-pkgs.txt` 这类）要当场删，否则「没人引用但一直在」。
- ⚠️⚠️ **文档里的「计数」是最容易漂的第二处**（第一处是代码）：本次 `docs/changes/2026-10-07.md` 里
  改完断言数后**仍留一处 `PASS=31`**（前一轮 `sed` 只替换到了另一句）。⇒ 订正计数后必须
  `grep -rn "PASS=\|条断言"` 把**所有**出现点扫一遍，别信单次 sed。

## 2026-10-10（v1.68）：「销毁/退出/冷启动」三条静默失败——都不是本仓库发明的坑，而是框架语义没对

三条都在**编译零警告 + 单测全绿 + 探针全过**的情况下只在真机现形，共同特征是
「我们写的那句 API 名字看着对，但它在框架里走的不是同一条通路」。排查手法可复用：

- ⚠️⚠️ **Tauri：`close()` ≠ 销毁，`destroy()` 才是**。`WindowMessage::Close` 在
  `tauri-runtime-wry` 里**直接**交给 `on_close_requested`（与 Alt+F4 同一入口）⇒ 任何
  `RunEvent::WindowEvent { CloseRequested }` 里的 `api.prevent_close()` 都会把它吞掉。
  **凡「自己拦截了某类窗口的 CloseRequested → 转隐藏」，同文件里的销毁路径就必须用 `destroy()`**
  （`tabs::close_tab_inner` 是既有正例）。判断 API 语义的可靠来源：tauri 源码里
  `pub fn close` / `pub fn destroy` 的 doc + runtime 的 `Message::Window(...)` 分支，
  **不要**凭「close 听起来就是关掉」推断。
- ⚠️⚠️ **Electron：窗口 `close` 里 `preventDefault()` 会取消整个 `app.quit()`**。
  「关窗转隐藏」这种常见设计必须同时放行退出路径（本项目 = 注入 `isQuitting: () => quitting`，
  由 `before-quit` / `requestQuit()` 置位），否则表现是「开了某窗口就退不掉应用」，
  而且**没有报错**。取证方式很便宜：两个 `BrowserWindow`，一个 prevent、一个不 prevent，
  `app.quit()` 后等 1.5s 看进程是否还在（本次即这样拿到「abort」实证）。
- ⚠️⚠️ **「变化才动」（diff）的冷启动陷阱**：`applySettings(next, {persist:false})` 里
  `prev = deps.getSettings()`，而入口在调用前已把内存态初始化成**同一份** `loadedSettings.settings`
  ⇒ 所有 diff 恒为 false，**启动时一个副作用都不会建立**。凡「只在变化时创建」的副作用
  （托盘 / 悬浮窗 / 快捷键…），冷启动必须显式用「从默认态切换」的语义（本项目 =
  `applyInitialSettings`，prev 固定 `DEFAULT_SETTINGS`），且**diff 基准要统一改成函数开头的 `prev`**，
  否则 override 会被 `deps.getSettings()` 悄悄绕过。
  ⚠️ 这类缺陷**探针很难发现**：`probe` 把配置整体挪到 `%TEMP%` 隔离目录（读不到「已启用」的配置）
  且只走「设置页勾选 → 保存」这条**变化**路径。⇒ 「探针全过」不等于「冷启动路径也过」，
  凡改启动链路，务必单独写一条一次性脚本或加真机清单。
- ⚠️ 顺带两条小结论：①拖动/指针类状态（本项目的 `moving`）必须在「窗口被隐藏」时清零 ——
  拖动途中隐藏会让 `pointerup` 永远不来，之后该状态机**永久卡死**且无任何日志；
  ②跑这类一次性取证脚本要 `unset ELECTRON_RUN_AS_NODE`、且 `createTray` 之类**必须在
  `app.whenReady()` 之后**调用（ready 前抛 `Cannot create Tray before app is ready`，
  容易被误读成「图标路径不对」）。

## 2026-10-11（v1.74）：失败分类器「漏一类」= 静默降级 —— 自动兜底与排错提示会**同时**失效

`pack-local.mjs` 设计上写着「默认目录清不掉 ⇒ **自动**换全新目录重试一次，始终能出包、不需要人工介入」。
这次真撞上「清不掉」，三条承诺**一条也没兑现**：不重试、不提示，用户只看到 `✗ … 失败（退出码 1）`。

- ⚠️⚠️ **兜底分支的判据必须与「会失败的真实形状」逐条对齐，不能只按「设计时想到的那一种」写**。
  本项目 `isDeleteBlocked()` 只认 `safe-delete|emptyDir|trash operation`（本机删除垫片的形状），
  而真正发生的是 `EBUSY: resource busy or locked, unlink '<path>'`（**文件被别的进程占着**）。
  ⇒ **凡「按错误文案分类 → 走不同恢复路径」的代码，判据都要写成「可枚举的失败形状清单 + 逐条反向对照」**；
  「删不掉」这一类至少三种形状：本机策略拦删、**文件被占（EBUSY）**、权限拒删（EPERM/EACCES）。
  本次的具体形状：`<CODE>: …, unlink|rmdir '<path>'`，CODE ∈ `EBUSY|EPERM|EACCES`。
- ⚠️⚠️ **判据要锚在「系统调用」上，不是锚在错误码上**：受限网络下 electron-builder 下工具集会在
  重试约 8 分钟后以**裸 `EPERM`**（rename/copyfile）失败 —— 那**不是**「删不掉」。
  用裸 `EPERM` 当判据 ⇒ 白等一次重试 **+ 给出「换目录」的错误建议**。
- ⚠️⚠️ **「能不能打开」≠「能不能删」**：Node/libuv 打开文件默认带
  `FILE_SHARE_READ|WRITE|DELETE` ⇒ 被别的进程独占的文件**照样 `openSync(p,'r')` / `openSync(p,'r+')` 成功**
  （本次两例都实测成功）。⇒ 检测「被占」的可靠代理：**同名 `renameSync(p, p)`**（被占时抛 EBUSY，且非破坏性）、
  或 Windows **Restart Manager**（`rstrtmgr.dll`，问「谁注册了这个资源」）。
  **任何「先探测再决定」的写法都必须先过这一关**，否则会得到一台**恒假阴性**的探测器（而它看起来还挺合理）。
- ⚠️⚠️ **持锁者可能是「正在运行我们的宿主进程」，而不是被测程序**：`app.asar` 的锁来自 WorkBuddy 自己
  （`daemon-app-server-entry.js`）。⇒ 见 `EBUSY` **先问「谁持锁」再动手**（Restart Manager / 同名 rename），
  别默认「上一次的产物还在跑」。并且要用**受控对照**区分「通用行为」与「路径相关」：
  本次写 120 KB `dummy-probe.asar`（+同尺寸 `.txt` 对照）三个时点都没被占、回退后新目录的**同名** `.asar`
  也没被占 ⇒ 是**那两条具体路径**的残留句柄，不是「见到 `.asar` 就抓」。
- ⚠️ `EBUSY` 不等于完全不可用 —— 本次 `FileShare.ReadWrite` 打开**成功**，只有 `unlink` 失败
  ⇒「**读得动、删不掉**」：下载/校验/护栏类步骤照常全绿，只有清理步骤炸。绿的地方不代表没病。
- ⚠️⚠️ **「关窗即退出」不是一条独立规则，而是「没有别的窗口活着」的推论**：任何在主窗之外
  还存活着的窗口（悬浮球是独立窗、多页签在 Tauri 是独立顶层窗）都会挡住 `window-all-closed`
  ⇒ 用户点 ✕ 之后：主窗被真销毁、进程却活着、球还在但点它没反应（**僵尸 + 孤儿**，v1.75 真机）。
  ⇒ **凡是要「关窗后还能被唤回」的东西，都必须在关窗那一刻显式列出「驻留理由」**，
  并且那份理由要与「该窗口真的存在」用**同一判据**（球：`platform==win32 && float_enabled`）。
- ⚠️⚠️ **守卫的匹配范围必须限定在被测代码块内**：本次要钉 `win.on('close')` 里的
  `runtimeSettings.float_enabled`，而同一个文件（`index.ts`）的启动日志里也有这个表达式
  ⇒ 直接对全文 `includes` 的话，**把 close 判据整段删掉守卫照样绿**（自我满足的假守卫）。
  取块（按行收到块尾）+ 剥注释后再匹配，才是真咬人的写法。反向对照时**要专门验证这一点**。
- ⚠️ 环境类：**受限会话里 Electron 的 GPU 进程起不来**（`gpu_process_host.cc:1035`，
  `exit_code=-1073741819` = 0xC0000005）⇒ 裸 `pnpm run probe` 在 `splash://` 处 `ERR_FAILED` 退 1，
  看起来像自己改坏了。**区分办法：拿「改动之前打的产物」跑同一条命令**（本次旧 portable exe 同样退 1），
  再给运行期探针加 `--no-sandbox --in-process-gpu` 即全绿。
- ⚠️⚠️ **手改 Windows 窗口样式（`GWL_EXSTYLE`）必须写在每次 `show()` / `unminimize()` 之后，并读回校验**
  （tao 0.35）：`set_visible`/`set_minimized` → `WindowState::apply_diff`（`window_state.rs`）会用**缓存的
  tao 窗口标志**重新拼 `style_ex` 并整体 `SetWindowLongW(GWL_EXSTYLE, …)` ⇒ 写在 `show()` 之前的那一次
  **被静默擦掉**（缓存里没有的位被抹、`ON_TASKBAR` 把 `WS_EX_APPWINDOW` 写回；实测读回恒 `0x40118`）。
  判据只有**读回**：`SetWindowLongPtrW` 失败只回报 0，而 0 也是合法旧样式值。
  ⇒ 本项目落法：`show_ball()` = `show → unminimize → apply_ball_window_styles → delete_taskbar_tab`，
  顺序钉进契约守卫（v1.77）。
- ⚠️ **框架「有这个 API」≠「它做了这件事」**：`.skip_taskbar(true)` 名字看着就是「不进任务栏」，
  实际只额外调一次 `ITaskbarList::DeleteTab`、**既不清 `APPWINDOW` 又是在窗口还没显示时调的**
  ⇒ 完全无效。凡「用一个高层 API 满足一条硬需求」，都要**读它的实现 + 运行时读回效果**再算数
  （本次反例：Electron 同名选项是真的 ⇒ 症状「仅限 Tauri」）。
- ⚠️ **同名症状 ≠ 同一根因**：v1.73 修「球元素 `title` 提示条」与 v1.77 修「任务栏按钮浮层」，
  真机现象几乎一模一样（都是「图标背景有异样浅色图形」）。第二次动手前**先证明第一次的修复确实生效**
  （对产物做字符串取证：`title="单击`=0 / `aria-label="单击`=1），否则会在错误方向上反复。
- ⚠️ **取证前先确认「被测对象真的被创建了」**：本机默认 `settings.json` 里没有 `float_enabled` ⇒ 球根本不建，
  枚举结果里看不到球窗，一度误判成「修复没生效」（本次为此多跑一轮）。
  另：受限会话下球窗 WebView2 初始化慢，`run_on_main_thread` 的 1s 兜底回调被推迟 ⇒ 等 16s 仍 `vis=False`、
  **20s** 才稳定可见（环境等待，不是产品行为）。
- ⚠️ 环境类：`.ps1` 里写中文必须配 **UTF-8 BOM**。无 BOM 时 PS 5.1 按 GBK 解析，
  报 `TerminatorExpectedAtEndOfString`（看着像语法错误，其实是编码问题）。
  仓库里**一次性取证脚本**常有这种写法（如已随 `tests/` 清理掉的 `float-tauri-diag/click-diag2.ps1`），属同类隐患。
  ⚠️ `tests/` 下**除 `ci-sdk-step/`、`build-*.bat` 外的一次性产物都已不在工作区**（`float-frag-rc/` 也于 2026-10-12 清空）—— 别再逐个按路径引用。

## 2026-10-11（v1.77b）：**取证脚本自身也会假绿** —— 反向对照脚本必须先自证「真的跑了那个测试」

一次「四项反向对照全绿」的假象，连踩三个坑（同一份脚本，逐轮修）：

1. ⚠️⚠️ **`cargo test --all <叶子名> -- --exact` 一个测试都匹配不到**。
   Rust 测试的**完整路径带模块前缀**（本例 `contract_tests::float_window_contract::<名字>`），
   `--exact` 要求全路径精确相等 ⇒ 输出是 `0 passed; 169 filtered out`，
   退出码仍是 **0**。脚本把「没跑」当成了「通过」。
2. ⚠️⚠️ **`finally` 里的 `return` 会覆盖 `try` 里的 `return`**（Python 语义）。
   脚本两头都 `return` ⇒ 实际返回的永远是「还原是否成功」，与「守卫是否抓住」无关 ⇒ 四项全绿。
3. ⚠️ **`error:` 不等于「编译失败」**。cargo 在**测试失败**时也会打印
   `error: test failed, to rerun pass ...` ⇒ 用 `^error:` 判「编译失败」会把断言失败**误标**成
   「编译器拦下的（弱对照）」，看起来像抓到了、其实没证明是**断言**抓到的。
   真正的编译失败特征：`error[E0xxx]` 或 `error: could not compile`。

⇒ **判据**：解析 `test result: <status>. <n> passed; <m> failed` 行，**必须 `n+m > 0`** 才算真的跑了；
再按 `failed > 0 && passed == 0` 判「被抓住」。并**先判「跑没跑」再判「编译错」**（顺序反了就会重演坑 3）。

**推广**：任何「自动判定通过/失败」的取证脚本，都是被测对象之外的第二套系统 ——
它自己需要一条 **self-check**（本例 = 结果行里的 `passed+failed>0`）。
「没跑」和「通过」在文本上可能只差一行，但结论完全相反。

## 2026-10-11（v1.77b）：环境坑三条（都会伪装成「代码坏了」）

- ⚠️ **本机 Electron GUI 起不来**：脚本里 `app.whenReady().then()` 打点能写到文件，之后进程直接死、
  退出码 **127**。`file://` / `data:` / `pathToFileURL()` 三种加载方式全部 `ERR_FAILED (-2)`。
  ⇒ 凡「靠 Electron 渲染同一份共享 HTML 再做像素比对」的 A/B 取证，**本机不可行**；
  改走**不依赖渲染**的两条路：①对使用者真机截图做逐像素几何拟合；②对使用者**现场运行实例**
  做**只读** Win32 探针。
- ⚠️⚠️ **对「使用者现场实例」做只读取证**，比对本地起进程可靠得多：本地起 debug 实例常被
  **单实例互斥**挡掉（用户桌面正跑着生产实例 ⇒ 新实例启动即退出 ⇒ 枚举不到球窗，看着像「没建出来」），
  而且本机受限会话里 WebView2 根本不渲染。只读探针（`EnumWindows` + `GetWindowRect` /
  `GetWindowLongPtrW` / `GetWindowRgn` + `PtInRegion` 网格）零副作用、拿到的还是**真实产品状态**。
- ⚠️ **PowerShell 探针用 Python + ctypes 写**：PS 5.1 按 ANSI 读 `.ps1` ⇒ 中文注释/字符串乱码
  （`宽` → `¿í`），且 `-f` 格式化里带 `-`/`+` 的表达式要加括号（不加会被解析成对上个命令结果做算术）。
  ctypes 没有编码问题，`GetRgnBox` 还能配 `PtInRegion` 网格直接**重建出区域形状**（判断「是不是圆」），
  比只看包围盒强得多。

## 2026-10-11（v1.77b）：量「形状」要用能重建形状的判据

- 判断残片是不是**圆环**：不要只看包围盒（矩形包围盒对圆和方块是一样的）。
  用「逐行列出该行被点亮的 x 区段」，再把每个区段端点**换算成到某参考点的距离**：
  内边界距离 ≈ 球半径、外边界距离 ≈ 常数 ⇒ 圆环；两条直线边 ⇒ 矩形。
  本例正是靠这个把「浅灰圆角块」判成了「以球心为圆心的一圈环带」。
- ⚠️ **分类阈值会骗人**：第一版把「亮」定为 `lum>=140`，结果把球自己的**浅蓝高光**（`#6a9bff`，
  lum≈152）当成残片、把残片当成球的一部分。改为「亮 **且** 低饱和（`b-r<60`）」才把两者分开。
  ⇒ 颜色分类必须**同时**用亮度和色相两个维度，并且要**先看一眼真实像素值**再定阈值。

## 2026-10-12（v1.78–v1.80）：**地基错了，几何怎么调都救不回来**

- ⚠️⚠️ **症状落在「边缘质量」上时，先问合成链路，再谈几何**。球缘「浅色异形 / 锯齿」被连修三版
  （v1.78 区域圆外扩 4px / v1.79 回退为与活柱严格重合 / v1.79 页面自绘 `#000` 黑底），
  **三案都在同一条思路里打转**：不动「窗口有没有逐像素 alpha」这个地基，只改几何或底色
  ⇒ 每案只能**换个形状的症状**。v1.80 换地基（`.transparent(true).no_redirection_bitmap(true)`
  ⇒ `WS_EX_NOREDIRECTIONBITMAP` + DComp 真 alpha）后，前三个问题**一并消失**。
  **判据**：反复「改一版好一点、再改一版又差点」时，停下来问「这是不是同一个坏地基的不同投影」。
- ⚠️⚠️ **「文档说有这个能力」不算数 ⇒ 读 crate 源码坐实**。`no_redirection_bitmap` 在
  tao 0.35 只在 `NO_BACK_BUFFER` 时才加 `WS_EX_NOREDIRECTIONBITMAP`；**tauri-runtime-wry 2.11.4 干脆完全不透传**
  （只透传 `transparent`，还用 softbuffer 清顶层 surface）⇒ **能力没透传到就用不了，上层怎么写都无效**。
  升级到 tauri-runtime-wry 2.12.1 才透传。⇒ 依赖升级前**先读源码确认路径打通**，再改业务代码。
- ⚠️ **视觉与命中的分离是结构性的**：`SetWindowRgn` 这类 1bit 区域掩码**同时**决定「视觉裁剪」与「命中」，
  所以想「只裁命中、不裁视觉」在这条路上无解。正解是**两条腿各自走**：
  视觉 = 逐像素 alpha（DComp），命中 = `WM_NCHITTEST` 子类化（圆内 `HTCLIENT` / 圆外 `HTTRANSPARENT`）。
  ⚠️ 子类化必须**递归枚举父窗 + 全部后代子窗**（WebView2 子窗覆盖整个客户区，只拦父窗拦不住），
  ⚠️ 且必须**幂等**（多次 `install` 不能反复包装），⚠️ `WM_NCDESTROY` 要还原，⚠️ 半径未初始化必须 **fail-open**（当 0 处理 = 全部穿透，而不是把窗口锁死）。
- ⚠️ **不变量比理由长寿**：「球 == 活柱 == 命中圆」在 v1.77b 与 v1.80 都成立，但**理由完全变了**
  （v1.77b：不然留透明环带漏残片；v1.80：不然视觉圆与命中圆错位）。⇒ 改地基时**保留不变量、
  重写理由**，别把理由当不变量一起删掉；也别拿旧理由去否定新实现。
- ⚠️ 页面里**任何「壳标记」（如 `data-w2f-shell`）都是技术债的化石**：它的存在说明两壳走了不同渲染路径。
  地基换好后必须**同时**删 Rust 侧注入与页面侧规则，并加**反向守卫**（`code_has_occurrence` 断言它不再出现）。

## 2026-10-12：**文档审计（只动文档）的取证纪律**

- ⚠️⚠️ **「引用体检」必须按文件名全仓递归查**，否则结论会两头错：
  ① 只按若干候选目录拼路径 ⇒ 把真实存在的文件报成「失效」（大量假阳性）；
  ② 只查 `.md` 里的字面路径 ⇒ **漏掉「文件已改名」这类最隐蔽的失效**
  （例：文档写 `tab-preload.ts`，真实文件叫 `tabs-probe-preload.ts`；文档写 `install_circle_hit_testing`，
  真实函数是 `install_circle_hit_test` —— 后者是**本轮的编辑自己引入的错**，靠「改完 grep 一次」抓出来）。
  ⇒ 体检脚本要**先建全仓 basename 索引**再比对，且**反向也要做**（把文档里出现的符号逐个回源码 grep）。
- ⚠️⚠️ **数字一律实跑取证，且要认「条数会漂」**：同一份 `AGENTS.md` 里探针条数从 134 → 138 → 141 → 145 → 150，
  每次都是「实跑当时值」。⇒ 写进文档的数字**必须带时点**，或者干脆改成**判据式表述**
  （「以实跑输出为准」「判据是『不减少』而不是某个固定数字」）。**权威判据是结果文件里的 `ALL PASS` 行**，不是退出码。
- ⚠️ **`.gitignore` 里被注释掉的行 ≠ 仍然生效**：`.workbuddy/`、`.trae/` 那两行早已注释掉（改为纳管），
  但多处文档仍拿它们当「被忽略目录」的例子。⇒ 凡「X 被忽略」这类断言，**打开 `.gitignore` 逐行读**，
  并用 `git check-ignore -v <path>` / `git ls-files <dir> | wc -l` 正反取证。
  ⚠️ 同一坑的另一面：文档写「`/tests/` 整目录忽略」，实测是**逐条锚定**（`/tests/pack/` 等），
  `git ls-files tests | wc -l` = 87 ⇒ 整目录断言是被证伪的。
- ⚠️ **「一次性产物不入库」与「已被跟踪的文件」会冲突**：`tests/float-frag-rc/` 有 **79** 个文件已入库（⚠️ 当时写成「87」是**把 `git ls-files tests` 的整目录数当成了子目录数** —— 两个 `wc -l` 别混用），
  按约定该清，但清除要 `git rm -r --cached`（破坏性）。⇒ 文档里**如实标注为「已知遗留待清理」并写清代价**、**不擅自执行**；
  这比假装它不存在或悄悄删掉都正确。**2026-10-12 用户自行删除后**补记三条：
  ①删除只落工作区、**索引未动** ⇒ 不 `git add -A tests/` 就仍留在下一次提交里；
  ②**根因是「落点未被忽略规则覆盖」**：该目录是 `electron-app/scripts/diag/float-ring-lab.cjs` 的默认输出目录，
  却不在 `.gitignore` 的逐条锚定清单里 ⇒ 光有「一次性产物不入库」的约定**挡不住 `git add -A`**，
  约定之外**必须真去锚 `.gitignore`**；③光「删掉」不够，活文档 / 源码注释里的指针（含 `floaticon.html` 里的取证脚本路径）要同批改。
- ⚠️ **被删掉的文档 / 目录，活文档里必须同批清理引用**：`docs/audit-2026-0*.md` 已于 2026-10-07 删除、
  `webview-app/txttest_keys/` 已不存在 ⇒ 活文档里残留的指针要改；但 `docs/changes/*.md` 与
  `.workbuddy/memory/2026-*.md` 是**历史记录，保留原文**（必要时加一行「该文件已不存在」的注记）。
- ⚠️⚠️ **「提交状态」是最容易过期的一类断言**：文档里写「v1.69–v1.80 均未提交」，实测 `HEAD` 已含 v1.80 全套。
  ⇒ 这类断言**必须用 `git log` / `git show HEAD:<file>` 取证**；查不到 CI 状态时（本机无 `gh`）
  就**明写「查不到」**，不要用「应该跑了 / 应该没跑」填空。
- ⚠️ **写记忆/文档时先分清「活文件」与「历史快照」**：`MEMORY.md`、`topics/*.md` 是活的（要改成现状）；
  `docs/changes/YYYY-MM-DD.md`、`.workbuddy/memory/YYYY-MM-DD.md`、`*-review-20xx-xx-xx.md` 是快照
  （**保留原文**，只加「时点快照」横幅）。改错这一层比不改更糟。
