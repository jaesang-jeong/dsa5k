$proc = Start-Process -FilePath "C:\agy\dsa5k\DSA5000_Rust\target\release\DSA5000_Rust.exe" -ArgumentList "--simulate", "--no-browser" -PassThru
Start-Sleep -Milliseconds 1200

try {
    $res = Invoke-WebRequest -Uri "http://localhost:8500/" -UseBasicParsing
    Write-Host "HTTP Root Status: $($res.StatusCode)"
    Write-Host "Content-Length: $($res.Content.Length)"
    Write-Host "Contains Title: $($res.Content.Contains('DSA5000 Studio'))"

    $css = Invoke-WebRequest -Uri "http://localhost:8500/style.css" -UseBasicParsing
    Write-Host "CSS Status: $($css.StatusCode), Length: $($css.Content.Length)"

    $js = Invoke-WebRequest -Uri "http://localhost:8500/app.js" -UseBasicParsing
    Write-Host "JS Status: $($js.StatusCode), Length: $($js.Content.Length)"

    $api = Invoke-RestMethod -Uri "http://localhost:8500/api/status"
    Write-Host "API Status JSON:" ($api | ConvertTo-Json -Compress)
}
finally {
    Stop-Process -Id $proc.Id -Force
}
