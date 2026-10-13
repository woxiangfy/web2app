/**
 * 多页签模式断言组（探针四拆分之四）：`runTabsProbe`，
 * 对应 docs/electron-tabs-design.md §8 的 P1/P2/P4/P6/P7/P7b/P8/P9/P10/P10b/P11/P12
 *（P7b = 3 页签邻接选择，2 页签验不出；P10b = 新建页签加载完成前即激活；
 *  P1h/P7h = 首页页签「固定第一位 + 带主页图标 + 不可关」的成对判据）。
 *
 * 与 `runProbe` 分开的原因：单窗口断言组（节 0–17）假设「主窗口自身是远程页面」，
 * 而页签模式下主窗口自身是壳页面（shell.html），没有 titlebar.js 注入的按钮/留白，
 * 那些断言不适用；这里只验页签本体语义。
 *
 * ⚠️ 判据成对（项目铁律）：对「侵入类」缺陷，位置 + 存在性都要断言。
 *   P4 的「仅激活页签可见」若只写「激活可见」会被「全都可见靠 z-order 盖」蒙混；
 *   若只写「只有一个可见」会被「干脆全删」满足 —— 所以两者都写。
 */
import { join } from 'node:path'
import { WebContentsView, type BrowserWindow } from 'electron'

import { MAX_TABS, TAB_BAR_HEIGHT, type TabManager } from './tabs'
import { Probe, sleep, waitUntil } from './probe-infra'

export interface TabsProbeContext {
  win: BrowserWindow
  base: string
  /** index.ts 已挂到主窗口上的页签管理器（复用真实 onSpawn 链路，不另建实例）。 */
  tabs: TabManager
  /** 设置窗查询 / 关闭（标签栏「设置」按钮的断言要用真实窗口，不是桩）。 */
  getSettingsWindow: () => BrowserWindow | null
  closeSettingsWindow: () => void
}

/**
 * 跑多页签断言组。与 `runProbe` 分开：单窗口断言组（节 0–17）假设「主窗口自身是远程页面」，
 * 而页签模式下主窗口自身是壳页面（shell.html），没有 titlebar.js 注入的按钮/留白，
 * 那些断言不适用。这里只验页签本体语义。
 *
 * ⚠️ 判据成对（项目铁律）：对「侵入类」缺陷，位置 + 存在性都要断言。
 *   P4 的「仅激活页签可见」若只写「激活可见」会被「全都可见靠 z-order 盖」蒙混；
 *   若只写「只有一个可见」会被「干脆全删」满足 —— 所以两者都写。
 */
export async function runTabsProbe(ctx: TabsProbeContext): Promise<boolean> {
  const { win, base, tabs } = ctx
  const p = new Probe()

  const children = (): WebContentsView[] => win.contentView.children as WebContentsView[]
  const contentBounds = () => {
    const cb = win.getContentBounds()
    return { x: 0, y: TAB_BAR_HEIGHT, width: cb.width, height: cb.height - TAB_BAR_HEIGHT }
  }
  // 读壳页面标签栏里每个 .tab .label 的文本（标题回传的最终呈现处）
  const shellLabels = async (): Promise<string[]> =>
    (await win.webContents.executeJavaScript(
      `Array.from(document.querySelectorAll('.tab .label')).map((e) => e.textContent)`,
      true
    )) as string[]

  /**
   * 读壳页面标签栏每个 .tab 的**首页标记三件套**：home 类名 / 房子图标 / 关闭按钮。
   *
   * ⚠️ 判据成对（项目铁律）：只断言「首页有图标」会被「给所有页签都加图标」满足；只断言
   * 「首页没有 ×」会被「谁都没有 ×」满足。所以图标与 × 必须在**同一批样本**上一起判。
   */
  const shellTabs = async (): Promise<
    { label: string; home: boolean; hasIcon: boolean; hasClose: boolean }[]
  > =>
    (await win.webContents.executeJavaScript(
      `Array.from(document.querySelectorAll('#tabs .tab')).map((el) => ({` +
        ` label: el.dataset.label,` +
        ` home: el.classList.contains('home'),` +
        ` hasIcon: !!el.querySelector('.home-ico'),` +
        ` hasClose: !!el.querySelector('.close') }))`,
      true
    )) as { label: string; home: boolean; hasIcon: boolean; hasClose: boolean }[]

  // ---------- 初始态 ----------
  p.check('页签初始为空（无视图）', tabs.count === 0 && children().length === 0,
    `count=${tabs.count} children=${children().length}`)

  // ---------- P2 + P1：建第一个页签（首屏，走 resolveInitialTarget 真实链路） ----------
  const label1 = await tabs.spawnTab()
  p.check('首个页签创建成功并激活', label1 !== null && tabs.active?.label === label1,
    `label1=${label1}`)
  const b = contentBounds()
  const v1 = tabs.active?.view
  const g1 = v1?.getBounds()
  p.check('P1 内容视图 bounds = {x:0, y:38, w:W, h:H-38}（getContentBounds 推算）',
    !!g1 && g1.x === b.x && g1.y === b.y && g1.width === b.width && g1.height === b.height,
    `实际 ${JSON.stringify(g1)}，期望 ${JSON.stringify(b)}`)
  p.check('P2 视图存在性（children 数 = 页签数）', children().length === tabs.count,
    `children=${children().length} count=${tabs.count}`)

  // ---------- P1h：首页页签（首签 = app_url 那一签，固定第一位、不可关） ----------
  // 三件套成对断言：主进程侧 home 标记 + 壳标签栏的「有房子图标」+「没有 ×」。
  const home = label1!
  const rowsOne = tabs.listForIpc()
  p.check('P1h get_tabs 首条 home=true（首页即列表第 0 位）',
    rowsOne.length === 1 && rowsOne[0][0] === home && rowsOne[0][3] === true,
    JSON.stringify(rowsOne))
  const homeDomOk = await waitUntil(async () => {
    const list = await shellTabs()
    return list.length === 1 && list[0].label === home && list[0].home &&
      list[0].hasIcon && !list[0].hasClose
  }, 4000)
  p.check('P1h 首页在标签栏带主页图标、且没有关闭按钮（固定第一位）',
    homeDomOk, JSON.stringify(await shellTabs()))

  // ---------- P4：建第二个页签，验证切换后「仅激活页签可见」 ----------
  const label2 = await tabs.spawnTab(`${base}/tabs-b.html`)
  p.check('第二个页签创建成功并激活', label2 !== null && tabs.active?.label === label2,
    `label2=${label2}`)
  // 成对判据的另一半：非首页必须有 ×、且没有房子图标（否则「所有页签都带图标」也能假绿）。
  const pairOk = await waitUntil(async () => {
    const list = await shellTabs()
    return list.length === 2 &&
      list[0].label === home && list[0].home && list[0].hasIcon && !list[0].hasClose &&
      list[1].label === label2 && !list[1].home && !list[1].hasIcon && list[1].hasClose
  }, 4000)
  p.check('P1h 非首页页签带关闭按钮、无主页图标（与首页成对）',
    pairOk, JSON.stringify(await shellTabs()))
  let views = children()
  let visible = views.filter((v) => v.getVisible())
  p.check('P4 新建后仅新页签可见（其余全隐藏）',
    visible.length === 1 && visible[0] === tabs.active?.view,
    `可见视图数=${visible.length}`)

  // 切回第一个，验证可见性翻转（位置 + 存在性成对）
  tabs.activate(label1!)
  await sleep(150)
  views = children()
  visible = views.filter((v) => v.getVisible())
  p.check('P4 切回后仅目标页签可见（不靠 z-order 蒙混）',
    visible.length === 1 && visible[0] === tabs.active?.view && tabs.active?.label === label1,
    `可见视图数=${visible.length} active=${tabs.active?.label}`)

  // ---------- P6：标题回传（page-title-updated → setTabTitle → 壳标签栏文本） ----------
  // label2 加载的是 /tabs-b.html（<title>页签B</title>）。等待事件链路把标题推进壳 DOM。
  const titleOk = await waitUntil(async () => {
    const labels = await shellLabels()
    return labels.includes('页签B')
  }, 5000)
  p.check('P6 页面标题回传到标签栏文本（页签B）', titleOk,
    `壳标签栏=${JSON.stringify(await shellLabels())}`)

  // ---------- 契约：get_tabs 返回元组数组（对齐 Tauri Vec<(String,String,bool,bool)>） ----------
  // shell.html 初始化用 list.map(([label,title,active,home]) => …) 解构（shell.html:345），
  // 若这里退回对象数组（或漏掉第 4 位 home），解构得到 undefined —— 标签栏渲染空白 / 首页失去标记。
  const tuples = tabs.listForIpc()
  p.check('get_tabs 契约：返回 [label,title,active,home] 四元组数组（非对象）',
    tuples.length === tabs.count &&
      tuples.every((t) =>
        Array.isArray(t) && t.length === 4 &&
        typeof t[0] === 'string' && typeof t[1] === 'string' &&
        typeof t[2] === 'boolean' && typeof t[3] === 'boolean'
      ),
    JSON.stringify(tuples))

  // ---------- P7h：首页不可关（关闭请求被拒 + 现场一点没动） ----------
  // 当前列表 [home(=label1), label2]。激活放在 label2，这样「激活不受影响」也被钉住。
  tabs.activate(label2!)
  const beforeHomeClose = tabs.count
  const homeCloseResult = tabs.closeTab(home)
  p.check('P7h 关闭首页被拒（closeTab 返回 false）', homeCloseResult === false,
    `返回=${homeCloseResult}`)
  p.check('P7h 首页被拒后现场不变（集合数 / 首位 / 激活 / children 全不动）',
    tabs.count === beforeHomeClose &&
      tabs.listForIpc()[0][0] === home && tabs.listForIpc()[0][3] === true &&
      tabs.active?.label === label2 && children().length === tabs.count,
    `count=${tabs.count} active=${tabs.active?.label} children=${children().length}`)
  // 首页在界面上根本没有 ×（用户无从触发关闭）—— 这是「前端没有入口 + 后端拒绝」的成对判据。
  // ⚠️ 用 some(首页存在 且 无 ×) 而不是 every(...)：every 在「DOM 还没渲染出来」时空真，
  //    会把「标签栏根本没画」也判成 PASS。
  const domAfterHomeRefuse = await shellTabs()
  p.check('P7h 首页在标签栏没有关闭按钮（用户点不到）',
    domAfterHomeRefuse.some((t) => t.label === home && t.home && !t.hasClose),
    JSON.stringify(domAfterHomeRefuse))

  // ---------- P7：关闭激活页签 → 激活态落到相邻（优先右侧），不悬空不全隐 ----------
  // 当前激活是 label2；列表 [home, label2] 里 label2 是末位 ⇒ 没有右邻，激活必须落到左邻
  // home。首页不可关这一点在这里反过来变成**兜底**：关掉最后一个普通页签也永远有页签可显示。
  tabs.activate(label2!)
  const before = tabs.count
  const closed = tabs.closeTab(label2!)
  p.check('P7 关闭激活页签成功', closed && tabs.count === before - 1,
    `count ${before} → ${tabs.count}`)
  p.check('P7 关闭后激活态落到相邻页签（不悬空）',
    tabs.active?.label === home && tabs.count === 1,
    `active=${tabs.active?.label} count=${tabs.count}`)
  views = children()
  visible = views.filter((v) => v.getVisible())
  p.check('P7 关闭后剩余页签仍可见（不全隐）',
    visible.length === 1 && visible[0] === tabs.active?.view,
    `可见视图数=${visible.length}`)
  p.check('P7 关闭后 children 数同步减少（位置 + 存在性）',
    children().length === tabs.count, `children=${children().length} count=${tabs.count}`)

  // ---------- P7b：多页签邻接选择（2 页签验不出的回归）+ 首位不可关 ----------
  // 旧实现在【删除之后】的列表里 findIndex(被删 label)，恒为 -1 → Math.min(0,len-1)=0
  // ⇒ 无论关中间还是关末尾都跳回第一个幸存页签；而上面的 P7 只有 2 个页签，左右邻恰好
  // 是同一个，探针因此假绿。规则本体由 test/tabs.test.ts（nextActiveLabelAfterClose）毫秒级
  // 钉住，这里补真实 TabManager 的端到端链路。当前 [home]，再开 3 个：[home,x1,x2,x3]。
  const x1 = await tabs.spawnTab('about:blank')
  const x2 = await tabs.spawnTab('about:blank')
  const x3 = await tabs.spawnTab('about:blank')
  p.check('P7b 三个附加页签创建成功（顺序 [home,x1,x2,x3]）',
    x1 !== null && x2 !== null && x3 !== null && tabs.count === 4,
    `x1=${x1} x2=${x2} x3=${x3} count=${tabs.count}`)
  // 关中间：激活 x2 后关闭 → 必须落到右邻 x3（旧实现错误落回首位）
  tabs.activate(x2!)
  tabs.closeTab(x2!)
  p.check('P7b 关中间页签 → 激活右邻', tabs.active?.label === x3,
    `active=${tabs.active?.label} 期望=${x3}`)
  // 关末尾：[home,x1,x3] 里 x3 是末位 → 必须落到左邻 x1
  tabs.closeTab(x3!)
  p.check('P7b 关末尾页签 → 激活左邻', tabs.active?.label === x1,
    `active=${tabs.active?.label} 期望=${x1}`)
  // 首位不可关：[home,x1] 里关闭首位（首页）必须被拒、且现场完全不动 —— 其余页签都关得掉、
  // 首页关不掉，这就是「首页固定第一位」的全部保证（没有第二条重排路径）。
  tabs.activate(x1!)
  const beforeHomeRefuse = tabs.count
  const refused = tabs.closeTab(home)
  p.check('P7b 关闭首位（首页）被拒 → 顺序与激活都不动',
    refused === false && tabs.count === beforeHomeRefuse &&
      tabs.listForIpc()[0][0] === home && tabs.active?.label === x1,
    `返回=${refused} count=${tabs.count} 首位=${tabs.listForIpc()[0][0]} active=${tabs.active?.label}`)
  // 关掉最后一个非首页：[home,x1] 关 x1 → 激活必须落回首页，且**不能**走「关最后一个 ⇒ 关宿主」。
  tabs.closeTab(x1!)
  const winAliveAfterLastClose = !win.isDestroyed()
  p.check('P7b 非首页全部关掉 → 激活落回首页（不悬空、不误关宿主）',
    tabs.count === 1 && tabs.active?.label === home && winAliveAfterLastClose,
    `count=${tabs.count} active=${tabs.active?.label} 窗口存活=${winAliveAfterLastClose}`)
  views = children()
  visible = views.filter((v) => v.getVisible())
  p.check('P7b 邻接切换后仅新激活页签可见（不靠 z-order 蒙混）',
    visible.length === 1 && visible[0] === tabs.active?.view,
    `可见视图数=${visible.length}`)

  // ---------- P7c：关闭【后台页签】壳标签栏必须即时刷新（幽灵页签回归） ----------
  // shell.html 点 × 后不做本地乐观更新，完全依赖宿主 pushToShell() 推送；旧实现在
  // closeTab 的非活动分支漏推 → 被关页签残留在壳 DOM 里直到下一次切换/标题推送。
  const shellTabCount = async (): Promise<number> =>
    (await win.webContents.executeJavaScript(
      `document.querySelectorAll('#tabs .tab').length`,
      true
    )) as number
  // ⚠️ 首页不可关 ⇒ 锚点必须是**另一个**页签（首页永远占着首位，不能拿它当被关对象）。
  // 当前 [home]，补锚点 y 与两个将被关掉的后台页签。
  const y = await tabs.spawnTab('about:blank')
  const y1 = await tabs.spawnTab('about:blank')
  const y2 = await tabs.spawnTab('about:blank')
  tabs.activate(y!) // 激活固定在 y：下面关的两个都是【后台页签】
  const shellSynced = await waitUntil(
    async () => (await shellTabCount()) === tabs.count,
    4000
  )
  p.check('P7c 前提：壳标签栏条目数与页签集合一致',
    shellSynced, `壳=${await shellTabCount()} 集合=${tabs.count}`)
  tabs.closeTab(y1!)
  const shellAfterClose = await waitUntil(
    async () => (await shellTabCount()) === tabs.count,
    4000
  )
  p.check('P7c 关闭后台页签后壳标签栏即时刷新（无幽灵页签）',
    shellAfterClose, `壳=${await shellTabCount()} 集合=${tabs.count}`)
  tabs.closeTab(y2!) // 还原到 [home, y] 双页签稳态
  await sleep(100)

  // 此后 [home, y]（激活 y）。y 建在 about:blank 上，而 P9 的 window.open('/opened.html')
  // 依赖真实 http 源来解析相对 URL（about:blank 没有 base，Chromium 无法解析相对地址），
  // 故先把 y 导航到夹具页，再作为 P9 的 opener（不再硬编码 label2）。
  await tabs.active!.view.webContents.loadURL(`${base}/tabs-b.html`)
  await sleep(200)
  const opener = y!

  // ---------- P9 / P10：window.open 只新增 1 页签 + target path 保留 ----------
  // 从当前激活页签发起 window.open('/opened.html')，走 index.ts 的 setWindowOpenHandler
  // → spawnTab(opened)，验证「一次点击只开 1 个」且「目标 path 未被降级成入口地址」。
  const countBeforeOpen = tabs.count
  await tabs.active!.view.webContents.executeJavaScript(
    `window.open('/opened.html', '_blank'); true`, true)
  // setWindowOpenHandler 用 setImmediate 异步 spawnTab。spawnTab 在【发起 loadURL 之前】
  // 就已 activate（标签栏即时切换），但新页签的 URL 此刻仍是空串，要等导航提交后才落到
  // /opened.html —— 所以判据仍是「active 换成新 label 且其 URL 已落到 /opened.html」，
  // 不能只看 count（即时激活回归见下面的 P10b）。
  const openedReady = await waitUntil(async () => {
    if (tabs.count !== countBeforeOpen + 1) return false
    const a = tabs.active
    if (!a || a.label === opener) return false
    try {
      return new URL(a.view.webContents.getURL()).pathname === '/opened.html'
    } catch {
      return false
    }
  }, 4000)
  const newUrl = tabs.active?.view.webContents.getURL() ?? ''
  const newPath = (() => { try { return new URL(newUrl).pathname } catch { return newUrl } })()
  p.check('P9 一次 window.open 只新增 1 个页签',
    openedReady && tabs.count === countBeforeOpen + 1,
    `count ${countBeforeOpen} → ${tabs.count}`)
  p.check('P10 target path 保留（未被降级成入口地址 /plain.html）',
    newPath === '/opened.html',
    `新页签 URL=${newUrl}（path=${newPath}）`)
  // 收掉 window.open 新开的页签，恢复单一页签稳态（opener），再进后续断言
  if (tabs.active && tabs.active.label !== opener) tabs.closeTab(tabs.active.label)
  tabs.activate(opener)

  // ---------- P10b：新建页签在页面【加载完成前】即激活（标签栏即时切换回归） ----------
  // 旧实现把 activate 放在 `await onSpawn`（≈ await loadURL 整页加载完成）之后：新页签的
  // 标签栏条目/激活态和内容切换全被拖到加载结束；加载中新视图默认可见且盖在子视图栈顶，
  // 用户此时切走，加载结束还会被迟到的 activate 拽回新页签。夹具 /slow.html?ms= 先发响应
  // 头（导航立即提交）、body 延迟下发，制造确定性的「加载中」窗口。
  const SLOW_MS = 2500
  /** 读壳标签栏当前 .active 页签的 label（pushToShell 渲染后的最终呈现）。 */
  const shellActiveLabel = async (): Promise<string | null> =>
    (await win.webContents.executeJavaScript(
      `(() => { const el = document.querySelector('#tabs .tab.active');` +
        ` return el ? el.dataset.label : null; })()`,
      true
    )) as string | null
  const countBeforeSlow = tabs.count
  const slowT0 = Date.now()
  // ⚠️ 刻意不 await：spawnTab 同步跑到 onSpawn 内的 loadURL 才让出事件循环，
  // 而激活应发生在那之前 —— 调用返回的瞬间激活就应当已完成。
  const slowP = tabs.spawnTab(`${base}/slow.html?ms=${SLOW_MS}`)
  // 期望值必须独立于 active 获取：新页签在 spawnTab 同步段就已登记进集合，
  // 而旧实现的病根恰恰是「此刻 active 还是旧页签」——直接拿 active 当期望值，
  // 旧实现会让下面的壳态断言与自己比较而假绿（实测踩过）。
  // ⚠️ 必须同时排除 **opener 与首页**：集合是 [home, opener, slow]，旧写法 `find(l => l !== opener)`
  //    取到的是**首页**（它永远在首位）—— 期望值当场错成首页，后面的断言全在自欺。
  const slowLabel = tabs.listForIpc()
    .map(([l]) => l)
    .filter((l) => l !== opener && l !== home)
    .pop()
  p.check('P10b spawnTab 返回时（页面远未加载完）新页签已在主进程侧激活',
    !!slowLabel && tabs.active?.label === slowLabel && tabs.count === countBeforeSlow + 1,
    `active=${tabs.active?.label} 新页签=${slowLabel} opener=${opener} count=${tabs.count}`)
  // 壳标签栏的 active 同步必须发生在慢页加载完成（SLOW_MS）之前；旧实现这里会轮询超时。
  const shellSwitched = await waitUntil(async () => (await shellActiveLabel()) === slowLabel, 1000)
  const switchElapsed = Date.now() - slowT0
  p.check('P10b 标签栏在页面加载完成前即切到新页签',
    shellSwitched && switchElapsed < SLOW_MS,
    `壳active=${await shellActiveLabel()} 用时=${switchElapsed}ms 加载窗口=${SLOW_MS}ms`)
  let visNow = children().filter((v) => v.getVisible())
  p.check('P10b 加载中仅新页签内容视图可见（旧页签已同步隐藏，不靠 z-order 盖）',
    visNow.length === 1 && visNow[0] === tabs.active?.view && tabs.active?.label === slowLabel,
    `可见视图数=${visNow.length}`)
  // 加载中手动切回旧页签：必须立即生效（旧实现这一步后视觉上是旧页签，但加载一结束
  // 又被 spawnTab 尾部迟到的 activate 拽回新页签 —— 用户感知的「切不过去」）。
  tabs.activate(opener)
  await sleep(100)
  visNow = children().filter((v) => v.getVisible())
  p.check('P10b 加载中切回旧页签立即生效（集合态 / 壳态 / 可见视图三者一致）',
    tabs.active?.label === opener && (await shellActiveLabel()) === opener &&
      visNow.length === 1 && visNow[0] === tabs.active?.view,
    `active=${tabs.active?.label} 壳active=${await shellActiveLabel()} 可见=${visNow.length}`)
  await slowP // 等慢页加载完成
  await sleep(150)
  p.check('P10b 慢页加载完成后激活态不被迟到的激活拽回',
    tabs.active?.label === opener && (await shellActiveLabel()) === opener,
    `active=${tabs.active?.label} 壳active=${await shellActiveLabel()}`)
  // 收尾：关掉后台慢页签（非激活分支），恢复 opener 单页签稳态再进后续断言
  p.check('P10b 收尾：后台慢页签关闭成功', slowLabel ? tabs.closeTab(slowLabel) : false,
    `label=${slowLabel}`)
  await sleep(100)

  // ---------- 标签栏「设置」按钮（最小化左侧）→ 打开设置窗 ----------
  // 位置 + 链路成对断言。链路上「三处逐字一致」最容易断：shell.html 的
  // invoke("open_settings") ↔ shell-preload 的 ALLOWED 白名单 ↔ 主进程
  // ipcMain.handle('tabs:open_settings')；任一漏改，表象都是「按钮点了没反应」。
  const gearJson = (await win.webContents.executeJavaScript(
    `(() => {
       const mn = document.getElementById('min'), st = document.getElementById('settings');
       if (!mn || !st) return JSON.stringify({ found: false });
       const a = mn.getBoundingClientRect(), b = st.getBoundingClientRect();
       // ⚠️ 顺序判据必须用「几何次序」而不是 DOM 次序：先比横坐标谁在左。
       return JSON.stringify({ found: true, gearRight: +b.right.toFixed(1), minLeft: +a.left.toFixed(1) });
     })()`,
    true
  )) as string
  const gear = JSON.parse(gearJson) as { found: boolean; gearRight?: number; minLeft?: number }
  p.check(
    '标签栏有「设置」按钮，且在最小化左侧',
    gear.found && (gear.gearRight ?? 0) <= (gear.minLeft ?? 0) + 1,
    gearJson
  )
  await win.webContents.executeJavaScript(`document.getElementById('settings').click(); true`, true)
  // 判据同单窗口组：等到设置页**真的渲染出面板**，而不是「窗口对象存在」——后者在
  // URL 加载失败时也成立（假绿），且提前 close 会中止加载并留下 ERR_FAILED(-2) 噪声。
  const gearOpened = await waitUntil(async () => {
    const w = ctx.getSettingsWindow()
    if (!w) return false
    try {
      return (await w.webContents.executeJavaScript(
        '!!document.querySelector(".panel")',
        true
      )) as boolean
    } catch {
      return false
    }
  }, 6000)
  p.check(
    '点标签栏设置按钮 → 打开设置窗且页面已渲染（shell.html → 白名单垫片 → IPC → openSettingsWindow）',
    gearOpened,
    '6s 内未渲染（查白名单 / IPC 注册名 / 页面字面量三处是否一致）'
  )
  ctx.closeSettingsWindow()
  const gearClosed = await waitUntil(async () => !ctx.getSettingsWindow(), 4000)
  p.check('设置按钮链路收尾：设置窗可关闭', gearClosed, '4s 内未关闭')

  // ---------- P11：特权命令 sender 校验（内容视图发 close_host 必须被拒） ----------
  // 生产内容视图（spawnTab）刻意不挂 preload，远程页面连 ipcRenderer 都碰不到；这里用
  // 探针专用 preload 把 IPC 通道硬塞给一个独立内容视图，模拟「未来误给内容视图加 preload」
  // 的最坏情形，验证 assertShell 的 sender.id 校验仍会拒绝它（且窗口不被误关）。
  const probePreload = join(__dirname, '..', 'renderer', 'tabs-probe-preload.js')
  const rogueView = new WebContentsView({
    webPreferences: {
      preload: probePreload,
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      backgroundThrottling: false,
    },
  })
  win.contentView.addChildView(rogueView)
  rogueView.setBounds(contentBounds())
  await rogueView.webContents.loadURL(`${base}/plain.html`).catch(() => {})
  await sleep(400) // 等 preload 注入 + 页面就绪
  const shellId = win.webContents.id
  const rogueId = rogueView.webContents.id
  p.check('P11 前提：内容视图 sender.id ≠ 壳 webContents.id（可区分）',
    rogueId !== shellId, `壳=${shellId} 内容=${rogueId}`)
  const rogueResult = (await rogueView.webContents.executeJavaScript(
    `window.__web2appProbeInvoke.invoke('tabs:close_host')` +
      `.then(() => 'OK').catch((e) => 'REJECTED:' + String(e && e.message))`,
    true
  )) as string
  const winStillAlive = !win.isDestroyed()
  p.check('P11 内容视图发特权命令 close_host 被拒（且窗口未误关）',
    rogueResult.startsWith('REJECTED') && winStillAlive,
    `结果=${rogueResult}；窗口存活=${winStillAlive}`)
  // 回收探针内容视图
  try { win.contentView.removeChildView(rogueView); rogueView.webContents.close() } catch { /* noop */ }

  // ---------- P12：层级语义（重复 addChildView 不增条目、抬到末尾） ----------
  const activeView = tabs.active?.view
  if (activeView) {
    const nBefore = children().length
    win.contentView.addChildView(activeView) // 重复 add 已存在视图
    const after = children()
    p.check('P12 重复 addChildView 不增条目', after.length === nBefore,
      `${nBefore} → ${after.length}`)
    p.check('P12 重复 add 后视图被抬到数组末尾（z-order 顶层）',
      after[after.length - 1] === activeView, '视图不在末尾')
  }

  // ---------- P8：熔断（连开至上限，第 N+1 次拒绝且 children 不增长） ----------
  // 用 about:blank 快速填充（不解析 target、不渲染），避免 32 个真实页拖慢探针。
  // 此时幸存页签 = [home, opener]；上限名额含首页（见 tabs.rs MAX_TABS 的注释）。
  const labels: string[] = [opener]
  while (tabs.count < MAX_TABS) {
    const l = await tabs.spawnTab('about:blank')
    if (l === null) break
    labels.push(l)
  }
  p.check('P8 连开至上限（32）', tabs.count === MAX_TABS, `count=${tabs.count}`)
  const countAtLimit = tabs.count
  const over = await tabs.spawnTab('about:blank')
  p.check('P8 第 N+1 次新建被拒绝（返回 null）', over === null, `over=${over}`)
  p.check('P8 超限后 children 不再增长', children().length === countAtLimit,
    `children=${children().length} count=${countAtLimit}`)

  // ---------- 清理：关掉所有【非首页】页签，首页是唯一幸存者 ----------
  // ⚠️ 首页关不掉 ⇒ 「关到最后一个 ⇒ 关宿主」在正常路径下不可达，本组不需要再刻意避开它。
  const rest = labels.slice(1) // 去掉第一个（opener），剩下的都是 P8 填充的
  for (const l of rest) tabs.closeTab(l)
  tabs.closeTab(opener)
  p.check('清理：非首页页签全部关闭后只剩首页（仍固定在首位、关闭仍被拒）',
    tabs.count === 1 && tabs.active?.label === home &&
      tabs.listForIpc()[0][0] === home && tabs.listForIpc()[0][3] === true &&
      tabs.closeTab(home) === false && !win.isDestroyed(),
    `count=${tabs.count} active=${tabs.active?.label} 窗口存活=${!win.isDestroyed()}`)
  p.check('清理：关闭到只剩一个页签（首页）', tabs.count === 1, `count=${tabs.count}`)

  // ---------- 汇总 ----------
  const failed = p.results.filter((r) => !r.ok)
  console.log('\n[probe] ===== 多页签断言组 =====')
  for (const r of p.results) {
    console.log(`[probe] ${r.ok ? 'PASS' : 'FAIL'}  ${r.name}${r.ok ? '' : '  ← ' + r.detail}`)
  }
  console.log(`[probe] ===== ${p.results.length - failed.length}/${p.results.length} 通过 =====\n`)
  return failed.length === 0
}
