# 全流程审计（2026-09-26，第三轮三类清单）

## 一、业务目标与验收标准

### 1.1 目标

【默认补全】本轮无用户指定的新功能诉求。按前两轮审计（v1.28 / v1.29 / docs/audit-2026-09-23.md）的同一口径，
目标定为：**对当前 HEAD（301aab9，工作区干净）做一次「不足 / 冗余 / 缺失」全流程审计并闭环**——
真实代码缺陷修复、文档与实现重新对齐、门禁全绿、09-26 未登记的实质改动补登变更记录。

用户明确授权：自主推进、不追问，模糊项按行业通用默认处理并标注【默认补全】；最小修改优先；
每个改动函数/文件配测试或探针断言；环境不支持时启动即报错的偏好体现在「不静默失败」类修复中。

### 1.2 验收标准（AC）

| # | 类型 | 判据 | 取证 |
| -- | -- | -- | -- |
| A1 | rule | closeTab 邻接选择与 Tauri `close_tab_inner` 语义一致（关中间→右邻、关末尾→左邻、关首个→右邻） | node:test 纯函数用例 + 探针 3 页签断言，且对旧代码做反向对照（红→绿） |
| A2 | rule | 多页签首个页签创建失败时壳内有**可见**错误文案与重试入口，重试成功后遮罩消失 | 代码审查 + typecheck（GUI 目视列入遗留，本机探针 GUI 可跑则实测） |
| A3 | rule | sidecar 握手成功行缺 proxyUrl 时按 protocol 失败处理；stdin 异步 error 不炸主进程 | typecheck + 代码审查 |
| A4 | rule | 应用设置保存：托盘已新建但持久化失败时，本次新建的托盘被回滚 | 代码审查 |
| A5 | rule | `pnpm run tabs-v4` 在 node 下能自动转由 electron 承载（不再 TypeError 崩溃） | 实跑脚本，electron 进程能启动（booted.txt 刷新） |
| A6 | rule | Rust 门禁全绿 | `cargo fmt --all -- --check` / `clippy --all-targets -D warnings` / `test --all` |
| A7 | rule | Electron 门禁全绿 | `pnpm run typecheck` / `pnpm run unit` / `probe` 单窗口与 `--tabs` 退出码 0 |
| A8 | rule | 文档中所有被本轮改动影响的计数/路径/描述与实测一致（workflow=7、bool 示例无引号、探针新条数、无 resources\inject 旧路径） | grep + 实跑计数 |
| A9 | rule | 09-26 实质改动（注入资源随 asar、titlebar/shell tooltip、settings 文案）登入根/子 README 变更记录 | git 变更记录条目 |
| A10 | rubric | 独立 Review 确认无「假绿」：新增守卫能咬人、每条问题有证据、无超范围改动 | review.md |

### 1.3 范围外（刻意不改，附理由）

- window.open 内部 scheme 白名单：Tauri 侧同样不筛（双壳一致性），风险为【推断】未真机证实；
- console.info/debug/trace 静默：文档显式承诺「三路」，扩面超出最小修改；
- config.ts ENOENT 分支：公开导出函数的防御分支，非死代码；
- typecheck 覆盖 test/：rootDir=src，修复需新增 tsconfig 文件，收益低；
- 混合 DPI 几何、sidecar 握手后看门狗等【推断】项：无真机不复现，记录入遗留；
- 两份 2026-09-22 过程稿归档、变更记录中段历史排序、时点数字：v1.34 已决定保留历史形态；
- Trilium 树拖拽（dnd_polyfill）：项目记忆中 parked 的开放问题。

## 二、审计发现（汇总，逐项方案见 tasks.md）

### P0（文档照做即失败）
1. webview-app/README 把 `tabs_enabled` 写成「字符串布尔」且示例带引号 —— 解析器 v1.16 起只认真 bool，照抄打包直接报错。
2. electron-app/README「先确认哪版包」的取证命令指向 09-26 起已不存在的 `resources\inject\*.js`，「三件套覆盖」建议不成立。

### P1 代码
3. Electron `TabManager.closeTab` 删除后再算索引（恒 -1 → 恒激活第一个幸存页签），与 Tauri 契约分叉；探针仅 2 页签放过。
4. 多页签首个页签 spawn 失败：空白壳 + 默认 debug=false 时 console 静默，零提示零重试（单窗口有 pendingFail，页签模式无）。

### P1 文档
5. webview README workflow 停在 6 且漏 electron-ci.yml；ci.yml 节漏 config-contract 作业。
6. webview README 仍写标签栏「+」按钮（已有意移除）。
7. electron README：config「运行期读改完重启即生效」与「随 asar 必须重打包」自相矛盾；探针「60 条/11–15 节」实为 61 条/11–14 节；「825 行」「29 处 */第 70 行」过期。
8. 过期代码注释：settings-preload.ts、settings-window.ts、index.ts:337、probe.ts:1414、electron-ci.yml:24、proxy-server/main.rs:9。

### P2 代码
9. sidecar 握手成功行不校验 proxyUrl（畸形 → loadURL('')）；stdin 异步 error 无监听（EPIPE 可炸主进程）。
10. applySettings 托盘新建成功但落盘失败时不回滚托盘。
11. tabs-v4.cjs 直接 require('electron')，node 下必崩（electron 承载脚本误当 node 脚本）。
12. 死代码：shortcuts.ts `currentWakeAccelerator` 零调用；sidecar.ts `upstreamHost/hadAuth` 零消费；tray.ts 永不命中的第三图标候选。

### P2 文档
13. electron-plan.md 文件清单（13/3 → 15/4）、契约 #11 单一真源缺 splash.html/settings.html、§1.1 portable 旧句；electron-size-reduction 附录 112；electron-analysis 62KB。
