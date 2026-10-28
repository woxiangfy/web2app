/**
 * 关闭按钮的「驻留 vs 退出」判据（自 index.ts 的 close 处理器抽出的纯函数）。
 *
 * 两个**驻留理由**并列（对齐 `lib.rs` 的 `handle_main_close`）：
 *   1. 「启用托盘」+「关闭时最小化到托盘」→ 隐藏窗口，应用继续驻留托盘；
 *   2. 「启用悬浮图标」（仅 Windows，球窗只在那时才存在）→ 隐藏窗口，应用继续驻留悬浮球；
 *   两者都不满足 → 正常关闭（最后一个窗口关闭后退出）。
 *
 * ⚠️⚠️ 第 2 条是**修一个真机缺陷**，不是锦上添花（2026-10-11 使用者实测：策略
 *    `tray_enabled:false, minimize_on_close:false, float_enabled:true`）：
 *     关闭窗口 → 主窗被**真销毁**，但球窗还活着 ⇒ ① 球窗挡住了
 *     `window-all-closed`（它只在**所有**窗口关闭时才发）⇒ 进程留成一个既看不见界面、
 *     也退不掉的僵尸；② 点球时 `toggleMainWindow → presentMainWindow → showMainWindow`
 *     开头就 `if (!win || win.isDestroyed()) return` ⇒ **一个动作都不做**，
 *     用户看到的就是「点了悬浮图标不回窗口」。球的产品语义就是「唤回主窗的入口」
 *     （见 floaticon.html 的 aria-label），故「球在 ⇒ 应用驻留」必须成立。
 *
 * ⚠️ `quitting` 标志不在这里（它是「应用正在退出」的进程级事实）：托盘菜单「退出」走
 *    `app.quit()` 时同样会触发窗口 close，置位后调用方直接放行本函数的结论
 *    （悬浮球窗的 close 也认 `isQuitting`，见 float-window.ts）。
 *
 * 判据纯函数化是为了能被 node 单测钉住（Electron 事件栈里的分支没法直接断言）；
 * 真实 close 路径的接线仍在 index.ts。
 */
import type { Settings } from './settings'

export type CloseAction = 'hide' | 'quit'

export interface CloseDecision {
  action: CloseAction
  /** `hide` 时的驻留理由文案（顺序固定「托盘 → 悬浮图标」，`join(' + ')` 即日志原文） */
  reasons: string[]
}

export function closeResidency(settings: Settings, platform: string): CloseDecision {
  const trayResident = settings.tray_enabled && settings.minimize_on_close
  // 与 float-window.ts 建球的前置条件同一个判据（非 Windows 永远不建球 ⇒ 不得驻留）。
  const floatResident = platform === 'win32' && settings.float_enabled
  const reasons: string[] = []
  if (trayResident) reasons.push('托盘')
  if (floatResident) reasons.push('悬浮图标')
  return { action: reasons.length > 0 ? 'hide' : 'quit', reasons }
}
