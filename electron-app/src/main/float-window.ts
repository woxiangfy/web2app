/**
 * 桌面悬浮球 —— Electron 侧实现（仅 Windows），与 Tauri 侧
 * `webview-app/src/floatwin.rs` 逐条对齐。
 *
 * v1.69 需求（纠正 v1.67 的错误方向；v1.68 只修了旧「小窗」的三条静默失败）：
 * 一个独立、置顶、透明、不占任务栏的 **56×56 小球**，
 * 加载的是内置本地页 `floaticon://localhost/`（双壳单一真源 floaticon.html；球内软件图标
 * 经同一协议 `/icon.png` 分发），**不加载任何网页**：
 *   ① 单击球 → **切换**【原来的主窗口】显隐（v1.71：可见且非最小化 → 隐藏；否则显示并
 *      聚焦，deps.toggleMainWindow；此前只「显示」，主窗可见时点球没有反应）；
 *   ② 可随时显示 / 隐藏；设置页开关创建 / 销毁；
 *   ③ 按住球可拖动；
 *   ④ 拖到屏幕左 / 右缘「半隐藏」吸附（只露半球），悬停展开、移开缩回；
 *   ⑤ 右键球 → **关闭悬浮图标功能并唤回主窗口**（v1.76）。通道本身不做事：「关功能
 *      （`float_enabled` 置 false 并落盘）+ 销毁球窗 + 显示主窗」三件事由入口经
 *      `onHideRequest` 注入 —— 与 Tauri 侧 `floatwin::disable_float_from_icon` 逐条对齐。
 *      ⚠️ 不能只把球藏起来：球是**唯一**可见入口时（没启用托盘 + 关窗驻留），一藏就是
 *      「进程还在跑、桌面上却再也唤不出窗口」（2026-10-11 使用者实测）。
 *      托盘菜单的「悬浮图标」项仍是临时显隐 toggle，走 `hideFloatWindow`，两者语义不同。
 *
 * 它不是第二个浏览器窗：没有导航、没有远程内容、不碰 bootstrap / sidecar / 数据目录。
 *
 * 坐标系：Electron 的窗口 bounds 与 display.bounds 都是 **DIP（逻辑像素）**，floaticon.html
 * 上报的是指针【屏幕绝对坐标】screenX/screenY（CSS 像素，与 DIP 1:1；v1.70 起不再上报
 * 增量 —— 窗口每帧 setBounds 会污染下一帧增量），所以边缘决策全部走 `float-layout.ts`
 * 的逻辑常量；只有落盘 / 恢复时用 screen.dipToScreenRect / screenToDipRect 换算物理像素
 * （settings.json 的 `float_window` 段与 Tauri 同形，单位是物理像素）。
 *
 * 安全模型：floaticon.html 只经 float-preload 的 contextBridge 白名单调用 `float:*` IPC；
 * 本模块对每条特权 IPC 都校验 sender === 悬浮球自己的 webContents，任何其它窗口 / 内容
 * 视图发起一律拒绝。
 */
import { BrowserWindow, ipcMain, screen } from 'electron'

import {
  FLOAT_WINDOW_KEY,
  readRawForUpdate,
  parseSettings,
  writeDocument,
  type StoreLocation,
} from './settings'
import {
  circleShapeRects,
  clampFreeInside,
  clampVisible,
  clampY,
  COLLAPSE_DELAY_MS,
  collapsedX,
  decideEdge,
  DEFAULT_FLOAT_H,
  DEFAULT_FLOAT_W,
  expandedX,
  geomToSegment,
  isUsablePhysicalGeom,
  MARGIN_LOGICAL,
  MIN_VISIBLE_LOGICAL,
  parseFloatGeometry,
  PEEK_LOGICAL,
  pointInRect,
  rectAt,
  SNAP_LOGICAL,
  type FloatEdge,
  type FloatGeom,
  type Rect,
} from './float-layout'

/** 一次吸附的运行期状态：吸附边 + 所在显示器 DIP 边界 + 当前是否处于半隐藏（缩回）态。 */
interface Dock {
  edge: Exclude<FloatEdge, 'none'>
  mon: Rect
  collapsed: boolean
}

export interface FloatWindowDeps {
  /** 窗口标题（config.json 的 title；同时是透明球窗的可访问名）。 */
  title: string
  /** float-preload.js 的绝对路径。 */
  preloadPath: string
  /** settings.json 的读-改-写位置（getter：探针模式会改到临时目录）。 */
  getStoreLoc: () => StoreLocation
  /**
   * 单击球时**切换**【原来的主窗口】显隐（可见且非最小化 → 隐藏；否则恢复并聚焦；
   * 多页签时显示分支含 tab 可见性同步 / 空壳自愈）。由 index.ts 注入，float-window
   * 不直接持有主窗引用，避免循环依赖。
   */
  toggleMainWindow: () => void
  /**
   * 球上**右键**（v1.76）：关闭悬浮图标功能 + 唤回主窗口。
   *
   * ⚠️ 不是「把球藏起来」，而是三件事（由入口注入，本模块不持设置 / 主窗引用）：
   *   ① `float_enabled` 置 false 并落盘（含同步已打开的设置页复选框）；
   *   ② 销毁球窗；③ 把主窗口显示出来（若此前被隐藏 / 最小化）。
   * 只藏球会留下「进程还在跑、桌面上却再也唤不出窗口」的死局（2026-10-11 使用者实测）。
   * 与 Tauri 侧 `floatwin::disable_float_from_icon` 逐条对齐。
   */
  onHideRequest: () => void
  /**
   * 应用是否正在退出（`before-quit` 之后为 true）。
   * ⚠️⚠️ 不可省：本模块的 close 默认转「隐藏」（`e.preventDefault()`），而 Electron 里
   * **窗口的 close 一旦被 prevent，正在进行的 `app.quit()` 会被整体取消** ——
   * 主窗已关、球窗只剩隐藏、进程却活着（实测复现）。退出路径必须显式放行。
   */
  isQuitting: () => boolean
}

let deps: FloatWindowDeps | null = null
/** 当前悬浮球窗（隐藏时仍存在；destroy 后置 null）。 */
let floatWin: BrowserWindow | null = null
let dock: Dock | null = null
/** 指针是否正在拖动（拖动期间忽略 peekleave 的缩回）。 */
let moving = false
/**
 * pointerdown 时指针的屏幕坐标（DIP）。只暂存、不展开 / 不置 moving；
 * 第一帧 move 取走：先展开（若半隐藏），再用「按下点 − 展开后窗口原点」算 anchor。
 * 不能在首帧 move 才用光标记锚点（首帧光标已离手，窗口会纹丝不动）。
 */
let press: { x: number; y: number } | null = null
/**
 * 本次拖动的抓取点偏移（按下点屏幕坐标 − 展开后窗口左上角，均 DIP）。第一帧 move 时
 * 由 press 计算，之后每帧 setBounds(光标 − 偏移)。v1.69 旧实现让页面上报 clientX
 * 增量，而窗口每帧都在 setBounds，下一帧增量被窗口自身位移抵消，球半速跟手 / 抖动。
 */
let anchor: { x: number; y: number } | null = null
/** 延迟缩回的定时器；peekenter / 隐藏 / 销毁 / 起拖都会取消。 */
let collapseTimer: NodeJS.Timeout | null = null
/** 悬停「代际」：每次展开 / 排队缩回 / 隐藏 / 销毁都自增，到期任务只认自己那一代。 */
let peekGen = 0
/** true = close 是真的销毁（退出 / 关开关）；否则 close 一律转成隐藏。 */
let allowClose = false
let registered = false

/** 取消排队中的延迟缩回（若有）并进入新一代悬停。 */
function cancelPendingCollapse(): void {
  peekGen += 1
  if (collapseTimer) {
    clearTimeout(collapseTimer)
    collapseTimer = null
  }
}

/** 清空一次按压 / 拖动的运行期状态（按下点 / 抓取偏移 / moving）；不动吸附态与几何。 */
function clearDragRuntime(): void {
  moving = false
  press = null
  anchor = null
}

// ===========================================================================
// 注册（启动一次）
// ===========================================================================

/** 注册悬浮球 IPC + 保存 deps。进程内幂等，只注册一次。 */
export function registerFloatWindow(d: FloatWindowDeps): void {
  deps = d
  if (registered) return
  registered = true

  // sx/sy 是指针的屏幕绝对坐标（DIP，来自页面 screenX/screenY），不是增量。
  ipcMain.handle('web2app:float-down', (e, sx: unknown, sy: unknown) => {
    assertFloatSender(e.sender.id)
    const n = (v: unknown): number => (typeof v === 'number' && Number.isFinite(v) ? v : 0)
    onDown(n(sx), n(sy))
    return null
  })
  ipcMain.handle('web2app:float-move', (e, sx: unknown, sy: unknown) => {
    assertFloatSender(e.sender.id)
    const n = (v: unknown): number => (typeof v === 'number' && Number.isFinite(v) ? v : 0)
    onMove(n(sx), n(sy))
    return null
  })
  ipcMain.handle('web2app:float-cancel', (e) => {
    assertFloatSender(e.sender.id)
    // 系统打断且未拖动：只复位，不弹主窗、不做吸附判定。
    clearDragRuntime()
    return null
  })
  ipcMain.handle('web2app:float-drag-end', (e) => {
    assertFloatSender(e.sender.id)
    onDragEnd()
    return null
  })
  ipcMain.handle('web2app:float-peek', (e, enter: unknown) => {
    assertFloatSender(e.sender.id)
    if (enter) onPeekEnter()
    else onPeekLeave()
    return null
  })
  // 右键球 = **关闭悬浮图标功能并唤回主窗口**（v1.76，见 FloatWindowDeps.onHideRequest）。
  // ⚠️ 通道只做转发，三件事（关功能 / 销毁球窗 / 显示主窗）由入口注入 —— 与 Tauri 侧
  //    `floatwin::disable_float_from_icon` 逐条对齐。不要再退回 `hideFloatWindow()`
  //    （那是托盘 toggle 用的「临时收起」，只藏球会留下无入口的死局）。
  ipcMain.handle('web2app:float-hide', (e) => {
    assertFloatSender(e.sender.id)
    deps?.onHideRequest()
    return null
  })
  // 单击球：切换【原来的主窗口】显隐（v1.71：v1.68 起只「显示」，主窗可见时点击无反应）；
  // 同时结束未达拖动阈值的按压，清掉 press/moving，避免残留状态让后续悬停缩回被
  // 「拖动中」早退永久挡掉。
  ipcMain.handle('web2app:float-click', (e) => {
    assertFloatSender(e.sender.id)
    clearDragRuntime()
    deps?.toggleMainWindow()
    return null
  })
}

/** 特权 IPC 闸门：只有悬浮球窗自己的 webContents 能驱动它（仿 app-ipc.ts 的 assertShell）。 */
function assertFloatSender(senderId: number): void {
  if (!floatWin || floatWin.isDestroyed() || senderId !== floatWin.webContents.id) {
    console.warn(`[web2app] 拒绝非悬浮球窗发起的 float IPC（sender.id=${senderId}）`)
    throw new Error('拒绝：仅悬浮球窗可操作悬浮球')
  }
}

// ===========================================================================
// 对外入口（设置开关 / 设置页按钮 / 托盘 / 启动 / 退出 调用）
// ===========================================================================

/** 悬浮球当前是否存活（探针 / 日志用；隐藏也算存活）。 */
export function isFloatAlive(): boolean {
  return !!floatWin && !floatWin.isDestroyed()
}

/**
 * 显示悬浮球：已存在则显现（吸附态先缩回成半球）；不存在则建窗。
 * 非 Windows 为刻意空操作（设置是跨平台资源，开关仍会持久化）。
 */
export function showFloatWindow(): void {
  if (process.platform !== 'win32' || !deps) return
  const w = floatWin
  if (w && !w.isDestroyed()) {
    // 用户主动唤球：排队中的延迟缩回一律作废，不能 300ms 后自己又缩回去。
    cancelPendingCollapse()
    w.show()
    w.restore()
    w.focus()
    // 重新显现时若处于吸附态，先缩回半球（悬停再展开），避免整球突然挡在屏幕边。
    if (dock && !dock.collapsed) collapse(dock)
    return
  }
  createFloatWindow()
}

/** 彻底销毁悬浮球（设置里关掉开关时调用）；几何已随拖动 / 退出落盘，下次开启原位恢复。 */
export function destroyFloatWindow(): void {
  cancelPendingCollapse()
  if (!floatWin || floatWin.isDestroyed()) {
    dock = null
    clearDragRuntime()
    return
  }
  persistGeometry(floatWin)
  allowClose = true
  floatWin.close()
  dock = null
  clearDragRuntime()
}

/**
 * 隐藏（不销毁）球窗 —— **只服务托盘菜单「悬浮图标」项的显隐 toggle** 与本窗 close 事件兜底。
 *
 * ⚠️ **球上右键不走这里**（v1.76 起走 `onHideRequest`：关功能 + 销毁球窗 + 唤回主窗）。
 * 右键在用户直觉里是「我不要这个球了」，而只藏球会留下「进程还在跑、桌面上却再也唤不出
 * 窗口」的死局（2026-10-11 使用者实测）；托盘那条路径不会死局 —— 有托盘就还有别的入口。
 */
export function hideFloatWindow(): void {
  const w = floatWin
  if (!w || w.isDestroyed()) return
  // 吸附态 dock 保留（再显现时恢复半球）；只取消一次性的悬停 / 拖动运行期状态。
  cancelPendingCollapse()
  persistGeometry(w)
  w.hide()
  // 收窗即清拖动标志：拖动途中被隐藏时 pointerup 不会再来，
  // 残留的 moving=true 会让下一次悬停展开 / 移开缩回永久失效（事件被「拖动中」早退挡掉）。
  clearDragRuntime()
}

/** 托盘菜单用的显隐切换：可见则隐藏，否则显示。 */
export function toggleFloatWindow(): void {
  const w = floatWin
  if (w && !w.isDestroyed() && w.isVisible()) hideFloatWindow()
  else showFloatWindow()
}

/** 退出前持久化（app 'will-quit' 调用）。 */
export function persistFloatGeometry(): void {
  const w = floatWin
  if (w && !w.isDestroyed()) persistGeometry(w)
}

// ===========================================================================
// 建窗
// ===========================================================================

function createFloatWindow(): void {
  if (!deps) return
  const saved = readSavedGeom()

  const win = new BrowserWindow({
    width: DEFAULT_FLOAT_W,
    height: DEFAULT_FLOAT_H,
    frame: false,
    // thickFrame 默认 true 会给无边框窗保留一圈 Windows 隐形缩放边框（实测外框
    // 59×58 而非 56×56），所有几何（吸附 / 半球 / 钳制）都会被这几像素带偏；球不需要缩放。
    thickFrame: false,
    // 透明是「圆形球浮在桌面上」的前提：窗口矩形透明，只有 CSS 圆形球可见。
    transparent: true,
    resizable: false,
    minimizable: false,
    maximizable: false,
    fullscreenable: false,
    // 不占任务栏：悬浮球是附属快捷入口，入口在设置页 / 托盘。
    skipTaskbar: true,
    // 置顶：悬浮球的意义就是始终浮在其它窗口之上。
    alwaysOnTop: true,
    show: false,
    title: deps.title,
    webPreferences: {
      preload: deps.preloadPath,
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      webviewTag: false,
    },
  })
  floatWin = win
  dock = null
  clearDragRuntime()
  collapseTimer = null
  peekGen = 0
  allowClose = false

  // 把窗口的**系统区域**裁成圆（`setShape` 只接受矩形数组，见 float-layout.circleShapeRects）：
  // `transparent: true` 只让四角「看不见」，却仍然「点得着」—— 不裁的话球四角会把桌面 /
  // 下层窗口的点击吃掉。区域只决定**命中**，视觉仍由 CSS 圆 + transparent 负责。
  // ⚠️ 区域与球严格同尺寸、绝不外扩：Electron 的 setShape 在 NOREDIRECTIONBITMAP 合成
  //    路径下不裁视觉（CSS 软边 / 外阴影天然光滑），外扩只会让球外环带重新吃掉桌面点击。
  //    v1.80 起 Tauri 同走真 alpha，命中改由 WM_NCHITTEST 圆形穿透（不再 SetWindowRgn、
  //    不再页面黑底）；两壳视觉 / 命中分离的架构一致，见 floaticon.html 文件头存档。
  // ⚠️ `setShape` 在 Electron 里标记 `@experimental`（win32 / linux）⇒ 失败**不能**让球不可用，
  //    只记日志（退化成「四角挡点击」的旧行为，其余一切照常）。
  try {
    win.setShape(circleShapeRects(DEFAULT_FLOAT_W))
  } catch (e) {
    console.warn(`[web2app] 悬浮球圆形区域设置失败（四角仍会挡点击）：${(e as Error).message}`)
  }

  // 加载内置本地球页（无导航、无远程内容）。与 Tauri 完全同 URL：
  // floaticon://localhost/（处理器在 protocols.ts，页面与 /icon.png 都只来自内置资源）；
  // 加载失败只记日志，绝不回退到任何远程 URL。
  win.loadURL('floaticon://localhost/').catch((e: Error) => {
    console.warn(`[web2app] 悬浮球页加载失败：${e.message}`)
  })

  // 先定位再显现：show 放在 ready-to-show，透明窗在首帧绘制前 show 在 Windows 上常见
  // 黑块 / 不透明矩形；这里先定位，显现时即在正确位置，不会从角落滑入。
  applyInitialGeometry(win, saved)
  let shown = false
  const reveal = (): void => {
    if (shown || win.isDestroyed()) return
    shown = true
    win.show()
    pushState(dock)
  }
  win.once('ready-to-show', reveal)
  // 兜底：个别环境 ready-to-show 可能不来（极旧显卡 / 远程桌面），1s 后强制显现，
  // 宁可轻微闪一下也不能「勾选了但球永远不出现」。
  const revealFallback = setTimeout(reveal, 1000)
  win.once('closed', () => clearTimeout(revealFallback))

  // 本地页加载完成后把当前 Dock 推一次（executeJavaScript 落在主框架，幂等）。
  win.webContents.on('did-finish-load', () => {
    if (floatWin === win) {
      // ready-to-show 已先 show 的情况这里只补状态；尚未 show（理论上不会）则代为显现。
      if (!shown) reveal()
      pushState(dock)
    }
  })

  // close 语义：右键隐藏 / 系统关闭都转成「隐藏不销毁」；只有 destroy / 退出才真关。
  // ⚠️⚠️ 退出必须放行（`isQuitting()`）：close 里 preventDefault 会**取消整个 app.quit()**，
  //    表现是「开着悬浮球就退不掉应用」——主窗关了、球窗只剩隐藏、进程仍在（实测复现）。
  //    `allowClose` 只覆盖「设置里关掉开关 / 主动销毁」，管不到 app 级退出。
  win.on('close', (e) => {
    if (allowClose || deps?.isQuitting()) {
      persistGeometry(win)
      return
    }
    e.preventDefault()
    hideFloatWindow()
  })
  win.on('closed', () => {
    if (collapseTimer) {
      clearTimeout(collapseTimer)
      collapseTimer = null
    }
    if (floatWin === win) {
      floatWin = null
      dock = null
      clearDragRuntime()
    }
  })
}

/** 读 settings.json 的 float_window 段（宽容解析；坏 / 无返回 null）。 */
function readSavedGeom(): FloatGeom | null {
  if (!deps) return null
  try {
    const raw = readRawForUpdate(deps.getStoreLoc())[FLOAT_WINDOW_KEY]
    const g = parseFloatGeometry(raw)
    return g && isUsablePhysicalGeom(g) ? g : null
  } catch {
    return null
  }
}

/**
 * 悬浮球**唯一**定位入口：始终携带完整且固定的 56×56 尺寸做 setBounds。
 *
 * 为什么不允许只传 {x,y} 的部分字段更新：Windows 上 frameless + transparent 窗口在
 * 出屏 / 负坐标移动后，部分字段 setBounds 会让 getBounds() 报出的宽逐次漂移
 * （合成指针 harness 实测 56→58→60），几何公式再盲信 b.width，半球落点就会
 * 越带越偏（期望 -28 → 实际 -30 → -32）。每次都带完整固定尺寸，既把可能发生的
 * 漂移当场纠正，也让几何与 Tauri 侧 live_px 固定常量语义一致。x/y 取整到 DIP。
 */
function placeFloat(win: BrowserWindow, x: number, y: number): void {
  win.setBounds({
    x: Math.round(x),
    y: Math.round(y),
    width: DEFAULT_FLOAT_W,
    height: DEFAULT_FLOAT_H,
  })
}

/**
 * 建窗后摆放：优先恢复保存的几何（含吸附态）；失效则主显示器右上角默认摆放。
 * 球窗尺寸固定 56×56（DIP），保存的 width/height 只用于校验段是否可信，不用来改尺寸。
 */
function applyInitialGeometry(win: BrowserWindow, saved: FloatGeom | null): void {
  if (saved) {
    // 落盘是物理像素，先换算回 DIP（win 已建，可提供该屏 DPI）。
    const dip = screen.screenToDipRect(win, {
      x: Math.round(saved.x),
      y: Math.round(saved.y),
      width: Math.round(saved.width),
      height: Math.round(saved.height),
    })
    const center = {
      x: Math.round(dip.x + DEFAULT_FLOAT_W / 2),
      y: Math.round(dip.y + DEFAULT_FLOAT_H / 2),
    }
    // 中心必须落在某个显示器上（拔过外接屏的旧坐标不该把球恢复到屏外）。
    const m = screen.getDisplayNearestPoint(center)
    if (pointInRect(center, m.bounds)) {
      const y = clampY(Math.round(dip.y), DEFAULT_FLOAT_H, m.bounds)
      if (saved.edge === 'left' || saved.edge === 'right') {
        dock = { edge: saved.edge, mon: m.bounds, collapsed: true }
        placeFloat(win, collapsedX(saved.edge, m.bounds, DEFAULT_FLOAT_W), y)
      } else {
        placeFloat(win, Math.round(dip.x), y)
      }
      return
    }
  }
  // 默认：主显示器右上角留白（自由态）。
  const b = screen.getPrimaryDisplay().bounds
  placeFloat(win, b.x + b.width - DEFAULT_FLOAT_W - MARGIN_LOGICAL, b.y + MARGIN_LOGICAL)
}

// ===========================================================================
// 拖动 / 吸附 / 悬停展开（全部 DIP）
// ===========================================================================

/**
 * pointerdown：暂存按下点（屏幕绝对坐标，DIP）。不展开半隐藏态、不置 moving ——
 * 这次按压可能只是点击；展开 / moving 推迟到越过阈值的第一帧 move。
 */
function onDown(sx: number, sy: number): void {
  press = { x: sx, y: sy }
}

/**
 * 拖动中（每帧）。sx/sy 是页面上报的指针【屏幕绝对坐标】（screenX/screenY，DIP）。
 * 首帧用 pointerdown 暂存的按下点计算抓取偏移：若从半隐藏态起拖，先展开、后记偏移，
 * 展开那一下 28px 平移不会变成球的跳动。不能用首帧 move 的当前光标记锚点 —— 那时
 * 光标已离手，首帧窗口会纹丝不动（合成指针真机取证实锤）。之后每帧按绝对坐标摆放，
 * IPC 丢帧 / 晚到都能在下一帧自愈。
 */
function onMove(sx: number, sy: number): void {
  const win = floatWin
  if (!win || win.isDestroyed()) return
  moving = true
  if (!anchor) {
    // 取走按下点；缺失（异常时序）时退回用当前帧坐标，保持可拖动。
    const p = press ?? { x: sx, y: sy }
    press = null
    if (dock?.collapsed) {
      // 起拖即作废旧一代延迟缩回（展开后不能被旧定时器缩回去）。
      cancelPendingCollapse()
      expand(dock)
    }
    const b0 = win.getBounds()
    anchor = { x: p.x - b0.x, y: p.y - b0.y }
  }
  let nx = Math.round(sx - anchor.x)
  let ny = Math.round(sy - anchor.y)
  // 拖动中软钳制：允许跨显示器，但在光标所在显示器上至少留一条窄边，防止球甩丢。
  // 宽高用固定常量，绝不读 getBounds()：Windows 出屏移动后它会漂移（见 placeFloat）。
  const mon = screen.getDisplayNearestPoint({ x: sx, y: sy }).bounds
  nx = clampVisible(nx, DEFAULT_FLOAT_W, mon.x, mon.x + mon.width, MIN_VISIBLE_LOGICAL)
  ny = clampVisible(ny, DEFAULT_FLOAT_H, mon.y, mon.y + mon.height, MIN_VISIBLE_LOGICAL)
  placeFloat(win, nx, ny)
}

function onDragEnd(): void {
  const win = floatWin
  if (!win || win.isDestroyed()) {
    clearDragRuntime()
    return
  }
  const raw = win.getBounds()
  // 只信当前原点 x/y；宽高恒为固定 56（getBounds 的宽在出屏移动后会漂移，见 placeFloat）。
  const b: Rect = { x: raw.x, y: raw.y, width: DEFAULT_FLOAT_W, height: DEFAULT_FLOAT_H }
  // 找窗口中心所在显示器；getDisplayNearestPoint 在屏间空隙也会给最近一块，
  // 与 Rust 回退主显示器等效，保证吸附 / 钳制总有边界可用。
  const mon = screen.getDisplayNearestPoint({
    x: Math.round(b.x + b.width / 2),
    y: Math.round(b.y + b.height / 2),
  }).bounds
  const edge = decideEdge(b, mon, SNAP_LOGICAL)
  // 松手即吸附的缩回是立即的；作废旧一代悬停定时器，避免稍后重复动作。
  cancelPendingCollapse()
  if (edge === 'left' || edge === 'right') {
    // 吸附：展开态贴边、纵向钳制，落盘展开态坐标，随即缩回成半球。
    const y = clampY(Math.round(b.y), b.height, mon)
    const x = expandedX(edge, mon, b.width)
    placeFloat(win, x, y)
    dock = { edge, mon, collapsed: false }
    persistGeometry(win)
    collapse(dock)
  } else {
    // 自由态：完整钳回显示器内（旧实现不钳，球可停在屏外，没有标题栏就抓不回来了）。
    // placeFloat 每次都带固定宽高，顺带把已发生的尺寸漂移纠正回 56×56。
    const p = clampFreeInside(b, mon)
    placeFloat(win, p.x, p.y)
    dock = null
    persistGeometry(win)
    pushState(null)
  }
  clearDragRuntime()
}

function onPeekEnter(): void {
  if (moving || !dock) return
  // 任何一次进入都必须先作废排队中的延迟缩回 —— 哪怕当前已展开（leave 已排队、
  // 300ms 未到又触发 enter），否则定时器到期会把用户鼠标下的球缩回（真机取证项）。
  cancelPendingCollapse()
  // 已展开 / 自由态无需进一步动作；只有半隐藏半球才展开。
  if (!dock.collapsed) return
  expand(dock)
}

function onPeekLeave(): void {
  if (moving || !dock || dock.collapsed) return
  // 不立刻缩回：边缘处窗口 28px 的展开 / 缩回平移会连环触发进出事件，即时缩回会让球
  // 在边缘抖动。延迟 COLLAPSE_DELAY_MS（连续 leave 重新计时），到期校验代际 + 真实光标位置。
  if (collapseTimer) clearTimeout(collapseTimer)
  const gen = peekGen + 1
  peekGen = gen
  collapseTimer = setTimeout(() => {
    collapseTimer = null
    finishPeekLeave(gen)
  }, COLLAPSE_DELAY_MS)
}

/** 延迟到期：条件不再成立则放弃；光标已在「缩回后矩形」内也保持展开，防止滑到光标下再展开。 */
function finishPeekLeave(gen: number): void {
  const win = floatWin
  if (gen !== peekGen || moving || !win || win.isDestroyed() || !win.isVisible() || !dock) return
  if (dock.collapsed) return
  const d = dock
  const b = win.getBounds()
  // 尺寸用固定常量（getBounds 的宽在出屏移动后会漂移，见 placeFloat）。
  const y = clampY(Math.round(b.y), DEFAULT_FLOAT_H, d.mon)
  const x = collapsedX(d.edge, d.mon, DEFAULT_FLOAT_W, PEEK_LOGICAL)
  const target: Rect = { x, y, width: DEFAULT_FLOAT_W, height: DEFAULT_FLOAT_H }
  if (pointInRect(screen.getCursorScreenPoint(), rectAt(x, y, target))) return
  collapse(d)
}

/** 展开到屏内并通知前端。 */
function expand(d: Dock): void {
  const win = floatWin
  if (!win || win.isDestroyed()) return
  const y = win.getBounds().y
  placeFloat(win, expandedX(d.edge, d.mon, DEFAULT_FLOAT_W), y)
  dock = { ...d, collapsed: false }
  pushState(dock)
}

/** 缩回屏外只露半球并通知前端。 */
function collapse(d: Dock): void {
  const win = floatWin
  if (!win || win.isDestroyed()) return
  const y = win.getBounds().y
  placeFloat(win, collapsedX(d.edge, d.mon, DEFAULT_FLOAT_W, PEEK_LOGICAL), y)
  dock = { ...d, collapsed: true }
  pushState(dock)
}

/**
 * 通知 floaticon.html 切换吸附 / 缩回样式（主框架执行；字段为 JSON 字面量）。
 * anchor 与 Tauri 侧逐字一致：右吸附活柱在右（'r'），其余在左（'l'）。Electron 宿主窗
 * 正好 56 DIP 宽，两种 anchor 视觉等价，但字段照发，保证双壳页面是同一份无分叉代码。
 */
function pushState(d: Dock | null): void {
  const win = floatWin
  if (!win || win.isDestroyed()) return
  const edge: FloatEdge = d?.edge ?? 'none'
  const collapsed = d?.collapsed ?? false
  const anchor = d?.edge === 'right' ? 'r' : 'l'
  const code =
    'window.__web2appFloat&&window.__web2appFloat.setState(' +
    `{edge:${JSON.stringify(edge)},collapsed:${collapsed},anchor:${JSON.stringify(anchor)}});`
  win.webContents.executeJavaScript(code, true).catch(() => {
    /* 文档尚未就绪：忽略，did-finish-load 会重推 */
  })
}

// ===========================================================================
// 持久化（读-改-写，只替换 float_window 段；物理像素）
// ===========================================================================

/**
 * 抓取当前悬浮球几何落盘。吸附态下存【展开态】x（半隐藏偏移运行期算），
 * y / 宽高取当前窗口实际值（物理像素，与 Tauri FloatWinGeom 同形）。
 */
function persistGeometry(win: BrowserWindow): void {
  if (!deps) return
  try {
    const b = win.getBounds() // 只取位置（DIP）；尺寸恒为固定 56，不读可能漂移的 b.width
    const edge: FloatEdge = dock?.edge ?? 'none'
    const dipX = dock ? expandedX(dock.edge, dock.mon, DEFAULT_FLOAT_W) : b.x
    const phys = screen.dipToScreenRect(win, {
      x: Math.round(dipX),
      y: Math.round(b.y),
      width: DEFAULT_FLOAT_W,
      height: DEFAULT_FLOAT_H,
    })
    const geom: FloatGeom = {
      x: Math.round(phys.x),
      y: Math.round(phys.y),
      width: Math.round(phys.width),
      height: Math.round(phys.height),
      edge,
    }
    // 读-改-写：只替换 float_window 段，设置与主窗口 window 段原样保留。
    const cur = readRawForUpdate(deps.getStoreLoc())
    writeDocument(deps.getStoreLoc(), {
      settings: parseSettings(cur),
      window: cur.window,
      float_window: geomToSegment(geom),
    })
  } catch (e) {
    console.warn(`[web2app] 写入悬浮球几何失败：${(e as Error).message}`)
  }
}
