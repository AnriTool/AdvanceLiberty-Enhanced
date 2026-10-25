@echo off
setlocal
set "PSModulePath="
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0tools\collect-native.ps1" %*
set "native_exit=%errorlevel%"
if not "%native_exit%"=="0" pause
exit /b %native_exit%
