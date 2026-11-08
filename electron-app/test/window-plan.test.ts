/**
 * `window-plan.ts` 纯判定单测（2026-11-07 架构审查 M-5）。
 *
 * 此前 `window-state.ts` 顶层 `import { screen } from 'electron'`，`parseWindowState` /
 * `planRestore` 在无 Electron 的单测里根本 import 不进来；判定搬进零依赖模块后由这里覆盖。
 * 四分支与 `webview-app/src/window_state.rs::apply()` 一一对应。
 */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { parseWindowState, planRestore, type RestoreContext } from '../src/main/window-plan.ts'

const IDENTITY: RestoreContext = {
  toDipPoint: (p) => p,
  toDipRect: (r) => r,
  displays: [{ x: 0, y: 0, width: 1920, height: 1080 }],
}

test('parseWindowState：完整字段原样保留', () => {
  assert.deepEqual(parseWindowState({ x: 100, y: 200, width: 800, height: 600, maximized: true }), {
    x: 100,
    y: 200,
    width: 800,
    height: 600,
    maximized: true,
  })
})

test('parseWindowState：非对象 / 缺坐标 → null（坏段不连累三个开关）', () => {
  for (const v of [null, undefined, 'nope', 42, [], { width: 800, height: 600 }, { x: 1, y: 2 }]) {
    assert.equal(parseWindowState(v), null, `应判无效：${JSON.stringify(v)}`)
  }
})

test('parseWindowState：maximized 只认严格的 true', () => {
  assert.equal(parseWindowState({ x: 0, y: 0, width: 800, height: 600, maximized: 'true' })?.maximized, false)
  assert.equal(parseWindowState({ x: 0, y: 0, width: 800, height: 600 })?.maximized, false)
})

test('planRestore：无保存状态 → 默认尺寸', () => {
  const p = planRestore(null, IDENTITY)
  assert.equal(p.maximize, false)
  assert.equal(p.bounds, undefined)
})

test('planRestore：上次最大化 → 直接最大化（不看坐标）', () => {
  const p = planRestore({ x: -99999, y: -99999, width: 800, height: 600, maximized: true }, IDENTITY)
  assert.equal(p.maximize, true)
})

test('planRestore：尺寸过小 → 最大化', () => {
  const a = planRestore({ x: 100, y: 100, width: 419, height: 700, maximized: false }, IDENTITY)
  const b = planRestore({ x: 100, y: 100, width: 900, height: 319, maximized: false }, IDENTITY)
  assert.equal(a.maximize, true)
  assert.equal(b.maximize, true)
})

test('planRestore：中心点落在显示器上 → 原位恢复（四舍五入到 DIP 整数）', () => {
  const p = planRestore({ x: 200, y: 200, width: 900, height: 700, maximized: false }, IDENTITY)
  assert.equal(p.maximize, false)
  assert.deepEqual(p.bounds, { x: 200, y: 200, width: 900, height: 700 })
})

test('planRestore：中心点不在任何显示器上 → 最大化（拔显示器后不留屏外窗口）', () => {
  const p = planRestore({ x: -99999, y: -99999, width: 800, height: 600, maximized: false }, {
    ...IDENTITY,
    displays: [],
  })
  assert.equal(p.maximize, true)
})

test('planRestore：物理 → DIP 换算按注入的适配器走（125% 缩放下落盘物理、恢复 DIP）', () => {
  // 物理 1000,500 1600×1200 @125% ⇒ 中心 (1800,1100) → DIP (1440,880) 仍在 1920×1080 内
  const p = planRestore({ x: 1000, y: 500, width: 1600, height: 1200, maximized: false }, {
    toDipPoint: (q) => ({ x: q.x / 1.25, y: q.y / 1.25 }),
    toDipRect: (r) => ({ x: r.x / 1.25, y: r.y / 1.25, width: r.width / 1.25, height: r.height / 1.25 }),
    displays: [{ x: 0, y: 0, width: 1920, height: 1080 }],
  })
  assert.equal(p.maximize, false)
  assert.deepEqual(p.bounds, { x: 800, y: 400, width: 1280, height: 960 })
})
