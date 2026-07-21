# Remotix

Helpdesk de soporte remoto: el usuario comparte su pantalla y chatea con un
técnico, y opcionalmente le da **control total** (mouse y teclado). Transporte
**WebRTC** (vídeo P2P); el servidor solo hace señalización, API y relay TURN.

## Dos formas de dar soporte

| Modo | Cómo | Instalación | Qué permite |
|---|---|---|---|
| **Rápido** | El usuario abre `/ayuda` y comparte su pantalla desde el navegador | **Ninguna** (solo navegador) | El técnico **ve** la pantalla + **chat** + **archivos** |
| **Control total** | El usuario descarga y ejecuta el **agente** (un `.exe`) | Descargar y abrir | El técnico **ve + controla** (mouse/teclado) + **chat** + **archivos** |

La **transferencia de archivos** es bidireccional (DataChannel WebRTC). En el modo
agente, los archivos recibidos se guardan en `%USERPROFILE%\Downloads\Remotix`, y
al "Pedir archivo" el usuario ve un diálogo nativo para elegir qué enviar.

### Remotix Lite (tipo TeamViewer, desatendido)

`remotix-lite.exe` es un ejecutable ligero (~9.7 MB) de acceso remoto desatendido:

- Al abrirse muestra una **clave de acceso FIJA y permanente** (se registra una vez
  en el servidor y se guarda en `%APPDATA%\Remotix\lite.json`; **es siempre la misma**).
- Se **arranca con Windows** automáticamente (clave Run del registro; toggle en la ventana).
- El técnico introduce esa clave en `/operador` (campo "código o clave") y, si el equipo
  está en línea, se conecta **por internet** para **ver, controlar (mouse/teclado) y
  transferir archivos** — sin que nadie tenga que aceptar en el equipo remoto.

Flujo: el Lite mantiene una conexión a `/ws/device` (presencia); al conectar por clave,
el servidor reserva una sesión y le ordena compartir. Compílalo apuntando a tu servidor
público para uso por internet:

```powershell
infra\build-agent.ps1 -Server https://tudominio.com -Sign
```

> Seguridad: la clave fija es pública (quien la tenga puede conectarse cuando el equipo
> esté encendido), como el ID de TeamViewer. Mantenla privada; una contraseña adicional
> por equipo es una mejora pendiente.

**Pedir permiso antes de conectar** (opcional, por equipo): checkbox en la ventana del
exe. Apagado por defecto (acceso desatendido puro). Activado, cada conexión entrante
muestra en el equipo un diálogo "¿Permitir la conexión?" con 30 s para responder;
rechazo o silencio → el técnico ve "el usuario rechazó la conexión". El valor se guarda
en el servidor (columna `require_confirm`), así la ventana y el ayudante del servicio
comparten el mismo estado.

En ambos casos el usuario obtiene un **código de 6 caracteres** que le dicta al
técnico; el técnico lo introduce en `/operador` y se conecta. Sin cuentas para
el soporte instantáneo (el código es el secreto, estilo TeamViewer QuickSupport).

> El modo rápido usa `getDisplayMedia`, que requiere **HTTPS** (o `localhost`).
> Para uso por internet sirve el sitio con TLS (ver *Despliegue*). El modo agente
> no tiene esa restricción.

## Componentes

- **`server/`** — Node + TypeScript (Hono). API REST del portal, señalización
  WebSocket (`/ws/signal`), endpoint de credenciales TURN, y sirve la consola web
  y la descarga del agente. Persistencia SQLite (Drizzle).
- **`web/`** — Consola web (React + Vite). Páginas públicas `/ayuda` (usuario) y
  `/operador` (técnico), más el portal de cuentas/proyectos.
- **`agent/`** — Agente nativo de Windows (Rust): captura de pantalla (DXGI),
  codificación H.264 (OpenH264), peer WebRTC, e inyección de mouse/teclado (enigo).
- **`infra/`** — Caddy (HTTPS), coturn (TURN) y scripts de build/firma del agente.

## Arranque local (todo automático)

```powershell
# Doble clic en start.bat, o:
powershell -ExecutionPolicy Bypass -File start.ps1
```

Esto instala dependencias, compila la consola y arranca todo en
`http://localhost:8080`. Abre:

- **Usuario (pide ayuda):** http://localhost:8080/ayuda
- **Técnico (consola):** http://localhost:8080/operador

Para probar el **modo control total** en local, compila el agente y ejecútalo:

```powershell
infra\build-agent.ps1            # genera y copia server\public\remotix-agent.exe
.\agent\target\release\remotix-agent.exe    # muestra un código; dáselo al técnico
```

(Requiere [Rust](https://rustup.rs). El agente por defecto se conecta a
`ws://localhost:8080`.)

## Despliegue por internet (tu VPS + dominio)

Requisitos: Docker, un dominio con un registro **A → IP del VPS**, y puertos
abiertos: `80/tcp`, `443/tcp`, `3478/udp`, `3478/tcp`, `49160-49200/udp`.

```bash
cp .env.example .env          # rellena DOMAIN, ACME_EMAIL, SESSION_SECRET, TURN_SECRET
cd web && npm ci && npm run build && cd ..   # genera web/dist (lo sirve Caddy)
docker compose up -d --build  # server (API/WS) + Caddy (HTTPS) + coturn (TURN)
```

Caddy obtiene el certificado TLS de Let's Encrypt automáticamente. Luego compila
el agente apuntando a tu dominio y publícalo:

```powershell
infra\build-agent.ps1 -Server https://TU_DOMINIO -Sign   # ver "Firma" abajo
```

El `.exe` queda en `server/public/` y se descarga desde el botón de `/ayuda`.

## Firma del agente (importante para usuarios no técnicos)

Sin firma, Windows SmartScreen/antivirus advierte al ejecutar el `.exe`. Para
evitarlo necesitas un **certificado de firma de código** (idealmente **EV**, que
elimina la advertencia de inmediato). No se puede generar desde el proyecto.

```powershell
infra\sign.ps1 -File agent\target\release\remotix-agent.exe -Thumbprint <HUELLA>
# o: -Pfx cert.pfx -Password (Read-Host -AsSecureString)
```

`build-agent.ps1 -Sign` lo hace en un paso. Mientras no firmes, sirve para
pruebas (aceptando la advertencia).

## Desarrollo

```powershell
cd server; npm install; npm run dev    # API + WS con recarga (tsx)  → :8080
cd web;    npm install; npm run dev     # Vite con proxy /api y /ws    → :5173
```

Tests de humo de la señalización: `node server/smoke-ws.mjs` (con el server
arriba) y `node server/agent-handshake-test.mjs <CODE>` (con un agente hosteando).

## Variables de entorno (server)

| Variable | Default | Descripción |
|---|---|---|
| `PORT` | `8080` | Puerto HTTP+WS |
| `MYSQL_HOST` / `MYSQL_PORT` | `127.0.0.1` / `3306` | Servidor MySQL |
| `MYSQL_USER` / `MYSQL_PASSWORD` | `remotix` / _(vacío)_ | Credenciales MySQL |
| `MYSQL_DATABASE` | `remotix` | Base de datos |
| `SESSION_SECRET` | `dev-only-change-me` | **Obligatorio en producción** |
| `STUN_URLS` | STUN de Google | Lista separada por comas |
| `TURN_HOST` | _(vacío)_ | Host del TURN; vacío = sin TURN (solo STUN) |
| `TURN_SECRET` | _(vacío)_ | Secreto compartido con coturn (credenciales efímeras) |
| `TURN_PORT` / `TURNS_PORT` | `3478` / `5349` | Puertos TURN / TURN-TLS |
| `TURN_TTL` | `3600` | Validez (s) de la credencial TURN |

## Estado del roadmap

| Fase | Entregable | |
|---|---|---|
| 0–0.5 | Scaffold + portal (cuentas, proyectos, equipos) | ✅ |
| 1 | Señalización + handshake por código + chat | ✅ |
| 2 | Consola de control + credenciales TURN | ✅ |
| 3 | Agente nativo (WebRTC vídeo H.264 + control mouse/teclado) | ✅ |
| 4 | Despliegue internet (HTTPS + TURN) | ✅ (config) |
| 5 | Empaquetado del agente (firmado-ready) | ✅ |
| 6 | Transferencia de archivos bidireccional (ambos modos) | ✅ |
| 7 | Instalador (Inno Setup) + servicio de Windows desatendido en el boot — ver [INSTALADOR.md](INSTALADOR.md) | ✅ |
| — | Siguiente: MSI/GPO, multi-monitor avanzado, tuning de bitrate/FPS | ⏳ |
