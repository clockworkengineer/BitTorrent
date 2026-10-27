<#
.SYNOPSIS
    BitTorrent-rs Universal Windows PowerShell Installer
.DESCRIPTION
    Installs the BitTorrent client and tool suite on Windows without requiring third-party installers.
    Supports file/URL protocol associations (.torrent and magnet:), Start Menu and Desktop shortcuts,
    user PATH registration, and headless daemon-only mode.
.PARAMETER InstallDir
    Target directory for installation (default: $env:LOCALAPPDATA\Programs\BitTorrent, or Program Files if Admin).
.PARAMETER DesktopShortcut
    Create a shortcut on the Desktop.
.PARAMETER NoDesktopShortcut
    Skip creating a Desktop shortcut.
.PARAMETER StartMenu
    Create shortcuts in the Windows Start Menu (default: enabled).
.PARAMETER RegisterProtocols
    Register .torrent file association and magnet: URL protocol handler in HKCU (default: enabled).
.PARAMETER AddToPath
    Add the installation directory to the User PATH environment variable (default: enabled).
.PARAMETER DaemonOnly
    Install only headless server daemon, CLI, and sync tools; skip desktop GUI.
.PARAMETER SkipBuild
    Do not compile binaries with cargo if missing in target/release or bin/.
.PARAMETER Uninstall
    Remove all installed binaries, shortcuts, registry associations, and PATH entries.
.EXAMPLE
    .\install.ps1
.EXAMPLE
    .\install.ps1 -DesktopShortcut -RegisterProtocols
.EXAMPLE
    .\install.ps1 -Uninstall
#>

[CmdletBinding(DefaultParameterSetName = "Install")]
param (
    [Parameter(ParameterSetName = "Install")]
    [Parameter(ParameterSetName = "Uninstall")]
    [string]$InstallDir,

    [Parameter(ParameterSetName = "Install")]
    [switch]$DesktopShortcut = $true,

    [Parameter(ParameterSetName = "Install")]
    [switch]$NoDesktopShortcut,

    [Parameter(ParameterSetName = "Install")]
    [switch]$StartMenu = $true,

    [Parameter(ParameterSetName = "Install")]
    [switch]$RegisterProtocols = $true,

    [Parameter(ParameterSetName = "Install")]
    [switch]$AddToPath = $true,

    [Parameter(ParameterSetName = "Install")]
    [switch]$DaemonOnly,

    [Parameter(ParameterSetName = "Install")]
    [switch]$SkipBuild,

    [Parameter(ParameterSetName = "Uninstall")]
    [switch]$Uninstall
)

$ErrorActionPreference = "Stop"

function Write-Info {
    param([string]$Message)
    Write-Host "[INFO] $Message" -ForegroundColor Cyan
}

function Write-Success {
    param([string]$Message)
    Write-Host "[OK]   $Message" -ForegroundColor Green
}

function Write-WarnMsg {
    param([string]$Message)
    Write-Host "[WARN] $Message" -ForegroundColor Yellow
}

function Write-ErrMsg {
    param([string]$Message)
    Write-Host "[ERR]  $Message" -ForegroundColor Red
}

function Test-IsAdmin {
    $currentPrincipal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
    return $currentPrincipal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

# ------------------------------------------------------------------------------
# 1. Determine Installation Directory
# ------------------------------------------------------------------------------
$IsAdmin = Test-IsAdmin
if (-not $InstallDir) {
    if ($IsAdmin) {
        $InstallDir = "$env:ProgramFiles\BitTorrent"
    } else {
        $InstallDir = "$env:LOCALAPPDATA\Programs\BitTorrent"
    }
}

$StartMenuDir = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\BitTorrent"
$DesktopDir = [Environment]::GetFolderPath("Desktop")

# ------------------------------------------------------------------------------
# 2. Uninstallation Flow
# ------------------------------------------------------------------------------
if ($Uninstall) {
    Write-Host "==========================================================" -ForegroundColor Cyan
    Write-Host "  BitTorrent Client - Windows Uninstaller                " -ForegroundColor Cyan
    Write-Host "  Target: $InstallDir                                     " -ForegroundColor Cyan
    Write-Host "==========================================================" -ForegroundColor Cyan

    # 1. Remove Desktop shortcut
    $DesktopLnk = Join-Path $DesktopDir "BitTorrent.lnk"
    if (Test-Path $DesktopLnk) {
        Remove-Item -Force $DesktopLnk -ErrorAction SilentlyContinue
        Write-Success "Removed Desktop shortcut: $DesktopLnk"
    }

    # 2. Remove Start Menu folder
    if (Test-Path $StartMenuDir) {
        Remove-Item -Recurse -Force $StartMenuDir -ErrorAction SilentlyContinue
        Write-Success "Removed Start Menu folder: $StartMenuDir"
    }

    # 3. Clean up registry associations in HKCU
    $regPaths = @(
        "HKCU:\Software\Classes\.torrent",
        "HKCU:\Software\Classes\BitTorrent.TorrentFile",
        "HKCU:\Software\Classes\magnet"
    )
    foreach ($rp in $regPaths) {
        if (Test-Path $rp) {
            Remove-Item -Recurse -Force $rp -ErrorAction SilentlyContinue
            Write-Success "Removed registry key: $rp"
        }
    }

    # 4. Remove from User PATH
    try {
        $UserPath = [Environment]::GetEnvironmentVariable("Path", [EnvironmentVariableTarget]::User)
        if ($UserPath) {
            $split = $UserPath -split ';' | Where-Object { $_ -and ($_ -ne $InstallDir) }
            $NewUserPath = $split -join ';'
            [Environment]::SetEnvironmentVariable("Path", $NewUserPath, [EnvironmentVariableTarget]::User)
            Write-Success "Removed $InstallDir from User PATH"
        }
    } catch {
        Write-WarnMsg "Could not update User PATH: $_"
    }

    # 5. Remove install folder
    if (Test-Path $InstallDir) {
        Remove-Item -Recurse -Force $InstallDir -ErrorAction SilentlyContinue
        Write-Success "Removed installation directory: $InstallDir"
    }

    Write-Host "`nUninstallation completed successfully!" -ForegroundColor Green
    return
}

# ------------------------------------------------------------------------------
# 3. Installation Flow
# ------------------------------------------------------------------------------
if ($NoDesktopShortcut) {
    $DesktopShortcut = $false
}

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "  BitTorrent Client - Universal Windows Installer         " -ForegroundColor Cyan
Write-Host "  Destination: $InstallDir                                " -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $ScriptDir) {
    $ScriptDir = (Get-Location).Path
}

function Find-Binary {
    param([string]$Name)
    $candidates = @(
        "$ScriptDir\bin\$Name.exe",
        "$ScriptDir\$Name.exe",
        "$ScriptDir\target\release\$Name.exe"
    )
    foreach ($cand in $candidates) {
        if (Test-Path $cand) {
            return $cand
        }
    }
    return $null
}

# Check for binaries or compile
$GuiBinary = Find-Binary "torrent_client"
$DaemonBinary = Find-Binary "bt_daemon"

if (-not $DaemonBinary -and (Test-Path "$ScriptDir\Cargo.toml") -and -not $SkipBuild) {
    Write-Info "Pre-compiled binaries not found. Building release binaries with cargo..."
    Push-Location $ScriptDir
    try {
        cargo build --release -p bt_daemon -p sync_hub
        if (-not $DaemonOnly) {
            cargo build --release -p torrent_client
        }
    } finally {
        Pop-Location
    }
    $GuiBinary = Find-Binary "torrent_client"
    $DaemonBinary = Find-Binary "bt_daemon"
}

# Create installation directory
if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    Write-Success "Created directory: $InstallDir"
}

# Copy Binaries
$AllBins = @("bt_daemon", "bt_cli", "bt_webui", "sync_hub")
if (-not $DaemonOnly) {
    $AllBins = @("torrent_client") + $AllBins
}

foreach ($b in $AllBins) {
    $src = Find-Binary $b
    if ($src -and (Test-Path $src)) {
        Copy-Item -Path $src -Destination "$InstallDir\$b.exe" -Force
        Write-Success "Installed binary: $b.exe"
    } else {
        Write-WarnMsg "Binary $b.exe not found, skipped."
    }
}

# Copy Assets & Documentation
$AssetsToCopy = @(
    @{ Src = "$ScriptDir\assets\icon.ico"; Dest = "$InstallDir\icon.ico" },
    @{ Src = "$ScriptDir\assets\icon.png"; Dest = "$InstallDir\icon.png" },
    @{ Src = "$ScriptDir\README.md";      Dest = "$InstallDir\README.md" },
    @{ Src = "$ScriptDir\LICENSE-MIT";   Dest = "$InstallDir\LICENSE-MIT" }
)

foreach ($asset in $AssetsToCopy) {
    if (Test-Path $asset.Src) {
        Copy-Item -Path $asset.Src -Destination $asset.Dest -Force
    }
}

# Save uninstaller script in install directory
$UninstallerScript = "$InstallDir\uninstall.ps1"
@"
& "$MyInvocation.MyCommand.Path" -Uninstall -InstallDir "$InstallDir"
"@ | Set-Content -Path $UninstallerScript -Encoding UTF8

# ------------------------------------------------------------------------------
# 4. Create Shortcuts (Start Menu & Desktop)
# ------------------------------------------------------------------------------
$ClientExe = "$InstallDir\torrent_client.exe"
$IconFile = "$InstallDir\icon.ico"
$WshShell = New-Object -ComObject WScript.Shell

if (-not $DaemonOnly -and (Test-Path $ClientExe)) {
    if ($StartMenu) {
        if (-not (Test-Path $StartMenuDir)) {
            New-Item -ItemType Directory -Force -Path $StartMenuDir | Out-Null
        }

        # App shortcut
        $appShortcut = $WshShell.CreateShortcut("$StartMenuDir\BitTorrent.lnk")
        $appShortcut.TargetPath = $ClientExe
        $appShortcut.WorkingDirectory = $InstallDir
        if (Test-Path $IconFile) { $appShortcut.IconLocation = $IconFile }
        $appShortcut.Description = "BitTorrent Desktop Client"
        $appShortcut.Save()

        # WebUI shortcut
        $webuiExe = "$InstallDir\bt_webui.exe"
        if (Test-Path $webuiExe) {
            $webShortcut = $WshShell.CreateShortcut("$StartMenuDir\BitTorrent WebUI.lnk")
            $webShortcut.TargetPath = $webuiExe
            $webShortcut.WorkingDirectory = $InstallDir
            if (Test-Path $IconFile) { $webShortcut.IconLocation = $IconFile }
            $webShortcut.Description = "BitTorrent WebUI Gateway"
            $webShortcut.Save()
        }

        Write-Success "Created Start Menu shortcuts at: $StartMenuDir"
    }

    if ($DesktopShortcut) {
        $desktopLnk = $WshShell.CreateShortcut("$DesktopDir\BitTorrent.lnk")
        $desktopLnk.TargetPath = $ClientExe
        $desktopLnk.WorkingDirectory = $InstallDir
        if (Test-Path $IconFile) { $desktopLnk.IconLocation = $IconFile }
        $desktopLnk.Description = "BitTorrent Desktop Client"
        $desktopLnk.Save()
        Write-Success "Created Desktop shortcut: $DesktopDir\BitTorrent.lnk"
    }
}

# ------------------------------------------------------------------------------
# 5. Register Protocol & File Associations in HKCU
# ------------------------------------------------------------------------------
if ($RegisterProtocols -and -not $DaemonOnly -and (Test-Path $ClientExe)) {
    Write-Info "Registering .torrent file and magnet: protocol associations in HKCU..."
    try {
        # .torrent association
        New-Item -Path "HKCU:\Software\Classes\.torrent" -Force | Out-Null
        Set-ItemProperty -Path "HKCU:\Software\Classes\.torrent" -Name "(Default)" -Value "BitTorrent.TorrentFile"

        New-Item -Path "HKCU:\Software\Classes\BitTorrent.TorrentFile" -Force | Out-Null
        Set-ItemProperty -Path "HKCU:\Software\Classes\BitTorrent.TorrentFile" -Name "(Default)" -Value "BitTorrent Torrent File"

        if (Test-Path $IconFile) {
            New-Item -Path "HKCU:\Software\Classes\BitTorrent.TorrentFile\DefaultIcon" -Force | Out-Null
            Set-ItemProperty -Path "HKCU:\Software\Classes\BitTorrent.TorrentFile\DefaultIcon" -Name "(Default)" -Value "$IconFile,0"
        }

        New-Item -Path "HKCU:\Software\Classes\BitTorrent.TorrentFile\shell\open\command" -Force | Out-Null
        Set-ItemProperty -Path "HKCU:\Software\Classes\BitTorrent.TorrentFile\shell\open\command" -Name "(Default)" -Value "`"$ClientExe`" `"%1`""

        # magnet: URL scheme
        New-Item -Path "HKCU:\Software\Classes\magnet" -Force | Out-Null
        Set-ItemProperty -Path "HKCU:\Software\Classes\magnet" -Name "(Default)" -Value "URL:BitTorrent Magnet Link"
        Set-ItemProperty -Path "HKCU:\Software\Classes\magnet" -Name "URL Protocol" -Value ""

        if (Test-Path $IconFile) {
            New-Item -Path "HKCU:\Software\Classes\magnet\DefaultIcon" -Force | Out-Null
            Set-ItemProperty -Path "HKCU:\Software\Classes\magnet\DefaultIcon" -Name "(Default)" -Value "$IconFile,0"
        }

        New-Item -Path "HKCU:\Software\Classes\magnet\shell\open\command" -Force | Out-Null
        Set-ItemProperty -Path "HKCU:\Software\Classes\magnet\shell\open\command" -Name "(Default)" -Value "`"$ClientExe`" `"%1`""

        Write-Success "Registered .torrent file type and magnet: URL protocol"
    } catch {
        Write-WarnMsg "Failed to register protocol associations: $_"
    }
}

# ------------------------------------------------------------------------------
# 6. Add to PATH
# ------------------------------------------------------------------------------
if ($AddToPath) {
    try {
        $UserPath = [Environment]::GetEnvironmentVariable("Path", [EnvironmentVariableTarget]::User)
        if (-not $UserPath) { $UserPath = "" }
        $parts = $UserPath -split ';' | Where-Object { $_ }
        if ($parts -notcontains $InstallDir) {
            $NewUserPath = if ($UserPath) { "$UserPath;$InstallDir" } else { $InstallDir }
            [Environment]::SetEnvironmentVariable("Path", $NewUserPath, [EnvironmentVariableTarget]::User)
            Write-Success "Added $InstallDir to User PATH"
        } else {
            Write-Info "$InstallDir is already present in User PATH"
        }
    } catch {
        Write-WarnMsg "Could not update User PATH: $_"
    }
}

# ------------------------------------------------------------------------------
# 7. Summary
# ------------------------------------------------------------------------------
Write-Host "`n==========================================================" -ForegroundColor Green
Write-Host "  Installation Completed Successfully!                   " -ForegroundColor Green
Write-Host "==========================================================" -ForegroundColor Green
Write-Host "Installed location : $InstallDir"
Write-Host "`nInstalled Binaries:"
foreach ($b in $AllBins) {
    if (Test-Path "$InstallDir\$b.exe") {
        Write-Host "  - $b.exe" -ForegroundColor Cyan
    }
}

Write-Host "`nTo uninstall in the future, run:" -ForegroundColor DarkGray
Write-Host "  powershell -ExecutionPolicy Bypass -File .\install.ps1 -Uninstall" -ForegroundColor DarkGray
Write-Host "  or: powershell -File `"$InstallDir\uninstall.ps1`"`n" -ForegroundColor DarkGray
