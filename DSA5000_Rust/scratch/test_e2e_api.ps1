$proc = Start-Process -FilePath "C:\agy\dsa5k\DSA5000_Rust\target\release\DSA5000_Rust.exe" -ArgumentList "--simulate", "--no-browser" -PassThru
Start-Sleep -Milliseconds 1200

try {
    Write-Host "[1] Testing Connect API..."
    $connPayload = @{ host = "127.0.0.1"; cmdPort = 2323; dataPort = 5503 } | ConvertTo-Json
    $connRes = Invoke-RestMethod -Uri "http://localhost:8500/api/connect" -Method Post -Body $connPayload -ContentType "application/json"
    Write-Host "Connect Result:" ($connRes | ConvertTo-Json -Compress)

    Write-Host "[2] Testing Valve Switch to CAL..."
    $valvePayload = @{ mode = "CAL" } | ConvertTo-Json
    $valveRes = Invoke-RestMethod -Uri "http://localhost:8500/api/valve" -Method Post -Body $valvePayload -ContentType "application/json"
    Write-Host "Valve Result:" ($valveRes | ConvertTo-Json -Compress)

    Write-Host "[3] Testing Start Scan..."
    $scanRes = Invoke-RestMethod -Uri "http://localhost:8500/api/start-scan" -Method Post -Body "{}" -ContentType "application/json"
    Write-Host "Start Scan Result:" ($scanRes | ConvertTo-Json -Compress)

    Start-Sleep -Milliseconds 1000

    Write-Host "[4] Checking Status during Scan..."
    $statusRes = Invoke-RestMethod -Uri "http://localhost:8500/api/status"
    Write-Host "Live Status:" ($statusRes | ConvertTo-Json -Compress)

    Write-Host "[5] Testing Stop Scan..."
    $stopRes = Invoke-RestMethod -Uri "http://localhost:8500/api/stop-scan" -Method Post -Body "{}" -ContentType "application/json"
    Write-Host "Stop Scan Result:" ($stopRes | ConvertTo-Json -Compress)

    Write-Host "[6] Testing Disconnect..."
    $dcRes = Invoke-RestMethod -Uri "http://localhost:8500/api/disconnect" -Method Post -Body "{}" -ContentType "application/json"
    Write-Host "Disconnect Result:" ($dcRes | ConvertTo-Json -Compress)

    Write-Host "`n>>> ALL REST/UI ENDPOINTS VERIFIED SUCCESSFULLY!"
}
finally {
    Stop-Process -Id $proc.Id -Force
}
