@echo off
setlocal
set "terrain_database=%~f1"
set "landcover_database=%~f2"
pushd "%~dp0..\.."
if errorlevel 1 goto failed
if not defined terrain_database if exist "%~dp0terrain-shared.sqlite3" set "terrain_database=%~dp0terrain-shared.sqlite3"
if not defined terrain_database set "terrain_database=%CD%\build\national-shared-z6-z13-v1\terrain-shared.sqlite3"
if not exist "%terrain_database%" (
    echo SQLite file not found: "%terrain_database%"
    goto failed_popd
)
if not exist "build\terrain-viewer-target\release\terrain-shared-viewer.exe" (
    pwsh.exe -NoProfile -File "%~dp0build.ps1"
    if errorlevel 1 goto failed_popd
)
if defined landcover_database (
    if not exist "%landcover_database%" (
        echo Landcover SQLite file not found: "%landcover_database%"
        goto failed_popd
    )
    start "" "build\terrain-viewer-target\release\terrain-shared-viewer.exe" --database "%terrain_database%" --landcover "%landcover_database%"
) else (
    start "" "build\terrain-viewer-target\release\terrain-shared-viewer.exe" --database "%terrain_database%"
)
if errorlevel 1 goto failed_popd
popd
exit /b 0
:failed_popd
popd
:failed
echo Failed to open the national terrain SQLite.
pause
exit /b 1
