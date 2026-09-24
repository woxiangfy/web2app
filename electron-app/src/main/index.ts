/**
 * 主进程入口（M1：窗口复刻 + 导航接入 + 兼容契约探针）。
 *
 * ⚠️ 顺序约束（改这个文件时务必保持）：
 *   1. setupPortablePaths()  —— 必须早于任何会读取 userData 的调用
 *   2. app.commandLine.appendSwitch 外部代理 —— 必须早于 app ready
 *   3. protocol.registerSchemesAsPrivileged —— 必须早于 app ready
 *   4. requestSingleInstanceLock() —— 单实例锁文件落在 userData 下，故在第 1 步之后
 */
import { app, BrowserWindow, ipcMain, protocol } from 'electron'
import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'

import { setupPortablePaths } from './portable'
import { buildProxySwitches, extraJson, loadConfigAny, sanitizeDirName, type AppConfig } from './config'
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
  hotkeyLabel,
  loadSettings,
  sameHotkey,
  saveSettings,
  SETTINGS_FILE,
  storeLocation,
  type Settings,
} from './settings'
import {
  captureWindowState,
  loadWindowState,
  planRestore,
  saveWindowState,
  type RestorePlan,
} from './window-state'
import {
  createTray,
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
  FALLBACK_HTML,
  getSettingsWindow,
  openSettingsWindow,
  readSettingsHtml,
  registerSettingsIpc,
} from './settings-window'
import { TabManager } from './tabs'
import { rawConsole, silenceConsole, startFileLogging, type FileLogHandle } from './startup-log'
import {
  attachContextMenu,
  attachDevtools,
  attachDownload,
  devtoolsEnabled,
} from './page-behavior'
import { clearWebData } from './web-data'

/** 自绘标题栏高度（逻辑像素）。与 Tauri 版一致，勿改。 */
const CHROME_H = 38

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
// 打包后 resources/inject/ 由 electron-builder 从 webview-app/src 拷贝；
// 开发时（electron .）直接读 webview-app/src，避免复制出第二份会分叉的脚本。
function injectDir(): string {
  const packed = join(process.resourcesPath, 'inject')
  if (existsSync(packed)) return packed
  return join(__dirname, '..', '..', '..', 'webview-app', 'src')
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
 *   · `--debug` / 环境变量 `WEB2APP_DEBUG=1` —— 运行时逃生口，交付出去的包也能临时取证，
 *     不必为了排障重新打包；
 *   · 探针 / 冒烟模式**强制打开** —— 它们存在的意义就是取证，探针的「退出码即结论」与冒烟
 *     的路径断言全都靠这些输出，静默会让自检退化成「永远绿」。
 */
const DEBUG = cfg.debug || process.env.WEB2APP_DEBUG === '1' || hasFlag('--debug') || PROBE || SMOKE

// ⚠️ 静默要**尽可能早**：晚一步就会漏掉 setupPortablePaths 那类启动早期输出。
//    此后 `console.*` 全是空操作，只有 rawConsole 还能出声（见 startup-log.ts）。
// ⚠️ 配置解析在上一步（loadConfigAny）**之前**完成，所以「config.json 坏了」那条致命
//    报错不会被静默吞掉 —— 那是必须让人看到的一类错误。
if (!DEBUG) silenceConsole()

const paths = setupPortablePaths({
  dataDir: cfg.data_dir,
  appName: sanitizeDirName(cfg.app_name),
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
])

const SPLASH_BG = '#f7f8fa' // = bootstrap::SPLASH_BG(0xf7,0xf8,0xfa)

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

/** 转成可安全嵌进 JS 字符串字面量的双引号串（含两侧引号），对齐 Rust 侧 `js_str`。 */
function jsStr(s: string): string {
  let out = '"'
  for (const ch of s) {
    switch (ch) {
      case '\\': out += '\\\\'; break
      case '"': out += '\\"'; break
      case '\n': out += '\\n'; break
      case '\r': out += '\\r'; break
      case '\u2028': out += '\\u2028'; break
      case '\u2029': out += '\\u2029'; break
      case '<': out += '\\u003c'; break
      default: out += ch
    }
  }
  return out + '"'
}

const FALLBACK_SPLASH = `<!doctype html><meta charset="utf-8">
<style>html,body{margin:0;height:100%;background:${SPLASH_BG}}
.d{display:flex;height:100%;align-items:center;justify-content:center;
font:14px/1.6 system-ui,sans-serif;color:#5f5e5a}</style><div class="d">加载中…</div>`

/**
 * 启动页阶段的失败态：写进启动页 HTML 而不是走 IPC。
 *
 * 理由：启动页把加载层内联在**主世界**，而 preload 的 IPC 通道在**隔离世界**里
 * （`window.__web2appLoading` 读不到）。启动页是我们自己生成的 HTML，把失败态直接
 * 烤进去最简单也最可靠；加载层挂载完成后会自己把它取走（见 loading.js 的 applyPendingFail）。
 * 真实页面上的失败走另一条路（IPC → preload → 同世界的加载层）。
 */
let pendingFail: { title: string; detail: string } | null = null

/**
 * 渲染 splash.html 模板。
 *
 * ⚠️⚠️ 必须用 split/join 而不是 String.replace —— 注入的是 titlebar.js / loading.js
 * 全文，其中含 `$&`、`$'` 这类序列，在 replace 的替换串里是特殊模式，会把注入内容
 * 改坏（这类 bug 表现为「脚本里少了一段、页面报奇怪的语法错」）。Rust 侧同理。
 */
function renderSplash(): string {
  const tpl = readInject('splash.html')
  if (!tpl) {
    console.warn('[web2app] 未找到 splash.html，使用内置占位启动页')
    return FALLBACK_SPLASH
  }
  let html = tpl
    .split('__WEB2APP_TITLE__').join(escapeHtml(cfg.title))
    .split('__WEB2APP_TITLEBAR_JS__').join(readInject('titlebar.js') ?? '')
    .split('__WEB2APP_LOADING_JS__').join(readInject('loading.js') ?? '')

  if (pendingFail) {
    // ⚠️⚠️ 这里同样**必须**用 split/join（与 renderSplash 顶部的占位符替换同一条理由）：
    //    detail 里含失败 URL（配置里的地址，或页面重定向过来的地址），其中完全可能出现
    //    `$&` / `` $` `` / `$'`；作为 `String.replace` 的**替换串**时它们是特殊模式，
    //    会把命中的 `</body>` 前后内容整段塞进这个 JS 字符串字面量 —— 启动页脚本直接语法错，
    //    用户看到的是「加载失败页什么都不显示」，比没有失败提示还糟。
    const failScript =
      `<script>try{window.__web2appPendingFail={title:${jsStr(pendingFail.title)},detail:${jsStr(pendingFail.detail)}};}catch(e){}</script>\n</body>`
    html = html.split('</body>').join(failScript)
  }
  return html
}

// ---------- 4. 单实例 ----------
const gotLock = app.requestSingleInstanceLock()

let mainWindow: BrowserWindow | null = null
let target: ResolvedTarget | null = null
/** 目标地址解析 + 首次导航只允许发生一次（重试路径会显式重置） */
let navStarted = false

/** 多页签管理器（仅 tabs_enabled=true 时非空）。 */
let tabManager: TabManager | null = null

/**
 * 是否多页签模式（只在最外层读一次，禁止在既有函数里散落 if）。
 *
 * ⚠️⚠️ **探针模式下必须让「起窗模式」与「断言组」一一对应**，因此 `cfg.tabs_enabled`
 *    **不参与**探针分叉：
 *      - `--probe`         → 强制单窗口模式，跑 130 条单窗口断言（按「主窗口加载
 *                            splash:// + 标题栏/滚动条/窗口按钮」设计）；
 *      - `--probe --tabs`  → 强制页签模式，跑 26 条页签断言。
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
    pendingFail = { title: r.title, detail: r.detail }
    // 重新灌一遍启动页（带着失败态），用户至少能看到发生了什么并点重试
    navStarted = false
    void win.loadURL('splash://localhost/').catch(() => {})
    return
  }
  if (target && target.url === r.url) return // 同地址重复调用（如重试）就不要再导航
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

// ---------- 6. 窗口按钮 IPC（titlebar.js 的对接面） ----------
function registerWindowIpc(): void {
  const winOf = (e: Electron.IpcMainInvokeEvent) => BrowserWindow.fromWebContents(e.sender)

  // ⚠️⚠️ 多页签模式下的安全硬化：内容视图（子 WebContentsView）的 webContents 经
  //   `BrowserWindow.fromWebContents()` **也会解析到宿主窗口**（两者 getType() 都是 'window'，
  //   实测见设计文档 §3.5-2）。所以仅靠 fromWebContents 无法区分「壳发的」还是「内容页发的」。
  //   窗口按钮这类特权命令必须校验 sender.id === 壳 webContents.id，不匹配**记日志并拒绝**
  //   （绝不允许静默失效——那正是本项目最忌的失败形态）。
  //
  //   单窗口模式下壳 webContents == 主窗口 webContents，这条校验天然通过，行为零变化。
  const assertPrivileged = (e: Electron.IpcMainInvokeEvent): void => {
    if (!TABS_MODE) return
    const host = mainWindow
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
    openSettings()
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
function registerTabsIpc(): void {
  const host = () => {
    const w = mainWindow
    if (!w || w.isDestroyed()) throw new Error('宿主窗口不存在')
    return w
  }
  const assertShell = (e: Electron.IpcMainInvokeEvent): BrowserWindow => {
    const w = host()
    if (e.sender.id !== w.webContents.id) {
      console.warn(`[web2app] 拒绝非壳页签命令（sender.id=${e.sender.id} ≠ 壳=${w.webContents.id}）`)
      throw new Error('拒绝：仅壳页面可操作页签')
    }
    return w
  }
  const mgr = () => {
    if (!tabManager) throw new Error('多页签未启用')
    return tabManager
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
    openSettings()
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
}

/** 同步送注入脚本源码给 preload（49KB，每次导航一次，开销可忽略）。 */
function registerBootstrapIpc(): void {
  ipcMain.on('web2app:bootstrap', (e) => {
    e.returnValue = {
      chromeH: CHROME_H,
      shell: 'electron' as const,
      extra: cfg.extra,
      titlebarJs: readInject('titlebar.js') ?? '',
      loadingJs: readInject('loading.js') ?? '',
    }
  })
}

/**
 * Basic 鉴权：直连模式下导航地址里的 userinfo 已被剥掉，凭据在这里回填。
 *
 * Tauri 版是 WebView2 原生事件 + 本地代理注入 Authorization 两条路；Electron 侧
 * Chromium 对应的是 `login` 事件。⚠️ 地址栏上不出现密码这条约束两端一致。
 */
function registerAuthHandler(): void {
  app.on('login', (event, _wc, _details, authInfo, callback) => {
    if (authInfo.isProxy) {
      // 外部代理自带凭据时（http://user:pass@proxy:8080）用它
      const from = cfg.proxy_http || cfg.proxy_https
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
   * 关闭按钮语义（对齐 `lib.rs` 的 `handle_main_close`）：
   *   「启用托盘」+「关闭时最小化到托盘」→ 隐藏窗口，应用继续驻留托盘；
   *   否则正常关闭（最后一个窗口关闭后退出）。
   *
   * ⚠️ `quitting` 标志不可省：托盘菜单「退出」走的是 `app.quit()`，
   *    它会触发窗口的 close 事件 —— 没有这个标志就会「点了退出，窗口只是藏起来了」。
   */
  win.on('close', (e) => {
    persistGeometry(win)
    if (quitting) return
    if (runtimeSettings.tray_enabled && runtimeSettings.minimize_on_close) {
      e.preventDefault()
      win.hide()
      console.log('[web2app] 关闭按钮 → 隐藏到托盘（设置里已启用「关闭时最小化到托盘」）')
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
        // 挂窗口打开处理器：内容页 window.open → 新建页签
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

function readSettingsPageHtml(): string | null {
  return readSettingsHtml(app.getAppPath(), process.resourcesPath)
}

/** 打开设置窗（托盘菜单 / Shift+F1 / 页面右键菜单共用的唯一入口，对齐旧版） */
function openSettings(): void {
  openSettingsWindow({
    preloadPath: settingsPreloadPath(),
    readHtml: readSettingsPageHtml,
  })
}

/**
 * 唤起主窗口（托盘点击/菜单「显示」/第二实例），并在多页签空壳时自愈补建。
 *
 * 死局场景：关掉最后一个页签会调 host.close()，而启用「关闭最小化到托盘」时 close
 * 被拦成 hide —— 恢复后窗口是没有任何内容视图的空壳（shell.html 无「+」按钮，
 * spawnTab 也没有 IPC 入口）。与 Tauri tabs.rs 的 set_tabs_visible 自愈语义对齐：
 * 窗口重新可见且页签数为 0 时自动补建一个默认页签。
 */
function presentMainWindow(): void {
  showMainWindow(mainWindow)
  // 窗口已销毁（无托盘驻留时关窗即退出）就别再往废 host 上挂视图。
  if (!mainWindow || mainWindow.isDestroyed() || !mainWindow.isVisible()) return
  if (TABS_MODE && tabManager && tabManager.count === 0) {
    void tabManager
      .spawnTab()
      .then((label) => {
        if (!label) console.warn('[web2app] 恢复窗口补建页签被拒（可能已达上限）')
      })
      .catch((e) => console.error('[web2app] 恢复窗口时补建页签失败：', e))
  }
}

function trayActions() {
  return {
    toggle: () => {
      // 可见 → 收进托盘；不可见 → 走带页签自愈的唤起路径（不能直接 toggle，
      // 否则最后一个页签关掉后的空壳无法恢复）。
      if (mainWindow && !mainWindow.isDestroyed() && mainWindow.isVisible()) {
        toggleMainWindow(mainWindow)
      } else {
        presentMainWindow()
      }
    },
    onShow: () => presentMainWindow(),
    onHide: () => hideMainWindow(mainWindow),
    onSettings: () => openSettings(),
    // ⚠️ 必须先置 quitting：否则 app.quit() 触发的 close 事件会被「关闭到托盘」分支拦下，
    //    表现就是「点了退出，窗口只是藏进托盘、进程还在」。
    onQuit: () => {
      quitting = true
      app.quit()
    },
  }
}

/**
 * 应用一份设置：运行时副作用（快捷键 / 托盘）+ 可选持久化。
 *
 * 启动时与设置窗保存时共用同一条路径 —— 两条路各写一份的典型后果是
 * 「启动时正常、从设置页改了就行为不一致」。旧版（`settings.rs::save_settings`）
 * 也是这么组织的，顺序（快捷键 → 托盘 → 持久化）刻意保持一致：
 * 任一步失败都不写内存态 —— 快捷键自身注册失败会把旧键回滚，**后续步骤**失败
 * 也会把已生效的快捷键回滚（见下面的 catch），否则「失败」就只是半真半假的说法。
 *
 * @throws 出错时抛出带用户可读文案的 Error（设置页会把它显示出来）
 */
function applySettings(next: Settings, opts: { persist: boolean }): void {
  const prev = runtimeSettings

  // 1) 唤醒快捷键
  const r = applyWakeHotkey(next.wake_hotkey, () => toggleMainWindow(mainWindow))
  if (!r.ok) throw new Error(r.error ?? '快捷键注册失败')

  try {
    // 2) 托盘开关变化 → 创建 / 销毁（与旧版一样是「变化才动」，避免每次保存都重建图标）
    if (next.tray_enabled !== runtimeSettings.tray_enabled) {
      if (next.tray_enabled) {
        const ok = createTray({
          title: cfg.title,
          iconPath: locateTrayIcon(app.getAppPath(), process.resourcesPath),
          actions: trayActions(),
        })
        if (!ok) throw new Error('托盘创建失败（未找到 icons/icon.png）')
      } else {
        destroyTray()
      }
    }

    // 3) 持久化
    if (opts.persist) {
      const written = saveSettings(storeLoc, next)
      console.log(`[web2app] 设置已保存：${written}`)
    }
  } catch (e) {
    // ⚠️ 回滚快捷键。第 1 步是**已经生效的副作用**（全局快捷键是进程级资源），
    //    后两步失败时如果不回滚，就会留下这种自相矛盾的状态：
    //    界面上提示「保存失败」、settings.json 没写（下次启动仍是旧键），
    //    但**本次会话里新键已经生效**。文件注释里承诺的「任一步失败都不写内存态」
    //    只有在把副作用也一并回滚之后才成立。
    if (!sameHotkey(prev.wake_hotkey, next.wake_hotkey)) {
      const rb = applyWakeHotkey(prev.wake_hotkey, () => toggleMainWindow(mainWindow))
      console.warn(
        `[web2app] 保存失败，唤醒快捷键已回滚到 ${hotkeyLabel(prev.wake_hotkey)}：` +
          `${rb.ok ? '成功' : '回滚也失败（当前无唤醒快捷键）'}`
      )
    }
    throw e
  }

  runtimeSettings = next
}

if (!gotLock) {
  app.quit()
} else {
  app.on('second-instance', () => {
    presentMainWindow()
  })

  app.whenReady().then(async () => {
    protocol.handle('splash', (req) => {
      const path = new URL(req.url).pathname.replace(/\/+$/, '')
      if (path === '/retry') {
        console.log('[web2app] 启动页请求重试，重新走一遍启动流程')
        pendingFail = null
        // 先把手上的响应交出去，再触发导航，避免同一次请求里嵌套导航
        setImmediate(() => {
          navStarted = false
          target = null
          void bootstrap()
        })
      }
      return new Response(renderSplash(), {
        headers: { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' },
      })
    })

    // settings:// → 设置页 HTML（单一真源：webview-app/dist/settings.html）
    protocol.handle('settings', () => {
      const html = readSettingsPageHtml()
      if (!html) {
        console.warn('[web2app] 未找到 settings.html，设置窗使用内置精简页')
      }
      return new Response(html ?? FALLBACK_HTML, {
        headers: { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' },
      })
    })

    // shell:// → 多页签壳页面（单一真源：webview-app/src/shell.html，零改动）
    protocol.handle('shell', () => {
      const html = readInject('shell.html')
      if (!html) {
        console.warn('[web2app] 未找到 shell.html，壳页面缺失（多页签模式将白屏）')
      }
      return new Response(html ?? '<!doctype html><meta charset="utf-8"><body style="background:#f7f8fa">标签栏资源缺失</body>', {
        headers: { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' },
      })
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

    registerBootstrapIpc()
    registerWindowIpc()
    registerTabsIpc()
    registerAuthHandler()
    registerSettingsIpc({
      getSettings: () => runtimeSettings,
      saveSettings: (next) => applySettings(next, { persist: true }),
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
        applySettings(loadedSettings.settings, { persist: false })
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
          saveSettings: (next) => applySettings(next, { persist: true }),
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
        },
      })
      probeServer?.close()
      console.log(ok ? '[web2app] 探针全部通过' : '[web2app] 探针存在失败项')
      app.exit(ok ? 0 : 1)
      return
    }

    if (TABS_MODE) {
      // 多页签：壳页面先出，首个内容视图由 tabManager.spawnTab 建立（复用 target 解析）。
      void mainWindow.loadURL('shell://localhost/').catch((e: Error) => {
        console.error(`[web2app] 壳页面加载失败：${e.message}`)
      })
      tabManager
        ?.spawnTab()
        .then((label) => {
          if (label === null) console.error('[web2app] 首个页签创建失败（已达上限？）')
        })
        .catch((e: Error) => {
          console.error(`[web2app] 首个页签创建失败：${e.message}`)
        })
    } else {
      void mainWindow.loadURL('splash://localhost/').catch((e: Error) => {
        console.error(`[web2app] 启动页加载失败：${e.message}`)
      })
    }

    // 冒烟自检：WEB2APP_SMOKE=1 时打完启动信息就退出，不需要人眼看着窗口。
    // 存在的意义：绿色版路径/配置解析/模板替换都是「错了也静默」的逻辑，
    // 有了这个开关才能在 CI 里无人值守地验证它们（Tauri 版当年为此专门编过探针 exe）。
    if (SMOKE) {
      // 顺手断言启动页模板替换是否完整 —— 三个占位符漏一个，
      // 表现就是「标题栏或加载动画不出现」这种要花半天定位的故障。
      const html = renderSplash()
      const leftover = ['__WEB2APP_TITLE__', '__WEB2APP_TITLEBAR_JS__', '__WEB2APP_LOADING_JS__'].filter(
        (p) => html.includes(p)
      )
      console.log(
        `[web2app] 启动页模板：${html.length} 字符；未替换占位符：${leftover.length ? leftover.join(', ') : '无'}`
      )

      // M3：设置链路的关键资源与契约。这三条都是「缺了也静默」的东西：
      //   · settings.html 找不到 → 设置窗只剩一个兜底页，用户看不出异常但功能少了；
      //   · 命令名对不上 → preload 垫片会拒绝调用，设置页的「保存」点了没反应；
      //   · 托盘图标缺失 → 勾了「启用托盘」也没图标。
      // ⚠️ 清缓存那条命令（clear_web_data）同样列进来：设置页点了没反应与上面同源 ——
      //    页面里的字面量、垫片白名单、IPC 注册名三处必须逐字一致。
      const page = readSettingsPageHtml() ?? ''
      const missingCmds = [
        'get_settings',
        'save_settings',
        'close_settings_window',
        'clear_web_data',
      ].filter((c) => !page.includes(c))
      console.log(
        `[web2app] 设置页资源：${page ? `${page.length} 字符` : '未找到！'}` +
          `；命令契约：${missingCmds.length ? '缺少 ' + missingCmds.join(', ') : '齐全'}`
      )
      const icon = locateTrayIcon(app.getAppPath(), process.resourcesPath)
      console.log(`[web2app] 托盘图标：${icon ?? '未找到'}`)

      // 窗口标题必须等于配置里的 title。
      // ⚠️ 这条断言是 2026-09-17 加的：createWindow 曾经漏传 `title`，
      //    而 page-title-updated 被拦，结果是标题永远停在 Electron 默认名
      //    （"web2app-electron"）—— 配置里的 title 静默失效，只有人眼看任务栏才发现。
      const actualTitle = mainWindow.getTitle()
      const titleOk = actualTitle === cfg.title
      console.log(
        `[web2app] 窗口标题：'${actualTitle}'（期望 '${cfg.title}'）${titleOk ? '' : '  ✗ 不一致！'}`
      )

      // 文件日志（M6）必须**真的写出来了**，且已包含 logStartup() 的内容。
      // ⚠️ 这条和上面三条一样属于「错了也静默」：文件没写出来，用户下次报
      //    「双击没反应」时我们手里就什么证据都没有。
      //    注意断言的是「内容已落盘」，不只是「文件存在」—— 只判存在的话，
      //    一个空文件（比如包装没装上、只有 header）也会让它通过。
      // ⚠️ 日志现在是 debug 门控的（默认关）⇒ 冒烟模式**强制** DEBUG=true（见上方 DEBUG
      //    的定义），否则这条断言会在默认配置下必然失败 —— 那不是发现缺陷，是判据错了。
      let logOk = false
      let logDetail: string
      let logPath = '(未启用)'
      if (!fileLog) {
        logDetail = '未启用（冒烟模式本应强制打开 debug，出现即 bug）'
      } else {
        logPath = fileLog.path
        if (!fileLog.ok) {
          logDetail = `不可写（${fileLog.error ?? '未知原因'}）`
        } else {
          try {
            const text = readFileSync(fileLog.path, 'utf8')
            logOk = text.includes('[web2app] 启动信息')
            logDetail = `${text.length} 字符`
          } catch (e) {
            logDetail = `读取失败：${(e as Error).message}`
          }
        }
      }
      console.log(
        `[web2app] 日志文件：${logPath}（${logDetail}）${logOk ? '' : '  ✗ 未包含启动信息！'}`
      )

      console.log('[web2app] WEB2APP_SMOKE=1，自检完成，退出')
      const bad =
        leftover.length > 0 || !page || missingCmds.length > 0 || !icon || !titleOk || !logOk
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
  console.log(`  分发形态      : ${paths.selfExtracting ? '单文件自解压 (portable)' : '解压目录 (zip/dir)'}`)
  console.log(`  数据目录      : ${paths.dataDir}  (mode=${paths.mode}${paths.fellBack ? ', 已回退' : ''})`)
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
