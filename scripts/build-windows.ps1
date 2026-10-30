# Build DECKARD for Windows: DECKARD.exe, zipped.
#
#   scripts\build-windows.ps1 [-Version 1.2.3] [-Target x86_64-pc-windows-msvc]
#
# Output: dist\DECKARD-<version>-windows-<arch>.zip
#
# Needs Rust (https://rustup.rs) and the Visual Studio Build Tools ("Desktop development
# with C++"). Signing (optional): set WINDOWS_SIGN_CERT (a .pfx path) and
# WINDOWS_SIGN_PASSWORD; signtool from the Windows SDK signs the .exe.
param(
    [string]$Version = "",
    [string]$Target = "x86_64-pc-windows-msvc"
)
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

if (-not $Version) {
    $Version = (Select-String -Path apps\viewer\Cargo.toml -Pattern '^version = "(.*)"' |
        Select-Object -First 1).Matches[0].Groups[1].Value
}
$Version = $Version -replace '^v', ''
$env:DECKARD_VERSION = $Version
$Arch = if ($Target -like "aarch64-*") { "arm64" } else { "x64" }
$Name = "DECKARD-$Version-windows-$Arch"

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "missing: Rust (install it from https://rustup.rs)"
}
# The MSVC linker comes with the Visual Studio Build Tools.
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path $vswhere) -or -not (& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)) {
    throw "missing: the Visual Studio Build Tools with 'Desktop development with C++' (https://visualstudio.microsoft.com/visual-cpp-build-tools/)"
}
if (Get-Command rustup -ErrorAction SilentlyContinue) {
    rustup target add $Target | Out-Null
}

Write-Host "building $Version for $Target"
cargo build --release --locked -p deckard --target $Target
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$Stage = "dist\$Name"
Remove-Item -Recurse -Force $Stage, "dist\$Name.zip" -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $Stage | Out-Null
Copy-Item "target\$Target\release\deckard.exe" "$Stage\DECKARD.exe"
Copy-Item "scripts\install-demucs.ps1" "$Stage\install-demucs.ps1"
@"
DECKARD $Version for Windows ($Arch)

Run DECKARD.exe, or open a .mxprj project with it.

Stem separation (Sampling > EDIT > STEMS) needs its model (84 MB), installed once:
right-click install-demucs.ps1 > Run with PowerShell, or
  powershell -NoProfile -ExecutionPolicy Bypass -File install-demucs.ps1
(DECKARD also runs it the first time you use STEMS.)
"@ | Set-Content -Encoding UTF8 "$Stage\README.txt"

if ($env:WINDOWS_SIGN_CERT) {
    $signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" |
        Sort-Object FullName -Descending | Select-Object -First 1
    if (-not $signtool) { throw "signtool not found (install the Windows SDK)" }
    & $signtool.FullName sign /f $env:WINDOWS_SIGN_CERT /p $env:WINDOWS_SIGN_PASSWORD `
        /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 "$Stage\DECKARD.exe"
    if ($LASTEXITCODE -ne 0) { throw "signing failed" }
}

Compress-Archive -Path "$Stage\*" -DestinationPath "dist\$Name.zip"
Remove-Item -Recurse -Force $Stage
Write-Host "wrote dist\$Name.zip"
