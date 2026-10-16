#!/usr/bin/env python3
"""把 wry crate 里的 Android Kotlin 模板文件**手动生成**到 gen/android 的
`java/<包名>/generated/` 目录。

## 为什么需要这个脚本

`cargo tauri android init` 只生成 MainActivity.kt 到 `java/<包名>/`，不会生成
wry 的 Kotlin 文件（RustWebViewClient.kt / RustWebChromeClient.kt /
RustWebView.kt / Rust.kt / WryActivity.kt / Ipc.kt / Logger.kt /
PermissionHelper.kt / proguard-wry.pro）。

这些文件由 **wry 的 build.rs** 在 **`cargo tauri android build` 的 Rust 编译阶段**
动态生成到 `java/<包名>/generated/`。

我们的补丁脚本（patch-android-extras.py / schemes.py / fullscreen.py）都需要修改
RustWebViewClient.kt 等文件，但它们在 **build 之前** 执行——那时 `generated/` 目录
还不存在，所以会 `FileNotFoundError`。

本脚本在补丁链之前手动触发一次 wry build.rs 做的事：读取 wry crate 里的
`src/android/kotlin/*.kt` 模板，替换占位符（`{{package}}` / `{{library}}`），
写到 `gen/android/app/src/main/java/<包名>/generated/`。这样补丁脚本能正常找到
并修改这些文件。

⚠️ **wry build.rs 会在后续 `cargo tauri android build` 时重新生成并覆盖**
`generated/` 里的文件。这是正常的——补丁会在 gradle 的 Kotlin 编译前通过
`inject-gradle-patch-task.py` 注入的 task 再次应用（见该脚本的注释）。

## 用法

    python3 ../.github/scripts/prepare-wry-kotlin.py            # 自动探测工程根和 wry 路径
    python3 ../.github/scripts/prepare-wry-kotlin.py --gen-root gen/android
    python3 ../.github/scripts/prepare-wry-kotlin.py --wry-src /path/to/wry/src/android/kotlin
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import subprocess
import sys

# 复用工程根探测逻辑（同目录下的 android_gen.py）。
sys.path.insert(0, str(pathlib.Path(__file__).parent))
from android_gen import resolve_gen_root  # noqa: E402

# wry android Kotlin 模板文件名（build.rs 逐个处理）。
WRY_KOTLIN_TEMPLATES = (
    "Ipc.kt",
    "Logger.kt",
    "PermissionHelper.kt",
    "proguard-wry.pro",
    "Rust.kt",
    "RustWebChromeClient.kt",
    "RustWebView.kt",
    "RustWebViewClient.kt",
    "WryActivity.kt",
)


def _run_stdout(cmd: list[str]) -> str | None:
    """执行命令并返回 stdout（trim 后）；失败/超时返回 None。

    timeout 给 120s：CI 冷 runner 上首次 `cargo metadata` 要读 registry 索引、
    必要时联网 fetch，30s 曾偏紧；失败后调用方会回落到 registry glob 探测。
    """
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
        if p.returncode == 0:
            return p.stdout.strip()
    except Exception:
        pass
    return None


def find_wry_kotlin_src() -> pathlib.Path:
    """定位 wry crate 里的 android kotlin 模板目录。

    优先级：
      1) 环境变量 `WRY_SRC_DIR`（显式指定，最高优先）
      2) `cargo metadata` 找 wry manifest，推导 `src/android/kotlin/`
         （CI 冷缓存时的**主路径**：metadata 会按需 fetch，registry/src
         里可能还没有 wry 源）
      3) 常见 cargo registry 位置 glob（Windows / Linux / macOS，取最高版本）
    """
    # 1) 环境变量
    env_val = os.environ.get("WRY_SRC_DIR")
    if env_val:
        env_dir = pathlib.Path(env_val)
        if env_dir.is_dir():
            return env_dir

    # 2) cargo metadata（不加 --no-deps，因为 wry 是 tauri 的传递依赖）
    meta_json = _run_stdout(["cargo", "metadata", "--format-version", "1"])
    if meta_json:
        try:
            data = json.loads(meta_json)
        except Exception:
            data = {}
        for pkg in data.get("packages", []):
            if pkg.get("name") == "wry":
                manifest = pathlib.Path(pkg["manifest_path"])
                kotlin = manifest.parent / "src" / "android" / "kotlin"
                if kotlin.is_dir():
                    return kotlin

    # 3) 常见 registry 位置搜索（宽松匹配所有 wry-*/src/android/kotlin，取版本最高的）
    import re as _re
    _ver = _re.compile(r"wry-(\d+)\.(\d+)\.(\d+)")
    home = pathlib.Path.home()
    registry_candidates = [
        home / ".cargo" / "registry" / "src",                  # Linux / macOS
        home / ".cargo" / "registry" / "src",                  # Windows (git bash / WSL)
    ]
    # Windows 用户目录
    if sys.platform.startswith("win"):
        registry_candidates.insert(
            0, pathlib.Path(os.environ.get("USERPROFILE", str(home))) / ".cargo" / "registry" / "src"
        )
    for reg_root in registry_candidates:
        if not reg_root.is_dir():
            continue
        best = None
        best_v = (-1, -1, -1)
        for wry_dir in reg_root.glob("index.crates.io-*/wry-*/src/android/kotlin"):
            if not wry_dir.is_dir():
                continue
            m = _ver.search(wry_dir.parent.parent.parent.name)
            if not m:
                continue
            v = tuple(int(x) for x in m.groups())
            if v > best_v:
                best_v = v
                best = wry_dir
        if best:
            return best

    raise FileNotFoundError(
        "无法定位 wry crate 的 src/android/kotlin/ 模板目录。"
        "请设置环境变量 WRY_SRC_DIR 指向该路径，或确认已执行过 cargo fetch。"
    )


def read_tauri_config(gen_root: pathlib.Path) -> tuple[str, str]:
    """从 tauri.conf.json 读取 identifier（包名）和 lib name。

    tauri.conf.json 不在 gen/android 里，它在 Rust 工程根。gen_root 的父目录的父目录
    就是 Rust 工程根（gen_root = gen/android 或 webview-app/gen/android）。
    """
    # gen_root 形如 `webview-app/gen/android` 或 `gen/android`（CI 里 cwd=webview-app）
    # 所以 Rust 工程根 = gen_root.parent.parent
    project_root = gen_root.parent.parent
    # 但也有可能 cwd 就是仓库根，gen_root = webview-app/gen/android，
    # 那 project_root.parent 就是仓库根，而 tauri.conf.json 在 webview-app/ 里
    # 先在 project_root 找，不行再在 project_root.parent 找
    for search_dir in (project_root, project_root.parent):
        cfg = search_dir / "tauri.conf.json"
        if cfg.is_file():
            break
    else:
        raise FileNotFoundError("找不到 tauri.conf.json（已尝试两个上级目录）")

    data = json.loads(cfg.read_text(encoding="utf-8"))
    identifier: str = data.get("identifier", "")

    # lib name 从 Cargo.toml 读（tauri.conf.json 里没有）
    cargo_toml = cfg.parent / "Cargo.toml"
    lib_name = "web2app_lib"  # 兜底
    if cargo_toml.is_file():
        text = cargo_toml.read_text(encoding="utf-8")
        # [[lib]] name = "xxx"（tauri 用 table 语法）
        m = re.search(r'^\[\[lib\]\][\s\S]*?^name\s*=\s*"([^"]+)"', text, re.M)
        if m:
            lib_name = m.group(1)
        else:
            # 也可能是 [lib]
            m2 = re.search(r'^\[lib\][\s\S]*?^name\s*=\s*"([^"]+)"', text, re.M)
            if m2:
                lib_name = m2.group(1)

    if not identifier:
        raise ValueError("tauri.conf.json 里缺少 identifier 字段")

    return identifier, lib_name


def escape_kotlin_identifier(s: str) -> str:
    """wry build.rs 用的 Kotlin 标识符清洗（反引号转义）。"""
    return s.replace("`", "")


def selftest() -> int:
    """用临时目录模拟完整流程。"""
    import tempfile

    # 0) _run_stdout 必须返回 str（历史 bug：曾把 stdout 包成 pathlib.Path 再
    #    read_text()，导致 cargo metadata 分支必然异常、静默回落 glob）
    got = _run_stdout([sys.executable, "-c", "print('w2a-stdout-probe')"])
    assert isinstance(got, str), f"_run_stdout 必须返回 str，实际 {type(got)!r}"
    assert got == "w2a-stdout-probe", f"_run_stdout 内容错误：{got!r}"

    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        tmp_path = pathlib.Path(tmp)

        # 1) 模拟 wry crate 的 android/kotlin 目录
        wry_src = tmp_path / "wry_src"
        wry_src.mkdir()
        (wry_src / "RustWebViewClient.kt").write_text(
            "package {{package}}\nclass RustWebViewClient: WebViewClient() {}\n",
            encoding="utf-8",
        )
        (wry_src / "Rust.kt").write_text(
            "package {{package}}\nobject Rust {\n    init { System.loadLibrary(\"{{library}}\") }\n}\n",
            encoding="utf-8",
        )
        (wry_src / "RustWebChromeClient.kt").write_text(
            "package {{package}}\nclass RustWebChromeClient: WebChromeClient() {}\n",
            encoding="utf-8",
        )
        (wry_src / "WryActivity.kt").write_text(
            "package {{package}}\nabstract class WryActivity {}\n",
            encoding="utf-8",
        )
        (wry_src / "RustWebView.kt").write_text(
            "package {{package}}\nclass RustWebView: WebView() {}\n",
            encoding="utf-8",
        )
        (wry_src / "Ipc.kt").write_text(
            "package {{package}}\n// ipc\n", encoding="utf-8"
        )
        (wry_src / "Logger.kt").write_text(
            "package {{package}}\nobject Logger\n", encoding="utf-8"
        )
        (wry_src / "PermissionHelper.kt").write_text(
            "package {{package}}\nobject PermissionHelper\n", encoding="utf-8"
        )
        (wry_src / "proguard-wry.pro").write_text(
            "# proguard template\n", encoding="utf-8"
        )

        # 2) 模拟 gen/android 结构 + tauri.conf.json + Cargo.toml
        gen_root = tmp_path / "gen" / "android"
        (gen_root / "app" / "src" / "main" / "java").mkdir(parents=True)
        (gen_root / "app" / "src" / "main" / "AndroidManifest.xml").write_text(
            "<manifest />\n", encoding="utf-8"
        )
        (gen_root / "app" / "build.gradle.kts").write_text(
            "// stub\n", encoding="utf-8"
        )
        # tauri.conf.json 放 gen/android 上两级（模拟 webview-app 目录结构）
        tauri_cfg = tmp_path / "tauri.conf.json"
        tauri_cfg.write_text(
            '{"identifier": "com.test.pkg", "productName": "Test"}\n',
            encoding="utf-8",
        )
        cargo_toml = tmp_path / "Cargo.toml"
        cargo_toml.write_text(
            '[package]\nname = "test"\n\n[[lib]]\nname = "test_lib"\n',
            encoding="utf-8",
        )

        # 3) 用 --gen-root 和 --wry-src 跑
        import os as _os
        _os.chdir(tmp_path)
        import subprocess as _sp
        result = _sp.run(
            [
                sys.executable,
                str(pathlib.Path(__file__)),
                "--gen-root",
                str(gen_root),
                "--wry-src",
                str(wry_src),
            ],
            capture_output=True,
            text=True,
            cwd=str(tmp_path),
        )
        assert result.returncode == 0, f"prepare-wry-kotlin 失败：{result.stderr}"

        # 4) 验证输出
        generated = gen_root / "app" / "src" / "main" / "java" / "com" / "test" / "pkg" / "generated"
        assert generated.is_dir(), f"generated 目录不存在：{generated}"
        for fname in ("RustWebViewClient.kt", "Rust.kt", "RustWebChromeClient.kt"):
            f = generated / fname
            assert f.is_file(), f"{fname} 未生成"
            content = f.read_text(encoding="utf-8")
            assert "package com.test.pkg" in content, f"{fname} package 未替换"
        # Rust.kt 里的 library 占位符
        rust_content = (generated / "Rust.kt").read_text(encoding="utf-8")
        assert '"test_lib"' in rust_content, f"library 未替换：{rust_content}"
        # 自动生成注释
        assert "AUTO-GENERATED" in (generated / "RustWebViewClient.kt").read_text(encoding="utf-8")

        # 5) 幂等：再跑一次应该成功且文件不变
        result2 = _sp.run(
            [
                sys.executable,
                str(pathlib.Path(__file__)),
                "--gen-root",
                str(gen_root),
                "--wry-src",
                str(wry_src),
            ],
            capture_output=True,
            text=True,
        )
        assert result2.returncode == 0, f"第二次运行失败：{result2.stderr}"

    print("prepare-wry-kotlin.py 自测通过 ✓")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(
        description="手动生成 wry Android Kotlin 模板到 gen/android/java/<pkg>/generated/"
    )
    parser.add_argument("--gen-root", default=None, help="gen/android 工程根（缺省自动探测）")
    parser.add_argument("--wry-src", default=None, help="wry crate src/android/kotlin/ 目录")
    parser.add_argument("--dry-run", action="store_true", help="只打印将要写入的文件，不实际写盘")
    parser.add_argument("--selftest", action="store_true", help="运行离线自测")
    args = parser.parse_args()

    if args.selftest:
        return selftest()

    gen_root = resolve_gen_root(args.gen_root)
    wry_src = pathlib.Path(args.wry_src) if args.wry_src else find_wry_kotlin_src()

    identifier, lib_name = read_tauri_config(gen_root)
    package = escape_kotlin_identifier(identifier)

    # 目标目录：gen/android/app/src/main/java/<包名>/generated/
    pkg_path = package.replace(".", "/")
    out_dir = (
        gen_root
        / "app"
        / "src"
        / "main"
        / "java"
        / pkg_path
        / "generated"
    )

    if args.dry_run:
        print(f"工程根     : {gen_root}")
        print(f"wry 模板  : {wry_src}")
        print(f"包名       : {package}")
        print(f"库名       : {lib_name}")
        print(f"输出目录   : {out_dir}")
        for fname in WRY_KOTLIN_TEMPLATES:
            src = wry_src / fname
            if src.is_file():
                print(f"  ✅ {fname}")
            else:
                print(f"  ❌ {fname} 缺失！")
        return 0

    # 确保 generated/ 存在
    out_dir.mkdir(parents=True, exist_ok=True)

    written = 0
    for fname in WRY_KOTLIN_TEMPLATES:
        src = wry_src / fname
        if not src.is_file():
            print(f"⚠️  wry 模板缺失：{fname}（跳过）", file=sys.stderr)
            continue

        raw = src.read_text(encoding="utf-8")

        # 占位符替换（和 wry build.rs 逻辑一致）
        content = (
            raw.replace("{{package}}", package)
            .replace("{{package-unescaped}}", package.replace("`", ""))
            .replace("{{library}}", lib_name)
            .replace("{{class-extension}}", "")
            .replace("{{class-init}}", "")
        )

        # 加自动生成注释（和 wry build.rs 一样）
        auto_comment = (
            "/* THIS FILE IS AUTO-GENERATED. DO NOT MODIFY!! */\n\n"
            if fname.endswith(".kt")
            else "# THIS FILE IS AUTO-GENERATED. DO NOT MODIFY!!\n\n"
        )
        out = auto_comment + content

        out_path = out_dir / fname
        # 和 wry build.rs 一样：只在内容变化时写盘（避免不必要的重建）
        existing = out_path.read_text(encoding="utf-8") if out_path.is_file() else ""
        if existing != out:
            out_path.write_text(out, encoding="utf-8", newline="\n")
            written += 1
        else:
            # 内容一致，跳过（但还是输出文件，确保文件存在）
            if not out_path.is_file():
                out_path.write_text(out, encoding="utf-8", newline="\n")
                written += 1

    print(
        f"prepare-wry-kotlin: {written}/{len(WRY_KOTLIN_TEMPLATES)} 个文件 "
        f"→ {out_dir}"
    )
    return 0 if written > 0 or (out_dir / "RustWebViewClient.kt").is_file() else 1


if __name__ == "__main__":
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except Exception:  # noqa: BLE001
            pass
    sys.exit(main())
