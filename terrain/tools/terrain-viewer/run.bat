@echo off
setlocal
set "terrain_database=%~f1"
pushd "%~dp0..\.."
if errorlevel 1 goto failed
if not defined terrain_database set "terrain_database=%CD%\build\terrain-national-xyz-20261024\terrain.sqlite3"
if not exist "%terrain_database%" (
    echo SQLite file not found: "%terrain_database%"
    goto failed_popd
)
if not exist "build\terrain-viewer-target\release\terrain-viewer.exe" (
    where pwsh.exe >nul 2>&1
    if errorlevel 1 (
        echo PowerShell 7 is required for the first build. Please install it and retry.
        goto failed_popd
    )
    pwsh.exe -NoProfile -File "%~dp0build.ps1"
    if errorlevel 1 goto failed_popd
)
start "" "build\terrain-viewer-target\release\terrain-viewer.exe" "%terrain_database%"
if errorlevel 1 goto failed_popd
popd
exit /b 0

:failed_popd
popd
:failed
echo Failed to launch the terrain viewer.
pause
exit /b 1
