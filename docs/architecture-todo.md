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

**状态**：☐ 待处理

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
