# webview-app 架构与组织结构审查 — 2026-10-25

> ⚠️ **本文是 2026-10-25 的时点快照**：模块行数、测试计数等为当时实测值，后续演进不回填；
> 现行状态一律以根 `AGENTS.md`、`webview-app/AGENTS.md` 为准。前次审查见
> [architecture-review-2026-09-29.md](architecture-review-2026-09-29.md)。

审查范围：`webview-app/`（cargo workspace 四成员）+ 跨壳复用面。
关注维度：模块划分、职责边界、依赖关系、复用性。原则：**最小重构**。
实测规模：`src/` 20 个 Rust 文件 / 约 15,200 行（其中 `contract_tests.rs` 3,061 行），
较 2026-09-29 快照（22 文件 / 10,330 行）增长约 47%。

## 0. 总体结论

工程组织水平仍然偏高：**跨壳真源没有重复实现**（sidecar 用 `#[path]` 直接复用主 crate 源码）、
可复用模块边界清晰（`proxy.rs` / `resolve.rs` 零内部依赖）、静态契约守卫覆盖广、
全局可变状态克制（仅 5 处 `OnceLock/Mutex` 静态，其余走 Tauri `State`）。

主要问题集中在三处：**模块级双向耦合**（2026-09-29「无循环依赖」结论已失效）、
**contract_tests.rs 单文件 75 个测试跨 10+ 领域**、**配置字段表靠逐字段手写守卫维护**（P2-1 未闭环）。

| 维度 | 评价 | 关键证据 |
| --- | --- | --- |
| 模块划分 | 良好，但 3 个文件过大 | floatwin 1,556 行 / tabs 1,264 / proxy 1,285 |
| 职责边界 | 总体清晰，`debug_log` 有轻微越界 | 见 §3 |
| 依赖关系 | ⚠️ 6 组双向耦合 | 见 §2 |
| 复用性 | 优秀（Rust 侧）/ 债务（配置侧） | `#[path]` 复用 vs 5 处字段表 |

## 1. 模块地图与依赖方向

| 层 | 模块 | 行数 | 内部依赖（`crate::x`） | 评价 |
| --- | --- | --- | --- | --- |
| 入口 | `main.rs` | 6 | — | 纯转发 ✅ |
| 装配 | `lib.rs` | 741 | config, debug_log, mobilebar, tabs | 52 处 cfg，装配 + IPC 注册 ✅（P1-1 已闭环） |
| 配置 | `config.rs` | 633 | — | 最底层，被 8 模块依赖 ✅ |
| 启动枢纽 | `bootstrap.rs` | 959 | config, loading, mobilebar, proxy, resolve, tabs, webview_hooks | ⚠️ `Navigation` 数据枢纽，7 条出边与 5 个模块互指 |
| 网络 | `proxy.rs` / `resolve.rs` | 1,285 / 213 | **零内部依赖** | ✅ 复用性关键（见 §4） |
| 平台 | `tabs.rs` / `webview_hooks.rs` / `floatwin.rs` | 1,264 / 795 / 1,556 | cfg(windows) | 干净的平台叶子，但体量偏大 |
| 持久化 | `store.rs` / `settings.rs` / `window_state.rs` | 720 / 506 / 101 | store↔settings 互指 | 三段分离清晰，互指是「同一 JSON 读-改-写」契约的代价 |
| 诊断 | `debug_log.rs` / `floatprobe.rs` | 417 / 1,124 | debug_log→6 个业务模块 | ⚠️ 见 §3-2、§3-3 |
| 测试 | `contract_tests.rs` | 3,061 | include_str! 跨项目读文件 | ⚠️ 见 §3-1 |

**双向耦合实测 6 组**（同一 crate 内可编译，但形成层级环）：

- `bootstrap↔tabs`、`bootstrap↔loading`、`bootstrap↔mobilebar`、`bootstrap↔webview_hooks`
- `settings↔store`（同一 `settings.json` 读-改-写契约，刻意保留）
- `floatwin↔settings↔tray`（悬浮球开关联动）

前 4 组根因相同：`bootstrap.rs` 的 `Navigation`（真实 URL / 凭据 / 代理 AbortHandle）是全应用
数据枢纽，谁都要读它，而它又要调 `tabs::tabs_enabled()` 这类上层判断。

## 2. 依赖方向健康度

- `config` 是纯底层（零依赖、被 8 个模块依赖）✅
- `proxy` / `resolve` 零内部依赖 —— 这正是它们能被 `proxy-server` 以 `#[path]` 复用的前提 ✅
- 平台叶子（tabs / webview_hooks / floatwin / save_dialog）cfg 边界干净 ✅
- ⚠️ `bootstrap` 是双向枢纽（见 §1）；`debug_log` 依赖 6 个业务模块（P1-1 迁移的连带后果）

## 3. 职责边界问题（按严重性）

### 3.1 `contract_tests.rs` 是测试侧「上帝文件」

3,061 行 / 75 个测试混住 10+ 领域：IPC 注册表、capabilities、悬浮球（约 30 个）、tabs、
mobilebar、Android 补丁链、CI workflow 布局、配置字段真源。且它 `include_str!` 越出本 crate
读 `../.github/`、`../scripts/`、`../electron-app/src/main/config.ts` —— webview-app 的
`cargo test` 与兄弟项目文件路径硬耦合（跨壳守卫的代价，可拆分归属）。
建议按领域拆为 `contract_tests/{ipc,float,tabs,mobile,packaging,…}.rs` 子模块，零行为变更。

### 3.2 `debug_log` 职责被 P1-1 迁移撑大

启动日志迁入后它依赖 `bootstrap, settings, store, tabs, resolve` 等业务模块。它已不是
「日志基础设施」而是「诊断 + 启动报告」，命名与职责出现偏差，可接受但应在模块注释钉死新定位。

### 3.3 `floatprobe.rs` 1,124 行诊断代码常驻生产二进制

`lib.rs` 直接调用 `floatprobe::run`，0 个单测。与 2026-09-29 P1-2（Electron probe 进产物）
是同一权衡 —— 探针必须在真产物上跑 —— 两侧口径应统一写进文档，避免反复被当问题报。

## 4. 复用性评估

**优秀面（不建议动）**：

1. `crates/proxy-server/src/main.rs` 用 `#[path]` 直接引入主 crate 的 `proxy.rs` / `resolve.rs`
   —— **零复制**，两壳鉴权链路无漂移可能。可复用边界恰好等于依赖干净的模块。
2. `resolve-core`（纯加密核）被主 crate 与 `resolve-tool` 共用，分层正确；两个 bin 是薄壳。
3. 共享 JS/HTML 由 `electron-app/scripts/stage-assets.cjs` 打包时从 `webview-app/` 复制，
   文件级单一真源成立；两侧各有守卫（contract_tests vs `electron-app/test/`）。

**债务面**：

1. **配置字段表 5 处副本**：`parse-packaging-config.py`、`packlib/constants.mjs`+`parser.mjs`、
   `electron config.ts`、`config.rs`、双 `config.example.json`、`packaging-config.md`。
   现状靠**逐字段手写守卫**兜底（`config_field_exists_in_every_source_of_truth`、
   `android_permissions_field_is_in_every_source_of_truth`）——每加一个字段要改 6 处 +
   新写 1 个守卫测试。即 2026-09-29 P2-1，仍未闭环。
2. electron 侧 `config.example.json` 只有 17 键（缺 4 个 android 键）：若属刻意裁剪应写进
   `packaging-config.md`，否则即漂移。
3. `#[path]` 复用依赖相对路径 `../../../src/`，「聪明但脆」；若未来出现第三个消费者，
   应升格为独立 `crates/` 共享 crate。

## 5. 建议优先级（可逐步执行）

| 级别 | 项 | 性质 | 状态 |
| --- | --- | --- | --- |
| P1 | contract_tests.rs 按领域拆子模块（零行为变更，守卫字面量不动） | 组织结构 | ✅ 2026-10-25 落地 |
| P1 | 逐字段守卫升级为字段表驱动：一份字段表喂 Python/Node 两侧（P2-1 落地） | 复用性 | ✅ 2026-10-25 落地 |
| P2 | 收敛 `bootstrap↔{tabs,loading,mobilebar,webview_hooks}` 双向耦合 | 依赖方向 | ✅ 2026-10-25 落地（新建 `nav.rs` 枢纽 + boot 回调反转，六模块 `crate::bootstrap` 归零） |
| P2 | electron `config.example.json` 17 键差异定性（裁剪则文档化，遗漏则补齐） | 漂移风险 | ✅ 2026-10-25 定性为有意裁剪（缺 4 个 Android 签名键），已写入 `docs/packaging-config.md` §8 |
| P3 | `debug_log` 模块注释更新定位；floatprobe 权衡口径写入 build-test.md | 文档 | ✅ 2026-10-25 落地 |

> P1–P3 全部落地，详情见 `docs/changes/2026-10-25.md`。

刻意不动：`store↔settings` 互指、`WindowGeom` 归属、锁守卫样板 —— 依据同 2026-09-29 §4。

## 6. 复检（2026-10-25 当日，批次①②③落地后重新实测）

**依赖图复测**（`crate::x` 依赖边，含新 `nav.rs`）：

| 模块 | 依赖 | 复检结论 |
| --- | --- | --- |
| `nav.rs`（新） | config | ✅ 干净枢纽（仅 `dlog!` 宏属日志基建） |
| `bootstrap.rs` | config, loading, mobilebar, nav, proxy, resolve, tabs, webview_hooks | ✅ 纯编排向下边 |
| `tabs / loading / mobilebar / webview_hooks` | config, nav(+各自下游) | ✅ **对 bootstrap 上行边 = 0** |
| `settings / debug_log / floatprobe / tray` | nav（不再指 bootstrap） | ✅ |
| `floatwin / store / settings / tray` | 互指 3 组 | ⚠️ 保留（契约性互指，见下） |

- **双向耦合 6 组 → 3 组**：`settings↔store`（同一 settings.json 读-改-写契约）、
  `settings↔floatwin`、`floatwin↔tray`（悬浮球/托盘开关联动）——均属「刻意不动」类，不在 P2 行动项内。
  P2 目标的 4 组（bootstrap↔tabs/loading/mobilebar/webview_hooks）**全部消除**。
- **验收点**：六功能模块 `crate::bootstrap` 出现次数 0；`contract_tests/` 79 条（拆分 75 + 数据驱动 4）；
  `cargo test --all` 282 通过 / 0 failed；`pack-local --selftest` ✓、`--parity` 55/55（含后续改动复跑）。
- **残余观察（无行动项，供后续批次参考）**：
  1. ~~三个大文件未拆~~ —— **已于批次④（P4）垂直拆分落地**（零行为变更纯代码搬家）：
     `floatwin.rs` 1,656 → 根 575 + `floatwin/{events,shape,window,tests}`；`tabs.rs` 1,375 →
     根 66 + `tabs/{state,window,ipc,tests}`；`proxy.rs` 1,411 → 根 358 + `proxy/{auth,ws,headers,tests}`
     （sidecar `#[path]` 复用不破：子模块声明带显式 `#[path]`，w2a-proxy 32 测试全过）。
     残留 `tabs/window.rs` 817 行 / `proxy/tests.rs` 692 行（纯测试）超 600 量级，可视需要再拆；
  2. `debug_log → 6 个业务模块` —— P3 已钉「诊断 + 启动报告」定位，属预期形态；
  3. Android 目标（`#[cfg(mobile)]`）须过 CI `check-android`；`.github/scripts/__pycache__/*.pyc` 为跟踪遗留文件的
     工作区改动，可 `git checkout --` 还原。
- **结论：§5 行动项 P1–P4 全部闭环，模块层级依赖方向健康，无新增环。**
