/**
 * 设置应用控制器 —— 从 index.ts 抽出的「一份设置如何落到进程级副作用」编排单元。
 *
 * 拥有两件事：
 *   1. `applySettings()`：唤醒快捷键 + 托盘的创建/销毁 + 持久化，以及失败时对**进程级副作用**
 *      的成组回滚（快捷键 / 托盘都要回滚，否则「保存失败」只是半真半假）；
 *   2. 托盘菜单动作 `trayActions()`（仅在创建托盘时内部使用）。
 *
 * 本模块**不直接触碰 Electron 运行时对象**（app / BrowserWindow）：窗口显隐、退出、图标定位
 * 全部经 deps 由入口注入，使这里的「成组回滚」编排是壳无关、可脱离窗口推理的纯逻辑。
 * 与 Tauri 侧 `settings.rs::save_settings` / `tray.rs` 的职责切分对齐。
 */
import { createTray, destroyTray, type TrayActions } from './tray'
import { applyWakeHotkey } from './shortcuts'
import { hotkeyLabel, sameHotkey } from './hotkey'
import { saveSettings, type Settings, type StoreLocation } from './settings'

export interface SettingsControllerDeps {
  /** 窗口 / 托盘 tooltip 标题（config.json 的 title）。 */
  title: string
  /** 读取当前生效的设置（设置窗保存后立即更新的那份内存态）。 */
  getSettings: () => Settings
  /** 提交新的内存态（仅在副作用 + 持久化全部成功后调用）。 */
  setSettings: (s: Settings) => void
  /** settings.json 的读-改-写位置（探针模式下会被改到临时目录，故用 getter）。 */
  getStoreLoc: () => StoreLocation
  /** 主窗口当前是否「存在且可见」（托盘左键判定可见→隐藏用）。 */
  isMainWindowVisible: () => boolean
  openSettings: () => void
  /** 唤起主窗口（多页签空壳时含自愈补建，由入口实现）。 */
  presentMainWindow: () => void
  hideMainWindow: () => void
  /** 切换主窗口显隐（唤醒快捷键 / 托盘左键共用）。 */
  toggleMainWindow: () => void
  /** 真正退出：入口先置 quitting 标志再 app.quit()（否则 close 会被「关闭到托盘」拦下）。 */
  requestQuit: () => void
  /** 托盘图标路径，缺失给 null。 */
  trayIconPath: () => string | null
}

export interface SettingsController {
  /**
   * 应用一份设置：运行时副作用（快捷键 / 托盘）+ 可选持久化。
   * @throws 出错时抛出带用户可读文案的 Error（设置页会把它显示出来）
   */
  applySettings: (next: Settings, opts: { persist: boolean }) => void
}

export function createSettingsController(deps: SettingsControllerDeps): SettingsController {
  function trayActions(): TrayActions {
    return {
      toggle: () => {
        // 可见 → 收进托盘；不可见 → 走带页签自愈的唤起路径（不能直接 toggle，
        // 否则最后一个页签关掉后的空壳无法恢复）。
        if (deps.isMainWindowVisible()) {
          deps.toggleMainWindow()
        } else {
          deps.presentMainWindow()
        }
      },
      onShow: () => deps.presentMainWindow(),
      onHide: () => deps.hideMainWindow(),
      onSettings: () => deps.openSettings(),
      onQuit: () => deps.requestQuit(),
    }
  }

  function applySettings(next: Settings, opts: { persist: boolean }): void {
    const prev = deps.getSettings()

    // 1) 唤醒快捷键
    const r = applyWakeHotkey(next.wake_hotkey, () => deps.toggleMainWindow())
    if (!r.ok) throw new Error(r.error ?? '快捷键注册失败')

    // 记录本调用是否【新建】了托盘：持久化失败时要把它一起回滚，否则界面提示
    // 「保存失败」、文件没写成，但托盘图标已在本次会话驻留 —— 进程级副作用有两个
    //（快捷键 / 托盘），只回滚一个就不叫「任一步失败都不写内存态」。
    let trayCreatedThisCall = false
    // 对称地记录本调用是否【销毁】了托盘：true→false 分支先 destroyTray()，随后若保存
    // 失败，托盘也必须 best-effort 重建 —— 否则界面提示「保存失败」、配置没写成，
    // 本次会话的托盘却没了（与新建侧是同一类自相矛盾，曾经只回滚了一半）。
    let trayExistedBefore = false
    try {
      // 2) 托盘开关变化 → 创建 / 销毁（与旧版一样是「变化才动」，避免每次保存都重建图标）
      if (next.tray_enabled !== deps.getSettings().tray_enabled) {
        if (next.tray_enabled) {
          const ok = createTray({
            title: deps.title,
            iconPath: deps.trayIconPath(),
            actions: trayActions(),
          })
          if (!ok) throw new Error('托盘创建失败（未找到 icons/icon.png）')
          trayCreatedThisCall = true
        } else {
          trayExistedBefore = true
          destroyTray()
        }
      }

      // 3) 持久化
      if (opts.persist) {
        const written = saveSettings(deps.getStoreLoc(), next)
        console.log(`[web2app] 设置已保存：${written}`)
      }
    } catch (e) {
      // ⚠️ 回滚本次已生效的进程级副作用。第 1 步是全局快捷键（进程级资源），
      //    第 2 步可能已经新建托盘 —— 后两步失败时如果不回滚，就会留下这种自相矛盾的状态：
      //    界面上提示「保存失败」、settings.json 没写（下次启动仍是旧值），
      //    但**本次会话里新键 / 新托盘已经生效**。承诺的「任一步失败都不写内存态」
      //    只有在把副作用也一并回滚之后才成立。
      if (trayCreatedThisCall) {
        destroyTray()
        console.warn('[web2app] 保存失败，本次新建的托盘已回滚')
      }
      // 对称回滚：本次销毁过托盘 → best-effort 按旧配置重建（图标文件恰好缺失时只告警，
      // 不能再抛 —— 我们正在错误传播路径上）。
      if (trayExistedBefore) {
        const restored = createTray({
          title: deps.title,
          iconPath: deps.trayIconPath(),
          actions: trayActions(),
        })
        console.warn(
          `[web2app] 保存失败，已尝试重建回滚前的托盘：${restored ? '成功' : '重建也失败（图标缺失？）'}`
        )
      }
      if (!sameHotkey(prev.wake_hotkey, next.wake_hotkey)) {
        const rb = applyWakeHotkey(prev.wake_hotkey, () => deps.toggleMainWindow())
        console.warn(
          `[web2app] 保存失败，唤醒快捷键已回滚到 ${hotkeyLabel(prev.wake_hotkey)}：` +
            `${rb.ok ? '成功' : '回滚也失败（当前无唤醒快捷键）'}`
        )
      }
      throw e
    }

    deps.setSettings(next)
  }

  return { applySettings }
}
