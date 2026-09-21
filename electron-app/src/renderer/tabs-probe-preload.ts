/**
 * 多页签探针专用 preload：给**内容视图**暴露一个 `ipcRenderer.invoke` 直通，
 * 供 `runTabsProbe` 验证 P11（特权命令 sender 校验）。
 *
 * ⚠️⚠️ 探针专用，**绝不用于生产**：生产内容视图（TabManager 的 spawnTab）刻意
 *   不挂 preload，这样远程页面连 `ipcRenderer` 都碰不到，特权命令（close_host 等）
 *   从内容页天然发不出去。P11 要验证的是「即便内容页获得了 IPC 通道，`assertShell`
 *   仍会用 sender.id 拒绝它」这层纵深防御 —— 所以才需要一个探针 preload 把通道
 *   硬塞给内容页，模拟「未来有人误给内容视图加了 preload」的最坏情形。
 *
 * ⚠️ 本文件会被 tsc 编进 out/renderer/，但只在探针模式被引用；生产内容视图
 *   不加载它，打包产物里多出这一个文件也无害（不激活）。
 */
import { contextBridge, ipcRenderer } from 'electron'

contextBridge.exposeInMainWorld('__web2appProbeInvoke', {
  /** 直通 ipcRenderer.invoke，探针用来从内容视图发起特权命令（验证被拒）。 */
  invoke: (channel: string, ...args: unknown[]): Promise<unknown> =>
    ipcRenderer.invoke(channel, ...args),
})
