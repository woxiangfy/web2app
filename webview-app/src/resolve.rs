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
use hickory_resolver::proto::rr::Name;
use hickory_resolver::system_conf::read_system_conf;
use hickory_resolver::Resolver;

// 从 resolve-core 复用纯加密核心，并向上层 re-export 关键 API。
pub use resolve_core::{
    decrypt_txt, generate_keypair, normalize_domain, parse_domains, parse_public_key,
    sign_and_encrypt, VerifyingKey, RESOLVE_FAILED,
};

/// 把配置里的域名转成查询用的 [`Name`]。
///
/// ⚠️⚠️ **必须用 `from_str_relaxed`，不能用默认的 `txt_lookup(&str)`。**
/// `Resolver::txt_lookup` 对 `&str` 走 `Name::from_utf8` ⇒ 每个 label 过 IDNA(`AsciiDenyList::STD3`)，
/// 而 STD3 **只允许字母/数字/连字符**，于是**下划线**只在「label 首字符」被特判放过
/// （SRV 记录那种 `_dmarc` / `_acme-challenge`），**出现在中间就整条拒绝**：
///
/// ```text
/// _dmarc.example.com   → OK          （首字符特判）
/// txt_w2a_abc.lcjm.cc  → 报错 protocol error: Label contains invalid characters: Err(Errors)
/// ```
///
/// 这个报错发生在**构造查询名字**这一步，请求根本没发出去，日志里只会看到
/// 「域名 X 解析失败」，用户侧则是「服务器连接失败，请检查网络后重试」——**指向网络，实际是命名**。
/// 2026-11-14 实测就是 `txt_w2a_lcjm26.*` 这种「下划线在中间」的 TXT 记录名，
/// 三个候选域名全部这样 ⇒ 装完必然打不开（且看起来像网络问题）。
///
/// `Name::from_str_relaxed` = 先按严格规则试，失败再按 `from_ascii`（允许 `_`、`*` 等）构造，
/// 与 hickory 文档给出的正是这个用途。DNS 协议本身允许 owner name 含下划线，严格校验只是解析器的
/// 自我约束 ⇒ 放宽后既能查 `_dmarc` 这类规范记录，也能查 `txt_w2a_*` 这类实际存在的记录。
fn lookup_name(domain: &str) -> Result<Name, String> {
    Name::from_str_relaxed(domain).map_err(|e| format!("域名 {domain} 不是合法的 DNS 名称：{e}"))
}

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
    let name = lookup_name(domain)?;
    let lookup = resolver
        .txt_lookup(name)
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
            Err(e) => {
                // 单个域名查询失败：记录**具体原因**后尝试下一个域名。
                // ⚠️ 必须带上 `{e}`：这里曾写成 `Err(_)`，把「查询失败 / 无 TXT 记录」的唯一线索
                //    吞掉，用户只能看到「服务器连接失败」而无法判断是 DNS 配置、查询超时还是真的没记录
                //    （与 `startup_visibility_contract` 对主 crate 的要求同一条纪律）。
                //    `dlog!` 走 stderr，不会污染 sidecar 的 stdout 单行 JSON 协议。
                dlog!("[resolve] 域名 {domain} 解析失败：{e}，尝试下一个");
                continue;
            }
        };
        match decrypt_first_valid_txt(&public_key, &txts) {
            Ok(url) => return Ok(url),
            Err(_) => {
                dlog!("[resolve] 域名 {domain} 的所有 TXT 均无法解出有效 URL，尝试下一个");
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
    //
    // 2026-10-07 复核：3 条用例当前均失败，属测试夹具失效、与解析代码无关——
    //   ① txttest1.lcjm.cc 已 NXDOMAIN；② 线上旧 TXT 均为 v1/v2/v3 格式，resolve-core
    //   只认 v4（尾部固定 4B 按天格式），旧记录在结构层即被拒绝（不再是「能解但过期」）；
    //   ③ txttest_keys 于 2026-09-22 被整体重新生成，旧公钥 Sg9z… 的私钥已遗失，下面的
    //   常量已换成现存私钥对应的公钥 HkCp…。恢复方法：续用/更换域名后用当前私钥重新签发
    //   v4 TXT（建议 --ttl-days 0）并更新 DNS，详见 docs/txttest-verify-materials.md。
    const TEST_PUB_B64: &str = "HkCpkH1tMpNIxWoWAkvlX8vm93BOc5PtIKrQxl8RUDo=";

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

    /// 记录名里**下划线在中间**必须能构造出查询名（2026-11-14 线上事故的回归用例）。
    ///
    /// 旧实现直接 `resolver.txt_lookup(&str)` ⇒ 走 IDNA/STD3 ⇒ `txt_w2a_…` 报
    /// `Label contains invalid characters`，请求根本没发出去，用户只看到「服务器连接失败」。
    #[test]
    fn lookup_name_accepts_underscore_inside_label() {
        for d in [
            "txt_w2a_lcjm26.lcjm.cc",   // 事故现场的真实记录名
            "txt_w2a_lcjm26.lassl.vip", // 同批其它候选域名
            "_dmarc.google.com",        // 首字符下划线（本来就能过，别被放宽改坏）
            "txttest1.lcjm.cc",         // 不含下划线
            "a.b.c",                    // 多级
        ] {
            let name = lookup_name(d).unwrap_or_else(|e| panic!("{d} 应可构造查询名：{e}"));
            // 尾部补点/大小写归一不影响「能查」，这里只断言 label 数与原文一致。
            assert_eq!(
                name.num_labels() as usize,
                d.split('.').count(),
                "{d} 的 label 数变了：{name}"
            );
        }
    }

    /// 真正非法的名字（含空格）仍然要报错，不能把校验整体关掉。
    ///
    /// ⚠️ 空串**不在此列**：hickory 把它当根域名 `.`（`from_ascii("")` 合法），而空串在
    /// `resolve_url_from_domains` 里已被 `if domain.is_empty() { continue; }` 提前跳过 ⇒
    /// 这里断言它报错会写出一个与实现不符的「假期望」。
    #[test]
    fn lookup_name_rejects_truly_invalid() {
        assert!(lookup_name("bad name.example.com").is_err());
    }
}
