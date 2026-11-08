/**
 * 出包前剔除开发期工具（2026-11-07 架构审查 H-4）。
 *
 * 探针（`probe*.js`，约 2300 行）与页签探针 preload 只在 `--probe` / `--smoke` 时用，
 * 运行期已改为 `await import(...)` 条件加载（见 `src/main/index.ts` 顶部说明），
 * 但编译产物仍会落在 `out/` 并被 `electron-builder.yml` 的 `files: out/**\/*` 打进 asar。
 * 这里在 `stage` 之后、`electron-builder` 之前删掉它们；`check-asar-config.cjs` 会
 * **反向断言**包内不含这些文件（删漏了立刻红灯）。
 *
 * 注意：本脚本只作用于 `out/`（构建产物）。之后跑 `pnpm run build` 会重新生成，
 * 因此 `pnpm run probe` 不受影响；打包脚本的顺序必须是 build → stage → prune → builder。
 */
const { existsSync, readdirSync, rmSync } = require('node:fs')
const { join } = require('node:path')

const appDir = join(__dirname, '..')
const outDir = join(appDir, 'out')

if (!existsSync(join(outDir, 'main', 'index.js'))) {
  console.error('[prune] ✗ out/main/index.js 不存在 —— 先跑 pnpm run build')
  process.exit(1)
}

let removed = 0
const mainDir = join(outDir, 'main')
for (const name of readdirSync(mainDir)) {
  // 探针相关产物：probe.js / probe-single.js / probe-tabs.js / probe-server.js / probe-infra.js
  // 以及 map；若将来拆成 probe/ 子目录也一并覆盖。
  if (name.startsWith('probe')) {
    rmSync(join(mainDir, name), { recursive: true, force: true })
    console.log(`[prune] 移除 out/main/${name}`)
    removed++
  }
}

for (const name of ['tabs-probe-preload.js', 'tabs-probe-preload.js.map']) {
  const p = join(outDir, 'renderer', name)
  if (existsSync(p)) {
    rmSync(p, { force: true })
    console.log(`[prune] 移除 out/renderer/${name}`)
    removed++
  }
}

if (removed === 0) {
  // 不静默：没有可删项说明构建产物命名变了，护栏可能已经失效。
  console.warn('[prune] ⚠️ 没有匹配到任何探针产物，请核对 out/main 的命名')
}
console.log(`[prune] 完成：移除 ${removed} 项开发期产物`)
