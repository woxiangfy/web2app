#!/usr/bin/env python3
"""`cargo tauri android init` **之后**：把 APK 产物名改成带应用名的名字。

抽自 `build-custom.yml` 的内联 python —— 该能力原先**只有手动打包有**，发版（release.yml）
没有，于是两条流水线出的 APK 文件名不一致（一个 `MyApp-universal-release-unsigned.apk`，
一个通用的 `app-universal-release-unsigned.apk`）。现两壳共用本脚本，能力对齐。

默认 Tauri 模板的 APK 产物名是通用的 `app-universal-release-unsigned.apk`；这里把 gradle 的
`base.archivesName` 设为**清洗后的 app_name**（AGP 7.4+ 推荐写法；模板不支持 `base` 命名空间时
回退到 `archivesBaseName`），下载时一目了然。`gen/android` 每轮现生成，故每轮 init 后都要打。

用法（cwd = webview-app）：
    python3 ../.github/scripts/set-android-apk-name.py
    python3 .github/scripts/set-android-apk-name.py --selftest
"""

from __future__ import annotations

import argparse
import os
import re
import sys

from android_gen import app_gradle, clean_name, gen_root_candidates_str, resolve_gen_root

ANDROID_BLOCK = re.compile(r"(^|\n)(\s*)android\s*\{")
# 幂等判据：既认 `archivesBaseName`（旧写法 / 无 android 块时的追加），
# 也认注入所用的 `base { archivesName.set(...) }`。
# ⚠️ 不要只写 `base\s*\.\s*archivesName` —— 那是**点号**形式，而实际注入的是**代码块**形式，
#    判据匹配不上 ⇒ 每轮都会再注入一次（原生内联写法即因此不幂等）。
ALREADY_SET = re.compile(r"archivesName|archivesBaseName")


def patch_gradle(src: str, safe: str) -> str:
    """注入 archivesName；已设置过则原样返回。"""
    if ALREADY_SET.search(src):
        return src
    m = ANDROID_BLOCK.search(src)
    if not m:
        # 没有 android 块（不太可能）：退而求其次在文件末尾追加
        return src.rstrip() + f'\narchivesBaseName = "{safe}"\n'
    indent = m.group(2) + "    "
    inject = f'\n{indent}base {{ archivesName.set("{safe}") }}\n'
    pos = m.end()
    return src[:pos] + inject + src[pos:]


def main() -> int:
    ap = argparse.ArgumentParser(description="把 APK 产物文件名改成带应用名的名字")
    ap.add_argument("--gen-root", default=None,
                    help="Android 工程根（缺省自动探测：" + gen_root_candidates_str() + "）")
    ap.add_argument("--selftest", action="store_true", help="运行离线自测")
    args = ap.parse_args()
    if args.selftest:
        return selftest()

    name = (os.environ.get("APP_NAME") or "").strip()
    if not name:
        print("未配置 app_name，APK 沿用默认文件名")
        return 0
    safe = clean_name(name)

    g = app_gradle(resolve_gen_root(args.gen_root))
    if g is None:
        print("::warning::未找到 app/build.gradle(.kts)，跳过 APK 重命名")
        return 0

    src = g.read_text(encoding="utf-8")
    out = patch_gradle(src, safe)
    if out == src:
        print(f"{g.name} 已设置 archivesBaseName，跳过")
    else:
        g.write_text(out, encoding="utf-8")
        print(f"APK 产物名 → {safe}（{g.name} 已注入 archivesName）")
    return 0


def selftest() -> int:
    """离线自测：注入位置、幂等、无 android 块的回落、文件名清洗。"""
    src = (
        "plugins {\n    id(\"com.android.application\")\n}\n\n"
        "android {\n"
        "    compileSdk = 37\n"
        "}\n\n"
        "dependencies {\n}\n"
    )
    out = patch_gradle(src, "MyApp")
    assert 'base { archivesName.set("MyApp") }' in out, "应在 android 块内注入 base.archivesName"
    # 注入点必须在 android { 之后、compileSdk 之前
    assert out.index("android {") < out.index("archivesName") < out.index("compileSdk")
    # 原有内容保留
    assert 'compileSdk = 37' in out and 'dependencies {' in out

    # 幂等：已设置过不再注入
    assert patch_gradle(out, "MyApp") == out, "已设置 archivesName 时不得重复注入"
    pre_set = 'android {\n  archivesBaseName = "Old"\n}\n'
    assert patch_gradle(pre_set, "New") == pre_set, "archivesBaseName 已存在时不得再注入"

    # 无 android 块 → 追加到末尾
    out2 = patch_gradle("plugins {\n}\n", "X")
    assert out2.rstrip().endswith('archivesBaseName = "X"'), "无 android 块时应追加 archivesBaseName"

    # 清洗规则与 productName 同源
    assert clean_name('a/b:c*d?e"f<g>h|i') == "a_b_c_d_e_f_g_h_i"
    assert clean_name("   ") == "Web2App"

    print("set-android-apk-name.py 自测通过 ✓（4 组场景）")
    return 0


if __name__ == "__main__":
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except Exception:  # noqa: BLE001
            pass
    sys.exit(main())
