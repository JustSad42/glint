; Glint installer (Inno Setup 6). Built by release.ps1, which passes /DAppVersion=x.y.z
; Installs for all users into Program Files, because the file indexer is a Windows service
; (it runs as SYSTEM to read the drives' file tables, so its exe must not be user-writable).
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#define AppName "Glint"
#define AppExe "Glint.exe"
#define IndexerExe "glint-indexer.exe"
#define RepoUrl "https://github.com/JustSad42/glint"

[Setup]
AppId={{6E2B1C4A-9F3D-4B7E-A1C8-52D9E0F7B3A6}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=JustSad42
AppPublisherURL={#RepoUrl}
AppSupportURL={#RepoUrl}/issues
AppUpdatesURL={#RepoUrl}/releases
PrivilegesRequired=admin
DefaultDirName={autopf}\{#AppName}
DisableProgramGroupPage=yes
DisableDirPage=auto
UsePreviousAppDir=yes
OutputDir=..\dist
OutputBaseFilename=Glint-{#AppVersion}-setup
SetupIconFile=..\rust\app\icons\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
CloseApplications=no
RestartApplications=no

[Files]
Source: "..\rust\target\release\glint.exe"; DestDir: "{app}"; DestName: "{#AppExe}"; Flags: ignoreversion
Source: "..\rust\target\release\glint-indexer.exe"; DestDir: "{app}"; DestName: "{#IndexerExe}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autoprograms}\{#AppName} Settings"; Filename: "{app}\{#AppExe}"; Parameters: "--settings"

[Run]
Filename: "{app}\{#IndexerExe}"; Parameters: "--install"; Flags: runhidden waituntilterminated; StatusMsg: "Starting the file indexer..."
; start-with-Windows is per user, so it runs as the signed-in user, not the elevated installer
Filename: "{app}\{#AppExe}"; Parameters: "--setup"; Flags: runhidden waituntilterminated runasoriginaluser
Filename: "{app}\{#AppExe}"; Description: "Open {#AppName}"; Flags: nowait postinstall skipifsilent runasoriginaluser
; after an in-app update (silent install), start again quietly in the tray
Filename: "{app}\{#AppExe}"; Parameters: "--tray"; Flags: nowait skipifnotsilent runasoriginaluser

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/f /im {#AppExe}"; Flags: runhidden; RunOnceId: "StopApp"
Filename: "{app}\{#IndexerExe}"; Parameters: "--uninstall"; Flags: runhidden waituntilterminated; RunOnceId: "RemoveService"
Filename: "{app}\{#AppExe}"; Parameters: "--remove-integration"; Flags: runhidden; RunOnceId: "RemoveIntegration"

[UninstallDelete]
; logs, and the rebuildable caches; your settings and history stay in case you reinstall
Type: filesandordirs; Name: "{commonappdata}\Glint"
Type: filesandordirs; Name: "{localappdata}\Glint\content-index"
Type: filesandordirs; Name: "{localappdata}\Glint\icons"
Type: files; Name: "{localappdata}\Glint\content-manifest.json"
Type: files; Name: "{localappdata}\Glint\apps.json"
Type: files; Name: "{localappdata}\Glint\update_state.json"

[Code]
// Stop the old indexer service and the running app so their files can be replaced.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
  OldIndexer: String;
begin
  OldIndexer := ExpandConstant('{app}\{#IndexerExe}');
  if FileExists(OldIndexer) then
    Exec(OldIndexer, '--stop', '', SW_HIDE, ewWaitUntilTerminated, Code)
  else
    Exec(ExpandConstant('{sys}\sc.exe'), 'stop GlintIndexer', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/f /im {#AppExe}', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Sleep(800);
  Result := '';
end;
