#!/usr/bin/env python3
"""往 gen/android/app/build.gradle.kts 里注入 gradle 任务链：
在 Rust 编译（wry build.rs 生成原始 Kotlin 文件）之后、Kotlin 编译之前，
重新运行我们的 Android 补丁脚本。

## 为什么需要

wry 的 Kotlin 文件（RustWebViewClient.kt 等）会在 `cargo tauri android build`
的 Rust 编译阶段被 wry build.rs 重新生成（覆盖）。我们在 init 之后提前打的补丁
会被这个覆盖掉。

本脚本在 build.gradle.kts 末尾追加配置，为**每个** `rustBuild<Variant>` 任务
（wry build.rs 在这些任务的 cargo 编译期生成 Kotlin；Variant 形如
UniversalRelease / Arm64Debug）动态配一条链：

  rustBuild<Variant>
    → patchWry<Variant>Step0/1/2（顺序重放 extras / schemes / fullscreen）
    → patchWry<Variant>
  compile<Variant>Kotlin 同名配对 dependsOn patchWry<Variant>

同名配对很关键：
  - release 出包只拉起 release 的 rustBuild，**不会白编一遍 debug .so**；
  - `--apk`（fat APK，assembleUniversalRelease）与 `--split-per-abi`
    （assembleArm64Release 等单架构）都能精确命中；
  - UnitTest / AndroidTest 的 Kotlin 编译任务找不到同名 rustBuild，不连线。

补丁链（Step0/1/2，顺序固定）：
  patch-android-extras.py    （RustWebViewClient.onPageFinished 密钥轮换）
  patch-android-schemes.py   （RustWebViewClient.shouldOverrideUrlLoading 外跳）
  patch-android-fullscreen.py（RustWebChromeClient.onShow/onHideCustomView 全屏）

MainActivity 相关补丁（insets / back / download）在 init 之后直接打一次即可，
因为 MainActivity.kt 不会被 wry build.rs 覆盖。

## Gradle 9 / configuration cache 兼容

gen/android/gradle.properties 默认 org.gradle.configuration-cache=true，
模板 wrapper 是 Gradle 9.6.1：Project#exec 在 8.11 弃用、9.0 已移除，
且执行期访问 project 违反 CC。故补丁一律用内置 org.gradle.api.tasks.Exec
（CC 安全），路径/命令在配置期固化；python 可执行名按**注入时的宿主平台**
写死（CI 是 Linux → python3；Windows 本地构建 → python）。

## 用法

    python3 ../.github/scripts/inject-gradle-patch-task.py
"""

from __future__ import annotations

import argparse
import os
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from android_gen import resolve_gen_root  # noqa: E402

# gradle task 里要重新执行的补丁脚本（仅文件名；Exec task 的 workingDir 已指向
# 仓库根的 .github/scripts/，路径与 --gen-root 均在脚本内计算）
PATCH_SCRIPTS = [
    "patch-android-extras.py",
    "patch-android-schemes.py",
    "patch-android-fullscreen.py",
]

# 幂等标记（追加到 build.gradle.kts 末尾）
IDEMPOTENT_MARKER = "// === web2app-android-patch-task ==="

GRADLE_TASK_BLOCK = """
{marker}
// 注入 web2app Android Kotlin 补丁任务链。
// 时序：Rust 编译（wry build.rs 在 cargo 编译期重新生成原始 Kotlin 文件）之后、
// Kotlin 编译开始前，按顺序重放 extras / schemes / fullscreen 三个补丁。
//
// ⚠️ 为什么补丁步骤一律用内置 org.gradle.api.tasks.Exec：
//   gen/android/gradle.properties 默认 org.gradle.configuration-cache=true，
//   模板 wrapper 是 Gradle 9.6.1 —— Project#exec 在 8.11 弃用、9.0 已移除，
//   且执行期访问 project 本身违反 configuration cache。Exec task + 配置期固化
//   路径/命令是 CC 安全写法。
//
// ⚠️ 为什么整个配对放进 afterEvaluate：
//   rustBuild<Variant> 任务由 buildSrc 的 RustPlugin 在它自己的 afterEvaluate
//   回调里创建（插件先于本脚本注册回调，故执行到这里时任务已就绪）。
//
// 路径推导（配置期计算）：
//   rootProject.projectDir = <repo>/webview-app/gen/android/（settings.gradle 所在目录）
//   ⇒ 仓库根在**上三级**（gen/android → gen → webview-app → <repo>）。
// ⚠️ 但**不要数层数**：早期版本写 parentFile.parentFile（只上两级），得到
//   webview-app/.github/scripts，CI 实测 2026-11-01 报
//   "Working directory '.../webview-app/.github/scripts' does not exist"。
//   之所以能潜伏：Exec 的 workingDir 只在**执行期**校验 ⇒ 配置期照样"成功"，
//   白等 5 分钟 Rust 编译才炸，且日志里看不出是路径算错。
// ⚠️ 也不要在本文件里写 java 包前缀（如 java 点 io 点 File）：build.gradle.kts 里 `java`
//   是 Gradle 生成的类型安全访问器（JavaPluginExtension），不是包名 ⇒ 会报
//   "Unresolved reference 'io'"（2026-11-01 CI 实测 108:26），整个脚本编译失败、零产物。
//   Kotlin 默认已导入 File，直接用即可。
// ⇒ 规则：从 projectDir 逐级向上找第一个含补丁脚本的 .github/scripts；
//   找不到再用「注入时固化下来的绝对路径」兜底；两者都没有则在**配置期**直接失败。
val w2aScriptsDir: File = generateSequence(rootProject.projectDir.absoluteFile) { it.parentFile }
    .map { File(it, ".github/scripts") }
    .firstOrNull { File(it, "patch-android-extras.py").isFile }
    ?: File("{scripts_dir_literal}").takeIf { it.isDirectory }
    ?: throw org.gradle.api.GradleException(
        "web2app: 找不到 .github/scripts —— 已从 ${rootProject.projectDir} 逐级向上查找，" +
            "注入时固化的路径 {scripts_dir_literal} 也不存在；" +
            "请确认 gen/android 位于 <repo>/webview-app/gen/android"
    )
val w2aGenRoot = rootProject.projectDir.absolutePath
val w2aPython = "{python_exe}"
val w2aPatchScripts = listOf(
{script_lines_block}
)

afterEvaluate {
    // ① 用 tasks.names 取 rustBuild 任务名集合（不 realize 任何任务，不破坏惰性配置；
    //    tasks.names 也包含 register 但尚未 realize 的任务）。立即为每个变体注册
    //    Step0→Step1→Step2 链 + 聚合任务，确保后续字符串依赖在图构建期一定能解析。
    val w2aRustTargets = tasks.names.filter {
        it.startsWith("rustBuild") &&
            (it.endsWith("Debug") || it.endsWith("Release"))
    }
    w2aRustTargets.forEach { w2aRustTaskName ->
        val w2aVariant = w2aRustTaskName.removePrefix("rustBuild")
        var w2aPrev: Any = w2aRustTaskName
        w2aPatchScripts.forEachIndexed { w2aIndex, w2aScript ->
            // 每轮用独立 val 固定「上一步」：register 的配置 lambda 惰性执行，
            // 直接捕获 var 会让三轮 lambda 最终都读到循环末值（Step2）→ 自依赖报错。
            val w2aDependsOn = w2aPrev
            val w2aStep = tasks.register<org.gradle.api.tasks.Exec>(
                "patchWry${w2aVariant}Step${w2aIndex}"
            ) {
                group = "web2app"
                description = "Apply web2app Android patch $w2aScript (after rustBuild$w2aVariant)"
                dependsOn(w2aDependsOn)
                workingDir = w2aScriptsDir
                commandLine(w2aPython, w2aScript, "--gen-root", w2aGenRoot)
            }
            w2aPrev = w2aStep
        }
        tasks.register("patchWry$w2aVariant") {
            group = "web2app"
            description = "Re-apply all web2app Android Kotlin patches for variant $w2aVariant"
            // lambda 惰性执行时 w2aPrev 已是最后一个 Step（聚合只依赖它，链由 Step 间串好）
            dependsOn(w2aPrev)
        }
    }

    // ② Kotlin 编译任务与 rustBuild 同名变体配对：
    //    compileUniversalReleaseKotlin → rustBuildUniversalRelease（--apk 走这里）
    //    compileArm64ReleaseKotlin     → rustBuildArm64Release（--split-per-abi）
    //    找不到同名 rustBuild 的（*UnitTest / *AndroidTest 等）一律不连线。
    //    tasks.named(...).configure {} 是惰性配置，不会提前 realize 编译任务。
    tasks.names
        .filter { it.startsWith("compile") && it.endsWith("Kotlin") }
        .forEach { w2aCompileName ->
            val w2aVariant = w2aCompileName.removePrefix("compile").removeSuffix("Kotlin")
            if (w2aRustTargets.contains("rustBuild$w2aVariant")) {
                tasks.named(w2aCompileName).configure {
                    dependsOn("patchWry$w2aVariant")
                }
            }
        }
}
""".strip("\n")


def _mirror_scripts_dir(start: pathlib.Path) -> pathlib.Path | None:
    """Kotlin 段 `generateSequence(...).firstOrNull { ... }` 的 Python 镜像 —— 同一判据。

    存在的意义：那段 Kotlin 本机跑不了（没有 JVM / gradle / SDK），只能把**判据**搬到
    Python 里对着一个真实目录树跑一遍，证明「从 gen/android 逐级向上能找到
    `<repo>/.github/scripts`」这条规则成立。Kotlin 侧一旦改判据，这里同步改（下面有
    字符串守卫钉住 generateSequence 的写法）。
    """
    for cur in (start, *start.parents):
        cand = cur / ".github" / "scripts"
        if (cand / "patch-android-extras.py").is_file():
            return cand
    return None


def selftest() -> int:
    """用临时目录模拟完整流程。"""
    import tempfile
    import subprocess as _sp

    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        tmp_path = pathlib.Path(tmp)
        # 模拟**真实仓布局**：<repo>/webview-app/gen/android —— 仓库根在 gen/android 上三级。
        # 早期 selftest 用的是 <tmp>/gen/android（只上两级就到顶），正好把这个缺陷放过去了。
        repo_root = tmp_path / "repo"
        scripts_dir = repo_root / ".github" / "scripts"
        scripts_dir.mkdir(parents=True)
        (scripts_dir / "patch-android-extras.py").write_text("", encoding="utf-8")
        gen_root = repo_root / "webview-app" / "gen" / "android"

        # 模拟 gen/android/app/build.gradle.kts
        build_gradle = gen_root / "app" / "build.gradle.kts"
        build_gradle.parent.mkdir(parents=True)
        build_gradle.write_text(
            "// 原始 build.gradle.kts\nandroid { namespace = \"com.test.pkg\" }\n",
            encoding="utf-8",
        )

        # 运行脚本
        result = _sp.run(
            [
                sys.executable,
                str(pathlib.Path(__file__)),
                "--gen-root",
                str(gen_root),
            ],
            capture_output=True,
            text=True,
        )
        assert result.returncode == 0, f"首次运行失败：{result.stderr}"

        # 验证注入内容
        content = build_gradle.read_text(encoding="utf-8")
        assert IDEMPOTENT_MARKER in content, "幂等标记未注入"
        # 三个补丁脚本按固定顺序注入
        ie = content.index("patch-android-extras.py")
        iss = content.index("patch-android-schemes.py")
        ifs = content.index("patch-android-fullscreen.py")
        assert ie < iss < ifs, "补丁脚本顺序错误（必须 extras → schemes → fullscreen）"
        # 同名变体配对 + afterEvaluate（rustBuild 任务在 afterEvaluate 才创建）
        assert "afterEvaluate" in content, "配对必须放在 afterEvaluate 中"
        assert "val w2aRustTargets = tasks.names" in content, "应用 tasks.names 非 realize 枚举"
        assert "tasks.named(w2aCompileName).configure" in content, "compile 配对应用惰性 configure"
        assert 'removePrefix("rustBuild")' in content, "缺少 rustBuild 变体提取"
        assert "compileUniversalReleaseKotlin" in content, "缺少 universal 配对说明"
        # 闭包捕获守卫：每步必须用独立 val 固定上一步，禁止直接在惰性 lambda 里捕获 var
        assert "val w2aDependsOn = w2aPrev" in content, "缺少每轮 val 固定（闭包捕获陷阱）"
        # Gradle 9 + configuration cache 兼容性守卫：
        # 必须用内置 Exec task，不得回退到任务动作里调 Project#exec（Gradle 9 已移除）
        assert "org.gradle.api.tasks.Exec" in content, "应使用内置 Exec task"
        # Kotlin DSL 命名空间陷阱：`java` 在这个作用域里是 Gradle 生成的类型安全访问器
        # （JavaPluginExtension），不是包名 ⇒ 写 java.io.File 会让整个脚本编译失败
        # （CI 实测 2026-11-01：app/build.gradle.kts:108:26 Unresolved reference 'io'）。
        # ⚠️ 注入块自己的注释里就带着这个反例字符串（讲解用），直接对 content 匹配会被
        #    注释满足 ⇒ 先剥掉 // 行注释再断言。
        code_only = "\n".join(
            ln for ln in content.splitlines() if not ln.lstrip().startswith("//")
        )
        assert "java.io." not in code_only, "Kotlin DSL 里 java 是扩展访问器，禁止写 java.io.*"
        # 路径推导守卫（2026-11-01 第二轮 CI：workingDir 少上一级 ⇒ 只在 Exec 执行期炸，
        # 白等 5 分钟 Rust 编译，日志还把根因说成「目录不存在」）
        assert "parentFile.parentFile" not in code_only, (
            "禁止用 parentFile 数层数推导仓库根：gen/android 到仓库根是三级，"
            "少一级 = workingDir 不存在，且只在执行期才报错"
        )
        assert "generateSequence(rootProject.projectDir.absoluteFile)" in code_only, (
            "仓库根必须从 projectDir 逐级向上查找 .github/scripts"
        )
        assert "GradleException" in code_only, (
            "找不到 .github/scripts 必须在配置期直接失败（别拖到执行期才报目录不存在）"
        )
        injector_scripts_dir = pathlib.Path(__file__).resolve().parent
        assert injector_scripts_dir.as_posix() in content, (
            "注入块必须固化注入时的绝对脚本目录作兜底（本脚本自己就在该目录里，无需猜层数）"
        )
        # 同一判据的 Python 镜像：在模拟的真实布局 <repo>/webview-app/gen/android 上跑一遍，
        # 证明「逐级向上找」确实命中仓库根的 .github/scripts（Kotlin 段本机执行不了）。
        assert _mirror_scripts_dir(gen_root) == scripts_dir, (
            f"从 {gen_root} 向上未找到 {scripts_dir}"
        )
        assert _mirror_scripts_dir(tmp_path / "elsewhere") is None, (
            "不在仓库内的目录不应误判出 .github/scripts"
        )
        # 语法跳闸（只查配平，不是解析器）：注入块的 `{}` / `()` 必须成对 —— 手工改这段
        # 模板时最容易掉一行 `}`，而这类错误同样只会在 gradle 配置期才暴露。
        for opener, closer in (("{", "}"), ("(", ")")):
            assert code_only.count(opener) == code_only.count(closer), (
                f"注入块括号不配平：{opener}={code_only.count(opener)} {closer}={code_only.count(closer)}"
            )
        assert "doLast" not in content, "不得在任务动作里调 Project#exec（Gradle 9 已移除）"
        assert ".configureEach" not in content, "不应使用 configureEach（注册时机不确定）"
        assert "findByName" not in content, "不应使用 findByName（会提前 realize 任务）"
        # 占位符必须全部替换，不得残留
        for leftover in ("{marker}", "{script_lines_block}", "{python_exe}",
                         "{scripts_dir_literal}", "{{", "}}"):
            assert leftover not in content, f"模板占位符/转义残留：{leftover}"
        # python 可执行名按宿主平台写死
        expected_exe = "python" if sys.platform.startswith("win") else "python3"
        assert f'val w2aPython = "{expected_exe}"' in content, (
            f"python 可执行名应为 {expected_exe}（当前平台 {sys.platform}）"
        )

        # 幂等：再跑一次应该跳过
        result2 = _sp.run(
            [
                sys.executable,
                str(pathlib.Path(__file__)),
                "--gen-root",
                str(gen_root),
            ],
            capture_output=True,
            text=True,
        )
        assert result2.returncode == 0, f"第二次运行失败：{result2.stderr}"
        content2 = build_gradle.read_text(encoding="utf-8")
        # 内容不变（没有重复注入）
        assert content2.count(IDEMPOTENT_MARKER) == 1, "幂等标记重复出现"

        # --force 应能覆盖
        result3 = _sp.run(
            [
                sys.executable,
                str(pathlib.Path(__file__)),
                "--gen-root",
                str(gen_root),
                "--force",
            ],
            capture_output=True,
            text=True,
        )
        assert result3.returncode == 0, f"--force 运行失败：{result3.stderr}"
        content3 = build_gradle.read_text(encoding="utf-8")
        assert content3.count(IDEMPOTENT_MARKER) == 1, "force 后仍只有一个标记"

        # 缺失 build.gradle.kts 应报错
        no_build = tmp_path / "nogen" / "android"
        result4 = _sp.run(
            [
                sys.executable,
                str(pathlib.Path(__file__)),
                "--gen-root",
                str(no_build),
            ],
            capture_output=True,
            text=True,
        )
        assert result4.returncode != 0, "缺失 build.gradle.kts 应返回非零"

        # 应急开关 W2A_SKIP_GRADLE_PATCH=1：正常退出但**不注入**（排障用的二分手段）。
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp2:
            gen2 = pathlib.Path(tmp2) / "gen" / "android"
            bg2 = gen2 / "app" / "build.gradle.kts"
            bg2.parent.mkdir(parents=True)
            bg2.write_text('android { namespace = "x.y" }\n', encoding="utf-8")
            env2 = dict(os.environ, W2A_SKIP_GRADLE_PATCH="1")
            result5 = _sp.run(
                [sys.executable, str(pathlib.Path(__file__)), "--gen-root", str(gen2)],
                capture_output=True,
                text=True,
                env=env2,
            )
            assert result5.returncode == 0, f"应急开关下不应失败：{result5.stderr}"
            assert IDEMPOTENT_MARKER not in bg2.read_text(encoding="utf-8"), \
                "设置 W2A_SKIP_GRADLE_PATCH 后不得再注入任务链"
            assert "W2A_SKIP_GRADLE_PATCH" in (result5.stdout + result5.stderr), \
                "应急开关生效时必须留一行警告（静默跳过 = 无人知道补丁没打）"

    print("inject-gradle-patch-task.py 自测通过 ✓")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="往 app/build.gradle.kts 注入补丁 task")
    parser.add_argument("--gen-root", default=None, help="gen/android 工程根")
    parser.add_argument("--force", action="store_true", help="已注入也重写")
    parser.add_argument("--selftest", action="store_true", help="运行离线自测")
    args = parser.parse_args()

    if args.selftest:
        return selftest()

    # 应急开关（默认关闭）：只跳过「gradle 里重放补丁」这一层。
    # 用途 = **一次运行就能二分**出问题是否出在注入的 Kotlin DSL 上：如果设了它 APK 就出来了，
    # 说明坏在注入块（gradle 配置期解析失败 ⇒ 连 app/build 都不会被创建）；否则坏在别处。
    # 代价：这三处补丁只在 init 之后被应用过一次，wry build.rs 在编译期重新生成 Kotlin 时会覆盖
    # 它们 ⇒ 外部 scheme 外跳 / 下载接管 / H5 全屏会在成包里**静默失效**。仅作排障，勿长期开。
    if os.environ.get("W2A_SKIP_GRADLE_PATCH"):
        print("::warning::W2A_SKIP_GRADLE_PATCH 已设置 —— 跳过 gradle 补丁任务链注入"
              "（extras/schemes/fullscreen 将不再在 Kotlin 编译前重放，成包里这三项会失效）")
        return 0

    gen_root = resolve_gen_root(args.gen_root)
    build_gradle = gen_root / "app" / "build.gradle.kts"

    if not build_gradle.is_file():
        print(f"❌ 找不到 {build_gradle}，请确认已执行 cargo tauri android init", file=sys.stderr)
        return 1

    existing = build_gradle.read_text(encoding="utf-8")
    if IDEMPOTENT_MARKER in existing and not args.force:
        print(f"ℹ️ {build_gradle} 已包含补丁 task（跳过，用 --force 重写）")
        return 0

    if IDEMPOTENT_MARKER in existing and args.force:
        # 删掉旧的注入块（从 marker 到文件末尾）
        idx = existing.index(IDEMPOTENT_MARKER)
        existing = existing[:idx].rstrip() + "\n"

    # 用 str.replace 而非 str.format：Kotlin 块里大量 { } 与 $ 模板，
    # format 会把它们当占位符/语法，改用 replace 只替换三个明确占位符。
    script_lines_block = "\n".join(f'    "{name}",' for name in PATCH_SCRIPTS)
    # python 可执行名按注入脚本运行的宿主平台决定（CI Android job 是 Linux → python3；
    # Windows 本地跑 tauri android build 时通常只有 python.exe）。
    python_exe = "python" if sys.platform.startswith("win") else "python3"
    # 兜底路径：本脚本自己就住在 .github/scripts 里 ⇒ 绝对路径是**可知的**，不靠猜层数。
    # 用 as_posix() 规避 Windows 反斜杠在 Kotlin 字符串里的转义问题（Java File 认 `/`）。
    scripts_dir_literal = pathlib.Path(__file__).resolve().parent.as_posix()

    block = (
        GRADLE_TASK_BLOCK
        .replace("{marker}", IDEMPOTENT_MARKER)
        .replace("{script_lines_block}", script_lines_block)
        .replace("{python_exe}", python_exe)
        .replace("{scripts_dir_literal}", scripts_dir_literal)
    )

    # 追加到文件末尾（确保末尾有换行）
    result = existing.rstrip() + "\n\n" + block + "\n"
    build_gradle.write_text(result, encoding="utf-8", newline="\n")

    print(f"✅ 已向 {build_gradle} 注入 web2app Android 补丁任务链")
    print(f"   依赖链：rustBuild<变体> → patchWry<变体>Step0..2 → compile<变体>Kotlin")
    print(f"   python 可执行名：{python_exe}")
    return 0


if __name__ == "__main__":
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except Exception:  # noqa: BLE001
            pass
    sys.exit(main())
