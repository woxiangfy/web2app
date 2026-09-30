/**
 * 启动页（splash://）模板渲染 —— 从 index.ts 抽出的纯渲染单元，与 Tauri 侧
 * `webview-app/src/loading.rs`（启动页拼装）同职责、同契约。
 *
 * 本模块不持有 Electron 依赖：标题与注入脚本内容全部由调用方经 deps 传入，
 * 因此「占位符替换 / 失败态烘焙」这条最容易静默坏掉的链路可以脱离窗口被调用
 * （冒烟自检 `--smoke` 就是直接 render() 做断言）。
 *
 * ⚠️⚠️ 占位符替换必须用 split/join 而不是 String.replace —— 注入的是 titlebar.js /
 *    loading.js 全文，其中含 `$&`、`$'` 这类序列，在 replace 的**替换串**里是特殊模式，
 *    会把注入内容改坏（表现为「脚本里少了一段、页面报奇怪的语法错」）。Rust 侧同理。
 *
 * ⚠️ 失败态为什么直接写进启动页 HTML 而不是走 IPC：启动页把加载层内联在**主世界**，
 *    preload 的 IPC 通道在**隔离世界**里（`window.__web2appLoading` 读不到）。启动页是
 *    我们自己生成的 HTML，把失败态烤进去最简单也最可靠；加载层挂载完成后会自己取走
 *    （见 loading.js 的 applyPendingFail）。真实页面上的失败走另一条路
 *    （IPC → preload → 同世界的加载层）。
 */

/** 启动页背景色（= bootstrap::SPLASH_BG(0xf7,0xf8,0xfa)）。createWindow 的
 * backgroundColor 也用它盖住 WebView 初始化瞬间的白闪，两处必须同源。 */
export const SPLASH_BG = '#f7f8fa'

/** 启动页阶段的失败态：烘焙进启动页 HTML（见文件头说明）。 */
export interface PendingFail {
  title: string
  detail: string
}

export interface SplashPageDeps {
  /** 窗口标题（config.json 的 title，注入 HTML 前做转义） */
  title: string
  /**
   * 注入资源读取（splash.html / titlebar.js / loading.js）。
   * 单一真源在 webview-app/src，由 index.ts 的 readInject 负责定位 + 缓存。
   */
  readInject: (name: string) => string | null
}

export interface SplashPage {
  /** 渲染最终下发给 splash:// 的 HTML（含失败态，若有）。 */
  render(): string
  /** 设置 / 清除烘焙进下一帧启动页的失败态。 */
  setPendingFail(fail: PendingFail | null): void
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

/** 转成可安全嵌进 JS 字符串字面量的双引号串（含两侧引号），对齐 Rust 侧 `js_str`。 */
function jsStr(s: string): string {
  let out = '"'
  for (const ch of s) {
    switch (ch) {
      case '\\': out += '\\\\'; break
      case '"': out += '\\"'; break
      case '\n': out += '\\n'; break
      case '\r': out += '\\r'; break
      case '\u2028': out += '\\u2028'; break
      case '\u2029': out += '\\u2029'; break
      case '<': out += '\\u003c'; break
      default: out += ch
    }
  }
  return out + '"'
}

const FALLBACK_SPLASH = `<!doctype html><meta charset="utf-8">
<style>html,body{margin:0;height:100%;background:${SPLASH_BG}}
.d{display:flex;height:100%;align-items:center;justify-content:center;
font:14px/1.6 system-ui,sans-serif;color:#5f5e5a}</style><div class="d">加载中…</div>`

/**
 * 构造一个启动页渲染器（状态只有 pendingFail）。
 *
 * 工厂函数而不是模块级单例：状态归属显式、可替换 readInject（探针 / 测试），
 * index.ts 持有唯一实例。
 */
export function createSplashPage(deps: SplashPageDeps): SplashPage {
  let pendingFail: PendingFail | null = null

  function render(): string {
    const tpl = deps.readInject('splash.html')
    if (!tpl) {
      console.warn('[web2app] 未找到 splash.html，使用内置占位启动页')
      return FALLBACK_SPLASH
    }
    let html = tpl
      .split('__WEB2APP_TITLE__').join(escapeHtml(deps.title))
      .split('__WEB2APP_TITLEBAR_JS__').join(deps.readInject('titlebar.js') ?? '')
      .split('__WEB2APP_LOADING_JS__').join(deps.readInject('loading.js') ?? '')

    if (pendingFail) {
      // ⚠️⚠️ 这里同样**必须**用 split/join（与上方占位符替换同一条理由）：
      //    detail 里含失败 URL（配置里的地址，或页面重定向过来的地址），其中完全可能出现
      //    `$&` / `` $` `` / `$'`；作为 `String.replace` 的**替换串**时它们是特殊模式，
      //    会把命中的 `</body>` 前后内容整段塞进这个 JS 字符串字面量 —— 启动页脚本直接语法错，
      //    用户看到的是「加载失败页什么都不显示」，比没有失败提示还糟。
      const failScript =
        `<script>try{window.__web2appPendingFail={title:${jsStr(pendingFail.title)},detail:${jsStr(pendingFail.detail)}};}catch(e){}</script>\n</body>`
      html = html.split('</body>').join(failScript)
    }
    return html
  }

  return {
    render,
    setPendingFail(fail) {
      pendingFail = fail
    },
  }
}
