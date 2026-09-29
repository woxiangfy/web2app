/**
 * 分发形态标记读取（**纯规则模块**：零 Electron 依赖）。
 *
 * 为什么单独成文件：`portable.ts` 顶层 `import { app } from 'electron'`，直接单测它
 * 需要 Electron 运行时；而「怎么判定安装版」本身是纯规则，跟 `tab-order.ts` 一样
 * 抽出来即可在 `node --test` 下零依赖验证。
 *
 * 标记从哪来：安装版（NSIS）打包命令通过 electron-builder 的
 *   `-c.extraMetadata.distribution=nsis`
 * 把字段合并进 `app.asar` 内的 package.json；绿色版（dir / zip / portable）不设该字段。
 * 之所以不用「检测 Uninstall*.exe 是否存在」之类的运行时启发式判据：安装目录可被改名、
 * 拷贝，启发式判定会静默失真；构建期写死的标记才是唯一可信来源。
 */
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

/** 安装版的 distribution 取值（NSIS 安装包）。 */
export const INSTALLER_DISTRIBUTION = 'nsis'

/**
 * 从指定目录下的 `package.json` 读取 `distribution` 字段。
 *
 * @param appPath 应用根目录（打包态 = `app.getAppPath()` 即 `…\resources\app.asar`，
 *                Electron 的 fs 对 asar 透明；开发态 = electron-app/）
 * @returns 字段值；文件缺失 / JSON 损坏 / 字段非字符串或为空串时一律返回 null。
 *          ⚠️ 绝不抛错 —— 这是启动早期路径，读不到标记只能说明「不是安装版」，
 *          不能因为一次读取失败阻断启动。
 */
export function readDistribution(appPath: string): string | null {
  try {
    const pkg = JSON.parse(readFileSync(join(appPath, 'package.json'), 'utf8')) as unknown
    if (typeof pkg !== 'object' || pkg === null) return null
    const d = (pkg as Record<string, unknown>).distribution
    return typeof d === 'string' && d.length > 0 ? d : null
  } catch {
    return null
  }
}

/**
 * 是否为安装版构建。
 *
 * 安装版的数据目录策略与绿色版不同（见 `portable.ts::setupPortablePaths`）：
 * 强制使用系统默认程序目录 `%APPDATA%\<app_name>`，**忽略 config.json 的 data_dir**，
 * 由操作系统统一管理安装 / 卸载 / 升级，即使安装目录（Program Files）对标准用户
 * 只读也不受影响。
 */
export function isInstallerBuild(appPath: string): boolean {
  return readDistribution(appPath) === INSTALLER_DISTRIBUTION
}
