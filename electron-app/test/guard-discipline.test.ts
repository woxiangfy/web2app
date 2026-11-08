/**
 * 守护「守卫本身」的两条纪律（2026-11-07 架构审查批次⑥）。
 *
 * 静态源码守卫最怕两种假绿：
 *   ① 「A 或 B 只要出现一个就算过」（`.includes(a) || .includes(b)`）—— 注释提到、别处碰巧
 *      有同名文字都能让断言通过；
 *   ② 直接对含注释的源码断言 —— 文件里为了解释「不得用 X」而写了 X，断言就被注释满足。
 * ② 无法通用自动判定（要理解每条断言的语义），但①可以机械扫出来，故钉①。
 */
import assert from 'node:assert/strict'
import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from 'node:test'

const testDir = join(process.cwd(), 'test')

test('静态守卫不得用 `.includes(a) || .includes(b)` 这种「任一命中即绿」写法', () => {
  const offenders: string[] = []
  for (const f of readdirSync(testDir).filter((n) => n.endsWith('.test.ts'))) {
    // 本文件自己的注释里就有这个模式，跳过。
    if (f === 'guard-discipline.test.ts') continue
    const src = readFileSync(join(testDir, f), 'utf8')
    // 只扫 assert 调用里的「includes(...) || ... includes(...)」，字符串字面量里的 || 不算。
    const re = /assert\.(?:ok|equal)\([\s\S]{0,200}?\.includes\([^)]*\)\s*\|\|\s*[\s\S]{0,80}?\.includes\(/g
    for (const m of src.match(re) ?? []) {
      offenders.push(`${f}: ${m.replace(/\s+/g, ' ').slice(0, 120)}`)
    }
  }
  assert.deepEqual(offenders, [], '改用「剥注释后逐条断言」，不要用任一命中即绿的兜底')
})
