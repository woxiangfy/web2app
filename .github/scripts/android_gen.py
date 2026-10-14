#!/usr/bin/env python3
"""Android 工程（`cargo tauri android init` 生成的 `gen/android`）路径解析的**单一真源**。

⚠️ 为什么需要它 —— 工程根在不同调用上下文里位置不同：

  · **CI**：两个 workflow 的 `defaults.run.working-directory` 都是 `webview-app`，
    脚本以 `../.github/scripts/xxx.py` 调用 ⇒ cwd = `<repo>/webview-app` ⇒ 工程根 = `gen/android`；
  · **本机在仓库根手跑** ⇒ 工程根 = `webview-app/gen/android`。

历史缺陷（2026-10-13 修复）：`patch-android-back.py` 的 `--gen-root` 默认值写死成后者，
CI 里按 cwd 解析成 `<repo>/webview-app/webview-app/gen/android` ⇒ `FileNotFoundError`
⇒ android job 在「接管返回键」这一步中断，后续**构建 APK / 签名 / 上传全部被跳过**；
而该 job 是 `continue-on-error: true`，所以「APK 根本没产出」看不出根因。

此后所有安卓补丁脚本一律经本模块解析工程根，不再各自写死。
"""

from __future__ import annotations

import pathlib
import re

# 候选顺序 = 探测优先级：`gen/android`（CI 形态，cwd=webview-app）优先于
# `webview-app/gen/android`（仓库根手跑形态）。
GEN_ROOT_CANDIDATES = ("gen/android", "webview-app/gen/android")

_JAVA_REL = ("app", "src", "main", "java")


def clean_name(name: str) -> str:
    """「安装名」清洗（productName / APK 文件名共用同一套规则）。

    去掉 Windows 文件名非法字符 → 压缩连续空白 → 去首尾空格与句点 → 空则回落 "Web2App"。
    单一真源：`prepare-android-conf.py`（productName）与 `set-android-apk-name.py`（APK 文件名）
    共用，避免两处规则漂移。
    """
    safe = re.sub(r'[\\/:*?"<>|]+', "_", name)
    safe = re.sub(r"\s+", " ", safe).strip(" .")
    return safe or "Web2App"


def gen_root_candidates_str() -> str:
    """给 `--help` 与错误信息用的候选串。"""
    return " / ".join(GEN_ROOT_CANDIDATES)


def resolve_gen_root(explicit: str | None = None) -> pathlib.Path:
    """确定 Android 工程根：显式给定则直接用；否则按候选顺序探测第一个真实存在的。

    一个都不存在时返回首选候选 —— 让上层报出可读错误，而不是静默按错路径找。
    """
    if explicit:
        return pathlib.Path(explicit)
    for cand in GEN_ROOT_CANDIDATES:
        if (pathlib.Path(cand).joinpath(*_JAVA_REL)).is_dir():
            return pathlib.Path(cand)
    return pathlib.Path(GEN_ROOT_CANDIDATES[0])


def java_root(gen_root: pathlib.Path) -> pathlib.Path:
    """安卓工程里 Java/Kotlin 源码根（模板里是 `app/src/main/java`）。"""
    return gen_root.joinpath(*_JAVA_REL)


def find_main_activity(gen_root: pathlib.Path) -> pathlib.Path:
    """定位 MainActivity.kt / MainActivity.java。"""
    jroot = java_root(gen_root)
    cands = sorted(p for p in jroot.rglob("MainActivity.*") if p.suffix in (".kt", ".java")) \
        if jroot.exists() else []
    if not cands:
        raise FileNotFoundError(
            f"未找到 {jroot}/**/MainActivity.{{kt,java}}"
            f"（请确认已先执行 cargo tauri android init；已尝试工程根："
            f"{gen_root_candidates_str()}）"
        )
    return cands[0]


def find_kotlin_file(gen_root: pathlib.Path, filename: str) -> pathlib.Path:
    """按文件名定位 wry 生成到 java 根下的 Kotlin 文件（如 `RustWebViewClient.kt`）。

    `cargo tauri android init` 会把 wry 的 kotlin 模板（包名占位符替换后）复制进
    `app/src/main/java/<包名路径>/`，与 MainActivity 同目录但包名来自 identifier，
    调用方不应硬编码包名路径 —— 统一 rglob。
    模板升级后文件若被移动 / 改名，这里抛错让补丁步显式失败（android job 不允许静默跳过）。
    """
    jroot = java_root(gen_root)
    cands = sorted(jroot.rglob(filename)) if jroot.exists() else []
    if not cands:
        raise FileNotFoundError(
            f"未找到 {jroot}/**/{filename}（请确认已先执行 cargo tauri android init；"
            f"若 wry 模板升级调整了该文件，需要更新对应补丁脚本）"
        )
    return cands[0]


def app_gradle(gen_root: pathlib.Path) -> pathlib.Path | None:
    """模板的 app 级 gradle 文件（优先 `.kts`；Tauri 2.12 起为 Kotlin DSL）。"""
    app = gen_root / "app"
    for name in ("build.gradle.kts", "build.gradle"):
        p = app / name
        if p.exists():
            return p
    return None


def strings_xml(gen_root: pathlib.Path) -> pathlib.Path:
    """安卓应用名资源文件。"""
    return gen_root / "app" / "src" / "main" / "res" / "values" / "strings.xml"


def android_manifest(gen_root: pathlib.Path) -> pathlib.Path:
    """AndroidManifest.xml。"""
    return gen_root / "app" / "src" / "main" / "AndroidManifest.xml"
