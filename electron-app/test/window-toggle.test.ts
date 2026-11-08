/**
 * `window-toggle.ts::planToggle` 单测（2026-11-07 架构审查 M-8）。
 *
 * 这条判据同时服务托盘左键、唤醒快捷键与悬浮球单击；四象限钉死，
 * 避免有人把「可见且非最小化」简化成 `isVisible()`（最小化状态下第一次点击会像没反应）。
 */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { planToggle } from '../src/main/window-toggle.ts'

test('可见且非最小化 → 隐藏', () => {
  assert.equal(planToggle(true, false), 'hide')
})

test('可见但最小化 → 呈现（不是隐藏）', () => {
  assert.equal(planToggle(true, true), 'present')
})

test('隐藏中 → 呈现', () => {
  assert.equal(planToggle(false, false), 'present')
})

test('隐藏且最小化 → 呈现', () => {
  assert.equal(planToggle(false, true), 'present')
})
