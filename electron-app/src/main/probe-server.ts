/**
 * 探针夹具站点（探针四拆分之一）—— 127.0.0.1 随机端口的 HTTP/WS 上游。
 *
 * 夹具全部内联在本模块（不落盘、不进打包产物）；上游侧把「自己实际收到了什么」
 * 记在 `startProbeServer().seen` 里供断言读取（「代理有没有注入 Authorization」
 * 只有上游知道答案）。
 *
 * 探针整体的设计动机与模块地图见桶文件 `probe.ts`。
 */
import { createHash } from 'node:crypto'
import { createServer, type Server } from 'node:http'

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

  /** 子框架隔离页：内嵌同源 iframe，验证 preload 不向任何子框架暴露特权窗口桥 */
  '/frame.html': {
    body: PAGE(
      '',
      '<div id="content"><h1>frame-host</h1>' +
        '<iframe id="f" src="/empty.html" width="200" height="120"></iframe></div>',
      ''
    ),
  },

  /** 新窗口目标页 */
  '/opened.html': {
    body: PAGE('', '<div id="content"><h1>opened</h1></div>', ''),
  },

  /**
   * 慢响应页：响应头立刻发出、响应体按 `?ms=` 延迟下发（在请求处理里特判，不在此表）。
   * 用途：制造「页面还在加载中」的确定时间窗，验证新建页签在加载完成前就已激活
   *（标签栏即时切换回归，见 probe-tabs.ts 的 P10b）。
   */
  '/slow.html': {
    body: PAGE('', '<div id="content"><h1>slow</h1></div>', '').replace(
      '<title>探针</title>',
      '<title>慢页</title>'
    ),
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
export const AUTH_HEADER = 'Basic cHJvYmU6c2VjcmV0'

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
      // /slow.html?ms=NNN：头先发出（导航即刻提交、URL 可读），body 延迟下发，
      // did-finish-load / loadURL 的 resolve 随之延后，制造确定性的「加载中」窗口。
      if (path === '/slow.html') {
        let ms = Number(new URL(req.url ?? '/slow.html', 'http://127.0.0.1').searchParams.get('ms'))
        if (!Number.isFinite(ms) || ms < 0) ms = 0
        ms = Math.min(ms, 5000) // 硬上限：防止误传把探针永久挂住
        setTimeout(() => res.end(fx.body), ms)
        return
      }
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
