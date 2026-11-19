#!/usr/bin/env python3
"""Android 可选运行时权限声明补丁：按打包配置往 AndroidManifest.xml 注入 <uses-permission>。

## 背景（为什么需要）

wry 0.57 对网页发起的三类受保护能力，授权判定**只认 Manifest 声明**：

  · `getUserMedia({ video: true })`（摄像头扫码 / 拍照）
      → `RustWebChromeClient.onPermissionRequest` 映射 RESOURCE_VIDEO_CAPTURE，
        经 `PermissionHelper` 查 `android.permission.CAMERA`；
  · `getUserMedia({ audio: true })`（录音）
      → 映射 RESOURCE_AUDIO_CAPTURE，查 `android.permission.RECORD_AUDIO`；
  · `navigator.geolocation`（H5 定位）
      → `definedGeolocationPermissions` 查 ACCESS_FINE/COARSE_LOCATION。

Manifest 没声明时 wry 直接 deny（连运行时授权弹窗都不会弹），页面侧表现为
getUserMedia / geolocation 永远 reject。运行时弹窗本身 wry 已经实现
（activity.requestPermissions），**本补丁只补 Manifest 声明，不改 Kotlin 授权流程**。

不做「一刀切全量声明」：用不到的权限写进 Manifest 会让应用商店审核额外解释用途，
所以打包配置新增 `android_permissions` 白名单数组（camera / microphone / location），
解析器经 GITHUB_ENV 下发 `ANDROID_PERMISSIONS`（紧凑 JSON 文本，默认 `[]`），
本脚本据此按需注入；空数组 = 不注入这些**可选**权限。

## 恒需权限（与打包配置无关，2026-11-19 新增）

另有一组**必装**权限，不受 `android_permissions` 控制、恒声明：

  · `ACCESS_NETWORK_STATE`：hickory-resolver 0.26 的 Android `system_conf` 实现是 JNI 调
    `ConnectivityManager.getActiveNetwork()` / `getLinkProperties()` 取系统 DNS 服务器，
    这两个接口受该权限保护。Manifest 未声明时 JNI 侧抛
    `SecurityException: ConnectivityService: Neither user N nor current process has
    android.permission.ACCESS_NETWORK_STATE`，本应用表现为启动期「解析目标地址失败 ⇒
    加载失败」整页失败（域名数组配置必现，2026-11-19 安卓真机复现）。
  · `INTERNET`：模板默认已声明，这里只做「缺失才补」的兜底，不重复写。

恒需权限按**缺失才补**处理（不做 marker 短路）：模板将来若自带其中某条，本脚本自动不加，
天然幂等。Rust 侧 `resolve.rs::fallback_config` 是同一失败链的第二道保险（读不到系统配置时
改用内置公共 DNS），两者一起才是「DNS 配置读不到也不至于整页失败」。

补丁链固定顺序（见 docs/android-improvements.md §0）：
insets → back → download → extras → schemes → fullscreen → permissions → cleartext → splash-theme。

gen/android 每次构建现生成，两个 workflow（build-custom.yml / release.yml）
都必须在 init 之后调用本脚本。

用法：
    python3 .github/scripts/patch-android-permissions.py --permissions '["camera"]'
    ANDROID_PERMISSIONS='["microphone","location"]' \\
        python3 .github/scripts/patch-android-permissions.py
    python3 .github/scripts/patch-android-permissions.py --selftest   # 离线自测
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import sys

from android_gen import (  # noqa: E402
    android_manifest,
    gen_root_candidates_str,
    resolve_gen_root,
)

# 幂等标记：已打过补丁的 Manifest 不再重复注入。
MARKER = "web2app-android-permissions"

# 恒需权限的注释标记（与可选权限分开：这块**不受打包配置控制**）。
REQUIRED_MARKER = "web2app-android-required"

# 恒需权限：与打包配置无关，恒声明（缺失才补，模板自带则不重复写）。
# 顺序即 Manifest 中的声明顺序；ACCESS_NETWORK_STATE 是 hickory 读系统 DNS 的前置条件。
REQUIRED_PERMISSIONS: tuple[str, ...] = (
    "android.permission.INTERNET",
    "android.permission.ACCESS_NETWORK_STATE",
)

# 打包配置白名单名 → Manifest 权限（声明顺序即本字典顺序，与解析器白名单一致）。
# location 同时声明 FINE/COARSE：wry 的 geolocation 判定两个都查；COARSE 是
# FINE 的子集声明，Android 要求申请 FINE 时一并给出 COARSE 的兼容口径。
PERMISSION_MAP: dict[str, tuple[str, ...]] = {
    "camera": ("android.permission.CAMERA",),
    "microphone": ("android.permission.RECORD_AUDIO",),
    "location": (
        "android.permission.ACCESS_FINE_LOCATION",
        "android.permission.ACCESS_COARSE_LOCATION",
    ),
}


def parse_permissions(raw: str | None) -> list[str]:
    """解析 ANDROID_PERMISSIONS（紧凑 JSON 数组文本），去重保序并按白名单校验。

    空串 / null 一律视为 []。拼错权限名在这里硬失败 —— 解析器已挡过一道，
    这里再挡一道是为了单独手工调用本脚本时也不可能静默漏注。
    """
    if raw is None or not raw.strip():
        return []
    try:
        value = json.loads(raw)
    except json.JSONDecodeError as e:
        raise ValueError(f"ANDROID_PERMISSIONS 不是合法 JSON：{raw!r}（{e}）") from e
    if not isinstance(value, list):
        raise ValueError(
            "ANDROID_PERMISSIONS 必须是权限名字符串数组"
            '（如 ["camera","microphone","location"]），'
            f"当前是 {type(value).__name__}"
        )
    choices = "/".join(PERMISSION_MAP)
    result: list[str] = []
    for item in value:
        if not isinstance(item, str):
            raise ValueError(
                f"权限名必须是字符串（允许：{choices}），当前元素是"
                f"{type(item).__name__}：{item!r}"
            )
        name = item.strip()
        if name not in PERMISSION_MAP:
            raise ValueError(f"未知权限 {name!r}，只允许：{choices}")
        if name not in result:
            result.append(name)
    return result


def build_permission_block(permissions: list[str]) -> str:
    """按入参顺序生成 <uses-permission> 块（marker 注释 + 每条一行）。"""
    lines = [
        f"    <!-- {MARKER}：以下运行时权限由打包配置 android_permissions 按需声明。",
        "         未列入的能力（如未选 camera）Manifest 不声明，wry 将直接 deny。 -->",
    ]
    for name in permissions:
        for android_name in PERMISSION_MAP[name]:
            lines.append(f'    <uses-permission android:name="{android_name}" />')
    return "\n".join(lines) + "\n"


def build_required_block(missing: list[str]) -> str:
    """生成恒需权限块（只含 Manifest 里尚缺的那几条）。"""
    lines = [
        f"    <!-- {REQUIRED_MARKER}：恒需权限，与打包配置无关（缺失才补）。",
        "         ACCESS_NETWORK_STATE：hickory 经 JNI 调 ConnectivityManager.getActiveNetwork()",
        "         读取系统 DNS 服务器，缺它启动期解析必失败（SecurityException）。 -->",
    ]
    for android_name in missing:
        lines.append(f'    <uses-permission android:name="{android_name}" />')
    return "\n".join(lines) + "\n"


def insert_before_manifest_end(src: str, block: str) -> str:
    """把 block 插到 </manifest> 前；模板结构变化时显式报错（不静默跳过）。"""
    idx = src.rfind("</manifest>")
    if idx < 0:
        raise ValueError("AndroidManifest.xml 中未找到 </manifest>，模板可能已变化")
    return src[:idx] + block + src[idx:]


def inject_required_permissions(src: str) -> str:
    """恒需权限：按「缺失才补」注入；模板已声明时原样返回（天然幂等）。"""
    missing = [name for name in REQUIRED_PERMISSIONS if f'android:name="{name}"' not in src]
    if not missing:
        return src
    return insert_before_manifest_end(src, build_required_block(missing))


def patch_manifest_src(src: str, permissions: list[str]) -> str:
    """AndroidManifest.xml：恒需权限恒补；可选权限按打包配置幂等注入。

    ⚠️ 空数组**不再等于「原样不动」**：恒需权限（INTERNET / ACCESS_NETWORK_STATE）与
    打包配置无关，任何一次打包都要声明 —— 2026-11-19 安卓端「解析目标地址失败」正是
    因为 `android_permissions` 默认为空时整步跳过，连 DNS 都读不到。
    """
    out = inject_required_permissions(src)
    if not permissions or MARKER in out:
        return out
    return insert_before_manifest_end(out, build_permission_block(permissions))


# ---------------------------------------------------------------------------
# 自测
# ---------------------------------------------------------------------------
MANIFEST_PRISTINE = (
    '<?xml version="1.0" encoding="utf-8"?>\n'
    "<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\">\n"
    '    <application android:label="Web2App" />\n'
    "</manifest>\n"
)

# tauri 模板的真实形态：INTERNET 已自带 ⇒ 恒需块只该补 ACCESS_NETWORK_STATE。
MANIFEST_TEMPLATE = (
    '<?xml version="1.0" encoding="utf-8"?>\n'
    "<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\">\n"
    '    <uses-permission android:name="android.permission.INTERNET" />\n'
    '    <application android:label="Web2App" />\n'
    "</manifest>\n"
)


def selftest() -> int:
    # 1) 空数组 ≠ 原样不动：恒需两条仍要注入，可选 marker 不得出现
    empty = patch_manifest_src(MANIFEST_PRISTINE, [])
    assert empty != MANIFEST_PRISTINE, "恒需权限必须在空 android_permissions 下也注入"
    assert REQUIRED_MARKER in empty
    assert 'android:name="android.permission.INTERNET"' in empty
    assert 'android:name="android.permission.ACCESS_NETWORK_STATE"' in empty, (
        "缺 ACCESS_NETWORK_STATE ⇒ Android 上 hickory 读不到 DNS 服务器"
    )
    assert MARKER not in empty, "未选任何可选权限时不应出现可选权限块"
    assert "CAMERA" not in empty and "LOCATION" not in empty
    assert empty.index("ACCESS_NETWORK_STATE") < empty.index("</manifest>")
    assert "<application" in empty, "application 节点必须保留"
    assert empty.count("<uses-permission") == len(REQUIRED_PERMISSIONS)

    # 2) 模板已自带 INTERNET：只补 ACCESS_NETWORK_STATE，不重复声明 INTERNET
    from_template = patch_manifest_src(MANIFEST_TEMPLATE, [])
    assert 'android:name="android.permission.ACCESS_NETWORK_STATE"' in from_template
    assert from_template.count('android:name="android.permission.INTERNET"') == 1, (
        "模板已声明的 INTERNET 不应被重复注入"
    )
    assert from_template.count("<uses-permission") == len(REQUIRED_PERMISSIONS)

    # 3) 单权限 camera：恒需块 + 一条 CAMERA，位置在 </manifest> 前
    cam = patch_manifest_src(MANIFEST_PRISTINE, ["camera"])
    assert MARKER in cam and REQUIRED_MARKER in cam
    assert 'android:name="android.permission.CAMERA"' in cam
    assert "RECORD_AUDIO" not in cam and "LOCATION" not in cam
    assert cam.index("android.permission.CAMERA") < cam.index("</manifest>")
    assert "<application" in cam

    # 4) 三权限齐全：映射正确（location = FINE + COARSE 两条），顺序固定
    full = patch_manifest_src(MANIFEST_PRISTINE, ["camera", "microphone", "location"])
    for needle in (
        REQUIRED_MARKER,
        MARKER,
        'android:name="android.permission.CAMERA"',
        'android:name="android.permission.RECORD_AUDIO"',
        'android:name="android.permission.ACCESS_FINE_LOCATION"',
        'android:name="android.permission.ACCESS_COARSE_LOCATION"',
    ):
        assert needle in full, f"补丁缺少要素：{needle}"
    # 顺序：恒需两条 < 可选四条（CAMERA < RECORD_AUDIO < FINE < COARSE）
    assert (
        full.index("android.permission.ACCESS_NETWORK_STATE")
        < full.index("android.permission.CAMERA")
        < full.index("android.permission.RECORD_AUDIO")
        < full.index("android.permission.ACCESS_FINE_LOCATION")
        < full.index("android.permission.ACCESS_COARSE_LOCATION")
    )
    # 共 6 条 uses-permission（恒需 2 + 可选 4）
    assert full.count("<uses-permission") == 2 + 4

    # 5) 幂等：重复注入不产生第二块
    assert patch_manifest_src(full, ["camera", "microphone", "location"]) == full
    # 已注入后即使换一组权限也不重复注入（gen/android 每次构建重新生成，不做增量改写）
    assert patch_manifest_src(full, ["camera"]) == full
    # 恒需块同样幂等：第二次空数组调用不得再长出新块
    assert patch_manifest_src(empty, []) == empty

    # 6) 缺 </manifest> → 显式报错（不静默跳过）
    for perms in ([], ["camera"]):
        try:
            patch_manifest_src("<manifest />\n", perms)
            raise AssertionError(f"缺 </manifest> 应报错（permissions={perms}）")
        except ValueError:
            pass

    # 7) parse_permissions：默认/空串、合法归一去重、空白修剪；非法形态硬失败
    assert parse_permissions(None) == []
    assert parse_permissions("") == []
    assert parse_permissions("   ") == []
    assert parse_permissions('["camera"," location ","camera"]') == ["camera", "location"]
    for bad in ("camera", "{}", "true", '["gps"]', '["camera", 1]', "["):
        try:
            parse_permissions(bad)
            raise AssertionError(f"非法 ANDROID_PERMISSIONS 应报错：{bad}")
        except ValueError:
            pass
    # 恒需权限不允许借 android_permissions 注入（它们恒声明，不在白名单里）
    for reserved in REQUIRED_PERMISSIONS:
        short = reserved.rsplit(".", 1)[-1].lower()
        try:
            parse_permissions(f'["{short}"]')
            raise AssertionError(f"恒需权限 {short} 不应出现在 android_permissions 白名单")
        except ValueError:
            pass

    # 8) 文件级集成：临时 gen/android 工程，经 android_manifest 走一遍
    import tempfile

    with tempfile.TemporaryDirectory() as tmp:
        gen_root = pathlib.Path(tmp) / "gen" / "android"
        manifest = android_manifest(gen_root)
        manifest.parent.mkdir(parents=True, exist_ok=True)
        manifest.write_text(MANIFEST_PRISTINE, encoding="utf-8")
        out = patch_manifest_src(manifest.read_text(encoding="utf-8"),
                                 ["microphone", "location"])
        manifest.write_text(out, encoding="utf-8", newline="\n")
        text = manifest.read_text(encoding="utf-8")
        assert (
            REQUIRED_MARKER in text
            and "ACCESS_NETWORK_STATE" in text
            and MARKER in text
            and "RECORD_AUDIO" in text
            and "ACCESS_FINE_LOCATION" in text
        )

    print("patch-android-permissions.py 自测通过 ✓（8 组场景）")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(
        description="给 AndroidManifest.xml 注入恒需权限 + 打包配置指定的可选运行时权限"
    )
    parser.add_argument(
        "--gen-root",
        default=None,
        help="Android 工程根（缺省按候选自动探测：" + gen_root_candidates_str() + "）",
    )
    parser.add_argument(
        "--permissions",
        default=None,
        help='紧凑 JSON 数组，如 \'["camera","microphone","location"]\'；'
        "缺省读环境变量 ANDROID_PERMISSIONS；两者皆空 = 只注入恒需权限",
    )
    parser.add_argument("--selftest", action="store_true", help="运行离线自测")
    args = parser.parse_args()

    if args.selftest:
        return selftest()

    raw = args.permissions if args.permissions is not None else os.environ.get("ANDROID_PERMISSIONS")
    permissions = parse_permissions(raw)

    gen_root = resolve_gen_root(args.gen_root)
    manifest = android_manifest(gen_root)
    msrc = manifest.read_text(encoding="utf-8")
    mout = patch_manifest_src(msrc, permissions)
    if mout == msrc:
        print(f"{manifest} 权限声明已齐（{REQUIRED_MARKER} / {MARKER}），跳过")
    else:
        added = "恒需权限" if not permissions else f"恒需权限 + 可选：{', '.join(permissions)}"
        manifest.write_text(mout, encoding="utf-8", newline="\n")
        print(f"已给 {manifest} 注入{added}")
    return 0


if __name__ == "__main__":
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except Exception:  # noqa: BLE001
            pass
    sys.exit(main())
