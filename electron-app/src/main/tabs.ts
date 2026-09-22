/**
 * 多页签管理（M5）—— 对齐 Tauri 版 `webview-app/src/tabs.rs` 的**职责划分**，但去掉
 * 几何校准与幽灵窗口清理那两套东西（Electron 侧用同一窗口内的子 `WebContentsView`，
 * 不需要 `place_tab` 闭环、也不需要 `destroy()` 防重入）。
 *
 * 形态 = 方案 A′（见 docs/electron-tabs-design.md）：
 *   BrowserWindow（壳，窗口自身页面 = shell.html 画标签栏）
 *   └─ contentView[i]：子 WebContentsView，bounds = { x:0, y:38, w:W, h:H-38 }
 *
 * ⚠️ 硬约束（都来自实测，Electron 44.4.1）：
 *   1. 内容视图 bounds 必须用 **getContentBounds()** 推算（无边框窗口在 Windows 上与
 *      getBounds() 差 1px，用错会在右/上边留缝或压住 1px）—— 见 §3.5-3。
 *   2. 切换用 setVisible（双向可读、不触发重排）；抬层用 remove+add（View 无 setZIndex）。
 *   3. 视图不会随窗口关闭自动销毁 ⇒ closeTab 必须 webContents.close() + 清掉 Map 引用。
 *   4. chrome（壳的 0–38）与内容永不重叠：重叠时拖拽区判定为并集且上游明确不修
 *      （electron#43320）。
 *
 * 本文件只负责「页签集合」的增删切换与几何；导航/代理/钩子由 index.ts 注入
 * （onSpawn 回调），避免在这里反向依赖主进程的 bootstrap 链路。
 */
import { WebContentsView, type BrowserWindow } from 'electron'

/** 标签栏高度（逻辑像素）。与 shell.html 的 `--bar-h`、Tauri 的 `TAB_BAR_HEIGHT` 一致，勿改。 */
export const TAB_BAR_HEIGHT = 38

/** 页签上限：对齐 Tauri `MAX_TABS`（防「异常建窗风暴」，见 tabs.rs 注释）。 */
export const MAX_TABS = 32

/** 单个页签。 */
export interface Tab {
  /** 稳定标识（用于 switch_tab / close_tab 的 label 契约，与 Tauri 侧 `label` 同义） */
  label: string
  /** 承载网页内容的子视图 */
  view: WebContentsView
  /** 页签标题（page-title-updated 回传，标签栏显示用） */
  title: string
}

/** 推给壳页面渲染的页签条目（字段与 shell.html 的 render(items) 消费契约一致）。 */
export interface TabItem {
  label: string
  title: string
  active: boolean
}

export interface TabsOptions {
  /** 宿主窗口（壳）。用于拿 getContentBounds 与挂子视图。 */
  host: BrowserWindow
  /**
   * 新建页签时的回调：index.ts 在这里做「复用 target 缓存 + loadURL」，
   * 而不是让 tabs.ts 反向调 bootstrap()（那会掐掉在服页签的代理，见设计文档必炸点②）。
   * @param view 刚挂好的内容视图（已 addChildView + setBounds）
   * @param label 页签标识
   * @param url 新建页签时要打开的目标（首屏为 undefined，走默认 target；window.open 时为具体 URL）
   * 抛异常表示 spawn 失败（如 target 尚未解析），此时本方法会回收刚建的视图。
   */
  onSpawn: (view: WebContentsView, label: string, url?: string) => Promise<void> | void
}

export class TabManager {
  private readonly host: BrowserWindow
  private readonly onSpawn: TabsOptions['onSpawn']
  /** 页签按创建顺序保存；`active` 用 Map 值而非数组下标，避免「删中间页签」时全体错位。 */
  private readonly tabs = new Map<string, Tab>()
  private activeLabel: string | null = null
  private seq = 0

  constructor(opts: TabsOptions) {
    this.host = opts.host
    this.onSpawn = opts.onSpawn
  }

  get count(): number {
    return this.tabs.size
  }

  get active(): Tab | null {
    return this.activeLabel ? (this.tabs.get(this.activeLabel) ?? null) : null
  }

  /** 内容视图的可绘制区域（物理上从 y=38 起，高度扣掉标签栏）。 */
  private contentBounds(): { x: number; y: number; width: number; height: number } {
    const cb = this.host.getContentBounds()
    return {
      x: 0,
      y: TAB_BAR_HEIGHT,
      width: cb.width,
      // ⚠️ 钳零：窗口被压到比标签栏还矮时（minHeight=300 只在正常情况下兜得住，
      //    「拖到屏幕边缘 / 程序化 setBounds」都能绕开），h-38 会是负数。
      //    负高度传给 setBounds 是未定义行为，轻则该视图不渲染、重则整层视图错位。
      height: Math.max(0, cb.height - TAB_BAR_HEIGHT),
    }
  }

  /**
   * 新建一个页签并置为激活。
   *
   * @returns 新页签的 label；超限返回 null（上层据此拒绝或提示，绝不静默建窗）。
   */
  async spawnTab(url?: string): Promise<string | null> {
    if (this.tabs.size >= MAX_TABS) {
      console.warn(`[web2app] 页签数已达上限 ${MAX_TABS}，拒绝新建`)
      return null
    }
    const label = `tab-${++this.seq}`

    const view = new WebContentsView({
      webPreferences: {
        // 安全基线与主窗口一致：不因为「是内容页」就放松任何一项
        contextIsolation: true,
        nodeIntegration: false,
        sandbox: true,
        webviewTag: false,
        // 探针下不节流（避免断言失真），与 index.ts 的 backgroundThrottling 口径一致。
        // 由 index.ts 通过第二个构造参数注入 PROBE 态更干净，但这里保持零依赖：
        // 节流与否不影响 tabs 本体的几何与切换语义，交给 index.ts 在挂完钩子后自行调整。
        backgroundThrottling: true,
      },
    })
    // 顶住首帧白闪（WebContentsView 默认白底；shell.html 的 --bg 是 #f7f8fa）
    view.setBackgroundColor('#f7f8fa')

    const tab: Tab = { label, view, title: '' }
    this.tabs.set(label, tab)

    // 先挂进 contentView 再定位：addChildView 之后 setBounds 才能稳定生效
    this.host.contentView.addChildView(view)
    view.setBounds(this.contentBounds())

    try {
      await this.onSpawn(view, label, url)
    } catch (e) {
      // spawn 失败：回收刚建的视图与条目，不留半成品
      this.tabs.delete(label)
      try {
        this.host.contentView.removeChildView(view)
        view.webContents.close()
      } catch {
        /* close 幂等，失败只影响资源回收 */
      }
      throw e
    }

    // 新页签置为激活（旧激活页签隐藏）
    this.activate(label)
    return label
  }

  /** 激活指定页签：只该页签可见，其余全隐藏（不许「全都可见、靠 z-order 盖」）。 */
  activate(label: string): void {
    const tab = this.tabs.get(label)
    if (!tab) return
    this.activeLabel = label
    for (const [l, t] of this.tabs) {
      t.view.setVisible(l === label)
    }
    this.pushToShell()
  }

  /** 关闭页签；返回是否真的关了（label 不存在返回 false）。 */
  closeTab(label: string): boolean {
    const tab = this.tabs.get(label)
    if (!tab) return false
    const wasActive = this.activeLabel === label

    this.tabs.delete(label)
    try {
      // ⚠️ 必须显式 removeChildView：只调 webContents.close() 时，视图对象仍挂在
      //   contentView.children 里（探针实测 children 不减少、关闭的视图仍可见），
      //   且 webContents 销毁后视图是否自动摘除有异步时序依赖 —— 显式摘除是确定性的。
      //   removeChildView 对「非子视图」是 no-op，安全幂等。
      this.host.contentView.removeChildView(tab.view)
      tab.view.webContents.close()
    } catch {
      /* close 幂等 */
    }

    if (this.tabs.size === 0) {
      // 关掉最后一个页签 → 关闭宿主窗口（走 index.ts 的统一退出语义）
      this.activeLabel = null
      this.host.close()
      return true
    }

    if (wasActive) {
      // 激活态落到相邻页签：优先右侧，没有则左侧（对齐 Tauri close_tab_inner 语义）
      const labels = [...this.tabs.keys()]
      const idx = labels.findIndex((l) => l === label)
      const next = labels[Math.min(idx + 1, labels.length - 1)]
      this.activate(next)
    }
    return true
  }

  /** 窗口 resize → 重算所有视图 bounds（DIP 一次到位，无需闭环校准）。 */
  relayout(): void {
    const b = this.contentBounds()
    for (const t of this.tabs.values()) {
      if (!t.view.webContents.isDestroyed()) t.view.setBounds(b)
    }
  }

  /**
   * 重新加载全部页签，返回实际重载的页签数。
   *
   * 用途：设置页「清除全部」清完浏览数据后，让**每个**页签都回到「无本地数据」的干净状态。
   * 只重载当前页签是不够的：其它页签的 DOM 里还留着登录后的内容，
   * 用户切过去会以为「清了但没生效」。
   */
  reloadAll(): number {
    let n = 0
    for (const t of this.tabs.values()) {
      if (t.view.webContents.isDestroyed()) continue
      t.view.webContents.reload()
      n++
    }
    return n
  }

  /** 销毁全部页签（宿主退出前显式 close，官方明示视图不随窗口销毁自动回收）。 */
  destroyAllTabs(): void {
    for (const t of this.tabs.values()) {
      try {
        this.host.contentView.removeChildView(t.view)
        t.view.webContents.close()
      } catch {
        /* ignore */
      }
    }
    this.tabs.clear()
    this.activeLabel = null
  }

  /** 当前页签列表（对象形态，供 pushToShell 的 render() 消费）。 */
  list(): TabItem[] {
    return [...this.tabs.values()].map((t) => ({
      label: t.label,
      title: t.title || t.label,
      active: t.label === this.activeLabel,
    }))
  }

  /**
   * get_tabs IPC 的返回契约：**元组数组** `[label, title, active]`。
   *
   * ⚠️ 与 `list()`（对象形态）刻意分开：Tauri 侧 `get_tabs` 返回
   *   `Vec<(String, String, bool)>`，序列化成 JSON 后是 `[[label,title,active], ...]`；
   *   `shell.html` 用 `list.map(([label, title, active]) => …)` 解构（见 shell.html:266）。
   *   若这里返回对象数组，解构会得到 `[undefined, undefined, undefined]`，标签栏渲染空白。
   *   而 `pushToShell` 走的是 `render({label,title,active})` 对象契约（shell.html:211），
   *   两者不能混用 —— 这是双壳单一真源下「同一份 shell.html 两套消费形态」的硬约束。
   */
  listForIpc(): [string, string, boolean][] {
    return [...this.tabs.values()].map((t) => [
      t.label,
      t.title || t.label,
      t.label === this.activeLabel,
    ])
  }

  /** 把当前页签列表推给壳页面刷新标签栏。 */
  pushToShell(): void {
    const items = this.list()
    this.host.webContents
      .executeJavaScript(
        `window.__web2appTabs && window.__web2appTabs.render(${JSON.stringify(items)})`,
        true
      )
      .catch(() => {
        /* 壳页面未就绪时静默：下一次 activate/push 会重推 */
      })
  }

  /** 更新页签标题（page-title-updated 回传后调用）。 */
  setTabTitle(label: string, title: string): void {
    const t = this.tabs.get(label)
    if (!t) return
    t.title = title
    this.pushToShell()
  }
}
