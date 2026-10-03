/**
 * Python 语义小工具
 * 解析器用 `str()` / `repr()` / `difflib.get_close_matches()` 拼文案与取值，要**逐字**复刻
 * 就不能用 JS 的同名直觉（`String(true)` 是 "true"，Python 是 "True"）。
 */

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

export { pyTypeName, pyStr, pyRepr, pyStrRepr, getCloseMatches }
