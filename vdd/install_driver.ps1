# Virtual Display Driver Installer
$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)

if (-not $isAdmin) {
    Write-Host "[!] Administrator privilege required. Launching UAC prompt..." -ForegroundColor Yellow
    Start-Process powershell.exe -ArgumentList "-NoProfile", "-ExecutionPolicy", "Bypass", "-File `"$PSScriptRoot\install_driver.ps1`"" -Verb RunAs
    exit
}

Write-Host "========================================================" -ForegroundColor Cyan
Write-Host " Virtual Display Driver (VDD) Installation" -ForegroundColor Cyan
Write-Host "========================================================" -ForegroundColor Cyan

# 1. Setup config
$targetDir = "C:\VirtualDisplayDriver"
if (-not (Test-Path $targetDir)) {
    New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
}

$srcXml = "$PSScriptRoot\control\Dependencies\vdd_settings.xml"
if (Test-Path $srcXml) {
    Copy-Item -Path $srcXml -Destination "$targetDir\vdd_settings.xml" -Force
    Write-Host "[*] Settings installed: $targetDir\vdd_settings.xml" -ForegroundColor Green
}

# 2. Install MttVDD driver
$devcon = "$PSScriptRoot\control\Dependencies\devcon.exe"
$inf = "$PSScriptRoot\control\SignedDrivers\x86\VDD\MttVDD.inf"

Write-Host "[*] Installing driver: $inf ..." -ForegroundColor Yellow
$proc = Start-Process -FilePath $devcon -ArgumentList "install `"$inf`" Root\MttVDD" -Wait -PassThru -NoNewWindow

if ($proc.ExitCode -eq 0) {
    Write-Host "[OK] Virtual Display Driver successfully installed!" -ForegroundColor Green
    
    $controlApp = "$PSScriptRoot\control\VDD Control.exe"
    if (Test-Path $controlApp) {
        Write-Host "[*] Launching VDD Control..." -ForegroundColor Cyan
        Start-Process -FilePath $controlApp
    }
} else {
    Write-Host "[!] devcon exit code: $($proc.ExitCode). Trying pnputil..." -ForegroundColor Yellow
    & pnputil /add-driver "$inf" /install
}

Write-Host "`nInstallation process completed. Press Enter to exit..." -ForegroundColor Gray
Read-Host
