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

// ---- password-manager:begin（本段在 preload.ts / tab-preload.ts 中逐字一致，由 test/password-contract.test.ts 锁）----
/**
 * 网页登录账号密码：提交时保存、进页面自动填充（Electron 自研）。
 *
 * 填充触发时机：首屏（DOMContentLoaded / load）、**URL 变化**（主世界 history 钩子覆盖
 * pushState / replaceState，另有 popstate / hashchange / pageshow）、**DOM 变动**
 * （MutationObserver 防抖）与 focusin。只要当前页面出现空着的密码框就填，不需要用户先点
 * 输入框；全程事件驱动，**没有常驻轮询定时器**。
 *
 * 为什么不是原生能力：Electron 只含 Chromium content 层，**没有** Chrome 的密码管理器 /
 * 自动填充 UI（详见 src/main/credentials-logic.ts 文件头）。Tauri 壳走 WebView2 原生气泡。
 *
 * ⚠️ 全部逻辑跑在 preload 的**隔离世界**：只读 DOM + `ipcRenderer.invoke`，不向页面主世界
 *    暴露任何桥 —— 页面脚本无法主动读出已保存的密码。
 * ⚠️ 只在**可信 submit**（`isTrusted`）时**暂存**：程序化 `form.submit()` / 站点脚本刷库都会被挡下。
 *    这里**不阻止提交**：DOM 气泡活不过一次导航，阻止提交再恢复会改站点行为；故采用
 *    「提交只暂存（主进程内存 60s，不落盘）→ 确认登录成功（新文档没有可见密码框 / 本页表单消失）
 *    才真正保存」——输错密码不会覆盖已存的好密码。
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
  let filling = false
  let lookupUrl = ''
  let lookupDone = false
  let cached: { username: string; password: string } | null = null
  /** 上一次填充的密码框：被整块替换（SPA 重新渲染登录表单）时允许对新表单再填一次。 */
  let filledPass: HTMLInputElement | null = null

  /** URL 变了就重置填充状态：SPA 退出登录后同一文档换路由，必须重新自动填。 */
  function syncUrl(): void {
    if (location.href === lookupUrl) return
    lookupUrl = location.href
    fillTried = false
    lookupDone = false
    cached = null
    filledPass = null
  }

  async function tryFill(): Promise<void> {
    syncUrl()
    if (fillTried || filling) return
    const f = findLogin()
    if (!f) return
    // 密码框已有内容：用户（或站点）已经处理过，不覆盖；用户名预填不影响补密码。
    if (f.pass.value !== '') {
      fillTried = true
      return
    }
    filling = true
    try {
      if (!lookupDone) {
        lookupDone = true
        try {
          const got = (await ipcRenderer.invoke('w2a:pwd-lookup')) as
            | { username?: unknown; password?: unknown }
            | null
          cached =
            got && typeof got.password === 'string' && got.password !== ''
              ? {
                  username: typeof got.username === 'string' ? got.username : '',
                  password: got.password,
                }
              : null
        } catch {
          // 主进程暂时不可用：别把这个 URL 永久判成「没存过」，留给下次触发重试。
          lookupDone = false
          cached = null
        }
      }
      if (!cached) {
        // 当前站点没存过：本 URL 下不再反复查（URL 变化会重置，登录后再退出仍能重填）。
        fillTried = true
        return
      }
      // 异步期间 DOM 可能重建 / URL 可能已变：写之前再确认一次。
      if (location.href !== lookupUrl) return
      const again = findLogin()
      if (!again || again.pass.value !== '') return
      const typedUser = again.user ? again.user.value.trim() : ''
      // 站点预填了**另一个**账号时不能把保存的密码灌进去，否则会登错账号。
      if (
        typedUser !== '' &&
        cached.username !== '' &&
        typedUser.toLowerCase() !== cached.username.toLowerCase()
      ) {
        fillTried = true
        return
      }
      // 用户名空缺才写；站点已填同一个账号时只补密码。
      if (again.user && again.user.value === '' && cached.username !== '') {
        setValue(again.user, cached.username)
      }
      setValue(again.pass, cached.password)
      fillTried = true
      filledPass = again.pass
      showBubble('密码已自动填充')
    } finally {
      filling = false
    }
  }

  /**
   * 右上角状态气泡：保存成功带「删除」按钮（直接删除该站点凭据），自动填充只做提示、无操作。
   * 按钮文案用「删除」而不是「撤销」—— 语义是删掉已存凭据，不是撤回一次操作。
   */
  function showBubble(text: string, action?: { label: string; onClick: () => void }): void {
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
    const span = document.createElement('span')
    span.textContent = text
    host.append(span)
    if (action) {
      const btn = document.createElement('button')
      btn.type = 'button'
      btn.textContent = action.label
      const b = btn.style
      b.all = 'unset'
      b.cursor = 'pointer'
      b.padding = '2px 8px'
      b.borderRadius = '6px'
      b.background = '#3a4149'
      b.color = '#9ecbff'
      b.font = 'inherit'
      btn.addEventListener('click', action.onClick)
      host.append(btn)
    }
    document.body.appendChild(host)
    // 纯提示短一些，带操作的留足点击时间。
    setTimeout(() => host.remove(), action ? 8000 : 4000)
  }

  let stagedHere = false
  let commitProbed = false
  let committing = false

  /**
   * 提交后**先暂存、不落盘**：只有确认登录成功（离开登录页 / 登录表单消失）才真正保存，
   * 避免输错一次密码就覆盖掉已存的好密码。主进程只在内存里暂存，60s 过期、不写盘。
   */
  async function maybeCommit(): Promise<void> {
    if (committing) return
    committing = true
    try {
      const res = (await ipcRenderer.invoke('w2a:pwd-commit', {
        hasPasswordField: findLogin() !== null,
      })) as { ok?: unknown; reason?: unknown; username?: unknown } | null
      if (res && res.ok === true) {
        stagedHere = false
        const username = typeof res.username === 'string' ? res.username : ''
        showBubble(username ? `已保存 ${username} 的账号密码` : '已保存这个网站的账号密码', {
          label: '删除',
          onClick: () => {
            void ipcRenderer.invoke('w2a:pwd-forget').finally(() => {
              const el = document.getElementById('__web2app_pwd_bubble__')
              if (el) el.remove()
            })
          },
        })
      } else if (res) {
        // 只要能拿到明确答复就**停止等待**：`still-login`（失败重载 / 多步登录，主进程已丢弃）、
        // `none`（暂存已过期或被消费）、写盘失败 / 加密不可用 —— 本页都不再重试。
        // ⚠️ 不要只处理 still-login：其余分支留着 stagedHere = true 会让每次 DOM 变动都再问一次
        //    主进程（重负载站点上是持续的 IPC），而且永远不会得到不同结果。
        stagedHere = false
      }
    } catch {
      // 主进程暂不可用（异常）：保留暂存，等下一次页面事件再试。
    } finally {
      committing = false
    }
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
        .invoke('w2a:pwd-stage', { username, password: f.pass.value })
        .then((res: unknown) => {
          const r = res as { ok?: unknown } | null
          if (r && r.ok === true) {
            stagedHere = true
            showBubble('登录成功后自动保存账号密码')
            // ⚠️ 竞态：SPA 常在 submit 的**同步阶段**就把登录表单隐藏 / 移除，那时暂存还没
            //    回到本进程（stagedHere 仍为 false），观察到的那次变动不会调度确认。这里补一次。
            if (findLogin() === null) void maybeCommit()
          } else console.warn('[web2app] 账号密码未暂存（输入不合法）')
        })
        .catch((e: unknown) => {
          console.warn('[web2app] 暂存账号密码失败：', e)
        })
    },
    true
  )

  const kick = (): void => {
    // 新文档（跨页导航）：等页面稳定后再问一次，让主进程判断暂存凭据是否已登录成功。
    // ⚠️ 触发时**必须重新判 stagedHere**：本页自己刚提交过（还在等站点响应、表单尚未消失）时
    //    这次探针会带着 `hasPasswordField: true` 过去，主进程按「登录失败」丢弃暂存 ⇒ 这一次
    //    登录永远存不进去。跨文档那条路只对「本页没有自己的暂存」有意义。
    if (!commitProbed && document.readyState === 'complete') {
      commitProbed = true
      setTimeout(() => {
        if (!stagedHere) void maybeCommit()
      }, 1200)
    }
    // 本页提交后登录表单消失（SPA 登录成功）：立即提交暂存凭据。
    if (stagedHere && findLogin() === null) void maybeCommit()
    void tryFill()
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', kick)
  else kick()
  window.addEventListener('load', kick)
  // 登录框常在首屏后才渲染（SPA）：聚焦输入框时补一次。
  document.addEventListener(
    'focusin',
    (ev) => {
      if (isInput(ev.target as Element | null)) kick()
    },
    true
  )
  // URL 变化：pushState / replaceState 不派发任何事件，Navigation API 的 `navigate` 也不会
  // 为它们触发（2026-11-18 探针实测）。因此在主世界打一个极小的 history 钩子，变化时用 DOM
  // 事件通知隔离世界；配合 popstate / hashchange / pageshow / DOM 变动，全程**事件驱动，
  // 没有任何常驻轮询定时器**。
  const URL_HOOK = `(() => {
    if (Object.prototype.hasOwnProperty.call(window, '__web2appUrlHook')) return;
    Object.defineProperty(window, '__web2appUrlHook', { value: 1 });
    const notify = () => document.dispatchEvent(new CustomEvent('__web2app:url-change'));
    for (const k of ['pushState', 'replaceState']) {
      const orig = history[k];
      history[k] = function (...args) { const r = orig.apply(this, args); notify(); return r; };
    }
  })()`
  document.addEventListener('__web2app:url-change', kick)
  window.addEventListener('popstate', kick)
  window.addEventListener('hashchange', kick)
  // bfcache 恢复时不会重跑 preload，补一次。
  window.addEventListener('pageshow', kick)
  void webFrame.executeJavaScript(URL_HOOK).catch(() => {
    console.warn('[web2app] URL 变化钩子注入失败（自动填充将只依赖 DOM 变动 / 聚焦）')
  })
  // 表单可能在任意时刻被插入 / 重建，也可能先 `display:none` 再切可见：
  // ⚠️ 必须观察 **document**，不能观察 `document.documentElement` —— preload 执行时文档树
  //    还没建好，`documentElement` 可能为 null，`observer.observe(null, …)` 会直接抛错，
  //    导致观察器整个失效，只剩 focusin 能触发（2026-11-18 真机复现并加了 18g 探针）。
  //    同时要观察属性变化（class / style / hidden）：显隐切换不产生 childList。
  let mutationTimer: ReturnType<typeof setTimeout> | undefined
  const observer = new MutationObserver(() => {
    syncUrl()
    if (fillTried) {
      // 填过的密码框被整块替换（SPA 重新渲染登录表单）：对新表单再填一次；
      // 同一元素只是被清空 / 隐藏则不重填，避免和站点互相打架。
      if (filledPass && !filledPass.isConnected) {
        fillTried = false
        filledPass = null
      } else if (!stagedHere) {
        // ⚠️ 这里**只能在不等待落盘时提前返回**。首版是无条件提前返回（filledPass 为空、
        //    或填过的密码框仍连着 DOM，都直接 return）—— 于是 SPA 登录成功后「表单只是被隐藏
        //    （密码框还在 DOM 里）」和「本页没存过凭据（filledPass 为 null）」两种情况下回调
        //    都直接返回，kick 永远不被调度 ⇒ `maybeCommit`（确认成功才落盘）等不到触发点，
        //    结果只有跨文档跳转那条路能存进密码。2026-11-18 复审修掉。
        return
      }
    }
    if (mutationTimer !== undefined) clearTimeout(mutationTimer)
    mutationTimer = setTimeout(kick, 300)
  })
  const observeOpts: MutationObserverInit = {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ['class', 'style', 'hidden'],
  }
  try {
    observer.observe(document, observeOpts)
  } catch {
    document.addEventListener('DOMContentLoaded', () => {
      try {
        observer.observe(document, observeOpts)
      } catch {
        console.warn('[web2app] DOM 观察器挂载失败，自动填充将只依赖 URL 变化 / 聚焦')
      }
    })
  }
  // 有限次补试（**不是轮询**）：覆盖 CSS / 布局 / 可见性晚于 DOMContentLoaded 的站点。
  // 每文档最多 4 个一次性定时器、3s 内结束，之后不再有任何定时器。
  for (const delay of [300, 800, 1600, 3000]) {
    setTimeout(() => {
      if (!fillTried) kick()
    }, delay)
  }
  // 窗口从后台回到前台（内容在隐藏期间才渲染完）时补一次。
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') kick()
  })
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
