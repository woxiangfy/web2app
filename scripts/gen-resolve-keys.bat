@echo off
rem ===========================================================================
rem Double-click helper: generate the dynamic-resolve KEY MATERIAL and print it
rem to the console ONLY --
rem   (1) a random symmetric Secret, and
rem   (2) an Ed25519 keypair (private PEM + public base64).
rem
rem This script writes NO files: nothing is created under deploy_keys\, no temp
rem files are left behind, and existing files (if any) are never read or reused.
rem Copy every value you need from the window before closing it; closing the
rem window discards the material forever, and running this again produces a
rem brand-new identity (new Secret + new keypair).
rem
rem Printed sections:
rem   RESOLVE_SECRET   32 random bytes encoded as standard base64 (44 chars;
rem                    same 256-bit entropy as the previous 64-char hex form --
rem                    the value is consumed as raw string bytes by HKDF, never
rem                    hex/base64-decoded, so any opaque string works as long as
rem                    the client build embeds the SAME one)
rem   private_key.pem  Ed25519 private key (server-side only, keep it secret)
rem   public_key       32-byte public key, base64 (client config: "public_key")
rem
rem Next step is separate and NOT done here: to turn a URL into TXT ciphertext,
rem encrypt accepts the private PEM inline as its first argument -- no file
rem needed. Join the printed PEM's three lines into one (header + body +
rem footer, drop the '#' comment line) and quote it. Pass the Secret printed
rem above through --secret so it matches the client build:
rem   cd webview-app
rem   cargo run -p resolve-tool -- encrypt "-----BEGIN PRIVATE KEY-----PEM-BODY-HERE-----END PRIVATE KEY-----" "https://user:pass@host" --ttl-days 0 --secret "paste-secret-here"
rem (A PEM file path still works as the first argument if you prefer files.)
rem
rem IMPORTANT - the client app still embeds its Secret at COMPILE TIME
rem (resolve-core option_env RESOLVE_SECRET); --secret only tells THIS encrypt
rem invocation which Secret to derive the AES key from. The two values MUST be
rem identical or client decryption fails. The old shell-env way still works
rem too (PowerShell: $env:RESOLVE_SECRET='paste-secret-here'; cmd:
rem set RESOLVE_SECRET=paste-secret-here) - with env set, just omit --secret.
rem With neither set, both sides silently use the built-in default secret, so
rem only default-secret ciphertexts can be decrypted by a default-secret app.
rem
rem Robustness notes (do not "simplify" these away):
rem   * cd /d into webview-app first: cargo needs the workspace root as its cwd.
rem     Explorer starts a double-clicked bat in the bat's own folder, so we
rem     cannot rely on the caller's cwd. (Skipping this is why a naive script
rem     "runs" but produces nothing: cargo aborts with "could not find Cargo.toml".)
rem   * pause at the end keeps the window open on double-click; with stdin
rem     redirected (CI / pipe) pause returns immediately and does not block.
rem   * bat-level text stays ASCII/English so the cmd parser is unaffected by
rem     the code-page switch; chcp 65001 is only for CJK output from cargo.
rem ===========================================================================

chcp 65001 >nul 2>&1
cd /d "%~dp0..\webview-app"

set "RC=1"

where cargo >nul 2>&1
if errorlevel 1 (
  echo [gen-resolve-keys] cargo was not found on PATH. Install the Rust toolchain first.
  goto :end
)
where node >nul 2>&1
if errorlevel 1 (
  echo [gen-resolve-keys] Node.js was not found on PATH. Install Node.js 18+ first.
  goto :end
)

echo [gen-resolve-keys] Generating key material; NOTHING is saved to disk.
echo.

rem --- 1) Secret: 32 random bytes as base64 (44 chars), printed directly -----
echo ======================= RESOLVE_SECRET =======================
node -e "process.stdout.write(require('crypto').randomBytes(32).toString('base64'))"
echo.
echo ==============================================================
echo.

rem --- 2) Ed25519 keypair: --stdout prints both keys and writes no files -----
echo [gen-resolve-keys] Generating an Ed25519 keypair ...
cargo run -p resolve-tool -- genkey --stdout
if errorlevel 1 goto :end

echo.
echo Copy the three values above from THIS window now - they were not saved.
echo.
echo Next step (separate, NOT done by this script) - turn a URL into TXT ciphertext:
echo   encrypt takes the private PEM inline (no file needed). Join the 3 PEM lines
echo   (BEGIN + body + END, no '#' line) into one quoted argument, and pass the
echo   Secret above via --secret so it matches the client build:
echo   cd webview-app
echo   cargo run -p resolve-tool -- encrypt "-----BEGIN PRIVATE KEY-----PEM-BODY-HERE-----END PRIVATE KEY-----" "https://user:pass@host" --ttl-days 0 --secret "paste-secret-here"
echo   (or keep using a PEM file path as the first argument)
echo.
echo   The client app still embeds its Secret at COMPILE TIME; --secret is the
echo   server-side, per-call way to supply the SAME value (any mismatch means
echo   client decryption fails). The shell-env way still works too:
echo     cmd:         set RESOLVE_SECRET=paste-secret-here
echo     PowerShell:  $env:RESOLVE_SECRET='paste-secret-here'
echo   With neither --secret nor env set, both sides use the built-in default.
set "RC=0"

:end
echo.
if not "%RC%"=="0" echo [gen-resolve-keys] FAILED ^(exit %RC%^). See the messages above.
pause
exit /b %RC%
