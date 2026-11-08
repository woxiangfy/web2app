/**
 * 主窗口「显隐切换」的纯判据（2026-11-07 架构审查 M-8）。
 *
 * 同一语义此前有三份实现：`tray.ts::toggleMainWindow`（托盘左键 / 唤醒快捷键）、
 * 入口里悬浮球单击的 `toggleMainWindow` 回调、以及设置控制器的回滚分支。
 * 规则只有一条 —— **可见且非最小化 → 隐藏；其余（隐藏中 / 最小化）→ 呈现并聚焦**。
 * 少判「非最小化」会让最小化状态下第一次按键像「没反应」（先隐藏、再按才出来）。
 *
 * 本模块零 Electron 依赖，`pnpm run unit` 直接覆盖四象限。
 * 「呈现」的具体动作由调用方决定（托盘/快捷键走 `show/restore/focus`，
 * 球单击走 `presentMainWindow`，多页签空壳时还会自愈补建首个页签）。
 */
export type ToggleAction = 'hide' | 'present'

export function planToggle(visible: boolean, minimized: boolean): ToggleAction {
  return visible && !minimized ? 'hide' : 'present'
}
