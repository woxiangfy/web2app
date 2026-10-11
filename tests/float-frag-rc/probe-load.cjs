/** 最小探针：在本沙箱里 Electron 到底能不能加载页面？（定位 ERR_FAILED 的边界） */
'use strict'
const { spawn } = require('node:child_process')
const { writeFileSync } = require('node:fs')
const { join } = require('node:path')
const target = join(process.env.TEMP, 'w2a-probe-load.js')
writeFileSync(
  target,
  `
const { app, BrowserWindow } = require('electron')
const fs = require('node:fs')
const OUT = ${JSON.stringify(join(__dirname, 'probe-load-result.txt'))}
const log = (s) => { try { fs.appendFileSync(OUT, s + '\\n') } catch (e) {} }
try { fs.writeFileSync(OUT, '') } catch (e) {}
process.on('uncaughtException', (e) => { log('UNCAUGHT ' + e.stack); app.exit(9) })
app.disableHardwareAcceleration()
app.commandLine.appendSwitch('no-sandbox')
app.commandLine.appendSwitch('in-process-gpu')
const cases = [
  ['tiny-data', 'data:text/html,<b>hi</b>'],
  ['tiny-file', 'file:///C:/dev/rust/web-app/tests/float-frag-rc/tiny.html'],
  ['file-cn', 'file:///C:/dev/rust/web-app/webview-app/src/floaticon.html'],
  ['about-blank', 'about:blank']
]
app.whenReady().then(async () => {
  log('READY')
  for (const [name, url] of cases) {
    const results = []
    for (const opts of [{ transparent: true, focusable: false }, { transparent: true }, {}]) {
      const w = new BrowserWindow({ width: 56, height: 56, frame: false, show: true, skipTaskbar: true, ...opts })
      let err = ''
      try { await w.loadURL(url) } catch (e) { err = e.message }
      results.push(JSON.stringify(opts) + ' -> ' + (err ? 'ERR ' + err : 'OK doc=' + w.webContents.getURL().slice(0, 60)))
      w.destroy()
    }
    log('CASE ' + name)
    for (const r of results) log('   ' + r)
  }
  app.exit(0)
}).catch((e) => { log('ERR ' + e.stack); app.exit(2) })
`,
  'utf8'
)
const electronPath = require(join(__dirname, '..', '..', 'electron-app', 'node_modules', 'electron'))
const env = { ...process.env }
delete env.ELECTRON_RUN_AS_NODE
const child = spawn(electronPath, [target, '--disable-gpu'], { stdio: 'inherit', env })
child.on('close', (c) => process.exit(c ?? 1))
