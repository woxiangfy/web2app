#!/usr/bin/env node
/**
 * 本地打包驱动（Node 版，**零第三方依赖**）：读一份配置 JSON，在本机跑出与 CI 同语义的产物。
 *
 * 运行：`node scripts/pack-local.mjs --config tests/test2.json --target electron-all`
 * 要求：Node ≥ 18（用到内置 `fetch` 与 `AbortSignal.timeout`）。
 *
 * ---------------------------------------------------------------------------
 * ## 为什么是 Node，而不是 Python
 *
 * 打包配置的**唯一真源**是 `.github/scripts/parse-packaging-config.py`，CI 的三条流水线都调它。
 * 本文件只保留入口与 CLI；实现拆在同目录 `packlib/` 子目录：
 *   · constants.mjs / pycompat.mjs / parser.mjs —— 契约常量、Python 语义复刻、配置解析；
 *   · util.mjs / driver-electron.mjs / driver-tauri.mjs —— 进程/文件工具与两壳打包分支；
 *   · selftest.mjs / parity.mjs —— 契约自测与对等性比对，两模式共用的 payload 真源在 selftest.mjs。
 *
 * 本地原本也复用它（`scripts/pack-local.py`），但 Python **不是**「拿着这个仓库就一定有」的依赖：
 *
 *   · 在 Windows 上实测（2026-09-24）：PATH 里的 `python` 来自会话注入的
 *     `~/.workbuddy/binaries/python/3.13.12/`，系统里既没有 `py.exe` launcher，
 *    也没有 `%LOCALAPPDATA%\Programs\Python\` / `Program Files\Python*` 任何一个安装
 *     ⇒ 出了这个会话就 `command not found`。
 *   · 而打 **Electron 包必然已经有 node** —— `electron-builder` 本身就是 node 程序，
 *     `pnpm run sidecar/build/stage` 也全是 node 脚本。**没有 node 就打不出 Electron 包**，
 *     所以 node 是「打这个包的必要条件」，Python 不是。
 *
 * ⇒ 本文件是那份 Python 解析器的 Node 复刻（解析部分在 packlib/parser.mjs），外加本地打包的驱动部分。
 *
 * ## ⚠️⚠️ 代价：解析逻辑从此有两份实现 ⇒ 必须用 `--parity` 钉住
 *
 * 这是本文件存在的最大风险，也是唯一需要长期维护成本的地方：
 *
 *   · **改任何一侧的解析语义，都必须改另一侧，并跑 `node scripts/pack-local.mjs --parity`。**
 *   · `--parity` 会在系统里找到 Python（`python` / `python3` / `py -3` / 会话托管路径）后，
 *     对一整批 payload 跑**两侧**解析器，逐项比对：退出码 / `::warning::` 行 / `::error::` 文案 /
 *     `--config-out` 写出的 config.json 字节。任一项不同即失败。
 *   · 找不到 Python 时 `--parity` 会明确报「跳过」而不是假装通过（本地无 Python 是常态，
 *     所以它同时给出「把本文件的核心语义与解析器对照」的人工路径）。
 *   · CI 里永远有 Python ⇒ `--parity` 可以在 CI 上真跑，漂移会在那里现形。
 *
 * ## 与 Python 版**有意不同**的地方（都记录在案）
 *
 * 1. **不落 env 文件，直接把 `env` 传给子进程** ⇒ 天然绕开本地三个坑：
 *    `--env-out` 在 Windows 上是 **CRLF**（`os.linesep` 翻译）会把值尾部粘上 `\r`；
 *    `PROXY_BYPASS` 含 `;` 会被 shell 拆断；`public_key` 多行时是 GitHub heredoc（bash 不认）；
 *    而且它还是 **append** 模式、不传时**静默跳过**。
 * 2. **写 config.json 一律 LF 换行**（`fs.writeFileSync` 不做换行转换）—— 与 CI（Linux）的
 *    Python 产物一致。⚠️ 反过来，Python 的 `Path.write_text()` 在 Windows 上会把 `\n` 翻成
 *    `\r\n`，所以本地跑 Python 版反而**与 CI 不一致**。`--parity` 比对时会归一化行尾并显式报告。
 * 3. **URL 读取用 `fetch`**，非 2xx 直接给出可读报错（Python 的 `urlopen` 在这里会抛 HTTPError）。
 * 4. **不实现 Android keystore 处理**：Android 本机根本做不了（`ring` 的 build script 需要
 *    `aarch64-linux-android-clang` 即 NDK），那部分只走 CI。
 *
 * ## ⚠️⚠️ tracked 文件会被改写，一律备份 + 还原
 *
 * 本地打包必须动两个**被版本控制**的文件（CI 是工作区一次性，本地会脏工作区）：
 *   · `webview-app/tauri.conf.json` —— 写入 `productName = app_name`、`mainBinaryName = web2app`；
 *   · `webview-app/icons/*` —— `icon_url` 非空时 `cargo tauri icon` 会重新生成整套图标。
 *
 * 另外 `electron-app/config.json` 虽**未被版本控制**，但往往是开发者本地调试用的那一份
 * （如 baidu 配置），直接覆盖等于静默毁掉本地状态 ⇒ 同样备份还原。
 *
 * ⚠️ **还原的前提是进程活着走到 `finally`。** 实测踩过：构建中途被 kill / Ctrl-C ⇒
 * 还原不执行 ⇒ 本地配置留在生成态。所以启动时就把**备份目录路径打印出来**，
 * 中断后照着手工拷回即可。
 *
 * ## ⚠️⚠️ 「JSON 定义行为」—— 本地与 CI 的同一条判据必须同源
 *
 * 配置里写了什么就产出什么；**不允许有「JSON 之外还得有人再做一步」的隐式前提**。
 * 本脚本违反过两条，都已修（记在这里防回归）：
 *
 * 1. **图标**：判据是 `icon_url` **非空**（CI 那步就是 `if [ -z "$ICON_URL" ] … exit 0`），
 *    而本地早先额外要求 `--icon` 开关 ⇒ 同一份 JSON 在 CI 带图标、在本地不带。
 *    现在只看 JSON；`--no-icon` 仅作逃逸口。
 * 2. **输出目录**：electron-builder 打包前要清空 appOutDir，清不掉就炸在 emptyDir，
 *    早先的解法是「让用户自己去删 release/」。现在改为：先让它自己清（CI 走的也是这条路），
 *    **只在它因「删不掉」而失败时**才自动换一个全新目录重试一次
 *    ⇒ 始终能出包、始终不需要人工介入。
 *    ⚠️ 不要改成「脚本先自己清一遍」：本机的删除有配额（50 次/轮），清一次上千个文件会把配额
 *      烧光，害得 electron-builder 连自己的中间产物都删不掉 ⇒「产物已出、构建却报失败」。已踩。
 *
 * ## 用法
 *
 *     node scripts/pack-local.mjs --config tests/test2.json --target electron-all --dry-run
 *     node scripts/pack-local.mjs --config tests/test2.json --target electron-all
 *     node scripts/pack-local.mjs --config tests/test1.json --target tauri-msi      # 需先装 tauri-cli
 *     node scripts/pack-local.mjs --json '{"app_url":"https://example.com"}' --target electron-zip
 *     node scripts/pack-local.mjs --selftest     # 契约自测（不需要 Python）
 *     node scripts/pack-local.mjs --parity       # 与 Python 解析器逐项比对（需要 Python）
 */

import { readFileSync } from 'node:fs'
import { ConfigError, FatalError, emit } from './packlib/diagnostic.mjs'
import { DEFAULTS, SKIP_ENV, TARGETS } from './packlib/constants.mjs'
import { buildMerged, display, parseJson, readRaw } from './packlib/parser.mjs'
import { Restorer, safeName } from './packlib/util.mjs'
import { doElectron } from './packlib/driver-electron.mjs'
import { doTauri } from './packlib/driver-tauri.mjs'
import { selftest } from './packlib/selftest.mjs'
import { parity } from './packlib/parity.mjs'

// ===========================================================================
// CLI
// ===========================================================================
const USAGE = `本地打包（Node 版）—— 读一份配置 JSON，产出与 CI 同语义的产物。

用法：
  node scripts/pack-local.mjs --config <file> --target <t> [选项]
  node scripts/pack-local.mjs --json '<JSON>' --target <t> [选项]
  node scripts/pack-local.mjs --url <URL>    --target <t> [选项]
  node scripts/pack-local.mjs --selftest          契约自测（不需要 Python）
  node scripts/pack-local.mjs --parity            与 Python 解析器逐项比对（需要 Python）

target：${TARGETS.join(' | ')}

选项：
  --no-icon       显式跳过 icon_url。**默认一切按 JSON 来**：icon_url 非空就下载并应用（与 CI 一致）
                  ⚠️ 应用图标会改写 tracked 的 icons/，脚本会备份并在打包后还原
  --out DIR       产物输出目录（本地测试打包产物，已被 /tests/ 忽略、不入库）：
                  Electron 默认 tests/pack/；Tauri 默认 tests/pack/tauri/（两壳绿色版同名，必须分子目录）
                  相对路径以**各自壳工程目录**为基准（electron-app/ 或 webview-app/）；
                  要在根 tests/ 下换目录写 --out ../tests/<名字>
                  ⚠️ Electron 默认目录清不掉时（被占用 / 环境禁删）脚本会**自动**换
                  tests/pack-local-<时间戳>/ 重试一次，不需要人工删任何目录
  --keep-config   打包后**不还原**被改写的文件（便于反复迭代，记得自己 git checkout）
  --dry-run       只打印将执行的命令，不动任何文件
  -h, --help      显示本帮助
`

function parseArgs(argv) {
  const a = {
    config: null,
    json: null,
    url: null,
    target: null,
    noIcon: false,
    out: null,
    keepConfig: false,
    dryRun: false,
    selftest: false,
    parity: false,
    help: false,
  }
  for (let i = 0; i < argv.length; i++) {
    const k = argv[i]
    const next = () => {
      if (i + 1 >= argv.length) throw new FatalError(`参数 ${k} 缺少取值`)
      return argv[++i]
    }
    if (k === '--config') a.config = next()
    else if (k === '--json') a.json = next()
    else if (k === '--url') a.url = next()
    else if (k === '--target') a.target = next()
    else if (k === '--out') a.out = next()
    else if (k === '--no-icon') a.noIcon = true
    else if (k === '--keep-config') a.keepConfig = true
    else if (k === '--dry-run') a.dryRun = true
    else if (k === '--selftest') a.selftest = true
    else if (k === '--parity') a.parity = true
    else if (k === '-h' || k === '--help') a.help = true
    else throw new FatalError(`未知参数：${k}（用 --help 看用法）`)
  }
  return a
}

async function main(argv) {
  const args = parseArgs(argv)
  if (args.help) {
    process.stdout.write(USAGE)
    return 0
  }
  if (args.selftest) return selftest()
  if (args.parity) return parity()

  const srcCount = [args.config, args.json, args.url].filter(Boolean).length
  if (srcCount === 0) throw new FatalError('必须给出 --config / --json / --url 之一（用 --help 看用法）')
  if (srcCount > 1) throw new FatalError('--config / --json / --url 三者互斥，只能给一个')
  if (!args.target) throw new FatalError('缺少 --target（用 --help 看可选值）')
  if (!TARGETS.includes(args.target)) {
    throw new FatalError(`未知 target：${args.target}（可选：${TARGETS.join(' | ')}）`)
  }

  // ---- 1) 解析配置（不落任何 env 文件）----
  let raw
  try {
    raw = args.json || args.url ? await readRaw(args.json, args.url) : readFileSync(args.config, 'utf8')
  } catch (e) {
    if (e instanceof ConfigError) throw e
    throw new FatalError(`读不到配置文件：${e.message}`)
  }
  const data = parseJson(raw)
  const merged = buildMerged(data)

  const env = { ...process.env }
  // ⚠️ 必须剥掉 `ELECTRON_RUN_AS_NODE`：本机 shell 里它是 `1`（宿主注入，实测确认）。
  //    它对**普通 node 无害**（所以本脚本自己能跑），但子进程里任何一个 **electron 可执行文件**
  //    都会退化成「node 模式」——`require('electron')` 只拿到路径字符串、`app` 是 `undefined`，
  //    表现为莫名其妙的 `Cannot read properties of undefined`。打包链里有被 electron 拉起的环节，
  //    统一清掉最稳（项目自带 `electron-app/scripts/launch.cjs` 也处理了同一件事）。
  delete env.ELECTRON_RUN_AS_NODE
  for (const [key, value] of Object.entries(merged)) {
    if (SKIP_ENV.has(key)) continue // 签名类字段不进环境变量（与解析器 write_env 同源）
    env[key.toUpperCase()] = value
  }

  const name = safeName(merged.app_name)
  emit('='.repeat(68))
  emit(`本地打包：target = ${args.target}｜安装名 = ${name}`)
  emit('='.repeat(68))
  for (const key of Object.keys(DEFAULTS)) {
    emit(`  ${key.toUpperCase().padEnd(22)} ${display(key, merged[key])}`)
  }

  const rr = new Restorer()
  try {
    if (args.target.startsWith('electron')) await doElectron(args, merged, env, name, rr)
    else doTauri(args, merged, env, name, rr)
  } finally {
    rr.restore(args.keepConfig)
  }

  emit('\n✓ 完成' + (args.dryRun ? '（dry-run，未动任何文件）' : ''))
  return 0
}

main(process.argv.slice(2))
  .then((code) => {
    process.exitCode = code
  })
  .catch((e) => {
    if (e instanceof ConfigError) {
      process.stderr.write(`::error::${e.message}\n`)
      process.stderr.write('  字段表与多端行为见 docs/packaging-config.md\n')
    } else if (e instanceof FatalError) {
      process.stderr.write(`✗ ${e.message}\n`)
    } else {
      process.stderr.write(`✗ 未预期的错误：${e.stack || e.message}\n`)
    }
    process.exitCode = 1
  })
