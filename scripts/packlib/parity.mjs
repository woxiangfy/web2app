/**
 * 与 Python 解析器的对等性比对（需要 Python）。
 */
import { spawnSync } from 'node:child_process'
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { ConfigError, captureOutput, emit } from './diagnostic.mjs'
import { PARSER_PY, rel } from './paths.mjs'
import { buildMerged, toConfigJson } from './parser.mjs'
import { PARITY_PAYLOADS } from './selftest.mjs'

function findPython() {
  const candidates = [['python', []], ['python3', []], ['py', ['-3']]]
  // 会话托管路径（本机实测系统里没有真 Python，PATH 里那份是注入的）
  const base = join(process.env.USERPROFILE || '', '.workbuddy', 'binaries', 'python', 'versions')
  if (existsSync(base)) {
    for (const v of ['3.13.12', '3.12.11', '3.11.9']) {
      const exe = join(base, v, process.platform === 'win32' ? 'python.exe' : 'bin/python3')
      if (existsSync(exe)) candidates.push([exe, []])
    }
  }
  for (const [cmd, pre] of candidates) {
    const r = spawnSync(cmd, [...pre, '--version'], { stdio: 'ignore' })
    if (!r.error && r.status === 0) return { cmd, pre }
  }
  return null
}

export function parity() {
  const py = findPython()
  if (!py) {
    emit('未能找到 Python —— `--parity` 需要本机有 Python 才能与解析器比对。')
    emit(`  要找的解析器：${rel(PARSER_PY)}`)
    emit('  这不影响本脚本正常工作（本地打包不需要 Python）；只是无法在此机上证明两侧等价。')
    emit('  CI 上一定有 Python ⇒ 该比对在 CI 里可以真跑。')
    return 2
  }
  emit(`使用 Python：${py.cmd} ${py.pre.join(' ')}`)
  const tmp = mkdtempSync(join(tmpdir(), 'w2a-parity-'))
  let pass = 0
  const failures = []
  // 「Python 侧进程压根没起来」与「语义不一致」是两回事，分开记：前者是环境限制，不该被误读成
  // 解析器对不上，也不能被当成通过。
  const unverified = []
  const eolOnlyCases = []
  const normEol = (s) => String(s).replace(/\r\n/g, '\n')

  for (const [i, payload] of PARITY_PAYLOADS.entries()) {
    const payloadJson = JSON.stringify(payload)
    const cfgPy = join(tmp, `py-${i}.json`)
    const cfgJs = join(tmp, `js-${i}.json`)

    // ---- Python 侧 ----
    const pr = spawnSync(py.cmd, [...py.pre, PARSER_PY, '--json', payloadJson, '--config-out', cfgPy], {
      encoding: 'utf8',
      env: { ...process.env },
    })
    // ⚠️ spawn 本身失败时 `status` 是 `null`、stdout/stderr 全空 —— 只报一句「退出码 null」
    //    让人完全摸不着头脑。本机实测（2026-09-24）：PATH 上那份 `python` 是会话注入的
    //    reparse point，从 Node 里 spawn 它**恒定** EBUSY；`findPython()` 兜底到绝对路径才跑得起来，
    //    但连续 spawn 到第 ~37 次又开始 EBUSY（句柄 / 配额类限制，与解析器无关）。
    if (pr.error) {
      unverified.push(
        `payload #${i} ${payloadJson}：Python 侧进程未能启动 —— ` +
          `${pr.error.code || ''} ${pr.error.message}`,
      )
      continue
    }
    const pyOut = (pr.stdout || '') + (pr.stderr || '')
    const pyWarn = pyOut.split(/\r?\n/).filter((l) => l.startsWith('::warning::'))
    // ⚠️ ConfigError 的文案可能是**多行**（reject_unknown_keys 会逐字段列一行），
    //    只取 `::error::` 那一行的匹配会把它截断 ⇒ 必须一路取到结尾。
    const pyErrMatch = pyOut.match(/::error::([\s\S]*)$/)
    const pyErr = pyErrMatch ? pyErrMatch[1].trim() : ''
    const pyCode = pr.status
    const pyCfg = existsSync(cfgPy) ? readFileSync(cfgPy, 'utf8') : null

    // ---- Node 侧（同一次调用里同时拿输出与产物）----
    let jsMerged = null
    const { lines, error } = captureOutput(() => {
      jsMerged = buildMerged(payload)
    })
    const jsWarnings = lines.filter((l) => l.startsWith('::warning::'))
    const jsCode = error ? 1 : 0
    const jsErr = error ? String(error.message).trim() : ''
    const jsCfg = error ? null : JSON.stringify(toConfigJson(jsMerged), null, 2) + '\n'
    if (error && !(error instanceof ConfigError)) throw error

    // ---- 比对 ----
    // ⚠️ 一律**归一化行尾**再比：Python 在本机是 Windows，`print()` 与 `Path.write_text()`
    //    都会把 `\n` 翻成 `\r\n`（`os.linesep` 翻译）；而 CI 是 Linux，两侧都是 LF。
    //    ⇒ 行尾差异是「本机 Python 的产物形态」，不是语义差异，单独汇总报告。
    const rawDiffs = []
    const eolDiffs = []
    const cmp = (label, a, b) => {
      if (a === b) return
      if (normEol(a) === normEol(b)) eolDiffs.push(label)
      else rawDiffs.push(`${label}：\n    Python: ${JSON.stringify(a)}\n    Node  : ${JSON.stringify(b)}`)
    }
    if (pyCode !== jsCode) rawDiffs.push(`退出码：Python=${pyCode} Node=${jsCode}`)
    cmp('错误文案', pyErr, jsErr)
    cmp('warning 序列', pyWarn.join('\n'), jsWarnings.join('\n'))
    cmp('config.json 内容', pyCfg === null ? '<无>' : pyCfg, jsCfg === null ? '<无>' : jsCfg)

    if (rawDiffs.length) failures.push(`payload #${i} ${payloadJson}\n  ${rawDiffs.join('\n  ')}`)
    else {
      pass++
      if (eolDiffs.length) eolOnlyCases.push(i)
    }
  }

  rmSync(tmp, { recursive: true, force: true })
  if (eolOnlyCases.length) {
    emit(
      `· ${eolOnlyCases.length} 例**仅换行符**不同（本机 Python 是 Windows 版，其 print() / Path.write_text()\n` +
        '  会把 \\n 翻成 \\r\\n；本脚本固定写 LF）。⚠️ CI 是 Linux ⇒ 两侧都是 LF，这条差异不会出现。\n' +
        `  涉及用例：${eolOnlyCases.map((i) => '#' + i).join(', ')}`,
    )
  }
  if (unverified.length) {
    emit(
      `\n⚠️ ${unverified.length} 例**未能验证** —— Python 侧进程起不来，属本机环境限制，不是语义差异：`,
    )
    for (const u of unverified) emit('  · ' + u)
    emit('  这不算通过：换台机器或交给 CI（Linux，无此限制）才能完整比对。')
  }
  if (failures.length) {
    emit(`\n✗ 对等性比对失败：${failures.length} / ${PARITY_PAYLOADS.length} 例不一致`)
    for (const f of failures) emit('\n' + f)
    return 1
  }
  if (unverified.length) return 2
  emit(`\n✓ 对等性比对通过：${pass} / ${PARITY_PAYLOADS.length} 例与 Python 解析器语义一致`)
  return 0
}
