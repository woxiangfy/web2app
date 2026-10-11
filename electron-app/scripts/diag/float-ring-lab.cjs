/**
 * 悬浮球「透明环带」A/B 像素取证（v1.77b）。
 *
 * 要证明的命题
 * ------------
 * Tauri 真机现象「不选中时 悬浮图标背景异样」= **球与窗口圆形区域之间的那圈透明环带**
 * 把窗口后面的东西透了出来。Tauri 侧 WebView2 的底层不是真透明 ⇒ 透出来的是一层浅色残留；
 * Electron 侧 `transparent: true` 是真透明 ⇒ 同一份几何下什么也看不到（这就是「仅限 Tauri」）。
 *
 * 怎么复刻
 * --------
 * 用 Electron 起一个和球窗**同尺寸**（56×56 DIP）的透明无边框窗，加载**同一份**
 * `webview-app/src/floaticon.html`（两壳的单一真源），然后：
 *   · 把 `html,body` 的 `background` 从 `transparent` 覆盖成一个浅色 —— 这一步就是在模拟
 *     「WebView2 底层不是真透明」：环带里本来什么都没有，现在会漏出这层浅色。
 *   · 按真实几何算出「区域圆」（直径 = 56 DIP，与球心同心）与球，数**区域内、球之外**的
 *     浅色像素。
 *
 * 为什么用 `capturePage()` 而不是系统截屏：与 layout-shot.cjs 同一原因 —— Chromium 走 DWM
 * 合成，系统级截屏拿到的是黑图；而且 `setShape` 是**窗口级**裁剪（OS 那层），capturePage
 * 拿到的是渲染器输出（裁剪之前）。**正是我们要的**：我们就是要看「区域内有多少浅色」。
 *
 * 判据（零容忍）
 * --------------
 *   修前（球 46 DIP）：环带内浅色像素 > 0（真机就是这个形状）
 *   修后（球 56 DIP）：环带内浅色像素 == 0（球铺满区域圆，环带在几何上不存在）
 *
 * 用法：`node scripts/diag/float-ring-lab.cjs`（退出码 0 = 命题成立）
 */
'use strict'
const { spawn } = require('node:child_process')
const { writeFileSync, readFileSync, mkdirSync } = require('node:fs')
const { join } = require('node:path')

const OUT_DIR = join(__dirname, '..', '..', '..', 'tests', 'float-frag-rc')
const FLOAT_HTML = join(__dirname, '..', '..', '..', 'webview-app', 'src', 'floaticon.html')
const pngDecodePath = join(__dirname, 'png-decode.cjs')
const target = join(process.env.TEMP, 'w2a-float-ring-lab.js')

try { mkdirSync(OUT_DIR, { recursive: true }) } catch { /* 已存在 */ }

writeFileSync(
  target,
  `
const { app, BrowserWindow, screen } = require('electron')
app.disableHardwareAcceleration()
app.commandLine.appendSwitch('no-sandbox')
app.commandLine.appendSwitch('in-process-gpu')
const { decodePng } = require(${JSON.stringify(pngDecodePath)})
const fs = require('node:fs')
const path = require('node:path')

const { pathToFileURL } = require('node:url')
const FLOAT_HTML = ${JSON.stringify(FLOAT_HTML)} // 球页（双壳单一真源）
const OUT_DIR = ${JSON.stringify(OUT_DIR)}
const PILLAR = 56 // 活柱边长（= ICON_SIZE_LOGICAL，DIP）

// 模拟「WebView2 底层不是真透明」时漏出来的那层浅色：取真机截图里量到的残片色 (186,199,217)。
const RESIDUE = '#bac7d9'

// 两个对照：
//   before = v1.77b 之前的球页几何（球 46 DIP、图标 30px、悬停 scale）
//   after  = 修好之后（球 56 DIP，铺满活柱）
const VARIANTS = [
  { name: 'before', css: '#w2f-ball{width:46px!important;height:46px!important}#w2f-ball img{width:30px!important;height:30px!important}' },
  { name: 'after',  css: '' }
]

/** 覆盖底色 + 贴变体 CSS，然后回报球的真实几何（DIP）。 */
function measure (win, variant) {
  const css = 'html,body{background:' + RESIDUE + '!important}' +
    '#w2f-ball img{visibility:hidden!important}\\n' + variant.css
  return win.webContents.executeJavaScript([
    '(async () => {',
    '  const st = document.createElement("style");',
    '  st.textContent = ' + JSON.stringify(css) + ';',
    '  document.head.appendChild(st);',
    '  await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));',
    '  const b = document.getElementById("w2f-ball").getBoundingClientRect();',
    '  return JSON.stringify({ ballW: b.width, ballH: b.height, cx: b.x + b.width / 2, cy: b.y + b.height / 2, dpr: devicePixelRatio });',
    '})()'
  ].join('\\n'), true)
}

app.whenReady().then(async () => {
  let code = 1
  try {
    const scale = screen.getPrimaryDisplay().scaleFactor || 1
    const report = []

    for (const variant of VARIANTS) {
      const win = new BrowserWindow({
        width: PILLAR, height: PILLAR, frame: false, show: true,
        transparent: true, resizable: false, skipTaskbar: true,
        // 与真实球窗一致：不抢焦点、不可见时不入任务栏
        focusable: false, hasShadow: false
      })
      try { win.setShape([{ x: 0, y: 0, width: PILLAR, height: PILLAR }]) } catch { /* 降级：无 shape 也照样量区域圆 */ }
      // ⚠️ 必须用 pathToFileURL 生成的 URL：win.loadFile(winPath) 在本机会拼出
      // file:///C:\dev\... （反斜杠没转正斜杠）⇒ ERR_FAILED；data: URL 对这么长的中文
      // 文档同样 ERR_FAILED。直接用 file URL 还能让页内的相对 icon.png 正常解析。
      await win.loadURL(pathToFileURL(FLOAT_HTML).href)
      const geo = JSON.parse(await measure(win, variant))
      await new Promise((r) => setTimeout(r, 400))
      const img = await win.webContents.capturePage()
      const png = img.toPNG()
      const file = path.join(OUT_DIR, 'ring-lab-' + variant.name + '.png')
      fs.writeFileSync(file, png)

      const dec = decodePng(png)
      const s = dec.w / PILLAR // 设备像素 / DIP
      const cx = geo.cx * s
      const cy = geo.cy * s
      const regionR = (PILLAR / 2) * s // 区域圆半径（设备像素）
      let inside = 0
      let ringPx = 0
      for (let y = 0; y < dec.h; y++) {
        for (let x = 0; x < dec.w; x++) {
          const d = Math.hypot(x - cx, y - cy)
          if (d > regionR) continue
          inside++
          const i = (y * dec.w + x) * dec.bpp
          const r = dec.px[i], g = dec.px[i + 1], b = dec.px[i + 2]
          const lum = 0.299 * r + 0.587 * g + 0.114 * b
          // 与真机截图同一判据：够亮 + 低饱和（球的高光是蓝色 b-r 很大，会被排除）
          if (lum >= 120 && b - r < 60) ringPx++
        }
      }
      report.push({ name: variant.name, file, size: dec.w + 'x' + dec.h, s, ballW: geo.ballW, regionR, inside, ringPx })
      win.destroy()
    }

    console.log('INFO 主显示器缩放 = ' + scale)
    for (const r of report) {
      console.log(
        'LAB  变体 ' + r.name.padEnd(6) + ' 球宽=' + r.ballW + ' DIP  图像=' + r.size +
        '  区域圆半径=' + r.regionR.toFixed(1) + 'px  圆内像素=' + r.inside +
        '  其中浅色(低饱和亮)= ' + r.ringPx
      )
      console.log('PNG  ' + r.file)
    }
    const before = report.find((r) => r.name === 'before')
    const after = report.find((r) => r.name === 'after')
    const okBefore = before && before.ringPx > 0
    const okAfter = after && after.ringPx === 0
    console.log('CHECK 修前环带里确有浅色像素（复刻真机现象） ' + (okBefore ? 'OK' : 'BAD'))
    console.log('CHECK 修后环带里浅色像素归零（球铺满区域圆） ' + (okAfter ? 'OK' : 'BAD'))
    code = okBefore && okAfter ? 0 : 1
  } catch (e) {
    console.log('ERR  ' + (e && e.stack ? e.stack : e))
  }
  app.exit(code)
}).catch((e) => { console.log('ERR  ' + (e && e.stack ? e.stack : e)); app.exit(2) })
`,
  'utf8'
)

const electronPath = require('electron')
const env = { ...process.env }
delete env.ELECTRON_RUN_AS_NODE
const child = spawn(electronPath, [target, '--disable-gpu'], { stdio: 'inherit', env })
child.on('close', (code) => process.exit(code ?? 1))
