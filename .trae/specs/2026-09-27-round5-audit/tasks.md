# 第五轮全流程审计闭环（2026-09-27）- 实施计划

> 优先级：P1 先行（功能/安全/契约），P2 随后（健壮性/冗余/文档）。无 P0（基线全绿、默认配置下无在燃事故）。

> **执行总状态（2026-09-27 收尾）**：Task 1–19 全部 completed（Task 17 的「spawnTab
> 在途锁」经静态分析确认 Node 事件循环下不可达，未加锁，见审计报告遗留 #10；Task 4
> 子框架探针的运行期反向对照受 Electron 44 默认行为限制，已如实降级为纵深防御）。
> 门禁：Rust fmt/clippy=0、144 passed/6 ignored；Electron typecheck=0、unit 39/39、
> 探针 138/138、33/33。反向对照：P7c/标题事件名/capability 三组真咬人。

## P1（high）

### Task 1: Electron 启动失败无限自动重试
- **Status**: completed
- **Completion Evidence**: index.ts `splashAutoBootConsumed`；探针两模式零回归（138/33）。
- **Priority**: high
- **Depends On**: None
- **Description**: index.ts splash `did-finish-load` 仅在「每进程首次自动启动」时自动 bootstrap；失败重载 splash 后必须等用户点 /retry。新增 `autoBootstrapConsumed`（或等价）状态；/retry 链路行为不变（显式重试不消费自动名额，失败后同样不再自动续跑）。
- **AC**: FR-1 / AC-2
- **Test Requirements**:
  - `rule` TR-1.1：连续两次 splash did-finish-load（第二次是失败重载）只自动 bootstrap 一次——用纯函数/可注入时钟单测或结构断言（node:test）；探针既有用例仍 134/134。

### Task 2: Electron 关闭非活动页签后壳 DOM 不刷新（幽灵页签）
- **Status**: pending
- **Priority**: high
- **Depends On**: None
- **Description**: tabs.ts closeTab 非活动分支补 `pushToShell()`；多页签探针新增「关后台页签 → #tabs .tab 数 == tabs.count」断言（先 activate 其它页签再关）。
- **AC**: FR-2 / AC-2
- **TR**:
  - `rule` TR-2.1：探针新断言通过；反向对照（删掉 pushToShell 行）断言必须 FAIL，再还原。

### Task 3: Electron 设置保存失败托盘销毁不回滚
- **Status**: pending
- **Priority**: high
- **Depends On**: None
- **Description**: applySettings 增 `trayDestroyedThisCall` 追踪；catch 中重建托盘（best-effort + warn），与新建回滚对称。
- **AC**: FR-3 / AC-2
- **TR**:
  - `rule` TR-3.1：结构/纯函数可测部分单测覆盖（若 create/destroy 不易注入，则注释+探针现有「保存后副作用」断言回归）；typecheck 通过。

### Task 4: Electron preload 子框架特权桥收窄
- **Status**: pending
- **Priority**: high
- **Depends On**: None
- **Description**: preload.ts 顶层先算 isTopFrame；非顶层框架直接 return（不 sendSync、不暴露 web2app/web2appExtra、不注册 overlay 监听、不注入）。保留类型导出可编译。
- **AC**: FR-4 / AC-2 / AC-3
- **TR**:
  - `rule` TR-4.1：探针夹具加同源 iframe 断言（iframe 内 `typeof window.web2app === 'undefined'`，通过 postMessage 回报）；反向对照（去掉 return）断言 FAIL 后还原。

### Task 5: Electron public_key 归一化对齐解析器
- **Status**: pending
- **Priority**: high
- **Depends On**: None
- **Description**: config.ts 新增 `normalizePublicKey`（`\\n`→`\n`、去 `\r`、trim；空串保持空）；loadConfig 使用。
- **AC**: FR-5 / AC-3
- **TR**:
  - `rule` TR-5.1：config.test.ts 新增用例（PEM 带字面量 \n / 真换行 / 裸 base64 / 空串）。

### Task 6: Electron sanitizeDirName 对齐 Rust
- **Status**: pending
- **Priority**: high
- **Depends On**: None
- **Description**: 补「连续空白压缩为一个、去首尾句点」两条规则，保持非法字符替换+空回落。
- **AC**: FR-6 / AC-3
- **TR**:
  - `rule` TR-6.1：config.test.ts 新增 `"My  App"`→`My App`、`"MyApp."`→`MyApp`、`" . . "`→回落 Web2App 等用例。

### Task 7: Tauri 多页签标题回传改 event 通路
- **Status**: pending
- **Priority**: high
- **Depends On**: None
- **Description**: ①新增 capabilities 覆盖 `"tab-*"`（remote http/https、仅 `core:event:allow-emit`）；②tab_title.js 改发 `web2app:tab-title` 事件（仿 titlebar.js openSettings 的 __TAURI__/__TAURI_INTERNALS__ 双通路）；③lib.rs setup 监听该事件，按 payload{label,title} 调 tabs::set_tab_title（window_label 校验 label 归属，防伪造）；④删除远程不可达的 `set_tab_title` 应用命令注册，tabs.rs 保留 pub(crate) 函数；⑤更新 ipc/事件静态契约守卫（tab_title.js ↔ 监听事件名逐字一致），更新 shell.html 注释。
- **AC**: FR-7 / AC-2
- **TR**:
  - `rule` TR-7.1：`cargo test` 新增/更新守卫通过；反向对照（事件名改一字 / capability 摘权限）守卫 FAIL 后还原；clippy 全绿。

### Task 8: Tauri 多页签窗口补外部代理 browser args
- **Status**: pending
- **Priority**: high
- **Depends On**: None
- **Description**: tabs.rs create_host_window 与 tab WebviewWindowBuilder 均按 config::build_proxy_args() 追加 `.additional_browser_args()`（cfg(windows)）；注释说明 WebView2 同 user-data-folder 共享 browser 进程、首个创建的环境（host）必须带参。
- **AC**: FR-8 / AC-2
- **TR**:
  - `rule` TR-8.1：静态守卫（grep/测试钉住 host 与 tab builder 两处都调用 additional_browser_args）；clippy/全量 test 绿。

### Task 9: Tauri 直连模式凭据主机白名单
- **Status**: pending
- **Priority**: high
- **Depends On**: None
- **Description**: 抽纯函数 `is_upstream_host(upstream: &Url, request: &Url) -> bool`（scheme+host+默认端口归一化）并单测；attach_webview_hooks 增加 upstream 参数（Navigation.upstream_host 改为携带端口或直接传 Url）；WebResourceRequested 仅对上游置 Authorization；BasicAuthenticationRequested 校验 args.Uri()。tabs resolve_popup_url_for 同源判定同步纳入端口。
- **AC**: FR-9 / AC-3 / AC-5
- **TR**:
  - `rule` TR-9.1：纯函数单测（异主机拒、同主机默认端口 443 与 :443 等价、非默认端口区分、子域名不混同）；cargo test/clippy 绿。

### Task 10: WEB2APP_DEBUG 真值集合两壳对齐
- **Status**: pending
- **Priority**: high
- **Depends On**: None
- **Description**: Electron 侧新增可单测的 `debugFlagTruthy(v)`（接受 1/true/yes，trim、小写，对齐 Rust flag_truthy）；index.ts DEBUG 判定改用它。
- **AC**: FR-11 / AC-3
- **TR**:
  - `rule` TR-10.1：单测 1/true/yes/TRUE/带空格 通过，0/false/no/空 拒绝；docs §3.5 表述与两侧实现一致。

## P2（medium/low）

### Task 11: Rust bootstrap scheme 校验
- **Status**: pending
- **Priority**: medium
- **Depends On**: None
- **Description**: prepare_navigation Url::parse 后拒绝非 http/https，文案对齐 sidecar；单测。
- **AC**: FR-10 / AC-2
- **TR**: `rule` TR-11.1：单测覆盖 ftp:// 拒绝、http/https 放行。

### Task 12: Tauri tab 首屏失败态可见 + 重试
- **Status**: pending
- **Priority**: medium
- **Depends On**: 7
- **Description**: ①dist/index.html 去掉可见占位文案（空 body，保留骨架/title），让 overlay 不再 ~240ms 误判完成；②loading.js overlay 重试：Tauri 环境改发 `web2app:retry-bootstrap` 事件（Electron/其它仍 reload）；③Rust 监听该事件 → bootstrap_for_label(app, event.window_label)；tab 快速失败时 fail() 在 loading 态内可达。
- **AC**: FR-10 / AC-5
- **TR**:
  - `rule` TR-12.1：loading.js 形状守卫测试更新（含事件名）；cargo test 绿；Electron 探针回归（loading/遮罩断言不破）。

### Task 13: 消除多 tab 并发 bootstrap 互杀竞态
- **Status**: pending
- **Priority**: medium
- **Depends On**: None
- **Description**: navigate_tab 缓存未命中时不再直接再起一轮 bootstrap：后台轮询等待缓存（上限 ~30s），就绪后导航；超时（首轮 bootstrap 失败）才回落 bootstrap_for_label。
- **AC**: FR-10
- **TR**: `rule` TR-13.1：cargo test/clippy 绿（竞态路径注释说明；纯等待函数尽量可测）。

### Task 14: close_tab 邻接选择抽纯函数 + 单测
- **Status**: pending
- **Priority**: medium
- **Depends On**: None
- **Description**: 将「labels+removed_idx+active_idx → 新 active_idx」抽成纯函数并穷举单测（对齐 tab-order.ts 语义）。
- **AC**: FR-10 / AC-3
- **TR**: `rule` TR-14.1：关中间/末尾/激活/非激活组合用例。

### Task 15: 代理响应路径剥离 Connection/Upgrade 逐跳头
- **Status**: pending
- **Priority**: medium
- **Depends On**: None
- **Description**: 响应头循环用包含 connection/upgrade 的判定（WS 101 不走此路径），单测。
- **AC**: FR-10
- **TR**: `rule` TR-15.1：单测构造带 Connection: keep-alive 的响应断言被剥离；现有代理 e2e 全绿。

### Task 16: Rust 小健壮性与冗余清理
- **Status**: pending
- **Priority**: low
- **Depends On**: None
- **Description**: ①tray.rs default_window_icon expect 改 Result 链（设置切换托盘不 panic）；②proxy e2e 首个 reqwest 客户端改 no_proxy()；③dbg_status 冗余 `|| is_redirection()` 删除；④tabs.rs 模块 cfg(windows) 内 3 处冗余属性删除；⑤store.rs 无调用方的 pub save 降 cfg(test)/删除并加注释；⑥相关过期注释订正。
- **AC**: FR-10
- **TR**: `rule` TR-16.1：cargo fmt/clippy/test 全绿。

### Task 17: Electron 侧 P2 健壮性/清理
- **Status**: pending
- **Priority**: low
- **Depends On**: None
- **Description**: ①settings-window.ts 四个 handler 增 sender==settingsWin 校验；②spawnTab 在途防护（MAX_TABS 检查-创建串行化）；③stopSidecar 幂等（重复 stop 不叠加 timer/监听）；握手成功后 sidecar 异常退出记一条 startup 日志；④过期注释订正（probe.ts 头「12 条契约」、「0–15」→实际分段、index.ts「49KB」→实测约 80KB、tabs.ts backgroundThrottling 不存在机制的注释、OverlayCommand slow 预留标注）。
- **AC**: FR-10
- **TR**: `rule` TR-17.1：typecheck/unit/探针全绿；探针新增断言通过。

### Task 18: 文档与 README 专项
- **Status**: pending
- **Priority**: medium
- **Depends On**: 1-17
- **Description**: ①docs/packaging-config.md：release 触发措辞（Publish Release，非 tag）、§7 编号顺序、§3.5 debug 真值表述；②docs/electron-size-reduction.md：§5.3 注入资源位置改 asar、Cargo.toml 行号；③docs/project-analysis.md：resolve_secret 告警非 fail-fast、tungstenite_msg_to_axum 符号名；④electron-analysis 量级/行号标注；electron-tabs-design extraResources 历史标注；⑤根 README 增 v1.37 行 + §四登记新审计文档；两壳 README 同步本轮行为/计数；⑥新增 docs/audit-2026-09-27.md。
- **AC**: AC-4
- **TR**: `rule` TR-18.1：每条订正附真源依据；审计文档列出三类清单、方案、闭环证据与遗留项。

### Task 19: 独立审查与复测
- **Status**: pending
- **Priority**: high
- **Depends On**: 1-18
- **Description**: 全新上下文只读独立审查全部 diff 与 AC 证据；失败项回灌本文件再闭环。
- **AC**: AC-1~AC-5
- **TR**: `rule` TR-19.1：review.md 记录全部 checkpoint 为 pass；`rubric` TR-19.2：AC-5 评分 >=4。
