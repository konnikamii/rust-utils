<#
package_exes.ps1
Builds the workspace and packages all produced executables into a zip

Usage:
  ./scripts/package_exes.ps1
#>
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

Write-Host "Building workspace ..."
cargo build --workspace --release

if (-not (Test-Path $repoRoot)) { New-Item -ItemType Directory -Path $repoRoot | Out-Null }

$timestamp = (Get-Date -Format 'yyyyMMddHHmmss')
$zipName = "executables.zip"
$zipPath = Join-Path $repoRoot $zipName

# Collect crate directories under ./crates
$crateDirs = Get-ChildItem -Path (Join-Path $repoRoot 'crates') -Directory -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Name

$tempDir = Join-Path ([IO.Path]::GetTempPath()) "rust-utils-exes-$timestamp"
New-Item -ItemType Directory -Path $tempDir | Out-Null

foreach ($crate in $crateDirs) {
    $exePath = Join-Path $repoRoot (Join-Path "target\release" "$crate.exe")
    if (Test-Path $exePath) {
        Copy-Item -Path $exePath -Destination $tempDir -Force
        Write-Host "Added $crate.exe"
    }
}

# Also include top-level binaries in target/release/ (if any)
Get-ChildItem -Path (Join-Path $repoRoot "target\release") -Filter *.exe -File -ErrorAction SilentlyContinue | ForEach-Object {
    $source = $_.FullName
    $name = $_.Name
    if (-not (Test-Path (Join-Path $tempDir $name))) {
        Copy-Item -Path $source -Destination $tempDir -Force
        Write-Host "Added $name"
    }
}

# Create zip
if (Test-Path $zipPath) { Remove-Item $zipPath -Force }
Compress-Archive -Path (Join-Path $tempDir '*') -DestinationPath $zipPath

# Cleanup
Remove-Item -Recurse -Force $tempDir

Write-Host "Created $zipPath"
Write-Host "Done."