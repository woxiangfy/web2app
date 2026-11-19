# Tauri 侧架构审查（2026-11-19）

> 范围：`webview-app/src`、`webview-app/crates/*`。重点 Android（移动底栏 / 单 webview 页签 / 启动链路）。
> 判定口径：只认「实跑 + 代码位置」，不认印象。行号为本次审查时点值。

> **⚠️ 2026-11-19 第二轮复核修订（本文件正文为时点快照，未改写）**
>
> 1. F4 / D4「抽 `settings/apply.rs` 断环」**不成立** —— apply 仍 import `tray` / `floatwin`，只是换文件名。
>    真断环须依赖注入。且三条环里只有 `settings → tray/floatwin` 一条是反向边，另两条是「功能 → 服务」的
>    健康依赖 ⇒ **改判不做**。
> 2. F5 / D5 `floatprobe` feature gate **改判不做**：体积收益 <3%，而漏配 feature 会让探针静默消失。
> 3. F1 影响面比正文更大（不止 401，还有「内网不可达 ⇒ 白屏」与「全程脱离代理」），且**不依赖真机**即可实施
>    （纯函数本机可单测）。
>
> 现行结论以 [`architecture-todo.md`](architecture-todo.md) 为准。

## 0. 摘要

| 结论 | 说明 |
| --- | --- |
| 整体健康 | 依赖方向已收敛（2026-10-25 P2-1 生效）：功能模块只 `→ nav / config`，`bootstrap` 不再被下行依赖；跨 crate 无重复实现（`strip_userinfo` 单点 + 两层薄包装）。 |
| 本轮执行 | 2 项**零行为变更**结构整理：① `bootstrap` 尾部 273 行 Android 可达性探测抽成 `src/probe.rs`；② `Settings`/`Hotkey` 下沉 `src/model.rs`，闭环待办 P-R1 的类型环。 |
| 最实质缺口 | 移动端 `open-tab`（`target=_blank` / `window.open`）**没有**复用桌面端的「上游地址 → 入口地址」改写 ⇒ 本地代理 + Basic 鉴权下，点页面里写死的绝对外链会落到 401（见 §3 F1）。 |
| 规模 | 生产 14.9k 行 / 静态契约 4.4k 行（96 条）/ 内联单测约 2.4k 行；`cargo test --all` **333 passed / 3 ignored**。 |

## 1. 结构快照

### 1.1 模块规模（`src/`，行数）

| 模块 | 行数 | 职责 | 平台 |
| --- | --- | --- | --- |
| `lib.rs` | 880 | 装配层：builder 链、`invoke_handler`、事件接线、退出语义 | 全平台（59 处 cfg） |
| `floatprobe.rs` | 1137 | 悬浮球真机探针 harness（`--float-probe`） | Windows |
| `floatwin/{native,events}` | 1386 | Win32 分层球窗 + 拖动/吸附状态机 | Windows |
| `mobilebar.rs` | 991 | 移动底栏契约 + 单 webview 页签状态机 + 事件落地 | Android（`test` 亦编译） |
| `store.rs` | 930 | `settings.json` 唯一文档（读-改-写 / 原子写 / 旧位置兼容） | 全平台 |
| `tabs/{window,ipc,state}` | 1336 | 桌面多页签（多 webview 并行） | Windows |
| `webview_hooks.rs` | 917 | WebView2 原生事件（鉴权 / 新窗 / 下载 / 右键 / 密码保存） | Windows |
| `settings.rs` | 794 | 设置业务：校验 / 命令 / 设置窗 / 运行期副作用 | 全平台（副作用仅桌面） |
| `config.rs` | 683 | 编译期常量 + 打包字段解析（19 键单一真源） | 全平台 |
| `bootstrap.rs` | 504 | 启动编排：等首文档 → 准备 → 导航 → 失败上报 | 全平台 |
| `loading.rs` | 524 | 启动页协议 + 注入式加载层 | 全平台 |
| `probe.rs` | 400 | Android 导航后可达性探测（本轮新增） | 非 Windows（`test` 亦编译） |
| `debug_log.rs` | 458 | 诊断日志 + 启动信息（与 Electron 逐字段对齐） | 全平台 |
| `nav.rs` | 365 | 导航会话枢纽：`Navigation` / `ProxyTask` / `BootReady` / label / URL 原语 | 全平台 |
| 其余 | ~1900 | `proxy`（根 + auth/headers/ws）/ `resolve` / `web_data` / `tray` / `window_state` / `save_dialog` / `main` / `model` | 见各模块 |

### 1.2 依赖方向（生产代码，`*` = 平台专属）

```
lib（装配层，唯一允许「认识所有人」）
├─ bootstrap → nav, config, loading, resolve, proxy, probe*, tabs*, mobilebar*, webview_hooks*
├─ settings  → model, store, config, nav, tray*, floatwin*
│    └─ store → model, config                      ← 原 ① settings↔store 环已断（本轮）
├─ tray*     → settings, tabs*, floatwin*, nav, config
├─ floatwin* → store, settings, tray*
├─ tabs*     → nav → config
├─ webview_hooks* → tabs*, nav, config, save_dialog*, settings
└─ debug_log → config, settings, store, resolve, nav, tabs*
```

剩余环全部落在「设置簇」（`settings ↔ tray ↔ floatwin`，三条，均 `cfg(desktop)/cfg(windows)`），
成因是**设置保存时顺手应用运行期副作用**（见 §3 F4）。
`debug_log` 反向认识 6 个模块是 2026-10-12 P1-1 的已知代价（把 120 行格式化从 `lib.rs` 搬来），
属**有意识的取舍**，不再重开。

### 1.3 Android 侧的三条硬约束（现状：都有护栏）

1. **UI 线程禁调阻塞型 webview getter**：`mobilebar` 一律读 `CurrentUrl` 缓存；静态守卫
   `mobile_bar_never_queries_webview_on_ui_thread` 扫代码行（剥注释）。
2. **启动必须先等首文档就绪**：`nav::BootReady` + `bootstrap::wait_first_page_ready`，契约测试钉住调用。
3. **页签是「单 webview + 记录」**：`MobileTabs` 纯状态机（不碰 Tauri API），真机跑不到的逻辑在本机可单测。

## 2. 本轮改动（2 项，零行为变更）

### 2.1 `bootstrap` → `probe`（职责拆分）

- **问题**：`bootstrap.rs` 873 行里，尾部以 banner 注释隔开的 273 行是「Android 导航后为什么还白屏」的诊断
  （UA / 三类文案 / 目标选择 / reqwest 探测 / 错误归类 + 5 条单测），与「何时导航」的编排无关。
- **处理**：整段搬到 `src/probe.rs`（400 行，含 106 行单测），`bootstrap` 只留两处调用
  （`crate::probe::probe_target` / `crate::probe::spawn_reachability_probe`）。873 → 504 行。
- **cfg 不变**：模块按 `any(not(windows), test)` 挂载 —— Windows 发布产物仍不携带这些文案，
  Windows 宿主 `cargo test` 仍编译纯逻辑部分（与拆分前同一条纪律）。
- **验证**：`cargo test --all` 333 passed / `clippy -D warnings` / `fmt --check` 全绿，测试数不变（纯搬家）。

### 2.2 `Settings` / `Hotkey` 下沉 `model.rs`（闭环待办 P-R1）

- **问题**：`store.rs` 的 `Document` 以 `settings::Settings` 为字段类型，而 `settings.rs` 又调 `store::load/update`
  ⇒ 持久化层向上引用业务层模型，依赖成环（编译无告警，纯结构债）。
- **处理**：新增 `src/model.rs`（106 行，**零 crate 内依赖**，只有 serde）承载两个纯数据结构与 `Default`；
  `store` 改 `use crate::model::Settings`；`settings.rs` 用 `pub use crate::model::{Hotkey, Settings}`
  保留旧路径 ⇒ 调用点零改动。方向变为 `store → model`、`settings → model`。
- **契约同步**：`settings_hotkey_contract` 里「提示必须 `#[serde(skip)]`」跟着字段改扫 `model.rs`
  （断言语义不变），另补 2 条 model 单测（旧文件缺 `float_enabled` 可解析 / `hotkey_notice` 不落盘）。
- **验证**：`cargo test --all` 333（+2），clippy / fmt 全绿。

## 3. 发现清单（按影响排序）

### F1（P1）移动端 `open-tab` 未复用桌面「上游 → 入口」地址改写

- **位置**：`mobilebar.rs::create_tab_with_url`（507）直接 `navigate_to(raw_url)`；
  桌面版规则在 `tabs/ipc.rs::resolve_popup_url_for`（202），可见性 `pub(super)` 且模块 `cfg(windows)`。
- **触发条件**：Android + `mobile_bar=true` + `tabs_enabled=true` + 本地反向代理（`PROXY_ENABLED` 且无外部代理）
  + 页面里 `target="_blank"` 指向**写死的上游绝对地址**（`https://real.example.com/...`）。
- **后果**：WebView 直连上游、绕过注入 Authorization 的本地代理 ⇒ 401 / 空页；桌面端同一操作正常
  （被改写成入口地址 + path/query）。
- **性质**：不是「少写一个 if」，而是**同一职责两份实现、语义不同**——典型复用缺口。
- **建议**：把纯函数上移 `nav.rs::resolve_popup_target(&Navigation, Option<&str>)`（跨平台、可单测），
  桌面 `tabs/ipc.rs` 保留薄包装，移动 `create_tab_with_url` 调用它。需真机复验（列入 §4 D1）。

### F2（P2）`save_settings` 的副作用与落盘不是事务

- **位置**：`settings.rs::save_settings`（458）。顺序是「应用快捷键 / 托盘 / 球（505-534）→ 落盘（536）→ 写内存态（541）」。
- **后果**：落盘失败（磁盘满 / 权限）时，快捷键已换、托盘已建/已删、球已建/已删，但内存态与文件仍是旧值
  ⇒ 运行期与文件不一致，且新注册的快捷键不会命中（回调比的是内存态旧键）；重启后又回到旧设置。
  托盘创建失败走 `?` 提前返回时同样留下已生效的快捷键变更。
- **建议**：见 §4 D2（三选一，需定口径：磁盘优先 / 补偿回滚 / 维持现状只记录）。

### F3（P2）`mobilebar.rs` 991 行单文件 vs 桌面 `tabs/` 五文件

- **现象**：桌面多页签是 `tabs.rs`（72，门面）+ `tabs/{window,state,ipc,tests}`；移动多页签是单文件，
  内部混装「布局契约常量 / 纯状态机 `MobileTabs` / Tauri 事件落地 / eval 推送 / 导航原语 / 218 行单测」。
- **影响**：可读性与改动半径（改一处 UI 文案要在大文件里跳）；与桌面同构性丢失，跨端对照成本高。
- **约束**：契约测试 `mobile_bar_contract` 以 `include_str!("../mobilebar.rs")` 单文件扫描（含 `app.listen(...)` 字面量、
  `.url()` 禁令、`#![cfg_attr(not(mobile), allow(dead_code))]` 位置），拆分需同步改为多片 `concat!`。
- **建议**：见 §4 D3（与下次 Android 功能改动同批，不单独占用真机验证窗口）。

### F4（P2）`settings.rs` 四合一 + 「设置簇」三条环

- **职责**：数据校验（`hotkey_problem`）、四条 IPC 命令、设置窗创建 + `settings://` 协议、运行期副作用
  （快捷键注册 / 托盘 / 球）。794 行。
- **环**：`settings → tray/floatwin`（应用副作用）与 `floatwin → settings::set_float_enabled`（球右键改开关）、
  `tray → settings::open_settings_window`（托盘菜单开设置）互指。
- **建议**：抽 `settings/apply.rs`（把 `Settings` diff 应用到运行期），使 `settings` 不再认识具体功能模块；
  球右键改开关改为「写设置 + 通知」，不再反向调 `settings`。见 §4 D4。

### F5（P3）`floatprobe` 探针随发布产物编译

- 1137 行、`#[cfg(windows)] mod floatprobe;` 无条件进 Windows 二进制，仅 `--float-probe` 可达。
  与 Electron 侧 P-E1（2174 行探针随包）同类，但探针**必须能在产物上跑**，不能简单 `cfg(test)` 或移出 `src/`。
- 可行解：cargo feature（默认关，CI/脚本显式开）。代价是三处出包入口都要加 feature，漏一处 = 探针静默不存在。
  见 §4 D5。

### F6（P3）cfg 双轨：`not(windows)`（35 处）与 `mobile`（7 处）混用

- 现状等价于「非 Windows 即 Android」（`bootstrap.rs` 头部有单点说明）。若将来真出 Linux 桌面目标，
  `BootReady` 等待、探测、`data_directory` 分支会**静默**套到 Linux 上。
- 建议：现在只在 L2 文档补一条总述；真要加 Linux 目标时再统一成 `mobile`（§4 D6）。

### F7（P3）`lib.rs` 880 行装配层被静态契约「钉住」

- `run()` 内约 570 行 + 59 处 cfg；4 段事件监听（标题 / 重试 / Ctrl+W / 文档就绪）是同一模板重复
  （payload struct + `match` + `dlog!`）。
- **不能**顺手抽公共函数：契约测试逐字断言 `app.listen(tabs::TAB_TITLE_EVENT` 等形式（4 处），
  抽成 `listen_typed(...)` 会让守卫失效。这是「静态契约换重构自由度」的已知代价，记录而非改动。
- 可做且不碰这些文本的：把「建窗」（`build_main_window` 闭包 + 三种模式分支）抽成 `window_build.rs`。

### F8（P3）静态契约的守护成本

- 4371 行 / 96 条，占 Rust 总量 23%。历史收益明确（`invoke_handler` 覆盖事故、假绿剥注释等），
  但每条新增功能都要写「存在性 + 位置」断言，边际成本在上升。
- 建议：维持现状，**不加码**语法级扫描；新契约优先断言「行为不变量」（如 F1 的目标改写规则用纯函数单测），
  而不是「某文件某行有某字符串」。

## 4. 待决策（本轮不执行）

### D1 移动端弹窗地址改写（F1）

| 方案 | 优 | 劣 |
| --- | --- | --- |
| A. 纯函数上移 `nav`，桌面/移动共用 | 消除行为分歧；纯函数可单测；桌面零行为变更 | 需真机复验（代理+鉴权+绝对外链），本机只能验纯逻辑 |
| B. 维持现状，文档标注限制 | 零风险 | 该场景静默 401，用户无从判断 |

**倾向 A**，排到下次真机出包窗口一并验。

### D2 `save_settings` 事务性（F2）

| 方案 | 优 | 劣 |
| --- | --- | --- |
| A. 先落盘 → 再应用副作用 | 磁盘为唯一真源；失败即「什么都没变」 | 副作用失败时用户看到「保存成功但未生效」 |
| B. 应用后落盘失败 → 补偿回滚（快捷键 / 托盘 / 球） | 内存 / 磁盘 / 运行期三者一致 | 多约 40 行回滚代码；回滚本身也可能失败 |
| C. 维持现状，仅记录 | 零改动 | 磁盘满 / 权限异常时状态分裂，最难排查 |

**倾向 B**（只对可逆的两项做补偿），A 作为最小可用替代。

### D3 `mobilebar` 拆分（F3）

| 方案 | 优 | 劣 |
| --- | --- | --- |
| A. 拆 `mobilebar/{mod,tabs,doc,ui,tests}.rs` | 与 `tabs/` 同构；单文件降到约 300 行 | 契约测试 `BAR_RS` 改多片 `concat!`；本机无法编译 Android 目标，只能静态验证 |
| B. 单文件 + 分区注释 | 零风险 | 结构不对称长期存在 |

**倾向 A**，与下次 Android 功能改动同批落地（纯搬家不单独占验证窗口）。

### D4 `settings` 拆解与破环（F4）

| 方案 | 优 | 劣 |
| --- | --- | --- |
| A. 抽 `settings/apply.rs`，副作用移出 | 消除 3 条环；`settings` 只认识「设置」 | 覆盖桌面三条链路（快捷键/托盘/球），需 GUI 冒烟 |
| B. 只做本轮已完成的模型下沉 | 已落地、零风险 | 只解决类型环，副作用环仍在 |

**本轮已做 B**；A 视下次动设置页时一并。

### D5 `floatprobe` feature gate（F5）

| 方案 | 优 | 劣 |
| --- | --- | --- |
| A. `--features float-probe` | 发布产物不含探针（与 Electron P-E1 同解） | 三处出包入口都要加 feature，漏一处静默失效 |
| B. 保持随包 | 零风险、探针永远可用 | 用户产物里带一套开发 harness |

**倾向 B**（当前仅内部自用）；对外发版前再执行 A。

### D6 cfg 语义统一（F6）

| 方案 | 优 | 劣 |
| --- | --- | --- |
| A. 35 处 `not(windows)` → `mobile` | 语义精确，将来加 Linux 不误触发 | 改动面大，Windows 发布路径需回归 |
| B. 文档登记（现状） | 零风险 | 隐含假设仍靠口口相传 |

**倾向 B 现在 + A 待真加 Linux 目标时**。

## 5. 门禁结果

```
cargo fmt --check                 ✅
cargo clippy --all-targets -D warnings  ✅
cargo test --all                  ✅ 333 passed / 3 ignored（web2app 255 + w2a-proxy 37 + resolve-tool 22 + resolve-core 19）
```

Android 目标：本机无 SDK/NDK，**未编译**；本轮两处改动均不触碰 `cfg(mobile)` 专属代码路径。
