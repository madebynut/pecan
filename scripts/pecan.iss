; Windows installer for Pecan, built by scripts/bundle-windows.ps1 with Inno Setup 6.
; ISCC /DVersion=1.0.0-beta.1 /DNumericVersion=1.0.0.0 /DSource=<staged dir> /DOutput=<dir> pecan.iss
#ifndef Version
  #error Pass /DVersion=<Cargo version>
#endif
#ifndef NumericVersion
  #define NumericVersion "0.0.0.0"
#endif
#ifndef Source
  #error Pass /DSource=<directory with Pecan.exe and licenses>
#endif
#ifndef Output
  #define Output "."
#endif
#define ProgId "Pecan.Document"

[Setup]
; Keep this AppId for every release: Windows uses it to upgrade and uninstall.
AppId={{5B0E3E7C-2D1A-4C8E-9F63-7A1D4E2B9C10}
AppName=Pecan
AppVerName=Pecan {#Version}
AppVersion={#Version}
AppPublisher=madebynut
AppPublisherURL=https://github.com/madebynut/pecan
AppSupportURL=https://github.com/madebynut/pecan/issues
VersionInfoVersion={#NumericVersion}
VersionInfoProductVersion={#Version}
DefaultDirName={autopf}\Pecan
DefaultGroupName=Pecan
DisableProgramGroupPage=yes
; Installs for the current user without administrator rights; the dialog offers all users.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17134
WizardStyle=modern
SetupIconFile={#Source}\pecan.ico
UninstallDisplayIcon={app}\Pecan.exe
UninstallDisplayName=Pecan · 碧果
LicenseFile={#Source}\LICENSE.txt
OutputDir={#Output}
OutputBaseFilename=Pecan-{#Version}-Windows-x64-Setup
Compression=lzma2/max
SolidCompression=yes
ChangesAssociations=yes
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "chinesesimplified"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Source}\Pecan.exe"; DestDir: "{app}"; Flags: ignoreversion
; pecan.ico in the staging directory is only the installer's own icon.
Source: "{#Source}\LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\NOTICE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
; The AppUserModelID groups the taskbar button and Start entry with the running process.
Name: "{group}\Pecan"; Filename: "{app}\Pecan.exe"; AppUserModelID: "com.pecan.reader"
Name: "{autodesktop}\Pecan"; Filename: "{app}\Pecan.exe"; AppUserModelID: "com.pecan.reader"; Tasks: desktopicon

[Registry]
; One ProgID opens every supported type. Types are offered under "Open with" without
; replacing the user's current default; Windows asks the user before changing it.
Root: HKA; Subkey: "Software\Classes\{#ProgId}"; ValueType: string; ValueData: "Pecan 文档"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\{#ProgId}\DefaultIcon"; ValueType: string; ValueData: """{app}\Pecan.exe"",0"
Root: HKA; Subkey: "Software\Classes\{#ProgId}\shell\open\command"; ValueType: string; ValueData: """{app}\Pecan.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\Pecan.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "Pecan"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\Pecan.exe\shell\open\command"; ValueType: string; ValueData: """{app}\Pecan.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\.md\OpenWithProgids"; ValueType: none; ValueName: "{#ProgId}"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\Pecan.exe\SupportedTypes"; ValueType: string; ValueName: ".md"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.markdown\OpenWithProgids"; ValueType: none; ValueName: "{#ProgId}"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\Pecan.exe\SupportedTypes"; ValueType: string; ValueName: ".markdown"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.mdown\OpenWithProgids"; ValueType: none; ValueName: "{#ProgId}"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\Pecan.exe\SupportedTypes"; ValueType: string; ValueName: ".mdown"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.docx\OpenWithProgids"; ValueType: none; ValueName: "{#ProgId}"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\Pecan.exe\SupportedTypes"; ValueType: string; ValueName: ".docx"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.docm\OpenWithProgids"; ValueType: none; ValueName: "{#ProgId}"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\Pecan.exe\SupportedTypes"; ValueType: string; ValueName: ".docm"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.doc\OpenWithProgids"; ValueType: none; ValueName: "{#ProgId}"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\Pecan.exe\SupportedTypes"; ValueType: string; ValueName: ".doc"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.odt\OpenWithProgids"; ValueType: none; ValueName: "{#ProgId}"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\Pecan.exe\SupportedTypes"; ValueType: string; ValueName: ".odt"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.rtf\OpenWithProgids"; ValueType: none; ValueName: "{#ProgId}"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\Pecan.exe\SupportedTypes"; ValueType: string; ValueName: ".rtf"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.epub\OpenWithProgids"; ValueType: none; ValueName: "{#ProgId}"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\Pecan.exe\SupportedTypes"; ValueType: string; ValueName: ".epub"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.pdf\OpenWithProgids"; ValueType: none; ValueName: "{#ProgId}"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\Pecan.exe\SupportedTypes"; ValueType: string; ValueName: ".pdf"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.pptx\OpenWithProgids"; ValueType: none; ValueName: "{#ProgId}"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\Pecan.exe\SupportedTypes"; ValueType: string; ValueName: ".pptx"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.ppt\OpenWithProgids"; ValueType: none; ValueName: "{#ProgId}"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\Pecan.exe\SupportedTypes"; ValueType: string; ValueName: ".ppt"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.odp\OpenWithProgids"; ValueType: none; ValueName: "{#ProgId}"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\Pecan.exe\SupportedTypes"; ValueType: string; ValueName: ".odp"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.xlsx\OpenWithProgids"; ValueType: none; ValueName: "{#ProgId}"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\Pecan.exe\SupportedTypes"; ValueType: string; ValueName: ".xlsx"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.xls\OpenWithProgids"; ValueType: none; ValueName: "{#ProgId}"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\Pecan.exe\SupportedTypes"; ValueType: string; ValueName: ".xls"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.ods\OpenWithProgids"; ValueType: none; ValueName: "{#ProgId}"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\Pecan.exe\SupportedTypes"; ValueType: string; ValueName: ".ods"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.csv\OpenWithProgids"; ValueType: none; ValueName: "{#ProgId}"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\Pecan.exe\SupportedTypes"; ValueType: string; ValueName: ".csv"; ValueData: ""

[Run]
Filename: "{app}\Pecan.exe"; Description: "{cm:LaunchProgram,Pecan}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; Settings, history, logs and recovered drafts stay; they belong to the user.
Type: dirifempty; Name: "{app}"
