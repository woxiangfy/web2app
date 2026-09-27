/**
 * `tabs.ts` 页签集合的**纯规则**单测 —— 不启动 Electron、不建任何窗口。
 *
 * 为什么单独测：「关页签后激活谁」是典型的「错了也不报错」—— 只是焦点跳错页签，
 * 探针若只用 2 个页签（关唯一的激活项，左邻右邻恰好是同一个）还会假绿。
 * 规则必须与 Tauri 侧 `tabs.rs::close_tab_inner`（优先右邻，末位取左邻）逐字对齐。
 *
 * 运行：`pnpm run unit`（node --test 直接跑 .ts，零 Electron 依赖）。
 */
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { nextActiveLabelAfterClose } from '../src/main/tab-order.ts'

test('关闭中间页签 → 激活右邻', () => {
  // [a,b,c] 关 b(1) → c；旧实现删除后 findIndex 恒 -1，会错误激活 a
  assert.equal(nextActiveLabelAfterClose(['a', 'b', 'c'], 1), 'c')
})

test('关闭末位页签 → 激活左邻', () => {
  // [a,b,c] 关 c(2) → b（没有右邻）
  assert.equal(nextActiveLabelAfterClose(['a', 'b', 'c'], 2), 'b')
})

test('关闭首位页签 → 激活右邻', () => {
  // [a,b,c] 关 a(0) → b
  assert.equal(nextActiveLabelAfterClose(['a', 'b', 'c'], 0), 'b')
})

test('仅剩两个页签时两个方向都落到对方', () => {
  assert.equal(nextActiveLabelAfterClose(['a', 'b'], 0), 'b')
  assert.equal(nextActiveLabelAfterClose(['a', 'b'], 1), 'a')
})

test('关闭唯一页签 → null（交给调用方走关闭宿主语义）', () => {
  assert.equal(nextActiveLabelAfterClose(['only'], 0), null)
})

test('空列表 / 非法下标 → null，绝不抛错', () => {
  assert.equal(nextActiveLabelAfterClose([], 0), null)
  assert.equal(nextActiveLabelAfterClose([], -1), null)
  assert.equal(nextActiveLabelAfterClose(['a', 'b'], -1), null)
  assert.equal(nextActiveLabelAfterClose(['a', 'b'], 9), null)
})
