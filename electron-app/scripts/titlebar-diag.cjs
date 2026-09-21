/**
 * 标题栏布局诊断：把「标题栏到底铺到哪、被谁挡住」这件事一次性问清楚。
 *
 * 起因：`getBoundingClientRect().width` 报 828（与 innerWidth 相等、断言通过），
 * 但 capturePage 的像素显示深色只画到 812.5 —— 差的正是一条滚动条。
 * 「布局宽度对、可见宽度少一条滚动条」= 裁剪，而不是宽度写错。
 *
 * 所以这里不只量宽度，而是：
 *   ① 实测 `100vw` 在本页到底解析成多少
 *   ② 标题栏的**祖先链**逐级打印 zoom / transform / overflow / contain / filter
 *      （fixed 的包含块一旦被 transform 之类改变，就会同时被那个祖先 overflow 裁剪）
 *   ③ `elementFromPoint` 在 x = 805..828 逐点问「这里是哪个元素」——
 *      直接回答那条 15px 里站的是滚动条、页面内容、还是标题栏自己
 *
 * 用法：`node scripts/titlebar-diag.cjs [--url=<url>] [--width=] [--height=] [--png=out.png]`
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
const target = join(process.env.TEMP, 'w2a-titlebar-diag-inner.js')

const DIAG = `(function () {
  var out = {};
  var bar = document.getElementById('__web2app_titlebar__');
  out.innerWidth = innerWidth;
  out.outerWidth = outerWidth;
  out.clientWidth = document.documentElement.clientWidth;
  out.scrollbarW = innerWidth - document.documentElement.clientWidth;
  out.dpr = devicePixelRatio;
  out.docZoom = getComputedStyle(document.documentElement).zoom;
  out.bodyZoom = document.body ? getComputedStyle(document.body).zoom : null;
  if (bar) {
    var r = bar.getBoundingClientRect();
    out.bar = {
      rectX: Math.round(r.x * 100) / 100,
      rectW: Math.round(r.width * 100) / 100,
      rectRight: Math.round(r.right * 100) / 100,
      offsetWidth: bar.offsetWidth,
      computedWidth: getComputedStyle(bar).width,
      inlineWidth: bar.style.width,
      barZoom: getComputedStyle(bar).zoom,
      parent: bar.parentNode ? bar.parentNode.tagName : null,
      offsetParent: bar.offsetParent ? bar.offsetParent.tagName : null,
      rects: bar.getClientRects().length
    };
    out.chain = [];
    var n = bar;
    while (n && n.nodeType === 1 && out.chain.length < 14) {
      var cs = getComputedStyle(n);
      var cr = n.getBoundingClientRect();
      out.chain.push({
        tag: n.tagName + (n.id ? '#' + n.id : '') + (n.className && typeof n.className === 'string' ? '.' + String(n.className).trim().split(/\\s+/).slice(0, 2).join('.') : ''),
        x: Math.round(cr.x * 100) / 100,
        w: Math.round(cr.width * 100) / 100,
        zoom: cs.zoom,
        transform: cs.transform === 'none' ? '' : cs.transform,
        overflow: cs.overflow,
        overflowX: cs.overflowX,
        contain: cs.contain,
        filter: cs.filter === 'none' ? '' : cs.filter,
        willChange: cs.willChange,
        position: cs.position,
        display: cs.display
      });
      n = n.parentNode;
    }
  }
  // 100vw / 100% 实测：把探针挂在 <html> 下，fixed + absolute 各测一次
  var mk = function (css) {
    var d = document.createElement('div');
    d.style.cssText = 'height:1px;left:0;top:0;visibility:hidden;' + css;
    document.documentElement.appendChild(d);
    var w = Math.round(d.getBoundingClientRect().width * 100) / 100;
    d.remove();
    return w;
  };
  out.probe = {
    fixed100vw: mk('position:fixed;width:100vw;'),
    fixed100pct: mk('position:fixed;width:100%;'),
    abs100vw: mk('position:absolute;width:100vw;'),
    fixedLeftRight0: mk('position:fixed;left:0;right:0;'),
    fixedLeftRight0Body: (function () {
      var d = document.createElement('div');
      d.style.cssText = 'position:fixed;left:0;right:0;top:0;height:1px;visibility:hidden;';
      (document.body || document.documentElement).appendChild(d);
      var w = Math.round(d.getBoundingClientRect().width * 100) / 100;
      d.remove();
      return w;
    })()
  };
  // 逐点问「这里是哪个元素」
  out.elementAt = {};
  var xs = [790, 805, 810, 811, 812, 813, 814, 816, 820, 824, 826, 827, 828];
  for (var i = 0; i < xs.length; i++) {
    var x = xs[i];
    if (x >= innerWidth) { out.elementAt[x] = 'OUT_OF_VIEWPORT'; continue; }
    var e = document.elementFromPoint(x, 20);
    out.elementAt[x] = e ? e.tagName + (e.id ? '#' + e.id : '') + (e.className && typeof e.className === 'string' ? '.' + String(e.className).trim().split(/\\s+/)[0] : '') : 'null';
  }
  // 逐点问「y=20 这条水平线上，从 0 到窗口右边，颜色边界在哪」——用覆盖检测代替取色
  // （注入一个 1px 高的探测条，看它被谁最右裁掉）
  var strip = document.createElement('div');
  strip.style.cssText = 'position:fixed;left:0;top:10px;height:4px;width:100vw;background:rgb(255,0,255);z-index:2147483646;visibility:hidden;';
  document.documentElement.appendChild(strip);
  out.stripRect = Math.round(strip.getBoundingClientRect().width * 100) / 100;
  strip.remove();
  return JSON.stringify(out, null, 1);
})()`

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
const DIAG = ${JSON.stringify(DIAG)}
const WIDTH = ${WIDTH}
const HEIGHT = ${HEIGHT}
const URL_ARG = ${JSON.stringify(URL_ARG)}
const PNG = ${JSON.stringify(join(process.env.TEMP, 'w2a-titlebar-diag.png'))}

app.whenReady().then(async () => {
  const win = new BrowserWindow({ width: WIDTH, height: HEIGHT, frame: false, show: true })
  if (URL_ARG) {
    await win.loadURL(URL_ARG)
  } else {
    const filler = Array.from({ length: 120 }, (_, i) => '<p>第 ' + i + ' 行，用于产生纵向滚动条。</p>').join('')
    const html =
      '<!doctype html><html><head><meta charset="utf-8"><style>' +
      'html,body{margin:0}#hdr{background:#2b6cb0;color:#fff;padding:22px;font:16px system-ui}' +
      'body{font:14px/1.6 system-ui}</style></head><body>' +
      '<div id="hdr">页面自己的页头</div>' + filler + '</body></html>'
    await win.loadURL('data:text/html;charset=utf-8,' + encodeURIComponent(html))
  }
  await win.webContents.executeJavaScript(TITLEBAR_JS, true)
  await new Promise((r) => setTimeout(r, 900))
  const info = await win.webContents.executeJavaScript(DIAG, true)
  console.log('DIAG ' + info)
  require('node:fs').writeFileSync(PNG, (await win.webContents.capturePage()).toPNG())
  console.log('PNG  ' + PNG)
  app.exit(0)
})
`,
  'utf8'
)

const electronPath = require('electron')
const env = { ...process.env }
delete env.ELECTRON_RUN_AS_NODE
const child = spawn(electronPath, [target, '--disable-gpu'], { stdio: 'inherit', env })
child.on('close', (code) => process.exit(code ?? 1))
