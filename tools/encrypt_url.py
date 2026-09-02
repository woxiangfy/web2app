#!/usr/bin/env python3
"""服务端 URL 加密脚本（Ed25519 签名 + AES-256-GCM 对称加密）。

设计目标：
  1. 防篡改（硬保证）：Ed25519 私钥签名，客户端公钥验签。攻击者无私钥无法伪造。
  2. 隐藏真实网址（软保证）：AES-256-GCM 认证加密，防公网批量爬取（不防反编译客户端）。
  3. 每次加密结果不同：Nonce(12B) 与随机盐(8B) 每次随机，密文每次都变。
  4. 密文尽可能短：约 130~150 字节，base64 后约 180~200 字符，单条 DNS TXT 可放下。

数据格式（与客户端 src/resolve.rs 保持一致）：
  Payload   = URL || 0x00 || 时间戳(8B 大端) || 0x00 || 随机盐(8B)
  Signature = Ed25519.sign(Payload)                            # 64 字节
  Key       = HKDF-SHA256(ikm=Secret, salt=Nonce, info="web2app-resolve")
  Cipher    = AES-256-GCM.encrypt(Key, Nonce, Payload || Signature)   # 附 16 字节 tag
  TXT 值    = base64( Nonce(12B) || Cipher )

用法：
  python encrypt_url.py genkey <output_dir>            # 生成 Ed25519 密钥对
  python encrypt_url.py <private_key.pem> <url>        # 加密单个 URL
  python encrypt_url.py <private_key.pem> --batch < urls.txt   # 批量加密

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

# 与客户端 src/resolve.rs 中的常量保持一致。
SECRET = b"web2app-fixed-secret-2026"
HKDF_INFO = b"web2app-resolve"
NONCE_LEN = 12
KEY_LEN = 32


def derive_key(nonce: bytes) -> bytes:
    """HKDF-SHA256(ikm=SECRET, salt=Nonce, info=HKDF_INFO) -> 32 字节密钥。"""
    hk = HKDF(algorithm=hashes.SHA256(), length=KEY_LEN, salt=nonce, info=HKDF_INFO)
    return hk.derive(SECRET)


def encrypt_url(priv_key: Ed25519PrivateKey, url: str, ts: int | None = None) -> str:
    """加密 + 签名一个 URL，返回 base64 密文（单行）。"""
    url_b = url.encode("utf-8")
    ts = ts if ts is not None else int(time.time())
    salt = os.urandom(8)

    # Payload = URL || 0x00 || ts(8B 大端) || 0x00 || salt(8B)
    payload = url_b + b"\x00" + ts.to_bytes(8, "big") + b"\x00" + salt

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

    urls = []
    if sys.argv[2] == "--batch":
        urls = [line.strip() for line in sys.stdin if line.strip()]
    else:
        urls = sys.argv[2:]

    for url in urls:
        cipher = encrypt_url(priv, url)
        print(f"# {url}")
        print(cipher)
        print()

    return 0


if __name__ == "__main__":
    sys.exit(main())
