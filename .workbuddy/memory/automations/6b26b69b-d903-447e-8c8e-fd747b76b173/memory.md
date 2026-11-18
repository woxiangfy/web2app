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
- 探针可直接 `pnpm run build && node scripts/launch.cjs --probe [--tabs]`（省掉 sidecar 编译，单次约 50s）。
- 反向对照的省事做法：改源码前把两个文件 cp 出去，`sed` 改成坏版本 → build → 探针应 FAIL → cp 回来 → `sha256sum -c`。

## 最近一次执行（2026-11-18，任务：Electron 密码保存功能复核）
- **结论**：加密/落盘/sender 闸门/填充触发时机都对；坏在「确认登录成功才落盘」的**同一文档(SPA)**那一半，
  原探针只覆盖跨文档跳转所以全绿漏过。
- **修 4 处**：① 观察器回调无条件提前 return ⇒ SPA 同文档登录永不落盘；② 新文档 1.2s 确认探针会把本页
  刚提交的暂存按 still-login 丢弃；③ `lookup` 重加密回写用旧 doc 快照（覆盖并发保存/复活已删条目）；
  ④ 清除全部浏览数据清不掉待落盘暂存。附带：maybeCommit 拿到任何明确答复都置 stagedHere=false、注释里的
  测试文件名笔误。
- **新增**：夹具 `/login-spa.html`（默认延迟 600ms 隐藏表单，`?remove=1` 同步移除）+ 探针 18i/18j +
  `password-contract` 2 条装配契约（共 215 unit）。
- **验证**：typecheck/build ✅、unit **215/215**、单窗口 **178/178**、页签 **69/69**；反向对照
  改回无条件早退 ⇒ **176/178**（18i/18j FAIL），还原后两文件 sha256 与改前相同。
- **文档**：`docs/changes/2026-11-18.md` 新增「补充（同日·五次）」、`electron-app/AGENTS.md` 约束 18 补两条不变量、根 `AGENTS.md` 摘要行更新。
- **未改（已知取舍）**：新文档 1.2s 时若登录表单是延迟渲染（>1.2s）仍可能误判成功；2FA 中间页无密码框会提前落盘；
  `safeStorage` 不可用时静默（Windows DPAPI 恒可用，未加气泡）。

