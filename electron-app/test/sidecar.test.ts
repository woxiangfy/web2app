/**
 * `sidecar.ts` 参数拼装与可执行文件定位单测（2026-11-07 架构审查 M-5）。
 *
 * 这两条是「加密解析是否真的生效」的第一道：参数拼错时 sidecar 照常起得来，
 * 只是行为变错（例如凭据进了命令行、域名模式漏了公钥），运行期探针不一定能发现。
 */
import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'

import { buildSidecarArgs, locateSidecar } from '../src/main/sidecar.ts'

const exe = process.platform === 'win32' ? 'w2a-proxy.exe' : 'w2a-proxy'

test('容量域名模式：--domains 逗号拼接 + --public-key', () => {
  assert.deepEqual(buildSidecarArgs({ domains: ['a.example', 'b.example'], publicKey: 'KEY' }), [
    '--domains',
    'a.example,b.example',
    '--public-key',
    'KEY',
  ])
})

test('直连 / 代理模式：地址走 --url-stdin（凭据绝不出现在命令行）', () => {
  const args = buildSidecarArgs({ url: 'https://user:pass@example.com/' })
  assert.deepEqual(args, ['--url-stdin'])
  assert.ok(!args.join(' ').includes('pass'), '凭据不得出现在命令行参数里')
})

test('domains 与 url 同时给 → 以 domains 为准（对齐 Tauri 侧）', () => {
  assert.deepEqual(buildSidecarArgs({ domains: ['a.example'], publicKey: 'K', url: 'https://x/' }), [
    '--domains',
    'a.example',
    '--public-key',
    'K',
  ])
})

test('空 domains + url → 仍按直连模式（空数组不算域名模式）', () => {
  assert.deepEqual(buildSidecarArgs({ domains: [], url: 'https://x/' }), ['--url-stdin'])
})

test('noProxy 追加 --no-proxy', () => {
  assert.deepEqual(buildSidecarArgs({ url: 'https://x/', noProxy: true }), ['--url-stdin', '--no-proxy'])
})

test('什么请求都没有 → 空数组（startSidecar 据此报 args 失败，绝不静默直连）', () => {
  assert.deepEqual(buildSidecarArgs({}), [])
})

test('locateSidecar：优先 resources/sidecar，其次 webview-app/target/release，最后 debug', () => {
  const root = mkdtempSync(join(tmpdir(), 'w2a-sidecar-'))
  try {
    const appPath = join(root, 'electron-app')
    mkdirSync(appPath, { recursive: true })
    const res = join(root, 'res')
    const release = join(root, 'webview-app', 'target', 'release')
    const debug = join(root, 'webview-app', 'target', 'debug')
    for (const d of [join(res, 'sidecar'), release, debug]) mkdirSync(d, { recursive: true })

    // 三者都不存在 → null（本次构建里没有 sidecar）
    assert.equal(locateSidecar(appPath, res), null)

    writeFileSync(join(debug, exe), '')
    assert.equal(locateSidecar(appPath, res), join(debug, exe), '只有 debug 时应落到 debug')

    writeFileSync(join(release, exe), '')
    assert.equal(locateSidecar(appPath, res), join(release, exe), 'release 优先于 debug')

    writeFileSync(join(res, 'sidecar', exe), '')
    assert.equal(locateSidecar(appPath, res), join(res, 'sidecar', exe), '打包位置优先于开发态')
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})
