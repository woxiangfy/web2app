#!/usr/bin/env python3
"""Android 启动底色主题补丁：让**首帧之前**那一层跟随系统日夜（#13）。

## 背景（为什么需要）

`splash.html` / `loading.js` 早就随 `prefers-color-scheme` 切换（`#f7f8fa` / `#14181f`），
但窗口在 WebView 画出第一帧之前还有一层根 Activity 的底色 —— 它由**主题的
`android:windowBackground`** 决定。模板默认是浅色 ⇒ 暗色系统上启动会先闪一下白。

⚠️ **改 `nav::SPLASH_BG` 没用**（这是上一版计划的错误前提）：tao 0.37.1 的
`WindowBuilder::with_background_color` 文档写明「iOS / Android: Unsupported」，
Android 侧 `platform_impl/android/mod.rs` 是空实现 ⇒ 那条 Rust 路径在安卓上从未生效。
所以本补丁**只碰资源文件**，Rust 侧一行不改（桌面与测试构建完全不受影响）。

## 做法

1. 从 `AndroidManifest.xml` 取主题引用（先看 `<application>`，再看 `<activity>`）；
   引用必须是 `@style/X` —— 框架主题（`@android:style/...`）改不了，直接报错说明。
2. 在 `res/values/*.xml` 里找到 `<style name="X">`，写入/改写
   `<item name="android:windowBackground">@color/web2app_splash</item>`。
3. 写两份颜色资源：`res/values/web2app_splash.xml`（亮色）与
   `res/values-night/web2app_splash.xml`（暗色）。日夜切换由系统资源机制完成 ——
   不需要在代码里判断当前主题，也就没有「判断时机」这一类问题。

⚠️ 色值必须与 `src/splash.html` 的两个 `background` 逐字一致：契约测试
`android_pipeline_contract::splash_theme_colors_match_splash_html` 会把两边比一遍
（改一处忘另一处 = 启动页与窗口底色撕裂）。

## 为什么用「颜色资源 + values-night」而不是两份 themes.xml

`themes.xml` 里再写一遍整个 `<style>`，就得跟着模板的 parent 与其它 item 一起漂移；
只插一个 item + 两个一行颜色文件，模板升级时几乎不会失效。

补丁链固定顺序（见 docs/android-improvements.md §0）：
insets → back → download → extras → schemes → fullscreen → permissions → cleartext →
**splash-theme**。（本脚本只写 res 资源与主题 item，与 cleartext 写 res/xml 互不重叠。）

用法：
    python3 .github/scripts/patch-android-splash-theme.py            # 打补丁（自动探测工程根）
    python3 .github/scripts/patch-android-splash-theme.py --selftest # 离线自测
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys
import tempfile

from android_gen import (  # noqa: E402
    android_manifest,
    gen_root_candidates_str,
    res_dir,
    resolve_gen_root,
)

# 幂等标记（写在两个颜色资源文件头部；主题 item 靠「值相等」天然幂等）。
MARKER = "web2app-android-splash-theme"

# 与 src/splash.html 的两个 background **逐字一致**（契约测试比对）。
SPLASH_LIGHT = "#f7f8fa"
SPLASH_DARK = "#14181f"

COLOR_NAME = "web2app_splash"
COLOR_REF = f"@color/{COLOR_NAME}"
WINDOW_BG_ITEM = f'<item name="android:windowBackground">{COLOR_REF}</item>'

MANIFEST_THEME = re.compile(r'android:theme\s*=\s*"([^"]+)"')
STYLE_REF = re.compile(r"^@style/(.+)$")
STYLE_REF_FRAMEWORK = re.compile(r"^@android:style/(.+)$")
WINDOW_BG_EXISTING = re.compile(
    r'<item\s+name\s*=\s*"android:windowBackground"\s*>.*?</item>', re.S
)
STYLE_OPEN = re.compile(r'<style\s+name\s*=\s*"([^"]+)"')


def colors_xml(color: str, label: str) -> str:
    """生成一个颜色资源文件（`values` 与 `values-night` 各一份，仅色值不同）。"""
    return (
        '<?xml version="1.0" encoding="utf-8"?>\n'
        f"<!-- {MARKER}：{label}启动底色，与 src/splash.html 的 background 逐字一致。\n"
        "     改这里必须同步 splash.html，否则窗口底色与启动页会撕裂。 -->\n"
        "<resources>\n"
        f'    <color name="{COLOR_NAME}">{color}</color>\n'
        "</resources>\n"
    )


def pick_theme(manifest_src: str) -> str:
    """从 Manifest 取应用主题的 **style 名**；框架主题改不了，显式报错。"""
    m = MANIFEST_THEME.search(manifest_src)
    if not m:
        raise ValueError("AndroidManifest.xml 没有 android:theme —— 模板可能已变化")
    value = m.group(1).strip()
    if STYLE_REF_FRAMEWORK.match(value):
        raise ValueError(
            f"Manifest 用的是框架主题 {value}，无法在其上追加 item；"
            "需要模板改成自定义 @style/..."
        )
    ref = STYLE_REF.match(value)
    if not ref:
        raise ValueError(f"无法识别的主题引用：{value}（期望 @style/名字）")
    return ref.group(1)


def patch_style_src(src: str, style_name: str) -> str:
    """在指定 `<style>`（同名）里插入/改写 `android:windowBackground`（幂等）。

    三条分支：已有该 item → 只换其值；没有 → 插在 `</style>` 前（沿用文件的缩进风格）。
    找不到同名 style / 该 style 是自闭合形式 → 抛错（模板变了，不许静默跳过）。
    """
    for m in STYLE_OPEN.finditer(src):
        if m.group(1) != style_name:
            continue
        open_end = src.find(">", m.end())
        if open_end == -1:
            break
        close = src.find("</style>", open_end)
        if close == -1:
            break
        block = src[open_end + 1 : close]
        if WINDOW_BG_EXISTING.search(block):
            new_block = WINDOW_BG_EXISTING.sub(WINDOW_BG_ITEM, block, count=1)
        else:
            # block 形如 "\n        <item …/>\n    "（**末尾那段空白就是 </style> 的缩进**，
            # 必须原样留住，否则插完会把闭合标签顶到行首、整个文件的缩进都歪掉）。
            tail = re.search(r"\n([ \t]*)$", block)
            close_indent = tail.group(1) if tail else ""
            body = block[: tail.start()] if tail else block
            indent_m = re.search(r"\n([ \t]+)<item", body)
            indent = indent_m.group(1) if indent_m else close_indent + "    "
            new_block = body.rstrip() + "\n" + indent + WINDOW_BG_ITEM + "\n" + close_indent
        return src[: open_end + 1] + new_block + src[close:]

    # 自闭合 `<style name="X" />`：没有可插 item 的位置。
    if re.search(r'<style\s+name\s*=\s*"' + re.escape(style_name) + r'"[^>]*/>', src):
        raise ValueError(f"{style_name} 是自闭合 <style/>，无法追加 item")
    raise ValueError(f"在资源文件里找不到 <style name=\"{style_name}\">")


def find_style_file(values_dir: pathlib.Path, style_name: str) -> pathlib.Path:
    """在 `res/values/*.xml` 里找定义该 style 的文件（模板用 themes.xml，但别写死）。"""
    exact = values_dir / "themes.xml"
    ordered = [exact] if exact.is_file() else []
    ordered += sorted(p for p in values_dir.glob("*.xml") if p != exact)
    for p in ordered:
        text = p.read_text(encoding="utf-8", errors="replace")
        if re.search(r'<style\s+name\s*=\s*"' + re.escape(style_name) + r'"', text):
            return p
    raise FileNotFoundError(
        f"{values_dir}/*.xml 里没有 <style name=\"{style_name}\"> —— "
        "若 tauri 模板改了主题组织方式，需要更新本脚本"
    )


# ---------------------------------------------------------------------------
# 自测
# ---------------------------------------------------------------------------
MANIFEST_APP_THEME = (
    '<?xml version="1.0" encoding="utf-8"?>\n'
    '<manifest xmlns:android="http://schemas.android.com/apk/res/android">\n'
    '    <application android:label="Web2App" android:theme="@style/Theme.web2app">\n'
    '        <activity android:name=".MainActivity" />\n'
    "    </application>\n"
    "</manifest>\n"
)
MANIFEST_ACTIVITY_THEME = (
    '<?xml version="1.0" encoding="utf-8"?>\n'
    '<manifest xmlns:android="http://schemas.android.com/apk/res/android">\n'
    '    <application android:label="Web2App">\n'
    '        <activity android:name=".MainActivity" android:theme="@style/Theme.web2app" />\n'
    "    </application>\n"
    "</manifest>\n"
)
MANIFEST_FRAMEWORK_THEME = (
    '<?xml version="1.0" encoding="utf-8"?>\n'
    '<manifest xmlns:android="http://schemas.android.com/apk/res/android">\n'
    '    <application android:theme="@android:style/Theme.Material.Light.NoActionBar" />\n'
    "</manifest>\n"
)
MANIFEST_NO_THEME = (
    '<?xml version="1.0" encoding="utf-8"?>\n'
    '<manifest xmlns:android="http://schemas.android.com/apk/res/android">\n'
    '    <application android:label="Web2App" />\n'
    "</manifest>\n"
)
THEMES_PLAIN = (
    '<?xml version="1.0" encoding="utf-8"?>\n'
    "<resources>\n"
    '    <style name="Theme.web2app" parent="Theme.MaterialComponents.DayNight.NoActionBar">\n'
    '        <item name="android:windowActionModeOverlay">true</item>\n'
    "    </style>\n"
    "</resources>\n"
)
THEMES_WITH_WINDOW_BG = (
    '<?xml version="1.0" encoding="utf-8"?>\n'
    "<resources>\n"
    '    <style name="Theme.web2app" parent="Theme.MaterialComponents.DayNight.NoActionBar">\n'
    '        <item name="android:windowBackground">@android:color/white</item>\n'
    "    </style>\n"
    "</resources>\n"
)
THEMES_OTHER_STYLE = (
    '<?xml version="1.0" encoding="utf-8"?>\n'
    "<resources>\n"
    '    <style name="Theme.other" parent="Theme.MaterialComponents.DayNight.NoActionBar">\n'
    "    </style>\n"
    "</resources>\n"
)
# 空 style（一个 item 都没有）：缩进只能从闭合标签推。
THEMES_EMPTY_STYLE = (
    '<?xml version="1.0" encoding="utf-8"?>\n'
    "<resources>\n"
    '    <style name="Theme.web2app" parent="Theme.MaterialComponents.DayNight.NoActionBar">\n'
    "    </style>\n"
    "</resources>\n"
)


def selftest() -> int:
    # 1) 取主题名：application 优先，其次 activity
    assert pick_theme(MANIFEST_APP_THEME) == "Theme.web2app"
    assert pick_theme(MANIFEST_ACTIVITY_THEME) == "Theme.web2app"

    # 2) 框架主题 / 没有主题 → 显式报错（不静默跳过）
    for bad in (MANIFEST_FRAMEWORK_THEME, MANIFEST_NO_THEME):
        try:
            pick_theme(bad)
            raise AssertionError(f"应报错：{bad[:60]}")
        except ValueError:
            pass

    # 3) 主题里没有 windowBackground → 插在 </style> 前，缩进沿用现有 item
    out = patch_style_src(THEMES_PLAIN, "Theme.web2app")
    assert WINDOW_BG_ITEM in out, f"未插入 item：{out}"
    assert out.index(WINDOW_BG_ITEM) < out.index("</style>"), "item 必须落在 style 内"
    assert "        " + WINDOW_BG_ITEM in out, "缩进应与同 style 内其它 item 一致"
    # 闭合标签的缩进不得被吃掉（插 item 最容易犯的错：`</style>` 被顶到行首）
    assert "\n    </style>" in out, f"</style> 缩进丢失：{out!r}"
    assert "\n        <item name=\"android:windowActionModeOverlay\">true</item>" in out, (
        "原 item 必须原样保留"
    )

    # 4) 幂等：再打一次逐字不变
    assert patch_style_src(out, "Theme.web2app") == out, "补丁不是幂等的"

    # 5) 已有 windowBackground（模板自带的浅色）→ 换值，不叠加第二份
    out2 = patch_style_src(THEMES_WITH_WINDOW_BG, "Theme.web2app")
    assert out2.count("android:windowBackground") == 1, "不得叠加第二个 windowBackground"
    assert "@android:color/white" not in out2, "旧值必须被替换"
    assert WINDOW_BG_ITEM in out2
    assert patch_style_src(out2, "Theme.web2app") == out2, "替换分支也必须幂等"

    # 6) 同名 style 不存在 → 抛错（模板改了就必须显式失败，别让底色静默不生效）
    try:
        patch_style_src(THEMES_OTHER_STYLE, "Theme.web2app")
        raise AssertionError("缺同名 style 应报错")
    except ValueError:
        pass

    # 6b) 空 style（无 item）：缩进从闭合标签推，结果仍要合法且幂等
    out_empty = patch_style_src(THEMES_EMPTY_STYLE, "Theme.web2app")
    assert "        " + WINDOW_BG_ITEM + "\n    </style>" in out_empty, (
        f"空 style 的缩进处理不对：{out_empty!r}"
    )
    assert patch_style_src(out_empty, "Theme.web2app") == out_empty, "空 style 分支也必须幂等"

    # 7) 颜色资源内容：两份文件、色值与常量一致、带 marker
    light = colors_xml(SPLASH_LIGHT, "亮色")
    dark = colors_xml(SPLASH_DARK, "暗色")
    assert MARKER in light and MARKER in dark
    assert f'<color name="{COLOR_NAME}">{SPLASH_LIGHT}</color>' in light
    assert f'<color name="{COLOR_NAME}">{SPLASH_DARK}</color>' in dark

    # 8) 文件级集成：临时 gen/android 走一遍（application 主题 + themes.xml）
    with tempfile.TemporaryDirectory() as tmp:
        gen_root = pathlib.Path(tmp) / "gen" / "android"
        manifest = android_manifest(gen_root)
        manifest.parent.mkdir(parents=True, exist_ok=True)
        manifest.write_text(MANIFEST_APP_THEME, encoding="utf-8", newline="\n")

        values = res_dir(gen_root) / "values"
        values.mkdir(parents=True, exist_ok=True)
        (values / "themes.xml").write_text(THEMES_PLAIN, encoding="utf-8", newline="\n")

        style_name = pick_theme(manifest.read_text(encoding="utf-8"))
        target = find_style_file(values, style_name)
        assert target.name == "themes.xml"
        target.write_text(
            patch_style_src(target.read_text(encoding="utf-8"), style_name),
            encoding="utf-8",
            newline="\n",
        )
        night = res_dir(gen_root) / "values-night"
        night.mkdir(parents=True, exist_ok=True)
        (values / "web2app_splash.xml").write_text(
            colors_xml(SPLASH_LIGHT, "亮色"), encoding="utf-8", newline="\n"
        )
        (night / "web2app_splash.xml").write_text(
            colors_xml(SPLASH_DARK, "暗色"), encoding="utf-8", newline="\n"
        )

        assert WINDOW_BG_ITEM in target.read_text(encoding="utf-8")
        assert (night / "web2app_splash.xml").is_file()

    print("patch-android-splash-theme.py 自测通过 ✓（8 组场景）")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Android 启动底色：主题 windowBackground 跟随系统日夜（只改资源）"
    )
    parser.add_argument(
        "--gen-root",
        default=None,
        help="Android 工程根（缺省按候选自动探测：" + gen_root_candidates_str() + "）",
    )
    parser.add_argument("--selftest", action="store_true", help="运行离线自测")
    args = parser.parse_args()

    if args.selftest:
        return selftest()

    gen_root = resolve_gen_root(args.gen_root)
    manifest = android_manifest(gen_root)
    if not manifest.is_file():
        raise SystemExit(
            f"未找到 {manifest}（请确认已先执行 cargo tauri android init；"
            f"已尝试工程根：{gen_root_candidates_str()}）"
        )

    style_name = pick_theme(manifest.read_text(encoding="utf-8"))
    values = res_dir(gen_root) / "values"
    night = res_dir(gen_root) / "values-night"

    style_file = find_style_file(values, style_name)
    src = style_file.read_text(encoding="utf-8")
    out = patch_style_src(src, style_name)
    if out == src:
        print(f"{style_file} 已设置 {COLOR_REF}，跳过")
    else:
        style_file.write_text(out, encoding="utf-8", newline="\n")
        print(f"已给 {style_file} 的 {style_name} 设置 windowBackground = {COLOR_REF}")

    for d, color, label in ((values, SPLASH_LIGHT, "亮色"), (night, SPLASH_DARK, "暗色")):
        d.mkdir(parents=True, exist_ok=True)
        p = d / "web2app_splash.xml"
        want = colors_xml(color, label)
        if p.is_file() and p.read_text(encoding="utf-8") == want:
            print(f"{p} 已是目标内容，跳过")
        else:
            p.write_text(want, encoding="utf-8", newline="\n")
            print(f"已写入 {p}（{color}）")
    return 0


if __name__ == "__main__":
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except Exception:  # noqa: BLE001
            pass
    sys.exit(main())
