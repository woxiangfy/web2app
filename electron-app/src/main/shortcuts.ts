/**
 * 全局快捷键 —— 对齐 Tauri 版（`lib.rs` 的 global_shortcut 插件 + `settings.rs`）。
 *
 * 两条：
 *   ① **唤醒主窗口**（显隐）—— 组合键取自 `settings.json`，默认**未设置**（空 key）；
 *   ② **打开设置窗口** —— 固定 `Shift+F1`，不可改。
 *
 * ⚠️⚠️ 注册失败**绝不**上抛：快捷键被别的程序占用是最常见的桌面情况，
 *    此时窗口 / 托盘 / 代理都还应该照常工作 —— 降级成「没这个快捷键」并打日志。
 *    Tauri 版同样如此（`lib.rs` 里显式写了「不能用 `?` 上抛」）。
 *
 * ⚠️ Electron 的 `globalShortcut.register()` **不抛异常**，只返回 boolean。
 *    回滚逻辑因此必须自己写：新键注册失败时要把旧键**重新注册回去**，
 *    否则用户会遇到「旧的已经注销、新的没生效」的空窗 —— 应用从此失去唤醒能力，
 *    而界面上还显示着旧快捷键（因为这次保存整体失败了）。
 */
import { globalShortcut } from 'electron'

import { SETTINGS_SHORTCUT, hotkeyLabel, hotkeyToAccelerator, sameHotkey, type Hotkey } from './settings'

/** 注册结果，供日志与设置页的报错文案使用。 */
export interface RegisterResult {
  ok: boolean
  /** 实际注册成功的 accelerator（空 = 未启用） */
  accelerator: string | null
  error?: string
}

/** 当前已注册的唤醒快捷键 accelerator（null = 未启用）。 */
let currentWakeAccel: string | null = null
/** 当前已注册的唤醒 hotkey 原值（回滚 / 变化判断用） */
let currentWakeHotkey: Hotkey | null = null

let wakeHandler: (() => void) | null = null
let settingsHandler: (() => void) | null = null

/**
 * 注册 `Shift+F1`（打开设置）。只需在启动时调一次。
 * 返回是否成功 —— 失败只打日志，不阻塞启动。
 */
export function registerSettingsShortcut(handler: () => void): boolean {
  settingsHandler = handler
  try {
    const ok = globalShortcut.register(SETTINGS_SHORTCUT, () => settingsHandler?.())
    if (!ok) {
      console.warn(`[web2app] 设置快捷键（${SETTINGS_SHORTCUT}）注册失败：可能已被其它程序占用`)
    }
    return ok
  } catch (e) {
    console.warn(`[web2app] 设置快捷键（${SETTINGS_SHORTCUT}）注册异常：${(e as Error).message}`)
    return false
  }
}

/**
 * 应用唤醒快捷键。可在设置保存时反复调用。
 *
 * 变化时的处理顺序与 `settings.rs::save_settings` 完全一致：
 *   注销旧键 → 注册新键 → 失败则把旧键注册回去并返回错误。
 */
export function applyWakeHotkey(h: Hotkey, handler: () => void): RegisterResult {
  wakeHandler = handler

  const accel = hotkeyToAccelerator(h)

  // 组合键有效但键名不认识 → 明确报错（绝不是「静默当作没设置」）
  if (h.key.trim() && accel === null) {
    return { ok: false, accelerator: null, error: `不支持的按键：${h.key}，请换一个按键` }
  }

  if (sameHotkey(currentWakeHotkey, h)) {
    return { ok: true, accelerator: currentWakeAccel }
  }

  const oldAccel = currentWakeAccel
  const oldHotkey = currentWakeHotkey

  // 注销旧键
  if (oldAccel) {
    try {
      globalShortcut.unregister(oldAccel)
    } catch {
      /* 没注册上也可能是被系统抢先释放了，忽略 */
    }
  }

  if (!accel) {
    currentWakeAccel = null
    currentWakeHotkey = { ...h, key: h.key.trim() }
    // 本来就是「没设置」，再说一遍只是噪音（启动时必然走到这条路径）
    if (oldAccel) console.log('[web2app] 唤醒快捷键已清除（未设置）')
    return { ok: true, accelerator: null }
  }

  let ok = false
  let err: string | undefined
  try {
    ok = globalShortcut.register(accel, () => wakeHandler?.())
    if (!ok) err = '可能已被其它程序占用'
  } catch (e) {
    err = (e as Error).message
  }

  if (!ok) {
    // 回滚：把旧键装回去，避免「旧的没了、新的没成」的空窗
    if (oldAccel) {
      try {
        if (globalShortcut.register(oldAccel, () => wakeHandler?.())) {
          currentWakeAccel = oldAccel
          currentWakeHotkey = oldHotkey
          console.warn(`[web2app] 唤醒快捷键 ${accel} 注册失败，已回滚到 ${oldAccel}：${err}`)
          return { ok: false, accelerator: oldAccel, error: `快捷键注册失败（${err}）` }
        }
      } catch {
        /* 回滚也失败：下面统一按「现在没有快捷键」处理 */
      }
    }
    currentWakeAccel = null
    currentWakeHotkey = oldHotkey
    console.warn(`[web2app] 唤醒快捷键 ${accel} 注册失败，当前无唤醒快捷键：${err}`)
    return { ok: false, accelerator: null, error: `快捷键注册失败（${err}）` }
  }

  currentWakeAccel = accel
  currentWakeHotkey = { ...h, key: h.key.trim() }
  console.log(`[web2app] 唤醒快捷键已注册：${hotkeyLabel(h)}（${accel}）`)
  return { ok: true, accelerator: accel }
}

/** 当前生效的 accelerator（探针 / 日志用）。 */
export function currentWakeAccelerator(): string | null {
  return currentWakeAccel
}

/** 退出前释放全部快捷键。不释放的话，进程退出后系统里可能残留占用（Windows 上一般随进程释放，但不保证）。 */
export function releaseShortcuts(): void {
  try {
    globalShortcut.unregisterAll()
  } catch {
    /* 退出路径，忽略 */
  }
  currentWakeAccel = null
  currentWakeHotkey = null
}
