#!/usr/bin/env node
/**
 * 注入脚本 / 内联脚本语法门禁 —— **两壳共用真源**，零第三方依赖。
 *
 * ## 为什么需要
 *
 * `webview-app/src/*.js`（`titlebar.js` / `loading.js` / …）与 `dist/settings.html`、
 * `shell.html` 里的内联脚本是**两壳共用**的（Electron 打包时由
 * `electron-app/scripts/stage-assets.cjs` 复制过去）。它们的语法错误**只在运行期现形**：
 * Rust 侧只是 `include_str!` 后 `initialization_script()` 注入，编译期一个字都不检查；
 * 探针要拉起 GUI 才能在 CI 上发现，Linux 的 lint 作业完全够不着。
 * 结果就是「一个少写的括号 ⇒ 自绘标题栏整体消失 / 设置页白屏」，而且排查方向会被
 * 带到「注入没生效」上（2026-09-16 `splash.html` 的注释里就记过同类事故）。
 *
 * 所以这里在**几秒内**用 Node 自己的解析器（`vm.Script` 只编译不执行）把三类脚本过一遍：
 *
 * 1. `webview-app/src/*.js` —— 逐个整文件检查；
 * 2. `webview-app/src/mobilebar/*.js` —— **分片**，单独拿出来都不是合法脚本（`core.js` 与
 *    `mount.js` 就是），必须按 `lib.rs` 里 `include_str!` 的**真实顺序**拼成同一个 IIFE 再检查。
 *    ⚠️ 顺序**从 lib.rs 解析**，不在这里手写第二份 —— 手写必然漂移，而漂移后的拼接能过语法
 *    检查、却在真机上语义错乱；
 * 3. `webview-app/src/*.html` + `webview-app/dist/*.html` —— 内联 `<script>` 块
 *    （先剥 HTML 注释：`splash.html` 的头注释里就写着 `<script>` 字样，不剥会把注释当脚本）。
 *
 * 另外顺带 `node --check` 一遍 CI 自己依赖的 Node 脚本（根 `scripts/**`、
 * `electron-app/scripts/**`）—— 它们坏了是「工作流跑到一半才炸」的那一类。
 *
 * ## 覆盖完整性
 *
 * 发现结果**必须非空**，且 `webview-app/src/` 下每个 `.js` 都必须被上面某条规则覆盖到；
 * 新增目录没纳入时直接红灯，而不是静默跳过（否则以后加一个 `src/foo/bar.js` 就等于没有门禁）。
 *
 * 用法：
 *     node scripts/check-js-syntax.mjs            # 真检查
 *     node scripts/check-js-syntax.mjs --selftest # 反向对照：确认检测器本身能报错
 */

import { execFileSync } from 'node:child_process'
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { dirname, join, relative, sep } from 'node:path'
import { fileURLToPath } from 'node:url'
import vm from 'node:vm'

/** 仓库根（本文件在 `<root>/scripts/` 下）。 */
const ROOT = dirname(dirname(fileURLToPath(import.meta.url)))

const posix = (p) => p.split(sep).join('/')

/** 只编译不执行：语法错返回错误信息，正常返回 null。 */
function compileError(code, label) {
  try {
    // eslint-disable-next-line no-new
    new vm.Script(code, { filename: label })
    return null
  } catch (e) {
    return `${e && e.message ? e.message : String(e)}`
  }
}

/** 从 lib.rs 里按出现顺序取 `include_str!("mobilebar/<n>.js")`。 */
export function mobilebarOrder(libRs) {
  const re = /include_str!\(\s*"mobilebar\/([\w.-]+\.js)"\s*\)/g
  const out = []
  let m
  while ((m = re.exec(libRs)) !== null) {
    if (!out.includes(m[1])) out.push(m[1])
  }
  return out
}

/** 剥 HTML 注释后取出内联 `<script>` 块（跳过 `src=` 外链与 `type="module"`）。 */
export function inlineScripts(html) {
  const stripped = html.replace(/<!--[\s\S]*?-->/g, '')
  const re = /<script\b([^>]*)>([\s\S]*?)<\/script>/gi
  const out = []
  let m
  let i = 0
  while ((m = re.exec(stripped)) !== null) {
    i += 1
    const attrs = m[1] || ''
    if (/\bsrc\s*=/.test(attrs)) continue
    if (/type\s*=\s*["']module["']/.test(attrs)) continue
    out.push({ index: i, code: m[2] })
  }
  return out
}

/** `files` 里没有出现在 `covered` 中的项（覆盖完整性判定，selftest 也直接用它）。 */
export function uncovered(files, ...covered) {
  const seen = new Set(covered.flat())
  return files.filter((f) => !seen.has(f))
}

function walk(dir, exts, acc = []) {
  for (const name of readdirSync(dir)) {
    if (name === 'node_modules' || name === '.git' || name === 'out' || name === 'dist-tmp') continue
    const p = join(dir, name)
    if (statSync(p).isDirectory()) walk(p, exts, acc)
    else if (exts.some((e) => name.endsWith(e))) acc.push(p)
  }
  return acc
}

function collect() {
  const wvSrc = join(ROOT, 'webview-app', 'src')
  const singles = readdirSync(wvSrc)
    .filter((n) => n.endsWith('.js'))
    .sort()
    .map((n) => join(wvSrc, n))

  const libRs = readFileSync(join(wvSrc, 'lib.rs'), 'utf8')
  const order = mobilebarOrder(libRs).map((n) => join(wvSrc, 'mobilebar', n))

  const htmls = [
    ...readdirSync(wvSrc).filter((n) => n.endsWith('.html')).map((n) => join(wvSrc, n)),
    ...readdirSync(join(ROOT, 'webview-app', 'dist'))
      .filter((n) => n.endsWith('.html'))
      .map((n) => join(ROOT, 'webview-app', 'dist', n)),
  ].sort()

  const nodeScripts = [
    ...walk(join(ROOT, 'scripts'), ['.mjs', '.cjs']),
    ...walk(join(ROOT, 'electron-app', 'scripts'), ['.mjs', '.cjs']),
  ].sort()

  const allJs = walk(wvSrc, ['.js'])
  return { wvSrc, singles, order, htmls, nodeScripts, allJs }
}

function run() {
  const { wvSrc, singles, order, htmls, nodeScripts, allJs } = collect()
  const errors = []
  let checked = 0

  // 0) 发现结果必须非空 —— 规则写错时「一条都没检查」也显示绿色是假绿。
  for (const [what, list] of [
    ['webview-app/src/*.js', singles],
    ['webview-app/src/mobilebar/*.js（lib.rs 顺序）', order],
    ['webview-app/**/*.html 内联脚本', htmls],
    ['scripts/** 与 electron-app/scripts/**', nodeScripts],
  ]) {
    if (list.length === 0) errors.push(`按规则没发现任何 ${what} —— 发现规则可能写错了（空集假绿）`)
  }

  // 1) 顶层注入脚本：逐个整文件
  for (const f of singles) {
    checked += 1
    const err = compileError(readFileSync(f, 'utf8'), posix(relative(ROOT, f)))
    if (err) errors.push(`${posix(relative(ROOT, f))}：${err}`)
  }

  // 2) 移动端底栏分片：按 lib.rs 顺序拼成同一个 IIFE
  if (order.length > 0) {
    checked += 1
    const concat = order.map((f) => readFileSync(f, 'utf8')).join('\n;\n')
    const err = compileError(concat, 'webview-app/src/mobilebar/(concat)')
    if (err) {
      errors.push(`mobilebar 分片拼接后语法错误（顺序取自 lib.rs）：${err}`)
    } else {
      console.log(`── mobilebar 分片拼接 ${order.length} 片 → 同一 IIFE ✓`)
    }
  }

  // 2b) mobilebar/ 下每个 .js 都必须在 lib.rs 的顺序表里（新增分片忘了挂 = 白写）
  const barFiles = readdirSync(join(wvSrc, 'mobilebar')).filter((n) => n.endsWith('.js'))
  for (const n of uncovered(barFiles, order.map((f) => f.split(/[\\/]/).pop()))) {
    errors.push(`webview-app/src/mobilebar/${n} 没有被 lib.rs 的 include_str! 拼进底栏（新增分片必须按序挂上）`)
  }

  // 3) HTML 内联脚本
  for (const f of htmls) {
    for (const { index, code } of inlineScripts(readFileSync(f, 'utf8'))) {
      checked += 1
      const label = `${posix(relative(ROOT, f))}#script${index}`
      const err = compileError(code, label)
      if (err) errors.push(`${label}：${err}`)
    }
  }

  // 4) CI 依赖的 Node 脚本（.mjs 有 import、.cjs 有 require ⇒ 只能交给 node 自己解析）
  for (const f of nodeScripts) {
    checked += 1
    const rel = posix(relative(ROOT, f))
    try {
      execFileSync(process.execPath, ['--check', f], { stdio: 'pipe' })
    } catch (e) {
      const out = `${e.stdout || ''}${e.stderr || ''}`.toString().trim().split('\n').slice(0, 3).join(' | ')
      errors.push(`${rel}：node --check 失败：${out.slice(0, 300)}`)
    }
  }

  // 5) 覆盖完整性：src 下每个 .js 必须被 1 或 2 覆盖
  const covered = [...singles, ...order]
  for (const f of uncovered(allJs, covered)) {
    errors.push(
      `${posix(relative(ROOT, f))} 未纳入语法门禁（新增子目录请在 check-js-syntax.mjs::collect() 里纳入，` +
        `或放进 mobilebar/ 并由 lib.rs 按序 include_str!）`
    )
  }

  if (errors.length) {
    console.error(`注入脚本语法门禁未通过（${errors.length} 项）：`)
    for (const e of errors) console.error(`  ✗ ${e}`)
    return 1
  }
  console.log(
    `注入脚本语法门禁通过 ✓（${checked} 段：${singles.length} 个整文件 + ` +
      `${order.length} 片 mobilebar 拼接 + ${htmls.length} 个 HTML 的内联块 + ` +
      `${nodeScripts.length} 个 Node 脚本）`
  )
  return 0
}

/** 反向对照：检测器本身必须能报错，否则「全绿」毫无意义。 */
function selftest() {
  const cases = []
  const expect = (name, got, want) => {
    const ok = JSON.stringify(got) === JSON.stringify(want)
    cases.push({ name, ok, got, want })
  }

  expect('坏脚本必须报错', compileError('(function(){', 'x') !== null, true)
  expect('好脚本不得误报', compileError('(function(){ })();', 'x'), null)
  expect(
    'lib.rs 顺序按出现顺序提取',
    mobilebarOrder('a include_str!("mobilebar/b.js") b include_str!("mobilebar/a.js")'),
    ['b.js', 'a.js']
  )
  expect('lib.rs 里非 mobilebar 的 include_str 不算分片', mobilebarOrder('include_str!("titlebar.js")'), [])
  expect(
    'HTML 注释里的 <script> 不参与检查',
    inlineScripts('<!-- <script>BAD(((</script> --><script>ok()</script>').map((s) => s.code),
    ['ok()']
  )
  expect('外链 script 跳过', inlineScripts('<script src="a.js"></script>').length, 0)
  expect(
    'type=module 跳过（vm 不能编译 ESM）',
    inlineScripts('<script type="module">import a from "b"</script>').length,
    0
  )
  expect('覆盖完整性：漏项必须被抓到', uncovered(['a.js', 'b.js'], ['a.js']), ['b.js'])

  const bad = cases.filter((c) => !c.ok)
  if (bad.length) {
    console.error('check-js-syntax --selftest 失败：')
    for (const c of bad) console.error(`  ✗ ${c.name}：got=${JSON.stringify(c.got)} want=${JSON.stringify(c.want)}`)
    return 1
  }
  console.log(`check-js-syntax --selftest 通过 ✓（${cases.length} 条反向对照）`)
  return 0
}

process.exit(process.argv.includes('--selftest') ? selftest() : run())
