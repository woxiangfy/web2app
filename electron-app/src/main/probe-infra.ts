/**
 * 探针断言基建（探针四拆分之二）—— DOM 快照脚本、断言记录器、时序/进程工具、
 * 以及注入给 index.ts 的 Context 钩子契约（ProbeContext / M3ProbeHooks）。
 *
 * 本模块不含具体断言：单窗口断言组在 `probe-single.ts`，页签断言组在 `probe-tabs.ts`。
 */
import type { BrowserWindow } from 'electron'

import type { ProbeServer } from './probe-server'
import type { SidecarHandle, SidecarRequest } from './sidecar'
import type { Settings, StoreLocation } from './settings'
import type { Hotkey } from './hotkey'
import type { RegisterResult } from './shortcuts'

export interface Rect {
  top: number
  bottom: number
  left: number
  right: number
  w: number
  h: number
}

export interface Snapshot {
  barCount: number
  barRect: Rect | null
  barBg: string | null
  barZ: string | null
  appRegion: string | null
  shell: string | null
  chromeH: string | null
  htmlPadTop: string
  overlay: boolean
  extra: string | null
  api: string | null
  title: string | null
  buttons: number
  /** 「设置」按钮与最小化按钮的相对位置（null = 有一个没找到） */
  gear: { gearRight: number; minLeft: number; gap: number } | null
  shiftAttr: boolean
  shrinkAttr: boolean
  r: { hdr: Rect | null; root: Rect | null; shell: Rect | null; ftr: Rect | null; content: Rect | null }
  view: { ih: number; iw: number; ch: number; cw: number }
  /**
   * 从右往左扫，第一个「能命中标题栏」的 x。
   *
   * ⚠️⚠️ 这是判断「标题栏有没有被滚动条盖掉一角」的**真判据**，不是 `barRect.width`。
   * Chromium 把视口滚动条绘制在所有 DOM 内容之上，标题栏的布局宽度一直是窗口宽
   * （`rect.width == innerWidth`，断言全绿），但那 15px 在绘制时被覆盖 —— 只有命中测试
   * 感受得到：滚动条区域的 `elementFromPoint` 返回 null，扫不过去。
   */
  hitRight: number
  /**
   * 「滚动条有没有越过标题栏」的**直接**判据：标题栏那 38px 里，窗口最右一列逐点做命中测试，
   * 把「命不中标题栏」的 y 收进来。空数组 = 干净。
   * 根滚动条时代这里是一串 `y`（滚动条不是 DOM 元素，那一列命不中任何东西）。
   */
  intrudeRows: number[]
  /** `body.offsetWidth - clientWidth`：滚动条**仍然存在**且归 body 管（占宽一条）时 > 0 */
  bodyScrollbarW: number
  /** 期望 `auto`：body 才是滚动容器 */
  bodyOverflowY: string | null
  /** 期望 `hidden`：html 不再承担滚动（否则根滚动条会横穿标题栏） */
  rootOverflowY: string | null
  /** 期望 `38px`：body 盒子被推到标题栏下沿 */
  bodyMarginTop: string | null
  /** 期望 ≈ 视口高度 - 38（页面写 `min-height:100vh` 时这里会露馅） */
  bodyH: number | null
  /** 页面是否仍能滚动（滚动能力必须保留 —— 不能靠「让页面滚不动」换掉那条缝） */
  bodyScrollable: boolean
  /** 旧契约观测量：html 是否还在滚（新契约下必须为 false） */
  rootScrollable: boolean
  scrollY: number
  path: string
}

/** 在主世界读一份几何快照（DOM 对主世界与隔离世界共享，注入脚本的结果都能读到）。 */
export const SNAPSHOT_JS = `(() => {
  const R = (sel) => { const e = document.querySelector(sel); if (!e) return null;
    const r = e.getBoundingClientRect();
    return { top:+r.top.toFixed(1), bottom:+r.bottom.toFixed(1), left:+r.left.toFixed(1), right:+r.right.toFixed(1), w:+r.width.toFixed(1), h:+r.height.toFixed(1) }; };
  const de = document.documentElement;
  const bar = document.getElementById('__web2app_titlebar__');
  let region = null, bg = null, z = null;
  if (bar) { const cs = getComputedStyle(bar); bg = cs.backgroundColor; z = cs.zIndex;
    try { region = cs.webkitAppRegion || cs.getPropertyValue('-webkit-app-region'); } catch (e) {} }
  const root = document.querySelector('#root');
  const hdr = document.querySelector('#hdr');
  return {
    barCount: document.querySelectorAll('#__web2app_titlebar__').length,
    barRect: bar ? R('#__web2app_titlebar__') : null,
    barBg: bg, barZ: z, appRegion: region,
    shell: de.getAttribute('data-web2app-shell'),
    chromeH: de.getAttribute('data-web2app-chrome-h'),
    htmlPadTop: getComputedStyle(de).paddingTop,
    overlay: !!document.getElementById('__web2app_loading__'),
    extra: (window.web2appExtra && typeof window.web2appExtra === 'object') ? JSON.stringify(window.web2appExtra) : null,
    api: (window.web2app && typeof window.web2app === 'object') ? Object.keys(window.web2app).sort().join(',') : null,
    title: bar ? (bar.querySelector('.w2t-title') || {}).textContent : null,
    buttons: ['w2t-back','w2t-fwd','w2t-min','w2t-settings','w2t-max','w2t-close'].filter((c) => bar && bar.querySelector('.' + c)).length,
    // 「设置按钮在最小化左侧」的位置判据：只比几何，不看 DOM 顺序 —— 顺序对了但被样式
    // 挪到别处（float / order）一样是错的，而使用者提的就是「位置」。
    gear: (() => {
      const mn = bar && bar.querySelector('.w2t-min');
      const st = bar && bar.querySelector('.w2t-settings');
      if (!mn || !st) return null;
      const a = mn.getBoundingClientRect(), b = st.getBoundingClientRect();
      return { gearRight: +b.right.toFixed(1), minLeft: +a.left.toFixed(1), gap: +(a.left - b.right).toFixed(1) };
    })(),
    shiftAttr: !!document.querySelector('[data-web2app-chrome-shift]'),
    shrinkAttr: !!document.querySelector('[data-web2app-chrome-shrink]'),
    r: { hdr: R('#hdr'), root: R('#root'), shell: R('#shell'), ftr: R('#ftr'), content: R('#content') },
    view: { ih: window.innerHeight, iw: window.innerWidth, ch: de.clientHeight, cw: de.clientWidth },
    // ⚠️ 命中测试：被滚动条占住的那一段 elementFromPoint 返回 null，所以「扫得到右缘」
    //    才等价于「标题栏右端没有被盖」。y=20 取在标题栏高度内（38px）。
    hitRight: (() => { for (let x = window.innerWidth - 1; x >= 0; x--) {
      const e = document.elementFromPoint(x, 20);
      if (e && e.closest && e.closest('#__web2app_titlebar__')) return x; } return -1; })(),
    // 「滚动条有没有越过标题栏」的**直接**判据：标题栏那 38px 里，窗口最右一列逐点命中标题栏。
    // 根滚动条时代这里会拿到一串 y（滚动条不是 DOM 元素，那一列命不中任何东西）。
    intrudeRows: (() => { const bad = [];
      for (let y = 0; y < 38; y++) {
        const e = document.elementFromPoint(window.innerWidth - 1, y);
        if (!(e && e.closest && e.closest('#__web2app_titlebar__'))) bad.push(y);
      } return bad; })(),
    bodyScrollbarW: document.body ? document.body.offsetWidth - document.body.clientWidth : 0,
    bodyOverflowY: document.body ? getComputedStyle(document.body).overflowY : null,
    rootOverflowY: getComputedStyle(de).overflowY,
    bodyMarginTop: document.body ? getComputedStyle(document.body).marginTop : null,
    bodyH: document.body ? Math.round(document.body.getBoundingClientRect().height) : null,
    bodyScrollable: document.body ? document.body.scrollHeight > document.body.clientHeight : false,
    rootScrollable: de.scrollHeight > de.clientHeight,
    scrollY: window.scrollY,
    path: location.pathname
  };
})()`

export class Probe {
  readonly results: { name: string; ok: boolean; detail: string }[] = []

  check(name: string, ok: boolean, detail = ''): void {
    this.results.push({ name, ok, detail: ok ? '' : detail })
  }

  /** 数值近似比较，失败时把两个值都打出来（探针的价值就在这些数字上） */
  near(name: string, got: number | null | undefined, want: number, tol = 1.5): void {
    const ok = typeof got === 'number' && Math.abs(got - want) <= tol
    this.check(name, ok, `期望 ${want}±${tol}，实际 ${got}`)
  }
}

export const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms))

/** 轮询等待条件成立。加载层淡出这类「有动画时长」的断言必须用它，固定 sleep 会假失败。 */
export const waitUntil = async (cond: () => Promise<boolean>, ms: number): Promise<boolean> => {
  const t0 = Date.now()
  for (;;) {
    if (await cond()) return true
    if (Date.now() - t0 > ms) return false
    await sleep(120)
  }
}

/**
 * 标题栏样式的浅 / 深两套背景色（`#eef0f2` / `#1f2530`，跟随 `prefers-color-scheme`）。
 * 断言只能「落在已知配色里」，不能写死浅色 —— 否则深色系统上必然误报。
 */
export const BAR_BG = ['rgb(238, 240, 242)', 'rgb(31, 37, 48)']

/**
 * 进程是否还活着。
 * `kill(pid, 0)` 不发送任何信号，只做存在性检查 —— Windows 上 Node 用 OpenProcess 实现。
 * 用来断言「sidecar 被 stop() 之后真的退出了」，而不是留下一个仍在监听端口的孤儿。
 */
export function isAlive(pid: number): boolean {
  if (!pid) return false
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

/** M3（系统集成）断言用的真实入口 —— 与设置窗 / 托盘 / 快捷键走的**完全同一批函数**。 */
export interface M3ProbeHooks {
  /** 运行期唯一那份配置文件的落点（设置 + `window` 段的窗口几何同住一个文件） */
  storeLoc: StoreLocation
  getSettings: () => Settings
  /** 走真实保存路径（校验 → 应用副作用 → 持久化），抛错即失败 */
  saveSettings: (next: Settings) => void
  openSettingsWindow: () => void
  closeSettingsWindow: () => void
  getSettingsWindow: () => BrowserWindow | null
  applyWakeHotkey: (h: Hotkey) => RegisterResult
  releaseShortcuts: () => void
  registerSettingsShortcut: (fn: () => void) => boolean
  toggleMainWindow: () => void
  hasTray: () => boolean
  destroyTray: () => void
  trayIconPath: () => string | null
}

export interface ProbeContext {
  win: BrowserWindow
  base: string
  /** 夹具站点的观测点（M2 断言要看上游实际收到了什么） */
  seen: ProbeServer['seen']
  /** 启动 sidecar（本地反向代理 / DNS 解析） */
  startSidecar: (req: SidecarRequest) => Promise<SidecarHandle>
  /** M3 系统集成的入口 */
  m3: M3ProbeHooks
}
