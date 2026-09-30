/**
 * 主进程 IPC / 鉴权事件注册（2026-10-01 从 index.ts 抽出）。
 *
 * 这里集中四组注册：
 *   1. win:*     标题栏窗口按钮（单窗口 / 页签壳共用，特权命令带 sender 校验）
 *   2. tabs:*    壳页面页签操作（命令名与 Tauri 侧逐字一致）
 *   3. web2app:bootstrap  同步给 preload 下发注入脚本源码
 *   4. app 'login'        Basic 凭据回填（仅入口主机 / 代理）
 *
 * 边界：本模块**不持有状态**。主窗口、页签管理器、入口目标在 index.ts 里都是「先声明、
 * ready 后才有值」的可变状态，这里一律以 getter 注入；注册动作发生在 whenReady 之后。
 */
import { app, BrowserWindow, ipcMain } from 'electron'

import type { AppConfig } from './config'
import type { ResolvedTarget } from './navigate'
import type { TabManager } from './tabs'

export interface AppIpcDeps {
  /** 多页签模式：特权命令必须校验 sender.id === 壳 webContents.id */
  tabsMode: boolean
  /** 主窗口（ready 后唯一实例）；注册时窗口可能尚未创建，故为 getter */
  getMainWindow: () => BrowserWindow | null
  /** 标题栏 / 标签栏「设置」入口（与托盘菜单、Shift+F1 共用同一个函数） */
  openSettings: () => void
  /** 多页签管理器（仅 tabsMode 下非 null） */
  getTabManager: () => TabManager | null
  /** 壳页面失败遮罩「重试」按钮：复用启动期首签 spawn 路径（fire-and-forget） */
  spawnFirstTab: () => void
  /** web2app:bootstrap 同步下发给 preload 的注入内容（仅顶层框架） */
  bootstrap: {
    /** 标题栏高度（= 页面 38px 让位的同一个数字） */
    chromeH: number
    /** config.json 的 extra（原样挂到 window.web2appExtra） */
    extra: AppConfig['extra']
    /** 注入资源读取（单一真源 webview-app/src，由 index.ts 负责定位 + 缓存） */
    readInject: (name: string) => string | null
  }
  /** 入口目标（Basic 凭据只回填给入口主机）；ready 后解析完成 */
  getTarget: () => ResolvedTarget | null
  /** 外部代理地址（代理返回 401 时从中取 userinfo 凭据），缺省为 '' */
  proxyHttp: string
  proxyHttps: string
}

export function registerAppIpc(deps: AppIpcDeps): void {
  registerWindowIpc(deps)
  registerTabsIpc(deps)
  registerBootstrapIpc(deps)
  registerAuthHandler(deps)
}

function registerWindowIpc(deps: AppIpcDeps): void {
  const winOf = (e: Electron.IpcMainInvokeEvent) => BrowserWindow.fromWebContents(e.sender)

  // ⚠️⚠️ 多页签模式下的安全硬化：内容视图（子 WebContentsView）的 webContents 经
  //   `BrowserWindow.fromWebContents()` **也会解析到宿主窗口**（两者 getType() 都是 'window'，
  //   实测见设计文档 §3.5-2）。所以仅靠 fromWebContents 无法区分「壳发的」还是「内容页发的」。
  //   窗口按钮这类特权命令必须校验 sender.id === 壳 webContents.id，不匹配**记日志并拒绝**
  //   （绝不允许静默失效——那正是本项目最忌的失败形态）。
  //
  //   单窗口模式下壳 webContents == 主窗口 webContents，这条校验天然通过，行为零变化。
  const assertPrivileged = (e: Electron.IpcMainInvokeEvent): void => {
    if (!deps.tabsMode) return
    const host = deps.getMainWindow()
    if (!host || host.isDestroyed()) throw new Error('宿主窗口不存在')
    if (e.sender.id !== host.webContents.id) {
      console.warn(
        `[web2app] 拒绝内容视图发起的窗口命令（sender.id=${e.sender.id} ≠ 壳=${host.webContents.id}）`
      )
      throw new Error('拒绝：仅壳页面可操作窗口')
    }
  }

  // ⚠️ 三个都用 handle（而不是 on）：调用失败要能变成 Promise rejection 传到页面，注入脚本
  // 会把它打到控制台。用 on 的话「按钮点了没反应」在页面侧完全无声（Tauri 版栽过这个坑）。
  ipcMain.handle('win:minimize', (e) => {
    assertPrivileged(e)
    winOf(e)?.minimize()
  })
  ipcMain.handle('win:toggle-maximize', (e) => {
    assertPrivileged(e)
    const w = winOf(e)
    if (!w) throw new Error('找不到窗口')
    if (w.isMaximized()) w.unmaximize()
    else w.maximize()
    return w.isMaximized()
  })
  ipcMain.handle('win:close', (e) => {
    assertPrivileged(e)
    winOf(e)?.close()
  })
  // 标题栏「设置」按钮（最小化左侧）—— 与托盘菜单 / Shift+F1 共用 openSettings()。
  ipcMain.handle('win:open-settings', (e) => {
    assertPrivileged(e)
    deps.openSettings()
  })
}

/**
 * 壳页面（shell.html）的 IPC 对接面 —— 命令名与 Tauri 侧一致（get_tabs / switch_tab /
 * close_tab / minimize_host / close_host / history_nav），由 shell-preload 的
 * `__TAURI_INTERNALS__.invoke` 垫片以 `tabs:` 前缀转发到这里。
 *
 * ⚠️ 全部校验 sender === 壳 webContents（内容视图发同类消息必须被拒，留日志）。关闭宿主
 *   走的是与托盘/退出统一的语义（closeTab 里 close 掉最后一个页签会触发 host.close()，
 *   host 的 close 处理器负责「隐藏到托盘 or 真正退出」）。
 */
function registerTabsIpc(deps: AppIpcDeps): void {
  const assertShell = (e: Electron.IpcMainInvokeEvent): BrowserWindow => {
    const w = deps.getMainWindow()
    if (!w || w.isDestroyed()) throw new Error('宿主窗口不存在')
    if (e.sender.id !== w.webContents.id) {
      console.warn(`[web2app] 拒绝非壳页签命令（sender.id=${e.sender.id} ≠ 壳=${w.webContents.id}）`)
      throw new Error('拒绝：仅壳页面可操作页签')
    }
    return w
  }
  const mgr = () => {
    const t = deps.getTabManager()
    if (!t) throw new Error('多页签未启用')
    return t
  }

  ipcMain.handle('tabs:get_tabs', (e) => {
    assertShell(e)
    // 返回元组数组 [label,title,active]，对齐 Tauri get_tabs 的 Vec<(String,String,bool)>
    return mgr().listForIpc()
  })
  ipcMain.handle('tabs:switch_tab', (e, args: { label?: unknown } | undefined) => {
    assertShell(e)
    const label = typeof args?.label === 'string' ? args.label : ''
    if (!label) throw new Error('参数错误：缺少 label')
    mgr().activate(label)
    return null
  })
  ipcMain.handle('tabs:close_tab', (e, args: { label?: unknown } | undefined) => {
    assertShell(e)
    const label = typeof args?.label === 'string' ? args.label : ''
    if (!label) throw new Error('参数错误：缺少 label')
    mgr().closeTab(label)
    return null
  })
  ipcMain.handle('tabs:minimize_host', (e) => {
    assertShell(e).minimize()
    return null
  })
  ipcMain.handle('tabs:close_host', (e) => {
    assertShell(e).close()
    return null
  })
  // 标签栏的「设置」按钮：与单窗口模式的最小化左侧那个按钮同一个入口，
  // 命令名 `open_settings` 与 shell.html / shell-preload 白名单 / 这里三处逐字一致。
  ipcMain.handle('tabs:open_settings', (e) => {
    assertShell(e)
    deps.openSettings()
    return null
  })
  ipcMain.handle('tabs:history_nav', (e, args: { direction?: unknown } | undefined) => {
    assertShell(e)
    const dir = args?.direction
    const active = mgr().active
    if (!active) return null
    const nav = active.view.webContents.navigationHistory
    // webContents.goBack() 在 Electron 44 已 Deprecated，用 navigationHistory.goBack()
    if (dir === 'back') nav.goBack()
    else if (dir === 'forward') nav.goForward()
    return null
  })
  // 壳页面失败遮罩上的「重试」按钮（仅主进程在首签失败时注入该遮罩，页面自身不调用）。
  // 复用启动期同一条 spawnFirstTab 路径：成功撤遮罩、失败重新挂遮罩。
  ipcMain.handle('tabs:retry_first_tab', (e) => {
    assertShell(e)
    deps.spawnFirstTab()
    return null
  })
}

/**
 * 同步送注入脚本源码给 preload（仅顶层框架；2026-09 实测 titlebar.js 60,236 字节 +
 * loading.js 22,653 字节 ≈ 81KB，每次启动一次同步 IPC，开销可忽略）。
 */
function registerBootstrapIpc(deps: AppIpcDeps): void {
  ipcMain.on('web2app:bootstrap', (e) => {
    e.returnValue = {
      chromeH: deps.bootstrap.chromeH,
      shell: 'electron' as const,
      extra: deps.bootstrap.extra,
      titlebarJs: deps.bootstrap.readInject('titlebar.js') ?? '',
      loadingJs: deps.bootstrap.readInject('loading.js') ?? '',
    }
  })
}

/**
 * Basic 鉴权：直连模式下导航地址里的 userinfo 已被剥掉，凭据在这里回填。
 *
 * Tauri 版是 WebView2 原生事件 + 本地代理注入 Authorization 两条路；Electron 侧
 * Chromium 对应的是 `login` 事件。⚠️ 地址栏上不出现密码这条约束两端一致。
 */
function registerAuthHandler(deps: AppIpcDeps): void {
  app.on('login', (event, _wc, _details, authInfo, callback) => {
    if (authInfo.isProxy) {
      // 外部代理自带凭据时（http://user:pass@proxy:8080）用它
      const from = deps.proxyHttp || deps.proxyHttps
      try {
        const u = new URL(from)
        if (u.username) {
          event.preventDefault()
          callback(decodeURIComponent(u.username), decodeURIComponent(u.password))
          return
        }
      } catch {
        /* 配的不是合法 URL，落到下面的日志分支 */
      }
      console.warn('[web2app] 代理要求鉴权，但配置里没有代理凭据，已取消')
      return
    }
    const target = deps.getTarget()
    if (target?.user) {
      // 只向【入口主机】回填凭据。页面里任何第三方子资源（或被注入内容诱导出的外域
      // 401）都可能触发 login 事件；不校验主机就等于把入口站点的账号密码盲发给任意域。
      // authInfo.host 含非默认端口（与 URL.host 同形态），直接比对即可。
      let entryHost: string | null = null
      try {
        entryHost = new URL(target.url).host
      } catch {
        entryHost = null
      }
      if (entryHost && authInfo.host === entryHost) {
        event.preventDefault()
        callback(target.user, target.pass)
        return
      }
      console.warn(
        `[web2app] 鉴权主机 ${authInfo.host} 与入口主机 ${entryHost ?? '(未知)'} 不一致，拒绝回填凭据`
      )
      return
    }
    // 不留 preventDefault：交给 Chromium 默认行为（取消），并留下可排查的日志
    console.warn(`[web2app] 站点要求鉴权，但配置里没有凭据：${authInfo.host}`)
  })
}
