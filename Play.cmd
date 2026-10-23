@echo off
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0play.ps1" %*
set "game_exit=%errorlevel%"
if not "%game_exit%"=="0" pause
exit /b %game_exit%
