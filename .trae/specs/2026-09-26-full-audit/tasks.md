# 任务队列（2026-09-26 全流程审计）

每条含：落点 / 方案 / 预期 / 影响 / 成本 / 验证要求。状态：pending → in_progress → completed。

## P0

- [x] **T1（P0）** `webview-app/README.md` tabs_enabled 节（约 425-433 行）
  - 改：「字符串布尔，默认 "false"」→「布尔，默认 false（不带引号）」；示例 `"tabs_enabled": "true"` → `"tabs_enabled": true`；正文「设为 "true"」→「设为 true」。
  - 预期：照文档粘贴的配置能过解析器。影响：无行为变化。成本：3 行。验证：与 parse-packaging-config.py:210-220 行为对照。

- [x] **T2（P0）** `electron-app/README.md` 122-143 行
  - 改：sha256 取证改为 asar 内路径不可直接 certutil（给出 `npx asar extract` 或「以 out/inject 随 app.asar 分发，核对源码 sha + 重新打包」的正确做法）；「三件套」改成「Web2App.exe + resources\app.asar 两件」，删除「只换注入脚本」旧法。
  - 验证：引用路径全部存在（out/inject 在打包后 asar 内这一事实与 stage-assets.cjs:114-128、electron-builder.yml 一致）。

## P1 代码

- [x] **T3（P1）** closeTab 邻接 bug（electron-app/src/main/tabs.ts:161-193）
  - 方案：删除前快照 labels 与 removedIndex；新增导出纯函数 `nextActiveLabelAfterClose(labelsBefore, removedIndex)`（规则对齐 tabs.rs:515：`remaining[min(removedIdx, len-1)]`；空数组返回 null）。closeTab 改用它。
  - 测试：新增 `electron-app/test/tabs.test.ts`（node:test，导入纯函数；覆盖关中/关末/关首/仅剩 1 个/空数组）。
  - 探针：runTabsProbe P7 后新增 4 条 3 页签邻接断言（26 → 30）；P9 收敛段泛化 opener 变量（不再硬编码 label2）。
  - 影响：多页签关闭焦点行为变化（修复）。成本：~60 行含测试。
  - 验证：先对旧实现跑新单测确认 FAILED（反向对照），修复后 PAS；探针 --tabs 实跑 30/30。

- [x] **T4（P1）** 首签失败可见错误态（electron-app/src/main/index.ts）
  - 方案：新增 `tabs:retry_first_tab` IPC（registerTabsIpc，assertShell 校验）：移除 shell 遮罩（存在则）→ tabManager.spawnTab() → 再失败重新挂遮罩；成功则移除遮罩。shell-preload ALLOWED 加 `retry_first_tab`。
  - 首个页签 spawnTab().catch：向壳 webContents executeJavaScript 注入遮罩 DOM（38px 以下，文案 + 「重试」按钮调 `__TAURI_INTERNALS__.invoke('retry_first_tab')`）。JS 文本集中为常量并 escape 细节（detail 经 JSON.stringify 嵌入）。
  - 影响：仅失败路径新增可见 UI；成功路径零变化。成本：~70 行。
  - 验证：typecheck；代码审查；探针 --tabs 30/30 不回归（GUI 真机目视列遗留）。

## P2 代码

- [x] **T5（P2）** sidecar.ts（electron-app/src/main/sidecar.ts）
  - 握手成功行：proxyUrl 必须是非空且 http(s) 开头，否则按 `kind:'protocol'` reject 并 killHard（避免 navigate.ts loadURL('')）。
  - spawn 后立即 `child.stdin?.on('error', …)`：EPIPE/EOF 类异步错误只记日志（退出由 exit 事件兜），避免无监听器抛异常。
  - 验证：typecheck + 现有探针 sidecar 断言不回归。

- [x] **T6（P2）** applySettings 回滚（index.ts:936-980）
  - 记录「本次调用是否新建了托盘」；catch 中除回滚快捷键外，新建过托盘则 destroyTray() 并 warn。
  - 验证：typecheck；探针 M3 设置链路不回归。

- [x] **T7（P2）** tabs-v4.cjs 启动修复
  - 文件顶部检测 `require('electron')` 返回字符串（node 模式）→ spawn electron 以本文件 + 原参数重跑（对齐 layout-shot.cjs:205-209，删 ELECTRON_RUN_AS_NODE、stdio inherit、透传退出码）；electron 模式下才解构 API。
  - 验证：`pnpm run tabs-v4 --negative` 至少能被 electron 承载（%TEMP%\w2a-t0-v4\booted.txt 刷新；退出码可能 1/2，不出现 node TypeError）。

- [x] **T8（P2）** 死代码与过期注释
  - shortcuts.ts 删 `currentWakeAccelerator`（全仓零调用，模块内变量保留）。
  - sidecar.ts 删 SidecarHandle 的 `upstreamHost`/`hadAuth` 死字段（43-44、226-227；grep 确认零消费，含 navigate.ts）。
  - tray.ts 删第三候选 `appPath/icons/icon.png`，注释改「两处」。
  - settings-preload.ts:13-14 改无边框表述；settings-window.ts:5 删标题表述（建窗选项无 title，仅任务栏/无障碍取系统默认）；index.ts:337 130→本轮新数；probe.ts:1414 127→新数；electron-ci.yml:24 130→134→新数；proxy-server/main.rs:9 去掉「825 行」易漂移表述。
  - 验证：typecheck + cargo test（proxy-server 注释）。

## P1/P2 文档

- [x] **T9** webview-app/README.md：workflow 6→7（正文列全 7 条并点名 electron-ci.yml）；ci.yml 节补 config-contract 第四作业；删「+」按钮表述（425 节与 29 行功能表「随时新建」措辞改为「由页面 target=_blank/window.open 新开」）。
- [x] **T10** electron-app/README.md：①337-339 矛盾句改为「配置随 asar，改配置必须重新打包（与 Tauri 差异在编译期 vs 打包期，都不是运行期改）」；②526「60 条 11–15」→「61 条 11–14」；③468「825 行」去行数；④580-582 轶事去掉易漂移精确数（改「数十处」+ 不写行号）；⑤453/510 探针数与 39/134 等表述按本轮实测更新；⑥变更记录：顶部加 2026-09-26 行，把末尾两条错位的 09-24 行移到表首区。
- [x] **T11** 根 README.md：§四索引无需动；§六新增 v1.35（09-26：注入资源随 app.asar 分发 + 共享真源 tooltip/文案 + 本轮三类清单审计修复，含探针 26→30、unit 29→35）。
- [x] **T12** docs：electron-plan.md 文件清单 13→15/3→4（补 page-behavior.ts、web-data.ts、shell-preload.ts）；契约 #11 补 splash.html、dist/settings.html；§1.1 portable 旧句标注【历史犹豫】；electron-size-reduction.md:198 112 加「时点记录」标注；electron-analysis.md:28 62KB 标注或删除该行（实测 54KB）。

## 收口

- [x] **T13** 门禁全复测：cargo fmt/clippy/test --all；pnpm typecheck/unit；探针单窗口 + --tabs（GUI）；记录实测条数。
- [x] **T14** 独立 Review（fresh context 子代理，只读）→ review.md；问题回环后输出最终报告。
