@echo off
setlocal EnableExtensions
rem Skarn - start the desktop app that is already built in this repo.
rem
rem   launch-skarn.bat
rem
rem Repo-relative: no absolute build paths. The binary is
rem target\release\skarn-desktop.exe, or %CARGO_TARGET_DIR%\release\ when
rem CARGO_TARGET_DIR is set (cargo resolves a relative CARGO_TARGET_DIR
rem against the directory it runs in, which is this repo root).
rem
rem If the binary is missing, print the two build commands and exit 1 - this
rem script never compiles anything.

cd /d "%~dp0"
if errorlevel 1 (
    echo launch-skarn: could not enter the repository root 1>&2
    exit /b 1
)

set "TARGET_DIR=target"
if defined CARGO_TARGET_DIR set "TARGET_DIR=%CARGO_TARGET_DIR%"

set "APP_EXE=%TARGET_DIR%\release\skarn-desktop.exe"
if not exist "%APP_EXE%" (
    echo launch-skarn: skarn-desktop.exe is not built yet 1>&2
    echo launch-skarn: expected "%CD%\%APP_EXE%" 1>&2
    echo launch-skarn: 1>&2
    echo launch-skarn: build it first: 1>&2
    echo launch-skarn:   npm --prefix apps\skarn-desktop\frontend run build 1>&2
    echo launch-skarn:   cargo build --release -p skarn-desktop 1>&2
    exit /b 1
)

start "" "%APP_EXE%"
if errorlevel 1 (
    echo launch-skarn: could not start "%APP_EXE%" 1>&2
    exit /b 1
)
exit /b 0