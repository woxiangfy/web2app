#!/usr/bin/env python3
"""服务端 URL 加密脚本（Ed25519 签名 + AES-256-GCM 对称加密）。

设计目标：
  1. 防篡改（硬保证）：Ed25519 私钥签名，客户端公钥验签。攻击者无私钥无法伪造。
  2. 隐藏真实网址（软保证）：AES-256-GCM 认证加密，防公网批量爬取（不防反编译客户端）。
  3. 每次加密结果不同：Nonce(12B) 每次随机（同时充当 HKDF 的 salt），密文每次都变。
  4. 密文尽可能短：固定开销 97 字节（Nonce12 + 分隔/签发日/有效期5 + 签名64 + tag16）+ URL，
     base64 后通常 160~180 字符，单条 DNS TXT 可放下。

数据格式（与客户端 crates/resolve-core/src/lib.rs 严格对称）：
  Payload   = URL || 0x00 || 签发日d(2B 大端) || 有效期ttl_d(2B 大端)
  Signature = Ed25519.sign(Payload)                            # 64 字节
  Key       = HKDF-SHA256(ikm=Secret, salt=Nonce, info="web2app-resolve")
  Cipher    = AES-256-GCM.encrypt(Key, Nonce, Payload || Signature)   # 附 16 字节 tag
  TXT 值    = base64( Nonce(12B) || Cipher )

旧格式尾部结构不同（v3 为 8B 秒级时间戳 + 8B 秒级 ttl；v2 多 0x00 || 随机盐 8B；
v1 无 ttl 字段），客户端解密端只认上面的 v4 格式，旧 TXT 密文一律拒绝、需重新签发。

签发日与有效期（ttl_days）语义：时间以 UTC 天为单位，由【生成时】决定，随密文下发。
  - d = 签发日距固定基准日 2026-01-01 的天数（u16，可覆盖至约 2205 年）。
  - ttl_d > 0：客户端校验 今日 - d <= ttl_d，超出即拒绝（最小粒度 1 天）。
  - ttl_d = 0：永不失效（客户端跳过日期校验）。

用法：
  python encrypt_url.py genkey <output_dir>                       # 生成 Ed25519 密钥对
  python encrypt_url.py <private-key> <url> [--ttl-days 天] [--secret 值]  # 加密单个 URL（默认 0=永不失效）
  python encrypt_url.py <private-key> --batch [--ttl-days 天] [--secret 值] < urls.txt  # 批量加密

<private-key> 与 resolve-tool 一致，二选一自动识别：
  - PEM 文件路径（历史用法）；
  - 直接内联 PEM 字符串本体（strip 后以 -----BEGIN 开头，须含完整换行，勿带 # 注释行）。

输出：base64 密文（单行，可直接写入 DNS TXT 记录）。
"""

import sys
import os
import base64
import datetime

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.hkdf import HKDF
from cryptography.hazmat.primitives import hashes

# 与客户端 crates/resolve-core/src/lib.rs 中的常量保持一致。
# Secret 取值优先级（与 resolve-tool 一致）：--secret 参数 > 环境变量 RESOLVE_SECRET > 内置默认。
# 注意：Secret 不一致时 HKDF 派生密钥不同、客户端解密必然失败，两端必须使用同一个值。
DEFAULT_SECRET = "web2app-fixed-secret-2026"
HKDF_INFO = b"web2app-resolve"
NONCE_LEN = 12
KEY_LEN = 32
# 默认有效期 0 = 永不失效；需要防过期时显式传 --ttl-days 正整数。
DEFAULT_TTL_DAYS = 0
# 日期基准日 2026-01-01（UTC）；签发日 d = 今日 - 基准日（u16，与 resolve-core 对称）。
BASE_DATE = datetime.date(2026, 1, 1)
FIELD_MAX = 65535  # 签发日 / ttl_days 的 u16 上界


def derive_key(nonce: bytes, secret: bytes | None = None) -> bytes:
    """HKDF-SHA256(ikm=Secret, salt=Nonce, info=HKDF_INFO) -> 32 字节密钥。

    secret 为 None 时按 环境变量 RESOLVE_SECRET > 内置默认 取值（UTF-8 字节）。
    """
    if secret is None:
        secret = os.environ.get("RESOLVE_SECRET", DEFAULT_SECRET).encode("utf-8")
    hk = HKDF(algorithm=hashes.SHA256(), length=KEY_LEN, salt=nonce, info=HKDF_INFO)
    return hk.derive(secret)


def current_day() -> int:
    """当前 UTC 日期相对基准日 2026-01-01 的天数（与 resolve-core 的 current_day 对称）。"""
    return (datetime.datetime.now(datetime.timezone.utc).date() - BASE_DATE).days


def encrypt_url(priv_key: Ed25519PrivateKey, url: str, day: int | None = None,
                ttl_days: int = DEFAULT_TTL_DAYS, secret: bytes | None = None) -> str:
    """加密 + 签名一个 URL，返回 base64 密文（单行）。

    day 为签发日（距 2026-01-01 的 UTC 天数），None 时取今日；ttl_days 为有效期
    天数（>0 按窗口校验，=0 永不失效，u16 上界 65535）。secret 为显式 Secret
    字节（对应 resolve-tool 的 --secret）；None 时走环境变量 RESOLVE_SECRET / 内置默认。
    """
    url_b = url.encode("utf-8")
    if day is None:
        day = current_day()

    # Payload = URL || 0x00 || d(2B 大端) || ttl_d(2B 大端)（v4 唯一格式，尾部固定 4B）。
    payload = url_b + b"\x00" + day.to_bytes(2, "big") + ttl_days.to_bytes(2, "big")

    # Ed25519 签名（64 字节）
    sig = priv_key.sign(payload)

    # Nonce + 派生密钥 + AES-GCM 加密
    nonce = os.urandom(NONCE_LEN)
    key = derive_key(nonce, secret)
    aesgcm = AESGCM(key)
    cipher = aesgcm.encrypt(nonce, payload + sig, None)

    # TXT = base64( Nonce || Cipher )
    return base64.b64encode(nonce + cipher).decode("ascii")


def gen_keypair(out_dir: str) -> None:
    """生成 Ed25519 密钥对，写私钥（PEM PKCS8）与公钥（裸 base64 32 字节 + SPKI PEM）。"""
    os.makedirs(out_dir, exist_ok=True)
    priv = Ed25519PrivateKey.generate()
    pub = priv.public_key()

    priv_path = os.path.join(out_dir, "private_key.pem")
    with open(priv_path, "wb") as f:
        f.write(
            priv.private_bytes(
                encoding=serialization.Encoding.PEM,
                format=serialization.PrivateFormat.PKCS8,
                encryption_algorithm=serialization.NoEncryption(),
            )
        )

    # 客户端用：裸 32 字节公钥的 base64（直接填 PUBLIC_KEY）。
    raw_pub = pub.public_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PublicFormat.Raw,
    )
    pub_b64_path = os.path.join(out_dir, "public_key.b64")
    with open(pub_b64_path, "w") as f:
        f.write(base64.b64encode(raw_pub).decode("ascii"))

    # 也导出一份 SPKI PEM 备用。
    pub_pem_path = os.path.join(out_dir, "public_key.pem")
    with open(pub_pem_path, "wb") as f:
        f.write(
            pub.public_bytes(
                encoding=serialization.Encoding.PEM,
                format=serialization.PublicFormat.SubjectPublicKeyInfo,
            )
        )

    print(f"私钥（服务端持有）: {priv_path}")
    print(f"公钥裸 base64（客户端 PUBLIC_KEY）: {pub_b64_path}")
    print(f"公钥 SPKI PEM（备用）: {pub_pem_path}")
    print()
    print(f"公钥 base64 内容：")
    print(base64.b64encode(raw_pub).decode("ascii"))


def load_private_key(path: str) -> Ed25519PrivateKey:
    with open(path, "rb") as f:
        pem = f.read()
    key = serialization.load_pem_private_key(pem, password=None)
    if not isinstance(key, Ed25519PrivateKey):
        raise TypeError("密钥不是 Ed25519 私钥")
    return key


def resolve_private_key(spec: str) -> Ed25519PrivateKey:
    """私钥入参二选一：strip 后以 -----BEGIN 开头 ⇒ 内联 PEM 字符串；否则按文件路径读取。

    与 resolve-tool 的 resolve_signing_key 判定规则保持一致。注意 cryptography 的 PEM
    加载器要求标准换行结构，内联时请传入完整多行 PEM（shell 下用引号 / $'...' 传入），
    且不要夹带 genkey --stdout 打印的 # 注释行。
    """
    spec = spec.strip()
    if spec.startswith("-----BEGIN"):
        pem = spec.encode("utf-8")
    else:
        with open(spec, "rb") as f:
            pem = f.read()
    key = serialization.load_pem_private_key(pem, password=None)
    if not isinstance(key, Ed25519PrivateKey):
        raise TypeError("密钥不是 Ed25519 私钥")
    return key


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2

    if sys.argv[1] == "genkey":
        out_dir = sys.argv[2] if len(sys.argv) > 2 else "keys"
        gen_keypair(out_dir)
        return 0

    if len(sys.argv) < 3:
        print(__doc__)
        return 2

    key_spec = sys.argv[1]
    priv = resolve_private_key(key_spec)

    # 解析 --ttl-days 参数（单位：天）。支持放在任意位置。
    ttl_days = DEFAULT_TTL_DAYS
    args = sys.argv[2:]
    if "--ttl-days" in args:
        i = args.index("--ttl-days")
        if i + 1 >= len(args):
            print("--ttl-days 需要跟一个天数（整数）", file=sys.stderr)
            return 2
        try:
            ttl_days = int(args[i + 1])
        except ValueError:
            print(f"--ttl-days 必须是整数天数，收到：{args[i + 1]}", file=sys.stderr)
            return 2
        if ttl_days < 0:
            print("--ttl-days 不能为负数", file=sys.stderr)
            return 2
        if ttl_days > FIELD_MAX:
            print(f"--ttl-days 超出上限 {FIELD_MAX}（收到 {ttl_days}）", file=sys.stderr)
            return 2
        # 去掉 --ttl-days 和它的值
        del args[i:i + 2]

    # 解析 --secret 参数：逐次调用显式指定 Secret（优先级高于环境变量）。
    secret_bytes = None
    if "--secret" in args:
        i = args.index("--secret")
        if i + 1 >= len(args):
            print("--secret 需要跟一个值（与客户端构建期一致的 Secret）", file=sys.stderr)
            return 2
        secret_value = args[i + 1]
        if not secret_value:
            print("--secret 不允许空字符串（如确需默认值，请省略该参数）", file=sys.stderr)
            return 2
        secret_bytes = secret_value.encode("utf-8")
        del args[i:i + 2]

    urls = []
    if args and args[0] == "--batch":
        urls = [line.strip() for line in sys.stdin if line.strip()]
    else:
        urls = args

    if not urls:
        print("未提供要加密的 URL", file=sys.stderr)
        return 2

    for url in urls:
        cipher = encrypt_url(priv, url, ttl_days=ttl_days, secret=secret_bytes)
        print(f"# {url}  (ttl_days={ttl_days})")
        print(cipher)
        print()

    return 0


if __name__ == "__main__":
    sys.exit(main())
