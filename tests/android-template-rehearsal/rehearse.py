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


if __name__ == "__main__":
    sys.exit(main())
