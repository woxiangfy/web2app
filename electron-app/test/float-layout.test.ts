/**
 * `float-layout.ts` 悬浮球几何纯逻辑单测 —— 与 Tauri 侧 floatwin.rs 的纯函数测试
 * （decide_edge / expanded_x / collapsed_x / clamp_y）逐条对应，外加落盘段的宽容解析。
 *
 * 运行：`pnpm run unit`（node:test，零 Electron 依赖，TS 直跑）。
 */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  circleShapeRects,
  clampFreeInside,
  clampVisible,
  clampY,
  COLLAPSE_DELAY_MS,
  collapsedX,
  decideEdge,
  DEFAULT_FLOAT_H,
  DEFAULT_FLOAT_W,
  expandedX,
  geomToSegment,
  isUsablePhysicalGeom,
  MARGIN_LOGICAL,
  MIN_VISIBLE_LOGICAL,
  parseFloatGeometry,
  PEEK_LOGICAL,
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

test('decideEdge：纵向无重叠不吸附（别的屏 / 完全在上下方）', () => {
  assert.equal(decideEdge({ ...WIN, y: 2000 }, MON, 42), 'none')
  assert.equal(decideEdge({ ...WIN, y: MON.height, height: 40 }, MON, 42), 'none')
})

test('decideEdge：零 / 负尺寸不吸附', () => {
  assert.equal(decideEdge({ x: 0, y: 0, width: 0, height: 0 }, MON, 42), 'none')
})

test('expandedX / collapsedX：左右吸附互为镜像', () => {
  assert.equal(expandedX('left', MON, WIN.width), 0)
  assert.equal(collapsedX('left', MON, WIN.width, 21), -35)
  assert.equal(expandedX('right', MON, WIN.width), 1864)
  // 右吸附半隐藏：x = 1920-21 = 1899（只露左侧 21 DIP）。
  assert.equal(collapsedX('right', MON, WIN.width, 21), 1899)
  // none 不参与半隐藏。
  assert.equal(collapsedX('none', MON, WIN.width, 21), 0)
})

test('clampY：贴上 / 贴下 / 合法 y 原样返回', () => {
  assert.equal(clampY(-50, 56, MON), 0)
  assert.equal(clampY(1100, 56, MON), 1024)
  assert.equal(clampY(200, 56, MON), 200)
  // 窗口比屏幕高：贴顶而不是算出负区间。
  assert.equal(clampY(50, 2000, MON), 0)
})

test('多显示器（负原点）：几何相对该屏而非主屏原点', () => {
  const right: Rect = { x: 1920, y: 0, width: 1920, height: 1080 }
  assert.equal(decideEdge({ x: 1920, y: 100, width: 56, height: 56 }, right), 'left')
  assert.equal(expandedX('left', right, 56), 1920)
  assert.equal(collapsedX('left', right, 56, PEEK_LOGICAL), 1920 - (56 - PEEK_LOGICAL))
  assert.equal(clampY(100, 56, right), 100)
})

test('parseFloatGeometry：宽容解析，坏值回 null，edge 只认三值', () => {
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
  // 非法 edge 归一为 none（Tauri serde 未知值直接失败，这里更宽容但落盘仍是合法三值）。
  assert.equal(parseFloatGeometry({ x: 1, y: 2, width: 3, height: 4, edge: 'top' })?.edge, 'none')
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
