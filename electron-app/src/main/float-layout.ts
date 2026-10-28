/**
 * 悬浮球几何纯逻辑 —— 与 Tauri 侧 `webview-app/src/floatwin.rs` 的纯函数**逐条对齐**。
 *
 * 本文件刻意**零 Electron 运行时依赖**（不 import 'electron'），以便 `test/` 里的 node:test
 * 直接跑（纯函数单测的既有约定）。所有矩形 / 坐标都在 **DIP（逻辑像素）** 空间：
 * Electron 的 `BrowserWindow.getBounds()/setBounds()` 与 `screen.getDisplayMatching().bounds`
 * 本来就是 DIP，而 floaticon.html 上报的指针屏幕绝对坐标 screenX/screenY 是 CSS 像素
 * （与 DIP 1:1；v1.70 起不再用增量），所以边缘决策直接用逻辑常量即可；只有**落盘 / 恢复**
 * 边界时才在 `float-window.ts` 里做物理像素换算（落盘格式与 Tauri 一致为物理像素）。
 *
 * v1.69 纠正（v1.68 只修了旧「小窗」的三条静默失败）：窗口从「320×460 小窗」改为 56×56
 * 透明悬浮球（v1.71 起单击 = 切换主窗显隐），几何 / 吸附逻辑不变，只是尺寸 / 半隐藏留白改为「半球」。
 */

/** 吸附边：无 / 左 / 右 / 上 / 下屏缘（落盘序列化为 'none' / 'left' / 'right' / 'top' / 'bottom'）。 */
export type FloatEdge = 'none' | 'left' | 'right' | 'top' | 'bottom'

/** 一个 DIP 矩形（窗口或显示器）。 */
export interface Rect {
  x: number
  y: number
  width: number
  height: number
}

/** 悬浮球几何段（`settings.json` 的 `float_window`），与 Tauri `FloatWinGeom` 同形：物理像素。 */
export interface FloatGeom {
  x: number
  y: number
  width: number
  height: number
  edge: FloatEdge
}

/** 球窗固定边长（逻辑像素，与 floatwin.rs::ICON_SIZE_LOGICAL 一致：正方形透明窗）。 */
export const DEFAULT_FLOAT_W = 56
export const DEFAULT_FLOAT_H = 56
/** 半隐藏时留在屏内的宽 / 高（逻辑像素）= 半球：56 的一半（左 / 右沿 x，上 / 下沿 y）。 */
export const PEEK_LOGICAL = 28
/** 距屏幕任一缘多近（逻辑像素）即判定吸附（窗口边与屏幕边间距 ≤ 该值即吸，四缘同阈值）。 */
export const SNAP_LOGICAL = 28
/** 默认摆放 / 纵向钳制相对屏幕边缘的留白（逻辑像素）。 */
export const MARGIN_LOGICAL = 12
/**
 * 拖动中球在光标所在显示器上至少要留在屏内的宽 / 高（DIP）：允许越过屏缘跨到相邻
 * 显示器，但不会整个球被甩出屏幕（与 floatwin.rs::MIN_VISIBLE_LOGICAL 一致）。
 */
export const MIN_VISIBLE_LOGICAL = 16
/**
 * 鼠标离开展开态球后延迟缩回的毫秒数：边缘小幅移动引起的进出事件在这段时间内合并，
 * 到期主进程还会校验真实光标位置（不在缩回后矩形内才真缩回）。与 Tauri COLLAPSE_DELAY_MS 一致。
 */
export const COLLAPSE_DELAY_MS = 300
/** 恢复几何认可的最小物理边长（与 floatwin.rs::MIN_PHYS 一致；球窗 56 逻辑像素）。 */
export const MIN_PHYS = 40

/** 展开态左上角 x（DIP）：贴住吸附边、完整落在屏内。 */
export function expandedX(edge: FloatEdge, mon: Rect, winWidth: number): number {
  if (edge === 'left') return mon.x
  if (edge === 'right') return mon.x + mon.width - winWidth
  return mon.x
}

/** 半隐藏态左上角 x（DIP）：绝大部分移出屏外，只留 peek 宽窄边。 */
export function collapsedX(edge: FloatEdge, mon: Rect, winWidth: number, peek: number = PEEK_LOGICAL): number {
  if (edge === 'left') return mon.x - (winWidth - peek)
  if (edge === 'right') return mon.x + mon.width - peek
  return mon.x
}

/** 展开态左上角 y（DIP）：贴住吸附边、完整落在屏内（与 `expandedX` 同构，y 轴）。 */
export function expandedY(edge: FloatEdge, monY: number, monHeight: number, winHeight: number): number {
  if (edge === 'top') return monY
  if (edge === 'bottom') return monY + monHeight - winHeight
  return monY
}

/** 半隐藏态左上角 y（DIP）：绝大部分移出屏外，只留 peek 高窄边（与 `collapsedX` 同构，y 轴）。 */
export function collapsedY(
  edge: FloatEdge,
  monY: number,
  monHeight: number,
  winHeight: number,
  peek: number = PEEK_LOGICAL
): number {
  if (edge === 'top') return monY - (winHeight - peek)
  if (edge === 'bottom') return monY + monHeight - peek
  return monY
}

/** 纵向把窗口钳在显示器内（DIP），上下不超出（高度超过屏幕时贴顶）。 */
export function clampY(y: number, winHeight: number, mon: Rect): number {
  const lo = mon.y
  const hi = Math.max(lo, mon.y + mon.height - winHeight)
  return Math.min(Math.max(y, lo), hi)
}

/** 横向把窗口钳在显示器内（DIP）（与 `clampY` 同构，x 轴；入参为裸数值便于两轴对称调用）。 */
export function clampX(x: number, winWidth: number, monX: number, monWidth: number): number {
  const lo = monX
  const hi = Math.max(lo, monX + monWidth - winWidth)
  return Math.min(Math.max(x, lo), hi)
}

/** 点是否在矩形内（DIP，半开区间：右 / 下沿不含）。 */
export function pointInRect(p: { x: number; y: number }, r: Rect): boolean {
  return p.x >= r.x && p.x < r.x + r.width && p.y >= r.y && p.y < r.y + r.height
}

/**
 * 拖动中的软钳制（DIP）：窗口可大部分越过显示器边缘（跨到相邻显示器），
 * 但至少留 minVisible 在 `[lo, hi)` 这段显示器范围里，防止球被整体甩丢。
 * 与 floatwin.rs::clamp_visible 同式。
 */
export function clampVisible(v: number, winSize: number, lo: number, hi: number, minVisible: number): number {
  return Math.min(Math.max(v, lo - winSize + minVisible), hi - minVisible)
}

/**
 * 松手后的自由态钳制（DIP）：把窗口完整放回显示器内（吸附态有自己的贴边算法，不走这里）。
 * 返回钳制后的左上角；旧实现松手不钳，球可停在屏外找不回来。
 */
export function clampFreeInside(pos: Rect, mon: Rect): { x: number; y: number } {
  return {
    x: clampX(pos.x, pos.width, mon.x, mon.width),
    y: clampY(pos.y, pos.height, mon),
  }
}

/** 按给定左上角与固定球尺寸构造 Rect（便于点 / 矩形判定）。 */
export function rectAt(x: number, y: number, win: { width: number; height: number }): Rect {
  return { x, y, width: win.width, height: win.height }
}

/**
 * 把边长 `d` 的正方形窗口用**扫描线矩形**逼近成圆 —— `win.setShape` 只接受 `Rectangle[]`
 * （没有椭圆 / 路径入口），所以逐行取一段 1 像素高的矩形拼出一个圆。
 *
 * 为什么需要它：`transparent: true` 只解决「**看得见**」——四角在系统看来仍是窗口的一部分，
 * 会把桌面 / 下层窗口在球四角上的点击吃掉（layered window 的完全透明像素**不会**自动穿透）。
 * `setShape` 的官方语义是「区域外不绘制、**不接收鼠标事件**，事件穿透到后面的窗口」，
 * 正好补上「**点得着**」这一半。视觉仍由 CSS 圆 + transparent 决定，本区域只决定**命中**，
 * 所以阶梯边缘不会被看到。
 *
 * ⚠️ 与 Tauri 壳的关系（v1.80 起两壳同构）：两壳的窗口视觉都走
 * WS_EX_NOREDIRECTIONBITMAP + DirectComposition 真逐像素 alpha（Tauri v1.80 升级
 * Tauri 2.12 后开 `.no_redirection_bitmap(true)`，与 Electron 长期以来的透明窗路径
 * 相同），border-radius 的抗锯齿软边 / box-shadow 原样上屏。命中则两壳各自实现圆形
 * 穿透：Electron 用这里的 `setShape` 扫描线区域，Tauri 用 WM_NCHITTEST 子类化
 * （HTTRANSPARENT）——两者都**只管命中、不裁视觉**，所以都与球严格同尺寸、绝不外扩
 * （Electron 外扩会把四角点击重新吃回来；Tauri 三代外扩 / 黑底方案均已真机证伪，
 * 见 floaticon.html 文件头存档）。
 *
 * 取整规则：按「**像素中心**是否落在圆内」逐行定左右端，使阶梯边缘与真圆的偏差 < 1 像素 ——
 * 既不切掉球面本来能点的像素，也不把四角放回命中区。
 *
 * @returns 覆盖圆内的矩形行（约 `d` 个）；`d <= 0` 返回空数组（调用方应跳过 `setShape`）。
 */
export function circleShapeRects(d: number): Rect[] {
  if (!(d > 0)) return []
  const r = d / 2
  const out: Rect[] = []
  for (let y = 0; y < d; y++) {
    // 该行像素中心的纵向偏移与其平方剩余；剩余 <= 0 说明整行都在圆外（d 为奇数时的极端行）。
    const dy = y + 0.5 - r
    const rem = r * r - dy * dy
    if (rem <= 0) continue
    const half = Math.sqrt(rem)
    // 左右端：像素中心 x+0.5 落在 [r-half, r+half] 内。
    const x0 = Math.max(0, Math.ceil(r - half - 0.5))
    const x1 = Math.min(d, Math.floor(r + half - 0.5) + 1)
    if (x1 > x0) out.push({ x: x0, y, width: x1 - x0, height: 1 })
  }
  return out
}

/**
 * 判定窗口贴近显示器哪条屏缘（DIP，v1.81 扩四缘）：
 * 左 / 右缘要求窗口**纵向**与屏幕有重叠（否则不算「贴」，是别的屏 / 完全在上下方）；
 * 上 / 下缘对称地要求**横向**有重叠。四条边都在阈值内时**就近择一**（|gap| 最小者），
 * 平局按 left → right → top → bottom 顺序取先者 —— 这条规则专治屏幕角上的歧义
 * （贴角的球同时离两条边都近，旧实现按 if 顺序固定偏爱左边，用户往角上拖时吸附边
 * 反直觉）。
 */
export function decideEdge(win: Rect, mon: Rect, snap: number = SNAP_LOGICAL): FloatEdge {
  if (win.width <= 0 || win.height <= 0) return 'none'
  const verticalOverlap = win.y < mon.y + mon.height && win.y + win.height > mon.y
  const horizontalOverlap = win.x < mon.x + mon.width && win.x + win.width > mon.x
  // 候选按 left → right → top → bottom 顺序收集（平局取先者的实现基础）。
  const candidates: Array<{ edge: FloatEdge; gap: number }> = []
  if (verticalOverlap) {
    const leftGap = win.x - mon.x
    const rightGap = mon.x + mon.width - (win.x + win.width)
    if (leftGap >= -snap && leftGap <= snap) candidates.push({ edge: 'left', gap: leftGap })
    if (rightGap >= -snap && rightGap <= snap) candidates.push({ edge: 'right', gap: rightGap })
  }
  if (horizontalOverlap) {
    const topGap = win.y - mon.y
    const bottomGap = mon.y + mon.height - (win.y + win.height)
    if (topGap >= -snap && topGap <= snap) candidates.push({ edge: 'top', gap: topGap })
    if (bottomGap >= -snap && bottomGap <= snap) candidates.push({ edge: 'bottom', gap: bottomGap })
  }
  if (candidates.length === 0) return 'none'
  let best = candidates[0]
  for (const c of candidates.slice(1)) {
    // 严格小于才替换 ⇒ |gap| 相同时保留顺序靠前者（left → right → top → bottom）。
    if (Math.abs(c.gap) < Math.abs(best.gap)) best = c
  }
  return best.edge
}

/** 松手（拖动结束）的落点决策产物。 */
export interface DropPlan {
  /** 吸附到的屏缘；'none' = 自由态。 */
  edge: FloatEdge
  /** 落点左上角 x（DIP）：吸附态 = 展开贴边坐标；自由态 = 钳回显示器内的坐标。 */
  x: number
  /** 落点左上角 y（DIP）：纵向钳回显示器内。 */
  y: number
}

/**
 * 松手（拖动结束）落点决策 —— 2026-10-24 批次⑤ P2-4 自 `float-window.ts::onDragEnd`
 * 抽出的决策编排（`decideEdge` → 吸附贴边 / 自由态钳回），只做几何、零副作用。
 * v1.81 起**按轴分派**（与半隐藏语义一致：左 / 右沿 x 半隐藏，上 / 下沿 y 半隐藏）：
 *   · 吸附 left / right：x = 展开贴边（`expandedX`），y = 纵向钳回（`clampY`）；
 *   · 吸附 top / bottom：y = 展开贴边（`expandedY`），x = 横向钳回（`clampX`）；
 *     随即缩回成半球是**运行期动作**，不在这里算；
 *   · 自由态（none）：完整钳回显示器内（`clampFreeInside`，旧实现不钳，球会丢在屏外）。
 * 调用方（onDragEnd）拿 `edge` 登记 dock、拿 `x/y` 经 `placeFloat` 摆放。
 */
export function planDrop(b: Rect, mon: Rect, snap: number = SNAP_LOGICAL): DropPlan {
  const edge = decideEdge(b, mon, snap)
  if (edge === 'left' || edge === 'right') {
    return { edge, x: expandedX(edge, mon, b.width), y: clampY(Math.round(b.y), b.height, mon) }
  }
  if (edge === 'top' || edge === 'bottom') {
    return {
      edge,
      x: clampX(Math.round(b.x), b.width, mon.x, mon.width),
      y: expandedY(edge, mon.y, mon.height, b.height),
    }
  }
  const p = clampFreeInside(b, mon)
  return { edge: 'none', x: p.x, y: p.y }
}

/** 悬停离开延迟到期的缩回决策产物。 */
export interface PeekCollapsePlan {
  /** 缩回目标左上角 x（DIP，半隐藏态）。 */
  x: number
  /** 缩回目标左上角 y（DIP，纵向钳回显示器内）。 */
  y: number
  /** true = 到期真缩回；false = 光标仍在缩回后矩形内，保持展开（防「滑到光标下又展开」）。 */
  collapse: boolean
}

/**
 * 悬停离开延迟到期的缩回决策 —— 2026-10-24 批次⑤ P2-4 自
 * `float-window.ts::finishPeekLeave` 抽出：算出半隐藏的缩回目标矩形，光标仍在其内则
 * **不缩**（边缘处展开 / 缩回平移会连环触发进出事件，即时缩回会让球在边缘抖动）。
 * v1.81 起与 `planDrop` 同构**按轴分派**：左 / 右只改 x（y 走 `clampY`），上 / 下只改 y
 * （x 走 `clampX`）。
 * 球窗恒 56×56（`placeFloat` 唯一定位入口保证，尺寸绝不读运行期 getBounds —— 出屏移动后会漂移），
 * 故尺寸一律取固定常量、不作参数。调用方只负责读真实光标位置与执行 `collapse`。
 */
export function planPeekCollapse(
  edge: Exclude<FloatEdge, 'none'>,
  mon: Rect,
  winX: number,
  winY: number,
  cursor: { x: number; y: number }
): PeekCollapsePlan {
  let x: number
  let y: number
  if (edge === 'left' || edge === 'right') {
    x = collapsedX(edge, mon, DEFAULT_FLOAT_W, PEEK_LOGICAL)
    y = clampY(Math.round(winY), DEFAULT_FLOAT_H, mon)
  } else {
    x = clampX(Math.round(winX), DEFAULT_FLOAT_W, mon.x, mon.width)
    y = collapsedY(edge, mon.y, mon.height, DEFAULT_FLOAT_H, PEEK_LOGICAL)
  }
  return {
    x,
    y,
    collapse: !pointInRect(cursor, rectAt(x, y, { width: DEFAULT_FLOAT_W, height: DEFAULT_FLOAT_H })),
  }
}

/** 宽容解析 `float_window` 段：字段缺失 / 类型错一律返回 null（本次不恢复，不影响其它段）。 */
export function parseFloatGeometry(v: unknown): FloatGeom | null {
  if (!v || typeof v !== 'object') return null
  const o = v as Record<string, unknown>
  const num = (k: string): number => (typeof o[k] === 'number' && Number.isFinite(o[k] as number) ? (o[k] as number) : NaN)
  const g: FloatGeom = {
    x: num('x'),
    y: num('y'),
    width: num('width'),
    height: num('height'),
    edge: o.edge === 'left' || o.edge === 'right' || o.edge === 'top' || o.edge === 'bottom' ? o.edge : 'none',
  }
  if (![g.x, g.y, g.width, g.height].every(Number.isFinite)) return null
  return g
}

/** 几何是否可用：尺寸不小于物理下限（过小视为坏几何，回退默认摆放）。 */
export function isUsablePhysicalGeom(g: FloatGeom): boolean {
  return g.width >= MIN_PHYS && g.height >= MIN_PHYS
}

/** 组装落盘段（键序 x/y/width/height/edge，与 Tauri serde 字段序逐字一致）。 */
export function geomToSegment(g: FloatGeom): Record<string, unknown> {
  return { x: g.x, y: g.y, width: g.width, height: g.height, edge: g.edge }
}
