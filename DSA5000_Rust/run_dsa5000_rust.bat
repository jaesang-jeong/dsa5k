@echo off
title Scanivalve DSA5000 SSEP Native Desktop Studio (v2.01)
cd /d "%~dp0"
echo ===============================================================
echo   Scanivalve DSA5000 SSEP Studio (Native Desktop v2.01)
echo   Standalone GUI Executable (Zero Web / Zero Browser Needed)
echo ===============================================================
echo.
echo Select Mode:
echo   [1] Native Desktop GUI + Multi-Module SSEP Simulator (Default)
echo   [2] Native Desktop GUI + Physical DSA5000 SSEP Controller
echo   [3] Headless / CLI Console Dashboard (Terminal Mode)
echo   [4] Automated SSEP Integration Self-Test Suite
echo.
set /p mode="Enter choice [1, 2, 3, or 4] (Default: 1): "

if "%mode%"=="" set mode=1

if "%mode%"=="1" (
    echo Starting SSEP Native Desktop GUI with Internal Simulator...
    if exist "DSA5000_Studio.exe" (
        start "" "DSA5000_Studio.exe"
    ) else if exist "target\release\DSA5000_Rust.exe" (
        start "" "target\release\DSA5000_Rust.exe"
    ) else (
        cargo run --release
    )
) else if "%mode%"=="2" (
    set /p ip="Enter DSA5000 Controller IP [Default: 192.168.1.100]: "
    if "%ip%"=="" set ip=192.168.1.100
    echo Starting SSEP Native Desktop GUI for %ip%...
    if exist "DSA5000_Studio.exe" (
        start "" "DSA5000_Studio.exe"
    ) else (
        cargo run --release
    )
) else if "%mode%"=="3" (
    echo Starting SSEP CLI Console Dashboard...
    if exist "target\release\DSA5000_Rust.exe" (
        target\release\DSA5000_Rust.exe --cli --simulate
    ) else (
        cargo run --release -- --cli --simulate
    )
) else if "%mode%"=="4" (
    echo Running Automated SSEP Self-Test Suite...
    if exist "target\release\DSA5000_Rust.exe" (
        target\release\DSA5000_Rust.exe --test
    ) else (
        cargo run --release -- --test
    )
) else (
    echo Starting SSEP Native Desktop GUI...
    if exist "DSA5000_Studio.exe" (
        start "" "DSA5000_Studio.exe"
    ) else (
        cargo run --release
    )
)
