# 自动化任务执行记忆 — electron-app 架构审查

## 任务
全面检查 electron-app 代码架构与组织结构（模块划分、职责边界、依赖关系、复用性），输出问题/依据/重构方案，低风险优化直接执行，影响行为的仅提方案。

## 最近一次执行（2026-10-16）
- **结论**：electron-app 组织健康，无职责混杂类严重问题，无值得立即执行的零行为变更重构。
- **产出**：`docs/architecture-review-electron-2026-10-16.md`（根 docs 下，electron-app 侧对等审查报告）。
- **基线取证**：`pnpm run typecheck` ✅、`pnpm run unit` 101/101 ✅。
- **待决策方案 4 项**：P-1 探针隔离（承接 09-29 P1-2）、P-4 normalizeIncoming/parseSettings 收敛规则统一、P-3 index.ts 瘦身（onSpawn 抽取 + logStartup 迁移）、P-5 IPC 命令集中真源。
- **刻意不动 6 项**：mustBool/toBool 语义不同、两处 about: 拒绝、toNum vs float num、escapeHtml/jsStr、三份 preload 白名单、JS/Rust stripUserinfo 跨语言。
- **代码改动**：0（未执行任何改动，onSpawn 抽取经评估判定为"可能影响接口"留在方案清单）。

## 关键背景（跨轮复用）
- Rust 侧审查在 `docs/architecture-review-2026-09-29.md` 与 `architecture-review-2026-10-16.md`，本任务是对等的 Electron 侧审查。
- electron-app 分层纪律：纯规则层（hotkey/tab-order/float-layout/distribution/startup-log/first-existing）零 Electron 依赖，可被 node:test 直接单测，是新增模块的既定模板。
- 变更后验证命令：`pnpm run typecheck` / `pnpm run unit` / `pnpm run probe` / `pnpm run probe --tabs`。
