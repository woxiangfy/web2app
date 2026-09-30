/**
 * `first-existing.ts` 候选扫描单测 —— 三个资源定位器（sidecar / 托盘图标 / 设置页 HTML）
 * 共用的底层规则：返回第一个实际存在的候选，其余不再探测。
 *
 * 运行：`pnpm run unit`
 */
import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, test } from 'node:test'

import { firstExisting } from '../src/main/first-existing.ts'

let dirs: string[] = []
afterEach(() => {
  for (const d of dirs) rmSync(d, { recursive: true, force: true })
  dirs = []
})

function makeDir(): string {
  const d = mkdtempSync(join(tmpdir(), 'w2a-fe-'))
  dirs.push(d)
  return d
}

test('firstExisting：空候选 / 全部不存在 → null，绝不抛错', () => {
  assert.equal(firstExisting([]), null)
  assert.equal(firstExisting([join(tmpdir(), 'w2a-no-such-a'), join(tmpdir(), 'w2a-no-such-b')]), null)
})

test('firstExisting：返回第一个存在的，且按顺序短路（目录也算存在）', () => {
  const d = makeDir()
  const missing = join(d, 'missing.exe')
  const present = join(d, 'present.png')
  writeFileSync(present, 'x')
  assert.equal(firstExisting([missing, present, d]), present, '跳过缺失项命中首个存在的文件')
  assert.equal(firstExisting([d, present]), d, '第一个候选（目录）存在即返回，不继续探测')
})
