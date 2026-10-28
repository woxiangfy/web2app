/**
 * 多页签「新建页签」回调（`TabsOptions['onSpawn']`，自 index.ts 的闭包平移）。
 *
 * 职责：内容视图建好后挂钩子（window.open 拦截 / 标题回传 / 加载失败 / devtools /
 * 右键菜单）并发起导航。⚠️ 新页签**不得**调 bootstrap()（其开头 stopActiveSidecar 会掐掉
 * 在服页签的代理，末尾会把主窗口导航走污染壳页面）—— 拿 target 走
 * `resolveInitialTarget`，只解析、不导航壳页面；导航目标是本内容视图。
 */
import type { ResolvedTarget } from './navigate'
import { attachContextMenu, attachDevtools } from './page-behavior'
import type { TabManager, TabsOptions } from './tabs'

export interface TabSpawnDeps {
  /** 目标解析（复用 target 缓存）；url 有值时（window.open 场景）根本不走它 */
  resolveInitialTarget: () => Promise<ResolvedTarget>
  /** 页签管理器（onSpawn 执行时早已建好）；getter 避免与 TabManager 循环引用 */
  getTabManager: () => TabManager | null
  /** F12 开发者工具开关（`devtoolsEnabled(app.isPackaged, cfg.debug)` 预计算） */
  devtools: boolean
  /** 右键菜单「设置」入口 */
  openSettings: () => void
}

export function makeTabSpawn(deps: TabSpawnDeps): TabsOptions['onSpawn'] {
  const { resolveInitialTarget, getTabManager, devtools, openSettings } = deps
  return async (view, _label, url) => {
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
    //   两边共享的只有 about: 拒绝这一条（1 行表达式），抽 helper 反而要参数化行为分叉。
    view.webContents.setWindowOpenHandler(({ url: opened }) => {
      console.warn(`[web2app] 页签模式拦截到新窗口请求：${opened}`)
      if (!opened || opened === 'about:blank' || opened.startsWith('about:')) {
        return { action: 'deny' }
      }
      setImmediate(() => {
        void getTabManager()
          ?.spawnTab(opened)
          .catch((e) => console.error(`[web2app] 新建页签失败：${(e as Error).message}`))
      })
      return { action: 'deny' }
    })
    // 标题回传：直接监听事件，无需注入脚本
    view.webContents.on('page-title-updated', (_e, title) => {
      getTabManager()?.setTabTitle(_label, title)
    })
    // 加载失败：内容视图没有加载层（不注入 loading.js），失败态只能体现在页签标题上。
    // ⚠️ 与单窗口的 did-fail-load 对齐「不静默」原则：记日志 + 给用户可见提示。
    //   -3 = ERR_ABORTED：正常重定向 / 被后续导航顶掉，不是错误。
    view.webContents.on('did-fail-load', (_e, code, desc, url2, isMainFrame) => {
      if (!isMainFrame || code === -3) return
      const detail = `无法打开 ${url2}（${desc}，${code}）`
      console.error(`[web2app] 页签加载失败：${detail}`)
      getTabManager()?.setTabTitle(_label, '加载失败')
    })
    // M4 页面行为：F12 / 右键菜单（与单窗口模式同一套实现，逐视图挂）
    attachDevtools(view.webContents, devtools)
    attachContextMenu(view.webContents, openSettings)
    await view.webContents.loadURL(t.url).catch((e) => {
      console.warn(`[web2app] 内容页 loadURL 被拒绝：${(e as Error).message}`)
    })
  }
}
