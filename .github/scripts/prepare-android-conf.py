#!/usr/bin/env python3
"""`cargo tauri android init` **之前**：把 version / productName / identifier 写进 tauri.conf.json。

抽自 `build-custom.yml` 与 `release.yml` 各自的**内联 python**（两壳曾各存一份，只靠人肉保持同步；
本文件的历史同类分裂就漏过 EXTRA / ICON_URL）。合并后 = 原来两份行为的并集：

  1. **version**：环境变量 `VERSION` 非空时写入 `tauri.conf.json` 的 `version`
     —— Tauri 会把它烘焙成 Android 的 `versionName`（`versionCode` 由其派生）。
     ⚠️ 必须在 app_name 早退**之前**写：即便没配 app_name，version 也要生效。
     `VERSION` 为空 ⇒ **不改写**（保留仓库里已提交的值）；不是 `主.次.补丁` 形式 ⇒ 告警并跳过。
  2. **app_name 为空** ⇒ 打印后正常退出（0），既不改 productName 也不改 identifier。
  3. **productName ← app_name**（清洗 Windows 文件名非法字符）；**identifier ←** 由 app_name 派生的
     `com.web2app.<seg>`。Tauri 2 **没有** `bundle.android.package`，安卓包名只能来自 identifier
     ⇒ 必须在 init 之前写，否则 init 把默认名 / 默认包名烘焙进安卓工程（所有配置的 APK 共用
     `com.example.web2app`，互相覆盖安装）。

用法（cwd = webview-app；两个 workflow 的 `defaults.run.working-directory` 已保证）：
    python3 ../.github/scripts/prepare-android-conf.py
    python3 .github/scripts/prepare-android-conf.py --selftest
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys

from android_gen import clean_name

CONF = "tauri.conf.json"

# version 必须是三段数字（与 docs/packaging-config.md §3.6 一致）。
SEMVER_RE = re.compile(r"^\d+\.\d+\.\d+$")

# 包名任一段都不能是 Java 关键字（aapt / gradle 会直接报 invalid package name）。
JAVA_KEYWORDS = frozenset({
    "abstract", "assert", "boolean", "break", "byte", "case", "catch", "char", "class", "const",
    "continue", "default", "do", "double", "else", "enum", "extends", "final", "finally", "float",
    "for", "goto", "if", "implements", "import", "instanceof", "int", "interface", "long", "native",
    "new", "package", "private", "protected", "public", "return", "short", "static", "strictfp",
    "super", "switch", "synchronized", "this", "throw", "throws", "transient", "try", "void",
    "volatile", "while",
})


def derive_package(name: str) -> str:
    """由 app_name 派生安卓包名 `com.web2app.<seg>`，同时满足 aapt 与 Java 的包名规则。"""
    seg = re.sub(r"[^0-9A-Za-z_]", "", name).lower()
    if not seg or seg[0].isdigit():
        seg = "app" + seg
    if seg == "app":
        # 原名里一个可用 ASCII 字符都没有（如纯中文）⇒ 用名字指纹兜底，
        # 保证不同 app_name 派生出不同包名，而不是统统撞在同一个上。
        seg = "app" + hashlib.sha1(name.encode("utf-8")).hexdigest()[:8]
    if seg in JAVA_KEYWORDS:
        seg += "_"
    # 前缀固定 com.web2app —— 上架应用市场要换成自己公司域名（如 com.yourcorp.*）时改这一行。
    return "com.web2app." + seg


def normalize_version(raw: str) -> str | None:
    """把版本输入归一成 `主.次.补丁`；不合规返回 None（调用方告警并跳过改写）。

    容忍前导 `v`/`V` —— release.yml 会用 `github.event.release.tag_name` 回落，
    而 tag 惯例是 `v1.2.3`。
    """
    v = (raw or "").strip()
    if v[:1] in ("v", "V"):
        v = v[1:]
    return v if SEMVER_RE.match(v) else None


def build_updates(version: str | None, name: str) -> dict:
    """返回需要写进 tauri.conf.json 的字段。version=None 表示「不改写」。"""
    upd: dict = {}
    if version is not None:
        upd["version"] = version
    if name:
        upd["productName"] = clean_name(name)
        upd["identifier"] = derive_package(name)
    return upd


def main() -> int:
    ap = argparse.ArgumentParser(description="android init 之前预写 tauri.conf.json")
    ap.add_argument("--conf", default=CONF, help=f"目标配置文件（默认 {CONF}）")
    ap.add_argument("--selftest", action="store_true", help="运行离线自测")
    args = ap.parse_args()
    if args.selftest:
        return selftest()

    version_in = (os.environ.get("VERSION") or "").strip()
    version: str | None = None
    if version_in:
        version = normalize_version(version_in)
        if version is None:
            print(f"::warning::VERSION={version_in!r} 不是「主.次.补丁」形式，跳过 version 改写")
    name = (os.environ.get("APP_NAME") or "").strip()

    with open(args.conf, encoding="utf-8") as f:
        conf = json.load(f)
    old_pkg = conf.get("identifier", "")
    conf.update(build_updates(version, name))
    with open(args.conf, "w", encoding="utf-8") as f:
        json.dump(conf, f, ensure_ascii=False, indent=2)
        f.write("\n")

    if version is not None:
        print(f"version     = {version}（android init 据此烘焙 versionName/versionCode）")
    else:
        print(f"version     未改写（保留 {conf.get('version', '')}）")
    if name:
        print(f"productName = {conf['productName']}（android init 据此烘焙安卓应用名称）")
        print(f"identifier  = {old_pkg} → {conf['identifier']}（android init 据此烘焙 Android 包名）")
    else:
        print("未配置 app_name，沿用默认名称与包名")
    return 0


def selftest() -> int:
    """离线自测：覆盖 version 三态、名称清洗、包名派生边界。"""
    # 1) version 三态 + 归一
    assert build_updates("1.2.3", "")["version"] == "1.2.3"
    assert "version" not in build_updates(None, ""), "version=None 时不得写入"
    assert build_updates("0.0.1", "")["version"] == "0.0.1"
    assert normalize_version("v1.2.3") == "1.2.3", "应剥掉 release tag 的前导 v"
    assert normalize_version("V2.0.0") == "2.0.0"
    assert normalize_version("1.2") is None, "两段式不是合法版本"
    assert normalize_version("1.2.3.4") is None
    assert normalize_version("1.2.x") is None
    assert normalize_version("") is None
    assert normalize_version("v") is None

    # 2) 名称清洗：非法字符 / 空白 / 首尾句点 / 空串回落
    assert clean_name('a/b:c*d?e"f<g>h|i') == "a_b_c_d_e_f_g_h_i"
    assert clean_name("  My   App  ") == "My App"
    assert clean_name("  .My App.  ") == "My App"
    assert clean_name("   ") == "Web2App"

    # 3) 包名派生
    assert derive_package("MyApp") == "com.web2app.myapp"
    assert derive_package("9lives") == "com.web2app.app9lives", "首字符是数字要加 app 前缀"
    # 纯中文 ⇒ 指纹兜底，且不同名不同包
    cn1, cn2 = derive_package("我的应用"), derive_package("另一个应用")
    assert cn1.startswith("com.web2app.app") and cn1 != cn2, "纯中文名必须各自派生不同包名"
    assert derive_package("class") == "com.web2app.class_", "Java 关键字要补下划线"
    assert derive_package("") == derive_package(""), "空名应稳定（app+空指纹）"

    # 4) build_updates 组合：name 非空才写 productName/identifier
    upd = build_updates("2.0.0", "MyApp")
    assert upd == {"version": "2.0.0", "productName": "MyApp", "identifier": "com.web2app.myapp"}
    assert build_updates(None, "") == {}

    print("prepare-android-conf.py 自测通过 ✓（4 组场景）")
    return 0


if __name__ == "__main__":
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except Exception:  # noqa: BLE001
            pass
    sys.exit(main())
