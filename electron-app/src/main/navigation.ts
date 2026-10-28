/**
 * 目标解析与导航装配（自 index.ts 拆出）。
 *
 * 职责：把「配置 → 目标地址（可能起 sidecar）→ 主窗口导航」这条链路连同它的
 * 运行期状态收进一个模块：
 *   · `target` —— 解析结果缓存（多页签复用同一份，不重复起 sidecar）；
 *   · `navSeq` —— 导航代际（重试连点时只有最后一次算数）；
 *   · `activeSidecar` —— 当前 sidecar 句柄（退出 / 重试必须回收，防端口泄漏 + 孤儿进程）；
 *   · `navStarted` / `splashAutoBootConsumed` —— 首次自动 bootstrap 的名额闸门。
 *
 * ⚠️ index.ts 只装配（把 window / splashPage / 路径经 deps 注进来），不碰这里的状态。
 * ⚠️ 本模块**零 Electron 运行期依赖**（BrowserWindow 仅作类型）：窗口经 getMainWindow
 *    取得，sidecar 定位经 appPath / resourcesPath 注入。
 */
import type { BrowserWindow } from 'electron'

import type { AppConfig } from './config'
import { resolveTarget, type ResolveHow, type ResolvedTarget } from './navigate'
import {
  locateSidecar,
  SidecarError,
  startSidecar,
  type SidecarHandle,
  type SidecarRequest,
} from './sidecar'
import type { SplashPage } from './splash-page'

export interface NavigatorDeps {
  /** 运行期配置（解析目标 / 起 sidecar 的参数来源） */
  cfg: AppConfig
  /** 启动页渲染器（失败态烘焙 + 重载） */
  splashPage: SplashPage
  /** 主窗口（导航目标）；getter：装配期窗口尚未创建 */
  getMainWindow: () => BrowserWindow | null
  /** `app.getAppPath()`：定位 sidecar 的打包根 */
  appPath: string
  /** `process.resourcesPath`：定位 sidecar 的 resources 目录 */
  resourcesPath: string
}

/** 加载层指令（推给页面；真实页面走 preload 的隔离世界通道）。 */
export type OverlayCmd = { kind: 'fail'; title: string; detail: string } | { kind: 'hide' }

/** [`resolveAndCache`] 的结论：成功带目标；失败带给用户看的标题与详情。 */
export type ResolveOutcome =
  | { ok: true; target: ResolvedTarget }
  | { ok: false; title: string; detail: string }

export interface Navigator {
  /** 启动 sidecar；定位不到可执行文件时抛 `missing`。探针的 M2 断言也走这一条。 */
  launchSidecar(req: SidecarRequest): Promise<SidecarHandle>
  /** 停掉当前 sidecar（幂等；退出 / 重试路径回收用）。 */
  stopActiveSidecar(): void
  /** 解析（或命中缓存）目标地址，**不导航任何窗口**。 */
  resolveAndCache(): Promise<ResolveOutcome>
  /** 多页签：内容视图拿默认 target（解析 + 缓存），导航交给调用方。 */
  resolveInitialTarget(): Promise<ResolvedTarget>
  /** splash 加载完成 → 首次自动解析导航（一生只自动发生一次）。 */
  autoBootFromSplash(): void
  /** 启动页「重试」按钮：清掉缓存后重新走 bootstrap。 */
  retryFromSplash(): void
  /** 解析目标并把主窗口导航过去（单窗口模式的主链路）。 */
  bootstrap(): Promise<void>
  /** 把加载层指令推给主窗口页面。 */
  sendOverlay(cmd: OverlayCmd): void
  /** 当前缓存的目标（app-ipc 的 `getTarget`）；未解析时为 null。 */
  getTarget(): ResolvedTarget | null
}

/** 解析路径 → 启动日志里的人类可读文案。 */
function howLabel(how: ResolveHow): string {
  return {
    direct: '直连',
    proxy: '本地反向代理',
    dns: '域名解析后直连',
    'dns+proxy': '域名解析 + 本地反向代理',
  }[how]
}

export function createNavigator(deps: NavigatorDeps): Navigator {
  const { cfg, splashPage, getMainWindow, appPath, resourcesPath } = deps

  /** 解析结果缓存（多页签复用；重试路径显式置 null）。 */
  let target: ResolvedTarget | null = null
  /** 目标地址解析 + 首次导航只允许发生一次（重试路径会显式重置） */
  let navStarted = false
  /**
   * 「splash 加载完成 → 自动 bootstrap」一生只允许自动发生**一次**。
   *
   * ⚠️ 没有它会出现紧密无限重试循环：bootstrap 失败分支会重新 `loadURL(splash://…)`
   * （为了把失败态刷给启动页），而 splash 每次 did-finish-load 又自动再 bootstrap ——
   * 失败 → 重载 → 再解析/再起 sidecar → 再失败……无计数、无退避，sidecar 缺失时是紧密
   * 循环，DNS 模式则每轮新 spawn 一个 w2a-proxy.exe。失败后必须**等用户点重试**
   *（splash /retry 处理器显式调 bootstrap，不经过这个自动名额）。
   */
  let splashAutoBootConsumed = false
  /**
   * 当前活跃的 sidecar（本地反向代理 / DNS 解析）。
   * ⚠️ 必须显式回收：它常驻监听 127.0.0.1 端口，留着不管是「端口泄漏 + 孤儿进程」。
   */
  let activeSidecar: SidecarHandle | null = null
  /** 导航代际：并发触发 bootstrap 时，只有最后一次的结果算数（中途起的 sidecar 会被回收）。 */
  let navSeq = 0

  function stopActiveSidecar(): void {
    const s = activeSidecar
    activeSidecar = null
    if (!s) return
    console.log(`[web2app] 停止 sidecar（pid=${s.pid}）`)
    try {
      s.stop()
    } catch (e) {
      console.warn(`[web2app] 停止 sidecar 失败：${(e as Error).message}`)
    }
  }

  /**
   * 启动 sidecar；定位不到可执行文件时抛 `missing`。
   *
   * ⚠️ 这里**绝不**退回直连：需要 sidecar 的两种配置（本地代理 / 域名解析）一旦静默降级，
   * 表现就是「加密解析没生效但页面照常打开」，排查时根本不会往这个方向想。
   */
  function launchSidecar(req: SidecarRequest): Promise<SidecarHandle> {
    const exe = locateSidecar(appPath, resourcesPath)
    if (!exe) {
      return Promise.reject(
        new SidecarError(
          '未找到 w2a-proxy；开发态请先在 webview-app 下执行 ' +
            '`cargo build --release -p w2a-proxy`（打包态应为 resources/sidecar/w2a-proxy.exe）',
          'missing'
        )
      )
    }
    console.log(`[web2app] 启动 sidecar：${exe}`)
    return startSidecar({
      ...req,
      exe,
      // sidecar 的 stderr 直接并进主进程日志，排障时两条日志在同一个文件里前后相邻
      log: (line) => console.log(line),
    }).then((h) => {
      activeSidecar = h
      return h
    })
  }

  /**
   * 解析目标地址，成功则记「目标地址」日志。**读缓存、不写缓存**：
   *
   * ⚠️ 写缓存（`target = …`）由调用方在各自的校验通过后进行 —— bootstrap 有代际校验
   *    （`seq !== navSeq` 那一轮作废），把写缓存放进这里会让被作废的那一轮把 `target`
   *    覆盖成「sidecar 已被回收」的失效地址（重试连点 + 并发返回时可复现）。
   * ⚠️ 缓存短路（`target` 已有值直接返回）是多页签复用的前提；bootstrap 的两个调用点
   *    （首启自动 / splash 重试前显式 target=null）进入时 target 恒为 null，短路不改变行为。
   */
  async function resolveAndCache(): Promise<ResolveOutcome> {
    if (target) return { ok: true, target }
    const r = await resolveTarget(cfg, { startSidecar: launchSidecar })
    if (!r.ok) return { ok: false, title: r.title, detail: r.detail }
    console.log(
      `[web2app] 目标地址：${r.url}（${howLabel(r.how)}${r.user ? '，已记录 Basic 鉴权凭据' : ''}）`
    )
    return { ok: true, target: r }
  }

  /**
   * 解析并缓存目标地址，但**不导航任何窗口**。
   *
   * 多页签模式专用：首个内容视图需要拿到解析后的 target（可能起 sidecar），但导航目标是
   * **内容视图**，而不是壳页面（`bootstrap()` 末尾会把主窗口导航走，页签模式下会污染壳）。
   * 这里只做 bootstrap 的前半段（解析 + 缓存 target + 处理失败态），导航交给调用方。
   *
   * ⚠️ 与 bootstrap 共享 target 缓存：页签复用同一份解析结果，不重复起 sidecar。
   */
  async function resolveInitialTarget(): Promise<ResolvedTarget> {
    const o = await resolveAndCache()
    if (!o.ok) {
      console.error(`[web2app] ${o.title}：${o.detail}`)
      throw new Error(o.detail)
    }
    target = o.target
    return o.target
  }

  async function bootstrap(): Promise<void> {
    const win = getMainWindow()
    if (!win || win.isDestroyed()) return
    const seq = ++navSeq

    // 重试场景：上一轮的 sidecar 可能还占着端口，先收掉（幂等）
    stopActiveSidecar()

    const o = await resolveAndCache()

    if (seq !== navSeq) {
      // 期间又发起了一次（重试连点）：本次结果作废，把它起的 sidecar 一并回收
      if (o.ok) o.target.sidecar?.stop()
      return
    }

    if (!o.ok) {
      console.error(`[web2app] ${o.title}：${o.detail}`)
      splashPage.setPendingFail({ title: o.title, detail: o.detail })
      // 重新灌一遍启动页（带着失败态），用户至少能看到发生了什么并点重试
      navStarted = false
      void win.loadURL('splash://localhost/').catch(() => {})
      return
    }
    // 注：曾经这里有一条 `if (target && target.url === r.url) return` 防重复导航，但
    // bootstrap 的两个调用点（首次自动启动 / splash /retry[显式 target=null]）进入时
    // target 恒为 null，该分支不可达；真正的并发去重由 splashAutoBootConsumed 负责。
    target = o.target

    // 不 await：失败由 did-fail-load 统一处理（loadURL 的 rejection 在「被后续导航中断」时
    // 也会触发，ERR_ABORTED 不是错误）。这里只记日志，绝不静默吞掉。
    win.loadURL(o.target.url).catch((e: Error) => {
      console.warn(`[web2app] loadURL 被拒绝（详情以 did-fail-load 为准）：${e.message}`)
    })
  }

  return {
    launchSidecar,
    stopActiveSidecar,
    resolveAndCache,
    resolveInitialTarget,
    autoBootFromSplash(): void {
      // 仅【首次】splash 加载自动解析；bootstrap 失败后重载 splash 走到这里时不再自动
      // 续跑（否则失败→重载→自动重试构成无限循环），等用户点启动页上的「重试」。
      if (navStarted) return
      if (splashAutoBootConsumed) return
      splashAutoBootConsumed = true
      navStarted = true
      void bootstrap()
    },
    retryFromSplash(): void {
      navStarted = false
      target = null
      void bootstrap()
    },
    bootstrap,
    sendOverlay(cmd: OverlayCmd): void {
      const win = getMainWindow()
      if (!win || win.isDestroyed()) return
      win.webContents.send('web2app:overlay', cmd)
    },
    getTarget(): ResolvedTarget | null {
      return target
    },
  }
}
