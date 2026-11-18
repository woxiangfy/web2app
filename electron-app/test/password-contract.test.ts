/**
 * Electron 账号密码保存的**装配契约**（2026-11-18）。
 *
 * 背景：Electron 不含 Chrome 的密码管理器（详见 `src/main/credentials-logic.ts` 文件头），
 * 功能由 preload 隔离世界 + 主进程 `safeStorage` 存储自研。两处最易漂移：
 *   ① `preload.ts` 与 `tab-preload.ts` 里那份逐字复制的密码管理段（沙箱 preload 不能
 *      相对 require 共享模块，只能靠本测试锁一致）；
 *   ② 主进程通道 / sender 闸门 / 清除浏览数据接线（漏一处就是静默失效）。
 *
 * 运行期行为由 `probe` 第 18 节（真实键入 + 自动填充）负责，本文件不能替代它。
 */
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from 'node:test'

const root = process.cwd()
const read = (p: string): string => readFileSync(join(root, p), 'utf8')
const stripComments = (src: string): string =>
  src.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '')

const mainPreload = read('src/renderer/preload.ts')
const tabPreload = read('src/renderer/tab-preload.ts')
const appIpc = stripComments(read('src/main/app-ipc.ts'))
const indexTs = stripComments(read('src/main/index.ts'))
const webData = stripComments(read('src/main/web-data.ts'))
const tabsTs = stripComments(read('src/main/tabs.ts'))
const credentialsTs = stripComments(read('src/main/credentials.ts'))

/** 抽出两个 preload 里的密码管理段（含首尾标记行，二者必须逐字一致）。 */
function passwordBlock(src: string, name: string): string {
  const begin = src.indexOf('// ---- password-manager:begin')
  const end = src.indexOf('// ---- password-manager:end')
  assert.ok(begin > 0, `${name} 缺少 password-manager:begin 标记`)
  assert.ok(end > begin, `${name} 缺少 password-manager:end 标记`)
  // 两个文件的换行符历史不同（preload.ts 是 CRLF、tab-preload.ts 是 LF），比较前归一。
  return src.slice(begin, end).replace(/\r\n/g, '\n')
}

test('两个 preload 的密码管理段逐字一致（改一侧必须同步另一侧）', () => {
  assert.equal(
    passwordBlock(mainPreload, 'preload.ts'),
    passwordBlock(tabPreload, 'tab-preload.ts'),
    'preload.ts 与 tab-preload.ts 的密码管理段已漂移'
  )
})

test('两个 preload 都真正调用 installPasswordManager()', () => {
  assert.ok(
    stripComments(mainPreload).includes('installPasswordManager()'),
    '单窗口 preload 必须装配密码管理'
  )
  assert.ok(
    stripComments(tabPreload).includes('installPasswordManager()'),
    '页签内容页 preload 必须装配密码管理（否则页签模式下功能恒不可用）'
  )
})

test('密码管理段：只用三条白名单通道，且不向页面暴露任何桥', () => {
  const block = passwordBlock(mainPreload, 'preload.ts')
  for (const ch of ['w2a:pwd-lookup', 'w2a:pwd-save', 'w2a:pwd-forget']) {
    assert.ok(block.includes(`'${ch}'`), `缺少通道 ${ch}`)
  }
  assert.ok(
    !block.includes('exposeInMainWorld'),
    '密码管理不得暴露页面可见的桥：页面脚本不能主动读出已保存凭据'
  )
  assert.ok(!block.includes('console.log'), '密码段不得用 console.log（避免把凭据带进日志）')
})

test('密码管理段：只认可信 submit，且不阻止站点提交', () => {
  const block = passwordBlock(mainPreload, 'preload.ts')
  assert.ok(block.includes('ev.isTrusted'), '必须过滤程序化 submit，避免站点脚本刷库')
  assert.ok(
    block.includes('navigator.userActivation?.isActive'),
    'requestSubmit() 的 submit 仍是 isTrusted，必须再要一次用户激活'
  )
  assert.ok(
    !block.includes('preventDefault'),
    '不得 preventDefault：DOM 气泡活不过一次导航，阻止提交会改站点行为'
  )
})

test('主进程：三条通道都在 registerCredentialIpc 内，且有顶层框架 + 内容页 sender 闸门', () => {
  assert.ok(appIpc.includes('registerCredentialIpc(deps)'), '注册函数必须被 registerAppIpc 调用')
  for (const ch of ['w2a:pwd-lookup', 'w2a:pwd-save', 'w2a:pwd-forget']) {
    assert.ok(appIpc.includes(`'${ch}'`), `app-ipc.ts 缺少通道 ${ch}`)
  }
  assert.ok(appIpc.includes('e.senderFrame !== e.sender.mainFrame'), '必须拒绝子框架')
  assert.ok(appIpc.includes('deps.isContentSender(e.sender.id)'), '必须校验内容页 sender')
  assert.ok(
    appIpc.includes('deps.credentials.lookup(url)'),
    'origin 必须由主进程从 senderFrame.url 推导（渲染进程不得指定）'
  )
})

test('index.ts：装配凭据存储与内容页判据', () => {
  assert.ok(indexTs.includes('createCredentialStore(credentialsFile)'), '必须创建凭据存储')
  assert.ok(indexTs.includes('CREDENTIALS_FILE'), '凭据文件名必须来自 credentials-logic（单一真源）')
  assert.ok(indexTs.includes('isContentSender,'), 'registerAppIpc 必须注入内容页判据')
  assert.ok(indexTs.includes('credentials,'), 'registerAppIpc 必须注入凭据存储')
  assert.ok(indexTs.includes('tabManager?.ownsWebContents(id)'), '页签内容页必须被认作内容 sender')
  assert.ok(tabsTs.includes('ownsWebContents(id: number): boolean'), 'TabManager 必须提供 ownsWebContents')
})

test('清除浏览数据必须一并清掉已保存的账号密码', () => {
  assert.ok(webData.includes('deps.clearCredentials()'), 'web-data.ts 必须调用 clearCredentials')
  assert.ok(
    indexTs.includes('clearCredentials: () => credentials.clear()'),
    'index.ts 必须把凭据清理接进 clearWebData'
  )
})

test('凭据存储：只存 safeStorage 密文，拒绝在加密不可用时保存', () => {
  assert.ok(credentialsTs.includes('safeStorage.encryptString(password)'), '必须加密后再落盘')
  assert.ok(credentialsTs.includes('isEncryptionAvailable()'), '加密不可用时必须拒绝保存')
  assert.ok(
    !/writeFileSync\([^)]*password/.test(credentialsTs),
    '不得把明文密码直接写盘'
  )
})
