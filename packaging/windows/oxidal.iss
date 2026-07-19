#ifndef AppVersion
#define AppVersion "0.0.0"
#endif
#ifndef Arch
#define Arch "x86_64"
#endif
#ifndef BinPath
#define BinPath "..\..\target\release\Oxidal.exe"
#endif

[Setup]
AppId={{6F1C2B7A-9D44-4C1B-8E2A-3B5D8F0C9A21}
AppName=Oxidal
AppVersion={#AppVersion}
AppPublisher=sh4den
AppPublisherURL=https://github.com/sh4den/Oxidal
AppSupportURL=https://github.com/sh4den/Oxidal/issues
DefaultDirName={autopf}\Oxidal
DefaultGroupName=Oxidal
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputBaseFilename=Oxidal-{#AppVersion}-windows-{#Arch}-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
SetupIconFile=icon.ico
UninstallDisplayIcon={app}\Oxidal.exe
#if Arch == "arm64"
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
#else
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
#endif

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked

[Files]
Source: "{#BinPath}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Oxidal"; Filename: "{app}\Oxidal.exe"
Name: "{autodesktop}\Oxidal"; Filename: "{app}\Oxidal.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\Oxidal.exe"; Description: "{cm:LaunchProgram,Oxidal}"; Flags: nowait postinstall skipifsilent
