@echo off
setlocal

set "PROJECT_DIR=%~dp0"
if "%PROJECT_DIR:~-1%"=="\" set "PROJECT_DIR=%PROJECT_DIR:~0,-1%"
set "NODE_BIN=C:\Users\ahmed\.cache\codex-runtimes\codex-primary-runtime\dependencies\node\bin"
set "PNPM=C:\Users\ahmed\.cache\codex-runtimes\codex-primary-runtime\dependencies\bin\fallback\pnpm.cmd"
set "PATH=%NODE_BIN%;%PATH%"

cd /d "%PROJECT_DIR%"

if not exist "%PROJECT_DIR%\node_modules\.bin\tauri.cmd" (
  echo Installing UniLoader dependencies...
  call "%PNPM%" install
)

set "RELEASE_EXE=%PROJECT_DIR%\src-tauri\target\release\uniloader.exe"
if exist "%RELEASE_EXE%" (
  start "" "%RELEASE_EXE%"
  exit /b 0
)

where.exe cargo >nul 2>nul
if errorlevel 1 (
  echo UniLoader has moved to Tauri and needs Rust/Cargo to run from source.
  echo Install Rust from https://www.rust-lang.org/tools/install.
  echo Tauri on Windows also needs Microsoft C++ Build Tools with the Windows SDK.
  echo Build Tools: https://aka.ms/vs/17/release/vs_BuildTools.exe
  echo.
  pause
  exit /b 1
)

call "%PNPM%" run dev
