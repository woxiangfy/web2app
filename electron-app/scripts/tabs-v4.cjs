/**
 * T0 前置校验 —— V4「子视图绘制在窗口自身内容之上」程序化验证（Electron 44.4.1）
 *
 * 背景：M5 多页签选型 A′（见 docs/electron-tabs-design.md v1.1）= BrowserWindow 当壳
 * （窗口自身页面 shell.html）+ 子 WebContentsView 当内容。它的**唯一硬前提**是：
 * 子视图必须绘制在窗口自身内容【之上】，否则内容区显示成壳页面（白/灰），功能不可用。
 * 对应 electron#41044（已关闭，但本项目此前从未实测）。
 *
 * 判定思路（像素级直接证明，比人眼目视更硬）：
 *   · 壳页面（窗口自身 webContents）：y∈[0,38) 纯红 #ff0000（标签栏），y≥38 纯蓝 #0000ff
 *   · 内容视图（WebContentsView）：bounds={x:0,y:38,...}，页面纯绿 #00ff00
 *   · 系统级抓屏后断言：
 *       1) y=20 → 红：壳的标签栏可见，未被内容视图盖
 *       2) y=60 → 绿：内容视图绘制在壳页面之上（若为蓝 = #41044 回归，V4 失败）
 *       3) y=60 绝非蓝：排除「子视图没画出来、壳页面蓝底透出」的假象
 *
 * ⚠️⚠️ 取证方法必须是 desktopCapturer，不能用 capturePage：
 *   `webContents.capturePage()` 只从「单个 guest webContents 的合成器 surface」拷贝
 *   （paseo 项目 browser-capture-harness.md 原文），**看不到同窗口下的子 WebContentsView**。
 *   2026-09-17 实测：capturePage 拿到 y=60 是「蓝色」（壳页面），误判成 #41044 回归；
 *   换 desktopCapturer 后 y=60 是「绿色」（子视图），真相才出来。这是「取证方法错 ⇒ 结论错」的实例。
 *
 * 负向对照（--negative）：让内容视图盖到 y=0 盖住标签栏，应观察到 y=20 变绿（barIsRed=false）。
 *   证明断言真的在测「合成顺序」，而不是恒绿假绿（项目铁律：侵入类判据必须位置 + 存在性成对）。
 *
 * 无 GPU 沙箱约束（项目 MEMORY 硬约束）：
 *   · 必须 --no-sandbox --in-process-gpu（只靠 disableHardwareAcceleration 不够）
 *   · desktopCapturer 的 thumbnailSize 必须显式给非零值，否则 thumbnail 为空 → PNG 解码
 *     "unexpected end of file"（2026-09-17 实测踩到）
 *
 * 结果写 %TEMP%\w2a-t0-v4\result.json（GUI 子系统 stdout 不可靠，项目反复踩过的坑）。
 * 退出码：0 = V4 通过；1 = #41044 回归；2 = 无法判定 / 负向对照不通过（含 --negative 正常形态）
 *
 * 用法：pnpm run tabs-v4            # 正向验证 V4
 *       pnpm run tabs-v4-negative   # 负向对照（验证断言有效性）
 */
'use strict'
const { app, BrowserWindow, WebContentsView, desktopCapturer } = require('electron')
const { writeFileSync } = require('node:fs')
const { join } = require('node:path')

const DIR = process.env.W2A_T0_DIR || join(process.env.TEMP, 'w2a-t0-v4')
const BAR_H = 38
// 负向对照：--negative 让内容视图盖住标签栏（y=0），证明断言在测合成顺序而非恒绿
const NEGATIVE = process.argv.includes('--negative')
const out = { electron: process.versions.electron, platform: process.platform, negative: NEGATIVE, steps: [], verdict: null }

// 顶部同步落盘标记（区分「脚本没加载」和「ready 未触发」，项目教训 #14）
try { writeFileSync(join(DIR, 'booted.txt'), 'loaded\n') } catch {}

// 抑制「所有窗口关闭即退出」的默认行为（项目教训 #14：探针里建窗又销毁会退出码 0 静默消失）
app.on('window-all-closed', () => {})

const wait = (ms) => new Promise((r) => setTimeout(r, ms))

// 像素读取（内联，逻辑同 scripts/png-decode.cjs，避免跨进程 require 路径问题）
function decodePng(buf) {
  const zlib = require('node:zlib')
  let off = 8, w = 0, h = 0, colorType = 6
  const idat = []
  while (off + 8 <= buf.length) {
    const len = buf.readUInt32BE(off)
    const type = buf.toString('ascii', off + 4, off + 8)
    const data = buf.subarray(off + 8, off + 8 + len)
    if (type === 'IHDR') { w = data.readUInt32BE(0); h = data.readUInt32BE(4); colorType = data[9] }
    else if (type === 'IDAT') idat.push(data)
    else if (type === 'IEND') break
    off += 12 + len
  }
  const bpp = colorType === 6 ? 4 : colorType === 2 ? 3 : 1
  const raw = zlib.inflateSync(Buffer.concat(idat))
  const stride = w * bpp + 1
  const px = Buffer.alloc(w * h * bpp)
  let prev = Buffer.alloc(w * bpp)
  for (let y = 0; y < h; y++) {
    const ft = raw[y * stride]
    const line = raw.subarray(y * stride + 1, y * stride + 1 + w * bpp)
    const cur = Buffer.alloc(w * bpp)
    for (let i = 0; i < w * bpp; i++) {
      const a = i >= bpp ? cur[i - bpp] : 0, b = prev[i], c = i >= bpp ? prev[i - bpp] : 0
      let v = line[i]
      if (ft === 1) v += a
      else if (ft === 2) v += b
      else if (ft === 3) v += (a + b) >> 1
      else if (ft === 4) { const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c); v += pa <= pb && pa <= pc ? a : pb <= pc ? b : c }
      cur[i] = v & 255
    }
    cur.copy(px, y * w * bpp)
    prev = cur
  }
  return { w, h, bpp, px }
}
const rgbAt = (dec, x, y) => {
  const i = (y * dec.w + x) * dec.bpp
  return [dec.px[i], dec.px[i + 1], dec.px[i + 2]]
}
const near = (c, t, tol = 18) => Math.abs(c[0] - t[0]) <= tol && Math.abs(c[1] - t[1]) <= tol && Math.abs(c[2] - t[2]) <= tol

app.whenReady().then(async () => {
  out.steps.push('ready')
  const win = new BrowserWindow({
    show: true, width: 640, height: 320, frame: false, title: 'W2A_T0_V4_PROBE',
    backgroundColor: '#000000',
    webPreferences: { offscreen: false },
  })
  await wait(300)

  // 壳页面：标签栏红、内容区蓝。data URL 必须 encodeURIComponent，否则 < > " 空格换行会截断 URL 导致页面空白
  const shellHtml = 'data:text/html,' + encodeURIComponent(`<!DOCTYPE html><html style="background:#0000ff;height:100%"><body style="margin:0;background:#0000ff;height:100%">
    <div id="bar" style="position:fixed;left:0;top:0;width:100%;height:${BAR_H}px;background:#ff0000;-webkit-app-region:drag"></div>
  </body></html>`)
  await win.webContents.loadURL(shellHtml).catch((e) => { out.loadErr = String(e) })
  await wait(400)

  // 内容视图：纯绿，从 y=38 起（--negative 时盖到 y=0）
  const view = new WebContentsView({ webPreferences: { offscreen: false } })
  const cb = win.getContentBounds()
  win.contentView.addChildView(view)
  view.setBounds({ x: 0, y: NEGATIVE ? 0 : BAR_H, width: cb.width, height: NEGATIVE ? cb.height : cb.height - BAR_H })
  const contentHtml = 'data:text/html,' + encodeURIComponent(`<!DOCTYPE html><html style="background:#00ff00;height:100%"><body style="margin:0;background:#00ff00;height:100%">CONTENT</body></html>`)
  await view.webContents.loadURL(contentHtml).catch((e) => { out.contentLoadErr = String(e) })
  await wait(500)

  out.children = win.contentView.children.length
  out.viewBounds = view.getBounds()
  out.contentBounds = cb
  out.dpr = await win.webContents.executeJavaScript('window.devicePixelRatio').catch(() => null)

  // 系统级抓屏（最终合成画面），带重试。thumbnailSize 必须显式非零。
  let img = null
  for (let i = 0; i < 3 && !img; i++) {
    try {
      const sources = await desktopCapturer.getSources({ types: ['window'], thumbnailSize: { width: 640, height: 320 } })
      const src = sources.find((s) => s.name === 'W2A_T0_V4_PROBE')
      if (src && src.thumbnail && !src.thumbnail.isEmpty()) img = src.thumbnail
      else out.thumbEmpty = src ? 'thumbnail empty' : 'source not found'
    } catch (e) { out.captureErr = String(e); await wait(400) }
  }

  if (img) {
    const dec = decodePng(img.toPNG())
    out.shotSize = { w: dec.w, h: dec.h }
    // 截图是物理像素，逻辑坐标需乘 dpr（本机 1.5）
    const dpr = out.dpr || 1
    const barY = Math.min(Math.round(20 * dpr), dec.h - 1)
    const contentY = Math.min(Math.round(60 * dpr), dec.h - 1)
    const xs = [0.5, 0.2, 0.8].map((r) => Math.floor(dec.w * r))
    const barSamples = xs.map((x) => rgbAt(dec, x, barY))
    const contentSamples = xs.map((x) => rgbAt(dec, x, contentY))
    out.samples = { dpr, barY, contentY, barSamples, contentSamples }

    const barIsRed = barSamples.every((c) => near(c, [255, 0, 0]))
    const contentIsGreen = contentSamples.every((c) => near(c, [0, 255, 0]))
    const contentIsBlue = contentSamples.every((c) => near(c, [0, 0, 255]))

    out.checks = { barIsRed, contentIsGreen, contentIsBlue }

    if (NEGATIVE) {
      // 负向对照：内容视图盖住标签栏 ⇒ barIsRed 应为 false，证明断言有效
      out.verdict = barIsRed ? 'NEGATIVE_CONTROL_FAIL' : 'NEGATIVE_CONTROL_OK'
    } else if (barIsRed && contentIsGreen && !contentIsBlue) {
      out.verdict = 'V4_PASS'
    } else if (contentIsBlue) {
      out.verdict = 'V4_FAIL_regression41044'
    } else {
      out.verdict = 'V4_INCONCLUSIVE'
    }
  } else {
    out.verdict = 'V4_UNPROVEN_no_capture'
  }

  writeFileSync(join(DIR, 'result.json'), JSON.stringify(out, null, 2))
  const code = out.verdict === 'V4_PASS' ? 0
    : out.verdict === 'NEGATIVE_CONTROL_OK' ? 0
    : out.verdict === 'V4_FAIL_regression41044' ? 1 : 2
  app.exit(code)
}).catch((e) => {
  out.verdict = 'FATAL'
  out.fatal = e && e.stack ? e.stack : String(e)
  try { writeFileSync(join(DIR, 'result.json'), JSON.stringify(out, null, 2)) } catch {}
  app.exit(1)
})
