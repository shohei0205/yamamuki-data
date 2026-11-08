@echo off
setlocal
if exist "%~dp0target\release\yamamuki-viewer.exe" (
    "%~dp0target\release\yamamuki-viewer.exe" %*
) else (
    cargo run --locked --release --manifest-path "%~dp0Cargo.toml" -- %*
)
exit /b %errorlevel%
