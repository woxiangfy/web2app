/**
 * 滚动条实验室 v2：滚动条必须**保留可见**，但不得侵入标题栏那 38px。
 *
 * 已查明的前提（2026-09-17 像素取证）：
 *   Chromium 把**视口（根）滚动条**绘制在所有 DOM 内容之上 —— 标题栏 `z-index:2147483647`
 *   也压不住；且它的几何就是「视口右缘 × 整个视口高」，没有任何 CSS 能把它往下缩。
 *   所以「标题栏盖住滚动条」在注入式架构里做不到，只能反过来：让**滚动的那块矩形**
 *   自己从 y=38 开始。
 *
 * 上一版把滚动条宽度归零（scrollbar-width:none）—— 缝没了，但滚动条也消失了，使用者不要这个。
 * 本 lab 用**逐像素**（不再信 getBoundingClientRect，它已两次给出错误结论）比较候选做法：
 *
 *   base      基线：原生滚动条                              → 预期侵入 y<38
 *   insetA    `::-webkit-scrollbar{border-top:38px}`         → 看 border 是否让 track/thumb 下移
 *   insetB    `::-webkit-scrollbar-track{border-top:38px}`   → 同上，另一处
 *   bodyroll  html 不滚、body 当滚动容器（盒子从 38px 起）  → 滚动条物理上不可能高于 38
 *
 * ⚠️ 三个候选的 track/thumb 都染成**刺眼的红/蓝**，这样纵向颜色剖面能一眼看出
 *    「哪一段在画滚动条」，不需要靠猜近似灰。
 *
 * 用法：`node scripts/scrollbar-lab.cjs [--url=<url>] [--width=] [--height=]`
 */
const { spawn } = require('node:child_process')
const { writeFileSync, readFileSync } = require('node:fs')
const { join } = require('node:path')

const argv = process.argv.slice(2)
const arg = (name, dflt) => {
  const hit = argv.find((a) => a.startsWith(`--${name}=`))
  return hit ? hit.slice(name.length + 3) : dflt
}
const WIDTH = Number(arg('width', 829))
const HEIGHT = Number(arg('height', 438))
const URL_ARG = arg('url', '')

const titlebarJs = readFileSync(join(__dirname, '..', '..', 'webview-app', 'src', 'titlebar.js'), 'utf8')

const SCROLLBAR_RESET =
  'html{scrollbar-width:auto!important;scrollbar-color:auto!important}' +
  'html::-webkit-scrollbar-button{display:none!important;width:0!important;height:0!important}'

const MODES = [
  { key: 'base', desc: '基线：原生滚动条（预期侵入标题栏）', css: '' },
  {
    key: 'insetA',
    desc: '::-webkit-scrollbar 加 border-top:38px',
    css:
      SCROLLBAR_RESET +
      'html::-webkit-scrollbar{width:15px!important;background:transparent!important;border-top:38px solid transparent!important}' +
      'html::-webkit-scrollbar-track{background:#ff0000!important}' +
      'html::-webkit-scrollbar-thumb{background:#0000ff!important}'
  },
  {
    key: 'insetB',
    desc: '::-webkit-scrollbar-track 加 border-top:38px',
    css:
      SCROLLBAR_RESET +
      'html::-webkit-scrollbar{width:15px!important;background:transparent!important}' +
      'html::-webkit-scrollbar-track{background:#ff0000!important;border-top:38px solid transparent!important}' +
      'html::-webkit-scrollbar-thumb{background:#0000ff!important}'
  },
  {
    key: 'bodyroll',
    desc: 'html 不滚，body 当滚动容器（盒子从 38px 起）',
    css:
      'html{padding-top:0!important;overflow:hidden!important;scrollbar-width:none!important}' +
      'html::-webkit-scrollbar{width:0!important;height:0!important}' +
      'body{margin-top:38px!important;height:calc(100vh - 38px)!important;overflow-y:auto!important;overflow-x:hidden!important}' +
      'body::-webkit-scrollbar{width:15px!important;background:transparent!important}' +
      'body::-webkit-scrollbar-track{background:#ff0000!important}' +
      'body::-webkit-scrollbar-thumb{background:#0000ff!important}'
  }
]

const FIXTURE = (() => {
  const filler = Array.from({ length: 120 }, (_, i) => '<p>第 ' + i + ' 行，用于产生纵向滚动条。</p>').join('')
  return (
    '<!doctype html><html><head><meta charset="utf-8"><style>' +
    'html,body{margin:0}#hdr{background:#2b6cb0;color:#fff;padding:22px;font:16px system-ui}' +
    'body{font:14px/1.6 system-ui}</style></head><body>' +
    '<div id="hdr">页面自己的页头</div>' + filler + '</body></html>'
  )
})()

// 采集：几何 + 命中测试。`elementFromPoint` 落在滚动条上返回 null —— 这是「此处有滚动条」
// 最直接的判据（滚动条不是 DOM 元素，绕不过去，也不用猜颜色）。
const measureJs = `
(function () {
  var de = document.documentElement;
  var bar = document.getElementById('__web2app_titlebar__');
  var inBar = function (e) { return !!(e && e.closest && e.closest('#__web2app_titlebar__')); };
  var W = innerWidth;
  var probe = function (y) {
    var e = document.elementFromPoint(W - 1, y);
    if (e === null) return 'null';
    return inBar(e) ? 'BAR' : (e.tagName || '?');
  };
  var out = {
    innerWidth: innerWidth,
    innerHeight: innerHeight,
    clientWidth: de.clientWidth,
    sbw: innerWidth - de.clientWidth,
    dpr: devicePixelRatio,
    docElOverflow: getComputedStyle(de).overflow,
    docElScrollbarWidth: getComputedStyle(de).scrollbarWidth,
    rootScrollable: de.scrollHeight > de.clientHeight,
    bodyClientW: document.body ? document.body.clientWidth : null,
    bodyScrollable: document.body ? document.body.scrollHeight > document.body.clientHeight : null,
    bodyOverflow: document.body ? getComputedStyle(document.body).overflowY : null,
    barRectW: bar ? Math.round(bar.getBoundingClientRect().width * 100) / 100 : null,
    // 命中测试：y=20 在标题栏内、y=60 在标题栏下
    hitY20: probe(20),
    hitY60: probe(60),
    // 从 y=0 往下找第一个「命中为 null」的 y —— 滚动条区段的起点
    nullFrom: (function () { for (var y = 0; y < 160; y++) { if (probe(y) === 'null') return y; } return -1; })(),
    // 标题栏右侧末端能否点到（close 按钮在最右，被滚动条盖住就点不到）
    barHitRight: (function () { for (var x = W - 1; x >= 0; x--) { if (inBar(document.elementFromPoint(x, 20))) return x; } return -1; })()
  };
  return JSON.stringify(out);
})()
`

function runMode(mode) {
  return new Promise((resolve) => {
    const target = join(process.env.TEMP, 'w2a-lab2-' + mode.key + '.js')
    writeFileSync(
      target,
      `
const { app, BrowserWindow } = require('electron')
app.disableHardwareAcceleration()
const PROXY = process.env.W2A_SHOT_PROXY || process.env.HTTPS_PROXY || process.env.https_proxy
if (PROXY) app.commandLine.appendSwitch('proxy-server', PROXY)
app.commandLine.appendSwitch('no-sandbox')
app.commandLine.appendSwitch('in-process-gpu')
const TITLEBAR_JS = ${JSON.stringify(titlebarJs)}
const EXTRA_CSS = ${JSON.stringify(mode.css)}
const WIDTH = ${WIDTH}
const HEIGHT = ${HEIGHT}
const URL_ARG = ${JSON.stringify(URL_ARG)}
const FIXTURE = ${JSON.stringify(FIXTURE)}

app.whenReady().then(async () => {
  const win = new BrowserWindow({ width: WIDTH, height: HEIGHT, frame: false, show: true })
  if (URL_ARG) await win.loadURL(URL_ARG)
  else await win.loadURL('data:text/html;charset=utf-8,' + encodeURIComponent(FIXTURE))
  await win.webContents.executeJavaScript(TITLEBAR_JS, true)
  if (EXTRA_CSS) {
    await win.webContents.executeJavaScript(
      '(function(){try{var s=new CSSStyleSheet();s.replaceSync(' + JSON.stringify(EXTRA_CSS) +
      ');document.adoptedStyleSheets=document.adoptedStyleSheets.concat([s]);}catch(e){' +
      'var t=document.createElement("style");t.textContent=' + JSON.stringify(EXTRA_CSS) +
      ';document.documentElement.appendChild(t);}})()',
      true
    )
  }
  await new Promise((r) => setTimeout(r, 900))

  const dom = JSON.parse(await win.webContents.executeJavaScript(${JSON.stringify(measureJs)}, true))

  // ---- 像素取证：纵向颜色剖面（右端 8px 那一列，从 y=0 一路到 y=160）----
  let px = {}
  try {
    const img = await win.webContents.capturePage()
    const size = img.getSize()
    const bmp = img.toBitmap() // BGRA
    const scale = size.width / dom.innerWidth
    const at = (x, y) => {
      const i = (Math.min(size.height - 1, y) * size.width + Math.min(size.width - 1, x)) * 4
      return { r: bmp[i + 2], g: bmp[i + 1], b: bmp[i] }
    }
    const fmt = (c) => 'rgb(' + c.r + ',' + c.g + ',' + c.b + ')'
    const colX = Math.round((dom.innerWidth - 8) * scale) // 必在滚动条竖条内
    px.scale = Math.round(scale * 1000) / 1000
    px.png = size.width + 'x' + size.height
    px.barColor = fmt(at(Math.round(4 * scale), Math.round(19 * scale)))  // 标题栏基准色（左端内侧）
    px.profile = []
    for (let y = 0; y <= 160; y += 8) {
      px.profile.push('y' + y + '=' + fmt(at(colX, Math.round(y * scale))))
    }
    // 判定：标题栏高度内（y<38）右端列是否出现「非标题栏色」
    const near = (c, h) =>
      Math.abs(c.r - h.r) <= 12 && Math.abs(c.g - h.g) <= 12 && Math.abs(c.b - h.b) <= 12
    const bar = at(Math.round(4 * scale), Math.round(19 * scale))
    let intrude = -1
    for (let y = 0; y < 38; y++) {
      const c = at(colX, Math.round(y * scale))
      if (!near(c, bar)) { intrude = y; break }
    }
    px.intrudeFromY = intrude // -1 = 标题栏那条 38px 干净
    px.hasScrollbarBelow = !near(at(colX, Math.round(80 * scale)), bar)
  } catch (e) {
    px = { error: String(e && e.message ? e.message : e) }
  }

  console.log('RESULT ' + JSON.stringify({ mode: ${JSON.stringify(mode.key)}, desc: ${JSON.stringify(mode.desc)}, dom: dom, px: px }))
  app.exit(0)
}).catch((e) => { console.log('ERR ' + String(e)); app.exit(2) })
`,
      'utf8'
    )

    const env = { ...process.env }
    delete env.ELECTRON_RUN_AS_NODE
    const child = spawn(require('electron'), [target, '--disable-gpu'], { stdio: ['ignore', 'pipe', 'pipe'], env })
    let buf = ''
    child.stdout.on('data', (d) => (buf += d.toString()))
    child.stderr.on('data', (d) => (buf += d.toString()))
    const timer = setTimeout(() => {
      child.kill()
      resolve({ mode: mode.key, desc: mode.desc, error: 'timeout 120s' })
    }, 120000)
    child.on('close', () => {
      clearTimeout(timer)
      const line = buf.split(/\r?\n/).find((l) => l.startsWith('RESULT '))
      if (line) {
        try {
          resolve(JSON.parse(line.slice(line.indexOf('{'))))
        } catch (e) {
          resolve({ mode: mode.key, desc: mode.desc, error: 'bad json: ' + line.slice(0, 200) })
        }
      } else {
        const err = buf.split(/\r?\n/).find((l) => l.startsWith('ERR '))
        resolve({ mode: mode.key, desc: mode.desc, error: err || 'no RESULT line' })
      }
    })
  })
}

;(async () => {
  const results = []
  for (const m of MODES) {
    process.stdout.write(`跑 ${m.key} … `)
    const r = await runMode(m)
    results.push(r)
    console.log(r.error ? 'ERR ' + r.error : 'ok')
  }

  const pad = (s, n) => String(s).padEnd(n)
  console.log('\n================ 对照表 ================')
  console.log(
    pad('模式', 10) + pad('innerW', 8) + pad('clientW', 9) + pad('滚动条宽', 10) +
    pad('侵入起点', 10) + pad('滚动条起点', 11) + pad('条内末端', 10) + '说明'
  )
  for (const r of results) {
    if (r.error) {
      console.log(pad(r.mode, 10) + pad('-', 8) + pad('-', 9) + pad('-', 10) + pad('-', 10) + pad('-', 11) + pad('-', 10) + r.desc + '  [' + r.error + ']')
      continue
    }
    const d = r.dom
    const p = r.px || {}
    console.log(
      pad(r.mode, 10) + pad(d.innerWidth, 8) + pad(d.clientWidth, 9) + pad(d.sbw, 10) +
      pad(p.intrudeFromY === -1 ? '无 ✓' : 'y=' + p.intrudeFromY, 10) +
      pad(d.nullFrom === -1 ? '无' : 'y=' + d.nullFrom, 11) +
      pad(d.barHitRight, 10) + r.desc
    )
  }

  console.log('\n--- 明细 ---')
  for (const r of results) {
    if (r.error) continue
    const d = r.dom
    const p = r.px || {}
    console.log(`[${r.mode}] overflow=${d.docElOverflow} scrollbarWidth=${d.docElScrollbarWidth} rootScrollable=${d.rootScrollable}`)
    console.log(`         body: overflowY=${d.bodyOverflow} clientW=${d.bodyClientW} scrollable=${d.bodyScrollable}`)
    console.log(`         命中 y=20 → ${d.hitY20}   y=60 → ${d.hitY60}   barRectW=${d.barRectW}`)
    console.log(`         标题栏基准色 ${p.barColor}  PNG ${p.png}@${p.scale}`)
    if (p.profile) console.log('         剖面 ' + p.profile.join(' '))
  }
})()
