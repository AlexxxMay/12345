; Установщик AlphaCad для Windows x64 (Inno Setup 6.7+ / 7.x — тёмная тема и фон).
; Сборка: iscc /DMyAppVersion=0.2.0 installer\windows\alphacad.iss

#if Ver < EncodeVer(6,7,0)
  #error Требуется Inno Setup 6.7 или новее (WizardBackImageFile)
#endif

#define MyAppName "AlphaCad"
#ifndef MyAppVersion
  #define MyAppVersion "0.2.0"
#endif
#define MyAppExe "alphacad.exe"

[Setup]
AppId={{3E8A51C4-92D7-4F0B-B6A3-71C5D9E24A80}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher=Alex Abmaev
AppPublisherURL=https://github.com/AlexxxMay/12345
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\{#MyAppExe}
UninstallDisplayName={#MyAppName} {#MyAppVersion}
OutputDir=..\..\target\installer
OutputBaseFilename=AlphaCad-{#MyAppVersion}-windows-x64-setup
SetupIconFile=..\..\assets\icon\alphacad.ico
Compression=lzma2/max
SolidCompression=yes
ArchitecturesAllowed=x64
ArchitecturesInstallIn64BitMode=x64
MinVersion=6.1sp1
PrivilegesRequired=admin
PrivilegesRequiredOverridesAllowed=dialog
; Тёмная тема мастера установки, логотип на фоне окна
WizardStyle=modern dark hidebevels
WizardBackColor=#1e1f22
WizardBackImageFile=images\wizard-back-100.png,images\wizard-back-150.png,images\wizard-back-200.png,images\wizard-back-250.png
WizardImageFile=
WizardSmallImageFile=
ChangesAssociations=yes
VersionInfoVersion={#MyAppVersion}
VersionInfoProductName={#MyAppName}
VersionInfoDescription={#MyAppName} Setup

[Languages]
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
russian.AssocTask=Открывать файлы STL, STEP, 3MF, OBJ, PLY и .acad в AlphaCad
english.AssocTask=Open STL, STEP, 3MF, OBJ, PLY and .acad files with AlphaCad
russian.AssocGroup=Ассоциации файлов:
english.AssocGroup=File associations:
russian.ModelType=3D-модель
english.ModelType=3D model
russian.ProjectType=Проект AlphaCad
english.ProjectType=AlphaCad project

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "assoc"; Description: "{cm:AssocTask}"; GroupDescription: "{cm:AssocGroup}"

[Files]
Source: "..\..\target\release\{#MyAppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; DestName: "README.txt"; Flags: ignoreversion
Source: "..\..\assets\fonts\LICENSE-DejaVu.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"; Tasks: desktopicon

[Registry]
; Тип «3D-модель»
Root: HKA; Subkey: "Software\Classes\AlphaCad.Model"; ValueType: string; ValueName: ""; ValueData: "{cm:ModelType}"; Flags: uninsdeletekey; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\AlphaCad.Model\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#MyAppExe},0"; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\AlphaCad.Model\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExe}"" ""%1"""; Tasks: assoc
; Тип «Проект AlphaCad»
Root: HKA; Subkey: "Software\Classes\AlphaCad.Project"; ValueType: string; ValueName: ""; ValueData: "{cm:ProjectType}"; Flags: uninsdeletekey; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\AlphaCad.Project\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#MyAppExe},0"; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\AlphaCad.Project\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExe}"" ""%1"""; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.acad"; ValueType: string; ValueName: ""; ValueData: "AlphaCad.Project"; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.forge\OpenWithProgids"; ValueType: string; ValueName: "AlphaCad.Project"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
; «Открыть с помощью» для моделей
Root: HKA; Subkey: "Software\Classes\.stl\OpenWithProgids"; ValueType: string; ValueName: "AlphaCad.Model"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.step\OpenWithProgids"; ValueType: string; ValueName: "AlphaCad.Model"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.stp\OpenWithProgids"; ValueType: string; ValueName: "AlphaCad.Model"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.3mf\OpenWithProgids"; ValueType: string; ValueName: "AlphaCad.Model"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.obj\OpenWithProgids"; ValueType: string; ValueName: "AlphaCad.Model"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.ply\OpenWithProgids"; ValueType: string; ValueName: "AlphaCad.Model"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\Applications\{#MyAppExe}\SupportedTypes"; ValueType: string; ValueName: ".stl"; ValueData: ""; Flags: uninsdeletekey; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\Applications\{#MyAppExe}\SupportedTypes"; ValueType: string; ValueName: ".step"; ValueData: ""; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\Applications\{#MyAppExe}\SupportedTypes"; ValueType: string; ValueName: ".stp"; ValueData: ""; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\Applications\{#MyAppExe}\SupportedTypes"; ValueType: string; ValueName: ".obj"; ValueData: ""; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\Applications\{#MyAppExe}\SupportedTypes"; ValueType: string; ValueName: ".ply"; ValueData: ""; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\Applications\{#MyAppExe}\SupportedTypes"; ValueType: string; ValueName: ".acad"; ValueData: ""; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\Applications\{#MyAppExe}\SupportedTypes"; ValueType: string; ValueName: ".3mf"; ValueData: ""; Tasks: assoc

[Run]
Filename: "{app}\{#MyAppExe}"; Description: "{cm:LaunchProgram,{#MyAppName}}"; Flags: nowait postinstall skipifsilent
