/**
 * `float-layout.ts` 悬浮球几何纯逻辑单测 —— 与 Tauri 侧 floatwin.rs 的纯函数测试
 * （decide_edge / expanded_x / collapsed_x / clamp_y）逐条对应，外加 P2-4 从
 * float-window.ts 抽出的两个决策编排（planDrop / planPeekCollapse）与落盘段的宽容解析。
 *
 * 运行：`pnpm run unit`（node:test，零 Electron 依赖，TS 直跑）。
 */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  circleShapeRects,
  clampFreeInside,
  clampVisible,
  clampX,
  clampY,
  COLLAPSE_DELAY_MS,
  collapsedX,
  collapsedY,
  decideEdge,
  DEFAULT_FLOAT_H,
  DEFAULT_FLOAT_W,
  expandedX,
  expandedY,
  geomToSegment,
  isUsablePhysicalGeom,
  MARGIN_LOGICAL,
  MIN_VISIBLE_LOGICAL,
  parseFloatGeometry,
  PEEK_LOGICAL,
  planDrop,
  planPeekCollapse,
  pointInRect,
  rectAt,
  SNAP_LOGICAL,
  type Rect,
} from '../src/main/float-layout.ts'

const MON: Rect = { x: 0, y: 0, width: 1920, height: 1080 }
const WIN: Rect = { x: 0, y: 100, width: 56, height: 56 }

test('decideEdge：贴左 / 贴右识别，容差内外一致', () => {
  // snap=42（DIP）与 Rust 用例同一组数。
  assert.equal(decideEdge(WIN, MON, 42), 'left')
  assert.equal(decideEdge({ ...WIN, x: -10 }, MON, 42), 'left')
  assert.equal(decideEdge({ ...WIN, x: MON.width - WIN.width }, MON, 42), 'right')
  assert.equal(decideEdge({ ...WIN, x: MON.width - WIN.width + 10 }, MON, 42), 'right') // 右缘外 10
  assert.equal(decideEdge({ ...WIN, x: 500 }, MON, 42), 'none')
  assert.equal(decideEdge({ ...WIN, x: 100 }, MON, 42), 'none')
  // 默认逻辑 snap=28：贴边吸附、离开 28 以上不吸附。
  assert.equal(decideEdge(WIN, MON), 'left')
  assert.equal(decideEdge({ ...WIN, x: 60 }, MON), 'none')
})

test('decideEdge：重叠是对应轴吸附的前提；四缘化后贴上 / 下缘改判 top/bottom', () => {
  // 完全在屏幕下方且超出阈值 → 四条边都不吸（别的屏 / 甩飞的窗口）。
  assert.equal(decideEdge({ ...WIN, y: 2000 }, MON, 42), 'none')
  // v1.81 四缘化：窗口整体在屏幕下缘外 40px、横向仍重叠 —— 旧实现只看左右判 'none'，
  // 现在贴下缘在阈值内就该吸 'bottom'（与贴角就近择一是同一语义）。
  assert.equal(decideEdge({ ...WIN, y: MON.height, height: 40 }, MON, 42), 'bottom')
  // 完全在屏幕上方且超出阈值 → 不吸（纵向重叠仍是左右吸附的前提）。
  assert.equal(decideEdge({ ...WIN, y: -100, height: 40 }, MON, 42), 'none')
})

test('decideEdge：屏幕角就近择一 + 平局按 left → right → top → bottom 取先', () => {
  // 贴角的球同时离两条边都近（gap 同为 2）：旧实现按 if 顺序固定偏爱左边，
  // v1.81 起就近择一，平局按 left → right → top → bottom 顺序取先者。
  assert.equal(decideEdge({ ...WIN, x: 2, y: 2 }, MON, 42), 'left') // 平局 → left
  // 不平局时谁更近吸谁（反直觉的「固定偏爱左边」被消除）。
  assert.equal(decideEdge({ ...WIN, x: 25, y: 2 }, MON, 42), 'top') // 上 gap 2 < 左 gap 25
  assert.equal(decideEdge({ ...WIN, x: 2, y: 25 }, MON, 42), 'left') // 左 gap 2 < 上 gap 25
  // 右下角同理：底 gap 4 < 右 gap 5 → bottom。
  assert.equal(decideEdge({ ...WIN, x: 1859, y: 1020 }, MON, 42), 'bottom')
  // 右下角平局（右 gap 7 = 底 gap 7）→ 顺序靠前的 right。
  assert.equal(decideEdge({ ...WIN, x: 1857, y: 1017 }, MON, 42), 'right')
  // 上 / 下吸附要求横向重叠：完全在屏幕左侧的窗口不吸上 / 下（对称于左右的前提）。
  assert.equal(decideEdge({ x: -100, y: 2, width: 56, height: 56 }, MON, 42), 'none')
})

test('decideEdge：零 / 负尺寸不吸附', () => {
  assert.equal(decideEdge({ x: 0, y: 0, width: 0, height: 0 }, MON, 42), 'none')
})

test('expandedX / collapsedX / expandedY / collapsedY：四缘吸附两轴互为镜像', () => {
  assert.equal(expandedX('left', MON, WIN.width), 0)
  assert.equal(collapsedX('left', MON, WIN.width, 21), -35)
  assert.equal(expandedX('right', MON, WIN.width), 1864)
  // 右吸附半隐藏：x = 1920-21 = 1899（只露左侧 21 DIP）。
  assert.equal(collapsedX('right', MON, WIN.width, 21), 1899)
  // none 不参与半隐藏。
  assert.equal(collapsedX('none', MON, WIN.width, 21), 0)
  // y 轴同构：上 / 下吸附互为镜像（v1.81 四缘化）。
  assert.equal(expandedY('top', MON.y, MON.height, WIN.height), 0)
  assert.equal(expandedY('bottom', MON.y, MON.height, WIN.height), 1024)
  assert.equal(collapsedY('top', MON.y, MON.height, WIN.height, 21), -35)
  // 下吸附半隐藏：y = 1080-21 = 1059（只露上侧 21 DIP）。
  assert.equal(collapsedY('bottom', MON.y, MON.height, WIN.height, 21), 1059)
  // 左 / 右与 none 不参与 y 半隐藏（y 由调用方钳制）。
  assert.equal(collapsedY('left', MON.y, MON.height, WIN.height, 21), 0)
  // 多显示器负原点同理。
  assert.equal(collapsedY('top', -1080, 1080, 56, PEEK_LOGICAL), -1080 - (56 - PEEK_LOGICAL))
})

test('clampY / clampX：纵 / 横向钳在显示器内，超屏贴边而不是算出负区间', () => {
  assert.equal(clampY(-50, 56, MON), 0)
  assert.equal(clampY(1100, 56, MON), 1024)
  assert.equal(clampY(200, 56, MON), 200)
  // 窗口比屏幕高：贴顶而不是算出负区间。
  assert.equal(clampY(50, 2000, MON), 0)
  // x 轴同构（v1.81 四缘化：上 / 下吸附的另一轴走横向钳制）。
  assert.equal(clampX(-50, 56, MON.x, MON.width), 0)
  assert.equal(clampX(2000, 56, MON.x, MON.width), 1864)
  assert.equal(clampX(200, 56, MON.x, MON.width), 200)
  // 窗口比屏幕宽：贴左而不是算出负区间。
  assert.equal(clampX(50, 2000, MON.x, MON.width), 0)
})

test('多显示器（负原点）：几何相对该屏而非主屏原点', () => {
  const right: Rect = { x: 1920, y: 0, width: 1920, height: 1080 }
  assert.equal(decideEdge({ x: 1920, y: 100, width: 56, height: 56 }, right), 'left')
  assert.equal(expandedX('left', right, 56), 1920)
  assert.equal(collapsedX('left', right, 56, PEEK_LOGICAL), 1920 - (56 - PEEK_LOGICAL))
  assert.equal(clampY(100, 56, right), 100)
})

test('parseFloatGeometry：宽容解析，坏值回 null，edge 只认五值', () => {
  assert.equal(parseFloatGeometry(null), null)
  assert.equal(parseFloatGeometry('nope'), null)
  assert.equal(parseFloatGeometry({ x: 1, y: 2, width: 3 }), null) // 缺 height
  assert.deepEqual(parseFloatGeometry({ x: 1, y: 2, width: 3, height: 4, edge: 'right' }), {
    x: 1,
    y: 2,
    width: 3,
    height: 4,
    edge: 'right',
  })
  // v1.81 四缘化：top / bottom 是合法吸附边（旧断言这里期望归 none，现改判保留）。
  assert.equal(parseFloatGeometry({ x: 1, y: 2, width: 3, height: 4, edge: 'top' })?.edge, 'top')
  assert.equal(parseFloatGeometry({ x: 1, y: 2, width: 3, height: 4, edge: 'bottom' })?.edge, 'bottom')
  // 非法 edge 归一为 none（Tauri serde 未知值直接失败，这里更宽容但落盘仍是合法五值）。
  assert.equal(parseFloatGeometry({ x: 1, y: 2, width: 3, height: 4, edge: 'up' })?.edge, 'none')
  // NaN / 字符串数字不接受（避免落进坏几何）。
  assert.equal(parseFloatGeometry({ x: '1', y: 2, width: 3, height: 4 }), null)
})

test('isUsablePhysicalGeom：小于物理下限视为坏几何', () => {
  assert.equal(isUsablePhysicalGeom({ x: 0, y: 0, width: 30, height: 30, edge: 'none' }), false)
  assert.equal(isUsablePhysicalGeom({ x: 0, y: 0, width: 40, height: 40, edge: 'none' }), true)
})

test('geomToSegment：键序固定 x/y/width/height/edge（与 Tauri serde 字段序逐字一致）', () => {
  const seg = geomToSegment({ x: 12, y: 240, width: 84, height: 84, edge: 'left' })
  assert.deepEqual(Object.keys(seg), ['x', 'y', 'width', 'height', 'edge'])
  assert.equal(seg.edge, 'left')
  // v1.81 四缘化：top / bottom 落盘往返（serialize 小写字符串 → parse 认回）不丢边。
  for (const edge of ['top', 'bottom'] as const) {
    const s = geomToSegment({ x: 1, y: 2, width: 84, height: 84, edge })
    assert.deepEqual(Object.keys(s), ['x', 'y', 'width', 'height', 'edge'])
    assert.equal(parseFloatGeometry(s)?.edge, edge)
  }
})

test('pointInRect：半开区间，右 / 下沿不含', () => {
  const r: Rect = { x: 10, y: 20, width: 56, height: 56 }
  assert.equal(pointInRect({ x: 10, y: 20 }, r), true)
  assert.equal(pointInRect({ x: 65, y: 75 }, r), true)
  assert.equal(pointInRect({ x: 66, y: 50 }, r), false)
  assert.equal(pointInRect({ x: 50, y: 76 }, r), false)
  assert.equal(pointInRect({ x: 9, y: 50 }, r), false)
})

test('rectAt：按左上角 + 固定球尺寸构造矩形', () => {
  assert.deepEqual(rectAt(100, 200, { width: 56, height: 56 }), {
    x: 100,
    y: 200,
    width: 56,
    height: 56,
  })
})

test('clampVisible：拖动中可跨屏但至少留窄边（与 Rust clamp_visible 同式）', () => {
  // 显示器 [0,1920)、球 56、最小可见 16。
  assert.equal(clampVisible(500, 56, 0, 1920, 16), 500)
  assert.equal(clampVisible(3000, 56, 0, 1920, 16), 1920 - 16)
  assert.equal(clampVisible(-500, 56, 0, 1920, 16), 0 - 56 + 16)
  // 负原点显示器。
  assert.equal(clampVisible(-3000, 56, -1920, 0, 16), -1920 - 56 + 16)
})

test('clampFreeInside：松手自由态完整钳回显示器内（旧实现不钳，球会丢在屏外）', () => {
  assert.deepEqual(clampFreeInside({ x: 500, y: 200, width: 56, height: 56 }, MON), {
    x: 500,
    y: 200,
  })
  assert.deepEqual(clampFreeInside({ x: 3000, y: 2000, width: 56, height: 56 }, MON), {
    x: 1920 - 56,
    y: 1080 - 56,
  })
  assert.deepEqual(clampFreeInside({ x: -80, y: -80, width: 56, height: 56 }, MON), { x: 0, y: 0 })
  // 负原点显示器（多屏）。
  const right: Rect = { x: 1920, y: 0, width: 1920, height: 1080 }
  assert.deepEqual(clampFreeInside({ x: 5000, y: 500, width: 56, height: 56 }, right), {
    x: 1920 + 1920 - 56,
    y: 500,
  })
})

test('常量与 Tauri floatwin.rs 一致（误改常量会被这里挡住）', () => {
  assert.equal(DEFAULT_FLOAT_W, 56)
  assert.equal(DEFAULT_FLOAT_H, 56)
  assert.equal(PEEK_LOGICAL, 28)
  assert.equal(SNAP_LOGICAL, 28)
  assert.equal(MARGIN_LOGICAL, 12)
  assert.equal(MIN_VISIBLE_LOGICAL, 16)
  assert.equal(COLLAPSE_DELAY_MS, 300)
})

test('circleShapeRects：56×56 窗裁成圆的扫描线区域（圆心在内、四角在外、面积≈π/4）', () => {
  const d = DEFAULT_FLOAT_W
  const rects = circleShapeRects(d)
  const covered = (x: number, y: number): boolean =>
    rects.some((r) => x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)

  // ① 每行恰一段、1 像素高、且都落在窗口范围内（scanline 定义）。
  assert.ok(rects.length > 0)
  for (const r of rects) {
    assert.equal(r.height, 1)
    assert.ok(r.width > 0)
    assert.ok(r.x >= 0 && r.y >= 0 && r.x + r.width <= d && r.y < d)
  }
  assert.equal(new Set(rects.map((r) => r.y)).size, rects.length)

  // ② 圆心必须在内 —— 球面中心永远可点（这是「不把球削瘦」的底线）。
  assert.ok(covered(d / 2 - 1, d / 2 - 1) && covered(d / 2, d / 2))

  // ③ 四个角必须在外 —— 这正是 setShape 的意义：四角穿透到下层窗口，不吞点击。
  for (const [x, y] of [
    [0, 0],
    [d - 1, 0],
    [0, d - 1],
    [d - 1, d - 1],
  ]) {
    assert.ok(!covered(x, y), `四角 (${x},${y}) 不该落在圆形区域内`)
  }

  // ④ 左右对称，且宽度按行「中宽两端窄」（是圆不是橄榄 / 不是方块）。
  const widths = rects.map((r) => r.width)
  assert.deepEqual(widths, [...widths].reverse())
  const mid = widths[Math.floor(widths.length / 2)]
  assert.ok(widths[0] < mid && widths[widths.length - 1] < mid)

  // ⑤ 面积贴近 π/4（±5%）：整块矩形会到 100%，削太瘦也会被挡下。
  const area = rects.reduce((s, r) => s + r.width, 0)
  const ratio = area / (d * d)
  assert.ok(Math.abs(ratio - Math.PI / 4) < 0.05, `面积占比 ${ratio} 偏离 π/4`)

  // ⑥ 退化输入：空 / 负尺寸返回空数组（调用方据此跳过 setShape）。
  assert.deepEqual(circleShapeRects(0), [])
  assert.deepEqual(circleShapeRects(-5), [])
})

test('planDrop：松手落点决策 —— 吸附贴边 / 自由态钳回（P2-4 自 onDragEnd 抽出）', () => {
  // 贴左缘 → 吸附 left：x = 展开贴边、y = 纵向钳回（越顶贴顶）。
  assert.deepEqual(planDrop({ x: 2, y: -20, width: 56, height: 56 }, MON), { edge: 'left', x: 0, y: 0 })
  // 贴右缘 → 吸附 right：x = 展开贴右（1920-56）。
  assert.deepEqual(planDrop({ x: 1859, y: 100, width: 56, height: 56 }, MON), {
    edge: 'right',
    x: 1864,
    y: 100,
  })
  // 上 / 下缘松手 → 吸附 top / bottom：y = 展开贴边，x = 横向钳回（v1.81 按轴分派）。
  assert.deepEqual(planDrop({ x: 300, y: 2, width: 56, height: 56 }, MON), {
    edge: 'top',
    x: 300,
    y: 0,
  })
  assert.deepEqual(planDrop({ x: 300, y: 1020, width: 56, height: 56 }, MON), {
    edge: 'bottom',
    x: 300,
    y: 1024,
  })
  // 就近择一 + 另一轴钳回：左上角附近上 gap 2 < 左 gap |-10| ⇒ 判 top，且 x 越左钳回 0。
  assert.deepEqual(planDrop({ x: -10, y: 2, width: 56, height: 56 }, MON), {
    edge: 'top',
    x: 0,
    y: 0,
  })
  // 屏中间 → 自由态：合法坐标原样（clampFreeInside 不动屏内坐标）。
  assert.deepEqual(planDrop({ x: 500, y: 300, width: 56, height: 56 }, MON), {
    edge: 'none',
    x: 500,
    y: 300,
  })
  // 屏外松手 → 自由态仍完整钳回显示器内（旧实现不钳，球会丢在屏外找不回来）。
  assert.deepEqual(planDrop({ x: 3000, y: 2000, width: 56, height: 56 }, MON), {
    edge: 'none',
    x: 1864,
    y: 1024,
  })
  // 负原点显示器（多屏）：吸附判定与落点相对该屏，不是主屏原点。
  const right: Rect = { x: 1920, y: 0, width: 1920, height: 1080 }
  assert.deepEqual(planDrop({ x: 1922, y: 120, width: 56, height: 56 }, right), {
    edge: 'left',
    x: 1920,
    y: 120,
  })
  // snap 容差边界与 decideEdge 同判据（容差外一律自由态）。
  assert.equal(planDrop({ x: 60, y: 100, width: 56, height: 56 }, MON).edge, 'none')
  assert.equal(planDrop({ x: 60, y: 100, width: 56, height: 56 }, MON, 42).edge, 'none')
  assert.equal(planDrop({ x: 40, y: 100, width: 56, height: 56 }, MON, 42).edge, 'left')
})

test('planPeekCollapse：延迟到期缩回决策 —— 光标在缩回后矩形内则不缩（P2-4 自 finishPeekLeave 抽出）', () => {
  // 左吸附：缩回目标 = 半隐藏（x = -28，只露 28 DIP 半球），y = 纵向钳回后的窗口 y。
  // v1.81 起签名按轴对称（winX, winY）：左 / 右只用 winY，上 / 下只用 winX。
  assert.deepEqual(planPeekCollapse('left', MON, 0, 300, { x: 0, y: 320 }), {
    x: -28,
    y: 300,
    collapse: false,
  })
  // 光标不在缩回后矩形 [-28,28)×[300,356) 内 → 到期真缩回。
  assert.equal(planPeekCollapse('left', MON, 0, 300, { x: 500, y: 320 }).collapse, true)
  // 半开区间：右 / 下沿恰在边界即算「不在」（与 pointInRect 同语义）。
  assert.equal(planPeekCollapse('left', MON, 0, 300, { x: 27, y: 320 }).collapse, false)
  assert.equal(planPeekCollapse('left', MON, 0, 300, { x: 28, y: 320 }).collapse, true)
  assert.equal(planPeekCollapse('left', MON, 0, 300, { x: 0, y: 356 }).collapse, true)

  // 右吸附镜像：缩回目标 x = 1920-28 = 1892，矩形 [1892,1948)×[y,y+56)。
  assert.deepEqual(planPeekCollapse('right', MON, 1864, 300, { x: 1920, y: 320 }), {
    x: 1892,
    y: 300,
    collapse: false,
  })
  assert.equal(planPeekCollapse('right', MON, 1864, 300, { x: 1891, y: 320 }).collapse, true)

  // y 越顶 / 越底：缩回目标纵向钳回显示器内（光标判定跟着钳后矩形走）。
  assert.deepEqual(planPeekCollapse('left', MON, 0, -20, { x: 0, y: 0 }), {
    x: -28,
    y: 0,
    collapse: false,
  })
  assert.deepEqual(planPeekCollapse('left', MON, 0, 2000, { x: 0, y: 1079 }), {
    x: -28,
    y: 1024,
    collapse: false,
  })
  assert.equal(planPeekCollapse('left', MON, 0, 2000, { x: 0, y: 1080 }).collapse, true)

  // 上 / 下吸附镜像（v1.81 按轴分派）：只改 y（半隐藏），x = 横向钳回后的窗口 x。
  assert.deepEqual(planPeekCollapse('top', MON, 300, 0, { x: 320, y: 0 }), {
    x: 300,
    y: -28,
    collapse: false,
  })
  // 半开区间：下沿恰在 28 即算「不在」。
  assert.equal(planPeekCollapse('top', MON, 300, 0, { x: 320, y: 28 }).collapse, true)
  assert.deepEqual(planPeekCollapse('bottom', MON, 300, 500, { x: 320, y: 1060 }), {
    x: 300,
    y: 1052,
    collapse: false,
  })
  // x 越左 / 越右：缩回目标横向钳回显示器内（光标判定跟着钳后矩形走）。
  assert.deepEqual(planPeekCollapse('top', MON, -20, 0, { x: 0, y: 0 }), {
    x: 0,
    y: -28,
    collapse: false,
  })
  assert.deepEqual(planPeekCollapse('bottom', MON, 2000, 500, { x: 1919, y: 1060 }), {
    x: 1864,
    y: 1052,
    collapse: false,
  })
})
