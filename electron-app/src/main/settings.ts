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
 * 契约四要素（改动任何一项都等于破坏双壳兼容，见 docs/electron-plan.md §4）：
 *   1. **字段名与嵌套结构**：`{ wake_hotkey: { ctrl, alt, shift, key }, tray_enabled, minimize_on_close }`；
 *      `key` 是 JS `KeyboardEvent.code`（`"Digit4"` / `"F1"` / `"KeyA"`）。
 *   2. **文件路径**：`%APPDATA%\<sanitize(app_name)>\settings.json`。
 *      ⚠️ Tauri 侧用的是 `app_config_dir()` 的 **父目录** 再拼自定义 app_name ——
 *      `app_config_dir()` 末段是 bundle identifier（`com.example.web2app`），
 *      父目录才是 `%APPDATA%`。所以最终路径**不含** identifier。
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
 * ⚠️⚠️ 既然要「与旧版可互相读取」，写入位置就必须落在旧版读得到的那个路径上，
 *    因此这个小 JSON **不在 `data\` 里**（这点与「绿色版数据写 exe 同级」的
 *    总体原则不一致，是有意为之的取舍）。但文档极小（< 1 KB），
 *    真正体量大的 session / 缓存 / 日志仍然全在 `data\`，绿色性不受影响。
 *    —— 若 `data\` 下已经有同名文件（用户手工放的，或早期 Electron 版留下的），
 *       则以它为准并在原地写回：**用户显式放在绿色目录里的东西优先级最高**。
 */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
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
  /** 绿色数据目录（exe 同级 `data\`，或 `%APPDATA%\<app_name>`） */
  dataDir: string
  /** `app.getPath('appData')`，即 `%APPDATA%` */
  appDataDir: string
  /** config.json 的 app_name（会做 Windows 目录名清洗） */
  appName: string
  /** 文件名，如 `settings.json` */
  file: string
}

/**
 * 解析存储位置。
 *
 * 优先级：`<dataDir>\<file>`（绿色目录，用户显式放置的东西）→
 *         `%APPDATA%\<app_name>\<file>`（旧版 Tauri 的位置）。
 * 写入：跟读取的**同一个**位置 —— 保证「读哪写哪」，不会出现两个副本各自漂移。
 */
export function storeLocation(opts: StoreLocationOptions): StoreLocation {
  const legacy = join(opts.appDataDir, sanitizeDirName(opts.appName), opts.file)
  const local = join(opts.dataDir, opts.file)

  if (local === legacy) return { read: [legacy], write: legacy }

  return {
    read: [local, legacy],
    write: existsSync(local) ? local : legacy,
  }
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
 * 读**写入位置**那份文档的原文（读-改-写的「读」必须认准写入位置，不能顺着读取候选列表走
 * —— 否则会拿着 `data\` 里那份的内容去覆盖 `%APPDATA%` 里的文件）。
 * 文件不存在 / 坏 JSON 一律给空对象：坏的那份马上就会被整份覆盖成规范形态。
 */
export function readRawDocument(writePath: string): Record<string, unknown> {
  try {
    const v: unknown = JSON.parse(readFileSync(writePath, 'utf8'))
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
  writeFileSync(loc.write, JSON.stringify(out, null, 2), 'utf8')
  return loc.write
}

/**
 * 保存设置。返回实际写入路径；写失败时抛出。
 *
 * ⚠️⚠️ **读-改-写**：设置与窗口几何同住一个 `settings.json`，整体覆盖会把「挪窗口」
 * 那一方刚写进去的几何抹掉（这正是当初分成两个文件的理由；既然合并了，
 * 就必须在这里把风险挡掉）。`window-state.ts` 的 `saveWindowState` 同理。
 */
export function saveSettings(loc: StoreLocation, s: Settings): string {
  const cur = readRawDocument(loc.write)
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

/** accelerator 主键名 → `KeyboardEvent.code`（设置页的「下拉选择」用不着，但反向断言用得上）。 */
export function isSupportedHotkeyKey(code: string): boolean {
  return Object.prototype.hasOwnProperty.call(CODE_TO_KEY, code)
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
