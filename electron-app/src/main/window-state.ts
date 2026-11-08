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
 * ⚠️⚠️ 所以写几何**必须走读-改-写**（`updateDocument` 统一收敛，只替换 `window` 段）：
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
  updateDocument,
  type StoreLocation,
} from './settings'
import {
  parseWindowState,
  planRestore as planRestorePure,
  type RestorePlan,
  type WindowState,
} from './window-plan'

// ⚠️ 纯判定（`parseWindowState` / `planRestore` 的四分支）已搬到 `window-plan.ts`，
//    本文件只保留真的要用 `screen` 的适配（读盘 / 抓取 / 换算）。见 2026-11-07 架构审查 M-5。
export { parseWindowState, type RestorePlan, type WindowState }

export function loadWindowState(loc: StoreLocation): { state: WindowState | null; source: string | null } {
  // 文档层已经把「文件不存在 / 顶层 JSON 坏 / 命中哪个候选」处理掉了，这里只解释 `window` 段。
  // ⚠️ 段坏了（比如手改成字符串）只是「这次没有几何可恢复」，不会连累三个开关 —— 见 parseWindowState。
  const doc = loadDocument(loc)
  return { state: parseWindowState(doc.window), source: doc.source }
}

/** 写几何（失败只在控制台留痕：为了记个窗口尺寸让用户看到报错不值得）。 */
export function saveWindowState(loc: StoreLocation, st: WindowState): void {
  try {
    // ⚠️⚠️ 读-改-写（updateDocument 统一收敛），只替换 `window` 段：整体覆盖会把用户
    // 刚保存的设置抹掉。⚠️ 「读」走候选顺序（readRawForUpdate），不是写入位置 ——
    // 落点刚搬到数据目录时写入位置还没有文件，设置还在旧位置，只认写入位置会把它们清零。
    updateDocument(loc, { window: st })
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

/**
 * 决定启动时窗口怎么摆（`screen` 适配层；纯判定在 `window-plan.ts::planRestore`）。
 *
 * 四种结局：
 *   1. 没有保存状态          → 用默认尺寸
 *   2. 上次是最大化          → 直接最大化
 *   3. 几何失效（中心点不在任何显示器上 / 尺寸过小）→ 最大化
 *   4. 其余                  → 原位恢复
 */
export function planRestore(st: WindowState | null): RestorePlan {
  return planRestorePure(st, {
    toDipPoint: (p) => screen.screenToDipPoint(p),
    toDipRect: (r) => screen.screenToDipRect(null, r),
    displays: screen.getAllDisplays().map((d) => ({
      x: d.bounds.x,
      y: d.bounds.y,
      width: d.bounds.width,
      height: d.bounds.height,
    })),
  })
}
