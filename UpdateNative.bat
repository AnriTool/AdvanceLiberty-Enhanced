@echo off
setlocal
rem Do not inherit PowerShell 7 modules into Windows PowerShell 5.1.
set "PSModulePath=%ProgramFiles%\WindowsPowerShell\Modules;%SystemRoot%\System32\WindowsPowerShell\v1.0\Modules"
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\windows\update-native.ps1" %*
set "task_exit=%errorlevel%"
if not "%task_exit%"=="0" pause
exit /b %task_exit%
