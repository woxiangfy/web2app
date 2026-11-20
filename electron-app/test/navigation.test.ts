/**
 * `navigation.ts` 状态机单测（2026-11-07 架构审查 M-5）。
 *
 * 此前这条链路只能靠运行期探针兜底：`target` 缓存、导航代际、`splashAutoBootConsumed`
 * 「一生只自动一次」、失败后重载启动页、重试重置 —— 每一处错了都表现成
 * 「启动页一直转圈 / 无限重试 / 重复起 sidecar」。本模块零 Electron 运行期依赖
 * （`BrowserWindow` 仅作类型），用假窗口 + 假启动页即可覆盖。
 */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import type { AppConfig } from '../src/main/config.ts'
import { createNavigator } from '../src/main/navigation.ts'
import type { SplashPage } from '../src/main/splash-page.ts'

const cfg = (app_url: string | string[] = 'https://example.com/') =>
  ({
    app_url,
    app_name: 't',
    title: 't',
    version: '0.1.0',
    extra: {},
    icon_url: '',
    proxy_enabled: false,
    proxy_http: '',
    proxy_https: '',
    proxy_bypass: '',
    tabs_enabled: false,
    public_key: '',
  }) as unknown as AppConfig

type FakeWin = {
  isDestroyed: () => boolean
  loadURL: (u: string) => Promise<void>
  webContents: { send: (ch: string, m: unknown) => void }
}

function harness(
  appUrl: string | string[] = 'https://example.com/',
  opts?: { destroyed?: boolean; debug?: boolean },
) {
  const loaded: string[] = []
  const sent: { ch: string; m: unknown }[] = []
  const failed: { title: string; detail: string }[] = []
  const win: FakeWin = {
    isDestroyed: () => opts?.destroyed === true,
    loadURL: (u: string) => {
      loaded.push(u)
      return Promise.resolve()
    },
    webContents: { send: (ch, m) => sent.push({ ch, m }) },
  }
  const splashPage = {
    setPendingFail: (v: { title: string; detail: string }) => failed.push(v),
  } as unknown as SplashPage
  let current: FakeWin | null = win
  const nav = createNavigator({
    cfg: cfg(appUrl),
    debug: opts?.debug ?? false,
    splashPage,
    getMainWindow: () => current as never,
    appPath: '',
    resourcesPath: '',
  })
  return { nav, loaded, sent, failed, win, setWindow: (w: FakeWin | null) => (current = w) }
}

/** bootstrap 是 fire-and-forget 链，等它跑完：让出几轮宏任务即可（无 IO）。 */
const flush = () => new Promise<void>((r) => setTimeout(r, 0))

test('resolveAndCache：直连模式解析，缓存由调用方写入后命中（不重复解析）', async () => {
  const h = harness('https://user:pass@example.com/x')
  const a = await h.nav.resolveInitialTarget()
  assert.equal(a.url, 'https://example.com/x', 'userinfo 必须剥离')
  assert.equal(a.user, 'user')
  assert.equal(a.how, 'direct')
  const b = await h.nav.resolveAndCache()
  assert.equal(b.ok && b.target, a, 'resolveInitialTarget 写入缓存后，resolveAndCache 应命中同一份对象')
})

test('resolveAndCache：非法地址 → 失败文案，不抛异常（debug 包保留原因）', async () => {
  const h = harness('ftp://example.com/', { debug: true })
  const r = await h.nav.resolveAndCache()
  assert.equal(r.ok, false)
  if (r.ok) return
  assert.equal(r.title, '加载失败')
  assert.ok(r.detail.includes('http'))
})

test('resolveAndCache：debug=false 的失败文案不泄漏协议 / 地址', async () => {
  const h = harness('ftp://example.com/')
  const r = await h.nav.resolveAndCache()
  assert.equal(r.ok, false)
  if (r.ok) return
  assert.equal(r.title, '加载失败')
  assert.ok(!r.detail.includes('ftp://'), '默认包不得把配置地址摆到屏幕上')
  assert.ok(!r.detail.includes('http'), '默认包不得把内部协议判定细节摆到屏幕上')
})

test('resolveInitialTarget：失败时抛错（多页签首签的调用方靠它中止）', async () => {
  const h = harness('nope', { debug: true })
  await assert.rejects(() => h.nav.resolveInitialTarget(), /不合法/)
})

test('bootstrap：成功后缓存 target 并导航主窗口', async () => {
  const h = harness()
  await h.nav.bootstrap()
  assert.deepEqual(h.loaded, ['https://example.com/'])
  assert.equal(h.nav.getTarget()?.url, 'https://example.com/')
})

test('bootstrap：失败 → 记失败态 + 重载启动页（用户可点重试），不导航目标', async () => {
  const h = harness('ftp://example.com/')
  await h.nav.bootstrap()
  assert.equal(h.failed.length, 1)
  assert.deepEqual(h.loaded, ['splash://localhost/'])
  assert.equal(h.nav.getTarget(), null)
})

test('autoBootFromSplash：一生只自动发生一次（失败 → 重载 splash 不得再自动 bootstrap）', async () => {
  const h = harness('ftp://example.com/')
  h.nav.autoBootFromSplash()
  await flush()
  assert.deepEqual(h.loaded, ['splash://localhost/'], '首轮：失败后重载启动页')
  // 启动页每次 did-finish-load 都会再调一次这里；第二轮必须被名额闸门挡掉
  h.nav.autoBootFromSplash()
  await flush()
  assert.deepEqual(h.loaded, ['splash://localhost/'], '第二轮不得再触发解析 / 重载')
})

test('retryFromSplash：重置缓存与名额，重新 bootstrap', async () => {
  const h = harness()
  await h.nav.bootstrap()
  h.nav.retryFromSplash()
  await flush()
  assert.equal(h.loaded.length, 2, '重试必须再导航一次')
  assert.equal(h.nav.getTarget()?.url, 'https://example.com/')
})

test('bootstrap：窗口已销毁 → 静默返回，不做任何导航', async () => {
  const h = harness()
  h.setWindow(null)
  await h.nav.bootstrap()
  assert.deepEqual(h.loaded, [])
})

test('sendOverlay：无主窗时不抛；有主窗时下发 web2app:overlay', () => {
  const h = harness()
  h.setWindow(null)
  assert.doesNotThrow(() => h.nav.sendOverlay({ kind: 'hide' }))

  h.setWindow(h.win)
  h.nav.sendOverlay({ kind: 'fail', title: 't', detail: 'd' })
  assert.deepEqual(h.sent, [{ ch: 'web2app:overlay', m: { kind: 'fail', title: 't', detail: 'd' } }])
})
