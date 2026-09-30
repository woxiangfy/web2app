# Independent Review - 第五轮全流程审计（2026-09-27）

- Reviewer：独立子代理（全新只读上下文，未参与实现）
- 输入：`spec.md` / `tasks.md` / 工作区代码（框架行为疑点直读 cargo 缓存
  tauri 2.11.5 / tauri-utils 2.9.3 / urlpattern 0.3.0 源码取证）
- 门禁口径：只读静态核查 + 独立重新计数，未重跑构建（实现侧的实跑证据见 tasks/audit）。

## R1（2026-09-27）— **pass**（1 Minor + 4 Advisory，无 Critical/Major）

### CP 复核（reviewer 实测）

| CP | 结论 | 关键证据 |
|---|---|---|
| AC-1 门禁 | pass（静态） | 独立计数与声称一致：Rust 13/7/29/95=**144**；Electron config 19 + page-behavior 14 + tabs 6 = **39**（修复后 40）；探针 runProfer 121 个 p.check 扣 1 互斥 catch + 18 near = **138**，runTabsProbe **33** |
| AC-2 P1 回归 | pass | P7c 为真端到端断言（waitUntil 轮询壳 DOM）；静态守卫含拆针防自指；白名单含负向断言；反向对照记录可信 |
| AC-3 契约不漂移 | pass（带 Advisory） | public_key / debug 真值逐字等价；目录名常用输入等价（NEL/BOM 微差转 A2）；凭据白名单对齐（scheme 维度差转 A3） |
| AC-4 文档 | pass（带 Advisory） | 计数/登记/v1.37 均落实；default.json 端口注释失实转 A1 |
| AC-5 最小性/质量 | 4/5 | 改动聚焦、注释充分、错误可见性原则贯彻 |

### 22 项重点核查

核实**正确**：1（自动重试一次性）、2（后台页签推送）、3（托盘对称回滚）、
4（preload 顶层收窄，类型导出仍在）、5（public_key 与 Python 逐字等价）、
6（目录名清洗常用输入等价）、7（debug 真值等价）、8（标题事件全链路：URLPattern
源码证实 `tab-*` + 带端口 remote 模式对代理随机端口与直连默认端口都命中；token
label 双校验；host 窗口无 emit 权限；textContent 无 XSS）、9（代理参数两处均在
build 前）、10（凭据白名单：origin_of 默认端口/IPv6/非 http/None 全覆盖；PWSTR
CoTaskMemFree 配对；Navigation 构造点无遗漏；弹窗同源新逻辑无回归）、
11（loading.js 三分支 + 模式标记注册顺序在 OVERLAY_SCRIPT 前）、
12（/retry 按 webview_label 引导）、13（缓存等待线程的窗口销毁/回落时序）、
14（catch_unwind 覆盖 &str/String panic）、15（101 经 on_upgrade 提前 return
不经响应逐跳循环；Content-Length 保留）、16（InvalidIcon(io::Error) 变体真实）、
17（save_at 活代码未误删）、19（settingsWin 调用时读现值）、
20（探针断言写法稳定、P7c 用 waitUntil 非固定 sleep）、21（cfg 纪律基本正确）、
22（计数全部属实）。

### Findings

| 级别 | 编号 | 内容 | R2 处置 |
|---|---|---|---|
| Minor | M1 | sidecar 正常 `stop()` 后 exit 也打「握手成功后异常退出」假警报（每次正常关应用一条） | **已修**：exit 分支查 `stoppingChildren`，主动停止打「已随宿主停止退出」，探针实测该日志出现且 138/138 绿 |
| Advisory | A1 | default.json 注释称「URLPattern 不写端口=只匹配默认端口」，与框架源码相反（缺省端口段按通配处理）；显式 `:*` 本身正确 | **已订正**注释（保留显式 `:*`，说明意图不依赖框架默认） |
| Advisory | A2 | NEL(U+0085)/BOM(U+FEFF) 上 Rust White_Space 与 JS `\s` 微差 | **已修**：TS 改 `new RegExp('(?:[^\\S\\uFEFF]|\\u0085)+','g')` 精确对齐，新增 1 条单测（39→**40**） |
| Advisory | A3 | Electron login 白名单只比 host（无 scheme 维度） | **暂不改（附理由）**：Electron AuthInfo 不带请求 URL/scheme，无法从事件精确判定；mixed-content 拦截兜底；属本轮之前既有逻辑，列入遗留 |
| Advisory | A4 | tabs.rs 仍有 1 处冗余 `#[cfg(windows)]`（open_new_tab_from_request） | **已删** |

## R2（2026-09-27）— 修复后复验

| 检查 | 命令/方式 | 结果 |
|---|---|---|
| Electron typecheck | `pnpm run typecheck` | exit 0 |
| Electron 单测 | `pnpm run unit` | **40/40**（含新 NEL/BOM 用例） |
| 单窗口探针（覆盖 sidecar 正常退出路径） | `pnpm run probe` | **138/138**；日志实测为「已随宿主停止退出（code=0）」，无假警报 |
| Rust fmt / clippy / test | fmt --check 后 `cargo fmt`；clippy `-D warnings`；`cargo test --all` | fmt 0（已格式化）、clippy exit 0、**144/144 通过**（6 ignored） |
| 回归面 | M1 只动日志分支、A1 只动 JSON 注释、A4 只删属性、A2 仅 TS 纯函数+测试 | 无行为面回归；多页签探针未受影响（33/33 的代码路径本轮未再触碰） |

**R2 结论：pass。** AC-1..AC-5 满足；M1 已闭环并取到运行期证据，A1/A2/A4 已闭环，
A3 经评估保留并在审计文档遗留项中记录（实现者与审查者一致认为现有信息无法安全收紧）。
