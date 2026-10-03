/**
 * PNG 像素级取证：把一张截图读成「结构图」+ 关键列的颜色扫描。
 *
 * 为什么需要它：标题栏 / 滚动条这类错位问题，肉眼看法受缩放误导（Read 工具会把大图缩略，
 * 1656px 宽的图看起来像 1064px，一眼看过去「好像铺满了」）。`capturePage` 出的 PNG 又没法
 * 用现成图片库（沙箱没有 PIL，PowerShell 的 `Add-Type` 被安全策略拦）。
 * 解码逻辑在 `png-decode.cjs`，本脚本只负责把它渲染成人能读的样子。
 *
 * 用法：`node scripts/diag/png-inspect.cjs <png> [--block=8] [--cols=24]`
 *   --block  结构图的降采样块大小（默认 8px）
 *   --cols   右边缘扫描的列数（默认 24）
 */
const { readFileSync } = require('node:fs')
const { decodePng, rgb } = require('./png-decode.cjs')

const file = process.argv[2]
const argv = process.argv.slice(2)
const num = (name, dflt) => {
  const hit = argv.find((a) => a.startsWith(`--${name}=`))
  return hit ? Number(hit.slice(name.length + 3)) : dflt
}
const BLOCK = num('block', 8)
const COLS = num('cols', 24)

if (!file) {
  console.error('用法：node scripts/diag/png-inspect.cjs <png> [--block=8] [--cols=24]')
  process.exit(2)
}

const dec = decodePng(readFileSync(file))
const { w, h } = dec
const at = (x, y) => rgb(dec, x, y)
const cls = ([r, g, b, a]) => {
  if (a !== undefined && a < 16) return ' ' // 透明
  const lum = 0.299 * r + 0.587 * g + 0.114 * b
  if (lum < 90) return '#' // 深色（标题栏 / 文字）
  if (lum > 240) return '.' // 近白（页面底 / 滚动条轨道 #f1f1f1 也落在这里）
  if (Math.abs(r - g) < 12 && Math.abs(g - b) < 12) return '~' // 中性灰（滚动条 thumb）
  return '+' // 彩色内容
}

console.log(`PNG ${file} ${w}x${h} bpp=${dec.bpp}`)
console.log(`--- 结构图（每块 ${BLOCK}px，x 轴压缩为 ${Math.ceil(w / BLOCK)} 列）---`)
for (let y = 0; y < h; y += BLOCK) {
  let row = ''
  for (let x = 0; x < w; x += BLOCK) {
    // 块内取众数，避免被单像素噪声带偏
    const cnt = new Map()
    for (let dy = 0; dy < BLOCK && y + dy < h; dy++) {
      for (let dx = 0; dx < BLOCK && x + dx < w; dx++) {
        const c = cls(at(x + dx, y + dy))
        cnt.set(c, (cnt.get(c) ?? 0) + 1)
      }
    }
    let best = ' '
    let bestN = -1
    for (const [c, n] of cnt) if (n > bestN) ((best = c), (bestN = n))
    row += best
  }
  console.log(String(y).padStart(4, ' ') + ' ' + row)
}

console.log(`--- 右边缘 ${COLS} 列逐像素扫描（x = w-${COLS} .. w-1，仅前 120 行）---`)
console.log(' y    ' + Array.from({ length: COLS }, (_, i) => (i % 10 === 0 ? '|' : ' ')).join(''))
console.log(' y    ' + Array.from({ length: COLS }, (_, i) => String((w - COLS + i) % 10)).join(''))
for (let y = 0; y < Math.min(h, 120); y += 2) {
  let row = ''
  for (let x = w - COLS; x < w; x++) row += cls(at(x, y))
  console.log(String(y).padStart(4, ' ') + '  ' + row)
}

// 关键判定：顶部深色带（标题栏）向右铺到哪里？
let barRight = -1
for (let x = w - 1; x >= 0; x--) {
  if (cls(at(x, 2)) === '#') {
    barRight = x
    break
  }
}
console.log(`--- 判定 ---`)
console.log(`顶部深色带最右到达 x=${barRight}（图宽 ${w}，右侧缺口 ${w - 1 - barRight}px）`)
console.log(`⚠️ 这里的「缺口」是**物理像素**；除以缩放比才是 CSS px（dpr=2 时除 2）。`)
for (const y of [0, 1, 2, 4, 8, 20, 36, 37, 40, 60, 100]) {
  if (y >= h) continue
  const c = at(w - 1, y)
  console.log(`  右边缘 (x=${w - 1}, y=${y}) rgb(${c.join(',')})  ${cls(c)}`)
}
