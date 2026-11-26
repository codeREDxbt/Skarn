@echo off
setlocal EnableExtensions
rem Skarn - start the desktop app that is already built in this repo.
rem Kept as a short entry point next to launch-skarn.bat; both do the same
rem thing (repo-relative, honors CARGO_TARGET_DIR, never builds).
rem
rem   start.bat
rem
rem The binary is target\release\skarn-desktop.exe, or
rem %CARGO_TARGET_DIR%\release\skarn-desktop.exe when CARGO_TARGET_DIR is set.

cd /d "%~dp0"
if errorlevel 1 (
    echo start: could not enter the repository root 1>&2
    exit /b 1
)

set "TARGET_DIR=target"
if defined CARGO_TARGET_DIR set "TARGET_DIR=%CARGO_TARGET_DIR%"

set "APP_EXE=%TARGET_DIR%\release\skarn-desktop.exe"
if not exist "%APP_EXE%" (
    echo start: skarn-desktop.exe is not built yet 1>&2
    echo start: expected "%CD%\%APP_EXE%" 1>&2
    echo start: 1>&2
    echo start: build it first: 1>&2
    echo start:   npm --prefix apps\skarn-desktop\frontend run build 1>&2
    echo start:   cargo build --release -p skarn-desktop 1>&2
    exit /b 1
)

start "" "%APP_EXE%"
if errorlevel 1 (
    echo start: could not start "%APP_EXE%" 1>&2
    exit /b 1
)
exit /b 0