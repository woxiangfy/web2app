/**
 * 应用设置（`settings.json`）—— 与 Tauri 版（`webview-app/src/settings.rs`）**共用同一份契约**。
 *
 * 契约三要素（改动任何一项都等于破坏双壳兼容，见 docs/electron-plan.md §4）：
 *   1. **字段名与嵌套结构**：`{ wake_hotkey: { ctrl, alt, shift, key }, tray_enabled, minimize_on_close }`；
 *      `key` 是 JS `KeyboardEvent.code`（`"Digit4"` / `"F1"` / `"KeyA"`）。
 *   2. **文件路径**：`%APPDATA%\<sanitize(app_name)>\settings.json`。
 *      ⚠️ Tauri 侧用的是 `app_config_dir()` 的 **父目录** 再拼自定义 app_name ——
 *      `app_config_dir()` 末段是 bundle identifier（`com.example.web2app`），
 *      父目录才是 `%APPDATA%`。所以最终路径**不含** identifier。
 *   3. **默认值全关**：不设唤醒快捷键、不启用托盘、关闭即退出。
 *
 * ⚠️⚠️ 既然要「与旧版可互相读取」，写入位置就必须落在旧版读得到的那个路径上，
 *    因此这两个小 JSON **不在 `data\` 里**（这点与「绿色版数据写 exe 同级」的
 *    总体原则不一致，是有意为之的取舍）。但窗口尺寸和 3 个开关都极小，
 *    真正体量大的 session / 缓存 / 日志仍然全在 `data\`，绿色性不受影响。
 *    —— 若 `data\` 下已经有同名文件（用户手工放的，或早期 Electron 版留下的），
 *       则以它为准并在原地写回：**用户显式放在绿色目录里的东西优先级最高**。
 */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'

import { sanitizeDirName } from './config'

export const SETTINGS_FILE = 'settings.json'

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

/**
 * 读设置。文件不存在 / 不是合法 JSON 都回落到默认值，**绝不抛错**。
 *
 * 理由：设置只是 3 个开关，为了它让应用起不来是完全错误的取舍；
 * 而 Tauri 侧（`load_settings`）同样是「解析失败即默认值」。
 */
export function loadSettings(loc: StoreLocation): LoadedSettings {
  const warnings: string[] = []
  for (const p of loc.read) {
    let text: string
    try {
      text = readFileSync(p, 'utf8')
    } catch {
      continue // 不存在（或读不动）→ 试下一个候选
    }
    try {
      return { settings: parseSettings(JSON.parse(text)), source: p, warnings }
    } catch (e) {
      // 存在但坏了：明确记一条警告再用默认值，而不是静默换文件 ——
      // 「设置莫名其妙回到默认」是最难查的一类反馈。
      warnings.push(`设置文件解析失败，已改用默认值：${p}（${(e as Error).message}）`)
      return { settings: { ...DEFAULT_SETTINGS, wake_hotkey: { ...DEFAULT_HOTKEY } }, source: p, warnings }
    }
  }
  return { settings: { ...DEFAULT_SETTINGS, wake_hotkey: { ...DEFAULT_HOTKEY } }, source: null, warnings }
}

/**
 * 写设置。返回实际写入路径；写失败时抛出（调用方负责转成给用户看的错误）。
 *
 * ⚠️ 格式对齐 `serde_json::to_string_pretty`（2 空格缩进），让两个壳写出来的文件
 * 在 diff / 人工查看时长得一样 —— 否则「同一个文件两版写出来的东西不同」会被当成 bug 排查。
 */
export function saveSettings(loc: StoreLocation, s: Settings): string {
  mkdirSync(dirname(loc.write), { recursive: true })
  writeFileSync(loc.write, JSON.stringify(s, null, 2), 'utf8')
  return loc.write
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
