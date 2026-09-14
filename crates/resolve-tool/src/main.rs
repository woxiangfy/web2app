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

/// 一次解析出的命令行参数：所有可选 flag 的取值 + 剩余位置参数。
struct ParsedArgs {
    ttl: u64,
    count: u64,
    out: String,
    batch: bool,
    positional: Vec<String>,
}

/// 统一解析 `--ttl / --count / --out / --batch` 及剩余位置参数（不引 clap，保持零重依赖）。
///
/// 之前 `cmd_encrypt` / `cmd_all` / `genkey` 各自手写一份 while 循环解析，重复且易漏新 flag；
/// 这里归并为一处，三个子命令共用同一套解析规则，便于后续扩展新 flag。
fn parse_args(args: &[String]) -> Result<ParsedArgs, String> {
    let mut ttl: Option<u64> = None;
    let mut count: Option<u64> = None;
    let mut out: Option<String> = None;
    let mut batch = false;
    let mut positional: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--ttl" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--ttl 需要一个秒数（整数）".to_string())?;
                ttl = Some(
                    v.parse::<u64>()
                        .map_err(|_| format!("--ttl 必须是整数秒数，收到：{v}"))?,
                );
                i += 2;
            }
            "--count" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--count 需要一个正整数".to_string())?;
                let n = v
                    .parse::<u64>()
                    .map_err(|_| format!("--count 必须是正整数，收到：{v}"))?;
                if n == 0 {
                    return Err("--count 必须 >= 1".to_string());
                }
                count = Some(n);
                i += 2;
            }
            "--out" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--out 需要一个目录路径".to_string())?;
                out = Some(v.clone());
                i += 2;
            }
            "--batch" => {
                batch = true;
                i += 1;
            }
            _ => {
                positional.push(a.to_string());
                i += 1;
            }
        }
    }

    Ok(ParsedArgs {
        ttl: ttl.unwrap_or(48 * 3600),
        count: count.unwrap_or(1),
        out: out.unwrap_or_else(|| ".".to_string()),
        batch,
        positional,
    })
}

/// 从 PEM（PKCS#8 纯私钥格式）解析 Ed25519 私钥。
///
/// 兼容两种来源：本工具 `genkey` 生成的纯私钥（48 字节 DER）、以及 Python
/// `cryptography` 导出的 `Ed25519PrivateKey`（同样 48 字节纯私钥 DER）。
/// 也兼容 ed25519-dalek 的 `to_pkcs8_pem` 输出的 RFC 8410 格式（83 字节，带内嵌公钥）。
///
/// PEM → SigningKey 的解析已下沉到 `resolve-core::parse_signing_key_pem`，与客户端测试
/// 共用同一套健壮实现（对空白完全健壮，即使私钥被折叠成单行也能正确提取）。
fn load_signing_key(path: &str) -> Result<ed25519_dalek::SigningKey, String> {
    let pem = std::fs::read_to_string(path).map_err(|e| format!("读取私钥失败：{e}"))?;
    resolve_core::parse_signing_key_pem(&pem)
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
    let a = parse_args(args)?;

    if a.positional.is_empty() {
        return Err("未提供要加密的 URL".to_string());
    }

    // 生成密钥对并写文件。
    let (priv_path, pub_path, pub_b64) = generate_and_write_keypair(&a.out)?;

    // 从刚写的私钥文件重新加载 SigningKey（复用同一套解析逻辑）。
    let signing_key = load_signing_key(&priv_path)?;

    // 逐个 URL、每个 URL 生成 count 份密文。
    let mut ciphers: Vec<String> = Vec::new();
    for url in &a.positional {
        for _ in 0..a.count {
            let cipher = resolve_core::sign_and_encrypt(&signing_key, url, a.ttl, None)?;
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
    for url in &a.positional {
        for _ in 0..a.count {
            let cipher = &ciphers[idx];
            println!(
                "# {url}  (ttl={}s, 第 {} 份)",
                a.ttl,
                idx % a.count as usize + 1
            );
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
    let a = parse_args(args)?;

    let key_path = a
        .positional
        .first()
        .ok_or_else(|| "缺少私钥路径参数".to_string())?;
    let signing_key = load_signing_key(key_path)?;

    let urls: Vec<String> = if a.batch {
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
        a.positional
            .iter()
            .skip(1)
            .map(|s| s.as_str().to_string())
            .collect()
    };

    if urls.is_empty() {
        return Err("未提供要加密的 URL".to_string());
    }

    for url in &urls {
        for n in 0..a.count {
            let cipher = resolve_core::sign_and_encrypt(&signing_key, url, a.ttl, None)?;
            if a.count > 1 {
                println!("# {url}  (ttl={}s, 第 {} 份)", a.ttl, n + 1);
            } else {
                println!("# {url}  (ttl={}s)", a.ttl);
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
        "genkey" => parse_args(&args[1..]).and_then(|a| {
            // 兼容历史用法：genkey 既支持 `--out DIR`，也支持 `genkey DIR`（裸位置参数当作目录）。
            let out = a.positional.last().cloned().unwrap_or(a.out);
            cmd_genkey(&out)
        }),
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
    fn parse_args_defaults() {
        let a = parse_args(&[]).unwrap();
        assert_eq!(a.ttl, 48 * 3600, "默认 48h");
        assert_eq!(a.count, 1, "默认 1 份");
        assert_eq!(a.out, ".", "默认输出目录 .");
        assert!(!a.batch);
        assert!(a.positional.is_empty());
    }

    #[test]
    fn parse_args_all_flags_and_positional() {
        let args: Vec<String> = [
            "--ttl", "60", "--count", "3", "--out", "/tmp", "--batch", "key.pem", "url1", "url2",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let a = parse_args(&args).unwrap();
        assert_eq!(a.ttl, 60);
        assert_eq!(a.count, 3);
        assert_eq!(a.out, "/tmp");
        assert!(a.batch);
        assert_eq!(a.positional, vec!["key.pem", "url1", "url2"]);
    }

    #[test]
    fn parse_args_errors() {
        assert!(
            parse_args(&["--ttl".to_string()]).is_err(),
            "--ttl 缺少数值应报错"
        );
        assert!(
            parse_args(&["--ttl".to_string(), "abc".to_string()]).is_err(),
            "--ttl 非整数应报错"
        );
        assert!(
            parse_args(&["--count".to_string(), "0".to_string()]).is_err(),
            "--count 0 不允许"
        );
        assert!(
            parse_args(&["--count".to_string(), "x".to_string()]).is_err(),
            "--count 非整数应报错"
        );
        assert!(
            parse_args(&["--out".to_string()]).is_err(),
            "--out 缺少数值应报错"
        );
    }
}
