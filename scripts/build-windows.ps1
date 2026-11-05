# Build DECKARD for Windows: DECKARD.exe, zipped, and an installer.
#
#   scripts\build-windows.ps1 [-Version 1.2.3] [-Target x86_64-pc-windows-msvc]
#
# Output: dist\DECKARD-<version>-windows-<arch>.zip (which the in-app update uses) and
# dist\DECKARD-<version>-windows-<arch>-setup.exe
#
# Needs Rust (https://rustup.rs), the Visual Studio Build Tools ("Desktop development
# with C++") and Inno Setup 6 (winget install JRSoftware.InnoSetup). Signing (optional):
# set WINDOWS_SIGN_CERT (a .pfx path) and WINDOWS_SIGN_PASSWORD; signtool from the Windows
# SDK signs the program and the installer.
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
$Iscc = @(
    (Get-Command iscc -ErrorAction SilentlyContinue).Source,
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $Iscc) {
    throw "missing: Inno Setup 6 (winget install JRSoftware.InnoSetup)"
}
if (Get-Command rustup -ErrorAction SilentlyContinue) {
    rustup target add $Target | Out-Null
}

function Invoke-Sign($File) {
    $signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" |
        Sort-Object FullName -Descending | Select-Object -First 1
    if (-not $signtool) { throw "signtool not found (install the Windows SDK)" }
    & $signtool.FullName sign /f $env:WINDOWS_SIGN_CERT /p $env:WINDOWS_SIGN_PASSWORD `
        /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 $File
    if ($LASTEXITCODE -ne 0) { throw "signing failed" }
}

# A PNG as an .ico frame's bitmap: 32-bit BGRA, bottom-up, then an empty 1-bit mask.
function ConvertTo-IconBitmap($Png, $Size) {
    Add-Type -AssemblyName System.Drawing
    $Image = [Drawing.Bitmap]::new($Png)
    try {
        $Stream = [IO.MemoryStream]::new()
        $Writer = [IO.BinaryWriter]::new($Stream)
        # BITMAPINFOHEADER; the height counts the mask too.
        $Writer.Write([uint32]40); $Writer.Write([int32]$Size); $Writer.Write([int32]($Size * 2))
        $Writer.Write([uint16]1); $Writer.Write([uint16]32)
        foreach ($Unused in 1..6) { $Writer.Write([uint32]0) }
        $Rect = [Drawing.Rectangle]::new(0, 0, $Size, $Size)
        $Data = $Image.LockBits($Rect, [Drawing.Imaging.ImageLockMode]::ReadOnly, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $Row = [byte[]]::new($Size * 4)
        for ($y = $Size - 1; $y -ge 0; $y--) {
            [Runtime.InteropServices.Marshal]::Copy([IntPtr]::Add($Data.Scan0, $y * $Data.Stride), $Row, 0, $Row.Length)
            $Writer.Write($Row)
        }
        $Image.UnlockBits($Data)
        $Writer.Write([byte[]]::new([Math]::Floor(($Size + 31) / 32) * 4 * $Size))
        $Writer.Flush()
        , $Stream.ToArray()
    } finally {
        $Image.Dispose()
    }
}

# The icon: DECKARD draws it, at each size Windows asks for, in an .ico (bitmaps, and a
# PNG at 256 as usual).
function Write-Icon($Exe, $Out) {
    $Sizes = 16, 24, 32, 48, 64, 128, 256
    $Dir = "$Out.parts"
    New-Item -ItemType Directory -Force $Dir | Out-Null
    $Pngs = foreach ($Size in $Sizes) {
        $Png = Join-Path $Dir "$Size.png"
        # Waits, as a GUI program wouldn't be waited for otherwise.
        $P = Start-Process $Exe -ArgumentList "--write-icon", "`"$Png`"", $Size -Wait -PassThru
        if ($P.ExitCode -ne 0) { throw "drawing the icon failed" }
        if ($Size -ge 256) { , [IO.File]::ReadAllBytes($Png) } else { , (ConvertTo-IconBitmap $Png $Size) }
    }
    $Writer = [IO.BinaryWriter]::new([IO.File]::Create($Out))
    try {
        $Writer.Write([uint16]0); $Writer.Write([uint16]1); $Writer.Write([uint16]$Sizes.Count)
        $Offset = 6 + 16 * $Sizes.Count
        for ($i = 0; $i -lt $Sizes.Count; $i++) {
            $Px = if ($Sizes[$i] -ge 256) { 0 } else { $Sizes[$i] }
            $Writer.Write([byte]$Px); $Writer.Write([byte]$Px); $Writer.Write([byte]0); $Writer.Write([byte]0)
            $Writer.Write([uint16]1); $Writer.Write([uint16]32)
            $Writer.Write([uint32]$Pngs[$i].Length); $Writer.Write([uint32]$Offset)
            $Offset += $Pngs[$i].Length
        }
        foreach ($Png in $Pngs) { $Writer.Write([byte[]]$Png) }
    } finally {
        $Writer.Dispose()
    }
    Remove-Item -Recurse -Force $Dir
}

Write-Host "building $Version for $Target"
cargo build --release --locked -p deckard --target $Target
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$Stage = "dist\$Name"
Remove-Item -Recurse -Force $Stage, "dist\$Name.zip", "dist\$Name-setup.exe" -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $Stage | Out-Null
Copy-Item "target\$Target\release\deckard.exe" "$Stage\DECKARD.exe"
Write-Icon (Resolve-Path "$Stage\DECKARD.exe").Path (Join-Path (Resolve-Path $Stage).Path "deckard.ico")
Copy-Item "LICENSE" "$Stage\LICENSE.txt"
Copy-Item "scripts\install-demucs.ps1" "$Stage\install-demucs.ps1"
Copy-Item "scripts\mk3-driver.ps1" "$Stage\mk3-driver.ps1"
@"
DECKARD $Version for Windows ($Arch)

Run DECKARD.exe, or open a .mxprj project with it.

Stem separation (Sampling > EDIT > STEMS) needs its model (84 MB), installed once:
right-click install-demucs.ps1 > Run with PowerShell, or
  powershell -NoProfile -ExecutionPolicy Bypass -File install-demucs.ps1
(DECKARD also runs it the first time you use STEMS.)

The screens of a Maschine MK3 or Maschine+ need Windows' WinUSB driver in place of NI's: DECKARD asks,
and Preferences > Hardware gives them back for Maschine (mk3-driver.ps1 does the change).
"@ | Set-Content -Encoding UTF8 "$Stage\README.txt"

if ($env:WINDOWS_SIGN_CERT) { Invoke-Sign "$Stage\DECKARD.exe" }

Compress-Archive -Path "$Stage\*" -DestinationPath "dist\$Name.zip"

$Numeric = if ($Version -match '^\d+(\.\d+){0,3}') { $Matches[0] } else { "0.0.0" }
$InnoArch = if ($Arch -eq "arm64") { "arm64" } else { "x64compatible" }
& $Iscc /Q "/DVersion=$Version" "/DNumericVersion=$Numeric" "/DArch=$InnoArch" `
    "/DStage=$((Resolve-Path $Stage).Path)" "/DOutput=$((Resolve-Path dist).Path)" "/DOutputName=$Name-setup" `
    scripts\windows\deckard.iss
if ($LASTEXITCODE -ne 0) { throw "building the installer failed" }
if ($env:WINDOWS_SIGN_CERT) { Invoke-Sign "dist\$Name-setup.exe" }

Remove-Item -Recurse -Force $Stage
Write-Host "wrote dist\$Name.zip and dist\$Name-setup.exe"
