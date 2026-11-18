/**
 * preload：主进程与页面之间唯一的通道，同时是**注入脚本的执行宿主**。
 *
 * ⚠️ 安全基线（不要为了「方便」破坏它）：
 *   - 只暴露【白名单方法】，绝不把 ipcRenderer / require / process 本体交给页面。
 *     本应用加载的是远程页面，暴露面越大等于把整台机器交出去。
 *   - 页面侧通过 `window.web2app` 使用；Tauri 版的 `window.__TAURI__` 在 Electron
 *     侧不存在，注入脚本按 (window.web2app ?? window.__TAURI__) 的顺序做能力探测。
 *   - 本文件运行在 sandbox 渲染进程里，只能 require('electron')，
 *     用不了 node:fs / node:path 之类的模块 —— 注入脚本的源码由主进程经
 *     `web2app:bootstrap`（同步 IPC）送进来。
 *
 * ⚠️⚠️ 注入必须落在【主世界】（`webFrame.executeJavaScript`），不要改回
 *   `new Function(src)()`（那是在隔离世界跑）。这里踩过坑，缘由值得记住：
 *     · contextBridge 暴露的 `window.web2app` **只存在于主世界**；
 *     · 但 DOM 是主世界与隔离世界【共享】的 —— 于是脚本在隔离世界跑时，标题栏照样
 *       画得出来、38px 让位也照样生效，从表象上看一切正常；
 *     · 真正失效的是三处「读 window」的逻辑：`shellName()` 永远返回 unknown、
 *       `callWindow()` 永远拿不到控制器（三个窗口按钮全部点不动，且页面侧只有一条
 *       console.error）、与启动页内联脚本（主世界）之间的防重标记互相看不见。
 *   Tauri 侧的 initialization_script 本来就在主世界执行 —— 放主世界才是行为对齐的那个选择。
 *   不受页面 CSP 限制：executeJavaScript 走 V8 直接求值（Runtime.evaluate），不是 <script> 注入。
 *
 * ⚠️ 注入时机（对齐 Tauri 版的 initialization_script）：preload 在「document 已创建、
 *   页面自身脚本尚未执行」时运行，正是 Tauri 侧 document-created 的等价时机。
 *
 * ⚠️ 只对【顶层框架】生效：webPreferences.preload 默认也作用于子框架（每个 iframe 都会
 *   执行一份）。两处都必须挡住：
 *     1. 注入：不判断 `window.self === window.top`，页面里每个 iframe 都会长标题栏；
 *     2. 桥梁：contextBridge / sendSync / 事件监听若在子框架注册，**跨源第三方 iframe**
 *        （广告 / 挂件等被壳页面嵌入的任何内容）就能在自己的主世界里调
 *        minimize()/close()/openSettings()、读到 web2appExtra，还会白白拉一份 ~80KB
 *        的 bootstrap 同步载荷。因此下面整个运行时主体都在顶层框架分支内，子框架
 *        拿到的是一个什么都没有的隔离世界 —— 白名单动作一个都不外露。
 *
 * ⚠️ 跳过内部页面（splash://）：启动页已经把这两个脚本内联在 HTML 里（同样是主世界），
 * 再注入一次会得到【两条标题栏】（同世界的 `__web2appTitle` 防重标记这时反而挡得住，
 * 但没必要依赖它）。
 */
import { contextBridge, ipcRenderer, webFrame } from 'electron'

/** 主进程送来的启动信息（见 main/index.ts 的 registerBootstrapIpc） */
interface Bootstrap {
  /** 自绘标题栏高度（逻辑像素）。与 Tauri 版一致，勿改。 */
  chromeH: number
  /** 壳标识，供注入脚本与调试使用 */
  shell: 'electron'
  /** config.json 的 extra 字段，作为 window.web2appExtra 注入 */
  extra: Record<string, unknown>
  /** 注入脚本源码（单一真源：webview-app/src/*.js） */
  titlebarJs: string
  loadingJs: string
}

/** 暴露给顶层页面的白名单窗口桥形状（仅供类型约束，勿往里加方法）。 */
export interface Web2AppApi {
  isElectron: true
  chromeH: number
  minimize(): Promise<void>
  toggleMaximize(): Promise<boolean>
  close(): Promise<void>
  openSettings(): Promise<void>
}

/** 壳 → 加载层 的指令（与注入脚本 loading.js 的状态机一一对应）。 */
export type OverlayCommand =
  | { kind: 'fail'; title: string; detail: string }
  // 预留：loading.js 实现了 slow()（慢提示），主进程当前只发 fail/hide；
  // 保留该分支是为了将来加「网络较慢」提示时不必改协议形状。
  | { kind: 'slow'; title: string }
  | { kind: 'hide' }

// 子框架（含跨源 iframe）：到此为止，不注册任何桥梁 / 监听，也不向主进程要 bootstrap。
// 实测 Electron 44 默认配置下 preload 不会注入子框架（需 webPreferences
// .nodeIntegrationInSubFrames=true），今天这条分支属于纵深防御 + 前向兼容：
// 一旦未来（或某人改配置）子框架开始执行 preload，桥梁也不会意外漏进第三方 iframe。
// 单窗口探针第 16 节用同源 iframe 钉着「子框架无桥」这一运行期事实。
//
// 2026-11-07 架构审查 M-9：主体按「桥 / 注入 / overlay 指令」拆成三个函数 ——
// ⚠️ 只能在**本文件内**分块，不能拆成独立模块：本 preload 跑在 `sandbox: true` 的渲染进程里，
//    Electron 45 起 `require` 只放行 electron（events / timers / url shim 与 Buffer /
//    setImmediate / clearImmediate 全局均已移除），相对 require 会在运行期失败。
if (window.self === window.top) {
  const boot = ipcRenderer.sendSync('web2app:bootstrap') as Bootstrap
  installBridge(boot)
  installOverlayChannel()
  installInjectScripts(boot)
  installPasswordManager()
}

/** ① 桥：`window.web2app` 白名单方法 + `window.web2appExtra`。 */
function installBridge(boot: Bootstrap): void {
  const api: Web2AppApi = {
    /** 壳标识，供注入脚本做能力探测 */
    isElectron: true,
    /** 标题栏占位高度：对齐 Tauri 版的 window.__web2appChromeH */
    chromeH: boot.chromeH,
    // ⚠️ 三个都用 invoke（而不是 send）：调用失败会变成 Promise rejection，注入脚本会
    // 把它打到控制台。用 send 的话「窗口按钮没反应」在页面侧完全无声（Tauri 版当年就是
    // 栽在静默失败上，排查从零开始）。
    minimize: (): Promise<void> => ipcRenderer.invoke('win:minimize'),
    toggleMaximize: (): Promise<boolean> => ipcRenderer.invoke('win:toggle-maximize'),
    close: (): Promise<void> => ipcRenderer.invoke('win:close'),
    // 打开设置窗（标题栏「设置」按钮，位置在最小化左侧）。
    // Tauri 侧没有对应的方法：主窗是远程页面，调不了 `open_settings` 这种应用命令，
    // 那边改发核心插件的事件（见 webview-app/src/titlebar.js 的 openSettings）。
    openSettings: (): Promise<void> => ipcRenderer.invoke('win:open-settings'),
  }

  contextBridge.exposeInMainWorld('web2app', api)

  // window.web2appExtra：与 Tauri 版同名全局变量，页面按此契约读取构建信息等。
  // 空对象不暴露，避免让页面误以为「配置里填了 extra」。
  if (boot.extra && Object.keys(boot.extra).length > 0) {
    contextBridge.exposeInMainWorld('web2appExtra', boot.extra)
  }
}

/** ② overlay 指令通道：壳 → 加载层（必须跨到主世界执行）。 */
function installOverlayChannel(): void {
  // Tauri 侧是在目标页面里 eval `window.__web2appLoading.fail(...)`（同一个世界）。
  // Electron 侧加载层挂在主世界，而本文件在隔离世界 —— 读不到对方的全局变量，
  // 所以这条指令同样要跨到主世界去执行（同一个 evalInPage 出口）。
  ipcRenderer.on('web2app:overlay', (_e, cmd: OverlayCommand) => {
    // 参数用 JSON 序列化后嵌入代码：title/detail 来自配置与运行时错误文本，可能含引号
    // 与换行；JSON 字符串字面量是合法 JS 表达式，且 U+2028/2029 需额外转义（它们是
    // JS 里的行终止符，未转义会截断语句）。
    const lit = JSON.stringify(cmd)
      .replace(/\u2028/g, '\\u2028')
      .replace(/\u2029/g, '\\u2029')
    const code =
      '(function(){var c=' + lit + ';' +
      'var L=window.__web2appLoading;' +
      'if(!L){console.warn("[web2app] 收到加载层指令时加载层未就绪："+c.kind);return;}' +
      'try{' +
      'if(c.kind==="fail")L.fail(c.title,c.detail);' +
      'else if(c.kind==="slow")L.slow(c.title);' +
      'else L.hide();' +
      '}catch(e){console.error("[web2app] 加载层指令执行失败：",e);}' +
      '})()'
    evalInPage('加载层指令', code)
  })
}

/** ③ 注入：标题栏 + 加载层（只对真实内容页，内部页面一律跳过）。 */
function installInjectScripts(boot: Bootstrap): void {
  function isInternalPage(): boolean {
    try {
      // 内部 scheme 全表（M-9）：splash / settings / shell / floaticon 都是我们自己的页面，
      // 启动页与壳/设置页已自带标题栏或采用无边框自绘，误注入会得到两条标题栏或错误留白。
      const p = location.protocol
      return (
        p === 'splash:' ||
        p === 'about:' ||
        p === 'chrome:' ||
        p === 'settings:' ||
        p === 'shell:' ||
        p === 'floaticon:'
      )
    } catch {
      return true // 拿不到 location 时宁可不注入（注入失败会静默，跳过更安全）
    }
  }

  // 桥梁对顶层框架的内部页面（splash 等）同样暴露，但脚本注入只对真实内容页：
  // 启动页已把两份脚本内联在 HTML 里，再注入会得到两条标题栏。
  if (isInternalPage()) return
  runInject('titlebar.js', boot.titlebarJs)
  runInject('loading.js', boot.loadingJs)
}

// ---- password-manager:begin（本段在 preload.ts / tab-preload.ts 中逐字一致，由 test/password-preload-contract.test.ts 锁）----
/**
 * 网页登录账号密码：提交时保存、进页面自动填充（Electron 自研）。
 *
 * 为什么不是原生能力：Electron 只含 Chromium content 层，**没有** Chrome 的密码管理器 /
 * 自动填充 UI（详见 src/main/credentials-logic.ts 文件头）。Tauri 壳走 WebView2 原生气泡。
 *
 * ⚠️ 全部逻辑跑在 preload 的**隔离世界**：只读 DOM + `ipcRenderer.invoke`，不向页面主世界
 *    暴露任何桥 —— 页面脚本无法主动读出已保存的密码。
 * ⚠️ 只在**可信 submit**（`isTrusted`）时保存：程序化 `form.submit()` 不写入，避免站点刷库。
 *    这里**不阻止提交**：DOM 气泡活不过一次导航，阻止提交再恢复会改站点行为；故采用
 *    「先保存、后提示」——提交瞬间写入，气泡只作结果反馈（可撤销）。
 */
function installPasswordManager(): void {
  const proto = location.protocol
  if (proto !== 'http:' && proto !== 'https:') return

  const isInput = (el: Element | null): el is HTMLInputElement =>
    !!el && el.tagName === 'INPUT'

  const visible = (el: HTMLElement): boolean => {
    const r = el.getBoundingClientRect()
    if (r.width <= 0 || r.height <= 0) return false
    const s = getComputedStyle(el)
    return s.visibility !== 'hidden' && s.display !== 'none'
  }

  const usableUser = (el: HTMLInputElement, pass: HTMLInputElement): boolean =>
    el !== pass &&
    !el.disabled &&
    !el.readOnly &&
    el.type !== 'password' &&
    (el.type === 'text' || el.type === 'email' || el.type === 'tel' || el.type === 'number')

  /** 找页面里第一个可见的密码框，并按 autocomplete / 相邻顺序 / name 猜用户名框。 */
  function findLogin(): { user: HTMLInputElement | null; pass: HTMLInputElement } | null {
    const passes = Array.from(document.querySelectorAll('input[type="password"]')) as HTMLInputElement[]
    for (const pass of passes) {
      if (pass.disabled || pass.readOnly || !visible(pass)) continue
      const scope: ParentNode = pass.form ?? document
      const inputs = Array.from(scope.querySelectorAll('input')) as HTMLInputElement[]
      const byAuto = inputs.find(
        (el) => usableUser(el, pass) && /username|email/i.test(el.getAttribute('autocomplete') ?? '')
      )
      let before: HTMLInputElement | null = null
      for (const el of inputs) {
        if (el === pass) break
        if (usableUser(el, pass) && visible(el)) before = el
      }
      const byName = inputs.find(
        (el) =>
          usableUser(el, pass) &&
          /user|email|account|phone|mobile|login/i.test(`${el.name} ${el.id}`)
      )
      return { user: byAuto ?? before ?? byName ?? null, pass }
    }
    return null
  }

  function setValue(el: HTMLInputElement, value: string): void {
    // 走原型上的 setter：React 等受控组件才会感知到变化（直接 el.value 会被忽略）。
    const desc = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')
    if (desc && desc.set) desc.set.call(el, value)
    else el.value = value
    el.dispatchEvent(new Event('input', { bubbles: true }))
    el.dispatchEvent(new Event('change', { bubbles: true }))
  }

  let fillTried = false
  async function tryFill(): Promise<void> {
    if (fillTried) return
    const f = findLogin()
    if (!f) return
    // 用户已经输入过（或浏览器自己填了）：不覆盖。
    if (f.pass.value !== '' || (f.user && f.user.value !== '')) {
      fillTried = true
      return
    }
    let got: unknown = null
    try {
      got = await ipcRenderer.invoke('w2a:pwd-lookup')
    } catch {
      return
    }
    const cred = got as { username?: unknown; password?: unknown } | null
    if (!cred || typeof cred.password !== 'string' || cred.password === '') return
    fillTried = true
    // 异步期间 DOM 可能重建 / 用户可能已开始输入：写之前再确认一次。
    const again = findLogin()
    if (!again || again.pass.value !== '' || (again.user && again.user.value !== '')) return
    if (again.user && typeof cred.username === 'string' && cred.username !== '') {
      setValue(again.user, cred.username)
    }
    setValue(again.pass, cred.password)
  }

  function showSavedBubble(username: string): void {
    if (!document.body) return
    const old = document.getElementById('__web2app_pwd_bubble__')
    if (old) old.remove()
    const host = document.createElement('div')
    host.id = '__web2app_pwd_bubble__'
    // 样式一律走 CSSOM（逐项赋值）：页面 CSP 的 style-src 不会拦 JS 赋值，
    // 而内联 <style> / cssText 在严格 CSP 下可能被丢弃。
    const s = host.style
    s.position = 'fixed'
    s.top = '14px'
    s.right = '14px'
    s.zIndex = '2147483647'
    s.display = 'flex'
    s.alignItems = 'center'
    s.gap = '10px'
    s.maxWidth = '340px'
    s.padding = '10px 12px'
    s.borderRadius = '8px'
    s.background = '#1f2329'
    s.color = '#fff'
    s.font = '13px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif'
    s.boxShadow = '0 6px 24px rgba(0,0,0,.35)'
    const text = document.createElement('span')
    text.textContent = username ? `已保存 ${username} 的账号密码` : '已保存这个网站的账号密码'
    const undo = document.createElement('button')
    undo.type = 'button'
    undo.textContent = '撤销'
    const b = undo.style
    b.all = 'unset'
    b.cursor = 'pointer'
    b.padding = '2px 8px'
    b.borderRadius = '6px'
    b.background = '#3a4149'
    b.color = '#9ecbff'
    b.font = 'inherit'
    undo.addEventListener('click', () => {
      void ipcRenderer.invoke('w2a:pwd-forget').finally(() => host.remove())
    })
    host.append(text, undo)
    document.body.appendChild(host)
    setTimeout(() => host.remove(), 8000)
  }

  document.addEventListener(
    'submit',
    (ev) => {
      // 只认真实用户提交（回车 / 点按钮）：`requestSubmit()` 由浏览器派发，isTrusted 仍为
      // true，故必须再要一次「当前有用户激活」。程序化提交 / 站点脚本刷库都会被挡下。
      if (!ev.isTrusted || !navigator.userActivation?.isActive) return
      const f = findLogin()
      if (!f || f.pass.value === '') return
      const username = f.user ? f.user.value : ''
      void ipcRenderer
        .invoke('w2a:pwd-save', { username, password: f.pass.value })
        .then((res: unknown) => {
          const r = res as { ok?: unknown } | null
          if (r && r.ok === true) showSavedBubble(username)
          else console.warn('[web2app] 账号密码未保存（系统加密不可用或写盘失败）')
        })
        .catch((e: unknown) => {
          console.warn('[web2app] 保存账号密码失败：', e)
        })
    },
    true
  )

  const kick = (): void => {
    void tryFill()
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', kick)
  else kick()
  window.addEventListener('load', kick)
  // 登录框常在首屏后才渲染（SPA）：聚焦输入框时再试一次；fillTried 会拦住重复查询。
  document.addEventListener(
    'focusin',
    (ev) => {
      if (isInput(ev.target as Element | null)) kick()
    },
    true
  )
}
// ---- password-manager:end ----

/**
 * 在主世界执行一段代码。
 *
 * 失败一律打到控制台：注入失败的表象是「顶部栏凭空消失」/「按钮点了没反应」，
 * 没有日志就只能靠猜。但绝不把异常抛给宿主页面。
 */
function evalInPage(label: string, code: string): void {
  try {
    const ret = webFrame.executeJavaScript(code, true) as unknown
    if (ret && typeof (ret as Promise<unknown>).then === 'function') {
      ;(ret as Promise<unknown>).catch((e: unknown) => {
        console.error(`[web2app] ${label} 执行失败：`, e)
      })
    }
  } catch (e) {
    console.error(`[web2app] ${label} 执行失败：`, e)
  }
}

function runInject(name: string, src: string): void {
  if (!src) {
    console.error(`[web2app] 注入脚本 ${name} 为空，已跳过`)
    return
  }
  // 脚本是我们自己的资产（不是页面输入），且都是自执行 IIFE。
  evalInPage(`注入 ${name}`, src)
}
