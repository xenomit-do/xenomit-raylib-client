# Build the Linux desktop client from a Windows host via WSL (native x86_64-unknown-linux-gnu).
# Cross-compiling raylib + bindgen on Windows MSVC without WSL is not supported here.
param(
    [string]$WslDistro = 'Ubuntu'
)
$ErrorActionPreference = 'Stop'

if (-not (Get-Command wsl.exe -ErrorAction SilentlyContinue)) {
    throw 'WSL is required to produce a Linux binary from Windows. Install WSL or run ./tools/build-linux.sh on a Linux machine.'
}
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$wslRepo = (wsl -d $WslDistro wslpath -a $repo).Trim()
if (-not $wslRepo) { throw "Could not map repo path into WSL distro '$WslDistro'." }

Write-Host "Building Linux client in WSL ($WslDistro) -> target/linux/release/xenomit-client"
wsl -d $WslDistro -e bash -lc "set -eu; cd '$wslRepo' && ./tools/build-linux.sh"
if ($LASTEXITCODE -ne 0) { throw 'Linux build failed in WSL.' }

$binary = Join-Path $repo 'target\linux\release\xenomit-client'
if (-not (Test-Path -LiteralPath $binary)) {
    throw "Expected Linux binary at $binary"
}
Write-Host "Linux binary: $binary"
