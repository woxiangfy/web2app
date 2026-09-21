/**
 * 在任意宿主里干净地拉起 Electron（`pnpm run smoke` / `pnpm run probe` 都走这里）。
 *
 * 存在的唯一理由：**宿主环境可能注入了 `ELECTRON_RUN_AS_NODE`**。
 *
 * 有些桌面型 IDE / agent 沙箱会给所有子进程注入 `ELECTRON_RUN_AS_NODE=1`
 *（配套 `NODE_OPTIONS=--require <语言服务 shim>`），目的是让 `node` 走它们自己的语言服务。
 * 一旦这个变量泄漏到 Electron，Electron 会**以纯 Node 身份启动**：
 *   - `require('electron')` 返回的是可执行文件路径【字符串】而不是 API 对象；
 *   - 于是 `app.disableHardwareAcceleration()` 之类的语句抛出
 *     `TypeError: Cannot read properties of undefined (reading '...')`，
 *     而错误位置在业务代码里，看上去像是我们自己的 bug；
 *   - `electron.exe --version` 会打印 Node 的版本（如 `v24.21.0`）而不是 `v44.x.y`——
 *     这是判断「是否掉进 Node 模式」最快的一招。
 * 更隐蔽的是它**时好时坏**：不同宿主、不同调用方式注入与否不一致，很容易被当成随机故障。
 *
 * ⚠️ 两个坑里的坑（排查时都踩过，别再走回头路）：
 *   1. 别用 `env -u ELECTRON_RUN_AS_NODE ...`：`env(1)` 会重建进程环境，把宿主用来
 *      捕获 stdout 的管道一起切断 —— 表现为「命令退出码 0、但一个字都没有」，
 *      于是你会去怀疑参数、怀疑代码，而真正的问题是**看不见输出**。
 *      改用 bash 内置 `unset`，或像本文件一样在 Node 里 `delete env.X`。
 *   2. `NODE_OPTIONS` 不要一起清：那个 shim 正是宿主捕获输出的通道，清了同样变哑巴。
 *      本文件只删 `ELECTRON_RUN_AS_NODE`，其余环境原样透传。
 *
 * 另外：Windows 上 `electron.exe` 是 GUI 子系统程序，**stdout 需要重定向才看得到**
 *（`electron . --smoke > log.txt 2>&1`）。直接跑可能什么都刷不出来，别误判成没执行。
 */
const { spawn } = require('node:child_process')
const { join } = require('node:path')

// require('electron') 在纯 Node 下返回可执行文件路径（见 node_modules/electron/index.js）
const electronPath = require('electron')

const env = { ...process.env }
delete env.ELECTRON_RUN_AS_NODE

// 默认入口是本项目根目录（package.json 的 main）；额外参数原样透传。
const args = [join(__dirname, '..'), ...process.argv.slice(2)]

const child = spawn(electronPath, args, { stdio: 'inherit', env })
child.on('close', (code, signal) => {
  if (code === null) {
    console.error(`[launch] electron 被信号终止：${signal}`)
    process.exit(1)
  }
  process.exit(code)
})
