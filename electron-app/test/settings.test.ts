/**
 * settings.ts 单测（node:test，零 Electron 依赖，直接跑 TS 源码）。
 *
 * 钉的是 `settings.json` 三段共居一个文件后最容易静默丢数据的一类回归：
 * 三个写入方（设置 / 主窗口几何 / 悬浮球几何）互相覆盖对方那一段。
 * 读-改-写的统一入口是 [`updateDocument`]，三个薄封装（`saveSettings` /
 * `window-state.ts::saveWindowState` / `float-window.ts::persistGeometry`）
 * 都收敛到它 —— 后两者依赖 Electron，无法在 node 单测里导入，故这里直接钉
 * `updateDocument` 各段替换的语义（三个薄封装只有一行调用）。
 *
 * ⚠️ 「node --test 直接跑 TS」的跨文件导入补丁（resolve 钩子把 src 内部无扩展名的
 * 相对导入补成 `.ts`）已收敛到 `test/ts-resolver.mjs`，由 `pnpm run unit` 经 `--import`
 * 预载到每个测试进程（钩子必须先于被测模块的导入图解析，故不能写在测试文件里）。
 * 下面的被测模块仍走**动态 import**：tsc 按 CJS 语义检查（package.json 无
 * `"type": "module"`）不许顶层 await，故只存 Promise，由各测试 await 取模块。
 *
 * 运行：`pnpm run unit`（Node ≥ 22.6 内置类型擦除；本机 Node 24 直接执行 .ts）。
 */
import assert from 'node:assert/strict'
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, test } from 'node:test'
import type { StoreLocation } from '../src/main/settings.ts'

const ready: Promise<typeof import('../src/main/settings.ts')> = import('../src/main/settings.ts')

const tmpDirs: string[] = []
afterEach(() => {
  while (tmpDirs.length) rmSync(tmpDirs.pop()!, { recursive: true, force: true })
})

/** 临时目录 + 单候选 StoreLocation（读=写=主位置）。 */
function makeLoc(): { loc: StoreLocation; dir: string } {
  const dir = mkdtempSync(join(tmpdir(), 'web2app-settings-'))
  tmpDirs.push(dir)
  const main = join(dir, 'settings.json')
  return { loc: { read: [main], write: main }, dir }
}

/** 种子文档：三段都有值，且各段取值可互相区分。 */
const SEED = {
  wake_hotkey: { ctrl: false, alt: true, shift: false, key: 'Digit4' },
  tray_enabled: true,
  minimize_on_close: true,
  float_enabled: true,
  window: { x: 1, y: 2, width: 800, height: 600, maximized: true },
  float_window: { x: 3, y: 4, width: 84, height: 84, edge: 'left' },
}

function seed(loc: StoreLocation): void {
  writeFileSync(loc.write, JSON.stringify(SEED), 'utf8')
}

function readRaw(loc: StoreLocation): Record<string, unknown> {
  return JSON.parse(readFileSync(loc.write, 'utf8')) as Record<string, unknown>
}

// ---------- ① 写 settings：window / float_window 原样保留 ----------

test('saveSettings：只替换设置段，两段几何原样保留', async () => {
  const { saveSettings } = await ready
  const { loc } = makeLoc()
  seed(loc)
  saveSettings(loc, {
    wake_hotkey: { ctrl: true, alt: false, shift: false, key: 'F1' },
    tray_enabled: false,
    minimize_on_close: false,
    float_enabled: false,
  })
  const out = readRaw(loc)
  assert.deepEqual(out.window, SEED.window, 'window 段被设置保存抹掉了')
  assert.deepEqual(out.float_window, SEED.float_window, 'float_window 段被设置保存抹掉了')
  assert.equal(out.tray_enabled, false)
  assert.deepEqual(out.wake_hotkey, { ctrl: true, alt: false, shift: false, key: 'F1' })
})

// ---------- ② 写 window：settings / float_window 原样保留 ----------

test('updateDocument(window)：只替换 window 段，设置与悬浮球几何原样保留', async () => {
  const { parseSettings, updateDocument } = await ready
  const { loc } = makeLoc()
  seed(loc)
  const st = { x: 10, y: 20, width: 1024, height: 768, maximized: false }
  updateDocument(loc, { window: st })
  const out = readRaw(loc)
  assert.deepEqual(out.window, st)
  assert.deepEqual(out.float_window, SEED.float_window, 'float_window 段被记主窗口几何抹掉了')
  assert.deepEqual(parseSettings(out), parseSettings(SEED), '设置段被记主窗口几何抹掉了')
})

// ---------- ③ 写 float_window：settings / window 原样保留 ----------

test('updateDocument(float_window)：只替换 float_window 段，其余原样保留', async () => {
  const { parseSettings, updateDocument } = await ready
  const { loc } = makeLoc()
  seed(loc)
  const geom = { x: 5, y: 6, width: 84, height: 84, edge: 'right' }
  updateDocument(loc, { float_window: geom })
  const out = readRaw(loc)
  assert.deepEqual(out.float_window, geom)
  assert.deepEqual(out.window, SEED.window, 'window 段被悬浮球落盘抹掉了')
  assert.deepEqual(parseSettings(out), parseSettings(SEED), '设置段被悬浮球落盘抹掉了')
})

// ---------- ④ 键序固定 + 原子写无临时文件残留 ----------

test('writeDocument 经 updateDocument：键序「设置四键 → window → float_window」且无 tmp 残留', async () => {
  const { updateDocument } = await ready
  const { loc, dir } = makeLoc()
  seed(loc)
  updateDocument(loc, { window: { x: 0, y: 0, width: 640, height: 480, maximized: false } })
  const out = readRaw(loc)
  assert.deepEqual(
    Object.keys(out),
    ['wake_hotkey', 'tray_enabled', 'minimize_on_close', 'float_enabled', 'window', 'float_window'],
    '键序与 Tauri 侧 store.rs::DocOut 字段序不一致'
  )
  // 原子写：临时文件必须已被 rename 收走（坏了会留下 .tmp-*）。
  assert.deepEqual(readdirSync(dir), ['settings.json'])
})

// ---------- ⑤ storeLocation 读候选顺序 + 旧位置升级兼容 ----------

test('storeLocation：读候选主位置优先、旧 %APPDATA% 位置兜底，写恒主位置', async () => {
  const { parseSettings, storeLocation, updateDocument } = await ready
  const dataDir = mkdtempSync(join(tmpdir(), 'web2app-data-'))
  const appDataDir = mkdtempSync(join(tmpdir(), 'web2app-appdata-'))
  tmpDirs.push(dataDir, appDataDir)
  const loc = storeLocation({ dataDir, appDataDir, appName: 'My App', file: 'settings.json' })
  const main = join(dataDir, 'settings.json')
  const legacy = join(appDataDir, 'My App', 'settings.json')
  assert.deepEqual(loc.read, [main, legacy], '读候选顺序必须是「主位置 → 旧位置」')
  assert.equal(loc.write, main, '写位置只能是数据目录下的主位置')

  // 升级兼容：新位置还没有文件、老设置还在旧位置 —— 第一次写盘不得把它们清零。
  mkdirSync(join(appDataDir, 'My App'), { recursive: true })
  writeFileSync(legacy, JSON.stringify(SEED), 'utf8')
  updateDocument(loc, {
    settings: { ...parseSettings(SEED), tray_enabled: false },
  })
  assert.ok(existsSync(main), '第一次写盘必须落到新位置')
  const out = readRaw(loc)
  assert.deepEqual(out.window, SEED.window, '旧位置里的 window 段在升级首写时被清零了')
  assert.deepEqual(out.float_window, SEED.float_window, '旧位置里的 float_window 段在升级首写时被清零了')
  assert.equal(out.tray_enabled, false)
})
