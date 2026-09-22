//! 动态解析真实 URL 的【纯 Rust 加密核心】。
//!
//! 本 crate 只包含与平台 / GUI 无关的加解密、签名验签、密钥对生成逻辑，
//! 供主 crate（`web2app_lib`，含 DNS 查询与 Tauri 集成）与命令行工具
//! `resolve-tool`（仅需加解密，不编译 Tauri）共用。
//!
//! # 加密与签名方案（Ed25519 签名 + AES-256-GCM 对称加密）
//!
//!   1. **防篡改（安全核心，硬保证）**：服务端用 Ed25519【私钥】对明文签名，客户端内置
//!      Ed25519【公钥】验签。
//!   2. **隐藏真实网址（软保证）**：明文用 AES-256-GCM 认证加密，对称密钥由固定 Secret
//!      经 HKDF 派生，防公网批量爬取，不防针对性反编译。
//!   3. **每次加密结果不同**：Nonce 与随机盐每次随机生成。
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
//! # 有效期（ttl）语义
//!
//! `ttl` 是【服务端生成时指定】的有效期时长（单位：秒）：
//!   - `ttl > 0`：客户端校验 `now - ts <= ttl`，超出即拒绝。
//!   - `ttl = 0`：永不失效，跳过时间戳校验。

use aes_gcm::aead::{Aead, KeyInit, Payload as AeadPayload};
use aes_gcm::{Aes256Gcm, Nonce as AesNonce};
use base64::Engine;
use ed25519_dalek::{Signature, Signer, Verifier};
use hkdf::Hkdf;
use sha2::Sha256;

// 重新导出签名相关类型，让上层（主 crate）无需直接依赖 ed25519-dalek 就能标注
// `decrypt_txt` 等公开 API 的参数/返回类型（如 `&VerifyingKey`、`SigningKey`）。
pub use ed25519_dalek::{SigningKey, VerifyingKey};

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

/// 对称加密的 Secret（与服务端 `tools/encrypt_url.py` 保持一致）。
///
/// 可在编译期通过环境变量 `RESOLVE_SECRET` 注入，未设置时回落到默认值。
///
/// 安全说明：对称加密要求「客户端必须能解密」，因此该 Secret 无论以何种形式提供，
/// 最终都会进入客户端二进制——**有决心反编译的攻击者仍可提取**。把它从硬编码常量
/// 改为编译期 env 注入，收益在于：每个打包产物可用不同的 Secret（而非所有包共享
/// 同一个固定串）；源码 / 二进制常量池里不再出现一个明显、可搜索的固定密钥串，
/// 挡住「随手 `strings` 一把」的低成本扒取。
///
/// 这只是**提高门槛**，不是根治（客户端持密钥的物理上限）。真正可靠的保护需换架构
/// （如运行时经 TLS 从服务端下发 URL，客户端不内置解密密钥）。
/// 注意：改 Secret 会令「旧 Secret 加密的密文」无法解密，服务端与客户端必须同步切换。
const SECRET: &[u8] = match option_env!("RESOLVE_SECRET") {
    Some(s) if !s.is_empty() => s.as_bytes(),
    _ => b"web2app-fixed-secret-2026",
};

/// 编译期是否由环境变量提供了非空的 `RESOLVE_SECRET`（`false` = 用的是上面那串公开默认值）。
///
/// ⚠️ 只暴露**状态**、绝不暴露 Secret 本体：唯一用途是启动日志里多一行
/// 「已由配置提供 / 未配置（用内置默认串）」。这条信息有实际价值 —— 未配置时所有产物共享
/// 同一个固定密钥串，而它的失败表现是「域名数组模式下必然解密失败」且**不报错**。
pub fn secret_is_configured() -> bool {
    matches!(option_env!("RESOLVE_SECRET"), Some(s) if !s.is_empty())
}

/// 从一段 PEM 文本或裸 base64 解析出 Ed25519 公钥（32 字节原始公钥）。
///
/// 支持两种输入：
///   - PKCS#8 SPKI PEM：`-----BEGIN PUBLIC KEY-----`（Ed25519 公钥 32 字节 + ASN.1 包装）
///   - 裸 base64：直接是 32 字节原始公钥的 base64
pub fn parse_public_key(pem: &str) -> Result<VerifyingKey, String> {
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
        // 健壮切片：长度已 >= 32，但用 get() 而非索引 + unwrap，杜绝任何越界 panic 隐患。
        let raw = der
            .get(der.len() - 32..)
            .ok_or_else(|| "公钥 DER 切片越界".to_string())?;
        let raw: &[u8; 32] = raw
            .try_into()
            .map_err(|_| "公钥 DER 长度异常".to_string())?;
        return VerifyingKey::from_bytes(raw).map_err(|e| format!("Ed25519 公钥解析失败：{e}"));
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
    let cipher_suite =
        Aes256Gcm::new_from_slice(&key).map_err(|e| format!("初始化 AES 失败：{e}"))?;
    let cipher = cipher_suite
        .encrypt(
            AesNonce::from_slice(&nonce_bytes),
            AeadPayload {
                msg: &[payload.as_slice(), sig.to_bytes().as_slice()].concat(),
                aad: b"",
            },
        )
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
pub fn generate_keypair() -> Result<(String, String), String> {
    let signing_key = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
    let verifying_key = signing_key.verifying_key();

    // 纯私钥 PKCS#8 DER（RFC 5208，Ed25519 的旧式导出格式，48 字节）：
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

/// 从 PEM 文本（Ed25519 私钥）解析出 `SigningKey`。
///
/// 兼容三种来源：
///   - 本 crate [`generate_keypair`] 生成的纯私钥（48 字节 DER）；
///   - Python `cryptography` 导出的 `Ed25519PrivateKey`（同样 48 字节纯私钥 DER）；
///   - ed25519-dalek `to_pkcs8_pem` 输出的 RFC 8410 格式（83 字节，带内嵌公钥）。
///
/// 对空白【完全健壮】：去掉 `-----BEGIN/END PRIVATE KEY-----` 头尾标记后，把剩余内容里的
/// 所有空白（换行、回车、空格、Tab）全部剔除，剩下即 base64 正文。这样即使私钥在多行传递时
/// 被折叠成一行（如 `-----BEGIN ... base64 ... -----END`），也能正确提取。
///
/// 复用点：`resolve-tool` 服务端加载私钥、各处测试里的 PEM → SigningKey 解析，统一走本函数，
/// 避免多处复制同一套「找 seed → 切 32 字节」的脆弱逻辑。
pub fn parse_signing_key_pem(pem: &str) -> Result<SigningKey, String> {
    // 去掉 PEM 头尾标记（可能被折叠在一行里，用字符串替换而非按行过滤）。
    let mut body = pem.replace("-----BEGIN PRIVATE KEY-----", "");
    body = body.replace("-----END PRIVATE KEY-----", "");
    // 兼容个别工具输出的 "BEGIN/END" 变体标记。
    body = body.replace("-----BEGIN", "").replace("-----END", "");
    body = body.replace("PRIVATE KEY", "");

    // 去掉所有空白字符，剩下的就是 base64 正文。
    let b64: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(&b64)
        .map_err(|e| format!("私钥 PEM base64 解码失败：{e}"))?;

    // Ed25519 私钥 seed（32 字节）位于 DER 末尾：
    //   - 纯私钥格式（48 字节）：`... 04 20 <seed>`，seed 是最后 32 字节。
    //   - RFC 8410 格式（83 字节）：`... 04 20 <seed> a1 23 03 21 00 <pub>`，seed 也是
    //     从内层 OCTET STRING 开始。统一做法：定位 `04 20`（内层 OCTET STRING 长度 32）
    //     后紧跟的 32 字节即 seed。
    if der.len() < 34 {
        return Err(format!(
            "私钥 DER 长度异常（解码后 {} 字节，应为 48 字节纯私钥）。请确认粘贴的是完整的 \
             Ed25519 私钥 PEM（含 BEGIN/END 行）",
            der.len()
        ));
    }
    let mut seed_pos = None;
    for i in 0..der.len().saturating_sub(1) {
        if der[i] == 0x04 && der[i + 1] == 0x20 {
            seed_pos = Some(i + 2);
            break;
        }
    }
    let seed_pos = seed_pos.ok_or_else(|| "私钥 DER 中未找到 Ed25519 seed".to_string())?;
    let seed = der
        .get(seed_pos..seed_pos + 32)
        .ok_or_else(|| "私钥 seed 长度不足 32 字节".to_string())?;
    let bytes: [u8; 32] = seed.try_into().map_err(|_| "seed 长度异常".to_string())?;
    Ok(SigningKey::from_bytes(&bytes))
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
pub fn decrypt_txt(public_key: &VerifyingKey, b64: &str) -> Result<String, String> {
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
    let cipher_suite =
        Aes256Gcm::new_from_slice(&key).map_err(|e| format!("初始化 AES 失败：{e}"))?;

    // 解密 + 认证。认证失败（密文被篡改）会在此抛错。
    let plain = cipher_suite
        .decrypt(
            nonce,
            AeadPayload {
                msg: cipher,
                aad: b"",
            },
        )
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
pub fn parse_payload(payload: &[u8]) -> Result<String, String> {
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
    if rest.len() != FIELD_LEN + FIELD_LEN + 1 + FIELD_LEN || rest[FIELD_LEN + FIELD_LEN] != 0x00 {
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
                return Err(format!(
                    "记录已过期（生成于 {ts}，有效期 {ttl}s，偏差 {diff}s）"
                ));
            }
        }
    }

    Ok(url)
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
        let signing_key = parse_signing_key_pem(&priv_pem)
            .expect("generate_keypair 产出的 PEM 应能被 parse_signing_key_pem 解析");
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
        payload.extend_from_slice(&946684800u64.to_be_bytes());
        payload.extend_from_slice(&3600u64.to_be_bytes()); // ttl = 1h，早已过期
        payload.push(0x00);
        payload.extend_from_slice(&[1u8; 8]);
        assert!(parse_payload(&payload).is_err());
    }

    #[test]
    fn parse_payload_ttl_zero_never_expires() {
        let mut payload = b"https://a.com".to_vec();
        payload.push(0x00);
        payload.extend_from_slice(&946684800u64.to_be_bytes()); // 2000-01-01
        payload.extend_from_slice(&0u64.to_be_bytes()); // ttl = 0 永不失效
        payload.push(0x00);
        payload.extend_from_slice(&[1u8; 8]);
        assert_eq!(parse_payload(&payload).unwrap(), "https://a.com");
    }

    // 端到端：Ed25519 签名 + AES-GCM 解密全链路。
    #[test]
    fn e2e_ed25519_aesgcm_decrypt() {
        let pub_b64 = "Sg9zFyUaBwWwGPm8TshK/LSNpZMbw0Ex4kMcwaK40jU=";
        let key = parse_public_key(pub_b64).unwrap();

        let b1 = "Ivc+gSeUshcKlXCyqr39RiH/euyPbLjnvTYrk+vze+pgYebyezjJPNEaHBIpZTMZC1beyZuAc+hCJxLLXgA9Ec5QjHPcJgiI6wd6xLGunj2choHitk11Ah+s8Eiavef+K3oeDKVHUtcx3GU2jiC/Nasg02xTdS0pwb3VJXGJ70w5QHkWiDRi/PBHHcc=";
        let b2 = "D3ajOlwF0LUFEPMSrk8gmM16Xi7ZDOOWSOGacW3LswC9dfyuNYTASNiZDmDTCzZzAN646ttKD6C6T+WKiL9jYhsY3ab3V0L8TUuHDBecOVUK0BbIZKaxlRqsfn2GEEk+KQFCB/xUvyDH9KRs0mzOUwGZUPeXDohglf9Oa+6KplWEpOqtK+WrvT/soi8=";
        let b3 = "w9GWFwaxTWjKfIKidWY4U9ShDavVgKqM7zM5rm9L7qJ8FfLmW1486ZdcPmEUNazCYjGXT/g42Mlv6nwi+tchpT9nzktd4RfZRqYjBAU3SwRXzFPof5DHvHd7C481+iA9A49tuua7UPeOQB23FZpIPNUp1tRtTTyiXaSVs0om40ssInqSU0lH77AG3x8=";
        let bili = "c3PSRYrZSj6wkY1vCVf+1W+hYdpmwjk6SO0J+XNP1Hq3NR/O2R5RAQtem5Dq/BQp+opCmj6Lu+MMf7wA/IKPZ0g00Tsr1ILIZXfGlUGP56F9whPhhYh+PGFG8McyvFp85JkDjmfcixaG88PLIhT1qRFTI3+G+ifZJxwKaL/uUOLFvY+RVG+V2suiWMjWbsw=";

        assert_ne!(b1, b2);
        assert_ne!(b2, b3);
        assert_ne!(b1, b3);

        assert_eq!(decrypt_txt(&key, b1).unwrap(), "https://www.baidu.com/");
        assert_eq!(decrypt_txt(&key, b2).unwrap(), "https://www.baidu.com/");
        assert_eq!(decrypt_txt(&key, b3).unwrap(), "https://www.baidu.com/");
        assert_eq!(
            decrypt_txt(&key, bili).unwrap(),
            "https://www.bilibili.com/"
        );
    }

    // 交叉验证：Python 端生成的密文，Rust 端解密结果必须一致。
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

    // 防篡改验证：篡改密文任一字节，AES-GCM 认证或 Ed25519 验签必须失败。
    #[test]
    fn e2e_tamper_detected() {
        let pub_b64 = "Sg9zFyUaBwWwGPm8TshK/LSNpZMbw0Ex4kMcwaK40jU=";
        let key = parse_public_key(pub_b64).unwrap();
        let b1 = "Ivc+gSeUshcKlXCyqr39RiH/euyPbLjnvTYrk+vze+pgYebyezjJPNEaHBIpZTMZC1beyZuAc+hCJxLLXgA9Ec5QjHPcJgiI6wd6xLGunj2choHitk11Ah+s8Eiavef+K3oeDKVHUtcx3GU2jiC/Nasg02xTdS0pwb3VJXGJ70w5QHkWiDRi/PBHHcc=";

        assert_eq!(decrypt_txt(&key, b1).unwrap(), "https://www.baidu.com/");

        let raw = base64::engine::general_purpose::STANDARD
            .decode(b1)
            .unwrap();
        let mut tampered = raw.clone();
        let idx = NONCE_LEN;
        tampered[idx] ^= 0x01;
        let tampered_b64 = base64::engine::general_purpose::STANDARD.encode(&tampered);
        assert!(
            decrypt_txt(&key, &tampered_b64).is_err(),
            "篡改密文应被拒绝"
        );
    }
}
