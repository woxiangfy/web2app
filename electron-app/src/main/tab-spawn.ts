/**
 * 多页签「新建页签」回调（`TabsOptions['onSpawn']`，自 index.ts 的闭包平移）。
 *
 * 职责：内容视图建好后挂钩子（window.open 拦截 / 标题回传 / 加载失败 / devtools /
 * 右键菜单）并发起导航。⚠️ 新页签**不得**调 bootstrap()（其开头 stopActiveSidecar 会掐掉
 * 在服页签的代理，末尾会把主窗口导航走污染壳页面）—— 拿 target 走
 * `resolveInitialTarget`，只解析、不导航壳页面；导航目标是本内容视图。
 */
import type { ResolvedTarget } from './navigate'
import { isInternalUrl, navFailText } from './nav-text'
import { attachCloseTabShortcut, attachContextMenu, attachDevtools } from './page-behavior'
import type { TabManager, TabsOptions } from './tabs'

/**
 * 隐藏弹窗窗口的兜底存活上限（毫秒）。
 *
 * 正常路径由「下载结束（`done`）」或「它自己提交了文档」收尾；这里是兜住「站点开了窗却
 * 什么也没做」的泄漏 —— 既没下载也没页面。
 */
const HIDDEN_POPUP_MAX_MS = 20000

export interface TabSpawnDeps {
  /** 目标解析（复用 target 缓存）；url 有值时（window.open 场景）根本不走它 */
  resolveInitialTarget: () => Promise<ResolvedTarget>
  /** 页签管理器（onSpawn 执行时早已建好）；getter 避免与 TabManager 循环引用 */
  getTabManager: () => TabManager | null
  /** F12 开发者工具开关（`devtoolsEnabled(app.isPackaged, cfg.debug)` 预计算） */
  devtools: boolean
  /** 诊断可见性（`DEBUG` 预计算）：决定加载失败提示里是否带 URL / net error code */
  debug: boolean
  /** 右键菜单「设置」入口 */
  openSettings: () => void
}

export function makeTabSpawn(deps: TabSpawnDeps): TabsOptions['onSpawn'] {
  const { resolveInitialTarget, getTabManager, devtools, debug, openSettings } = deps
  return async (view, label, url) => {
    // ⚠️⚠️ 新页签**不得**调 bootstrap()（其开头 stopActiveSidecar 会掐掉在服页签的代理，
    //   末尾会把主窗口导航走污染壳页面）。这里用 resolveInitialTarget 复用 target 缓存，
    //   只解析、不导航壳页面；导航目标是本内容视图。
    //   url 有值时（window.open 场景）直接用；否则用默认 target（首屏）。
    const t = url ? { url, user: '', pass: '', how: 'direct' as const } : await resolveInitialTarget()
    // 挂窗口打开处理器：内容页 window.open → 新建页签。
    // ⚠️ 与单窗口版（index.ts 的 registerWindowOpenHandler）看着像，实则两处刻意不同、勿强行合并：
    //   1) 动作是「新建页签」（走 spawnTab → resolveTarget 的代理/DNS 完整链路），不是原地导航；
    //   2) 不做 new URL 预校验 —— spawnTab 内部解析目标时自会失败（失败进页签标题 + 遮罩），
    //      单窗口版那道校验是为了区分「原地导航失败交给 did-fail-load」才需要的。
    //   两边共享的只有内部页面拒绝这一条，现在共用 nav-text.ts::isInternalUrl。
    view.webContents.setWindowOpenHandler(({ url: opened }) => {
      console.warn(`[web2app] 页签模式拦截到新窗口请求：${opened}`)
      // 内部页面（about: / chrome: / 启动页）没有可独立显示的实体页 → 拒绝。
      // 与单窗口版共用 nav-text.ts::isInternalUrl 一条判据（M-4）。
      if (isInternalUrl(opened)) {
        return { action: 'deny' }
      }
      // 纵深防御：即便某些站点在拿到非 null 窗口后仍做同地址兜底，也按「URL + 打开者」
      // 认出来丢掉（见 tabs.ts::shouldDropPopupFallback；主判据是上面的非 null 窗口）。
      getTabManager()?.noteExpectedPopupDownload(label, opened)
      // ⚠️⚠️ **不能 `deny`**：Electron 在 deny 时让页面的 `window.open()` 拿到 **null**，而
      // 站点普遍据此判定「弹窗被拦截」，立刻对同一地址（可能还带新签名）再走一次
      // `location.href` 兜底 —— 一次点击两次下载、两个「另存为」，用户点掉第一个后第二个还在。
      // 「按 URL 判重」天然不可靠（兜底地址可能与目标不同字），所以从根上解决：
      //   允许 Electron 建一个**隐藏窗口**（show:false + skipTaskbar）⇒ 页面拿到真实
      //   WindowProxy ⇒ `if (!w)` 这条判据失效；该窗口的存活期必须覆盖整次下载
      //   （否则 `w.closed` 变 true，站点另一条判据照样触发兜底），见下面的 did-create-window。
      return {
        action: 'allow',
        outlivesOpener: false,
        overrideBrowserWindowOptions: {
          show: false,
          skipTaskbar: true,
          webPreferences: { nodeIntegration: false, contextIsolation: true, sandbox: true },
        },
      }
    })
    // 隐藏弹窗的收尾策略（⚠️ 这是「一次点击两个另存为」的根因所在，别改回「立刻销毁」）：
    //   · 站点判断「弹窗被拦截」的写法是 `if (!w || w.closed) location.href = url`，有的还会
    //     延迟几十~几百毫秒再查。**立刻销毁** ⇒ `w.closed === true` ⇒ 站点照样兜底 ⇒ 第二次
    //     下载、第二个「另存为」（2026-11-15 两次失败的修法都栽在这上面）。
    //   · 所以隐藏窗口要活到**这次下载结算**为止（页面里 `w.closed` 始终 false），这也贴合
    //     浏览器语义：那个 popup 本来就活到下载开始/结束才关。
    //   · 三条收尾路：绑定的新页签关闭（下载结算后自动关）、它自己提交了文档（普通页面弹窗）、
    //     20s 兜底超时（站点开了窗却什么也没做）。
    view.webContents.on('did-create-window', (child, details) => {
      const mgr = getTabManager()
      const childWc = child.webContents
      const childId = childWc.id
      const target = details.url
      let closed = false
      const cleanup = (): void => {
        if (closed) return
        closed = true
        // 延后一拍：避免在「它自己的 webContents 事件回调」里销毁自己（Electron 未定义安全边界）
        setImmediate(() => {
          try {
            if (!child.isDestroyed()) child.destroy()
          } catch (e) {
            console.error(`[web2app] 销毁隐藏弹窗失败：${(e as Error).message}`)
          }
        })
      }
      mgr?.registerHiddenPopup(childId, cleanup, HIDDEN_POPUP_MAX_MS)
      // 排障用：隐藏窗口建出来的那一刻记下 wc / 目标 / disposition（站点究竟是
      // `window.open` 还是 `_blank` 表单，以及它想打开哪个地址，只有这里看得到）
      console.warn(
        `[web2app] 隐藏弹窗创建：wc=${childId} disposition=${details.disposition} url=${target}`
      )
      // ⚠️⚠️ 这个窗口的唯一用途是「让页面里的 window.open() 拿到非 null」，它自己**不许导航、
      //   不许下载、不许冒头**（2026-11-15 真机截图：它以下载地址为标题出现在主窗口里，并在
      //   自己身上弹了第二个「另存为」——那条下载虽然被 shouldDropHiddenPopupDownload 丢弃，
      //   但窗口本身已经是可见的第二现场）：
      //   · stop() + about:blank：掐掉 Electron 已经替它发起的那次附件导航 ⇒ 它永远不产生下载；
      //   · will-navigate 一律 preventDefault：站点若 `w.location = 下载地址`，同样拦下，
      //     真正的导航与下载只走上面那个新页签（一次点击、一个「另存为」）；
      //   · hide()：兜住「继承来的 window features 里带 show」的情况。
      try {
        childWc.stop()
      } catch {
        /* 尚未开始加载：忽略 */
      }
      childWc.on('will-navigate', (e) => e.preventDefault())
      childWc.setWindowOpenHandler(() => ({ action: 'deny' }))
      child.hide()
      void childWc.loadURL('about:blank').catch(() => {
        /* 窗口可能已在下一拍被收尾销毁：忽略 */
      })
      // 普通页面弹窗：文档提交即说明它不是下载 ⇒ 隐藏窗口的使命结束（页面内容已由上面那个
      // 新页签承载，这里**不能**再开第二个页签）。
      // ⚠️ 附件下载**不提交文档** ⇒ 不走这里；且 `about:blank` 不算「页面」，忽略。
      childWc.on('did-navigate', (_e, url) => {
        if (!url || url === 'about:blank') return
        mgr?.releaseHiddenPopupOnNavigate(childId)
      })
      if (!target) {
        console.error('[web2app] 新窗口请求没有可用 URL，已忽略')
        mgr?.releaseHiddenPopup(childId)
        return
      }
      // 承载这次导航与下载的是新页签（用户报障里那个「应自动关闭的页签」）。
      // 隐藏窗口绑到它身上：页签关闭（下载结算后自动关）时才销毁隐藏窗口。
      void mgr
        ?.spawnTab(target, label)
        .then((tabLabel) => {
          if (!mgr) return
          if (tabLabel) mgr.attachHiddenPopupToTab(childId, tabLabel)
          else mgr.releaseHiddenPopup(childId)
        })
        .catch((e) => {
          console.error(`[web2app] 新建页签失败：${(e as Error).message}`)
          mgr?.releaseHiddenPopup(childId)
        })
    })
    // 标题回传：直接监听事件，无需注入脚本
    view.webContents.on('page-title-updated', (_e, title) => {
      getTabManager()?.setTabTitle(label, title)
    })
    // 文档提交标记：下载响应**不会**触发 did-navigate ⇒ 用它区分「页面」与「只为下载而开的空页签」
    // （前端不展示下载页，下载开始后要自动关掉，见 tabs.ts::closeTabAfterDownload）。
    view.webContents.on('did-navigate', () => {
      getTabManager()?.markTabCommitted(label)
    })
    // 加载失败：内容视图没有加载层（不注入 loading.js），失败态只能体现在页签标题上。
    // ⚠️ 与单窗口的 did-fail-load 对齐「不静默」原则：记日志 + 给用户可见提示。
    //   -3 = ERR_ABORTED：正常重定向 / 被后续导航顶掉，不是错误。
    view.webContents.on('did-fail-load', (_e, code, desc, url2, isMainFrame) => {
      if (!isMainFrame || code === -3) return
      const detail = navFailText(url2, desc, code, debug)
      console.error(`[web2app] 页签加载失败：${detail}`)
      getTabManager()?.setTabTitle(label, '加载失败')
    })
    // M4 页面行为：F12 / 右键菜单（与单窗口模式同一套实现，逐视图挂）
    attachDevtools(view.webContents, devtools)
    attachContextMenu(view.webContents, openSettings)
    // M5 快捷键：Ctrl+W 关闭当前页签（焦点在网页里时走这条；焦点在壳上时走 index.ts 那条）。
    // ⚠️ 关闭动作必须 `setImmediate`：本回调运行在该视图自己的 before-input-event 派发栈里，
    //    就地 close 属于「在自己的回调里销毁自己」（Electron 未定义安全边界），推到下一轮再关。
    attachCloseTabShortcut(view.webContents, () => {
      setImmediate(() => {
        const mgr = getTabManager()
        const active = mgr?.active
        // 首页那一签不可关 ⇒ 交给 closeTab 的 isPinnedHome 拦截（**不在这里重复判断**：
        // 「首页判定」全项目只有 tab-order.ts::isPinnedHome 一份实现）。
        if (mgr && active) mgr.closeTab(active.label)
      })
    })
    await view.webContents.loadURL(t.url).catch((e) => {
      console.warn(`[web2app] 内容页 loadURL 被拒绝：${(e as Error).message}`)
    })
  }
}
