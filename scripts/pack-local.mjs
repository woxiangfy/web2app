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
 * 本地原本也复用它（`scripts/pack-local.py`），但 Python **不是**「拿着这个仓库就一定有」的依赖：
 *
 *   · 在 Windows 上实测（2026-09-24）：PATH 里的 `python` 来自会话注入的
 *     `~/.workbuddy/binaries/python/3.13.12/`，系统里既没有 `py.exe` launcher，
 *     也没有 `%LOCALAPPDATA%\Programs\Python\` / `Program Files\Python*` 任何一个安装
 *     ⇒ 出了这个会话就 `command not found`。
 *   · 而打 **Electron 包必然已经有 node** —— `electron-builder` 本身就是 node 程序，
 *     `pnpm run sidecar/build/stage` 也全是 node 脚本。**没有 node 就打不出 Electron 包**，
 *     所以 node 是「打这个包的必要条件」，Python 不是。
 *
 * ⇒ 本文件是那份 Python 解析器的 **Node 复刻**，外加本地打包的驱动部分。
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

import { spawn, spawnSync } from 'node:child_process'
import {
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs'
import { createHash } from 'node:crypto'
import { tmpdir } from 'node:os'
import { dirname, isAbsolute, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const WEBVIEW = join(ROOT, 'webview-app')
const ELECTRON = join(ROOT, 'electron-app')
const PARSER_PY = join(ROOT, '.github', 'scripts', 'parse-packaging-config.py')

const rel = (p) => relative(ROOT, p).replace(/\\/g, '/') || '.'

// ===========================================================================
// 契约常量 —— 与 .github/scripts/parse-packaging-config.py **逐字对应**
// 顺序即打印顺序，也是写进 config.json 的键顺序。改这里必须同步改那边 + docs/packaging-config.md
// ===========================================================================
const DEFAULTS = {
  app_url: '',
  app_name: 'Web2App',
  title: '',
  extra: {},
  icon_url: '',
  proxy_enabled: false,
  proxy_http: '',
  proxy_https: '',
  // ⚠️ 两壳必须一致：webview-app 的 config.rs 编译期默认值、electron 的 DEFAULTS
  //    都用这同一个串。留空 = 不下发 bypass 参数（即空值不覆盖 Chromium 默认）。
  proxy_bypass: 'localhost;127.0.0.1;<-loopback>',
  // 空串在本字段里**是有含义的取值**（= 系统默认程序目录），不是「未设置」。
  data_dir: 'data',
  tabs_enabled: false,
  public_key: '',
  resolve_secret: '',
  debug: false,
  android_keystore_base64: '',
  android_key_alias: '',
  android_keystore_password: '',
  android_key_password: '',
}

const BOOL_KEYS = new Set(['proxy_enabled', 'tabs_enabled', 'debug'])
// 需要保持 JSON 结构（对象/数组/标量）的字段：序列化成单行 JSON 文本进 env。
const JSON_KEYS = new Set(['extra'])
// 签名相关字段不进环境变量：keystore 体积大 / 密码敏感。
const SIGNING_KEYS = new Set([
  'android_keystore_base64',
  'android_key_alias',
  'android_keystore_password',
  'android_key_password',
])
const SKIP_ENV = SIGNING_KEYS

// 已移除的字段 → **可直接照做的改法说明**（只报「不认识这个键」的话，用户还得自己猜怎么改）。
const REMOVED_KEYS = {
  window_title: '该字段**已移除**，请改用 title（语义相同；移除原因是同一概念曾有三个名字）',
  data_dir_default:
    '该字段**已移除**（旧的是布尔，新的是字符串），请改用 data_dir：\n' +
    '       旧值 false ⇒ "data_dir": "data"  （默认：程序所在目录下的 data\\，跟着程序走）\n' +
    '       旧值 true  ⇒ "data_dir": ""      （空串 = 系统默认程序目录，不受程序位置影响）',
}

// Windows 上永远非法的目录名字符。`:` 与 `\` `/` 是路径语法的一部分，不在此列。
const DATA_DIR_BAD = new Set('<>"|?*'.split(''))

// electron-nsis：Windows 安装版（NSIS setup），与绿色版分开 —— 它必须带
// `-c.extraMetadata.distribution=nsis` 标记（见 doElectron），与 zip/portable 不能同次构建。
const TARGETS = ['electron-zip', 'electron-portable', 'electron-nsis', 'electron-all', 'tauri-msi', 'tauri-portable', 'tauri-all']

// ===========================================================================
// Python 语义小工具
// 解析器用 `str()` / `repr()` / `difflib.get_close_matches()` 拼文案与取值，要**逐字**复刻
// 就不能用 JS 的同名直觉（`String(true)` 是 "true"，Python 是 "True"）。
// ===========================================================================

/** Python `type(v).__name__`。⚠️ JSON 不区分 int/float，故 `1.0` 在两侧会被判成不同名字（仅影响文案）。 */
function pyTypeName(v) {
  if (v === null || v === undefined) return 'NoneType'
  if (typeof v === 'boolean') return 'bool'
  if (typeof v === 'number') return Number.isInteger(v) ? 'int' : 'float'
  if (typeof v === 'string') return 'str'
  if (Array.isArray(v)) return 'list'
  if (typeof v === 'object') return 'dict'
  return typeof v
}

/** Python `str(v)`：bool → "True"/"False"，None → "None"，容器走 repr。 */
function pyStr(v) {
  if (v === null || v === undefined) return 'None'
  if (typeof v === 'boolean') return v ? 'True' : 'False'
  if (typeof v === 'string') return v
  if (typeof v === 'number') return String(v)
  return pyRepr(v)
}

/** Python `repr(v)`（够用即可：覆盖字符串/数字/布尔/None/数组/对象）。 */
function pyRepr(v) {
  if (v === null || v === undefined) return 'None'
  if (typeof v === 'boolean') return v ? 'True' : 'False'
  if (typeof v === 'number') return String(v)
  if (typeof v === 'string') return pyStrRepr(v)
  if (Array.isArray(v)) return '[' + v.map(pyRepr).join(', ') + ']'
  if (typeof v === 'object') {
    return '{' + Object.entries(v).map(([k, x]) => pyStrRepr(k) + ': ' + pyRepr(x)).join(', ') + '}'
  }
  return String(v)
}

function pyStrRepr(s) {
  // Python 优先单引号；含单引号且不含双引号时改用双引号。
  const q = s.includes("'") && !s.includes('"') ? '"' : "'"
  let body = ''
  for (const ch of s) {
    if (ch === '\\') body += '\\\\'
    else if (ch === q) body += '\\' + ch
    else if (ch === '\n') body += '\\n'
    else if (ch === '\r') body += '\\r'
    else if (ch === '\t') body += '\\t'
    else body += ch
  }
  return q + body + q
}

/** 最长公共子串（Ratcliff-Obershelp 的核心步骤）。 */
function longestMatch(a, b) {
  let best = { size: 0, aStart: 0, bStart: 0 }
  const prev = new Array(b.length + 1).fill(0)
  for (let i = 0; i < a.length; i++) {
    const cur = new Array(b.length + 1).fill(0)
    for (let j = 0; j < b.length; j++) {
      if (a[i] === b[j]) {
        cur[j + 1] = prev[j] + 1
        if (cur[j + 1] > best.size) best = { size: cur[j + 1], aStart: i + 1 - cur[j + 1], bStart: j + 1 - cur[j + 1] }
      }
    }
    for (let j = 0; j <= b.length; j++) prev[j] = cur[j]
  }
  return best
}

function countMatches(a, b) {
  const m = longestMatch(a, b)
  if (m.size === 0) return 0
  const left = countMatches(a.slice(0, m.aStart), b.slice(0, m.bStart))
  const right = countMatches(a.slice(m.aStart + m.size), b.slice(m.bStart + m.size))
  return m.size + left + right
}

/** `difflib.SequenceMatcher.ratio()` = 2M/T。 */
function ratio(a, b) {
  const total = a.length + b.length
  if (total === 0) return 1
  return (2 * countMatches(a, b)) / total
}

/** `difflib.get_close_matches(word, possibilities, n, cutoff)`：同分时保持原顺序（= heapq.nlargest）。 */
function getCloseMatches(word, possibilities, n = 1, cutoff = 0.5) {
  const scored = []
  for (const p of possibilities) {
    const r = ratio(p, word)
    if (r >= cutoff) scored.push([r, p])
  }
  // Array.prototype.sort 自 ES2019 起稳定 ⇒ 同分时保留 possibilities 的原顺序，与 Python 一致。
  scored.sort((x, y) => y[0] - x[0])
  return scored.slice(0, n).map(([, p]) => p)
}

// ===========================================================================
// 错误与输出
// ===========================================================================
class ConfigError extends Error {}

/** 进程级致命错误（工具缺失、命令失败）—— 与配置错误区分开。 */
class FatalError extends Error {}

// 解析期间会往 stdout 打 `::warning::`（CI 注解）。`--selftest` / `--parity` 需要捕获它们，
// 这里用一个可替换的 sink，而不是到处传一个 logger 参数。
let sink = null
function emit(line) {
  if (sink) sink.push(line)
  else process.stdout.write(line + '\n')
}
/**
 * 捕获 `emit` 的输出，供 `--selftest` / `--parity` 使用。
 *
 * ⚠️ 返回 `{ lines, error }` 而不是「成功才返回」：解析器抛 `ConfigError` **之前**往往已经
 * 打过 warning（如 `data_dir 含非法字符`），比对时那些行也必须拿到。
 */
function captureOutput(fn) {
  const prev = sink
  sink = []
  try {
    fn()
    return { lines: sink, error: null }
  } catch (e) {
    return { lines: sink, error: e }
  } finally {
    sink = prev
  }
}

// ===========================================================================
// 读取
// ===========================================================================
async function readRaw(jsonText, url) {
  let raw = (jsonText || '').trim()
  const u = (url || '').trim()
  if (!raw && u) {
    let res
    try {
      res = await fetch(u, { headers: { 'User-Agent': 'web2app-ci' }, signal: AbortSignal.timeout(30000) })
    } catch (e) {
      throw new ConfigError(`拉取配置 URL 失败：${e.message}`)
    }
    // ⚠️ 与 Python 版有意不同：`urllib.request.urlopen` 对非 2xx 会抛 HTTPError（栈很丑），
    //    这里直接给出可读文案。
    if (!res.ok) throw new ConfigError(`拉取配置 URL 失败：HTTP ${res.status} ${res.statusText}`)
    raw = (await res.text()).trim()
  }
  if (!raw) throw new ConfigError('必须提供 config_json 或 config_url，二者至少填一个')
  return raw
}

function parseJson(raw) {
  let data
  try {
    data = JSON.parse(raw)
  } catch (e) {
    throw new ConfigError(`配置不是合法 JSON：${e.message}`)
  }
  if (data === null || typeof data !== 'object' || Array.isArray(data)) {
    throw new ConfigError(`配置 JSON 必须是一个对象（{...}），当前是 ${pyTypeName(data)}`)
  }
  return data
}

// ===========================================================================
// 校验与规范化
// ===========================================================================

/**
 * 未知字段**硬失败**。
 *
 * 曾经只是 warning 后跳过 —— 后果是「字段名拼错」与「字段值不生效」看起来完全一样（都静默），
 * 用户只能靠猜。硬失败 + 近似名建议是唯一能让这类手误在打包时就现形的做法。
 */
function rejectUnknownKeys(data) {
  const unknown = Object.keys(data).filter((k) => !(k in DEFAULTS))
  if (unknown.length === 0) return
  const lines = []
  for (const k of [...unknown].sort()) {
    if (k in REMOVED_KEYS) {
      lines.push(`  ${k}：${REMOVED_KEYS[k]}`)
      continue
    }
    const near = getCloseMatches(k, Object.keys(DEFAULTS), 1, 0.5)
    const hint = near.length ? `，是否想写 ${near[0]}？` : ''
    lines.push(`  ${k}：未知字段${hint}`)
  }
  throw new ConfigError('配置里有无法识别的字段（拼错即静默失效，故在此硬失败）：\n' + lines.join('\n'))
}

/**
 * 布尔字段只接受真 bool。
 *
 * 旧模板里写的是 `"proxy_enabled": "false"`（字符串）。字符串 `"false"` 是**真值**，
 * 两壳各自用不同的宽容规则去猜它，结果 `"on"` 在 Tauri 是 false、在 Electron 是 true
 * —— 同一份配置两种行为。所以直接拒绝字符串，把歧义消灭在打包期。
 */
function normalizeBool(key, value) {
  if (typeof value === 'boolean') return value
  throw new ConfigError(
    `字段 ${key} 必须是布尔值（true / false，**不带引号**），当前是 ` +
      `${pyTypeName(value)}：${pyRepr(value)}\n` +
      `  正确写法："${key}": true\n` +
      `  错误写法："${key}": "true"   ← 字符串，已不再接受`,
  )
}

/**
 * app_url 支持两种形态，统一出口是**一段可以直接进环境变量的文本**：
 *   · string  → 原样（单个 URL，可含 `user:pass@`）
 *   · array   → JSON 数组文本（一组跳板域名，运行时做 DNS TXT + 验签解密）
 */
function normalizeAppUrl(value, publicKey) {
  if (Array.isArray(value)) {
    const domains = value.map((d) => pyStr(d).trim()).filter((d) => d.length > 0)
    if (domains.length === 0) throw new ConfigError('app_url 为数组时不能为空')
    if (!publicKey.trim()) {
      throw new ConfigError(
        'app_url 为域名数组时必须提供 public_key（Ed25519 公钥）——' +
          '域名模式下地址由 DNS TXT 密文解密得到，没有公钥无法验签',
      )
    }
    return JSON.stringify(domains)
  }

  if (typeof value === 'boolean' || !(typeof value === 'string' || typeof value === 'number')) {
    throw new ConfigError(`app_url 必须是字符串或字符串数组，当前是 ${pyTypeName(value)}`)
  }

  const text = pyStr(value).trim()
  if (!text) throw new ConfigError('缺少必填字段 app_url')

  // 兼容：字符串形态但内容是 JSON 数组文本（`"[\"a.com\"]"`）。这曾是唯一可行的数组写法，
  // 老配置里可能有；仍按数组处理，但提示改用真数组。
  if (text.startsWith('[')) {
    let parsed = null
    try {
      parsed = JSON.parse(text)
    } catch {
      parsed = null
    }
    if (Array.isArray(parsed)) {
      const domains = parsed.map((d) => pyStr(d).trim()).filter((d) => d.length > 0)
      if (domains.length === 0) throw new ConfigError('app_url 的数组内容为空')
      if (!publicKey.trim()) throw new ConfigError('app_url 为域名数组时必须提供 public_key（Ed25519 公钥）')
      emit(
        '::warning::app_url 是「JSON 数组文本」写法，建议直接写真的 JSON 数组：' +
          'app_url: ["a.com","b.com"]（去掉外层引号）',
      )
      return JSON.stringify(domains)
    }
    throw new ConfigError(
      "app_url 以 '[' 开头但不是合法的 JSON 字符串数组；" + '若要用域名数组模式请写 ["a.com","b.com"]',
    )
  }

  if (!/^https?:\/\//i.test(text)) {
    // 不硬失败：本地夹具 / 特殊环境可能用别的 scheme，最终由运行时 Url::parse 判定。
    // 但这条几乎总是手误（漏了 scheme），值得在打包日志里现形。
    emit(
      `::warning::app_url 未以 http:// 或 https:// 开头（${mask(text)}），` +
        '运行时若无法解析会提示「应用配置的网址不合法」',
    )
  }
  return text
}

/** 把 JSON 里手写的字面量 `\n` 还原成真换行，并压掉首尾空白（PEM 多行 / 单行都接受）。 */
function normalizePublicKey(value) {
  const text = value === null || value === undefined ? '' : pyStr(value)
  if (!text) return ''
  return text.replace(/\\n/g, '\n').replace(/\\r/g, '').trim()
}

/** extra 统一序列化为单行 JSON 文本（默认 {} 也要转成 "{}"，供 Rust 侧原样解析）。 */
function normalizeExtra(value) {
  try {
    return JSON.stringify(value)
  } catch (e) {
    throw new ConfigError(`extra 无法序列化为 JSON：${e.message}`)
  }
}

/**
 * 数据目录。**空串在这里是有含义的取值**（= 系统默认程序目录），不是「未设置」。
 *
 * 三种形态（两壳行为一致，见 docs/packaging-config.md §4.3）：
 *   · `""`      → 系统默认程序目录（`%LOCALAPPDATA%\<app_name>`），不受程序位置影响；
 *   · 相对路径   → 程序**自身所在目录**（exe 目录）下的该路径，跟着程序走；
 *   · 绝对路径   → 原样使用。
 *
 * 非字符串一律拒绝：`"data_dir": true` 若被宽容地 str() 成 `"True"`，会静默建出一个名叫
 * `True` 的目录 ——「看起来配置生效了、其实完全不是想要的」是最难查的一类。
 */
function normalizeDataDir(value) {
  if (value === null || value === undefined) return String(DEFAULTS.data_dir)
  if (typeof value !== 'string') {
    throw new ConfigError(
      `字段 data_dir 必须是字符串（目录路径），当前是 ${pyTypeName(value)}：${pyRepr(value)}\n` +
        '  正确写法："data_dir": "data"   ← 默认：程序所在目录下的 data\\\n' +
        '  正确写法："data_dir": ""       ← 空串 = 系统默认程序目录（由 app_name 命名）',
    )
  }
  const text = value.trim()
  if (text.includes('\u0000')) throw new ConfigError('data_dir 不能包含 NUL 字符')
  const bad = [...new Set(text.split('').filter((c) => DATA_DIR_BAD.has(c)))].sort()
  if (bad.length) {
    emit(
      `::warning::data_dir 含 Windows 目录名非法字符 ${bad.join('')}（${text}）` +
        '—— Windows 上创建该目录会失败，届时会自动回退到系统默认目录',
    )
  }
  return text
}

/** 打印时隐去 URL 里的 `user:pass@`。 */
function mask(u) {
  return pyStr(u).replace(/(:\/\/[^\/@\s]+)@/g, '://***@')
}

/** 把用户配置合并进默认值，输出「已规范化的字符串表」。 */
function buildMerged(data) {
  rejectUnknownKeys(data)

  const merged = { ...DEFAULTS }
  // app_url / extra / data_dir 单独取：
  //   · app_url 是唯一允许「字符串或数组」两种 JSON 类型的字段；
  //   · extra 要保持 JSON 结构（先 str() 会丢形态）；
  //   · data_dir 必须按**原始类型**校验 —— 若先 str() 掉，`true` 会变成字符串 "True" 而被放行。
  let rawAppUrl = DEFAULTS.app_url
  let rawExtra = DEFAULTS.extra
  let rawDataDir = DEFAULTS.data_dir

  for (const [key, value] of Object.entries(data)) {
    if (value === null) continue // 显式 null = 用默认值
    if (key === 'app_url') {
      rawAppUrl = value
      continue
    }
    if (JSON_KEYS.has(key)) {
      rawExtra = value
      continue
    }
    if (key === 'data_dir') {
      rawDataDir = value
      continue
    }
    if (BOOL_KEYS.has(key)) merged[key] = normalizeBool(key, value)
    else merged[key] = typeof value === 'boolean' ? value : pyStr(value)
  }

  // ⚠️ 顺序有意义：下面的 warning 输出顺序与 Python 版逐字对应（app_url → data_dir → icon_url
  //    → resolve_secret），对等性比对会检查这个序列。
  // 先归一 public_key：normalizeAppUrl 要用它判断「数组模式是否带公钥」。
  merged.public_key = normalizePublicKey(merged.public_key)
  merged.app_url = normalizeAppUrl(rawAppUrl, pyStr(merged.public_key))
  merged.extra = normalizeExtra(rawExtra)
  merged.data_dir = normalizeDataDir(rawDataDir)
  merged.icon_url = pyStr(merged.icon_url).trim()
  merged.resolve_secret = pyStr(merged.resolve_secret).trim()

  if (merged.icon_url && !/^https?:\/\//i.test(pyStr(merged.icon_url))) {
    emit(
      `::warning::icon_url 不是 http(s) 地址（${merged.icon_url}），` +
        '打包时会下载失败并回落到仓库默认图标',
    )
  }

  for (const key of BOOL_KEYS) merged[key] = normalizeBool(key, merged[key])

  // title（显示名）与 app_name（安装名）分离；显式空值 → 回落 app_name。
  // 取值链只有一跳：title → app_name。（旧字段 window_title 已移除，见 REMOVED_KEYS）
  let title = pyStr(merged.title).trim()
  if (!title) title = pyStr(merged.app_name)
  merged.title = title

  // ⚠️⚠️ 告警的**触发条件**必须对齐「运行期到底哪条路会解密」：
  //    · Tauri：`bootstrap.rs` 里 `parse_domains(APP_URL)` 命中（文本以 `[` 开头）才走 DNS TXT → 解密；
  //    · Electron：`navigate.ts` 的 `needsDnsResolve` = `Array.isArray(app_url)`。
  //    `SECRET` 的唯一消费点是 `resolve-core::derive_key`（只被 `decrypt_txt` 用），所以单 URL
  //    字符串模式下这个字段在运行期**没有任何调用点** —— 此时告警是狼来了。
  if (!merged.resolve_secret) {
    if (pyStr(merged.app_url).trim().startsWith('[')) {
      emit(
        '::warning::resolve_secret 未配置：app_url 是域名数组，运行期要靠它解密 TXT 密文；' +
          '将使用代码内置默认 Secret（所有产物共享同一固定密钥串，安全门槛较低）。' +
          '建议配置独立的 resolve_secret，并同步服务端 encrypt_url.py / resolve-tool。',
      )
    } else {
      // 单 URL 模式：明确说明「当前不参与」，比默认告警更有用，且不产生 CI 注解。
      emit(
        'resolve_secret 未配置：app_url 是单 URL 字符串，运行期不解密 DNS TXT ' +
          '⇒ 该字段当前不参与，无需配置（将来改用域名数组模式时才需要）。',
      )
    }
  }

  // 全部收敛为字符串，供 env / 打印使用。
  const out = {}
  for (const [k, v] of Object.entries(merged)) out[k] = typeof v === 'string' ? v : pyStr(v)
  return out
}

/**
 * 把 merged 表转回**保持 JSON 类型**的对象（Electron 进 asar 的那一份 config.json）。
 *
 * ⚠️ 与 env 不同：这里 bool 就是 bool、app_url 数组就是数组。
 * Electron 的 config.ts 按真实类型解析，不做字符串宽容。
 */
function toConfigJson(merged) {
  const out = {}
  for (const [key, value] of Object.entries(merged)) {
    if (JSON_KEYS.has(key)) out[key] = JSON.parse(value)
    else if (BOOL_KEYS.has(key)) {
      // merged 里是已规范化的字符串（"True"/"False"），这里转回真 bool。
      // ⚠️ 不能写 Boolean(value)：非空字符串恒为 true，会把 "False" 变成 true。
      out[key] = value.trim().toLowerCase() === 'true'
    } else if (key === 'app_url') {
      out[key] = value.trimStart().startsWith('[') ? JSON.parse(value) : value
    } else out[key] = value
  }
  // android_* 仅 Android 使用，不进 Windows 绿色版的 config.json ——
  // 那 4 个字段里含 keystore base64 与明文密码，没有任何理由随包分发。
  for (const key of SIGNING_KEYS) delete out[key]
  return out
}

/** 打卡日志时的字段展示（数值与敏感项做脱敏 / 摘要）。 */
function display(key, value) {
  if (key === 'app_url' || key === 'icon_url') return mask(value)
  if (key === 'android_keystore_password' || key === 'android_key_password' || key === 'resolve_secret') {
    return value ? '***（已配置）' : '（未配置）'
  }
  if (key === 'android_keystore_base64') return value ? `（已配置，${value.length} 字符）` : '（未配置）'
  return value
}

// ===========================================================================
// 进程与文件工具
// ===========================================================================
function have(cmd) {
  const r = spawnSync(cmd[0], cmd.slice(1), { stdio: 'ignore' })
  return !r.error && r.status === 0
}

/**
 * 执行子进程。⚠️ **不要加 `shell: true`**：实测 `spawnSync('pnpm', ...)` 无 shell 即可工作，
 * 而 shell 模式下 Node 只做 `join(' ')` 不做转义 ⇒ `-c.productName=带空格的名字` 会被 cmd.exe 拆成两个参数。
 * （另外 Node ≥ 22 明确禁止无 shell 直接执行 `.cmd`，但把名字写成 `pnpm` 反而能正常解析。）
 */
function run(cmd, cwd, env, dry) {
  const printable = cmd.map((c) => (c.includes(' ') ? `"${c}"` : c)).join(' ')
  emit(`\n$ (cd ${rel(cwd)}/) ${printable}`)
  if (dry) return
  const r = spawnSync(cmd[0], cmd.slice(1), { cwd, env, stdio: 'inherit' })
  if (r.error) throw new FatalError(`命令无法执行（${cmd[0]}）：${r.error.message}`)
  if (r.status !== 0) throw new FatalError(`命令失败（退出码 ${r.status}）：${printable}`)
}

function sha256(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex')
}

/** `20260924-215530` —— 给自动降级目录用的时间戳（本地时区，够唯一）。 */
function stamp() {
  const d = new Date()
  const p = (n) => String(n).padStart(2, '0')
  return (
    `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}` +
    `-${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}`
  )
}

/**
 * 跑子进程，输出**边跑边透传**给用户，同时留一份尾部用于判断失败类型。
 *
 * ⚠️ 必须边跑边透传：一次 electron-builder 要几分钟，若全缓冲到最后才打印，
 *    用户完全无法判断「卡住了」还是在干活。
 * 返回 `{ status, tail }`（`status` 为负 = 进程没能起来）。**不抛错** —— 由调用方决定怎么处理。
 */
function runTee(cmd, cwd, env, dry) {
  const printable = cmd.map((c) => (c.includes(' ') ? `"${c}"` : c)).join(' ')
  emit(`\n$ (cd ${rel(cwd)}/) ${printable}`)
  if (dry) return Promise.resolve({ status: 0, tail: [] })
  return new Promise((done) => {
    const p = spawn(cmd[0], cmd.slice(1), { cwd, env })
    const tail = []
    const onData = (chunk) => {
      const s = chunk.toString()
      process.stdout.write(s) // 透传，保持「看得见进度」
      for (const line of s.split(/\r?\n/)) if (line.trim()) tail.push(line)
      if (tail.length > 400) tail.splice(0, tail.length - 400)
    }
    p.stdout.on('data', onData)
    p.stderr.on('data', onData)
    p.on('error', (e) => {
      emit(`  子进程无法启动：${e.message}`)
      done({ status: -1, tail })
    })
    p.on('close', (code) => done({ status: code === null ? -1 : code, tail }))
  })
}

/**
 * 失败是不是「这台机器不让删东西」这一类 —— 而不是配置写错 / 代码坏。
 * electron-builder 打包前要清空 appOutDir，清不掉时它给的就是这两种标记。
 */
function isDeleteBlocked(tail) {
  return tail.some((l) => /safe-delete|emptyDir|trash operation/i.test(l))
}

/**
 * 记录「原文件 → 备份」，退出时无条件还原。
 *
 * ⚠️ 用 `finally` 而不是「成功才还原」：打包中途失败时更要还原，否则工作区留着一份被改过的
 * `tauri.conf.json`，下次 git diff 会莫名其妙。
 */
class Restorer {
  constructor() {
    this.items = []
    this.dirs = []
    this.tmp = mkdtempSync(join(tmpdir(), 'w2a-pack-backup-'))
    // ⚠️⚠️ 把备份目录**打印出来**：`finally` 还原的前提是进程活着走到那里。
    //    实测踩过：构建被 kill ⇒ 不还原 ⇒ `electron-app/config.json` 留在生成态。
    //    备份本身是好的，只是没人知道它在哪 —— 这一行就是那个入口。
    emit(`（原文件备份目录：${this.tmp}）`)
  }

  file(path) {
    if (!existsSync(path)) return
    const bak = join(this.tmp, `${this.items.length}-${path.split(/[\\/]/).pop()}`)
    copyFileSync(path, bak)
    this.items.push([path, bak])
  }

  dir(path) {
    if (!existsSync(path)) return
    const bak = join(this.tmp, `dir-${this.dirs.length}-${path.split(/[\\/]/).pop()}`)
    cpSync(path, bak, { recursive: true })
    this.dirs.push([path, bak])
  }

  restore(keep) {
    if (keep) {
      emit(`⚠️ --keep-config：刻意保留改动，备份留在 ${this.tmp}`)
      return
    }
    for (const [path, bak] of this.items) {
      copyFileSync(bak, path)
      emit(`已还原 ${rel(path)}（sha256 ${sha256(path).slice(0, 12)}）`)
    }
    for (const [path, bak] of this.dirs) {
      if (existsSync(path)) rmSync(path, { recursive: true, force: true })
      cpSync(bak, path, { recursive: true })
      emit(`已还原目录 ${rel(path)}/`)
    }
    rmSync(this.tmp, { recursive: true, force: true })
  }
}

/** 与 CI 逐字一致的「安装名」清洗：去 Windows 文件名非法字符、压空白、去首尾空格与句点。 */
function safeName(raw) {
  let s = raw ? raw.replace(/[\\/:*?"<>|]+/g, '_') : ''
  s = s ? s.replace(/\s+/g, ' ').replace(/^[ .]+|[ .]+$/g, '') : ''
  return s || 'Web2App'
}

/**
 * Tauri 本地打包的默认产物目录：仓库根 `tests/pack/tauri/`。
 * 与 Electron 的 `tests/pack/` 平级但**刻意分子目录** —— 两壳的绿色版都叫 `<name>.exe`
 * （electron-builder portable 同样产出 `<name>.exe`），混放会互相覆盖。
 * 整棵 `/tests/` 被根 .gitignore 忽略，产物不进版本库。
 */
const TAURI_OUT_DEFAULT = join(ROOT, 'tests', 'pack', 'tauri')

/**
 * 解析 `--out`：相对路径一律以**对应壳工程目录**为基准（electron-builder 在 electron-app/ 下跑、
 * cargo 在 webview-app/ 下跑，与各自配置里的相对路径语义一致），不按调用方 cwd 解析；
 * 绝对路径原样使用。
 *
 * 本地打包的默认目录都在仓库根 `tests/pack/` 下（见 doElectron / doTauri），
 * 整棵 `/tests/` 被根 .gitignore 忽略，产物不进版本库；`--out ../tests/<名字>` 可在 tests/ 下另换目录。
 */
function resolvePackOut(arg, baseDir) {
  return isAbsolute(arg) ? resolve(arg) : resolve(baseDir, arg)
}

/** Electron 侧的 `--out`：相对路径锚定 electron-app/。 */
function resolveElectronOut(arg) {
  return resolvePackOut(arg, ELECTRON)
}

/**
 * 跨卷也安全的文件移动：先复制再删源（`rename` 在跨盘符时会 EXDEV）。
 * 打包产物是个位数的大文件，复制成本可接受；目标同名文件直接覆盖。
 */
function moveFile(src, dest) {
  copyFileSync(src, dest)
  rmSync(src, { force: true })
}

// ===========================================================================
// 打包分支
// ===========================================================================

/** Electron：配置在**打包期**被读（`electron-app/config.json` → `out/config.json` → 进 `app.asar`）。 */
async function doElectron(args, merged, env, name, rr) {
  if (!have(['pnpm', '--version'])) {
    throw new FatalError('未找到 pnpm。Electron 侧本地打包需要 pnpm（见 electron-app/package.json）')
  }
  const dry = args.dryRun

  // 1) 落一份**规范化后**的 config.json —— 本地手工改配置最容易做错的一步：
  //    布尔写成字符串 "false"、app_url 数组写成字符串，都是「能出包但行为不对」。
  const cfgPath = join(ELECTRON, 'config.json')
  if (dry) {
    emit(`\n[1/4] 将写出规范化配置 → ${rel(cfgPath)}（原文件备份，打包后还原）`)
  } else {
    rr.file(cfgPath)
    // ⚠️ 一律 LF：与 CI（Linux）的 Python 产物一致；`writeFileSync` 不做换行转换。
    writeFileSync(cfgPath, JSON.stringify(toConfigJson(merged), null, 2) + '\n', { encoding: 'utf8' })
    emit(`\n[1/4] 已写出规范化配置 → ${rel(cfgPath)}（原文件已备份，打包后还原）`)
  }

  // 2) 图标 —— **判据只看 JSON**：`icon_url` 非空就下载并应用，与 CI 逐字一致
  //    （CI 那步的条件正是 `if [ -z "${ICON_URL:-}" ]; then … exit 0; fi`，没有任何额外开关）。
  //    ⚠️ 早先这里写的是 `args.icon && merged.icon_url` ⇒ 同一份 JSON 在 CI 会带图标、本地不带，
  //       一把「配置说了算」的事变成「命令行说了算」。要显式跳过就用 `--no-icon`（逃逸口，不是默认路径）。
  let extra = []
  if (merged.icon_url && !args.noIcon) {
    const url = merged.icon_url
    // ⚠️ 必须先建 build/：它被 .gitignore 忽略、fresh checkout 里不存在，
    //    而 curl 写不进去时报的是 `curl: (23) client returned ERROR on write`
    //    —— 看着像网络问题，换 URL 毫无用处（2026-09-22 实测）。
    const buildDir = join(ELECTRON, 'build')
    const dest = join(buildDir, 'icon.png')
    if (dry) {
      emit(`\n[2/4] 将下载图标 ${url} → ${rel(dest)}`)
    } else {
      mkdirSync(buildDir, { recursive: true })
      run(['curl', '-fsSL', '--retry', '3', url, '-o', dest], ELECTRON, env, false)
      const magic = readFileSync(dest).subarray(0, 8).toString('hex')
      if (magic !== '89504e470d0a1a0a') throw new FatalError(`icon_url 下载的不是 PNG（文件头=${magic}）`)
      // 托盘图标取 resources/icons/icon.png（由 extraResources 从 webview-app/icons/ 拷进包），
      // 只设 -c.win.icon 对托盘无效 ⇒ 必须同步覆盖那一份。它是 tracked，所以先备份。
      const tray = join(WEBVIEW, 'icons', 'icon.png')
      rr.file(tray)
      copyFileSync(dest, tray)
      emit(`托盘图标已同步 → ${rel(tray)}（打包后还原）`)
    }
    extra = ['-c.win.icon=build/icon.png']
  } else {
    const why = !merged.icon_url ? '配置里 icon_url 为空' : '--no-icon 显式跳过'
    emit(`\n[2/4] 跳过图标：${why}，沿用仓库默认图标`)
  }

  // 3) sidecar + 编译 + 铺资源
  //    ⚠️⚠️ 顺序关键：`w2a-proxy.exe` 必须**在 RESOLVE_SECRET 存在的同一环境里**编译。
  //    resolve-core 的 Secret 是 `option_env!("RESOLVE_SECRET")` 的编译期常量，
  //    先编后设 ⇒ sidecar 内置公开默认 Secret，用户配的 resolve_secret **静默失效**。
  emit('\n[3/4] 编译 sidecar（继承 RESOLVE_SECRET）→ 编译 TS → 铺打包资源')
  run(['pnpm', 'run', 'sidecar'], ELECTRON, env, dry)
  run(['pnpm', 'run', 'build'], ELECTRON, env, dry)
  run(['pnpm', 'run', 'stage'], ELECTRON, env, dry)

  // 4) 出包
  const forms = []
  if (args.target === 'electron-zip' || args.target === 'electron-all') forms.push('zip')
  if (args.target === 'electron-portable' || args.target === 'electron-all') forms.push('portable')
  // ⚠️ nsis 刻意**不并入 electron-all**：打安装版必须带 distribution=nsis 标记（见
  // builderCmd），而该标记会合并进同次构建的 app.asar，若与 zip/portable 同次构建，
  // 绿色产物会被一起误标成安装版（数据目录被强制到 %APPDATA%）。要全出就分别跑两次。
  if (args.target === 'electron-nsis') forms.push('nsis')
  emit(`\n[4/4] electron-builder：${forms.join(' + ')}`)

  // ⚠️ 受限网络下 electron-builder 会**自己去下载** Electron 运行时（223MB）并卡到超时：
  //       ⨯ Timeout awaiting 'request' for 600000ms  failedTask=build
  //    这是环境问题、不是配置或代码问题 —— `node_modules/electron/dist` 里其实已经有一份。
  //    ⚠️ 必须写长写法 `--config.electronDist=`；`-c.electronDist=` 在部分版本上会被当成
  //       配置文件路径而报 ENOENT。
  const extraBuilder = []
  const localDist = join(ELECTRON, 'node_modules', 'electron', 'dist')
  if (existsSync(localDist)) {
    extraBuilder.push('--config.electronDist=node_modules/electron/dist')
    emit(`    使用本地 Electron 运行时（跳过下载）：${rel(localDist)}`)
  }

  // 输出目录：本地打包默认落到**仓库根的 `tests/pack/`**。README §3.2 约定 tests/ 只装本地测试过程的
  // 临时/中间产物，根 `.gitignore` 的 `/tests/` 整目录忽略 ⇒ 150MB 量级、上千文件的 win-unpacked/ 树
  // **不会进 git、不污染版本库**。这与 CI 不冲突：CI 工作区一次性，electron-builder.yml 的
  // directories.output=release 只在 CI / 直接跑 pnpm dist 时生效；本地一律用下面的绝对路径覆盖。
  //
  // ⚠️⚠️ 这里**刻意不自己清空输出目录**，而是让 electron-builder 去清（它本来就会做，CI 走的也是这条路）。
  //    「脚本自己先清一遍」的写法两处都吃亏（2026-09-24 实测）：
  //      · 本机的删除有**配额**（`SAFE_DELETE_BULK_CONFIRM_REQUIRED`，50 次/轮）：自己清一次上千个
  //        文件就把配额烧光 ⇒ electron-builder 随后连自己的中间产物 `*.nsis.7z` 都删不掉，
  //        zip / portable **明明已经产出**，构建却以「失败」收尾；
  //      · 清空等于**先毁掉上一次的产物**，万一这次构建失败，用户两手空空。
  //    ⇒ 先让它试；只有**它自己**因删不掉而失败时，才自动换一个全新目录重试一次。
  //      全新目录没有东西可清 ⇒ emptyDir 无事可做 ⇒ 必然能出包；人工依旧不需要做任何事。
  // ⚠️ 显式给的相对 `--out` 以 **electron-app/** 为基准（electron-builder 就在那下面跑）；
  //    想落到根 tests/ 写 `--out ../tests/<名字>`，或直接给绝对路径。
  const defaultOut = join(ROOT, 'tests', 'pack')
  let outDir = args.out ? resolveElectronOut(args.out) : defaultOut

  const builderCmd = (dir) => [
    'pnpm',
    'exec',
    'electron-builder',
    '--win',
    ...forms,
    `-c.productName=${name}`,
    // 安装版标记：打进 app.asar 的 package.json，运行时 distribution.ts 据此强制
    // 使用系统默认数据目录（%APPDATA%），忽略 config 的 data_dir。绿色版不带此参数。
    ...(forms.includes('nsis') ? ['-c.extraMetadata.distribution=nsis'] : []),
    ...extraBuilder,
    `--config.directories.output=${dir}`,
    ...extra,
  ]

  let attempt = await runTee(builderCmd(outDir), ELECTRON, env, dry)
  if (attempt.status !== 0 && !dry && outDir === defaultOut && isDeleteBlocked(attempt.tail)) {
    const fallback = join(ROOT, 'tests', `pack-local-${stamp()}`)
    emit(
      `\n⚠️ electron-builder 清空 ${rel(outDir)} 时被删除策略拦住了（safe-delete / emptyDir）。\n` +
        `   这台机器不允许它删掉旧产物 ⇒ **自动**改用全新目录重试：${rel(fallback)}\n` +
        `   （旧产物原样留在 ${rel(defaultOut)}，没被动过；清掉它之后即可恢复用 tests/pack/）`,
    )
    outDir = fallback
    attempt = await runTee(builderCmd(outDir), ELECTRON, env, dry)
  }
  if (attempt.status !== 0) {
    const hint = isDeleteBlocked(attempt.tail)
      ? '\n  看起来是输出目录删不掉（不是配置/代码问题）：换一个全新目录再试即可，例如\n' +
        `    node scripts/pack-local.mjs --config <file> --target ${args.target} --out ../tests/pack-local-${stamp()}`
      : ''
    throw new FatalError(`electron-builder 失败（退出码 ${attempt.status}）${hint}`)
  }

  // 护栏：配置没进 asar 时 electron-builder 照样出包、双击照样打开，只是静默退回内置默认地址
  // （example.com），构建日志全绿 ⇒ 必须显式校验。
  // ⚠️ **一律显式传** asar 路径：check-asar-config.cjs 不给参数时会自己在 release/ 下递归找，
  //    而那里可能躺着上一次的旧产物 ⇒ 会出现「绿了，但绿的是上一次的包」。
  run(
    [
      'node',
      'scripts/check-asar-config.cjs',
      join(outDir, 'win-unpacked', 'resources', 'app.asar'),
    ],
    ELECTRON,
    env,
    dry,
  )

  emit(`\n产物目录：${rel(outDir)}`)
}

/** Tauri：配置在**编译期**被读（`option_env!` → 二进制常量）⇒ 必须在 cargo 之前设好环境变量。 */
function doTauri(args, merged, env, name, rr) {
  if (!have(['cargo', '--version'])) throw new FatalError('未找到 cargo')
  if (!have(['cargo', 'tauri', '--version'])) {
    throw new FatalError(
      '未安装 tauri-cli。本地 Tauri 打包需要它：\n' +
        '    cargo install tauri-cli --version "^2"\n' +
        '  （CI 的 windows job 也是这一步；首次安装要编几分钟）',
    )
  }
  const dry = args.dryRun
  const conf = join(WEBVIEW, 'tauri.conf.json')

  // 1) 安装名 → productName（与 CI 同一个清洗规则）
  if (dry) {
    emit(`\n[1/4] 将改写 ${rel(conf)}：productName = ${name}，mainBinaryName = web2app`)
  } else {
    rr.file(conf) // tracked，必须备份
    const data = JSON.parse(readFileSync(conf, 'utf8'))
    data.productName = name
    // 固定主程序文件名 ⇒ target/release/web2app.exe 路径稳定，单文件版那步可以直接引用
    data.mainBinaryName = 'web2app'
    writeFileSync(conf, JSON.stringify(data, null, 2) + '\n', { encoding: 'utf8' })
    emit(`\n[1/4] 已改写 ${rel(conf)}：productName = ${name}（打包后还原）`)
  }

  // 2) 图标 —— 同样**判据只看 JSON**（CI 那步是 `[ -z "${ICON_URL:-}" ]` 判断后跑 `cargo tauri icon`）
  //    ⚠️ `cargo tauri icon` 会重新生成**整套** `icons/`，而它是 tracked ⇒ 整目录备份 + 还原。
  if (merged.icon_url && !args.noIcon) {
    const url = merged.icon_url
    const png = join(ROOT, 'icon-custom.png')
    if (dry) {
      emit(`\n[2/4] 将下载图标 ${url} 并执行 cargo tauri icon`)
    } else {
      rr.dir(join(WEBVIEW, 'icons'))
      run(['curl', '-fsSL', '--retry', '3', url, '-o', png], ROOT, env, false)
      if (readFileSync(png).subarray(0, 8).toString('hex') !== '89504e470d0a1a0a') {
        throw new FatalError('icon_url 下载的不是 PNG')
      }
      run(['cargo', 'tauri', 'icon', png], WEBVIEW, env, false)
      rmSync(png, { force: true })
      emit('icons/ 已重新生成（打包后整目录还原）')
    }
  } else {
    const why = !merged.icon_url ? '配置里 icon_url 为空' : '--no-icon 显式跳过'
    emit(`\n[2/4] 跳过图标：${why}，沿用仓库默认图标`)
  }

  // 3) 编译 + 打包
  //    ⚠️ 环境变量必须在 cargo 之前就位 —— Tauri 的配置是 `option_env!` 编译期常量，
  //    改了环境变量必须**重新编译**（cargo 通过 dep-info 的 env-dep 感知，无需 clean）。
  const wantMsi = args.target === 'tauri-msi' || args.target === 'tauri-all'
  const wantZip = args.target === 'tauri-portable' || args.target === 'tauri-all'

  if (wantMsi) {
    emit('\n[3/4] cargo tauri build（产安装包，同时也产出 web2app.exe）')
    run(['cargo', 'tauri', 'build'], WEBVIEW, env, dry)
  } else if (wantZip) {
    emit('\n[3/4] cargo tauri build --no-bundle（只编译 exe，不产安装包）')
    run(['cargo', 'tauri', 'build', '--no-bundle'], WEBVIEW, env, dry)
  }

  // 4) 产物归集 —— 与 Electron 同一约定：**交付产物一律搬到仓库根 tests/ 下**，
  //    不留在 cargo 构建树（webview-app/target/release/）里。Tauri 不能像 electron-builder
  //    那样配置输出目录，只能在构建后搬运：
  //      · 单文件绿色版：web2app.exe（cargo 主产物，固定文件名）**复制**为 <out>/<name>.exe
  //        —— 是复制不是移动：web2app.exe 是增量编译的中间产物，删了下次要全量重编；
  //      · 安装包：bundle/msi/*.msi、bundle/nsis/* **移动**到 <out>/（tauri.conf.json 的
  //        bundle.targets 同时含 msi 与 nsis，本机构建两种都产）。
  //    默认目录 tests/pack/tauri/ 与 Electron 的 tests/pack/ 分开：两壳绿色版都叫 <name>.exe，混放互相覆盖。
  const releaseDir = join(WEBVIEW, 'target', 'release')
  const outDir = args.out ? resolvePackOut(args.out, WEBVIEW) : TAURI_OUT_DEFAULT

  if (dry) {
    emit('\n[4/4] 产物归集（dry-run，不搬文件）：')
    if (wantZip) {
      emit(`  [单文件绿色版] ${rel(join(releaseDir, 'web2app.exe'))} → ${rel(join(outDir, `${name}.exe`))}`)
    }
    if (wantMsi) {
      emit(`  [msi 安装包]  ${rel(join(releaseDir, 'bundle', 'msi'))}/*.msi  → ${rel(outDir)}/`)
      emit(`  [nsis 安装包] ${rel(join(releaseDir, 'bundle', 'nsis'))}/*       → ${rel(outDir)}/`)
    }
    emit(`\n产物目录：${rel(outDir)}/`)
    return
  }

  // 默认目录由本脚本**完全掌控** ⇒ 整目录重建，顺手清掉上一版本号的陈旧安装包（如升版后的旧 MSI）。
  // ⚠️ 自定义 --out 可能指向用户自己的目录，**绝不整目录清空**，只搬运/覆盖同名文件。
  if (!args.out && existsSync(outDir)) rmSync(outDir, { recursive: true, force: true })
  mkdirSync(outDir, { recursive: true })

  if (wantZip) {
    const exe = join(releaseDir, 'web2app.exe')
    if (!existsSync(exe)) throw new FatalError(`未找到 ${rel(exe)}，主程序没编译出来`)
    const dest = join(outDir, `${name}.exe`)
    copyFileSync(exe, dest) // ⚠️ 复制：web2app.exe 是 cargo 增量编译产物，必须留在 target/
    emit(`  [单文件绿色版] ${rel(dest)}（改名后即为 ${name}.免安装版.exe）`)
  }

  if (wantMsi) {
    // tauri.conf.json 的 bundle.targets = ["msi","nsis"] ⇒ 两种安装包都要收。
    let installerCount = 0
    for (const kind of ['msi', 'nsis']) {
      const subDir = join(releaseDir, 'bundle', kind)
      if (!existsSync(subDir)) continue
      for (const file of readdirSync(subDir)) {
        const src = join(subDir, file)
        if (!statSync(src).isFile()) continue
        const dest = join(outDir, file)
        moveFile(src, dest) // 安装包是交付产物，搬走（清空 bundle/，下次构建重新生成）
        emit(`  [${kind} 安装包] ${rel(dest)}`)
        installerCount++
      }
    }
    if (installerCount === 0) {
      throw new FatalError(
        `cargo tauri build 已返回成功，但在 ${rel(join(releaseDir, 'bundle'))}/ 下没找到任何安装包`,
      )
    }
  }

  emit(`\n产物目录：${rel(outDir)}/`)
}

// ===========================================================================
// 契约自测（不需要 Python）
// ===========================================================================
function assertThat(cond, msg) {
  if (!cond) throw new Error(`断言失败：${msg}`)
}

function expectError(data, needle) {
  try {
    buildMerged(data)
  } catch (e) {
    if (e instanceof ConfigError) {
      assertThat(String(e.message).includes(needle), `报错文案里应包含 ${JSON.stringify(needle)}，实际是：${e.message}`)
      return
    }
    throw e
  }
  throw new Error(`应当报错但通过了：${JSON.stringify(data)}（期望提到 ${JSON.stringify(needle)}）`)
}

/** 把两侧都要跑的一批 payload 抽出来，`--selftest` 与 `--parity` 共用同一份。 */
const PARITY_PAYLOADS = [
  { app_url: 'https://example.com' },
  { app_url: 'https://a.com', title: '我的应用' },
  { app_url: 'https://a.com', app_name: 'MyApp', title: '' },
  { app_url: 'https://a.com', proxy_enabled: true },
  { app_url: 'https://a.com', tabs_enabled: true, debug: true },
  { app_url: 'https://a.com', proxy_bypass: '' },
  { app_url: 'https://a.com', data_dir: '' },
  { app_url: 'https://a.com', data_dir: '   ' },
  { app_url: 'https://a.com', data_dir: ' mydata ' },
  { app_url: 'https://a.com', data_dir: 'D:\\appdata' },
  { app_url: 'https://a.com', data_dir: 'bad<name' },
  { app_url: 'https://a.com', extra: { k: 'v', n: 1 } },
  { app_url: 'https://a.com', extra: [1, 2] },
  { app_url: 'https://a.com', resolve_secret: ' s ' },
  { app_url: 'https://a.com', icon_url: 'https://x.com/i.png' },
  { app_url: 'https://a.com', icon_url: 'not-a-url' },
  { app_url: 'https://a.com', app_name: 123 },
  { app_url: 'https://a.com', title: '   ' },
  { app_url: ['a.com', 'b.com'], public_key: 'AAAA' },
  { app_url: ['a.com'], public_key: 'AAAA', resolve_secret: 'secret' },
  { app_url: '["a.com"]', public_key: 'AAAA' },
  { app_url: 'https://user:pass@a.com/Path' },
  { app_url: 'ftp://a.com' },
  { app_url: 'https://a.com', android_keystore_base64: 'eHh4' },
  // ---- 以下以「必须报错」为主（`app_url: 123` 例外：它只产生 warning，仍会成功）----
  { app_url: 'https://a.com', debug: 'true' },
  { app_url: 'https://a.com', proxy_enabled: 'on' },
  { app_url: 'https://a.com', proxy_enabled: 1 },
  { app_url: 'https://a.com', proxy_enabled: null },
  { app_url: ['a.com'] },
  { app_url: [] },
  { app_url: '   ' },
  { app_url: '[a.com]' },
  { app_url: 123 },
  { app_url: true },
  { app_url: 'https://a.com', data_dir: true },
  { app_url: 'https://a.com', data_dir: 123 },
  { app_url: 'https://a.com', window_title: 'X' },
  { app_url: 'https://a.com', data_dir_default: false },
  { app_url: 'https://a.com', proxy_enable: true },
]

function selftest() {
  // 自测会把 buildMerged 跑很多遍，其中的 ::warning:: 提示会刷屏；这里整体静音，
  // 只有断言失败才需要看到输出（断言异常照常抛出）。
  const { error } = captureOutput(() => {
    // 最小配置：只有 app_url
    let m = buildMerged({ app_url: 'https://example.com' })
    assertThat(m.app_url === 'https://example.com', 'app_url 原样')
    assertThat(m.app_name === 'Web2App', 'app_name 默认')
    assertThat(m.title === 'Web2App', 'title 空 → 回落 app_name')
    assertThat(m.proxy_enabled === 'False' && m.tabs_enabled === 'False', '布尔默认 False')
    assertThat(m.debug === 'False', 'debug 默认必须关闭（默认开启＝往用户数据目录里塞日志文件）')
    assertThat(m.data_dir === 'data', 'data_dir 默认是相对路径 data')
    assertThat(m.proxy_bypass === 'localhost;127.0.0.1;<-loopback>', 'proxy_bypass 默认串')

    assertThat(buildMerged({ app_url: 'https://a.com', title: '我的应用' }).title === '我的应用', 'title 显式给出不回落')

    assertThat(buildMerged({ app_url: 'https://a.com', proxy_enabled: true }).proxy_enabled === 'True', '真 true 通过')
    assertThat(buildMerged({ app_url: 'https://a.com', debug: true }).debug === 'True', 'debug 真 true 通过')
    expectError({ app_url: 'https://a.com', debug: 'true' }, '必须是布尔值')
    for (const bad of ['true', 'false', '1', 'yes', 'on', '']) {
      expectError({ app_url: 'https://a.com', proxy_enabled: bad }, '必须是布尔值')
    }

    // app_url
    m = buildMerged({ app_url: ['a.com', 'b.com'], public_key: 'AAAA' })
    assertThat(JSON.parse(m.app_url).join(',') === 'a.com,b.com', '数组保持数组')
    expectError({ app_url: ['a.com'] }, '必须提供 public_key')
    expectError({ app_url: [] }, '不能为空')
    expectError({ app_url: '   ' }, '缺少必填字段 app_url')
    m = buildMerged({ app_url: '["a.com"]', public_key: 'AAAA' })
    assertThat(JSON.parse(m.app_url).join(',') === 'a.com', 'JSON 数组文本老写法仍可用')
    expectError({ app_url: '[a.com]' }, '不是合法的 JSON 字符串数组')
    // Python 的 str() 语义：数组里的非字符串元素会走 "True"/"None"/repr
    m = buildMerged({ app_url: [123, true, null, ' x '], public_key: 'AAAA' })
    assertThat(JSON.parse(m.app_url).join('|') === '123|True|None|x', 'str() 语义与 Python 一致')

    // 已移除 / 未知字段
    expectError({ app_url: 'https://a.com', window_title: 'X' }, '已移除')
    expectError({ app_url: 'https://a.com', proxy_enable: true }, 'proxy_enabled')

    // data_dir
    assertThat(buildMerged({ app_url: 'https://a.com' }).data_dir === 'data', 'data_dir 默认')
    assertThat(buildMerged({ app_url: 'https://a.com', data_dir: '' }).data_dir === '', '空串必须保留语义')
    assertThat(buildMerged({ app_url: 'https://a.com', data_dir: '   ' }).data_dir === '', '纯空白视同空串')
    assertThat(buildMerged({ app_url: 'https://a.com', data_dir: ' mydata ' }).data_dir === 'mydata', '相对路径去空白')
    assertThat(buildMerged({ app_url: 'https://a.com', data_dir: 'D:\\appdata' }).data_dir === 'D:\\appdata', '绝对路径原样')
    expectError({ app_url: 'https://a.com', data_dir: true }, '必须是字符串')
    expectError({ app_url: 'https://a.com', data_dir: 123 }, '必须是字符串')
    expectError({ app_url: 'https://a.com', data_dir_default: false }, '已移除')
    expectError({ app_url: 'https://a.com', data_dir_default: false }, '"data_dir": "data"')

    // extra
    assertThat(buildMerged({ app_url: 'https://a.com', extra: { k: 'v' } }).extra === '{"k":"v"}', 'extra 序列化')
    assertThat(buildMerged({ app_url: 'https://a.com' }).extra === '{}', 'extra 默认 {}')

    assertThat(buildMerged({ app_url: 'https://a.com', resolve_secret: ' s ' }).resolve_secret === 's', 'secret 去空白')

    // resolve_secret 提示**只在域名数组模式**出现
    let out = captureOutput(() => buildMerged({ app_url: 'https://a.com' })).lines.join('\n')
    assertThat(!out.includes('::warning::'), `单 URL 模式不该产生 warning：${out}`)
    assertThat(out.includes('resolve_secret') && out.includes('不参与'), '应说明该字段当前不参与')
    out = captureOutput(() => buildMerged({ app_url: ['a.com'], public_key: 'AAAA' })).lines.join('\n')
    assertThat(out.includes('::warning::resolve_secret 未配置'), '域名数组模式必须告警')
    out = captureOutput(() => buildMerged({ app_url: '["a.com"]', public_key: 'AAAA' })).lines.join('\n')
    assertThat(out.includes('::warning::resolve_secret 未配置'), 'JSON 数组文本模式也必须告警')
    for (const payload of [
      { app_url: 'https://a.com', resolve_secret: 's' },
      { app_url: ['a.com'], public_key: 'AAAA', resolve_secret: 's' },
    ]) {
      out = captureOutput(() => buildMerged(payload)).lines.join('\n')
      assertThat(!out.includes('resolve_secret'), '配了 Secret 时两种模式都安静（连说明行也不出现）')
    }

    // toConfigJson：bool 保持真 bool、数组保持数组、android_* 不落盘
    const merged = buildMerged({
      app_url: ['a.com'],
      public_key: 'AAAA',
      tabs_enabled: true,
      debug: true,
      data_dir: '',
      android_keystore_base64: 'eHh4',
    })
    const cfg = toConfigJson(merged)
    assertThat(cfg.tabs_enabled === true, 'config.json 里必须是真 bool')
    assertThat(cfg.debug === true, 'debug 在 config.json 里也必须是真 bool')
    assertThat(Array.isArray(cfg.app_url) && cfg.app_url[0] === 'a.com', 'config.json 里数组要保持数组')
    assertThat(!('android_keystore_base64' in cfg), 'android_* 不应进 Windows 包的 config.json')
    assertThat(cfg.data_dir === '', '空串必须原样落到 config.json（不被改成默认值）')

    // 安装名清洗（与 CI 逐字一致）
    assertThat(safeName('a/b:c*d') === 'a_b_c_d', '非法字符替换')
    assertThat(safeName('  x  y ') === 'x y', '压空白 + 去首尾')
    assertThat(safeName('') === 'Web2App', '空名字回落')
    assertThat(safeName('.hidden.') === 'hidden', '去首尾句点')

    // 相似度算法必须与 difflib 一致（--parity 无法覆盖「建议哪个名字」，因为那是纯函数）
    assertThat(getCloseMatches('proxy_enable', Object.keys(DEFAULTS), 1, 0.5)[0] === 'proxy_enabled', '近似名建议')
    assertThat(getCloseMatches('windo_title', Object.keys(DEFAULTS), 1, 0.5)[0] === 'title', 'windo_title → title')

    // --out 解析：相对路径锚定 electron-app/；../tests/ 越界到仓库根 tests/ 是默认产物位置，必须放行
    assertThat(
      resolveElectronOut('release/local-x') === join(ELECTRON, 'release', 'local-x'),
      '相对 --out 以 electron-app/ 为基准',
    )
    assertThat(
      resolveElectronOut('../tests/pack') === join(ROOT, 'tests', 'pack'),
      '../tests/<名字> 解析到仓库根 tests/（本地打包默认产物根，已被 /tests/ 忽略）',
    )
    assertThat(
      resolveElectronOut(join(ROOT, 'tests', 'pack-local-x')) === join(ROOT, 'tests', 'pack-local-x'),
      '绝对路径原样使用',
    )

    // Tauri 侧 --out：相对路径锚定 webview-app/；默认产物目录必须落在 tests/pack/tauri（与 Electron 分子目录）
    assertThat(
      resolvePackOut('../tests/myapp', WEBVIEW) === join(ROOT, 'tests', 'myapp'),
      'Tauri 相对 --out 以 webview-app/ 为基准，../tests/<名字> 越界到仓库根 tests/',
    )
    assertThat(
      TAURI_OUT_DEFAULT === join(ROOT, 'tests', 'pack', 'tauri'),
      'Tauri 默认产物目录是 tests/pack/tauri/（两壳绿色版同名，混放会互相覆盖）',
    )
  })
  if (error) throw error
  emit('Node 版解析器自测全部通过 ✓')
  return 0
}

// ===========================================================================
// 与 Python 解析器的对等性比对（需要 Python）
// ===========================================================================
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

function parity() {
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
