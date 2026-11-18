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

test('密码管理段：只用四条白名单通道，且不向页面暴露任何桥', () => {
  const block = passwordBlock(mainPreload, 'preload.ts')
  for (const ch of ['w2a:pwd-lookup', 'w2a:pwd-stage', 'w2a:pwd-commit', 'w2a:pwd-forget']) {
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

test('自动填充：URL 变化 / DOM 变动都立即触发，全程事件驱动不常驻轮询', () => {
  const block = passwordBlock(mainPreload, 'preload.ts')
  assert.ok(
    block.includes("['pushState', 'replaceState']"),
    'pushState / replaceState 不派发事件，必须在主世界打 history 钩子'
  )
  assert.ok(
    block.includes("'__web2app:url-change'"),
    '主世界钩子必须通过 DOM 事件通知隔离世界'
  )
  assert.ok(block.includes('__web2appUrlHook'), '钩子必须幂等（同一文档只装一次）')
  assert.ok(block.includes("addEventListener('popstate'"), '必须监听 popstate')
  assert.ok(block.includes("addEventListener('hashchange'"), '必须监听 hashchange')
  assert.ok(
    !block.includes('setInterval('),
    '不得常驻轮询：URL 变化必须靠事件驱动（性能取舍，2026-11-18 二次调整）'
  )
  assert.ok(block.includes('MutationObserver'), '必须观察 DOM 变动（登录框常在后渲染）')
  assert.ok(
    block.includes('observer.observe(document,'),
    '必须观察 document：preload 阶段 documentElement 可能为 null，observe(null) 会抛错导致观察器失效'
  )
  assert.ok(
    !block.includes('observer.observe(document.documentElement'),
    '不得观察 documentElement（2026-11-18 真机复现：整个观察器失效，只剩 focusin）'
  )
  assert.ok(
    block.includes("attributeFilter: ['class', 'style', 'hidden']"),
    '必须观察显隐相关属性：display:none → block 不产生 childList'
  )
  assert.ok(
    block.includes('filledPass.isConnected'),
    '填过的密码框被整块替换后必须允许对新表单再填一次'
  )
  assert.ok(
    block.includes('typedUser.toLowerCase()'),
    '站点预填其他账号时不得把保存的密码灌进去'
  )
  assert.ok(
    block.includes('syncUrl()'),
    'URL 变化必须重置 fillTried / 缓存，否则同一文档换路由后不会重新填充'
  )
})

test('气泡文案：填充有提示、保存的按钮是「删除」而不是「撤销」', () => {
  const block = passwordBlock(mainPreload, 'preload.ts')
  assert.ok(block.includes('密码已自动填充'), '自动填充后必须有提示气泡')
  assert.ok(block.includes("label: '删除'"), '保存气泡的按钮文案必须是「删除」')
  assert.ok(
    !block.includes("textContent = '撤销'"),
    '「撤销」语义不清，已统一改为「删除」'
  )
})

test('主进程：四条通道都在 registerCredentialIpc 内，且有顶层框架 + 内容页 sender 闸门', () => {
  assert.ok(appIpc.includes('registerCredentialIpc(deps)'), '注册函数必须被 registerAppIpc 调用')
  for (const ch of ['w2a:pwd-lookup', 'w2a:pwd-stage', 'w2a:pwd-commit', 'w2a:pwd-forget']) {
    assert.ok(appIpc.includes(`'${ch}'`), `app-ipc.ts 缺少通道 ${ch}`)
  }
  assert.ok(!appIpc.includes("'w2a:pwd-save'"), '提交即落盘的旧通道必须删除')
  assert.ok(appIpc.includes('e.senderFrame !== e.sender.mainFrame'), '必须拒绝子框架')
  assert.ok(appIpc.includes('deps.isContentSender(e.sender.id)'), '必须校验内容页 sender')
  assert.ok(
    appIpc.includes('deps.credentials.lookup(url)'),
    'origin 必须由主进程从 senderFrame.url 推导（渲染进程不得指定）'
  )
})

test('提交只暂存：登录成功确认后才落盘，失败不覆盖旧密码', () => {
  assert.ok(appIpc.includes('new Map<number, StagedCredential>()'), '暂存必须只存在主进程内存')
  assert.ok(appIpc.includes('STAGE_TTL_MS'), '暂存必须有 TTL，不能无限期留内存')
  assert.ok(
    appIpc.indexOf('if (hasPasswordField)') > 0 &&
      appIpc.indexOf('if (hasPasswordField)') < appIpc.indexOf('deps.credentials.save(s.origin'),
    '必须先在「仍有密码框」时丢弃暂存，再谈落盘'
  )
  const block = passwordBlock(mainPreload, 'preload.ts')
  assert.ok(block.includes("invoke('w2a:pwd-stage'"), '提交只能暂存')
  assert.ok(block.includes("invoke('w2a:pwd-commit'"), '必须在页面稳定后确认落盘')
  assert.ok(!block.includes("invoke('w2a:pwd-save'"), 'preload 不得再直连写盘通道')
  assert.ok(block.includes('登录成功后自动保存账号密码'), '提交后必须提示「登录成功后自动保存」')
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

test('凭据存储：只用异步 safeStorage 密文，拒绝在加密不可用时保存', () => {
  assert.ok(credentialsTs.includes('encryptStringAsync(password)'), '必须异步加密后再落盘')
  assert.ok(
    credentialsTs.includes('isAsyncEncryptionAvailable()'),
    '加密不可用时必须拒绝保存（Electron 45 起同步版已废弃）'
  )
  assert.ok(!credentialsTs.includes('safeStorage.encryptString('), '不得再用废弃的同步加密 API')
  assert.ok(!credentialsTs.includes('safeStorage.decryptString('), '不得再用废弃的同步解密 API')
  assert.ok(
    !/writeFileSync\([^)]*password/.test(credentialsTs),
    '不得把明文密码直接写盘'
  )
})
