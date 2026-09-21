/**
 * PNG 解码 + 常用像素测量（零依赖，只用 node:zlib）。
 *
 * 为什么需要：这个项目里「标题栏有没有铺满整窗宽」必须用**像素**回答。
 * `getBoundingClientRect()` 会撒谎 —— 它给的是布局宽度，而滚动条是在**绘制**阶段
 * 覆盖上去的（见 titlebar.js 里 `scrollbar-width:none` 那段）。2026-09-17 之前
 * 两次断言都因为信任 rect 而把缺陷锁死成了「期望值」。
 *
 * 沙箱约束：没有 PIL，PowerShell 的 `Add-Type` 被安全策略拦，所以纯 JS 解码。
 * 只支持 8bit、非隔行的 PNG（`capturePage().toPNG()` / 系统截图都满足）。
 */
'use strict'
const zlib = require('node:zlib')

function decodePng(buf) {
  let off = 8
  let w = 0
  let h = 0
  let colorType = 6
  const idat = []
  while (off + 8 <= buf.length) {
    const len = buf.readUInt32BE(off)
    const type = buf.toString('ascii', off + 4, off + 8)
    const data = buf.subarray(off + 8, off + 8 + len)
    if (type === 'IHDR') {
      w = data.readUInt32BE(0)
      h = data.readUInt32BE(4)
      const bitDepth = data[8]
      colorType = data[9]
      const interlace = data[12]
      if (bitDepth !== 8) throw new Error('只支持 8bit PNG，实际 ' + bitDepth)
      if (interlace !== 0) throw new Error('不支持隔行 PNG')
    } else if (type === 'IDAT') {
      idat.push(data)
    } else if (type === 'IEND') break
    off += 12 + len
  }
  const bpp = colorType === 6 ? 4 : colorType === 2 ? 3 : colorType === 0 ? 1 : 4
  const raw = zlib.inflateSync(Buffer.concat(idat))
  const stride = w * bpp + 1
  const px = Buffer.alloc(w * h * bpp)
  let prev = Buffer.alloc(w * bpp)
  for (let y = 0; y < h; y++) {
    const ft = raw[y * stride]
    const line = raw.subarray(y * stride + 1, y * stride + 1 + w * bpp)
    const cur = Buffer.alloc(w * bpp)
    for (let i = 0; i < w * bpp; i++) {
      const a = i >= bpp ? cur[i - bpp] : 0
      const b = prev[i]
      const c = i >= bpp ? prev[i - bpp] : 0
      let v = line[i]
      if (ft === 1) v += a
      else if (ft === 2) v += b
      else if (ft === 3) v += (a + b) >> 1
      else if (ft === 4) {
        const p = a + b - c
        const pa = Math.abs(p - a)
        const pb = Math.abs(p - b)
        const pc = Math.abs(p - c)
        v += pa <= pb && pa <= pc ? a : pb <= pc ? b : c
      }
      cur[i] = v & 255
    }
    cur.copy(px, y * w * bpp)
    prev = cur
  }
  return { w, h, bpp, px }
}

/** 某点的亮度（0-255）。 */
function lum(dec, x, y) {
  const i = (y * dec.w + x) * dec.bpp
  const r = dec.px[i]
  const g = dec.px[i + 1]
  const b = dec.px[i + 2]
  const a = dec.bpp === 4 ? dec.px[i + 3] : 255
  if (a < 16) return 255 // 透明当作亮
  return 0.299 * r + 0.587 * g + 0.114 * b
}

/** 某点的 rgb 数组。 */
function rgb(dec, x, y) {
  const i = (y * dec.w + x) * dec.bpp
  return [dec.px[i], dec.px[i + 1], dec.px[i + 2]]
}

/**
 * 在 y 行上从右往左找第一个「深色」像素的 x —— 即深色条的**可见**右边界。
 * 返回 -1 表示整行都没有深色。
 */
function rightmostDark(dec, y, maxLum = 90) {
  const yy = Math.max(0, Math.min(dec.h - 1, y))
  for (let x = dec.w - 1; x >= 0; x--) {
    if (lum(dec, x, yy) < maxLum) return x
  }
  return -1
}

/** 在 y 行上从右往左找第一个「不接近指定色」的 x（用于量色带边界）。 */
function rightmostNot(dec, y, target, tol = 12) {
  const yy = Math.max(0, Math.min(dec.h - 1, y))
  for (let x = dec.w - 1; x >= 0; x--) {
    const c = rgb(dec, x, yy)
    if (Math.abs(c[0] - target[0]) > tol || Math.abs(c[1] - target[1]) > tol || Math.abs(c[2] - target[2]) > tol) {
      return x
    }
  }
  return -1
}

module.exports = { decodePng, lum, rgb, rightmostDark, rightmostNot }
