@echo off
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0prepare_national_shared.ps1" %*
exit /b %errorlevel%
