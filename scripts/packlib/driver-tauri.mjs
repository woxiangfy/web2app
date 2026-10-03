/**
 * Tauri 打包分支。
 * 配置在**编译期**被读（`option_env!` → 二进制常量）⇒ 必须在 cargo 之前设好环境变量。
 */
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { FatalError, emit } from './diagnostic.mjs'
import { ROOT, WEBVIEW, rel } from './paths.mjs'
import { TAURI_OUT_DEFAULT, have, moveFile, resolvePackOut, run } from './util.mjs'

export function doTauri(args, merged, env, name, rr) {
  if (!have(['cargo', '--version'])) throw new FatalError('未找到 cargo')
  if (!have(['cargo', 'tauri', '--version'])) {
    throw new FatalError(
      '未安装 tauri-cli。本地 Tauri 打包需要它：\n' +
        '    cargo install tauri-cli --version "^2"\n' +
        '  （CI 的 windows job 也是这一步；首次安装要编几分钟）',
    )
  }
  const dry = args.dryRun
  const conf = join(WEBVIEW, 'tauri.conf.json')

  // 1) 安装名 → productName（与 CI 同一个清洗规则）
  if (dry) {
    emit(`\n[1/4] 将改写 ${rel(conf)}：productName = ${name}，mainBinaryName = web2app`)
  } else {
    rr.file(conf) // tracked，必须备份
    const data = JSON.parse(readFileSync(conf, 'utf8'))
    data.productName = name
    // 固定主程序文件名 ⇒ target/release/web2app.exe 路径稳定，单文件版那步可以直接引用
    data.mainBinaryName = 'web2app'
    writeFileSync(conf, JSON.stringify(data, null, 2) + '\n', { encoding: 'utf8' })
    emit(`\n[1/4] 已改写 ${rel(conf)}：productName = ${name}（打包后还原）`)
  }

  // 2) 图标 —— 同样**判据只看 JSON**（CI 那步是 `[ -z "${ICON_URL:-}" ]` 判断后跑 `cargo tauri icon`）
  //    ⚠️ `cargo tauri icon` 会重新生成**整套** `icons/`，而它是 tracked ⇒ 整目录备份 + 还原。
  if (merged.icon_url && !args.noIcon) {
    const url = merged.icon_url
    const png = join(ROOT, 'icon-custom.png')
    if (dry) {
      emit(`\n[2/4] 将下载图标 ${url} 并执行 cargo tauri icon`)
    } else {
      rr.dir(join(WEBVIEW, 'icons'))
      run(['curl', '-fsSL', '--retry', '3', url, '-o', png], ROOT, env, false)
      if (readFileSync(png).subarray(0, 8).toString('hex') !== '89504e470d0a1a0a') {
        throw new FatalError('icon_url 下载的不是 PNG')
      }
      run(['cargo', 'tauri', 'icon', png], WEBVIEW, env, false)
      rmSync(png, { force: true })
      emit('icons/ 已重新生成（打包后整目录还原）')
    }
  } else {
    const why = !merged.icon_url ? '配置里 icon_url 为空' : '--no-icon 显式跳过'
    emit(`\n[2/4] 跳过图标：${why}，沿用仓库默认图标`)
  }

  // 3) 编译 + 打包
  //    ⚠️ 环境变量必须在 cargo 之前就位 —— Tauri 的配置是 `option_env!` 编译期常量，
  //    改了环境变量必须**重新编译**（cargo 通过 dep-info 的 env-dep 感知，无需 clean）。
  const wantMsi = args.target === 'tauri-msi' || args.target === 'tauri-all'
  const wantZip = args.target === 'tauri-portable' || args.target === 'tauri-all'

  if (wantMsi) {
    emit('\n[3/4] cargo tauri build（产安装包，同时也产出 web2app.exe）')
    run(['cargo', 'tauri', 'build'], WEBVIEW, env, dry)
  } else if (wantZip) {
    emit('\n[3/4] cargo tauri build --no-bundle（只编译 exe，不产安装包）')
    run(['cargo', 'tauri', 'build', '--no-bundle'], WEBVIEW, env, dry)
  }

  // 4) 产物归集 —— 与 Electron 同一约定：**交付产物一律搬到仓库根 tests/ 下**，
  //    不留在 cargo 构建树（webview-app/target/release/）里。Tauri 不能像 electron-builder
  //    那样配置输出目录，只能在构建后搬运：
  //      · 单文件绿色版：web2app.exe（cargo 主产物，固定文件名）**复制**为 <out>/<name>.exe
  //        —— 是复制不是移动：web2app.exe 是增量编译的中间产物，删了下次要全量重编；
  //      · 安装包：bundle/msi/*.msi、bundle/nsis/* **移动**到 <out>/（tauri.conf.json 的
  //        bundle.targets 同时含 msi 与 nsis，本机构建两种都产）。
  //    默认目录 tests/pack/tauri/ 与 Electron 的 tests/pack/ 分开：两壳绿色版都叫 <name>.exe，混放互相覆盖。
  const releaseDir = join(WEBVIEW, 'target', 'release')
  const outDir = args.out ? resolvePackOut(args.out, WEBVIEW) : TAURI_OUT_DEFAULT

  if (dry) {
    emit('\n[4/4] 产物归集（dry-run，不搬文件）：')
    if (wantZip) {
      emit(`  [单文件绿色版] ${rel(join(releaseDir, 'web2app.exe'))} → ${rel(join(outDir, `${name}.exe`))}`)
    }
    if (wantMsi) {
      emit(`  [msi 安装包]  ${rel(join(releaseDir, 'bundle', 'msi'))}/*.msi  → ${rel(outDir)}/`)
      emit(`  [nsis 安装包] ${rel(join(releaseDir, 'bundle', 'nsis'))}/*       → ${rel(outDir)}/`)
    }
    emit(`\n产物目录：${rel(outDir)}/`)
    return
  }

  // 默认目录由本脚本**完全掌控** ⇒ 整目录重建，顺手清掉上一版本号的陈旧安装包（如升版后的旧 MSI）。
  // ⚠️ 自定义 --out 可能指向用户自己的目录，**绝不整目录清空**，只搬运/覆盖同名文件。
  if (!args.out && existsSync(outDir)) rmSync(outDir, { recursive: true, force: true })
  mkdirSync(outDir, { recursive: true })

  if (wantZip) {
    const exe = join(releaseDir, 'web2app.exe')
    if (!existsSync(exe)) throw new FatalError(`未找到 ${rel(exe)}，主程序没编译出来`)
    const dest = join(outDir, `${name}.exe`)
    copyFileSync(exe, dest) // ⚠️ 复制：web2app.exe 是 cargo 增量编译产物，必须留在 target/
    emit(`  [单文件绿色版] ${rel(dest)}（改名后即为 ${name}.免安装版.exe）`)
  }

  if (wantMsi) {
    // tauri.conf.json 的 bundle.targets = ["msi","nsis"] ⇒ 两种安装包都要收。
    let installerCount = 0
    for (const kind of ['msi', 'nsis']) {
      const subDir = join(releaseDir, 'bundle', kind)
      if (!existsSync(subDir)) continue
      for (const file of readdirSync(subDir)) {
        const src = join(subDir, file)
        if (!statSync(src).isFile()) continue
        const dest = join(outDir, file)
        moveFile(src, dest) // 安装包是交付产物，搬走（清空 bundle/，下次构建重新生成）
        emit(`  [${kind} 安装包] ${rel(dest)}`)
        installerCount++
      }
    }
    if (installerCount === 0) {
      throw new FatalError(
        `cargo tauri build 已返回成功，但在 ${rel(join(releaseDir, 'bundle'))}/ 下没找到任何安装包`,
      )
    }
  }

  emit(`\n产物目录：${rel(outDir)}/`)
}
