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
import { dirname, join } from 'node:path'

/** 数据目录策略：sibling = exe 同级 data\（默认，真绿色）；user = %APPDATA%（config 指定或只读介质退路） */
export type DataDirMode = 'sibling' | 'user'

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
   * 来自 config.json 的 `data_dir_default`（对齐 webview-app 语义）：
   *   false（默认）= 数据放 exe 同级 data\，整目录可搬走；
   *   true         = 跟随系统，放 %APPDATA%\<app_name>。
   * 注意与 Tauri 版的差异：Tauri 的 false 是写 %LOCALAPPDATA%\<app_name>，
   * Electron 绿色版改写同级 data\，该差异已记录在 docs/electron-plan.md §1.4。
   */
  preferUserDir?: boolean
  /** 应用名（config.app_name），用于 user 模式下的子目录名 */
  appName?: string
}

/**
 * 装配绿色版数据路径，返回实际生效的路径信息供启动日志 / 关于页展示。
 */
export function setupPortablePaths(opts: SetupOptions = {}): PortablePaths {
  const root = installRoot()
  const selfExtracting = isSelfExtracting()

  if (opts.preferUserDir) {
    const dir = join(app.getPath('appData'), opts.appName || 'Web2App')
    app.setPath('userData', dir)
    app.setPath('sessionData', join(dir, 'session'))
    app.setPath('logs', join(dir, 'logs'))
    app.setPath('crashDumps', join(dir, 'crash'))
    return { root, dataDir: dir, mode: 'user', fellBack: false, selfExtracting }
  }

  const sibling = join(root, 'data')

  if (!isWritable(sibling)) {
    // 只读介质（写保护 U 盘 / Program Files / 只读网络盘）：
    // 退到用户目录，保证「能启动」优先于「够绿色」，并在日志里明示。
    const fallback = app.getPath('userData')
    console.error(
      `[web2app] 同级数据目录不可写，已回退用户目录：${sibling} → ${fallback}`
    )
    return { root, dataDir: fallback, mode: 'user', fellBack: true, selfExtracting }
  }

  // ⚠️ 以下四项必须一起重定向。只改 userData 会留下 sessionData（Cookies /
  // Local Storage 的一部分）与日志、崩溃转储仍在系统盘，导致「绿色版不干净」。
  app.setPath('userData', sibling)
  app.setPath('sessionData', join(sibling, 'session'))
  app.setPath('logs', join(sibling, 'logs'))
  app.setPath('crashDumps', join(sibling, 'crash'))

  return { root, dataDir: sibling, mode: 'sibling', fellBack: false, selfExtracting }
}
