/**
 * 悬浮球专用 preload：为内置本地球页 `floaticon.html`（双壳单一真源）提供 Electron 桥。
 *
 * 球页是**本地内置页**（v1.69 起 `loadFile`，v1.71 起改经 `floaticon://localhost/` 协议加载，
 * 与 Tauri 同 URL；不再是注入远程页面的 floatbar.js），因此不需要同步取脚本 / webFrame 注入那一套，
 * 只暴露白名单方法即可。
 *
 * 安全基线与主 preload（preload.ts）一致：
 *   · 只在【顶层框架】暴露（纵深防御：即便页面结构变化，子框架也拿不到窗口控制）；
 *   · 只暴露 7 个白名单方法（floatClick / floatDown / floatMove / floatCancel /
 *     floatDragEnd / floatPeek / floatHide），绝不交出 ipcRenderer / require / process；
 *   · 主进程对每条 IPC 再做 sender === 悬浮球窗 webContents 的校验（见 float-window.ts）。
 *
 * floaticon.html 的 Electron 分支按方法名做能力探测（`E.floatMove` / `E.floatClick` 均为
 * 函数），这些名字是跨壳契约，改名必须同步改 floaticon.html。
 */
import { contextBridge, ipcRenderer } from 'electron'

// 子框架：到此为止，不注册桥（纵深防御，同 preload.ts）。
if (window.self === window.top) {
  contextBridge.exposeInMainWorld('web2app', {
    isElectron: true as const,
    // 单击球：把【原来的主窗口】调出来；主进程同时清掉未达拖动阈值的按压暂存。
    floatClick: (): Promise<void> => ipcRenderer.invoke('web2app:float-click'),
    floatHide: (): Promise<void> => ipcRenderer.invoke('web2app:float-hide'),
    // pointerdown 即报按下点（屏幕绝对坐标 DIP）：主进程只暂存，第一帧 move 才展开
    // 吸附态并据此计算抓取偏移（不能等首帧 move 才记 —— 那时锚点已偏）。
    floatDown: (sx: number, sy: number): Promise<void> =>
      ipcRenderer.invoke('web2app:float-down', sx, sy),
    // sx/sy 是指针的屏幕绝对坐标 screenX/screenY（CSS 像素 = DIP）。
    // v1.70：不能传相对球窗视口的 clientX 增量 —— 窗口每帧跟着光标移动，增量会被
    // 窗口自身位移抵消（球半速跟手 / 抖动）；主进程按「光标 − 抓取偏移」绝对定位。
    floatMove: (sx: number, sy: number): Promise<void> =>
      ipcRenderer.invoke('web2app:float-move', sx, sy),
    // 系统打断且未拖动：只清按压 / 拖动态，不弹主窗、不做吸附。
    floatCancel: (): Promise<void> => ipcRenderer.invoke('web2app:float-cancel'),
    floatDragEnd: (): Promise<void> => ipcRenderer.invoke('web2app:float-drag-end'),
    floatPeek: (enter: boolean): Promise<void> =>
      ipcRenderer.invoke('web2app:float-peek', enter === true),
  })
}
