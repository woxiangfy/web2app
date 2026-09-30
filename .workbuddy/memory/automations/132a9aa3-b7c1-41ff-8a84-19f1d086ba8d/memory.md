# 自动化执行记录：项目全盘分析与优化建议

## 2026-09-14（首次执行）
- 任务：对 webview-app 项目全盘分析 + 优化建议（不改变功能点）+ 合理化建议。
- 执行结果：通读 3 crate 全部源码、5 个注入脚本、CI/打包工作流；实测 `resolve-core`+`resolve-tool` 19 个单测全过。
- 产出：`C:\dev\rust\web-app\PROJECT_ANALYSIS.md`（含架构地图、优点、优化清单、功能/UI 建议、真机待办）。
- 关键结论：工程整体质量高；可安全落地的优化集中在 proxy.rs WS 帧去拷贝(P1)、sanitize 去重(P1)、打包强制非默认 RESOLVE_SECRET(P1)；功能建议优先级为「新建页签入口/窗口标题同步 title/设置页展示入口信息」。
- 未做代码改动（约束为"不改变功能点"，且无人值守不宜动跨平台 CI 敏感代码）。

## 2026-09-25（第二次执行）
- 任务：现有文档精简更新（删失效/过期/不符实现的内容、压缩冗长段落、校正术语参数示例；保结构、保语气，不增未确认信息）。
- 执行方式：先实跑建「代码事实基线」，再逐份核对；拿不准的标「待确认」而非删除。
- 实跑取证：`cargo test --all` 133 通过 / 6 ignored；Electron `node --test` 29；探针 134/134 与 26/26；workflow 7 条；契约 18 键。
- 改动 11 份文档（README.md + docs/ 下 10 份），根 README 新增 v1.34 变更行；未动任何代码。
- 三类改动：①计数/行号/路径按实测订正；②过期实现描述校正（asar 内配置、页签「+」已移除、38px 让位写法、17→18 字段、布尔只认真 bool）；③历史快照类只加「已失效」标注、保留时点数字。
- 过程与可复用原则记在 `.workbuddy/memory/2026-09-25.md`。

## 2026-09-29（第三次执行）
- 任务：代码架构与组织结构全面检查（模块划分/职责边界/依赖/复用性）；低风险零行为变更的直接执行，影响行为或接口的只提方案。
- 产出：`docs/architecture-review-2026-09-29.md`（问题+依据+方案+刻意不动项）；过程明细在 `.workbuddy/memory/2026-09-29.md`。
- 已执行 4 项代码重构（行为中性，门禁全绿）：
  ①「主窗口 label」4 处重复 → `bootstrap::main_window_label()` / `sync_tabs_visibility()`（落点选 bootstrap 因其无条件编译，tray 是 cfg(desktop)、tabs 是 cfg(windows)，而 handle_main_close 在 Android 也走）；
  ②`tabs::tab_token_matches()` 统一两处重复的安全校验；
  ③`tabs::tab_rows()` 让 IPC 与标签栏共用同一快照；
  ④`store::update_at` 加 `#[cfg(test)]`（实测仅测试调用的隐形死函数）。
- 取证方法：临时摘 `#![allow(dead_code)]` + `cargo check --all-targets` 查真死代码（暴露 1 处，已还原）。
- 门禁（前后逐位一致）：fmt ✅ / clippy `-- -D warnings` 0 告警 ✅ / build ✅ / `cargo test --all` **144 passed（13+7+29+95）/ 6 ignored / 0 failed**。
- 未执行、仅提方案：P1-1 lib.rs 启动日志 5 函数迁出（lib.rs 被 include_str! 自解析，需单独提交+守卫反向对照）；
  P1-2 Electron probe.ts 1,852 行打进产物（out/main/probe.js 86,185 字节 = 该目录最大文件）；
  P1-3 `#![allow(dead_code)]` 全局豁免毯收窄（需 Android 目标覆盖，本机无 NDK）；
  P2-1 配置解析两份实现（py vs mjs 复刻关系，已知债）；
  P2-2 `.github/scripts/__pycache__` 2 个 .pyc 已被 git 跟踪（待用户确认后 `git rm --cached`）。
- 刻意不动：debug_log 手写日期、WindowGeom 归 store、37 处 lock 样板（批量替换会改变临界区，非无行为影响）。
