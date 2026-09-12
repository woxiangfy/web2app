//! resolve-tool —— 动态解析的【服务端】命令行工具。
//!
//! 用于在打包之外，本地/CI 上用 Ed25519 私钥生成：
//!   1. 密钥对（Ed25519 私钥 PEM + 公钥裸 base64）
//!   2. TXT 密文（私钥签名 + AES-256-GCM 加密，写入 DNS TXT）
//!
//! 与客户端 `resolve-core` 的解密逻辑严格对称；与 `tools/encrypt_url.py` 等价，
//! 提供纯 Rust 版本，方便在 CI（无需 Python cryptography 依赖）里使用。
//!
//! 本 crate 只依赖 `resolve-core`（纯 Rust），**不依赖 Tauri**，因此可在 ubuntu-latest
//! 等没有 glib-2.0 开发库的 Linux runner 上直接 `cargo run` 编译，无需装系统库。
//!
//! 用法：
//!   resolve-tool genkey [--out DIR]             生成密钥对，写 private_key.pem + public_key.b64
//!   resolve-tool encrypt <private_key.pem> <url> [--ttl 秒] [--count N]
//!      用私钥加密一个 URL，stdout 输出 base64 密文；--count N 表示同一 URL 生成 N 份密文
//!   resolve-tool encrypt --batch <private_key.pem> [--ttl 秒] [--count N]
//!      从 stdin 逐行读 URL，每个 URL 生成 N 份密文
//!   resolve-tool all <url> [--out DIR] [--ttl 秒] [--count N]
//!      一键：生成密钥对 + 立即加密一个 URL（生成 N 份密文），同时输出密钥对文件与密文
//!
//! 说明：
//!   --ttl   有效期秒数，默认 172800（48h）；0 表示永不失效。
//!   --count 每个 URL 生成的密文份数（默认 1）。每份用不同的随机 Nonce/盐，密文不同，
//!           但解密后是同一个 URL，可分发到多个域名 / 多条 DNS TXT 记录。

use std::process::ExitCode;

fn print_usage() {
    eprintln!(
        "\
resolve-tool —— 动态解析服务端工具（Ed25519 签名 + AES-256-GCM）

用法：
  resolve-tool genkey [--out DIR]
      生成 Ed25519 密钥对，写 DIR/private_key.pem 与 DIR/public_key.b64（默认 .）

  resolve-tool encrypt <private_key.pem> <url> [--ttl 秒] [--count N]
      用私钥加密一个 URL，stdout 输出 base64 密文（单行，写入 DNS TXT）。
      --count N 让同一 URL 生成 N 份不同密文（不同随机 Nonce/盐），可分发到多个域名。

  resolve-tool encrypt --batch <private_key.pem> [--ttl 秒] [--count N]
      从 stdin 逐行读 URL，每个 URL 生成 N 份密文

  resolve-tool all <url> [--out DIR] [--ttl 秒] [--count N]
      一键：生成密钥对 + 立即加密一个 URL（N 份密文），同时输出密钥对文件与密文

参数：
  --ttl   有效期秒数（默认 172800=48h；0=永不失效）
  --count 每个 URL 生成的密文份数（默认 1）
  --out   输出目录（genkey / all 用，默认 .）
"
    );
}

fn parse_ttl(args: &[String]) -> Result<u64, String> {
    // 找 --ttl，返回其后的整数值；不存在则默认 48h。
    for (i, a) in args.iter().enumerate() {
        if a == "--ttl" {
            let v = args
                .get(i + 1)
                .ok_or_else(|| "--ttl 需要一个秒数（整数）".to_string())?;
            return v
                .parse::<u64>()
                .map_err(|_| format!("--ttl 必须是整数秒数，收到：{v}"));
        }
    }
    Ok(48 * 3600)
}

fn parse_count(args: &[String]) -> Result<u64, String> {
    // 找 --count，返回其后的正整数；不存在则默认 1。
    for (i, a) in args.iter().enumerate() {
        if a == "--count" {
            let v = args
                .get(i + 1)
                .ok_or_else(|| "--count 需要一个正整数".to_string())?;
            let n = v
                .parse::<u64>()
                .map_err(|_| format!("--count 必须是正整数，收到：{v}"))?;
            if n == 0 {
                return Err("--count 必须 >= 1".to_string());
            }
            return Ok(n);
        }
    }
    Ok(1)
}

/// 从 PEM（PKCS#8 纯私钥格式）解析 Ed25519 私钥。
///
/// 兼容两种来源：本工具 `genkey` 生成的纯私钥（48 字节 DER）、以及 Python
/// `cryptography` 导出的 `Ed25519PrivateKey`（同样 48 字节纯私钥 DER）。
/// 也兼容 ed25519-dalek 的 `to_pkcs8_pem` 输出的 RFC 8410 格式（83 字节，带内嵌公钥）。
///
/// 解析对空白【完全健壮】：不假设 PEM 的 base64 正文单独成行。做法是去掉 `-----BEGIN/END`
/// 头尾标记后，把剩余内容里的所有空白（换行、回车、空格、Tab）全部剔除，剩下的即为
/// base64 正文。这样即使私钥在多行传递时被折叠成一行（如 `-----BEGIN ... base64 ... -----END`），
/// 也能正确提取。
fn load_signing_key(path: &str) -> Result<ed25519_dalek::SigningKey, String> {
    let pem = std::fs::read_to_string(path).map_err(|e| format!("读取私钥失败：{e}"))?;

    // 去掉 PEM 头尾标记（可能被折叠在一行里，用字符串替换而非按行过滤）。
    let mut body = pem.replace("-----BEGIN PRIVATE KEY-----", "");
    body = body.replace("-----END PRIVATE KEY-----", "");
    // 兼容 ed25519-dalek 的 RFC 8410 标签（`to_pkcs8_pem` 用 "PRIVATE KEY"，但个别工具
    // 可能输出 "PRIVATE KEY" 变体；这里再兜底去掉常见的 "BEGIN/END" 标记）。
    body = body.replace("-----BEGIN", "").replace("-----END", "");
    body = body.replace("PRIVATE KEY", "");

    // 去掉所有空白字符，剩下的就是 base64 正文。
    let b64: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    let der = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &b64)
        .map_err(|e| format!("私钥 PEM base64 解码失败：{e}"))?;

    // Ed25519 私钥 seed（32 字节）位于 DER 末尾：
    //   - 纯私钥格式（48 字节）：`... 04 20 <seed>`，seed 是最后 32 字节。
    //   - RFC 8410 格式（83 字节）：`... 04 20 <seed> a1 23 03 21 00 <pub>`，seed 也是
    //     从内层 OCTET STRING 开始，但公钥跟在后面。统一做法：定位 `04 20`（内层
    //     OCTET STRING 长度 32）后紧跟的 32 字节即 seed。
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
    Ok(ed25519_dalek::SigningKey::from_bytes(&bytes))
}

/// 生成密钥对并写入 out_dir，返回（私钥路径，公钥路径，公钥 base64）。
fn generate_and_write_keypair(out_dir: &str) -> Result<(String, String, String), String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("创建目录 {out_dir} 失败：{e}"))?;
    let (priv_pem, pub_b64) = resolve_core::generate_keypair()?;

    let priv_path = std::path::Path::new(out_dir).join("private_key.pem");
    let pub_path = std::path::Path::new(out_dir).join("public_key.b64");
    std::fs::write(&priv_path, priv_pem.as_bytes()).map_err(|e| format!("写私钥失败：{e}"))?;
    std::fs::write(&pub_path, pub_b64.as_bytes()).map_err(|e| format!("写公钥失败：{e}"))?;
    Ok((
        priv_path.display().to_string(),
        pub_path.display().to_string(),
        pub_b64,
    ))
}

fn cmd_genkey(out_dir: &str) -> Result<(), String> {
    let (priv_path, pub_path, pub_b64) = generate_and_write_keypair(out_dir)?;

    println!("私钥（服务端持有，切勿公开）: {priv_path}");
    println!("公钥裸 base64（客户端 PUBLIC_KEY）: {pub_path}");
    println!();
    println!("公钥内容：");
    println!("{pub_b64}");
    Ok(())
}

/// 一键：生成密钥对 + 立即用新私钥加密一个 URL（N 份密文）。
/// args 形如：[all, <url>, (--out DIR), (--ttl N), (--count N)]
fn cmd_all(args: &[String]) -> Result<(), String> {
    let ttl = parse_ttl(args)?;
    let count = parse_count(args)?;

    // 解析 --out 与位置参数（URL 列表）。
    let mut out = String::from(".");
    let mut urls: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--ttl" || a == "--count" {
            i += 2;
            continue;
        } else if a == "--out" {
            match args.get(i + 1) {
                Some(v) => {
                    out = v.clone();
                    i += 2;
                }
                None => return Err("--out 需要一个目录路径".to_string()),
            }
        } else {
            urls.push(a.clone());
            i += 1;
        }
    }

    if urls.is_empty() {
        return Err("未提供要加密的 URL".to_string());
    }

    // 生成密钥对并写文件。
    let (priv_path, pub_path, pub_b64) = generate_and_write_keypair(&out)?;

    // 从刚写的私钥文件重新加载 SigningKey（复用同一套解析逻辑）。
    let signing_key = load_signing_key(&priv_path)?;

    // 逐个 URL、每个 URL 生成 count 份密文。
    let mut ciphers: Vec<String> = Vec::new();
    for url in &urls {
        for _ in 0..count {
            let cipher = resolve_core::sign_and_encrypt(&signing_key, url, ttl, None)?;
            ciphers.push(cipher);
        }
    }

    println!("========== 密钥对 ==========");
    println!("私钥: {priv_path}");
    println!("公钥: {pub_path}");
    println!();
    println!("公钥内容（填 build-custom.yml 的 public_key）：");
    println!("{pub_b64}");
    println!();
    println!(
        "========== TXT 密文（共 {} 条，均可用于 DNS TXT） ==========",
        ciphers.len()
    );
    let mut idx = 0;
    for url in &urls {
        for _ in 0..count {
            let cipher = &ciphers[idx];
            println!("# {url}  (ttl={ttl}s, 第 {} 份)", idx % count as usize + 1);
            println!("{cipher}");
            println!();
            idx += 1;
        }
    }
    Ok(())
}

fn cmd_encrypt(args: &[String]) -> Result<(), String> {
    // args 形如：[encrypt, <private_key.pem>, <url>, (--ttl N), (--count N)]
    //          或 [encrypt, --batch, <private_key.pem>, (--ttl N), (--count N)]
    let ttl = parse_ttl(args)?;
    let count = parse_count(args)?;

    let mut positional: Vec<&String> = Vec::new();
    let mut batch = false;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--ttl" || a == "--count" {
            i += 2; // 跳过选项及其值
            continue;
        } else if a == "--batch" {
            batch = true;
            i += 1;
            continue;
        } else {
            positional.push(a);
            i += 1;
        }
    }

    let key_path = positional
        .first()
        .ok_or_else(|| "缺少私钥路径参数".to_string())?
        .as_str();
    let signing_key = load_signing_key(key_path)?;

    let urls: Vec<String> = if batch {
        // 从 stdin 逐行读 URL。
        let mut input = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)
            .map_err(|e| format!("读取 stdin 失败：{e}"))?;
        input
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    } else {
        positional
            .iter()
            .skip(1)
            .map(|s| s.as_str().to_string())
            .collect()
    };

    if urls.is_empty() {
        return Err("未提供要加密的 URL".to_string());
    }

    for url in &urls {
        for n in 0..count {
            let cipher = resolve_core::sign_and_encrypt(&signing_key, url, ttl, None)?;
            if count > 1 {
                println!("# {url}  (ttl={ttl}s, 第 {} 份)", n + 1);
            } else {
                println!("# {url}  (ttl={ttl}s)");
            }
            println!("{cipher}");
            println!();
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        print_usage();
        return ExitCode::from(2);
    }

    let result = match args[0].as_str() {
        "genkey" => {
            // 解析 --out
            let mut out = String::from(".");
            let mut i = 1;
            while i < args.len() {
                if args[i] == "--out" {
                    match args.get(i + 1) {
                        Some(v) => {
                            out = v.clone();
                            i += 2;
                        }
                        None => {
                            print_usage();
                            return ExitCode::from(2);
                        }
                    }
                } else {
                    out = args[i].clone();
                    i += 1;
                }
            }
            cmd_genkey(&out)
        }
        "encrypt" => cmd_encrypt(&args[1..]),
        "all" => cmd_all(&args[1..]),
        _ => {
            print_usage();
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("错误：{e}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    /// 把内容写到系统临时目录下的唯一文件，返回路径（测试结束前由调用方清理）。
    fn write_temp_pem(content: &str, tag: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "resolve-tool-test-{}-{tag}.pem",
            std::process::id()
        ));
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn load_signing_key_pure_private_pem() {
        let (priv_pem, _pub_b64) = resolve_core::generate_keypair().unwrap();
        let path = write_temp_pem(&priv_pem, "pure");
        let key = load_signing_key(path.to_str().unwrap()).expect("纯私钥 PEM 应能解析");
        assert_eq!(key.to_bytes().len(), 32);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_signing_key_folded_single_line() {
        // 头尾标记与 base64 被折叠成一行（多行传输损坏的常见形态）也应能解析。
        let (priv_pem, _pub_b64) = resolve_core::generate_keypair().unwrap();
        let folded = priv_pem.replace('\n', "");
        let path = write_temp_pem(&folded, "folded");
        let key = load_signing_key(path.to_str().unwrap()).expect("折叠单行 PEM 应能解析");
        assert_eq!(key.to_bytes().len(), 32);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_signing_key_rfc8410_with_embedded_pubkey() {
        // RFC 8410 格式（83 字节内容，seed 后内嵌公钥）：解析应跳过内嵌公钥、取到 seed。
        let (_, pub_b64) = resolve_core::generate_keypair().unwrap();
        let pub_raw =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &pub_b64).unwrap();
        let seed = [7u8; 32];
        let mut der = Vec::new();
        der.extend_from_slice(&[0x30, 0x53]); // SEQUENCE len=83
        der.extend_from_slice(&[0x02, 0x01, 0x00]); // INTEGER version
        der.extend_from_slice(&[0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70]); // Ed25519 OID
        der.extend_from_slice(&[0x04, 0x22]); // OCTET STRING len=34
        der.extend_from_slice(&[0x04, 0x20]); // 内层 OCTET STRING len=32
        der.extend_from_slice(&seed);
        der.extend_from_slice(&[0xa1, 0x23, 0x03, 0x21, 0x00]); // [1] BIT STRING 32 字节
        der.extend_from_slice(&pub_raw);

        let b64 = base64::engine::general_purpose::STANDARD.encode(&der);
        let pem = format!("-----BEGIN PRIVATE KEY-----\n{b64}\n-----END PRIVATE KEY-----\n");
        let path = write_temp_pem(&pem, "rfc8410");
        let key = load_signing_key(path.to_str().unwrap()).expect("RFC8410 应能解析");
        assert_eq!(key.to_bytes(), seed);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_signing_key_invalid_returns_err() {
        let path = write_temp_pem("这不是 PEM", "garbage");
        assert!(load_signing_key(path.to_str().unwrap()).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn parse_ttl_default_and_custom() {
        assert_eq!(parse_ttl(&[]).unwrap(), 48 * 3600, "默认 48h");
        assert_eq!(
            parse_ttl(&["--ttl".to_string(), "60".to_string()]).unwrap(),
            60
        );
        assert!(
            parse_ttl(&["--ttl".to_string(), "abc".to_string()]).is_err(),
            "非整数应报错"
        );
        assert!(parse_ttl(&["--ttl".to_string()]).is_err(), "缺少数值应报错");
    }

    #[test]
    fn parse_count_default_and_custom() {
        assert_eq!(parse_count(&[]).unwrap(), 1, "默认 1 份");
        assert_eq!(
            parse_count(&["--count".to_string(), "3".to_string()]).unwrap(),
            3
        );
        assert!(
            parse_count(&["--count".to_string(), "0".to_string()]).is_err(),
            "0 不允许"
        );
        assert!(
            parse_count(&["--count".to_string(), "x".to_string()]).is_err(),
            "非整数应报错"
        );
    }
}
