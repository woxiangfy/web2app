/**
 * 唤醒快捷键家族：类型 / 与 Electron `globalShortcut` accelerator 的互转 / 人类可读文案。
 *
 * 这是**壳无关的纯规则模块**（零 Electron 导入）：
 *   - 持久化形态（`settings.json` 里的 `wake_hotkey`）的宽容解析仍归 `settings.ts::parseHotkey`，
 *     本文件只拥有「解释一个已解析好的 Hotkey」的规则；
 *   - 可读文案 `hotkeyLabel()` 是跨壳契约的一部分 —— Rust 侧 `settings.rs::hotkey_label`
 *     必须与这里逐字对齐（2026-10-03 已把美化规则同步到 Rust）。
 *
 * `key` 一律存 JS `KeyboardEvent.code`（`"Digit4"` / `"KeyA"` / `"F1"`）。
 */

/** 唤醒快捷键结构。`key` 为 JS `KeyboardEvent.code`，空串表示「不启用」。 */
export interface Hotkey {
  ctrl: boolean
  alt: boolean
  shift: boolean
  key: string
}

export const DEFAULT_HOTKEY: Hotkey = { ctrl: false, alt: false, shift: false, key: '' }

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
