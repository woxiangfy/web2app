/**
 * 运行期配置文档（`settings.json`）—— 与 Tauri 版（`webview-app/src/store.rs`）**共用同一份契约**。
 *
 * 这是全应用**唯一**的一份持久化 JSON：设置字段平铺在顶层，窗口几何在 `window` 段。
 *
 * ```json
 * {
 *   "wake_hotkey": { "ctrl": false, "alt": true, "shift": false, "key": "Digit4" },
 *   "tray_enabled": true,
 *   "minimize_on_close": false,
 *   "window": { "x": 100, "y": 200, "width": 1100, "height": 650, "maximized": false }
 * }
 * ```
 *
 * 契约四要素（改动任何一项都等于破坏双壳兼容，见 docs/electron-plan.md §3）：
 *   1. **字段名与嵌套结构**：`{ wake_hotkey: { ctrl, alt, shift, key }, tray_enabled, minimize_on_close }`；
 *      `key` 是 JS `KeyboardEvent.code`（`"Digit4"` / `"F1"` / `"KeyA"`）。
 *   2. **文件路径**：`<data_dir>\settings.json` —— **与 Tauri 版同一个规则、同一个位置**
 *      （`portable.ts` 已按 `config.json` 的 `data_dir` 解析出 `dataDir`，这里只拼文件名；
 *      Tauri 侧对应 `webview-app/src/config.rs::store_dir`）。默认 `data_dir = "data"`
 *      ⇒ `<exe 同级>\data\settings.json`：设置与 cookie / 缓存 / 日志**一起**跟着程序走。
 *      ⚠️ `data_dir = ""`（系统默认目录）时两壳的「系统默认」不是同一个磁盘位置
 *      （Tauri `%LOCALAPPDATA%` / Electron `%APPDATA%`，见 `docs/packaging-config.md` §4.3 要点 4）
 *      ⇒ 该模式下这个文件各自落在自己那份数据目录里，属数据目录既有的、已文档化的差异。
 *   3. **默认值全关**：不设唤醒快捷键、不启用托盘、关闭即退出。
 *   4. **键序**：设置三键在前、`window` 在后（与 serde 的字段序一致），
 *      两壳写出来的字节形态必须一样（`writeDocument()` 统一规范化）。
 *
 * **2026-09-23 合并**：此前窗口几何单独占一个 `window-state.json`。现在只剩这一个文件，
 * 旧的 `window-state.json` **不再读也不再写**（已确认不做迁移）—— 升级后窗口会回一次
 * 默认尺寸/位置，那个文件就此成为死文件，可手工删除。
 *
 * ⚠️⚠️ 合并后的唯一真风险是**两段互相覆盖**：写入方有两个（设置窗保存 vs 窗口关闭时记几何），
 * 从前各写各的文件天然隔离，现在共用一个文件。所以两个写入方**都必须**走
 * [`saveSettings`] / `saveWindowState` 这条「读盘 → 只替换自己那一段 → 写回」的路径
 * —— 拿内存里的整份文档直接 `writeFileSync` 会在「挪了下窗口 + 改了设置」同时发生时
 * 静默吃掉其中一方。（`window-state.ts` 的文件头正是用这个理由论证过「不该合并」；
 * 既然合并了，就用读-改-写把风险挡掉，并由探针钉住。）
 *
 * ⚠️⚠️ **落点 = 数据目录**（2026-09-23 使用者指出）：此前这个小 JSON 刻意留在
 * `%APPDATA%\<app_name>\`（「与旧版同路径」），结果是选了绿色版（`data_dir = "data"`）后
 * cookie / 缓存 / 日志都跟着 exe 走，**唯独设置文件留在系统盘** —— 同一套「程序的数据」里
 * 唯一不听话的一项。现在写入位置一律是 `<dataDir>\settings.json`，与 `data_dir` 字段
 * （`docs/packaging-config.md` §4.3）完全一致。
 * ⚠️ 旧位置（`%APPDATA%\<app_name>\settings.json`）保留为**读取候选**（升级兼容）：新位置还没有
 *    文件时读它、第一次写盘就落到新位置，此后旧文件是死文件。读取顺序由 [`storeLocation`]
 *    统一给出，**读也用它、写只用主位置** —— 否则「用旧文件的值算出来的新文档」会写到新位置、
 *    而旧文件被绕过，用户看到的是设置被重置。
 */
import { existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'

import { sanitizeDirName } from './config'

export const SETTINGS_FILE = 'settings.json'

/** 文档里「窗口几何」那一段的键名（Tauri 侧 `store.rs` 的 `window` 字段）。 */
export const WINDOW_KEY = 'window'

/** 唤醒快捷键结构。`key` 为 JS `KeyboardEvent.code`，空串表示「不启用」。 */
export interface Hotkey {
  ctrl: boolean
  alt: boolean
  shift: boolean
  key: string
}

export interface Settings {
  /** 显示 / 隐藏主窗口的全局快捷键 */
  wake_hotkey: Hotkey
  /** 是否显示系统托盘图标 */
  tray_enabled: boolean
  /** 点关闭按钮时隐藏到托盘（需同时启用托盘，否则关闭即退出） */
  minimize_on_close: boolean
}

export const DEFAULT_HOTKEY: Hotkey = { ctrl: false, alt: false, shift: false, key: '' }

/** 与 `settings.rs::Settings::default()` 逐字段一致。 */
export const DEFAULT_SETTINGS: Settings = {
  wake_hotkey: { ...DEFAULT_HOTKEY },
  tray_enabled: false,
  minimize_on_close: false,
}

/**
 * 一个「落在磁盘上的 JSON 小文件」的读写位置。
 *
 * 之所以把「读多个候选、写单个位置」做成显式结构：两个壳并行期间，
 * 文件可能出现在旧版目录、也可能被用户放进绿色目录，靠隐式约定迟早出错。
 */
export interface StoreLocation {
  /** 读取顺序，先命中者生效 */
  read: string[]
  /** 写入位置 */
  write: string
}

export interface StoreLocationOptions {
  /** 数据目录（`data_dir` 解析结果：exe 同级 `data\`、绝对路径、或系统默认程序目录） */
  dataDir: string
  /** `app.getPath('appData')`，即 `%APPDATA%` —— **只用来拼旧位置**（读取候选） */
  appDataDir: string
  /** config.json 的 app_name（会做 Windows 目录名清洗） */
  appName: string
  /** 文件名，如 `settings.json` */
  file: string
}

/**
 * 解析存储位置。
 *
 * 写入：**一律** `<dataDir>\<file>`（= `data_dir` 字段解析出来的数据目录，与 Tauri 版同位置）。
 * 读取：`<dataDir>\<file>` → `%APPDATA%\<app_name>\<file>`（旧位置，升级兼容），先命中者生效。
 *
 * ⚠️⚠️ 写位置只有一个，读位置有两个 —— 这是有意的：写要「搬家搬到位」（数据跟着程序走），
 * 读要「别把老用户的设置弄丢」。⚠️ 但**读-改-写的「读」必须走同一份候选**
 * （[`readRawForUpdate`]），否则升级后第一次保存会用「新位置（空）算出来的文档」
 * 覆盖掉旧位置里的设置与几何。
 */
export function storeLocation(opts: StoreLocationOptions): StoreLocation {
  const legacy = join(opts.appDataDir, sanitizeDirName(opts.appName), opts.file)
  const main = join(opts.dataDir, opts.file)

  // 系统默认模式下两者可能是同一个目录（Android 之外，Electron 的 dataDir 恒为具体目录，
  // 这里只是防御性地去重，避免 read 列表里出现同一个路径两次）。
  if (main === legacy) return { read: [main], write: main }

  return { read: [main, legacy], write: main }
}

/** 空串 / 非字符串按默认值处理；宽容解析（上游模板里布尔值是字符串）。 */
function toBool(v: unknown, dflt: boolean): boolean {
  if (typeof v === 'boolean') return v
  if (typeof v === 'string') {
    const s = v.trim().toLowerCase()
    if (s === '') return dflt
    return s === 'true' || s === '1' || s === 'yes' || s === 'on'
  }
  return dflt
}

function toStr(v: unknown, dflt = ''): string {
  return typeof v === 'string' ? v : v == null ? dflt : String(v)
}

/** 解析 hotkey。缺字段按 false / 空串补齐，非法类型不抛错。 */
export function parseHotkey(v: unknown): Hotkey {
  if (!v || typeof v !== 'object') return { ...DEFAULT_HOTKEY }
  const o = v as Record<string, unknown>
  return {
    ctrl: toBool(o.ctrl, false),
    alt: toBool(o.alt, false),
    shift: toBool(o.shift, false),
    key: toStr(o.key).trim(),
  }
}

export function parseSettings(v: unknown): Settings {
  if (!v || typeof v !== 'object') return { ...DEFAULT_SETTINGS, wake_hotkey: { ...DEFAULT_HOTKEY } }
  const o = v as Record<string, unknown>
  return {
    wake_hotkey: parseHotkey(o.wake_hotkey),
    tray_enabled: toBool(o.tray_enabled, DEFAULT_SETTINGS.tray_enabled),
    minimize_on_close: toBool(o.minimize_on_close, DEFAULT_SETTINGS.minimize_on_close),
  }
}

export interface LoadedSettings {
  settings: Settings
  /** 实际命中的文件；都没有时为 null（用默认值） */
  source: string | null
  /** 非致命提示，用于启动日志 */
  warnings: string[]
}

export interface LoadedDocument extends LoadedSettings {
  /**
   * `window` 段的原始值（没写过时为 undefined）。
   * ⚠️ 刻意保持 `unknown`：语义解释是 `window-state.ts` 的事（`parseWindowState` 宽容解析），
   * 这里只负责「原样取出来 / 原样写回去」，免得两个模块各有一份几何校验。
   */
  window: unknown
}

function defaultDoc(): { settings: Settings; window: undefined } {
  return { settings: { ...DEFAULT_SETTINGS, wake_hotkey: { ...DEFAULT_HOTKEY } }, window: undefined }
}

/**
 * 读整份文档（设置 + `window` 段）。文件不存在 / 不是合法 JSON 都回落到默认值，**绝不抛错**。
 *
 * 理由：文档里只有 3 个开关和一段几何，为了它让应用起不来是完全错误的取舍；
 * 而 Tauri 侧（`store::load`）同样是「解析失败即默认值」。
 */
export function loadDocument(loc: StoreLocation): LoadedDocument {
  const warnings: string[] = []
  for (const p of loc.read) {
    let text: string
    try {
      text = readFileSync(p, 'utf8')
    } catch {
      continue // 不存在（或读不动）→ 试下一个候选
    }
    let parsed: unknown
    try {
      parsed = JSON.parse(text)
    } catch (e) {
      // 存在但坏了：明确记一条警告再用默认值，而不是静默换文件 ——
      // 「设置莫名其妙回到默认」是最难查的一类反馈。
      warnings.push(`配置文件解析失败，已改用默认值：${p}（${(e as Error).message}）`)
      return { ...defaultDoc(), source: p, warnings }
    }
    const o = parsed && typeof parsed === 'object' ? (parsed as Record<string, unknown>) : {}
    // ⚠️ 两段各自独立解释：`window` 段坏掉（比如手改成字符串）不该把三个开关一起打回默认；
    //    反之亦然 —— 这是 `parseSettings` / `parseWindowState` 各自宽容解析的意义。
    return { settings: parseSettings(parsed), window: o[WINDOW_KEY], source: p, warnings }
  }
  return { ...defaultDoc(), source: null, warnings }
}

/** 只要设置那一段（探针 / 启动日志用得最多）。 */
export function loadSettings(loc: StoreLocation): LoadedSettings {
  const { settings, source, warnings } = loadDocument(loc)
  return { settings, source, warnings }
}

/**
 * 读**写入用**的那份文档原文：按 `loc.read` 的顺序取第一份**存在**的文件。
 *
 * ⚠️⚠️ 必须是「第一份存在的候选」而不是「写入位置」，也不能只认写入位置：
 * 升级到「落点 = 数据目录」之后，用户的老设置还在 `%APPDATA%\<app_name>\` 里，
 * 写入位置此时**还不存在**。若这里读的是写入位置，第一次保存就会拿着空文档去算新内容
 * ⇒ 用户勾过的托盘开关、`window` 段几何一并归零（表现为「升级后设置被重置」）。
 * 文件不存在 / 坏 JSON 一律给空对象：坏的那份马上就会被整份覆盖成规范形态。
 */
export function readRawForUpdate(loc: StoreLocation): Record<string, unknown> {
  for (const p of loc.read) {
    if (existsSync(p)) return readRawDocument(p)
  }
  return {}
}

/** 读某一份文件的原文（不存在 / 坏 JSON → 空对象，绝不抛错）。 */
export function readRawDocument(path: string): Record<string, unknown> {
  try {
    const v: unknown = JSON.parse(readFileSync(path, 'utf8'))
    return v && typeof v === 'object' ? (v as Record<string, unknown>) : {}
  } catch {
    return {}
  }
}

/**
 * 写整份文档。返回实际写入路径；写失败时抛出（调用方负责转成给用户看的错误）。
 *
 * ⚠️ 键序固定为「设置三键 → window」，与 Tauri 侧 `store.rs` 的 `DocOut` 字段序一致；
 *    格式对齐 `serde_json::to_string_pretty`（2 空格缩进），让两个壳写出来的文件
 *    在 diff / 人工查看时长得一样 —— 否则「同一个文件两版写出来的东西不同」会被当成 bug 排查。
 * ⚠️ 传入的 `window` 原样写回（不在这里校验），`undefined` / `null` 时不写这一段。
 */
export function writeDocument(
  loc: StoreLocation,
  doc: { settings: Settings; window?: unknown }
): string {
  // parseSettings 顺带做规范化：缺字段补默认、布尔写成字符串这类脏值也一并纠正。
  const out: Record<string, unknown> = { ...parseSettings(doc.settings) }
  if (doc.window !== undefined && doc.window !== null) out[WINDOW_KEY] = doc.window
  mkdirSync(dirname(loc.write), { recursive: true })
  // **原子替换**：先写同目录临时文件（flush 落盘）再 rename（Windows 上
  // libuv 用 MOVEFILE_REPLACE_EXISTING，目标存在也能整体替换）。直接覆盖写的风险是
  // 写一半进程被杀 / 断电，留下截断的 settings.json —— 下次启动整份解析失败，设置全丢。
  // 临时文件必须与目标同目录：跨卷 rename 会退化成非原子的复制+删除。
  const tmp = `${loc.write}.tmp-${process.pid}`
  try {
    writeFileSync(tmp, JSON.stringify(out, null, 2), { encoding: 'utf8', flush: true })
    renameSync(tmp, loc.write)
  } catch (e) {
    rmSync(tmp, { force: true })
    throw e
  }
  return loc.write
}

/**
 * 保存设置。返回实际写入路径；写失败时抛出。
 *
 * ⚠️⚠️ **读-改-写**：设置与窗口几何同住一个 `settings.json`，整体覆盖会把「挪窗口」
 * 那一方刚写进去的几何抹掉（这正是当初分成两个文件的理由；既然合并了，
 * 就必须在这里把风险挡掉）。`window-state.ts` 的 `saveWindowState` 同理。
 * ⚠️ 「读」走 [`readRawForUpdate`]（候选顺序，主位置优先）而不是写入位置：
 * 落点刚搬到数据目录时，写入位置还没有文件，老设置还在旧位置 —— 见那里的说明。
 */
export function saveSettings(loc: StoreLocation, s: Settings): string {
  const cur = readRawForUpdate(loc)
  return writeDocument(loc, { settings: s, window: cur[WINDOW_KEY] })
}

// ---------- Hotkey ↔ Electron accelerator ----------

/**
 * `KeyboardEvent.code` → Electron accelerator 的主键名。
 *
 * ⚠️ 只列**能确定映射**的键。不在表里的一律返回 null（上层提示「不支持的按键」），
 * 绝不猜 —— 猜错的话 `globalShortcut.register()` 会静默返回 false，
 * 表现是「设置里保存成功了但快捷键没用」，比直接报错难查得多。
 *
 * ⚠️ `+` 是 accelerator 的分隔符，所以符号键一律走「等号 / 逗号」这类原始字符，
 * 不能用 `Shift+Equal` 再指望它变成 `+`。
 */
const CODE_TO_KEY: Record<string, string> = {
  // 字母（用大写，accelerator 大小写不敏感）
  ...Object.fromEntries(
    Array.from({ length: 26 }, (_, i) => {
      const c = String.fromCharCode(65 + i)
      return [`Key${c}`, c]
    })
  ),
  // 主键盘数字
  ...Object.fromEntries(Array.from({ length: 10 }, (_, i) => [`Digit${i}`, String(i)])),
  // 小键盘
  ...Object.fromEntries(Array.from({ length: 10 }, (_, i) => [`Numpad${i}`, `num${i}`])),
  NumpadDecimal: 'numdec',
  NumpadAdd: 'numadd',
  NumpadSubtract: 'numsub',
  NumpadMultiply: 'nummult',
  NumpadDivide: 'numdiv',
  // 功能键
  ...Object.fromEntries(Array.from({ length: 24 }, (_, i) => [`F${i + 1}`, `F${i + 1}`])),
  // 命名键（Electron 的用词与 DOM 略有差异：Enter→Return、Escape→Escape）
  Space: 'Space',
  Enter: 'Return',
  Escape: 'Escape',
  Tab: 'Tab',
  Backspace: 'Backspace',
  Delete: 'Delete',
  Insert: 'Insert',
  Home: 'Home',
  End: 'End',
  PageUp: 'PageUp',
  PageDown: 'PageDown',
  ArrowUp: 'Up',
  ArrowDown: 'Down',
  ArrowLeft: 'Left',
  ArrowRight: 'Right',
  // 符号键（写原始字符；`\` 在 accelerator 里可直接用）
  Minus: '-',
  Equal: '=',
  BracketLeft: '[',
  BracketRight: ']',
  Backslash: '\\',
  Semicolon: ';',
  Quote: "'",
  Comma: ',',
  Period: '.',
  Slash: '/',
  Backquote: '`',
}

/**
 * 把设置里的 Hotkey 转成 Electron accelerator 字符串。
 * 返回 null 表示「这个组合不可用」：key 为空（= 未启用）或不认识的键名。
 */
export function hotkeyToAccelerator(h: Hotkey): string | null {
  const key = h.key.trim()
  if (!key) return null
  const main = CODE_TO_KEY[key]
  if (!main) return null

  const mods: string[] = []
  if (h.ctrl) mods.push('Control')
  if (h.alt) mods.push('Alt')
  if (h.shift) mods.push('Shift')
  mods.push(main)
  return mods.join('+')
}

/** 人类可读的组合键文案（日志 / 错误提示用），与设置页的 `hkLabel` 保持一致。 */
export function hotkeyLabel(h: Hotkey): string {
  const key = h.key.trim()
  if (!key) return '(未设置)'
  const pretty = key.startsWith('Digit')
    ? key.slice(5)
    : key.startsWith('Key')
      ? key.slice(3)
      : key.startsWith('Numpad')
        ? 'Num' + key.slice(6)
        : ({ ArrowUp: '↑', ArrowDown: '↓', ArrowLeft: '←', ArrowRight: '→' } as Record<string, string>)[key] || key
  const mods: string[] = []
  if (h.ctrl) mods.push('Ctrl')
  if (h.alt) mods.push('Alt')
  if (h.shift) mods.push('Shift')
  mods.push(pretty)
  return mods.join('+')
}

/** 两个 hotkey 是否等价（用于判断「快捷键有没有变」以及失败回滚）。 */
export function sameHotkey(a: Hotkey | null | undefined, b: Hotkey | null | undefined): boolean {
  if (!a || !b) return a === b
  return a.ctrl === b.ctrl && a.alt === b.alt && a.shift === b.shift && a.key.trim() === b.key.trim()
}

/** 打开设置窗口的固定快捷键（不可在设置里修改，对齐 `settings.rs::settings_open_shortcut`）。 */
export const SETTINGS_SHORTCUT = 'Shift+F1'
