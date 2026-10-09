/**
 * 启动页失败卡「到底看不看得见」取证脚本。
 *
 * 为什么需要它：真机上用户只看到一张「启动超时」卡，我们隔着屏幕无法判断
 * 「诊断行是没生成、还是生成了但被推出可视区」。这个脚本把**同一份** splash.html +
 * loading.js 按指定尺寸渲染出来（用真实 Chromium、真实 CSS），把 40s 兜底卡提前到 1.4s，
 * 然后客观测量：
 *   · 卡片是否出现、标题对不对；
 *   · 「进度行」是否排在最前（notes 前置）；
 *   · 「地址」行在首屏的 y 坐标（<0 或 >视口高 = 首屏看不见）；
 *   · 内容是否超出视口、是否**可滚动**、滚动到顶/底后关键行是否进入视口；
 *   · 地址里的 w2a_key 是否被脱敏（脚本故意用带假密钥的 URL 加载）。
 * 附一张 PNG 供人眼确认。
 *
 * ⚠️ 只改两处**测试用**常量（FAIL_MS / SLOW_MS 缩短），其余逻辑逐字用生产代码 ——
 *    走的是真实兜底路径，不是另写一份「模拟卡」。
 * ⚠️ 用 http://127.0.0.1:<port>/start.html?w2a_key=… 加载：既复现「本地代理入口」的
 *    真实地址形态（可验证脱敏），又避免 file:// 对 query 的处理差异。
 * ⚠️ 无 GPU 环境必须 no-sandbox + in-process-gpu（同 layout-shot.cjs，否则渲染进程起不来）。
 *
 * 用法：
 *   node scripts/diag/splash-fail-card.cjs --js=<path/to/loading.js> --w=360 --h=640 --notes=4 --tag=after
 * 产物：PNG 落 %TEMP%\w2a-splash-fail-<tag>.png
 */
const { spawn } = require('node:child_process')
const { writeFileSync, readFileSync } = require('node:fs')
const { join } = require('node:path')

const argv = process.argv.slice(2)
const arg = (name, dflt) => {
  const hit = argv.find((a) => a.startsWith(`--${name}=`))
  return hit ? hit.slice(name.length + 3) : dflt
}

const ROOT = join(__dirname, '..', '..', '..')
const JS_PATH = arg('js', join(ROOT, 'webview-app', 'src', 'loading.js'))
const W = Number(arg('w', 360))
const H = Number(arg('h', 640))
const NOTES = Number(arg('notes', 4))
const TAG = arg('tag', 'run')

// —— 组装被测页面：与壳侧同样的三处占位符替换 ——
let loadingJs = readFileSync(JS_PATH, 'utf8')
const splash = readFileSync(join(ROOT, 'webview-app', 'src', 'splash.html'), 'utf8')

const FAIL_RE = /var FAIL_MS = IS_SPLASH \? 40000 : 30000;/
const SLOW_RE = /var SLOW_MS = IS_SPLASH \? 5000 : 8000;/
if (!FAIL_RE.test(loadingJs) || !SLOW_RE.test(loadingJs)) {
  console.log('ERR  未找到 FAIL_MS / SLOW_MS 常量行 —— loading.js 结构已变，请同步本脚本')
  process.exit(2)
}
// 仅缩短时间常量（测试用），其余一字不动
loadingJs = loadingJs
  .replace(FAIL_RE, 'var FAIL_MS = IS_SPLASH ? 1400 : 30000;')
  .replace(SLOW_RE, 'var SLOW_MS = IS_SPLASH ? 300 : 8000;')

const pageHtml = splash
  .split('__WEB2APP_TITLE__').join('splash-probe')
  .split('__WEB2APP_TITLEBAR_JS__').join('/* probe: 不注入标题栏，聚焦失败卡布局 */')
  .split('__WEB2APP_LOADING_JS__').join(loadingJs)

const NOTES_JSON = JSON.stringify([
  '正在打开 example.com…',
  '已发起导航，等待页面响应…',
  '可达性探测：HTTP 200，187ms（网络通，问题在 WebView 内核侧）',
  '提示：本行用于验证「进度行是否排在卡面最前」'
].slice(0, NOTES))

const inner = `
const { app, BrowserWindow } = require('electron')
const http = require('node:http')
const fs = require('node:fs')
const path = require('node:path')
// 无 GPU 环境必须这一组开关（见 layout-shot.cjs 的说明）
app.disableHardwareAcceleration()
app.commandLine.appendSwitch('no-sandbox')
app.commandLine.appendSwitch('in-process-gpu')

const W = ${W}, H = ${H}, TAG = ${JSON.stringify(TAG)}
const PAGE = ${JSON.stringify(pageHtml)}
const NOTES = ${NOTES_JSON}
const PNG = path.join(process.env.TEMP, 'w2a-splash-fail-' + TAG + '.png')
const SECRET = 'LEAKTEST_SECRET_9f3a'

const MEASURE = \`(() => {
  const r = document.getElementById('__web2app_loading__');
  const d = r && r.querySelector('.w2l-detail');
  const t = r && r.querySelector('.w2l-title');
  const cs = r ? getComputedStyle(r) : null;
  const txt = d ? d.textContent : '';
  // 用 Range 精确测某一行在**当前**视口里的 y（比整块 rect 更能回答「这行看得见吗」）
  function lineY(needle) {
    if (!d || !d.firstChild) return null;
    const i = txt.indexOf(needle);
    if (i < 0) return null;
    const rng = document.createRange();
    try { rng.setStart(d.firstChild, i); rng.setEnd(d.firstChild, Math.min(i + needle.length, txt.length)); }
    catch (e) { return null; }
    const rc = rng.getBoundingClientRect();
    return Math.round(rc.top);
  }
  const out = {
    innerW: innerWidth, innerH: innerHeight,
    cardFound: !!r,
    title: t ? t.textContent : null,
    detailLen: txt.length,
    detailHead: txt.slice(0, 70),
    notesFirst: txt.indexOf('正在打开') === 0,
    addrRedacted: txt.indexOf('w2a_key=***') >= 0,
    addrLeaked: txt.indexOf('LEAKTEST_SECRET_9f3a') >= 0,
    hasAddrLine: txt.indexOf('地址:') >= 0,
    overflowY: cs ? cs.overflowY : null,
    justify: cs ? cs.justifyContent : null,
    userSelect: cs ? (cs.webkitUserSelect || cs.userSelect) : null,
    scrollH: r ? r.scrollHeight : 0,
    clientH: r ? r.clientHeight : 0,
    overflowPx: r ? r.scrollHeight - r.clientHeight : 0,
    addrYFirstScreen: lineY('地址:'),
    uaLen: (navigator.userAgent || '').length
  };
  if (r) { r.scrollTop = 999999; }
  out.scrollTopAfter = r ? Math.round(r.scrollTop) : 0;
  out.addrYAfterScroll = lineY('地址:');
  out.detailBottomAfterScroll = d ? Math.round(d.getBoundingClientRect().bottom) : null;
  return JSON.stringify(out);
})()\`

app.whenReady().then(() => {
  let code = 1
  // 用本地 http 服务提供页面：地址形如 http://127.0.0.1:<port>/start.html?w2a_key=<key>
  const srv = http.createServer((req, res) => {
    res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' })
    res.end(PAGE)
  })
  srv.listen(0, '127.0.0.1', async () => {
    const port = srv.address().port
    const url = 'http://127.0.0.1:' + port + '/start.html?w2a_key=' + SECRET
    console.log('URL  ' + url)
    try {
      const win = new BrowserWindow({ width: W, height: H, frame: false, show: true })
      await win.loadURL(url)
      // 立刻推「进度行」（模拟壳侧 eval 推送），必须赶在 1.4s 兜底卡之前
      for (const n of NOTES) {
        await win.webContents.executeJavaScript(
          'try{window.__web2appLoading&&window.__web2appLoading.note(' + JSON.stringify(n) + ');}catch(e){}', true)
      }
      await new Promise((r) => setTimeout(r, 2000)) // 等兜底卡出现（FAIL_MS=1400）
      const info = await win.webContents.executeJavaScript(MEASURE, true)
      console.log('MEASURE ' + info)
      const m = JSON.parse(info)
      const vis = m.addrYFirstScreen !== null && m.addrYFirstScreen >= 0 && m.addrYFirstScreen < m.innerH
      console.log('CHECK 卡片出现 ' + (m.cardFound ? 'OK' : 'BAD') + '（标题=「' + m.title + '」）')
      console.log('CHECK 进度行前置 ' + (m.notesFirst ? 'OK' : 'BAD') + '（detail 开头=「' + m.detailHead.slice(0, 24) + '…」）')
      console.log('CHECK 地址脱敏 ' + ((m.addrRedacted && !m.addrLeaked) ? 'OK' : 'BAD') +
        '（含 w2a_key=*** =' + m.addrRedacted + '，泄漏明文 =' + m.addrLeaked + '）')
      console.log('CHECK 可滚动 ' + (m.overflowY === 'auto' ? 'OK' : 'BAD') +
        '（overflowY=' + m.overflowY + '，justify=' + m.justify + '，userSelect=' + m.userSelect + '）')
      console.log('INFO  内容高=' + m.scrollH + ' 视口高=' + m.clientH + ' 超出=' + m.overflowPx + 'px' +
        '（UA 长度 ' + m.uaLen + '）')
      console.log('INFO  「地址」行首屏 y=' + m.addrYFirstScreen + '（视口 ' + m.innerH + '）⇒ 首屏' +
        (m.addrYFirstScreen === null ? '无该行' : (vis ? '可见' : '不可见')))
      console.log('INFO  滚到底 scrollTop=' + m.scrollTopAfter + '，此时「地址」行 y=' + m.addrYAfterScroll +
        '，detail 底部=' + m.detailBottomAfterScroll)
      const img = await win.webContents.capturePage().catch((e) => {
        console.log('WARN capturePage 失败（无 GPU 环境常见）：' + (e && e.message)); return null
      })
      if (img && !img.isEmpty()) { fs.writeFileSync(PNG, img.toPNG()); console.log('PNG  ' + PNG) }
      code = m.cardFound ? 0 : 1
    } catch (e) {
      console.log('ERR  ' + (e && e.message ? e.message : e))
    }
    try { srv.close() } catch (e) { /* noop */ }
    app.exit(code)
  })
}).catch((e) => { console.log('ERR  ' + (e && e.message ? e.message : e)); app.exit(2) })
`

const target = join(process.env.TEMP, `w2a-splash-fail-inner-${TAG}.js`)
writeFileSync(target, inner, 'utf8')

const electronPath = require('electron')
const env = { ...process.env }
delete env.ELECTRON_RUN_AS_NODE // 否则 electron 可执行文件退化成 node 模式
const child = spawn(electronPath, [target, '--disable-gpu'], { stdio: 'inherit', env })
child.on('close', (c) => process.exit(c ?? 1))
