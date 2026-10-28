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
  /**
   * 设置页资源缺失时的内置精简页（资源本体留在 settings-window.ts 的 FALLBACK_HTML，
   * 经 deps 注入 —— 2026-10-24 批次⑤ P2-8：protocols 是底层协议分发，不得反向依赖
   * 设置窗实现）。
   */
  fallbackHtml: string
  /** 读取注入资产（shell.html / floaticon.html 等），缺失给 null。 */
  readInject: (name: string) => string | null
  /**
   * 读取悬浮球内的**软件图标** PNG 字节（与 Tauri floaticon 协议的 /icon.png 对齐）。
   * 打包态来自 out/inject/icon.png（stage 同源拷贝），开发态直接读 webview-app/icons/。
   */
  readFloatIcon: () => Uint8Array | null
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
    return new Response(html ?? deps.fallbackHtml, { headers: HTML_HEADERS })
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

  // floaticon:// → 悬浮球本地页与软件图标（与 Tauri 侧 handle_floaticon 逐条对齐）：
  //   /          → floaticon.html（双壳单一真源）
  //   /icon.png  → 软件图标（打包配置 icon_url 替换后的那一份）
  //   其它路径   → 404（本协议只暴露这两个内置资源）
  protocol.handle('floaticon', (req) => {
    const path = new URL(req.url).pathname
    if (path === '/icon.png') {
      const png = deps.readFloatIcon()
      if (!png) {
        console.warn('[web2app] 未找到 icon.png，悬浮球图标缺失')
        return new Response(null, { status: 404 })
      }
      // Blob 携带 image/png 类型。TS 5.7 的 Uint8Array 是 Uint8Array<ArrayBufferLike>，
      // 不能直接做 BlobPart（SharedArrayBuffer 不在联合内），这里拷到独立 ArrayBuffer。
      const ab = new ArrayBuffer(png.length)
      new Uint8Array(ab).set(png)
      return new Response(new Blob([ab], { type: 'image/png' }), {
        headers: { 'cache-control': 'no-store' },
      })
    }
    if (path === '/' || path === '') {
      return new Response(deps.readInject('floaticon.html') ?? '', { headers: HTML_HEADERS })
    }
    return new Response(null, { status: 404 })
  })
}
