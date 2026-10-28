/**
 * 纯工具函数（零 Electron、零业务依赖）—— 2026-10-24 自 config.ts 抽出
 * （架构审查批次⑤ P2-5）。
 *
 * 落到这里的标准：**两处以上共用、且语义是「纯函数」**。配置解析的「非字符串怎么收敛 /
 * 目录名怎么清洗 / 拼错字段怎么提示」在 config.json 与 settings.json 两边必须同规则，
 * 集中一处维护，避免各养一份会漂移的私有拷贝（toStr 就曾是 settings.ts 里的逐字拷贝）。
 */

/**
 * 字符串字段宽容收敛：字符串原样；null/undefined 取默认值；其它类型 String() 转换。
 *
 * 这是 config.json / settings.json 解析共用的唯一实现（settings.ts 此前有一份逐字
 * 相同的私有拷贝，2026-10-01 合并）—— 两壳配置解析的「非字符串怎么收敛」必须同规则。
 */
export function toStr(v: unknown, dflt = ''): string {
  return typeof v === 'string' ? v : v == null ? dflt : String(v)
}

/**
 * 清洗成 Windows 合法目录名（与 webview-app 的 `sanitize_dir_name` 同规则）。
 *
 * ⚠️ 三条规则一条都不能少（2026-09-27 对齐，旧实现漏了后两条 → 两壳算出不同目录名，
 * data_dir="" 模式下 settings.json 读取候选会分叉）：
 *   1. 非法字符 < > : " / \\ | ? * 与控制字符 → 下划线；
 *   2. 压缩连续空白为一个空格、去首尾空白；
 *   3. 去首尾句点（Windows 下以点结尾的目录名会被规范化掉）；
 * 清洗后为空（含全空白 / 全是句点）→ 回落 "Web2App"。
 *
 * ⚠️ 空白集合不用裸 `\s`：Rust `char::is_whitespace`（Unicode White_Space）与 JS `\s`
 * 差两个冷门码点——U+0085 NEL（Rust 算空白、JS 不算）、U+FEFF BOM（JS 算、Rust 不算）。
 * `[^\S﻿]` 即「JS 空白但剔除 BOM」，再并上 NEL，与 Rust 逐字同源。
 */
// Rust White_Space 口径的连续空白（= JS \s − U+FEFF + U+0085）。
const RUST_WHITESPACE_RUN = new RegExp('(?:[^\\S\\uFEFF]|\\u0085)+', 'g')

export function sanitizeDirName(name: string): string {
  const replaced = name.replace(/[<>:"/\\|?*\u0000-\u001f]/g, '_')
  // 压缩连续空白（Rust White_Space 口径：JS \s − U+FEFF + U+0085，见函数头注释）
  const collapsed = replaced.replace(RUST_WHITESPACE_RUN, ' ').trim()
  const cleaned = collapsed.replace(/^\.+|\.+$/g, '')
  return cleaned.length > 0 ? cleaned : 'Web2App'
}

/** 极简近似名查找（编辑距离 ≤ 2 或前缀包含），只为给出「是不是想写 X」的提示。 */
export function nearestKey(key: string, candidates: string[]): string | null {
  let best: string | null = null
  let bestScore = Infinity
  for (const c of candidates) {
    const d = editDistance(key.toLowerCase(), c.toLowerCase())
    if (d < bestScore) {
      bestScore = d
      best = c
    }
  }
  return best !== null && bestScore <= 2 && bestScore < key.length ? best : null
}

export function editDistance(a: string, b: string): number {
  const prev = Array.from({ length: b.length + 1 }, (_, i) => i)
  for (let i = 1; i <= a.length; i++) {
    let diag = prev[0]
    prev[0] = i
    for (let j = 1; j <= b.length; j++) {
      const tmp = prev[j]
      prev[j] = Math.min(prev[j] + 1, prev[j - 1] + 1, diag + (a[i - 1] === b[j - 1] ? 0 : 1))
      diag = tmp
    }
  }
  return prev[b.length]
}
