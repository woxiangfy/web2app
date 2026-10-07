# txttest 动态解析验证材料（Ed25519 签名 + AES-256-GCM 方案）

> 记录日期 2026-09-03 ｜ 现状核对 2026-09-25、复核 **2026-10-07（v1.49）**（单测 172 通过 / 6 ignored；
> 真机 DNS 用例因测试域名/记录失效暂红，见下文「域名 TXT 现状」）。
> 历史 RSA 方案已废弃，其密文仅保留作容错用例；v1/v2/v3 密文格式不再被解密端接受。

## 用途
验证 `app_url` 为域名数组时，通过 DNS TXT + Ed25519 验签 + AES-GCM 解密获取真实 URL 的逻辑。

## 密钥对（`webview-app/txttest_keys/`）
| 文件 | 说明 |
|------|------|
| `private_key.pem` | 私钥（**服务端持有**，用于签名 + 加密，切勿公开） |
| `public_key.b64` | 公钥裸 base64（32 字节原始公钥），客户端 `PUBLIC_KEY` 直接填它 |
| `public_key.pem` | 同一公钥的 SPKI PEM 形式，备用 |

> ⚠️ **2026-10-07 换钥说明**：`txttest_keys/` 在 2026-09-22 被整体重新生成过（私钥被
> `.gitignore` 忽略、旧私钥未留存），2026-09-03 文档里的公钥 `Sg9z…` 与现存私钥已不配对，
> 用它签的旧 TXT 密文全部作废。以下公钥为**现存私钥**（2026-09-22 对）对应值，重新部署
> DNS 时必须用这把。

### 公钥（客户端 `PUBLIC_KEY`，裸 base64）
```
HkCpkH1tMpNIxWoWAkvlX8vm93BOc5PtIKrQxl8RUDo=
```

### 公钥（SPKI PEM，备用）
```
-----BEGIN PUBLIC KEY-----
MCowBQYDK2VwAyEAHkCpkH1tMpNIxWoWAkvlX8vm93BOc5PtIKrQxl8RUDo=
-----END PUBLIC KEY-----
```

## 数据格式（服务端 `resolve-tool` 与客户端 `resolve-core` 严格对称）
```
Payload   = URL || 0x00 || 签发日 d(2B 大端) || 有效期 ttl_d(2B 大端)
Signature = Ed25519.sign(Payload)                                    # 64 字节
Key       = HKDF-SHA256(ikm=Secret, salt=Nonce, info="web2app-resolve")
Cipher    = AES-256-GCM.encrypt(Key, Nonce, Payload || Signature)    # 附 16 字节 tag
TXT 值    = base64( Nonce(12B) || Cipher )
```
> **解密端只认 v4 一种格式**（分隔符 `0x00` 后尾部必须恰好 4 字节 = `d(2) || ttl_d(2)`）：
> v4 = 按天紧凑格式（签发日 = 距基准日 2026-01-01 的 UTC 天数；ttl_d 单位也是天）；
> 历史 v3（秒级 16B 尾）/ v2（尾部 25B 带盐）/ v1（尾部 17B 无 ttl、固定 ±48h 窗口）
> 记录在结构层一律返回「Payload 尾部结构异常」并拒绝，线上旧 TXT 需重新签发。
- `Secret` 客户端侧仍是编译期内置值，默认 `web2app-fixed-secret-2026`（防公网爬取，不防反编译）：
  客户端构建前设 `RESOLVE_SECRET=<自定义值>` 即可注入（`option_env!("RESOLVE_SECRET")`，见
  `webview-app/crates/resolve-core/src/lib.rs`）。
  **服务端加密侧自 v1.46 起新增逐次调用入口** `encrypt --secret <值>`（Python 同名参数），
  取值优先级：命令行参数 > 环境变量 `RESOLVE_SECRET` > 内置默认；同一份工具二进制无需为不同
  Secret 分别编译。两端值必须一致，否则 HKDF 派生密钥不同、客户端必然解密失败。
- `ttl_days` 由**生成方**决定、随密文下发：时间以 UTC 天为单位，`ttl_d > 0` 时客户端校验
  `今日 - d <= ttl_d`，超出即拒绝（最小粒度 1 天）；`ttl_d = 0` 表示永不失效。
- 权威实现见 `webview-app/crates/resolve-core/src/lib.rs`（`build_payload` / `sign_and_encrypt` / `decrypt_txt`）。

## 域名 TXT 现状

> **2026-10-07（v1.49）复核（实查系统 DNS + 跑 ignored 用例）**：下表两个正向域名的测试记录都已失效——
> `txttest1` 域名 **NXDOMAIN**（已不存在）；`txttest2` 现存记录是 **v1 首发格式**且由旧私钥
> （`Sg9z…` 对）签发，解析端在结构层即拒绝 v1/v2/v3，验签公钥也已更换，不存在「能解但过期」
> 的中间态。恢复 3 条 ignored 真机用例需重新部署 DNS（域名续用/更换后，用**现存** txttest 私钥
> 以 v4 重新签发 TXT，建议 `--ttl-days 0`）。下表密文按 2026-09-03 时点原样保留（仅作历史溯源）。

| 域名 | TXT 密文（2026-09-03 时点） | 设计解密结果 | 2026-10-07（v1.49）复核 |
|------|----------|----------|------|
| `txttest1.lcjm.cc` | `k2zRmLjpkYY/R8vlT8f/D6Pz647j7r2icyZ8uplqIYhSRloPCk9dtwxPOCx1E51gL/hwBpbmcB4jil3GDc6XCUXMNWgP3E/yFT3QBjk30Lp5ErkgUMObvLZY4wEs69k7VHk0lib8MSpY5qjBkRLW45HhSUF937ul6MgAV2BwPXu84yQw`（v1 格式，176 字符） | `https://www.baidu.com/` | ❌ 域名 NXDOMAIN；即使记录仍在，v1 结构与旧公钥均被拒 |
| `txttest2.mygit.top` | `FSCztkRMy2a197pFybrzSTzTZVZyOYMyHyRm19TuIIz96Gb8x+xWeshrsL1yJo8I7sDm0iAOGJN69mJzepq8cKIPOSNvDD50HOo6zAsTcRUaUld2OIIJPNYWsBYBeCeDu0X6FBbHlwOIGeqdktAAl+fPYAgB/CC9bZkZC8ZM/r+iVLijsuPz`（v1 格式，180 字符） | `https://www.bilibili.com/` | ❌ v1 结构直接拒绝（不再是过期问题）+ 旧公钥私钥已遗失 |
| `txttest3.lcjm.cloudns.biz` | （旧 RSA 密文） | 解密失败 | ⚠️ 故意保留，用于验证容错 |

## 验证结论
- **单元测试全过**（2026-10-07 v1.49 实测：`cargo test --all` 172 通过 / 6 ignored，其中
  resolve-core 19、resolve-tool 22）：覆盖 `normalize_domain`、`parse_domains`、v4 `parse_payload`
  （有效/边界/过期/`ttl_days=0`/缺分隔符）、`parse_payload_rejects_legacy_and_bad_tails`
  （v3/v2/v1/20B/15B/3B/5B 尾部全部拒绝）、`rejects_out_of_range_days`（>65535 报错）、
  `e2e_fixed_vectors_v4_decrypt`（现存 txttest 私钥 `--ttl-days 0` 生成的 5 条 v4
  固定向量：baidu×3 / bilibili / example）、`e2e_tamper_detected`、
  `e2e_custom_secret_roundtrip`（`--secret` 对应的 with_secret API 双向隔离）、
  `e2e_decrypt_record_parts`（结构化解密返回 URL + 签发日 + TTL 天数）等。
- **真实 DNS 端到端用例**（`cargo test -- --ignored`，需联网）：2026-09-17 曾 3 条全过；
  2026-10-07 因上面的域名/记录/密钥三重失效 3 条均失败，**与代码改动无关**，重新部署 DNS 后可恢复。
- **Python ↔ Rust 交叉验证**：`webview-app/tools/encrypt_url.py` 与 `resolve-tool` 格式严格对称
  （含同名 `--secret` 参数，本机无 Python 时以格式常量与单测对齐保证）。
- **每次加密结果不同**：同一 URL 多次加密密文互不相同（随机 Nonce），解密结果一致。
- **密文长度**：v4 唯一格式 = 固定开销 97 字节 + URL 长度，base64 后通常 160~180 字符
  （实测 22 字符 URL 为 **160**；上一版 v3 为 176，旧 RSA 方案为 344 字符）。

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

# 解密验证一条 TXT 密文（公钥直接内联裸 base64，不用文件；密文省略走 stdin）
cargo run -p resolve-tool -- decrypt "<公钥裸 base64>" "<TXT 密文>" --secret "<与客户端一致的 Secret>"
```

`--ttl-days 天`：默认 **0（永不失效）**；给正整数则生成后 N 天内有效（最小粒度 1 天，上限 65535）。
`--count N`：每个 URL 生成 N 份不同密文（随机 Nonce），
可把同一地址分发到多个域名 / 多条 TXT 记录。`--secret <值>`：显式指定本次加密使用的 Secret
（必须与客户端构建期注入值一致；省略则取编译期/环境变量 `RESOLVE_SECRET`，再缺省为内置默认）：
```bash
# 客户端构建期注入了自定义 Secret 时，加密侧逐次显式传入（无需为重编工具）
cargo run -p resolve-tool -- encrypt txttest_keys/private_key.pem "https://www.baidu.com/" --ttl-days 0 --secret "与客户端一致的 Secret"
```

等价备选：`webview-app/tools/encrypt_url.py`（需 `pip install cryptography`，私钥/密文格式与
`--secret` 参数语义完全互通，用于跨语言交叉验证；本机未装 Python 时以 resolve-tool 为准）：
```bash
python tools/encrypt_url.py genkey txttest_keys
python tools/encrypt_url.py txttest_keys/private_key.pem "https://www.baidu.com/" --secret "与客户端一致的 Secret"
```
