#!/usr/bin/env python3
"""用【真模板】离线重放 build-custom.yml 的 Android 补丁链（不联网）。

为什么要做：本机没有 Android SDK/gradle，但补丁链里 10 个 Python 脚本全是纯文本改写，
只要有「tauri-cli 生成的 gen/android 长什么样」就能离线跑一遍，验证我们的改写锚点在
**当前模板**（AGP 9.3.1 / Gradle 9.6.1 / compileSdk 37）上是否仍然成立。

模板内容取自 tauri dev 分支 crates/tauri-cli/templates/mobile/android/（占位符已按
本仓库 tauri.conf.json 的 identifier 展开）。
"""
import os
import pathlib
import re
import shutil
import subprocess
import sys

REPO = pathlib.Path(r"D:\web2app")
SCRIPTS = REPO / ".github" / "scripts"
WORK = pathlib.Path(os.environ.get("TEMP", "/tmp")) / "w2a-rehearse"
IDENT = "com.example.web2app"
APP_NAME = "演示应用"
TITLE = "演示应用"

BUILD_GRADLE_KTS = """import java.util.Properties
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

android {
    compileSdk = 37
    namespace = "com.example.web2app"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "com.example.web2app"
        minSdk = 24
        targetSdk = 37
        versionCode = tauriProperties.getProperty("tauri.android.versionCode", "1").toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0")
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
        }
        getByName("release") {
            optimization {
               enable = true
            }
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
    buildFeatures {
        buildConfig = true
    }
}

kotlin {
    compilerOptions {
        jvmTarget = JvmTarget.JVM_1_8
    }
}

rust {
    rootDirRel = "../.."
}

dependencies {
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
}

apply(from = file("tauri.build.gradle.kts"))
"""

MANIFEST = """<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android">
    <uses-permission android:name="android.permission.INTERNET" />

    <uses-feature android:name="android.software.leanback" android:required="false" />

    <application
        android:icon="@mipmap/ic_launcher"
        android:label="@string/app_name"
        android:theme="@style/Theme.web2app"
        android:usesCleartextTraffic="${usesCleartextTraffic}">
        <activity
            android:configChanges="orientation|keyboardHidden|keyboard|screenSize|locale|smallestScreenSize|screenLayout|uiMode"
            android:launchMode="singleTask"
            android:label="@string/main_activity_title"
            android:name=".MainActivity"
            android:exported="true">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
                <category android:name="android.intent.category.LEANBACK_LAUNCHER" />
            </intent-filter>
        </activity>

        <provider
          android:name="androidx.core.content.FileProvider"
          android:authorities="${applicationId}.fileprovider"
          android:exported="false"
          android:grantUriPermissions="true">
          <meta-data
            android:name="android.support.FILE_PROVIDER_PATHS"
            android:resource="@xml/file_paths" />
        </provider>
    </application>
</manifest>
"""

MAIN_ACTIVITY = """package com.example.web2app

import android.os.Bundle
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }
}
"""

STRINGS = """<resources>
    <string name="app_name">"Web2App"</string>
    <string name="main_activity_title">"Web2App"</string>
</resources>
"""

# 真模板的主题文件（`app/src/main/res/values/themes.xml`）：patch-android-splash-theme.py
# 往 `<style name="Theme.<identifier>">` 里插 `android:windowBackground`。
THEMES = """<?xml version="1.0" encoding="utf-8"?>
<resources>
    <style name="Theme.web2app" parent="Theme.MaterialComponents.DayNight.NoActionBar">
        <item name="android:windowActionModeOverlay">true</item>
    </style>
</resources>
"""

# CI 里的执行顺序（build-custom.yml），跳过大改 tauri.conf.json 的 prepare-android-conf
STEPS = [
    ("apply-android-labels.py", {}),
    ("prepare-wry-kotlin.py", {}),
    ("inject-gradle-patch-task.py", {}),
    ("patch-android-insets.py", {}),
    ("patch-android-back.py", {}),
    ("patch-android-download.py", {}),
    ("patch-android-extras.py", {}),
    ("patch-android-schemes.py", {}),
    ("patch-android-fullscreen.py", {}),
    ("patch-android-permissions.py", {}),
    ("patch-android-cleartext.py", {}),
    ("patch-android-splash-theme.py", {}),
    ("set-android-apk-name.py", {}),
]


def build_skeleton() -> pathlib.Path:
    if WORK.exists():
        shutil.rmtree(WORK)
    gen = WORK / "gen" / "android"
    gen.mkdir(parents=True, exist_ok=True)
    # tauri.conf.json / Cargo.toml 供 prepare-wry-kotlin 读取 identifier 与 lib name
    shutil.copy(REPO / "webview-app" / "tauri.conf.json", WORK / "tauri.conf.json")
    shutil.copy(REPO / "webview-app" / "Cargo.toml", WORK / "Cargo.toml")
    for rel, content in {
        "app/build.gradle.kts": BUILD_GRADLE_KTS,
        "gradle.properties": "org.gradle.configuration-cache=true\n",
        "app/src/main/AndroidManifest.xml": MANIFEST,
        f"app/src/main/java/{IDENT.replace('.', '/')}/MainActivity.kt": MAIN_ACTIVITY,
        "app/src/main/res/values/strings.xml": STRINGS,
        "app/src/main/res/values/themes.xml": THEMES,
    }.items():
        p = gen / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(content, encoding="utf-8", newline="\n")
    return gen


def run_step(script: str, env_extra: dict) -> tuple[int, str]:
    env = dict(os.environ)
    env.update({
        "APP_NAME": APP_NAME,
        "TITLE": TITLE,
        "APP_URL": "https://example.com",
        "VERSION": "0.0.1",
        "ANDROID_PERMISSIONS": '["camera"]',
        "DEBUG": "false",
        "MOBILE_BAR": "false",
        "TABS_ENABLED": "false",
        "EXTRA": "{}",
        "PYTHONUTF8": "1",
        "PYTHONIOENCODING": "utf-8",
    })
    env.update(env_extra)
    p = subprocess.run([sys.executable, str(SCRIPTS / script)],
                       cwd=WORK, env=env, capture_output=True, text=True)
    return p.returncode, (p.stdout + p.stderr).strip()


def main() -> int:
    gen = build_skeleton()
    print(f"重放目录：{WORK}\n")
    bad = []
    for script, extra in STEPS:
        rc, out = run_step(script, extra)
        head = "\n".join(out.splitlines()[:6])
        print(f"[{'OK ' if rc == 0 else 'FAIL'}] {script}")
        if head:
            print("      " + head.replace("\n", "\n      "))
        if rc != 0:
            bad.append(script)

    print("\n──────── 结果检查 ────────")
    gradle = (gen / "app/build.gradle.kts").read_text(encoding="utf-8")
    if "web2app-android-patch-task" in gradle:
        tail = gradle[gradle.index("// === web2app-android-patch-task"):]
        print(f"app/build.gradle.kts 注入块：{len(tail.splitlines())} 行；"
              f"含 Exec={'org.gradle.api.tasks.Exec' in tail}，含 python3={'\"python3\"' in tail}")
        print("注入块首 6 行：")
        for line in tail.splitlines()[:6]:
            print("   " + line)
    else:
        print("!! 未注入 gradle 补丁任务")
        bad.append("排他性检查：gradle 注入块缺失")

    act = (gen / f"app/src/main/java/{IDENT.replace('.', '/')}/MainActivity.kt").read_text(encoding="utf-8")
    print(f"MainActivity.kt：{len(act.splitlines())} 行；"
          f"insets={'WindowInsets' in act or 'setOnApplyWindowInsetsListener' in act}，"
          f"back={'onWebViewCreate' in act or 'copyBackForwardList' in act}，"
          f"bridge={'__web2appAndroid' in act}")

    man = (gen / "app/src/main/AndroidManifest.xml").read_text(encoding="utf-8")
    print(f"Manifest：permissions={man.count('<uses-permission')} 条；"
          f"WRITE_EXTERNAL={('WRITE_EXTERNAL_STORAGE' in man)}；CAMERA={('permission.CAMERA' in man)}")

    print("\n──────── 生成的文件 ────────")
    for p in sorted(gen.rglob("*.kt")):
        print(f"  {p.relative_to(gen)}  ({len(p.read_text(encoding='utf-8').splitlines())} 行)")

    if bad:
        print("\n::error::补丁链在真模板上失败：" + ", ".join(bad))
        return 1
    print("\n补丁链在【tauri 真模板】上全部跑通 ✓")
    return 0


# ---------------------------------------------------------------------------
# 升级演练（--drill）：证明「上游升级不需要改这些脚本」
# ---------------------------------------------------------------------------
# 做法 = 对**真 wry 模板生成出来的 Kotlin** 施加 4 类「升级时真会发生的变化」，
# 再看补丁链的结局是否符合预期：
#
#   1. reformat             缩进/注释被上游格式化改掉         → 三个补丁照样打上
#   2. param_rename         override 的参数名被改名           → 照样打上（锚点只认类型）
#   3. upstream_fullscreen  上游自己把全屏实现了              → 补丁自动退场（不报错、不覆盖）
#   4. anchor_gone          上游把 shouldOverrideUrlLoading 删了 → 默认降级跳过；严格模式才红
#   5. wry_file_renamed     wry 把 RustWebViewClient.kt 改名/挪位置 → 改它的两个补丁降级，
#                          其余照打，且半成品方向安全（MainActivity 的 JS 桥仍完整）
#
# 为什么这比「等升级了再试」强：它把「升级免疫」变成每次 CI 都能复跑的断言。
GLUE_STEPS = (
    "patch-android-extras.py",
    "patch-android-schemes.py",
    "patch-android-fullscreen.py",
)
_ALL_STEPS = [s for s, _ in STEPS]
_GLUE_AT = _ALL_STEPS.index(GLUE_STEPS[0])
PRE_STEPS = _ALL_STEPS[:_GLUE_AT]
POST_STEPS = _ALL_STEPS[_GLUE_AT + len(GLUE_STEPS):]


def gen_dir(gen: pathlib.Path) -> pathlib.Path:
    return gen / "app/src/main/java" / IDENT.replace(".", "/") / "generated"


def method_span(src: str, name: str):
    """按 `override fun <name>(` 定位并做花括号配对，返回 (start, end)。"""
    m = re.search(rf"override\s+fun\s+{re.escape(name)}\s*\(", src)
    if not m:
        return None
    brace = src.find("{", m.end())
    if brace < 0:
        return None
    depth = 0
    for j in range(brace, len(src)):
        if src[j] == "{":
            depth += 1
        elif src[j] == "}":
            depth -= 1
            if depth == 0:
                return m.start(), j + 1
    return None


def _edit(path: pathlib.Path, fn) -> None:
    src = path.read_text(encoding="utf-8")
    path.write_text(fn(src), encoding="utf-8", newline="\n")


def mutate_reformat(gen: pathlib.Path) -> None:
    d = gen_dir(gen)
    for f in ("RustWebViewClient.kt", "RustWebChromeClient.kt"):
        def fn(src: str) -> str:
            src = re.sub(r"(?m)^( +)", r"  \1", src)  # 缩进整体 +2（模拟上游重新格式化）
            return re.sub(
                r"(?m)^( *)override fun onShowCustomView",
                r"\1// upstream refactor: 缩进与注释都可能变，锚点不能依赖它们\n\1override fun onShowCustomView",
                src,
                count=1,
            )
        _edit(d / f, fn)


def mutate_param_rename(gen: pathlib.Path) -> None:
    def fn(src: str) -> str:
        span = method_span(src, "shouldOverrideUrlLoading")
        assert span, "真模板里找不到 shouldOverrideUrlLoading"
        a, b = span
        body = re.sub(r"\bview\b", "w", src[a:b])
        body = re.sub(r"\brequest\b", "req", body)
        return src[:a] + body + src[b:]
    _edit(gen_dir(gen) / "RustWebViewClient.kt", fn)


def mutate_upstream_fullscreen(gen: pathlib.Path) -> None:
    def fn(src: str) -> str:
        span = method_span(src, "onShowCustomView")
        assert span, "真模板里找不到 onShowCustomView"
        a, b = span
        head = src[a: src.index("{", a)]
        body = (
            "{\n"
            "    val content = activity.findViewById<android.view.ViewGroup>(android.R.id.content)\n"
            "    content.addView(view)\n"
            "  }"
        )
        return src[:a] + head + body + src[b:]
    _edit(gen_dir(gen) / "RustWebChromeClient.kt", fn)


def mutate_wry_file_renamed(gen: pathlib.Path) -> None:
    """模拟 wry 升级把胶水文件改名 / 挪位置（`find_kotlin_file` 找不到）。"""
    d = gen_dir(gen)
    (d / "RustWebViewClient.kt").rename(d / "RustWebViewClientImpl.kt")


def mutate_anchor_gone(gen: pathlib.Path) -> None:
    def fn(src: str) -> str:
        span = method_span(src, "shouldOverrideUrlLoading")
        assert span, "真模板里找不到 shouldOverrideUrlLoading"
        return src[: span[0]] + src[span[1]:]
    _edit(gen_dir(gen) / "RustWebViewClient.kt", fn)


def glue_state(gen: pathlib.Path) -> dict:
    """三个胶水补丁的落盘情况 + 两条「没把上游搞坏」的证据。

    文件可能被上游改名（演练 `wry_file_renamed`）⇒ 读不到就当空串。
    """
    d = gen_dir(gen)

    def read(p: pathlib.Path) -> str:
        return p.read_text(encoding="utf-8") if p.is_file() else ""

    client = read(d / "RustWebViewClient.kt")
    chrome = read(d / "RustWebChromeClient.kt")
    activity = read(gen / "app/src/main/java" / IDENT.replace(".", "/") / "MainActivity.kt")
    return {
        "extras": "web2app-android-bridge-key" in client,
        "schemes": "web2app-android-external-scheme" in client,
        "fullscreen": "web2app-android-fullscreen" in chrome,
        # 上游实现必须原样留着（自动退场 ≠ 覆盖）
        "upstream_kept": "content.addView(view)" in chrome,
        "our_fullscreen": "web2appCustomView" in chrome,
        # 半成品方向检查：client 降级时 MainActivity 的 JS 桥仍应完整（桥不反向依赖 client）
        "activity_bridge": "__web2appAndroid" in activity,
    }


def drill() -> int:
    cases = [
        # name, 变异, 期望的补丁落盘状态（部分键）, 输出里必须出现的话
        ("reformat", mutate_reformat,
         {"extras": True, "schemes": True, "fullscreen": True}, None),
        ("param_rename", mutate_param_rename,
         {"extras": True, "schemes": True, "fullscreen": True}, None),
        ("upstream_fullscreen", mutate_upstream_fullscreen,
         {"fullscreen": False, "schemes": True,
          "upstream_kept": True, "our_fullscreen": False}, "自动退场"),
        ("anchor_gone", mutate_anchor_gone,
         {"schemes": False, "extras": True, "fullscreen": True}, "降级跳过"),
        # 上游改名/挪文件：两个改 client 的补丁降级，fullscreen 照打；
        # 且已写盘的 MainActivity JS 桥保持完整（半成品方向必须是安全的）
        ("wry_file_renamed", mutate_wry_file_renamed,
         {"schemes": False, "extras": False, "fullscreen": True,
          "activity_bridge": True}, "降级跳过"),
    ]
    bad = []
    for name, mutate, expect, needle in cases:
        gen = build_skeleton()
        outputs = {}

        def run(script_list, env_extra=None):
            for script in script_list:
                rc, out = run_step(script, env_extra or {})
                outputs[script] = out
                if rc != 0:
                    bad.append(f"{name}: {script} rc={rc}")
                    return False
            return True

        ok = run(PRE_STEPS)
        mutate(gen)
        ok = run(GLUE_STEPS) and ok
        ok = run(POST_STEPS) and ok

        state = glue_state(gen)
        for key, want in expect.items():
            if state[key] != want:
                bad.append(f"{name}: {key} 期望 {want}，实际 {state[key]}")
                ok = False
        if needle:
            blob = "\n".join(outputs.values())
            if needle not in blob:
                bad.append(f"{name}: 输出里没有「{needle}」")
                ok = False

        # anchor_gone 额外证明「严格模式才红」：门禁可以自己开，但默认不打断下游。
        if name == "anchor_gone":
            rc, _ = run_step("patch-android-schemes.py",
                             {"W2A_ANDROID_PATCH_STRICT": "1"})
            if rc == 0:
                bad.append("anchor_gone: 严格模式必须让构建红")
                ok = False

        print(f"[{'OK ' if ok else 'FAIL'}] {name:<20} 落盘={state}")

    if bad:
        print("\n::error::升级演练失败：\n  " + "\n  ".join(bad))
        return 1
    print(f"\n升级演练通过 ✓（{len(cases)} 类上游变化下，补丁链无需改动脚本）")
    return 0


if __name__ == "__main__":
    if "--drill" in sys.argv[1:]:
        sys.exit(drill())
    sys.exit(main())
