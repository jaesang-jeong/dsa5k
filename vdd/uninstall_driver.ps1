$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)

if (-not $isAdmin) {
    Write-Host "[!] Administrator privilege required. Launching UAC prompt..." -ForegroundColor Yellow
    Start-Process powershell.exe -ArgumentList "-NoProfile", "-ExecutionPolicy", "Bypass", "-File `"$PSScriptRoot\uninstall_driver.ps1`"" -Verb RunAs
    exit
}

Write-Host "========================================================" -ForegroundColor Cyan
Write-Host " Virtual Display Driver (VDD) Uninstallation" -ForegroundColor Cyan
Write-Host "========================================================" -ForegroundColor Cyan

$devcon = "$PSScriptRoot\control\Dependencies\devcon.exe"
Write-Host "[*] Removing Root\MttVDD device..." -ForegroundColor Yellow
& $devcon remove Root\MttVDD

Write-Host "`nUninstallation completed. Press Enter to exit..." -ForegroundColor Gray
Read-Host
