# Install the stem separation model (EDIT > STEMS) on Windows: Meta's Demucs v4
# `htdemucs` weights (MIT) into DECKARD's settings folder, %APPDATA%\deckard\models (or
# $env:DECKARD_SETTINGS\models), checked against their published SHA-256. As
# scripts/install-demucs.sh:
#
#   pwsh -NoProfile -File scripts/install-demucs.ps1
$ErrorActionPreference = "Stop"
# Windows PowerShell 5.1: its progress bar slows downloads to a crawl, and older .NET
# doesn't offer TLS 1.2, which Hugging Face needs.
$ProgressPreference = "SilentlyContinue"
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

# CI builds and tests DECKARD without it.
if ($env:CI -and -not $env:DECKARD_INSTALL_DEMUCS) {
    Write-Host "stem separation model: skipped on CI (set DECKARD_INSTALL_DEMUCS=1 to install)"
    exit 0
}

$File = "htdemucs.safetensors"
$Url = "https://huggingface.co/set-soft/audio_separation/resolve/main/Demucs/$File"
$Sha256 = "8193504cdfb3943adaf039b8acb524a46e87ebf232c383ac7a32c80a6578423e"

$Settings = if ($env:DECKARD_SETTINGS) { $env:DECKARD_SETTINGS } else { Join-Path $env:APPDATA "deckard" }
$Dir = Join-Path $Settings "models"
$Path = Join-Path $Dir $File

if ((Test-Path $Path) -and ((Get-FileHash $Path -Algorithm SHA256).Hash.ToLower() -eq $Sha256)) {
    Write-Host "stem separation model: installed ($Path)"
    exit 0
}

New-Item -ItemType Directory -Force -Path $Dir | Out-Null
$Tmp = Join-Path $Dir ".$File.download"
Write-Host "stem separation model: downloading $File (84 MB)"
try {
    Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $Tmp
    $Got = (Get-FileHash $Tmp -Algorithm SHA256).Hash.ToLower()
    if ($Got -ne $Sha256) {
        throw "stem separation model: the download isn't the expected file (SHA-256 $Got)"
    }
    Move-Item -Force $Tmp $Path
} finally {
    if (Test-Path $Tmp) { Remove-Item -Force $Tmp }
}
Write-Host "stem separation model: installed ($Path)"
