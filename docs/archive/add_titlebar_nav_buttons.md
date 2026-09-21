# 为窗口标题栏加入"前进/后退"导航按钮（统一自绘无边框标题栏）

## Context（为什么做）

用户在当前应用（Rust + Tauri v2，WebView2 内嵌 Trilium）的窗口标题栏加入"前进/后退"，用来控制当前窗体对应 webview 的浏览器历史导航。

现状：
- **单窗口模式**（`TABS_ENABLED=false`，默认）：主窗口 "main" 使用**系统原生标题栏**（[lib.rs](file:///c:/dev/rust/web-app/webview-app/src/lib.rs) 的 `build_main_window`，未设 `decorations(false)`）。原生标题栏无法追加按钮。
- **多页签模式**（Windows，`TABS_ENABLED=true`）：宿主窗口 "host" 已是**无边框（`decorations(false)`）+ 自绘 38px 标签栏**（[shell.html](file:///c:/dev/rust/web-app/webview-app/src/shell.html)），tab 窗口也是无边框内容窗。

用户拍板的两点决策：
1. **统一自绘无边框标题栏**：单窗口主窗也改为无边框，绘制与多页签宿主一致的标题栏（前进/后退/标题/最小化/最大化/关闭全自绘）。为此单窗口主窗需要引入一条持续的顶部标题栏（因为主窗加载的是远程 Trilium 页，标题栏只能经 `initialization_script` 注入其文档——沿用 [loading.js](file:///c:/dev/rust/web-app/webview-app/src/loading.js) 的 document-created 挂载模式）。
2. **前进/后退按钮始终可用**：无可进退历史时点击为 no-op，不做置灰监听。

## 方案概览

目标行为：
- 单窗口：主窗顶部一条固定 38px 自绘标题栏，左侧「← 前进 / → 后退」，中间标题，右侧「— ▢ ✕」。拖动标题栏空白处可移动窗口，双击最大化。
- 多页签：宿主 shell 标签栏内追加「前进/后退」，作用于**当前激活 tab** 的 webview。
- 前进/后退统一走一个新 IPC 命令 `history_nav(direction)`，由 Rust 解析"当前窗体对应的 webview"后 `eval("window.history.back()/forward()")`。

## 改动清单

### 1. 新增 `webview-app/src/titlebar.js`（单窗口主窗注入脚本）
- `initialization_script` 注入到单窗口主窗的**每个文档**（document-created）。
- 复用 loading.js 手法：全程 try/catch、`if(window.__web2appTitlebar)return;` 防重复、ID 选择器 + `!important` 防宿主样式污染。
- 构建 `position: fixed; top:0; left:0; right:0; height:38px`（常量 `TITLEBAR_H=38`）的 `.app-titlebar`：
  - 整条 `-webkit-app-region: drag`（沿用 shell.html 的可拖动写法，wry/WebView2 已支持）。
  - 前进/后退按钮 `-webkit-app-region: no-drag`，点击 `invoke('history_nav', {direction:'back'|'forward'})`（用 `window.__TAURI__.core.invoke`）。
  - 标题文本 = `document.title`（Windows 下主窗被 `<title>` 覆盖，取远程页真实标题）。
  - 右侧最小化/最大化/关闭：`window.__TAURI__.window.getCurrentWindow().minimize() / toggleMaximize() / close()`。
  - 标题栏 `dblclick` → `toggleMaximize()`。
  - `z-index` 取极高位（≥ loading 的 2147483647），保证加载期间标题栏始终可点。
- **预留顶部空间**：注入 CSS `html,body{padding-top:38px!important}`，让远程页正文不被标题栏盖住。已知局限：远程页自带 `position:fixed` 顶栏时可能仍重叠，需在 Trilium 上实测确认。

### 2. `webview-app/src/lib.rs`
- `build_main_window` 中的主窗 builder 追加 `.decorations(false)`（单窗口改为无边框）。
- 追加 `.initialization_script(include_str!("titlebar.js"))`（仅单窗口主窗；多页签 host 用 shell 栏，不注入）。
- 新增并注册命令 `history_nav`，加进基础 `invoke_handler`（[lib.rs#L79](file:///c:/dev/rust/web-app/webview-app/src/lib.rs#L79) 那张列表，两模式共用）。实现：
  ```rust
  #[tauri::command]
  fn history_nav(app: tauri::AppHandle, direction: String) -> Result<(), String> {
      // 解析"当前窗体"对应的 webview：
      //   · 多页签(windows)：TabManager.active 指向的 tab 标签 → get_webview_window(tab label)
      //   · 否则：主窗 "main"
      // 然后 webview.eval(if dir=="back" { "window.history.back()" } else { "window.history.forward()" })
  }
  ```
  多页签下解析激活 tab 时，复用 `tabs::TabManager` 的 `tabs`/`active` 状态取 label。

### 3. `webview-app/src/tabs.rs`
- 无 Rust 侧改动（`history_nav` 归 lib.rs；激活 tab 解析内部复用 `TabManager`）。
- 仅确认 `TabManager.active`/`tabs` 字段可被 lib.rs 访问（`pub(crate)`，已是）。

### 4. `webview-app/src/shell.html`（多页签宿主标签栏）
- 在 `.bar` 最左追加「前进/后退」两个按钮（`-webkit-app-region: no-drag`），点击 `invoke("history_nav", {direction})`。
- 样式与现有 `.tab`/`.add` 一致（继承 `--muted`/`--text` 变量，深色模式自动适配），高度对齐 38px。

## 多窗口适配要点
- **单窗口**：标题栏经 titlebar.js 注入主窗每个文档，天然覆盖 splash→远程页整个生命周期。
- **多页签**：宿主 shell 栏加按钮控制激活 tab；tab 内容窗不进标题栏（内容独占）。
- **按钮语义按"当前窗体"**：单窗口=主窗；多页签=激活 tab，由 `history_nav` 统一裁决，两模式共用一条命令。

## 验证
1. `cargo test --lib`（新增用例可放 history_nav 的"方向→脚本"纯函数，若有抽出）。
2. `cargo build` 编译通过。
3. 实机验证（Windows）：
   - 单窗口模式：标题栏显示完整（前进/后退/标题/最小化/最大化/关闭），可拖动窗口、双击最大化、三个窗口按钮可用；Trilium 内前进/后退正确（SAP 历史 + 浏览器历史）；远程页顶栏不被标题栏覆盖。
   - 多页签模式：宿主栏出现前进/后退，控制当前激活 tab 的导航；tab 间切换后按钮作用于新激活 tab。
   - 深色/浅色主题下按钮观感正常。