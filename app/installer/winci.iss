; WINCI installer (Inno Setup 6). Build the exe first, then from app\:
;   cargo build --release
;   iscc installer\winci.iss        -> target\installer\WINCI-<version>-setup.exe
; The version comes from the exe itself (Cargo.toml → build.rs), so there's
; nothing to bump here.

#define AppExe "..\target\release\winci.exe"
#define AppVersion GetVersionNumbersString(AppExe)

[Setup]
AppId={{6F1C2B0E-8D4A-4C7B-9E21-5A3D7F0B9C41}
AppName=WINCI
AppVersion={#AppVersion}
AppVerName=WINCI {#AppVersion}
AppPublisher=WINCI
DefaultDirName={autopf}\WINCI
DefaultGroupName=WINCI
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\target\installer
OutputBaseFilename=WINCI-{#AppVersion}-setup
SetupIconFile=..\assets\WINCI.ico
UninstallDisplayIcon={app}\winci.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\WINCI"; Filename: "{app}\winci.exe"
Name: "{autodesktop}\WINCI"; Filename: "{app}\winci.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\winci.exe"; Description: "Launch WINCI"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; the app's "Allow WINCI through the firewall" rule points at this exe
Filename: "netsh"; Parameters: "advfirewall firewall delete rule name=""WINC Data Crossing"""; Flags: runhidden; RunOnceId: "DelFirewallRule"
