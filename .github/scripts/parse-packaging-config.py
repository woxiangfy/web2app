#!/usr/bin/env python3
r"""打包配置 JSON 的**唯一解析实现**（Tauri / Android / Electron 三处共用）。

## 为什么要有这个文件

打包配置 JSON 的契约（字段名、类型、默认值、取值链）此前在 `build-custom.yml` 里
**各写了一份**（windows job 一份、android job 一份），两份靠人工同步。任何一次只改一处，
就会得到「同一份配置在两个产物里含义不同」——这是最难查的一类缺陷，因为它不报错。

本文件把「配置 → 编译期环境变量 / 规范化的 config.json」这段逻辑收敛成一份实现，
所有打包入口（`build-custom.yml` ×2、`electron-build-custom.yml`）都调它。

## 契约（改这里 = 改契约，必须同步 docs/packaging-config.md）

字段与类型（**布尔一律真 bool，不再接受 "true"/"false" 字符串**）：

| 字段                     | 类型                  | 默认值                              |
|--------------------------|-----------------------|-------------------------------------|
| app_url                  | string \| string[]   | 必填                                |
| app_name                 | string                | "Web2App"                           |
| title                    | string                | "" → 回落 app_name                  |
| version                  | string                | "0.0.1"（三段数字 x.y.z）           |
| extra                    | 任意 JSON             | {}                                  |
| icon_url                 | string                | ""                                  |
| proxy_enabled            | bool                  | false                               |
| proxy_http               | string                | ""                                  |
| proxy_https              | string                | ""                                  |
| proxy_bypass             | string                | "localhost;127.0.0.1;<-loopback>"   |
| data_dir                 | string                | "data"                              |
| tabs_enabled             | bool                  | false                               |
| mobile_bar               | bool                  | false                               |
| public_key               | string                | ""（app_url 为数组时必填）          |
| resolve_secret           | string                | ""                                  |
| debug                    | bool                  | false                               |
| android_keystore_base64  | string                | ""（仅 Android）                    |
| android_key_alias        | string                | ""（仅 Android）                    |
| android_keystore_password| string                | ""（仅 Android）                    |
| android_key_password     | string                | ""（仅 Android）                    |
| android_permissions      | string[]              | []（仅 Android，白名单三值）        |

`data_dir` 三种取值（两壳行为一致，见 docs/packaging-config.md §4.3）：

| 取值             | 含义                                                             |
|------------------|------------------------------------------------------------------|
| `"data"`（默认） | 程序自身所在目录（exe 目录）下的 `data\`，跟着程序走（绿色版）    |
| `""`（空串）     | 系统默认程序目录（`%LOCALAPPDATA%\<app_name>`），不受程序位置影响 |
| 其它相对路径     | 同上，基准仍是 exe 目录（**不是 shell 的 cwd**）                   |
| 绝对路径         | 原样使用                                                          |

⚠️ 基准必须是 **exe 所在目录**而不是进程工作目录：快捷方式的「起始位置」、命令行、拖拽启动都会改变
cwd，用它会让同一个包在不同启动方式下把数据落到不同地方。

`debug`（诊断日志开关，默认 **false** = 不输出任何日志，两壳行为一致）：

| 取值    | 行为                                                                          |
|---------|-------------------------------------------------------------------------------|
| `false` | 不写日志文件；stdout / stderr 的诊断也一并静默（终端启动同样安静）             |
| `true`  | 写 `<数据目录>\logs\startup.log`，并把同一批诊断同时打到 stdout / stderr       |

⚠️ 它是**开关**而不是「日志级别」：关闭时没有任何落盘产物，因此「用户报双击没反应」这类问题默认
**没有第一现场**。排障时两种打开方式：重新打包（`"debug": true`），或在命令行临时打开
（Tauri 与 Electron 都认 `--debug`；Electron 另认环境变量 `WEB2APP_DEBUG=1`）——后者不需要重打包。

**已移除**（配置里出现即报错，并给出对应改法）：

- `window_title`（旧字段，等价于 `title`）；
- `data_dir_default`（旧**布尔**字段，改为**字符串** `data_dir`）：
  `false` → `"data"`，`true` → `""`。

## 用法

    # 从字符串 / URL 读配置，把编译期环境变量写进 GITHUB_ENV
    python3 .github/scripts/parse-packaging-config.py --json '<JSON>' --env-out "$GITHUB_ENV"

    # Android：额外把 keystore 落盘并注入 ANDROID_KEYSTORE_* 变量，并启用目标校验
    # （内嵌账号密码必须走代理，否则真机必 401，见 validate_for_target）
    ... --target android --android-keystore

    # Electron：额外把**规范化后的** config.json 落盘（进 asar 的那一份）
    ... --config-out electron-app/config.json

    # 自测（CI 里跑，防止契约悄悄漂移）
    python3 .github/scripts/parse-packaging-config.py --selftest

`config_json` / `config_url` 也可经环境变量 `CONFIG_JSON` / `CONFIG_URL` 传入（CI 用法）。
"""

from __future__ import annotations

import argparse
import base64
import difflib
import json
import os
import pathlib
import re
import sys
import urllib.request

# ---------------------------------------------------------------------------
# 契约：默认值表。**机器可读唯一真源是 `scripts/packaging-fields.json`**（21 字段：
# name / kind / default / 需出现的真源），本文件与 Node 复刻（scripts/packlib/
# constants.mjs）、契约守卫（webview-app/src/contract_tests/packaging_field_contract.rs）
# 都从它加载 —— 加字段只改那一份 JSON + 各真源实现，不再逐处改字段表副本。
#
# **顺序即打印顺序**（= 写进 GITHUB_ENV 的顺序）= 字段表里的排列顺序。
# 布尔字段（kind=「bool」）的默认值必须是真 bool（不是 "false" 字符串）——
# 这是本文件的全部意义之一。
#
# 各字段语义速记（完整契约见 docs/packaging-config.md）：
#   version            打包产物版本（Tauri 写入 tauri.conf.json 的 version；Electron 经
#                      extraMetadata.version 覆盖 package.json）。只接受三段纯数字（见
#                      normalize_version）—— Windows MSI ProductVersion 与 exe VERSIONINFO
#                      都不认 semver 预发布标签 / v 前缀 / 四段。
#   proxy_bypass       ⚠️ 两壳必须一致：webview-app 的 config.rs 编译期默认值、electron 的
#                      DEFAULTS 都用这同一个串。留空 = 不下发 bypass 参数（即空值不覆盖
#                      Chromium 默认）。
#   data_dir           默认 "data" = 程序所在目录（exe 目录）下的 data\，跟着程序走；
#                      空串 = 系统默认程序目录（%LOCALAPPDATA%\<app_name>）。
#                      ⚠️ 空串在本字段里**是有含义的取值**，不是「未设置」——两壳的读取
#                      都必须保留空串，Rust 侧因此不能用 env_str!（它把空串当未设置）。
#   mobile_bar         移动端底栏（Android/iOS 专用）：桌面壳（Windows Tauri / Electron）
#                      即使打包时注入了 true 也一律忽略（Rust 侧常量按 #[cfg(mobile)]
#                      编译）。默认必须 false（默认开启 = 凭空给每个移动端包加一条底栏）。
#   debug              诊断日志开关（默认关）：false = 两壳都不写启动日志、也不输出诊断；
#                      true = 写 <数据目录>\logs\startup.log 并同时打印。详见文件头。
#                      ⚠️ 默认必须是 false：默认开启会在用户的数据目录里留下交付物以外的
#                      文件（非期望行为）。
#   android_permissions 仅 Android：按需向 AndroidManifest.xml 注入运行时权限声明。白名单
#                      三值 "camera" / "microphone" / "location"（见
#                      normalize_android_permissions），默认 [] = 不声明任何可选权限。
#                      由 patch-android-permissions.py 消费 GITHUB_ENV 的 ANDROID_PERMISSIONS。
# ---------------------------------------------------------------------------
_FIELDS_PATH = pathlib.Path(__file__).resolve().parents[2] / "scripts" / "packaging-fields.json"
_FIELDS: list[dict] = json.loads(_FIELDS_PATH.read_text(encoding="utf-8"))["fields"]

DEFAULTS: dict[str, object] = {f["name"]: f["default"] for f in _FIELDS}
BOOL_KEYS = {f["name"] for f in _FIELDS if f["kind"] == "bool"}
# 需要保持 JSON 结构（对象/数组/标量）的字段：序列化成单行 JSON 文本进 env，
# 供 Rust 侧以 EXTRA 编译期变量原样内联成 `window.web2appExtra = <JSON>;`。
JSON_KEYS = {"extra"}
# 签名相关字段不进 GITHUB_ENV：keystore 体积大 / 密码敏感，由 --android-keystore 单独处理。
SIGNING_KEYS = {
    "android_keystore_base64",
    "android_key_alias",
    "android_keystore_password",
    "android_key_password",
}
SKIP_ENV = SIGNING_KEYS

# 已移除的字段 → **可直接照做的改法说明**。命中的报错要告诉用户具体怎么改，
# 而不是只说「未知字段」——只报「不认识这个键」，用户还得自己猜新名字与新旧值怎么对应。
REMOVED_KEYS: dict[str, str] = {
    "window_title": "该字段**已移除**，请改用 title（语义相同；移除原因是同一概念曾有三个名字）",
    "data_dir_default": (
        "该字段**已移除**（旧的是布尔，新的是字符串），请改用 data_dir：\n"
        '       旧值 false ⇒ "data_dir": "data"  （默认：程序所在目录下的 data\\，跟着程序走）\n'
        '       旧值 true  ⇒ "data_dir": ""      （空串 = 系统默认程序目录，不受程序位置影响）'
    ),
}


class ConfigError(Exception):
    """配置错误：消息会以 `::error::` 形式出现在 Actions 日志里，必须可操作。"""


# ---------------------------------------------------------------------------
# 读取
# ---------------------------------------------------------------------------
def read_raw(json_text: str | None, url: str | None) -> str:
    raw = (json_text or "").strip()
    u = (url or "").strip()
    if not raw and u:
        req = urllib.request.Request(u, headers={"User-Agent": "web2app-ci"})
        with urllib.request.urlopen(req, timeout=30) as r:
            raw = r.read().decode("utf-8").strip()
    if not raw:
        raise ConfigError("必须提供 config_json 或 config_url，二者至少填一个")
    return raw


def parse_json(raw: str) -> dict:
    try:
        data = json.loads(raw)
    except Exception as e:  # noqa: BLE001 — 用户输入，任何解析异常都要变成可读文案
        raise ConfigError(f"配置不是合法 JSON：{e}") from e
    if not isinstance(data, dict):
        raise ConfigError("配置 JSON 必须是一个对象（{...}），当前是 "
                          f"{type(data).__name__}")
    return data


# ---------------------------------------------------------------------------
# 校验与规范化
# ---------------------------------------------------------------------------
def reject_unknown_keys(data: dict) -> None:
    """未知字段**硬失败**。

    曾经这里只是 `::warning::` 后跳过 —— 后果是「字段名拼错」与「字段值不生效」
    看起来完全一样（都是静默），用户只能靠猜。硬失败 + 近似名建议是唯一能让
    这类手误在打包时就现形的做法。
    """
    unknown = [k for k in data if k not in DEFAULTS]
    if not unknown:
        return
    lines = []
    for k in sorted(unknown):
        if k in REMOVED_KEYS:
            lines.append(f"  {k}：{REMOVED_KEYS[k]}")
            continue
        near = difflib.get_close_matches(k, list(DEFAULTS), n=1, cutoff=0.5)
        hint = f"，是否想写 {near[0]}？" if near else ""
        lines.append(f"  {k}：未知字段{hint}")
    raise ConfigError("配置里有无法识别的字段（拼错即静默失效，故在此硬失败）：\n"
                      + "\n".join(lines))


def normalize_bool(key: str, value: object) -> bool:
    """布尔字段只接受真 bool。

    旧模板里写的是 `"proxy_enabled": "false"`（字符串）。字符串形式的 `"false"` 是
    **真值**，两壳各自用不同的宽容规则去猜它，结果 `"on"` 在 Tauri 是 false、
    在 Electron 是 true —— 同一份配置两种行为。所以这里直接拒绝字符串，把歧义消灭在打包期。
    """
    if isinstance(value, bool):
        return value
    raise ConfigError(
        f"字段 {key} 必须是布尔值（true / false，**不带引号**），当前是 "
        f"{type(value).__name__}：{value!r}\n"
        f"  正确写法：\"{key}\": true\n"
        f"  错误写法：\"{key}\": \"true\"   ← 字符串，已不再接受"
    )


def normalize_app_url(value: object, public_key: str) -> str:
    """app_url 支持两种形态，统一出口是**一段可以直接进环境变量的文本**：
      · string  → 原样（单个 URL，可含 `user:pass@`）
      · array   → JSON 数组文本（一组跳板域名，运行时做 DNS TXT + 验签解密）
    """
    if isinstance(value, list):
        domains = [str(d).strip() for d in value if str(d).strip()]
        if not domains:
            raise ConfigError("app_url 为数组时不能为空")
        if not public_key.strip():
            raise ConfigError(
                "app_url 为域名数组时必须提供 public_key（Ed25519 公钥）——"
                "域名模式下地址由 DNS TXT 密文解密得到，没有公钥无法验签"
            )
        return json.dumps(domains, ensure_ascii=False, separators=(",", ":"))

    if isinstance(value, bool) or not isinstance(value, (str, int, float)):
        raise ConfigError(f"app_url 必须是字符串或字符串数组，当前是 {type(value).__name__}")

    text = str(value).strip()
    if not text:
        raise ConfigError("缺少必填字段 app_url")

    # 兼容：字符串形态但内容是 JSON 数组文本（`"[\"a.com\"]"`）。
    # 这曾是唯一可行的数组写法，老配置里可能有；仍按数组处理，但提示改用真数组。
    if text.startswith("["):
        try:
            parsed = json.loads(text)
        except Exception:  # noqa: BLE001
            parsed = None
        if isinstance(parsed, list):
            domains = [str(d).strip() for d in parsed if str(d).strip()]
            if not domains:
                raise ConfigError("app_url 的数组内容为空")
            if not public_key.strip():
                raise ConfigError("app_url 为域名数组时必须提供 public_key（Ed25519 公钥）")
            print("::warning::app_url 是「JSON 数组文本」写法，建议直接写真的 JSON 数组："
                  'app_url: ["a.com","b.com"]（去掉外层引号）')
            return json.dumps(domains, ensure_ascii=False, separators=(",", ":"))
        raise ConfigError(
            "app_url 以 '[' 开头但不是合法的 JSON 字符串数组；"
            '若要用域名数组模式请写 ["a.com","b.com"]'
        )

    if not re.match(r"^https?://", text, re.IGNORECASE):
        # 不硬失败：本地夹具 / 特殊环境可能用别的 scheme，最终由运行时 Url::parse 判定。
        # 但这条几乎总是手误（漏了 scheme），值得在打包日志里现形。
        print(f"::warning::app_url 未以 http:// 或 https:// 开头（{mask(text)}），"
              "运行时若无法解析会提示「应用配置的网址不合法」")
    return text


def normalize_public_key(value: object) -> str:
    """把 JSON 里手写的字面量 `\\n` 还原成真换行，并压掉首尾空白（PEM 多行 / 单行都接受）。"""
    text = "" if value is None else str(value)
    if not text:
        return ""
    return text.replace("\\n", "\n").replace("\\r", "").strip()


def normalize_extra(value: object) -> str:
    """extra 统一序列化为单行 JSON 文本（默认 {} 也要转成 "{}"，供 Rust 侧原样解析）。"""
    try:
        return json.dumps(value, ensure_ascii=False, separators=(",", ":"))
    except Exception as e:  # noqa: BLE001
        raise ConfigError(f"extra 无法序列化为 JSON：{e}") from e


# android_permissions 白名单：打包配置里只接受这三个名字，Manifest 注入脚本再把它们
# 映射成真正的 <uses-permission>。在这里限定取值，拼错权限名就不可能静默产出一份
# 「装上了但授权弹窗永远被 deny」的 APK。
ANDROID_PERMISSION_CHOICES: tuple[str, ...] = ("camera", "microphone", "location")


def normalize_android_permissions(value: object) -> str:
    """android_permissions 归一为**去重保序**的紧凑 JSON 数组文本（默认 []）。

    只接受字符串数组，元素去首尾空白后必须在白名单内；重复值去重但保留首次出现顺序。
    出口形态与 extra 一致（单行 JSON 文本进 env，写 config.json 时再还原成真数组）。
    """
    if not isinstance(value, list):
        raise ConfigError(
            "字段 android_permissions 必须是权限名字符串数组"
            '（如 ["camera","microphone","location"]），'
            f"当前是 {type(value).__name__}"
        )
    choices = "/".join(ANDROID_PERMISSION_CHOICES)
    result: list[str] = []
    for item in value:
        if not isinstance(item, str):
            raise ConfigError(
                f"字段 android_permissions 的元素必须是字符串（允许：{choices}），"
                f"当前元素是 {type(item).__name__}：{item!r}"
            )
        name = item.strip()
        if not name:
            raise ConfigError(f"字段 android_permissions 的元素不能为空（允许：{choices}）")
        if name not in ANDROID_PERMISSION_CHOICES:
            raise ConfigError(
                f"字段 android_permissions 含未知权限 {name!r}，只允许：{choices}"
            )
        if name not in result:
            result.append(name)
    return json.dumps(result, ensure_ascii=False, separators=(",", ":"))


# 版本号：三段纯数字（semver 的 x.y.z 基础形态，**不接受** v 前缀 / 四段 / 预发布标签）。
_VERSION_RE = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")


def normalize_version(value: object) -> str:
    """打包产物版本。只接受 "主.次.补丁" 三段纯数字字符串。

    为什么比 semver 更严：两条打包链路的 Windows 产物版本最终都要过 Windows 自己的规则 ——
      · Tauri 的 MSI（WiX ProductVersion）：主、次 ∈ [0,255]，补丁 ∈ [0,65535]，
        且只认三段数字（"1.2.3-beta" / "v1.2.3" / 四段都会在打包期报错）；
      · electron-builder 同样把版本归一进 exe 的 VERSIONINFO（16 位字段）。
    在配置解析处统一挡住，比「同一份配置一个壳能出包、另一个壳炸在 WiX」更容易排查。
    Tauri 的 Android versionName / versionCode 也由这同一个版本号烘焙派生。
    """
    if not isinstance(value, str):
        raise ConfigError(
            f"字段 version 必须是字符串（三段数字版本号），当前是 {type(value).__name__}：{value!r}\n"
            '  正确写法："version": "1.2.3"'
        )
    text = value.strip()
    m = _VERSION_RE.match(text)
    if not m:
        raise ConfigError(
            '字段 version 必须是三段数字版本号 "主.次.补丁"（如 "1.2.3"），'
            "不带 v 前缀、不加第四段、不写预发布标签（-beta 等），"
            f"当前是 {value!r}"
        )
    major, minor, patch = (int(g) for g in m.groups())
    if major > 255 or minor > 255 or patch > 65535:
        raise ConfigError(
            "字段 version 超出 Windows 版本范围（主、次 ≤ 255，补丁 ≤ 65535）："
            f"{text}（解析为主={major} 次={minor} 补丁={patch}）"
        )
    return text


# Windows 上永远非法的目录名字符。`:` 与 `\` `/` 是路径语法的一部分，不在此列。
_DATA_DIR_BAD = set('<>"|?*')


def normalize_data_dir(value: object) -> str:
    """数据目录。**空串在这里是有含义的取值**（= 系统默认程序目录），不是「未设置」。

    三种形态（两壳行为一致，见 docs/packaging-config.md §4.3）：
      · `""`      → 系统默认程序目录（`%LOCALAPPDATA%\\<app_name>`），不受程序位置影响；
      · 相对路径   → 程序**自身所在目录**（exe 目录）下的该路径，跟着程序走；
      · 绝对路径   → 原样使用。

    非字符串一律拒绝：`"data_dir": true` 若被宽容地 `str()` 成 `"True"`，会静默建出一个
    名叫 `True` 的目录 —— 「看起来配置生效了、其实完全不是想要的」是最难查的一类。
    """
    if value is None:
        return str(DEFAULTS["data_dir"])
    if not isinstance(value, str):
        raise ConfigError(
            f"字段 data_dir 必须是字符串（目录路径），当前是 {type(value).__name__}：{value!r}\n"
            '  正确写法："data_dir": "data"   ← 默认：程序所在目录下的 data\\\n'
            '  正确写法："data_dir": ""       ← 空串 = 系统默认程序目录（由 app_name 命名）'
        )
    text = value.strip()
    if "\x00" in text:
        raise ConfigError("data_dir 不能包含 NUL 字符")
    bad = sorted(set(text) & _DATA_DIR_BAD)
    if bad:
        print(f"::warning::data_dir 含 Windows 目录名非法字符 {''.join(bad)}（{text}）"
              "—— Windows 上创建该目录会失败，届时会自动回退到系统默认目录")
    return text


def mask(u: str) -> str:
    """打印时隐去 URL 里的 `user:pass@`。"""
    return re.sub(r"(://[^/@\s]+)@", "://***@", str(u))


# ---------------------------------------------------------------------------
# 按打包目标的跨字段校验（解析器本身与目标无关；只有 CI 的 Android 调用传 --target android）
# ---------------------------------------------------------------------------
_USERINFO_RE = re.compile(r"^[a-zA-Z][a-zA-Z0-9+.-]*://[^/@\s]+@")


def url_contains_userinfo(text: str) -> bool:
    """单 URL 文本是否内嵌了 `user:pass@`（scheme://userinfo@host[/...]）。"""
    return bool(_USERINFO_RE.match(text.strip()))


def validate_for_target(target: str | None, merged: dict[str, str]) -> None:
    """打包目标相关的组合校验。目前只有 Android 有必须在打包期挡住的组合。

    **Android + 内嵌账号密码 + 关代理 = 必然 401**：
      · `bootstrap.rs` 的 `#[cfg(not(windows))]` 导航分支不做任何鉴权注入
        （nav.user/nav.pass 被 `let _ =` 压掉），鉴权只靠本地反向代理；
      · wry 0.57 的 `RustWebChromeClient` 未覆写 `onReceivedHttpAuthRequest`，
        默认取消 Basic Auth 弹窗 ⇒ 直连内嵌凭据在真机上必然 401 / 卡启动失败卡。
    Windows 直连有 WebView2 原生鉴权钩子（webview_hooks.rs），不受此限。
    """
    if target != "android":
        return
    app_url = str(merged["app_url"]).strip()
    proxy_on = str(merged["proxy_enabled"]).strip().lower() == "true"

    # 域名数组模式：解密结果静态不可知，只能提示（真实地址若含账号密码同样必须走代理）。
    if app_url.startswith("["):
        if not proxy_on:
            print("::warning::目标为 Android 且 proxy_enabled=false：域名数组模式下若解密出的"
                  "真实地址包含账号密码（user:pass@host），Android 无法注入 Basic 鉴权"
                  "（只有本地反向代理能注入），真机将 401。目标站需要登录时请改配 "
                  '"proxy_enabled": true。')
        return

    if url_contains_userinfo(app_url) and not proxy_on:
        raise ConfigError(
            "Android 目标不支持「URL 内嵌账号密码 + 直连（proxy_enabled=false）」组合：\n"
            f"  app_url 内嵌了 user:pass@（{mask(app_url)}）。\n"
            "  Android 壳没有 HTTP Basic 鉴权钩子（bootstrap.rs 的非 Windows 分支不注入 "
            "Authorization，wry 0.57 也未覆写 onReceivedHttpAuthRequest），"
            "该组合在真机上必然 401 / 卡在启动失败卡。\n"
            "  修复（任选其一）：\n"
            '    1) 开启本地反向代理："proxy_enabled": true'
            "（由代理统一注入 Authorization，推荐，与 Windows 鉴权链路同源）；\n"
            "    2) 若站点本身不依赖 Basic Auth，删掉 app_url 里的 user:pass@。"
        )


def build_merged(data: dict) -> dict[str, str]:
    """把用户配置合并进默认值，输出「已规范化的字符串表」。"""
    reject_unknown_keys(data)

    merged: dict[str, object] = dict(DEFAULTS)
    # app_url / extra / data_dir 单独取：
    #   · app_url 是唯一允许「字符串或数组」两种 JSON 类型的字段；
    #   · extra 要保持 JSON 结构（把 list 变成 Python repr 会丢 JSON 形态；dict 编码后又编码会双重转义）；
    #   · data_dir 必须按**原始类型**校验 —— 若先 str() 掉，`true` 会变成字符串 "True" 而被放行。
    raw_app_url: object = DEFAULTS["app_url"]
    raw_extra: object = DEFAULTS["extra"]
    raw_data_dir: object = DEFAULTS["data_dir"]
    raw_android_permissions: object = DEFAULTS["android_permissions"]
    for key, value in data.items():
        if value is None:
            continue  # 显式 null = 用默认值
        if key == "app_url":
            raw_app_url = value
            continue
        if key in JSON_KEYS:
            raw_extra = value
            continue
        if key == "data_dir":
            raw_data_dir = value
            continue
        if key == "android_permissions":
            raw_android_permissions = value
            continue
        if key in BOOL_KEYS:
            merged[key] = normalize_bool(key, value)
        else:
            merged[key] = value if isinstance(value, bool) else str(value)

    # 先归一 public_key：normalize_app_url 要用它判断「数组模式是否带公钥」。
    merged["public_key"] = normalize_public_key(merged["public_key"])
    merged["app_url"] = normalize_app_url(raw_app_url, str(merged["public_key"]))
    merged["extra"] = normalize_extra(raw_extra)
    merged["data_dir"] = normalize_data_dir(raw_data_dir)
    merged["android_permissions"] = normalize_android_permissions(raw_android_permissions)
    merged["icon_url"] = str(merged["icon_url"]).strip()
    merged["resolve_secret"] = str(merged["resolve_secret"]).strip()

    if merged["icon_url"] and not re.match(r"^https?://", str(merged["icon_url"]), re.IGNORECASE):
        print(f"::warning::icon_url 不是 http(s) 地址（{merged['icon_url']}），"
              "打包时会下载失败并回落到仓库默认图标")

    for key in BOOL_KEYS:
        merged[key] = normalize_bool(key, merged[key])

    # title（显示名）与 app_name（安装名）分离；显式空值 → 回落 app_name。
    # 取值链只有一跳：title → app_name。（旧字段 window_title 已移除，见 REMOVED_KEYS）
    title = str(merged["title"]).strip()
    if not title:
        title = str(merged["app_name"])
    merged["title"] = title

    # version 单独按版本规则校验（三段数字 + Windows 取值范围），不能走通用 str() 通道。
    merged["version"] = normalize_version(merged["version"])

    # ⚠️⚠️ 告警的**触发条件**必须对齐「运行期到底哪条路会解密」：
    #    · Tauri：`bootstrap.rs` 里 `parse_domains(APP_URL)` 命中（文本以 `[` 开头）才走
    #      DNS TXT → 解密的路径；否则直接用字符串 URL。
    #    · Electron：`navigate.ts` 的 `needsDnsResolve` = `Array.isArray(app_url)`；
    #      字符串模式只可能是 direct / proxy 两条路，都不解密。
    #    `SECRET` 的唯一消费点是 `resolve-core::derive_key`（而它只被 `decrypt_txt` 用），
    #    所以**单 URL 字符串模式下这个字段在运行期没有任何调用点** —— 此时报 warning 是
    #    狼来了：真该修的配置问题会被淹没在噪声里。原实现对两种形态一律告警，与此处
    #    `docs/packaging-config.md`（§3.4 与校验表）写的「仅域名数组模式才需要」相矛盾。
    if not merged["resolve_secret"]:
        if str(merged["app_url"]).strip().startswith("["):
            print("::warning::resolve_secret 未配置：app_url 是域名数组，运行期要靠它解密 TXT 密文；"
                  "将使用代码内置默认 Secret（所有产物共享同一固定密钥串，安全门槛较低）。"
                  "建议配置独立的 resolve_secret，并同步服务端 encrypt_url.py / resolve-tool。")
        else:
            # 单 URL 模式：明确说明「当前不参与」，比默认告警更有用，且不产生 CI 注解。
            print("resolve_secret 未配置：app_url 是单 URL 字符串，运行期不解密 DNS TXT "
                  "⇒ 该字段当前不参与，无需配置（将来改用域名数组模式时才需要）。")

    # 全部收敛为字符串，供 env / 打印使用。
    return {k: (v if isinstance(v, str) else str(v)) for k, v in merged.items()}


# ---------------------------------------------------------------------------
# 输出
# ---------------------------------------------------------------------------
def write_env(path: str, merged: dict[str, str]) -> None:
    with open(path, "a", encoding="utf-8") as f:
        for key, value in merged.items():
            if key in SKIP_ENV:
                continue
            env_name = key.upper()
            # public_key 是 PEM 多行文本，含换行符：必须用 GITHUB_ENV 的 heredoc 语法，
            # 否则换行会被截断成多个环境变量，Rust 侧读到的是残缺公钥（解析失败）。
            if "\n" in value:
                f.write(f"{env_name}<<EOF\n{value.rstrip()}\nEOF\n")
            else:
                f.write(f"{env_name}={value}\n")


def write_config_json(path: str, merged: dict[str, str], data: dict) -> None:
    """落一份**规范化后**的 config.json（Electron 侧进 asar 的那一份）。

    ⚠️ 与 env 不同：这里要保持 JSON 类型 —— bool 就是 bool、app_url 数组就是数组。
    Electron 的 config.ts 按真实类型解析，不做字符串宽容。
    """
    out: dict[str, object] = {}
    for key, value in merged.items():
        if key in JSON_KEYS:
            out[key] = json.loads(value)
        elif key in BOOL_KEYS:
            # merged 里是已规范化的字符串（"True"/"False"），这里转回真 bool。
            # ⚠️ 不能写 bool(value)：非空字符串恒为 True，会把 "False" 变成 true。
            out[key] = value.strip().lower() == "true"
        elif key == "app_url":
            stripped = value.lstrip()
            if stripped.startswith("["):
                out[key] = json.loads(value)
            else:
                out[key] = value
        elif key == "android_permissions":
            # merged 里是紧凑 JSON 数组文本（"[]" / "[\"camera\"]"），config.json 要保持真数组；
            # 本壳（Electron/Windows）不消费它（config.ts 的 ANDROID_ONLY_KEYS 接受并忽略）。
            out[key] = json.loads(value)
        else:
            out[key] = value
    # android_* 是仅 Android 使用的字段，不进 Windows 绿色版的 config.json ——
    # 那 4 个字段里含 keystore base64 与明文密码，没有任何理由随包分发。
    for key in SIGNING_KEYS:
        out.pop(key, None)

    p = pathlib.Path(path)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(out, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"已写出规范化配置：{path}")


def setup_android_keystore(merged: dict[str, str], out_dir: str) -> dict[str, str]:
    """把 JSON 里的 keystore 落盘，并返回要追加进 GITHUB_ENV 的签名变量。"""
    ks_b64 = merged["android_keystore_base64"].strip()
    if not ks_b64:
        print("Android 签名：JSON 未提供 keystore（将回落到仓库 secrets / 一次性 CI keystore）")
        return {}

    alias = merged["android_key_alias"].strip()
    storepass = merged["android_keystore_password"]
    if not alias or not storepass:
        raise ConfigError("配置了 android_keystore_base64 时，"
                          "必须同时提供 android_key_alias 和 android_keystore_password")
    try:
        ks_bytes = base64.b64decode(ks_b64, validate=False)
    except Exception as e:  # noqa: BLE001
        raise ConfigError(f"android_keystore_base64 不是合法 base64：{e}") from e
    if len(ks_bytes) < 64:
        raise ConfigError("android_keystore_base64 解码后内容过小，疑似不完整"
                          "（正常 keystore 至少几百字节）")

    directory = pathlib.Path(out_dir)
    directory.mkdir(parents=True, exist_ok=True)
    ks_path = directory / "release.keystore"
    ks_path.write_bytes(ks_bytes)

    keypass = merged["android_key_password"] or storepass
    for secret in (storepass, keypass):
        if secret:
            print(f"::add-mask::{secret}")

    print(f"Android 签名：使用 JSON 提供的 keystore（{len(ks_bytes)} 字节，已写入 {ks_path}）")
    return {
        "ANDROID_KEYSTORE_PATH": str(ks_path),
        "ANDROID_KEY_ALIAS": alias,
        "ANDROID_KEYSTORE_PASSWORD": storepass,
        "ANDROID_KEY_PASSWORD": keypass,
    }


def display(key: str, value: str) -> str:
    if key in ("app_url", "icon_url"):
        return mask(value)
    if key in ("android_keystore_password", "android_key_password", "resolve_secret"):
        return "***（已配置）" if value else "（未配置）"
    if key == "android_keystore_base64":
        return f"（已配置，{len(value)} 字符）" if value else "（未配置）"
    return value


# ---------------------------------------------------------------------------
# 自测：把契约钉死在 CI 里
# ---------------------------------------------------------------------------
def _expect_error(data: dict, needle: str) -> None:
    try:
        build_merged(data)
    except ConfigError as e:
        assert needle in str(e), f"报错文案里应包含 {needle!r}，实际是：{e}"
        return
    raise AssertionError(f"应当报错但通过了：{data}（期望提到 {needle!r}）")


def selftest() -> int:
    # 自测会把 build_merged 跑很多遍，其中的 ::warning:: 提示会刷屏；这里整体静音，
    # 只有断言失败才需要看到输出（断言异常照常抛出）。
    import contextlib
    import io

    with contextlib.redirect_stdout(io.StringIO()):
        _selftest_cases()
    print("解析器自测全部通过 ✓")
    return 0


def _selftest_cases() -> None:
    # 最小配置：只有 app_url
    m = build_merged({"app_url": "https://example.com"})
    assert m["app_url"] == "https://example.com"
    assert m["app_name"] == "Web2App"
    assert m["title"] == "Web2App", "title 空 → 回落 app_name"
    assert m["proxy_enabled"] == "False" and m["tabs_enabled"] == "False"
    assert m["debug"] == "False", "debug 默认必须关闭（默认开启＝往用户数据目录里塞日志文件）"
    # mobile_bar 默认必须关闭：默认开启等于给每个移动端包凭空加一条常驻底栏，
    # 而「打包的人没写这个字段」却多出一块 UI 是最难解释的那类默认行为。
    assert m["mobile_bar"] == "False", "mobile_bar 默认必须关闭（默认＝无底栏）"
    assert m["data_dir"] == "data", "data_dir 默认是相对路径 data"
    assert m["proxy_bypass"] == "localhost;127.0.0.1;<-loopback>"

    # title 显式给出时不回落
    assert build_merged({"app_url": "https://a.com", "title": "我的应用"})["title"] == "我的应用"

    # ---- version：默认 0.0.1；只认三段数字（Windows MSI/VERSIONINFO 规则）----
    assert build_merged({"app_url": "https://a.com"})["version"] == "0.0.1", "version 默认 0.0.1"
    assert build_merged({"app_url": "https://a.com", "version": " 1.2.3 "})["version"] == "1.2.3", \
        "version 去首尾空白"
    assert build_merged({"app_url": "https://a.com", "version": "255.255.65535"})["version"] == "255.255.65535"
    for bad in ("1.2", "v1.2.3", "1.2.3.4", "1.2.3-beta", "1.2.3+build"):
        _expect_error({"app_url": "https://a.com", "version": bad}, "三段数字版本号")
    for bad in ("256.0.0", "1.256.0", "0.0.65536"):
        _expect_error({"app_url": "https://a.com", "version": bad}, "超出 Windows 版本范围")
    # 数字经通用通道先 str() 成 "1" ⇒ 落在格式校验（bool 不被 str，落在「必须是字符串」）
    _expect_error({"app_url": "https://a.com", "version": 1}, "三段数字版本号")
    _expect_error({"app_url": "https://a.com", "version": True}, "必须是字符串")
    # 显式 null = 用默认值（与其它字段同规则）
    assert build_merged({"app_url": "https://a.com", "version": None})["version"] == "0.0.1"

    # 布尔：真 true 通过
    assert build_merged({"app_url": "https://a.com", "proxy_enabled": True})["proxy_enabled"] == "True"
    # debug 也是布尔字段：真 true 通过，"true" 字符串拒绝
    assert build_merged({"app_url": "https://a.com", "debug": True})["debug"] == "True"
    _expect_error({"app_url": "https://a.com", "debug": "true"}, "必须是布尔值")
    # mobile_bar 同样是布尔字段：真 true 通过，任何字符串拒绝（与 tabs_enabled 同规则）
    assert build_merged({"app_url": "https://a.com", "mobile_bar": True})["mobile_bar"] == "True"
    _expect_error({"app_url": "https://a.com", "mobile_bar": "true"}, "必须是布尔值")
    _expect_error({"app_url": "https://a.com", "mobile_bar": 1}, "必须是布尔值")
    # 布尔：字符串拒绝（两壳曾对 "on" 判定相反）
    for bad in ("true", "false", "1", "yes", "on", ""):
        _expect_error({"app_url": "https://a.com", "proxy_enabled": bad}, "必须是布尔值")

    # app_url：真数组 + 公钥
    m = build_merged({"app_url": ["a.com", "b.com"], "public_key": "AAAA"})
    assert json.loads(m["app_url"]) == ["a.com", "b.com"]
    # app_url：数组但无公钥 → 失败
    _expect_error({"app_url": ["a.com"]}, "必须提供 public_key")
    # app_url：空数组 / 空串 → 失败
    _expect_error({"app_url": []}, "不能为空")
    _expect_error({"app_url": "   "}, "缺少必填字段 app_url")
    # app_url：字符串形态的 JSON 数组（老写法）仍可用
    m = build_merged({"app_url": '["a.com"]', "public_key": "AAAA"})
    assert json.loads(m["app_url"]) == ["a.com"]
    # app_url：以 [ 开头但不是合法数组 → 明确报错
    _expect_error({"app_url": "[a.com]"}, "不是合法的 JSON 字符串数组")

    # window_title 已移除 → 硬失败并指路
    _expect_error({"app_url": "https://a.com", "window_title": "X"}, "已移除")
    # 未知字段 → 硬失败 + 近似名建议
    _expect_error({"app_url": "https://a.com", "proxy_enable": True}, "proxy_enabled")

    # ---- data_dir ----
    # 默认 = 相对路径 "data"（程序所在目录下的 data\）
    assert build_merged({"app_url": "https://a.com"})["data_dir"] == "data"
    # ⚠️⚠️ 空串是**有含义的取值**（系统默认程序目录），必须原样保留 ——
    #    一旦被当成「未设置」而回落默认值，用户就再也表达不出「用系统目录」这个意图。
    assert build_merged({"app_url": "https://a.com", "data_dir": ""})["data_dir"] == ""
    assert build_merged({"app_url": "https://a.com", "data_dir": "   "})["data_dir"] == "", \
        "纯空白视同空串（但保留空串语义）"
    # 自定义相对路径 / 绝对路径：只去首尾空白，内容原样
    assert build_merged({"app_url": "https://a.com", "data_dir": " mydata "})["data_dir"] == "mydata"
    assert build_merged({"app_url": "https://a.com", "data_dir": "D:\\appdata"})["data_dir"] == "D:\\appdata"
    # 非字符串 → 硬失败。若宽容地 str() 掉，`true` 会变成 "True" 并静默建出一个叫 True 的目录。
    _expect_error({"app_url": "https://a.com", "data_dir": True}, "必须是字符串")
    _expect_error({"app_url": "https://a.com", "data_dir": 123}, "必须是字符串")
    # 旧布尔字段已移除 → 硬失败，且把 false / true 各自对应的新写法讲清楚
    _expect_error({"app_url": "https://a.com", "data_dir_default": False}, "已移除")
    _expect_error({"app_url": "https://a.com", "data_dir_default": False},
                  '"data_dir": "data"')

    # extra：对象 / 标量都能序列化
    assert build_merged({"app_url": "https://a.com", "extra": {"k": "v"}})["extra"] == '{"k":"v"}'
    assert build_merged({"app_url": "https://a.com"})["extra"] == "{}"

    # ---- android_permissions：默认 []；白名单三值；去空白 + 去重保序；非法形态硬失败 ----
    assert build_merged({"app_url": "https://a.com"})["android_permissions"] == "[]", \
        "android_permissions 默认必须是 []（不声明任何可选权限）"
    m = build_merged({"app_url": "https://a.com",
                      "android_permissions": ["camera", "microphone", "location"]})
    assert json.loads(m["android_permissions"]) == ["camera", "microphone", "location"]
    # 去首尾空白；重复值去重但保留首次顺序
    m = build_merged({"app_url": "https://a.com",
                      "android_permissions": [" camera ", "camera", "location", "location "]})
    assert json.loads(m["android_permissions"]) == ["camera", "location"]
    # 显式 null = 用默认值
    assert build_merged({"app_url": "https://a.com",
                         "android_permissions": None})["android_permissions"] == "[]"
    # 非数组（字符串 / 布尔 / 对象）→ 硬失败
    _expect_error({"app_url": "https://a.com", "android_permissions": "camera"}, "权限名字符串数组")
    _expect_error({"app_url": "https://a.com", "android_permissions": True}, "权限名字符串数组")
    _expect_error({"app_url": "https://a.com", "android_permissions": {"camera": True}},
                  "权限名字符串数组")
    # 数组里的坏元素：未知权限名 / 非字符串 / 空串
    _expect_error({"app_url": "https://a.com", "android_permissions": ["gps"]}, "未知权限")
    _expect_error({"app_url": "https://a.com", "android_permissions": ["camera", "gps"]},
                  "只允许：camera/microphone/location")
    _expect_error({"app_url": "https://a.com", "android_permissions": [123]}, "元素必须是字符串")
    _expect_error({"app_url": "https://a.com", "android_permissions": [""]}, "元素不能为空")
    _expect_error({"app_url": "https://a.com", "android_permissions": ["   "]}, "元素不能为空")

    # resolve_secret 去空白
    assert build_merged({"app_url": "https://a.com", "resolve_secret": " s "})["resolve_secret"] == "s"

    # 未配置 resolve_secret 时的提示**只在域名数组模式**出现 —— 判据与
    # `resolve-core::parse_domains` / `navigate.ts::needsDnsResolve` 同源（文本以 `[` 开头）。
    import contextlib
    import io

    def _stdout_of(payload: dict) -> str:
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            build_merged(payload)
        return buf.getvalue()

    # 单 URL 字符串：不告警，只说明该字段当前不参与
    out = _stdout_of({"app_url": "https://a.com"})
    assert "::warning::" not in out, f"单 URL 模式不该产生 warning：{out!r}"
    assert "resolve_secret" in out and "不参与" in out, f"应说明该字段当前不参与：{out!r}"

    # 域名数组（含「JSON 数组文本」老写法）：必须告警
    assert "::warning::resolve_secret 未配置" in _stdout_of(
        {"app_url": ["a.com"], "public_key": "AAAA"})
    assert "::warning::resolve_secret 未配置" in _stdout_of(
        {"app_url": '["a.com"]', "public_key": "AAAA"})

    # 配了 Secret：两种模式都安静（连说明行也不出现）
    for payload in ({"app_url": "https://a.com", "resolve_secret": "s"},
                    {"app_url": ["a.com"], "public_key": "AAAA", "resolve_secret": "s"}):
        assert "resolve_secret" not in _stdout_of(payload)

    # write_config_json：bool 保持真 bool、数组保持数组、android_* 不落盘
    import tempfile

    with tempfile.TemporaryDirectory() as d:
        p = os.path.join(d, "config.json")
        merged = build_merged({
            "app_url": ["a.com"],
            "public_key": "AAAA",
            "tabs_enabled": True,
            "mobile_bar": True,
            "debug": True,
            "data_dir": "",
            "android_permissions": ["camera", "location"],
            "android_keystore_base64": "eHh4",
        })
        write_config_json(p, merged, {})
        out = json.loads(pathlib.Path(p).read_text(encoding="utf-8"))
        assert out["tabs_enabled"] is True, "config.json 里必须是真 bool"
        assert out["mobile_bar"] is True, "mobile_bar 在 config.json 里也必须是真 bool"
        assert out["debug"] is True, "debug 在 config.json 里也必须是真 bool（Electron 侧按真 bool 解析）"
        assert out["app_url"] == ["a.com"], "config.json 里数组要保持数组"
        assert out["android_permissions"] == ["camera", "location"], \
            "android_permissions 在 config.json 里必须是真数组（不是 JSON 文本）"
        assert "android_keystore_base64" not in out, "签名 4 字段不应进 Windows 包的 config.json"
        assert out["data_dir"] == "", "空串必须原样落到 config.json（不被改成默认值）"

    # ---- 按打包目标的跨字段校验（--target android）----
    # 内嵌凭据 + 关代理：Windows 允许（WebView2 鉴权钩子），Android 必须硬失败。
    assert url_contains_userinfo("https://u:p@host/path")
    assert url_contains_userinfo("  http://a@host  ")
    assert not url_contains_userinfo("https://host/p@th")
    assert not url_contains_userinfo("host/u@x")
    m = build_merged({"app_url": "https://u:p@host", "proxy_enabled": True})
    validate_for_target("android", m)  # 开了代理：放行
    validate_for_target("windows", build_merged({"app_url": "https://u:p@host"}))  # Windows 直连放行
    validate_for_target(None, build_merged({"app_url": "https://u:p@host"}))  # 未指定目标不校验
    validate_for_target("android", build_merged({"app_url": "https://plain.host"}))  # 无凭据直连放行
    _expect_error_android = build_merged({"app_url": "https://u:p@host"})
    try:
        validate_for_target("android", _expect_error_android)
        raise AssertionError("Android + 内嵌凭据 + 直连应硬失败")
    except ConfigError as e:
        assert "proxy_enabled" in str(e) and "401" in str(e), f"报错要给修法与后果：{e}"
        assert "***@" in str(e), "报错里的 URL 必须脱敏"
    # 域名数组 + Android + 关代理：只 warning（解密结果静态不可知），不硬失败
    m = build_merged({"app_url": ["a.com"], "public_key": "K"})
    import io as _io

    buf = _io.StringIO()
    import contextlib as _cl

    with _cl.redirect_stdout(buf):
        validate_for_target("android", m)
    assert "::warning::" in buf.getvalue() and "proxy_enabled" in buf.getvalue()
    # 数组 + 开代理：不应再有本校验的代理警告（数组模式仍可能有 resolve_secret 的既有警告，
    # 那是 build_merged 的职责，这里只按代理关键词判定）。
    buf2 = _io.StringIO()
    with _cl.redirect_stdout(buf2):
        validate_for_target("android", build_merged(
            {"app_url": ["a.com"], "public_key": "K", "proxy_enabled": True}))
    assert "proxy_enabled" not in buf2.getvalue()

    # write_env：空串必须写成 `DATA_DIR=`（而不是整行消失），否则 Rust 侧读不到「用系统目录」这个意图
    with tempfile.TemporaryDirectory() as d:
        env_path = os.path.join(d, "env")
        with open(env_path, "w", encoding="utf-8"):
            pass
        write_env(env_path, build_merged({"app_url": "https://a.com", "data_dir": ""}))
        env_text = pathlib.Path(env_path).read_text(encoding="utf-8")
        assert "DATA_DIR=\n" in env_text, f"空串必须落成一行 DATA_DIR=，实际：{env_text!r}"
        # debug 也要落进 env（Rust 侧 DEBUG 是编译期常量，Electron 侧读 config.json）
        assert "DEBUG=False\n" in env_text, f"debug 默认必须写出一行 DEBUG=False，实际：{env_text!r}"
        # mobile_bar 同理：Rust 侧的编译期常量靠这一行拿到取值（桌面目标读不到就等于关闭）
        assert "MOBILE_BAR=False\n" in env_text, f"默认必须写出一行 MOBILE_BAR=False，实际：{env_text!r}"
        # android_permissions 默认落 ANDROID_PERMISSIONS=[]（Manifest 补丁脚本读它决定注入项）
        assert "ANDROID_PERMISSIONS=[]\n" in env_text, \
            f"默认必须写出一行 ANDROID_PERMISSIONS=[]，实际：{env_text!r}"
        with open(env_path, "w", encoding="utf-8"):
            pass
        write_env(env_path, build_merged({"app_url": "https://a.com", "debug": True}))
        assert "DEBUG=True\n" in pathlib.Path(env_path).read_text(encoding="utf-8"), \
            "debug=true 必须落成 DEBUG=True（落成 1/on 之类会被 Rust 侧 env_bool! 白名单挡下）"
        with open(env_path, "w", encoding="utf-8"):
            pass
        write_env(env_path, build_merged(
            {"app_url": "https://a.com", "android_permissions": ["camera", "location"]}))
        env_text2 = pathlib.Path(env_path).read_text(encoding="utf-8")
        assert 'ANDROID_PERMISSIONS=["camera","location"]\n' in env_text2, \
            f"权限数组必须以紧凑 JSON 单行落 env，实际：{env_text2!r}"


# ---------------------------------------------------------------------------
# 入口
# ---------------------------------------------------------------------------
def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description="打包配置 JSON 解析（唯一真源）")
    ap.add_argument("--json", default=os.environ.get("CONFIG_JSON"),
                    help="配置 JSON 字符串（默认取环境变量 CONFIG_JSON）")
    ap.add_argument("--url", default=os.environ.get("CONFIG_URL"),
                    help="返回配置 JSON 的 URL（默认取环境变量 CONFIG_URL）")
    ap.add_argument("--env-out", default=os.environ.get("GITHUB_ENV"),
                    help="把编译期环境变量追加到该文件（CI 里就是 $GITHUB_ENV）")
    ap.add_argument("--config-out", help="额外写出规范化后的 config.json（Electron 进 asar 的那份）")
    ap.add_argument("--target", choices=("android", "windows", "electron"), default=None,
                    help="打包目标：启用目标相关的跨字段校验（目前仅 android 有额外硬规则："
                         "URL 内嵌账号密码必须走代理）。CI 的 android job 必传。")
    ap.add_argument("--android-keystore", action="store_true",
                    help="处理 android_keystore_base64：落盘并注入 ANDROID_KEYSTORE_* 变量")
    ap.add_argument("--keystore-out-dir",
                    default=os.path.join(os.environ.get("RUNNER_TEMP", "/tmp"), "android-signing"),
                    help="keystore 落盘目录（默认 $RUNNER_TEMP/android-signing）")
    ap.add_argument("--selftest", action="store_true", help="跑内置契约自测后退出")
    ap.add_argument("--print-contract-keys", action="store_true",
                    help="打印契约字段名（单行 JSON 数组）后退出；供 CI 断言「生成的模板没漏字段」")
    args = ap.parse_args(argv)

    # Windows runner 上 Python 默认 stdout 是 cp1252，输出中文会 UnicodeEncodeError。
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except Exception:  # noqa: BLE001
            pass

    if args.print_contract_keys:
        print(json.dumps(list(DEFAULTS), ensure_ascii=False, separators=(",", ":")))
        return 0

    if args.selftest:
        return selftest()

    try:
        data = parse_json(read_raw(args.json, args.url))
        merged = build_merged(data)
        # 目标相关组合校验放在任何落盘 / 注入之前：硬失败时不写 GITHUB_ENV、不落 keystore。
        validate_for_target(args.target, merged)

        signing: dict[str, str] = {}
        if args.android_keystore:
            signing = setup_android_keystore(merged, args.keystore_out_dir)

        if args.env_out:
            write_env(args.env_out, merged)
            if signing:
                with open(args.env_out, "a", encoding="utf-8") as f:
                    for k, v in signing.items():
                        f.write(f"{k}={v}\n")
        if args.config_out:
            write_config_json(args.config_out, merged, data)
    except ConfigError as e:
        print(f"::error::{e}")
        return 1

    print("已注入环境变量：")
    for key, value in merged.items():
        print(f"  {key.upper()}={display(key, value)}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
