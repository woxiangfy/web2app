/**
 * 页签内容页 preload 的装配契约（2026-11-16）。
 *
 * 背景：`tabs_enabled: true` 的产物里内容页是独立 `WebContentsView`，此前**没挂任何 preload**
 * ⇒ `window.web2appExtra` 恒为 undefined（单窗口模式正常），而配置文件里 extra 非空、文档也
 * 承诺前端可读。修法见 `src/renderer/tab-preload.ts`。
 *
 * 本测试锁的是「**装配**」而不是「实现」：单测拿不到真实 webContents，所以只能钉住
 *   ① 该 preload 只暴露 extra、不含窗口桥（安全基线）；
 *   ② tabs.ts 真的把它传给了 WebContentsView；
 *   ③ index.ts 真的把路径装配进 TabManager；
 *   ④ 主进程真的有对应通道。
 * 运行期真伪由 `probe --tabs` 的 P2b 两条断言负责（本文件不能替代它）。
 *
 * ⚠️ 断言前一律剥注释（项目教训）：这些源码里大量注释**故意提到**被禁字面量
 * （例如「不能挂 preload.js，否则两条标题栏」），不剥就会把注释满足当成代码满足。
 */
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from 'node:test'

const root = process.cwd()
const read = (p: string): string => readFileSync(join(root, p), 'utf8')
const stripComments = (src: string): string =>
  src.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '')

const tabPreload = stripComments(read('src/renderer/tab-preload.ts'))
const tabsTs = stripComments(read('src/main/tabs.ts'))
const indexTs = stripComments(read('src/main/index.ts'))
const appIpc = stripComments(read('src/main/app-ipc.ts'))

test('tab-preload 只注入 web2appExtra，不暴露任何窗口桥', () => {
  assert.ok(
    tabPreload.includes("exposeInMainWorld('web2appExtra'"),
    'tab-preload 必须暴露 window.web2appExtra'
  )
  for (const bridge of ['web2appShell', "exposeInMainWorld('web2app'", 'minimize', 'openSettings']) {
    assert.ok(
      !tabPreload.includes(bridge),
      `tab-preload 不得暴露窗口桥（命中 ${bridge}）：内容页在页签模式下没有 API，这是安全基线`
    )
  }
})

test('tab-preload：空 extra 不暴露（与单窗口 preload 同一判据）', () => {
  assert.ok(
    tabPreload.includes('Object.keys(obj).length > 0'),
    '空对象必须不定义该全局，避免页面误判「配置里填了 extra」'
  )
})

test('tab-preload：sandbox 约束 —— 不得相对 require（AGENTS.md #17）', () => {
  assert.ok(!/\brequire\s*\(/.test(tabPreload), 'sandbox preload 里相对 require 会在运行期失败')
  assert.ok(
    tabPreload.includes("from 'electron'"),
    '只允许 import electron'
  )
})

test('tabs.ts：preload 真的传给了内容视图的 webPreferences', () => {
  assert.ok(
    tabsTs.includes('preload: this.tabPreload'),
    'WebContentsView.webPreferences 必须带 preload，否则页签模式读不到 extra'
  )
})

test('index.ts：TabManager 装配了 tab-preload 路径', () => {
  assert.ok(
    indexTs.includes("tabPreload: preloadPath('tab-preload.js')"),
    'index.ts 必须装配 tabPreload（漏装配 ⇒ 文件存在但永远不生效）'
  )
})

test('app-ipc.ts：主进程注册了 w2a:tab-extra 同步通道', () => {
  assert.ok(appIpc.includes("'w2a:tab-extra'"), '缺少通道 ⇒ preload 的 sendSync 抛错、静默拿不到 extra')
  assert.ok(
    appIpc.includes('registerTabExtraIpc(deps)'),
    '注册函数必须被 registerAppIpc 真正调用'
  )
})
