/**
 * 启动日志落盘（M6 收尾）—— 绿色版的「第一现场」。
 *
 * ⚠️ 为什么必须有这个文件：Windows 上 `electron.exe` 是 **GUI 子系统**程序，
 *    用户双击运行时 `console.log` 没有任何去处（没有控制台可挂），进程里发生的
 *    一切只存在于空气里；portable 单文件形态更彻底 —— 自解压包装器会吞掉内层
 *    进程的 stdout。于是「双击没反应 / 打开的是 example.com / 标题不对」这类反馈
 *    在用户侧**零证据**，只能靠远程猜。
 *    落一份 `data\logs\startup.log` 之后，用户发一条日志就能定位。
 *
 * 设计取舍（都是刻意的）：
 *   1. **零 Electron 依赖**：只用 node:fs / node:path / node:util。这样它在
 *      `app.setPath()` 之前也能用，也能被纯 Node 脚本直接调用做故障注入测试
 *      （见 `electron-app/AGENTS.md`「日志目录」一节的负向对照命令）。
 *   2. **fail-open**：日志写不出去（只读介质 / 被占用 / 磁盘满）**绝不影响启动** ——
 *      为了记日志让应用起不来是本末倒置。失败只在原始 console 上告警一次，
 *      并停止后续写入尝试（避免每行都抛一次、把真正的启动日志淹没）。
 *   3. **同步追加**：`appendFileSync` 而不是流。启动日志量级是几十行，
 *      同步写的开销可以忽略；好处是异常退出（任务管理器结束 / 崩溃）时
 *      **崩溃前最后的几行一定已经落盘** —— 那正是最需要的东西。
 *   4. **不碰 stdout**：包装 console 时会先调用原始实现，宿主捕获的输出一字不变
 *      （探针 / 冒烟的结论都靠 stdout，绝不能因为加日志而变哑）。
 *
 * ⚠️ 轮转失败（例如沙箱禁止删除）**不**降级为「不写日志」：轮转本身只是防止
 *    文件无限增长，写不进去才是真故障。两者必须是独立的失败域。
 *
 * ⚠️⚠️ 2026-09-22 起它变成**可选**的（`config.json` 的 `debug`，默认 false）：
 *    关闭时本模块什么都不做 —— 不建 `logs\` 目录、不落文件，而且**连 stdout/stderr 的诊断
 *    也一并静默**（见 `silenceConsole()`）——因为「每次启动都在用户数据目录里留一份日志」
 *    不是期望行为，而终端启动时的满屏输出对最终用户同样是噪声。
 *    代价必须写清楚：上面那两大段论证的「用户侧第一现场」在默认配置下**不存在**。
 *    所以留了运行时逃生口 —— `--debug` / 环境变量 `WEB2APP_DEBUG=1`（见 index.ts 的 DEBUG），
 *    不必为了排障重新打包；探针与冒烟模式则内部强制打开（它们的存在意义就是取证）。
 */
import { appendFileSync, mkdirSync, renameSync, statSync, unlinkSync } from 'node:fs'
import { join } from 'node:path'
import { format } from 'node:util'

export const LOG_FILE_NAME = 'startup.log'
/** 轮转后的旧文件（只保留一代，启动日志不需要归档体系） */
export const PREV_LOG_FILE_NAME = 'startup.prev.log'
/** 超过这个大小就在**启动时**轮转一次（运行期不检查，避免每行都 stat） */
const MAX_BYTES = 1024 * 1024

export interface FileLogHandle {
  /** 是否成功落盘。false 时应用照常运行，只是没有日志文件 */
  ok: boolean
  /** 日志文件绝对路径（无论成功与否都给，便于排障时告诉用户去哪儿看） */
  path: string
  /** ok=false 时的失败原因 */
  error?: string
}

export function logFilePath(logDir: string): string {
  return join(logDir, LOG_FILE_NAME)
}

/** 超过上限就把当前文件挪成 startup.prev.log。**失败不抛**，见文件头第 4 条。 */
function rotateIfTooLarge(path: string): void {
  try {
    if (statSync(path).size < MAX_BYTES) return
  } catch {
    return // 文件不存在（首次启动）→ 无需轮转
  }
  const prev = join(path, '..', PREV_LOG_FILE_NAME)
  try {
    unlinkSync(prev)
  } catch {
    /* 旧文件不存在 / 删不掉都无所谓：下面的 rename 若因它存在而失败，也只是这次不轮转 */
  }
  try {
    renameSync(path, prev)
  } catch {
    /* 被占用等情况：放弃轮转，继续往原文件追加 */
  }
}

/** 写一行（带时间戳）。抛错由调用方处理 —— 这里不做静默。 */
function writeLine(path: string, text: string): void {
  appendFileSync(path, `${new Date().toISOString()} ${text}\n`, 'utf8')
}

/** 原始 console 的引用：包装后仍要调用它们，stdout 行为一字不变。 */
const originals = {
  log: console.log.bind(console),
  warn: console.warn.bind(console),
  error: console.error.bind(console),
}

/**
 * 原始 console（未被包装、也不受静默影响）。
 *
 * ⚠️ 只给「必须让人看到」的兜底路径用 —— 目前只有一处：`index.ts` 里 ready 之后
 *    异常终止那条（`启动流程异常终止（已强制非 0 退出）`）。默认 debug 关闭时
 *    `console.error` 是空操作，用它打印等于把唯一的线索也抹掉，所以那儿走这里。
 */
export const rawConsole = originals

let patched = false

/**
 * 把 console 的三种输出同时写一份到日志文件。
 *
 * ⚠️ 只包装一次（`patched` 守卫）：重复调用会把同一行写 N 遍（包装套包装）。
 *    本函数是幂等的，第二次调用只换不重包 —— 保持首次的路径。
 */
function patchConsole(path: string): void {
  if (patched) return
  patched = true

  // 写入失败后置位，避免「每行都失败、每行都告警」把日志刷爆（见文件头第 2 条）
  let broken = false

  const wrap = (kind: 'log' | 'warn' | 'error'): void => {
    const orig = originals[kind]
    console[kind] = ((...args: unknown[]): void => {
      orig(...args)
      if (broken) return
      try {
        writeLine(path, format(...args))
      } catch (e) {
        broken = true
        originals.warn(`[web2app] 日志文件写入失败，本次运行不再尝试：${path}（${(e as Error).message}）`)
      }
    }) as typeof console.log
  }

  wrap('log')
  wrap('warn')
  wrap('error')
}

let silenced = false

/**
 * 静默控制台（`debug=false` 的默认路径）。
 *
 * ⚠️ 与 `patchConsole()` 互斥、且**必须先于**可能出错的那段启动代码调用（index.ts 里紧跟
 *    DEBUG 判定）：晚一步就会漏掉几条本该安静的输出。两者都幂等。
 * ⚠️ 静默后 `console.*` 是空操作 ⇒ 任何「必须让人看到」的错误都要改用 `rawConsole`。
 * ⚠️ 只替换 log/warn/error 三种（与 `patchConsole` 对齐）：console.table 之类本仓库不用。
 */
export function silenceConsole(): void {
  if (silenced || patched) return
  silenced = true
  console.log = () => {}
  console.warn = () => {}
  console.error = () => {}
}

/**
 * 装配文件日志。**必须在 `setupPortablePaths()` 之后调用**（要拿最终的 dataDir）。
 *
 * @param logDir 日志目录（约定为 `<dataDir>\logs`，与 `portable.ts` 里
 *               `app.setPath('logs', …)` 的目标一致）
 */
export function startFileLogging(logDir: string): FileLogHandle {
  const path = logFilePath(logDir)

  try {
    mkdirSync(logDir, { recursive: true })
  } catch (e) {
    const error = (e as Error).message
    // 用**原始** console：此时包装还没装上，也不会递归
    originals.warn(`[web2app] 无法创建日志目录，已跳过文件日志（不影响使用）：${logDir}（${error}）`)
    return { ok: false, path, error }
  }

  rotateIfTooLarge(path)

  try {
    writeLine(path, `===== web2app 启动 =====`)
  } catch (e) {
    const error = (e as Error).message
    originals.warn(`[web2app] 日志文件不可写，已跳过文件日志（不影响使用）：${path}（${error}）`)
    return { ok: false, path, error }
  }

  patchConsole(path)
  return { ok: true, path }
}
