/**
 * 多页签管理（M5）—— 对齐 Tauri 版 `webview-app/src/tabs.rs` 的**职责划分**，但去掉
 * 几何校准与幽灵窗口清理那两套东西（Electron 侧用同一窗口内的子 `WebContentsView`，
 * 不需要 `place_tab` 闭环、也不需要 `destroy()` 防重入）。
 *
 * 形态 = 方案 A′（见 docs/electron-tabs-design.md）：
 *   BrowserWindow（壳，窗口自身页面 = shell.html 画标签栏）
 *   └─ contentView[i]：子 WebContentsView，bounds = { x:0, y:38, w:W, h:H-38 }
 *
 * ⚠️ 硬约束（都来自实测：Electron 44.4.1 定基线，45.3.2 复测一致）：
 *   1. 内容视图 bounds 必须用 **getContentBounds()** 推算（无边框窗口在 Windows 上与
 *      getBounds() 差 1px，用错会在右/上边留缝或压住 1px）—— 见 §3.5-3。
 *   2. 切换用 setVisible（双向可读、不触发重排）；抬层用 remove+add（View 无 setZIndex）。
 *   3. 视图不会随窗口关闭自动销毁 ⇒ closeTab 必须 webContents.close() + 清掉 Map 引用。
 *   4. chrome（壳的 0–38）与内容永不重叠：重叠时拖拽区判定为并集且上游明确不修
 *      （electron#43320）。
 *
 * 本文件只负责「页签集合」的增删切换与几何；导航/代理/钩子由 index.ts 注入
 * （onSpawn 回调），避免在这里反向依赖主进程的 bootstrap 链路。
 *
 * **首页页签**：集合里的**第一个**（打开 app_url 的那一签）固定在第一位且**不允许关闭**，
 * 条目里以 `home: true` 下发给标签栏（→ 名字前画房子图标、不画 ×）。判定规则在
 * `tab-order.ts::isPinnedHome`，与 Tauri `tabs.rs::is_home_index` 同一口径。
 */
import { WebContentsView, type BrowserWindow } from 'electron'
// 页签的纯顺序 / 归属规则（关闭后的邻接选择、首页判定）抽在无 Electron 依赖的 tab-order.ts
// 里，可直接 node:test 钉住（tabs.ts 顶层 import electron，node 下加载必炸）。
import { isPinnedHome, nextActiveLabelAfterClose, shouldCloseTabAfterDownload } from './tab-order'

/** 标签栏高度（逻辑像素）。与 shell.html 的 `--bar-h`、Tauri 的 `TAB_BAR_HEIGHT` 一致，勿改。 */
export const TAB_BAR_HEIGHT = 38

/** 页签上限：对齐 Tauri `MAX_TABS`（防「异常建窗风暴」，见 tabs.rs 注释）。 */
export const MAX_TABS = 32

/**
 * 「弹窗兜底重复下载」的识别窗口（毫秒）。
 *
 * 站点兜底是近同步发生的（`window.open()` 返回 null 后立即 `location.href = 同一地址`），
 * 3s 足够宽；窗口越短，越不容易误伤「用户真的又点了一次同一个下载链接」。
 */
const POPUP_FALLBACK_WINDOW_MS = 3000

/** 单个页签。 */
export interface Tab {
  /** 稳定标识（用于 switch_tab / close_tab 的 label 契约，与 Tauri 侧 `label` 同义） */
  label: string
  /** 承载网页内容的子视图 */
  view: WebContentsView
  /** 页签标题（page-title-updated 回传，标签栏显示用） */
  title: string
  /**
   * 是否已提交过文档（`did-navigate` 之后置 true，由 `tab-spawn.ts` 的钩子写入）。
   *
   * 下载响应**不产生文档提交**，所以「为一个下载而开的页签」始终是 `false` ——
   * 这正是 `closeTabAfterDownload()` 判断该不该自动关页签的唯一依据
   * （规则本体在 `tab-order.ts::shouldCloseTabAfterDownload`）。
   */
  committed: boolean
  /**
   * 打开本页签的那个页签 label（只有 `window.open` / `target="_blank"` 建的页签才有）。
   *
   * 下载完成后自动关页签时用它把激活态拉回**点击时的页面**（浏览器语义），
   * 而不是 `closeTab` 默认的「相邻页签」（页签栈里打开者未必紧挨在左边）。
   */
  opener?: string
}

/** 推给壳页面渲染的页签条目（字段与 shell.html 的 render(items) 消费契约一致）。 */
export interface TabItem {
  label: string
  title: string
  active: boolean
  /**
   * 是否首页页签（多页签列表里的**第一个**，即入口 app_url 那一签）。
   *
   * 首页固定在第一位、**不允许关闭**，标签栏里名字前带房子图标作标记 —— shell.html 靠这个
   * 字段决定「画不画图标 / 画不画 ×」。判定规则见 `tab-order.ts::isPinnedHome`（与 Tauri
   * `tabs.rs::is_home_index` 同一条：位次 0）。
   */
  home: boolean
}

export interface TabsOptions {
  /** 宿主窗口（壳）。用于拿 getContentBounds 与挂子视图。 */
  host: BrowserWindow
  /**
   * 新建页签时的回调：index.ts 在这里做「挂钩子 + 复用 target 缓存 + loadURL」，
   * 而不是让 tabs.ts 反向调 bootstrap()（那会掐掉在服页签的代理，见设计文档必炸点②）。
   *
   * ⚠️ 调用时机：本回调被调用时视图**已挂好（addChildView + setBounds）并已置为激活**
   * （spawnTab 在调用前先 activate），所以这里只负责挂钩子 + 发起导航；loadURL 可以
   * 慢慢等整页加载——标签栏与内容视图的切换反馈已经发出，不需要、也不应当由本回调
   * 来决定何时激活。spawnTab 仍会 await 本回调，仅为捕获建签失败做回滚。
   * @param view 已挂好并激活的内容视图
   * @param label 页签标识
   * @param url 新建页签时要打开的目标（首屏为 undefined，走默认 target；window.open 时为具体 URL）
   * 抛异常表示 spawn 失败（如 target 尚未解析），此时本方法会回收刚建的视图并回滚激活态。
   */
  onSpawn: (view: WebContentsView, label: string, url?: string) => Promise<void> | void
  /**
   * 内容页 preload 的绝对路径（`out/renderer/tab-preload.js`，由 index.ts 经 `paths.ts`
   * 的 `preloadPath()` 解析）。
   *
   * 只注入 `window.web2appExtra` —— 页签内容页**不挂**主 preload（那套会把自绘标题栏 /
   * 加载层塞进页面，而页签模式的标题栏属于壳页面）。缺省不传 = 不注入（单测 / 探针桩可用），
   * 生产路径必须传，否则 `window.web2appExtra` 在页签模式下恒为 undefined（2026-11-16 实测）。
   */
  tabPreload?: string
}

export class TabManager {
  private readonly host: BrowserWindow
  private readonly onSpawn: TabsOptions['onSpawn']
  /** 内容页最小 preload（注入 `window.web2appExtra`）；undefined = 不注入。 */
  private readonly tabPreload: string | undefined
  /** 页签按创建顺序保存；`active` 用 Map 值而非数组下标，避免「删中间页签」时全体错位。 */
  private readonly tabs = new Map<string, Tab>()
  private activeLabel: string | null = null
  private seq = 0
  /**
   * `window.open` 目标 URL → 打开它的页签 webContents id（带过期时间）。
   *
   * ⚠️ 这是**纵深防御**，不是主判据。历史：页签模式曾 `deny` 弹窗 ⇒ 页面 `window.open()`
   * 拿到 **null** ⇒ 站点据此判「弹窗被拦截」并对同一地址再做 `location.href` 兜底 ⇒ 两次
   * 下载、两个「另存为」。现在改为 allow + 隐藏窗口（页面拿到真实 WindowProxy，见
   * `tab-spawn.ts` 的 `did-create-window`），这条兜底不再触发；本登记只用于兜住「站点在
   * 隐藏窗口被销毁后轮询 `w.closed` 再兜底」这类写法（同地址 + 同打开者 + 3s，见
   * `shouldDropPopupFallback`）。
   */
  private readonly expectedPopupDownloads = new Map<
    string,
    { openerWebContentsId: number; expiresAt: number }
  >()
  /** 被判定为「弹窗兜底重复下载」而取消的次数（观测用，页签探针 P15 断言依赖）。 */
  private droppedPopupFallbacks = 0
  /**
   * 「隐藏弹窗窗口」登记：webContents id → 收尾（幂等销毁该窗口）+ 兜底定时器。
   *
   * 页签模式不再 `deny` 弹窗（否则页面的 `window.open()` 返回 null，站点会走
   * `location.href` 兜底造成双下载），而是允许 Electron 建一个**隐藏窗口**让页面拿到真实
   * WindowProxy。这个窗口的生命周期必须**覆盖整次下载**：站点常见的拦截判据是
   * `if (!w || w.closed)`，甚至延迟几十~几百毫秒再查 —— 太早销毁会让 `w.closed === true`，
   * 站点照样兜底，于是又出现第二个「另存为」（只靠 URL 判重挡不住地址不同字的兜底）。
   *
   * 收尾时机：下载 `done`（完成 / 取消 / 中断）、它提交了文档（普通页面弹窗）、或兜底超时。
   */
  private readonly hiddenPopups = new Map<
    number,
    {
      cleanup: () => void
      timer: ReturnType<typeof setTimeout>
      tabLabel?: string
      /** 它自己已经发起过下载（此后即使有文档提交事件也不能提前销毁） */
      downloading?: boolean
    }
  >()

  constructor(opts: TabsOptions) {
    this.host = opts.host
    this.onSpawn = opts.onSpawn
    this.tabPreload = opts.tabPreload
  }

  get count(): number {
    return this.tabs.size
  }

  get active(): Tab | null {
    return this.activeLabel ? (this.tabs.get(this.activeLabel) ?? null) : null
  }

  /**
   * 该 webContents 是否属于某个页签的内容视图。
   *
   * 用途：内容页专用 IPC（账号密码查询 / 保存）的 sender 闸门 —— 页签模式下内容页是
   * 独立 `WebContentsView`，其 id 既不等于宿主壳，也不是 `BrowserWindow.fromWebContents`
   * 能区分出来的（见 `ipc-guard.ts` 的说明）。
   */
  ownsWebContents(id: number): boolean {
    for (const tab of this.tabs.values()) {
      if (tab.view.webContents.id === id) return true
    }
    return false
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
   * 新建一个页签并**立即**置为激活（标签栏与内容视图当场切换），导航在激活状态下进行，
   * 不等待页面加载完成 —— 与 Tauri spawn_tab_with_target 的「先发起导航、立刻激活」对齐。
   *
   * @returns 新页签的 label；超限返回 null（上层据此拒绝或提示，绝不静默建窗）。
   * @param url 目标地址（首屏为 undefined；window.open 场景是具体 URL）
   * @param opener 发起这次打开的页签 label（仅 window.open 场景传入，供下载后回到原页签）
   */
  async spawnTab(url?: string, opener?: string): Promise<string | null> {
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
        // 探针下不节流（避免隐藏视图的定时器被节流导致断言失真）。生产视图不设此项，
        // 用 Chromium 默认（后台标签降速省电）；2026-09 订正：旧注释声称「index.ts 挂完
        // 钩子后会自行调整 backgroundThrottling」，该机制不存在（index.ts 从不改它）。
        backgroundThrottling: true,
        // 只注入 `window.web2appExtra` 的最小 preload（见 tab-preload.ts 顶部注释）。
        // ⚠️ 不能挂 `preload.js`：那套会往页面塞自绘标题栏 / 加载层，页签模式的标题栏
        //    属于壳页面 shell.html，塞进去就是两条标题栏。
        ...(this.tabPreload ? { preload: this.tabPreload } : {}),
      },
    })
    // 顶住首帧白闪（WebContentsView 默认白底；#f7f8fa 与启动页 / 加载层底色一致）
    view.setBackgroundColor('#f7f8fa')

    const tab: Tab = { label, view, title: '', committed: false, ...(opener ? { opener } : {}) }
    this.tabs.set(label, tab)

    // 先挂进 contentView 再定位：addChildView 之后 setBounds 才能稳定生效
    this.host.contentView.addChildView(view)
    view.setBounds(this.contentBounds())

    // ⚠️⚠️ 必须在 onSpawn（其内部 await loadURL，整页加载完成才 resolve）**之前**激活：
    //   旧实现把 activate 放在 `await onSpawn` 之后，导致新页签的标签栏条目/激活态
    //   （pushToShell）和内容视图切换全部拖到页面加载完成才发生——正是「点了标签要等
    //   页面加载完 UI 才跳过来」的根因（Tauri 侧 spawn_tab_with_target 是建窗后先
    //   navigate_tab 发起导航、**立刻** activate_tab，不等加载）。
    //   另一重隐患：新视图 addChildView 后默认可见且位于子视图栈顶，未激活期间会盖住
    //   旧页签；此时用户点别的页签把它隐藏，加载结束后迟到的 activate 又会把视图拽回
    //   新页签。先激活则旧页签同步隐藏、且加载结束后不再有二次激活。
    const previousLabel = this.activeLabel
    this.activate(label)

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
      // 回滚激活态：失败页签已在上面对旧页签调过隐藏，这里把它恢复；
      // 首签失败（无旧页签）时推空列表，由 spawnFirstTab 的失败遮罩接管。
      if (previousLabel && this.tabs.has(previousLabel)) this.activate(previousLabel)
      else this.pushToShell()
      throw e
    }

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

  /**
   * 关闭页签；返回是否真的关了。
   *
   * 返回 `false` 有两种含义（调用方只需知道「没关掉」，日志/探针靠本注释区分）：
   *   · label 不在集合里；
   *   · label 是**首页页签**（`isPinnedHome`）—— 固定第一位、不允许关闭。
   */
  closeTab(label: string): boolean {
    const tab = this.tabs.get(label)
    if (!tab) return false
    const wasActive = this.activeLabel === label

    // ⚠️ 必须在【删除前】快照 label 列表与下标：删除后再 findIndex(label) 恒为 -1，
    // 邻接选择会退化成「永远激活第一个幸存页签」（见 nextActiveLabelAfterClose 注释）。
    const labelsBefore = [...this.tabs.keys()]
    const removedIndex = labelsBefore.indexOf(label)

    // ⚠️⚠️ 首页拦截必须排在**一切副作用之前**（不删登记、不销毁视图、不推壳），与 Tauri
    //    `close_tab_inner` 的第 ① 步同一条纪律：先动后判会留下「登记没了、窗口还在」的半具
    //    残骸 —— 而标签栏本就不给首页渲染 ×，用户既点不开也关不掉它。
    if (isPinnedHome(labelsBefore, label)) {
      console.warn(`[web2app] 拒绝关闭首页页签（固定在第一位、不可关）：${label}`)
      return false
    }

    this.tabs.delete(label)
    // 绑在这个页签上的隐藏弹窗（只为让它的 window.open() 拿到非 null）随页签一起收尾
    for (const [id, rec] of [...this.hiddenPopups]) {
      if (rec.tabLabel === label) this.releaseHiddenPopup(id)
    }
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
      // 关掉最后一个页签 → 关闭宿主窗口（走 index.ts 的统一退出语义）。
      // ⚠️ 首页固定第一位且不可关闭 ⇒ 正常运行期走不到这里；保留它是为了兜住
      //    「首页视图异常未建立 / 集合被外部清空」这类非正常态（不兜会留个关不掉的空壳）。
      this.activeLabel = null
      this.host.close()
      return true
    }

    if (wasActive) {
      // 激活态落到相邻页签：优先右邻，末位则左邻（对齐 Tauri close_tab_inner 语义）。
      // 走到这里至少还有一个幸存页签（size===0 的分支上面已 return），next 必非 null。
      const next = nextActiveLabelAfterClose(labelsBefore, removedIndex)
      if (next) this.activate(next)
    } else {
      // ⚠️ 关的是【后台页签】：activate() 不会被走到，而 shell.html 点 × 后不做本地
      // 乐观更新、完全依赖宿主推送 —— 不补这条推送，被关页签会以「幽灵页签」残留在
      // 标签栏 DOM 里，直到下一次切换/新建/标题推送（对齐 Tauri close_tab_inner
      // 非活动分支里显式的 push_tabs_to_shell）。
      this.pushToShell()
    }
    return true
  }

  /**
   * 标记页签已提交过文档（`tab-spawn.ts` 的 `did-navigate` 钩子调用）。
   * 下载响应不提交文档，故「只为下载而开」的页签永远是未提交状态。
   */
  markTabCommitted(label: string): void {
    const t = this.tabs.get(label)
    if (t) t.committed = true
  }

  /** 按 webContents id 找页签（下载回调用）。找不到 = 不是页签（宿主窗口 / 已关闭）。 */
  private findByWebContentsId(webContentsId: number): Tab | null {
    for (const t of this.tabs.values()) {
      if (t.view.webContents.id === webContentsId) return t
    }
    return null
  }

  /**
   * 在 `setWindowOpenHandler` 里**同步**登记「即将交给新页签的目标地址」。
   *
   * `deny` 会让页面的 `window.open()` 返回 null，站点常据此判定「弹窗被拦截」并立即对
   * 同一地址做 `location.href` 兜底（同一次点击 → 两次下载 / 两个「另存为」）。这里提前
   * 记下 URL + 打开者，等那条兜底下载到达时由 {@link shouldDropPopupFallback} 认出来取消。
   * 普通页面 URL 也会登记，但判重只在真的发生下载时生效，对导航无影响。
   */
  noteExpectedPopupDownload(openerLabel: string, url: string): void {
    const opener = this.tabs.get(openerLabel)
    if (!opener) return
    this.expectedPopupDownloads.set(url, {
      openerWebContentsId: opener.view.webContents.id,
      expiresAt: Date.now() + POPUP_FALLBACK_WINDOW_MS,
    })
  }

  /**
   * 登记「临时隐藏弹窗窗口」（见 {@link hiddenPopups}）。由 tab-spawn 在 `did-create-window`
   * 里、建窗之后立刻调用。
   *
   * @param webContentsId 隐藏窗口的 webContents id（也是它发起下载时的发起者 id）
   * @param cleanup 幂等收尾（销毁那个窗口）；由本类在下载结束 / 提交文档 / 超时时调用
   * @param capMs 兜底存活上限：既没下载也没提交文档（站点开了窗却什么也没做）时强制收尾
   */
  registerHiddenPopup(webContentsId: number, cleanup: () => void, capMs: number): void {
    const timer = setTimeout(() => {
      console.warn(`[web2app] 隐藏弹窗 ${webContentsId} 超时未产生下载或页面，已销毁`)
      this.releaseHiddenPopup(webContentsId)
    }, capMs)
    // 不让这个兜底定时器成为「进程为何不退出」的理由
    ;(timer as unknown as { unref?: () => void }).unref?.()
    this.hiddenPopups.set(webContentsId, { cleanup, timer })
  }

  /** 这个 webContents 是不是「隐藏弹窗窗口」（它发起的下载都要丢弃）。 */
  isHiddenPopup(webContentsId: number): boolean {
    return this.hiddenPopups.has(webContentsId)
  }

  /**
   * 隐藏弹窗开始下载了 ⇒ 它要活到下载结束，取消兜底定时器。
   *
   * ⚠️ 不能在这里就销毁窗口：WebView2/Chromium 的下载依附发起它的 webContents，
   *    下载中途销毁会让文件截断；收尾一律等 `onDownloadDone`。
   */
  noteHiddenPopupDownloadStarted(webContentsId: number): void {
    const rec = this.hiddenPopups.get(webContentsId)
    if (!rec) return
    rec.downloading = true
    clearTimeout(rec.timer)
  }

  /**
   * 隐藏弹窗提交了文档 ⇒ 说明这是**普通页面弹窗**（不是下载），可以收尾了。
   *
   * ⚠️ 已经发起过下载的绝不在此收尾：那会让页面里的 `w.closed` 变 true，站点据此兜底
   *    再下一次（「一次点击两个另存为」）。那种情况只能等页签关闭。
   */
  releaseHiddenPopupOnNavigate(webContentsId: number): void {
    const rec = this.hiddenPopups.get(webContentsId)
    if (!rec || rec.downloading) return
    this.releaseHiddenPopup(webContentsId)
  }

  /**
   * 把隐藏弹窗绑到「承载它这次下载的那个新页签」：页签关闭（下载结算后自动关）时才销毁它。
   *
   * 这是最精确的收尾时机 —— 早于下载结算就销毁，页面里的 `w.closed` 会变 true，站点据此
   * 兜底再下一次（用户看到的第二个「另存为」就是这么来的）。页签已不在（极端竞态）则立即收尾。
   */
  attachHiddenPopupToTab(webContentsId: number, tabLabel: string): void {
    const rec = this.hiddenPopups.get(webContentsId)
    if (!rec) return
    if (!this.tabs.has(tabLabel)) {
      this.releaseHiddenPopup(webContentsId)
      return
    }
    rec.tabLabel = tabLabel
  }

  /** 收尾并销毁某个隐藏弹窗（幂等）。下载结束、提交文档、超时、宿主退出时调用。 */
  releaseHiddenPopup(webContentsId: number): void {
    const rec = this.hiddenPopups.get(webContentsId)
    if (!rec) return
    this.hiddenPopups.delete(webContentsId)
    clearTimeout(rec.timer)
    try {
      rec.cleanup()
    } catch (e) {
      console.error(`[web2app] 销毁隐藏弹窗失败：${(e as Error).message}`)
    }
  }

  /** 被判定为「弹窗兜底重复下载」而取消的次数（页签探针断言用）。 */
  get droppedPopupFallbackCount(): number {
    return this.droppedPopupFallbacks
  }

  /**
   * 判重：这次下载是不是站点「弹窗被拦截」的兜底重复下载。
   *
   * 判据三条同时成立才丢：① 地址与刚才交给新页签的 `window.open` 目标**逐字相同**；
   * ② 发起下载的是**那个打开者页签**；③ 在 `POPUP_FALLBACK_WINDOW_MS` 内。命中即消费
   * 记录并返回 true（调用方 `item.cancel()`，不弹「另存为」）。
   *
   * ⚠️ 不能只按「打开者 + 时间窗」丢：页面在弹框遮挡前仍可能程序化批量下载多个文件，
   *   那些地址不同，必须放行。
   */
  shouldDropPopupFallback(webContentsId: number, url: string): boolean {
    const rec = this.expectedPopupDownloads.get(url)
    if (!rec) return false
    if (Date.now() > rec.expiresAt) {
      this.expectedPopupDownloads.delete(url)
      return false
    }
    if (rec.openerWebContentsId !== webContentsId) return false
    this.expectedPopupDownloads.delete(url)
    this.droppedPopupFallbacks += 1
    return true
  }

  /**
   * 下载落盘 / 取消后调用：若发起下载的页签**从未提交过文档**（只为这次下载而开），
   * 自动关掉它并把激活态拉回打开它的页签（浏览器语义）。
   *
   * @param webContentsId `will-download` 里发起下载的 webContents id
   * @returns 是否真的关掉了某个页签
   *
   * ⚠️ 已加载过页面的页签不关（`shouldCloseTabAfterDownload` 的唯一判据）；首页页签
   *   由 `closeTab` 的 `isPinnedHome` 拒绝，这里不重复判定。
   */
  closeTabAfterDownload(webContentsId: number): boolean {
    const target = this.findByWebContentsId(webContentsId)
    if (!target || !shouldCloseTabAfterDownload(target.committed)) return false
    const opener = target.opener
    if (!this.closeTab(target.label)) return false
    // closeTab 选的是相邻页签；浏览器语义是回到**点击时的页面**（打开者），补一次激活。
    if (opener && this.tabs.has(opener)) this.activate(opener)
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
    // 隐藏弹窗窗口是独立 BrowserWindow，同样不随宿主视图回收 ⇒ 一并收尾
    for (const id of [...this.hiddenPopups.keys()]) this.releaseHiddenPopup(id)
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
    const labels = [...this.tabs.keys()]
    return [...this.tabs.values()].map((t) => ({
      label: t.label,
      title: t.title || t.label,
      active: t.label === this.activeLabel,
      // 首页 = 位次 0（见 tab-order.ts::isPinnedHome 的文档：固定第一位 + 不可关）。
      // 等价简写是 `i === 0`，但这里刻意走 isPinnedHome —— 「首页判定」全项目只有这一份实现。
      home: isPinnedHome(labels, t.label),
    }))
  }

  /**
   * get_tabs IPC 的返回契约：**元组数组** `[label, title, active, home]`。
   *
   * ⚠️ 与 `list()`（对象形态）刻意分开：Tauri 侧 `get_tabs` 返回
   *   `Vec<(String, String, bool, bool)>`，序列化成 JSON 后是
   *   `[[label,title,active,home], ...]`；
   *   `shell.html` 用 `list.map(([label, title, active, home]) => …)` 解构（见 shell.html:345）。
   *   若这里返回对象数组，解构会得到 `[undefined, undefined, undefined, undefined]`，标签栏渲染空白。
   *   而 `pushToShell` 走的是 `render({label,title,active,home})` 对象契约，两者不能混用
   *   —— 这是双壳单一真源下「同一份 shell.html 两套消费形态」的硬约束。
   */
  listForIpc(): [string, string, boolean, boolean][] {
    const labels = [...this.tabs.keys()]
    return [...this.tabs.values()].map((t) => [
      t.label,
      t.title || t.label,
      t.label === this.activeLabel,
      isPinnedHome(labels, t.label),
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

  /**
   * 更新页签标题（page-title-updated 回传后调用）。
   *
   * ⚠️ `trim()` + 「空标题不覆盖旧标题」与 Tauri 注入脚本 `tab_title.js::report()` 逐字对齐：
   * 页面把 `<title>` 清空时，Tauri 保留旧标题，Electron 此前会把标签栏变成空白 —— 双壳同
   * 一套页面却两种表现（2026-11-07 架构审查 M-2）。
   */
  setTabTitle(label: string, title: string): void {
    const t = this.tabs.get(label)
    if (!t) return
    const next = title.trim()
    if (!next) return
    t.title = next
    this.pushToShell()
  }
}
