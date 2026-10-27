; Inno Setup Script for BitTorrent Desktop Client & Tool Suite
; Generated for BitTorrent-rs

#define MyAppName "BitTorrent Client"
#define MyAppVersion "0.3.0"
#define MyAppPublisher "BitTorrent-rs Contributors"
#define MyAppURL "https://github.com/clockworkengineer/BitTorrent"
#define MyAppExeName "torrent_client.exe"

[Setup]
AppId={{5F81D6C4-3F8E-4A73-9C12-88C117562DA1}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={autopf}\BitTorrent
DefaultGroupName=BitTorrent
AllowNoIcons=yes
LicenseFile=..\..\LICENSE-MIT
OutputDir=..\..\dist
OutputBaseFilename=BitTorrent-Setup-v{#MyAppVersion}
SetupIconFile=..\..\assets\icon.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=commandline dialog

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "assoctorrent"; Description: "Associate with .torrent files"; GroupDescription: "File Associations:"
Name: "assocmagnet"; Description: "Associate with magnet: links"; GroupDescription: "URL Protocols:"

[Files]
Source: "..\..\target\release\torrent_client.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\target\release\bt_daemon.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\target\release\bt_cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\target\release\bt_webui.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\target\release\sync_hub.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\assets\icon.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\assets\icon.png"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\icon.ico"
Name: "{group}\BitTorrent WebUI"; Filename: "{app}\bt_webui.exe"; IconFilename: "{app}\icon.ico"
Name: "{group}\Uninstall {#MyAppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\icon.ico"; Tasks: desktopicon

[Registry]
; .torrent file association
Root: HKA; Subkey: "Software\Classes\.torrent"; ValueType: string; ValueName: ""; ValueData: "BitTorrent.TorrentFile"; Flags: uninsdeletevalue; Tasks: assoctorrent
Root: HKA; Subkey: "Software\Classes\BitTorrent.TorrentFile"; ValueType: string; ValueName: ""; ValueData: "BitTorrent Torrent File"; Flags: uninsdeletekey; Tasks: assoctorrent
Root: HKA; Subkey: "Software\Classes\BitTorrent.TorrentFile\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\icon.ico"; Tasks: assoctorrent
Root: HKA; Subkey: "Software\Classes\BitTorrent.TorrentFile\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: assoctorrent

; magnet: URL protocol handler
Root: HKA; Subkey: "Software\Classes\magnet"; ValueType: string; ValueName: ""; ValueData: "URL:BitTorrent Magnet Link"; Flags: uninsdeletekey; Tasks: assocmagnet
Root: HKA; Subkey: "Software\Classes\magnet"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""; Tasks: assocmagnet
Root: HKA; Subkey: "Software\Classes\magnet\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\icon.ico"; Tasks: assocmagnet
Root: HKA; Subkey: "Software\Classes\magnet\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: assocmagnet

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent
