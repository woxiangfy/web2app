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
//!   3. **每次加密结果不同**：Nonce(12B) 每次随机生成（它同时充当 HKDF 的 salt，
//!      故派生出的 AES 密钥也每次不同；无需额外随机盐）。
//!
//! # 数据格式（服务端 `tools/encrypt_url.py` / `resolve-tool` 与客户端必须一致）
//!
//! ```text
//! Payload   = URL || 0x00 || 签发日d(2B 大端) || 有效期ttl_d(2B 大端)
//! Signature = Ed25519.sign(Payload)                       // 64 字节
//! Key       = HKDF-SHA256(ikm=Secret, salt=Nonce, info="web2app-resolve")  // 32 字节
//! Cipher    = AES-256-GCM.encrypt(Key, Nonce, Payload || Signature)  // 附 16 字节 tag
//! TXT 值    = base64( Nonce(12B) || Cipher )
//! ```
//!
//! 解密端**只认当前 v4 格式**（见 [`parse_payload_parts`]，尾部固定 4B）；v3 秒级
//! 时间戳（16B 尾）与更早的 v1/v2 格式一律拒绝——线上旧 TXT 需用当前工具重新签发。
//!
//! # 日期与有效期（ttl_days）语义
//!
//! 时间以** UTC 天**为单位，`d` = 签发日距固定基准日 **2026-01-01** 的天数（u16，
//! 可覆盖至约 2205 年）；`ttl_d` 是有效期天数（u16）：
//!   - `ttl_d > 0`：客户端校验 `今日 - d <= ttl_d`，超出即拒绝（最小粒度 1 天）。
//!   - `ttl_d = 0`：永不失效，跳过日期校验。

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
/// Payload 里签发日 / 有效期各占 2 字节（u16，尾部固定 4B，也是唯一受支持的格式）。
const FIELD_LEN: usize = 2;
/// 日期基准日 2026-01-01 对应的 UNIX 天数（距 1970-01-01；56 年含 14 个闰年 ⇒ 20454）。
/// 签发日 `d` 与它相加即 UNIX 天数，可换算成公历日期（`resolve-tool` 展示用）。
pub const BASE_UNIX_DAY: i64 = 20454;
/// 有效期 / 日期字段的 u16 上界（超过即拒绝；65535 天 ≈ 179 年）。
const MAX_DAYS: u64 = u16::MAX as u64;

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

/// 用 HKDF-SHA256 从【显式传入】的 Secret + Nonce 派生 AES-256 密钥。
///
/// 服务端工具 `resolve-tool encrypt --secret <值>` / Python `--secret` 走这一支：
/// 同一份编译产物可以按调用指定任意 Secret，无需为不同 Secret 分别编译。客户端解密
/// 仍只用编译期注入的 [`SECRET`]（见 [`decrypt_txt`]）。
pub fn derive_key_with_secret(nonce: &[u8], secret: &[u8]) -> [u8; KEY_LEN] {
    let hk = Hkdf::<Sha256>::new(Some(nonce), secret);
    let mut key = [0u8; KEY_LEN];
    hk.expand(HKDF_INFO, &mut key)
        .expect("HKDF 展开 32 字节不应失败");
    key
}

/// 构造 Payload：`URL || 0x00 || d(2B 大端) || ttl_d(2B 大端)`（v4 按天紧凑格式）。
fn build_payload(url: &str, day: u16, ttl_days: u16) -> Vec<u8> {
    let mut p = Vec::with_capacity(url.len() + 1 + FIELD_LEN + FIELD_LEN);
    p.extend_from_slice(url.as_bytes());
    p.push(0x00);
    p.extend_from_slice(&day.to_be_bytes());
    p.extend_from_slice(&ttl_days.to_be_bytes());
    p
}

/// 取当前 UTC 日期对应的 UNIX 天数（距 1970-01-01）。
///
/// 系统时钟早于 UNIX 纪元时返回明确错误。供 [`current_day`] 与 `resolve-tool`
/// 展示「距今/剩余天数」共用，避免各处重复写时钟换算。
pub fn current_unix_day() -> Result<i64, String> {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("系统时钟早于 UNIX 纪元：{e}"))?
        .as_secs();
    Ok((secs / 86_400) as i64)
}

/// 取当前 UTC 日期相对基准日 2026-01-01 的天数（u16）。
///
/// 时钟早于基准日（理论上不会发生）或超出 u16 范围（约 2205 年）时返回明确错误，
/// 由调用方决定如何失败，而不是静默截断成错误日期。
fn current_day() -> Result<u16, String> {
    let day = current_unix_day()? - BASE_UNIX_DAY;
    u16::try_from(day).map_err(|_| format!("当前日期超出 v4 可表达范围（day={day}）"))
}

/// 用编译期 Secret 签名 + 加密一个 URL（客户端 / 默认服务端用法）。
pub fn sign_and_encrypt(
    signing_key: &ed25519_dalek::SigningKey,
    url: &str,
    ttl_days: u64,
    sign_day: Option<u64>,
) -> Result<String, String> {
    sign_and_encrypt_with_secret(signing_key, url, ttl_days, sign_day, SECRET)
}

/// 用 Ed25519 私钥签名 + AES-GCM 加密一个 URL，返回 base64 密文（单行，写入 DNS TXT）。
///
/// - `url`：要隐藏的真实地址。
/// - `ttl_days`：有效期**天数**（>0 按天窗口校验；=0 永不失效；上限 65535）。
/// - `sign_day`：可选，指定签发日（距 2026-01-01 的天数，测试用；默认取当前 UTC 日）。
/// - `secret`：本次加密使用的对称 Secret 原始字节（服务端 `--secret` 参数透传；
///   HKDF 直接吃字符串字节，不做 hex/base64 解码）。**必须与目标客户端构建内置的
///   RESOLVE_SECRET 逐字节一致**，否则客户端解密必然失败。
pub fn sign_and_encrypt_with_secret(
    signing_key: &ed25519_dalek::SigningKey,
    url: &str,
    ttl_days: u64,
    sign_day: Option<u64>,
    secret: &[u8],
) -> Result<String, String> {
    if ttl_days > MAX_DAYS {
        return Err(format!("有效期天数超出上限 {MAX_DAYS}（收到 {ttl_days}）"));
    }
    let day = match sign_day {
        Some(d) => {
            if d > MAX_DAYS {
                return Err(format!("签发日超出上限 {MAX_DAYS}（收到 {d}）"));
            }
            d as u16
        }
        None => current_day()?,
    };

    // 随机 Nonce(12B)：既作 GCM nonce，又作 HKDF 的 salt，已足以保证每次密文与
    // 派生密钥均不同，无需再放随机盐。
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut nonce_bytes);

    let payload = build_payload(url, day, ttl_days as u16);

    // Ed25519 签名（64 字节）。
    let sig: Signature = signing_key.sign(&payload);

    // AES-GCM 加密 Payload || Signature。
    let key = derive_key_with_secret(&nonce_bytes, secret);
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

/// 一条密文解密 + 验签后的结构化结果（供 `resolve-tool decrypt` 等需要展示
/// 签发日期 / TTL 的入口使用；客户端运行时只用其中的 [`DecryptedRecord::url`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecryptedRecord {
    /// 真实目标 URL。
    pub url: String,
    /// 签发日（距基准日 2026-01-01 的 UTC 天数，Payload 固定字段，ttl_days=0 时依然存在）。
    pub sign_day: u16,
    /// 有效期天数；0 = 永不失效。
    pub ttl_days: u16,
}

impl DecryptedRecord {
    /// 有效期窗口校验（与客户端 `parse_payload` 的判定完全一致）：
    /// `ttl_days > 0` 且今日与签发日偏差超过 ttl_days ⇒ 拒绝；`ttl_days = 0` 恒通过。
    pub fn check_ttl(&self) -> Result<(), String> {
        check_ttl(self.sign_day, self.ttl_days)
    }
}

/// 用编译期 Secret 解密 + 验签一条 TXT 密文（客户端唯一入口）。
pub fn decrypt_txt(public_key: &VerifyingKey, b64: &str) -> Result<String, String> {
    decrypt_txt_with_secret(public_key, b64, SECRET)
}

/// 用编译期 Secret 解密 + 验签一条 TXT 密文，返回结构化结果（URL + 签发时间 + TTL）。
pub fn decrypt_record(public_key: &VerifyingKey, b64: &str) -> Result<DecryptedRecord, String> {
    decrypt_record_with_secret(public_key, b64, SECRET)
}

/// 解密 + 验签一条 TXT 密文，返回真实 URL（可显式指定 Secret，供服务端工具验证）。
///
/// 流程与 [`decrypt_record_with_secret`] 相同；在结构化解密成功后额外做有效期窗口
/// 校验（过期即拒绝，与客户端运行时行为一致），只返回 URL。
pub fn decrypt_txt_with_secret(
    public_key: &VerifyingKey,
    b64: &str,
    secret: &[u8],
) -> Result<String, String> {
    let record = decrypt_record_with_secret(public_key, b64, secret)?;
    record.check_ttl()?;
    Ok(record.url)
}

/// 解密 + 验签一条 TXT 密文，返回结构化结果（URL + 签发日 + TTL，可显式指定 Secret）。
///
/// 与 [`decrypt_txt_with_secret`] 的区别：**不做有效期窗口判定**，即使记录已过期
/// （ttl_days > 0 且已超时）也照常返回解出的全部字段，由调用方自行决定是否拒绝——
/// `resolve-tool decrypt` 借此把过期记录的 URL / 签发日也展示出来用于诊断。
/// 密码学两道硬验证（AES-GCM 认证、Ed25519 验签）一步不少，任一失败即返回 Err。
pub fn decrypt_record_with_secret(
    public_key: &VerifyingKey,
    b64: &str,
    secret: &[u8],
) -> Result<DecryptedRecord, String> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|e| format!("密文 base64 解码失败：{e}"))?;

    // 至少要有 Nonce(12) + 密文(≥ tag 16 字节)。
    if raw.len() < NONCE_LEN + 16 {
        return Err("密文长度异常".to_string());
    }
    let (nonce_bytes, cipher) = raw.split_at(NONCE_LEN);
    let nonce = AesNonce::from_slice(nonce_bytes);

    let key = derive_key_with_secret(nonce_bytes, secret);
    let cipher_suite =
        Aes256Gcm::new_from_slice(&key).map_err(|e| format!("初始化 AES 失败：{e}"))?;

    // 解密 + 认证。认证失败（密文被篡改 / Secret 不匹配）会在此抛错。
    let plain = cipher_suite
        .decrypt(
            nonce,
            AeadPayload {
                msg: cipher,
                aad: b"",
            },
        )
        .map_err(|_| "密文认证失败（数据被篡改或 Secret 不匹配）".to_string())?;

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

    // 只解析 Payload 字段（不做有效期判定），组装结构化结果。
    let (url, sign_day, ttl_days) = parse_payload_parts(payload)?;
    Ok(DecryptedRecord {
        url,
        sign_day,
        ttl_days,
    })
}

/// 解析 Payload，提取 URL 并校验有效期（按天防重放）。
///
/// 唯一受支持的 v4 结构：`URL || 0x00 || d(2B 大端) || ttl_d(2B 大端)`，
/// 分隔符之后固定 4 字节。v3 秒级格式（16B 尾）与更早 v1/v2 密文一律按
/// 「Payload 尾部结构异常」拒绝，不做向后兼容（线上旧 TXT 需重新签发）。
pub fn parse_payload(payload: &[u8]) -> Result<String, String> {
    let (url, sign_day, ttl_days) = parse_payload_parts(payload)?;
    check_ttl(sign_day, ttl_days)?;
    Ok(url)
}

/// 只解析 v4 Payload 的字段，返回（URL，签发日，有效期天数），不做有效期判定。
///
/// 与 [`parse_payload`] 共用同一套格式校验（含对 v3/v1/v2 旧尾部的拒绝），区别仅在于
/// 跳过日期窗口检查，供结构化解密入口使用。
pub fn parse_payload_parts(payload: &[u8]) -> Result<(String, u16, u16), String> {
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

    // 尾部必须恰好为 d(2) || ttl_d(2)，共 4 字节；任何其他长度（含 v3 的 16B）拒绝。
    let rest = &payload[sep + 1..];
    let fixed = FIELD_LEN + FIELD_LEN;
    if rest.len() != fixed {
        return Err("Payload 尾部结构异常".to_string());
    }
    let sign_day = u16::from_be_bytes(rest[..FIELD_LEN].try_into().unwrap());
    let ttl_days = u16::from_be_bytes(rest[FIELD_LEN..fixed].try_into().unwrap());
    Ok((url, sign_day, ttl_days))
}

/// 有效期窗口校验（UTC 天）：`ttl_days > 0` 时今日与签发日偏差超过 ttl_days ⇒ 拒绝；
/// `ttl_days = 0` 表示永不失效，直接通过。系统时钟不可用时无法判定，按放行处理。
fn check_ttl(sign_day: u16, ttl_days: u16) -> Result<(), String> {
    if ttl_days == 0 {
        return Ok(());
    }
    let today = match current_day() {
        Ok(d) => d,
        // 取不到今日（时钟异常）无法判定时效，按放行处理——不因此阻断客户端启动。
        Err(_) => return Ok(()),
    };
    let diff = today.abs_diff(sign_day);
    if diff > ttl_days {
        return Err(format!(
            "记录已过期（签发日第 {sign_day} 天，有效期 {ttl_days} 天，偏差 {diff} 天）"
        ));
    }
    Ok(())
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

    // 基准日常量钉死：2026-01-01 ⇒ 20454（56 年含 14 个闰年）。
    #[test]
    fn base_unix_day_constant() {
        assert_eq!(BASE_UNIX_DAY, 20454);
    }

    // 服务端 sign_and_encrypt → 客户端 decrypt_txt 往返对称性（v4 按天，含 ttl_days 各种情况）。
    #[test]
    fn e2e_sign_encrypt_roundtrip() {
        let (priv_pem, pub_b64) = generate_keypair().unwrap();
        let signing_key = parse_signing_key_pem(&priv_pem)
            .expect("generate_keypair 产出的 PEM 应能被 parse_signing_key_pem 解析");
        let key = parse_public_key(&pub_b64).unwrap();

        // ttl_days = 0（永不失效）。
        let c = sign_and_encrypt(&signing_key, "https://a.com/", 0, None).unwrap();
        assert_eq!(decrypt_txt(&key, &c).unwrap(), "https://a.com/");

        // ttl_days = 7（7 天窗口，当日签发应有效）。
        let c = sign_and_encrypt(&signing_key, "https://b.com/", 7, None).unwrap();
        assert_eq!(decrypt_txt(&key, &c).unwrap(), "https://b.com/");

        // ttl_days = 1 但签发日是第 0 天（2026-01-01，距今远超 1 天）→ 应因过期被拒绝。
        let c = sign_and_encrypt(&signing_key, "https://c.com/", 1, Some(0)).unwrap();
        assert!(decrypt_txt(&key, &c).is_err(), "过期密文应被拒绝");
    }

    // 构造 v4 Payload：URL || 0x00 || d(2) || ttl_d(2)（尾部固定 4 字节）。
    fn v4_payload(url: &str, day: u16, ttl_days: u16) -> Vec<u8> {
        let mut p = url.as_bytes().to_vec();
        p.push(0x00);
        p.extend_from_slice(&day.to_be_bytes());
        p.extend_from_slice(&ttl_days.to_be_bytes());
        p
    }

    #[test]
    fn parse_payload_missing_separator() {
        assert!(parse_payload(b"no-separator").is_err());
    }

    #[test]
    fn parse_payload_valid() {
        let today = current_day().unwrap();
        let payload = v4_payload("https://a.com", today, 7); // 7 天窗口
        assert_eq!(parse_payload(&payload).unwrap(), "https://a.com");
    }

    #[test]
    fn parse_payload_boundary_inclusive() {
        // 边界：今日 - 签发日 == ttl_days 时仍有效（<= 判定）。
        let today = current_day().unwrap();
        let payload = v4_payload("https://a.com", today - 7, 7);
        assert_eq!(parse_payload(&payload).unwrap(), "https://a.com");
    }

    #[test]
    fn parse_payload_expired() {
        // 签发日 0（2026-01-01），窗口 1 天 → 今日必然已过期。
        let payload = v4_payload("https://a.com", 0, 1);
        assert!(parse_payload(&payload).is_err());
    }

    #[test]
    fn parse_payload_ttl_zero_never_expires() {
        let payload = v4_payload("https://a.com", 0, 0);
        assert_eq!(parse_payload(&payload).unwrap(), "https://a.com");
    }

    // 只认 v4（4B 尾）：v3 秒级（16B 尾）/ v2 带盐 / v1 无 ttl 及任何其他尾部长度都必须拒绝。
    #[test]
    fn parse_payload_rejects_legacy_and_bad_tails() {
        // v3：ts8 + ttl8（16B，秒级时间戳，上一版格式）。
        let mut v3 = b"https://a.com".to_vec();
        v3.push(0x00);
        v3.extend_from_slice(&[0u8; 16]);
        assert!(parse_payload(&v3).is_err(), "v3 秒级旧格式必须拒绝");

        // v2：ts8 + ttl8 + 0x00 + salt8（25B）。
        let mut v2 = b"https://a.com".to_vec();
        v2.push(0x00);
        v2.extend_from_slice(&[0u8; 16]);
        v2.push(0x00);
        v2.extend_from_slice(&[1u8; 8]);
        assert!(parse_payload(&v2).is_err(), "v2 带盐旧格式必须拒绝");

        // v1：ts8 + 0x00 + salt8（17B）。
        let mut v1 = b"https://a.com".to_vec();
        v1.push(0x00);
        v1.extend_from_slice(&[0u8; 8]);
        v1.push(0x00);
        v1.extend_from_slice(&[1u8; 8]);
        assert!(parse_payload(&v1).is_err(), "v1 无 ttl 旧格式必须拒绝");

        // 任意其他长度（20B / 15B / 3B / 5B）同样拒绝。
        for tail_len in [20usize, 15, 3, 5] {
            let mut bad = b"https://a.com".to_vec();
            bad.push(0x00);
            bad.extend_from_slice(&vec![0u8; tail_len]);
            assert!(parse_payload(&bad).is_err(), "尾部 {tail_len}B 必须拒绝");
        }
    }

    // 密文长度钉死：raw = Nonce12 + URL + 1 + d2 + ttl_d2 + 签名64 + tag16，
    // base64 长度 = 4*ceil(raw/3)。
    #[test]
    fn e2e_ciphertext_length_formula() {
        let (priv_pem, pub_b64) = generate_keypair().unwrap();
        let signing_key = parse_signing_key_pem(&priv_pem).unwrap();
        let key = parse_public_key(&pub_b64).unwrap();

        for url in [
            "https://a.com/",
            "https://user:pass@host",
            "https://x.io/a/b/c?d=1",
        ] {
            let c = sign_and_encrypt(&signing_key, url, 0, None).unwrap();
            assert_eq!(decrypt_txt(&key, &c).unwrap(), url);

            let raw = NONCE_LEN + url.len() + 1 + FIELD_LEN + FIELD_LEN + SIG_LEN + 16;
            assert_eq!(c.len(), ((raw + 2) / 3) * 4, "URL={url} 密文长度不符");
        }
    }

    // 签发日 / ttl_days 超 u16 范围必须明确报错（不静默截断）。
    #[test]
    fn rejects_out_of_range_days() {
        let (priv_pem, _pub_b64) = generate_keypair().unwrap();
        let signing_key = parse_signing_key_pem(&priv_pem).unwrap();

        assert!(sign_and_encrypt(&signing_key, "https://a.com/", 65536, None).is_err());
        assert!(sign_and_encrypt(&signing_key, "https://a.com/", 0, Some(65536)).is_err());
        assert!(sign_and_encrypt(&signing_key, "https://a.com/", 65535, Some(65535)).is_ok());
    }

    // v4 固定向量：用 txttest_keys/private_key.pem（公钥如下）经 resolve-tool 以
    // `--ttl-days 0` 于 2026-10-07（第 279 天）生成，永不失效，作为跨次运行的稳定验签素材。
    // 重新生成：cargo run -p resolve-tool -- encrypt txttest_keys/private_key.pem <url> --ttl-days 0 [--count N]
    // 注意：这把公钥必须与磁盘 txttest_keys/private_key.pem 成对（2026-10-07 起
    // 该目录为 9/22 重新生成的密钥对，旧 9/3 公钥 Sg9z… 的私钥已遗失、旧密文全部作废）。
    const TXTTEST_PUB_B64: &str = "HkCpkH1tMpNIxWoWAkvlX8vm93BOc5PtIKrQxl8RUDo=";

    #[test]
    fn e2e_fixed_vectors_v4_decrypt() {
        let key = parse_public_key(TXTTEST_PUB_B64).unwrap();

        let baidu = [
            "lYZ/Zc9lcULVU1P4Yi2rWphUpDbRQB8eNc7F8JEJ+tuehBvKYPyrG2+7w6nFxP3bUxUs7h2hxJcLJ1DYqYSYkF02lShVfqugBzjYnn/AhL1MaA4ccAOus4hbMw7jyCauZNlVpFJQFpEtXwuEMV+oHbhbncRrfww=",
            "UkNSfLPY61NRy4gNTWnnlcoRku+KlahZxjgBEYXECslCr5zZKZ/SCbVVWvaIsd83i54P0M/8HgRK0HGXnSDfI3eEl7zzjcCZp39u2Ax+PntG5xppR9GwX6YFJxUCV6qwIN4sHlM2GrxrbqQnNRpaQtzUsj/tkKE=",
            "/qWiSuR+S3zDrV/bAIQt70r+I2yGnr0jQewLiCu72HwsUuSXyptXasjRH/KFOaSj3h+fkdOBQuBKmcZ1wxuyIl2J4ARuJQRMBYe9jm8UvymWABjuAOVlFPOvQYP6hGmXBSsuwiiiNdDV1xmqTSyDsvinwFtw9d8=",
        ];
        let bili = [
            "Gr/hABqoSiiWf6EKrjB+Dh875I9WjGMr6jezUnFlcqJLd7ZwtjY9N6Xfnkpwkegi2le/17Cgnbk6Kfgv3mPPraYIkawTy7GUtoLYx44pjqJ5Zkf3rmEqSvR94eesCnmlKbWVrbXvx4+CJ895cM1wzeZ8KyaeY10Md0Y=",
        ];
        let example = [
            "qCBIQUvebOEpwdIDJmf1gVDKpSBTfTMM1GqWIusdjnCWauNmQJZl5oZ7LUHJL1Y5XanRf5USMWOY3MPOcIhU7f6Hs9lDhMuA6741Wn2/HkXGqCK49nngf8B536vjQPFtyAhXzS7MZEZwH5rTEsupT1PuGE/0",
        ];

        assert_ne!(baidu[0], baidu[1]);
        assert_ne!(baidu[1], baidu[2]);
        for c in baidu {
            assert_eq!(decrypt_txt(&key, c).unwrap(), "https://www.baidu.com/");
            // 22 字符 URL：raw=119 ⇒ base64 160 字符（v3 同期为 176）。
            assert_eq!(c.len(), 160);
        }
        for c in bili {
            assert_eq!(decrypt_txt(&key, c).unwrap(), "https://www.bilibili.com/");
        }
        for c in example {
            assert_eq!(decrypt_txt(&key, c).unwrap(), "https://example.com/");
        }
    }

    // 防篡改：翻转密文首块一个字节，AES-GCM 认证必须失败。
    #[test]
    fn e2e_tamper_detected() {
        let key = parse_public_key(TXTTEST_PUB_B64).unwrap();
        let b1 = "lYZ/Zc9lcULVU1P4Yi2rWphUpDbRQB8eNc7F8JEJ+tuehBvKYPyrG2+7w6nFxP3bUxUs7h2hxJcLJ1DYqYSYkF02lShVfqugBzjYnn/AhL1MaA4ccAOus4hbMw7jyCauZNlVpFJQFpEtXwuEMV+oHbhbncRrfww=";

        assert_eq!(decrypt_txt(&key, b1).unwrap(), "https://www.baidu.com/");

        let raw = base64::engine::general_purpose::STANDARD
            .decode(b1)
            .unwrap();
        let mut tampered = raw.clone();
        tampered[NONCE_LEN] ^= 0x01;
        let tampered_b64 = base64::engine::general_purpose::STANDARD.encode(&tampered);
        assert!(
            decrypt_txt(&key, &tampered_b64).is_err(),
            "篡改密文应被拒绝"
        );
    }

    // --secret：显式 Secret 加密只有同一 Secret 能解；编译期默认 Secret 解不开。
    #[test]
    fn e2e_custom_secret_roundtrip() {
        let (priv_pem, pub_b64) = generate_keypair().unwrap();
        let signing_key = parse_signing_key_pem(&priv_pem).unwrap();
        let key = parse_public_key(&pub_b64).unwrap();
        let url = "https://secret.example.com/";
        let custom = b"a-one-off-custom-secret-from-cli";

        let c = sign_and_encrypt_with_secret(&signing_key, url, 0, None, custom).unwrap();
        // 同一自定义 Secret：正常往返。
        assert_eq!(decrypt_txt_with_secret(&key, &c, custom).unwrap(), url);
        // 错误 Secret（含编译期默认值）：GCM 认证失败。
        assert!(
            decrypt_txt(&key, &c).is_err(),
            "默认 Secret 解不开自定义密文"
        );
        assert!(
            decrypt_txt_with_secret(&key, &c, b"wrong-secret").is_err(),
            "错误 Secret 必须解密失败"
        );

        // 反过来：默认 Secret 加密的密文，自定义 Secret 也解不开。
        let c_default = sign_and_encrypt(&signing_key, url, 0, None).unwrap();
        assert!(decrypt_txt_with_secret(&key, &c_default, custom).is_err());
        assert_eq!(decrypt_txt(&key, &c_default).unwrap(), url);
    }

    // 结构化解密：返回 URL + 签发日 + TTL；过期记录也能取出全部字段（不做窗口判定）。
    #[test]
    fn e2e_decrypt_record_parts() {
        let (priv_pem, pub_b64) = generate_keypair().unwrap();
        let signing_key = parse_signing_key_pem(&priv_pem).unwrap();
        let key = parse_public_key(&pub_b64).unwrap();

        // ttl_days=0 + 固定签发日（第 100 天）：结构化结果逐字段对齐。
        let c = sign_and_encrypt_with_secret(
            &signing_key,
            "https://parts.example.com/",
            0,
            Some(100),
            SECRET,
        )
        .unwrap();
        assert_eq!(
            decrypt_record(&key, &c).unwrap(),
            DecryptedRecord {
                url: "https://parts.example.com/".to_string(),
                sign_day: 100,
                ttl_days: 0,
            }
        );

        // ttl_days>0 且早已过期（第 0 天签发，窗口 1 天）：decrypt_record 照常返回，decrypt_txt 必须拒绝。
        let c_expired = sign_and_encrypt_with_secret(
            &signing_key,
            "https://old.example.com/",
            1,
            Some(0),
            SECRET,
        )
        .unwrap();
        let r = decrypt_record(&key, &c_expired).unwrap();
        assert_eq!(r.url, "https://old.example.com/");
        assert_eq!(r.sign_day, 0);
        assert_eq!(r.ttl_days, 1);
        assert!(r.check_ttl().is_err(), "过期记录 check_ttl 必须拒绝");
        assert!(decrypt_txt(&key, &c_expired).is_err());
    }
}
