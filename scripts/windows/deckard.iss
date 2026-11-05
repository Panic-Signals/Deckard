; The Windows installer (Inno Setup 6), built by scripts/build-windows.ps1 from its staged
; files. Installs for the current user, without administrator approval, into a folder the
; in-app update can write to; the Start menu entry is what Windows search finds.
;
;   ISCC /DVersion=1.2.3 /DNumericVersion=1.2.3 /DArch=x64compatible /DStage=<dir>
;        /DOutput=<dir> /DOutputName=<name> scripts\windows\deckard.iss

[Setup]
; Keep this id: it's how a new version finds the one it replaces.
AppId={{6F2B6C1E-3A47-4E0B-9C55-7D1E8A40B2D3}
AppName=DECKARD
AppVersion={#Version}
AppVerName=DECKARD {#Version}
AppPublisher=DECKARD
AppPublisherURL=https://github.com/Panic-Signals/Deckard
VersionInfoVersion={#NumericVersion}
PrivilegesRequired=lowest
ArchitecturesAllowed={#Arch}
ArchitecturesInstallIn64BitMode={#Arch}
DefaultDirName={autopf}\DECKARD
DisableProgramGroupPage=yes
SetupIconFile={#Stage}\deckard.ico
UninstallDisplayIcon={app}\deckard.ico
UninstallDisplayName=DECKARD
ChangesAssociations=yes
CloseApplications=yes
RestartApplications=no
WizardStyle=modern
Compression=lzma2
SolidCompression=yes
OutputDir={#Output}
OutputBaseFilename={#OutputName}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; Flags: unchecked

[Files]
Source: "{#Stage}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{autoprograms}\DECKARD"; Filename: "{app}\DECKARD.exe"; IconFilename: "{app}\deckard.ico"
Name: "{autodesktop}\DECKARD"; Filename: "{app}\DECKARD.exe"; IconFilename: "{app}\deckard.ico"; Tasks: desktopicon

; .mxprj projects can open with DECKARD (Open with), without taking them from Maschine.
[Registry]
Root: HKA; Subkey: "Software\Classes\.mxprj\OpenWithProgids"; ValueType: string; ValueName: "DECKARD.Project"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\DECKARD.Project"; ValueType: string; ValueName: ""; ValueData: "Maschine Project"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\DECKARD.Project\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\deckard.ico"
Root: HKA; Subkey: "Software\Classes\DECKARD.Project\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\DECKARD.exe"" ""%1"""

[Run]
Filename: "{app}\DECKARD.exe"; Description: "Start DECKARD"; Flags: nowait postinstall skipifsilent
