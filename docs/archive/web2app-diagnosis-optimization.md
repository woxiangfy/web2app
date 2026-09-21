# web2app 全面诊断与优化方案

## Context（背景）

用户要求对 `c:\dev\rust\web-app\webview-app`（Tauri 2 桌面/移动端 WebView 壳）做一次全面诊断与优化：补测试、稳架构、提性能，**不破坏现有功能**。

已建立的基线（诊断前）：
- `cargo test --all`：24 个测试通过，3 个网络 e2e 测试标记 `#[ignore]`
- `cargo clippy --all-targets`：零告警
- git 工作树干净

已完成全部 13 个 Rust 源文件的通读。诊断结论与优化点如下。

## 诊断发现

### A. 真实健壮性缺陷（须修）
1. **`src/proxy.rs` `rewrite_cookie_domain` 可能 panic**：`p.len() >= 7 && p[..7].eq_ignore_ascii_case("domain=")` 用字节索引切片 `&str`，当 cookie 属性含多字节 UTF-8 且长度 ≥7 时（如 `中文文`，字节 7 落在字符中间）会触发切片边界 panic。上游返回不规范的非 ASCII cookie 值即可触发 → 该响应连接被重置。
2. **`src/proxy.rs` HTTP 转发全量内存缓冲**：请求体 `body.collect()`、响应体 `proxied_resp.bytes()` 均整体载入内存。大文件上传/下载（WebDAV、附件库等）内存峰值 = 文件大小，存在 OOM 风险。→ 改流式（用户已确认）。

### B. 测试缺口（用户偏好：每个文件/函数都要测试）
无测试的纯逻辑函数：
- `config.rs`：`sanitize_dir_name`、`str_eq`、`build_proxy_args`（cfg windows）
- `settings.rs`：`hotkey_to_shortcut`（cfg not android）、`Hotkey` serde 往返
- `bootstrap.rs`：`strip_userinfo`
- `window_state.rs`：`State` serde 往返
- `proxy.rs`：`axum_msg_to_tungstenite` / `tungstenite_msg_to_axum` 消息转换；`rewrite_cookie_domain` 的 UTF-8 边界用例；HTTP 转发缺**集成测试**（可起真实 mock 上游验证鉴权注入 + cookie 重写）
- `crates/resolve-tool/src/main.rs`：`load_signing_key`（纯私钥 / RFC8410 / 折叠单行）、`parse_ttl` / `parse_count`

### C. 性能优化点
1. `Cargo.toml` release profile 用 `opt-level = "z"`（体积优先）→ 改为 `"3"`（用户已确认），保留 `lto` / `codegen-units=1` / `strip`。
2. `settings.rs` `save_settings`：在持有 `SettingsState` 锁期间执行文件持久化 → 缩短持锁时间。

### D. 已确认无需改动
- 模块分层清晰（resolve-core 纯加密 / proxy 零平台依赖 / 桌面专属模块 cfg 隔离），CI 已含 Ubuntu + Windows + Android 三平台检查。
- `resolve-core`（13 测试）、`loading.rs`（3 测试）覆盖良好。

## 实施步骤

### 第 1 步：修复 cookie 重写 panic（`src/proxy.rs`）
- 第 396 行 `if p.len() >= 7 && p[..7].eq_ignore_ascii_case("domain=")` →
  `if p.get(..7).is_some_and(|s| s.eq_ignore_ascii_case("domain="))`
- 同文件 tests 模块新增：多字节 UTF-8 cookie 属性不 panic、且非 Domain 属性原样保留。

### 第 2 步：代理 HTTP 转发改流式（`src/proxy.rs`）
依赖版本已核实：axum 0.8.9（`Body::from_stream`）、reqwest 0.12.28（`Body::wrap_stream` / `Body::sized`）、http-body-util 0.1.5（`into_data_stream`）均在。

- **请求**：不再 `body.collect()`。`body.into_data_stream()`（`http_body_util::BodyExt`），错误映射为 `std::io::Error` 后：
  - 若客户端带可解析的 `Content-Length` → `reqwest::Body::sized(stream, len)`（reqwest 自动设置长度）
  - 否则 → `reqwest::Body::wrap_stream(stream)`（chunked）
  - `Content-Length` 头仍不逐条透传（现状如此，reqwest 按 body 实际处理）
- **响应**：不再 `proxied_resp.bytes()`。`Body::from_stream(proxied_resp.bytes_stream())`；仍跳过 `Content-Length`（与现状一致，避免流中断时客户端挂起）。
- **保持不变**：header 透传/跳过头、`Host`/`X-Forwarded-*`、`Authorization` 强制注入、Set-Cookie Domain 重写、WebSocket 分支全部不动。
- **新增集成测试**（`#[tokio::test]`，tokio 已带 rt-multi-thread+macros）：
  1. mock 上游（axum Router）校验：收到的请求带 `Authorization: Basic ...`；返回 `Set-Cookie: session=x; Domain=real.example.com` 与一个较大 body（如 1MB 随机字节）。
  2. 调 `start_proxy` 启动真实代理 → reqwest 客户端访问本地代理 URL。
  3. 断言：上游收到正确 Basic 头；响应 cookie 的 Domain 已被重写为 127.0.0.1；1MB 响应完整到达（验证流式无截断）。

### 第 3 步：补齐单元测试
按文件追加 `#[cfg(test)] mod tests`（均为纯函数/无 GUI 依赖，能随 `cargo test --all` 在 CI 全平台运行）：
- `config.rs`：`sanitize_dir_name`（非法字符替换、连续空白压缩、空串回落 Web2App）、`str_eq`；`build_proxy_args` 加 `#[cfg(windows)]`（本机为 Windows 可直接跑，CI 的 windows 作业覆盖）
- `settings.rs`：`Hotkey` JSON 往返；`hotkey_to_shortcut` 映射与无效键返回 None（加 `#[cfg(not(target_os = "android"))]`）
- `bootstrap.rs`：`strip_userinfo` 去掉 userinfo 保留其余
- `window_state.rs`：`State` serde 往返（含 maximized 字段）
- `proxy.rs`：`axum_msg_to_tungstenite` / `tungstenite_msg_to_axum` 各消息类型（Text/Binary/Ping/Pong/Close）往返
- `crates/resolve-tool/src/main.rs`：`load_signing_key` 三种格式（48B 纯私钥 / RFC8410 83B / 头尾折叠单行）+ 无效输入报错；`parse_ttl`/`parse_count` 默认值与非法参数。临时文件用 `std::env::temp_dir()` + 唯一名，测试后清理。

### 第 4 步：性能与锁优化
- `Cargo.toml` `[profile.release]`：`opt-level = "z"` → `"3"`（其余保留）。
- `settings.rs` `save_settings`：重构为「锁内克隆 old → 释放锁 → 执行快捷键/托盘副作用与持久化 → 重新加锁写入 new_settings」，缩短持锁时间；失败语义与现状一致（持久化失败则不更新内存状态）。

### 第 5 步（可选）：CI 增强
- `ci.yml` 的 `check-windows` 作业增加一步 `cargo test`（Windows 目标跑单测），让新增的 `#[cfg(windows)]` 测试（build_proxy_args、cookie 相关）进入 CI 覆盖。

## 验证

本机为 Windows，可覆盖桌面端全部路径：
1. `cargo fmt --all -- --check`（格式）
2. `cargo clippy --all-targets -- -D warnings`（零告警）
3. `cargo test --all`（新旧测试全绿，新增约 20+ 用例）
4. `cargo check --all-targets`；再以 `TABS_ENABLED=true` 环境变量重复一次，验证多页签 Windows 路径可编译
5. `cargo build --release` 确认发布构建通过

不改动：所有运行时行为（设置/托盘/快捷键/多页签/动态解析）保持不变；代理语义（鉴权注入、Host 透传、cookie 重写）由集成测试锁定。
