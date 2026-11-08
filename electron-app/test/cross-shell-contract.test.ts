/**
 * 跨壳字面量 / 判据对照守卫（2026-11-07 架构审查 M-2）。
 *
 * 背景：同一个语义在 Tauri/共用件与 Electron 各写一份时，**只测单侧的 CI 永远是绿的**，
 * 漂移只能等用户在两壳看到不同行为才暴露。本文件同时读两侧源码做逐字对照：
 *
 *   1. 内置页 URL 判定（`titlebar.js::isBuiltinUrl` ↔ `page-behavior.ts::isBuiltinUrl`）
 *   2. 标签栏高度 / 页签上限（`shell.html` ↔ `tabs.rs` ↔ `tabs.ts`）
 *   3. 页签标题 `trim()` + 空标题不覆盖（`tab_title.js` ↔ `tabs.ts::setTabTitle`）
 *   4. 系统占用快捷键表（`settings.html::HK_RESERVED` ↔ `hotkey.ts::RESERVED_HOTKEYS`）
 *
 * 这些是「行为契约对齐、实现不同」的跨壳共用项（见根 AGENTS.md §3.4.1），两侧都改才该变红。
 */
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from 'node:test'

import { isBuiltinUrl } from '../src/main/page-behavior.ts'

const root = join(process.cwd(), '..')
const read = (p: string): string => readFileSync(join(root, p), 'utf8')

const titlebarJs = read('webview-app/src/titlebar.js')
const tabTitleJs = read('webview-app/src/tab_title.js')
const shellHtml = read('webview-app/src/shell.html')
const settingsHtml = read('webview-app/dist/settings.html')
const tabsRs = read('webview-app/src/tabs/state.rs')
const pageBehaviorTs = read('electron-app/src/main/page-behavior.ts')
const navTextTs = read('electron-app/src/main/nav-text.ts')
const tabsTs = read('electron-app/src/main/tabs.ts')
const hotkeyTs = read('electron-app/src/main/hotkey.ts')

/** 从一段 `KEY: "desc"` / `'KEY': 'desc'` 文本里抽映射（用于两壳表对照）。 */
function pairsOf(block: string): Record<string, string> {
  const out: Record<string, string> = {}
  for (const m of block.matchAll(/['"]([A-Za-z0-9+]+)['"]\s*:\s*['"]([^'"]+)['"]/g)) {
    out[m[1]] = m[2]
  }
  return out
}

test('内置页判定：两种 splash URL 形态都算内置页（与 titlebar.js 同判据）', () => {
  assert.equal(isBuiltinUrl('splash://localhost/'), true)
  assert.equal(isBuiltinUrl('http://splash.localhost/'), true)
  assert.equal(isBuiltinUrl('https://splash.localhost/#x'), true)
  assert.equal(isBuiltinUrl('https://splash.localhost.evil.com/'), false)
  assert.equal(isBuiltinUrl('https://example.com/'), false)
  assert.equal(isBuiltinUrl(''), false)
})

test('内置页判定的正则两侧逐字一致', () => {
  for (const lit of ['/^splash:/i', '/^https?:\\/\\/splash\\.localhost(?:[/?#]|$)/i']) {
    assert.ok(titlebarJs.includes(lit.replace(/^\/|\/i$/g, '')), `titlebar.js 缺少正则 ${lit}`)
    // 实现已迁到 nav-text.ts（M-4）；page-behavior.ts 只保留旧名再导出。
    assert.ok(navTextTs.includes(lit), `nav-text.ts 缺少正则 ${lit}`)
    assert.ok(
      pageBehaviorTs.includes("export { isSplashUrl as isBuiltinUrl, isInternalUrl } from './nav-text'"),
      'page-behavior.ts 必须从 nav-text.ts 再导出，不得另写一份判定'
    )
  }
})

test('标签栏高度与页签上限三处一致', () => {
  assert.ok(tabsTs.includes('TAB_BAR_HEIGHT = 38'))
  assert.ok(shellHtml.includes('--bar-h: 38px'))
  assert.ok(tabsRs.includes('TAB_BAR_HEIGHT: i32 = 38'))
  assert.ok(tabsTs.includes('MAX_TABS = 32'))
  assert.ok(tabsRs.includes('MAX_TABS: usize = 32'))
})

test('页签标题：两侧都 trim，且空标题不覆盖旧标题', () => {
  assert.ok(tabTitleJs.includes("(document.title || '').trim()"), 'tab_title.js 必须 trim 标题')
  assert.ok(tabTitleJs.includes('if (!t || t === last) return'), 'tab_title.js 空标题必须不发送')
  assert.ok(tabsTs.includes('title.trim()'), 'tabs.ts::setTabTitle 必须 trim')
  assert.ok(
    /const next = title\.trim\(\)\s*\n\s*if \(!next\) return/.test(tabsTs),
    'tabs.ts 空标题必须保留旧标题'
  )
})

test('系统占用快捷键表两侧键与说明逐字一致', () => {
  const shellBlock = settingsHtml.slice(
    settingsHtml.indexOf('const HK_RESERVED'),
    settingsHtml.indexOf('};', settingsHtml.indexOf('const HK_RESERVED'))
  )
  const mainBlock = hotkeyTs.slice(
    hotkeyTs.indexOf('const RESERVED_HOTKEYS'),
    hotkeyTs.indexOf('\n}', hotkeyTs.indexOf('const RESERVED_HOTKEYS'))
  )
  const a = pairsOf(shellBlock)
  const b = pairsOf(mainBlock)
  assert.ok(Object.keys(a).length >= 7, 'settings.html 的 HK_RESERVED 抽取失败')
  assert.deepEqual(b, a, 'hotkey.ts 与 settings.html 的系统占用表已漂移')
})
