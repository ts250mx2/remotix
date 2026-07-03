# Instalador desatendido de Remotix

Remotix se instala y queda **corriendo solo desde el arranque del equipo, sin
ninguna interacción humana** (acceso desatendido estilo TeamViewer). Al terminar
la instalación el equipo ya es accesible por su **clave fija**, incluso tras
reiniciar y antes de que nadie inicie sesión.

## Cómo funciona (arquitectura)

```
Instalador (RemotixSetup.exe, requiere admin)
   └─ copia  Remotix.exe → C:\Program Files\Remotix\
   └─ ejecuta  Remotix.exe install   → registra y arranca el servicio

Servicio de Windows "Remotix"  (LocalSystem, arranque automático en el boot)
   ├─ Registra el equipo y guarda su identidad/clave en HKLM (compartida)
   ├─ Sigue la sesión interactiva ACTIVA y lanza ahí el ayudante:
   │     · usuario logueado  → en su escritorio (CreateProcessAsUser)
   │     · pantalla de login → como SYSTEM en winsta0\Winlogon
   └─ Relanza el ayudante al cambiar de sesión (login/logout) o si muere

Remotix.exe helper  (sesión interactiva, solo icono en la bandeja)
   ├─ Presencia permanente contra /ws/device
   └─ Al recibir la clave, hospeda la sesión (pantalla + control + archivos)
```

Puntos clave:

- **Arranca en el boot**, no solo al iniciar sesión: es un servicio, no una clave
  `Run`. Puedes conectarte aunque el equipo esté en la pantalla de login.
- **Sigue la sesión activa**: la captura y el control siempre corren en el
  escritorio del usuario logueado (que es donde Windows permite capturar e
  inyectar teclado/ratón). Al cambiar de usuario, el ayudante se relanza solo.
- **Identidad compartida en HKLM**: el servicio (SYSTEM) y el ayudante (usuario)
  usan la misma clave. Reinstalar **conserva la misma clave** del equipo.
- **Sin ventana**: durante el uso normal solo se ve un icono discreto en la
  bandeja del sistema con el estado y la clave.

## Requisitos para generar el instalador

- Rust (https://rustup.rs) con el target MSVC.
- Inno Setup 6:  `winget install JRSoftware.InnoSetup`  (o https://jrsoftware.org/isdl.php).

## Generar el instalador

```powershell
# Servidor de produccion baked por defecto (wss://remotix.hlsistemas.com)
infra\build-installer.ps1

# Otro servidor
infra\build-installer.ps1 -Server wss://soporte.midominio.com

# Con version y firma de codigo (recomendado para produccion: evita SmartScreen)
infra\build-installer.ps1 -Version 1.2.0 -Sign
```

Salida: `infra\installer\Output\RemotixSetup.exe`.

## Instalar en el equipo remoto

Con asistente (doble clic) o de forma **totalmente silenciosa**:

```cmd
:: Silencioso, sin barra de progreso ni cuadros de dialogo (ideal para desplegar)
RemotixSetup.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART
```

Tras la instalación (silenciosa o no), el servicio queda **arrancado y en
automático**; no hace falta reiniciar. Al reiniciar, arranca solo.

## Desinstalar

Desde "Aplicaciones instaladas" de Windows, o silencioso:

```cmd
"C:\Program Files\Remotix\unins000.exe" /VERYSILENT
```

El desinstalador detiene y elimina el servicio antes de borrar los archivos. La
identidad/clave en `HKLM\SOFTWARE\Remotix` **no se borra** (para que reinstalar
recupere la misma clave). Para eliminarla del todo, borra esa clave del registro.

## Comprobación y diagnóstico

```powershell
sc query Remotix                         # estado del servicio
Get-Content "$env:ProgramData\Remotix\service.log"   # log del servicio
```

El servicio escribe un log en `%ProgramData%\Remotix\service.log` (arranque,
lanzamiento del ayudante por sesión, errores). Útil para diagnosticar in situ.

## Subcomandos del exe (para referencia)

`Remotix.exe` es un único binario con varios modos:

| Comando                | Quién lo usa            | Qué hace                                          |
|------------------------|-------------------------|---------------------------------------------------|
| `Remotix.exe install`  | instalador (admin)      | Registra y arranca el servicio                    |
| `Remotix.exe uninstall`| desinstalador (admin)   | Detiene y elimina el servicio                     |
| `Remotix.exe service`  | el SCM de Windows       | Ejecuta el servicio (seguimiento de sesión)       |
| `Remotix.exe helper`   | el servicio             | Ayudante en bandeja (presencia + hosting)         |
| `Remotix.exe`          | uso manual / pruebas    | Ventana clásica con la clave (modo QuickSupport)  |
| `Remotix.exe console`  | pruebas                 | Sin ventana, imprime clave/estado por consola     |

## Limitaciones conocidas

- **Captura en la pantalla de bloqueo mientras hay un usuario logueado**: cuando
  el usuario bloquea el equipo (Win+L), Windows muestra el escritorio seguro
  (propiedad de SYSTEM). El ayudante corre como ese usuario y puede ver una
  pantalla en negro hasta que se desbloquee. La pantalla de login inicial (sin
  usuario) sí se captura porque ahí el ayudante corre como SYSTEM.
- **Firma de código**: sin `-Sign`, Windows SmartScreen advertirá al instalar.
  Usa un certificado de firma para producción.
