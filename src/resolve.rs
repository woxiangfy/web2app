//! 动态解析真实 URL：当 APP_URL 是一组域名（JSON 数组）时，依次对每个域名做 DNS TXT
//! 查询，拿到 TXT 记录值（base64 密文），解密 + 验签得到真实服务器地址。
//! 某个域名解析/解密/验签失败则自动尝试下一个；全部失败则报错"服务器连接失败"。
//!
//! 背景：app_url 可能变动，打包方只写死一组"跳板域名"，运行时读取这些域名的 TXT 解析
//! 记录值，再解密得到最终 URL，从而在不重新打包的前提下切换后端地址。
//!
//! # 加密与签名方案（Ed25519 签名 + AES-256-GCM 对称加密）
//!
//! 设计目标与实现方式：
//!   1. **防篡改（安全核心，硬保证）**：服务端用 Ed25519【私钥】对明文签名，客户端内置
//!      Ed25519【公钥】验签。攻击者即使拿到公钥 + 完全控制 DNS TXT，没有私钥就无法伪造
//!      出能通过验签的密文，最多只能破坏记录导致解密失败（DoS），无法劫持到恶意网址。
//!   2. **隐藏真实网址（软保证，防公网爬取）**：明文用 AES-256-GCM 认证加密包装，外部
//!      直接查 TXT 只能看到 base64 密文。注意：对称密钥由客户端内置的固定 Secret 经 HKDF
//!      派生，**攻击者反编译客户端可还原 Secret 并解密**，因此这层只防"公网批量爬虫/随手
//!      dig"，不承诺防针对性反编译。真正的安全边界是上面的 Ed25519 验签。
//!   3. **每次加密结果不同**：Nonce（12 字节）与 Payload 内随机盐（8 字节）每次随机生成，
//!      即使真实 URL 长期不变，每次密文也完全不同（防重放/防密文比对猜测）。
//!   4. **密文尽可能短**：Ed25519 签名 64 字节 + AEAD 密文（明文长度 + 16 字节 tag）+
//!      12 字节 Nonce。以 30~50 字节 URL 计，总密文约 130~150 字节，base64 后约 180~200
//!      字符，可放入单条 DNS TXT 记录（上限 255 字节）。
//!
//! # 数据格式（服务端 `tools/encrypt_url.py` / `resolve-tool` 与客户端必须一致）
//!
//! ```text
//! Payload   = URL || 0x00 || 时间戳(8B, 大端) || 有效期ttl(8B, 大端) || 0x00 || 随机盐(8B)
//! Signature = Ed25519.sign(Payload)                       // 64 字节
//! Key       = HKDF-SHA256(ikm=Secret, salt=Nonce, info="web2app-resolve")  // 32 字节
//! Cipher    = AES-256-GCM.encrypt(Key, Nonce, Payload || Signature)  // 附 16 字节 tag
//! TXT 值    = base64( Nonce(12B) || Cipher )
//! ```
//!
//! 其中 `Secret` 是客户端内置的固定值（编译期常量，服务端与客户端共享）。
//!
//! # 有效期（ttl）语义
//!
//! `ttl` 是【服务端生成时指定】的有效期时长（单位：秒），随密文一起下发：
//!   - `ttl > 0`：客户端校验 `now - ts <= ttl`，超出即拒绝（防重放/防过期记录）。
//!   - `ttl = 0`：表示【永不失效】，客户端跳过时间戳校验（适合后端地址长期不变的场景）。
//!
//! 这样是否设置有效期、有效期多长，完全由生成密文的一方决定，客户端无需重新编译。

use aes_gcm::aead::{Aead, KeyInit, Payload as AeadPayload};
use aes_gcm::{Aes256Gcm, Nonce as AesNonce};
use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use hkdf::Hkdf;
use sha2::Sha256;

/// 解析失败的统一错误文案（用户可见）。
pub const RESOLVE_FAILED: &str = "服务器连接失败";

/// HKDF 的 info 参数（上下文标签，服务端与客户端必须一致）。
const HKDF_INFO: &[u8] = b"web2app-resolve";
/// 对称密钥长度（AES-256 = 32 字节）。
const KEY_LEN: usize = 32;
/// AES-GCM 的 Nonce 长度（12 字节）。
const NONCE_LEN: usize = 12;
/// Ed25519 签名长度（64 字节）。
const SIG_LEN: usize = 64;
/// Payload 里时间戳 / 有效期 / 随机盐各占 8 字节。
const FIELD_LEN: usize = 8;

/// 客户端内置的固定 Secret（与服务端 `tools/encrypt_url.py` 保持一致）。
/// 注意：这**不是**保密凭据——它会被打进客户端二进制，攻击者反编译可得。
/// 它的作用仅仅是配合 Ed25519 验签，把真实网址从"DNS 明文可见"变成"需还原才能读"，
/// 即防公网批量爬取，不提供针对反编译的机密性。
const SECRET: &[u8] = b"web2app-fixed-secret-2026";

/// 从一段 PEM 文本或裸 base64 解析出 Ed25519 公钥（32 字节原始公钥）。
///
/// 支持两种输入：
///   - PKCS#8 SPKI PEM：`-----BEGIN PUBLIC KEY-----`（Ed25519 公钥 32 字节 + ASN.1 包装）
///   - 裸 base64：直接是 32 字节原始公钥的 base64
fn parse_public_key(pem: &str) -> Result<VerifyingKey, String> {
    let pem = pem.trim();
    if pem.is_empty() {
        return Err("公钥为空".to_string());
    }

    // PEM（SPKI）格式：Ed25519 公钥在 SPKI 里是固定 32 字节，位于 DER 末尾。
    if pem.starts_with("-----BEGIN") {
        let der = pem
            .lines()
            .filter(|l| !l.starts_with("-----"))
            .collect::<String>();
        let der = base64::engine::general_purpose::STANDARD
            .decode(der.trim())
            .map_err(|e| format!("公钥 PEM base64 解码失败：{e}"))?;
        // Ed25519 SPKI DER 结构：SEQUENCE(外层) 内含 AlgorithmIdentifier + BIT STRING。
        // 原始公钥 32 字节位于 DER 的最后一个 32 字节（BIT STRING 的 content 部分）。
        if der.len() < 32 {
            return Err("公钥 DER 长度异常".to_string());
        }
        let raw = &der[der.len() - 32..];
        return VerifyingKey::from_bytes(raw.try_into().unwrap())
            .map_err(|e| format!("Ed25519 公钥解析失败：{e}"));
    }

    // 裸 base64：32 字节原始公钥。
    let raw = base64::engine::general_purpose::STANDARD
        .decode(pem)
        .map_err(|e| format!("公钥 base64 解码失败：{e}"))?;
    if raw.len() != 32 {
        return Err(format!("Ed25519 公钥应为 32 字节，实际 {} 字节", raw.len()));
    }
    VerifyingKey::from_bytes(raw.as_slice().try_into().unwrap())
        .map_err(|e| format!("Ed25519 公钥解析失败：{e}"))
}

/// 用 HKDF-SHA256 从 Secret + Nonce 派生 AES-256 密钥。
fn derive_key(nonce: &[u8]) -> [u8; KEY_LEN] {
    let hk = Hkdf::<Sha256>::new(Some(nonce), SECRET);
    let mut key = [0u8; KEY_LEN];
    hk.expand(HKDF_INFO, &mut key)
        .expect("HKDF 展开 32 字节不应失败");
    key
}

// ---------------------------------------------------------------------------
// 以下为【服务端】方向的函数（供 `resolve-tool` 独立工具使用）：用 Ed25519 私钥
// 签名 + AES-GCM 加密，生成 TXT 密文。与客户端 `decrypt_txt` 严格对称。
// ---------------------------------------------------------------------------

/// 构造 Payload：`URL || 0x00 || ts(8B 大端) || ttl(8B 大端) || 0x00 || salt(8B)`。
fn build_payload(url: &str, ts: u64, ttl: u64, salt: &[u8; 8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(url.len() + 1 + 8 + 8 + 1 + 8);
    p.extend_from_slice(url.as_bytes());
    p.push(0x00);
    p.extend_from_slice(&ts.to_be_bytes());
    p.extend_from_slice(&ttl.to_be_bytes());
    p.push(0x00);
    p.extend_from_slice(salt);
    p
}

/// 用 Ed25519 私钥签名 + AES-GCM 加密一个 URL，返回 base64 密文（单行，写入 DNS TXT）。
///
/// - `url`：要隐藏的真实地址。
/// - `ttl`：有效期秒数（>0 按窗口校验；=0 永不失效）。
/// - `ts`：可选，指定生成时间戳（默认取当前系统时间；测试可传固定值）。
pub fn sign_and_encrypt(
    signing_key: &ed25519_dalek::SigningKey,
    url: &str,
    ttl: u64,
    ts: Option<u64>,
) -> Result<String, String> {
    use ed25519_dalek::Signer;

    let ts = ts.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    });

    // 随机 Nonce(12B) + 随机盐(8B)。
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut nonce_bytes);
    let mut salt = [0u8; 8];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut salt);

    let payload = build_payload(url, ts, ttl, &salt);

    // Ed25519 签名（64 字节）。
    let sig: Signature = signing_key.sign(&payload);

    // AES-GCM 加密 Payload || Signature。
    let key = derive_key(&nonce_bytes);
    let cipher_suite = Aes256Gcm::new_from_slice(&key).map_err(|e| format!("初始化 AES 失败：{e}"))?;
    let cipher = cipher_suite
        .encrypt(AesNonce::from_slice(&nonce_bytes), AeadPayload {
            msg: &[payload.as_slice(), sig.to_bytes().as_slice()].concat(),
            aad: b"",
        })
        .map_err(|_| "加密失败".to_string())?;

    // TXT = base64( Nonce(12B) || Cipher )。
    let mut raw = Vec::with_capacity(NONCE_LEN + cipher.len());
    raw.extend_from_slice(&nonce_bytes);
    raw.extend_from_slice(&cipher);
    Ok(base64::engine::general_purpose::STANDARD.encode(&raw))
}

/// 生成一对 Ed25519 密钥，返回（私钥 PEM(PKCS#8)，公钥裸 base64 32 字节）。
///
/// 私钥采用【纯私钥】PKCS#8 格式（48 字节 DER，仅含 32 字节 seed），与 Python
/// `cryptography` 的 `Ed25519PrivateKey` 导出格式完全一致，两端私钥可互相导入。
/// （ed25519-dalek 的 `to_pkcs8_pem` 会额外内嵌公钥成 83 字节 RFC 8410 格式，
///   Python `cryptography` 老版本读不了，故这里手动构造纯私钥格式。）
pub fn generate_keypair() -> Result<(String, String), String> {
    let signing_key = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
    let verifying_key = signing_key.verifying_key();

    // 纯私钥 PKCS#8 DER（RFC 5208，Ed25519 的旧式导出格式，48 字节）：
    //   30 2e                        SEQUENCE (46 字节)
    //      02 01 00                  INTEGER version = 0
    //      30 05                     SEQUENCE (AlgorithmIdentifier)
    //         06 03 2b 65 70         OID 1.3.101.112 (Ed25519)
    //      04 22                     OCTET STRING (34 字节)
    //         04 20 <32字节 seed>    Ed25519 私钥（内层 OCTET STRING）
    let seed = signing_key.to_bytes(); // 32 字节
    let mut der = Vec::with_capacity(48);
    der.extend_from_slice(&[0x30, 0x2e]); // SEQUENCE len=46
    der.extend_from_slice(&[0x02, 0x01, 0x00]); // INTEGER version
    der.extend_from_slice(&[0x30, 0x05]); // AlgorithmIdentifier
    der.extend_from_slice(&[0x06, 0x03, 0x2b, 0x65, 0x70]); // OID Ed25519
    der.extend_from_slice(&[0x04, 0x22]); // OCTET STRING len=34
    der.extend_from_slice(&[0x04, 0x20]); // 内层 OCTET STRING len=32
    der.extend_from_slice(&seed);

    let priv_pem = pem_encode("PRIVATE KEY", &der);

    // 公钥裸 base64（32 字节原始公钥），客户端直接填 PUBLIC_KEY。
    let raw_pub = verifying_key.to_bytes();
    let pub_b64 = base64::engine::general_purpose::STANDARD.encode(raw_pub);

    Ok((priv_pem, pub_b64))
}

/// 把 DER 字节编码成 PEM 文本（LF 换行，64 字符一行）。
fn pem_encode(label: &str, der: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).unwrap());
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

/// 解密 + 验签一条 TXT 密文，返回真实 URL。
///
/// 流程：base64 解码 → 取前 12 字节 Nonce → HKDF 派生 Key → AES-GCM 解密（认证）→
/// 分离 Payload 与 Signature（末 64 字节）→ Ed25519 公钥验签 → 校验时间戳 → 返回 URL。
fn decrypt_txt(public_key: &VerifyingKey, b64: &str) -> Result<String, String> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|e| format!("密文 base64 解码失败：{e}"))?;

    // 至少要有 Nonce(12) + 密文(≥ tag 16 字节)。
    if raw.len() < NONCE_LEN + 16 {
        return Err("密文长度异常".to_string());
    }
    let (nonce_bytes, cipher) = raw.split_at(NONCE_LEN);
    let nonce = AesNonce::from_slice(nonce_bytes);

    let key = derive_key(nonce_bytes);
    let cipher_suite = Aes256Gcm::new_from_slice(&key).map_err(|e| format!("初始化 AES 失败：{e}"))?;

    // 解密 + 认证。认证失败（密文被篡改）会在此抛错。
    let plain = cipher_suite
        .decrypt(nonce, AeadPayload { msg: cipher, aad: b"" })
        .map_err(|_| "密文认证失败（数据被篡改）".to_string())?;

    // 明文 = Payload || Signature(64 字节)。
    if plain.len() < SIG_LEN + 1 {
        return Err("明文长度异常".to_string());
    }
    let (payload, sig_bytes) = plain.split_at(plain.len() - SIG_LEN);
    let sig = Signature::from_slice(sig_bytes).map_err(|e| format!("签名解析失败：{e}"))?;

    // Ed25519 验签 —— 防篡改的核心保证。
    public_key
        .verify(payload, &sig)
        .map_err(|_| "签名验签失败".to_string())?;

    // 解析 Payload：URL || 0x00 || ts(8) || 0x00 || salt(8)。
    parse_payload(payload)
}

/// 解析 Payload，提取 URL 并校验有效期（防重放）。
///
/// Payload 结构：`URL || 0x00 || ts(8B 大端) || ttl(8B 大端) || 0x00 || salt(8B)`。
/// - `ts` 为 Unix 秒（生成时间）。
/// - `ttl` 为有效期秒数（服务端生成时指定）；`ttl = 0` 表示永不失效。
///
/// 校验：`now - ts <= ttl`（`ttl > 0` 时），超出视为过期/重放，拒绝。
fn parse_payload(payload: &[u8]) -> Result<String, String> {
    // 找到 URL 后的第一个分隔符 0x00。
    let sep = payload
        .iter()
        .position(|&b| b == 0x00)
        .ok_or_else(|| "Payload 缺少分隔符".to_string())?;
    let url = std::str::from_utf8(&payload[..sep])
        .map_err(|e| format!("URL 非合法 UTF-8：{e}"))?
        .trim()
        .to_string();
    if url.is_empty() {
        return Err("Payload 中 URL 为空".to_string());
    }

    // 剩余应为 ts(8) || ttl(8) || 0x00 || salt(8)，共 25 字节。
    let rest = &payload[sep + 1..];
    if rest.len() != FIELD_LEN + FIELD_LEN + 1 + FIELD_LEN
        || rest[FIELD_LEN + FIELD_LEN] != 0x00
    {
        return Err("Payload 尾部结构异常".to_string());
    }
    let ts = u64::from_be_bytes(rest[..FIELD_LEN].try_into().unwrap());
    let ttl = u64::from_be_bytes(rest[FIELD_LEN..FIELD_LEN + FIELD_LEN].try_into().unwrap());

    // 有效期校验：ttl > 0 时才校验时间窗口；ttl = 0 表示永不失效，跳过。
    if ttl > 0 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if now != 0 {
            let diff = now.abs_diff(ts);
            if diff > ttl {
                return Err(format!("记录已过期（生成于 {ts}，有效期 {ttl}s，偏差 {diff}s）"));
            }
        }
    }

    Ok(url)
}

/// 查询一个域名的 TXT 记录（拼接所有字符串段，返回首个非空值）。
///
/// 走系统默认 DNS。hickory-resolver 0.26 起移除了同步便捷方法 `Resolver::from_system_conf`，
/// 改为：`system_conf::read_system_conf()` 读系统配置 → `Resolver::builder_with_config` 构建
/// （泛型 `Resolver<P>`，provider 用 `TokioRuntimeProvider`）→ `txt_lookup` 是 async，用
/// `tauri::async_runtime::block_on` 驱动。
fn query_txt(domain: &str) -> Result<String, String> {
    use hickory_resolver::net::runtime::TokioRuntimeProvider;
    use hickory_resolver::system_conf::read_system_conf;
    use hickory_resolver::Resolver;

    // 读取系统 DNS 配置（/etc/resolv.conf 或 Windows 注册表）。
    let (config, _opts) =
        read_system_conf().map_err(|e| format!("读取系统 DNS 配置失败：{e}"))?;

    let resolver = Resolver::builder_with_config(config, TokioRuntimeProvider::default())
        .build()
        .map_err(|e| format!("初始化 DNS 解析器失败：{e}"))?;

    // txt_lookup 是 async，用 Tauri 的异步运行时阻塞驱动。
    let lookup = tauri::async_runtime::block_on(resolver.txt_lookup(domain))
        .map_err(|e| format!("查询 {domain} TXT 记录失败：{e}"))?;

    // 遍历 answers 里的每条记录，拼接 TXT 的所有字符串段；返回第一个非空结果。
    for record in lookup.answers() {
        if let hickory_resolver::proto::rr::RData::TXT(txt) = &record.data {
            let mut joined = String::new();
            for part in txt.txt_data.iter() {
                // txt_data 是 Box<[Box<[u8]>]>，按 UTF-8 尽力转字符串（域名 TXT 通常为 ASCII/UTF-8）。
                if let Ok(s) = std::str::from_utf8(part) {
                    joined.push_str(s);
                } else {
                    joined.push_str(&String::from_utf8_lossy(part));
                }
            }
            let joined = joined.trim().to_string();
            if !joined.is_empty() {
                return Ok(joined);
            }
        }
    }
    Err(format!("域名 {domain} 无 TXT 记录"))
}

/// 依次尝试一组域名：查询 TXT → 解密 + 验签 → 得到真实 URL。
/// 任一环节失败则尝试下一个域名；全部失败返回 `RESOLVE_FAILED`。
pub fn resolve_url_from_domains(
    domains: &[String],
    public_key_pem: &str,
) -> Result<String, String> {
    let public_key = parse_public_key(public_key_pem)?;

    for domain in domains {
        let domain = domain.trim();
        if domain.is_empty() {
            continue;
        }
        let txt = match query_txt(domain) {
            Ok(t) => t,
            Err(_e) => {
                // 单个域名失败：记录后尝试下一个。
                eprintln!("[resolve] 域名 {domain} 解析失败，尝试下一个");
                continue;
            }
        };
        match decrypt_txt(&public_key, &txt) {
            Ok(url) => {
                let url = url.trim().to_string();
                if !url.is_empty() {
                    return Ok(url);
                }
                eprintln!("[resolve] 域名 {domain} 解密结果为空，尝试下一个");
            }
            Err(_e) => {
                eprintln!("[resolve] 域名 {domain} 解密/验签失败，尝试下一个");
            }
        }
    }
    Err(RESOLVE_FAILED.to_string())
}

/// 判断 APP_URL 是否为"域名数组"模式：配置为 JSON 数组（如 `["a.com","b.com"]`）。
/// 解析失败（非法 JSON）返回 None，交由上层按"普通 URL"处理。
pub fn parse_domains(app_url: &str) -> Option<Vec<String>> {
    let trimmed = app_url.trim();
    if !trimmed.starts_with('[') {
        return None;
    }
    match serde_json::from_str::<Vec<String>>(trimmed) {
        Ok(list) if !list.is_empty() => Some(list),
        _ => None,
    }
}

/// 把域名规范化为可查询的主机名（去掉 scheme / 端口 / 路径 / userinfo）。
pub fn normalize_domain(domain: &str) -> String {
    let d = domain.trim();
    // 去掉可能的 userinfo（@ 之前）——必须在切 scheme/端口之前处理，
    // 否则 `user:pass@host` 会被误切成 `user`。
    let d = d.split('@').next_back().unwrap_or(d);
    // 去掉 scheme:// 前缀
    let d = d.split_once("://").map(|(_, rest)| rest).unwrap_or(d);
    // 去掉端口与路径
    let d = d.split('/').next().unwrap_or(d);
    let d = d.split(':').next().unwrap_or(d);
    // 去掉首尾点
    d.trim_matches('.').to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_bare_domain() {
        assert_eq!(normalize_domain("a.com"), "a.com");
    }

    #[test]
    fn normalize_domain_with_scheme() {
        assert_eq!(normalize_domain("https://a.com"), "a.com");
        assert_eq!(normalize_domain("https://a.com:8443/path"), "a.com");
    }

    // 服务端 sign_and_encrypt → 客户端 decrypt_txt 往返对称性（含 ttl 各种情况）。
    #[test]
    fn e2e_sign_encrypt_roundtrip() {
        let (priv_pem, pub_b64) = generate_keypair().unwrap();
        let signing_key = {
            // 从 PEM 解析回 SigningKey（复刻 bin 工具的 load_signing_key 逻辑）。
            let b64: String = priv_pem
                .lines()
                .filter(|l| !l.starts_with("-----"))
                .collect();
            let der = base64::engine::general_purpose::STANDARD
                .decode(b64.trim())
                .unwrap();
            let seed_pos = der.windows(2).position(|w| w == [0x04, 0x20]).unwrap() + 2;
            ed25519_dalek::SigningKey::from_bytes(der[seed_pos..seed_pos + 32].try_into().unwrap())
        };
        let key = parse_public_key(&pub_b64).unwrap();

        // ttl = 0（永不失效）
        let c = sign_and_encrypt(&signing_key, "https://a.com/", 0, None).unwrap();
        assert_eq!(decrypt_txt(&key, &c).unwrap(), "https://a.com/");

        // ttl = 3600（1 小时，当前时间戳下应有效）
        let c = sign_and_encrypt(&signing_key, "https://b.com/", 3600, None).unwrap();
        assert_eq!(decrypt_txt(&key, &c).unwrap(), "https://b.com/");

        // ttl = 3600 但时间戳是 2000 年 → 应因过期被拒绝。
        let c = sign_and_encrypt(&signing_key, "https://c.com/", 3600, Some(946684800)).unwrap();
        assert!(decrypt_txt(&key, &c).is_err(), "过期密文应被拒绝");
    }

    #[test]
    fn normalize_domain_with_userinfo() {
        assert_eq!(normalize_domain("https://u:p@a.com"), "a.com");
    }

    #[test]
    fn parse_domains_array() {
        let v = parse_domains(r#"["a.com","b.com"]"#).unwrap();
        assert_eq!(v, vec!["a.com", "b.com"]);
    }

    #[test]
    fn parse_domains_not_array() {
        assert!(parse_domains("https://example.com").is_none());
    }

    #[test]
    fn parse_payload_valid() {
        // URL "hi" || 0x00 || ts(8) || ttl(8) || 0x00 || salt(8)
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut payload = b"https://a.com".to_vec();
        payload.push(0x00);
        payload.extend_from_slice(&now.to_be_bytes());
        payload.extend_from_slice(&(48u64 * 3600).to_be_bytes()); // ttl = 48h
        payload.push(0x00);
        payload.extend_from_slice(&[1u8; 8]);
        let url = parse_payload(&payload).unwrap();
        assert_eq!(url, "https://a.com");
    }

    #[test]
    fn parse_payload_missing_separator() {
        assert!(parse_payload(b"no-separator").is_err());
    }

    #[test]
    fn parse_payload_expired() {
        let mut payload = b"https://a.com".to_vec();
        payload.push(0x00);
        // 过期时间戳：2000-01-01
        payload.extend_from_slice(&946684800u64.to_be_bytes());
        payload.extend_from_slice(&3600u64.to_be_bytes()); // ttl = 1h，早已过期
        payload.push(0x00);
        payload.extend_from_slice(&[1u8; 8]);
        assert!(parse_payload(&payload).is_err());
    }

    #[test]
    fn parse_payload_ttl_zero_never_expires() {
        // ttl = 0 表示永不失效：即使时间戳是 2000 年，也应通过。
        let mut payload = b"https://a.com".to_vec();
        payload.push(0x00);
        payload.extend_from_slice(&946684800u64.to_be_bytes()); // 2000-01-01
        payload.extend_from_slice(&0u64.to_be_bytes()); // ttl = 0 永不失效
        payload.push(0x00);
        payload.extend_from_slice(&[1u8; 8]);
        assert_eq!(parse_payload(&payload).unwrap(), "https://a.com");
    }

    // 端到端：Ed25519 签名 + AES-GCM 解密全链路。
    // 密文由 tools/encrypt_url.py 生成（服务端私钥签名 + AEAD 加密）。
    // 公钥：Sg9zFyUaBwWwGPm8TshK/LSNpZMbw0Ex4kMcwaK40jU=
    #[test]
    fn e2e_ed25519_aesgcm_decrypt() {
        let pub_b64 = "Sg9zFyUaBwWwGPm8TshK/LSNpZMbw0Ex4kMcwaK40jU=";
        let key = parse_public_key(pub_b64).unwrap();

        // 同一 URL 三次加密，密文互不相同（每次 Nonce + salt 随机）。ttl=0 永不失效。
        let b1 = "Ivc+gSeUshcKlXCyqr39RiH/euyPbLjnvTYrk+vze+pgYebyezjJPNEaHBIpZTMZC1beyZuAc+hCJxLLXgA9Ec5QjHPcJgiI6wd6xLGunj2choHitk11Ah+s8Eiavef+K3oeDKVHUtcx3GU2jiC/Nasg02xTdS0pwb3VJXGJ70w5QHkWiDRi/PBHHcc=";
        let b2 = "D3ajOlwF0LUFEPMSrk8gmM16Xi7ZDOOWSOGacW3LswC9dfyuNYTASNiZDmDTCzZzAN646ttKD6C6T+WKiL9jYhsY3ab3V0L8TUuHDBecOVUK0BbIZKaxlRqsfn2GEEk+KQFCB/xUvyDH9KRs0mzOUwGZUPeXDohglf9Oa+6KplWEpOqtK+WrvT/soi8=";
        let b3 = "w9GWFwaxTWjKfIKidWY4U9ShDavVgKqM7zM5rm9L7qJ8FfLmW1486ZdcPmEUNazCYjGXT/g42Mlv6nwi+tchpT9nzktd4RfZRqYjBAU3SwRXzFPof5DHvHd7C481+iA9A49tuua7UPeOQB23FZpIPNUp1tRtTTyiXaSVs0om40ssInqSU0lH77AG3x8=";
        let bili = "c3PSRYrZSj6wkY1vCVf+1W+hYdpmwjk6SO0J+XNP1Hq3NR/O2R5RAQtem5Dq/BQp+opCmj6Lu+MMf7wA/IKPZ0g00Tsr1ILIZXfGlUGP56F9whPhhYh+PGFG8McyvFp85JkDjmfcixaG88PLIhT1qRFTI3+G+ifZJxwKaL/uUOLFvY+RVG+V2suiWMjWbsw=";

        // 三次密文互不相同。
        assert_ne!(b1, b2);
        assert_ne!(b2, b3);
        assert_ne!(b1, b3);

        // 解密结果一致。
        assert_eq!(decrypt_txt(&key, b1).unwrap(), "https://www.baidu.com/");
        assert_eq!(decrypt_txt(&key, b2).unwrap(), "https://www.baidu.com/");
        assert_eq!(decrypt_txt(&key, b3).unwrap(), "https://www.baidu.com/");
        assert_eq!(decrypt_txt(&key, bili).unwrap(), "https://www.bilibili.com/");
    }

    // 交叉验证：Python 端（tools/encrypt_url.py）生成的密文，Rust 端解密结果必须一致。
    // 密文由本次会话用私钥 txttest_keys/private_key.pem 实时生成。
    #[test]
    fn e2e_crosscheck_python_generated() {
        let pub_b64 = "Sg9zFyUaBwWwGPm8TshK/LSNpZMbw0Ex4kMcwaK40jU=";
        let key = parse_public_key(pub_b64).unwrap();

        let baidu = [
            "Ivc+gSeUshcKlXCyqr39RiH/euyPbLjnvTYrk+vze+pgYebyezjJPNEaHBIpZTMZC1beyZuAc+hCJxLLXgA9Ec5QjHPcJgiI6wd6xLGunj2choHitk11Ah+s8Eiavef+K3oeDKVHUtcx3GU2jiC/Nasg02xTdS0pwb3VJXGJ70w5QHkWiDRi/PBHHcc=",
            "D3ajOlwF0LUFEPMSrk8gmM16Xi7ZDOOWSOGacW3LswC9dfyuNYTASNiZDmDTCzZzAN646ttKD6C6T+WKiL9jYhsY3ab3V0L8TUuHDBecOVUK0BbIZKaxlRqsfn2GEEk+KQFCB/xUvyDH9KRs0mzOUwGZUPeXDohglf9Oa+6KplWEpOqtK+WrvT/soi8=",
            "w9GWFwaxTWjKfIKidWY4U9ShDavVgKqM7zM5rm9L7qJ8FfLmW1486ZdcPmEUNazCYjGXT/g42Mlv6nwi+tchpT9nzktd4RfZRqYjBAU3SwRXzFPof5DHvHd7C481+iA9A49tuua7UPeOQB23FZpIPNUp1tRtTTyiXaSVs0om40ssInqSU0lH77AG3x8=",
        ];
        let bili = [
            "c3PSRYrZSj6wkY1vCVf+1W+hYdpmwjk6SO0J+XNP1Hq3NR/O2R5RAQtem5Dq/BQp+opCmj6Lu+MMf7wA/IKPZ0g00Tsr1ILIZXfGlUGP56F9whPhhYh+PGFG8McyvFp85JkDjmfcixaG88PLIhT1qRFTI3+G+ifZJxwKaL/uUOLFvY+RVG+V2suiWMjWbsw=",
        ];
        let example = [
            "hcqBZDDLey30KUF+L8PkjPIweNEUFQdu2ZXtoZg9tW35YP5UD4kB41crP4bv767vKGHYqB8vH5YgsnzP4lN3MurevuYwjAUc2XdcdzuEvT6EYebQH0Sd8zVi/ACIb8Q118H4FrQpYny3zmSZHicgKe8K7Twd8SToo+XhXg68y4dIoQpCwoYROjaK",
        ];

        for c in baidu {
            assert_eq!(decrypt_txt(&key, c).unwrap(), "https://www.baidu.com/");
        }
        for c in bili {
            assert_eq!(decrypt_txt(&key, c).unwrap(), "https://www.bilibili.com/");
        }
        for c in example {
            assert_eq!(decrypt_txt(&key, c).unwrap(), "https://example.com/");
        }
    }

    // 真实 DNS 端到端：查询真实域名 TXT → 解密 + 验签，得到真实 URL。
    // 依赖外部 DNS（需联网），用 #[ignore] 标注，默认不跑；手动 `cargo test -- --ignored` 触发。
    const TEST_PUB_B64: &str = "Sg9zFyUaBwWwGPm8TshK/LSNpZMbw0Ex4kMcwaK40jU=";

    #[test]
    #[ignore = "需要联网查询真实 DNS TXT 记录"]
    fn e2e_real_dns_resolve_baidu() {
        let domains = vec!["txttest1.lcjm.cc".to_string()];
        let url = resolve_url_from_domains(&domains, TEST_PUB_B64).unwrap();
        assert_eq!(url, "https://www.baidu.com/");
    }

    #[test]
    #[ignore = "需要联网查询真实 DNS TXT 记录"]
    fn e2e_real_dns_resolve_bilibili() {
        let domains = vec!["txttest2.mygit.top".to_string()];
        let url = resolve_url_from_domains(&domains, TEST_PUB_B64).unwrap();
        assert_eq!(url, "https://www.bilibili.com/");
    }

    #[test]
    #[ignore = "需要联网查询真实 DNS TXT 记录"]
    fn e2e_real_dns_failover_to_next() {
        // txttest3 是旧 RSA 密文（新方案解不开），txttest1 是新密文。
        // 应自动跳过 txttest3 失败，命中 txttest1 成功。
        let domains = vec![
            "txttest3.lcjm.cloudns.biz".to_string(), // 旧密文，解密失败
            "txttest1.lcjm.cc".to_string(),          // 新密文，成功
        ];
        let url = resolve_url_from_domains(&domains, TEST_PUB_B64).unwrap();
        assert_eq!(url, "https://www.baidu.com/");
    }

    // 防篡改验证：篡改密文任一字节，AES-GCM 认证或 Ed25519 验签必须失败。
    #[test]
    fn e2e_tamper_detected() {
        let pub_b64 = "Sg9zFyUaBwWwGPm8TshK/LSNpZMbw0Ex4kMcwaK40jU=";
        let key = parse_public_key(pub_b64).unwrap();
        let b1 = "Ivc+gSeUshcKlXCyqr39RiH/euyPbLjnvTYrk+vze+pgYebyezjJPNEaHBIpZTMZC1beyZuAc+hCJxLLXgA9Ec5QjHPcJgiI6wd6xLGunj2choHitk11Ah+s8Eiavef+K3oeDKVHUtcx3GU2jiC/Nasg02xTdS0pwb3VJXGJ70w5QHkWiDRi/PBHHcc=";

        // 正常可解。
        assert_eq!(decrypt_txt(&key, b1).unwrap(), "https://www.baidu.com/");

        // 篡改密文（改一个字符），必须失败。
        let raw = base64::engine::general_purpose::STANDARD
            .decode(b1)
            .unwrap();
        let mut tampered = raw.clone();
        // 改动密文部分的第一个字节（Nonce 之后的第一个字节）。
        let idx = NONCE_LEN;
        tampered[idx] ^= 0x01;
        let tampered_b64 = base64::engine::general_purpose::STANDARD.encode(&tampered);
        assert!(decrypt_txt(&key, &tampered_b64).is_err(), "篡改密文应被拒绝");
    }
}
