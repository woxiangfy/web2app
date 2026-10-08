/**
 * Electron 打包分支。
 * 配置在**打包期**被读（`electron-app/config.json` → `out/config.json` → 进 `app.asar`）。
 */
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { FatalError, emit } from './diagnostic.mjs'
import { ELECTRON, ROOT, WEBVIEW, rel } from './paths.mjs'
import { toConfigJson } from './parser.mjs'
import {
  ELECTRON_OUT_DEFAULT,
  have,
  isDeleteBlocked,
  pngToIco,
  resolveBuilderBinariesMirror,
  resolveElectronOut,
  run,
  runTee,
  stamp,
} from './util.mjs'

export async function doElectron(args, merged, env, name, rr) {
  if (!have(['pnpm', '--version'])) {
    throw new FatalError('未找到 pnpm。Electron 侧本地打包需要 pnpm（见 electron-app/package.json）')
  }
  const dry = args.dryRun

  // 1) 落一份**规范化后**的 config.json —— 本地手工改配置最容易做错的一步：
  //    布尔写成字符串 "false"、app_url 数组写成字符串，都是「能出包但行为不对」。
  const cfgPath = join(ELECTRON, 'config.json')
  if (dry) {
    emit(`\n[1/4] 将写出规范化配置 → ${rel(cfgPath)}（原文件备份，打包后还原）`)
  } else {
    rr.file(cfgPath)
    // ⚠️ 一律 LF：与 CI（Linux）的 Python 产物一致；`writeFileSync` 不做换行转换。
    writeFileSync(cfgPath, JSON.stringify(toConfigJson(merged), null, 2) + '\n', { encoding: 'utf8' })
    emit(`\n[1/4] 已写出规范化配置 → ${rel(cfgPath)}（原文件已备份，打包后还原）`)
  }

  // 2) 图标 —— **判据只看 JSON**：`icon_url` 非空就下载并应用，与 CI 逐字一致
  //    （CI 那步的条件正是 `if [ -z "${ICON_URL:-}" ]; then … exit 0; fi`，没有任何额外开关）。
  //    ⚠️ 早先这里写的是 `args.icon && merged.icon_url` ⇒ 同一份 JSON 在 CI 会带图标、本地不带，
  //       一把「配置说了算」的事变成「命令行说了算」。要显式跳过就用 `--no-icon`（逃逸口，不是默认路径）。
  let extra = []
  if (merged.icon_url && !args.noIcon) {
    const url = merged.icon_url
    // ⚠️ 必须先建 build/：它被 .gitignore 忽略、fresh checkout 里不存在，
    //    而 curl 写不进去时报的是 `curl: (23) client returned ERROR on write`
    //    —— 看着像网络问题，换 URL 毫无用处（2026-09-22 实测）。
    const buildDir = join(ELECTRON, 'build')
    const dest = join(buildDir, 'icon.png')
    if (dry) {
      emit(`\n[2/4] 将下载图标 ${url} → ${rel(dest)}，并本地转换 PNG→ICO`)
    } else {
      mkdirSync(buildDir, { recursive: true })
      run(['curl', '-fsSL', '--retry', '3', url, '-o', dest], ELECTRON, env, false)
      const png = readFileSync(dest)
      const magic = png.subarray(0, 8).toString('hex')
      if (magic !== '89504e470d0a1a0a') throw new FatalError(`icon_url 下载的不是 PNG（文件头=${magic}）`)
      // 托盘图标取 resources/icons/icon.png（由 extraResources 从 webview-app/icons/ 拷进包），
      // 只设 -c.win.icon 对托盘无效 ⇒ 必须同步覆盖那一份。它是 tracked，所以先备份。
      const tray = join(WEBVIEW, 'icons', 'icon.png')
      rr.file(tray)
      copyFileSync(dest, tray)
      emit(`托盘图标已同步 → ${rel(tray)}（打包后还原）`)
      // exe 图标必须给 .ico：给 PNG 会逼 electron-builder 运行时从 GitHub 下载 icons-bundle
      // 转换（受限网络必败，见 pngToIco 注释）。icons/icon.ico 是 tracked，stage-assets 会把它
      // 铺到 build/icon.ico；与 icon.png 同样先备份、打包后还原。
      const ico = join(WEBVIEW, 'icons', 'icon.ico')
      rr.file(ico)
      writeFileSync(ico, pngToIco(png))
      emit(`exe 图标已本地生成 PNG→ICO → ${rel(ico)}（免 GitHub 下载，打包后还原）`)
    }
    extra = ['-c.win.icon=build/icon.ico']
  } else {
    const why = !merged.icon_url ? '配置里 icon_url 为空' : '--no-icon 显式跳过'
    emit(`\n[2/4] 跳过图标：${why}，沿用仓库默认图标`)
  }

  // 3) sidecar + 编译 + 铺资源
  //    ⚠️⚠️ 顺序关键：`w2a-proxy.exe` 必须**在 RESOLVE_SECRET 存在的同一环境里**编译。
  //    resolve-core 的 Secret 是 `option_env!("RESOLVE_SECRET")` 的编译期常量，
  //    先编后设 ⇒ sidecar 内置公开默认 Secret，用户配的 resolve_secret **静默失效**。
  emit('\n[3/4] 编译 sidecar（继承 RESOLVE_SECRET）→ 编译 TS → 铺打包资源')
  run(['pnpm', 'run', 'sidecar'], ELECTRON, env, dry)
  run(['pnpm', 'run', 'build'], ELECTRON, env, dry)
  run(['pnpm', 'run', 'stage'], ELECTRON, env, dry)

  // 4) 出包
  const forms = []
  if (args.target === 'electron-zip' || args.target === 'electron-all') forms.push('zip')
  if (args.target === 'electron-portable' || args.target === 'electron-all') forms.push('portable')
  // ⚠️ nsis 刻意**不并入 electron-all**：打安装版必须带 distribution=nsis 标记（见
  // builderCmd），而该标记会合并进同次构建的 app.asar，若与 zip/portable 同次构建，
  // 绿色产物会被一起误标成安装版（数据目录被强制到 %APPDATA%）。要全出就分别跑两次。
  if (args.target === 'electron-nsis') forms.push('nsis')
  emit(`\n[4/4] electron-builder：${forms.join(' + ')}`)

  // ⚠️ 受限网络下 electron-builder 会**自己去下载** Electron 运行时（223MB）并卡到超时：
  //       ⨯ Timeout awaiting 'request' for 600000ms  failedTask=build
  //    这是环境问题、不是配置或代码问题 —— `node_modules/electron/dist` 里其实已经有一份。
  //    ⚠️ 必须写长写法 `--config.electronDist=`；`-c.electronDist=` 在部分版本上会被当成
  //       配置文件路径而报 ENOENT。
  const extraBuilder = []
  const localDist = join(ELECTRON, 'node_modules', 'electron', 'dist')
  if (existsSync(localDist)) {
    extraBuilder.push('--config.electronDist=node_modules/electron/dist')
    emit(`    使用本地 Electron 运行时（跳过下载）：${rel(localDist)}`)
  }
  // NSIS（portable/nsis 目标）等工具集是 electron-builder 另一条按需下载链，GitHub 不通时
  // 自动换镜像（见 resolveBuilderBinariesMirror）；必须在构造 builder 命令前改好 env。
  await resolveBuilderBinariesMirror(env, dry)

  // 输出目录：本地打包默认落到**仓库根的 `tests/pack/electron/`**（与 Tauri 的 tests/pack/tauri/
  // 平级分子目录：两壳绿色版都叫 <name>.exe，混放互相覆盖）。根 `AGENTS.md` §3.2 约定 tests/ 只装
  // 本地测试过程的临时/中间产物，根 `.gitignore` 的 `/tests/` 整目录忽略 ⇒ 150MB 量级、上千文件的
  // win-unpacked/ 树**不会进 git、不污染版本库**。这与 CI 不冲突：CI 工作区一次性，
  // electron-builder.yml 的 directories.output=release 只在 CI / 直接跑 pnpm dist 时生效；本地一律用
  // 下面的绝对路径覆盖。
  //
  // ⚠️⚠️ 这里**刻意不自己清空输出目录**，而是让 electron-builder 去清（它本来就会做，CI 走的也是这条路）。
  //    「脚本自己先清一遍」的写法两处都吃亏（2026-09-24 实测）：
  //      · 本机的删除有**配额**（`SAFE_DELETE_BULK_CONFIRM_REQUIRED`，50 次/轮）：自己清一次上千个
  //        文件就把配额烧光 ⇒ electron-builder 随后连自己的中间产物 `*.nsis.7z` 都删不掉，
  //        zip / portable **明明已经产出**，构建却以「失败」收尾；
  //      · 清空等于**先毁掉上一次的产物**，万一这次构建失败，用户两手空空。
  //    ⇒ 先让它试；只有**它自己**因删不掉而失败时，才自动换一个全新目录重试一次。
  //      全新目录没有东西可清 ⇒ emptyDir 无事可做 ⇒ 必然能出包；人工依旧不需要做任何事。
  // ⚠️ 显式给的相对 `--out` 以 **electron-app/** 为基准（electron-builder 就在那下面跑）；
  //    想落到根 tests/ 写 `--out ../tests/<名字>`，或直接给绝对路径。
  const defaultOut = ELECTRON_OUT_DEFAULT
  let outDir = args.out ? resolveElectronOut(args.out) : defaultOut

  const builderCmd = (dir) => [
    'pnpm',
    'exec',
    'electron-builder',
    '--win',
    ...forms,
    `-c.productName=${name}`,
    // 版本来自配置 JSON 的 version（默认 0.0.1）：extraMetadata 合并进打包用的
    // package.json，产物文件名（zip/portable/setup）与 exe 版本资源都取它。
    `-c.extraMetadata.version=${merged.version}`,
    // 安装版标记：打进 app.asar 的 package.json，运行时 distribution.ts 据此强制
    // 使用系统默认数据目录（%APPDATA%），忽略 config 的 data_dir。绿色版不带此参数。
    ...(forms.includes('nsis') ? ['-c.extraMetadata.distribution=nsis'] : []),
    // 清空 package.json 的 description：electron-builder 把它原样写进桌面/开始菜单
    // 快捷方式的「备注」（lnk Comment，即鼠标悬停提示）与卸载注册表 Comments。
    // 那句壳工程自述（"web2app for Windows — Electron 绿色版壳…"）对终端用户无意义，
    // 空串 ⇒ CreateShortCut 收到空备注，悬停不再有提示（2026-10-01 使用者要求）。
    // 仅安装版覆盖，绿色版没有快捷方式、package.json 保持原样。
    ...(forms.includes('nsis') ? ['-c.extraMetadata.description='] : []),
    // 桌面 / 开始菜单**快捷方式名**用配置的 title（显示名，如 "Note"），而非安装名。
    // productName=app_name 仍承担：exe 文件名、安装目录、「程序与功能」卸载条目；
    // shortcutName 只改快捷方式本身的显示名（2026-10-01 使用者要求）。
    ...(forms.includes('nsis') ? [`-c.nsis.shortcutName=${merged.title}`] : []),
    ...extraBuilder,
    `--config.directories.output=${dir}`,
    ...extra,
  ]

  let attempt = await runTee(builderCmd(outDir), ELECTRON, env, dry)
  if (attempt.status !== 0 && !dry && outDir === defaultOut && isDeleteBlocked(attempt.tail)) {
    const fallback = join(ROOT, 'tests', 'pack', `electron-local-${stamp()}`)
    emit(
      `\n⚠️ electron-builder 清空 ${rel(outDir)} 时被删除策略拦住了（safe-delete / emptyDir）。\n` +
        `   这台机器不允许它删掉旧产物 ⇒ **自动**改用全新目录重试：${rel(fallback)}\n` +
        `   （旧产物原样留在 ${rel(defaultOut)}/，没被动过；清掉它之后即可恢复用 tests/pack/electron/）`,
    )
    outDir = fallback
    attempt = await runTee(builderCmd(outDir), ELECTRON, env, dry)
  }
  if (attempt.status !== 0) {
    const hint = isDeleteBlocked(attempt.tail)
      ? '\n  看起来是输出目录删不掉（不是配置/代码问题）：换一个全新目录再试即可，例如\n' +
        `    node scripts/pack-local.mjs --config <file> --target ${args.target} --out ../tests/pack/electron-local-${stamp()}`
      : ''
    throw new FatalError(`electron-builder 失败（退出码 ${attempt.status}）${hint}`)
  }

  // 护栏：配置没进 asar 时 electron-builder 照样出包、双击照样打开，只是静默退回内置默认地址
  // （example.com），构建日志全绿 ⇒ 必须显式校验。
  // ⚠️ **一律显式传** asar 路径：check-asar-config.cjs 不给参数时会自己在 release/ 下递归找，
  //    而那里可能躺着上一次的旧产物 ⇒ 会出现「绿了，但绿的是上一次的包」。
  run(
    [
      'node',
      'scripts/check-asar-config.cjs',
      join(outDir, 'win-unpacked', 'resources', 'app.asar'),
    ],
    ELECTRON,
    env,
    dry,
  )

  emit(`\n产物目录：${rel(outDir)}`)
}
