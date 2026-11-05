# The rest of setting up on Windows, which `mise install` runs after the Rust and the
# stem model: the Visual Studio Build Tools (C++), which Rust links with, if they're
# missing, then a debug build.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/setup-windows.ps1
#
# Installing the tools takes several GB and asks for administrator approval. Nothing is
# installed if they're there already, and the build is incremental after the first.
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

# CI's runners have the tools, and its jobs build what they need.
if ($env:CI) {
    Write-Host "windows setup: skipped on CI"
    exit 0
}

function Test-BuildTools {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    (Test-Path $vswhere) -and [bool](& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
}

$Manual = "https://visualstudio.microsoft.com/visual-cpp-build-tools/"
if (Test-BuildTools) {
    Write-Host "windows setup: Visual Studio C++ tools are installed"
} else {
    if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
        throw "windows setup: winget is missing; install the Visual Studio Build Tools with 'Desktop development with C++' from $Manual"
    }
    Write-Host "windows setup: installing the Visual Studio Build Tools 2022 (C++), several GB; approve the administrator prompt."
    Write-Host "windows setup: this accepts the winget source's and Microsoft's license terms for them."
    winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --source winget `
        --accept-source-agreements --accept-package-agreements `
        --override "--passive --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
    # Not winget's exit code: it's nonzero for "needs a restart" and for "already installed"
    # (an existing Visual Studio without its C++ workload, which winget won't add).
    if (-not (Test-BuildTools)) {
        throw "windows setup: the C++ tools still aren't installed (winget exit $LASTEXITCODE). Add 'Desktop development with C++' in the Visual Studio Installer, or see $Manual"
    }
}

Write-Host "windows setup: building (debug)"
cargo build --workspace
if ($LASTEXITCODE -ne 0) { throw "windows setup: the build failed" }
Write-Host "windows setup: done"
