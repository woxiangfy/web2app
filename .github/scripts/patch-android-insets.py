#!/usr/bin/env python3
"""`cargo tauri android init` **之后**：适配系统状态栏 / 导航栏，避免页面被顶部通栏遮挡。

抽自 `build-custom.yml` / `release.yml` 的内联 python（两壳各一份，实测两份逐字节相同）。

Tauri 模板的 MainActivity 调用了 `enableEdgeToEdge()`：WebView 会延伸绘制到状态栏 / 导航栏下方，
页面顶部被系统通栏遮挡。这里【保留原文件结构】，只做外科手术式注入：
  1. 补 androidx.core 的 3 个 import（若模板已有则跳过）；
  2. 在 `super.onCreate(...)` 之后注册 WindowInsets 监听，让内容视图按 systemBars 实际高度内缩
     —— edge-to-edge 仍生效（应用全屏），但页面内容不被系统通栏遮挡。

采用「只注入、不重写」的方式，避免破坏模板原有的 import / 基类写法，兼容性更好。
`gen/android` 每轮现生成，因此每轮 init 后都要打一遍。

用法（cwd = webview-app）：
    python3 ../.github/scripts/patch-android-insets.py
    python3 .github/scripts/patch-android-insets.py --selftest
"""

from __future__ import annotations

import argparse
import re
import sys

from android_gen import app_gradle, find_main_activity, gen_root_candidates_str, resolve_gen_root

# 幂等标记：模板改过或已注入过时不再重复注入（模板自带时也会命中这个判据）。
IDEMPOTENT_NEEDLE = "setOnApplyWindowInsetsListener"

# 需要补的 import（缺失才加）。
NEEDED_IMPORTS = (
    "import androidx.core.graphics.Insets",
    "import androidx.core.view.ViewCompat",
    "import androidx.core.view.WindowInsetsCompat",
)

# androidx.core 兜底依赖（模板的 androidx.activity/appcompat 一般已传递引入）。
ANDROIDX_DEP_LINE = '    implementation("androidx.core:core-ktx:1.9.0")'

IMPORT_LINE = re.compile(r"^import .*$", re.M)
PACKAGE_LINE = re.compile(r"^package .*$", re.M)
SUPER_ONCREATE = re.compile(r"super\.onCreate\s*\([^)]*\)\s*;?")


def inset_listener(is_kotlin: bool) -> str:
    """按语言给出「按 systemBars 内缩」的监听代码块（缩进对齐模板的 8 空格）。"""
    if is_kotlin:
        return (
            "        ViewCompat.setOnApplyWindowInsetsListener(findViewById(android.R.id.content)) { v, insets ->\n"
            "            val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars())\n"
            "            v.setPadding(bars.left, bars.top, bars.right, bars.bottom)\n"
            "            insets\n"
            "        }\n"
        )
    return (
        "        ViewCompat.setOnApplyWindowInsetsListener(findViewById(android.R.id.content),\n"
        "                (v, insets) -> {\n"
        "                    Insets bars = insets.getInsets(WindowInsetsCompat.Type.systemBars());\n"
        "                    v.setPadding(bars.left, bars.top, bars.right, bars.bottom);\n"
        "                    return insets;\n"
        "                });\n"
    )


def patch_src(src: str, is_kotlin: bool) -> str:
    """给 MainActivity 源码注入 insets 适配；已包含则原样返回。"""
    if IDEMPOTENT_NEEDLE in src:
        return src

    # 1) 补 import（缺失才加；插到最后一条 import 之后，若完全没有 import 则插在 package 之后）
    for imp in NEEDED_IMPORTS:
        if imp in src:
            continue
        imports = list(IMPORT_LINE.finditer(src))
        if imports:
            pos = imports[-1].end()
            src = src[:pos] + "\n" + imp + src[pos:]
        else:
            mp = PACKAGE_LINE.search(src)
            pos = mp.end() if mp else 0
            src = (src[:pos] + "\n\n" + imp + src[pos:]) if pos else (imp + "\n" + src)

    # 2) 在 super.onCreate(...) 之后注入监听
    if not SUPER_ONCREATE.search(src):
        raise ValueError("MainActivity 中未找到 super.onCreate 调用，模板可能已变化，请人工确认")
    return SUPER_ONCREATE.sub(lambda m: m.group(0) + "\n" + inset_listener(is_kotlin), src, count=1)


def ensure_androidx_core(text: str) -> str:
    """gradle 文本已含 androidx.core / appcompat 则原样返回；否则在 dependencies{} 补一行。"""
    if ("androidx.core" in text) or ("appcompat" in text):
        return text
    out, n = re.subn(r"(dependencies\s*\{)",
                     lambda mm: mm.group(1) + "\n" + ANDROIDX_DEP_LINE, text, count=1)
    return out if n else text


def main() -> int:
    ap = argparse.ArgumentParser(description="给 MainActivity 注入系统栏 insets 适配")
    ap.add_argument("--gen-root", default=None,
                    help="Android 工程根（缺省自动探测：" + gen_root_candidates_str() + "）")
    ap.add_argument("--selftest", action="store_true", help="运行离线自测")
    args = ap.parse_args()
    if args.selftest:
        return selftest()

    gen_root = resolve_gen_root(args.gen_root)
    f = find_main_activity(gen_root)
    src = f.read_text(encoding="utf-8")
    out = patch_src(src, f.suffix == ".kt")
    if out == src:
        print("MainActivity 已包含系统栏适配，跳过改写")
    else:
        f.write_text(out, encoding="utf-8")
        print(f"已改写 {f}（内容视图按系统栏 insets 内缩，避免顶部通栏遮挡）")

    g = app_gradle(gen_root)
    if g is not None:
        t = g.read_text(encoding="utf-8")
        t2 = ensure_androidx_core(t)
        if t2 != t:
            g.write_text(t2, encoding="utf-8")
            print(f"{g.name} 已补充 androidx.core 依赖")
        else:
            print(f"{g.name} 已含 androidx.core/appcompat 依赖")
    return 0


def selftest() -> int:
    """离线自测：Kotlin / Java 注入、幂等、结构保留、gradle 兜底依赖。"""
    kt = (
        "package com.web2app.app\n\n"
        "import android.os.Bundle\n"
        "import androidx.activity.enableEdgeToEdge\n\n"
        "class MainActivity : TauriActivity() {\n"
        "  override fun onCreate(savedInstanceState: Bundle?) {\n"
        "    enableEdgeToEdge()\n"
        "    super.onCreate(savedInstanceState)\n"
        "  }\n"
        "}\n"
    )
    java = (
        "package com.web2app.app;\n\n"
        "import android.os.Bundle;\n\n"
        "public class MainActivity extends TauriActivity {\n"
        "  @Override\n"
        "  public void onCreate(Bundle savedInstanceState) {\n"
        "    super.onCreate(savedInstanceState);\n"
        "  }\n"
        "}\n"
    )

    # 1) Kotlin：要素齐全 + 原有内容保留
    out = patch_src(kt, True)
    for needle in NEEDED_IMPORTS + (
        "ViewCompat.setOnApplyWindowInsetsListener(findViewById(android.R.id.content)) { v, insets ->",
        "bars.top",
        "insets\n",
    ):
        assert needle in out, f"Kotlin 注入缺少要素：{needle}"
    assert "enableEdgeToEdge()" in out and "super.onCreate(savedInstanceState)" in out

    # 2) 幂等
    assert patch_src(out, True) == out, "Kotlin 补丁不是幂等的"

    # 3) 注入位置：必须在 super.onCreate 之后
    assert out.index("super.onCreate") < out.index("setOnApplyWindowInsetsListener")

    # 4) Java 分支用 Java 语法
    outj = patch_src(java, False)
    assert "Insets bars = insets.getInsets(WindowInsetsCompat.Type.systemBars());" in outj
    assert "return insets;" in outj
    assert "super.onCreate(savedInstanceState);" in outj
    assert patch_src(outj, False) == outj, "Java 补丁不是幂等的"

    # 5) 无 super.onCreate → 报错
    try:
        patch_src("package x\n\nclass MainActivity : TauriActivity() {\n}\n", True)
        raised = False
    except ValueError:
        raised = True
    assert raised, "缺 super.onCreate 时应报错"

    # 6) gradle 兜底依赖
    g = "android {\n}\n\ndependencies {\n}\n"
    assert ANDROIDX_DEP_LINE in ensure_androidx_core(g), "缺依赖时应补 androidx.core"
    assert ensure_androidx_core("dependencies {\n    implementation(\"androidx.appcompat:appcompat:1.6.1\")\n}\n") \
        == "dependencies {\n    implementation(\"androidx.appcompat:appcompat:1.6.1\")\n}\n", \
        "已有 appcompat 时不应重复补"

    print("patch-android-insets.py 自测通过 ✓（6 组场景）")
    return 0


if __name__ == "__main__":
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except Exception:  # noqa: BLE001
            pass
    sys.exit(main())
