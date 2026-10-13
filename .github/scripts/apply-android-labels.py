#!/usr/bin/env python3
"""`cargo tauri android init` **之后**：把应用显示名写进 strings.xml 与 AndroidManifest.xml。

抽自 `build-custom.yml` / `release.yml` 的内联 python（两壳各一份，逻辑等价）。

显示名取环境变量 `TITLE`，未配置时回落 `APP_NAME` —— 与 Windows 窗口标题同源；`app_name`
只负责「安装名」（安装包 / 快捷方式 / 安装目录）。两者分离后可实现「安装名用英文、显示名用中文」。

⚠️ 执行顺序：`prepare-android-conf.py` 已让 init 把**安装名**烘焙进 strings.xml，本脚本在 init
之后用 `TITLE` 覆盖，因此最终生效的是 `TITLE`。`gen/android` 每轮现生成，故每轮 init 后都要打。

用法（cwd = webview-app）：
    python3 ../.github/scripts/apply-android-labels.py
    python3 .github/scripts/apply-android-labels.py --selftest
"""

from __future__ import annotations

import argparse
import os
import re
import sys

from android_gen import (
    android_manifest,
    gen_root_candidates_str,
    resolve_gen_root,
    strings_xml,
)

# application 的 label 与 MainActivity 的 label（后者才是桌面图标/任务栏真正显示的名字）。
# tauri v2 模板里两者都等于 productName，必须【都】改成显示名，否则桌面仍显示安装名。
RES_NAMES = ("app_name", "main_activity_title")


def esc_res(v: str) -> str:
    """Android 资源字符串转义。

    单引号必须转义为 `\\'`，否则 aapt 报 "Apostrophe not preceded by backslash"
    （gradle assemble 阶段直接挂）。⚠️ 不能用 xml.sax.saxutils.escape —— 它只转义 & < >，
    不处理单引号。
    """
    return (v.replace("&", "&amp;")
             .replace("<", "&lt;")
             .replace(">", "&gt;")
             .replace("'", "\\'"))


def patch_strings(src: str, label: str) -> tuple[str, list[str]]:
    """改写（或补齐）两条应用名资源；返回 (新内容, 每个资源名的结果说明)。"""
    logs: list[str] = []
    for res_name in RES_NAMES:
        # 容忍 name 上可能带额外属性（如 translatable="false"）
        pat = rf'(<string\s+name="{res_name}"(?:\s+[\w:-]+="[^"]*")*\s*>)[^<]*(</string>)'
        src, n = re.subn(pat, lambda m: m.group(1) + label + m.group(2), src)
        if n == 0 and "</resources>" in src:
            src = src.replace("</resources>",
                              f'    <string name="{res_name}">{label}</string>\n</resources>')
            n = 1
        logs.append(f"{res_name}: {'覆盖' if n else '未找到，已跳过'}")
    return src, logs


def patch_manifest(src: str, label: str) -> tuple[str, int]:
    """Manifest 若用内联 label（而非 @string/app_name 引用）则替换；返回 (新内容, 替换数)。"""
    return re.subn(r'(android:label=")(?!@string/)[^"]*(")',
                   lambda m: m.group(1) + label + m.group(2), src)


def main() -> int:
    ap = argparse.ArgumentParser(description="把显示名写进安卓 strings.xml / AndroidManifest.xml")
    ap.add_argument("--gen-root", default=None,
                    help="Android 工程根（缺省自动探测：" + gen_root_candidates_str() + "）")
    ap.add_argument("--selftest", action="store_true", help="运行离线自测")
    args = ap.parse_args()
    if args.selftest:
        return selftest()

    name = (os.environ.get("TITLE") or "").strip() or (os.environ.get("APP_NAME") or "").strip()
    if not name:
        print("未配置 TITLE / APP_NAME，Android 沿用默认名称")
        return 0
    label = esc_res(name)

    gen_root = resolve_gen_root(args.gen_root)

    res = strings_xml(gen_root)
    if res.exists():
        s2, logs = patch_strings(res.read_text(encoding="utf-8"), label)
        res.write_text(s2, encoding="utf-8")
        for line in logs:
            print(f"strings.xml: {line} → {name}")
    else:
        print("::warning::未找到 strings.xml")

    mf = android_manifest(gen_root)
    if mf.exists():
        s2, n = patch_manifest(mf.read_text(encoding="utf-8"), label)
        if n:
            mf.write_text(s2, encoding="utf-8")
            print(f"AndroidManifest: 内联 android:label → {name}")
    return 0


def selftest() -> int:
    """离线自测：转义、两条资源覆盖/补齐、Manifest 内联与 @string/ 引用。"""
    # 1) 资源字符串转义
    assert esc_res("Tom's & <App>") == "Tom\\'s &amp; &lt;App&gt;"

    # 2) 覆盖已有两条资源
    src = (
        '<resources>\n'
        '    <string name="app_name">Web2App</string>\n'
        '    <string name="main_activity_title">Web2App</string>\n'
        '</resources>\n'
    )
    out, logs = patch_strings(src, "我的应用")
    assert out.count(">我的应用<") == 2, "两条资源都必须被覆盖"
    assert all("覆盖" in x for x in logs), f"应报告为覆盖：{logs}"

    # 3) 带额外属性（translatable）也要能覆盖
    src2 = '<resources>\n    <string name="app_name" translatable="false">X</string>\n</resources>\n'
    out2, _ = patch_strings(src2, "Y")
    assert 'name="app_name" translatable="false">Y<' in out2, "带属性的资源行必须能覆盖"

    # 4) 缺资源时补齐到 </resources> 之前
    src3 = '<resources>\n</resources>\n'
    out3, logs3 = patch_strings(src3, "Z")
    assert out3.count('<string name=') == 2, "缺的两条都要补齐"
    assert out3.rstrip().endswith("</resources>"), "补齐后 </resources> 仍须是最后一行"
    assert all("覆盖" in x for x in logs3)

    # 5) Manifest：内联 label 替换，@string/ 引用不动
    mf_inline = '<application android:label="Web2App">'
    out4, n4 = patch_manifest(mf_inline, "显示名")
    assert n4 == 1 and 'android:label="显示名"' in out4
    mf_ref = '<application android:label="@string/app_name">'
    out5, n5 = patch_manifest(mf_ref, "显示名")
    assert n5 == 0 and out5 == mf_ref, "@string/ 引用不得被内联替换"

    print("apply-android-labels.py 自测通过 ✓（5 组场景）")
    return 0


if __name__ == "__main__":
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except Exception:  # noqa: BLE001
            pass
    sys.exit(main())
