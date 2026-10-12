@echo off
rem ===========================================================================
rem Double-click launcher for scripts/init-env.mjs (Node >= 18, no deps).
rem
rem Why this file exists: double-clicking a .mjs opens it in an editor instead
rem of running it. This wrapper lets you just double-click to initialize both
rem shells (cargo fetch + pnpm install).
rem
rem Robustness notes (do not "simplify" these away):
rem   * cd /d to the repo root first: Explorer usually starts the bat in its
rem     own folder, but other launchers do not. Never rely on the caller's cwd.
rem   * Reference init-env.mjs via %~dp0 (this file's own absolute directory).
rem   * This bat stays pure ASCII; chcp 65001 is only here so the UTF-8 CJK
rem     output printed by node renders correctly. All bat-level text is English
rem     so cmd's parser is unaffected by the code-page switch.
rem   * pause at the end keeps the window open on double-click. Under a pipe /
rem     CI (no console) pause returns immediately, so it does not block scripts.
rem
rem Optional args are passed through, e.g. from a terminal:
rem   scripts\init-env.bat --check
rem   scripts\init-env.bat --check --webview
rem ===========================================================================

chcp 65001 >nul 2>&1
cd /d "%~dp0.."

rem RC carries init-env.mjs's exit code past pause: without the final
rem `exit /b %RC%`, callers would see pause's exit code (always 0).
set RC=0

where node >nul 2>&1
if errorlevel 1 (
  echo [init-env] Node.js was not found on PATH. Please install Node.js 18 or newer:
  echo            https://nodejs.org/
  set RC=1
  goto :end
)

node "%~dp0init-env.mjs" %*
set RC=%errorlevel%
if not "%RC%"=="0" echo [init-env] Exit code %RC% - the failing command's output tail and the full log file path were printed above by init-env.mjs.

:end
echo.
pause
exit /b %RC%
