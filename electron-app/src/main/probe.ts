/**
 * 兼容契约探针（`WEB2APP_PROBE=1`）。
 *
 * 存在的理由：本壳的价值全在「与 Tauri 版行为一致」这 12 条契约上，而它们全都是
 * **错了也静默**的逻辑（顶部栏没挂上、38px 没让位、按钮没反应、新窗口开成裸窗口）。
 * Tauri 侧当年只能靠「截图 + 人眼比对」，还因为 WebView2 走 DWM 合成而拿不到内容
 * （GDI CopyFromScreen 抓到的是下层窗口、PrintWindow 全黑）——这边不必受这个罪：
 * Electron 可以 `executeJavaScript` 读 DOM、可以 `capturePage()` 截图。
 *
 * 因此把契约写成可执行断言，`pnpm run probe` 一把跑完，退出码即结论。
 * 这是双壳长期并行时「不跑偏」的唯一保障（见 docs/electron-plan.md §6）。
 *
 * 实现方式：探针自己起一个 127.0.0.1 随机端口的 HTTP 站点，逐个导航到夹具页并断言。
 * 夹具全部内联在本文件里（不落盘、不进打包产物）。
 */
import { createHash } from 'node:crypto'
import { createServer, type Server } from 'node:http'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { WebContentsView, type BrowserWindow } from 'electron'
import { screen } from 'electron'
import { MAX_TABS, TAB_BAR_HEIGHT, type TabManager } from './tabs'
import type { SidecarHandle, SidecarRequest } from './sidecar'
import {
  hotkeyLabel,
  hotkeyToAccelerator,
  loadSettings,
  parseSettings,
  sameHotkey,
  saveSettings,
  storeLocation,
  type Hotkey,
  type Settings,
  type StoreLocation,
} from './settings'
import {
  captureWindowState,
  loadWindowState,
  parseWindowState,
  planRestore,
  saveWindowState,
} from './window-state'
import type { RegisterResult } from './shortcuts'

interface Fixture {
  /** 响应头（CSP 用） */
  headers?: Record<string, string>
  body: string
}

/**
 * 夹具页骨架。
 *
 * ⚠️ `h1{margin:0}` 不是随手写的：h1 的默认 `margin: 0.67em`（2em 字号 ⇒ 约 18.8px）
 * 会**塌陷**到父容器上，把 `#content.top` 从 38 顶到 56.8 —— 于是「html 的 38px 内缩
 * 是否真的生效」这条断言测的会变成字体度量，而不是我们关心的契约。
 */
const PAGE = (head: string, body: string, style: string) => `<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><title>探针</title>
<style>html,body{margin:0;padding:0}h1{margin:0}body{font:14px/1.5 system-ui,sans-serif}${style}</style>
${head}</head><body>${body}</body></html>`

const FILLER = Array.from({ length: 200 }, (_, i) => `<p>第 ${i} 行填充内容，用于把页面撑到可滚动。</p>`).join('')

const FIXTURES: Record<string, Fixture> = {
  /** 普通页面：标题栏应挂上、38px 应让位、加载层应自行淡出 */
  '/plain.html': {
    body: PAGE('', `<div id="content"><h1>plain</h1>${FILLER}</div>`, ''),
  },

  /** 贴顶 fixed 页头：应被打上 shift 标记并下移 38px，否则被标题栏压住上半截 */
  '/fixed.html': {
    body: PAGE(
      '',
      `<header id="hdr">fixed header</header><main>${FILLER}</main>`,
      'header{position:fixed;top:0;left:0;right:0;height:44px;background:#cde}main{padding-top:60px}'
    ),
  },

  /** min-height:100vh 的整页容器：应被缩高一个标题栏，否则页脚会被窗口下沿吞掉 */
  '/vh.html': {
    body: PAGE(
      '',
      `<div id="root"><div style="height:200px">顶部</div><footer id="ftr">页脚</footer></div>`,
      '#root{min-height:100vh;display:flex;flex-direction:column}footer{margin-top:auto;height:56px;background:#ddd}'
    ),
  },

  /** fixed;inset:0 的整屏外壳：顶边应下移到 38，底边仍钉视口底（既不压头也不切尾） */
  '/inset.html': {
    body: PAGE(
      '',
      `<div id="shell"><header id="hdr" style="height:44px;background:#cde">壳头</header><div style="flex:1">${FILLER}</div><footer id="ftr" style="height:56px;background:#ddd">壳脚</footer></div>`,
      '#shell{position:fixed;inset:0;display:flex;flex-direction:column;background:#fff}'
    ),
  },

  /** sticky 页头：滚动后仍应停在 38px（不是 0） */
  '/sticky.html': {
    body: PAGE(
      '',
      `<header id="hdr">sticky header</header><main>${FILLER}</main>`,
      'header{position:sticky;top:0;height:44px;background:#cde}'
    ),
  },

  /**
   * 严格 CSP 页面：`style-src 'self'` 不允许内联样式。
   * 用途：验证「注入脚本在隔离世界执行、且样式注入若被 CSP 拦住会怎样」这两个假设
   *（隔离世界的脚本本身不受页面 CSP 限制，但注入的 <style> 元素受不受限制是本用例要问的问题）。
   */
  '/csp.html': {
    headers: { 'Content-Security-Policy': "default-src 'self'; script-src 'self'; style-src 'self'" },
    body: PAGE('', `<div id="content"><h1>csp</h1>${FILLER}</div>`, ''),
  },

  /** 空 body 页面：加载层应一直留在屏幕上（无内容可判就绪） */
  '/empty.html': {
    body: PAGE('', '', ''),
  },

  /** 新窗口目标页 */
  '/opened.html': {
    body: PAGE('', '<div id="content"><h1>opened</h1></div>', ''),
  },

  /** 页签模式夹具：带**可区分**的 <title>，供 P6 标题回传断言（标题→标签栏文本）。 */
  '/tabs-a.html': {
    body: PAGE('', '<div id="content"><h1>tabs-a</h1></div>', '').replace(
      '<title>探针</title>',
      '<title>页签A</title>'
    ),
  },
  '/tabs-b.html': {
    body: PAGE('', '<div id="content"><h1>tabs-b</h1></div>', '').replace(
      '<title>探针</title>',
      '<title>页签B</title>'
    ),
  },

  /** 带 target="_blank" 链接与 window.open 的页面 */
  '/opener.html': {
    body: PAGE(
      '',
      `<div id="content"><a id="lnk" href="/opened.html" target="_blank">新窗口链接</a></div>`,
      ''
    ),
  },

  /**
   * 要求 Basic 鉴权的页面（见 `AUTH_PATHS`）。
   * 用途：验证本地反向代理**确实**给每个请求注入了 Authorization —— 而且要先证明
   * 「不带凭据真的会被拒」（见 `直连上游必须 401` 那条），否则这条断言是假通过。
   */
  '/auth.html': {
    body: PAGE('', '<div id="content"><h1>auth</h1></div>', ''),
  },

  /**
   * 页内建立 WebSocket 的夹具。
   *
   * ⚠️ 这条用例是 M2 的**核心理由**：WebView2 的原生鉴权事件对 WebSocket 不生效，
   * 所以 Tauri 版才不得不自己写一个反向代理给 WS 握手也注入 Basic 头。Electron 侧
   * 复用同一份代理，这条断言就是「复用没有退化」的证据。
   * 连的是 `location.host`（= 本地代理的 127.0.0.1:端口），由代理转发到上游 `/ws`。
   */
  '/ws.html': {
    body: PAGE('', '<div id="content"><h1>ws</h1></div>', '').replace(
      '</body>',
      `<script>
window.__ws = { state: 'connecting', msgs: [] };
try {
  var ws = new WebSocket('ws://' + location.host + '/ws');
  ws.onopen = function () { window.__ws.state = 'open'; ws.send('hello-from-page'); };
  ws.onmessage = function (ev) { window.__ws.msgs.push(String(ev.data)); };
  ws.onerror = function () { if (window.__ws.state !== 'open') window.__ws.state = 'error'; };
  ws.onclose = function (ev) { window.__ws.closed = ev.code; };
} catch (e) { window.__ws.state = 'throw:' + e.message; }
</script></body>`
    ),
  },
}

/** 夹具站点里要求 Basic 鉴权的路径。凭据固定为 `probe:secret`（见 `AUTH_HEADER`）。 */
const AUTH_PATHS = new Set(['/auth.html', '/ws.html', '/ws'])

/**
 * `probe:secret` 的 Base64。
 * ⚠️ 与 `proxy.rs::build_basic_auth` 的格式（`base64("user:pass")`）逐字节对应，
 * 写错了会得到「代理没注入头」的假结论，故单列成常量并在断言里直接比对。
 */
const AUTH_HEADER = 'Basic cHJvYmU6c2VjcmV0'

/** 编码一个**未掩码**的文本帧（服务端 → 客户端方向按规范不掩码）。 */
function wsFrame(text: string): Buffer {
  const payload = Buffer.from(text, 'utf8')
  if (payload.length < 126) {
    return Buffer.concat([Buffer.from([0x81, payload.length]), payload])
  }
  const head = Buffer.alloc(4)
  head[0] = 0x81
  head[1] = 126
  head.writeUInt16BE(payload.length, 2)
  return Buffer.concat([head, payload])
}

/**
 * 解析一个客户端帧（客户端 → 服务端方向按规范必须掩码）。不够一整帧时返回 null。
 *
 * 手写而不是引 `ws` 包：探针的依赖越少越好，而这里只需要「一个带掩码的文本帧」这一种形状。
 * ⚠️ TCP 是字节流，一次 `data` 事件未必等于一整帧 —— 调用方必须累积后再解析（见 `acc`）。
 */
function wsParseFrame(
  buf: Buffer
): { opcode: number; text: string; consumed: number } | null {
  if (buf.length < 2) return null
  const opcode = buf[0] & 0x0f
  const masked = (buf[1] & 0x80) !== 0
  let len = buf[1] & 0x7f
  let off = 2
  if (len === 126) {
    if (buf.length < 4) return null
    len = buf.readUInt16BE(2)
    off = 4
  } else if (len === 127) {
    if (buf.length < 10) return null
    len = buf.readUInt32BE(2) * 2 ** 32 + buf.readUInt32BE(6)
    off = 10
  }
  let mask: Buffer | null = null
  if (masked) {
    if (buf.length < off + 4) return null
    mask = buf.subarray(off, off + 4)
    off += 4
  }
  if (buf.length < off + len) return null
  const payload = Buffer.from(buf.subarray(off, off + len))
  if (mask) {
    for (let i = 0; i < payload.length; i++) payload[i] ^= mask[i % 4]
  }
  return { opcode, text: payload.toString('utf8'), consumed: off + len }
}

export interface ProbeServer {
  url: string
  close: () => void
  /**
   * 上游侧观测点：夹具站点把「自己实际收到了什么」记在这里供断言读取。
   *
   * 这是整套探针的关键手法 —— 断言不靠猜，而是直接看**上游**收到了什么头。
   *（「代理有没有注入 Authorization」这个问题，只有上游知道答案。）
   */
  seen: {
    /** `/auth.html` 每次请求带的 Authorization（空串 = 没带） */
    httpAuth: string[]
    /** `/auth.html` 每次请求带的 Origin（验证代理把它重写成了上游源） */
    authOrigins: string[]
    /** WebSocket 升级请求：路径 / Authorization / Origin */
    wsUpgrades: { path: string; auth: string; origin: string }[]
    /** 上游从 WebSocket 收到的客户端消息 */
    wsFromClient: string[]
  }
}

export function startProbeServer(): Promise<ProbeServer> {
  const seen: ProbeServer['seen'] = {
    httpAuth: [],
    authOrigins: [],
    wsUpgrades: [],
    wsFromClient: [],
  }

  return new Promise((resolve, reject) => {
    const server: Server = createServer((req, res) => {
      const path = (req.url ?? '/').split('?')[0]
      const auth = req.headers.authorization ?? ''

      if (AUTH_PATHS.has(path)) {
        seen.httpAuth.push(auth)
        seen.authOrigins.push(req.headers.origin ?? '')
        if (auth !== AUTH_HEADER) {
          // 带上 WWW-Authenticate 才是规范的 401，Chromium 也才会去走 login 流程
          res.writeHead(401, {
            'www-authenticate': 'Basic realm="probe"',
            'content-type': 'text/plain; charset=utf-8',
          })
          res.end('需要鉴权')
          return
        }
      }

      const fx = FIXTURES[path]
      if (!fx) {
        res.writeHead(404, { 'content-type': 'text/plain; charset=utf-8' })
        res.end('not found')
        return
      }
      res.writeHead(200, {
        'content-type': 'text/html; charset=utf-8',
        'cache-control': 'no-store',
        ...(fx.headers ?? {}),
      })
      res.end(fx.body)
    })

    // WebSocket 升级：手写一个最小的 WS 上游（101 + 一个文本帧 + echo）。
    // 上游按规范要校验 Sec-WebSocket-Key 并回正确的 Accept，否则 tokio-tungstenite
    //（代理侧的上游客户端）会拒连，探针就会误判成「代理的 WS 支持坏了」。
    server.on('upgrade', (req, socket) => {
      const path = (req.url ?? '/').split('?')[0]
      const auth = req.headers.authorization ?? ''
      seen.wsUpgrades.push({ path, auth, origin: req.headers.origin ?? '' })

      if (path !== '/ws' || auth !== AUTH_HEADER) {
        socket.write('HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n')
        socket.destroy()
        return
      }

      const key = String(req.headers['sec-websocket-key'] ?? '')
      const accept = createHash('sha1')
        .update(key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11')
        .digest('base64')
      socket.write(
        'HTTP/1.1 101 Switching Protocols\r\n' +
          'Upgrade: websocket\r\n' +
          'Connection: Upgrade\r\n' +
          `Sec-WebSocket-Accept: ${accept}\r\n\r\n`
      )
      // 服务端主动先发一条：验证「上游 → 客户端」方向也通
      socket.write(wsFrame('hello-from-upstream'))

      let acc = Buffer.alloc(0)
      socket.on('data', (chunk: Buffer) => {
        acc = Buffer.concat([acc, chunk])
        for (;;) {
          const f = wsParseFrame(acc)
          if (!f) break
          acc = acc.subarray(f.consumed)
          if (f.opcode === 0x8) {
            socket.end()
            return
          }
          if (f.opcode === 0x1) {
            seen.wsFromClient.push(f.text)
            socket.write(wsFrame('echo:' + f.text))
          }
        }
      })
      socket.on('error', () => {
        /* 探针收尾时连接会断，属正常 */
      })
    })

    server.on('error', reject)
    // 端口交给系统挑：避免与本地反向代理（随机端口）或用户环境撞车
    server.listen(0, '127.0.0.1', () => {
      const addr = server.address()
      const port = typeof addr === 'object' && addr ? addr.port : 0
      resolve({
        url: `http://127.0.0.1:${port}`,
        close: () => server.close(),
        seen,
      })
    })
  })
}

// ---------------------------------------------------------------------------
// 断言
// ---------------------------------------------------------------------------

interface Rect {
  top: number
  bottom: number
  left: number
  right: number
  w: number
  h: number
}

interface Snapshot {
  barCount: number
  barRect: Rect | null
  barBg: string | null
  barZ: string | null
  appRegion: string | null
  shell: string | null
  chromeH: string | null
  htmlPadTop: string
  overlay: boolean
  extra: string | null
  api: string | null
  title: string | null
  buttons: number
  shiftAttr: boolean
  shrinkAttr: boolean
  r: { hdr: Rect | null; root: Rect | null; shell: Rect | null; ftr: Rect | null; content: Rect | null }
  view: { ih: number; iw: number; ch: number; cw: number }
  /**
   * 从右往左扫，第一个「能命中标题栏」的 x。
   *
   * ⚠️⚠️ 这是判断「标题栏有没有被滚动条盖掉一角」的**真判据**，不是 `barRect.width`。
   * Chromium 把视口滚动条绘制在所有 DOM 内容之上，标题栏的布局宽度一直是窗口宽
   * （`rect.width == innerWidth`，断言全绿），但那 15px 在绘制时被覆盖 —— 只有命中测试
   * 感受得到：滚动条区域的 `elementFromPoint` 返回 null，扫不过去。
   */
  hitRight: number
  /**
   * 「滚动条有没有越过标题栏」的**直接**判据：标题栏那 38px 里，窗口最右一列逐点做命中测试，
   * 把「命不中标题栏」的 y 收进来。空数组 = 干净。
   * 根滚动条时代这里是一串 `y`（滚动条不是 DOM 元素，那一列命不中任何东西）。
   */
  intrudeRows: number[]
  /** `body.offsetWidth - clientWidth`：滚动条**仍然存在**且归 body 管（占宽一条）时 > 0 */
  bodyScrollbarW: number
  /** 期望 `auto`：body 才是滚动容器 */
  bodyOverflowY: string | null
  /** 期望 `hidden`：html 不再承担滚动（否则根滚动条会横穿标题栏） */
  rootOverflowY: string | null
  /** 期望 `38px`：body 盒子被推到标题栏下沿 */
  bodyMarginTop: string | null
  /** 期望 ≈ 视口高度 - 38（页面写 `min-height:100vh` 时这里会露馅） */
  bodyH: number | null
  /** 页面是否仍能滚动（滚动能力必须保留 —— 不能靠「让页面滚不动」换掉那条缝） */
  bodyScrollable: boolean
  /** 旧契约观测量：html 是否还在滚（新契约下必须为 false） */
  rootScrollable: boolean
  scrollY: number
  path: string
}

/** 在主世界读一份几何快照（DOM 对主世界与隔离世界共享，注入脚本的结果都能读到）。 */
const SNAPSHOT_JS = `(() => {
  const R = (sel) => { const e = document.querySelector(sel); if (!e) return null;
    const r = e.getBoundingClientRect();
    return { top:+r.top.toFixed(1), bottom:+r.bottom.toFixed(1), left:+r.left.toFixed(1), right:+r.right.toFixed(1), w:+r.width.toFixed(1), h:+r.height.toFixed(1) }; };
  const de = document.documentElement;
  const bar = document.getElementById('__web2app_titlebar__');
  let region = null, bg = null, z = null;
  if (bar) { const cs = getComputedStyle(bar); bg = cs.backgroundColor; z = cs.zIndex;
    try { region = cs.webkitAppRegion || cs.getPropertyValue('-webkit-app-region'); } catch (e) {} }
  const root = document.querySelector('#root');
  const hdr = document.querySelector('#hdr');
  return {
    barCount: document.querySelectorAll('#__web2app_titlebar__').length,
    barRect: bar ? R('#__web2app_titlebar__') : null,
    barBg: bg, barZ: z, appRegion: region,
    shell: de.getAttribute('data-web2app-shell'),
    chromeH: de.getAttribute('data-web2app-chrome-h'),
    htmlPadTop: getComputedStyle(de).paddingTop,
    overlay: !!document.getElementById('__web2app_loading__'),
    extra: (window.web2appExtra && typeof window.web2appExtra === 'object') ? JSON.stringify(window.web2appExtra) : null,
    api: (window.web2app && typeof window.web2app === 'object') ? Object.keys(window.web2app).sort().join(',') : null,
    title: bar ? (bar.querySelector('.w2t-title') || {}).textContent : null,
    buttons: ['w2t-back','w2t-fwd','w2t-min','w2t-max','w2t-close'].filter((c) => bar && bar.querySelector('.' + c)).length,
    shiftAttr: !!document.querySelector('[data-web2app-chrome-shift]'),
    shrinkAttr: !!document.querySelector('[data-web2app-chrome-shrink]'),
    r: { hdr: R('#hdr'), root: R('#root'), shell: R('#shell'), ftr: R('#ftr'), content: R('#content') },
    view: { ih: window.innerHeight, iw: window.innerWidth, ch: de.clientHeight, cw: de.clientWidth },
    // ⚠️ 命中测试：被滚动条占住的那一段 elementFromPoint 返回 null，所以「扫得到右缘」
    //    才等价于「标题栏右端没有被盖」。y=20 取在标题栏高度内（38px）。
    hitRight: (() => { for (let x = window.innerWidth - 1; x >= 0; x--) {
      const e = document.elementFromPoint(x, 20);
      if (e && e.closest && e.closest('#__web2app_titlebar__')) return x; } return -1; })(),
    // 「滚动条有没有越过标题栏」的**直接**判据：标题栏那 38px 里，窗口最右一列逐点命中标题栏。
    // 根滚动条时代这里会拿到一串 y（滚动条不是 DOM 元素，那一列命不中任何东西）。
    intrudeRows: (() => { const bad = [];
      for (let y = 0; y < 38; y++) {
        const e = document.elementFromPoint(window.innerWidth - 1, y);
        if (!(e && e.closest && e.closest('#__web2app_titlebar__'))) bad.push(y);
      } return bad; })(),
    bodyScrollbarW: document.body ? document.body.offsetWidth - document.body.clientWidth : 0,
    bodyOverflowY: document.body ? getComputedStyle(document.body).overflowY : null,
    rootOverflowY: getComputedStyle(de).overflowY,
    bodyMarginTop: document.body ? getComputedStyle(document.body).marginTop : null,
    bodyH: document.body ? Math.round(document.body.getBoundingClientRect().height) : null,
    bodyScrollable: document.body ? document.body.scrollHeight > document.body.clientHeight : false,
    rootScrollable: de.scrollHeight > de.clientHeight,
    scrollY: window.scrollY,
    path: location.pathname
  };
})()`

class Probe {
  readonly results: { name: string; ok: boolean; detail: string }[] = []

  check(name: string, ok: boolean, detail = ''): void {
    this.results.push({ name, ok, detail: ok ? '' : detail })
  }

  /** 数值近似比较，失败时把两个值都打出来（探针的价值就在这些数字上） */
  near(name: string, got: number | null | undefined, want: number, tol = 1.5): void {
    const ok = typeof got === 'number' && Math.abs(got - want) <= tol
    this.check(name, ok, `期望 ${want}±${tol}，实际 ${got}`)
  }
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms))

/** 轮询等待条件成立。加载层淡出这类「有动画时长」的断言必须用它，固定 sleep 会假失败。 */
const waitUntil = async (cond: () => Promise<boolean>, ms: number): Promise<boolean> => {
  const t0 = Date.now()
  for (;;) {
    if (await cond()) return true
    if (Date.now() - t0 > ms) return false
    await sleep(120)
  }
}

/**
 * 标题栏样式的浅 / 深两套背景色（`#eef0f2` / `#1f2530`，跟随 `prefers-color-scheme`）。
 * 断言只能「落在已知配色里」，不能写死浅色 —— 否则深色系统上必然误报。
 */
const BAR_BG = ['rgb(238, 240, 242)', 'rgb(31, 37, 48)']

/**
 * 进程是否还活着。
 * `kill(pid, 0)` 不发送任何信号，只做存在性检查 —— Windows 上 Node 用 OpenProcess 实现。
 * 用来断言「sidecar 被 stop() 之后真的退出了」，而不是留下一个仍在监听端口的孤儿。
 */
function isAlive(pid: number): boolean {
  if (!pid) return false
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

/** M3（系统集成）断言用的真实入口 —— 与设置窗 / 托盘 / 快捷键走的**完全同一批函数**。 */
export interface M3ProbeHooks {
  settingsLoc: StoreLocation
  windowStateLoc: StoreLocation
  getSettings: () => Settings
  /** 走真实保存路径（校验 → 应用副作用 → 持久化），抛错即失败 */
  saveSettings: (next: Settings) => void
  openSettingsWindow: () => void
  closeSettingsWindow: () => void
  getSettingsWindow: () => BrowserWindow | null
  applyWakeHotkey: (h: Hotkey) => RegisterResult
  releaseShortcuts: () => void
  registerSettingsShortcut: (fn: () => void) => boolean
  toggleMainWindow: () => void
  hasTray: () => boolean
  destroyTray: () => void
  trayIconPath: () => string | null
}

export interface ProbeContext {
  win: BrowserWindow
  base: string
  /** 夹具站点的观测点（M2 断言要看上游实际收到了什么） */
  seen: ProbeServer['seen']
  /** 启动 sidecar（本地反向代理 / DNS 解析） */
  startSidecar: (req: SidecarRequest) => Promise<SidecarHandle>
  /** M3 系统集成的入口 */
  m3: M3ProbeHooks
}

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
  p.check('五个按钮齐全', s.buttons === 5, `实际 ${s.buttons}`)
  p.check('window.web2app 白名单 API 就位', s.api === 'chromeH,close,isElectron,minimize,toggleMaximize', `实际 ${s.api}`)
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
    p.check(
      'sidecar 启动并完成握手',
      /^http:\/\/127\.0\.0\.1:\d+\/$/.test(side.proxyUrl),
      `proxyUrl=${side.proxyUrl}`
    )
  } catch (e) {
    p.check('sidecar 启动并完成握手', false, `启动失败：${(e as Error).message}`)
  }

  if (side) {
    // 先证明「不带凭据确实会被拒」：否则后面的「代理带上了凭据」是假通过
    //（比如夹具站点压根没在鉴权）。
    const direct = await fetch(`${base}/auth.html`)
    p.check('直连上游（不带凭据）被拒 401', direct.status === 401, `实际 ${direct.status}`)

    const viaProxy = await fetch(`${side.proxyUrl}auth.html`)
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
    await fetch(`${side.proxyUrl}auth.html`, { headers: { Origin: 'http://127.0.0.1:9999' } })
    const lastOrigin = ctx.seen.authOrigins[ctx.seen.authOrigins.length - 1]
    p.check(
      '上游收到的 Origin 被改写为上游源（且只有一个）',
      lastOrigin === `http://${upstream.host}`,
      `实际 ${lastOrigin === undefined ? '(未记录)' : JSON.stringify(lastOrigin)}`
    )

    // ---- WebSocket：M2 的核心用例 ----
    // WebView2 时代「页面能开但实时联动失效」的根因就是 WS 握手没带凭据。
    await win.loadURL(side.proxyUrl + 'ws.html')
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
    'data\\ 无同名文件时写向旧版位置（%APPDATA%\\<app_name>\\settings.json）',
    fakeLoc.write === join(tmpdir(), 'w2a-no-such-appdata', 'Demo_App', 'settings.json'),
    fakeLoc.write
  )
  p.check(
    '读取候选同时覆盖 data\\ 与旧版目录（先本地后旧版）',
    fakeLoc.read.length === 2 &&
      fakeLoc.read[0] === join(tmpdir(), 'w2a-no-such-data', 'settings.json') &&
      fakeLoc.read[1] === fakeLoc.write,
    JSON.stringify(fakeLoc.read)
  )

  // 11.4 真实读写 roundtrip（写临时目录，绝不碰用户设置）
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
  } finally {
    rmSync(tmpStore, { recursive: true, force: true })
  }

  // 11.5 窗口几何解析与恢复策略
  const ws1 = parseWindowState({ x: 100, y: 200, width: 800, height: 600 })
  p.check('window-state 缺 maximized → false（兼容旧版文件）', ws1 !== null && !ws1.maximized, JSON.stringify(ws1))
  p.check('window-state 缺坐标 → 视为无效', parseWindowState({ width: 800, height: 600 }) === null)
  p.check('window-state 非对象 → 无效', parseWindowState('nope') === null)

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
    saveWindowState(ctx.m3.windowStateLoc, live)
    const reloaded = loadWindowState(ctx.m3.windowStateLoc)
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
    p.check('设置窗未注入自绘标题栏（它是带系统标题栏的普通窗）', barCount === 0, `barCount=${barCount}`)

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
    const onDisk = loadSettings(ctx.m3.settingsLoc).settings
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

  // ---------- 15. 关闭按钮（收尾：会真的销毁窗口） ----------
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

// ---------------------------------------------------------------------------
// 多页签模式断言组（M5 T2）—— 对应 docs/electron-tabs-design.md §8 的 P1/P2/P4/P6/P7/P8/P12
// ---------------------------------------------------------------------------

export interface TabsProbeContext {
  win: BrowserWindow
  base: string
  /** index.ts 已挂到主窗口上的页签管理器（复用真实 onSpawn 链路，不另建实例）。 */
  tabs: TabManager
}

/**
 * 跑多页签断言组。与 `runProbe` 分开：单窗口断言组（0–15）假设「主窗口自身是远程页面」，
 * 而页签模式下主窗口自身是壳页面（shell.html），没有 titlebar.js 注入的按钮/留白，
 * 那些断言不适用。这里只验页签本体语义。
 *
 * ⚠️ 判据成对（项目铁律）：对「侵入类」缺陷，位置 + 存在性都要断言。
 *   P4 的「仅激活页签可见」若只写「激活可见」会被「全都可见靠 z-order 盖」蒙混；
 *   若只写「只有一个可见」会被「干脆全删」满足 —— 所以两者都写。
 */
export async function runTabsProbe(ctx: TabsProbeContext): Promise<boolean> {
  const { win, base, tabs } = ctx
  const p = new Probe()

  const children = (): WebContentsView[] => win.contentView.children as WebContentsView[]
  const contentBounds = () => {
    const cb = win.getContentBounds()
    return { x: 0, y: TAB_BAR_HEIGHT, width: cb.width, height: cb.height - TAB_BAR_HEIGHT }
  }
  // 读壳页面标签栏里每个 .tab .label 的文本（标题回传的最终呈现处）
  const shellLabels = async (): Promise<string[]> =>
    (await win.webContents.executeJavaScript(
      `Array.from(document.querySelectorAll('.tab .label')).map((e) => e.textContent)`,
      true
    )) as string[]

  // ---------- 初始态 ----------
  p.check('页签初始为空（无视图）', tabs.count === 0 && children().length === 0,
    `count=${tabs.count} children=${children().length}`)

  // ---------- P2 + P1：建第一个页签（首屏，走 resolveInitialTarget 真实链路） ----------
  const label1 = await tabs.spawnTab()
  p.check('首个页签创建成功并激活', label1 !== null && tabs.active?.label === label1,
    `label1=${label1}`)
  const b = contentBounds()
  const v1 = tabs.active?.view
  const g1 = v1?.getBounds()
  p.check('P1 内容视图 bounds = {x:0, y:38, w:W, h:H-38}（getContentBounds 推算）',
    !!g1 && g1.x === b.x && g1.y === b.y && g1.width === b.width && g1.height === b.height,
    `实际 ${JSON.stringify(g1)}，期望 ${JSON.stringify(b)}`)
  p.check('P2 视图存在性（children 数 = 页签数）', children().length === tabs.count,
    `children=${children().length} count=${tabs.count}`)

  // ---------- P4：建第二个页签，验证切换后「仅激活页签可见」 ----------
  const label2 = await tabs.spawnTab(`${base}/tabs-b.html`)
  p.check('第二个页签创建成功并激活', label2 !== null && tabs.active?.label === label2,
    `label2=${label2}`)
  let views = children()
  let visible = views.filter((v) => v.getVisible())
  p.check('P4 新建后仅新页签可见（其余全隐藏）',
    visible.length === 1 && visible[0] === tabs.active?.view,
    `可见视图数=${visible.length}`)

  // 切回第一个，验证可见性翻转（位置 + 存在性成对）
  tabs.activate(label1!)
  await sleep(150)
  views = children()
  visible = views.filter((v) => v.getVisible())
  p.check('P4 切回后仅目标页签可见（不靠 z-order 蒙混）',
    visible.length === 1 && visible[0] === tabs.active?.view && tabs.active?.label === label1,
    `可见视图数=${visible.length} active=${tabs.active?.label}`)

  // ---------- P6：标题回传（page-title-updated → setTabTitle → 壳标签栏文本） ----------
  // label2 加载的是 /tabs-b.html（<title>页签B</title>）。等待事件链路把标题推进壳 DOM。
  const titleOk = await waitUntil(async () => {
    const labels = await shellLabels()
    return labels.includes('页签B')
  }, 5000)
  p.check('P6 页面标题回传到标签栏文本（页签B）', titleOk,
    `壳标签栏=${JSON.stringify(await shellLabels())}`)

  // ---------- 契约：get_tabs 返回元组数组（对齐 Tauri Vec<(String,String,bool)>） ----------
  // shell.html 初始化用 list.map(([label,title,active]) => …) 解构（shell.html:266），
  // 若这里退回对象数组，解构得到 [undefined,undefined,undefined]，标签栏渲染空白。
  const tuples = tabs.listForIpc()
  p.check('get_tabs 契约：返回 [label,title,active] 元组数组（非对象）',
    tuples.length === tabs.count &&
      tuples.every((t) =>
        Array.isArray(t) && t.length === 3 &&
        typeof t[0] === 'string' && typeof t[1] === 'string' && typeof t[2] === 'boolean'
      ),
    JSON.stringify(tuples))

  // ---------- P7：关闭激活页签 → 激活态落到相邻（优先右侧），不悬空不全隐 ----------
  // 当前激活是 label1；先激活 label1，再关闭它，激活应落到 label2（右侧相邻）。
  tabs.activate(label1!)
  const before = tabs.count
  const closed = tabs.closeTab(label1!)
  p.check('P7 关闭激活页签成功', closed && tabs.count === before - 1,
    `count ${before} → ${tabs.count}`)
  p.check('P7 关闭后激活态落到相邻页签（不悬空）',
    tabs.active?.label === label2 && tabs.count === 1,
    `active=${tabs.active?.label} count=${tabs.count}`)
  views = children()
  visible = views.filter((v) => v.getVisible())
  p.check('P7 关闭后剩余页签仍可见（不全隐）',
    visible.length === 1 && visible[0] === tabs.active?.view,
    `可见视图数=${visible.length}`)
  p.check('P7 关闭后 children 数同步减少（位置 + 存在性）',
    children().length === tabs.count, `children=${children().length} count=${tabs.count}`)

  // ---------- P9 / P10：window.open 只新增 1 页签 + target path 保留 ----------
  // 当前只剩 label2（激活，加载 /tabs-b.html）。从它发起 window.open('/opened.html')，
  // 走 index.ts 的 setWindowOpenHandler → spawnTab(opened)，验证「一次点击只开 1 个」且
  // 「目标 path 未被降级成入口地址（/plain.html）」。入口 = cfg.app_url = /plain.html。
  const countBeforeOpen = tabs.count
  await tabs.active!.view.webContents.executeJavaScript(
    `window.open('/opened.html', '_blank'); true`, true)
  // setWindowOpenHandler 用 setImmediate 异步 spawnTab。count 在 spawnTab 一开头就 +1，
  // 但激活（activate）在 onSpawn（loadURL /opened.html）完成之后 —— 所以「新页签可见」要
  // 等到 active 换成新 label 且其 URL 已落到 /opened.html，不能只看 count。
  const openedReady = await waitUntil(async () => {
    if (tabs.count !== countBeforeOpen + 1) return false
    const a = tabs.active
    if (!a || a.label === label2) return false
    try {
      return new URL(a.view.webContents.getURL()).pathname === '/opened.html'
    } catch {
      return false
    }
  }, 4000)
  const newUrl = tabs.active?.view.webContents.getURL() ?? ''
  const newPath = (() => { try { return new URL(newUrl).pathname } catch { return newUrl } })()
  p.check('P9 一次 window.open 只新增 1 个页签',
    openedReady && tabs.count === countBeforeOpen + 1,
    `count ${countBeforeOpen} → ${tabs.count}`)
  p.check('P10 target path 保留（未被降级成入口地址 /plain.html）',
    newPath === '/opened.html',
    `新页签 URL=${newUrl}（path=${newPath}）`)
  // 收掉 window.open 新开的页签，恢复单一页签稳态，再进后续断言
  if (tabs.active && tabs.active.label !== label2) tabs.closeTab(tabs.active.label)
  tabs.activate(label2!)

  // ---------- P11：特权命令 sender 校验（内容视图发 close_host 必须被拒） ----------
  // 生产内容视图（spawnTab）刻意不挂 preload，远程页面连 ipcRenderer 都碰不到；这里用
  // 探针专用 preload 把 IPC 通道硬塞给一个独立内容视图，模拟「未来误给内容视图加 preload」
  // 的最坏情形，验证 assertShell 的 sender.id 校验仍会拒绝它（且窗口不被误关）。
  const probePreload = join(__dirname, '..', 'renderer', 'tabs-probe-preload.js')
  const rogueView = new WebContentsView({
    webPreferences: {
      preload: probePreload,
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      backgroundThrottling: false,
    },
  })
  win.contentView.addChildView(rogueView)
  rogueView.setBounds(contentBounds())
  await rogueView.webContents.loadURL(`${base}/plain.html`).catch(() => {})
  await sleep(400) // 等 preload 注入 + 页面就绪
  const shellId = win.webContents.id
  const rogueId = rogueView.webContents.id
  p.check('P11 前提：内容视图 sender.id ≠ 壳 webContents.id（可区分）',
    rogueId !== shellId, `壳=${shellId} 内容=${rogueId}`)
  const rogueResult = (await rogueView.webContents.executeJavaScript(
    `window.__web2appProbeInvoke.invoke('tabs:close_host')` +
      `.then(() => 'OK').catch((e) => 'REJECTED:' + String(e && e.message))`,
    true
  )) as string
  const winStillAlive = !win.isDestroyed()
  p.check('P11 内容视图发特权命令 close_host 被拒（且窗口未误关）',
    rogueResult.startsWith('REJECTED') && winStillAlive,
    `结果=${rogueResult}；窗口存活=${winStillAlive}`)
  // 回收探针内容视图
  try { win.contentView.removeChildView(rogueView); rogueView.webContents.close() } catch { /* noop */ }

  // ---------- P12：层级语义（重复 addChildView 不增条目、抬到末尾） ----------
  const activeView = tabs.active?.view
  if (activeView) {
    const nBefore = children().length
    win.contentView.addChildView(activeView) // 重复 add 已存在视图
    const after = children()
    p.check('P12 重复 addChildView 不增条目', after.length === nBefore,
      `${nBefore} → ${after.length}`)
    p.check('P12 重复 add 后视图被抬到数组末尾（z-order 顶层）',
      after[after.length - 1] === activeView, '视图不在末尾')
  }

  // ---------- P8：熔断（连开至上限，第 N+1 次拒绝且 children 不增长） ----------
  // 用 about:blank 快速填充（不解析 target、不渲染），避免 32 个真实页拖慢探针。
  const labels: string[] = [label2!]
  while (tabs.count < MAX_TABS) {
    const l = await tabs.spawnTab('about:blank')
    if (l === null) break
    labels.push(l)
  }
  p.check('P8 连开至上限（32）', tabs.count === MAX_TABS, `count=${tabs.count}`)
  const countAtLimit = tabs.count
  const over = await tabs.spawnTab('about:blank')
  p.check('P8 第 N+1 次新建被拒绝（返回 null）', over === null, `over=${over}`)
  p.check('P8 超限后 children 不再增长', children().length === countAtLimit,
    `children=${children().length} count=${countAtLimit}`)

  // ---------- 清理：关掉所有页签，验证「关最后一个 → 关闭宿主」的语义不在这里触发 ----------
  // ⚠️ 不能真的关到最后一个（那会 host.close()，探针还没汇总）。留一个，其余 close。
  const rest = labels.slice(1) // 去掉第一个（label2）
  for (const l of rest) tabs.closeTab(l)
  p.check('清理：关闭到只剩一个页签', tabs.count === 1, `count=${tabs.count}`)

  // ---------- 汇总 ----------
  const failed = p.results.filter((r) => !r.ok)
  console.log('\n[probe] ===== 多页签断言组 =====')
  for (const r of p.results) {
    console.log(`[probe] ${r.ok ? 'PASS' : 'FAIL'}  ${r.name}${r.ok ? '' : '  ← ' + r.detail}`)
  }
  console.log(`[probe] ===== ${p.results.length - failed.length}/${p.results.length} 通过 =====\n`)
  return failed.length === 0
}
