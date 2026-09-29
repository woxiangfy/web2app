/**
 * `distribution.ts` 分发形态标记读取的单测 —— 不启动 Electron。
 *
 * 为什么必须测：安装版/绿色版的数据目录分流完全靠这个标记 —— 标记读错，绿色版会被
 * 强制到 %APPDATA%（不再「绿色」），或安装版把数据写进只读的 Program Files 旁（启动即
 * fallback）。这些失败都不报错，只表现为路径悄悄变了，必须用规则测试钉死。
 *
 * 运行：`pnpm run unit`（node --test 直接跑 .ts，零 Electron 依赖）。
 */
import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'

import {
  INSTALLER_DISTRIBUTION,
  isInstallerBuild,
  readDistribution,
} from '../src/main/distribution.ts'

/** 建一个临时目录并写入 package.json，返回目录路径（测试结束自动清理）。 */
function withPkgJson(content: string): string {
  const dir = mkdtempSync(join(tmpdir(), 'w2a-dist-'))
  writeFileSync(join(dir, 'package.json'), content, 'utf8')
  return dir
}

test('distribution=nsis → 识别为安装版', () => {
  const dir = withPkgJson(JSON.stringify({ distribution: 'nsis' }))
  try {
    assert.equal(readDistribution(dir), INSTALLER_DISTRIBUTION)
    assert.equal(isInstallerBuild(dir), true)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('其它取值（含绿色版常见的缺省）→ 不是安装版', () => {
  const dir = withPkgJson(JSON.stringify({ name: 'web2app-electron' }))
  try {
    assert.equal(readDistribution(dir), null)
    assert.equal(isInstallerBuild(dir), false)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('非 nsis 的字符串 → 原样读出但不判定为安装版', () => {
  const dir = withPkgJson(JSON.stringify({ distribution: 'portable' }))
  try {
    assert.equal(readDistribution(dir), 'portable')
    assert.equal(isInstallerBuild(dir), false)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('空串 / 非字符串 / package.json 不是对象 → null，绝不抛错', () => {
  const cases = [
    JSON.stringify({ distribution: '' }),
    JSON.stringify({ distribution: 123 }),
    JSON.stringify({ distribution: null }),
    '42',
    '"a string"',
  ]
  for (const c of cases) {
    const dir = withPkgJson(c)
    try {
      assert.equal(readDistribution(dir), null, `输入：${c}`)
      assert.equal(isInstallerBuild(dir), false)
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  }
})

test('package.json 损坏 / 目录不存在 → null（启动早期不得因此中断）', () => {
  const broken = withPkgJson('{ not json')
  try {
    assert.equal(readDistribution(broken), null)
    assert.equal(isInstallerBuild(join(tmpdir(), 'w2a-no-such-dir-xyz')), false)
  } finally {
    rmSync(broken, { recursive: true, force: true })
  }
})
