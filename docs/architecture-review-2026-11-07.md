# webview-app 架构与组织结构审查 — 2026-11-07

> ⚠️ **本文是 2026-11-07 的时点快照**：模块行数、测试计数、问题清单均为当天实测值，后续演进不回填；
> 现行状态一律以根 `AGENTS.md`、`webview-app/AGENTS.md` 为准。
> 前次审查：[architecture-review-2026-10-25.md](architecture-review-2026-10-25.md)。
> 本次审查距上次已隔两轮较大变更（悬浮球 v1.81 改 Win32 分层窗口、Android 打包链路五轮修复、
> `cargo fmt --all`），**旧结论一律重新取样，不照搬**。

审查范围：`webview-app/` 整个 cargo workspace（四成员，含 Android 侧与补丁链）+ 与 `electron-app/`
共用的注入脚本 / 页面 + 代理与解析链路。关注维度：模块划分、职责边界、依赖方向与循环、
复用性（两壳共用件是否仍同源）、以及实际会咬人的问题（静默失效 / 校验缺口 / 测试盲区 / 文档漂移）。
原则：**最小重构**，不做为了整齐的大迁移。

---

## 0. 一句话结论

组织结构仍然健康：**跨壳真源零重复实现**（sidecar 用 `#[path]` 直接复用主 crate 源码）、
`nav.rs` 枢纽成立（六个功能模块对 `crate::bootstrap` 引用实测归零）、静态契约守卫覆盖面在全仓属上乘。
本轮新出现的问题主要集中在**三处**：

1. **新增代码（v1.81 悬浮球 Win32 原生层）的测试盲区**：891 行 / 24 处 `unsafe` 零单测，其中三个纯像素函数本可离线测；
2. **Android 补丁链的「后半段」无守卫**：7 个补丁脚本是否仍在 workflow 里被调用、顺序是否还对，`cargo test` 管不到；
3. **文档与实现漂移成片**（子模块计数、测试计数、共用件清单、workflow 条数），其中「共用件清单」会直接误导后续的双壳验证。

---

## 1. 现状实测

### 1.1 workspace 四成员

| 成员 | 角色 | 现状 |
| --- | --- | --- |
| 主 crate `web2app` | Windows/Android 应用 | `src/` 24 个 Rust 文件 + 8 个前端资源 |
| `crates/resolve-core` | 纯加密核（Ed25519 + AES-256-GCM + HKDF） | 零平台依赖，被主 crate 与 resolve-tool 共用 ✅ |
| `crates/resolve-tool` | 服务端 CLI（生成密钥 / 生成 TXT / 解密验证） | 薄壳 ✅ |
| `crates/proxy-server` | Electron 侧 sidecar（bin `w2a-proxy`） | `#[path]` 复用 `src/proxy.rs` / `src/resolve.rs`，**零复制** ✅ |

### 1.2 大文件实测（>400 行，仅列关键项）

| 文件 | 行数 | 备注 |
| --- | --- | --- |
| `webview-app/src/mobilebar/` | 1,280（5 片；原 1,260 + 分片头注释 20） | Android 底栏注入脚本，本轮按「核心 + 功能」拆分；仍**无 JS 侧测试** |
| `webview-app/src/floatprobe.rs` | 1,046 | 真机合成指针 harness（`--float-probe`），常驻生产二进制 |
| `webview-app/src/titlebar.js` | 1,019 | 双壳共用的自绘标题栏 |
| `webview-app/src/mobilebar.rs` | 909 | Android 底栏 Rust 侧 |
| `webview-app/src/floatwin/native.rs` | 891 | **v1.81 新增**：Win32 分层窗口 + 位图 alpha，24 处 `unsafe` |
| `webview-app/src/contract_tests/float_window_contract.rs` | 822 | 静态契约 |
| `webview-app/src/webview_hooks.rs` | 789 | WebView2 原生事件 |
| `webview-app/src/lib.rs` | 772 | 装配 + 主窗建窗 + 历史导航脚本 + 关闭语义 + 命令注册 |
| `webview-app/src/tabs/window.rs` | 756 | 多页签建窗 |
| `webview-app/src/store.rs` | 729 | 运行期唯一 JSON 的读-改-写 |

> 2026-10-25 的 P4 大文件拆分**确已落地**（`floatwin` / `tabs` / `proxy` 三个根文件都 ≤ 452 行）；
> 但悬浮球 v1.81 重写后又长出 `floatwin/native.rs`(891)，属于**新代码，不是旧债回潮**。

### 1.3 依赖方向（实测）

- `config.rs` 纯底层（零内部依赖，被 8 模块依赖）✅
- `nav.rs` 仍是唯一数据枢纽；`tabs` / `loading` / `mobilebar` / `webview_hooks` / `settings` / `debug_log`
  对 `crate::bootstrap` 的引用**逐个 grep 实测 = 0** ✅（2026-10-25 P2 的验收点仍成立）
- `proxy.rs` / `resolve.rs` 零内部依赖 —— 这是 sidecar `#[path]` 复用成立的前提 ✅
- 残余刻意互指 3 组（`settings↔store`、`settings↔floatwin`、`floatwin↔tray`），属同一份 `settings.json`
  读-改-写契约的代价，维持 10-25 的「刻意不动」结论。

---

## 2. 分级问题清单

严重度定义：**P1 = 会静默咬人**（功能悄悄坏掉而 CI 仍绿）；**P2 = 结构 / 复用 / 盲区**（短期不咬人但会累积）；
**P3 = 文案漂移与低价值整理**。

### P1-1 Android 补丁链「后半段」的调用与顺序无人守

- **位置**：`webview-app/src/contract_tests/android_pipeline_contract.rs:31-51`（清单只列 6 个脚本）；
  `.github/workflows/build-custom.yml`、`.github/workflows/release.yml`（各自重复 Python 调用块）；
  `.github/workflows/ci.yml`（selftest 循环）；
  `webview-app/docs/android.md`（声明「补丁链顺序固定」）。
- **问题本质**：脚本清单在三份 workflow 里各写一遍，契约测试只钉住
  `prepare-android-conf` / `apply-android-labels` / `patch-android-insets` / `set-android-apk-name` /
  `patch-android-back` / `install-android-ndk`。以下 7 个脚本**没有任何「workflow 是否调用」的断言**：
  `prepare-wry-kotlin.py`、`inject-gradle-patch-task.py`、`patch-android-download.py`、
  `patch-android-extras.py`、`patch-android-schemes.py`、`patch-android-fullscreen.py`、
  `patch-android-permissions.py`；
  `docs/android.md` 声明的先后顺序（prepare-wry-kotlin → insets → back → download → extras →
  schemes → fullscreen → permissions）**也没有任何断言**。
- **影响**：任一 workflow 少一行 `python3 patch-android-download.py`，`cargo test --all` 仍全绿
  （CI 的 selftest 只验证「脚本自己能跑」，不验证「有没有被调用」）；Android 的下载接管 / 外部
  scheme 外跳 / H5 视频全屏会**静默消失**，只有真机才能发现 —— 与 2026-11-01「零产物」那轮
  的教训同一类。
- **建议改法**：在 `android_pipeline_contract` 增加一条**表驱动**断言：把 `docs/android.md` 的链
  作为常量（脚本名 + 顺序），对 `build-custom.yml` / `release.yml` 校验「全部出现且下标递增」，
  并校验 `ci.yml` 的 selftest 列表包含全部脚本。不抽新文件、不改 workflow。
- **工作量**：小（约 50 行测试，零行为变更）。

### P1-2 `settings.json` 顶层损坏 = 静默清空，且下一次写入把它覆盖掉

- **位置**：`webview-app/src/store.rs:273` `parse_document`；`:249` `update_with`；`:158` `load`。
- **问题本质**：整份 JSON 解析失败 → 所有字段回默认值，只留一行 `dlog!`（而 dlog 默认关闭）；
  此后任何一次写入（主窗几何 / 悬浮球几何 / 设置页保存）都会用默认值**原子覆盖**原文件。
- **影响**：磁盘写满、断电、用户手工编辑写坏，都会让「快捷键 / 托盘 / 最小化 / 悬浮球开关 +
  窗口几何」一次性永久丢失，且用户无感知。字段级解析失败已被隔离（不会波及其他段），
  但**顶层**损坏没有出口。
- **建议改法**：`parse_document` 失败时先把原文件改名成 `settings.json.bad`（一次性备份，
  不参与解析），再返回默认值；dlog 写明备份路径。约 10 行 + 2 条单测（损坏 → 备份存在；
  字段级缺段 → 不备份、不误伤）。
- **取舍**：新增一个落盘产物 ⇒ 列入「需拍板清单」（我倾向做，成本极低、只在故障路径触发）。
- **工作量**：小。

### P2-1 悬浮球原生层（v1.81）零单测 + 8 个模块级静态

- **位置**：`webview-app/src/floatwin/native.rs:69-83`（`BALL_HWND` / `BALL_APP` / `BALL_HOVERED` /
  `BALL_COLLAPSED` / `PTR_DOWN` / `PTR_MOVED` / `PTR_START` / `TRACKING_LEAVE`）；
  `:427` `render_ball`、`:611` `composite_icon`、`:673` `premultiply_bgra`。
- **问题本质**：球从 WebView2 页面改成 Win32 分层窗口后，891 行 / 24 处 `unsafe` 全部只靠真机
  `--float-probe` 验证；而其中**三个像素合成函数是纯函数**（入参全是标量 / 切片），本可离线测。
  同时状态从 Tauri `State` 搬到了 8 个模块级静态 + 一个全局 `AppHandle`，`destroy` 路径是否
  清干净没有任何机器可验的证据。
- **影响**：「边缘毛刺 / 整球透明 / 点击穿透失效」这类回归只能靠肉眼看真机；`BALL_APP` 残留会让
  已销毁的球继续持有 `AppHandle`（进程生命周期内不可见，但会掩盖 destroy 逻辑的真实缺陷）。
- **建议改法**：① 给三个纯函数加 4–6 条单测（缓冲区尺寸、四角 `alpha == 0`、中心 `alpha == 255`、
  预乘不变式 `r,g,b ≤ a`、`dark/hover/collapsed` 三态确有差异）；② 在 `floatwin/tests.rs` 加一条
  静态契约钉住 `destroy_float_window` 必清 `BALL_HWND` + `BALL_APP`。**不动结构、不抽抽象**。
- **工作量**：小（1 个 `#[cfg(test)] mod`，约 60 行）。

### P2-2 `#![allow(dead_code)]` 全 crate 兜底

- **位置**：`webview-app/src/lib.rs:26`（注释里写明「临时注释本行即可查死代码」的纪律）。
- **问题本质**：纪律靠人记，机器不拦。子模块拆分后的遗留转发函数、只在某个 cfg 目标里用的条目
  都会被这条兜底放过，长期堆积在生产二进制里。
- **建议改法**：实测（临时移除该行跑 `cargo check --all-targets --all-targets`），
  无告警则直接删；有则逐条收窄成行级 `#[allow(dead_code)]` + 注释（保留原纪律）。
- **工作量**：小（取决于实测结果）。
- **本轮已落地（批次②）**：实测 `cargo check --all-targets` 仅剩 `mobilebar` 模块的 21 处告警
  （桌面/测试构建里该模块实现天然无调用方）。改法 = 删 crate 级豁免 + 模块内
  `#![cfg_attr(not(mobile), allow(dead_code))]`，移动端构建仍受检查；新增契约
  `dead_code_allowance_stays_scoped_to_mobile_module` 钉住「不得回退到全 crate 兜底」。
  收窄后 clippy 立刻抓到一处真死代码（`mobile_bar_contract.rs::CI_YML` 未使用），已删除。

### P2-3 跨壳「共用件」清单与实际不符（会误导双壳验证）

- **位置**：根 `AGENTS.md` §3.4.1；`webview-app/AGENTS.md` 硬约束 1。
- **问题本质**：`electron-app/scripts/stage-assets.cjs` 实际只从 `webview-app/` 复制
  `titlebar.js`、`loading.js`、`shell.html`、`splash.html`、`floaticon.html`、`dist/settings.html`、`icons/icon.png`。
  而文档把 `tab_title.js`、`tab_keys.js`、`dnd_polyfill.js` 也列进「两壳共用 ⇒ 改一必双壳验证」，
  但这三个文件 Electron **完全不消费**：`tab_title.js` / `tab_keys.js` 由 Tauri 侧
  `include_str!` 注入（`tabs/window.rs`），Electron 用主进程 `before-input-event` / 事件通道自行实现；
  `dnd_polyfill.js` 是 WebView2 专属。
- **影响**：真共用件（`titlebar.js` 里的 Ctrl+W 分支、`shell.html` 里的键盘通路）**必须**双壳验证，
  但混在一起列会让人对三个 Tauri 专属文件做无效的 Electron 验证，或反过来以为「Electron 没这个文件
  ⇒ 不用管」。`shell.html:371-373` 已经写明「与 Electron 侧 `isCloseTabKey` / Tauri 侧 `tab_keys.js`
  逐条对齐」，说明这条**行为契约**真实存在、只是没在清单里表达。
- **建议改法**：把清单拆成三类 —— 真共用（stage-assets 复制）/ Tauri 专属（`tab_title.js`、
  `tab_keys.js`、`dnd_polyfill.js`、`mobilebar/*.js`）/ Electron 专属（`floaticon.html`），
  并单列一行「行为对齐但实现不同」（Ctrl+W、标题回传）。纯文档，两处。
- **工作量**：小。

### P2-4 文档计数与目录树漂移

- **位置 / 事实**：
  - `webview-app/AGENTS.md:15`：「`src/contract_tests/`（按领域 13 个子模块）」「当前 282 通过 / 6 ignored」
    → 实际 **14 个子模块**（2026-11-03 新增 `settings_hotkey_contract`），测试计数见 §6。
  - `webview-app/docs/build-test.md:19`：同样写「13 个子模块」，领域枚举里缺「设置快捷键」；
    `:104` 的计数同样过期。
  - `webview-app/docs/build-test.md` 目录树**缺 `src/tab_keys.js`**（真实注入脚本，被
    `tabs/window.rs` `include_str!` 使用，且有 5 条契约守卫）。
  - `webview-app/docs/workflows.md:3` 与 `webview-app/AGENTS.md:45,71`：「7 条 workflow」
    → `.github/workflows/` 现为 **8 个文件**（2026-11-01 新增 `cleanup-runs.yml`）。
- **影响**：读文档的人会先怀疑「我是不是漏了第 14 个子模块 / 少了一个 workflow」，属于纯误导成本。
- **建议改法**：按 §6 实测数字统一更新；目录树补 `tab_keys.js`。
- **工作量**：小（纯文档）。

### P3-1 `lib.rs` 772 行（**建议不拆**，理由如下）

- **位置**：`webview-app/src/lib.rs`（`build_main_window` / `HISTORY_BACK_SCRIPT` /
  `HISTORY_FWD_SCRIPT` / `history_nav` / `handle_main_close` / 命令注册 / `run`）。
- **为什么不拆**：`generate_handler!` 必须留在 `lib.rs`；且有 **6 个契约测试模块**
  `include_str!("../lib.rs")` 并对**行序 / 字面量**做断言
  （`loading_mode_contract`、`startup_visibility_contract`、`startup_log_placement_contract`、
  `ipc_command_registry`、`mobile_bar_contract`、`float_window_contract`）——
  搬家等于同时改 6 份守卫 + 跨 `cfg(mobile)` 目标重验，风险远大于「文件整齐」的收益。
- **结论**：写入「刻意不动」清单，不做。

### P3-2 `mobilebar.js`（1,260 行 → 5 片，已落地）

- **位置**：`webview-app/src/mobilebar/`（Android 专属；Rust 侧 `mobilebar.rs` 仍 909 行不拆）。
- **现状**：行为验证靠 `mobile_bar_contract`（23 条静态断言）+ 真机；**无 JS 单测**（本仓库两壳都没有
  JS 测试基建）。
- **本轮已落地**：按「核心 + 功能」拆成 **5 片**（core / bottom_space / tabs / more / mount），
  由 `lib.rs` 的 `concat!(include_str!(…))` 按序拼成**同一个 IIFE**（闭包作用域不变 ⇒ 纯搬运）：
  - `core.js`（底座：配置、常量/图标/样式、宿主与事件工具、底栏本体、面板宿主、共享状态）；
  - `bottom_space.js`（底部空间契约：让位探测 / 贴底元素上移 / 100vh 缩高 / 重排调度）；
  - `tabs.js`（页签：列表面板、文档 url·标题上报、`_blank`/`window.open` 拦截）；
  - `more.js`（「更多」：原生桥、复制链接、六行动作面板）；
  - `mount.js`（入口生命周期；**必须最后一片**，负责关闭 IIFE）。
  - 注入契约：`lib.rs` 是分片顺序的**唯一权威**，契约测试从它派生清单（不再手抄）；
    构建侧无拷贝路径（`stage-assets.cjs` 不复制底栏脚本）。真机验证清单见
    `webview-app/docs/android.md`「拆分后逐项真机验证」。

### P3-3 CI 三份脚本清单无单一真源

- 同一病根见 P1-1。**已改判**：不做「抽一份清单文件」（那会造出第二个真源），改为
  **测试架构问题 —— 打包 workflow 才是唯一权威**，判据从手写名单改为**目录派生**：
  - 管线脚本 = `.github/scripts/*.py` 中引用 `android_gen` 或文件名 `patch-android-*` 开头者（`android_gen.py` 自身除外）；
  - 每个被发现脚本都必须被 `build-custom.yml` 与 `release.yml` **真实调用**（排除 `echo "::error::"` 文案行）；
  - 两份 workflow 不得调用目录里不存在的脚本，且调用顺序必须互相一致；
  - `ci.yml` 的 `--selftest` 循环按目录发现，且发现结果必须非空（空集不许假绿）。
- **落地**：`contract_tests/android_pipeline_contract.rs` ⑧ + `ci.yml` 自测步骤；新增/删除补丁脚本不必再改测试。

### P3-4 `floatprobe.rs`（1,046 行）常驻生产二进制

- 已知权衡（探针必须在**真产物**上跑才能复现 WebView2 / 分层窗口行为），`docs/build-test.md`
  与 `docs/float-ball.md` 已写明口径。**不建议动**，仅在此记录「不是遗漏」。

---

## 3. 与 2026-10-25 结论对照

| 10-25 结论 | 本轮复核 | 证据 |
| --- | --- | --- |
| `floatwin` 拆为 `{events,shape,window,tests}` | ❌ **失效** | v1.81 原生球重写后实为 `floatwin.rs`(452) + `{events,native,tests}` |
| `tabs` 拆为 `{state,window,ipc,tests}` | ✅ 仍成立 | `tabs.rs` 60 行 |
| `proxy` 拆为 `{auth,ws,headers,tests}`，sidecar `#[path]` 不破 | ✅ 仍成立 | `proxy.rs` 324 行，sidecar 零改动 |
| 六模块对 `crate::bootstrap` 引用归零 | ✅ 仍成立 | 逐个 grep 实测 0 |
| `contract_tests` 13 子模块 / 79 条 | ❌ **失效** | 14 子模块（+`settings_hotkey_contract` 5 条） |
| `cargo test --all` 282 通过 / 6 ignored | ❌ **过期** | 见 §6 实测 |
| electron 样例 17 键差异 = 有意裁剪 | ✅ 仍成立 | `docs/packaging-config.md` §8 |
| 「P4 大文件拆分因高风险未执行」 | ❌ **已过期** | 拆分已落地（见 §1.2） |
| 「全局可变状态克制（仅 5 处静态）」 | ❌ **失效** | v1.81 新增 8 个 `floatwin/native.rs` 静态 + `BALL_APP` 全局 `AppHandle` |

---

## 4. 本轮优化批次（按风险从低到高）

| 批次 | 内容 | 性质 | 风险 |
| --- | --- | --- | --- |
| ① | P1-1 补丁链守卫 + P2-1 像素层单测 | 只加测试 | 极低（零生产代码改动） |
| ② | P2-2 `allow(dead_code)` 实测收窄 | 生产代码清理 | 低（编译期可见） |
| ③ | P2-3 / P2-4 文档纠偏（根 `AGENTS.md`、`webview-app/AGENTS.md`、`build-test.md`、`workflows.md`） | 纯文档 | 无 |
| ④ | P1-2 损坏文件备份（**已拍板执行**） | 行为变更（新增落盘产物） | 低但需确认 |

不进入本轮：P3-1（不拆 `lib.rs`）、P3-4（已知权衡）。P3-2 已按「核心 + 功能」落地（见上）。

---

## 5. 刻意不动清单（连同理由存档，避免下轮重复讨论）

1. `lib.rs` 不拆（6 份契约守卫 + `generate_handler!` 位置约束）。
2. `mobilebar.rs` 不拆（Android 侧验证成本最高，收益只是整齐）；JS 侧已按「核心 + 功能」拆分（P3-2）。
3. `floatprobe.rs` 留在生产二进制（探针必须在真产物跑，已文档化）。
4. `settings↔store`、`settings↔floatwin`、`floatwin↔tray` 三组互指保留（同一份 JSON 读-改-写契约）。
5. `contract_tests/*` 里 `include_str!` 越出本 crate 读 `../.github/`、`../scripts/`、
   `../electron-app/` —— 跨壳守卫的**必要代价**，不搬到独立 crate。

---

## 6. 门禁实测

> 2026-11-07 当天实跑（主 crate 的编译一度被本机内存阻塞，机器重启后已复跑通过，见 6.2）。

### 6.1 实测通过

| 门禁 | 命令 | 结果 |
| --- | --- | --- |
| 格式化 | `cargo fmt --all --check` | ✅ exit 0 |
| **全量测试** | `cargo test --all` | ✅ **297 passed / 6 ignored / 0 failed**（主 crate 224 + 3 ignored、`resolve-core` 19、`resolve-tool` 22、`w2a-proxy` 32 + 3 ignored） |
| **clippy** | `cargo clippy --all-targets -- -D warnings` | ✅ exit 0（无 warning） |
| sidecar 独立可测 | `cargo test -p w2a-proxy` | ✅ 32 passed / 3 ignored —— 顺带证明 `proxy.rs` / `resolve.rs` 脱离 Tauri 也能编译与测通 |
| 解析器自测 | `node scripts/pack-local.mjs --selftest` | ✅ 全部通过 |
| 解析器对等性 | `node scripts/pack-local.mjs --parity` | ✅ **55 / 55**（本机 Windows Python 会把 `\n` 写成 `\r\n`，脚本已按 LF 归一后比对；CI 为 Linux，无此差异） |

297 = 11-03 基线 287 + 批次① 新增 5 条（补丁链 1 + 悬浮球像素层 4）+ 批次② 新增 1 条
（`dead_code_allowance_stays_scoped_to_mobile_module`）+ 批次④ 新增 3 条（损坏备份 / 不覆盖旧备份 /
备份失败降级）+ `mobilebar` 拆分新增 1 条（分片顺序 / IIFE 闭合），**预期值与实测一致**。

### 6.2 历史：一度受阻的记录（保留供下次排查参照）

首轮 `cargo test --all` 在 `windows v0.62.2` 处失败：rustc 以 `0xc0000409 STATUS_STACK_BUFFER_OVERRUN`
退出，日志首行 `memory allocation of 1044480 / 2097040 bytes failed`。实测根因是本机**系统提交上限耗尽**
（`% Committed Bytes In Use = 95%`，Commit Limit 22.8 GB / 已提交 ≈ 20.4 GB；而全部进程私有提交合计
仅 2.9 GB ⇒ 差额是环境级不可见占用），非仓库问题：同环境 `node` 单进程 800 MB 可分配、900 MB 即失败。
`jobs=1` / `debug=0` / `incremental=false` / 免沙箱四种组合均在同一处复现。

机器重启后同一命令直接通过（重启后 `% Committed` 降到 40.8%）。**结论：这是本机环境问题，
不是代码问题；以后再遇到同一条 `0xc0000409`，先看 `% Committed Bytes In Use`，别去改代码。**
