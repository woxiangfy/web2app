# 架构待决策清单（TODO）

> 来源：2026-10-16 架构审查（Rust 侧 + Electron 侧）。
> 这些是**接口级改动**，按「最小重构」原则仅提方案、未执行，留待后续单独评审。
> 处理完一项就划掉一项，并回填处理结论与所在 changes 文档。

---

## Rust 侧（webview-app）

### P-R1 `store` ↔ `settings` 双向类型引用 → 单向

**现状**：
- `settings.rs` 定义数据模型 `Settings` / `Hotkey`，**调用** `store::load` / `store::update` 做持久化（settings → store 运行时调用）；
- `store.rs` 定义持久化文档 `Document { settings: Settings, ... }`，**引用** `settings::Settings` 作为字段类型（store → settings 类型引用）。

即「持久化层引用业务层的数据模型，业务层又调用持久化层的读写」——依赖图成环。Rust 同 crate 内 `crate::` 路径无循环模块限制，编译零告警，但 `Settings` 这个纯数据模型被「业务逻辑层」占有，持久化层反而要向上引用。

**方案**：把 `Settings` / `Hotkey` 下沉为 `store` 的（或独立 `models` 模块的）类型，`settings.rs` 变成纯「业务逻辑 + 读写封装」层，只 `use store::{Settings, Hotkey, Document}`。方向变为单向 `settings → store`。

**影响**：`Settings` / `Hotkey` 路径从 `crate::settings::` 变为 `crate::store::`（或 `crate::models::`），涉及所有 `use crate::settings::Settings` 处（`store.rs`、`lib.rs` 的 `SettingsState` 等）及 `pub use` re-export 的可见性。属接口级改动，需同步契约测试。

**状态**：✅ 已完成（2026-11-19，采用「独立 `model` 模块」方案而非并入 `store`）

**处理结论**：`Settings` / `Hotkey` 下沉到 `webview-app/src/model.rs`（零 crate 内依赖，只依赖 serde），
`store` 直接 `use crate::model::Settings`；`settings.rs` 用 `pub use crate::model::{Hotkey, Settings}`
保留旧路径 ⇒ 全部调用点零改动。方向变为 `store → model`、`settings → model`，类型环消失。
`settings_hotkey_contract` 的「提示必须 `#[serde(skip)]`」改扫 `model.rs`（断言语义不变），
另补 2 条 model 单测。见 [architecture-review-2026-11-19.md](architecture-review-2026-11-19.md) §2.2。

---

### P-R2 两个 token 生成函数命名/语义不统一

**现状**：`proxy.rs` 内 `generate_access_token`（256-bit → URL 安全 base64，入口门禁密钥）与 `random_hex_token`（128-bit → 小写 hex，事件回传防伪令牌）都叫「token」，格式与用途却不同，语义边界靠注释而非命名区分。

**方案**（低优先级，可选）：重命名让「用途」而非「长度」主导命名（如 `proxy_access_key()` 与 `event_nonce()`）。

**影响**：涉及 `generate_access_token` 的调用点与 2 个单测名，纯重命名但会动接口。

**状态**：☐ 待处理

---

## Electron 侧（electron-app）

### P-E1 探针混在生产源码树、并静态 import 进产物

> 🔁 **承接 2026-09-29 的 P1-2**（当时未落地）。

**现状**：
- `probe.ts` + `probe-{server,infra,single,tabs}.ts` 合计 **2174 行**，位于 `src/main/`（生产树）。
- `index.ts:32` **静态 import**：`import { runProbe, runTabsProbe, startProbeServer } from './probe'`。
- `src/renderer/tabs-probe-preload.ts` 也属探针资产，混在 renderer 里。
- `electron-builder.yml` 的 `files: - out/**/*` ⇒ 探针随包交付（探针存在意义是「在产物上跑」，不能简单剔除）。

**方案**（两档，建议后者）：
- 保守：`probe*.ts` + `tabs-probe-preload.ts` 移到 `diagnostics/` 顶层目录，`index.ts` 改**动态 import**（`await import()`）且仅 `PROBE` 为真时加载。
- 干净：`tsconfig.json` 增 `diagnostics` 独立编译目标，`files:` 显式排除该目录。

**风险**：必须先在打包产物上验证「动态 import 可用」再落地。

**状态**：☐ 待处理

---

### P-E2 `settings-window.ts::normalizeIncoming` 与 `settings.ts::parseSettings` 职责重叠

**现状**：两者都是「把未知对象收敛成 `Settings`」：
- `settings.ts::parseSettings`（磁盘读取路径）—— `toBool` **宽容**接受 `"true"/"1"/"yes"/"on"` 字符串。
- `settings-window.ts::normalizeIncoming`（设置页 IPC 上传路径）—— 布尔只认 `v === true`，字符串一律 `''`，是**严格**收敛。

同一条职责散在两处、规则还不一致。`float_enabled` / `tray_enabled` / `minimize_on_close` / `wake_hotkey` 四个字段已**逐字段**重复了一遍。

**方案**：把 `normalizeIncoming` 的「结构收敛」职责合并进 `settings.ts`，暴露 `normalizeIncomingSettings(o): Settings`（严格模式）与 `parseSettings`（宽容模式）并列，在同一文件内对字段清单单一真源（共享 `SETTINGS_KEYS` 列表）。设置窗只做 IPC 包装。

**影响**：会改动设置页保存路径的行为面（需确认严格收敛语义不变）。

**状态**：☐ 待处理

---

### P-E3 `index.ts` 仍是 1165 行的装配中枢

**现状**：`index.ts` 仍持有全部模块级可变状态（`mainWindow` / `target` / `tabManager` / `activeSidecar` / `navSeq` / `quitting` / `runtimeSettings`…），且 `createWindow` 里的 `onSpawn` 回调（导航 + 窗口打开 + 标题回传 + 失败态 + F12/右键，约 45 行）与 `createWindow` 本体混在一起，是当前最「职责混杂」的一处。

**方案**（低风险，可分步）：
1. 把 `createWindow` 内的 `onSpawn` 回调抽为独立函数 `attachContentView(view, label, url)`（或独立模块 `tab-content.ts`），与 `registerWindowOpenHandler` 平级。
2. 把 `logStartup` 迁到 `startup-log.ts`（零 Electron 依赖，迁过去更内聚；Rust 侧 P1-1 已做过对等动作）。

**影响**：`onSpawn` 抽取是纯函数边界重排（零行为变更，可安全执行）；`logStartup` 迁移涉及 `index.ts` 里 10+ 处快照传参重构，建议单独一次提交。

**状态**：☐ 待处理（其中 `onSpawn` 抽取为唯一「零行为变更」项，建议作为下一轮入口）

---

### P-E4 探针断言组依赖「三处逐字一致」的字面量契约，缺乏集中真源

**现状**：设置命令（`get_settings` 等 6 个）、页签命令（`get_tabs` 等 8 个）、悬浮球命令（`float-*` 7 个）的**命令名字面量**散在至少三处：页面侧（双壳单一真源）、preload 白名单、主进程 IPC 注册。目前靠 `float-contract.test.ts` 静态守卫 + `smoke.ts` 的 `missingCmds` 断言兜底。

**方案**（可选）：把主进程侧的 IPC 命令名收敛为单一常量表（如 `ipc-commands.ts` 导出 `SETTINGS_COMMANDS` / `TABS_COMMANDS` / `FLOAT_COMMANDS`），`register*Ipc` 与 `smoke.ts` 的 `missingCmds` 都从它读。**注意**：preload 白名单与页面字面量仍无法共享（跨进程/跨语言），只能收敛主进程侧这一处。收益有限，低优先级。

**状态**：☐ 待处理

---

# 2026-11-19 架构审查（Rust 侧）· 待决策清单

> 来源 [architecture-review-2026-11-19.md](architecture-review-2026-11-19.md)。
> **2026-11-19 第二轮复核已重排下列 6 条**：P-R3 升级为「立即可做」，P-R4 收敛为小修，
> P-R5 缩水为「测试外置」，P-R6 / P-R7 改判**不做**（理由见各节）。

## 一、建议执行（2 项）

### P-R3 移动端 `open-tab` 绕过本地代理（P1，立即可做）

**事实**：`mobilebar.rs:507 create_tab_with_url` 直接 `navigate_to(raw_url)`；桌面规则
`tabs/ipc.rs:202 resolve_popup_url_for` 在 `#[cfg(windows)]` 模块内，Android 根本不编译。
Android 只要 `proxy_enabled` 就恒走本地代理（`nav::local_proxy_enabled()` 非 Windows 分支不判 `--proxy-server`）。

**影响（比首轮描述更大）**：

| 上游情况 | 用户看到 |
| --- | --- |
| 需 Basic 鉴权 | 无 Authorization ⇒ 401 |
| 内网域名 / 仅本机代理可达 | 白屏；`probe` 报「连接失败」，会被误判成网络故障 |
| 公网可直连 | 页面能开，但**全程脱离代理**，该页后续相对链接一起脱管 |

**方案对比**：

| 方案 | 成本 | 判 |
| --- | --- | --- |
| A 纯函数上移 `nav::resolve_popup_target(&Navigation, Option<&str>)`，桌面留薄包装，移动侧调用 | ~30 行搬迁 + 10 条测试迁移 | **推荐**；`upstream_origin` 已在 `Navigation`（跨平台） |
| B 移动端复制一份规则 | ~30 行 | 否；两处漂移，违背单一真源 |
| C 不改 | 0 | 否；缺陷保留且两壳行为不一致 |

**纠正**：首轮「等真机验证窗口」不成立 —— 改写规则是纯函数，本机单测即可覆盖语义；真机只是端到端确认，不阻塞排期。

**状态**：☐ 待处理（下一轮可直接做）

---

### P-R4 `save_settings` 落盘失败后副作用不回滚（P2，小修）

**事实链（复核补全）**：副作用（快捷键 / 托盘 / 球）先应用 → `persist_settings` 失败 `?` 返回 →
内存态与文件仍旧值。此时新键已注册、旧键已注销，而快捷键回调比对的是**内存旧值**：
新键无反应、旧键失效。用户重试保存时 diff 仍是「旧≠新」⇒ `register(新)` 返回「已注册」Err
⇒ 走回滚 `register(旧)` ⇒ **该键再也存不进去，直到重启**。

触发条件窄（磁盘满 / 只读 / 文件被占），但后果是静默的能力丢失。

| 方案 | 说明 | 判 |
| --- | --- | --- |
| A 磁盘优先（先 persist 再 apply） | apply 失败需回写旧值，两次写盘 | 次选 |
| B persist 失败时补偿回滚已应用项（快捷键 + 托盘） | ~25 行；球是纯 UI，下次保存自然纠正 | **推荐** |
| C 只记录不改 | 0 行 | 可接受（概率极低） |

**状态**：☐ 待处理（倾向 B，与下次设置页改动同批）

---

## 二、建议不做（3 项，附驳回理由）

### P-R5 `mobilebar.rs` 拆分 —— 缩水为「测试外置」（P3）

**复核**：生产 772 行 / 测试 218 行；生产部分已是「1 个状态机（140 行）+ 6 个事件入口 + 4 个 URL 工具 + 推送 / 导航」，
耦合来源是共享 `MobileTabsState`，不是乱堆。
- 拆 5 文件的主要收益是阅读体验，编辑器跳转已可替代。
- 成本明确：Android 本机不可编译（只能静态验证）+ 契约测试 1049 行钉住单文件路径 + 每片重复 cfg。
- **改结论**：只把 218 行测试搬到 `mobilebar/tests.rs`（`#[path]` 挂载，`BAR_RS` 扫描方式不变），
  生产文件补 4 条章节 banner。

**状态**：☐ 待处理（低优先，风险≈0）

---

### P-R6 「设置簇三条环」 —— 高估，不做（P3）

**复核后按方向分层**：

| 边 | 性质 | 判 |
| --- | --- | --- |
| `tray → settings::open_settings_window` | 功能 → 服务 | 健康 |
| `floatwin → settings::set_float_enabled` / `→ tray::show_window` | 功能 → 服务 | 健康 |
| `settings → tray::create_tray` / `floatwin::show / destroy` | 服务 → 功能 | **唯一反向边，3 个调用** |

**纠正**：首轮方案「抽 `settings/apply.rs`」**断不了环** —— apply 仍 import tray / floatwin，只是换个文件名。
真断环必须依赖注入（settings 持 `Box<dyn Fn>`，装配层注册），引入间接层且副作用错误不便回传。

**结论**：不做。

**状态**：☒ 已关闭（2026-11-19 复核）

---

### P-R7 `floatprobe` feature gate —— 明确不做（P3）

**复核**：1137 行、零测试，经 `probe_mode()` 与 setup 的 `run` 全量进 Windows 产物。
- 收益：一条 GUI 探针路径约 100–300KB（需出包对比才能精确测，估 <3%）。
- 风险：feature 要三处出包入口同步（`tests/build-*.bat` / CI / 本地 pack），漏一处 = 探针**静默消失**；
  而探针的价值正是「能在真产物上跑」。
- 若为体积，优先做图标 / 资源 / 依赖 feature 这类零风险项。

**状态**：☒ 已关闭（2026-11-19 复核）

---

## 三、维持「只做文档登记」

### P-R8 cfg 双轨 `not(windows)` / `mobile`（P3）

**复核计数**（含注释）：`not(windows)` 54、`cfg(mobile)` 12、`cfg(desktop)` 37。语义仍是「非 Windows 即 Android」。
真加 Linux 目标时需**逐条判定**这 54 处的归属（Android 专属 vs 通用非 Windows），而非机械替换。

**状态**：☐ 待处理（文档登记已完成，真加目标时再改）

---

## 四、本轮新增观察

### N1 移动端缺单一装配边界（结构性，未定级）

`not(windows)` 逻辑分散 5 处：`bootstrap.rs`（启动编排）、`nav.rs`（`BootReady` 时序）、
`lib.rs`（事件接线）、`mobilebar.rs`（多页签）、`probe.rs`（可达性）。
代价：追溯「Android 启动 → 首个页面」要跨 5 个文件；新增移动端能力没有明确落点。
- **半步方案**：`mobilebar.rs` + `probe.rs` 归入 `mobile/`（纯路径搬迁，`include_str!` 与契约路径同步改）；
  `bootstrap` / `nav` 的移动分支暂留（与启动时序强耦合，搬动风险高）。
- 不做整包搬迁：Android 本机不能编译，无处验证。

**状态**：☐ 待决策（建议「半步」，与下次 Android 改动同批）

### N2 移动端无 `opener` / 「下载后关页签」——不是缺陷

Android 下载走系统 DownloadManager（`patch-android-download.py`），不产生应用内
「下载完成后回到发起页签」的场景。**不要照桌面行为补齐**。
