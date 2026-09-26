# txttest 动态解析验证材料（Ed25519 签名 + AES-256-GCM 方案）

> 记录日期 2026-09-03 ｜ 现状核对 **2026-09-25**（`cargo test -p resolve-core -p resolve-tool` 实测 13 + 7 = 20 通过 / 0 失败）。
> 历史 RSA 方案已废弃，其密文仅保留作容错用例。

## 用途
验证 `app_url` 为域名数组时，通过 DNS TXT + Ed25519 验签 + AES-GCM 解密获取真实 URL 的逻辑。

## 密钥对（`webview-app/txttest_keys/`）
| 文件 | 说明 |
|------|------|
| `private_key.pem` | 私钥（**服务端持有**，用于签名 + 加密，切勿公开） |
| `public_key.b64` | 公钥裸 base64（32 字节原始公钥），客户端 `PUBLIC_KEY` 直接填它 |
| `public_key.pem` | 同一公钥的 SPKI PEM 形式，备用 |

### 公钥（客户端 `PUBLIC_KEY`，裸 base64）
```
Sg9zFyUaBwWwGPm8TshK/LSNpZMbw0Ex4kMcwaK40jU=
```

### 公钥（SPKI PEM，备用）
```
-----BEGIN PUBLIC KEY-----
MCowBQYDK2VwAyEASg9zFyUaBwWwGPm8TshK/LSNpZMbw0Ex4kMcwaK40jU=
-----END PUBLIC KEY-----
```

## 数据格式（服务端 `resolve-tool` 与客户端 `resolve-core` 严格对称）
```
Payload   = URL || 0x00 || 时间戳 ts(8B 大端) || 有效期 ttl(8B 大端) || 0x00 || 随机盐(8B)
Signature = Ed25519.sign(Payload)                                    # 64 字节
Key       = HKDF-SHA256(ikm=Secret, salt=Nonce, info="web2app-resolve")
Cipher    = AES-256-GCM.encrypt(Key, Nonce, Payload || Signature)    # 附 16 字节 tag
TXT 值    = base64( Nonce(12B) || Cipher )
```
- `Secret` = 客户端内置固定值 `web2app-fixed-secret-2026`（防公网爬取，不防反编译）。
- `ttl` 由**生成方**决定、随密文下发：`ttl > 0` 时客户端校验 `now - ts <= ttl`，超出即拒绝（防重放/防过期记录）；`ttl = 0` 表示永不失效。
- 权威实现见 `webview-app/crates/resolve-core/src/lib.rs`（`build_payload` / `sign_and_encrypt` / `decrypt_txt`）。

## 域名 TXT 现状（2026-09-03 更新后）
| 域名 | TXT 密文 | 解密结果 | 状态 |
|------|----------|----------|------|
| `txttest1.lcjm.cc` | `k2zRmLjpkYY/R8vlT8f/D6Pz647j7r2icyZ8uplqIYhSRloPCk9dtwxPOCx1E51gL/hwBpbmcB4jil3GDc6XCUXMNWgP3E/yFT3QBjk30Lp5ErkgUMObvLZY4wEs69k7VHk0lib8MSpY5qjBkRLW45HhSUF937ul6MgAV2BwPXu84yQw` | `https://www.baidu.com/` | ✅ 新方案 |
| `txttest2.mygit.top` | `FSCztkRMy2a197pFybrzSTzTZVZyOYMyHyRm19TuIIz96Gb8x+xWeshrsL1yJo8I7sDm0iAOGJN69mJzepq8cKIPOSNvDD50HOo6zAsTcRUaUld2OIIJPNYWsBYBeCeDu0X6FBbHlwOIGeqdktAAl+fPYAgB/CC9bZkZC8ZM/r+iVLijsuPz` | `https://www.bilibili.com/` | ✅ 新方案 |
| `txttest3.lcjm.cloudns.biz` | （旧 RSA 密文） | 解密失败 | ⚠️ 故意保留，用于验证容错 |

## 验证结论
- **20 个单元测试全过**（`cargo test -p resolve-core -p resolve-tool`，2026-09-17 实测：resolve-core 13 + resolve-tool 7，0 failed）：
  覆盖 `normalize_domain`、`parse_domains`、`parse_payload`（有效/过期/缺分隔符）、`e2e_ed25519_aesgcm_decrypt`、
  `e2e_tamper_detected`、`e2e_crosscheck_python_generated` 等。
- **3 个真实 DNS 端到端测试全过**（`cargo test -- --ignored`）：
  1. `txttest1.lcjm.cc` → 解出 `https://www.baidu.com/` ✓
  2. `txttest2.mygit.top` → 解出 `https://www.bilibili.com/` ✓
  3. 容错：`txttest3`（旧密文）排在前面 → 自动跳过 → 命中 `txttest1` ✓
- **Python ↔ Rust 交叉验证**：`webview-app/tools/encrypt_url.py` 生成的密文，Rust 端解密结果一致。
- **每次加密结果不同**：同一 URL 多次加密密文互不相同（随机 Nonce/盐），解密结果一致。
- **密文长度**：约 176~184 字符（随 URL 长度变化；旧 RSA 方案为 344 字符）。

## 重新生成密文（服务端）
推荐用纯 Rust 的 `resolve-tool`（无需 Python 依赖；独立 crate，不编译 Tauri）：
```bash
cd webview-app

# 生成密钥对（已生成，勿重复覆盖）
cargo run -p resolve-tool -- genkey --out txttest_keys

# 加密单个 URL（stdout 输出单行 base64，写入 DNS TXT）
cargo run -p resolve-tool -- encrypt txttest_keys/private_key.pem "https://www.baidu.com/"

# 批量加密（从 stdin 逐行读 URL）
printf "https://a.com/\nhttps://b.com/\n" | cargo run -p resolve-tool -- encrypt --batch txttest_keys/private_key.pem

# 一键：生成密钥对 + 密文，同一 URL 出 3 份不同密文
cargo run -p resolve-tool -- all "https://www.baidu.com/" --out txttest_keys --count 3
```

`--ttl 秒`：默认 172800（48h）；`0` = 永不失效。`--count N`：每个 URL 生成 N 份不同密文（随机 Nonce/盐），
可把同一地址分发到多个域名 / 多条 TXT 记录。

等价备选：`webview-app/tools/encrypt_url.py`（需 `pip install cryptography`，私钥格式与密文格式完全互通，
用于跨语言交叉验证）：
```bash
python tools/encrypt_url.py genkey txttest_keys
python tools/encrypt_url.py txttest_keys/private_key.pem "https://www.baidu.com/"
```
