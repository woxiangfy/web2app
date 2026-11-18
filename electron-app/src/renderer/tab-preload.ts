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
import { contextBridge, ipcRenderer } from 'electron'

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
