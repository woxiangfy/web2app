/**
 * 单窗口模式契约断言组（探针四拆分之三）：`runProbe`，原 probe.ts 的节 0–17。
 *
 * ⚠️ 顺序有讲究：最后一步是「点关闭按钮」，它会真的把窗口销毁，只能放在收尾。
 * 本文件只有这一个入口函数；夹具站/快照/时序工具来自另外两块，见 probe.ts 地图。
 */
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { screen } from 'electron'

import { AUTH_HEADER } from './probe-server'
import {
  BAR_BG,
  isAlive,
  Probe,
  sleep,
  SNAPSHOT_JS,
  waitUntil,
  type ProbeContext,
  type Snapshot,
} from './probe-infra'
import type { SidecarHandle } from './sidecar'
import {
  loadSettings,
  parseSettings,
  saveSettings,
  storeLocation,
  writeDocument,
  type Settings,
  type StoreLocation,
} from './settings'
import { hotkeyLabel, hotkeyToAccelerator, sameHotkey, type Hotkey } from './hotkey'
import {
  captureWindowState,
  loadWindowState,
  parseWindowState,
  planRestore,
  saveWindowState,
  type WindowState,
} from './window-state'

/**
 * 跑完整套契约断言。返回是否全部通过。
 *
 * ⚠️ 顺序有讲究：最后一步是「点关闭按钮」，它会真的把窗口销毁，只能放在收尾。
 */
export async function runProbe(ctx: ProbeContext): Promise<boolean> {
  const { win, base } = ctx
  const p = new Probe()

  const nav = async (path: string, settle = 450): Promise<void> => {
    await win.loadURL(base + path)
    // chrome-fit 的首轮扫描在 DOMContentLoaded 前就跑完了；再等一小段让 MutationObserver 补扫。
    await sleep(settle)
  }
  const snap = async (): Promise<Snapshot> =>
    (await win.webContents.executeJavaScript(SNAPSHOT_JS, true)) as Snapshot

  let s: Snapshot

  // ---------- 0. 启动页：窗口一出现就该有顶部栏 ----------
  // 启动页把两个脚本内联在**主世界**（不靠注入），这条断言就是「内联是否真的生效」。
  // 顺带验证：preload 暴露的 window.web2app 在启动页（splash: 协议）里也可用，
  // 否则标题栏画得出来但按钮点不动。
  await win.loadURL('splash://localhost/')
  await sleep(500)
  s = await snap()
  p.check('启动页：标题栏已内联挂载', s.barCount === 1 && !!s.barRect, `barCount=${s.barCount}`)
  p.near('启动页：标题栏高 38px', s.barRect?.h, 38, 0.5)
  p.check('启动页：壳标识已写入 DOM', s.shell === 'electron', `实际 ${s.shell}`)
  p.check('启动页：窗口 API 可用（按钮才点得动）', s.api !== null, 'window.web2app 未暴露')

  // ---------- 1. 普通页面：标题栏 + 38px 契约 ----------
  await nav('/plain.html')
  s = await snap()
  p.check('标题栏已挂载（远程 http 页面）', s.barCount === 1 && !!s.barRect, `barCount=${s.barCount}`)
  p.near('标题栏贴顶且高 38px', s.barRect?.h, 38, 0.5)
  p.near('标题栏 top=0', s.barRect?.top, 0, 0.5)
  // ⚠️⚠️ 「滚动条不得越过标题栏」的判据必须**能感受到覆盖**，`rect.width` 做不到。
  // 2026-09-17 查明：Chromium 把**视口（根）滚动条**绘制在所有 DOM 内容之上（`z-index` 再高也压不住），
  // 且它的几何就是「视口右缘 × 整个视口高」—— 于是只要页面由 html 来滚，那条滚动条必然横穿标题栏，
  // 顺带盖住最右侧的关闭按钮。此前两轮误判都源于此：**断言的期望值取自「实现当前的表现」就等于把
  // 缺陷锁死成了期望值**（第一版断言写的是 `barRect.w == clientWidth`，还在注释里把那 15px 的缝
  // 解释成「WebView 的正常行为」）。
  //
  // 现在的契约是「html 不滚、body 滚」：页面滚动的矩形从 38px 开始，滚动条物理上到不了标题栏。
  // 判据全部取自使用者能说清楚的目标（「滚动条在，只是别超过菜单栏」）：
  //   ① 标题栏那 38px 里，窗口最右一列逐点命中标题栏（有滚动条压着时这里命不中）
  //   ② 滚动条**仍然存在**、且占宽一条 —— 不能靠「把滚动条藏掉」来消灭那道缝（那是被否掉的方案）
  //   ③ 滚动能力与 window 级滚动 API 都保留（懒加载 / 吸顶导航 / 回到顶部全靠它们）
  p.check('plain 页：标题栏 38px 内没有滚动条（右端一列逐点命中标题栏）',
    s.intrudeRows.length === 0, `命不中的 y：${JSON.stringify(s.intrudeRows)}`)
  p.near('plain 页：标题栏命中区覆盖到窗口最右缘', s.hitRight, s.view.iw - 1, 0)
  p.near('plain 页：标题栏布局宽度 = 窗口宽', s.barRect?.w, s.view.iw, 1)
  p.check('plain 页：html 不再承担滚动（根滚动条已撤）',
    s.rootOverflowY === 'hidden' && !s.rootScrollable,
    `html overflowY=${s.rootOverflowY} rootScrollable=${s.rootScrollable}`)
  p.check('plain 页：body 是滚动容器', s.bodyOverflowY === 'auto' && s.bodyScrollable,
    `body overflowY=${s.bodyOverflowY} bodyScrollable=${s.bodyScrollable}`)
  p.near('plain 页：滚动条仍然可见（占宽一条，归 body）', s.bodyScrollbarW, 15, 6)
  p.near('plain 页：body 盒子从标题栏下沿开始（38px）', parseFloat(s.bodyMarginTop ?? 'NaN'), 38, 0.5)
  p.near('plain 页：body 高度 = 视口 - 38', s.bodyH, s.view.ih - 38, 2)
  // 宽度自愈：上面几条只证明「此刻是对的」，这条证明「错了能自己回来」——
  // 把 inline 宽度改写成 `innerWidth - 15`（正好少一条滚动条宽，与使用者截图里的缝一模一样），
  // 再触发一次 resize：期望宽度被重新钉回窗口宽，而不是维持窄一条滚动条的样子。
  // 期望值取自「窗口宽」这个使用者能说清楚的目标，而不是脚本当前的表现。
  await win.webContents.executeJavaScript(
    "(() => { const b = document.getElementById('__web2app_titlebar__'); if (!b) return;" +
      " b.style.setProperty('width', (window.innerWidth - 15) + 'px', 'important');" +
      " window.dispatchEvent(new Event('resize')); })()",
    true
  )
  await sleep(200)
  s = await snap()
  p.near('宽度被改写后自愈回窗口宽（不留缝）', s.barRect?.w, s.view.iw, 1)

  // window 级滚动 API 的转发（这是「body 当滚动容器」唯一的代价，必须有断言钉住）：
  // `window.scrollTo` / `window.scrollY` / window 上的 `scroll` 事件，是懒加载、吸顶导航、
  // 回到顶部、阅读进度这类页面逻辑的公共依赖。
  // ⚠️ `scroll` 事件是**异步**派发的，必须等一帧再读 —— 同步读会永远得到「未触发」的假失败
  // （探针第一版就是这么写的，并且它差点让我以为转发坏了）。
  const fwd = JSON.parse(
    await win.webContents.executeJavaScript(
      `(async () => {
      const b = document.body;
      let fired = 0;
      const h = () => { fired++; };
      window.addEventListener('scroll', h, { passive: true });
      window.scrollTo(0, 200);
      await new Promise((r) => setTimeout(r, 160));
      const y = Math.round(window.scrollY);
      const byBody = Math.round(b.scrollTop);
      window.removeEventListener('scroll', h);
      window.scrollTo(0, 0);
      await new Promise((r) => setTimeout(r, 100));
      const back = Math.round(window.scrollY);
      // 另外两条老式回顶写法：document.documentElement.scrollTop / document.scrollingElement
      document.documentElement.scrollTop = 300;
      const byHtmlEl = Math.round(b.scrollTop);
      const seIsBody = document.scrollingElement === b;
      window.scrollTo(0, 0);
      return JSON.stringify({ y, byBody, fired, back, byHtmlEl, seIsBody });
    })()`,
      true
    )
  ) as { y: number; byBody: number; fired: number; back: number; byHtmlEl: number; seIsBody: boolean }
  p.near('window.scrollTo 仍能滚动（转发到 body）', fwd.y, 200, 40)
  p.near('window.scrollY 读到的是 body 的真实位置', fwd.byBody, fwd.y, 2)
  p.check('window 上的 scroll 事件仍会触发（异步）', fwd.fired > 0, `触发次数 ${fwd.fired}`)
  p.near('window.scrollTo(0,0) 能回到顶部', fwd.back, 0, 2)
  // 老式回顶写法（`document.documentElement.scrollTop = 0`）与 `document.scrollingElement`
  // 也必须被转发，否则一票「回到顶部」按钮会静默失效。
  p.near('document.documentElement.scrollTop 写入转发到 body', fwd.byHtmlEl, 300, 40)
  p.check('document.scrollingElement 指向 body', fwd.seIsBody, '仍指向 html（页面内的相对滚动会失效）')
  p.check('标题栏 z-index 极高', s.barZ === '2147483647', `实际 ${s.barZ}`)
  p.check('标题栏背景已着色（样式表生效）', BAR_BG.indexOf(s.barBg ?? '') >= 0, `实际 ${s.barBg}`)
  p.check('壳标识已写入 DOM', s.shell === 'electron', `实际 ${s.shell}`)
  p.check('38px 契约标记已写入 DOM', s.chromeH === '38', `实际 ${s.chromeH}`)
  p.check('顶部让位只做一次：html 不再留白（避免 76px 双留白）', s.htmlPadTop === '0px', `实际 ${s.htmlPadTop}`)
  p.check('六个按钮齐全', s.buttons === 6, `实际 ${s.buttons}`)
  // 「设置」按钮必须在【最小化左侧】（使用者指定的位置）。顺序靠 DOM，位置靠几何 ——
  // 两者都断言，免得「DOM 顺序对、CSS 把它挪走了」这种半对状态混过去。
  p.check(
    '设置按钮紧挨最小化左侧',
    !!s.gear && s.gear.gearRight <= s.gear.minLeft + 1 && s.gear.minLeft - s.gear.gearRight < 12,
    s.gear ? `gearRight=${s.gear.gearRight} minLeft=${s.gear.minLeft}` : '未找到设置按钮'
  )
  p.check(
    'window.web2app 白名单 API 就位',
    s.api === 'chromeH,close,isElectron,minimize,openSettings,toggleMaximize',
    `实际 ${s.api}`
  )
  p.check('window.web2appExtra 已注入', s.extra === '{"build":"probe","env":"test"}', `实际 ${s.extra}`)
  p.near('拖动区已声明（-webkit-app-region=drag）', s.appRegion === 'drag' ? 1 : 0, 1, 0)
  // 加载层有「最短展示时长 + 淡出动画」（loading.js：MIN_MS 220 + FADE_MS 280，另加
  // DOMContentLoaded 后的 DCL_DELAY 240），元素是动画**结束后**才移除的 —— nav 的
  // 450ms settle 不够，固定 sleep 会得到假失败。这里轮询等待。
  const overlayGone = await waitUntil(async () => !(await snap()).overlay, 3000)
  p.check('加载层已自行淡出（普通页面）', overlayGone, '普通页面不该一直盖着加载层')

  // 页面正文不能被标题栏压住：fixed 页头由 chrome-fit 下移，正常流内容则因 html 的
  // padding-top 天然从 38 起。
  p.near('页面正文未被标题栏压住', s.r.content?.top, 38, 2)

  // ---------- 2. fixed 页头下移 ----------
  await nav('/fixed.html')
  s = await snap()
  p.check('fixed 页头被打上 shift 标记', s.r.hdr !== null, '未找到 #hdr')
  p.near('fixed 页头下移到 38px', s.r.hdr?.top, 38, 1)

  // ---------- 3. min-height:100vh 容器缩高 ----------
  await nav('/vh.html')
  s = await snap()
  p.check('整页容器被打上 shrink 标记', s.shrinkAttr, '未打上 data-web2app-chrome-shrink')
  p.check('页脚底边不越出视口下沿', (s.r.ftr?.bottom ?? 1e9) <= s.view.ch + 1.5, `页脚 bottom=${s.r.ftr?.bottom}，视口底=${s.view.ch}`)
  p.check('页脚仍贴在视口底部（没有被压扁）', (s.r.ftr?.bottom ?? 0) >= s.view.ch - 6, `页脚 bottom=${s.r.ftr?.bottom}，视口底=${s.view.ch}`)

  // ---------- 4. fixed;inset:0 整屏外壳 ----------
  await nav('/inset.html')
  s = await snap()
  p.near('整屏外壳顶边下移到 38px', s.r.shell?.top, 38, 1.5)
  p.check('整屏外壳底边仍钉在视口底', (s.r.shell?.bottom ?? 0) >= s.view.ch - 2, `外壳 bottom=${s.r.shell?.bottom}，视口底=${s.view.ch}`)

  // ---------- 5. sticky 页头滚动后仍在 38（而且**不需要我们动手**） ----------
  await nav('/sticky.html')
  await win.webContents.executeJavaScript('window.scrollTo(0, 700)', true)
  await sleep(350)
  s = await snap()
  // ⚠️ 新契约下 sticky **不该**被打上 shift 标记：body 已经是滚动容器、盒子上沿就在 38px，
  // `top:0` 的 sticky 天然停在 y=38（正是要的位置）。再给它加 38 就成了 76px
  //（页面自己的吸顶导航凭空多出一条空白）。这条断言把两件事一起钉住：位置对 + 我们没多手。
  p.check('sticky 页头不被打 shift 标记（body 已是滚动容器）', !s.shiftAttr,
    '不该打上 data-web2app-chrome-shift')
  p.near('sticky 页头滚动后停在 38px', s.r.hdr?.top, 38, 1.5)

  // ---------- 6. 严格 CSP 页面 ----------
  await nav('/csp.html')
  s = await snap()
  p.check('严格 CSP 页面：标题栏仍挂载', s.barCount === 1, `barCount=${s.barCount}`)
  // 这条断言最初是**失败**的：`<style>` 元素被 `style-src 'self'` 拦掉，标题栏裸奔
  //（透明背景、按钮挤成一团）。修法是改用构造函数式样式表（document.adoptedStyleSheets，
  // 不受页面 CSP 约束），见 titlebar.js 的 mountStyle。留着这条盯着它别再退化。
  p.check('严格 CSP 页面：标题栏样式仍生效', BAR_BG.indexOf(s.barBg ?? '') >= 0, `背景=${s.barBg}`)
  p.check('严格 CSP 页面：滚动根契约仍生效（html 不滚 + body 让位 38px）',
    s.bodyOverflowY === 'auto' && s.rootOverflowY === 'hidden' && s.htmlPadTop === '0px',
    `body overflowY=${s.bodyOverflowY} html overflowY=${s.rootOverflowY} html paddingTop=${s.htmlPadTop}`)

  // ---------- 7. 空 body 页面：加载层留在屏幕上 ----------
  await nav('/empty.html')
  s = await snap()
  p.check('空 body 页面：加载层已铺上', s.overlay === true, '未找到加载层')

  // ---------- 8. target=_blank / window.open 一律原地导航 ----------
  await nav('/opener.html')
  await win.webContents.executeJavaScript('document.getElementById("lnk").click()', true)
  await sleep(700)
  s = await snap()
  p.check('target=_blank 链接原地导航（不开裸窗口）', s.path === '/opened.html', `实际路径 ${s.path}`)
  p.check('原地导航后窗口未增加', win.webContents.getURL().includes('/opened.html'), win.webContents.getURL())

  await nav('/opener.html')
  await win.webContents.executeJavaScript('window.open("/opened.html", "_blank")', true)
  await sleep(700)
  s = await snap()
  p.check('window.open 原地导航', s.path === '/opened.html', `实际路径 ${s.path}`)

  // ---------- 9. 窗口按钮真的能驱动原生窗口 ----------
  await nav('/plain.html')
  const clickBtn = (cls: string) =>
    win.webContents.executeJavaScript(`document.querySelector('#__web2app_titlebar__ .${cls}').click()`, true)

  await clickBtn('w2t-min')
  await sleep(400)
  p.check('最小化按钮生效', win.isMinimized(), `isMinimized=${win.isMinimized()}`)
  win.restore()
  await sleep(400)

  await clickBtn('w2t-max')
  await sleep(500)
  p.check('最大化按钮生效', win.isMaximized(), `isMaximized=${win.isMaximized()}`)
  await clickBtn('w2t-max')
  await sleep(500)
  p.check('最大化按钮可还原', !win.isMaximized(), `isMaximized=${win.isMaximized()}`)

  // ---------- 9.1 标题栏「设置」按钮（最小化左侧）→ 真的打开设置窗 ----------
  // 完整链路：注入脚本 → window.web2app.openSettings()（preload 白名单）→ IPC win:open-settings
  //          → 主进程 openSettings() → openSettingsWindow()。
  // 这里必须**真的点按钮**：任何一环断掉，表象都是「按钮点了没反应」，而直接调
  // ctx.m3.openSettingsWindow()（13 段会做）完全绕过了前两环、抓不到这种断链。
  await clickBtn('w2t-settings')
  // ⚠️ 判据要等到**页面真的渲染出面板**，不能只等「窗口对象存在」：窗口建好但 URL 加载失败
  //    时它照样存在，断言会假绿。而且提前 close 会中止加载，日志里留下一句
  //    `设置窗加载失败：ERR_FAILED (-2)` —— 那是探针自己制造的噪声，不是缺陷。
  const gearOpened = await waitUntil(async () => {
    const w = ctx.m3.getSettingsWindow()
    if (!w) return false
    try {
      return (await w.webContents.executeJavaScript(
        '!!document.querySelector(".panel")',
        true
      )) as boolean
    } catch {
      return false
    }
  }, 6000)
  p.check(
    '点标题栏设置按钮 → 打开设置窗且页面已渲染',
    gearOpened,
    '6s 内未渲染（查 preload 的 openSettings / IPC win:open-settings / 页面按钮绑定 / settings:// 协议）'
  )
  ctx.m3.closeSettingsWindow()
  const gearClosed = await waitUntil(async () => !ctx.m3.getSettingsWindow(), 4000)
  p.check('设置按钮链路收尾：设置窗可关闭', gearClosed, '4s 内未关闭')

  // ---------- 10. 本地反向代理 sidecar（M2） ----------
  //
  // 这一组断言合起来回答一个问题：**代理有没有给每一个请求（含 WebSocket 握手）注入
  // Authorization** —— 这正是 proxy.rs 存在的理由（WebView2 的原生鉴权事件对 WS 不生效），
  // 也是「Electron 侧复用同一份 Rust 代理」这件事值不值得的全部依据。
  const upstream = new URL(base)
  const authUrl = `http://probe:secret@${upstream.host}/auth.html`

  let side: SidecarHandle | null = null
  try {
    side = await ctx.startSidecar({ url: authUrl, noProxy: false })
    // proxyUrl 自带一次性随机门禁密钥（256-bit → 43 字符 URL-safe base64）。
    p.check(
      'sidecar 启动并完成握手（入口 URL 带 w2a_key 门禁密钥）',
      /^http:\/\/127\.0\.0\.1:\d+\/\?w2a_key=[A-Za-z0-9_-]{43}$/.test(side.proxyUrl),
      `proxyUrl=${side.proxyUrl}`
    )
  } catch (e) {
    p.check('sidecar 启动并完成握手', false, `启动失败：${(e as Error).message}`)
  }

  if (side) {
    // 用入口 URL 里的密钥拼出同源下的具体路径（直接字符串拼接会把 path 接进 query）。
    const gate = (path: string): string => {
      const u = new URL(side!.proxyUrl)
      u.pathname = path
      return u.toString()
    }
    const rawRoot = new URL(side.proxyUrl).origin

    // 先证明「不带凭据确实会被拒」：否则后面的「代理带上了凭据」是假通过
    //（比如夹具站点压根没在鉴权）。
    const direct = await fetch(`${base}/auth.html`)
    p.check('直连上游（不带凭据）被拒 401', direct.status === 401, `实际 ${direct.status}`)

    // 门禁先于转发生效：无密钥 / 错密钥一律 404（不透露代理存在，防本机扫描借道）。
    const noKey = await fetch(`${rawRoot}/auth.html`)
    p.check('无门禁密钥访问代理返回 404（隐藏代理身份）', noKey.status === 404, `实际 ${noKey.status}`)
    const badKey = await fetch(`${rawRoot}/auth.html?w2a_key=wrong`)
    p.check('错误门禁密钥访问代理返回 404', badKey.status === 404, `实际 ${badKey.status}`)

    const viaProxy = await fetch(gate('/auth.html'))
    p.check('经代理访问上游返回 200', viaProxy.status === 200, `实际 ${viaProxy.status}`)
    const lastAuth = ctx.seen.httpAuth[ctx.seen.httpAuth.length - 1]
    p.check(
      '上游收到代理注入的 Authorization（与 build_basic_auth 格式一致）',
      lastAuth === AUTH_HEADER,
      `实际 ${lastAuth || '(空)'}`
    )
    const body = await viaProxy.text()
    p.check('代理转发的响应体是上游内容', body.includes('<h1>auth</h1>'), body.slice(0, 60))

    // HTTP 方向的 Origin 也必须被重写成「上游源」。
    // ⚠️ 这条断言是为了钉住一个真实踩过的坑：代理里「先透传、后覆盖」的写法在
    // `Builder::header`（append 语义）下会向上游发出**两个** Origin（逗号拼接），
    // 比不改写更糟 —— 做 Origin 校验的后端会直接拒连。修法是透传时就跳过 origin。
    // 这里显式带一个假的浏览器 Origin，看上游收到什么。
    await fetch(gate('/auth.html'), { headers: { Origin: 'http://127.0.0.1:9999' } })
    const lastOrigin = ctx.seen.authOrigins[ctx.seen.authOrigins.length - 1]
    p.check(
      '上游收到的 Origin 被改写为上游源（且只有一个）',
      lastOrigin === `http://${upstream.host}`,
      `实际 ${lastOrigin === undefined ? '(未记录)' : JSON.stringify(lastOrigin)}`
    )

    // ---- WebSocket：M2 的核心用例 ----
    // WebView2 时代「页面能开但实时联动失效」的根因就是 WS 握手没带凭据。
    // 文档 URL 带 query 密钥（首访回种 HttpOnly cookie）；页面里的 WS 只连 /ws 不带密钥，
    // 能通就证明根绝对路径资源/WS 靠 cookie 过门禁这条链路真实成立。
    await win.loadURL(gate('/ws.html'))
    const wsReady = await waitUntil(async () => {
      const n = (await win.webContents.executeJavaScript(
        'window.__ws && window.__ws.msgs ? window.__ws.msgs.length : 0',
        true
      )) as number
      return typeof n === 'number' && n >= 2
    }, 6000)
    const wsDump = (await win.webContents.executeJavaScript(
      'JSON.stringify(window.__ws || null)',
      true
    )) as string

    const upg = ctx.seen.wsUpgrades.find((u) => u.path === '/ws')
    p.check('WebSocket 升级请求到达上游', !!upg, '上游未收到 /ws 升级请求')
    p.check(
      'WebSocket 握手也带上了 Authorization',
      upg?.auth === AUTH_HEADER,
      `实际 ${upg?.auth || '(空)'}`
    )
    // Origin 应被代理改写为「等价浏览器直连上游」的源：后端（如 Trilium）常做 Origin 校验，
    // 不改写就会出现「页面能开、但 WS 被拒」——Tauri 版的注释里专门记了这条。
    p.check(
      'WebSocket 的 Origin 已被改写为上游源（不是 127.0.0.1 代理）',
      upg?.origin === `http://${upstream.host}`,
      `实际 ${upg?.origin || '(空)'}`
    )
    p.check(
      'WebSocket 双向通信打通',
      wsReady &&
        ctx.seen.wsFromClient.includes('hello-from-page') &&
        wsDump.includes('echo:hello-from-page'),
      `上游收到=${JSON.stringify(ctx.seen.wsFromClient)} 页面状态=${wsDump}`
    )

    // ---- 回收：stop() 之后进程必须真的没了 ----
    const pid = side.pid
    side.stop()
    const dead = await waitUntil(async () => !isAlive(pid), 5000)
    p.check('sidecar 已退出（未留孤儿进程）', dead, `pid=${pid} 仍在运行`)
  }

  // ---------- 11. 设置存储契约（M3） ----------
  //
  // 这一段的断言绝大多数是**纯函数级**的，但它们钉住的都是「错了也静默」的东西：
  // 破坏这些契约，Electron 版自身照样跑得好好的，只是和 Tauri 版的设置悄悄对不上 ——
  // 那是最难发现的一类回归（用户只会在两个壳之间来回切换时才隐约觉得「设置老是丢」）。
  const HK = (o: Partial<Hotkey>): Hotkey => ({ ctrl: false, alt: false, shift: false, key: '', ...o })

  // 11.1 Hotkey（旧版存 JS event.code）→ Electron accelerator
  p.check(
    'Alt+4 → Alt+4（契约 #6 的唤醒键）',
    hotkeyToAccelerator(HK({ alt: true, key: 'Digit4' })) === 'Alt+4',
    String(hotkeyToAccelerator(HK({ alt: true, key: 'Digit4' })))
  )
  p.check(
    'Ctrl+A → Control+A（修饰键命名差异已收敛）',
    hotkeyToAccelerator(HK({ ctrl: true, key: 'KeyA' })) === 'Control+A',
    String(hotkeyToAccelerator(HK({ ctrl: true, key: 'KeyA' })))
  )
  p.check(
    '三修饰键 → Control+Alt+Shift+F9',
    hotkeyToAccelerator(HK({ ctrl: true, alt: true, shift: true, key: 'F9' })) === 'Control+Alt+Shift+F9',
    String(hotkeyToAccelerator(HK({ ctrl: true, alt: true, shift: true, key: 'F9' })))
  )
  p.check('空 key（= 未启用）→ null', hotkeyToAccelerator(HK({})) === null)
  // ⚠️ 这条是「宁可不注册也不猜」的守门断言：猜错的后果是 globalShortcut.register 静默返回
  //    false，界面上看是「保存成功了但快捷键没用」，比直接报错难查得多。
  p.check(
    '不认识的键名 → null（绝不猜）',
    hotkeyToAccelerator(HK({ key: 'NotAKey' })) === null,
    String(hotkeyToAccelerator(HK({ key: 'NotAKey' })))
  )
  p.check(
    'hotkeyLabel 文案与设置页一致',
    hotkeyLabel(HK({ alt: true, key: 'Digit4' })) === 'Alt+4',
    hotkeyLabel(HK({ alt: true, key: 'Digit4' }))
  )
  p.check(
    '快捷键等价判定正确（决定「要不要重注册」）',
    sameHotkey(HK({ key: 'Digit4' }), HK({ key: 'Digit4' })) &&
      !sameHotkey(HK({ key: '' }), HK({ key: 'Digit4' })),
    '等价判定错误'
  )

  // 11.2 settings.json 解析 / 序列化
  const tolerant = parseSettings({
    tray_enabled: 'true',
    minimize_on_close: 'yes',
    wake_hotkey: { alt: 'true', key: 'Digit4' },
  })
  p.check(
    '宽容读取：字符串布尔值（兼容上游模板风格）',
    tolerant.tray_enabled && tolerant.minimize_on_close && tolerant.wake_hotkey.alt,
    JSON.stringify(tolerant)
  )
  // ⚠️⚠️ 这条是「两壳互通」最硬的一条：字段名、嵌套结构、布尔**类型**、甚至键顺序
  //     都必须与 serde 写出来的一致。任何一处走样，旧版就读不动我们写的文件。
  p.check(
    '序列化输出为规范 JSON（旧版 serde_json 必须能原样读回）',
    JSON.stringify(tolerant) ===
      '{"wake_hotkey":{"ctrl":false,"alt":true,"shift":false,"key":"Digit4"},' +
        '"tray_enabled":true,"minimize_on_close":true}',
    JSON.stringify(tolerant)
  )
  const emptySt = parseSettings(null)
  p.check(
    '设置全缺失 → 默认值（不设快捷键 / 不启用托盘 / 关闭即退出，与旧版一致）',
    emptySt.wake_hotkey.key === '' &&
      !emptySt.wake_hotkey.alt &&
      !emptySt.tray_enabled &&
      !emptySt.minimize_on_close,
    JSON.stringify(emptySt)
  )
  const partial = parseSettings({ wake_hotkey: { key: 'F1' } })
  p.check(
    'wake_hotkey 缺字段补齐为 false（不因缺项整份作废）',
    partial.wake_hotkey.key === 'F1' && partial.wake_hotkey.alt === false,
    JSON.stringify(partial)
  )

  // 11.3 存储位置契约
  const fakeLoc = storeLocation({
    dataDir: join(tmpdir(), 'w2a-no-such-data'),
    appDataDir: join(tmpdir(), 'w2a-no-such-appdata'),
    appName: 'Demo:App', // 故意带一个 Windows 非法字符，验证清洗
    file: 'settings.json',
  })
  p.check(
    '写入一律落在数据目录（与 data_dir 契约一致，绿色版搬目录不会漏掉设置）',
    fakeLoc.write === join(tmpdir(), 'w2a-no-such-data', 'settings.json'),
    fakeLoc.write
  )
  p.check(
    '读取候选：数据目录优先、旧位置（%APPDATA%\\<app_name>）兜底（升级兼容）',
    fakeLoc.read.length === 2 &&
      fakeLoc.read[0] === fakeLoc.write &&
      fakeLoc.read[1] === join(tmpdir(), 'w2a-no-such-appdata', 'Demo_App', 'settings.json'),
    JSON.stringify(fakeLoc.read)
  )

  // 11.4 真实读写 roundtrip（写临时目录，绝不碰用户配置）
  const tmpStore = mkdtempSync(join(tmpdir(), 'w2a-probe-settings-'))
  try {
    const loc: StoreLocation = {
      read: [join(tmpStore, 'settings.json')],
      write: join(tmpStore, 'settings.json'),
    }
    const want: Settings = {
      wake_hotkey: { ctrl: false, alt: true, shift: false, key: 'Digit4' },
      tray_enabled: true,
      minimize_on_close: true,
    }
    saveSettings(loc, want)
    const back = loadSettings(loc)
    p.check(
      'settings.json 落盘后可原样读回（字段全保真）',
      JSON.stringify(back.settings) === JSON.stringify(want),
      JSON.stringify(back.settings)
    )

    // ⚠️⚠️ 设置与窗口几何**同住这一个 JSON**（2026-09-23 合并，替代原来的 window-state.json）。
    // 合并后唯一真正的风险就是「两个写入方互相覆盖」：设置窗保存 vs 窗口关闭时记几何。
    // 所以两个方向各钉一条 —— 少了任何一条，就会出现「挪了下窗口，设置被还原成上一次的」
    // 或「改了设置，窗口位置丢了」这种只在用户操作顺序恰巧时才现形的故障。
    const geom: WindowState = { x: 120, y: 80, width: 900, height: 700, maximized: false }
    saveWindowState(loc, geom)
    p.check(
      '窗口几何落盘后可原样读回（同一 JSON 的 window 段）',
      JSON.stringify(loadWindowState(loc).state) === JSON.stringify(geom),
      JSON.stringify(loadWindowState(loc).state)
    )
    p.check(
      '写几何不吃掉设置（同一文件读-改-写）',
      JSON.stringify(loadSettings(loc).settings) === JSON.stringify(want),
      JSON.stringify(loadSettings(loc).settings)
    )

    const want2: Settings = { ...want, tray_enabled: false, minimize_on_close: false }
    saveSettings(loc, want2)
    p.check(
      '写设置不吃掉窗口几何（同一文件读-改-写）',
      JSON.stringify(loadWindowState(loc).state) === JSON.stringify(geom),
      JSON.stringify(loadWindowState(loc).state)
    )

    // 键序：设置三键在前、`window` 在后 —— 与 Tauri 侧 serde 的字段序一致，
    // 否则「同一个文件两版写出来的东西不同」会被当成 bug 排查。
    const text = readFileSync(loc.write, 'utf8')
    p.check(
      '文档键序规范（设置三键 → window，两空格缩进）',
      text.indexOf('"wake_hotkey"') < text.indexOf('"window"') &&
        text.includes('\n  "window": {'),
      text.replace(/\s+/g, ' ').slice(0, 140)
    )

    // ⚠️⚠️ 落点搬家的守卫（2026-09-23）：老位置（`%APPDATA%\<app_name>\`）里还有用户设置时，
    // 第一次保存必须「读老位置、写新位置（数据目录）」，且**老位置的值（含 window 段）不能丢**。
    // 只认写入位置那份（还不存在）就会读成空文档 ⇒ 用户升级后看到设置被重置。
    const dirNew = join(tmpStore, 'data')
    const dirOld = join(tmpStore, 'appdata')
    const locNew = join(dirNew, 'settings.json')
    const locOld = join(dirOld, 'settings.json')
    const legacyLoc: StoreLocation = { read: [locOld], write: locOld }
    writeDocument(legacyLoc, { settings: want, window: geom })
    const oldBefore = readFileSync(locOld, 'utf8')
    saveSettings({ read: [locNew, locOld], write: locNew }, want2)
    const moved = loadSettings({ read: [locNew, locOld], write: locNew })
    p.check(
      '落点搬家：读旧位置、写数据目录，且 window 段跟着搬过去',
      existsSync(locNew) &&
        JSON.stringify(moved.settings) === JSON.stringify(want2) &&
        JSON.stringify(loadWindowState({ read: [locNew], write: locNew }).state) ===
          JSON.stringify(geom),
      `new=${locNew} settings=${JSON.stringify(moved.settings)}`
    )
    p.check(
      '落点搬家：旧位置的文件一个字节都不动（只读兼容，不迁移不删除）',
      readFileSync(locOld, 'utf8') === oldBefore,
      locOld
    )
  } finally {
    rmSync(tmpStore, { recursive: true, force: true })
  }

  // 11.5 窗口几何解析与恢复策略
  const ws1 = parseWindowState({ x: 100, y: 200, width: 800, height: 600 })
  p.check('window 段缺 maximized → false（兼容旧版文件）', ws1 !== null && !ws1.maximized, JSON.stringify(ws1))
  p.check('window 段缺坐标 → 视为无效', parseWindowState({ width: 800, height: 600 }) === null)
  p.check('window 段非对象 → 无效', parseWindowState('nope') === null)

  p.check(
    '恢复策略：无保存状态 → 用默认尺寸',
    planRestore(null).maximize === false && !planRestore(null).bounds
  )
  p.check(
    '恢复策略：上次最大化 → 直接最大化',
    planRestore({ x: 0, y: 0, width: 800, height: 600, maximized: true }).maximize === true
  )
  p.check(
    '恢复策略：尺寸过小 → 最大化（不恢复成一个没法用的窄条）',
    planRestore({ x: 100, y: 100, width: 100, height: 100, maximized: false }).maximize === true
  )
  // ⚠️ 最经典的一条：拔掉外接显示器 / 改分辨率 / 改缩放后，保存的坐标会在屏幕外 ——
  //    直接恢复就是「程序启动了但看不到窗口」，用户会以为没启动。
  p.check(
    '恢复策略：位置在屏幕外 → 最大化（保证窗口一定可见）',
    planRestore({ x: -99999, y: -99999, width: 800, height: 600, maximized: false }).maximize === true
  )
  const goodPlan = planRestore({ x: 200, y: 200, width: 900, height: 700, maximized: false })
  p.check(
    '恢复策略：有效几何 → 原位恢复',
    !!goodPlan.bounds && goodPlan.maximize === false,
    goodPlan.reason
  )

  // 11.6 真实窗口几何落盘（capture → save → load）
  const live = captureWindowState(win)
  p.check(
    '可从真实窗口抓取几何（物理像素）',
    live !== null && live.width > 0 && live.height > 0,
    JSON.stringify(live)
  )
  if (live) {
    // DIP→物理 的换算必须正确，否则 125%/150% 缩放的机器上恢复出来的窗口会缩水
    const phys = screen.dipToScreenRect(win, win.getBounds())
    p.check(
      '几何换算正确（DIP → 物理，跨 DPI 不错位）',
      Math.abs(live.width - Math.round(phys.width)) <= 2 &&
        Math.abs(live.height - Math.round(phys.height)) <= 2,
      `保存 ${live.width}×${live.height}，实际物理 ${Math.round(phys.width)}×${Math.round(phys.height)}`
    )
    saveWindowState(ctx.m3.storeLoc, live)
    const reloaded = loadWindowState(ctx.m3.storeLoc)
    p.check(
      '窗口几何落盘后可原样读回',
      JSON.stringify(reloaded.state) === JSON.stringify(live),
      JSON.stringify(reloaded.state)
    )
  }

  // ---------- 12. 托盘 / 全局快捷键（M3） ----------
  const icon = ctx.m3.trayIconPath()
  p.check('托盘图标可定位（icons/icon.png）', !!icon, '未找到图标文件')

  const baseSettings = ctx.m3.getSettings()
  ctx.m3.destroyTray()
  ctx.m3.saveSettings({ ...baseSettings, tray_enabled: true })
  p.check('设置里启用托盘 → 图标已创建', ctx.m3.hasTray(), 'hasTray() = false')
  ctx.m3.saveSettings({ ...baseSettings, tray_enabled: false })
  p.check('设置里关闭托盘 → 图标已销毁', !ctx.m3.hasTray(), 'hasTray() = true')

  // 唤醒快捷键：注册 / 报错 / 清除 三条路径
  const rOn = ctx.m3.applyWakeHotkey(HK({ ctrl: true, alt: true, shift: true, key: 'F9' }))
  p.check(
    '唤醒快捷键可注册（Ctrl+Alt+Shift+F9）',
    rOn.ok && rOn.accelerator === 'Control+Alt+Shift+F9',
    rOn.error ?? `accelerator=${rOn.accelerator}`
  )
  const rBad = ctx.m3.applyWakeHotkey(HK({ key: 'NotAKey' }))
  p.check(
    '不支持的键名 → 明确报错（不是静默忽略）',
    !rBad.ok && !!rBad.error,
    JSON.stringify(rBad)
  )
  const rOff = ctx.m3.applyWakeHotkey(HK({}))
  p.check('清空快捷键 → 注销成功', rOff.ok && rOff.accelerator === null, JSON.stringify(rOff))
  p.check(
    '设置窗快捷键 Shift+F1 可注册',
    ctx.m3.registerSettingsShortcut(() => {}),
    '可能已被其它程序占用（本机环境问题，非代码缺陷）'
  )
  ctx.m3.releaseShortcuts()

  // ---------- 13. 设置窗（M3） ----------
  ctx.m3.openSettingsWindow()
  const opened = await waitUntil(async () => !!ctx.m3.getSettingsWindow(), 5000)
  p.check('设置窗已打开', opened, '5s 内未创建')
  const swin = ctx.m3.getSettingsWindow()
  if (swin) {
    const urlOk = await waitUntil(
      async () => swin.webContents.getURL().startsWith('settings://localhost'),
      5000
    )
    p.check('设置窗 URL 为 settings://localhost/（对齐旧版协议）', urlOk, swin.webContents.getURL())

    await sleep(500) // 等页面脚本跑起来
    const shim = (await swin.webContents.executeJavaScript(
      'typeof window.__TAURI_INTERNALS__ === "object" && typeof window.__TAURI_INTERNALS__.invoke === "function"',
      true
    )) as boolean
    p.check('Tauri invoke 垫片已注入（设置页 HTML 一行都不用改）', shim === true, String(shim))

    const barCount = (await swin.webContents.executeJavaScript(
      'document.querySelectorAll("#__web2app_titlebar__").length',
      true
    )) as number
    p.check('设置窗未注入主窗的自绘标题栏（它自带面板头部）', barCount === 0, `barCount=${barCount}`)

    // 设置窗是**无边框窗**（frame:false）：版式是「没有顶部的窗体、面板铺满整窗、正文可滚动」，
    // 标题 / 关闭 / 拖动都由 dist/settings.html 自绘。
    // 判据用「窗口区 == 内容区」：带系统标题栏时两者至少差一条标题栏（~31px），无边框时应为 0
    //（容差 2px 兜住 DWM 取整）。
    const wb = swin.getBounds()
    const cb = swin.getContentBounds()
    const frameDelta = Math.abs(wb.width - cb.width) + Math.abs(wb.height - cb.height)
    p.check(
      '设置窗是无边框窗（窗口区 == 内容区，没有系统标题栏）',
      frameDelta <= 2,
      `bounds=${JSON.stringify(wb)} content=${JSON.stringify(cb)}`
    )

    // 版式契约逐条钉住：面板铺满整窗 / 无外框装饰 / 正文是唯一滚动容器 / 头部可拖动。
    // 这些正是使用者这次提的四件事，缺任何一条都会在界面上一眼看出来。
    const uiJson = (await swin.webContents.executeJavaScript(
      `(() => {
        const g = (s) => { const e = document.querySelector(s); if (!e) return null;
          const r = e.getBoundingClientRect();
          return { l:+r.left.toFixed(1), t:+r.top.toFixed(1), r:+r.right.toFixed(1), b:+r.bottom.toFixed(1) }; };
        const bd = document.querySelector('.panel-bd');
        const hd = document.querySelector('.panel-hd');
        const p = document.querySelector('.panel');
        const pcs = p ? getComputedStyle(p) : null;
        let drag = null;
        if (hd) { const cs = getComputedStyle(hd);
          try { drag = cs.webkitAppRegion || cs.getPropertyValue('-webkit-app-region'); } catch (e) {} }
        return JSON.stringify({
          panel: g('.panel'), bd: g('.panel-bd'),
          panelBorder: pcs ? pcs.borderTopWidth : null,
          panelRadius: pcs ? pcs.borderTopLeftRadius : null,
          panelShadow: pcs ? pcs.boxShadow : null,
          bdOv: bd ? getComputedStyle(bd).overflowY : null,
          htmlOv: getComputedStyle(document.documentElement).overflowY,
          bodyOv: getComputedStyle(document.body).overflowY,
          drag: drag || null,
          hasX: !!document.getElementById('closeBtn'),
          iw: window.innerWidth, ih: window.innerHeight
        });
      })()`,
      true
    )) as string
    const u = JSON.parse(uiJson) as {
      panel: { l: number; t: number; r: number; b: number } | null
      bd: { l: number; t: number; r: number; b: number } | null
      panelBorder: string | null
      panelRadius: string | null
      panelShadow: string | null
      bdOv: string | null
      htmlOv: string | null
      bodyOv: string | null
      drag: string | null
      hasX: boolean
      iw: number
      ih: number
    }
    // ⚠️ 2026-09-23 使用者指「设置界面有大块空边框」⇒ 版式从「居中面板 + 四周底衬」改成
    //    「面板铺满整窗」。判据相应反过来：面板四边必须贴住窗口（旧版断言的是「留出底衬」，
    //    它现在会**故意失败**，这是版本演进的正常代价，不是回归）。
    p.check(
      '设置页：面板铺满整窗（无底衬，四边贴住窗口）',
      !!u.panel &&
        u.panel.l <= 0.5 &&
        u.panel.t <= 0.5 &&
        Math.abs(u.panel.r - u.iw) <= 0.5 &&
        u.panel.b >= u.ih - 1,
      uiJson
    )
    p.check(
      '设置页：面板无外框装饰（无 border / 圆角 / 外阴影）',
      u.panelBorder === '0px' && u.panelRadius === '0px' && u.panelShadow === 'none',
      `border=${u.panelBorder} radius=${u.panelRadius} shadow=${u.panelShadow}`
    )
    p.check(
      '设置页：正文是唯一的滚动容器（内容多时可滚动）',
      u.bdOv === 'auto' && u.htmlOv === 'hidden' && u.bodyOv === 'hidden',
      `panel-bd=${u.bdOv} html=${u.htmlOv} body=${u.bodyOv}`
    )
    p.check(
      '设置页：面板头部可拖动窗口（无边框窗只能靠自绘拖动区）',
      u.drag === 'drag',
      `-webkit-app-region = ${u.drag}`
    )
    p.check('设置页：关闭按钮自绘在面板头部', u.hasX, '未找到 #closeBtn')

    const got = (await swin.webContents.executeJavaScript(
      'window.__TAURI_INTERNALS__.invoke("get_settings").then(function(s){return JSON.stringify(s)})',
      true
    )) as string
    p.check('设置页能读到设置（get_settings 打通）', got.includes('tray_enabled'), got)

    // 完整走一遍「设置页点保存」的真实链路：页面 → 垫片 → IPC → 校验 → 应用副作用 → 落盘。
    // ⚠️ 只断言读是不够的：保存路径里有快捷键注册、托盘创建、持久化三件副作用，
    //    任何一件坏掉都只在「用户改了设置」时才暴露，而这恰好是探针最容易漏测的动作。
    const savedFromPage = (await swin.webContents.executeJavaScript(
      'window.__TAURI_INTERNALS__.invoke("save_settings",{newSettings:{' +
        'wake_hotkey:{ctrl:false,alt:true,shift:false,key:"Digit9"},' +
        'tray_enabled:true,minimize_on_close:false}})' +
        '.then(function(){return "ok"}).catch(function(e){return "err:"+String(e)})',
      true
    )) as string
    p.check('设置页点保存：调用成功', savedFromPage === 'ok', savedFromPage)
    const afterSave = ctx.m3.getSettings()
    p.check(
      '保存后内存态已更新（设置页 → 主进程）',
      afterSave.tray_enabled === true && afterSave.wake_hotkey.key === 'Digit9',
      JSON.stringify(afterSave)
    )
    p.check('保存后副作用已生效（托盘图标出现）', ctx.m3.hasTray(), 'hasTray() = false')
    const onDisk = loadSettings(ctx.m3.storeLoc).settings
    p.check(
      '保存后已持久化到磁盘',
      onDisk.tray_enabled === true && onDisk.wake_hotkey.key === 'Digit9',
      JSON.stringify(onDisk)
    )

    // 复原（后面 12/13 段的断言从「干净状态」重新开始）
    ctx.m3.saveSettings({ ...ctx.m3.getSettings(), wake_hotkey: HK({}), tray_enabled: false })
    p.check('复原：托盘图标已销毁', !ctx.m3.hasTray(), 'hasTray() = true')

    // ⚠️ 垫片必须只放行白名单：设置页是本地页面，但它是**跨壳共用**的资源，
    //    将来若被改动/被替换，白名单是最后一道闸。
    const denied = (await swin.webContents.executeJavaScript(
      'window.__TAURI_INTERNALS__.invoke("drop_database").then(function(){return "resolved"}).catch(function(){return "rejected"})',
      true
    )) as string
    p.check('未授权命令被垫片拒绝', denied === 'rejected', denied)

    // 清除缓存 / Cookie（设置页「清除全部」）：走完整链路
    // 设置页 → 垫片白名单 → IPC 注册名 → 处理器 → 返回。
    // ⚠️ 探针下处理器走**桩**分支（index.ts 传 `probe: PROBE`，见 web-data.ts）：
    //    探针跑在用户自己的机器上，把人家真实的登录态与缓存清掉是不可接受的副作用。
    //    但「页面字面量 / 垫片白名单 / IPC 命令名」三处逐字一致这条最容易断的链，
    //    桩一样能验 —— 而它断掉时的表现正是「按钮点了没反应」。
    const cleared = (await swin.webContents.executeJavaScript(
      'window.__TAURI_INTERNALS__.invoke("clear_web_data")' +
        '.then(function(r){return "ok:"+String(r)}).catch(function(e){return "err:"+String(e)})',
      true
    )) as string
    p.check(
      '设置页可调用 clear_web_data（清除缓存 / Cookie 链路打通）',
      cleared.startsWith('ok:'),
      cleared
    )

    ctx.m3.closeSettingsWindow()
    const gone = await waitUntil(async () => !ctx.m3.getSettingsWindow(), 4000)
    p.check('设置窗可关闭', gone, '4s 内未关闭')
  }

  // ---------- 14. 关闭到托盘（真窗口语义） ----------
  // 这一组必须在最后：它依赖「窗口还在」，而下一步的关闭按钮断言会把窗口真的销毁。
  ctx.m3.saveSettings({
    wake_hotkey: HK({}),
    tray_enabled: true,
    minimize_on_close: true,
  })
  win.close()
  await sleep(500)
  p.check('勾选「关闭到托盘」后点关闭：窗口未被销毁', !win.isDestroyed(), '窗口被销毁了')
  p.check('勾选「关闭到托盘」后点关闭：窗口被隐藏', !win.isVisible(), '窗口仍然可见')
  ctx.m3.toggleMainWindow()
  await sleep(400)
  p.check('显隐切换语义正确（隐藏后再切 → 显示）', win.isVisible(), '窗口仍处于隐藏')
  // 复原成「关闭即退出」，否则下一步的关闭按钮断言会被拦下来
  ctx.m3.saveSettings({
    wake_hotkey: HK({}),
    tray_enabled: false,
    minimize_on_close: false,
  })
  p.check('关闭托盘开关后图标已销毁（复原）', !ctx.m3.hasTray(), 'hasTray() = true')

  // ---------- 15. M4 页面行为：F12 / 右键菜单 / 下载接管 ----------
  // 探针在这里只验「挂上了 + F12 真能开」这类**集成**事实；
  // 「菜单该出哪些项、另存为对话框给什么默认值」属纯规则，由 test/page-behavior.test.ts 钉。
  //
  // ⚠️ 三条 listenerCount 断言是防「改了 index.ts 忘了挂」：不挂的话功能静默消失、
  //    编译照过、探针其它断言全绿 —— 正是最难发现的那一类回归。
  const wc = win.webContents
  p.check(
    'F12：已挂 before-input-event（未打包态应启用）',
    wc.listenerCount('before-input-event') > 0,
    `listenerCount=${wc.listenerCount('before-input-event')}`
  )
  p.check(
    '右键菜单：已挂 context-menu',
    wc.listenerCount('context-menu') > 0,
    `listenerCount=${wc.listenerCount('context-menu')}`
  )
  p.check(
    '下载接管：session 已挂 will-download',
    wc.session.listenerCount('will-download') > 0,
    `listenerCount=${wc.session.listenerCount('will-download')}`
  )
  // 真按一次 F12（detach 模式，不占页面空间），验证不只是「挂了个空监听器」
  wc.sendInputEvent({ type: 'keyDown', keyCode: 'F12' })
  const devtoolsOpened = await waitUntil(async () => wc.isDevToolsOpened(), 3000)
  p.check('F12：按下后开发者工具真的打开了', devtoolsOpened, '3s 内 isDevToolsOpened() 仍为 false')
  if (devtoolsOpened) wc.closeDevTools()

  // ---------- 16. 子框架桥梁隔离（iframe 不得拿到窗口按钮 / extra） ----------
  // preload 默认对每个子框架都执行；若不判断 window.self === window.top，被壳页面嵌入的
  // 任意第三方 iframe 就能在自己的主世界调 minimize()/close()/openSettings()、读到 extra。
  await nav('/frame.html')
  const frameReady = await waitUntil(async () => {
    try {
      return Boolean(await wc.executeJavaScript(
        `(() => { const f = document.getElementById('f');
           return !!(f && f.contentWindow && f.contentDocument &&
                     f.contentDocument.readyState === 'complete'); })()`,
        true
      ))
    } catch {
      return false
    }
  }, 5000)
  p.check('16 前提：同源 iframe 已加载完成', frameReady, '5s 内未就绪')
  const frameJson = (await wc.executeJavaScript(
    `(() => {
       const f = document.getElementById('f');
       if (!f || !f.contentWindow) return JSON.stringify({ err: 'no-frame' });
       // 必须在【子框架自己的 realm】里求值：contextBridge 代理有跨帧膜，父框架直接读
       // f.contentWindow.web2app 恒为 undefined（不能那样验）。
       try {
         // 再取一份子框架内的 web2appExtra（探针第 16 节）。
         const probe = f.contentWindow.eval(
           'JSON.stringify({ bridge: typeof window.web2app, extra: typeof window.web2appExtra })'
         );
         return JSON.stringify({
           topHasBridge: typeof window.web2app === 'object' &&
             typeof window.web2app.minimize === 'function',
           child: JSON.parse(probe),
         });
       } catch (e) {
         return JSON.stringify({ evalErr: String(e) });
       }
     })()`,
    true
  )) as string
  const frameInfo = JSON.parse(frameJson) as {
    err?: string
    evalErr?: string
    topHasBridge?: boolean
    child?: { bridge?: string; extra?: string }
  }
  p.check('顶层框架暴露窗口桥 web2app（功能不收窄）',
    frameInfo.topHasBridge === true, frameJson)
  p.check('子框架不暴露窗口桥（iframe 拿不到 minimize/close/openSettings）',
    frameInfo.child?.bridge === 'undefined', frameJson)
  p.check('子框架不暴露 web2appExtra',
    frameInfo.child?.extra === 'undefined', frameJson)

  // ---------- 17. 关闭按钮（收尾：会真的销毁窗口） ----------
  await clickBtn('w2t-close')
  const closed = await new Promise<boolean>((resolve) => {
    const t = setTimeout(() => resolve(false), 3000)
    win.once('closed', () => {
      clearTimeout(t)
      resolve(true)
    })
  })
  p.check('关闭按钮生效', closed, '3s 内窗口未关闭')

  // ---------- 汇总 ----------
  const failed = p.results.filter((r) => !r.ok)
  console.log('\n[probe] ===== 兼容契约断言 =====')
  for (const r of p.results) {
    console.log(`[probe] ${r.ok ? 'PASS' : 'FAIL'}  ${r.name}${r.ok ? '' : '  ← ' + r.detail}`)
  }
  console.log(`[probe] ===== ${p.results.length - failed.length}/${p.results.length} 通过 =====\n`)
  return failed.length === 0
}
