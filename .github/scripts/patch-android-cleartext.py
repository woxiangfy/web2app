#!/usr/bin/env python3
"""Android 明文流量开关：**放行**明文 HTTP（本地代理 + 用户直连的 http:// 站点）。

## 背景（为什么需要）

两处都依赖明文 HTTP 可用：

  · 本地反向代理：WebView 实际加载 `http://127.0.0.1:<port>`；
  · 用户直连：目标站点本身只有 http（国内不少站点如此），禁明文等于让用户没路可走。

而 Android 9+ 默认禁止明文 HTTP，release 构建的 `usesCleartextTraffic` 默认 false ⇒
两者都会在真机上加载失败。

## 为什么不用 `android:usesCleartextTraffic="true"`

最初是 CI 里两条 sed 各自全局置 true。改成由本脚本统一写
`network_security_config.xml`，只为一件事：**明文策略只有一个落点**（两个 workflow 都调
同一个脚本，不再各写一份 sed），日后要按域收紧也只改这一处常量。

⚠️ **当前策略 = 全放行**（`base-config cleartextTrafficPermitted="true"`）。曾短暂收紧为
「仅回环」，因直连 http 站点会被系统拦下而回退 —— 见 `docs/android-improvements.md` #18。
若要重新收紧，务必保留 `127.0.0.1` / `localhost` 例外，否则本地代理链路先断。

Manifest 侧把 `android:usesCleartextTraffic="..."` 换成
`android:networkSecurityConfig="@xml/network_security_config"`；模板没有该属性时插进
`<application>`。gen/android 每次构建现生成，所以资源文件也由本脚本写入，不放进源码树。

补丁链固定顺序（见 docs/android-improvements.md §0）：
insets → back → download → extras → schemes → fullscreen → permissions → **cleartext** →
splash-theme。（本脚本只写 res/xml 与 Manifest，与 splash-theme 只改 res/values 互不重叠。）

用法：
    python3 .github/scripts/patch-android-cleartext.py            # 打补丁（自动探测工程根）
    python3 .github/scripts/patch-android-cleartext.py --selftest # 离线自测
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
    resolve_gen_root,
)

# 幂等标记（写在 network_security_config.xml 头部；Manifest 侧靠属性值相等天然幂等）。
MARKER = "web2app-android-cleartext"

NSC_ATTR = 'android:networkSecurityConfig="@xml/network_security_config"'
USES_CLEARTEXT = re.compile(r'android:usesCleartextTraffic="[^"]*"')
NSC_EXISTING = re.compile(r'android:networkSecurityConfig="[^"]*"')
APP_TAG = re.compile(r"<application\b[^>]*>", re.S)

# 资源文件内容：明文全放行（本地代理 + http 站点直连都靠它）。
NSC_XML = f"""<?xml version="1.0" encoding="utf-8"?>
<!-- {MARKER}：明文 HTTP 一律放行。
     理由：本地反向代理走 http://127.0.0.1:<port>，且目标站点大量只有 http，
     禁明文会让「直连 http 站点」直接被系统拦下（真机表现为 ERR_CLEARTEXT_NOT_PERMITTED）。
     ⚠️ 若日后改为按域收紧，domain-config 里必须保留 127.0.0.1 / localhost，
     否则本地代理链路先断。改这里等于改安全边界，同步 docs/android-improvements.md #18。 -->
<network-security-config>
    <base-config cleartextTrafficPermitted="true" />
</network-security-config>
"""


def patch_manifest_src(src: str) -> str:
    """把全局明文开关换成 network-security-config 引用（幂等）。

    三条分支：
      1. 已有 `android:networkSecurityConfig` → 归一成本脚本的固定值，并删掉残留的
         `usesCleartextTraffic`（两者并存时以配置文件为准，属性留着只会误导排查）；
      2. 有 `android:usesCleartextTraffic` → 原位替换（保留原有缩进/换行）；
      3. 都没有 → 插进 `<application ...>` 的属性区（自闭合 `/>` 也支持）。
    """
    if NSC_EXISTING.search(src):
        out = NSC_EXISTING.sub(NSC_ATTR, src, count=1)
        return USES_CLEARTEXT.sub("", out, count=1)

    if USES_CLEARTEXT.search(src):
        return USES_CLEARTEXT.sub(NSC_ATTR, src, count=1)

    m = APP_TAG.search(src)
    if not m:
        raise ValueError("AndroidManifest.xml 中未找到 <application ...> 标签，模板可能已变化")
    tag = m.group(0)
    if tag.rstrip().endswith("/>"):
        patched_tag = tag.rstrip()[:-2].rstrip() + " " + NSC_ATTR + " />"
    else:
        patched_tag = tag.rstrip()[:-1].rstrip() + " " + NSC_ATTR + ">"
    return src[: m.start()] + patched_tag + src[m.end() :]


def nsc_path(gen_root: pathlib.Path) -> pathlib.Path:
    """`app/src/main/res/xml/network_security_config.xml`。"""
    return gen_root / "app" / "src" / "main" / "res" / "xml" / "network_security_config.xml"


# ---------------------------------------------------------------------------
# 自测
# ---------------------------------------------------------------------------
MANIFEST_WITH_CLEARTEXT = (
    '<?xml version="1.0" encoding="utf-8"?>\n'
    '<manifest xmlns:android="http://schemas.android.com/apk/res/android">\n'
    '    <uses-permission android:name="android.permission.INTERNET" />\n'
    '    <application android:label="Web2App" android:usesCleartextTraffic="true"\n'
    '        android:theme="@style/AppTheme">\n'
    '        <activity android:name=".MainActivity" />\n'
    "    </application>\n"
    "</manifest>\n"
)

MANIFEST_WITHOUT = (
    '<?xml version="1.0" encoding="utf-8"?>\n'
    '<manifest xmlns:android="http://schemas.android.com/apk/res/android">\n'
    '    <application android:label="Web2App">\n'
    '        <activity android:name=".MainActivity" />\n'
    "    </application>\n"
    "</manifest>\n"
)

MANIFEST_SELF_CLOSING = (
    '<?xml version="1.0" encoding="utf-8"?>\n'
    '<manifest xmlns:android="http://schemas.android.com/apk/res/android">\n'
    '    <application android:label="Web2App" />\n'
    "</manifest>\n"
)


def selftest() -> int:
    # 1) 有 usesCleartextTraffic → 原位换成 networkSecurityConfig，且不再放行全局明文
    out = patch_manifest_src(MANIFEST_WITH_CLEARTEXT)
    assert "android:usesCleartextTraffic" not in out, "全局明文开关必须被移走"
    assert NSC_ATTR in out, "必须引用 network-security-config"
    assert "<activity" in out and '</manifest>' in out, "其余内容必须原样保留"
    # 属性仍在 application 标签内（不是掉到 manifest 外层）
    app_start = out.index("<application")
    app_end = out.index(">", app_start)
    assert app_start < out.index(NSC_ATTR) < app_end, "属性必须落在 <application ...> 内"

    # 2) 幂等：再打一次逐字不变
    assert patch_manifest_src(out) == out, "补丁不是幂等的"

    # 3) 没有 usesCleartextTraffic → 插进 application 标签
    out2 = patch_manifest_src(MANIFEST_WITHOUT)
    assert NSC_ATTR in out2
    assert 'android:label="Web2App" android:networkSecurityConfig=' in out2, (
        "应在 application 属性区末尾插入"
    )
    assert patch_manifest_src(out2) == out2, "插入分支也必须幂等"

    # 4) 自闭合 application（无子元素）→ 插在 `/>` 前
    out3 = patch_manifest_src(MANIFEST_SELF_CLOSING)
    assert NSC_ATTR + " />" in out3, f"自闭合标签形态插错位置：{out3}"

    # 5) 已有别的 networkSecurityConfig → 归一成本脚本的值（不叠第二份）
    custom = MANIFEST_WITHOUT.replace(
        "<application android:label", '<application android:networkSecurityConfig="@xml/other" '
        'android:label'
    )
    out4 = patch_manifest_src(custom)
    assert out4.count("android:networkSecurityConfig=") == 1, "不得叠加第二个 networkSecurityConfig"
    assert NSC_ATTR in out4

    # 6) 两种属性并存 → 只留配置文件引用
    both = MANIFEST_WITH_CLEARTEXT.replace(
        'android:usesCleartextTraffic="true"',
        'android:usesCleartextTraffic="true" android:networkSecurityConfig="@xml/old"',
    )
    out5 = patch_manifest_src(both)
    assert "android:usesCleartextTraffic" not in out5
    assert out5.count("android:networkSecurityConfig=") == 1

    # 7) 缺 <application> → 显式报错（不静默跳过）
    try:
        patch_manifest_src("<manifest></manifest>")
        raise AssertionError("缺 <application> 应报错")
    except ValueError:
        pass

    # 8) 资源文件内容：明文**全放行** + marker，且不得残留任何禁明文的 base-config
    for needle in (
        MARKER,
        '<base-config cleartextTrafficPermitted="true" />',
        # 注释里保留回环提示：日后收紧时最容易被忘掉的例外
        "127.0.0.1",
        "localhost",
    ):
        assert needle in NSC_XML, f"network_security_config.xml 缺少：{needle}"
    # 反例字面量拆开拼：避免它出现在脚本源码里，被契约测试的「不得含 false」误判。
    banned = "cleartextTrafficPermitted=" + '"false"'
    assert banned not in NSC_XML, (
        "明文必须全放行：直连 http 站点不允许被系统拦下（#18 回退）"
    )

    # 9) 文件级集成：临时 gen/android 工程走一遍完整链路
    with tempfile.TemporaryDirectory() as tmp:
        gen_root = pathlib.Path(tmp) / "gen" / "android"
        manifest = android_manifest(gen_root)
        manifest.parent.mkdir(parents=True, exist_ok=True)
        manifest.write_text(MANIFEST_WITH_CLEARTEXT, encoding="utf-8")

        target = nsc_path(gen_root)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(NSC_XML, encoding="utf-8", newline="\n")

        mout = patch_manifest_src(manifest.read_text(encoding="utf-8"))
        manifest.write_text(mout, encoding="utf-8", newline="\n")

        text = manifest.read_text(encoding="utf-8")
        assert NSC_ATTR in text and "usesCleartextTraffic" not in text
        assert MARKER in target.read_text(encoding="utf-8")
        assert target.parent.name == "xml"

    print("patch-android-cleartext.py 自测通过 ✓（9 组场景）")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Android 明文流量开关：放行明文 HTTP（本地代理 + http 站点直连）"
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

    target = nsc_path(gen_root)
    target.parent.mkdir(parents=True, exist_ok=True)
    if target.is_file() and target.read_text(encoding="utf-8") == NSC_XML:
        print(f"{target} 已是目标内容，跳过")
    else:
        target.write_text(NSC_XML, encoding="utf-8", newline="\n")
        print(f"已写入 {target}（明文 HTTP 全放行）")

    msrc = manifest.read_text(encoding="utf-8")
    mout = patch_manifest_src(msrc)
    if mout == msrc:
        print(f"{manifest} 已引用 networkSecurityConfig，跳过")
    else:
        manifest.write_text(mout, encoding="utf-8", newline="\n")
        print(f"已把 {manifest} 的明文开关改为 networkSecurityConfig")
    return 0


if __name__ == "__main__":
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except Exception:  # noqa: BLE001
            pass
    sys.exit(main())
