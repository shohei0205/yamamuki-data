@echo off
powershell.exe -NoProfile -File "%~dp0prepare_national_natural.ps1" %*
exit /b %errorlevel%
