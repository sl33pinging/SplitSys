@echo off
rem SplitSys booth cold start: double-click, or start_demo.bat serial:COM5
rem Wraps start_demo.ps1 (see it for options). Owner: host-dashboard.
set T=%1
if "%T%"=="" set T=serial:auto
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0start_demo.ps1" -Transport %T%
if errorlevel 1 pause
