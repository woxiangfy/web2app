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
import { readFileSync, readdirSync } from 'node:fs'
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

/**
 * 本地打包这条路（`tests/build-electron-all.bat` → 根 `scripts/pack-local.mjs` → 本文件）
 * 是**第二个**出包入口。2026-11-07 那次整改只给 `package.json` 的 pack/dist 加了 `prune`，
 * pack-local 被漏掉 ⇒ 2026-11-14 本地打包出的 asar 混进 6 个探针产物、护栏红灯。
 * 这里把「第二个入口也必须有 prune」钉住，防止再靠实测发现。
 */
test('pack-local 的 Electron 驱动同样必须 build → stage → prune → electron-builder', () => {
  const driverPath = join(appDir, '..', 'scripts', 'packlib', 'driver-electron.mjs')
  const raw = readFileSync(driverPath, 'utf8')
  // 先剥注释：该文件里大量注释解释「为什么要 prune」，不剥会被注释满足而假绿。
  const code = raw.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '')
  const steps = {
    build: "run(['pnpm', 'run', 'build']",
    stage: "run(['pnpm', 'run', 'stage']",
    prune: "run(['pnpm', 'run', 'prune']",
    builder: 'runTee(builderCmd(',
  }
  const idx: Record<string, number> = {}
  for (const [name, needle] of Object.entries(steps)) {
    idx[name] = code.indexOf(needle)
    assert.ok(idx[name] >= 0, `driver-electron.mjs 缺少 ${name} 步骤`)
  }
  assert.ok(
    idx.build < idx.stage && idx.stage < idx.prune && idx.prune < idx.builder,
    'driver-electron.mjs 步骤顺序不对：必须 build → stage → prune → electron-builder'
  )
})

/**
 * 第三个出包入口：`.github/workflows/` 里**自己拼步骤**的 electron 构建。
 * 2026-11-14 实测同一缺陷：`electron-build-custom.yml` 只跑 build → stage 就
 * `pnpm exec electron-builder`（它不调 `pnpm run dist`，因为要按勾选组合出包）⇒ 探针进 asar。
 * 这里按「文件里出现 electron-builder」扫描全部 workflow，要求二选一：
 * ① 走复合脚本（`pnpm run dist` / `pnpm run pack`，内部已含 prune）；
 * ② 自己拼步骤 ⇒ 必须显式 `pnpm run prune` 且位置在第一个 electron-builder 之前，
 *    并且保留 `check-asar-config.cjs`（否则漏了也不会有人发现）。
 */
test('workflow 里的 electron 出包入口同样不得漏 prune', () => {
  const wfDir = join(appDir, '..', '.github', 'workflows')
  const offenders: string[] = []
  for (const f of readdirSync(wfDir).filter((n) => /\.ya?ml$/.test(n))) {
    // 只剥**整行注释**：这些 workflow 的注释里大量出现 `electron-builder` / `pnpm run dist` 字样，
    // 不剥就会把「注释提到过」当成「真的跑了」，正是 2026-11-07 批次⑥ 那类假绿。
    const code = readFileSync(join(wfDir, f), 'utf8')
      .split('\n')
      .filter((line) => !/^\s*#/.test(line))
      .join('\n')
    const iBuilder = code.indexOf('electron-builder')
    if (iBuilder < 0) continue
    if (/pnpm run (dist|pack)\b/.test(code)) {
      if (!code.includes('check-asar-config.cjs')) offenders.push(`${f}：缺 check-asar-config.cjs`)
      continue
    }
    const iPrune = code.indexOf('pnpm run prune')
    if (iPrune < 0) offenders.push(`${f}：缺 pnpm run prune`)
    else if (iPrune > iBuilder) offenders.push(`${f}：prune 位置在 electron-builder 之后`)
    if (!code.includes('check-asar-config.cjs')) offenders.push(`${f}：缺 check-asar-config.cjs`)
  }
  assert.deepEqual(
    offenders,
    [],
    `这些 workflow 出 electron 包但没把探针挡在 asar 外：\n  ${offenders.join('\n  ')}`
  )
})

test('prune 脚本覆盖探针 preload 且缺 index.js 时报错', () => {
  const prune = read('scripts/prune-probe.cjs')
  assert.ok(prune.includes('tabs-probe-preload.js'), 'prune 必须删除页签探针 preload')
  assert.ok(prune.includes("startsWith('probe')"), 'prune 必须按前缀删除 out/main/probe*')
  assert.ok(prune.includes('out/main/index.js'), 'prune 在未构建时应显式失败')
})
