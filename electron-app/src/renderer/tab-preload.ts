/**
 * 多页签模式下**内容页**专用的最小 preload：只注入 `window.web2appExtra`。
 *
 * 为什么需要它（2026-11-16 实测）：`tabs_enabled: true` 的产物里，内容页是 `WebContentsView`
 * （tabs.ts::spawnTab），它**不挂** `preload.ts` —— 那套会往页面里塞自绘标题栏 / 加载层，
 * 而页签模式的标题栏属于壳页面 `shell.html`。结果 `window.web2appExtra` 在页签模式下
 * **恒为 undefined**（单窗口模式正常），与 `docs/packaging-config.md` 承诺的
 * 「全平台、前端读 `window.web2appExtra`」不符，也与 Tauri 页签
 * （`webview-app/src/tabs/window.rs` 的 `initialization_script`）不一致。
 *
 * 契约（与 `preload.ts` 的 extra 部分逐条对齐，改动必须两边一起看）：
 *   · 只暴露**数据**，不暴露任何窗口桥 —— 页签内容页确实没有 `window.web2app`，这是安全基线；
 *   · `extra` 为空对象时**不定义**该全局（避免页面误判「配置里填了 extra」）；
 *   · 同步取（`sendSync`）：preload 执行于「document 已创建、页面自身脚本未跑」，
 *     与 Tauri 的 document-created 注入同一切点，页面首屏即可读。
 *
 * ⚠️ 硬约束（electron-app/AGENTS.md #17）：**不得相对 require**。本文件只 `import 'electron'`。
 *    Electron 45 起 sandbox preload 的 `require` 只放行 `electron`。
 */
import { contextBridge, ipcRenderer, webFrame } from 'electron'

// ⚠️ 子框架：preload 默认不作用于子框架（需 nodeIntegrationInSubFrames=true），这里只做纵深防御，
//    与 `preload.ts` 顶部注释第 2 条同一道闸 —— 一旦将来打开该开关，跨源 iframe 也拿不到配置。
if (window.self === window.top) {
  let extra: unknown = null
  try {
    // 主进程侧见 app-ipc.ts::registerTabExtraIpc（只做顶层框架闸：本通道不涉及任何窗口能力）。
    extra = ipcRenderer.sendSync('w2a:tab-extra')
  } catch {
    // 主进程未注册 / 页签已销毁等：静默降级为空，绝不因为「配置读不到」把页面搞崩。
    extra = null
  }
  const obj = extra as Record<string, unknown> | null
  if (obj && typeof obj === 'object' && Object.keys(obj).length > 0) {
    contextBridge.exposeInMainWorld('web2appExtra', obj)
  }
  installPasswordManager()
}

// ---- password-manager:begin（本段在 preload.ts / tab-preload.ts 中逐字一致，由 test/password-preload-contract.test.ts 锁）----
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
      } else if (res && res.reason === 'still-login') {
        // 登录失败 / 多步登录：主进程已丢弃暂存，本页不再重试。
        stagedHere = false
      }
    } catch {
      // 主进程暂不可用：保留暂存，等下一次页面事件再试。
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
    if (!commitProbed && document.readyState === 'complete') {
      commitProbed = true
      setTimeout(() => void maybeCommit(), 1200)
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
      // 同一元素只是被清空则不重填，避免和站点互相打架。
      if (!filledPass || filledPass.isConnected) return
      fillTried = false
      filledPass = null
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
