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
| extra                    | 任意 JSON             | {}                                  |
| icon_url                 | string                | ""                                  |
| proxy_enabled            | bool                  | false                               |
| proxy_http               | string                | ""                                  |
| proxy_https              | string                | ""                                  |
| proxy_bypass             | string                | "localhost;127.0.0.1;<-loopback>"   |
| data_dir                 | string                | "data"                              |
| tabs_enabled             | bool                  | false                               |
| public_key               | string                | ""（app_url 为数组时必填）          |
| resolve_secret           | string                | ""                                  |
| android_keystore_base64  | string                | ""（仅 Android）                    |
| android_key_alias        | string                | ""（仅 Android）                    |
| android_keystore_password| string                | ""（仅 Android）                    |
| android_key_password     | string                | ""（仅 Android）                    |

`data_dir` 三种取值（两壳行为一致，见 docs/packaging-config.md §4.3）：

| 取值             | 含义                                                             |
|------------------|------------------------------------------------------------------|
| `"data"`（默认） | 程序自身所在目录（exe 目录）下的 `data\`，跟着程序走（绿色版）    |
| `""`（空串）     | 系统默认程序目录（`%LOCALAPPDATA%\<app_name>`），不受程序位置影响 |
| 其它相对路径     | 同上，基准仍是 exe 目录（**不是 shell 的 cwd**）                   |
| 绝对路径         | 原样使用                                                          |

⚠️ 基准必须是 **exe 所在目录**而不是进程工作目录：快捷方式的「起始位置」、命令行、拖拽启动都会改变
cwd，用它会让同一个包在不同启动方式下把数据落到不同地方。

**已移除**（配置里出现即报错，并给出对应改法）：

- `window_title`（旧字段，等价于 `title`）；
- `data_dir_default`（旧**布尔**字段，改为**字符串** `data_dir`）：
  `false` → `"data"`，`true` → `""`。

## 用法

    # 从字符串 / URL 读配置，把编译期环境变量写进 GITHUB_ENV
    python3 .github/scripts/parse-packaging-config.py --json '<JSON>' --env-out "$GITHUB_ENV"

    # Android：额外把 keystore 落盘并注入 ANDROID_KEYSTORE_* 变量
    ... --android-keystore

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
# 契约：默认值表。**顺序即打印顺序**，也等于写进 GITHUB_ENV 的顺序。
# 布尔字段的默认值必须是真 bool（不是 "false" 字符串）—— 这是本文件的全部意义之一。
# ---------------------------------------------------------------------------
DEFAULTS: dict[str, object] = {
    "app_url": "",
    "app_name": "Web2App",
    "title": "",
    "extra": {},
    "icon_url": "",
    "proxy_enabled": False,
    "proxy_http": "",
    "proxy_https": "",
    # ⚠️ 两壳必须一致：webview-app 的 config.rs 编译期默认值、electron 的 DEFAULTS
    #    都用这同一个串。留空 = 不下发 bypass 参数（即空值不覆盖 Chromium 默认）。
    "proxy_bypass": "localhost;127.0.0.1;<-loopback>",
    # 数据目录：默认 "data" = 程序所在目录（exe 目录）下的 data\，跟着程序走；
    # 空串 = 系统默认程序目录（%LOCALAPPDATA%\<app_name>），不受程序位置影响。
    # ⚠️ 空串在本字段里**是有含义的取值**，不是「未设置」——两壳的读取都必须保留空串，
    #    Rust 侧因此不能用 env_str!（它把空串当未设置）。
    "data_dir": "data",
    "tabs_enabled": False,
    "public_key": "",
    "resolve_secret": "",
    "android_keystore_base64": "",
    "android_key_alias": "",
    "android_keystore_password": "",
    "android_key_password": "",
}

BOOL_KEYS = {"proxy_enabled", "tabs_enabled"}
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
        if key in BOOL_KEYS:
            merged[key] = normalize_bool(key, value)
        else:
            merged[key] = value if isinstance(value, bool) else str(value)

    # 先归一 public_key：normalize_app_url 要用它判断「数组模式是否带公钥」。
    merged["public_key"] = normalize_public_key(merged["public_key"])
    merged["app_url"] = normalize_app_url(raw_app_url, str(merged["public_key"]))
    merged["extra"] = normalize_extra(raw_extra)
    merged["data_dir"] = normalize_data_dir(raw_data_dir)
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

    if not merged["resolve_secret"]:
        print("::warning::resolve_secret 未配置：将使用代码内置默认 Secret"
              "（所有产物共享同一固定密钥串，安全门槛较低）。"
              "建议配置独立的 resolve_secret，并同步服务端 encrypt_url.py / resolve-tool。")

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
    assert m["data_dir"] == "data", "data_dir 默认是相对路径 data"
    assert m["proxy_bypass"] == "localhost;127.0.0.1;<-loopback>"

    # title 显式给出时不回落
    assert build_merged({"app_url": "https://a.com", "title": "我的应用"})["title"] == "我的应用"

    # 布尔：真 true 通过
    assert build_merged({"app_url": "https://a.com", "proxy_enabled": True})["proxy_enabled"] == "True"
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

    # resolve_secret 去空白
    assert build_merged({"app_url": "https://a.com", "resolve_secret": " s "})["resolve_secret"] == "s"

    # write_config_json：bool 保持真 bool、数组保持数组、android_* 不落盘
    import tempfile

    with tempfile.TemporaryDirectory() as d:
        p = os.path.join(d, "config.json")
        merged = build_merged({
            "app_url": ["a.com"],
            "public_key": "AAAA",
            "tabs_enabled": True,
            "data_dir": "",
            "android_keystore_base64": "eHh4",
        })
        write_config_json(p, merged, {})
        out = json.loads(pathlib.Path(p).read_text(encoding="utf-8"))
        assert out["tabs_enabled"] is True, "config.json 里必须是真 bool"
        assert out["app_url"] == ["a.com"], "config.json 里数组要保持数组"
        assert "android_keystore_base64" not in out, "android_* 不应进 Windows 包的 config.json"
        assert out["data_dir"] == "", "空串必须原样落到 config.json（不被改成默认值）"

    # write_env：空串必须写成 `DATA_DIR=`（而不是整行消失），否则 Rust 侧读不到「用系统目录」这个意图
    with tempfile.TemporaryDirectory() as d:
        env_path = os.path.join(d, "env")
        with open(env_path, "w", encoding="utf-8"):
            pass
        write_env(env_path, build_merged({"app_url": "https://a.com", "data_dir": ""}))
        env_text = pathlib.Path(env_path).read_text(encoding="utf-8")
        assert "DATA_DIR=\n" in env_text, f"空串必须落成一行 DATA_DIR=，实际：{env_text!r}"


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
