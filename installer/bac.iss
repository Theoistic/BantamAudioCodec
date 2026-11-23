; The Windows installer of bac.exe (the BAC previewer and command line), built by Inno Setup 6:
;   iscc /DAppVersion=0.1.1 /DSourceDir=..\target\release installer\bac.iss
; Needs installer\bac.ico: cargo run -p bantam-audio-codec-player --example icon -- installer\bac.ico
; Installs for the current user, no administrator rights.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\target\release"
#endif

[Setup]
AppId={{6C1B5E0A-2F7D-4C59-9B5E-BAC0A0D10001}
AppName=BAC
AppVersion={#AppVersion}
AppVerName=BAC {#AppVersion}
AppPublisher=Theodor Solbjorg (Theoistic)
AppPublisherURL=https://github.com/Theoistic/BantamAudioCodec
AppSupportURL=https://github.com/Theoistic/BantamAudioCodec/issues
AppUpdatesURL=https://github.com/Theoistic/BantamAudioCodec/releases
AppComments=The Bantam Audio Codec: voice audio with its transcript, speakers and range tags in one file
AppCopyright=Copyright Theodor Solbjorg (Theoistic)
DefaultDirName={autopf}\BAC
DefaultGroupName=BAC
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
LicenseFile=..\LICENSE-MIT
SetupIconFile=bac.ico
UninstallDisplayIcon={app}\bac.exe
UninstallDisplayName=BAC {#AppVersion}
OutputDir=..\dist
OutputBaseFilename=bac-setup-v{#AppVersion}-x86_64
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ChangesAssociations=yes
ChangesEnvironment=yes

[Tasks]
Name: "assoc"; Description: "Open .bac files with the BAC previewer"
Name: "addtopath"; Description: "Add bac to the PATH (for the command line)"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\bac.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\BAC"; Filename: "{app}\bac.exe"; Comment: "The BAC previewer"
Name: "{autodesktop}\BAC"; Filename: "{app}\bac.exe"; Tasks: desktopicon

[Registry]
Root: HKA; Subkey: "Software\Classes\.bac"; ValueType: string; ValueName: ""; ValueData: "BAC.File"; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.bac\OpenWithProgids"; ValueType: string; ValueName: "BAC.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\BAC.File"; ValueType: string; ValueName: ""; ValueData: "BAC audio"; Flags: uninsdeletekey; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\BAC.File\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\bac.exe,0"; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\BAC.File\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\bac.exe"" ""%1"""; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\Applications\bac.exe\SupportedTypes"; ValueType: string; ValueName: ".bac"; ValueData: ""; Flags: uninsdeletekey
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Tasks: addtopath; Check: NeedsAddPath(ExpandConstant('{app}'))

[Run]
Filename: "{app}\bac.exe"; Description: "Open the BAC previewer"; Flags: nowait postinstall skipifsilent unchecked

[Code]
function NeedsAddPath(Dir: string): Boolean;
var
  Path: string;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', Path) then
  begin
    Result := True;
    exit;
  end;
  Result := Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Path) + ';') = 0;
end;

{ takes the install folder off the user's PATH again }
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Path, Dir: string;
  I: Integer;
begin
  if CurUninstallStep <> usPostUninstall then
    exit;
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', Path) then
    exit;
  Dir := ExpandConstant('{app}');
  Path := ';' + Path + ';';
  I := Pos(';' + Uppercase(Dir) + ';', Uppercase(Path));
  if I = 0 then
    exit;
  Delete(Path, I, Length(Dir) + 1);
  Path := Copy(Path, 2, Length(Path) - 2);
  RegWriteExpandStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', Path);
end;
