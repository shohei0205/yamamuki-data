@echo off
setlocal

set "VIEWER_CARGO="
for %%I in (cargo.exe) do set "VIEWER_CARGO=%%~$PATH:I"
if not defined VIEWER_CARGO if defined CARGO_HOME if exist "%CARGO_HOME%\bin\cargo.exe" set "VIEWER_CARGO=%CARGO_HOME%\bin\cargo.exe"
if not defined VIEWER_CARGO if exist "%USERPROFILE%\.cargo\bin\cargo.exe" set "VIEWER_CARGO=%USERPROFILE%\.cargo\bin\cargo.exe"
if not defined VIEWER_CARGO goto no_cargo

if not defined CARGO_TARGET_DIR set "CARGO_TARGET_DIR=%~dp0target"
echo Building yamamuki-viewer...
"%VIEWER_CARGO%" build --locked --release --manifest-path "%~dp0Cargo.toml" --target-dir "%CARGO_TARGET_DIR%"
if errorlevel 1 goto build_failed

set "VIEWER_EXE=%CARGO_TARGET_DIR%\release\yamamuki-viewer.exe"
if not exist "%VIEWER_EXE%" goto missing_exe
start "" "%VIEWER_EXE%" %*
exit /b %errorlevel%

:no_cargo
echo Cargo was not found. Install Rust stable or put cargo.exe on PATH. 1>&2
echo See "%~dp0README.md". 1>&2
exit /b 1

:missing_exe
echo Built executable was not found at "%VIEWER_EXE%". 1>&2
exit /b 1

:build_failed
echo Could not build yamamuki-viewer. See the error above. 1>&2
exit /b 1
