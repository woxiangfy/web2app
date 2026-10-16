# 自绘标题栏与页面滚动条（`html` 不滚、**`body` 滚**）

> 本文是 electron-app「标题栏 / 滚动条」契约的唯一详细记录：为什么换滚动根、代价与边界、判据。
> 速览见 [../AGENTS.md](../AGENTS.md)「自绘标题栏与页面滚动条」一节的指针；取证脚本用法同本文。

**一句话结论**：注入的标题栏不改滚动条宽度，而是**换滚动根** —— `html{height:100%;overflow:hidden}`
不滚，`body{margin-top:38px;height:calc(100vh - 38px);overflow-y:auto}` 滚。
页面滚动的矩形从 38px 开始，滚动条只在 body 盒子内绘制，**物理上不可能越到标题栏上**。
（使用者原话：「滚动条不应该强制隐藏，只是不要超过菜单栏而已」。）

## 为什么只能这么做（2026-09-17 逐像素 + 对照实验查明）

**Chromium 把视口（根）滚动条绘制在所有 DOM 内容之上**，`z-index:2147483647` 也压不住；
而且它的几何恒为「视口右缘 × **整个**视口高」，没有任何 CSS 能把它往下缩。于是只要由 `html`
来滚，滚动条必然从 y=0 开始画、横穿标题栏，还顺带盖住最右侧的关闭按钮
（`elementFromPoint(innerWidth-1, 20)` 返回 `null` = 那里点不动）。

标题栏的 `getBoundingClientRect().width` 一直是 `innerWidth`（828，断言全绿），但右端那 15px 在
**绘制**阶段被滚动条覆盖 —— **rect 只证明布局，不证明可见**。此前两轮误判都源于把「现状」当期望值：
第一版断言写 `barRect.w == clientWidth` 并把那条缝注释成「WebView 的正常行为」，第二版写
`rect.width == innerWidth`。

实测（Electron 44 / 828×438 窗口 / 经典 15px 滚动条 / dpr=2，`pnpm run lab` 四模式对照）：

| 配置 | 标题栏那 38px 内 | 结论 |
|---|---|---|
| 原生（`html` 滚） | 全是滚动条像素 | ✗ 横穿标题栏 |
| `--enable-features=OverlayScrollbar` | 同左 | ✗ 该开关在 Electron 44 **已失效** |
| `--enable-features=FluentOverlayScrollbar` | 同左 | ✗ 同上 |
| `::-webkit-scrollbar{border-top:38px}` | thumb 仍从 y=0 起 | ✗ border 不参与滚动条零件布局 |
| `::-webkit-scrollbar-track{border-top:38px}` | 同上 | ✗ 同上 |
| `scrollbar-width:none` | 干净 | ✗ 滚动条也没了 —— 使用者明确否掉 |
| **`html` 不滚 + `body` 滚** | **y0..y32 = 纯标题栏色** | **✓ 唯一可行** |

## 代价与边界（都有断言或兜底）

- **`window` 级滚动 API** 会指向一个不再滚动的 `documentElement` ⇒ `shimWindowScroll()` 转发：
  `scrollY/pageYOffset/scrollX/pageXOffset`（getter → body）、`scrollTo/scroll/scrollBy`、
  body 的 `scroll` 事件补发到 `window` + `document`、`Document.prototype.scrollingElement` → body、
  `<html>` 实例上的 `scrollTop/scrollLeft` 遮蔽转发（只给这一个实例挂，不碰 `Element.prototype`）。
  懒加载 / 吸顶导航 / 回到顶部 / 阅读进度全靠这组 API。
- **`applyChromeFit` 不再动页面自己的 `position:sticky`**：body 已是滚动容器，`top:0` 的 sticky
  天然停在 y=38；再给它加 38 就成了 76px（吸顶导航凭空多一条空白）。`fixed` 仍照旧下移
  —— 它锚的是视口，不是滚动盒。
- **页面用 `!important` 压掉契约**会变成「html 不滚 + body 也不滚 = 页面钉死滚不动」，
  比原缺陷严重得多 ⇒ `ensureScrollRoot()` 探测（html `hidden` + body `auto` + body 高度 ≈ 视口-38，
  第三条专门抓 `body{height:auto!important}` 这种只查 overflow 抓不到的），失败即退回
  「html 顶部留白」旧模式并 `console.error` 说明原因。
  ⚠️ 它**必须在样式表挂上之后**跑：早一步时 computed style 还是页面原样，必然误判成失败、
  把退路写进 inline style（`!important` 优先级最高）—— 新契约就永久起不来了。
- `width:100vw` 保留：标题栏铺满整窗宽（含滚动条本该占的那一条），JS 再 `syncBarWidth()` 钉一次 px。
  加载层遮罩 `100vw/100vh` 同理。

## 盯住它的一组判据

| 判据 | 期望 | 说明 |
|---|---|---|
| `intrudeRows` | `[]` | x=`innerWidth-1` 逐点 y=0..37 必须**全部命中标题栏** ← 直接对应「滚动条别超过菜单栏」 |
| `bodyScrollbarW` | > 0（本机 15） | 滚动条**仍然存在**，且是占宽的那种，归 body 管 |
| `bodyScrollable` | `true` | 页面照常能滚 |
| `window.scrollTo(0,200)` | `scrollY≈200` **且** scroll 事件触发 | 转发没坏（事件异步，要等一帧再读） |

`pnpm run probe`（150 条）与 `pnpm run shot` 都是这套判据；`pnpm run shot` 另给 PNG 与 `INFO` 明细。
⚠️ **不要**再用 `rect.width == innerWidth` 或「滚动条宽度归零」当判据 —— 前者测不出覆盖，
后者把缺陷锁死成期望值。**改完必须重新打包**：脚本随 asar 分发在 `out/inject/titlebar.js` 里
（2026-09-26 起不再放 `resources/inject/`），跑旧的 exe 永远看不到修复。

> 排查这类「断言过、人眼不对」的问题请用 `scripts/diag/scrollbar-lab.cjs`（多模式对照，几种做法一次跑完）
> 与 `scripts/diag/png-inspect.cjs`（把 PNG 读成结构图 + 逐像素扫描）。两者都零依赖，
> 专治沙箱里没有图片库、`Add-Type` 被安全策略拦的情况。
