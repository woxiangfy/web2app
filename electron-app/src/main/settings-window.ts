/**
 * 设置窗口 —— 对齐 Tauri 版（`webview-app/src/settings.rs`）。
 *
 * 窗口参数逐项照抄 `open_settings_window()`：620×600 内容区、可缩放（最小 460×400）、
 * 居中、标题 `设置 - <APP_NAME>`。
 *
 * ⚠️⚠️ 设置页 HTML **不重写**，直接用 `webview-app/dist/settings.html`（单一真源）。
 *    那份 HTML 是写给 Tauri 的，调用的是 `window.__TAURI_INTERNALS__.invoke(...)`。
 *    这边的做法是在设置窗的 preload 里**提供一个同名垫片**，把三个命令映射到 IPC ——
 *    而不是去改 HTML 让两个壳各认一套 API。理由和注入脚本一样：
 *    界面一旦分叉，两个壳的设置页就会各修各的 bug，这正是不想看到的。
 *    见 `src/renderer/settings-preload.ts`。
 *
 * ⚠️ 设置窗是**无边框窗**（`frame: false`，对齐 Tauri 版改的 `decorations(false)`）：
 *    用户要求的版式是「没有顶部的窗体、面板居中、内容多可滚动」—— 标题与关闭按钮由
 *    `dist/settings.html` 自绘在面板头部（`-webkit-app-region:drag` 负责拖动，
 *    `frame:false` 下 Chromium 才会把 drag 区当标题栏处理）。
 *    窗口本身居中显示（`center()`），底色取与页面底衬同一个浅灰，避免出现白边。
 */
import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { app, BrowserWindow, ipcMain, type BrowserWindow as BW } from 'electron'

import type { Settings } from './settings'

export const SETTINGS_URL = 'settings://localhost/'

/** 设置窗内容区尺寸（与 Tauri 版 `inner_size(620, 600)` 一致）。 */
const SETTINGS_SIZE = { width: 620, height: 600 }
/** 设置窗最小尺寸（与 Tauri 版 `min_inner_size(460, 400)` 一致）。 */
const SETTINGS_MIN_SIZE = { width: 460, height: 400 }
/** 窗口底色 = 设置页的底衬（`--backdrop` 浅色值），无边框窗四周露出的那一圈就是它。 */
const SETTINGS_BG = '#eceef1'

/**
 * 定位设置页 HTML。
 *
 * 顺序：打包副本（app.asar/out/inject/settings.html，2026-09-26 起注入资源随 asar 分发，
 *   不再放 resources/inject/）→ 开发态的 `webview-app/dist/`。
 * 与 `index.ts::injectDir()` 同思路：打包后是 asar 内拷贝，开发时直接引用原文件，绝不复制第二份。
 * ⚠️ 用 `app.isPackaged` 区分而非 `existsSync`：stage 会在开发目录生成 out/inject/，若按
 *    「存在即打包副本」判，开发态会误读 stage 副本、失去单一真源（与 injectDir 同理）。
 */
export function locateSettingsHtml(appPath: string): string | null {
  const candidates = app.isPackaged
    ? [join(appPath, 'out', 'inject', 'settings.html')]
    : [join(appPath, '..', 'webview-app', 'dist', 'settings.html'), join(appPath, 'dist', 'settings.html')]
  for (const c of candidates) {
    if (existsSync(c)) return c
  }
  return null
}

export function readSettingsHtml(appPath: string): string | null {
  const p = locateSettingsHtml(appPath)
  if (!p) return null
  try {
    return readFileSync(p, 'utf8')
  } catch {
    return null
  }
}

/** 兜底设置页：只在 HTML 文件缺失时使用，让用户至少能改设置，而不是面对一个空白窗。 */
const FALLBACK_HTML = `<!doctype html><html lang="zh-CN"><head><meta charset="utf-8">
<title>设置</title><style>
body{font:13px/1.6 system-ui,"Microsoft YaHei",sans-serif;margin:0;padding:22px 24px;color:#1f2937}
@media (prefers-color-scheme:dark){body{background:#18181b;color:#f4f4f5}}
label{display:flex;gap:10px;align-items:center;margin:10px 0}
#s{margin-top:14px;font-size:12px}
</style></head><body>
<h1>设置</h1>
<p>未能读到设置页资源（settings.html），以下为精简版设置。</p>
<label><input type="checkbox" id="tray"/> 启用系统托盘图标</label>
<label><input type="checkbox" id="min"/> 关闭窗口时最小化到托盘</label>
<button id="save">保存</button>
<div id="s"></div>
<script>
var inv=function(c,a){return window.__TAURI_INTERNALS__.invoke(c,a)};
inv('get_settings').then(function(s){document.getElementById('tray').checked=s.tray_enabled;
document.getElementById('min').checked=s.minimize_on_close}).catch(function(e){document.getElementById('s').textContent='读取失败：'+e});
document.getElementById('save').onclick=function(){
 inv('save_settings',{newSettings:{wake_hotkey:{ctrl:false,alt:false,shift:false,key:''},
 tray_enabled:document.getElementById('tray').checked,
 minimize_on_close:document.getElementById('min').checked}})
 .then(function(){document.getElementById('s').textContent='已保存'})
 .catch(function(e){document.getElementById('s').textContent='保存失败：'+e})};
</script></body></html>`

export interface SettingsWindowDeps {
  /** 当前设置（内存态） */
  getSettings: () => Settings
  /**
   * 保存设置：校验 → 应用运行时副作用（快捷键 / 托盘）→ 持久化。
   * 抛异常表示失败，异常消息会原样显示在设置页上。
   */
  saveSettings: (next: Settings) => void
  /**
   * 清除全部浏览数据（缓存 / Cookie / 本地存储 …）。返回给设置页展示的一句话；
   * 抛异常表示失败。实现见 `web-data.ts` —— 与 Tauri 版的 `clear_web_data` 对齐。
   */
  clearWebData: () => Promise<string>
  /** 设置窗用的 preload 绝对路径 */
  preloadPath: string
  /** 设置页 HTML 源码；null 时用兜底页 */
  readHtml: () => string | null
}

let settingsWin: BW | null = null

/**
 * 注册设置相关的 IPC（命令名与 Tauri 一致，去掉前缀后即 `get_settings` 等）。
 *
 * ⚠️ 全部用 `handle`（返回 Promise）：设置页靠 `await` 拿结果，用 `on` 的话
 *    保存失败在界面上完全无声 —— 与主窗口按钮那条教训是同一个。
 *
 * ⚠️ 命令名必须与 `webview-app/dist/settings.html` 里 `invoke(...)` 的字面量逐字一致，
 *    且要同时加进 `settings-preload.ts` 的 ALLOWED 白名单 —— 三处任一漏改，
 *    表现都是「按钮点了没反应 / 报未授权命令」，而页面本身看不出任何异常。
 */
export function registerSettingsIpc(deps: SettingsWindowDeps): void {
  ipcMain.handle('tauri:get_settings', () => deps.getSettings())

  ipcMain.handle('tauri:save_settings', (_e, args: { newSettings?: unknown } | undefined) => {
    const raw = args?.newSettings
    if (!raw || typeof raw !== 'object') throw new Error('参数错误：缺少 newSettings')
    deps.saveSettings(normalizeIncoming(raw as Record<string, unknown>))
    return null
  })

  ipcMain.handle('tauri:clear_web_data', () => deps.clearWebData())

  ipcMain.handle('tauri:close_settings_window', () => {
    closeSettingsWindow()
    return null
  })
}

/**
 * 设置页是**未信任输入**吗？不是 —— 但也不能直接信。
 * 这里只做「结构收敛」：不认识的字段丢掉，类型不对的回落默认值，
 * 避免页面上传进来的怪东西把 settings.json 写坏（写坏了下次启动两个壳都读不出来）。
 */
function normalizeIncoming(o: Record<string, unknown>): Settings {
  const hk = (o.wake_hotkey ?? {}) as Record<string, unknown>
  const str = (v: unknown): string => (typeof v === 'string' ? v : '')
  const bool = (v: unknown): boolean => v === true
  return {
    wake_hotkey: {
      ctrl: bool(hk.ctrl),
      alt: bool(hk.alt),
      shift: bool(hk.shift),
      key: str(hk.key).trim(),
    },
    tray_enabled: bool(o.tray_enabled),
    minimize_on_close: bool(o.minimize_on_close),
  }
}

/** 打开（或聚焦）设置窗。已存在时只做 show/unminimize/focus，与旧版一致。 */
export function openSettingsWindow(deps: Pick<SettingsWindowDeps, 'preloadPath' | 'readHtml'>): void {
  if (settingsWin && !settingsWin.isDestroyed()) {
    if (settingsWin.isMinimized()) settingsWin.restore()
    settingsWin.show()
    settingsWin.focus()
    return
  }

  const win = new BrowserWindow({
    // useContentSize：Tauri 的 inner_size(620,600) 说的是**内容区**。
    // 无边框窗下两者本来就相等，写上是为了与 Tauri 侧同一个语义、将来加回边框也不会错。
    useContentSize: true,
    width: SETTINGS_SIZE.width,
    height: SETTINGS_SIZE.height,
    minWidth: SETTINGS_MIN_SIZE.width,
    minHeight: SETTINGS_MIN_SIZE.height,
    resizable: true,
    minimizable: true,
    maximizable: true,
    fullscreenable: false,
    // 无边框：没有系统标题栏，标题 / 关闭 / 拖动都由设置页自绘（见文件头）。
    frame: false,
    backgroundColor: SETTINGS_BG,
    show: false,
    webPreferences: {
      preload: deps.preloadPath,
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      webviewTag: false,
    },
  })

  settingsWin = win
  win.once('ready-to-show', () => {
    // 居中放在 show 之前：先 show 再 center 会看到窗口跳一下。
    win.center()
    win.show()
  })
  win.on('closed', () => {
    if (settingsWin === win) settingsWin = null
  })

  void win.loadURL(SETTINGS_URL).catch((e: Error) => {
    console.error(`[web2app] 设置窗加载失败：${e.message}`)
  })
}

/** 关闭设置窗（幂等）。 */
export function closeSettingsWindow(): void {
  const w = settingsWin
  settingsWin = null
  if (!w || w.isDestroyed()) return
  w.close()
}

/** 是否存在设置窗（探针断言用）。 */
export function getSettingsWindow(): BW | null {
  return settingsWin && !settingsWin.isDestroyed() ? settingsWin : null
}

export { FALLBACK_HTML }
