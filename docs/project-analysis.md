# web2app 项目全盘分析与优化建议

> 分析日期：2026-09-14 ｜ 现状核对：2026-09-17 ｜ 精简：2026-09-22
> 范围：`webview-app/`（含 3 个 crate、注入脚本、CI/打包工作流）。
> 后续新增的 `electron-app/`（Windows 绿色版壳）另见 `docs/electron-plan.md` / `docs/electron-analysis.md`。
> 约束：所有优化建议**不改变现有功能点**，仅做质量/性能/安全/可维护性层面的改进。

---

## 一、项目概览

| 维度 | 结论 |
|---|---|
| 定位 | Rust + Tauri 2 的跨平台 WebView 壳：嵌网址 + 托盘最小化 + 全局快捷键 + 代理/Basic 鉴权 |
| 目标平台 | Windows（WebView2，主战场）+ Android（交叉编译验证，未真机交付） |
| 架构 | cargo workspace 三 crate，职责边界清晰 |
| 代码规模 | 主 crate `src/` 19 个文件（含 4 个注入脚本）+ 3 个 crate + 5 条 CI/打包工作流 |
| 当前状态 | 单测全绿（resolve-core 13 + resolve-tool 7 + 主 crate 44 = 64 条）；双壳契约探针 112/112、多页签 23/23 |

### crate 职责

| crate | 职责 | 依赖 | 平台 |
|---|---|---|---|
| `web2app`（主） | Tauri GUI：建窗、托盘、快捷键、代理、多页签、注入 | Tauri 2 + axum + reqwest + hickory | Windows / Android |
| `resolve-core` | 纯加密核心：Ed25519 签名 + AES-256-GCM + HKDF 派生 | ed25519-dalek / aes-gcm / hkdf（**零平台依赖**） | 全平台 |
| `resolve-tool` | 服务端 CLI：生成密钥对 / 生成 DNS TXT 密文 | 仅 resolve-core（**不依赖 Tauri**） | 全平台（CI Linux） |
| `proxy-server` | Electron 侧 sidecar（bin 名 `w2a-proxy`），以 `#[path]` 复用主 crate 的 `proxy.rs`/`resolve.rs` | 同主 crate 代理依赖 | Windows |

---

## 二、模块职责地图

| 模块 | 职责 | 平台 | 关键设计/踩坑记录 |
|---|---|---|---|
| `config.rs` | 编译期 env 注入常量 + 目录名清洗 | 全平台 | `env_bool!`/`env_str!`/`str_eq` 绕 const 上下文限制；`sanitize_windows_name` 为目录名/文件名共用的清洗基元 |
| `bootstrap.rs` | 后台解析真实地址 → 就绪后导航 | 全平台 | 耗时操作挪后台线程，先建窗显示启动页；`Navigation` 缓存供多页签复用 |
| `proxy.rs` | 本地反向代理（HTTP + WS 统一注入 Basic 鉴权） | 全平台 | 流式转发、透传 Host/Content-Length、Set-Cookie Domain 重写、WS Origin 重写 |
| `resolve.rs` | DNS TXT 查询 + 验签解密层 | 全平台 | 单 Resolver 复用；多域名 failover；re-export resolve-core |
| `loading.rs` + `loading.js` | 启动过渡（splash + overlay 加载层） | 全平台 | 两段接力同一状态机；`whenHostReady` 等 DOM 宿主，失败轮询重试 |
| `titlebar.js` | 自绘窗口标题栏（前进/后退/最小化/关闭） | Windows 单窗 | 首个文档拿不到 initialization_script → 内联进 splash.html |
| `webview_hooks.rs` | WebView2 原生事件：鉴权/新窗口/右键/下载 | Windows | `HOOKS_REGISTERED` 去重闸门；后台线程轮询 with_webview |
| `save_dialog.rs` | 下载时弹 Windows 原生「另存为」对话框（`IFileSaveDialog`） | Windows | 复用系统对话框，自带覆盖确认 / 新建文件夹 / 最近位置记忆 |
| `tabs.rs` | 多页签（宿主窗口 + shell 标签栏 + tab 窗口） | Windows | owned window 防遮挡；`place_tab` 闭环校准；`destroy()` 防重入；`MAX_TABS` 熔断 |
| `tray.rs` | 系统托盘 + 窗口切换 | 桌面端 | 显隐同步多页签 |
| `window_state.rs` | 窗口几何持久化 | 桌面端 | 失效几何自动最大化；物理像素 |
| `settings.rs` | 设置持久化 + 设置窗口 | 全平台 | 快捷键/托盘副作用锁外执行；Android 不取 parent 目录 |
| `dnd_polyfill.js` | WebView2 拖拽 bug polyfill | Windows | 鼠标事件合成完整 HTML5 DnD 序列（**效果待真机验证，见「遗留问题」**） |
| `tab_title.js` | 多页签真实标题回传 | Windows | MutationObserver + 轮询兜底 |

---

## 三、优化建议落地情况

> 优先级：P0=建议尽快 / P1=值得做 / P2=可选。风险：低/中/高。所有项均不改变对外行为。
> 状态为 2026-09-17 逐条核对结果（✅ 已落地 / ⬜ 未落地 / ➖ 有意不采纳）。

### 3.1 性能 / 可维护性 / 安全 / CI（全部已核对）

| # | 位置 | 建议 | 状态 |
|---|---|---|---|
| 1 | `proxy.rs` WS 二进制帧 | `Bytes::from(b.to_vec())` 多余堆拷贝 → 直接 `A::Binary(b)` | ✅ 已落地（`proxy.rs:438`）+ 单测 |
| 2 | `proxy.rs` 诊断日志 | `status==200` 且非 PDF 时短路，避免每次构造 `dbg_ct` | ✅ 已落地（`proxy.rs:236-256`） |
| 3 | `bootstrap.rs` `strip_userinfo` | clone 代价很小 | ➖ 有意不采纳（收益低于改动风险） |
| 4 | `sanitize_dir_name` vs `sanitize_filename` | 提取共享 `sanitize_windows_name(name, collapse_ws)` | ✅ 已落地（`config.rs:88`，`webview_hooks.rs:158` 调用） |
| 5 | `loading.js` / `titlebar.js` 的 `whenHostReady` 重复 ~70 行 | 构建期拼接或 CI 断言一致性 | ⬜ 未落地（仍两份各自维护；等待逻辑已重构为轮询重试） |
| 6 | `resolve-tool` 手写 args 解析重复 3 份 | 归并统一 `parse_args` | ✅ 已落地（`main.rs:65`） |
| 7 | 公钥解析重复 | 下沉到 `resolve-core::parse_signing_key_pem`，两端复用 | ✅ 已落地（`lib.rs:232`） |
| 8 | `resolve-core` `SECRET` 缺省回落公开默认值 | 打包阶段强制校验非空非默认，否则 fail-fast | ✅ 已落地（`build-custom.yml`） |
| 9 | `EXTRA` 原样内联 | 注入前 `serde_json::from_str::<Value>` 校验，非法回落 `"{}"` | ✅ 已落地（`config.rs:154`） |
| 10 | `ci.yml` fmt 用 `continue-on-error` | 改为阻断（显式 pin rustfmt 组件版本） | ✅ 已落地 |
| 11 | Android 作业不跑 clippy | 增加 `cargo clippy --target aarch64-linux-android` 并阻断 | ✅ 已落地 |
| 12 | 一次性 3s 重试未存 handle | 改为轮询 `tick` + `POLL_MS` | ✅ 已落地 |
| 13 | `resolve.rs` `Err(_e)` 注释掉变量 | 改 `Err(_)` 避免 lint 隐患 | ⬜ 未落地（`resolve.rs:96`，纯洁癖） |

### 3.2 功能 / UI 建议（涉及功能，未纳入「不改变功能点」范围，供排期）

| 优先级 | 建议 | 状态 |
|---|---|---|
| ~~P1~~ | 标签栏恢复「新建页签」入口 | 已确认有意移除（新 tab 改由页面内 `target="_blank"` / `window.open()` 触发），不再排期 |
| P1 | 窗口标题同步页面 title | 单窗口模式自绘标题栏已跟随 `document.title`；**原生窗口标题**仍固定编译期常量 `TITLE`（`lib.rs:208`，2026-09-22 由 `WINDOW_TITLE` 改名）。**复核：仍未落地** |
| P2 | 标签页右键菜单 | 关闭其他 / 关闭右侧 / 刷新 / 复制链接 |
| P2 | 标签页会话持久化 | 重启恢复上次打开的 tabs（`window_state` 已有持久化基建） |
| P2 | 下载进度 / 取消 | 下载已走原生「另存为」对话框且可取消；**进度 UI 仍未做** |
| P2 | 多上游白名单 | 代理模式下跨域外链「直接打开但不注入凭据」；可扩展「上游白名单 + 按 Host 注入」 |
| P3 | 更新提醒 / 版本展示 | 托盘/右键菜单增加「关于」项 |
| P1 | 设置页展示「当前入口/代理模式」只读信息 | **复核：设置页仍无相关字段，未落地** |
| P2 | 加载层品牌化 | `loading.js` 文案/底色做成可配置 |
| P2 | 深色模式统一 | 已适配 `prefers-color-scheme`；可进一步显式下发 `preferred_color_scheme` |
| P2 | 标签栏溢出交互 | 当前仅 `overflow-x:auto`；可加横向滚动/左右箭头/下拉列表 |
| P3 | 首次引导 | splash 叠加「Shift+F1 打开设置」轻提示 |

---

## 四、遗留问题（需真机复验，未自动化覆盖）

1. **Trilium note-tree 拖拽（Windows）仍异常**：`dnd_polyfill.js` 合成 HTML5 DnD 序列对 fancytree 无效（多轮方案无变化，尚未根因，见 `.workbuddy/memory/project_memory.md`）。
2. **「代理 / 域名解析」真机项**：sidecar 链路的打包态真机验证留待后续（探针 M2 断言已在源码态与打包态跑通）。

---

## 五、结论

整体工程质量**很高**：架构分层清晰、平台隔离规范、防御性设计与「踩坑即注释」的沉淀习惯突出，核心加密链路有完整的往返/篡改/跨语言交叉测试。

**13 项优化建议（截至 2026-09-17）：10 项已落地 / 2 项未落地（#5 脚本合并、#13 纯洁癖）/ 1 项有意不采纳（#3）。**
**建议优先排期的功能项**：原生窗口标题同步页面 title、设置页展示入口/代理信息；「新建页签入口」已确认移除。
