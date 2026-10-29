@echo off
powershell.exe -NoProfile -File "%~dp0start_national_natural.ps1" %*
exit /b %errorlevel%
