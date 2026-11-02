@echo off
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0start_national_shared.ps1" %*
exit /b %errorlevel%
