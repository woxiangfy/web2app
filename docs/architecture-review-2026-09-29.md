# 架构与组织结构审查 — 2026-09-29

> ⚠️ **本文是 2026-09-29 的时点快照**：其中的模块行数、测试计数（144）等都是**当时**的实测值，后续版本演进**不回填**；
> 只在「结论已失效」处补一行现值。现行状态一律以根 `AGENTS.md`、`webview-app/AGENTS.md`、`electron-app/AGENTS.md` 为准。

审查范围：`webview-app/`（Rust + Tauri 2，`src/` 22 文件 / 10,330 行）、`electron-app/`（TypeScript，17 个 `src/main` 模块 / 6,568 行）、`scripts/pack-local.mjs`、`docs/` 与 CI 脚本。
原则：职责单一、边界清晰、结构合理、依赖简单、**最小重构**。

## 0. 总体结论

工程组织水平偏高：模块边界有显式文档、跨文件字面量契约有静态守卫（含反向对照纪律）、平台差异一律下沉到模块内部（`invoke_handler` 只调用一次等踩坑都以注释固化）。
本次未发现「职责混杂到影响正确性」的严重问题；可落地的收益集中在**重复实现的收敛**与**产物/清单卫生**两类。

| 类别 | 数量 | 本次处理 |
| --- | --- | --- |
| 已执行（零行为变更） | 4 | ✅ 全部通过门禁 |
| 待决策方案（可能影响行为/接口） | 4 | 📋 仅提方案 |
| 刻意不动（动它反而更差） | 3 | ⛔ 说明依据 |

---

## 1. 模块现状地图

| 层 | 模块 | 行数 | 职责纯度 |
| --- | --- | --- | --- |
| 入口 | `main.rs` | 7 | 纯转发 ✅ |
| 装配中枢 | `lib.rs` | 1,188 | ⚠️ 见 P1-1（装配 + IPC 命令 + 启动日志混住） |
| 配置 | `config.rs` | 587 | 编译期常量 + 目录解析，纯函数 ✅ |
| 持久化 | `store.rs` / `settings.rs` / `window_state.rs` | 678/328/114 | 三段分离清晰 ✅ |
| 网络 | `bootstrap.rs` / `proxy.rs` / `resolve.rs` | 417/1,311/227 | ✅ |
| 平台 | `webview_hooks.rs` / `save_dialog.rs` / `tabs.rs` | 814/127/1,174 | Windows 专属，cfg 干净 ✅ |
| 基础设施 | `debug_log.rs` / `loading.rs` / `tray.rs` / `web_data.rs` | 316/259/120/252 | ✅ |

依赖方向健康：无循环依赖；`config` 是最底层（被 8 个模块依赖），`tabs` / `webview_hooks` 为平台叶子节点。

---

## 2. 已执行（低风险、零行为变更）

### ✅ A-1 收敛「主窗口 label」判断 — 4 处重复 → 1 处

**依据**：同一条「多页签 → 宿主窗口 label，否则 → `"main"`」判断散在 4 处（实测 grep 取证）：

| 位置 | 原形态 |
| --- | --- |
| `lib.rs:205`（单实例聚焦） | `if tabs::tabs_enabled() { HOST_LABEL } else { "main" }` |
| `lib.rs:493`（退出时几何持久化） | 同上，再写一遍 |
| `lib.rs:669-682`（主窗关闭语义） | 同上，第三遍 |
| `tray.rs:12`（托盘切换/菜单） | 同上，第四遍 |

风险：改一处忘另一处的表象是「托盘点开、或第二次启动聚焦到的不是那个窗口」这类难复现错位，且**编译期零告警**。

**方案**：提升为 `bootstrap::main_window_label()`。
❗ 为什么不放在 `tray.rs`（已有实现）或 `tabs.rs`：`tray` 是 `cfg(desktop)`、`tabs` 是 `cfg(windows)`，而主窗关闭语义 `handle_main_close` 在 Android 上同样要走 —— 放进去会让非桌面目标拿不到该函数。最终落在**无条件编译**的 `bootstrap.rs`，与该文件已有的 `local_proxy_enabled()`（同样是编译期派生 + cfg 吸收）同构。
同步把 `sync_tabs_visibility()` 一并上提（`lib.rs` 2 处 + `tray.rs` 1 处重复）。

净效果：`lib.rs` 减 44 行、`tray.rs` 减 30 行，新增 39 行集中在单一实现处；**调用点的 cfg 分叉全部消失**，消除了「cfg 决定可变性」这类历史事故面。

### ✅ A-2 抽出 `tabs::tab_token_matches()` — 2 处重复的安全校验

**依据**：`report_tab_title`（标题回传）与 `retry_tab_bootstrap`（首屏重试）各自实现了一遍「label + token 命中在册记录」校验。
这条是**安全边界**：Tauri 2.11 的 `app.listen` Event 不携带发送窗口标识，伪造防护完全依赖它。重复实现的风险是只加固一边。

### ✅ A-3 抽出 `tabs::tab_rows()` — IPC 与界面不再是两个真源

**依据**：`get_tabs`（IPC 命令）与 `push_tabs_to_shell`（标签栏渲染）各自实现了一遍「取三把锁 → 标题取缓存、缺失回落 → 拼 active 标志」。两处漂移时，现象是**IPC 返回的标题与标签栏显示的不是同一个**，静默且无报错。
抽出时同时固定了锁顺序（`tabs → active → titles`），并在文档里注明与 `set_tabs_visible` 保持同序以避免 AB-BA 死锁。

### ✅ A-4 `store::update_at` 标记 `#[cfg(test)]` — 清掉一个隐形死函数

**取证方法**：临时摘掉 crate 根的 `#![allow(dead_code)]` 后 `cargo check --all-targets`，实测暴露 1 处真死代码：

```
warning: function `update_at` is never used
```

确认其调用方只有 `store.rs` 内 3 个单测，是**测试专用helper 住在生产代码里**。加 `#[cfg(test)]` 后生产符号消失。验完即还原 `lib.rs`（未改动该 blanket，见 P1-3）。

### 🔬 行为一致性验证（改动后）

| 门禁 | 结果 |
| --- | --- |
| `cargo fmt --check` | ✅ 通过 |
| `cargo clippy --all-targets -- -D warnings` | ✅ 0 error / 0 warning |
| `cargo build` | ✅ Finished |
| `cargo test --all` | ✅ **144 passed / 6 ignored / 0 failed** |

测试数与改动**前后基线逐位一致**（resolve-core 13 + resolve-tool 7 + w2a-proxy 29 + web2app_lib 95 = 144，忽略 3+3 不变）。
受影响的自研静态守卫 `ipc_command_registry`（3 条）与 `capability_contract`（6 条，`include_str!` 依赖文件存在）全部随测试通过；未触碰 `generate_handler!` 注册块与任何事件名字面量。

---

## 3. 待决策方案（可能影响行为/接口，仅提方案）

### 📋 P1-1 `lib.rs` 是「装配中枢 + IPC 命令 + 启动日志」三合一

> ✅ **2026-10-12 现值：已落地（零行为变更）**。四个函数（`hotkey_label` 已先落
> `settings.rs`）已迁入 `debug_log.rs`，`lib.rs` 只留一次 `debug_log::log_startup(...)`；
> 新增静态守卫 `startup_log_lives_in_debug_log_and_not_in_lib` 钉死归属并完成反向对照
> （注入重复定义 → 守卫 FAILED → 还原）。既有 `include_str!` 守卫均不引用这四个函数名，
> 未触发守卫重排。详见 `docs/changes/2026-10-12.md` 第三轮。

**依据**：`lib.rs` 1,188 行里约 120 行是纯格式化职责 —— `log_startup` / `describe_target` / `describe_proxy` / `describe_tabs` / `hotkey_label`。
其中 `hotkey_label()` 与 Electron 侧 `hotkeyLabel()` 同名同形，是跨壳契约的一部分，却藏在入口文件里。

**方案**：把这 5 个函数整体迁到 `debug_log.rs`（它已经是「诊断」的职责所有者），`lib.rs` 只留 `debug_log::log_startup(...)`。
**为什么本次不做**：`lib.rs` 被两组静态守卫用 `include_str!` 自解析（`EVENT_IN_LIB`、`TAB_EVENT_LIB_LISTEN`、`app.listen(tabs::TAB_RETRY_EVENT` 等都对字面量位置敏感），搬迁属于**跨模块的守卫重排**，宜单独一次提交配反向对照（故意改坏 → 期望 FAILED → 还原比 sha256）。

### 📋 P1-2 Electron 的诊断探针混在生产源码树、并被打进产物

**依据（实跑取证）**：

| 事实 | 证据 |
| --- | --- |
| `probe.ts` 1,852 行，位于 `src/main/`（生产源码树） | `wc -l` |
| 被 `index.ts:32` **静态 import**，无法被摇树 | grep |
| 编译产物 `out/main/probe.js` = **86,185 字节**，是 `out/main/` 里最大的文件（> `index.js` 的 76,173） | `ls -la` |
| `electron-builder.yml` 的 `files: - out/**/*` + `asar: true` ⇒ 随包交付 | grep |

诊断脚手架成为发布包里最大的单个 JS 文件。

**方案**（两档，建议选后者）：
- 保守：把 `probe.ts` / `src/renderer/tabs-probe-preload.ts` 移到 `diagnostics/` 顶层目录，`index.ts` 改为**动态 import**（`await import()`）且仅在 `PROBE` 为真时加载。
- 干净：`tsconfig.json` 增加 `diagnostics` 独立编译目标，`files:` 里显式排除该目录。
**风险**：探针必须在**打包产物**上跑（这是它存在的意义），因此不能简单从 `out` 剔除；应先验证「动态 import + 产物内可用」再落地。故本次不执行。

### 📋 P1-3 crate 根 `#![allow(dead_code)]` 是全局豁免毯

> 🟡 **2026-10-12 现值：Windows 侧已闭环，Android 补盲仍挂起**。摘除 blanket 后
> `cargo check --all-targets`（Windows）实测暴出 3 个真死代码（`bootstrap.rs` 的
> `ProbeFailure` / `probe_failure_message` / `probe_ok_message`，仅 `not(windows)`
> 生产路径使用）⇒ 三项改挂 `#[cfg(any(not(windows), test))]`，复检零告警；blanket 按原
> 决策保留（Android 目标本机不可验证），但已补「新增代码不得依赖豁免毯」纪律注释。
> CI 的 `--target aarch64-linux-android` check 作业仍待补。详见
> `docs/changes/2026-10-12.md` 第三轮。

**依据**：本次实测证明它正在掩盖真实死代码（A-4 的 `update_at`）。它存在有正当理由 —— Android 目标下 `DEVTOOLS_ENABLED` 等部分常量本就不参与编译，摘掉会直接让 `-D warnings` 的 CI 挂掉；而本机**做不了非 Windows 目标**（无 NDK），属于盲区。

**方案**：不改 blanket 本身，改为「**新增 code 一律不得依赖它**」：新增分支 item 用行级 `#[allow(dead_code)]` + 注释说明为何该目标用不到。CI 侧建议补一个 `--target aarch64-linux-android` 的 check 作业把盲区补上。
**为什么本次不做**：收窄需要覆盖 Android 目标，本机无法验证。

### 📋 P2-1 配置解析仍存在两份实现

**依据**：`.github/scripts/parse-packaging-config.py`（2026-09-29 时点 37,721 字节 / 18 键；**现 42,168 字节 / 785 行 / 19 键**）与 `scripts/pack-local.mjs`（2026-09-29 时点是 1,510 行单文件；**现已拆成入口薄壳 `pack-local.mjs`（241 行）+ `packlib/` 10 个模块（≈1,626 行）**，仍是前者的**复刻**而非复用）。这是项目已知并已用 `--selftest` / `--parity` 护栏兜住的技术债。

**方案**：把字段表抽成一份 JSON Schema，`pack-local.mjs` 读 JSON、Python 侧同样读 JSON + 保留 `str()` 等校验语义，让两侧从「复刻」变成「同源不同实现」。
**收益**：消除 `为一侧加字段、另一侧同步遗漏` 这一类事故。建议在中大型配置变更窗口做，不要在功能开发中途动。

### 📋 P2-2 仓库清单卫生（零逻辑风险）

`.github/scripts/__pycache__/` 下 2 个 `.pyc` **已被 git 跟踪**（`.gitignore` 第 36 行有注释承认这是遗留）：

```
.github/scripts/__pycache__/check-workflows.cpython-313.pyc
.github/scripts/__pycache__/parse-packaging-config.cpython-313.pyc
```

建议：`git rm --cached -r .github/scripts/__pycache__` 一次性清理（`.gitignore` 已覆盖，不影响工作副本）。涉及版本历史操作，交由你确认后执行。

---

## 4. 刻意不动的地方（并说明依据）

| 项 | 现状 | 为什么不动 |
| --- | --- | --- |
| `debug_log.rs` 手写 ISO-8601（`civil_from_days`，约 25 行） | 零依赖日期实现 | 引入 `chrono` 只为格式化不划算；已有 3 条边界单测（闰年/跨日/纪元）覆盖 ⛔ |
| `WindowGeom` 类型定义在 `store.rs` 而非 `window_state.rs` | 类型归 store、算法归 window_state | 这是**有意**的：两段共写一个 JSON，`store` 是文档所有者；搬走会让「读-改-写」的契约失去单一责任点 ⛔ |
| 37 处 `lock().unwrap_or_else(\|p\| p.into_inner())` 样板 | 分布在 5 个文件 | ❗ **不推荐机械替换**：锁守卫的**生命周期**就是语义（提前 drop 等于提前释放锁）。逐个替换会悄悄改变临界区范围 —— 这不是「无行为影响」优化，应逐个审查而非批量 sed |

---

## 5. 建议的最小落地顺序

1. ✅ **已完成**：A-1 ~ A-4（本次，144 测试全绿）
2. P2-2 清 2 个 `.pyc`（一行命令，零风险；仍待授权执行）
3. ✅ **2026-10-12 已完成**：P1-1 `lib.rs` 启动日志迁出（含新归属守卫与反向对照，241 测试全绿）；同日 P1-3 Windows 侧死代码收窄（3 项精确 cfg + blanket 纪律注释），Android CI 补盲仍排期
4. P1-2 Electron 探针隔离（需先在产物上验证动态 import 可用）
5. P1-3 Android 补盲 / P2-1 需要 NDK CI / 配置窗口等外部条件，排期处理
