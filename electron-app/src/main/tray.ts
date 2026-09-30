/**
 * 系统托盘 —— 对齐 Tauri 版（`webview-app/src/tray.rs`）。
 *
 * 行为契约（docs/electron-plan.md §4 第 7 条）：
 *   · 菜单四项，顺序与文案一致：显示窗口 / 设置 / 隐藏到托盘 / 退出；
 *   · **左键单击**托盘图标 = 切换主窗口显隐（不弹菜单）；
 *   · 只有设置里 `tray_enabled=true` 时才创建；关掉时立即销毁。
 *
 * ⚠️ 「左键显隐、右键弹菜单」在 Electron 里是**默认**行为（`setContextMenu` 只绑右键），
 *    不像 Tauri 需要显式写 `show_menu_on_left_click(false)`。但它依赖平台实现，
 *    所以点击断言仍然写进了探针 —— 这属于「错了也静默」的那类逻辑。
 */
import { join } from 'node:path'
import { Menu, Tray, nativeImage, type BrowserWindow } from 'electron'

import { firstExisting } from './first-existing'

export interface TrayActions {
  /** 左键单击：显隐主窗口 */
  toggle: () => void
  onShow: () => void
  onHide: () => void
  onSettings: () => void
  onQuit: () => void
}

/** 托盘图标定位。开发态与打包态各一处，都找不到时返回 null。 */
export function locateTrayIcon(appPath: string, resourcesPath: string): string | null {
  return firstExisting([
    // 打包态：electron-builder 的 extraResources（见 electron-builder.yml）
    join(resourcesPath, 'icons', 'icon.png'),
    // 开发态：直接复用 Tauri 版的图标资源，不复制出第二份。
    // （appPath 是 electron-app/，其下没有 icons 目录，不要再加 appPath/icons 候选 —— 恒不命中）
    join(appPath, '..', 'webview-app', 'icons', 'icon.png'),
  ])
}

let tray: Tray | null = null

/** 当前是否存在托盘图标（探针断言 / 日志用）。 */
export function hasTray(): boolean {
  return tray !== null && !tray.isDestroyed()
}

/**
 * 创建托盘。已存在时先销毁再重建（设置里来回勾选不会堆出多个图标）。
 *
 * 返回是否成功：图标文件缺失时**不抛错**（托盘是可选功能，因为一个图标让应用起不来
 * 是本末倒置），但要在日志里说清楚。
 */
export function createTray(opts: {
  title: string
  iconPath: string | null
  actions: TrayActions
}): boolean {
  destroyTray()

  if (!opts.iconPath) {
    console.warn('[web2app] 未找到托盘图标（icons/icon.png），托盘无法创建')
    return false
  }

  let image = nativeImage.createFromPath(opts.iconPath)
  if (image.isEmpty()) {
    console.warn(`[web2app] 托盘图标无法解码：${opts.iconPath}`)
    return false
  }
  // Windows 的系统托盘按 DPI 取 16/20/24/32px；图标源是 256px，缩一下更清晰。
  image = image.resize({ width: 16, height: 16 })

  // ⚠️ 四项、无分隔线 —— 与 `tray.rs` 的菜单逐项一致（双壳的托盘长得不一样会直接被当成 bug 反馈）
  const menu = Menu.buildFromTemplate([
    { label: '显示窗口', click: () => opts.actions.onShow() },
    { label: '设置', click: () => opts.actions.onSettings() },
    { label: '隐藏到托盘', click: () => opts.actions.onHide() },
    { label: '退出', click: () => opts.actions.onQuit() },
  ])

  try {
    tray = new Tray(image)
  } catch (e) {
    console.warn(`[web2app] 创建托盘失败：${(e as Error).message}`)
    tray = null
    return false
  }

  tray.setToolTip(opts.title)
  tray.setContextMenu(menu)
  tray.on('click', () => opts.actions.toggle())
  return true
}

/** 销毁托盘（幂等）。 */
export function destroyTray(): void {
  const t = tray
  tray = null
  if (!t || t.isDestroyed()) return
  try {
    t.destroy()
  } catch {
    /* 已经没了 */
  }
}

/**
 * 切换主窗口显隐（托盘左键 / 唤醒快捷键共用同一入口，与 `tray.rs::toggle_window` 一致）。
 *
 * 语义：**可见且非最小化 → 隐藏；其余（隐藏中 / 最小化）→ 恢复并聚焦**。
 * ⚠️ 少了「非最小化」这个判断，最小化状态下按快捷键会先隐藏、再按才显示，
 *    用户会觉得「第一次按没反应」。
 */
export function toggleMainWindow(win: BrowserWindow | null): void {
  if (!win || win.isDestroyed()) return
  const visible = win.isVisible()
  const minimized = win.isMinimized()
  if (visible && !minimized) {
    win.hide()
  } else {
    if (minimized) win.restore()
    win.show()
    win.focus()
  }
}

/** 显示并聚焦主窗口（托盘菜单「显示窗口」/ 第二实例）。 */
export function showMainWindow(win: BrowserWindow | null): void {
  if (!win || win.isDestroyed()) return
  if (win.isMinimized()) win.restore()
  win.show()
  win.focus()
}

/** 隐藏主窗口（托盘菜单「隐藏到托盘」）。 */
export function hideMainWindow(win: BrowserWindow | null): void {
  if (!win || win.isDestroyed()) return
  win.hide()
}
