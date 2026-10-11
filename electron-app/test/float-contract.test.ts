/**
 * 悬浮球跨文件契约静态守卫 —— floaticon.html（双壳单一真源）↔ float-preload ↔ 主进程
 * float-window ↔ 设置页 / 资源 stage / Tauri capability 之间「少接一根线就静默失效」
 * 的那些字面量。
 *
 * v1.68：悬浮球只加载内置本地页 floaticon.html（单击调出主窗口），不再有 floatbar.js 注入、
 * float-bootstrap 同步通道、target 缓存轮询与远程 capability。
 *
 * 为什么需要它：悬浮球是多文件拼装的功能（球页方法名、IPC 通道名、sender 校验、
 * preload 白名单、设置页 invoke、stage 资源清单、Tauri capability），任何一处改名 / 漏接，
 * 编译与单测都不会红，真机表现只是「拖动没反应 / 点击没反应 / 打包后缺页面」。
 * 与 Tauri 侧 contract_tests.rs::float_window_contract 同一目的，这里钉 Electron 侧。
 *
 * 运行：`pnpm run unit`（node:test，只读源码文本，零 Electron 依赖）。
 */
import assert from 'node:assert/strict'
import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from 'node:test'

// 单测经 `pnpm run unit` 从 electron-app/ 目录运行（node --test），故以 cwd 为根定位。
const root = process.cwd()
const main = join(root, 'src', 'main')
const renderer = join(root, 'src', 'renderer')
const scripts = join(root, 'scripts')
const tauriSrc = join(root, '..', 'webview-app', 'src')
const tauriCaps = join(root, '..', 'webview-app', 'capabilities')

const floatWin = readFileSync(join(main, 'float-window.ts'), 'utf8')
const floatLayout = readFileSync(join(main, 'float-layout.ts'), 'utf8')
const protocols = readFileSync(join(main, 'protocols.ts'), 'utf8')
const floatController = readFileSync(join(main, 'settings-controller.ts'), 'utf8')
const indexTs = readFileSync(join(main, 'index.ts'), 'utf8')
const floatPreload = readFileSync(join(renderer, 'float-preload.ts'), 'utf8')
const floatIconHtml = readFileSync(join(tauriSrc, 'floaticon.html'), 'utf8')
const settingsHtml = readFileSync(join(tauriSrc, '..', 'dist', 'settings.html'), 'utf8')
const settingsPreload = readFileSync(join(renderer, 'settings-preload.ts'), 'utf8')
const settingsWindow = readFileSync(join(main, 'settings-window.ts'), 'utf8')
const smokeTs = readFileSync(join(main, 'smoke.ts'), 'utf8')
const stage = readFileSync(join(scripts, 'stage-assets.cjs'), 'utf8')
const capability = readFileSync(join(tauriCaps, 'float-icon.json'), 'utf8')

/** 取引号内的 JS 字符串字面量出现次数（防止把同名子串误判）。 */
const countQuoted = (src: string, lit: string): number =>
  (src.match(new RegExp(`['"\`]${lit.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}['"\`]`, 'g')) ?? []).length

test('floaticon.html ↔ float-preload：七个桥方法名逐字一致（含单击调出主窗 / down / cancel）', () => {
  for (const m of ['floatClick', 'floatDown', 'floatMove', 'floatCancel', 'floatDragEnd', 'floatPeek', 'floatHide']) {
    assert.ok(floatIconHtml.includes(`E.${m}`) || floatIconHtml.includes(m), `floaticon.html 缺少桥方法 ${m}`)
    assert.ok(floatPreload.includes(`${m}:`), `float-preload 未暴露 ${m}`)
  }
  // isElectron 能力探测标记两側都在（页面靠它区分 Electron / Tauri 桥）。
  assert.ok(floatPreload.includes('isElectron: true'))
  assert.ok(floatIconHtml.includes('E.isElectron'))
})

test('float-preload ↔ float-window：七条 invoke 通道逐字一致，且旧 bootstrap 通道已移除', () => {
  for (const ch of [
    'web2app:float-click',
    'web2app:float-down',
    'web2app:float-move',
    'web2app:float-cancel',
    'web2app:float-drag-end',
    'web2app:float-peek',
    'web2app:float-hide',
  ]) {
    assert.ok(countQuoted(floatPreload, ch) >= 1, `float-preload 缺少通道 ${ch}`)
    assert.ok(countQuoted(floatWin, ch) >= 1, `float-window 未注册通道 ${ch}`)
  }
  assert.ok(!floatPreload.includes('float-bootstrap'), '球页是本地内置页，不再需要 bootstrap 同步通道')
  assert.ok(!floatWin.includes('float-bootstrap'), '主进程不得再注册 float-bootstrap')
  // 只允许 import contextBridge + ipcRenderer：本地球页不再注入远程脚本，故不得 import webFrame。
  assert.match(
    floatPreload,
    /import \{ contextBridge, ipcRenderer \} from 'electron'/,
    'float-preload 只能 import contextBridge / ipcRenderer（不得再带 webFrame）'
  )
})

test('floaticon.html：点击 vs 拖动判定 + 八个事件名 + 右键隐藏', () => {
  // 未越过阈值的 pointerup 必须发 click（单击 = 切换主窗显隐，v1.71 的核心语义）；
  // v1.72 起判定改成两段式：拖动松手先走 dragend，其余才走（经双击去抖的）click。
  assert.ok(floatIconHtml.includes("if (moved) { send('dragend'); return; }"))
  assert.ok(floatIconHtml.includes("send('click')"))
  // pointercancel：已拖动→dragend 重吸附，未拖动→cancel 只复位（不得弹主窗）。
  assert.ok(floatIconHtml.includes("send(moved ? 'dragend' : 'cancel')"))
  for (const ev of [
    'web2app:float-down',
    'web2app:float-move',
    'web2app:float-cancel',
    'web2app:float-drag-end',
    'web2app:float-peek-enter',
    'web2app:float-peek-leave',
    'web2app:float-hide',
    'web2app:float-click',
  ]) {
    assert.ok(floatIconHtml.includes(ev), `floaticon.html 缺少事件 ${ev}`)
  }
  assert.ok(floatIconHtml.includes("send('down', e.screenX, e.screenY)"), 'pointerdown 必须立即上报按下点')
  assert.ok(floatIconHtml.includes("send('hide')"), '右键菜单必须发 hide')
})

test('球不得挂原生提示（title）：always-on-top 球窗会让提示条从透明环带漏出残片（v1.73）', () => {
  // 球窗 alwaysOnTop ⇒ 原生提示条是**独立的非置顶窗口**，只会被画在球窗**之后**，从球窗的
  // 透明环带（圆内、球本体之外）漏出一块浅色残片，看起来就像「图标背景上有异样图形」。
  // 2026-10-11 真机截图 + Electron 复刻本页（同一份 floaticon.html）双重取证坐实。
  // 只取球元素所在的**那一个标签**判定：页面注释 / 别处的同名字样不参与。
  const idx = floatIconHtml.indexOf('id="w2f-ball"')
  assert.ok(idx > 0, 'floaticon.html 里找不到球元素 #w2f-ball')
  const ballTag = floatIconHtml.slice(floatIconHtml.lastIndexOf('<', idx), floatIconHtml.indexOf('>', idx) + 1)
  assert.ok(
    !ballTag.includes('title='),
    `球不得挂 title 属性：原生提示条会被 always-on-top 球窗遮住、只从透明环带漏出残片。实际标签：${ballTag}`
  )
  assert.ok(ballTag.includes('aria-label='), `球的作用说明必须保留在 aria-label。实际标签：${ballTag}`)
})

test('球必须铺满活柱：球径 == 窗宽（= 区域圆直径），否则透明环带会漏出窗口后面的东西（v1.77b）', () => {
  // 与 Tauri 侧 `contract_tests.rs::float_page_ball_fills_the_live_pillar` 同一个根因：
  // 球窗的可视 / 命中区域是「直径 = 窗宽」的圆；球比这个圆小 ⇒ 两者之间留出一圈**透明环带**，
  // 而窗口是 transparent 的 ⇒ 那圈环带会把窗口后面的东西透出来。Tauri 侧 WebView2 的底层不是
  // 真透明，所以露成一圈浅色残片（真机现象「不选中时 悬浮图标背景异样」，2026-10-11 两次报告）；
  // Electron 是真透明，平时只透出桌面，但这条不变量两壳共用同一份 floaticon.html ⇒ 一并钉死。
  const w = /DEFAULT_FLOAT_W = (\d+)/.exec(floatLayout)?.[1]
  assert.ok(w, 'float-layout.ts 里找不到 DEFAULT_FLOAT_W，无法与球径比对')
  const block = floatIconHtml.slice(floatIconHtml.indexOf('#w2f-ball {'))
  const css = block.slice(0, block.indexOf('}'))
  assert.ok(css.length > 0, 'floaticon.html 里找不到 #w2f-ball 的 CSS 规则块')
  assert.ok(
    css.includes(`width: ${w}px;`) && css.includes(`height: ${w}px;`),
    `#w2f-ball 的宽高必须都等于 ${w}px（= DEFAULT_FLOAT_W = 区域圆直径）：比它小就会留出透明环带`
  )
  assert.ok(
    !css.includes('scale('),
    '#w2f-ball 不得用 transform: scale() 放大：球已铺满活柱，放大只会被区域裁掉并重新露出环带'
  )
  // 半隐藏态的「压暗」必须用 filter —— opacity 会让整颗球半透明，等于把刚堵上的保护自己拆掉。
  const collapsed = floatIconHtml.slice(floatIconHtml.indexOf('html.w2f-collapsed #w2f-ball {'))
  const collapsedCss = collapsed.slice(0, collapsed.indexOf('}'))
  assert.ok(collapsedCss.includes('filter: brightness('), '半隐藏态必须用 filter: brightness() 压暗')
  assert.ok(!collapsedCss.includes('opacity'), '半隐藏态不得用 opacity（会把窗口底色透出来）')
})

test('安全：七条特权 IPC 全部经 sender 校验，preload 有主框架门控，球窗透明隔离', () => {
  // 七个 handle（click/down/move/cancel/drag-end/peek/hide）各自必须调 assertFloatSender(e.sender.id)。
  assert.ok((floatWin.match(/assertFloatSender\(e\.sender\.id\)/g) ?? []).length >= 7)
  assert.ok(floatPreload.includes('window.self === window.top'))
  // 窗口属性：透明 / 置顶 / 不占任务栏 / 无边框 / 无隐形缩放边框 / 隔离沙箱。
  for (const opt of [
    'transparent: true',
    'frame: false',
    'thickFrame: false',
    'skipTaskbar: true',
    'alwaysOnTop: true',
    'sandbox: true',
  ]) {
    assert.ok(floatWin.includes(opt), `float-window 缺少窗口属性 ${opt}`)
  }
})

test('悬浮球只加载内置本地页：floaticon:// 协议加载，不导航远程、不 bootstrap、不碰 sidecar', () => {
  assert.ok(!floatWin.includes('launchSidecar'), 'float-window 不得触发 sidecar')
  assert.ok(!floatWin.includes('resolveTarget('), 'float-window 不得自行解析入口')
  assert.ok(!floatWin.includes('getTargetUrl'), '球页不再读启动期 target 缓存')
  // v1.71 起经 floaticon:// 协议加载（与 Tauri 同 URL，图标也走同一协议）；
  // 任何 http(s) 远程 loadURL 仍然禁止。
  assert.ok(
    floatWin.includes("win.loadURL('floaticon://localhost/')"),
    '球窗必须 loadURL floaticon://localhost/ 内置页（v1.71 起与 Tauri 同协议同 URL）'
  )
  assert.ok(
    !/loadURL\(\s*['"`]https?:/.test(floatWin),
    '球窗不得 loadURL 任何 http(s) 远程地址'
  )
  assert.ok(
    !floatWin.includes('loadFile('),
    '不得再用 loadFile 加载球页（相对路径取不到 webview-app/icons 下的图标）'
  )
  // index.ts：scheme 特权声明 + floaticon 协议注册（处理器在 protocols.ts）+ 图标路径。
  assert.ok(
    /scheme: 'floaticon'/.test(indexTs),
    "index.ts 必须把 floaticon 声明为特权 scheme（standard/secure/supportFetchAPI）"
  )
  assert.ok(
    /function floatIconPath\(\)/.test(indexTs) &&
      /'out', 'inject', 'icon\.png'/.test(indexTs) &&
      /'webview-app', 'icons', 'icon\.png'/.test(indexTs),
    'index.ts 必须提供 floatIconPath：打包态 out/inject/icon.png、开发态 webview-app/icons/icon.png'
  )
  // 旧的远程注入脚本必须已删除（防止单一真源两份并存）。
  assert.ok(!existsSync(join(tauriSrc, 'floatbar.js')), 'webview-app/src/floatbar.js 必须删除')
})

test('floaticon:// 协议分发：/ 返回球页、/icon.png 返回软件图标、其它路径 404（双壳同 URL）', () => {
  assert.ok(protocols.includes("protocol.handle('floaticon'"), 'protocols.ts 必须注册 floaticon 处理器')
  assert.ok(protocols.includes('readFloatIcon'), 'ProtocolDeps 必须含 readFloatIcon')
  assert.ok(protocols.includes("path === '/icon.png'"), 'floaticon 协议必须分流 /icon.png')
  assert.ok(protocols.includes("readInject('floaticon.html')"), 'floaticon 协议根路径必须返回 floaticon.html')
  assert.ok(protocols.includes("status: 404"), 'floaticon 协议对未登记路径必须回 404')
  assert.ok(
    /readFloatIcon: \(\) => \{[\s\S]*?readFileSync\(floatIconPath\(\)\)/.test(indexTs),
    'index.ts 必须把 readFloatIcon（readFileSync floatIconPath，失败回 null）接给 registerProtocols'
  )
  // stage 必须把单一真源图标拷进打包资源（与 floaticon.html 同目录，页面相对路径 icon.png）。
  assert.ok(
    stage.includes("['webview-app/icons/icon.png', 'icon.png']"),
    'stage-assets.cjs 必须登记 webview-app/icons/icon.png → out/inject/icon.png'
  )
  // 页面侧：img 取 icon.png，旧的统一 SVG 图形必须移除。
  assert.ok(floatIconHtml.includes('src="icon.png"'), 'floaticon.html 必须用 <img src="icon.png"> 渲染软件图标')
  assert.ok(!/<svg/.test(floatIconHtml), 'floaticon.html 不得再内嵌统一硬编码 SVG')
  assert.ok(
    existsSync(join(tauriSrc, '..', 'icons', 'icon.png')),
    'webview-app/icons/icon.png 必须真实存在（双壳共用单一真源）'
  )
})

test('单击球必须【切换】原来主窗口的显隐（v1.71：不再只是调出）', () => {
  assert.ok(floatWin.includes('toggleMainWindow: () => void'), 'FloatWindowDeps 必须声明 toggleMainWindow')
  assert.ok(
    floatWin.includes('deps?.toggleMainWindow()'),
    'float-click 处理器必须调 deps.toggleMainWindow()'
  )
  assert.ok(!floatWin.includes('showMainWindow'), 'float-window 不得再依赖只显示不隐藏的 showMainWindow')
  // index.ts 接线：可见且非最小化 → hide；否则 presentMainWindow（含多页签自愈）。
  const wiring = indexTs.slice(indexTs.indexOf('registerFloatWindow({'))
  assert.ok(
    /toggleMainWindow: \(\) => \{[\s\S]*?mainWindow\.isVisible\(\)[\s\S]*?mainWindow\.isMinimized\(\)[\s\S]*?mainWindow\.hide\(\)[\s\S]*?presentMainWindow\(\)/.test(
      wiring
    ),
    'index.ts 必须把 toggle 语义（可见非最小化→hide，否则 presentMainWindow）接给 registerFloatWindow'
  )
})

test('设置页 / 设置 preload / stage：float 接线一处不漏', () => {
  // settings.html 单页含开关、显示按钮、保存字段与 invoke 命令。
  for (const lit of ['id="floatEnabled"', 'id="showFloatBtn"', 'float_enabled', 'show_float_window']) {
    assert.ok(settingsHtml.includes(lit), `settings.html 缺少 ${lit}`)
  }
  // 文案必须是「悬浮图标」语义（单击调出主窗），不再是 v1.67 的「悬浮小窗」。
  assert.ok(settingsHtml.includes('单击图标可切换主窗口显隐'))
  // v1.72：「立即显示」必须顺手把开关勾上，否则「点了按钮看到球、重启后没了」。
  assert.ok(
    /showFloatBtn[\s\S]*?checked\s*=\s*true/.test(settingsHtml),
    '「立即显示悬浮图标」必须同步勾上「启用桌面悬浮图标」'
  )
  assert.ok(settingsPreload.includes("'show_float_window'"))
  // floaticon.html 必须进打包资源清单（缺了 stage 会 exit 1，但先在这里钉住清单登记）。
  assert.ok(stage.includes("['webview-app/src/floaticon.html', 'floaticon.html']"))
  assert.ok(!stage.includes('floatbar.js'), 'stage 清单不得再登记 floatbar.js')
})

test('Tauri capability：float-icon.json 只授权 float 窗口的核心事件 emit 且无 remote 段', () => {
  const cap = JSON.parse(capability) as {
    identifier?: string
    windows?: string[]
    permissions?: string[]
    remote?: { urls?: string[] }
  }
  assert.deepEqual(cap.windows, ['float'])
  assert.ok(cap.permissions?.includes('core:event:allow-emit'))
  for (const denied of ['core:window:allow-close', 'core:app:allow-invoke']) {
    assert.ok(!cap.permissions?.includes(denied))
  }
  assert.equal(cap.remote, undefined, '球页是本地页，capability 不得再授任何远程 origin')
  assert.ok(
    !existsSync(join(tauriCaps, 'float-remote.json')),
    'capabilities/float-remote.json 必须删除（已被 float-icon.json 取代）'
  )
})

test('退出路径：悬浮球的 close 默认转隐藏，但 app 退出必须放行', () => {
  // close 处理器必须同时认「主动销毁（allowClose）」与「应用正在退出（isQuitting）」。
  // 只认前者时，app.quit() 会被这一窗的 preventDefault 整个取消（实测复现：
  // 主窗已关、球窗只剩隐藏、进程仍活着）⇒ 用户看到「开着悬浮球就退不掉程序」。
  const closeBlock = floatWin.slice(floatWin.indexOf("win.on('close'"))
  assert.ok(
    /if \(allowClose \|\| deps\?\.isQuitting\(\)\)/.test(closeBlock),
    'float-window 的 close 处理器必须放行 isQuitting（否则 app.quit() 被取消）'
  )
  assert.ok(
    floatWin.includes('isQuitting: () => boolean'),
    'FloatWindowDeps 必须声明 isQuitting 依赖'
  )
  assert.ok(
    /isQuitting: \(\) => quitting/.test(indexTs),
    'index.ts 必须把（before-quit / requestQuit 置位的）quitting 接给悬浮球'
  )
})

test('v1.70 拖动协议：页面上报屏幕绝对坐标，主进程按抓取偏移绝对定位（不能再用 clientX 增量）', () => {
  // 页面侧：阈值判定与移动上报必须用 screenX/screenY；真实代码行里不得再出现
  // clientX/clientY（先剥 HTML 注释块与行内 // 注释，避免解释性注释满足 / 骗过守卫）。
  // 旧实现用相对球窗视口的增量，窗口每帧 setBounds 后下一帧增量被自身位移抵消（半速跟手 / 抖动）。
  const codeLines = floatIconHtml
    .replace(/<!--[\s\S]*?-->/g, '')
    .split('\n')
    .map((l) => {
      const i = l.indexOf('//')
      return i >= 0 ? l.slice(0, i) : l
    })
  assert.ok(floatIconHtml.includes('e.screenX'), 'floaticon.html 拖动必须读取 e.screenX')
  assert.ok(floatIconHtml.includes('e.screenY'), 'floaticon.html 拖动必须读取 e.screenY')
  assert.ok(!codeLines.some((l) => /client[XY]/.test(l)), 'floaticon.html 不得再用 clientX/clientY 算拖动增量')
  assert.ok(floatIconHtml.includes("send('move', latestSX, latestSY)"), 'rAF 必须上报最新屏幕绝对坐标')
  // 抓取偏移必须取自 pointerdown 按下点（真机取证：在首帧 move 才用当前光标记锚点，
  // 窗口首帧纹丝不动）：down 通道 → press 暂存 → 首帧 move 用 press 算 anchor。
  assert.ok(floatIconHtml.includes("send('down', e.screenX, e.screenY)"), 'pointerdown 必须立即发 down')
  assert.ok(floatPreload.includes('floatDown: (sx: number, sy: number)'))
  assert.ok(floatWin.includes('onDown(n(sx), n(sy))'), 'float-down 通道必须把按下点交给 onDown')
  assert.ok(floatWin.includes('let press'), '主进程必须有按下点暂存 press')
  assert.ok(floatWin.includes('const p = press ?? { x: sx, y: sy }'), '首帧 move 必须取走 press（缺失才回退当前帧）')
  assert.ok(floatWin.includes('anchor = { x: p.x - b0.x, y: p.y - b0.y }'), '锚点必须按下点 − 展开后原点')
  // 打断 / 点击 / 松手都必须统一清按压 / 拖动态（残留 moving=true 会让悬停永久失效）。
  assert.ok(floatWin.includes('function clearDragRuntime()'), '必须有 clearDragRuntime')
  assert.ok(floatWin.includes("ipcMain.handle('web2app:float-cancel'"), '未拖动的 pointercancel 必须有 cancel 通道')
  // preload / 主进程参数名同步为 sx/sy。
  assert.ok(floatPreload.includes('floatMove: (sx: number, sy: number)'))
  assert.ok(floatWin.includes('onMove(n(sx), n(sy))'), 'float-move 通道必须把屏幕坐标交给 onMove')
  // 主进程必须有拖动中软钳制 + 松手自由态钳制。
  assert.ok(floatWin.includes('clampVisible('), '拖动中必须软钳制（留窄边、可跨屏、不甩丢）')
  assert.ok(floatWin.includes('clampFreeInside('), '自由态松手必须把球完整钳回显示器')
})

test('v1.70 悬停缩回：延迟 + 代际 + 真实光标位置校验（治边缘抖动）', () => {
  assert.ok(floatWin.includes('COLLAPSE_DELAY_MS'), '必须经共享常量延迟缩回')
  assert.ok(floatWin.includes('finishPeekLeave'), '必须有延迟到期的统一收口函数')
  assert.ok(floatWin.includes('getCursorScreenPoint'), '到期必须读真实光标位置（在缩回后矩形内则不缩）')
  assert.ok(floatWin.includes('pointInRect('), '光标位置必须做点包含判定')
  assert.ok(floatWin.includes('cancelPendingCollapse'), '展开 / 隐藏 / 销毁 / 起拖必须取消排队中的缩回')
})

test('v1.70 建窗：ready-to-show 后再显现透明球（防 Windows 黑块 / 空窗）', () => {
  assert.ok(floatWin.includes("win.once('ready-to-show', reveal)"), '透明球窗必须等 ready-to-show 再 show')
  assert.ok(!/applyInitialGeometry\(win, saved\)\s*\n\s*win\.show\(\)/.test(floatWin), '不得建窗后立刻 show')
})

test('v1.71 几何定位：唯一入口 placeFloat 始终带固定 56×56 完整 rect（治出屏移动后 getBounds 宽漂移）', () => {
  // Windows 上 frameless + transparent 球窗出屏 / 负坐标移动后，部分字段 setBounds 会让
  // getBounds() 的宽逐次漂移（harness 实测 56→58→60），几何盲信 b.width 半球落点就会
  // -28→-30→-32 越带越偏：必须用「带固定宽高的完整 rect」唯一定位，并与 Tauri live_px 对齐。
  assert.ok(
    floatWin.includes('function placeFloat(win: BrowserWindow, x: number, y: number): void'),
    '必须有 placeFloat 唯一定位入口'
  )
  // 按行取到 placeFloat 函数体结束（兼容 CRLF / LF，不能写死 '\n}\n'）。
  const rest = floatWin.slice(floatWin.indexOf('function placeFloat')).split(/\r?\n/)
  const fn: string[] = []
  for (const l of rest) {
    fn.push(l)
    if (l === '}') break
  }
  const fnText = fn.join('\n')
  assert.ok(
    fnText.includes('width: DEFAULT_FLOAT_W') && fnText.includes('height: DEFAULT_FLOAT_H'),
    'placeFloat 每次都必须携带固定 56×56（完整 rect，杜绝漂移累积）'
  )
  // 剥掉块注释与行注释后，真实代码里 setBounds/setPosition 只能有 placeFloat 内一处。
  const codeLines = floatWin
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .split('\n')
    .map((l) => {
      const i = l.indexOf('//')
      return i >= 0 ? l.slice(0, i) : l
    })
    .filter((l) => /\.setBounds\(|\.setPosition\(/.test(l))
  assert.ok(
    codeLines.length === 1 && codeLines[0].includes('win.setBounds'),
    `窗口定位必须只经 placeFloat（实际残留 ${codeLines.length} 处 setBounds/setPosition）`
  )
  // 吸附 / 落盘几何不得再直接读运行期 getBounds 的宽高（漂移源）：expand / collapse /
  // persist 三处必须显式传 DEFAULT_FLOAT_W，且不得读取 raw.width / raw.height。
  assert.ok(
    floatWin.includes('expandedX(d.edge, d.mon, DEFAULT_FLOAT_W)')
      && floatWin.includes('collapsedX(d.edge, d.mon, DEFAULT_FLOAT_W, PEEK_LOGICAL)')
      && floatWin.includes('expandedX(dock.edge, dock.mon, DEFAULT_FLOAT_W)'),
    'expand / collapse / persist 的吸附 x 必须用 DEFAULT_FLOAT_W（不得用 getBounds().width）'
  )
  assert.ok(
    !floatWin.includes('raw.width') && !floatWin.includes('raw.height'),
    'onDragEnd 只允许信 getBounds 的 x/y，严禁读 raw.width / raw.height（漂移源）'
  )
  assert.ok(
    floatWin.includes('const b: Rect = { x: raw.x, y: raw.y, width: DEFAULT_FLOAT_W'),
    'onDragEnd 必须用固定 56 合成几何 Rect（只取 getBounds 的 x/y）'
  )
})

test('启动期必须建立进程级副作用：走 applyInitialSettings（prev = DEFAULT_SETTINGS）', () => {
  // 若入口用 applySettings(loadedSettings.settings) 做冷启动：runtimeSettings 已是同一份，
  // 托盘 / 悬浮球的「变化才动」diff 恒为 false ⇒ 一个副作用都不会建立（实测调用数 0），
  // 表现是「勾了启用托盘 / 悬浮球，重启后图标与悬浮球都不再出现」。
  assert.ok(
    /settingsCtl\.applyInitialSettings\(loadedSettings\.settings\)/.test(indexTs),
    'index.ts 冷启动必须调 applyInitialSettings(loadedSettings.settings)'
  )
  assert.ok(
    !/settingsCtl\.applySettings\(loadedSettings\.settings/.test(indexTs),
    'index.ts 冷启动不得用 applySettings（diff 恒 false，副作用不建立）'
  )
  // 控制器侧：applyInitialSettings 必须以 DEFAULT_SETTINGS 为比较基准，且托盘 / 悬浮球
  // 两处 diff 都改用函数开头的 prev（否则 prevOverride 会被 deps.getSettings() 绕过）。
  const initial = floatController.slice(floatController.indexOf('function applyInitialSettings'))
  assert.ok(
    /applySettings\(next, \{ persist: false \}, DEFAULT_SETTINGS\)/.test(initial),
    'applyInitialSettings 必须以 DEFAULT_SETTINGS 为 prev 调用 applySettings'
  )
  assert.ok(
    floatController.includes('next.tray_enabled !== prev.tray_enabled') &&
      floatController.includes('next.float_enabled !== prev.float_enabled'),
    '托盘 / 悬浮球的开关 diff 必须以 prev 为基准（不得再调 deps.getSettings()）'
  )
})
test('v1.72 双击去抖：窗口期内的第二次单击必须丢弃（否则主窗闪现）', () => {
  // 系统双击在 ~100ms 内送来两次 pointerup（两次 click），而 click 是 toggle 主窗：
  // 两次相互抵消，用户只看到主窗「闪现」一下。窗口期内的第二次点击必须丢弃。
  assert.ok(floatIconHtml.includes('var CLICK_DEBOUNCE_MS = 250;'), '球页必须有双击去抖窗口常量')
  assert.ok(floatIconHtml.includes('var lastClickAt = 0;'))
  assert.ok(
    floatIconHtml.includes('if (now - lastClickAt < CLICK_DEBOUNCE_MS) return;'),
    '窗口期内的第二次点击必须早退（丢弃），不得再发 click'
  )
  assert.ok(floatIconHtml.includes('lastClickAt = now;'), '通过去抖的单击必须记录时间戳')
  assert.ok(
    !floatIconHtml.includes("send(moved ? 'dragend' : 'click')"),
    '旧的未去抖单击写法必须消失（双击闪现的来源）'
  )
})

test('v1.72 球页 28px 必须等于球边长的一半（防与 PEEK_LOGICAL 数值撞车）', () => {
  assert.ok(floatLayout.includes('export const DEFAULT_FLOAT_W = 56'), '球边长必须是 56（页面 28 = 一半）')
  assert.ok(floatIconHtml.includes('left: 28px;'), '锚左时球心必须在左起 28px = DEFAULT_FLOAT_W/2')
  assert.ok(
    floatIconHtml.includes('left: calc(100% - 28px);'),
    '锚右时球心必须在视口右起 28px = DEFAULT_FLOAT_W/2'
  )
})

test('v1.72 capability description 与实现同步（八个事件 + toggle 语义）', () => {
  const cap = JSON.parse(capability) as { description?: string }
  const desc = cap.description ?? ''
  assert.ok(desc.includes('切换主窗口显隐'), 'description 必须写明单击是 toggle 主窗显隐')
  for (const ev of [
    'float-down',
    'float-move',
    'float-cancel',
    'float-drag-end',
    'float-peek-enter',
    'float-peek-leave',
    'float-hide',
    'float-click',
  ]) {
    assert.ok(desc.includes(ev), `capability description 缺事件 ${ev}`)
  }
  assert.ok(!desc.includes('单击球调出主窗口'), 'description 不得再写「单击球调出主窗口」')
})

test('v1.72 球窗圆形区域：setShape(circleShapeRects(...)) 必须存在且必须包在 try/catch 里', () => {
  // 为什么钉它：`transparent: true` 只让四角**看不见**，四角在系统看来仍属窗口、仍吃点击；
  // 补「四角穿透」的唯一手段就是 `setShape`。而它是 Electron 的 `@experimental`（win32/linux），
  // 一旦抛错就不能让球整个不可用 —— 所以「调用」和「降级」缺一不可：
  //   · 少了调用 ⇒ 悄悄退回「四角挡点击」（老缺陷复活，肉眼与单测都看不出来）；
  //   · 少了 try/catch ⇒ 未来某平台抛错时球窗直接挂掉（比老缺陷更糟）。
  // 反向对照（做守卫时必须验证它真咬人）：把 `win.setShape(...)` 那行删掉 → 本条必须 FAILED；
  // 把 `try {` 改成普通块 → 本条必须 FAILED。
  assert.ok(
    floatLayout.includes('export function circleShapeRects('),
    'float-layout.ts 必须导出 circleShapeRects（球窗裁圆几何的单一真源）'
  )
  // ⚠️ 必须**先去掉注释**再匹配：第一版守卫直接对原文 `includes`，结果把
  //    `// win.setShape(...)`（反向对照时注释掉的那行）也算成「存在」⇒ 守卫反向对照
  //    时**不报错**、等于没咬人。踩过一次，记录在此。
  const stripComments = (src: string): string =>
    src.replace(/^[ \t]*\/\/[^\n]*$/gm, '').replace(/[ \t]\/\/[^\n]*/g, '')
  const winCode = stripComments(floatWin)
  assert.ok(
    winCode.includes('win.setShape(circleShapeRects(DEFAULT_FLOAT_W))'),
    'float-window.ts 必须用 circleShapeRects(DEFAULT_FLOAT_W) 调 setShape（注释掉的那行不算）'
  )
  const at = winCode.indexOf('win.setShape(circleShapeRects(DEFAULT_FLOAT_W))')
  const before = winCode.slice(Math.max(0, at - 120), at)
  const after = winCode.slice(at, at + 400)
  assert.ok(/try\s*\{[^}]*$/.test(before), 'setShape 必须包在 try 块内（失败要能降级，不能让球不可用）')
  assert.ok(after.includes('catch'), 'try 之后必须有 catch（降级分支）')
})

test('v1.75 关窗驻留：只启用悬浮图标时点 ✕ 必须转隐藏（否则球成孤儿 + 进程僵尸）', () => {
  // 真机缺陷（2026-10-11 使用者实测，他机器上的 settings.json 就是
  // tray_enabled:false / minimize_on_close:false / float_enabled:true）：
  // 关窗走原来的 else ⇒ 主窗被【真销毁】，而球窗还活着 ——
  //   ① 球窗挡住 window-all-closed（它只在**所有**窗口关闭时才发）⇒ 进程退不掉的僵尸；
  //   ② 点球时 presentMainWindow 的 `isDestroyed()` 早退 ⇒ 用户看到「点了悬浮图标不回窗口」。
  // 球的产品语义就是「唤回主窗的入口」，故「球在 ⇒ 应用驻留」必须成立。
  //
  // ⚠️ 必须**限定在 close 处理器内部**再匹配：`runtimeSettings.float_enabled` 在
  //    index.ts 的启动日志里也出现一次（`悬浮图标 : …`）。对全文 includes 的话，
  //    把 close 判据整段删掉照样能绿 —— 那是自我满足的假守卫。
  const lines = indexTs.slice(indexTs.indexOf("win.on('close'")).split(/\r?\n/)
  const block: string[] = []
  for (const l of lines) {
    block.push(l)
    if (l === '  })') break
  }
  // 先剥注释再匹配（注释里的解释文字不得满足守卫）。
  const body = block
    .join('\n')
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/^[ \t]*\/\/[^\n]*$/gm, '')
  assert.ok(body.includes('win.hide()'), 'close 处理器必须仍在（取块失败要立刻现形）')
  assert.ok(
    /process\.platform === 'win32' && runtimeSettings\.float_enabled/.test(body),
    '关窗驻留判据必须含「Windows 且 float_enabled」（与建球的前置条件同一个判据）'
  )
  assert.ok(
    /runtimeSettings\.tray_enabled && runtimeSettings\.minimize_on_close/.test(body),
    '托盘驻留必须保持「两个开关都要」—— 不得被顺手放宽成只看 tray_enabled'
  )
  assert.ok(
    /if \(trayResident \|\| floatResident\)/.test(body),
    '两个驻留理由必须并列进同一个分支（任一成立即 preventDefault + hide）'
  )
})

test('v1.75 设置页「退出程序」：HTML 字面量 / 垫片白名单 / 主进程处理器 三处同改', () => {
  // 启用托盘或悬浮图标后，「关闭窗口」按设计只等于隐藏 ⇒ 没启用托盘的用户原本**没有**
  // 任何退出入口（托盘的「退出」菜单项并不存在）。这条按钮是那种状态下唯一的出口，
  // 而「页面字面量 / 垫片白名单 / IPC 命令名」三处漏一，表现都只是「按钮点了没反应」。
  assert.ok(settingsHtml.includes('id="quitBtn"'), 'settings.html 必须有 #quitBtn（脚本 / 探针的接口）')
  assert.ok(/invoke\("quit_app"\)/.test(settingsHtml), 'settings.html 必须 invoke("quit_app")')
  assert.ok(settingsPreload.includes("'quit_app'"), 'settings-preload 的 ALLOWED 必须放行 quit_app')
  assert.ok(
    /ipcMain\.handle\('tauri:quit_app'/.test(settingsWindow),
    'settings-window 必须注册 tauri:quit_app 处理器（否则页面拿到的是一条「命令不存在」）'
  )
  // 处理器必须接到入口的 requestQuit —— 它是「**先置 quitting** 再 app.quit()」的唯一实现；
  // 直接用裸 app.quit() 会被 close 处理里的驻留分支拦下（v1.68 缺陷② 的同一形态）。
  assert.ok(
    /quitApp: requestQuit/.test(indexTs),
    'index.ts 必须把 requestQuit 接给设置窗（先置 quitting 再 app.quit）'
  )
  assert.ok(
    /function requestQuit\(\): void \{[\s\S]*?quitting = true[\s\S]*?app\.quit\(\)/.test(indexTs),
    'requestQuit 必须先置 quitting 再 app.quit()（顺序不可交换）'
  )
  // 还有第四处：启动冒烟的「命令契约」清单（`smoke.ts`）—— 它只判「页面里还在不在」，
  // 但清单本身必须与 ALLOWED **同集合**：漏了名字，那条命令就少了运行期绊线
  // （show_float_window 与 quit_app 都曾长期缺席，v1.75 补齐）。
  const allowed = [...settingsPreload.matchAll(/^[ \t]*'([a-z_]+)',$/gm)].map((m) => m[1])
  assert.ok(allowed.length >= 6, `settings-preload 的 ALLOWED 解析异常（只取到 ${allowed.length} 个）`)
  for (const cmd of allowed) {
    assert.ok(
      smokeTs.includes(`'${cmd}'`),
      `smoke.ts 的「命令契约」清单缺少 ${cmd}（ALLOWED 里有，清单里必须有）`
    )
  }
})

test('v1.76 右键球 = 关闭悬浮图标功能 + 唤回主窗口（不是「只把球藏起来」）', () => {
  // 真机缺陷（2026-10-11 使用者实测，策略 tray_enabled:false / minimize_on_close:false /
  // float_enabled:true）：主窗「关闭」只等于隐藏到球（驻留理由②），球成了桌面上**唯一**
  // 可见的入口；此刻右键把球一藏 —— 进程因 float_enabled 仍为 true 继续驻留，桌面上却
  // 再也唤不出任何窗口。右键在用户直觉里就是「我不要这个球了」，正确语义是回到主窗口：
  //   ① float_enabled 置 false 并落盘（含同步已打开设置页的复选框）；② 销毁球窗；③ 显示主窗。
  //
  // ⚠️ 必须把三件事逐条限定在**接线块内部**断言：只判「通道存在」或对全文 includes 是自我
  //    满足的假守卫（改回 hideFloatWindow 照样能绿）。
  const hideHead = floatWin.slice(floatWin.indexOf("ipcMain.handle('web2app:float-hide'"))
  const hideBlock = hideHead.slice(0, hideHead.indexOf('})') + 2)
  assert.ok(
    /deps\?\.onHideRequest\(\)/.test(hideBlock),
    'float-hide 通道必须转发 deps.onHideRequest()（不是 hideFloatWindow —— 那是托盘 toggle 用的临时收起）'
  )
  assert.ok(
    !/hideFloatWindow\(\)/.test(hideBlock),
    'float-hide 通道不得退回 hideFloatWindow()（v1.76 死局回归）'
  )
  assert.ok(
    floatWin.includes('onHideRequest: () => void'),
    'FloatWindowDeps 必须声明 onHideRequest'
  )

  // index.ts 接线：实现提取成模块级 hideFloatFromIcon，注册处与探针 m3 钩子共用**同一个**
  // 函数（否则探针验的不是真机路径）。函数体必须：只翻转 float_enabled 一位 + 落盘 +
  // 同步设置页 + 销毁球窗 + 唤回主窗，且延后一拍（当前栈是球窗自己的 IPC 派发栈，
  // 不能同步销毁事件源窗口）。
  assert.ok(
    /onHideRequest: hideFloatFromIcon/.test(indexTs),
    'registerFloatWindow 的 onHideRequest 必须接 hideFloatFromIcon'
  )
  assert.ok(
    /hideFloatFromIcon: \(\) => hideFloatFromIcon\(\)/.test(indexTs),
    'm3 探针钩子必须复用 hideFloatFromIcon（否则探针断言的不是真机路径）'
  )
  const wiring = indexTs.slice(indexTs.indexOf('function hideFloatFromIcon('))
  const wiringBlock = wiring.slice(0, wiring.indexOf('\n}'))
  assert.ok(wiringBlock.length > 0, 'index.ts 必须有模块级 hideFloatFromIcon 函数')
  assert.ok(
    /runtimeSettings = \{ \.\.\.runtimeSettings, float_enabled: false \}/.test(wiringBlock),
    'onHideRequest 必须把 float_enabled 置 false（刻意只动这一位，不经 applySettings 连带重注册快捷键）'
  )
  assert.ok(
    /saveSettings\(storeLoc, runtimeSettings\)/.test(wiringBlock),
    'onHideRequest 必须把设置落盘（否则重启后球又回来）'
  )
  assert.ok(
    /updateSettingsWindowFloat\(false\)/.test(wiringBlock),
    '必须同步设置页复选框（否则用户随手一点「保存」就把 float_enabled 推回 true）'
  )
  assert.ok(/destroyFloatWindow\(\)/.test(wiringBlock), '必须销毁球窗')
  assert.ok(
    /presentMainWindow\(\)/.test(wiringBlock),
    '必须把主窗口显示出来（用户点完右键要看到窗口，而不是一片空桌面）'
  )
  assert.ok(
    /setImmediate\(/.test(wiringBlock),
    '必须延后一拍执行（当前栈是球窗自己的 IPC 派发栈，不能同步销毁事件源窗口）'
  )

  // 设置窗侧推送实现 + 复选框 id 跨文件一致 + 球页 aria 新语义。
  assert.ok(
    /export function updateSettingsWindowFloat\(enabled: boolean\)/.test(settingsWindow),
    'settings-window.ts 必须导出 updateSettingsWindowFloat'
  )
  assert.ok(settingsWindow.includes("getElementById('floatEnabled')"), '推送脚本必须写 #floatEnabled')
  assert.ok(settingsHtml.includes('id="floatEnabled"'), '共享设置页必须有 #floatEnabled')
  assert.ok(
    floatIconHtml.includes('右键关闭悬浮图标'),
    'floaticon.html 的 aria-label 必须写明「右键关闭悬浮图标」'
  )
})
