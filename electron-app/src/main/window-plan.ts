/**
 * 窗口几何的**纯判定**部分（无 Electron 依赖，可被 node:test 直接覆盖）。
 *
 * `window-state.ts` 只保留「读盘 / 抓取 / 换算」这些真的要用 `screen` 的适配层，
 * 判定本身搬到这里（2026-11-07 架构审查 M-5）：此前 `window-state.ts` 顶层
 * `import { screen } from 'electron'`，`parseWindowState` / `planRestore` 这些纯逻辑
 * 在无 Electron 的单测里根本 import 不进来。
 *
 * 契约与 `webview-app/src/window_state.rs::apply()` 一一对应，别单独改一侧。
 */

/** 物理像素的窗口几何快照。 */
export interface WindowState {
  x: number
  y: number
  width: number
  height: number
  /** 旧版文件可能没有这个字段，缺省按 false（窗口不会被误判成最大化） */
  maximized: boolean
}

/**
 * 与 Tauri 版一致的「几何是否还够用」阈值（物理像素）。
 * 小于它就丢弃几何直接最大化 —— 尺寸过小的窗口（比如上次被压到只剩一条）
 * 恢复出来用户根本没法用，还不如铺满。
 */
const MIN_W = 420
const MIN_H = 320

function toNum(v: unknown, dflt: number): number {
  return typeof v === 'number' && Number.isFinite(v) ? v : dflt
}

/** 宽容解析：字段缺失/类型不对按默认处理，绝不让一个坏文件把窗口打不开。 */
export function parseWindowState(v: unknown): WindowState | null {
  if (!v || typeof v !== 'object') return null
  const o = v as Record<string, unknown>
  const st: WindowState = {
    x: toNum(o.x, NaN),
    y: toNum(o.y, NaN),
    width: toNum(o.width, NaN),
    height: toNum(o.height, NaN),
    maximized: o.maximized === true,
  }
  if (!Number.isFinite(st.x) || !Number.isFinite(st.y)) return null
  if (!Number.isFinite(st.width) || !Number.isFinite(st.height)) return null
  return st
}

/** 启动时如何摆放主窗口。 */
export interface RestorePlan {
  /** 要恢复的 DIP 边界；undefined = 用创建时的默认尺寸 */
  bounds?: { x: number; y: number; width: number; height: number }
  maximize: boolean
  /** 决策原因，写进启动日志（几何没恢复时这是唯一能看出为什么的地方） */
  reason: string
}

/** 判定所需的「显示器环境」——由适配层把 `screen` 模块换算好后注进来。 */
export interface RestoreContext {
  /** 物理点 → DIP 点（`screen.screenToDipPoint`） */
  toDipPoint: (p: { x: number; y: number }) => { x: number; y: number }
  /** 物理矩形 → DIP 矩形（`screen.screenToDipRect(null, r)`） */
  toDipRect: (r: { x: number; y: number; width: number; height: number }) => {
    x: number
    y: number
    width: number
    height: number
  }
  /** 各显示器的 DIP 边界（`display.bounds` 本身就是 DIP） */
  displays: { x: number; y: number; width: number; height: number }[]
}

/**
 * 决定启动时窗口怎么摆。
 *
 * 四种结局，与 `window_state.rs::apply()` 一一对应：
 *   1. 没有保存状态          → 用默认尺寸
 *   2. 上次是最大化          → 直接最大化
 *   3. 几何失效（中心点不在任何显示器上 / 尺寸过小）→ 最大化
 *      ⚠️ 显示器被拔掉、分辨率或 DPI 变化后，窗口会被留在屏幕外，用户以为程序没启动 ——
 *         这是几何持久化最经典的坑，必须挡掉。
 *   4. 其余                  → 原位恢复
 */
export function planRestore(st: WindowState | null, ctx: RestoreContext): RestorePlan {
  if (!st) return { maximize: false, reason: '无保存状态，使用默认尺寸' }
  if (st.maximized) return { maximize: true, reason: '上次为最大化' }

  if (st.width < MIN_W || st.height < MIN_H) {
    return { maximize: true, reason: `保存的尺寸过小（${st.width}×${st.height}）` }
  }

  const cx = st.x + st.width / 2
  const cy = st.y + st.height / 2
  // 在 DIP 空间判断（display.bounds 就是 DIP）；物理值只是落盘格式。
  const dipCenter = ctx.toDipPoint({ x: cx, y: cy })
  const onScreen = ctx.displays.some(
    (d) =>
      dipCenter.x >= d.x &&
      dipCenter.x <= d.x + d.width &&
      dipCenter.y >= d.y &&
      dipCenter.y <= d.y + d.height
  )
  if (!onScreen) {
    return { maximize: true, reason: '保存的位置已不在任何显示器上' }
  }

  const dip = ctx.toDipRect({ x: st.x, y: st.y, width: st.width, height: st.height })
  return {
    bounds: {
      x: Math.round(dip.x),
      y: Math.round(dip.y),
      width: Math.round(dip.width),
      height: Math.round(dip.height),
    },
    maximize: false,
    reason: `恢复几何 ${st.x},${st.y} ${st.width}×${st.height}（物理）`,
  }
}
