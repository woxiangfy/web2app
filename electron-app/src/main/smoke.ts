/**
 * `--smoke` 启动自检 —— 从 index.ts 抽出的自检单元（与 probe.ts 同为 CI 取证设施）。
 *
 * 存在的意义：绿色版路径 / 配置解析 / 模板替换都是「错了也静默」的逻辑，有了这个开关
 * 才能在 CI 里无人值守地验证它们（Tauri 版当年为此专门编过探针 exe）。
 *
 * 边界：本模块只做检查 + 输出结论行，**退出动作留给调用方**（index.ts 调 app.exit），
 * 因此它不依赖 Electron 运行时，传入的值都是已解析好的快照。
 *
 * ⚠️ 日志现在是 debug 门控的（默认关）⇒ 冒烟模式在 index.ts 被**强制** DEBUG=true，
 *    否则「日志已包含启动信息」这条断言在默认配置下必然失败 —— 那不是发现缺陷，是判据错了。
 */
import { readFileSync } from 'node:fs'

import type { FileLogHandle } from './startup-log'

export interface SmokeCheckDeps {
  /** 配置里的期望窗口标题（config.json 的 title） */
  title: string
  /** 窗口实际标题（createWindow 后 mainWindow.getTitle() 的快照） */
  actualTitle: string
  /** 启动页 HTML 渲染结果（splash-page.ts 的 render） */
  renderSplash: () => string
  /** 设置页 HTML 源码（null = 资源缺失） */
  readSettingsPage: () => string | null
  /** 托盘图标绝对路径（null = 缺失） */
  trayIconPath: () => string | null
  /** 文件日志句柄（冒烟模式本应非 null 且 ok=true） */
  fileLog: FileLogHandle | null
}

/**
 * 跑完全部冒烟断言并逐条打印结论。
 * @returns 是否存在失败项（true = 有失败，调用方应以非 0 退出码退出）
 */
export function runSmokeChecks(deps: SmokeCheckDeps): boolean {
  // 顺手断言启动页模板替换是否完整 —— 三个占位符漏一个，
  // 表现就是「标题栏或加载动画不出现」这种要花半天定位的故障。
  const html = deps.renderSplash()
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
  //  页面里的字面量、垫片白名单、IPC 注册名三处必须逐字一致。
  const page = deps.readSettingsPage() ?? ''
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
  const icon = deps.trayIconPath()
  console.log(`[web2app] 托盘图标：${icon ?? '未找到'}`)

  // 窗口标题必须等于配置里的 title。
  // ⚠️ 这条断言是 2026-09-17 加的：createWindow 曾经漏传 `title`，
  // 而 page-title-updated 被拦，结果是标题永远停在 Electron 默认名
  // （"web2app-electron"）—— 配置里的 title 静默失效，只有人眼看任务栏才发现。
  const titleOk = deps.actualTitle === deps.title
  console.log(
    `[web2app] 窗口标题：'${deps.actualTitle}'（期望 '${deps.title}'）${titleOk ? '' : '  ✗ 不一致！'}`
  )

  // 文件日志（M6）必须**真的写出来了**，且已包含 logStartup() 的内容。
  // ⚠️ 这条和上面三条一样属于「错了也静默」：文件没写出来，用户下次报
  //    「双击没反应」时我们手里就什么证据都没有。
  //    注意断言的是「内容已落盘」，不只是「文件存在」—— 只判存在的话，
  //    一个空文件（比如包装没装上、只有 header）也会让它通过。
  let logOk = false
  let logDetail: string
  let logPath = '(未启用)'
  if (!deps.fileLog) {
    logDetail = '未启用（冒烟模式本应强制打开 debug，出现即 bug）'
  } else {
    logPath = deps.fileLog.path
    if (!deps.fileLog.ok) {
      logDetail = `不可写（${deps.fileLog.error ?? '未知原因'}）`
    } else {
      try {
        const text = readFileSync(deps.fileLog.path, 'utf8')
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

  return leftover.length > 0 || !page || missingCmds.length > 0 || !icon || !titleOk || !logOk
}
