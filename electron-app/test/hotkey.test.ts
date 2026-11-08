/**
 * `hotkey.ts::hotkeyProblem` 单测（2026-11-07 架构审查 M-2）。
 *
 * 这些判据同时存在于共享设置页 `settings.html::hkProblem`（页面即时提示）；
 * 主进程这一份是第二道，拦住「绕过页面直接写 settings.json / 兜底页」。
 * 两边文案/判据的一致性由 `cross-shell-contract.test.ts` 对照。
 */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { hotkeyProblem, type Hotkey } from '../src/main/hotkey.ts'

const hk = (p: Partial<Hotkey>): Hotkey => ({ ctrl: false, alt: false, shift: false, key: '', ...p })

test('未设置（key 为空）永远合法', () => {
  assert.equal(hotkeyProblem(hk({})), '')
  // 只勾了修饰键、没有主键：等价于「不设置」（页面保存前也会归一成空）
  assert.equal(hotkeyProblem(hk({ ctrl: true, alt: true })), '')
})

test('裸单键被拒绝：会占满该键的输入', () => {
  const msg = hotkeyProblem(hk({ key: 'F9' }))
  assert.match(msg, /只有单个按键/)
  assert.match(msg, /至少勾选一个修饰键/)
})

test('系统占用组合被拒绝（Alt+Tab / Ctrl+Space …）', () => {
  assert.match(hotkeyProblem(hk({ alt: true, key: 'Tab' })), /已被占用/)
  assert.match(hotkeyProblem(hk({ ctrl: true, key: 'Space' })), /输入法中英文切换/)
})

test('Shift+F1（打开设置）被拒绝', () => {
  assert.match(hotkeyProblem(hk({ shift: true, key: 'F1' })), /Shift\+F1/)
})

test('accelerator 无法映射的键被拒绝（不会静默注册失败）', () => {
  assert.match(hotkeyProblem(hk({ ctrl: true, key: 'F25' })), /不支持的按键/)
})

test('合法组合通过（Ctrl+Alt+Digit4 / Shift+F2）', () => {
  assert.equal(hotkeyProblem(hk({ ctrl: true, alt: true, key: 'Digit4' })), '')
  assert.equal(hotkeyProblem(hk({ shift: true, key: 'F2' })), '')
})
