; Instalador de la app interactiva de Remotix (Inno Setup 6).
;
; Instala remotix.exe (la app con ventana: iniciar sesión, ver tus PCs, conectar
; por clave y quedar accesible por su propia clave). Instalación POR USUARIO (sin
; admin/UAC), con accesos directos. La app se minimiza a la bandeja y puede
; iniciarse con Windows desde su propio checkbox.
;
; Compílalo con  infra\build-app-installer.ps1  (compila el exe con el servidor
; de producción "baked" y luego invoca ISCC sobre este .iss).

#ifndef AppVersion
  #define AppVersion "1.0.0"
#endif
#ifndef AppExe
  ; Ruta al exe ya compilado (relativa a este .iss).
  #define AppExe "..\..\agent\target\release\remotix.exe"
#endif

#define AppName "Remotix"
#define AppPublisher "HL Sistemas"
#define AppExeName "Remotix.exe"

[Setup]
AppId={{7B3F2A10-8C4E-4E2A-9D1B-REMOTIXAPP01}}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
DefaultDirName={autopf}\Remotix
DisableProgramGroupPage=yes
UninstallDisplayName={#AppName}
UninstallDisplayIcon={app}\{#AppExeName}
OutputDir=Output
OutputBaseFilename=RemotixSetup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; App interactiva: instalación por usuario, sin necesidad de administrador.
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "es"; MessagesFile: "compiler:Languages\Spanish.isl"

[Tasks]
Name: "desktopicon"; Description: "Crear un acceso directo en el escritorio"; GroupDescription: "Accesos directos:"

[Files]
Source: "{#AppExe}"; DestDir: "{app}"; DestName: "{#AppExeName}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Remotix"; Filename: "{app}\{#AppExeName}"
Name: "{autodesktop}\Remotix"; Filename: "{app}\{#AppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExeName}"; Description: "Iniciar Remotix ahora"; Flags: nowait postinstall skipifsilent
