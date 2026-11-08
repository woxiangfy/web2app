/**
 * 开发期工具隔离守卫（2026-11-07 架构审查 H-4）。
 *
 * 探针（约 2300 行）此前被**静态 import** 进生产入口，随 `out/**\/*` 进 asar。
 * 现在要求：入口只允许 `import type` + 运行期 `await import(...)`；
 * 打包链路必须 build → stage → prune → electron-builder；asar 护栏必须有反向断言。
 *
 * 这些是构建/打包纪律，跑 electron-builder 太重，这里用源码文本 + package.json 结构钉住。
 */
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from 'node:test'

const appDir = process.cwd()
const read = (p: string): string => readFileSync(join(appDir, p), 'utf8')
const indexTs = read('src/main/index.ts')
const pkg = JSON.parse(read('package.json')) as { scripts: Record<string, string> }
const asarGuard = read('scripts/check-asar-config.cjs')

test('入口不得静态 import 探针 / 冒烟实现，只能 import type + 动态 import', () => {
  assert.ok(
    !/^import\s+(?!type\b)[^\n;]*from\s*'\.\/probe'/m.test(indexTs),
    "index.ts 必须写成 `import type { ... } from './probe'`，不得静态引入实现"
  )
  assert.ok(!/^import\s+(?!type\b)[^\n;]*from\s*'\.\/smoke'/m.test(indexTs), 'index.ts 不得静态引入 ./smoke')
  assert.ok(indexTs.includes("await import('./probe.js')"), '探针必须运行期条件加载')
  assert.ok(indexTs.includes("await import('./smoke.js')"), '冒烟自检必须运行期条件加载')
})

test('打包脚本顺序：build → stage → prune → electron-builder', () => {
  for (const name of ['pack', 'dist', 'dist:installer', 'dist:all']) {
    const s = pkg.scripts[name]
    assert.ok(s, `缺少脚本 ${name}`)
    const iBuild = s.indexOf('pnpm run build')
    const iStage = s.indexOf('pnpm run stage')
    const iPrune = s.indexOf('pnpm run prune')
    const iBuilder = s.indexOf('electron-builder')
    assert.ok(iBuild >= 0 && iStage >= 0 && iPrune >= 0 && iBuilder >= 0, `${name} 缺少必备步骤`)
    assert.ok(iBuild < iStage && iStage < iPrune && iPrune < iBuilder, `${name} 步骤顺序不对`)
  }
})

test('asar 护栏反向断言：包内不得含探针产物', () => {
  assert.ok(asarGuard.includes("f.startsWith('/out/main/probe')"), '缺少 out/main/probe* 断言')
  assert.ok(asarGuard.includes("'/out/renderer/tabs-probe-preload.js'"), '缺少探针 preload 断言')
})

test('prune 脚本覆盖探针 preload 且缺 index.js 时报错', () => {
  const prune = read('scripts/prune-probe.cjs')
  assert.ok(prune.includes('tabs-probe-preload.js'), 'prune 必须删除页签探针 preload')
  assert.ok(prune.includes("startsWith('probe')"), 'prune 必须按前缀删除 out/main/probe*')
  assert.ok(prune.includes('out/main/index.js'), 'prune 在未构建时应显式失败')
})
