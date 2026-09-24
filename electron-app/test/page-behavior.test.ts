/**
 * `page-behavior.ts`（M4）纯函数单测 —— 零 Electron 依赖，直接跑 TS 源码。
 *
 * 为什么这些规则要脱离 GUI 覆盖：右键菜单「该出哪些项、哪些置灰」、F12「什么时候可用」
 * 都是典型的**错了也不报错**——菜单少一项没人会提 bug，F12 在打包版打不开只会让人以为是
 * 自己按错了。靠探针能验「事件挂上了」，但验不了「规则对不对」，所以放这里钉住。
 *
 * 运行：`pnpm run unit`
 */
import assert from 'node:assert/strict'
import { join } from 'node:path'
import { test } from 'node:test'
import {
  contextMenuTemplate,
  devtoolsEnabled,
  isDevtoolsKey,
  saveDialogOptions,
  type MenuItemSpec,
} from '../src/main/page-behavior.ts'

/** 构造一份「什么都没有」的右键参数（普通页面空白处）。 */
function blankParams() {
  return {
    isEditable: false,
    selectionText: '',
    linkURL: '',
    editFlags: { canCopy: false, canCut: false, canPaste: false },
  }
}

const NO_NAV = { canGoBack: false, canGoForward: false }
const FULL_NAV = { canGoBack: true, canGoForward: true }

function labels(items: MenuItemSpec[]): (string | null)[] {
  return items.map((i) => i.label)
}

// ---------- devtoolsEnabled：对齐 Tauri 的「release 不开」 ----------

test('devtoolsEnabled：未打包恒开（等价 cargo debug）；打包版只看 debug 配置', () => {
  assert.equal(devtoolsEnabled(false, false), true, '未打包 = debug 构建，不看配置')
  assert.equal(devtoolsEnabled(false, true), true)
  assert.equal(devtoolsEnabled(true, false), false, '打包版默认关（对齐 Tauri release）')
  assert.equal(devtoolsEnabled(true, true), true, '打包版 debug=true 才开')
})

// ---------- isDevtoolsKey：只认 F12 的 keyDown ----------

test('isDevtoolsKey：只认 F12 的 keyDown（keyUp 不算，否则一次按键切两遍）', () => {
  assert.equal(isDevtoolsKey({ type: 'keyDown', key: 'F12' }), true)
  assert.equal(isDevtoolsKey({ type: 'keyUp', key: 'F12' }), false)
  assert.equal(isDevtoolsKey({ type: 'keyDown', key: 'F11' }), false)
  assert.equal(isDevtoolsKey({ type: 'keyDown' }), false)
  assert.equal(isDevtoolsKey(null), false)
  assert.equal(isDevtoolsKey(undefined), false)
})

// ---------- 右键菜单：对齐 Tauri 的裁剪（不给打印 / 另存为 / 更多工具） ----------

test('右键菜单：不含打印 / 另存为 / 更多工具（与 Tauri 的裁剪口径一致）', () => {
  const ls = labels(contextMenuTemplate(blankParams(), FULL_NAV)).filter(Boolean) as string[]
  for (const banned of ['打印', '另存为', '保存', '更多工具']) {
    assert.ok(!ls.includes(banned), `不该出现「${banned}」`)
  }
})

test('右键菜单：基础项齐备，且「设置」在最后（与 Shift+F1 同一入口）', () => {
  const items = contextMenuTemplate(blankParams(), FULL_NAV)
  const ls = labels(items).filter(Boolean) as string[]
  for (const want of ['返回', '前进', '重新加载', '复制', '剪切', '粘贴', '全选', '设置']) {
    assert.ok(ls.includes(want), `缺少「${want}」`)
  }
  assert.equal(ls[ls.length - 1], '设置', '「设置」应压在最后一项')
})

test('右键菜单：无链接时不给「复制链接地址」', () => {
  const ls = labels(contextMenuTemplate(blankParams(), FULL_NAV))
  assert.ok(!ls.includes('复制链接地址'))
  const withLink = labels(contextMenuTemplate({ ...blankParams(), linkURL: 'https://a.test/x' }, FULL_NAV))
  assert.ok(withLink.includes('复制链接地址'))
})

test('右键菜单：首尾与连续处不留空分隔线', () => {
  for (const params of [blankParams(), { ...blankParams(), linkURL: 'https://a.test/x' }]) {
    const items = contextMenuTemplate(params, FULL_NAV)
    assert.notEqual(items[0].label, null, '首项不能是分隔线')
    assert.notEqual(items[items.length - 1].label, null, '末项不能是分隔线')
    for (let i = 1; i < items.length; i++) {
      assert.ok(
        !(items[i].label === null && items[i - 1].label === null),
        `第 ${i} 项出现连续分隔线`
      )
    }
  }
})

test('右键菜单：导航不可用时返回/前进置灰（而不是隐藏）', () => {
  const items = contextMenuTemplate(blankParams(), NO_NAV)
  const back = items.find((i) => i.label === '返回')
  const fwd = items.find((i) => i.label === '前进')
  assert.equal(back?.enabled, false)
  assert.equal(fwd?.enabled, false)
  // 置灰项仍然在位：突然少一项比灰着更难用
  assert.ok(back && fwd)
})

test('右键菜单：复制放行「有选中文本」（editFlags 不及时更新的兜底）', () => {
  const copy = contextMenuTemplate(
    { ...blankParams(), selectionText: '选中的字' },
    FULL_NAV,
  ).find((i) => i.label === '复制')
  assert.equal(copy?.enabled, true, '有选中文本时复制应可用')
})

test('右键菜单：粘贴只在可编辑处可用（canPaste 且 isEditable）', () => {
  const paste = (p: ReturnType<typeof blankParams>) =>
    contextMenuTemplate(p, FULL_NAV).find((i) => i.label === '粘贴')?.enabled
  assert.equal(
    paste({ ...blankParams(), isEditable: true, editFlags: { canCopy: false, canCut: false, canPaste: true } }),
    true,
  )
  assert.equal(
    paste({ ...blankParams(), isEditable: false, editFlags: { canCopy: false, canCut: false, canPaste: true } }),
    false,
    '不可编辑处粘贴应置灰',
  )
})

// ---------- saveDialogOptions：对齐 save_dialog.rs ----------

test('saveDialogOptions：默认落点 = 下载目录 + 站点给的文件名', () => {
  // ⚠️ 期望值必须用 join 构造：Windows 上 join 产出反斜杠，写死 'D:/dl/a.zip' 会假红
  assert.equal(saveDialogOptions('a.zip', 'D:/dl').defaultPath, join('D:/dl', 'a.zip'))
})

test('saveDialogOptions：空文件名兜底为 download（空输入框比 "download" 更难用）', () => {
  assert.equal(saveDialogOptions('', 'D:/dl').defaultPath, join('D:/dl', 'download'))
  assert.equal(saveDialogOptions('   ', 'D:/dl').defaultPath, join('D:/dl', 'download'))
})

test('saveDialogOptions：取不到下载目录时不拼路径（退化成只有文件名）', () => {
  assert.equal(saveDialogOptions('a.zip', '').defaultPath, 'a.zip')
})

test('saveDialogOptions：按后缀收敛过滤器；无后缀时不给过滤器', () => {
  const z = saveDialogOptions('setup.exe', 'D:/dl')
  assert.deepEqual(z.filters, [{ name: 'exe 文件 (*.exe)', extensions: ['exe'] }])
  assert.equal(saveDialogOptions('noext', 'D:/dl').filters, undefined)
})

test('saveDialogOptions：开覆盖确认（对齐 save_dialog.rs 的 FOS_OVERWRITEPROMPT）', () => {
  const props = saveDialogOptions('a.zip', 'D:/dl').properties ?? []
  assert.ok(props.includes('showOverwriteConfirmation'), '同名文件必须弹覆盖确认')
})
