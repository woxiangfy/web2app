/**
 * 壳页面（shell.html）专用 preload：为**原封不动**的 Tauri 标签栏页面提供
 * `window.__TAURI_INTERNALS__.invoke` 白名单垫片。
 *
 * 为什么是垫片而不是改 shell.html（同 settings-preload.ts 的取舍）：shell.html 是双壳共用的
 * 单一真源，它按 Tauri 的约定调用 `invoke("switch_tab")` / `invoke("close_tab")` /
 * `invoke("get_tabs")` / `invoke("minimize_host")` / `invoke("close_host")` /
 * `invoke("history_nav")`。这边把这几条接到 IPC 上，页面一行都不用动。
 *
 * ⚠️ 垫片只暴露 `invoke`，且**只放行白名单里的命令**。多暴露的每一寸都是交给页面的攻击面。
 *   注意：壳页面是窗口自身 webContents，理论上它比远程内容页更可信，但白名单仍然收紧 ——
 *   与主进程侧「特权命令必须校验 sender」双保险（见 index.ts 的 registerTabsIpc）。
 *
 * ⚠️ 与主窗口 preload 是**两个文件**：主窗口那份要注入 titlebar.js/loading.js，
 *   壳页面只画标签栏，注入那套会多出 38px 留白与浮空标题栏。
 */
import { contextBridge, ipcRenderer } from 'electron'

/** 允许壳页面调用的命令（前缀 `tabs:` 是主进程侧命名空间，避免与 tauri:* 设置命令撞车）。 */
const ALLOWED = new Set([
  'get_tabs',
  'switch_tab',
  'close_tab',
  'minimize_host',
  'close_host',
  'history_nav',
  // 「设置」按钮（位置在最小化左侧）—— shell.html 是双壳共用的单一真源，
  // 那边写的是 invoke("open_settings")，这边必须逐字放行，否则按钮点了没反应。
  'open_settings',
  // 首个页签创建失败时，注入到壳页面的失败遮罩用它发起重试（页面本身不调用，
  // 仅主进程 executeJavaScript 注入的错误态用）；主进程侧同样有 assertShell 校验。
  'retry_first_tab',
])

contextBridge.exposeInMainWorld('__TAURI_INTERNALS__', {
  invoke: (cmd: string, args?: Record<string, unknown>): Promise<unknown> => {
    if (typeof cmd !== 'string' || !ALLOWED.has(cmd)) {
      return Promise.reject(new Error(`拒绝调用未授权的命令：${String(cmd)}`))
    }
    return ipcRenderer.invoke('tabs:' + cmd, args ?? {})
  },
})

/** 壳标识：供注入脚本/调试判断（与 settings-preload 保持一致）。 */
contextBridge.exposeInMainWorld('web2appShell', 'electron')
