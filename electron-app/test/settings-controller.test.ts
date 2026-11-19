/**
 * 设置应用控制器的成组回滚矩阵单测（2026-11-18 架构复查）。
 *
 * 此前该编排进不了 node:test：模块顶层 import `tray.ts` / `shortcuts.ts` / `float-window.ts`
 * （三者都顶层 import electron，node 下加载即炸），而回滚矩阵恰恰是「错了也不报错」、
 * 且真实踩过坑（曾经只回滚一半）的逻辑。现在副作用全部经 deps 注入，这里用假实现钉住：
 *   · 成功路径：副作用建立 + 持久化 + 内存态提交；
 *   · 持久化失败：本次新建的回滚销毁、本次销毁的重建/重显、快捷键回滚到旧值；
 *   · 「本来就没有」的副作用不凭空回滚（M-8）；
 *   · 回滚自身失败不得掩盖原始错误；
 *   · 冷启动 applyInitialSettings 以 DEFAULT_SETTINGS 为比较基准。
 */
import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test, type TestContext } from 'node:test'

import { createSettingsController, type SettingsControllerDeps } from '../src/main/settings-controller.ts'
import { type Settings, type StoreLocation } from '../src/main/settings.ts'

const S = (patch: Partial<Settings> = {}): Settings => ({
  wake_hotkey: { ctrl: true, alt: false, shift: false, key: 'KeyA' },
  tray_enabled: false,
  minimize_on_close: false,
  float_enabled: false,
  ...patch,
})

/** 一个「写不进去」的 StoreLocation：write 落在【文件】之下，mkdirSync 必抛 ENOTDIR。 */
function brokenStore(t: TestContext): StoreLocation {
  const dir = mkdtempSync(join(tmpdir(), 'w2a-settings-ctl-'))
  t.after(() => rmSync(dir, { recursive: true, force: true }))
  const asFile = join(dir, 'not-a-dir')
  writeFileSync(asFile, 'x')
  return { read: [join(dir, 'settings.json')], write: join(asFile, 'settings.json') }
}

function okStore(t: TestContext): StoreLocation {
  const dir = mkdtempSync(join(tmpdir(), 'w2a-settings-ctl-'))
  t.after(() => rmSync(dir, { recursive: true, force: true }))
  return { read: [join(dir, 'settings.json')], write: join(dir, 'settings.json') }
}

interface Recorder {
  calls: string[]
  hotkeys: string[]
}

/** 造一套假 deps：全部副作用只记账；行为按 opts 调节。 */
function fakeDeps(
  current: Settings,
  store: StoreLocation,
  opts: {
    createTrayOk?: boolean
    createTrayThrowOn?: number // 第 N 次 createTray 调用抛异常（1 基）
    hasTray?: boolean
    isFloatAlive?: boolean
  } = {}
): { deps: SettingsControllerDeps; rec: Recorder; setCalls: Settings[] } {
  const rec: Recorder = { calls: [], hotkeys: [] }
  const setCalls: Settings[] = []
  let createTrayN = 0
  const deps: SettingsControllerDeps = {
    title: 't',
    getSettings: () => current,
    setSettings: (s) => {
      setCalls.push(s)
    },
    getStoreLoc: () => store,
    isMainWindowVisible: () => true,
    openSettings: () => rec.calls.push('openSettings'),
    presentMainWindow: () => rec.calls.push('presentMainWindow'),
    hideMainWindow: () => rec.calls.push('hideMainWindow'),
    toggleMainWindow: () => rec.calls.push('toggleMainWindow'),
    requestQuit: () => rec.calls.push('requestQuit'),
    trayIconPath: () => 'icon.png',
    showFloatWindow: () => rec.calls.push('showFloatWindow'),
    destroyFloatWindow: () => rec.calls.push('destroyFloatWindow'),
    toggleFloatWindow: () => rec.calls.push('toggleFloatWindow'),
    applyWakeHotkey: (h) => {
      rec.hotkeys.push(h.key)
      return { ok: true, accelerator: h.key || null }
    },
    createTray: () => {
      createTrayN += 1
      if (opts.createTrayThrowOn === createTrayN) throw new Error('假托盘炸了')
      rec.calls.push('createTray')
      return opts.createTrayOk !== false
    },
    destroyTray: () => rec.calls.push('destroyTray'),
    hasTray: () => opts.hasTray ?? false,
    isFloatAlive: () => opts.isFloatAlive ?? false,
  }
  return { deps, rec, setCalls }
}

test('冷启动 applyInitialSettings：以 DEFAULT_SETTINGS 为基准，diff 恒真也要建托盘/悬浮球', (t) => {
  // getSettings 返回的内存态与 next **相同**（正是 2026-10-10 实测踩坑的形态）：
  // 若比较基准不是 DEFAULT_SETTINGS，createTray / showFloatWindow 一次都不会被调。
  const next = S({ tray_enabled: true, float_enabled: true })
  const { deps, rec, setCalls } = fakeDeps(next, okStore(t))
  createSettingsController(deps).applyInitialSettings(next)
  assert.deepEqual(
    rec.calls.filter((c) => c === 'createTray' || c === 'showFloatWindow'),
    ['createTray', 'showFloatWindow'],
    '冷启动必须从默认态切过去（托盘 / 悬浮球都要建）'
  )
  assert.equal(setCalls.length, 1, '内存态要提交')
})

test('保存成功：新建托盘 + 换快捷键 + 新建悬浮球，全部生效且内存态提交', (t) => {
  const prev = S()
  const next = S({
    tray_enabled: true,
    float_enabled: true,
    wake_hotkey: { ctrl: true, alt: false, shift: false, key: 'KeyB' },
  })
  const { deps, rec, setCalls } = fakeDeps(prev, okStore(t))
  createSettingsController(deps).applySettings(next, { persist: true })
  assert.ok(rec.calls.includes('createTray'))
  assert.ok(rec.calls.includes('showFloatWindow'))
  assert.deepEqual(rec.hotkeys, ['KeyB'])
  assert.equal(setCalls.length, 1)
})

test('持久化失败 + 本次新建托盘：destroyTray 回滚、内存态不提交、原始错误上抛', (t) => {
  const prev = S()
  const next = S({ tray_enabled: true })
  const { deps, rec, setCalls } = fakeDeps(prev, brokenStore(t))
  assert.throws(
    () => createSettingsController(deps).applySettings(next, { persist: true }),
    /EEXIST|ENOTDIR|not a directory|EISDIR|EINVAL|ENOENT/i,
    '持久化错误必须原样上抛'
  )
  assert.deepEqual(
    rec.calls,
    ['createTray', 'destroyTray'],
    '本次新建的托盘必须回滚销毁'
  )
  assert.equal(setCalls.length, 0, '任一步失败都不写内存态')
})

test('持久化失败 + 本次销毁托盘（此前真有）：best-effort 重建', (t) => {
  const prev = S({ tray_enabled: true })
  const next = S({ tray_enabled: false })
  const { deps, rec } = fakeDeps(prev, brokenStore(t), { hasTray: true })
  assert.throws(() => createSettingsController(deps).applySettings(next, { persist: true }))
  assert.deepEqual(rec.calls, ['destroyTray', 'createTray'], '销毁过的托盘必须重建回滚')
})

test('持久化失败 + 托盘本来就没有：不得凭空造托盘（M-8）', (t) => {
  const prev = S({ tray_enabled: true })
  const next = S({ tray_enabled: false })
  const { deps, rec } = fakeDeps(prev, brokenStore(t), { hasTray: false })
  assert.throws(() => createSettingsController(deps).applySettings(next, { persist: true }))
  assert.deepEqual(rec.calls, ['destroyTray'], 'hasTray=false 时回滚不得 createTray')
})

test('持久化失败 + 悬浮球新建 / 销毁：对称回滚', (t) => {
  // 新建 → 回滚销毁
  {
    const { deps, rec } = fakeDeps(S(), brokenStore(t))
    assert.throws(() =>
      createSettingsController(deps).applySettings(S({ float_enabled: true }), { persist: true })
    )
    assert.deepEqual(rec.calls, ['showFloatWindow', 'destroyFloatWindow'])
  }
  // 销毁（此前真有）→ 回滚重显
  {
    const { deps, rec } = fakeDeps(S({ float_enabled: true }), brokenStore(t), {
      isFloatAlive: true,
    })
    assert.throws(() =>
      createSettingsController(deps).applySettings(S({ float_enabled: false }), { persist: true })
    )
    assert.deepEqual(rec.calls, ['destroyFloatWindow', 'showFloatWindow'])
  }
  // 销毁（此前没有）→ 不重显
  {
    const { deps, rec } = fakeDeps(S({ float_enabled: true }), brokenStore(t), {
      isFloatAlive: false,
    })
    assert.throws(() =>
      createSettingsController(deps).applySettings(S({ float_enabled: false }), { persist: true })
    )
    assert.deepEqual(rec.calls, ['destroyFloatWindow'])
  }
})

test('持久化失败 + 快捷键变更：回滚到旧键；未变更则不重复注册', (t) => {
  const prev = S({ wake_hotkey: { ctrl: true, alt: false, shift: false, key: 'KeyA' } })
  const next = S({ wake_hotkey: { ctrl: true, alt: false, shift: false, key: 'KeyB' } })
  {
    const { deps, rec } = fakeDeps(prev, brokenStore(t))
    assert.throws(() => createSettingsController(deps).applySettings(next, { persist: true }))
    assert.deepEqual(rec.hotkeys, ['KeyB', 'KeyA'], '失败后必须把旧键注册回去')
  }
  {
    const { deps, rec } = fakeDeps(prev, brokenStore(t))
    assert.throws(() => createSettingsController(deps).applySettings(prev, { persist: true }))
    assert.deepEqual(rec.hotkeys, ['KeyA'], '键未变不得二次注册')
  }
})

test('回滚自身失败（createTray 抛异常）不得掩盖原始持久化错误', (t) => {
  const prev = S({ tray_enabled: true })
  const next = S({ tray_enabled: false })
  // 第 1 次 createTray 不会发生（这是 true→false 路径）；回滚重建是第 1 次调用 → 抛。
  const { deps } = fakeDeps(prev, brokenStore(t), { hasTray: true, createTrayThrowOn: 1 })
  assert.throws(
    () => createSettingsController(deps).applySettings(next, { persist: true }),
    (e: Error) => {
      assert.ok(
        !/假托盘炸了/.test(e.message),
        `原始错误不得被回滚异常掩盖，实际得到：${e.message}`
      )
      return true
    }
  )
})

test('托盘创建失败：抛业务错误、不持久化、不写内存态', (t) => {
  const { deps, setCalls } = fakeDeps(S(), okStore(t), { createTrayOk: false })
  assert.throws(
    () => createSettingsController(deps).applySettings(S({ tray_enabled: true }), { persist: true }),
    /托盘创建失败/
  )
  assert.equal(setCalls.length, 0)
})
