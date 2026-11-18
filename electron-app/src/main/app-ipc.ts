/**
 * 主进程 IPC / 鉴权事件注册（2026-10-01 从 index.ts 抽出）。
 *
 * 这里集中五组注册：
 *   1. win:*     标题栏窗口按钮（单窗口 / 页签壳共用，特权命令带 sender 校验）
 *   2. tabs:*    壳页面页签操作（命令名与 Tauri 侧逐字一致）
 *   3. web2app:bootstrap  同步给 preload 下发注入脚本源码
 *   4. app 'login'        Basic 凭据回填（仅入口主机 / 代理）
 *   5. w2a:pwd-*          网页登录账号密码的查询 / 保存 / 撤销（Electron 自研，见 credentials.ts）
 *
 * 边界：本模块**不持有状态**。主窗口、页签管理器、入口目标在 index.ts 里都是「先声明、
 * ready 后才有值」的可变状态，这里一律以 getter 注入；注册动作发生在 whenReady 之后。
 */
import { app, BrowserWindow, ipcMain } from 'electron'

import type { AppConfig } from './config'
import type { CredentialStore, SaveResult } from './credentials'
import type { ResolvedTarget } from './navigate'
import type { TabManager } from './tabs'
import { assertTrustedSender } from './ipc-guard'
import { canHistoryGoBack, canHistoryGoForward } from './page-behavior'

export interface AppIpcDeps {
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
  /**
   * 内容页 sender 判据：单窗口模式 = 主窗 webContents；页签模式 = 任一页签内容视图。
   * ⚠️ 不要用 `BrowserWindow.fromWebContents()` 代替：页签视图也会解析到宿主窗口。
   */
  isContentSender: (webContentsId: number) => boolean
  /** 账号密码存储（Electron 自研，见 credentials.ts） */
  credentials: CredentialStore
}

export function registerAppIpc(deps: AppIpcDeps): void {
  registerWindowIpc(deps)
  registerTabsIpc(deps)
  registerBootstrapIpc(deps)
  registerTabExtraIpc(deps)
  registerCredentialIpc(deps)
  registerAuthHandler(deps)
}

/** 可信宿主（主窗）的 webContents.id；窗口未就绪 / 已销毁时为 null（闸门一律拒绝）。 */
function expectedSenderId(deps: AppIpcDeps): number | null {
  const w = deps.getMainWindow()
  return w && !w.isDestroyed() ? w.webContents.id : null
}

function registerWindowIpc(deps: AppIpcDeps): void {
  const winOf = (e: Electron.IpcMainInvokeEvent) => BrowserWindow.fromWebContents(e.sender)

  // ⚠️⚠️ sender 闸门在**两种模式**下都校验（2026-11-07 架构审查 H-3）：
  //   多页签模式下，内容视图（子 WebContentsView）的 webContents 经
  //   `BrowserWindow.fromWebContents()` **也会解析到宿主窗口**（两者 getType() 都是 'window'，
  //   实测见设计文档 §3.5-2），所以仅靠 fromWebContents 无法区分「壳发的」还是「内容页发的」。
  //   单窗口模式此前认为「只有一个窗口」而跳过校验 —— 但安全边界应按「sender 是否可信」定义，
  //   而不是按「是否多页签」定义：一旦误把带 invoke 的 preload 挂到内容视图 / 子框架，
  //   远程页面即可关窗、开设置，且无日志。现在两种模式统一要求
  //   `sender.id === 主窗 webContents.id`，不匹配**记日志并拒绝**（绝不允许静默失效）。
  const assertPrivileged = (e: Electron.IpcMainInvokeEvent): void => {
    assertTrustedSender(e.sender.id, expectedSenderId(deps), { label: '窗口命令' })
  }

  // ⚠️ 三个都用 handle（而不是 on）：调用失败要能变成 Promise rejection 传到页面，注入脚本
  // 会把它打到控制台。用 on 的话「按钮点了没反应」在页面侧完全无声（Tauri 版栽过这个坑）。
  ipcMain.handle('win:minimize', (e) => {
    assertPrivileged(e)
    const w = winOf(e)
    if (!w) throw new Error('找不到窗口')
    w.minimize()
  })
  ipcMain.handle('win:toggle-maximize', async (e) => {
    assertPrivileged(e)
    const w = winOf(e)
    if (!w) throw new Error('找不到窗口')
    if (w.isMaximized()) w.unmaximize()
    else w.maximize()
    // 最大化 / 还原是异步落到窗口上的：先让出一轮再读，否则可能拿到切换前的旧值
    //（2026-11-07 架构审查 L-3）。
    await new Promise<void>((resolve) => setImmediate(resolve))
    return w.isMaximized()
  })
  ipcMain.handle('win:close', (e) => {
    assertPrivileged(e)
    const w = winOf(e)
    if (!w) throw new Error('找不到窗口')
    w.close()
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
    assertTrustedSender(e.sender.id, expectedSenderId(deps), { label: '页签命令' })
    return deps.getMainWindow()!
  }
  const mgr = () => {
    const t = deps.getTabManager()
    if (!t) throw new Error('多页签未启用')
    return t
  }

  ipcMain.handle('tabs:get_tabs', (e) => {
    assertShell(e)
    // 返回元组数组 [label,title,active,home]，对齐 Tauri get_tabs 的
    // Vec<(String,String,bool,bool)>（四元组：漏掉 home 时首页会退化成普通页签，见 listForIpc）
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
    // webContents.goBack() 在 Electron 44 已 Deprecated，用 navigationHistory；
    // 且必须先判定落点：相邻条目是内置 splash 启动页时不触发（与 titlebar.js 同规则）。
    const entries = nav.getAllEntries()
    const index = nav.getActiveIndex()
    if (dir === 'back') {
      if (canHistoryGoBack(entries, index)) nav.goBack()
    } else if (dir === 'forward') {
      if (canHistoryGoForward(entries, index)) nav.goForward()
    }
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
    // 与窗口命令同一条闸：只有主窗 webContents 能取注入源码（约 80KB + extra），
    // 且必须是**顶层框架** —— preload 默认对子框架也执行（见 renderer/preload.ts 的
    // `window.self === window.top` 判断），这里补主进程侧的同一道闸（2026-11-07 H-3）。
    try {
      assertTrustedSender(e.sender.id, expectedSenderId(deps), { label: 'bootstrap 命令' })
    } catch {
      // on 的处理器抛错会变成未捕获异常：这里必须吞掉并给 sendSync 一个明确返回。
      e.returnValue = null
      return
    }
    if (e.senderFrame !== e.sender.mainFrame) {
      console.warn('[web2app] 拒绝子框架读取 bootstrap 注入源码')
      e.returnValue = null
      return
    }
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
 * 页签内容页取 `window.web2appExtra`（消费方 `renderer/tab-preload.ts`）。
 *
 * 为什么另起一条通道：页签模式下内容页是独立 `WebContentsView`，其 webContents **不是**
 * 主窗（`expectedSenderId`），不能复用 `web2app:bootstrap` 的 sender 闸；而它又只需要 extra
 * 这一份数据，不该为它同步拉约 81KB 的注入脚本。
 *
 * 闸门只到「顶层框架」：本通道返回的 `extra` 本来就是**要注入远程页面**的公开配置，不涉及
 * 任何窗口能力（关闭 / 设置 / 页签命令仍在 `assertTrustedSender` 之后），因此没有越权面。
 */
function registerTabExtraIpc(deps: AppIpcDeps): void {
  ipcMain.on('w2a:tab-extra', (e) => {
    if (e.senderFrame !== e.sender.mainFrame) {
      console.warn('[web2app] 拒绝子框架读取 web2appExtra')
      e.returnValue = null
      return
    }
    e.returnValue = deps.bootstrap.extra
  })
}

/**
 * 网页登录账号密码：查询 / 保存 / 撤销（Electron 自研，preload 消费）。
 *
 * 为什么不用 `assertTrustedSender`（只认宿主 webContents）：页签模式下内容页是独立
 * `WebContentsView`，它本来就不是宿主；但本组命令只回传**该页面自己 origin** 的凭据，
 * 不接受渲染进程指定 origin，所以闸门改用「顶层框架 + 内容页 sender」。
 *
 * ⚠️ origin 一律由主进程从 `e.senderFrame.url` 推导，页面无法伪造；
 *    子框架（跨源 iframe）直接拒绝 —— preload 默认也不进子框架，这里是第二道闸。
 */
function registerCredentialIpc(deps: AppIpcDeps): void {
  const assertContentSender = (e: Electron.IpcMainInvokeEvent): string => {
    if (e.senderFrame !== e.sender.mainFrame) {
      console.warn('[web2app] 拒绝子框架的账号密码命令')
      throw new Error('拒绝：账号密码命令只能由顶层框架发起')
    }
    if (!deps.isContentSender(e.sender.id)) {
      console.warn(`[web2app] 拒绝账号密码命令（sender.id=${e.sender.id}）`)
      throw new Error('拒绝：账号密码命令只能由内容页发起')
    }
    return e.senderFrame.url
  }

  ipcMain.handle('w2a:pwd-lookup', (e) => {
    const url = assertContentSender(e)
    return deps.credentials.lookup(url)
  })

  ipcMain.handle('w2a:pwd-save', (e, args: unknown) => {
    const url = assertContentSender(e)
    const o = (args ?? {}) as { username?: unknown; password?: unknown }
    const username = typeof o.username === 'string' ? o.username.slice(0, 256) : ''
    const password = typeof o.password === 'string' ? o.password : ''
    // 长度上限是防御性的（不是业务校验）：别让一个恶意页面把整个文件写爆。
    if (password === '' || password.length > 1024) {
      return { ok: false, reason: 'invalid' } satisfies SaveResult
    }
    return deps.credentials.save(url, { username, password })
  })

  ipcMain.handle('w2a:pwd-forget', (e) => {
    const url = assertContentSender(e)
    deps.credentials.forget(url)
    return null
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
