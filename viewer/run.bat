@echo off
setlocal
set "VIEWER_EXE=%~dp0target\release\yamamuki-viewer.exe"
if exist "%VIEWER_EXE%" goto run

set "VIEWER_CARGO="
for %%I in (cargo.exe) do set "VIEWER_CARGO=%%~$PATH:I"
if not defined VIEWER_CARGO if defined CARGO_HOME if exist "%CARGO_HOME%\bin\cargo.exe" set "VIEWER_CARGO=%CARGO_HOME%\bin\cargo.exe"
if not defined VIEWER_CARGO if exist "%USERPROFILE%\.cargo\bin\cargo.exe" set "VIEWER_CARGO=%USERPROFILE%\.cargo\bin\cargo.exe"
if not defined VIEWER_CARGO goto no_cargo

if not defined CARGO_TARGET_DIR set "CARGO_TARGET_DIR=%~dp0target"
echo Building yamamuki-viewer...
"%VIEWER_CARGO%" build --locked --release --manifest-path "%~dp0Cargo.toml" --target-dir "%CARGO_TARGET_DIR%"
if errorlevel 1 goto build_failed
if exist "%VIEWER_EXE%" goto run
if not exist "%~dp0target\release\." mkdir "%~dp0target\release"
copy /y "%CARGO_TARGET_DIR%\release\yamamuki-viewer.exe" "%VIEWER_EXE%" >nul
if errorlevel 1 goto build_failed

:run
start "" "%VIEWER_EXE%" %*
exit /b %errorlevel%

:no_cargo
echo Cargo was not found. Install Rust stable or put cargo.exe on PATH. 1>&2
echo See "%~dp0README.md". 1>&2
exit /b 1

:build_failed
echo Could not build yamamuki-viewer. See the error above. 1>&2
exit /b 1
