/**
 * lifecycle.ts 单测（node:test，纯判据，零 Electron 依赖）。
 *
 * 钉的是主窗口关闭按钮的「驻留 vs 退出」判据：它在 Electron 的 close 事件栈里
 * 没法直接断言，而判据错一条就是真机死局（球在却唤不回窗口 / 该驻留却退了）。
 * closeResidency 只 type-import settings.ts（类型擦除后无运行期依赖），可直接跑 TS。
 *
 * 运行：`pnpm run unit`（Node ≥ 22.6 内置类型擦除；本机 Node 24 直接执行 .ts）。
 */
import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { Settings } from '../src/main/settings.ts'
import { closeResidency } from '../src/main/lifecycle.ts'

function settings(over: Partial<Settings>): Settings {
  return {
    wake_hotkey: { ctrl: false, alt: false, shift: false, key: '' },
    tray_enabled: false,
    minimize_on_close: false,
    float_enabled: false,
    ...over,
  }
}

test('托盘驻留：启用托盘 + 关闭时最小化 → hide，理由「托盘」', () => {
  const d = closeResidency(settings({ tray_enabled: true, minimize_on_close: true }), 'win32')
  assert.equal(d.action, 'hide')
  assert.deepEqual(d.reasons, ['托盘'])
})

test('悬浮图标驻留：win32 + 启用悬浮图标 → hide（真机缺陷修复的关键分支）', () => {
  const d = closeResidency(settings({ float_enabled: true }), 'win32')
  assert.equal(d.action, 'hide')
  assert.deepEqual(d.reasons, ['悬浮图标'])
})

test('双理由并存 → hide，reasons 顺序「托盘 → 悬浮图标」（join 即日志原文）', () => {
  const d = closeResidency(
    settings({ tray_enabled: true, minimize_on_close: true, float_enabled: true }),
    'win32'
  )
  assert.equal(d.action, 'hide')
  assert.deepEqual(d.reasons, ['托盘', '悬浮图标'])
  assert.equal(d.reasons.join(' + '), '托盘 + 悬浮图标')
})

test('非 Windows 的悬浮开关无行为：不得驻留（球窗根本不会建）', () => {
  const d = closeResidency(settings({ float_enabled: true }), 'linux')
  assert.equal(d.action, 'quit')
  assert.deepEqual(d.reasons, [])
})

test('只勾「关闭时最小化」而未启用托盘 → quit（托盘不在，无处驻留）', () => {
  const d = closeResidency(settings({ minimize_on_close: true }), 'win32')
  assert.equal(d.action, 'quit')
})

test('全关 → quit（默认策略：关闭即退出）', () => {
  const d = closeResidency(settings({}), 'win32')
  assert.equal(d.action, 'quit')
  assert.deepEqual(d.reasons, [])
})
