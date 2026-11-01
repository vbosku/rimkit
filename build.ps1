# Builds RimKit (release) and copies rimkit.exe next to this script.
# Cargo's own output goes to D:\cargo-target\rimkit (see .cargo\config.toml) because C: is nearly full.
$mingw = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\BrechtSanders.WinLibs*\mingw64\bin" -ErrorAction SilentlyContinue | Select-Object -First 1
if ($mingw) { $env:PATH = "$env:USERPROFILE\.cargo\bin;$($mingw.FullName);$env:PATH" }
Push-Location $PSScriptRoot
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    $target = (cargo metadata --format-version 1 --no-deps | ConvertFrom-Json).target_directory
    Copy-Item (Join-Path $target "release\rimkit.exe") (Join-Path $PSScriptRoot "rimkit.exe") -Force
    Write-Host "Built: $(Join-Path $PSScriptRoot 'rimkit.exe')"
} finally { Pop-Location }
