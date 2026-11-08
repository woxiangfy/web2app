/**
 * `ipc-guard.ts` 单测（node:test，零 Electron 依赖）。
 *
 * 钉的是 2026-11-07 架构审查 H-3 的语义：sender 闸门**两种模式都校验**，
 * 且「宿主未就绪 / 已销毁」时一律拒绝（此前单窗口模式直接放行）。
 *
 * 行为层由探针钉（P11：内容视图发特权命令被拒），这里钉判据本身 ——
 * 一处纯函数测试比四份同构代码各自「看起来对」更可靠。
 *
 * 运行：`pnpm run unit`。
 */
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from 'node:test'

import { assertTrustedSender } from '../src/main/ipc-guard.ts'

test('sender.id 等于宿主 id → 放行', () => {
  assert.doesNotThrow(() => assertTrustedSender(7, 7, { label: '窗口命令' }))
})

test('sender.id 不等于宿主 id → 拒绝并说明期望值', () => {
  assert.throws(() => assertTrustedSender(8, 7, { label: '窗口命令' }), /窗口命令/)
})

test('宿主未就绪 / 已销毁（expected = null）→ 一律拒绝，不得放行', () => {
  // 这条正是旧 assertPrivileged 在单窗口模式下的缺口：没有期望值就必须拒绝。
  assert.throws(() => assertTrustedSender(3, null, { label: '页签命令' }), /页签命令/)
})

test('app-ipc 静态守卫：不得退回「单窗口模式跳过校验」，bootstrap 必须校验顶层框架', () => {
  const src = readFileSync(join(process.cwd(), 'src', 'main', 'app-ipc.ts'), 'utf8')
  assert.ok(!/deps\.tabsMode/.test(src), 'app-ipc 不得再按 tabsMode 决定是否校验 sender')
  assert.ok(src.includes('e.senderFrame !== e.sender.mainFrame'), 'bootstrap 必须拒绝子框架')
  // 两条关键闸都必须走共用判据（而不是各自内联一份比较）。
  assert.ok(src.includes("label: '窗口命令'"))
  assert.ok(src.includes("label: '页签命令'"))
})
