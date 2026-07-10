# Actualizar Remotix en el VPS

Runbook para desplegar una versión nueva en el VPS (modo **híbrido**: Docker para
`server` + `coturn`, detrás del **Nginx propio** del VPS, base de datos **MySQL**).

## Publicar una versión nueva de la APP (auto-actualización de la flota)

1. Sube `version` en `agent/Cargo.toml` (única fuente de verdad).
2. `infra\build-app-installer.ps1 -Version X.Y.Z [-Sign]` — compila, genera
   `RemotixSetup.exe` y publica exe + manifiesto en `server/public`.
3. Commit + push, y en el VPS los pasos de abajo (git pull + rebuild del server).

Al arrancar el server nuevo, **avisa por WebSocket a todos los equipos
conectados** (y a los que se reconecten, y cada 10 min a los rezagados). Cada
Remotix con versión vieja:
- **inactivo** (en la bandeja, sin sesión remota ni visor) → se actualiza SOLO
  en silencio y se relanza oculto (`--tray`);
- **en uso** → muestra la tarjeta "Actualizar ahora" y se auto-aplica en cuanto
  queda inactivo.
- `"mandatory": true` en `server/public/remotix-latest.json` → se aplica aunque
  la ventana esté visible (nunca a mitad de una sesión remota).

La instalación con **servicio de Windows** (RemotixHostSetup) usa el canal
separado `remotix-host-latest.json`; si ese manifiesto no existe, no se toca.

> ⚠️ Desde la versión TeamViewer, el server usa **MySQL** (ya no SQLite). Asegúrate
> de tener las variables `MYSQL_*` en el `.env` o el server **no arrancará**.

## Paso 0 — En tu PC: subir el código
```powershell
git checkout main
git merge feat/teamviewer-mysql      # si vienes de una rama
git push origin main                 # sube código + server/public/remotix.exe
```

## En el VPS (por SSH)

### 1. Traer los cambios
```bash
cd ~/remotix          # ajusta a la ruta donde clonaste el repo
git pull
```

### 2. Variables MySQL en el `.env` (obligatorio)
```bash
nano .env
```
Añade (junto a `DOMAIN`, `SESSION_SECRET`, `TURN_SECRET`, `ACME_EMAIL` que ya tenías):
```
MYSQL_HOST=74.208.192.90
MYSQL_PORT=3306
MYSQL_USER=kyk
MYSQL_PASSWORD=tu_password
MYSQL_DATABASE=BDRemotix
```
Puedes borrar la línea vieja `REMOTIX_DB` (ya no se usa).

### 3. Reconstruir y reiniciar server + coturn (sin Caddy)
```bash
docker compose -f docker-compose.yml -f docker-compose.nginx.yml up -d --build server coturn
```

### 4. Verificar
```bash
docker compose -f docker-compose.yml -f docker-compose.nginx.yml logs --tail=20 server
curl -s http://127.0.0.1:8080/health
```
Debes ver `[db]   MySQL conectado a 74.208.192.90:3306/BDRemotix` y `listening …`,
y el health debe devolver `{"ok":true}`. El esquema se crea solo (idempotente).

### 5. Recompilar la web (cambió la SPA)
```bash
docker run --rm -v "$PWD/web":/web -w /web node:22-alpine sh -lc "npm ci || npm install; npm run build"
sudo cp -r web/dist/* /var/www/remotix/
```

### 6. Comprobar el sitio público
```bash
curl -sI https://remotix.hlsistemas.com/health                 # 200
curl -sI https://remotix.hlsistemas.com/download/remotix.exe   # 200
```

## Notas importantes
- **NO** uses `docker compose up -d` a secas: arrancaría Caddy y chocaría con tu
  Nginx en 80/443. Usa **siempre** `-f docker-compose.yml -f docker-compose.nginx.yml … server coturn`.
- No hay que tocar Nginx: las rutas (`/api`, `/ws`, `/download`, SPA) no cambiaron.
- `MYSQL_HOST=74.208.192.90` funciona desde Docker (es un host accesible por red).
  Solo si tu MySQL estuviera en `localhost` del propio VPS necesitarías `host.docker.internal`.
- El `remotix.exe` viene versionado en el repo (`server/public/remotix.exe`), baked a
  tu dominio; se actualiza con el `git pull` y se sirve en `/download/remotix.exe`.
- Antes de distribuir a usuarios reales, **firma** el exe:
  `infra\build-agent.ps1 -Server https://remotix.hlsistemas.com -Sign`.

---

## Apéndice — Vaciar datos de demo en `BDRemotix`
Si quedaron datos de prueba, vacía las tablas (conserva el esquema). Desde cualquier
cliente MySQL conectado a `BDRemotix`:
```sql
SET FOREIGN_KEY_CHECKS=0;
TRUNCATE device_access; TRUNCATE group_members; TRUNCATE sessions;
TRUNCATE devices; TRUNCATE `groups`; TRUNCATE users;
TRUNCATE messages; TRUNCATE channel_members; TRUNCATE channels;
TRUNCATE enrollment_tokens; TRUNCATE equipos; TRUNCATE project_members; TRUNCATE projects;
SET FOREIGN_KEY_CHECKS=1;
```
