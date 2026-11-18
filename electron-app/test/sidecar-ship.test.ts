/**
 * 打包期 sidecar 随包裁剪（2026-11-18）。
 *
 * 背景：`proxy_enabled=false` + 字符串 `app_url` 时运行期 `needsSidecar` 恒假，
 * `w2a-proxy.exe` 一次都不会被执行；但 `extraResources` 会无条件拷进包。包内留一个
 * 永不执行的二进制，会在首次被拉起/扫描时触发「这个程序要联网」的提示。
 *
 * 本文件钉三件事：
 *   1. 判据真值表与运行期完全一致（打包期不得另写一份规则）；
 *   2. 落盘动作的四种组合（保留 / 删除 / 无需 / 需要却缺失必须抛错）；
 *   3. 三个出包入口共享的接线（yml 钩子 + 编译产物 + 护栏双向断言）。
 */
import assert from 'node:assert/strict'
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test, type TestContext } from 'node:test'

import type { AppConfig } from '../src/main/config.ts'
import { needsSidecar } from '../src/main/navigate.ts'
import {
  packagedSidecarPath,
  shouldShipSidecar,
  sidecarExeName,
  syncPackagedSidecar,
} from '../src/main/sidecar-ship.ts'

const appDir = process.cwd()

const cfg = (patch: Partial<AppConfig> = {}): AppConfig =>
  ({
    app_url: 'https://example.com/',
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
    ...patch,
  }) as unknown as AppConfig

test('随包判据真值表：与运行期 needsSidecar 完全一致', () => {
  const cases: { name: string; c: AppConfig; want: boolean }[] = [
    { name: '不启用代理 + 字符串地址', c: cfg(), want: false },
    { name: '不启用代理 + 域名数组', c: cfg({ app_url: ['a.example.com'], public_key: 'k' }), want: true },
    { name: '启用代理 + 无外部代理', c: cfg({ proxy_enabled: true }), want: true },
    { name: '启用代理 + 外部代理', c: cfg({ proxy_enabled: true, proxy_http: 'http://127.0.0.1:8080' }), want: false },
  ]
  for (const { name, c, want } of cases) {
    assert.equal(needsSidecar(c), want, `${name}：运行期判据`)
    assert.equal(shouldShipSidecar(c), want, `${name}：打包期判据必须复用运行期实现`)
  }
})

test('sidecar 文件名 / 包内路径按平台拼装', () => {
  assert.equal(sidecarExeName('win32'), 'w2a-proxy.exe')
  assert.equal(sidecarExeName('linux'), 'w2a-proxy')
  assert.equal(
    packagedSidecarPath(join('D:', 'app', 'resources'), 'win32'),
    join('D:', 'app', 'resources', 'sidecar', 'w2a-proxy.exe'),
  )
})

/** 造一个「已铺好 extraResources」的假产物目录，返回 resources 目录。 */
function fakeResources(t: TestContext): string {
  const root = mkdtempSync(join(tmpdir(), 'w2a-sidecar-ship-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const resources = join(root, 'resources')
  mkdirSync(join(resources, 'sidecar'), { recursive: true })
  writeFileSync(join(resources, 'sidecar', 'w2a-proxy.exe'), 'stub')
  return resources
}

test('需要 sidecar：文件在则保留', (t) => {
  const resources = fakeResources(t)
  assert.equal(syncPackagedSidecar(resources, true, 'win32'), 'kept')
  assert.equal(existsSync(packagedSidecarPath(resources, 'win32')), true)
})

test('不需要 sidecar：删除文件并移除空目录', (t) => {
  const resources = fakeResources(t)
  assert.equal(syncPackagedSidecar(resources, false, 'win32'), 'removed')
  assert.equal(existsSync(packagedSidecarPath(resources, 'win32')), false)
  assert.equal(existsSync(join(resources, 'sidecar')), false)
})

test('不需要 sidecar 且本来就没有：幂等 no-op', (t) => {
  const resources = fakeResources(t)
  rmSync(join(resources, 'sidecar'), { recursive: true, force: true })
  assert.equal(syncPackagedSidecar(resources, false, 'win32'), 'absent')
})

test('需要 sidecar 但产物里缺失：必须抛错（不能静默出包）', (t) => {
  const resources = fakeResources(t)
  rmSync(join(resources, 'sidecar'), { recursive: true, force: true })
  assert.throws(
    () => syncPackagedSidecar(resources, true, 'win32'),
    /需要 sidecar/,
    '缺 sidecar 时必须让构建失败，而不是出一个启动即报错的包',
  )
})

// ---------- 接线守卫（跑 electron-builder 太重，用源码文本钉住） ----------

const read = (p: string): string => readFileSync(join(appDir, p), 'utf8')
/** 剥掉注释，避免「注释里写了关键字」造成假绿。 */
const stripComments = (s: string): string =>
  s.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '')

test('electron-builder.yml 必须挂上 afterPack 钩子（三个出包入口共享本文件）', () => {
  const yml = read('electron-builder.yml')
    .split(/\r?\n/)
    .filter((line) => !line.trim().startsWith('#'))
    .join('\n')
  assert.match(yml, /^afterPack:\s*\.\/scripts\/after-pack\.cjs\s*$/m, '缺少 afterPack 钩子接线')
})

test('after-pack 钩子只调用编译产物里的判据，不重写代理规则', () => {
  const code = stripComments(read('scripts/after-pack.cjs'))
  assert.ok(code.includes("'sidecar-ship.js'"), '钩子必须 require out/main/sidecar-ship.js')
  assert.ok(code.includes("'config.js'"), '钩子必须用 out/main/config.js 解析配置')
  assert.ok(code.includes('shouldShipSidecar'), '钩子必须复用 shouldShipSidecar 判据')
  assert.ok(code.includes('syncPackagedSidecar'), '钩子必须复用 syncPackagedSidecar 动作')
  assert.ok(
    !code.includes('needsLocalProxy') && !code.includes('proxy_http'),
    '不得在钩子里重新实现代理判据（必须由 sidecar-ship.ts 复用 navigate.ts）',
  )
})

test('asar 护栏必须双向断言 sidecar 的存在性', () => {
  const guard = read('scripts/check-asar-config.cjs')
  assert.ok(guard.includes('shouldShipSidecar'), '护栏必须复用同一判据')
  assert.ok(guard.includes('packagedSidecarPath'), '护栏必须按包内路径检查文件')
  assert.ok(guard.includes('配置需要 sidecar，但包里没有'), '缺少「该有却没有」的断言')
  assert.ok(guard.includes('配置不需要 sidecar，但包里仍有'), '缺少「不该有却还在」的断言')
})
