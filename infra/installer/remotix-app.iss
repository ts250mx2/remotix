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
; Reemplazo en sitio: cierra la app en ejecución para poder sobrescribir el exe.
CloseApplications=yes
RestartApplications=no

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
; Interactivo: casilla "Iniciar Remotix" en la última página del asistente.
Filename: "{app}\{#AppExeName}"; Description: "Iniciar Remotix ahora"; Flags: nowait postinstall skipifsilent
; Silencioso (auto-actualización): relanza la app oculta en la bandeja (--tray)
; para no plantar la ventana de repente sobre lo que el usuario esté haciendo.
Filename: "{app}\{#AppExeName}"; Parameters: "--tray"; Flags: nowait; Check: WizardSilent

[Code]
// Antes de copiar, cierra cualquier Remotix.exe en ejecución (incl. el que lanza
// su propia auto-actualización) para poder reemplazar el exe sin bloqueos.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var ResultCode: Integer;
begin
  Exec('taskkill.exe', '/F /IM Remotix.exe', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Result := '';
end;
