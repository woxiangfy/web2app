//! 动态解析真实 URL 的【DNS 查询层】。
//!
//! 加解密 / 签名验签 / 密钥对生成等纯 Rust 逻辑已下沉到 `resolve-core` crate；
//! 本模块只保留依赖 hickory-resolver 的 DNS TXT 查询与域名遍历逻辑，
//! 并 re-export `resolve-core` 的 API 供 lib.rs / resolve-tool 使用。
//!
//! ⚠️⚠️ 本模块**刻意不依赖 Tauri**，且对外接口一律是 `async`。
//! 原因：`src/proxy.rs` 与本文件被 Electron 侧的 sidecar（`crates/proxy-server`）以
//! `#[path]` 引入复用（单一真源）。sidecar 是纯 Rust 二进制，拉入 tauri 会带进整套
//! glib/webview 依赖树，且 `tauri::async_runtime::block_on` 在非 Tauri 进程里根本没初始化。
//! 所以「谁来驱动这些 future」交给调用方：
//!   · 主 crate（Tauri）：`tauri::async_runtime::block_on(resolve_url_from_domains(..))`
//!   · sidecar（纯 tokio）: 自己建 runtime 驱动。
//! 往本文件里加 `use tauri::...` 会立刻编译不过 —— 这是有意的护栏。

use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::system_conf::read_system_conf;
use hickory_resolver::Resolver;

// 从 resolve-core 复用纯加密核心，并向上层 re-export 关键 API。
pub use resolve_core::{
    decrypt_txt, generate_keypair, normalize_domain, parse_domains, parse_public_key,
    sign_and_encrypt, VerifyingKey, RESOLVE_FAILED,
};

/// 查询一个域名的【所有】 TXT 记录（每条拼接其字符串段），返回全部非空值。
///
/// 一个域名可挂多条 TXT（对应「同一 URL 的多份密文」），客户端应对每条都尝试解密，
/// 任一成功即可。故这里返回 Vec 而非单条。
///
/// 走系统默认 DNS。hickory-resolver 0.26 起移除了同步便捷方法 `Resolver::from_system_conf`，
/// 改为：`system_conf::read_system_conf()` 读系统配置 → `Resolver::builder_with_config` 构建
/// （泛型 `Resolver<P>`，provider 用 `TokioRuntimeProvider`）→ `txt_lookup` 是 async。
/// 构建一次 DNS Resolver，供整轮解析复用。
///
/// 原来的实现每查一个域名都重建 Resolver（重新读系统配置、重建内部 reactor），
/// 既浪费又无法共享连接池；这里改为只建一次，多个域名共享同一个实例。
fn build_resolver() -> Result<Resolver<TokioRuntimeProvider>, String> {
    // 读取系统 DNS 配置（/etc/resolv.conf 或 Windows 注册表）。
    let (config, _opts) = read_system_conf().map_err(|e| format!("读取系统 DNS 配置失败：{e}"))?;
    Resolver::builder_with_config(config, TokioRuntimeProvider::default())
        .build()
        .map_err(|e| format!("初始化 DNS 解析器失败：{e}"))
}

/// 用给定的 Resolver 查询一个域名的【所有】 TXT 记录。
async fn query_txt_all(
    resolver: &Resolver<TokioRuntimeProvider>,
    domain: &str,
) -> Result<Vec<String>, String> {
    let lookup = resolver
        .txt_lookup(domain)
        .await
        .map_err(|e| format!("查询 {domain} TXT 记录失败：{e}"))?;

    // 遍历 answers 里的每条记录，拼接 TXT 的所有字符串段；收集全部非空结果。
    let mut results: Vec<String> = Vec::new();
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
                results.push(joined);
            }
        }
    }
    if results.is_empty() {
        return Err(format!("域名 {domain} 无 TXT 记录"));
    }
    Ok(results)
}

/// 对一组 TXT 密文逐条解密 + 验签，返回第一条能解出【非空 URL】的结果。
///
/// 纯函数（不碰 DNS），便于单测。任一成功即返回，全部失败返回 `Err`。
fn decrypt_first_valid_txt(
    public_key: &resolve_core::VerifyingKey,
    txts: &[String],
) -> Result<String, String> {
    for txt in txts {
        match decrypt_txt(public_key, txt) {
            Ok(url) => {
                let url = url.trim().to_string();
                if !url.is_empty() {
                    return Ok(url);
                }
            }
            Err(_e) => {
                // 单条 TXT 解密/验签失败（别的公钥签的、已过期、被篡改）——继续下一条。
            }
        }
    }
    Err("所有 TXT 记录均无法解出有效 URL".to_string())
}

/// 依次尝试一组域名：查询 TXT → 解密 + 验签 → 得到真实 URL。
///
/// 对每个域名：读取其【全部】 TXT 记录，逐条尝试解密 + 验签，任一成功即返回；
/// 该域名所有 TXT 都失败则尝试下一个域名；全部失败返回 `RESOLVE_FAILED`。
///
/// 调用方负责驱动这个 future（见文件头注释）。
pub async fn resolve_url_from_domains(
    domains: &[String],
    public_key_pem: &str,
) -> Result<String, String> {
    let public_key = parse_public_key(public_key_pem)?;

    // Resolver 只构建一次，多个域名复用（原实现每域重建，浪费系统配置读取与 reactor 初始化）。
    let resolver = build_resolver()?;

    for domain in domains {
        let domain = domain.trim();
        if domain.is_empty() {
            continue;
        }
        let txts = match query_txt_all(&resolver, domain).await {
            Ok(t) => t,
            Err(_) => {
                // 单个域名查询失败：记录后尝试下一个域名。
                eprintln!("[resolve] 域名 {domain} 解析失败，尝试下一个");
                continue;
            }
        };
        match decrypt_first_valid_txt(&public_key, &txts) {
            Ok(url) => return Ok(url),
            Err(_) => {
                eprintln!("[resolve] 域名 {domain} 的所有 TXT 均无法解出有效 URL，尝试下一个");
            }
        }
    }
    Err(RESOLVE_FAILED.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // 真实 DNS 端到端：查询真实域名 TXT → 解密 + 验签，得到真实 URL。
    // 依赖外部 DNS（需联网），用 #[ignore] 标注，默认不跑；手动 `cargo test -- --ignored` 触发。
    // ⚠️ 用 multi_thread flavor：hickory 内部会 spawn 后台任务（UDP/TCP socket 读写），
    // current_thread runtime 下容易出现「任务已 spawn 但没机会跑」的假超时。
    const TEST_PUB_B64: &str = "Sg9zFyUaBwWwGPm8TshK/LSNpZMbw0Ex4kMcwaK40jU=";

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "需要联网查询真实 DNS TXT 记录"]
    async fn e2e_real_dns_resolve_baidu() {
        let domains = vec!["txttest1.lcjm.cc".to_string()];
        let url = resolve_url_from_domains(&domains, TEST_PUB_B64)
            .await
            .unwrap();
        assert_eq!(url, "https://www.baidu.com/");
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "需要联网查询真实 DNS TXT 记录"]
    async fn e2e_real_dns_resolve_bilibili() {
        let domains = vec!["txttest2.mygit.top".to_string()];
        let url = resolve_url_from_domains(&domains, TEST_PUB_B64)
            .await
            .unwrap();
        assert_eq!(url, "https://www.bilibili.com/");
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "需要联网查询真实 DNS TXT 记录"]
    async fn e2e_real_dns_failover_to_next() {
        // txttest3 是旧 RSA 密文（新方案解不开），txttest1 是新密文。
        // 应自动跳过 txttest3 失败，命中 txttest1 成功。
        let domains = vec![
            "txttest3.lcjm.cloudns.biz".to_string(), // 旧密文，解密失败
            "txttest1.lcjm.cc".to_string(),          // 新密文，成功
        ];
        let url = resolve_url_from_domains(&domains, TEST_PUB_B64)
            .await
            .unwrap();
        assert_eq!(url, "https://www.baidu.com/");
    }

    // ---- 纯逻辑单测（不依赖 DNS）----

    /// 同一 URL 多份密文：第 1 条是垃圾、第 2 条有效、第 3 条是别的公钥签的，
    /// `decrypt_first_valid_txt` 应跳过无效项、命中第 2 条有效密文。
    #[test]
    fn decrypt_first_valid_txt_skips_bad_and_hits_valid() {
        let (priv_pem, pub_b64) = resolve_core::generate_keypair().unwrap();
        let signing_key = resolve_core::parse_signing_key_pem(&priv_pem).unwrap();
        let public_key = parse_public_key(&pub_b64).unwrap();

        // 同一 URL 生成 3 份密文。
        let good2 =
            resolve_core::sign_and_encrypt(&signing_key, "https://real.example.com/", 0, None)
                .unwrap();

        // 用另一把公钥签一条密文，模拟「别的公钥签的 / 被篡改」。
        let (other_priv, _) = resolve_core::generate_keypair().unwrap();
        let other_key = resolve_core::parse_signing_key_pem(&other_priv).unwrap();
        let other_signed =
            resolve_core::sign_and_encrypt(&other_key, "https://evil.example.com/", 0, None)
                .unwrap();

        let txts = vec![
            "绝对不是合法密文".to_string(), // 第 1 条：垃圾，base64 解码即失败
            good2,                          // 第 2 条：有效
            other_signed,                   // 第 3 条：别的公钥，验签失败
        ];

        let url = decrypt_first_valid_txt(&public_key, &txts).unwrap();
        assert_eq!(url, "https://real.example.com/");
    }

    /// 所有 TXT 都无效时，返回 Err。
    #[test]
    fn decrypt_first_valid_txt_all_bad_returns_err() {
        let (_, pub_b64) = resolve_core::generate_keypair().unwrap();
        let public_key = parse_public_key(&pub_b64).unwrap();
        let txts = vec!["垃圾1".to_string(), "垃圾2".to_string()];
        assert!(decrypt_first_valid_txt(&public_key, &txts).is_err());
    }
}
