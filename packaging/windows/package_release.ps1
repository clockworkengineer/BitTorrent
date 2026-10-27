<#
.SYNOPSIS
    Builds optimized release binaries and creates portable ZIP and Windows Installer packages.
.DESCRIPTION
    1. Compiles release binaries with optimizations enabled.
    2. Packages binaries and assets into a portable ZIP in dist/.
    3. If Inno Setup (ISCC) is installed, compiles BitTorrent-Setup-v0.3.0.exe.
#>

[CmdletBinding()]
param (
    [string]$Version = "0.3.0",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

$RootDir = Resolve-Path "$PSScriptRoot\..\.."
$DistDir = "$RootDir\dist"
$StageDir = "$DistDir\BitTorrent-v$Version-windows-x86_64"

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "  BitTorrent Client - Windows Release Packaging Script   " -ForegroundColor Cyan
Write-Host "  Version: $Version                                      " -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

# 1. Build optimized release binaries
if (-not $SkipBuild) {
    Write-Host "`n[1/4] Compiling release binaries with cargo..." -ForegroundColor Yellow
    Push-Location $RootDir
    try {
        cargo build --release -p torrent_client -p bt_daemon -p sync_hub
        if ($LASTEXITCODE -ne 0) {
            throw "Cargo release build failed with exit code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
    }
} else {
    Write-Host "`n[1/4] Skipping cargo build (-SkipBuild specified)..." -ForegroundColor DarkGray
}

# 2. Stage distribution directory
Write-Host "`n[2/4] Staging files into $StageDir..." -ForegroundColor Yellow

if (Test-Path $StageDir) {
    Remove-Item -Recurse -Force $StageDir
}
New-Item -ItemType Directory -Force -Path $StageDir | Out-Null

$Binaries = @(
    "torrent_client.exe",
    "bt_daemon.exe",
    "bt_cli.exe",
    "bt_webui.exe",
    "sync_hub.exe"
)

foreach ($bin in $Binaries) {
    $src = "$RootDir\target\release\$bin"
    if (Test-Path $src) {
        Copy-Item -Path $src -Destination $StageDir
        Write-Host "  + Staged: $bin" -ForegroundColor Green
    } else {
        Write-Warning "  - Missing binary: $src"
    }
}

# Copy assets and legal files
Copy-Item "$RootDir\assets\icon.ico" -Destination $StageDir
Copy-Item "$RootDir\assets\icon.png" -Destination $StageDir
Copy-Item "$RootDir\README.md" -Destination $StageDir
Copy-Item "$RootDir\LICENSE-MIT" -Destination $StageDir
Copy-Item "$RootDir\LICENSE-APACHE" -Destination $StageDir

# 3. Create portable ZIP archive
Write-Host "`n[3/4] Creating portable ZIP release..." -ForegroundColor Yellow
$ZipPath = "$DistDir\BitTorrent-v$Version-windows-x86_64-portable.zip"
if (Test-Path $ZipPath) {
    Remove-Item -Force $ZipPath
}

Start-Sleep -Seconds 1
for ($attempt = 1; $attempt -le 3; $attempt++) {
    try {
        Compress-Archive -Path "$StageDir\*" -DestinationPath $ZipPath -CompressionLevel Optimal
        Write-Host "  [OK] Portable package created: $ZipPath" -ForegroundColor Green
        break
    } catch {
        if ($attempt -eq 3) {
            Write-Warning "Could not create ZIP archive: $_"
        } else {
            Start-Sleep -Seconds 2
        }
    }
}

# 4. Check for Inno Setup compiler (ISCC)
Write-Host "`n[4/4] Checking for Inno Setup compiler (ISCC)..." -ForegroundColor Yellow

$ISCC_Paths = @(
    "ISCC.exe",
    "C:\Program Files (x86)\Inno Setup 6\ISCC.exe",
    "C:\Program Files\Inno Setup 6\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
)

$IsccExe = $null
foreach ($p in $ISCC_Paths) {
    $found = Get-Command $p -ErrorAction SilentlyContinue
    if ($found) {
        $IsccExe = $found.Source
        break
    } elseif (Test-Path $p) {
        $IsccExe = $p
        break
    }
}

if ($IsccExe) {
    Write-Host "  Found Inno Setup at: $IsccExe" -ForegroundColor Cyan
    Write-Host "  Compiling Windows Setup installer..." -ForegroundColor Yellow
    $IssScript = "$RootDir\packaging\windows\installer.iss"
    $proc = Start-Process -FilePath $IsccExe -ArgumentList $IssScript -Wait -PassThru -NoNewWindow
    $code = $proc.ExitCode
    if ($code -eq 0) {
        Write-Host "  [OK] Setup Installer created: $DistDir\BitTorrent-Setup-v$Version.exe" -ForegroundColor Green
    } else {
        Write-Warning "  Inno Setup compiler returned non-zero exit code: $code"
    }
} else {
    Write-Host "  Inno Setup not detected on local system." -ForegroundColor DarkYellow
    Write-Host "  To generate 'BitTorrent-Setup-v$Version.exe':" -ForegroundColor Gray
    Write-Host "    1. Install Inno Setup (winget install JRSoftware.InnoSetup or https://jrsoftware.org)" -ForegroundColor Gray
    Write-Host "    2. Run: ISCC packaging\windows\installer.iss" -ForegroundColor Gray
}

Write-Host "`n==========================================================" -ForegroundColor Cyan
Write-Host "  Release Packaging Complete!                             " -ForegroundColor Cyan
Write-Host "  Artifacts available in: $DistDir                        " -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan
