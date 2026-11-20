/**
 * `nav-text.ts` 共享文案 / 判定单测（2026-11-07 架构审查 M-4）。
 *
 * 覆盖的是「两处各写一份就会分叉」的两类东西：加载失败文案、内部页面 scheme 表。
 */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { isInternalUrl, isSplashUrl, navFailText } from '../src/main/nav-text.ts'

test('navFailText：debug 包保留 URL 与 net error code（单窗口与页签共用）', () => {
  assert.equal(
    navFailText('https://example.com/x', 'ERR_NAME_NOT_RESOLVED', -105, true),
    '无法打开 https://example.com/x（ERR_NAME_NOT_RESOLVED，-105）'
  )
})

test('navFailText：默认包（debug=false）不把 URL / 错误码摆给最终用户', () => {
  const s = navFailText('http://127.0.0.1:8123/auth.html?w2a_key=abc', 'ERR_CONNECTION_REFUSED', -102)
  assert.equal(s, '页面无法打开，请检查网络连接后重试。')
  assert.ok(!s.includes('127.0.0.1'), '内部入口地址不得出现在用户可见文案里')
  assert.ok(!s.includes('-102'), 'net error code 属诊断')
})

test('isSplashUrl：splash:// 与 wry 改写形态都认，伪装域名不认', () => {
  assert.equal(isSplashUrl('splash://localhost/'), true)
  assert.equal(isSplashUrl('http://splash.localhost/'), true)
  assert.equal(isSplashUrl('https://splash.localhost/?x'), true)
  assert.equal(isSplashUrl('https://splash.localhost.evil.com/'), false)
  assert.equal(isSplashUrl('https://example.com/'), false)
  assert.equal(isSplashUrl(''), false)
})

test('isInternalUrl：内部 scheme 六项全覆盖，真实网页与空串判定分明', () => {
  for (const u of [
    'splash://localhost/',
    'https://splash.localhost/',
    'about:blank',
    'chrome://version',
    'devtools://x',
    'settings://localhost/',
    'shell://localhost/',
    'floaticon://localhost/',
    '',
  ]) {
    assert.equal(isInternalUrl(u), true, `应判内部地址：${u || '(空串)'}`)
  }
  for (const u of ['https://example.com/', 'http://127.0.0.1:8080/', 'file:///c:/x.html']) {
    assert.equal(isInternalUrl(u), false, `应判真实地址：${u}`)
  }
})
