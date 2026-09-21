/**
 * 布局取证脚本：把一个真实页面（含滚动条）加载起来、注入 titlebar.js，然后
 *   ① 打印标题栏 / 滚动条 / 视口 的对照数字
 *   ② 「滚动条不得侵入标题栏」的**直接判据**：标题栏那 38px 里，窗口最右一列必须每一点都命中标题栏
 *   ③ 可选截图（`%TEMP%\w2a-layout-shot.png`），人眼确认
 *
 * 为什么需要它：这类「自绘标题栏 + 远程页滚动条」的错位问题，**断言能过而人眼觉得不对**，
 * 反过来也有「截图一看就知道、但断言写不出来」的情况。有这个脚本，改完布局 5 秒就能看到结果，
 * 不必经历「打包 → 双击 → 拖窄窗口 → 截图」这一圈。
 *
 * ⚠️⚠️ 判定指标**不能用 `getBoundingClientRect().width`**。2026-09-17 查明：Chromium 把
 * 视口（根）滚动条绘制在所有 DOM 内容之上，标题栏的 rect.width 一直是 innerWidth（828）却仍被
 * 盖掉右端 15px —— 那条断言因此把缺陷当成了「期望值」，害我们误判了整整两轮。
 * 现在用三条**能感受到覆盖 / 能感受到存在**的指标：
 *   · `intrudeRows`：x=innerWidth-1 这一列逐点 y=0..37 做 elementFromPoint，**必须全部命中标题栏**
 *   · `bodyScrollbarW = body.offsetWidth - body.clientWidth > 0`：滚动条**必须还在**，且归 body 管
 *   · `scrollYForwarded`：`window.scrollTo(0,200)` 之后 `window.scrollY` 要真的动了（转发没坏）
 *
 * ⚠️ 为什么用 `webContents.capturePage()` 而不是系统截屏：WebView2/Chromium 走 DWM 合成，
 * 系统级 `CopyFromScreen` / `PrintWindow` 会拿到黑图或下层窗口；capturePage 是渲染器自己出的图。
 * 但它在无 GPU 环境会抛 `UnknownVizError` —— 所以截图只作附加信息，且失败不能影响退出码
 * （否则调用方会挂住不返回）。
 *
 * 用法：`pnpm run shot`（可选：`--width=<n>` / `--height=<n>` / `--url=<http(s)://…>` / `--no-shot`）
 */
const { spawn } = require('node:child_process')
const { writeFileSync, readFileSync } = require('node:fs')
const { join } = require('node:path')

const argv = process.argv.slice(2)
const arg = (name, dflt) => {
  const hit = argv.find((a) => a.startsWith(`--${name}=`))
  return hit ? hit.slice(name.length + 3) : dflt
}
const WIDTH = Number(arg('width', 520))
const HEIGHT = Number(arg('height', 380))
const URL_ARG = arg('url', '')
const NO_SHOT = argv.includes('--no-shot')

const titlebarJs = readFileSync(join(__dirname, '..', '..', 'webview-app', 'src', 'titlebar.js'), 'utf8')
const pngDecodePath = join(__dirname, 'png-decode.cjs')
const target = join(process.env.TEMP, 'w2a-layout-shot-inner.js')

writeFileSync(
  target,
  `
const { app, BrowserWindow } = require('electron')
// ⚠️ 无 GPU 环境（容器 / 远程桌面 / 部分虚拟机）Chromium 会直接以
// "GPU process isn't usable. Goodbye." 退出，脚本连窗口都拿不到 —— 与主程序 index.ts 同一处理。
app.disableHardwareAcceleration()
// 沙箱 / 公司内网里浏览器联网走的是 HTTP(S)_PROXY 环境变量，而 Chromium 只读系统代理设置
// （IE 选项），直连必然 ERR_FAILED —— 取证脚本得把环境变量里的代理显式转过去。
const PROXY = process.env.W2A_SHOT_PROXY || process.env.HTTPS_PROXY || process.env.https_proxy
if (PROXY) app.commandLine.appendSwitch('proxy-server', PROXY)
// ⚠️ 无 GPU / 容器沙箱里，Chromium 的 GPU 子进程会反复崩溃并把整个进程带走
// （"GPU process isn't usable. Goodbye."），连 data: URL 都加载不了（ERR_FAILED），
// 现象看着像「网络不通」，其实是渲染进程死了。实测只有这一组开关能活下来：
// in-process-gpu 单独用照样崩，必须同时 no-sandbox。仅本取证脚本使用，主程序不开 no-sandbox。
app.commandLine.appendSwitch('no-sandbox')
app.commandLine.appendSwitch('in-process-gpu')
const { decodePng, rightmostDark } = require(${JSON.stringify(pngDecodePath)})
const fs = require('node:fs')
const path = require('node:path')
const TITLEBAR_JS = ${JSON.stringify(titlebarJs)}
const WIDTH = ${WIDTH}
const HEIGHT = ${HEIGHT}
const URL_ARG = ${JSON.stringify(URL_ARG)}
const NO_SHOT = ${NO_SHOT}
const PNG = path.join(process.env.TEMP, 'w2a-layout-shot.png')

// 采集脚本：注意是 **async**（window.scrollTo 之后的 scroll 事件是异步派发的，
// 同步读会永远读到「没触发」；executeJavaScript 会等这个 Promise 的 resolve 值）。
const MEASURE = [
  '(async () => {',
  '  const de = document.documentElement;',
  '  const b = document.body;',
  '  const bar = document.getElementById("__web2app_titlebar__");',
  '  const hitBar = (x, y) => { const e = document.elementFromPoint(x, y);',
  '    return !!(e && e.closest && e.closest("#__web2app_titlebar__")); };',
  '  const out = {',
  '    innerWidth: innerWidth, innerHeight: innerHeight,',
  '    clientWidth: de.clientWidth,',
  '    titlebarW: bar ? bar.getBoundingClientRect().width : null,',
  '    titlebarH: bar ? bar.getBoundingClientRect().height : null,',
  // 判据一（侵入）：标题栏自己的 38px 里，窗口最右一列必须**全部命中标题栏**。
  // 根滚动条曾经就压在这一列上（命中返回 null），现在滚动条归 body 管、只在 38px 以下绘制。
  '    intrudeRows: (() => { const bad = []; for (let y = 0; y < 38; y++) {',
  '      if (hitBar(innerWidth - 1, y)) continue;',
  '      const e = document.elementFromPoint(innerWidth - 1, y);',
  '      bad.push(y + (e ? ":" + e.tagName : ":null")); } return bad.slice(0, 8); })(),',
  // 判据二（存在性）：滚动条必须在，而且是**自己占宽一条**的那种（不是被隐藏掉、也不是 overlay）
  '    bodyScrollbarW: b.offsetWidth - b.clientWidth,',
  '    bodyOverflowY: getComputedStyle(b).overflowY,',
  '    rootOverflowY: getComputedStyle(de).overflowY,',
  '    htmlPadTop: getComputedStyle(de).paddingTop,',
  '    bodyMarginTop: getComputedStyle(b).marginTop,',
  '    bodyH: Math.round(b.getBoundingClientRect().height),',
  '    bodyScrollable: b.scrollHeight > b.clientHeight,',
  '    rootScrollable: de.scrollHeight > de.clientHeight,',
  '    dpr: devicePixelRatio',
  '  };',
  // 判据三（转发）：body 当滚动容器后，window 级滚动 API 必须仍然可用
  '  try {',
  '    let fired = 0;',
  '    const h = () => { fired++; };',
  '    window.addEventListener("scroll", h, { passive: true });',
  '    window.scrollTo(0, 200);',
  '    await new Promise((r) => setTimeout(r, 150));',
  '    out.scrollYForwarded = Math.round(window.scrollY);',
  '    out.scrollEventFired = fired > 0;',
  '    window.removeEventListener("scroll", h);',
  '    window.scrollTo(0, 0);',
  '    await new Promise((r) => setTimeout(r, 80));',
  '  } catch (e) { out.scrollYForwarded = -1; out.scrollEventFired = false; }',
  '  return JSON.stringify(out, null, 1);',
  '})()'
].join('\\n')

app.whenReady().then(async () => {
  let code = 1
  try {
    const win = new BrowserWindow({ width: WIDTH, height: HEIGHT, frame: false, show: true })
    if (URL_ARG) {
      await win.loadURL(URL_ARG)
    } else {
      // 内建夹具：自带页头 + 足够长的正文 ⇒ 必然出现纵向滚动条。
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

    const info = await win.webContents.executeJavaScript(MEASURE, true)
    console.log('LAYOUT ' + info)
    const m = JSON.parse(info)

    // 判据一：标题栏那 38px 里不许有滚动条（右端一整列逐点命中标题栏）
    const clean = m.intrudeRows.length === 0
    // 判据二：滚动条必须还在（占宽一条），且归 body 管
    const sbPresent = m.bodyScrollbarW >= 8 && m.bodyOverflowY === 'auto' && m.rootOverflowY === 'hidden'
    // 判据三：滚动能力与 window 级 API 转发
    const scrollable = m.bodyScrollable === true
    const yOk = m.scrollYForwarded >= 150 && m.scrollEventFired === true

    console.log('CHECK 标题栏 38px 内无滚动条 ' + (clean ? 'OK' : 'BAD') +
      '（intrudeRows=' + JSON.stringify(m.intrudeRows) + '）')
    console.log('CHECK 滚动条仍可见且归 body 管 ' + (sbPresent ? 'OK' : 'BAD') +
      '（body 滚动条宽=' + m.bodyScrollbarW + '，body overflowY=' + m.bodyOverflowY + '，html overflowY=' + m.rootOverflowY + '）')
    console.log('CHECK 滚动能力保留 ' + (scrollable ? 'OK' : 'BAD') +
      '（body.scrollHeight > clientHeight：' + m.bodyScrollable + '）')
    console.log('CHECK window 级滚动 API 仍可用 ' + (yOk ? 'OK' : 'BAD') +
      '（window.scrollTo(0,200) → window.scrollY=' + m.scrollYForwarded + '，scroll 事件=' +
      (m.scrollEventFired ? '触发' : '未触发') + '）')
    console.log('INFO  body: marginTop=' + m.bodyMarginTop + ' 高=' + m.bodyH + '（视口 ' + m.innerHeight +
      ' → 期望 ' + (m.innerHeight - 38) + '）  html: paddingTop=' + m.htmlPadTop)
    if (Math.abs(m.bodyH - (m.innerHeight - 38)) > 2) {
      console.log('WARN body 高度 ' + m.bodyH + ' 与「视口 - 38 = ' + (m.innerHeight - 38) + '」不符')
    }
    if (m.innerWidth - m.titlebarW > 1) {
      console.log('NOTE rect.width 与 innerWidth 差 ' + (m.innerWidth - m.titlebarW) + 'px —— 这才是「rect 会撒谎」的现场')
    }

    // 附加：截图 + 像素复核（无 GPU 环境 capturePage 可能抛 UnknownVizError，绝不能影响退出码）
    if (!NO_SHOT) {
      const img = await win.webContents.capturePage().catch((e) => {
        console.log('WARN capturePage 失败（无 GPU 环境常见，不影响判定）：' + (e && e.message))
        return null
      })
      if (img && !img.isEmpty()) {
        const png = img.toPNG()
        fs.writeFileSync(PNG, png)
        console.log('PNG  ' + PNG)
        try {
          const dec = decodePng(png)
          const scale = dec.w / m.innerWidth
          // 标题栏深色主题下：右端可见边界应在窗口最右缘；顺带看标题栏那条里有没有别的颜色
          const row = Math.round(2 * scale)
          const right = rightmostDark(dec, row, 90)
          console.log('PIXEL ' + dec.w + 'x' + dec.h + '@' + Math.round(scale * 100) / 100 +
            ' 标题栏行(y=2) 深色最右 x=' + right + '，窗口宽 ' + dec.w +
            '（浅色主题下该值无意义：判据看上面的 intrudeRows）')
        } catch (e) {
          console.log('WARN PNG 解码失败：' + (e && e.message))
        }
      }
    }
    code = clean && sbPresent && scrollable && yOk ? 0 : 1
  } catch (e) {
    console.log('ERR  ' + (e && e.message ? e.message : e))
  }
  app.exit(code)
}).catch((e) => {
  console.log('ERR  ' + (e && e.message ? e.message : e))
  app.exit(2)
})
`,
  'utf8'
)

const electronPath = require('electron')
const env = { ...process.env }
delete env.ELECTRON_RUN_AS_NODE
const child = spawn(electronPath, [target, '--disable-gpu'], { stdio: 'inherit', env })
child.on('close', (code) => process.exit(code ?? 1))
