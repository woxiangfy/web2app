/**
 * 主进程入口（M1：窗口复刻 + 导航接入 + 兼容契约探针）。
 *
 * ⚠️ 顺序约束（改这个文件时务必保持）：
 *   1. setupPortablePaths()  —— 必须早于任何会读取 userData 的调用
 *   2. app.commandLine.appendSwitch 外部代理 —— 必须早于 app ready
 *   3. protocol.registerSchemesAsPrivileged —— 必须早于 app ready
 *   4. requestSingleInstanceLock() —— 单实例锁文件落在 userData 下，故在第 1 步之后
 */
import { app, BrowserWindow, protocol } from 'electron'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { setupPortablePaths } from './portable'
import { isInstallerBuild } from './distribution'
import {
  buildProxySwitches,
  debugFlagTruthy,
  extraJson,
  loadConfigAny,
  sanitizeDirName,
  type AppConfig,
} from './config'
import { needsSidecar, resolveTarget, type ResolveHow, type ResolvedTarget } from './navigate'
import {
  locateSidecar,
  SidecarError,
  startSidecar,
  type SidecarHandle,
  type SidecarRequest,
} from './sidecar'
import { runProbe, runTabsProbe, startProbeServer, type ProbeServer } from './probe'
import {
  loadSettings,
  saveSettings,
  SETTINGS_FILE,
  storeLocation,
  type Settings,
} from './settings'
import { hotkeyLabel } from './hotkey'
import { createSettingsController } from './settings-controller'
import { registerProtocols } from './protocols'
import {
  captureWindowState,
  loadWindowState,
  planRestore,
  saveWindowState,
  type RestorePlan,
} from './window-state'
import {
  destroyTray,
  hasTray,
  hideMainWindow,
  locateTrayIcon,
  showMainWindow,
  toggleMainWindow,
} from './tray'
import { applyWakeHotkey, registerSettingsShortcut, releaseShortcuts } from './shortcuts'
import {
  closeSettingsWindow,
  getSettingsWindow,
  openSettingsWindow,
  readSettingsHtml,
  registerSettingsIpc,
  updateSettingsWindowFloat,
} from './settings-window'
import { MAX_TABS, TAB_BAR_HEIGHT, TabManager } from './tabs'
import { hideShellStartupError, showShellStartupError } from './shell-overlay'
import { rawConsole, silenceConsole, startFileLogging, type FileLogHandle } from './startup-log'
import { createSplashPage, SPLASH_BG } from './splash-page'
import { runSmokeChecks } from './smoke'
import { registerAppIpc } from './app-ipc'
import {
  attachContextMenu,
  attachDevtools,
  attachDownload,
  devtoolsEnabled,
} from './page-behavior'
import { clearWebData } from './web-data'
import {
  destroyFloatWindow,
  isFloatAlive,
  persistFloatGeometry,
  registerFloatWindow,
  showFloatWindow,
  toggleFloatWindow,
} from './float-window'

/** 命令行开关。
 *
 *  npm scripts 里跨 shell 设环境变量很别扭（cmd 不认 `VAR=1 cmd`），所以同时接受 `--flag`；
 *  环境变量保留给 CI / 手动一次性覆盖。
 *
 *  ⚠️⚠️ 排查前提：**本项目的 electron 一律经 `scripts/launch.cjs` 拉起**
 *（`pnpm run start/smoke/probe` 都已如此）。直接 `npx electron .` 时，若宿主注入了
 *  `ELECTRON_RUN_AS_NODE=1`（桌面型 IDE / agent 沙箱会这么做），Electron 会以**纯 Node
 *  身份**启动：`require('electron')` 得到的是路径字符串，紧接着这里就抛
 *  `Cannot read properties of undefined (reading 'disableHardwareAcceleration')` ——
 *  错误位置落在业务代码里，看上去像我们自己的 bug，实际是环境问题。
 *  判别方法：`electron.exe --version` 打印 Node 版本（`v24.x`）而不是 `v44.x`。
 *  2026-09-16 在这上面绕了很久，完整记录与修法见 scripts/launch.cjs。
 *
 *  另：自定义开关不要用 `--no-xxx` 形式 —— Node 的选项解析把 `--no-X` 当作「把 X 设为
 *  false」，与既有开关语义容易撞车。一律用正向的 `--flag`。
 *
 *  已定义的开关：`--probe`（兼容契约探针）、`--tabs`（配合 probe 走页签断言组）、
 *  `--smoke`（启动自检后退出）、`--debug`（打开诊断日志，见下方 DEBUG）。
 */
const argFlags = new Set(process.argv.slice(1))
const hasFlag = (f: string): boolean => argFlags.has(f)

/** 探针模式：起本地夹具站点、逐条断言兼容契约，退出码即结论 */
const PROBE = process.env.WEB2APP_PROBE === '1' || hasFlag('--probe')

/** 多页签探针模式：在探针基础上以页签模式起窗，跑页签断言组（`runTabsProbe`）。 */
const TABS_PROBE = PROBE && hasFlag('--tabs')

/** 冒烟自检：打印生效路径 / 配置 + 断言启动页模板替换完整，然后退出 */
const SMOKE = process.env.WEB2APP_SMOKE === '1' || hasFlag('--smoke')

// ⚠️ 无 GPU 环境（容器 / 远程桌面 / 部分虚拟机）Chromium 会直接以
// "GPU process isn't usable. Goodbye." 退出，窗口都不出来。探针与 CI 一律关掉硬件加速；
// 真机不要关（关了滚动与动画会明显变卡）。
if (PROBE || process.env.WEB2APP_NO_GPU === '1' || hasFlag('--disable-gpu')) {
  app.disableHardwareAcceleration()
}

// ---------- 0. 注入脚本单一真源：优先打包副本，其次 webview-app/src ----------
// 打包后 resources/inject/ 已**不再存在**（2026-09-26 改造）：注入脚本 / 壳页面
// （titlebar.js / loading.js / splash.html / shell.html）由 stage-assets.cjs 拷贝到
// `out/inject/`，随 asar 封进包内（app.asar/out/inject/），与 config.json 同等待遇 ——
// 明文放在 asar 外等于注入逻辑形同虚设，任何人都能改掉标题栏 / 加载层 / 多页签壳，
// 再原样分发出去而使用者看不出区别。现在想改必须重新打包（与 Tauri 版一致）。
// 开发时（electron .）直接读 webview-app/src，避免复制出第二份会分叉的脚本。
// ⚠️ 用 `app.isPackaged` 区分而非 `existsSync`：stage 也会在开发目录生成 out/inject/，
//    若按「存在即打包副本」判，开发态会误读 stage 副本、失去单一真源。
function injectDir(): string {
  if (app.isPackaged) return join(app.getAppPath(), 'out', 'inject')
  return join(__dirname, '..', '..', '..', 'webview-app', 'src')
}

/**
 * 悬浮球内软件图标（icon.png）的路径：打包态在 out/inject/（stage 同源拷贝），
 * 开发态直接读单一真源 webview-app/icons/icon.png（floaticon.html 在 src/，
 * 两目录不同级，故不能靠 loadFile 的相对路径，必须走 floaticon:// 协议分发）。
 */
function floatIconPath(): string {
  if (app.isPackaged) return join(app.getAppPath(), 'out', 'inject', 'icon.png')
  return join(__dirname, '..', '..', '..', 'webview-app', 'icons', 'icon.png')
}

const injectCache = new Map<string, string | null>()

function readInject(name: string): string | null {
  const hit = injectCache.get(name)
  if (hit !== undefined) return hit
  let out: string | null = null
  try {
    out = readFileSync(join(injectDir(), name), 'utf8')
  } catch {
    out = null
  }
  injectCache.set(name, out)
  return out
}

// ---------- 1. 运行期配置：读自包内 asar（不可读改） ----------
//
// ⚠️ 为什么**不再**从 exe 同级读 `config.json`：
//    那份是明文、可读、可改的。放在包外等于配置形同虚设 —— 任何人都能改掉 app_url、
//    或关掉域名校验，再把包分发出去，而使用者看不出任何区别。
//    现在它随 `out/config.json` 打进 `app.asar`，想改配置就得重新打包（与 Tauri 版一致）。
//
// - 打包后：`app.getAppPath()` = `…\resources\app.asar`，配置在 `app.asar\out\config.json`
//   （Electron 的 fs 对 asar 透明，`readFileSync` 直接可读）；
// - 开发态：项目根目录的 `config.json`（`pnpm run stage` 会把它同步到 `out/`）。
function configRoot(): string {
  return app.isPackaged ? join(app.getAppPath(), 'out') : join(__dirname, '..', '..')
}

const loaded = loadConfigAny([configRoot()])
const cfg: AppConfig = loaded.config

/**
 * 诊断日志开关（**默认关**）。
 *
 * 判据（任一成立即开启）：
 *   · `config.json` 的 `debug` —— 打包期定下来，改它必须重新打包；
 *   · `--debug` / 环境变量 `WEB2APP_DEBUG=1`（真值集合 1/true/yes，与 Tauri 壳逐字一致，
 *     见 config.debugFlagTruthy）—— 运行时逃生口，交付出去的包也能临时取证，不必重新打包；
 *   · 探针 / 冒烟模式**强制打开** —— 它们存在的意义就是取证，探针的「退出码即结论」与冒烟
 *     的路径断言全都靠这些输出，静默会让自检退化成「永远绿」。
 */
const DEBUG =
  cfg.debug || debugFlagTruthy(process.env.WEB2APP_DEBUG) || hasFlag('--debug') || PROBE || SMOKE

// ⚠️ 静默要**尽可能早**：晚一步就会漏掉 setupPortablePaths 那类启动早期输出。
//    此后 `console.*` 全是空操作，只有 rawConsole 还能出声（见 startup-log.ts）。
// ⚠️ 配置解析在上一步（loadConfigAny）**之前**完成，所以「config.json 坏了」那条致命
//    报错不会被静默吞掉 —— 那是必须让人看到的一类错误。
if (!DEBUG) silenceConsole()

// 安装版（NSIS，打包期由 -c.extraMetadata.distribution=nsis 在 package.json 里打标）：
// setupPortablePaths 内部强制走系统默认数据目录，cfg.data_dir 被刻意忽略。
const installerBuild = isInstallerBuild(app.getAppPath())
const paths = setupPortablePaths({
  dataDir: cfg.data_dir,
  appName: sanitizeDirName(cfg.app_name),
  forceSystem: installerBuild,
})

// ---------- 1.1 settings.json 的位置（M3） ----------
// ⚠️ 这是**运行期唯一**的一份 JSON：设置的三个开关 + `window` 段的窗口几何同住一个文件
//    （2026-09-23 起；此前几何单独占一个 window-state.json）。
//    **落点 = 数据目录**（`<data_dir>\settings.json`，即 `paths.dataDir` 下），与 `data_dir`
//    字段的语义完全一致（`docs/packaging-config.md` §4.3）：绿色版把 exe 搬走时，设置与
//    cookie / 缓存 / 日志**一起**跟着走；Tauri 版同规则、同位置（`config.rs::store_dir`）。
//    旧位置 `%APPDATA%\<app_name>\settings.json` 只作**读取候选**（升级兼容）——
//    新位置还没写过时读它，第一次写盘即落到新位置。细节与取舍见 src/main/settings.ts 的文件头。
const storeOpts = {
  dataDir: paths.dataDir,
  appDataDir: app.getPath('appData'),
  appName: cfg.app_name,
}
let storeLoc = storeLocation({ ...storeOpts, file: SETTINGS_FILE })

// ⚠️⚠️ 探针**绝不能**读写用户的真实配置：它会一路走到「保存设置 / 开托盘 / 抢占快捷键 /
//    记窗口几何」，跑一次 `pnpm run probe` 就顺手改掉用户的托盘开关和窗口尺寸是不可接受的。
//    所以探针模式把这唯一的配置文件整体改到 %TEMP% 下的隔离目录，真实文件一个字节都不碰。
if (PROBE) {
  const tmpStore = join(app.getPath('temp'), 'w2a-probe-store')
  storeLoc = { read: [join(tmpStore, SETTINGS_FILE)], write: join(tmpStore, SETTINGS_FILE) }
}

// ---------- 1.2 文件日志（M6）：**只在 debug 打开时才有** ----------
// ⚠️ 为什么它存在：Windows 上 electron.exe 是 GUI 子系统程序，用户双击运行时
//    `console.log` 没有任何去处；portable 更是被自解压包装器吞掉内层 stdout。
//    没有那份文件，「双击没反应 / 打开的是 example.com / 标题不对」在用户侧零证据。
// ⚠️⚠️ 但「每次启动都往用户数据目录里留一份日志」本身不是期望行为（2026-09-22 改为可选）：
//    debug=false（默认）时**连 logs 目录都不建**、不落文件、也不输出任何诊断。
//    代价是上面那条「用户侧第一现场」在默认配置下不存在 —— 排障时用 `--debug`（或环境
//    变量 WEB2APP_DEBUG=1）临时打开，不必重新打包。
// ⚠️ 探针**不得**往真实目录写：与上面两个 JSON 同理，在打包产物上跑一次 `--probe`
//    就会在用户 exe 旁边留下 `data\logs\`，污染交付物（`data\session` 已经踩过一次）。
const logDir = PROBE
  ? join(app.getPath('temp'), 'w2a-probe-store')
  : join(paths.dataDir, 'logs')
const fileLog: FileLogHandle | null = DEBUG ? startFileLogging(logDir) : null

const loadedSettings = loadSettings(storeLoc)
/** 运行期当前生效的设置（设置窗保存后立即更新） */
let runtimeSettings: Settings = loadedSettings.settings

/** 主窗口是否正在被销毁（区分「关闭到托盘」与「真的退出」） */
let quitting = false

// ---------- 2. 外部代理 / 本地反向代理的决策（对齐 webview-app 三态语义） ----------
//   proxy_enabled=false                    → 直连
//   proxy_enabled=true 且 proxy_http(s) 非空 → 外部代理（Chromium 原生 switch）
//   proxy_enabled=true 且两者都为空         → 自身本地反向代理（M2 接 sidecar）
const proxy = buildProxySwitches(cfg)
if (proxy.proxyServer) {
  app.commandLine.appendSwitch('proxy-server', proxy.proxyServer)
  if (proxy.proxyBypassList) {
    app.commandLine.appendSwitch('proxy-bypass-list', proxy.proxyBypassList)
  }
}

// ---------- 3. 内部自定义协议（对齐 Tauri 版的 splash:// 与 settings://） ----------
// 必须在 ready 之前声明；secure=true 让它们成为安全上下文，行为与 Tauri 侧一致。
//   splash://localhost/   启动页（加载动画 + 失败态 + 重试）
//   settings://localhost/ 设置窗（供 webview-app/dist/settings.html 使用）
protocol.registerSchemesAsPrivileged([
  { scheme: 'splash', privileges: { standard: true, secure: true, supportFetchAPI: true } },
  { scheme: 'settings', privileges: { standard: true, secure: true, supportFetchAPI: true } },
  { scheme: 'shell', privileges: { standard: true, secure: true, supportFetchAPI: true } },
  // 悬浮球页 / 球内软件图标（与 Tauri floaticon:// 完全同 URL、同语义）。
  { scheme: 'floaticon', privileges: { standard: true, secure: true, supportFetchAPI: true } },
])

/**
 * 启动页渲染器（占位符替换 + 失败态烘焙），实现与契约见 `splash-page.ts`。
 * 实例化必须晚于 readInject 定义；pendingFail 状态封在渲染器内部。
 */
const splashPage = createSplashPage({ title: cfg.title, readInject })

// ---------- 4. 单实例 ----------
const gotLock = app.requestSingleInstanceLock()

let mainWindow: BrowserWindow | null = null
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

/** 多页签管理器（仅 tabs_enabled=true 时非空）。 */
let tabManager: TabManager | null = null

/**
 * 是否多页签模式（只在最外层读一次，禁止在既有函数里散落 if）。
 *
 * ⚠️⚠️ **探针模式下必须让「起窗模式」与「断言组」一一对应**，因此 `cfg.tabs_enabled`
 *    **不参与**探针分叉：
 *      - `--probe`         → 强制单窗口模式，跑 134 条单窗口断言（按「主窗口加载
 *                            splash:// + 标题栏/滚动条/窗口按钮」设计）；
 *      - `--probe --tabs`  → 强制页签模式，跑 31 条页签断言（含 3 页签邻接选择）。
 *    若写成 `cfg.tabs_enabled || TABS_PROBE`，则用户一旦把 config.json 的页签开关打开
 *    （正是多页签版交付形态），同一条 `--probe` 会以**页签模式**起窗：单窗口断言往壳页面
 *    （`shell://`）注入脚本会直接抛 `Error: Script failed to execute`，回归门禁退化成
 *    「看用户当前配置过不过」，且报错完全不指向根因（2026-09-18 实测：打包产物
 *    `--probe --tabs` 退 0，同包 `--probe` 退 1）。
 *    非探针模式下行为不变：由 `tabs_enabled` 决定。
 */
const TABS_MODE = TABS_PROBE ? true : PROBE ? false : cfg.tabs_enabled

/**
 * 当前活跃的 sidecar（本地反向代理 / DNS 解析）。
 * ⚠️ 必须显式回收：它常驻监听 127.0.0.1 端口，留着不管是「端口泄漏 + 孤儿进程」。
 */
let activeSidecar: SidecarHandle | null = null

/** 停掉当前 sidecar（幂等）。 */
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
  const exe = locateSidecar(app.getAppPath(), process.resourcesPath)
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

/** 导航代际：并发触发 bootstrap 时，只有最后一次的结果算数（中途起的 sidecar 会被回收）。 */
let navSeq = 0

/** 解析路径 → 启动日志里的人类可读文案（单一真源，两处调用点共用）。 */
function howLabel(how: ResolveHow): string {
  return {
    direct: '直连',
    proxy: '本地反向代理',
    dns: '域名解析后直连',
    'dns+proxy': '域名解析 + 本地反向代理',
  }[how]
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
  if (target) return target
  const r = await resolveTarget(cfg, { startSidecar: launchSidecar })
  if (!r.ok) {
    console.error(`[web2app] ${r.title}：${r.detail}`)
    throw new Error(r.detail)
  }
  target = r
  console.log(
    `[web2app] 目标地址：${r.url}（${howLabel(r.how)}${r.user ? '，已记录 Basic 鉴权凭据' : ''}）`
  )
  return r
}

async function bootstrap(): Promise<void> {
  const win = mainWindow
  if (!win || win.isDestroyed()) return
  const seq = ++navSeq

  // 重试场景：上一轮的 sidecar 可能还占着端口，先收掉（幂等）
  stopActiveSidecar()

  const r = await resolveTarget(cfg, { startSidecar: launchSidecar })

  if (seq !== navSeq) {
    // 期间又发起了一次（重试连点）：本次结果作废，把它起的 sidecar 一并回收
    if (r.ok) r.sidecar?.stop()
    return
  }

  if (!r.ok) {
    console.error(`[web2app] ${r.title}：${r.detail}`)
    splashPage.setPendingFail({ title: r.title, detail: r.detail })
    // 重新灌一遍启动页（带着失败态），用户至少能看到发生了什么并点重试
    navStarted = false
    void win.loadURL('splash://localhost/').catch(() => {})
    return
  }
  // 注：曾经这里有一条 `if (target && target.url === r.url) return` 防重复导航，但
  // bootstrap 的两个调用点（首次自动启动 / splash /retry[显式 target=null]）进入时
  // target 恒为 null，该分支不可达；真正的并发去重由 splashAutoBootConsumed 负责。
  target = r

  console.log(
    `[web2app] 目标地址：${r.url}（${howLabel(r.how)}${r.user ? '，已记录 Basic 鉴权凭据' : ''}）`
  )
  // 不 await：失败由 did-fail-load 统一处理（loadURL 的 rejection 在「被后续导航中断」时
  // 也会触发，ERR_ABORTED 不是错误）。这里只记日志，绝不静默吞掉。
  win.loadURL(r.url).catch((e: Error) => {
    console.warn(`[web2app] loadURL 被拒绝（详情以 did-fail-load 为准）：${e.message}`)
  })
}

/** 把加载层指令推给页面（真实页面走 preload 的隔离世界通道）。 */
function sendOverlay(cmd: { kind: 'fail'; title: string; detail: string } | { kind: 'hide' }): void {
  const win = mainWindow
  if (!win || win.isDestroyed()) return
  win.webContents.send('web2app:overlay', cmd)
}

// ---------- 6. 窗口按钮 IPC / 新窗口拦截 ----------
// 四组 IPC（win:* / tabs:* / web2app:bootstrap）与 app 'login' 鉴权注册已抽到
// app-ipc.ts；whenReady 后以 registerAppIpc(deps) 一次性注入（状态经 getter 读取）。
// 下面只留主窗口自身的 setWindowOpenHandler（建窗时挂载）。

/**
 * 新窗口请求：与 Tauri 版**逐条对齐**（webview_hooks.rs 的单窗口模式分支）。
 *
 *   · about:blank / 空地址 → 忽略并打日志（单窗口模式提供不了独立窗口，
 *     硬套会把用户当前页面变成白板）；
 *   · 其它 → `action: 'deny'` + 把当前窗口**原地导航**过去。这样新页面走普通导航路径，
 *     注入的标题栏 / 加载层与所有钩子照常生效。
 *
 * ⚠️ 代价两端一致：`window.open()` 的返回值不再是可用窗口对象。
 * ⚠️ 与 Tauri 一样不做 scheme 过滤（mailto: 之类也走原地导航 → Chromium 会拒，
 *    表现为 did-fail-load）。这是「保持一致」优先于「顺手优化」的有意取舍。
 */
function registerWindowOpenHandler(win: BrowserWindow): void {
  win.webContents.setWindowOpenHandler(({ url }) => {
    console.warn(`[web2app] 单窗口模式拦截到新窗口请求：${url}`)
    if (!url || url === 'about:blank' || url.startsWith('about:')) {
      console.warn('[web2app] 忽略无地址的 window.open()（单窗口模式无法提供独立窗口）')
      return { action: 'deny' }
    }
    try {
      new URL(url)
    } catch (e) {
      console.warn(`[web2app] 新窗口目标地址不合法，已忽略：${(e as Error).message}（${url}）`)
      return { action: 'deny' }
    }
    // 延后一拍：当前还处在渲染进程发起的 open 调用栈里，同步导航会打断它
    setImmediate(() => {
      void win.loadURL(url).catch(() => {
        /* 失败由 did-fail-load 报，别在这里静默掉 */
      })
    })
    return { action: 'deny' }
  })
}

/** 无保存几何时的默认窗口尺寸（与旧版 tauri.conf.json 的窗口配置一致）。 */
const DEFAULT_W = 1100
const DEFAULT_H = 650

/** 记下窗口当前几何（在关闭 / 隐藏到托盘前调用）。 */
function persistGeometry(win: BrowserWindow): void {
  const st = captureWindowState(win)
  if (st) saveWindowState(storeLoc, st)
}

function createWindow(show: boolean, plan: RestorePlan): BrowserWindow {
  const win = new BrowserWindow({
    width: plan.bounds?.width ?? DEFAULT_W,
    height: plan.bounds?.height ?? DEFAULT_H,
    // 只在有保存几何时给坐标：不给的话系统自己挑位置（居中/级联），与旧版首次启动一致
    ...(plan.bounds ? { x: plan.bounds.x, y: plan.bounds.y } : {}),
    // ⚠️⚠️ 必须显式给 title。下面拦了 page-title-updated，页面标题永远进不来，
    //    于是**不写这一行**的结果不是「显示页面标题」，而是永远停在 Electron 的默认值
    //    （package.json 的 `name`，本壳是 "web2app-electron"）—— 配置里的 title 完全失效。
    //    2026-09-17 实测踩到：任务栏/Alt+Tab 显示 "web2app-electron" 而不是「百度」。
    //    注释里写的「窗口标题固定为配置里的 title」在此之前只是愿望，不是事实。
    title: cfg.title,
    minWidth: 400,
    minHeight: 300,
    // 无系统标题栏，改由注入的 titlebar.js 自绘（与 Tauri 版 decorations(false) 对齐）
    frame: false,
    // 盖住 WebView 初始化与导航提交瞬间的白闪（对齐 bootstrap::SPLASH_BG）
    backgroundColor: SPLASH_BG,
    show,
    webPreferences: {
      // 多页签模式：壳页面用 shell-preload（只暴露 tabs 命令垫片，不注入 titlebar/loading）；
      // 单窗口模式：用 preload（注入 titlebar.js / loading.js）。
      preload: TABS_MODE
        ? join(__dirname, '..', 'renderer', 'shell-preload.js')
        : join(__dirname, '..', 'renderer', 'preload.js'),
      // 安全基线：全部保持默认值，任何一项都不要为省事打开（见 docs/electron-plan.md §5）
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      webviewTag: false,
      // 探针要窗口不可见也能跑：定时器被节流会让加载层与 chrome-fit 的兜底扫描失真
      backgroundThrottling: !PROBE,
    },
  })

  // 与 Tauri 版对齐：窗口标题固定为配置里的 title，不跟随 document.title。
  // （Electron 默认跟随页面标题，差异已在 docs/electron-plan.md §4 记录为「有意保持一致」）
  win.on('page-title-updated', (e) => e.preventDefault())

  registerWindowOpenHandler(win)

  /**
   * 关闭按钮语义（对齐 `lib.rs` 的 `handle_main_close`）——两个**驻留理由**并列：
   *   1. 「启用托盘」+「关闭时最小化到托盘」→ 隐藏窗口，应用继续驻留托盘；
   *   2. 「启用悬浮图标」（仅 Windows，球窗只在那时才存在）→ 隐藏窗口，应用继续驻留悬浮球；
   *   两者都不满足 → 正常关闭（最后一个窗口关闭后退出）。
   *
   * ⚠️⚠️ 第 2 条是**修一个真机缺陷**，不是锦上添花（2026-10-11 使用者实测：策略
   *    `tray_enabled:false, minimize_on_close:false, float_enabled:true`）：
   *     关闭窗口 → 主窗被**真销毁**，但球窗还活着 ⇒ ① 球窗挡住了
   *     `window-all-closed`（它只在**所有**窗口关闭时才发）⇒ 进程留成一个既看不见界面、
   *     也退不掉的僵尸；② 点球时 `toggleMainWindow → presentMainWindow → showMainWindow`
   *     开头就 `if (!win || win.isDestroyed()) return` ⇒ **一个动作都不做**，
   *     用户看到的就是「点了悬浮图标不回窗口」。球的产品语义就是「唤回主窗的入口」
   *     （见 floaticon.html 的 aria-label），故「球在 ⇒ 应用驻留」必须成立。
   *
   * ⚠️ `quitting` 标志不可省：托盘菜单「退出」走的是 `app.quit()`，
   *    它会触发窗口的 close 事件 —— 没有这个标志就会「点了退出，窗口只是藏起来了」。
   *    （设置页「退出程序」走同一条 `requestQuit`，见下方 registerSettingsIpc。）
   */
  win.on('close', (e) => {
    persistGeometry(win)
    if (quitting) return
    const trayResident = runtimeSettings.tray_enabled && runtimeSettings.minimize_on_close
    // 与 float-window.ts 建球的前置条件同一个判据（非 Windows 永远不建球 ⇒ 不得驻留）。
    const floatResident = process.platform === 'win32' && runtimeSettings.float_enabled
    if (trayResident || floatResident) {
      e.preventDefault()
      win.hide()
      console.log(
        `[web2app] 关闭按钮 → 隐藏（驻留理由：${trayResident ? '托盘' : ''}${trayResident && floatResident ? ' + ' : ''}${floatResident ? '悬浮图标' : ''}）`,
      )
    }
  })

  // 导航失败 → 把错误态推给加载层（含重试入口），不再让用户对着白屏
  win.webContents.on('did-fail-load', (_e, code, desc, url, isMainFrame) => {
    // -3 = ERR_ABORTED：正常重定向 / 被后续导航顶掉，不是错误
    if (!isMainFrame || code === -3) return
    const detail = `无法打开 ${url}（${desc}，${code}）`
    console.error(`[web2app] ${detail}`)
    sendOverlay({ kind: 'fail', title: '加载失败', detail })
  })

  // 下载接管（M4）：弹「另存为」让用户定落盘位置，对标 Tauri 的 save_dialog.rs。
  // ⚠️ session 是共享的：主窗口与所有内容视图（WebContentsView）都没设 partition，都挂在
  //   defaultSession 上，所以这里挂一次 `will-download` 即覆盖**所有**视图的下载（§7-9 的
  //   「逐视图挂载」在共享 session 前提下等价于「挂一次全局生效」）。若未来给内容视图设
  //   独立 partition，这条必须改成逐视图挂载。
  attachDownload(win.webContents.session, () => win)

  // 启动页加载完成 → 解析目标地址并导航过去（Tauri 侧同样是「启动页先出，后台准备完再跳」）
  win.webContents.on('did-finish-load', () => {
    const cur = win.webContents.getURL()
    if (cur.startsWith('splash:')) {
      if (PROBE) return // 探针自己控制导航节奏
      if (navStarted) return
      // 仅【首次】splash 加载自动解析；bootstrap 失败后重载 splash 走到这里时不再自动
      // 续跑（否则失败→重载→自动重试构成无限循环），等用户点启动页上的「重试」。
      if (splashAutoBootConsumed) return
      splashAutoBootConsumed = true
      navStarted = true
      void bootstrap()
      return
    }
  })

  win.once('ready-to-show', () => {
    // 上次是最大化（或几何已失效）→ 在显示之前先最大化，避免先闪一下小窗口
    if (plan.maximize) win.maximize()
    win.show()
  })

  // ---------- 多页签模式：挂 TabManager + 首个内容视图 ----------
  if (TABS_MODE) {
    tabManager = new TabManager({
      host: win,
      onSpawn: async (view, _label, url) => {
        // ⚠️⚠️ 新页签**不得**调 bootstrap()（其开头 stopActiveSidecar 会掐掉在服页签的代理，
        //   末尾会把主窗口导航走污染壳页面）。这里用 resolveInitialTarget 复用 target 缓存，
        //   只解析、不导航壳页面；导航目标是本内容视图。
        //   url 有值时（window.open 场景）直接用；否则用默认 target（首屏）。
        const t = url ? { url, user: '', pass: '', how: 'direct' as const } : await resolveInitialTarget()
        // 挂窗口打开处理器：内容页 window.open → 新建页签。
        // ⚠️ 与单窗口版（registerWindowOpenHandler）看着像，实则两处刻意不同、勿强行合并：
        //   1) 动作是「新建页签」（走 spawnTab → resolveTarget 的代理/DNS 完整链路），不是原地导航；
        //   2) 不做 new URL 预校验 —— spawnTab 内部解析目标时自会失败（失败进页签标题 + 遮罩），
        //      单窗口版那道校验是为了区分「原地导航失败交给 did-fail-load」才需要的。
        //   两边共享的只有 about: 拒绝这一条（1 行表达式），抽 helper 反而要参数化行为分叉。
        view.webContents.setWindowOpenHandler(({ url: opened }) => {
          console.warn(`[web2app] 页签模式拦截到新窗口请求：${opened}`)
          if (!opened || opened === 'about:blank' || opened.startsWith('about:')) {
            return { action: 'deny' }
          }
          setImmediate(() => {
            void tabManager?.spawnTab(opened).catch((e) =>
              console.error(`[web2app] 新建页签失败：${(e as Error).message}`)
            )
          })
          return { action: 'deny' }
        })
        // 标题回传：直接监听事件，无需注入脚本
        view.webContents.on('page-title-updated', (_e, title) => {
          tabManager?.setTabTitle(_label, title)
        })
        // 加载失败：内容视图没有加载层（不注入 loading.js），失败态只能体现在页签标题上。
        // ⚠️ 与单窗口的 did-fail-load 对齐「不静默」原则：记日志 + 给用户可见提示。
        //   -3 = ERR_ABORTED：正常重定向 / 被后续导航顶掉，不是错误。
        view.webContents.on('did-fail-load', (_e, code, desc, url, isMainFrame) => {
          if (!isMainFrame || code === -3) return
          const detail = `无法打开 ${url}（${desc}，${code}）`
          console.error(`[web2app] 页签加载失败：${detail}`)
          tabManager?.setTabTitle(_label, '加载失败')
        })
        // M4 页面行为：F12 / 右键菜单（与单窗口模式同一套实现，逐视图挂）
        attachDevtools(view.webContents, devtoolsEnabled(app.isPackaged, cfg.debug))
        attachContextMenu(view.webContents, openSettings)
        await view.webContents.loadURL(t.url).catch((e) => {
          console.warn(`[web2app] 内容页 loadURL 被拒绝：${(e as Error).message}`)
        })
      },
    })

    // 窗口 resize → 重排全部视图 bounds
    win.on('resize', () => tabManager?.relayout())
  }

  // ---------- M4 页面行为：F12 / 右键菜单 ----------
  // 多页签模式下主窗口的 webContents 只是壳页面（shell.html + 标签栏），用户并不跟它交互，
  // 给它挂右键菜单会在标签栏上弹出「返回 / 前进」这类**错位**的项 ⇒ 只给内容视图挂
  // （见上面 onSpawn）。单窗口模式下页面就在这个 webContents 里，挂它。
  if (!TABS_MODE) {
    attachDevtools(win.webContents, devtoolsEnabled(app.isPackaged, cfg.debug))
    attachContextMenu(win.webContents, openSettings)
  }

  return win
}

// ---------- 6.5 系统集成（M3）：设置应用 / 托盘 / 全局快捷键 / 设置窗 ----------

/** 设置窗专用 preload 的绝对路径 */
function settingsPreloadPath(): string {
  return join(__dirname, '..', 'renderer', 'settings-preload.js')
}

/** 悬浮窗专用 preload 的绝对路径 */
function floatPreloadPath(): string {
  return join(__dirname, '..', 'renderer', 'float-preload.js')
}

function readSettingsPageHtml(): string | null {
  return readSettingsHtml(app.getAppPath())
}

/** 打开设置窗（托盘菜单 / Shift+F1 / 页面右键菜单共用的唯一入口，对齐旧版） */
function openSettings(): void {
  openSettingsWindow({
    preloadPath: settingsPreloadPath(),
    readHtml: readSettingsPageHtml,
  })
}

/**
 * 创建多页签模式的首个页签（启动 / 失败重试 / 空壳自愈共用）。
 *
 * 失败与「达上限」都在壳页面上给出可见状态：绝不允许空白壳 + 静默日志。
 * 成功时撤掉失败遮罩（新内容视图本身也会盖住壳页面，显式撤一次更干净）。
 *
 * ⚠️ **不要在开头撤遮罩**：重试是异步的（sidecar 解析可能几秒），开局就撤会让用户
 *    对着空白壳干等 —— 正是本函数要消灭的失败形态。遮罩里的按钮此时已切成
 *    「重试中…」并置灰，保留遮罩才有持续反馈；成功才撤（见下方 then 的成功分支）。
 */
function spawnFirstTab(): Promise<void> {
  const win = mainWindow
  if (!win || win.isDestroyed() || !tabManager) return Promise.resolve()
  return tabManager.spawnTab().then(
    (label) => {
      if (label === null) {
        showShellStartupError(win, `首个页签创建被拒（页签数已达 ${MAX_TABS} 个上限？）`)
        console.error('[web2app] 首个页签创建失败：已达页签上限')
      } else {
        hideShellStartupError(win)
      }
    },
    (e: Error) => {
      console.error(`[web2app] 首个页签创建失败：${e.message}`)
      showShellStartupError(win, `首个页签加载失败：${e.message}`)
    },
  )
}

/**
 * 唤起主窗口（托盘点击/菜单「显示」/第二实例），并在多页签空壳时自愈补建。
 *
 * 死局场景：关掉最后一个页签会调 host.close()，而启用「关闭最小化到托盘」时 close
 * 被拦成 hide —— 恢复后窗口是没有任何内容视图的空壳（shell.html 无「+」按钮，
 * spawnTab 也没有页面侧入口）。与 Tauri tabs.rs 的 set_tabs_visible 自愈语义对齐：
 * 窗口重新可见且页签数为 0 时自动补建一个默认页签（失败时同样挂可见错误遮罩）。
 */
function presentMainWindow(): void {
  showMainWindow(mainWindow)
  // 窗口已销毁（无托盘驻留时关窗即退出）就别再往废 host 上挂视图。
  if (!mainWindow || mainWindow.isDestroyed() || !mainWindow.isVisible()) return
  if (TABS_MODE && tabManager && tabManager.count === 0) {
    void spawnFirstTab()
  }
}

/**
 * 请求退出应用：**先置 `quitting`**，再 `app.quit()`。
 *
 * ⚠️⚠️ 顺序不可交换：`app.quit()` 触发的窗口 close 事件会被 `win.on('close')` 的
 *    「驻留」分支（托盘 / 悬浮图标）拦下 —— 表现就是「点了退出，窗口只是藏起来、进程还在」。
 *    置位后那条分支直接放行（悬浮球窗的 close 也认 `isQuitting`，见 float-window.ts）。
 *
 * 托盘菜单「退出」与设置页「退出程序」（v1.75，常驻后台时唯一稳当的退出入口）共用这一条路径。
 */
function requestQuit(): void {
  quitting = true
  app.quit()
}

/**
 * 球上**右键**（v1.76）= 关闭悬浮图标功能 + 唤回主窗口，与 Tauri 侧
 * `floatwin::disable_float_from_icon` 逐条对齐：
 *   ① `float_enabled` 置 false 并落盘 —— **只动这一位**，刻意不经 `settingsCtl.applySettings`：
 *      后者第一步会重新注册唤醒快捷键，快捷键恰好被占用时会整条抛错、把「关功能」一起放弃；
 *   ② 销毁球窗；③ 把主窗口显示出来（presentMainWindow 含多页签空壳自愈）。
 *
 * 真机缺陷（2026-10-11 使用者实测，策略 tray_enabled:false / minimize_on_close:false /
 * float_enabled:true）：主窗「关闭」只等于隐藏到球，球成了桌面上**唯一**的可见入口；
 * 此刻右键把球一藏 —— 进程因 `float_enabled` 仍为 true 继续驻留，桌面上却再也唤不出窗口。
 * 右键在用户直觉里就是「我不要这个球了」，正确语义是回到主窗口。
 *
 * ⚠️ 延后一拍再执行：调用栈来自**球窗自己的** IPC 派发（`web2app:float-hide`），而这里要
 *    close 掉那个球窗 —— 等于在事件源窗口的回调栈里同步销毁它自己（Electron 会告警，
 *    页面也拿不到响应）。
 * ⚠️ 落盘失败**不阻断**后两步：先把窗口还给用户，再让日志留痕（与 Tauri 侧同一条纪律）。
 *
 * 探针经 m3 钩子 `hideFloatFromIcon` 调的就是本函数 —— 断言与真机同源。
 */
function hideFloatFromIcon(): void {
  setImmediate(() => {
    try {
      runtimeSettings = { ...runtimeSettings, float_enabled: false }
      saveSettings(storeLoc, runtimeSettings)
    } catch (e) {
      console.error(
        `[web2app] 右键关闭悬浮图标功能：写入设置失败（仍继续销毁球窗并唤回主窗）：${(e as Error).message}`
      )
    }
    // 设置页若开着，把复选框拉齐 —— 否则用户随手一点「保存」会把球推回 true。
    updateSettingsWindowFloat(false)
    destroyFloatWindow()
    presentMainWindow()
  })
}

/**
 * 设置应用控制器：唤醒快捷键 / 托盘的创建销毁 / 持久化，以及失败时对**进程级副作用**
 * （快捷键 + 托盘）的成组回滚。启动时与设置窗保存共用这同一个入口（对齐旧版
 * `settings.rs::save_settings`：快捷键 → 托盘 → 持久化，任一步失败都不写内存态）。
 * 实现与回滚注释见 settings-controller.ts；窗口显隐 / 退出 / 图标定位经 deps 注入。
 */
const settingsCtl = createSettingsController({
  title: cfg.title,
  getSettings: () => runtimeSettings,
  setSettings: (s) => {
    runtimeSettings = s
  },
  // 探针模式下 storeLoc 会被改到 %TEMP%（见上方 PROBE 分叉），故用 getter 延迟读。
  getStoreLoc: () => storeLoc,
  isMainWindowVisible: () => !!mainWindow && !mainWindow.isDestroyed() && mainWindow.isVisible(),
  openSettings: () => openSettings(),
  presentMainWindow: () => presentMainWindow(),
  hideMainWindow: () => hideMainWindow(mainWindow),
  toggleMainWindow: () => toggleMainWindow(mainWindow),
  requestQuit,
  trayIconPath: () => locateTrayIcon(app.getAppPath(), process.resourcesPath),
  // 悬浮窗开关（仅 Windows 有行为；float-window 内部对其它平台空操作）。
  showFloatWindow,
  destroyFloatWindow,
  toggleFloatWindow,
})

if (!gotLock) {
  app.quit()
} else {
  app.on('second-instance', () => {
    presentMainWindow()
  })

  // 关闭 Chromium 拼写检查：Electron 默认开启，远程网页里的非英文词（用户名、
  // 随机串、中文站点的英文片段）会被画上红色波浪线（2026-10-01 使用者反馈）。
  // Tauri/WebView2 默认不做这层检查，这里对齐它。挂在 web-contents-created 上，
  // 主窗口、设置窗、多页签的每个 WebContentsView（含各自 partition 的 session）
  // 一创建就关掉，比逐窗口写 webPreferences.spellcheck 更不容易漏。
  app.on('web-contents-created', (_e, contents) => {
    contents.session.setSpellCheckerEnabled(false)
  })

  app.whenReady().then(async () => {
    // splash:// / settings:// / shell:// 三个处理器（scheme 特权声明在文件顶部，
    // 必须早于 ready；这里只注册 handle）。重试 / 渲染 / 资源读取经 deps 接回本文件状态。
    registerProtocols({
      renderSplash: () => splashPage.render(),
      setSplashPendingFail: (f) => splashPage.setPendingFail(f),
      onSplashRetry: () => {
        navStarted = false
        target = null
        void bootstrap()
      },
      readSettingsHtml: readSettingsPageHtml,
      readInject,
      readFloatIcon: () => {
        try {
          return readFileSync(floatIconPath())
        } catch {
          return null
        }
      },
    })

    // 探针模式：起夹具站点、覆盖 app_url，并把导航节奏交给 runProbe
    let probeBase = ''
    let probeServer: ProbeServer | null = null
    if (PROBE) {
      const srv = await startProbeServer()
      probeServer = srv
      probeBase = srv.url
      cfg.app_url = `${probeBase}/plain.html`
      cfg.extra = { build: 'probe', env: 'test' }
      cfg.proxy_enabled = false
      console.log(`[web2app] 探针模式：夹具站点 ${probeBase}`)
    }

    registerAppIpc({
      tabsMode: TABS_MODE,
      getMainWindow: () => mainWindow,
      openSettings,
      getTabManager: () => tabManager,
      spawnFirstTab: () => void spawnFirstTab(),
      // chromeH 与标签栏同高（跨壳 38px 契约，唯一字面量在 tabs.ts 的 TAB_BAR_HEIGHT）。
      bootstrap: { chromeH: TAB_BAR_HEIGHT, extra: cfg.extra, readInject },
      getTarget: () => target,
      proxyHttp: cfg.proxy_http,
      proxyHttps: cfg.proxy_https,
    })
    registerSettingsIpc({
      getSettings: () => runtimeSettings,
      saveSettings: (next) => settingsCtl.applySettings(next, { persist: true }),
      preloadPath: settingsPreloadPath(),
      readHtml: readSettingsPageHtml,
      // 设置页「清除全部」：清浏览数据 + 重载内容页（实现见 web-data.ts）。
      // ⚠️ 只重载**内容**页：多页签模式 = 每个 tab 视图；单窗口模式 = 主窗口。
      //    设置窗自己**不重载** —— 用户正盯着它看这行结果提示。
      clearWebData: () =>
        clearWebData({
          probe: PROBE,
          reloadContentPages: () => {
            if (TABS_MODE) return tabManager?.reloadAll() ?? 0
            const w = mainWindow
            if (!w || w.isDestroyed()) return 0
            w.webContents.reload()
            return 1
          },
        }),
      showFloat: () => showFloatWindow(),
      // 设置页「退出程序」（v1.75）：与托盘「退出」同一条 requestQuit。
      // ⚠️ 这条不是可选装饰：启用托盘 / 悬浮图标后关闭窗口只等于隐藏，而没启用托盘时
      //    托盘菜单并不存在 ⇒ 这是那种状态下**唯一**的退出入口（见 settings.html 的说明）。
      quitApp: requestQuit,
    })

    // 悬浮球：注册白名单 IPC + 内置球页 / 主窗切换接线（仅 Windows 会真正建球）。
    // 球窗只加载 floaticon://localhost/ 本地协议页，不导航、不 bootstrap、不碰 sidecar。
    registerFloatWindow({
      title: cfg.title,
      preloadPath: floatPreloadPath(),
      getStoreLoc: () => storeLoc,
      // 单击球 = 切换【原来的主窗口】显隐（与托盘左键 / 唤醒快捷键同语义）：
      // 可见且非最小化 → 隐藏；隐藏中 / 最小化 → 经 presentMainWindow 恢复并聚焦，
      // 多页签空壳时还会自愈补建首个页签（不能直接调 toggleMainWindow，会漏掉自愈）。
      toggleMainWindow: () => {
        if (
          mainWindow &&
          !mainWindow.isDestroyed() &&
          mainWindow.isVisible() &&
          !mainWindow.isMinimized()
        ) {
          mainWindow.hide()
        } else {
          presentMainWindow()
        }
      },
      // 右键球（v1.76）= 关闭悬浮图标功能 + 唤回主窗口。实现见 hideFloatFromIcon ——
      // 与 Tauri 侧 `floatwin::disable_float_from_icon` 逐条对齐，且**同一个函数**也经
      // m3 钩子交给探针，保证探针断言的就是真机走的那条路径。
      onHideRequest: hideFloatFromIcon,
      // 退出路径必须放行悬浮球的 close（否则 preventDefault 会把 app.quit() 整个取消）。
      isQuitting: () => quitting,
    })

    // 窗口几何：必须在 ready 之后读（planRestore 要用 screen 模块，它在 ready 前不可用）
    const ws = loadWindowState(storeLoc)
    const plan = planRestore(ws.state)
    console.log(
      `[web2app] 窗口几何：${plan.reason}${ws.source ? `  ← ${ws.source}` : ''}`
    )

    mainWindow = createWindow(!PROBE, plan)
    logStartup()

    // ---------- M3：托盘 / 全局快捷键 ----------
    // ⚠️ 启动期在探针模式下**不**自动应用真实设置：否则探针一跑就创建托盘图标、抢占全局
    //    快捷键，在用户机器上留下「测试顺手改了桌面环境」的副作用。
    //    探针改用 ctx.m3 的钩子**按需**触发同一条 applySettings 路径（m3 断言的就是这些
    //    真实副作用，见 probe.ts M3 段落；断言语境下的创建/抢占是被显式验证并收尾的）。
    if (!PROBE) {
      try {
        // ⚠️⚠️ 必须走 `applyInitialSettings`（语义 = 从默认态切换）而不是 `applySettings`：
        //    `runtimeSettings` 在上面就被初始化成同一份 `loadedSettings.settings`，而托盘 /
        //    悬浮窗都是「变化才动」的 diff ⇒ 用 applySettings 时 diff 恒为 false，两个副作用
        //    一个都不会建立（2026-10-10 实测副作用调用数为 0），表现是「勾了启用托盘 /
        //    悬浮窗，重启后图标与悬浮窗都不再出现」。与 Tauri 侧 setup 的显式建窗同语义。
        settingsCtl.applyInitialSettings(loadedSettings.settings)
      } catch (e) {
        // 启动期失败只记日志：托盘图标缺失 / 快捷键被占用都不该让应用打不开
        console.error(`[web2app] 应用设置时出错（已降级继续启动）：${(e as Error).message}`)
      }
      registerSettingsShortcut(() => openSettings())
    }

    if (PROBE) {
      if (TABS_PROBE) {
        // 多页签探针：壳页面先出，页签管理器已在 createWindow 挂到窗口上，
        // 断言组自己 spawnTab 驱动（复用真实 onSpawn 链路）。
        mainWindow.show()
        await mainWindow.loadURL('shell://localhost/').catch((e: Error) => {
          console.error(`[web2app] 壳页面加载失败：${e.message}`)
        })
        const ok = await runTabsProbe({
          win: mainWindow,
          base: probeBase,
          tabs: tabManager!,
          getSettingsWindow: () => getSettingsWindow(),
          closeSettingsWindow: () => closeSettingsWindow(),
        })
        probeServer?.close()
        console.log(ok ? '[web2app] 多页签探针全部通过' : '[web2app] 多页签探针存在失败项')
        app.exit(ok ? 0 : 1)
        return
      }

      // 探针自己 loadURL，窗口保持可见（最小化 / 最大化断言需要真实窗口状态）
      mainWindow.show()
      const ok = await runProbe({
        win: mainWindow,
        base: probeBase,
        // M2 的断言要看「上游实际收到了什么头」，所以夹具的观测点必须传进去
        seen: probeServer!.seen,
        // 真实 sidecar（不是 mock）：M2 要验的就是「同一份 Rust 代理能被 Electron 驱动起来」
        startSidecar: launchSidecar,
        // M3 断言用的真实入口（勾的是与设置窗完全相同的那些函数）
        m3: {
          storeLoc,
          getSettings: () => runtimeSettings,
          saveSettings: (next) => settingsCtl.applySettings(next, { persist: true }),
          openSettingsWindow: () => openSettings(),
          closeSettingsWindow: () => closeSettingsWindow(),
          getSettingsWindow: () => getSettingsWindow(),
          applyWakeHotkey: (h) => applyWakeHotkey(h, () => toggleMainWindow(mainWindow)),
          releaseShortcuts: () => releaseShortcuts(),
          registerSettingsShortcut: (fn) => registerSettingsShortcut(fn),
          toggleMainWindow: () => toggleMainWindow(mainWindow),
          hasTray: () => hasTray(),
          destroyTray: () => destroyTray(),
          trayIconPath: () => locateTrayIcon(app.getAppPath(), process.resourcesPath),
          hasFloat: () => isFloatAlive(),
          // 球上右键的真机路径（v1.76）：探针断言「关闭功能 + 销毁球窗 + 唤回主窗」三件事。
          hideFloatFromIcon: () => hideFloatFromIcon(),
        },
      })
      probeServer?.close()
      console.log(ok ? '[web2app] 探针全部通过' : '[web2app] 探针存在失败项')
      app.exit(ok ? 0 : 1)
      return
    }

    if (TABS_MODE) {
      // 多页签：壳页面先出，首个内容视图由 tabManager.spawnTab 建立（复用 target 解析）。
      // 等壳页面加载完再建首个内容视图：①失败遮罩要注入壳 DOM，壳没就绪注入会被吞；
      //   ②shell:// 是零网络本地协议，等待成本只有几十毫秒（标签栏先出也更像 Tauri 时序）。
      // 失败（解析/sidecar/达上限）时壳页面上会出现可见错误遮罩 + 重试入口，
      // 不再只是 console.error（默认 debug=false 时用户什么都看不到）。
      void mainWindow
        .loadURL('shell://localhost/')
        .catch((e: Error) => {
          console.error(`[web2app] 壳页面加载失败：${e.message}`)
        })
        .then(() => spawnFirstTab())
    } else {
      void mainWindow.loadURL('splash://localhost/').catch((e: Error) => {
        console.error(`[web2app] 启动页加载失败：${e.message}`)
      })
    }

    // 冒烟自检：WEB2APP_SMOKE=1 时打完启动信息就退出，不需要人眼看着窗口。
    // 检查项与退出码规则见 smoke.ts（探针之外的第二条 CI 取证通道）。
    if (SMOKE) {
      const bad = runSmokeChecks({
        title: cfg.title,
        actualTitle: mainWindow.getTitle(),
        renderSplash: () => splashPage.render(),
        readSettingsPage: readSettingsPageHtml,
        trayIconPath: () => locateTrayIcon(app.getAppPath(), process.resourcesPath),
        fileLog,
      })
      console.log('[web2app] WEB2APP_SMOKE=1，自检完成，退出')
      app.exit(bad ? 1 : 0)
    }
  }).catch((e: unknown) => {
    // ⚠️⚠️ 兜底：ready 之后**任何** await 抛错都必须收敛成一个非 0 退出码。
    //    没接住的话它只是一个 UnhandledPromiseRejection —— 既不打印结论、也不 app.exit()，
    //    在 CI 里的表现是**挂死**（而不是失败），比直接报错难查得多。
    //    2026-09-17 实测踩到：无 GPU 沙箱下 probe.ts 的 `win.loadURL('splash://localhost/')`
    //    抛 ERR_FAILED，日志里只留下 `UnhandledPromiseRejectionWarning: Error: ERR_FAILED
    //    (-2) loading 'splash://localhost/'`，探针既没说「哪一条失败」也没退出。
    //    「退出码即结论」是探针存在的全部意义，这条兜底就是它的最后一道保险。
    const err = e as Error | undefined
    // ⚠️⚠️ 必须走 rawConsole：debug=false 时 `console.error` 已被替换成空操作，用它打印
    //    等于把唯一的线索也抹掉。这条属于「必须让人看到」的错误，不是可静默的诊断。
    rawConsole.error(`[web2app] 启动流程异常终止（已强制非 0 退出）：${err?.stack ?? String(e)}`)
    app.exit(1)
  })

  // 主窗口真的被关掉（没有「关闭到托盘」）→ 退出。
  // 注意：勾了「关闭到托盘」时窗口只是 hide，不会走到这里，「驻留托盘」因此不受影响。
  app.on('window-all-closed', () => app.quit())

  // ⚠️ 必须在窗口 close 之前置位：托盘菜单「退出」/ 系统会话结束时，
  //    close 处理逻辑要能区分「用户点了关闭按钮」与「整个应用要退出了」。
  app.on('before-quit', () => {
    quitting = true
  })

  // 退出前收掉 sidecar / 托盘 / 全局快捷键。
  // ⚠️ 这是**正常退出**路径；异常退出（任务管理器结束 / 崩溃）时不会走到这里，
  // 那条路由 sidecar 的 stdin-EOF 兜底。两条路都留着 —— 任一失效都还有下一条，
  // 这是「不留孤儿进程」唯一稳妥的做法（Windows 上要 Job Object 才能等效，更绕）。
  app.on('will-quit', () => {
    releaseShortcuts()
    destroyTray()
    // 悬浮窗几何（吸附态展开坐标 / 尺寸）随退出落盘；随后其 BrowserWindow 随进程一起回收。
    persistFloatGeometry()
    stopActiveSidecar()
    // 多页签：显式回收所有内容视图（官方明示视图不随窗口销毁自动回收）
    tabManager?.destroyAllTabs()
  })
}

// ---------- 7. 启动日志：绿色版排障的第一现场（**仅在 debug 打开时可见**） ----------
/** 文件日志落点的一行描述（debug 关闭时如实说明，而不是打印一个并不存在的路径）。 */
function describeFileLog(log: FileLogHandle | null): string {
  if (!log) return '(debug 未开启，未落盘)'
  return `${log.path}${log.ok ? '' : `  ✗ 不可写（${log.error ?? '未知原因'}）`}`
}

function logStartup(): void {
  console.log('[web2app] 启动信息')
  console.log(`  入口目录      : ${paths.root}`)
  console.log(
    `  分发形态      : ${
      paths.forcedSystem
        ? '安装版 (NSIS，系统管理数据目录)'
        : paths.selfExtracting
          ? '单文件自解压 (portable)'
          : '解压目录 (zip/dir)'
    }`,
  )
  console.log(
    `  数据目录      : ${paths.dataDir}  (mode=${paths.mode}${
      paths.fellBack ? ', 已回退' : ''
    }${paths.forcedSystem ? ', 强制系统目录（忽略 data_dir）' : ''})`,
  )
  console.log(`  配置文件      : ${loaded.path ?? '(未找到，使用默认值)'}`)
  console.log(`  应用名 / 标题 : ${cfg.app_name} / ${cfg.title}`)
  console.log(`  注入脚本目录  : ${injectDir()}`)
  // 诊断开关本身必须写进日志：否则「日志文件里怎么只有一行 header / 怎么压根没有日志文件」
  // 这类追问会反复出现 —— 用户看不出是「开关没开」还是「写盘失败」。
  console.log(`  诊断日志      : ${DEBUG ? '已开启' : '未开启（config.json 的 debug 或命令行 --debug）'}`)
  // 文件日志是用户侧唯一能提供证据的东西，路径必须打在**第一行区域**（用户直接截图给我们）
  console.log(`  日志文件      : ${describeFileLog(fileLog)}`)
  console.log(`  外部代理      : ${proxy.proxyServer ?? '(未启用)'}`)
  // M3：配置文件的落点必须写进日志 —— 「我明明勾了托盘，重启又没了」这类反馈，
  // 只有靠这一行才能一眼看出是写到了哪儿、以及有没有读到旧版的文件。
  // ⚠️ 这一个文件同时装着设置与窗口几何（2026-09-23 合并），所以「窗口尺寸没记住」
  //    也看这一行 + 上面那条「窗口几何：<原因>」。
  console.log(
    `  配置文件      : ${loadedSettings.source ?? '(未找到，使用默认值)'}` +
      `  → 写入 ${storeLoc.write}`
  )
  console.log(`  唤醒快捷键    : ${hotkeyLabel(runtimeSettings.wake_hotkey)}`)
  console.log(
    `  托盘 / 关闭行为: ${runtimeSettings.tray_enabled ? '已启用' : '未启用'}` +
      ` / ${runtimeSettings.tray_enabled && runtimeSettings.minimize_on_close ? '关闭时隐藏到托盘' : '关闭即退出'}`
  )
  console.log(
    `  悬浮图标      : ${runtimeSettings.float_enabled && process.platform === 'win32' ? '已启用' : '未启用'}`
  )
  // 需要 sidecar 时把它的位置和存在性写进日志：这一步「找不到」是最常见的配置类故障，
  // 而失败文案是收敛过的（不暴露路径），所以启动日志是唯一能直接看到原因的地方。
  if (needsSidecar(cfg)) {
    const exe = locateSidecar(app.getAppPath(), process.resourcesPath)
    console.log(`  本地代理组件  : ${exe ?? '(未找到！请先 cargo build --release -p w2a-proxy)'}`)
  }
  console.log(`  web2appExtra  : ${extraJson(cfg)}`)
  for (const w of loaded.warnings) console.warn(`  ⚠️ ${w}`)
  for (const w of loadedSettings.warnings) console.warn(`  ⚠️ ${w}`)
}
