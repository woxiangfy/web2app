# 第五轮全流程审计闭环（2026-09-27）- 需求规格

## Overview

- **Summary**：对 web2app 双壳仓库（`webview-app` Rust+Tauri 2 / `electron-app` Electron+TS）做第五轮「不足 / 冗余 / 缺失」三类清单审计，逐项最小修复并复测，最后专项更新 README 与出具审计报告。
- **Purpose**：用户要求按既定四轮审计的同一口径自主完成全流程；工作区当前干净、基线门禁全绿，本轮目标是把已复核的功能/安全/契约漂移缺陷收敛到「可提交、可发布」。
- **Target Users**：双壳维护者与最终用户（Windows 为主；Android 走 CI 交叉验证）。

## 业务目标（原始诉求未指定具体功能，按惯例【默认补全】）

- 【默认补全】本轮无新功能诉求，目标 = **把独立复核确认的存量缺陷清零，门禁保持全绿，文档与实现重新逐条对齐**（与 audit-2026-09-20 / 09-23 同一口径）。
- 【默认补全】修复优先级判定：P0=功能错误/安全/数据损坏且在支持配置下可触发；P1=健壮性缺陷、边界 bug、双壳契约漂移、安全纵深缺口；P2=冗余/死代码/过期注释/小边界。
- 【默认补全】所有信息缺口按行业通用默认处理：最小修改优先、不做大重构、可单测的逻辑抽纯函数、安全失败必须可见（项目既有铁律）。

## Goals

- G1：修复 Electron 侧 6 项 P1（失败无限重试、幽灵页签、托盘回滚不对称、子框架特权桥、public_key 归一化漂移、目录名清洗漂移）。
- G2：修复 Rust/Tauri 侧 3 项 P1（tab 标题 IPC 死路、tab/宿主缺外部代理参数、直连模式凭据外发缺口）。
- G3：收敛 P2 健壮性与冗余项（scheme 校验、tab 失败态、bootstrap 竞态、逐跳头、邻接选择纯函数化等）。
- G4：文档/README 专项：清除失实、压缩低价值、补充本轮变更；新增 `docs/audit-2026-09-27.md`。
- G5：全部改动有自动化回归（单测/探针/静态守卫），门禁实测全绿。

## Non-Goals

- 不做未被证据支持的推断性改动：WebSocket 子协议透传、WS 半关闭排空/消息上限、w2a_key 恒定时间比较/限流（本机回环+一次性 256-bit 密钥威胁模型下可接受）。
- 不做 store.rs 跨进程文件锁（当前全部写入方在主线程，不可达；只加注释固化前提）。
- 不重构 `#![allow(dead_code)]` 结构（Android cfg 依赖，改动跨平台风险大）。
- 不做真机 GUI 目视（环境限制；探针可覆盖 Electron，Tauri 侧靠单测+框架源码证据，遗留项显式标注）。
- 不提交 git（用户未要求）。

## Functional Requirements

- **FR-1**（E-P1-1）：Electron 启动失败时不得自动无限重试；只有首次 splash 加载自动 bootstrap，失败后等待用户点重试。
- **FR-2**（E-P1-2）：Electron 关闭任意页签（含非活动页签）后壳标签栏 DOM 必须立即与页签集合一致。
- **FR-3**（E-P1-3）：Electron 设置保存任一步失败时，本次新建/销毁的托盘与快捷键都必须回滚或重建。
- **FR-4**（E-P1-4）：Electron preload 不得向子框架（含跨源 iframe）暴露 `window.web2app` / `web2appExtra`，也不在子框架发起 bootstrap 同步 IPC、不注册 overlay 监听。
- **FR-5**（E-P1-5）：Electron `public_key` 必须与打包解析器同规则归一化（字面量 `\n`→真换行、去 `\r`、trim），PEM 与裸 base64 都可用。
- **FR-6**（E-P1-6）：Electron `sanitizeDirName` 必须与 Rust `sanitize_dir_name` 同规则（压缩连续空白、去首尾空白与首尾句点、空回落）。
- **FR-7**（R-P1-1）：Tauri 多页签真实标题必须能回传：远程页面走 core event（非应用命令），capability 覆盖 `tab-*` + remote urls + `core:event:allow-emit`，Rust 监听落地；并有静态契约守卫。
- **FR-8**（R-P1-2）：Tauri 多页签模式宿主窗口与每个 tab 窗口必须带与主窗口一致的 `additional_browser_args`（外部代理开关）。
- **FR-9**（R-P1-3）：Tauri 直连模式（不走本地代理）下，Authorization 注入与 BasicAuthenticationRequested 自动应答只允许发给入口上游主机（含默认端口归一化）；Electron login 已有同规则，两壳对齐。
- **FR-10**（P2 集合）：scheme 校验、tab 失败态可见+重试、bootstrap 竞态去重、响应逐跳头剥离、邻接选择纯函数+测试、托盘 panic 点、代理测试 no_proxy、Electron 设置 IPC sender 校验、spawnTab 在途防护、sidecar 异常退出可观测、小冗余清理。
- **FR-11**：`WEB2APP_DEBUG` 真值集合两壳逐字一致（`1`/`true`/`yes`，大小写不敏感按 Rust 侧现状）。
- **FR-12**：README/docs 清除 4 项 P1 失实与若干 P2 漂移，登记本轮变更与审计文档。

## Non-Functional Requirements

- **NFR-1**：最小修改、不改变既有正确行为；每项改动可独立回退。
- **NFR-2**：安全相关改动默认失败（拒绝）而非放行；错误必须可见不静默。
- **NFR-3**：跨平台 cfg 纪律：改动后 Windows 本地全绿，并静态核对非 Windows cfg 不引入 warning。
- **NFR-4**：双壳单一真源（titlebar.js/loading.js/shell.html/settings.html/proxy.rs）改动后两侧语义仍一致；共享脚本改动必须同时验证两条链路。

## Constraints

- **Technical**：Windows 主机；Rust stable + Tauri 2.11.5 / wry 0.55.1；Electron 44 + TS 7（node:test 毫秒级单测）；探针需 GUI（sRGB.icm 沙箱噪声不影响断言结论）。
- **Business**：历史硬约束（Content-Length 原样透传、WS Origin 重写、w2a_key 404 门禁、代理日志规则）不得破坏。
- **Dependencies**：Electron 侧复用 `w2a-proxy` sidecar（`#[path]` 复用 proxy.rs，物理同文件）。

## Assumptions

- 【默认补全】「行业通用默认」= 最小修复 + 纯函数抽离 + 自动化回归 + 文档同步。
- 【默认补全】外部代理 + TABS_ENABLED 虽为小众组合，仍按支持配置修复（FR-8）。
- 【默认补全】探针「134」为运行期正确计数：静态 135 个调用点中「sidecar 握手成功/失败」两条互斥分支恰执行其一（已 diff 实证），不是文档漂移。

## Acceptance Criteria

### AC-1：门禁全绿
- **Type**：`rule`
- **Pass Condition**：`cargo fmt --all --check`、`cargo clippy --all-targets -D warnings`、`cargo test --all` 全绿；Electron `typecheck`、`node --test` 全绿；探针单窗口与多页签两组退出码 0（计数据实更新）。
- **Evidence**：本会话实跑命令输出。

### AC-2：P1 修复各有回归证据
- **Type**：`rule`
- **Pass Condition**：FR-1~FR-9 每项至少一条自动化证据（新增/修改的单测、探针断言、静态守卫），且对新增守卫做反向对照（故意改坏必须 FAIL，再还原）。
- **Evidence**：测试输出 + 反向对照记录。

### AC-3：双壳契约不漂移
- **Type**：`rule`
- **Pass Condition**：public_key 归一化、sanitizeDirName、debug 真值、凭据主机白名单四处两壳行为逐字对齐（各有跨壳引用的同组测试用例）。
- **Evidence**：单测用例对照 + 代码审查。

### AC-4：文档与实现一致
- **Type**：`rule`
- **Pass Condition**：本轮触及的所有计数/路径/行为表述均经实跑或读真源核实；新增 `docs/audit-2026-09-27.md` 并在根 README §四登记；根 README 增 v1.37 变更行。
- **Evidence**：文档 diff 与真源逐条对照。

### AC-5：改动质量与最小性
- **Type**：`rubric`
- **Dimension**：最小修改、注释充分、无副作用外溢、共享真源双壳一致
- **Scale**：1-5；**Anchors**：1=大重构/引入新风险；3=修复正确但有冗余或注释缺失；5=最小闭环、注释与测试齐备、无行为外溢；**Pass Threshold**：>= 4
- **Evidence**：独立审查代码 diff。

## Open Questions

- 无（用户授权全程自主；缺口均按【默认补全】处理）。
