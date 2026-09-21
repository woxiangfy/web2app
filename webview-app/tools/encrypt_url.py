#!/usr/bin/env python3
"""服务端 URL 加密脚本（Ed25519 签名 + AES-256-GCM 对称加密）。

设计目标：
  1. 防篡改（硬保证）：Ed25519 私钥签名，客户端公钥验签。攻击者无私钥无法伪造。
  2. 隐藏真实网址（软保证）：AES-256-GCM 认证加密，防公网批量爬取（不防反编译客户端）。
  3. 每次加密结果不同：Nonce(12B) 与随机盐(8B) 每次随机，密文每次都变。
  4. 密文尽可能短：约 130~150 字节，base64 后约 180~200 字符，单条 DNS TXT 可放下。

数据格式（与客户端 src/resolve.rs 保持一致）：
  Payload   = URL || 0x00 || 时间戳(8B 大端) || 有效期ttl(8B 大端) || 0x00 || 随机盐(8B)
  Signature = Ed25519.sign(Payload)                            # 64 字节
  Key       = HKDF-SHA256(ikm=Secret, salt=Nonce, info="web2app-resolve")
  Cipher    = AES-256-GCM.encrypt(Key, Nonce, Payload || Signature)   # 附 16 字节 tag
  TXT 值    = base64( Nonce(12B) || Cipher )

有效期（ttl）语义：由【生成时】决定，随密文下发。
  - ttl > 0：客户端校验 now - ts <= ttl，超出即拒绝（防重放）。
  - ttl = 0：永不失效（客户端跳过时间校验）。

用法：
  python encrypt_url.py genkey <output_dir>                    # 生成 Ed25519 密钥对
  python encrypt_url.py <private_key.pem> <url> [--ttl 秒]     # 加密单个 URL（ttl 默认 172800=48h）
  python encrypt_url.py <private_key.pem> --batch [--ttl 秒] < urls.txt   # 批量加密

输出：base64 密文（单行，可直接写入 DNS TXT 记录）。
"""

import sys
import os
import time
import base64

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.hkdf import HKDF
from cryptography.hazmat.primitives import hashes

# 与客户端 crates/resolve-core/src/lib.rs 中的常量保持一致。
# Secret 支持通过环境变量 RESOLVE_SECRET 覆盖，未设置时用默认值（向后兼容旧密文）。
# 注意：改 Secret 会让旧 Secret 加密的密文无法被解密，服务端与客户端必须同步切换。
SECRET = os.environ.get("RESOLVE_SECRET", "web2app-fixed-secret-2026").encode("utf-8")
HKDF_INFO = b"web2app-resolve"
NONCE_LEN = 12
KEY_LEN = 32
DEFAULT_TTL = 48 * 3600  # 默认有效期 48 小时


def derive_key(nonce: bytes) -> bytes:
    """HKDF-SHA256(ikm=SECRET, salt=Nonce, info=HKDF_INFO) -> 32 字节密钥。"""
    hk = HKDF(algorithm=hashes.SHA256(), length=KEY_LEN, salt=nonce, info=HKDF_INFO)
    return hk.derive(SECRET)


def encrypt_url(priv_key: Ed25519PrivateKey, url: str, ts: int | None = None,
                ttl: int = DEFAULT_TTL) -> str:
    """加密 + 签名一个 URL，返回 base64 密文（单行）。

    ttl 为有效期秒数（服务端生成时指定）：>0 按窗口校验，=0 永不失效。
    """
    url_b = url.encode("utf-8")
    ts = ts if ts is not None else int(time.time())
    salt = os.urandom(8)

    # Payload = URL || 0x00 || ts(8B 大端) || ttl(8B 大端) || 0x00 || salt(8B)
    payload = (url_b + b"\x00" + ts.to_bytes(8, "big")
               + ttl.to_bytes(8, "big") + b"\x00" + salt)

    # Ed25519 签名（64 字节）
    sig = priv_key.sign(payload)

    # Nonce + 派生密钥 + AES-GCM 加密
    nonce = os.urandom(NONCE_LEN)
    key = derive_key(nonce)
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

    key_path = sys.argv[1]
    priv = load_private_key(key_path)

    # 解析 --ttl 参数（单位：秒）。支持放在任意位置。
    ttl = DEFAULT_TTL
    args = sys.argv[2:]
    if "--ttl" in args:
        i = args.index("--ttl")
        if i + 1 >= len(args):
            print("--ttl 需要跟一个秒数（整数）", file=sys.stderr)
            return 2
        try:
            ttl = int(args[i + 1])
        except ValueError:
            print(f"--ttl 必须是整数秒数，收到：{args[i + 1]}", file=sys.stderr)
            return 2
        if ttl < 0:
            print("--ttl 不能为负数", file=sys.stderr)
            return 2
        # 去掉 --ttl 和它的值
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
        cipher = encrypt_url(priv, url, ttl=ttl)
        print(f"# {url}  (ttl={ttl}s)")
        print(cipher)
        print()

    return 0


if __name__ == "__main__":
    sys.exit(main())
