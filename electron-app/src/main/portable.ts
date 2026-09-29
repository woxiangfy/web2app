/**
 * 绿色版路径装配。
 *
 * ⚠️⚠️ 本文件必须在【任何 Electron API 使用之前】被调用（模块顶层第一句），
 *    原因：`app.setPath('userData', …)` 只允许在 app ready 之前生效，一旦
 *    有任何 store / session / 窗口先读过 userData，重定向就会部分失效，
 *    表现是「一部分数据写在 exe 旁边、一部分写在 %APPDATA%」这种最难查的状态。
 *
 * 支持的两种分发形态（同一份代码，靠环境变量自动区分）：
 *   1. 解压目录版（zip / dir target）—— app.getPath('exe') 就是用户放置位置；
 *   2. 单文件自解压版（portable target）—— ⚠️ app.getPath('exe') 指向
 *      %TEMP% 里的解压副本，**不是**用户放置 exe 的位置。此时必须改用
 *      electron-builder 注入的 PORTABLE_EXECUTABLE_DIR。
 *
 * 两者共同目标是「用户把 exe 放在哪，data\ 就生在哪」——即 installRoot()。
 */
import { app } from 'electron'
import { mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, isAbsolute, join } from 'node:path'

/**
 * 数据目录策略（用于启动日志 / 关于页展示，不参与路径决策）：
 *   · `relative` —— 配置的是相对路径，已解析到 exe 目录下（默认 `data\`，真绿色）；
 *   · `absolute` —— 配置的是绝对路径，原样使用；
 *   · `system`   —— 配置为空串，用系统默认程序目录 `%APPDATA%\<app_name>`（不受程序位置影响）；
 *   · `fallback` —— 配置的目录不可写，已退回系统默认目录（保证「能启动」优先）。
 */
export type DataDirMode = 'relative' | 'absolute' | 'system' | 'fallback'

export interface PortablePaths {
  /** 用户放置 exe 的目录（绿色基准目录） */
  root: string
  /** 最终生效的数据目录 */
  dataDir: string
  /** 数据目录策略 */
  mode: DataDirMode
  /** 是否因为同级目录不可写而退回用户目录 */
  fellBack: boolean
  /** 单文件自解压模式（用于日志 / 关于页展示） */
  selfExtracting: boolean
  /**
   * 安装版（NSIS）模式：数据目录被**强制**为系统默认目录，config.json 的 data_dir
   * 已被刻意忽略（用于日志 / 关于页展示，不参与路径决策）。
   */
  forcedSystem: boolean
}

const WRITE_PROBE = '.web2app-write-probe'

/**
 * 真实的「用户放置目录」。
 *
 * PORTABLE_EXECUTABLE_DIR 由 electron-builder 的 portable 包装器在启动时注入，
 * 指向包含正在运行的 exe 的那个目录。目录版（zip/dir）不会设置它。
 *
 * ⚠️ 开发态（`electron .`）必须单独处理：此时 app.getPath('exe') 指向
 *    node_modules/electron/dist/electron.exe，照它算会把 data\ 和 config.json
 *    定位到 Electron 自己的安装目录里去。故未打包时回落到应用根目录。
 */
export function installRoot(): string {
  const portableDir = process.env.PORTABLE_EXECUTABLE_DIR
  if (portableDir && portableDir.length > 0) return portableDir
  if (!app.isPackaged) return app.getAppPath()
  return dirname(app.getPath('exe'))
}

/** 单文件自解压模式判定（仅用于展示与日志，不参与路径决策） */
export function isSelfExtracting(): boolean {
  const d = process.env.PORTABLE_EXECUTABLE_DIR
  return typeof d === 'string' && d.length > 0
}

/**
 * 可写性探测：真写一个文件再删掉。
 *
 * 只检查 access(W_OK) 是不够的 —— U 盘写保护、只读网络盘、某些企业策略下的
 * Program Files，都在 access 上表现为可写但在真正写时失败。
 */
function isWritable(dir: string): boolean {
  try {
    mkdirSync(dir, { recursive: true })
    const probe = join(dir, WRITE_PROBE)
    writeFileSync(probe, '')
    rmSync(probe, { force: true })
    return true
  } catch {
    return false
  }
}

export interface SetupOptions {
  /**
   * 来自 config.json 的 `data_dir`（与 webview-app 同一契约、同一规则）：
   *   · `"data"`（默认）→ exe 同级 `data\`，整目录可搬走（真绿色）；
   *   · `""`           → `%APPDATA%\<app_name>`，系统默认程序目录，不受程序位置影响；
   *   · 其它相对路径    → 同上，基准是 **exe 目录**（不是 shell 的 cwd）；
   *   · 绝对路径        → 原样使用。
   */
  dataDir?: string
  /** 应用名（config.app_name），用于系统默认模式下的子目录名 */
  appName?: string
  /**
   * 强制系统默认数据目录（安装版 = true）。
   *
   * 为 true 时**忽略 `dataDir` 配置的一切取值**，一律使用 `%APPDATA%\<app_name>`：
   * 安装版由系统管理安装目录（perMachine 安装时是 Program Files，对标准用户只读），
   * 数据跟程序放在一起既不可写也不符合安装/卸载语义。判据来自构建期标记，见
   * `distribution.ts::isInstallerBuild`（由 index.ts 读出后传入）。
   */
  forceSystem?: boolean
}

/** 系统默认程序目录：`%APPDATA%\<app_name>`（由 app_name 命名，不受程序位置影响） */
function systemDataDir(appName?: string): string {
  return join(app.getPath('appData'), appName || 'Web2App')
}

/**
 * 把四个路径**一起**重定向到 `dir`。
 *
 * ⚠️ 必须一起改。只改 userData 会留下 sessionData（Cookies / Local Storage 的一部分）与
 * 日志、崩溃转储仍在系统盘，导致「绿色版不干净」。
 */
function applyDataDir(dir: string): void {
  app.setPath('userData', dir)
  app.setPath('sessionData', join(dir, 'session'))
  app.setPath('logs', join(dir, 'logs'))
  app.setPath('crashDumps', join(dir, 'crash'))
}

/**
 * 装配数据路径，返回实际生效的路径信息供启动日志 / 关于页展示。
 */
export function setupPortablePaths(opts: SetupOptions = {}): PortablePaths {
  const root = installRoot()
  const selfExtracting = isSelfExtracting()

  // 安装版（NSIS）：**刻意忽略 config.json 的 data_dir**，数据一律进系统默认程序目录。
  // 安装目录由系统管理（可能位于只读的 Program Files），且卸载/升级语义都以系统目录为准，
  // 这条分支必须先于下面一切 data_dir 解析。
  if (opts.forceSystem) {
    const dir = systemDataDir(opts.appName)
    applyDataDir(dir)
    return { root, dataDir: dir, mode: 'system', fellBack: false, selfExtracting, forcedSystem: true }
  }

  // config.ts 的 toDataDir 已归一成字符串；默认值与契约同为 "data"。
  const configured = (opts.dataDir ?? 'data').trim()

  // 空串 = 系统默认程序目录（由 app_name 命名），与程序放在哪无关。
  if (configured === '') {
    const dir = systemDataDir(opts.appName)
    applyDataDir(dir)
    return { root, dataDir: dir, mode: 'system', fellBack: false, selfExtracting, forcedSystem: false }
  }

  // 相对路径的基准是 **exe 所在目录**（installRoot），不是进程工作目录 —— 后者会随启动方式
  // 变化（快捷方式「起始位置」、命令行、拖拽启动），同一个包会把数据落到不同地方。
  const absolute = isAbsolute(configured)
  const target = absolute ? configured : join(root, configured)

  if (!isWritable(target)) {
    // 只读介质（写保护 U 盘 / Program Files / 只读网络盘）：
    // 退到系统默认目录，保证「能启动」优先于「够绿色」，并在日志里明示。
    const fallback = systemDataDir(opts.appName)
    console.error(`[web2app] 数据目录不可写，已回退系统默认目录：${target} → ${fallback}`)
    applyDataDir(fallback)
    return {
      root,
      dataDir: fallback,
      mode: 'fallback',
      fellBack: true,
      selfExtracting,
      forcedSystem: false,
    }
  }

  applyDataDir(target)
  return {
    root,
    dataDir: target,
    mode: absolute ? 'absolute' : 'relative',
    fellBack: false,
    selfExtracting,
    forcedSystem: false,
  }
}
