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
本脚本据此按需注入；空数组 = 什么都不注入（直接跳过）。

补丁链固定顺序（见 docs/android-improvements.md §0）：
insets → back → download → extras → schemes → fullscreen → permissions → cleartext sed。

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


def patch_manifest_src(src: str, permissions: list[str]) -> str:
    """AndroidManifest.xml：幂等注入可选权限；空数组原样不动；缺 </manifest> 报错。"""
    if not permissions:
        return src
    if MARKER in src:
        return src
    idx = src.rfind("</manifest>")
    if idx < 0:
        raise ValueError("AndroidManifest.xml 中未找到 </manifest>，模板可能已变化")
    return src[:idx] + build_permission_block(permissions) + src[idx:]


# ---------------------------------------------------------------------------
# 自测
# ---------------------------------------------------------------------------
MANIFEST_PRISTINE = (
    '<?xml version="1.0" encoding="utf-8"?>\n'
    "<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\">\n"
    '    <application android:label="Web2App" />\n'
    "</manifest>\n"
)


def selftest() -> int:
    # 1) 空数组：完全不动 Manifest（连 marker 都不应出现）
    assert patch_manifest_src(MANIFEST_PRISTINE, []) == MANIFEST_PRISTINE, "空权限必须原样不动"

    # 2) 单权限 camera：一条 CAMERA，位置在 </manifest> 前，application 保留
    cam = patch_manifest_src(MANIFEST_PRISTINE, ["camera"])
    assert MARKER in cam
    assert 'android:name="android.permission.CAMERA"' in cam
    assert "RECORD_AUDIO" not in cam and "LOCATION" not in cam
    assert cam.index("android.permission.CAMERA") < cam.index("</manifest>")
    assert "<application" in cam

    # 3) 三权限齐全：映射正确（location = FINE + COARSE 两条），顺序固定
    full = patch_manifest_src(MANIFEST_PRISTINE, ["camera", "microphone", "location"])
    for needle in (
        MARKER,
        'android:name="android.permission.CAMERA"',
        'android:name="android.permission.RECORD_AUDIO"',
        'android:name="android.permission.ACCESS_FINE_LOCATION"',
        'android:name="android.permission.ACCESS_COARSE_LOCATION"',
    ):
        assert needle in full, f"补丁缺少要素：{needle}"
    # 顺序：CAMERA < RECORD_AUDIO < FINE_LOCATION < COARSE_LOCATION
    assert (
        full.index("android.permission.CAMERA")
        < full.index("android.permission.RECORD_AUDIO")
        < full.index("android.permission.ACCESS_FINE_LOCATION")
        < full.index("android.permission.ACCESS_COARSE_LOCATION")
    )
    # 共 4 条 uses-permission（1+1+2）
    assert full.count("<uses-permission") == 4

    # 4) 幂等：重复注入不产生第二块
    assert patch_manifest_src(full, ["camera", "microphone", "location"]) == full
    # 已注入后即使换一组权限也不重复注入（gen/android 每次构建重新生成，不做增量改写）
    assert patch_manifest_src(full, ["camera"]) == full

    # 5) 缺 </manifest> → 显式报错（不静默跳过）
    try:
        patch_manifest_src("<manifest />\n", ["camera"])
        raise AssertionError("缺 </manifest> 应报错")
    except ValueError:
        pass

    # 6) parse_permissions：默认/空串、合法归一去重、空白修剪；非法形态硬失败
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

    # 7) 文件级集成：临时 gen/android 工程，经 android_manifest 走一遍
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
        assert MARKER in text and "RECORD_AUDIO" in text and "ACCESS_FINE_LOCATION" in text

    print("patch-android-permissions.py 自测通过 ✓（7 组场景）")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="按打包配置给 AndroidManifest.xml 注入可选运行时权限")
    parser.add_argument(
        "--gen-root",
        default=None,
        help="Android 工程根（缺省按候选自动探测：" + gen_root_candidates_str() + "）",
    )
    parser.add_argument(
        "--permissions",
        default=None,
        help='紧凑 JSON 数组，如 \'["camera","microphone","location"]\'；'
        "缺省读环境变量 ANDROID_PERMISSIONS；两者皆空 = 不注入",
    )
    parser.add_argument("--selftest", action="store_true", help="运行离线自测")
    args = parser.parse_args()

    if args.selftest:
        return selftest()

    raw = args.permissions if args.permissions is not None else os.environ.get("ANDROID_PERMISSIONS")
    permissions = parse_permissions(raw)
    if not permissions:
        print("android_permissions 为空，不向 AndroidManifest.xml 注入可选权限")
        return 0

    gen_root = resolve_gen_root(args.gen_root)
    manifest = android_manifest(gen_root)
    msrc = manifest.read_text(encoding="utf-8")
    mout = patch_manifest_src(msrc, permissions)
    if mout == msrc:
        print(f"{manifest} 已包含权限声明（{MARKER}），跳过")
    else:
        manifest.write_text(mout, encoding="utf-8", newline="\n")
        print(f"已给 {manifest} 注入运行时权限：{', '.join(permissions)}")
    return 0


if __name__ == "__main__":
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except Exception:  # noqa: BLE001
            pass
    sys.exit(main())
