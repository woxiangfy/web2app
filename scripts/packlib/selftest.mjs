/**
 * 契约自测（不需要 Python）。
 * PARITY_PAYLOADS 同时供 parity.mjs 使用 —— `--selftest` 与 `--parity` 共用同一批 payload。
 */
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { ConfigError, captureOutput, emit } from './diagnostic.mjs'
import { buildMerged, toConfigJson } from './parser.mjs'
import { getCloseMatches } from './pycompat.mjs'
import { DEFAULTS } from './constants.mjs'
import { ELECTRON_OUT_DEFAULT, TAURI_OUT_DEFAULT, isDeleteBlocked, resolveElectronOut, resolvePackOut, safeName } from './util.mjs'
import { ELECTRON, ROOT, WEBVIEW } from './paths.mjs'

function assertThat(cond, msg) {
  if (!cond) throw new Error(`断言失败：${msg}`)
}

function expectError(data, needle) {
  try {
    buildMerged(data)
  } catch (e) {
    if (e instanceof ConfigError) {
      assertThat(String(e.message).includes(needle), `报错文案里应包含 ${JSON.stringify(needle)}，实际是：${e.message}`)
      return
    }
    throw e
  }
  throw new Error(`应当报错但通过了：${JSON.stringify(data)}（期望提到 ${JSON.stringify(needle)}）`)
}

/** 把两侧都要跑的一批 payload 抽出来，`--selftest` 与 `--parity` 共用同一份。 */
export const PARITY_PAYLOADS = [
  { app_url: 'https://example.com' },
  { app_url: 'https://a.com', title: '我的应用' },
  { app_url: 'https://a.com', app_name: 'MyApp', title: '' },
  { app_url: 'https://a.com', proxy_enabled: true },
  { app_url: 'https://a.com', tabs_enabled: true, debug: true },
  { app_url: 'https://a.com', proxy_bypass: '' },
  { app_url: 'https://a.com', data_dir: '' },
  { app_url: 'https://a.com', data_dir: '   ' },
  { app_url: 'https://a.com', data_dir: ' mydata ' },
  { app_url: 'https://a.com', data_dir: 'D:\\appdata' },
  { app_url: 'https://a.com', data_dir: 'bad<name' },
  { app_url: 'https://a.com', extra: { k: 'v', n: 1 } },
  { app_url: 'https://a.com', extra: [1, 2] },
  { app_url: 'https://a.com', resolve_secret: ' s ' },
  { app_url: 'https://a.com', icon_url: 'https://x.com/i.png' },
  { app_url: 'https://a.com', icon_url: 'not-a-url' },
  { app_url: 'https://a.com', app_name: 123 },
  { app_url: 'https://a.com', title: '   ' },
  { app_url: ['a.com', 'b.com'], public_key: 'AAAA' },
  { app_url: ['a.com'], public_key: 'AAAA', resolve_secret: 'secret' },
  { app_url: '["a.com"]', public_key: 'AAAA' },
  { app_url: 'https://user:pass@a.com/Path' },
  { app_url: 'ftp://a.com' },
  { app_url: 'https://a.com', android_keystore_base64: 'eHh4' },
  { app_url: 'https://a.com', version: '2.3.4' },
  { app_url: 'https://a.com', version: ' 255.255.65535 ' },
  // ---- 以下以「必须报错」为主（`app_url: 123` 例外：它只产生 warning，仍会成功）----
  { app_url: 'https://a.com', debug: 'true' },
  { app_url: 'https://a.com', proxy_enabled: 'on' },
  { app_url: 'https://a.com', proxy_enabled: 1 },
  { app_url: 'https://a.com', proxy_enabled: null },
  { app_url: 'https://a.com', version: '1.0' },
  { app_url: 'https://a.com', version: 1 },
  { app_url: ['a.com'] },
  { app_url: [] },
  { app_url: '   ' },
  { app_url: '[a.com]' },
  { app_url: 123 },
  { app_url: true },
  { app_url: 'https://a.com', data_dir: true },
  { app_url: 'https://a.com', data_dir: 123 },
  { app_url: 'https://a.com', window_title: 'X' },
  { app_url: 'https://a.com', data_dir_default: false },
  { app_url: 'https://a.com', proxy_enable: true },
]

export function selftest() {
  // 自测会把 buildMerged 跑很多遍，其中的 ::warning:: 提示会刷屏；这里整体静音，
  // 只有断言失败才需要看到输出（断言异常照常抛出）。
  const { error } = captureOutput(() => {
    // 最小配置：只有 app_url
    let m = buildMerged({ app_url: 'https://example.com' })
    assertThat(m.app_url === 'https://example.com', 'app_url 原样')
    assertThat(m.app_name === 'Web2App', 'app_name 默认')
    assertThat(m.title === 'Web2App', 'title 空 → 回落 app_name')
    assertThat(m.proxy_enabled === 'False' && m.tabs_enabled === 'False', '布尔默认 False')
    assertThat(m.debug === 'False', 'debug 默认必须关闭（默认开启＝往用户数据目录里塞日志文件）')
    assertThat(m.mobile_bar === 'False', 'mobile_bar 默认必须关闭（默认＝无移动端底栏）')
    assertThat(m.data_dir === 'data', 'data_dir 默认是相对路径 data')
    assertThat(m.proxy_bypass === 'localhost;127.0.0.1;<-loopback>', 'proxy_bypass 默认串')

    assertThat(buildMerged({ app_url: 'https://a.com', title: '我的应用' }).title === '我的应用', 'title 显式给出不回落')

    // version：默认 0.0.1；只认三段数字（Windows MSI/VERSIONINFO 规则）
    assertThat(buildMerged({ app_url: 'https://a.com' }).version === '0.0.1', 'version 默认 0.0.1')
    assertThat(buildMerged({ app_url: 'https://a.com', version: ' 1.2.3 ' }).version === '1.2.3', 'version 去空白')
    assertThat(
      buildMerged({ app_url: 'https://a.com', version: '255.255.65535' }).version === '255.255.65535',
      'version 边界值 255.255.65535',
    )
    for (const bad of ['1.2', 'v1.2.3', '1.2.3.4', '1.2.3-beta', '1.2.3+build']) {
      expectError({ app_url: 'https://a.com', version: bad }, '三段数字版本号')
    }
    for (const bad of ['256.0.0', '1.256.0', '0.0.65536']) {
      expectError({ app_url: 'https://a.com', version: bad }, '超出 Windows 版本范围')
    }
    // 数字先经通用通道 str() 成 "1" ⇒ 格式校验；bool 不被 str ⇒ 「必须是字符串」
    expectError({ app_url: 'https://a.com', version: 1 }, '三段数字版本号')
    expectError({ app_url: 'https://a.com', version: true }, '必须是字符串')
    assertThat(buildMerged({ app_url: 'https://a.com', version: null }).version === '0.0.1', 'version 显式 null 回落默认')

    assertThat(buildMerged({ app_url: 'https://a.com', proxy_enabled: true }).proxy_enabled === 'True', '真 true 通过')
    assertThat(buildMerged({ app_url: 'https://a.com', debug: true }).debug === 'True', 'debug 真 true 通过')
    expectError({ app_url: 'https://a.com', debug: 'true' }, '必须是布尔值')
    assertThat(
      buildMerged({ app_url: 'https://a.com', mobile_bar: true }).mobile_bar === 'True',
      'mobile_bar 真 true 通过',
    )
    expectError({ app_url: 'https://a.com', mobile_bar: 'true' }, '必须是布尔值')
    for (const bad of ['true', 'false', '1', 'yes', 'on', '']) {
      expectError({ app_url: 'https://a.com', proxy_enabled: bad }, '必须是布尔值')
    }

    // app_url
    m = buildMerged({ app_url: ['a.com', 'b.com'], public_key: 'AAAA' })
    assertThat(JSON.parse(m.app_url).join(',') === 'a.com,b.com', '数组保持数组')
    expectError({ app_url: ['a.com'] }, '必须提供 public_key')
    expectError({ app_url: [] }, '不能为空')
    expectError({ app_url: '   ' }, '缺少必填字段 app_url')
    m = buildMerged({ app_url: '["a.com"]', public_key: 'AAAA' })
    assertThat(JSON.parse(m.app_url).join(',') === 'a.com', 'JSON 数组文本老写法仍可用')
    expectError({ app_url: '[a.com]' }, '不是合法的 JSON 字符串数组')
    // Python 的 str() 语义：数组里的非字符串元素会走 "True"/"None"/repr
    m = buildMerged({ app_url: [123, true, null, ' x '], public_key: 'AAAA' })
    assertThat(JSON.parse(m.app_url).join('|') === '123|True|None|x', 'str() 语义与 Python 一致')

    // 已移除 / 未知字段
    expectError({ app_url: 'https://a.com', window_title: 'X' }, '已移除')
    expectError({ app_url: 'https://a.com', proxy_enable: true }, 'proxy_enabled')

    // data_dir
    assertThat(buildMerged({ app_url: 'https://a.com' }).data_dir === 'data', 'data_dir 默认')
    assertThat(buildMerged({ app_url: 'https://a.com', data_dir: '' }).data_dir === '', '空串必须保留语义')
    assertThat(buildMerged({ app_url: 'https://a.com', data_dir: '   ' }).data_dir === '', '纯空白视同空串')
    assertThat(buildMerged({ app_url: 'https://a.com', data_dir: ' mydata ' }).data_dir === 'mydata', '相对路径去空白')
    assertThat(buildMerged({ app_url: 'https://a.com', data_dir: 'D:\\appdata' }).data_dir === 'D:\\appdata', '绝对路径原样')
    expectError({ app_url: 'https://a.com', data_dir: true }, '必须是字符串')
    expectError({ app_url: 'https://a.com', data_dir: 123 }, '必须是字符串')
    expectError({ app_url: 'https://a.com', data_dir_default: false }, '已移除')
    expectError({ app_url: 'https://a.com', data_dir_default: false }, '"data_dir": "data"')

    // extra
    assertThat(buildMerged({ app_url: 'https://a.com', extra: { k: 'v' } }).extra === '{"k":"v"}', 'extra 序列化')
    assertThat(buildMerged({ app_url: 'https://a.com' }).extra === '{}', 'extra 默认 {}')

    assertThat(buildMerged({ app_url: 'https://a.com', resolve_secret: ' s ' }).resolve_secret === 's', 'secret 去空白')

    // resolve_secret 提示**只在域名数组模式**出现
    let out = captureOutput(() => buildMerged({ app_url: 'https://a.com' })).lines.join('\n')
    assertThat(!out.includes('::warning::'), `单 URL 模式不该产生 warning：${out}`)
    assertThat(out.includes('resolve_secret') && out.includes('不参与'), '应说明该字段当前不参与')
    out = captureOutput(() => buildMerged({ app_url: ['a.com'], public_key: 'AAAA' })).lines.join('\n')
    assertThat(out.includes('::warning::resolve_secret 未配置'), '域名数组模式必须告警')
    out = captureOutput(() => buildMerged({ app_url: '["a.com"]', public_key: 'AAAA' })).lines.join('\n')
    assertThat(out.includes('::warning::resolve_secret 未配置'), 'JSON 数组文本模式也必须告警')
    for (const payload of [
      { app_url: 'https://a.com', resolve_secret: 's' },
      { app_url: ['a.com'], public_key: 'AAAA', resolve_secret: 's' },
    ]) {
      out = captureOutput(() => buildMerged(payload)).lines.join('\n')
      assertThat(!out.includes('resolve_secret'), '配了 Secret 时两种模式都安静（连说明行也不出现）')
    }

    // toConfigJson：bool 保持真 bool、数组保持数组、android_* 不落盘
    const merged = buildMerged({
      app_url: ['a.com'],
      public_key: 'AAAA',
      tabs_enabled: true,
      mobile_bar: true,
      debug: true,
      data_dir: '',
      android_keystore_base64: 'eHh4',
    })
    const cfg = toConfigJson(merged)
    assertThat(cfg.tabs_enabled === true, 'config.json 里必须是真 bool')
    assertThat(cfg.mobile_bar === true, 'mobile_bar 在 config.json 里也必须是真 bool')
    assertThat(cfg.debug === true, 'debug 在 config.json 里也必须是真 bool')
    assertThat(Array.isArray(cfg.app_url) && cfg.app_url[0] === 'a.com', 'config.json 里数组要保持数组')
    assertThat(!('android_keystore_base64' in cfg), 'android_* 不应进 Windows 包的 config.json')
    assertThat(cfg.data_dir === '', '空串必须原样落到 config.json（不被改成默认值）')

    // 安装名清洗（与 CI 逐字一致）
    assertThat(safeName('a/b:c*d') === 'a_b_c_d', '非法字符替换')
    assertThat(safeName('  x  y ') === 'x y', '压空白 + 去首尾')
    assertThat(safeName('') === 'Web2App', '空名字回落')
    assertThat(safeName('.hidden.') === 'hidden', '去首尾句点')

    // 相似度算法必须与 difflib 一致（--parity 无法覆盖「建议哪个名字」，因为那是纯函数）
    assertThat(getCloseMatches('proxy_enable', Object.keys(DEFAULTS), 1, 0.5)[0] === 'proxy_enabled', '近似名建议')
    assertThat(getCloseMatches('windo_title', Object.keys(DEFAULTS), 1, 0.5)[0] === 'title', 'windo_title → title')

    // --out 解析：相对路径锚定 electron-app/；../tests/ 越界到仓库根 tests/ 是默认产物位置，必须放行
    assertThat(
      resolveElectronOut('release/local-x') === join(ELECTRON, 'release', 'local-x'),
      '相对 --out 以 electron-app/ 为基准',
    )
    assertThat(
      resolveElectronOut('../tests/pack') === join(ROOT, 'tests', 'pack'),
      '../tests/<名字> 解析到仓库根 tests/（本地打包默认产物根，已被 /tests/ 忽略）',
    )
    assertThat(
      resolveElectronOut(join(ROOT, 'tests', 'pack-local-x')) === join(ROOT, 'tests', 'pack-local-x'),
      '绝对路径原样使用',
    )
    assertThat(
      ELECTRON_OUT_DEFAULT === join(ROOT, 'tests', 'pack', 'electron'),
      'Electron 默认产物目录是 tests/pack/electron/（与 tests/pack/tauri/ 平级分子目录，绿色版同名混放会互相覆盖）',
    )

    // Tauri 侧 --out：相对路径锚定 webview-app/；默认产物目录必须落在 tests/pack/tauri（与 Electron 分子目录）
    assertThat(
      resolvePackOut('../tests/myapp', WEBVIEW) === join(ROOT, 'tests', 'myapp'),
      'Tauri 相对 --out 以 webview-app/ 为基准，../tests/<名字> 越界到仓库根 tests/',
    )
    assertThat(
      TAURI_OUT_DEFAULT === join(ROOT, 'tests', 'pack', 'tauri'),
      'Tauri 默认产物目录是 tests/pack/tauri/（两壳绿色版同名，混放会互相覆盖）',
    )

    // isDeleteBlocked：把失败分成「删不掉」与「真失败」两大类，判错的代价不对称 ——
    //   漏判 ⇒ 本该自动换目录重试的场景变成硬失败，且用户只看到一句「退出码 1」；
    //   误判 ⇒ 白跑一次重试 + 一句不对路的建议。
    // 2026-10-11 实测就是这么漏的：判据只认 safe-delete/emptyDir，而真实报错是
    // `EBUSY: resource busy or locked, unlink '…\win-unpacked\resources\app.asar'`
    // （WorkBuddy 宿主进程抓着新落盘的 *.asar 不放）⇒ 既不重试也不提示。
    const DELETED = (s) => isDeleteBlocked([s])
    // ① 必须认成「删不掉」—— 文件被别的进程占着（本次实测的原文）/ 权限拒删 / 删除垫片
    assertThat(
      DELETED(
        "⨯ EBUSY: resource busy or locked, unlink 'C:/dev/rust/web-app/tests/pack/electron/win-unpacked/resources/app.asar'  failedTask=build",
      ),
      'EBUSY unlink 必须算「删不掉」（否则不会自动换目录重试）',
    )
    assertThat(DELETED("EPERM: operation not permitted, unlink 'C:/x/a.dll'"), 'EPERM unlink 必须算')
    assertThat(DELETED("EACCES: permission denied, unlink 'C:/x/a.dll'"), 'EACCES unlink 必须算')
    assertThat(DELETED("EBUSY: resource busy or locked, rmdir 'C:/x/appdir'"), '目录 rmdir 被占同理')
    assertThat(DELETED('⨯ safe-delete: 需要确认才能删除'), 'safe-delete 标记仍算（原有行为不许丢）')
    assertThat(DELETED('⨯ emptyDir failed'), 'emptyDir 标记仍算（原有行为不许丢）')
    assertThat(DELETED('Error: trash operation not permitted'), 'trash operation 标记仍算（原有行为不许丢）')
    // ② 不许误判成「删不掉」
    assertThat(
      !DELETED("⨯ Timeout awaiting 'request' for 600000ms  failedTask=build"),
      '网络超时是环境问题的另一类，不能算「删不掉」',
    )
    assertThat(
      !DELETED("EPERM: operation not permitted, rename 'C:/a' -> 'C:/b'"),
      '裸 EPERM 但非删除类调用（受限网络下下工具集会这样失败）不能算 —— 否则白等一次重试还给出错误建议',
    )
    assertThat(
      !DELETED("EBUSY: resource busy or locked, open 'C:/x/a.dll'"),
      'EBUSY 但发生在 open（读不到，不是删不掉）不能算',
    )
    assertThat(!DELETED('✗ 配置字段未知：proxy_enable'), '配置类报错显然不算')

    // proxy_bypass 默认串是跨壳 / 跨打包期 / 跨示例的**单一字面量契约**
    //（完整清单与原因见 docs/packaging-config.md §4.4）。改默认串时漏掉任何一处都会让
    // 「没配 bypass 时两壳直连范围不同」之类的分叉悄悄溜过去 ⇒ --selftest 在这里逐文件钉住。
    const BYPASS = DEFAULTS.proxy_bypass
    // ① 运行期 / 打包期真正读取的默认值
    for (const f of [
      'webview-app/src/config.rs',
      'electron-app/src/main/config.ts',
      '.github/scripts/parse-packaging-config.py',
      'scripts/packlib/constants.mjs',
      'electron-app/scripts/stage-assets.cjs',
    ]) {
      assertThat(readFileSync(join(ROOT, f), 'utf8').includes(BYPASS), `proxy_bypass 默认串必须出现在 ${f}`)
    }
    // ② 两份入库示例：必须是**真 JSON 字段**且等于默认串（不只是文案提到）
    for (const f of ['webview-app/config.example.json', 'electron-app/config.example.json']) {
      assertThat(
        JSON.parse(readFileSync(join(ROOT, f), 'utf8')).proxy_bypass === BYPASS,
        `${f} 的 proxy_bypass 必须等于默认串`,
      )
    }
    // ③ 工作流模板 / 注释与根 `AGENTS.md` 示例片段中的同一字面量
    for (const f of [
      '.github/workflows/generate-signing-key.yml',
      '.github/workflows/build-custom.yml',
      'webview-app/AGENTS.md',
    ]) {
      assertThat(readFileSync(join(ROOT, f), 'utf8').includes(BYPASS), `proxy_bypass 默认串必须出现在 ${f}`)
    }
  })
  if (error) throw error
  emit('Node 版解析器自测全部通过 ✓')
  return 0
}
