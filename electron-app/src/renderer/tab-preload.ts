/**
 * 多页签模式下**内容页**专用的最小 preload：只注入 `window.web2appExtra`。
 *
 * 为什么需要它（2026-11-16 实测）：`tabs_enabled: true` 的产物里，内容页是 `WebContentsView`
 * （tabs.ts::spawnTab），它**不挂** `preload.ts` —— 那套会往页面里塞自绘标题栏 / 加载层，
 * 而页签模式的标题栏属于壳页面 `shell.html`。结果 `window.web2appExtra` 在页签模式下
 * **恒为 undefined**（单窗口模式正常），与 `docs/packaging-config.md` 承诺的
 * 「全平台、前端读 `window.web2appExtra`」不符，也与 Tauri 页签
 * （`webview-app/src/tabs/window.rs` 的 `initialization_script`）不一致。
 *
 * 契约（与 `preload.ts` 的 extra 部分逐条对齐，改动必须两边一起看）：
 *   · 只暴露**数据**，不暴露任何窗口桥 —— 页签内容页确实没有 `window.web2app`，这是安全基线；
 *   · `extra` 为空对象时**不定义**该全局（避免页面误判「配置里填了 extra」）；
 *   · 同步取（`sendSync`）：preload 执行于「document 已创建、页面自身脚本未跑」，
 *     与 Tauri 的 document-created 注入同一切点，页面首屏即可读。
 *
 * ⚠️ 硬约束（electron-app/AGENTS.md #17）：**不得相对 require**。本文件只 `import 'electron'`。
 *    Electron 45 起 sandbox preload 的 `require` 只放行 `electron`。
 */
import { contextBridge, ipcRenderer } from 'electron'

// ⚠️ 子框架：preload 默认不作用于子框架（需 nodeIntegrationInSubFrames=true），这里只做纵深防御，
//    与 `preload.ts` 顶部注释第 2 条同一道闸 —— 一旦将来打开该开关，跨源 iframe 也拿不到配置。
if (window.self === window.top) {
  let extra: unknown = null
  try {
    // 主进程侧见 app-ipc.ts::registerTabExtraIpc（只做顶层框架闸：本通道不涉及任何窗口能力）。
    extra = ipcRenderer.sendSync('w2a:tab-extra')
  } catch {
    // 主进程未注册 / 页签已销毁等：静默降级为空，绝不因为「配置读不到」把页面搞崩。
    extra = null
  }
  const obj = extra as Record<string, unknown> | null
  if (obj && typeof obj === 'object' && Object.keys(obj).length > 0) {
    contextBridge.exposeInMainWorld('web2appExtra', obj)
  }
}
