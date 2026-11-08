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
set "viewer_dir=build\terrain-shared-viewer-target\release"
set "viewer_binary="
for /f "delims=" %%F in ('dir /b /a:-d /o:-d /t:w "%viewer_dir%\terrain-shared-viewer*.exe" 2^>nul') do (
    if not defined viewer_binary if /i "%%F"=="terrain-shared-viewer.exe" set "viewer_binary=%viewer_dir%\%%F"
    if not defined viewer_binary if /i "%%F"=="terrain-shared-viewer-nearest.exe" set "viewer_binary=%viewer_dir%\%%F"
)
if not defined viewer_binary (
    pwsh.exe -NoProfile -File "%~dp0build.ps1" -Shared
    if errorlevel 1 goto failed_popd
    set "viewer_binary=%viewer_dir%\terrain-shared-viewer.exe"
)
if defined landcover_database (
    if not exist "%landcover_database%" (
        echo Landcover SQLite file not found: "%landcover_database%"
        goto failed_popd
    )
    start "" "%viewer_binary%" --database "%terrain_database%" --landcover "%landcover_database%"
) else (
    start "" "%viewer_binary%" --database "%terrain_database%"
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
