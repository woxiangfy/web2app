/**
 * 多页签模式断言组（探针四拆分之四）：`runTabsProbe`，
 * 对应 docs/electron-tabs-design.md §8 的 P1/P2/P4/P6/P7/P7b/P8/P9/P10/P11/P12
 *（P7b = 3 页签邻接选择，2 页签验不出）。
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

  // ---------- P4：建第二个页签，验证切换后「仅激活页签可见」 ----------
  const label2 = await tabs.spawnTab(`${base}/tabs-b.html`)
  p.check('第二个页签创建成功并激活', label2 !== null && tabs.active?.label === label2,
    `label2=${label2}`)
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

  // ---------- 契约：get_tabs 返回元组数组（对齐 Tauri Vec<(String,String,bool)>） ----------
  // shell.html 初始化用 list.map(([label,title,active]) => …) 解构（shell.html:266），
  // 若这里退回对象数组，解构得到 [undefined,undefined,undefined]，标签栏渲染空白。
  const tuples = tabs.listForIpc()
  p.check('get_tabs 契约：返回 [label,title,active] 元组数组（非对象）',
    tuples.length === tabs.count &&
      tuples.every((t) =>
        Array.isArray(t) && t.length === 3 &&
        typeof t[0] === 'string' && typeof t[1] === 'string' && typeof t[2] === 'boolean'
      ),
    JSON.stringify(tuples))

  // ---------- P7：关闭激活页签 → 激活态落到相邻（优先右侧），不悬空不全隐 ----------
  // 当前激活是 label1；先激活 label1，再关闭它，激活应落到 label2（右侧相邻）。
  tabs.activate(label1!)
  const before = tabs.count
  const closed = tabs.closeTab(label1!)
  p.check('P7 关闭激活页签成功', closed && tabs.count === before - 1,
    `count ${before} → ${tabs.count}`)
  p.check('P7 关闭后激活态落到相邻页签（不悬空）',
    tabs.active?.label === label2 && tabs.count === 1,
    `active=${tabs.active?.label} count=${tabs.count}`)
  views = children()
  visible = views.filter((v) => v.getVisible())
  p.check('P7 关闭后剩余页签仍可见（不全隐）',
    visible.length === 1 && visible[0] === tabs.active?.view,
    `可见视图数=${visible.length}`)
  p.check('P7 关闭后 children 数同步减少（位置 + 存在性）',
    children().length === tabs.count, `children=${children().length} count=${tabs.count}`)

  // ---------- P7b：三页签邻接选择（2 页签验不出的回归） ----------
  // 旧实现在【删除之后】的列表里 findIndex(被删 label)，恒为 -1 → Math.min(0,len-1)=0
  // ⇒ 无论关中间还是关末尾都跳回第一个幸存页签；而上面的 P7 只有 2 个页签，左右邻恰好
  // 是同一个，探针因此假绿。规则本体由 test/tabs.test.ts（nextActiveLabelAfterClose）毫秒级
  // 钉住，这里补真实 TabManager 的端到端链路。当前 [label2]，再开 3 个：[label2,x1,x2,x3]。
  const x1 = await tabs.spawnTab('about:blank')
  const x2 = await tabs.spawnTab('about:blank')
  const x3 = await tabs.spawnTab('about:blank')
  p.check('P7b 三个附加页签创建成功（顺序 [label2,x1,x2,x3]）',
    x1 !== null && x2 !== null && x3 !== null && tabs.count === 4,
    `x1=${x1} x2=${x2} x3=${x3} count=${tabs.count}`)
  // 关中间：激活 x2 后关闭 → 必须落到右邻 x3（旧实现错误落 label2）
  tabs.activate(x2!)
  tabs.closeTab(x2!)
  p.check('P7b 关中间页签 → 激活右邻', tabs.active?.label === x3,
    `active=${tabs.active?.label} 期望=${x3}`)
  // 关末尾：[label2,x1,x3] 里 x3 是末位 → 必须落到左邻 x1
  tabs.closeTab(x3!)
  p.check('P7b 关末尾页签 → 激活左邻', tabs.active?.label === x1,
    `active=${tabs.active?.label} 期望=${x1}`)
  // 关首位：激活 label2 后关闭 → 必须落到右邻 x1
  tabs.activate(label2!)
  tabs.closeTab(label2!)
  p.check('P7b 关首位页签 → 激活右邻', tabs.active?.label === x1,
    `active=${tabs.active?.label} 期望=${x1}`)
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
  const y1 = await tabs.spawnTab('about:blank')
  const y2 = await tabs.spawnTab('about:blank')
  tabs.activate(x1!) // 激活固定在 x1：下面关的两个都是【后台页签】
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
  tabs.closeTab(y2!) // 还原到 [x1] 单页签稳态
  await sleep(100)

  // 此后只剩 [x1]（激活）。它建在 about:blank 上，而 P9 的 window.open('/opened.html')
  // 依赖真实 http 源来解析相对 URL（about:blank 没有 base，Chromium 无法解析相对地址），
  // 故先把幸存页签导航到夹具页，再作为 P9 的 opener（不再硬编码 label2）。
  await tabs.active!.view.webContents.loadURL(`${base}/tabs-b.html`)
  await sleep(200)
  const opener = x1!

  // ---------- P9 / P10：window.open 只新增 1 页签 + target path 保留 ----------
  // 从当前激活页签发起 window.open('/opened.html')，走 index.ts 的 setWindowOpenHandler
  // → spawnTab(opened)，验证「一次点击只开 1 个」且「目标 path 未被降级成入口地址」。
  const countBeforeOpen = tabs.count
  await tabs.active!.view.webContents.executeJavaScript(
    `window.open('/opened.html', '_blank'); true`, true)
  // setWindowOpenHandler 用 setImmediate 异步 spawnTab。count 在 spawnTab 一开头就 +1，
  // 但激活（activate）在 onSpawn（loadURL /opened.html）完成之后 —— 所以「新页签可见」要
  // 等到 active 换成新 label 且其 URL 已落到 /opened.html，不能只看 count。
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
  // 此时幸存的首个页签是 opener（P7b 后 label2 已关）。
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

  // ---------- 清理：关掉所有页签，验证「关最后一个 → 关闭宿主」的语义不在这里触发 ----------
  // ⚠️ 不能真的关到最后一个（那会 host.close()，探针还没汇总）。留一个，其余 close。
  const rest = labels.slice(1) // 去掉第一个（opener）
  for (const l of rest) tabs.closeTab(l)
  p.check('清理：关闭到只剩一个页签', tabs.count === 1, `count=${tabs.count}`)

  // ---------- 汇总 ----------
  const failed = p.results.filter((r) => !r.ok)
  console.log('\n[probe] ===== 多页签断言组 =====')
  for (const r of p.results) {
    console.log(`[probe] ${r.ok ? 'PASS' : 'FAIL'}  ${r.name}${r.ok ? '' : '  ← ' + r.detail}`)
  }
  console.log(`[probe] ===== ${p.results.length - failed.length}/${p.results.length} 通过 =====\n`)
  return failed.length === 0
}
