/**
 * preload：主进程与页面之间唯一的通道，同时是**注入脚本的执行宿主**。
 *
 * ⚠️ 安全基线（不要为了「方便」破坏它）：
 *   - 只暴露【白名单方法】，绝不把 ipcRenderer / require / process 本体交给页面。
 *     本应用加载的是远程页面，暴露面越大等于把整台机器交出去。
 *   - 页面侧通过 `window.web2app` 使用；Tauri 版的 `window.__TAURI__` 在 Electron
 *     侧不存在，注入脚本按 (window.web2app ?? window.__TAURI__) 的顺序做能力探测。
 *   - 本文件运行在 sandbox 渲染进程里，只能 require('electron')，
 *     用不了 node:fs / node:path 之类的模块 —— 注入脚本的源码由主进程经
 *     `web2app:bootstrap`（同步 IPC）送进来。
 *
 * ⚠️⚠️ 注入必须落在【主世界】（`webFrame.executeJavaScript`），不要改回
 *   `new Function(src)()`（那是在隔离世界跑）。这里踩过坑，缘由值得记住：
 *     · contextBridge 暴露的 `window.web2app` **只存在于主世界**；
 *     · 但 DOM 是主世界与隔离世界【共享】的 —— 于是脚本在隔离世界跑时，标题栏照样
 *       画得出来、38px 让位也照样生效，从表象上看一切正常；
 *     · 真正失效的是三处「读 window」的逻辑：`shellName()` 永远返回 unknown、
 *       `callWindow()` 永远拿不到控制器（三个窗口按钮全部点不动，且页面侧只有一条
 *       console.error）、与启动页内联脚本（主世界）之间的防重标记互相看不见。
 *   Tauri 侧的 initialization_script 本来就在主世界执行 —— 放主世界才是行为对齐的那个选择。
 *   不受页面 CSP 限制：executeJavaScript 走 V8 直接求值（Runtime.evaluate），不是 <script> 注入。
 *
 * ⚠️ 注入时机（对齐 Tauri 版的 initialization_script）：preload 在「document 已创建、
 *   页面自身脚本尚未执行」时运行，正是 Tauri 侧 document-created 的等价时机。
 *
 * ⚠️ 只对【顶层框架】注入：webPreferences.preload 默认也作用于子框架，若不判断
 *   `window.self === window.top`，页面里每个 iframe 都会长出一条标题栏。
 *
 * ⚠️ 跳过内部页面（splash://）：启动页已经把这两个脚本内联在 HTML 里（同样是主世界），
 *   再注入一次会得到【两条标题栏】（同世界的 `__web2appTitlebar` 防重标记这时反而挡得住，
 *   但没必要依赖它）。
 */
import { contextBridge, ipcRenderer, webFrame } from 'electron'

/** 主进程送来的启动信息（见 main/index.ts 的 registerBootstrapIpc） */
interface Bootstrap {
  /** 自绘标题栏高度（逻辑像素）。与 Tauri 版一致，勿改。 */
  chromeH: number
  /** 壳标识，供注入脚本与调试使用 */
  shell: 'electron'
  /** config.json 的 extra 字段，作为 window.web2appExtra 注入 */
  extra: Record<string, unknown>
  /** 注入脚本源码（单一真源：webview-app/src/*.js） */
  titlebarJs: string
  loadingJs: string
}

const boot = ipcRenderer.sendSync('web2app:bootstrap') as Bootstrap

const api = {
  /** 壳标识，供注入脚本做能力探测 */
  isElectron: true as const,
  /** 标题栏占位高度：对齐 Tauri 版的 window.__web2appChromeH */
  chromeH: boot.chromeH,
  // ⚠️ 三个都用 invoke（而不是 send）：调用失败会变成 Promise rejection，注入脚本会
  // 把它打到控制台。用 send 的话「窗口按钮没反应」在页面侧完全无声（Tauri 版当年就是
  // 栽在静默失败上，排查从零开始）。
  minimize: (): Promise<void> => ipcRenderer.invoke('win:minimize'),
  toggleMaximize: (): Promise<boolean> => ipcRenderer.invoke('win:toggle-maximize'),
  close: (): Promise<void> => ipcRenderer.invoke('win:close'),
  // 打开设置窗（标题栏「设置」按钮，位置在最小化右侧）。
  // Tauri 侧没有对应的方法：主窗是远程页面，调不了 `open_settings` 这种应用命令，
  // 那边改发核心插件的事件（见 webview-app/src/titlebar.js 的 openSettings）。
  openSettings: (): Promise<void> => ipcRenderer.invoke('win:open-settings'),
}

contextBridge.exposeInMainWorld('web2app', api)

// window.web2appExtra：与 Tauri 版同名全局变量，页面按此契约读取构建信息等。
// 空对象不暴露，避免让页面误以为「配置里填了 extra」。
if (boot.extra && Object.keys(boot.extra).length > 0) {
  contextBridge.exposeInMainWorld('web2appExtra', boot.extra)
}

// ---------- 注入 ----------

function isInternalPage(): boolean {
  try {
    const p = location.protocol
    return p === 'splash:' || p === 'about:' || p === 'chrome:'
  } catch {
    return true // 拿不到 location 时宁可不注入（注入失败会静默，跳过更安全）
  }
}

/**
 * 在主世界执行一段代码。
 *
 * 失败一律打到控制台：注入失败的表象是「顶部栏凭空消失」/「按钮点了没反应」，
 * 没有日志就只能靠猜。但绝不把异常抛给宿主页面。
 */
function evalInPage(label: string, code: string): void {
  try {
    const ret = webFrame.executeJavaScript(code, true) as unknown
    if (ret && typeof (ret as Promise<unknown>).then === 'function') {
      ;(ret as Promise<unknown>).catch((e: unknown) => {
        console.error(`[web2app] ${label} 执行失败：`, e)
      })
    }
  } catch (e) {
    console.error(`[web2app] ${label} 执行失败：`, e)
  }
}

function runInject(name: string, src: string): void {
  if (!src) {
    console.error(`[web2app] 注入脚本 ${name} 为空，已跳过`)
    return
  }
  // 脚本是我们自己的资产（不是页面输入），且都是自执行 IIFE。
  evalInPage(`注入 ${name}`, src)
}

const isTopFrame = window.self === window.top

if (isTopFrame && !isInternalPage()) {
  runInject('titlebar.js', boot.titlebarJs)
  runInject('loading.js', boot.loadingJs)
}

// ---------- 壳 → 加载层 的指令通道 ----------
// Tauri 侧是在目标页面里 eval `window.__web2appLoading.fail(...)`（同一个世界）。
// Electron 侧加载层挂在主世界，而本文件在隔离世界 —— 读不到对方的全局变量，
// 所以这条指令同样要跨到主世界去执行（同一个 evalInPage 出口）。
export type OverlayCommand =
  | { kind: 'fail'; title: string; detail: string }
  | { kind: 'slow'; title: string }
  | { kind: 'hide' }

ipcRenderer.on('web2app:overlay', (_e, cmd: OverlayCommand) => {
  // 参数用 JSON 序列化后嵌入代码：title/detail 来自配置与运行时错误文本，可能含引号
  // 与换行；JSON 字符串字面量是合法 JS 表达式，且 U+2028/2029 需额外转义（它们是
  // JS 里的行终止符，未转义会截断语句）。
  const lit = JSON.stringify(cmd)
    .replace(/\u2028/g, '\\u2028')
    .replace(/\u2029/g, '\\u2029')
  const code =
    '(function(){var c=' + lit + ';' +
    'var L=window.__web2appLoading;' +
    'if(!L){console.warn("[web2app] 收到加载层指令时加载层未就绪："+c.kind);return;}' +
    'try{' +
    'if(c.kind==="fail")L.fail(c.title,c.detail);' +
    'else if(c.kind==="slow")L.slow(c.title);' +
    'else L.hide();' +
    '}catch(e){console.error("[web2app] 加载层指令执行失败：",e);}' +
    '})()'
  evalInPage('加载层指令', code)
})

export type Web2AppApi = typeof api
