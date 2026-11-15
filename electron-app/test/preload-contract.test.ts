/**
 * preload 三件套的结构契约（2026-11-07 架构审查 M-9 / L-4）。
 *
 * ⚠️ 为什么这里只做文本对照、不抽 `tauri-shim.ts` 共享模块：三个 preload 都跑在
 * `sandbox: true` 的渲染进程里，那里 `require` 只放行 electron（Electron 45 起 events / timers /
 * url shim 已移除），**相对 require 会在运行期失败**。所以两个 invoke 垫片只能各自内联，靠本测试锁住
 * 「形状一致」，防的是改了 A 忘了 B 的漂移。
 */
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from 'node:test'

const root = process.cwd()
const read = (p: string): string => readFileSync(join(root, p), 'utf8')

const mainPreload = read('src/renderer/preload.ts')
const shellPreload = read('src/renderer/shell-preload.ts')
const settingsPreload = read('src/renderer/settings-preload.ts')

/**
 * 剥掉注释再断言：这些文件里大量注释**故意提到**被禁字面量（例如「不得照搬
 * `transformCallback`」），不剥就会把注释满足当成「代码里真的有」（2026-11-07
 * 架构审查批次⑥的教训）。
 */
const stripComments = (src: string): string =>
  src.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '')

/** 抽出 `contextBridge.exposeInMainWorld('__TAURI_INTERNALS__', { … })` 的垫片主体并按前缀归一。 */
function shimBody(src: string, prefix: string): string {
  const start = src.indexOf("contextBridge.exposeInMainWorld('__TAURI_INTERNALS__', {")
  assert.ok(start > 0, `${prefix} 垫片缺失`)
  const body = src.slice(start, src.indexOf('})', start) + 2)
  return body.replace(new RegExp(`invoke\\('${prefix}:' \\+ cmd`, 'g'), "invoke('<prefix>:' + cmd")
}

test('两个 invoke 垫片形状逐字一致（只允许前缀不同）', () => {
  assert.equal(
    shimBody(shellPreload, 'tabs'),
    shimBody(settingsPreload, 'tauri'),
    'shell-preload 与 settings-preload 的垫片形状已漂移：改一侧必须同步另一侧'
  )
})

test('垫片只暴露 invoke，且必须拒绝白名单外的命令', () => {
  for (const [name, src] of [
    ['shell-preload', shellPreload],
    ['settings-preload', settingsPreload],
  ] as const) {
    const code = stripComments(src)
    assert.ok(!code.includes('transformCallback'), `${name} 不得照搬 __TAURI_INTERNALS__ 全部表面`)
    assert.ok(code.includes('!ALLOWED.has(cmd)'), `${name} 必须做白名单校验`)
    assert.ok(code.includes('拒绝调用未授权的命令'), `${name} 拒绝时必须给出可读错误`)
  }
})

test('L-4：未使用的 web2appShell 已从两个 preload 移除', () => {
  for (const [name, src] of [
    ['shell-preload', shellPreload],
    ['settings-preload', settingsPreload],
  ] as const) {
    assert.ok(!src.includes('web2appShell'), `${name} 不应再暴露无消费方的 web2appShell`)
  }
})

test('主 preload：内部页面 scheme 白名单覆盖六个内部协议', () => {
  const start = mainPreload.indexOf('function isInternalPage')
  assert.ok(start > 0, 'preload.ts 里找不到 isInternalPage')
  const block = mainPreload.slice(start, mainPreload.indexOf('} catch', start))
  for (const p of ['splash:', 'about:', 'chrome:', 'settings:', 'shell:', 'floaticon:']) {
    assert.ok(block.includes(`'${p}'`), `isInternalPage 必须包含 ${p}（否则会往内部页注入标题栏）`)
  }
})

test('主 preload：主体只分块不分文件（沙箱 preload 不能相对 require）', () => {
  // 正面钉住三块：桥 / overlay 通道 / 注入
  for (const fn of ['installBridge', 'installOverlayChannel', 'installInjectScripts']) {
    assert.ok(mainPreload.includes(`function ${fn}(`), `缺少分块函数 ${fn}`)
  }
  // 反面：出现相对 require / import 即会在 sandbox 渲染进程里运行期失败
  assert.ok(!/require\(['"]\.\.?\//.test(mainPreload), 'preload 不得相对 require（沙箱不支持）')
  assert.ok(!/^import .* from '\.\.?\//m.test(shellPreload + settingsPreload), '垫片 preload 不得引入本地模块')
})
