/**
 * 兼容契约探针（`WEB2APP_PROBE=1`）—— 公共入口（桶文件）。
 *
 * 存在的理由：本壳的价值全在「与 Tauri 版行为一致」的一整套兼容契约上，而它们全都是
 * **错了也静默**的逻辑（顶部栏没挂上、38px 没让位、按钮没反应、新窗口开成裸窗口、
 * 子框架误拿特权桥、关后台页签不刷新……）。
 * Tauri 侧当年只能靠「截图 + 人眼比对」，还因为 WebView2 走 DWM 合成而拿不到内容
 * （GDI CopyFromScreen 抓到的是下层窗口、PrintWindow 全黑）——这边不必受这个罪：
 * Electron 可以 `executeJavaScript` 读 DOM、可以 `capturePage()` 截图。
 *
 * 因此把契约写成可执行断言，`pnpm run probe` 一把跑完，退出码即结论。
 * 这是双壳长期并行时「不跑偏」的唯一保障（见 docs/electron-plan.md §6）。
 *
 * 模块地图（2026-10-01 由 1852 行单文件拆为四块，本文件只保留公共导出面，
 * index.ts 的导入路径与符号不变）：
 *   · probe-server.ts  探针夹具站点：内联夹具页、HTTP 401 上游、手写最小 WebSocket，
 *                      以及上游侧观测点 seen（断言「代理注入了什么」的唯一取证处）。
 *   · probe-infra.ts   断言基建：DOM 快照脚本 SNAPSHOT_JS、Probe 记录器、
 *                      sleep/waitUntil、配色与进程工具、Context/M3ProbeHooks 契约类型。
 *   · probe-single.ts  runProbe：单窗口模式断言组（节 0–17，收尾动作是真的关闭窗口）。
 *   · probe-tabs.ts    runTabsProbe：多页签模式断言组（P1–P12）。
 */
export { AUTH_HEADER, startProbeServer, type ProbeServer } from './probe-server'
export {
  type M3ProbeHooks,
  type ProbeContext,
} from './probe-infra'
export { runProbe } from './probe-single'
export { runTabsProbe, type TabsProbeContext } from './probe-tabs'
