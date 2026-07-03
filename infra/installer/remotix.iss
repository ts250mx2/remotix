; Instalador de Remotix (Inno Setup 6) — acceso remoto desatendido.
;
; Instala Remotix.exe en Archivos de programa, registra el servicio de Windows
; "Remotix" (arranque automático en el boot) y lo pone en marcha. A partir de
; ahí el equipo queda accesible sin ninguna interacción humana: el servicio se
; encarga de registrar la clave, seguir la sesión activa y lanzar el ayudante.
;
; Compílalo con  infra\build-installer.ps1  (que primero compila el exe con el
; servidor de producción "baked" y luego invoca ISCC sobre este .iss).

#ifndef AppVersion
  #define AppVersion "1.0.0"
#endif
#ifndef AppExe
  ; Ruta al exe ya compilado (relativa a este .iss).
  #define AppExe "..\..\agent\target\release\remotix-lite.exe"
#endif

#define AppName "Remotix Host (desatendido)"
#define AppPublisher "HL Sistemas"
#define AppExeName "Remotix.exe"

[Setup]
AppId={{7B3F2A10-8C4E-4E2A-9D1B-REMOTIX00001}}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
DefaultDirName={autopf}\Remotix
DisableProgramGroupPage=yes
DisableDirPage=yes
DisableReadyPage=no
UninstallDisplayName={#AppName}
UninstallDisplayIcon={app}\{#AppExeName}
OutputDir=Output
OutputBaseFilename=RemotixHostSetup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; El servicio corre como LocalSystem y escribe en HKLM: requiere elevación.
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "es"; MessagesFile: "compiler:Languages\Spanish.isl"

[Files]
Source: "{#AppExe}"; DestDir: "{app}"; DestName: "{#AppExeName}"; Flags: ignoreversion

[Run]
; Registra y arranca el servicio tras copiar los archivos.
Filename: "{app}\{#AppExeName}"; Parameters: "install"; \
    Flags: runhidden waituntilterminated; \
    StatusMsg: "Instalando el servicio Remotix y poniéndolo en marcha…"

[UninstallRun]
; Detiene y elimina el servicio antes de borrar los archivos.
Filename: "{app}\{#AppExeName}"; Parameters: "uninstall"; \
    Flags: runhidden waituntilterminated; RunOnceId: "RemotixServiceUninstall"

[UninstallDelete]
; Limpia el log del servicio (no borra la identidad en HKLM: así reinstalar
; conserva la MISMA clave de acceso del equipo).
Type: filesandordirs; Name: "{commonappdata}\Remotix"

[Code]
// En una ACTUALIZACIÓN el servicio está en marcha y bloquea Remotix.exe. Antes de
// copiar los archivos (ssInstall) detenemos y eliminamos el servicio ejecutando el
// exe ANTIGUO (que aún se puede lanzar aunque el archivo esté bloqueado para
// escritura). Así el exe queda libre para reemplazarlo; luego [Run] lo reinstala.
procedure CurStepChanged(CurStep: TSetupStep);
var
  ResultCode: Integer;
  Exe: String;
begin
  if CurStep = ssInstall then
  begin
    Exe := ExpandConstant('{app}\Remotix.exe');
    if FileExists(Exe) then
    begin
      Exec(Exe, 'uninstall', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
      Sleep(1500);
    end;
  end;
end;
