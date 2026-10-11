// 取证复刻：按 float-window.ts 生产参数渲染【当前版】floaticon.html（aria-label，无 title），
// 验证「无 title 也冒原生提示条」是否成立（v1.77 线索：残片是 74x38 物理的圆角浅色卡片，
// 不被球窗椭圆裁剪 ⇒ 独立窗口 ⇒ 疑似 tooltip 类）。
// 运行：electron.exe main.cjs（由 run.cmd 驱动）；窗口摆在主屏 (600,400) DIP，40s 后自退。
const { app, BrowserWindow, protocol, net } = require('electron')
const path = require('path')
const fs = require('fs')

const ROOT = path.resolve(__dirname, '..', '..') // -> web-app 仓库根
const PAGE = path.join(ROOT, 'webview-app', 'src', 'floaticon.html')
const ICON = path.join(ROOT, 'webview-app', 'icons', 'icon.png')

// 与 electron-app/src/main/float-layout.ts 同实现（编译产物直接复用，保证区域一致）
const { circleShapeRects, DEFAULT_FLOAT_W, DEFAULT_FLOAT_H } = require(path.join(
  ROOT, 'electron-app', 'out', 'main', 'float-layout.js'))

app.whenReady().then(() => {
  // 与 protocols.ts 同机制：floaticon://localhost/ 本地内置页
  protocol.handle('floaticon', (req) => {
    const u = new URL(req.url)
    if (u.pathname === '/icon.png') {
      return new Response(fs.readFileSync(ICON), { headers: { 'content-type': 'image/png' } })
    }
    return new Response(fs.readFileSync(PAGE), {
      headers: { 'content-type': 'text/html; charset=utf-8' },
    })
  })

  const win = new BrowserWindow({
    width: DEFAULT_FLOAT_W,
    height: DEFAULT_FLOAT_H,
    x: 600,
    y: 400,
    frame: false,
    thickFrame: false,
    transparent: true,
    resizable: false,
    minimizable: false,
    maximizable: false,
    fullscreenable: false,
    skipTaskbar: true,
    alwaysOnTop: true,
    show: false,
    title: 'w2a-frag-probe',
    webPreferences: { contextIsolation: true, nodeIntegration: false, sandbox: true },
  })
  try {
    win.setShape(circleShapeRects(DEFAULT_FLOAT_W))
  } catch (e) {
    console.log('[probe] setShape failed: ' + e.message)
  }
  win.once('ready-to-show', () => {
    win.show()
    console.log('[probe] SHOWN at 600,400 DIP size ' + DEFAULT_FLOAT_W + 'x' + DEFAULT_FLOAT_H)
  })
  win.loadURL('floaticon://localhost/').catch((e) => console.log('[probe] load fail: ' + e.message))
  setTimeout(() => app.quit(), 40000)
})
