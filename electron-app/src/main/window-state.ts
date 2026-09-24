/**
 * 窗口几何（`settings.json` 的 `window` 段）—— 与 Tauri 版（`webview-app/src/window_state.rs`）共用契约。
 *
 * 段形态（**物理像素**，字段与旧版逐字一致）：
 *   "window": { "x": 100, "y": 200, "width": 800, "height": 600, "maximized": false }
 *
 * ⚠️⚠️ **2026-09-23 起几何与设置同住一个 `settings.json`**（此前是独立的 `window-state.json`，
 *     用户要求「只需要一个 JSON」）。旧的 `window-state.json` **不再读也不再写**，升级后
 *     窗口会回一次默认尺寸/位置。类型与读写入口都在 settings.ts。
 *
 * ⚠️⚠️ 所以写几何**必须走读-改-写**（`readRawForUpdate` → 改 `window` → `writeDocument`）：
 *     拿内存里的整份文档整体覆盖，会把用户刚在设置窗里保存的三个开关抹掉。
 *     反过来 `settings.ts::saveSettings` 同理保留 `window` 段 —— 两侧都靠这个约定活着。
 *
 * ⚠️⚠️ 物理像素不是随便选的：Tauri 版用 `outer_position()` / `outer_size()`，
 *    它们返回**物理像素**。若这边存 DIP，在 125%/150% 缩放的机器上恢复出来的窗口
 *    会比用户上次的位置缩水一截，而且和旧版互相读取会整体错位。
 *    Electron 的 `getBounds()` / `setBounds()` 走的是 DIP，所以要显式换算：
 *    存盘 `dipToScreenRect`、恢复 `screenToDipRect`。
 */
import { screen, type BrowserWindow } from 'electron'

import {
  loadDocument,
  parseSettings,
  readRawForUpdate,
  writeDocument,
  type StoreLocation,
} from './settings'

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

export function loadWindowState(loc: StoreLocation): { state: WindowState | null; source: string | null } {
  // 文档层已经把「文件不存在 / 顶层 JSON 坏 / 命中哪个候选」处理掉了，这里只解释 `window` 段。
  // ⚠️ 段坏了（比如手改成字符串）只是「这次没有几何可恢复」，不会连累三个开关 —— 见 parseWindowState。
  const doc = loadDocument(loc)
  return { state: parseWindowState(doc.window), source: doc.source }
}

/** 写几何（失败只在控制台留痕：为了记个窗口尺寸让用户看到报错不值得）。 */
export function saveWindowState(loc: StoreLocation, st: WindowState): void {
  try {
    // ⚠️⚠️ 读-改-写，只替换 `window` 段：整体覆盖会把用户刚保存的设置抹掉。
    // ⚠️ 「读」走 readRawForUpdate（候选顺序），不是写入位置 —— 落点刚搬到数据目录时
    //    写入位置还没有文件，设置还在旧位置，只认写入位置会把它们清零。
    const cur = readRawForUpdate(loc)
    writeDocument(loc, { settings: parseSettings(cur), window: st })
  } catch (e) {
    console.warn(`[web2app] 保存窗口几何失败：${(e as Error).message}`)
  }
}

/** 从窗口抓取当前几何，换成物理像素。窗口已销毁 / 读不到时返回 null。 */
export function captureWindowState(win: BrowserWindow): WindowState | null {
  if (win.isDestroyed()) return null
  try {
    // ⚠️ 用 getBounds()（当前实际边界）而不是 getNormalBounds()：
    //    旧版在最大化时存的也是最大化后的矩形，且因为 `maximized=true` 恢复时根本不看坐标，
    //    两边行为因此完全一致。
    const dip = win.getBounds()
    const phys = screen.dipToScreenRect(win, dip)
    return {
      x: Math.round(phys.x),
      y: Math.round(phys.y),
      width: Math.round(phys.width),
      height: Math.round(phys.height),
      maximized: win.isMaximized(),
    }
  } catch (e) {
    console.warn(`[web2app] 读取窗口几何失败：${(e as Error).message}`)
    return null
  }
}

/** 启动时如何摆放主窗口。 */
export interface RestorePlan {
  /** 要恢复的 DIP 边界；undefined = 用创建时的默认尺寸 */
  bounds?: { x: number; y: number; width: number; height: number }
  maximize: boolean
  /** 决策原因，写进启动日志（几何没恢复时这是唯一能看出为什么的地方） */
  reason: string
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
export function planRestore(st: WindowState | null): RestorePlan {
  if (!st) return { maximize: false, reason: '无保存状态，使用默认尺寸' }
  if (st.maximized) return { maximize: true, reason: '上次为最大化' }

  if (st.width < MIN_W || st.height < MIN_H) {
    return { maximize: true, reason: `保存的尺寸过小（${st.width}×${st.height}）` }
  }

  const cx = st.x + st.width / 2
  const cy = st.y + st.height / 2
  // 在 DIP 空间判断（display.bounds 就是 DIP）；物理值只是落盘格式。
  const dipCenter = screen.screenToDipPoint({ x: cx, y: cy })
  const onScreen = screen
    .getAllDisplays()
    .some(
      (d) =>
        dipCenter.x >= d.bounds.x &&
        dipCenter.x <= d.bounds.x + d.bounds.width &&
        dipCenter.y >= d.bounds.y &&
        dipCenter.y <= d.bounds.y + d.bounds.height
    )
  if (!onScreen) {
    return { maximize: true, reason: '保存的位置已不在任何显示器上' }
  }

  const dip = screen.screenToDipRect(null, {
    x: st.x,
    y: st.y,
    width: st.width,
    height: st.height,
  })
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
