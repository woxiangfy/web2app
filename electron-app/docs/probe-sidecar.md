# 探针、sidecar 与安全基线（electron-app）

> 自 `AGENTS.md` 拆出：`launch.cjs` 的必要性、本地代理 sidecar 的定位与进程间契约、
> 兼容契约探针的覆盖明细、渲染进程安全基线与占位符契约。

## ⚠️ 为什么必须经 `scripts/launch.cjs`

部分宿主环境（桌面型 IDE、agent 沙箱等）会给所有子进程注入 `ELECTRON_RUN_AS_NODE=1`
（配套还注入 `NODE_OPTIONS=--require <它们的语言服务 shim>`）。这个变量一旦泄漏进 Electron，
Electron 会**以纯 Node 身份启动**：

- `require('electron')` 返回的是**可执行文件路径字符串**，不是 API 对象；
- 于是业务代码第一句 `app.disableHardwareAcceleration()` 就抛
  `TypeError: Cannot read properties of undefined`，错误位置在**我们自己的代码里**，
  看起来完全像是本项目的 bug；
- `electron.exe --version` 会打印 Node 版本（`v24.x`）而不是 `v44.x` —— 这是判别「是否掉进
  Node 模式」最快的一招。

更麻烦的是它**时好时坏**（不同宿主 / 不同调用方式注入与否不一致），极易被误判为随机故障。

`scripts/launch.cjs` 在 spawn 前 `delete env.ELECTRON_RUN_AS_NODE`，其余环境原样透传。

> 排查时踩过的两个反向坑，别再走回头路：
> 1. **别用 `env -u ELECTRON_RUN_AS_NODE ...`**：`env(1)` 会重建进程环境，把宿主用来捕获
>    stdout 的管道一起切断，表现为「退出码 0 但一个字都没有」——于是你会去怀疑参数、怀疑代码，
>    而真正的问题是**看不见输出**。用 bash 内置 `unset`，或像 `launch.cjs` 那样在 Node 里处理。
> 2. **`NODE_OPTIONS` 不要一起清**：那个 shim 正是宿主捕获输出的通道，清了同样变哑巴。

另：Windows 上 `electron.exe` 是 GUI 子系统程序，**stdout 需要重定向才稳定可见**
（`... > log.txt 2>&1`）。直接跑可能什么都刷不出来，别误判成没执行。

## 本地代理 sidecar（`w2a-proxy.exe`）

Electron 侧有两块能力**不复用就会重开一遍验证**：

| 能力 | 为什么不能（也不该）用 JS 重写 |
|---|---|
| 本地反向代理 | 给**每一个**请求（含 WebSocket 握手）注入 `Authorization: Basic`。经过充分测试的 Rust：流式转发、Cookie Domain 重写、WS 双向隧道、一次性随机密钥门禁（无密钥一律 404） |
| 动态域名解析 | DNS TXT → Ed25519 验签 → AES-GCM 解密。Node 侧要引 Ed25519 + HKDF + AES-GCM 一整套，且必须与 `resolve-core` **字节级**一致 |

所以 `crates/proxy-server` 用 `#[path]` **直接引用** `webview-app/src/proxy.rs` 与 `src/resolve.rs`
（零重写、零复制），编出 `w2a-proxy.exe` 作为 sidecar。定位顺序见 `src/main/sidecar.ts::locateSidecar()`：

1. `process.resourcesPath/sidecar/w2a-proxy.exe`（打包后，由 `electron-builder.yml` 的 `extraResources` 放置）
2. `../webview-app/target/release/w2a-proxy.exe`（开发态，优先 release —— 代理是 CPU 敏感路径）
3. `../webview-app/target/debug/w2a-proxy.exe`

⚠️ **必须放 `extraResources`，绝不能进 asar**：asar 内的可执行文件无法被 `spawn`。

### 进程间契约

```
宿主 spawn → 传参 → sidecar 起代理 → stdout 打一行 JSON（宿主据此导航）→ 常驻转发
                                                    ↓
                              stdin EOF = 宿主已退出 ⇒ sidecar 自杀（防僵尸）
```

* **stdout 只给机器读**：成功/失败都打**且仅打一行** JSON；日志一律走 stderr（宿主并进自己的日志）。
  往 stdout 多打一行，宿主就会一直等握手 —— 表现是「窗口一直转圈、日志里什么都看不到」。
* **握手行必须 flush**（管道上是块缓冲，不 flush 宿主永远等不到）。
* **凭据走 stdin**（`--url-stdin`），不走命令行：Windows 上同用户的其它进程能通过
  `GetCommandLine` / WMI 读到命令行参数，而 URL 里内嵌的就是站点账号密码。
* **`--no-proxy`**（域名解析 + 直连）时 sidecar 只解析地址、不起代理，并在握手后立即退出；
  此时它会把凭据放在 `authUser` / `authPass` 字段里交给宿主（宿主用 Chromium 的 `login` 事件注入）。
  ⚠️ 这两个字段是**明文凭据**，任何地方都不得写日志。
* **握手成功行的 `proxyUrl` 自带一次性随机门禁密钥**（`http://127.0.0.1:端口<上游路径>?w2a_key=<43 字符>`；
  路径取自上游 URL，无子路径时即 `/`，与直连 / 外部代理模式落点一致）：
  宿主只把该入口 URL 交给窗口；无密钥 / 错密钥访问代理一律 **404**（防同机进程扫描借道），
  首访回种的 HttpOnly cookie 让页面根绝对路径资源与 WS 自动通过，密钥不会转发给上游。

手动调试：

```bash
cd ../webview-app && cargo build --release -p w2a-proxy
./target/release/w2a-proxy.exe --help
echo "https://example.com" | ./target/release/w2a-proxy.exe --url-stdin   # 看握手 JSON
```

## 兼容契约探针（`pnpm run probe`）

`src/main/probe.ts` 自己起一个 `127.0.0.1` 随机端口的夹具站点，逐个导航并断言 **150 条**契约，
**退出码即结论**（不通过则非 0，可以直接挂 CI）。夹具全部内联在代码里，不落盘、不进打包产物。

覆盖范围：启动页内联注入、标题栏几何与配色、38px 让位（fixed / sticky / `100vh` / `inset:0` 四种页面形态）、
严格 CSP 站点的样式降级、加载层淡出、`target="_blank"` 与 `window.open` 的原地导航、
最小化 / 最大化 / 关闭三个按钮真的驱动了原生窗口，以及 **sidecar 的代理契约**：

* 直连夹具（不带凭据）必须 **401** —— 先证明夹具真在鉴权，否则后面的断言是假通过；
* **门禁**：无密钥、错密钥访问代理都必须 **404**（不透露代理存在），带入口 query 密钥才放行；
* 经代理必须 200，且**上游**收到的 `Authorization` 与 `proxy.rs::build_basic_auth` 的格式逐字节一致；
* **WebSocket 握手也带上了 `Authorization`** —— 这是本地代理当初存在的全部理由
  （WebView2 的原生鉴权事件对 WS 不生效），也是「Electron 复用同一份 Rust 代理」值不值的唯一依据；
* WebSocket **双向**通信打通（上游收到的客户端消息 + 页面收到的服务端消息）；
* `Origin` 被改写为**上游源**（且只有一个）—— 详见下面那条已修的坑；
* `stop()` 之后 sidecar 进程**真的退出**（`kill(pid, 0)` 探活），不留孤儿。

M3 之后还覆盖**系统集成**（**61 条，即第 11–14 节**；第 15 节是 M4 页面行为 4 条；
第 16 节是**子框架桥梁隔离** 4 条——同源 iframe 拿不到 `web2app` / `web2appExtra`，
顶框仍正常暴露；第 17 节关闭按钮 1 条；v1.72 / v1.75 / v1.76 又陆续补入悬浮球真实窗口语义
（v1.75 为第 14b 段 +4 条）—— 单窗口断言总数因此由 138 走到 **150**，条数以实跑输出为准）：

* `Hotkey`（旧版存 JS `event.code`）→ Electron accelerator 的映射，含「不认识的键名返回 null，绝不猜」；
* `settings.json` 的**序列化字节形态**必须与 `serde_json` 写出来的一致（字段名 / 嵌套 / 布尔类型 /
  **键序**）—— 这是「与 Tauri 版可互相读取」的硬保证，反向由 `webview-app` 侧的
  `store::tests::parse_accepts_electron_written_document` 盯着；
* **设置与窗口几何同住一个文件**后，各写入方互不吃掉对方（`写几何不吃掉设置` /
  `写设置不吃掉窗口几何` / `float_window` 段在三个写入方下都不丢）—— 合并的唯一真风险，每个方向都钉住；
* 悬浮球：`float-layout.ts` 纯几何单测 + `float-contract.test.ts` 静态契约（七通道 IPC 名一致、
  sender 校验、`thickFrame:false` 与透明窗属性、屏幕绝对坐标拖动协议、只 loadFile 本地页、
  stage 登记 floaticon.html、`win.setShape(...)` 必须存在**且必须包 try/catch**——
  第一版裸 `includes` 曾被注释行骗过，改为先剥注释再匹配）；明细见 [float-ball.md](float-ball.md)；
* 存储位置规则：写入一律 `<data_dir>\settings.json`（= 数据目录，与 `data_dir` 字段一致），
  读取候选为「数据目录 → `%APPDATA%\<app_name>\`」（后者的 `window` 段也会被一并搬到新位置）；
* 窗口几何的**DIP ↔ 物理像素**换算（跨 DPI 不错位）与四种恢复策略（含「位置在屏外 → 最大化」）；
* 托盘：启用后图标出现、关闭后图标销毁；
* 全局快捷键：注册 / 不支持的键名明确报错 / 清空注销 三条路径；
* 设置窗：`settings://localhost/` 能打开、垫片就位、**点保存的完整链路**（页面 → IPC → 副作用 → 落盘）、
  未授权命令被拒、能关闭；
* 「关闭到托盘」：关窗后窗口未销毁且被隐藏，再切换能恢复。

> ⚠️ 探针**不碰用户真实配置**：它把 `settings.json`（唯一那份，设置 + 窗口几何）整体重定向到
> `%TEMP%\w2a-probe-store\`，跑 `pnpm run probe` 不会改掉你机器上的托盘开关和窗口尺寸。

**注入了 `titlebar.js` / `loading.js`、改过 `proxy.rs`、或动过设置/托盘/快捷键链路之后必须跑这个** ——
前两个是本壳与 Tauri 版共用的单一真源，`proxy.rs` 现在也是，设置页 HTML 同理。
探针是唯一能在改动后立刻发现「另一个壳被带坏」的手段。

> ⚠️ 探针抓到的第一个**真 bug（Tauri 版同样中招）**：代理里「先透传所有头、后面再设一次 Origin
> 来覆盖」的写法是无效的 —— `http::request::Builder::header` 与 `reqwest::RequestBuilder::header`
> 都是 **append**（`HeaderMap::append`）而不是 insert。结果是上游收到**两个** Origin
> （`http://127.0.0.1:代理端口, http://127.0.0.1:上游端口`），比不改写更糟：
> 任何做 Origin 校验的后端都会拒连，表现为「页面能开，但 WS / POST 全挂」。
> 修法是在透传时就跳过 `origin`，并有 `proxy_sends_exactly_one_rewritten_origin` 盯着。

> 无 GPU 的环境（容器 / 部分远程桌面）里 Chromium 会以 `GPU process isn't usable. Goodbye.` 退出。
> 探针模式**已自动**关闭硬件加速，但**这还不够** —— 2026-09-17 实测：GPU 子进程仍会反复崩
> （`exit_code=1`），最后一条 `FATAL` 直接把整个进程带走，退出码是 `-2147483645`（异常码，不是 1）。
> 这种环境必须再显式加 `--no-sandbox --in-process-gpu`（写法见 `AGENTS.md`「命令」下 pnpm 一节）。
> 真机不要长期关硬件加速（滚动与动画会明显变卡）。

## 安全基线

渲染进程加载的是**远程页面**，`webPreferences` 保持全默认值：`contextIsolation: true` /
`nodeIntegration: false` / `sandbox: true` / `webviewTag: false`。
页面只能通过 `contextBridge` 暴露的白名单 `window.web2app`（`isElectron` / `chromeH` /
`minimize` / `toggleMaximize` / `close`）与壳交互，拿不到 `ipcRenderer`、`require`、`process`。

注入脚本经 **`webFrame.executeJavaScript` 在主世界**执行（不是隔离世界）—— 原因与踩坑记录见
`src/renderer/preload.ts` 顶部注释。

## 占位符契约（已加固）

`splash.html` 用三个占位符承载运行时数据：`__WEB2APP_TITLE__`、`__WEB2APP_TITLEBAR_JS__`、
`__WEB2APP_LOADING_JS__`，由 `loading.rs`（Tauri）与 `main/index.ts`（Electron）各自替换。

⚠️ 曾经的两个隐患，都已有回归测试盯着：
1. 占位符曾被包在 `/* … */` 注释里，而 `titlebar.js` 的头注释本身就含数十处 `*/`
   （脚本持续在增长，具体数字/行号会漂移，勿再写死）—— 外层注释随时会**提前闭合**。
   现已改为**裸 token**（`<script>__WEB2APP_TITLEBAR_JS__</script>`），并有
   `splash_script_tokens_are_not_comment_wrapped` 等测试。
2. 替换必须用 `split/join`，**绝不能用 `String.replace`** —— 替换串（即整个 JS 源码）里的
   `$&`、`$'` 会被当作替换模式，直接把脚本改坏。
