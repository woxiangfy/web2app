/**
 * 三个本地自定义协议的处理器（splash:// / settings:// / shell://）—— 从 index.ts 抽出。
 *
 * scheme 的**特权声明**仍留在 index.ts 顶部（`protocol.registerSchemesAsPrivileged`
 * 必须早于 app ready，是严格启动顺序第 3 步）；本文件只负责 ready 之后的 `protocol.handle`
 * 注册，故调用方必须在 `app.whenReady()` 之后调用 `registerProtocols()`。
 *
 * 本模块不持有可变状态：渲染 / 读取 / 重试编排全部由 deps 注入，与 index.ts 的
 * 启动期闭包状态（target / navStarted / …）解耦。
 */
import { protocol } from 'electron'

import { FALLBACK_HTML } from './settings-window'
import type { PendingFail } from './splash-page'

export interface ProtocolDeps {
  /** 渲染最终下发给 splash:// 的启动页 HTML（失败态烘焙在 splash-page 内部）。 */
  renderSplash: () => string
  /** 设置 / 清除下一帧启动页的失败态。 */
  setSplashPendingFail: (fail: PendingFail | null) => void
  /**
   * 用户点启动页「重试」。调用方在这里重置导航代际（navStarted/target）并重新 bootstrap；
   * 本模块已用 `setImmediate` 保证「先把 /retry 响应交出去，再触发导航」，deps 内无需再延迟。
   */
  onSplashRetry: () => void
  /** 读取设置页 HTML（单一真源 webview-app/dist/settings.html），缺失给 null。 */
  readSettingsHtml: () => string | null
  /** 读取注入资产（shell.html 等），缺失给 null。 */
  readInject: (name: string) => string | null
}

const HTML_HEADERS = { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' }

export function registerProtocols(deps: ProtocolDeps): void {
  protocol.handle('splash', (req) => {
    const path = new URL(req.url).pathname.replace(/\/+$/, '')
    if (path === '/retry') {
      console.log('[web2app] 启动页请求重试，重新走一遍启动流程')
      deps.setSplashPendingFail(null)
      // 先把手上的响应交出去，再触发导航，避免同一次请求里嵌套导航
      setImmediate(() => {
        deps.onSplashRetry()
      })
    }
    return new Response(deps.renderSplash(), { headers: HTML_HEADERS })
  })

  // settings:// → 设置页 HTML（单一真源：webview-app/dist/settings.html）
  protocol.handle('settings', () => {
    const html = deps.readSettingsHtml()
    if (!html) {
      console.warn('[web2app] 未找到 settings.html，设置窗使用内置精简页')
    }
    return new Response(html ?? FALLBACK_HTML, { headers: HTML_HEADERS })
  })

  // shell:// → 多页签壳页面（单一真源：webview-app/src/shell.html，零改动）
  protocol.handle('shell', () => {
    const html = deps.readInject('shell.html')
    if (!html) {
      console.warn('[web2app] 未找到 shell.html，壳页面缺失（多页签模式将白屏）')
    }
    return new Response(
      html ??
        '<!doctype html><meta charset="utf-8"><body style="background:#f7f8fa">标签栏资源缺失</body>',
      { headers: HTML_HEADERS }
    )
  })
}
