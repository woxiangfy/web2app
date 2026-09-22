/**
 * 设置窗专用 preload：为 **原封不动** 的 Tauri 设置页提供 `window.__TAURI_INTERNALS__` 垫片。
 *
 * 为什么是垫片而不是改 HTML（见 settings-window.ts 的说明）：设置页是双壳共用的单一真源，
 * 它按 Tauri 的约定调用 `invoke("get_settings")` / `invoke("save_settings", { newSettings })` /
 * `invoke("close_settings_window")` / `invoke("clear_web_data")`。这边只要把这四条命令接到 IPC 上，
 * 页面一行都不用动 —— 也就不会出现「Tauri 版设置页修了、Electron 版忘了改」这种分叉。
 *
 * ⚠️ 垫片只暴露 `invoke` 一个方法，且**只放行白名单里的命令**。
 *    照搬 `__TAURI_INTERNALS__` 的全部表面（`transformCallback` 等）没有必要，
 *    多暴露的每一寸都是交给页面的攻击面。
 *
 * ⚠️ 与主窗口的 preload 是**两个文件**：主窗口那份要注入标题栏/加载层脚本，
 *    而设置窗是带系统标题栏的普通窗口，注入了会多出 38px 留白与一条浮空标题栏。
 */
import { contextBridge, ipcRenderer } from 'electron'

/** 允许设置页调用的命令（前缀 `tauri:` 是主进程侧的命名空间） */
const ALLOWED = new Set([
  'get_settings',
  'save_settings',
  'close_settings_window',
  'clear_web_data',
])

contextBridge.exposeInMainWorld('__TAURI_INTERNALS__', {
  invoke: (cmd: string, args?: Record<string, unknown>): Promise<unknown> => {
    if (typeof cmd !== 'string' || !ALLOWED.has(cmd)) {
      return Promise.reject(new Error(`拒绝调用未授权的命令：${String(cmd)}`))
    }
    return ipcRenderer.invoke('tauri:' + cmd, args ?? {})
  },
})

/** 壳标识：与主窗口保持一致，便于将来设置页做渐进增强时判断。 */
contextBridge.exposeInMainWorld('web2appShell', 'electron')
