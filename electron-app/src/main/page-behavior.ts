/**
 * M4 页面行为：**开发者工具（F12）/ 右键菜单 / 下载接管**；外加多页签的
 * **Ctrl+W 关闭当前页签**（M5 快捷键，见文件末尾）。
 *
 * 前四项都对齐 Tauri 版（后者在 `webview_hooks.rs` / `tabs::window` 里用 WebView2 原生 API
 * 与注入脚本实现）：
 *
 * | 能力 | Tauri 侧 | 本壳 |
 * |---|---|---|
 * | F12 | `DEVTOOLS_ENABLED`（debug 构建开 / release 关） | `devtoolsEnabled()`：`!isPackaged \|\| debug` |
 * | 右键菜单 | 原生菜单上**移除** print / save* / moreTools，**追加**「设置」 | 自建菜单，同样不给打印/另存为，同样带「设置」 |
 * | 下载 | 系统「另存为」（`save_dialog.rs`）→ 用户定落盘位置 | `dialog.showSaveDialog` → `item.setSavePath()` |
 * | Ctrl+W 关页签 | tab 页面注入 `tab_keys.js` → 核心事件 `web2app:close-tab` | 本文件的 `attachCloseTabShortcut()`（`before-input-event`） |
 *
 * ⚠️ 本文件的分层是**刻意**的，别顺手糊在一起：
 *   - **纯函数区**（`isDevtoolsKey` / `devtoolsEnabled` / `contextMenuTemplate` /
 *     `saveDialogOptions`）：不碰任何 Electron API、不需要窗口，故能被
 *     `test/page-behavior.test.ts` 在毫秒级直接钉住。菜单「该出哪些项、哪些禁用」
 *     正是那种「错了也不报错」的规则，值得脱离 GUI 覆盖；
 *   - **挂载区**（`attachDevtools` / `attachContextMenu` / `attachDownload`）：只做
 *     「把纯函数的结论接到 Electron 事件上」，真实行为由探针（`probe.ts`）验。
 *
 * ⚠️ 为什么右键菜单要**自建**而不是沿用 Chromium 原生菜单：Electron 根本不把 WebView2 /
 *   Chromium 的原生上下文菜单暴露给主进程，只有 `context-menu` 事件 + 一堆参数。
 *   所以「对齐 Tauri」在这项上的体现是**菜单项的取舍一致**，不是实现路径一致。
 */
// ⚠️⚠️ 必须用**默认导入再解构**，不能 `import { Menu } from 'electron'`：
//   electron 的入口是 CommonJS（`module.exports = <exe 路径>`），而 `pnpm run unit` 是
//   `node --test` 直接跑 **.ts** —— Node 按 **ESM** 解析 .ts，命名导入 CJS 会直接抛
//   `SyntaxError: Named export 'Menu' not found`。默认导入在「tsc 编译成 CJS」与
//   「Node 直接跑 ESM」两条路径下都成立。
//   纯 Node 下解构出来的是 undefined（字符串没有这些属性），但**不会报错** ——
//   这正是纯函数区能被单测直接 import 的前提（顶层不碰任何 Electron API）。
import electron from 'electron'
import type { SaveDialogOptions } from 'electron'
import { extname, join } from 'node:path'
import type { BrowserWindow, Session, WebContents } from 'electron'
import { isSplashUrl } from './nav-text'

const { app, clipboard, dialog, Menu } = electron

// ---------------------------------------------------------------------------
// 纯函数区（零 Electron 调用，可直接单测）
// ---------------------------------------------------------------------------

/**
 * F12 是否可用。
 *
 * 对齐 Tauri 的 `DEVTOOLS_ENABLED`（`#[cfg(debug_assertions)]`）：那边是**编译期**常量，
 * debug 开 / release 关。本壳没有 debug 构建分发（绿色版就是打包产物），故取：
 *   · `!isPackaged` —— 未打包（源码态 `pnpm start` / 探针）⇒ 恒开，等价于 cargo debug；
 *   · `debug` —— 配置里的 `debug: true` ⇒ 打包版也开。
 *
 * ⚠️ 这个差异是**有意**的：绿色版没有「编一个 debug 版给用户」这条路径，
 *    用户在打包版里想排查 Network / Console 只能靠配置开关。已记进 `electron-app/AGENTS.md` 的 M4 条。
 */
export function devtoolsEnabled(isPackaged: boolean, debug: boolean): boolean {
  return !isPackaged || debug
}

/** `before-input-event` 给的按键信息里，本文件关心的字段。 */
export interface KeyInputLike {
  type?: string
  key?: string
  /** 以下四项由 `before-input-event` 一并给出（`WebContents` 的 Input 对象） */
  control?: boolean
  shift?: boolean
  alt?: boolean
  meta?: boolean
  /** 长按产生的重复按键 */
  isAutoRepeat?: boolean
}

/**
 * 是否是「打开开发者工具」的那次按键。
 *
 * ⚠️ 只认 `keyDown`：keyUp 也判一次的话一次按键会触发两遍（开着→关→开，看上去像没反应）。
 */
export function isDevtoolsKey(input: KeyInputLike | null | undefined): boolean {
  return !!input && input.type === 'keyDown' && input.key === 'F12'
}

/**
 * 是否是「关闭当前页签」的那次按键：**Ctrl+W**，且不带 Shift / Alt / Meta。
 *
 * 为什么自己做判断而不是给窗口挂菜单 accelerator：本壳是**无边框自绘标题栏**（没有系统菜单
 * 栏），而 Ctrl+W 必须由主进程在 `before-input-event` 里先接住 —— 否则远程页面自己的
 * Ctrl+W（编辑器类站点常见）会先吃掉这次按键，用户侧表现为「有时能关、有时关不掉」。
 *
 * ⚠️ 三条刻意排除，改前先读：
 *   · `shiftKey`：Ctrl+Shift+W 在浏览器里是「关闭窗口」，本壳**不提供**这条（关窗会触发
 *     托盘驻留语义，和「关页签」完全不是一回事），混进来等于给用户一个意外出口；
 *   · `altKey` / `metaKey`：只认最窄的那个组合，避免与输入法 / 系统组合键撞车；
 *   · `isAutoRepeat`：长按不连关。浏览器会连关，但桌面壳里一按到底烧掉整叠页签是不可逆的
 *     体验事故（页签状态不再有历史栈），宁可要求「按一次关一个」。
 */
export function isCloseTabKey(input: KeyInputLike | null | undefined): boolean {
  if (!input || input.type !== 'keyDown') return false
  if (input.control !== true || input.shift || input.alt || input.meta) return false
  if (input.isAutoRepeat) return false
  // `key` 在按住 Ctrl 时仍是 'w'；同时接受 'W'（大写锁定开着时个别布局会给大写）
  // ⚠️ 不要改成按 `input.code` 兜底：`code` 按 US 布局命名，AZERTY 上「生出 w 的物理键」
  //   是 KeyZ ⇒ 用户按 Ctrl+Z（撤销）会被当成 Ctrl+W 关掉页签。双壳契约测试正反两项钉住。
  return input.key === 'w' || input.key === 'W'
}

/** 右键菜单项规格（纯数据：`label` 为 null 表示分隔线）。 */
export interface MenuItemSpec {
  /** null = 分隔线 */
  label: string | null
  action?: ContextMenuAction
  /** 缺省 = 启用 */
  enabled?: boolean
}

export type ContextMenuAction =
  | 'back'
  | 'forward'
  | 'reload'
  | 'copy'
  | 'cut'
  | 'paste'
  | 'selectAll'
  | 'copyLink'
  | 'settings'

/** `context-menu` 事件参数里本文件用到的部分。 */
export interface ContextMenuParamsLike {
  isEditable: boolean
  selectionText: string
  linkURL: string
  editFlags: { canCopy: boolean; canCut: boolean; canPaste: boolean }
}

/** 导航历史可用性（构造菜单时按历史条目实时计算一次）。 */
export interface NavFlags {
  canGoBack: boolean
  canGoForward: boolean
}

/**
 * 内置页面判定（历史落点用）：实现已收敛到 `nav-text.ts::isSplashUrl`（M-4），
 * 这里保留旧名避免调用方与跨壳契约测试大改。内置页永远不该成为前进/后退落点。
 */
export { isSplashUrl as isBuiltinUrl, isInternalUrl } from './nav-text'

/**
 * 后退落点是否为真实网页：当前条目的上一条历史存在，且其 URL 不是内置 splash 页。
 * 应用启动时 splash 是第一条历史（splash → 目标地址 loadURL），裸 canGoBack() 会误放行。
 */
export function canHistoryGoBack(
  entries: ReadonlyArray<{ url: string }>,
  activeIndex: number
): boolean {
  const prev = entries[activeIndex - 1]
  return !!prev && !isSplashUrl(prev.url)
}

/** 前进落点同理：下一条历史存在且不是内置 splash 页。 */
export function canHistoryGoForward(
  entries: ReadonlyArray<{ url: string }>,
  activeIndex: number
): boolean {
  const next = entries[activeIndex + 1]
  return !!next && !isSplashUrl(next.url)
}

/**
 * 构造右键菜单。
 *
 * 取舍与 Tauri 侧**一致**：不给「打印」「另存为」「更多工具」（那边的 `ContextMenuRequested`
 * 回调正是按 name 把 print / save* / moreTools 从原生菜单里删掉的）—— 封装壳里留这些入口
 * 既没意义又容易让页面被另存成残缺的本地副本。
 *
 * 「设置」项对齐 Tauri：右键菜单与 Shift+F1、托盘菜单走**同一个**入口。
 */
export function contextMenuTemplate(p: ContextMenuParamsLike, nav: NavFlags): MenuItemSpec[] {
  const items: MenuItemSpec[] = [
    { label: '返回', action: 'back', enabled: nav.canGoBack },
    { label: '前进', action: 'forward', enabled: nav.canGoForward },
    { label: '重新加载', action: 'reload' },
    { label: null },
    // 编辑四项：浏览器里它们**常驻但会置灰**（突然少一项更难用），故禁用态保留不隐藏。
    // 复制额外放行「有选中文本」的情况：某些页面里 editFlags 没及时更新，
    // 只信 flags 会出现「明明选了字、复制却是灰的」。
    { label: '复制', action: 'copy', enabled: p.editFlags.canCopy || p.selectionText.length > 0 },
    { label: '剪切', action: 'cut', enabled: p.editFlags.canCut },
    // 粘贴只在可编辑处有意义（输入框 / contenteditable）
    { label: '粘贴', action: 'paste', enabled: p.editFlags.canPaste && p.isEditable },
    { label: '全选', action: 'selectAll' },
  ]
  if (p.linkURL) {
    items.push({ label: null }, { label: '复制链接地址', action: 'copyLink' })
  }
  items.push({ label: null }, { label: '设置', action: 'settings' })
  return stripRedundantSeparators(items)
}

/** 去掉**开头的**、**结尾的**与**连续重复**的分隔线（菜单首尾挂一条空线很难看）。 */
export function stripRedundantSeparators(items: MenuItemSpec[]): MenuItemSpec[] {
  const out: MenuItemSpec[] = []
  for (const it of items) {
    if (it.label === null && (out.length === 0 || out[out.length - 1].label === null)) continue
    out.push(it)
  }
  while (out.length > 0 && out[out.length - 1].label === null) out.pop()
  return out
}

/**
 * 下载「另存为」对话框的参数。
 *
 * 对齐 `save_dialog.rs`：默认落点是**系统下载目录**、默认文件名用站点给的名字、
 * 按后缀收敛过滤器、开**覆盖确认**（`FOS_OVERWRITEPROMPT` ⇒ `showOverwriteConfirmation`）。
 */
export function saveDialogOptions(filename: string, downloadDir: string): SaveDialogOptions {
  // 站点没给名字时给个兜底：空名字会让对话框的文件名输入框空白，比 "download" 更难用
  const name = filename.trim() || 'download'
  const ext = extname(name).replace(/^\./, '')
  return {
    title: '另存为',
    defaultPath: downloadDir ? join(downloadDir, name) : name,
    properties: ['showOverwriteConfirmation', 'createDirectory'],
    ...(ext ? { filters: [{ name: `${ext} 文件 (*.${ext})`, extensions: [ext] }] } : {}),
  }
}

// ---------------------------------------------------------------------------
// 挂载区（接 Electron 事件；真实行为由探针验）
// ---------------------------------------------------------------------------

/**
 * F12 → 打开 / 关闭开发者工具（再按一次关掉，浏览器就是这个手感）。
 *
 * ⚠️ `mode: 'detach'` 是必须的：本壳是**无边框自绘标题栏**的窗口，内嵌 DevTools 会占掉
 *    一整条页面区域，还会把自绘标题栏挤变形；独立窗口更接近 WebView2 的表现。
 */
export function attachDevtools(wc: WebContents, enabled: boolean): void {
  if (!enabled) return
  wc.on('before-input-event', (_e, input) => {
    if (!isDevtoolsKey(input)) return
    if (wc.isDevToolsOpened()) wc.closeDevTools()
    else wc.openDevTools({ mode: 'detach' })
  })
}

/**
 * Ctrl+W → 关闭**当前页签**（多页签模式；单窗口模式不挂，见调用点）。
 *
 * 挂两个地方，两处合起来才是「无论焦点在哪都生效」：
 *   ① 每个内容视图（`tab-spawn.ts`）—— 用户在网页里打字 / 点链接时焦点在这里；
 *   ② 壳页面（`index.ts`）—— 用户点过标签栏（或刚关掉一个页签）时焦点在壳上。
 *
 * ⚠️⚠️ `event.preventDefault()` 不可省：不拦的话这次按键会继续派发给页面与菜单快捷键 ——
 *   页面若自己也处理 Ctrl+W 就会「关页签 + 页面动作」双触发；而菜单若带 close 角色的
 *   accelerator，还会顺带把窗口一起关掉。
 *
 * ⚠️ 回调里**不要**同步关闭 `wc` 所属的视图：本处理器正跑在 `wc` 的事件派发栈里，就地
 * `webContents.close()` 属于「在自己的回调里销毁自己」。调用方（tab-spawn / index）用
 * `setImmediate` 推到下一轮再关。
 */
export function attachCloseTabShortcut(wc: WebContents, onClose: () => void): void {
  wc.on('before-input-event', (event, input) => {
    if (!isCloseTabKey(input)) return
    event.preventDefault()
    onClose()
  })
}

/** 把菜单动作落到目标 `WebContents` 上。 */
function runMenuAction(wc: WebContents, action: ContextMenuAction | undefined, linkURL: string, onSettings: () => void): void {
  switch (action) {
    case 'back': {
      // 落点不是真实网页时不触发（不能只信 canGoBack：splash 也在历史里）。
      const h = wc.navigationHistory
      if (canHistoryGoBack(h.getAllEntries(), h.getActiveIndex())) h.goBack()
      break
    }
    case 'forward': {
      const h = wc.navigationHistory
      if (canHistoryGoForward(h.getAllEntries(), h.getActiveIndex())) h.goForward()
      break
    }
    case 'reload':
      wc.reload()
      break
    case 'copy':
      wc.copy()
      break
    case 'cut':
      wc.cut()
      break
    case 'paste':
      wc.paste()
      break
    case 'selectAll':
      wc.selectAll()
      break
    case 'copyLink':
      if (linkURL) clipboard.writeText(linkURL)
      break
    case 'settings':
      onSettings()
      break
    default:
      break
  }
}

/**
 * 挂右键菜单。
 *
 * ⚠️ `popup()` **不传 window**：Electron 会按当前焦点窗口自己定位，多页签模式下
 *    传壳窗口会让菜单错位到内容视图之外（`context-menu` 的 `params.x/y` 是相对内容视图的）。
 */
export function attachContextMenu(wc: WebContents, onSettings: () => void): void {
  wc.on('context-menu', (_e, params) => {
    // 菜单可用性按历史落点实时算：相邻条目是内置 splash 页时「返回 / 前进」置灰。
    const navHistory = wc.navigationHistory
    const allEntries = navHistory.getAllEntries()
    const activeIndex = navHistory.getActiveIndex()
    const specs = contextMenuTemplate(
      {
        isEditable: params.isEditable,
        selectionText: params.selectionText,
        linkURL: params.linkURL,
        editFlags: {
          canCopy: params.editFlags.canCopy,
          canCut: params.editFlags.canCut,
          canPaste: params.editFlags.canPaste,
        },
      },
      {
        canGoBack: canHistoryGoBack(allEntries, activeIndex),
        canGoForward: canHistoryGoForward(allEntries, activeIndex),
      },
    )
    const template = specs.map((s) =>
      s.label === null
        ? { type: 'separator' as const }
        : {
            label: s.label,
            enabled: s.enabled !== false,
            click: () => runMenuAction(wc, s.action, params.linkURL, onSettings),
          },
    )
    Menu.buildFromTemplate(template).popup()
  })
}

/**
 * 下载接管的可选注入点：多页签模式用 `onSaveChoice` 自动关空页签，
 * 探针用 `pickSavePath` 免弹模态对话框。单窗口生产路径一个都不传。
 */
export interface DownloadHooks {
  /**
   * 提交弹框前的判重：返回 true 表示这次下载是**重复的**，调用方直接 `item.cancel()`，
   * 不弹「另存为」、不落盘。
   *
   * 两类来源：① 「隐藏弹窗窗口」抢在新页签之前发起的那条（它只是为了给页面一个非 null
   * 的 `window.open()` 返回值，真正的下载由新页签做）；② 站点 `location.href` 同地址兜底
   * （`window.open()` 被判「拦截」时才会走，Electron 侧已从根上避免，留作纵深防御）。
   */
  shouldDropDownload?: (webContentsId: number, url: string) => boolean
  /**
   * 用户对「另存为」做完选择（或取消）后回调，入参是**发起下载**的 webContents id。
   *
   * 多页签模式用它自动关掉「只为下载而打开、从未提交过文档」的空页签
   * （见 `tabs.ts::closeTabAfterDownload`）。⚠️ 放在选择之后而不是 `will-download`
   * 同步段里：下载项此时已 `setSavePath` + `resume`（或已取消），关掉发起页签不会
   * 打断下载本身；且「另存为」是模态框，期间用户也点不到那个空页签。
   */
  onSaveChoice?: (webContentsId: number, saved: boolean) => void
  /**
   * 选落盘路径。默认弹系统「另存为」；探针注入固定临时路径，避免模态对话框把断言挂住。
   * 返回 null 表示取消（对齐用户点「取消」）。
   */
  pickSavePath?: (filename: string, downloadDir: string) => Promise<string | null>
  /**
   * 每次下载刚建立时回调（早于判重 / 弹框），入参是**发起下载**的 webContents id。
   *
   * 多页签模式用它标记「隐藏弹窗窗口」这次是真的在下载 ⇒ 该窗口要一直活到下载结束
   * （站点常见判据 `if (!w || w.closed)`，过早关掉它会触发 `location.href` 兜底二次下载）。
   */
  onDownloadStart?: (webContentsId: number) => void
}

/**
 * 接管下载：弹「另存为」让用户定落盘位置，然后才真正开始写文件。
 *
 * ⚠️⚠️ `item.pause()` 不可省：`dialog.showSaveDialog` 是**异步**的，事件处理器一返回，
 *   Electron 就会弹**它自己**的保存对话框 —— 用户看到的是两个对话框抢注意力。
 *   先 pause 再（用户选完）`setSavePath` + `resume`，才是「一次下载、一个对话框」。
 *
 * ⚠️ 用户取消走 `item.cancel()`，**不要**存到别处：
 *   「点取消却悄悄下载到下载目录」是最招骂的那类自作主张（`save_dialog.rs` 里
 *   `SaveChoice::Cancelled` 的注释同理）。
 */
export function attachDownload(
  session: Session,
  getWindow: () => BrowserWindow | null,
  hooks: DownloadHooks = {},
): void {
  session.on('will-download', (_e, item, webContents) => {
    const name = item.getFilename() || 'download'
    // ⚠️ 排障必需：同一次点击出现两个「另存为」时，唯一能区分来源的就是 wc（发起下载的
    //    webContents id）+ url（站点究竟请求了哪两个地址）。这两项不要删。
    console.log(
      `[web2app] 下载请求：${name} wc=${webContents?.id ?? 'none'} url=${item.getURL()}`
    )
    // ⚠️ 顺序：先报告「下载已建立」（隐藏弹窗要靠它延命），再判重、再 pause/弹框。
    if (webContents) hooks.onDownloadStart?.(webContents.id)
    // ⚠️ 判重必须早于 pause / 弹框：重复下载要在出现「第二个另存为」之前就掐掉。
    if (webContents && hooks.shouldDropDownload?.(webContents.id, item.getURL())) {
      console.log(
        `[web2app] 忽略重复下载（隐藏弹窗 / 弹窗兜底）：${name} wc=${webContents.id} url=${item.getURL()}`
      )
      item.cancel()
      return
    }
    let dir = ''
    try {
      dir = app.getPath('downloads')
    } catch {
      /* 取不到就用空串：saveDialogOptions 会退化成「只有文件名」，对话框仍可用 */
    }
    item.pause()
    // ⚠️ 先取 id：下载完成前发起页签可能已被自动关闭（多页签下载场景），
    //    届时 webContents.id 会抛「Object has been destroyed」。
    const sourceId: number | undefined = webContents?.id
    const pick =
      hooks.pickSavePath ??
      ((filename: string, downloadDir: string) => {
        const opts = saveDialogOptions(filename, downloadDir)
        const win = getWindow()
        const asked = win ? dialog.showSaveDialog(win, opts) : dialog.showSaveDialog(opts)
        return asked.then(({ canceled, filePath }) =>
          canceled || !filePath ? null : filePath
        )
      })
    void pick(name, dir)
      .then((filePath) => {
        if (!filePath) {
          console.log(`[web2app] 下载已取消：${name}`)
          item.cancel()
          if (sourceId !== undefined) hooks.onSaveChoice?.(sourceId, false)
          return
        }
        console.log(`[web2app] 下载：${name} → ${filePath} wc=${sourceId}`)
        item.setSavePath(filePath)
        item.resume()
        if (sourceId !== undefined) hooks.onSaveChoice?.(sourceId, true)
      })
      .catch((e: unknown) => {
        // 对话框自己异常了也不能让下载悬着：取消掉，宁可让用户再点一次
        console.error(`[web2app] 「另存为」对话框异常，已取消本次下载：${(e as Error).message}`)
        item.cancel()
        if (sourceId !== undefined) hooks.onSaveChoice?.(sourceId, false)
      })
    item.once('done', (_e, state) => {
      console.log(`[web2app] 下载结束（${state}）：${name} wc=${sourceId}`)
    })
  })
}
