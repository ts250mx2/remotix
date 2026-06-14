# Despliegue de Remotix en tu VPS

Pone en marcha todo (consola web + API + chat + señalización + TURN) con HTTPS
automático, en un solo comando. **Solo necesitas Docker en el VPS** (la web se
compila dentro de la imagen; no hace falta Node).

## 1. Requisitos
- Un VPS Linux con **Docker** y **docker compose** (v2).
- Un **dominio** (ej. `soporte.tudominio.com`).
- Poder editar el **DNS** del dominio y abrir puertos en el firewall.

## 2. DNS
Crea un registro **A**: `soporte.tudominio.com` → **IP pública del VPS**.
(Espera a que propague: `ping soporte.tudominio.com` debe devolver tu IP.)

## 3. Firewall
Abre en el VPS:
| Puerto | Protocolo | Para |
|---|---|---|
| 80, 443 | tcp | Web + HTTPS (Let's Encrypt) |
| 3478 | udp y tcp | TURN/STUN |
| 49160–49200 | udp | Relay de medios TURN |

Ejemplo con `ufw`:
```bash
sudo ufw allow 80,443/tcp
sudo ufw allow 3478
sudo ufw allow 49160:49200/udp
```

## 4. Configuración
```bash
git clone <tu-repo> remotix && cd remotix
cp .env.example .env
```
Edita `.env`:
```
DOMAIN=soporte.tudominio.com
ACME_EMAIL=tu-email@tudominio.com
SESSION_SECRET=<pega aquí: openssl rand -hex 32>
TURN_SECRET=<pega aquí: openssl rand -hex 32>
```
Genera los secretos:
```bash
openssl rand -hex 32   # SESSION_SECRET
openssl rand -hex 32   # TURN_SECRET (distinto)
```

## 5. Levantar
```bash
docker compose up -d --build
```
Caddy obtendrá el certificado TLS automáticamente en el primer arranque.
Ver logs: `docker compose logs -f caddy server coturn`.

## 6. Verificar
- `https://soporte.tudominio.com/` → portal (regístrate, crea una empresa, copia su **UUID**).
- `/chat` y `/operador` → consola del técnico.
- `/conectar` → cliente: pega el UUID y chatea como su PC.
- Salud: `curl https://soporte.tudominio.com/health` → `{"ok":true}`.

## 7. Remotix (exe único cliente/servidor) apuntando a tu dominio
En una máquina Windows con Rust:
```powershell
infra\build-agent.ps1 -Server https://soporte.tudominio.com -Sign   # firma si tienes certificado
```
Genera `agent\target\release\remotix.exe` y lo copia a `server/public/remotix.exe`
(súbelo al VPS si compilas en otra máquina); se descarga desde
`https://soporte.tudominio.com/download/remotix.exe` (enlace en `/` y `/ayuda`).
Un solo programa para todos: **sin login** solo acepta conexiones (muestra su clave);
**con login** además se conecta a las PCs que le dieron acceso (visor nativo).
**Sin firma**, Windows mostrará advertencia a los usuarios (ver `infra/sign.ps1`).

## 8. Actualizar (modo híbrido, detrás de tu Nginx)
```bash
git pull
# 1) .env: asegúrate de tener las variables MySQL (ver paso 5). Sin ellas el server no arranca.
# 2) Reconstruye y reinicia SOLO server + coturn (NO Caddy):
docker compose -f docker-compose.yml -f docker-compose.nginx.yml up -d --build server coturn
docker compose -f docker-compose.yml -f docker-compose.nginx.yml logs --tail=20 server   # "[db] MySQL conectado…"
curl -s http://127.0.0.1:8080/health    # {"ok":true}
# 3) Recompila la web (cambió la SPA) y cópiala donde la sirve Nginx:
docker run --rm -v "$PWD/web":/web -w /web node:22-alpine sh -lc "npm ci || npm install; npm run build"
sudo cp -r web/dist/* /var/www/remotix/
# 4) El remotix.exe viene versionado en el repo; se sirve en /download/remotix.exe.
```
No hace falta tocar Nginx (las rutas no cambiaron). El esquema MySQL se crea solo al arrancar.

## Alternativa: detrás de tu propio Nginx (sin Caddy)

Si el VPS ya tiene **Nginx** sirviendo otros sitios (ocupa 80/443), no uses el
Caddy de este compose. Nginx hace de frontal TLS y el `server` corre solo en
localhost; coturn sigue igual (modo host).

1. **Compila la consola web** (sin instalar Node) y déjala donde Nginx la sirva:
   ```bash
   docker run --rm -v "$PWD/web":/web -w /web node:22-alpine sh -lc "npm ci || npm install; npm run build"
   sudo mkdir -p /var/www/remotix && sudo cp -r web/dist/* /var/www/remotix/
   ```
2. **Levanta solo server + coturn** (exponiendo el server en localhost vía el override):
   ```bash
   docker compose -f docker-compose.yml -f docker-compose.nginx.yml up -d --build server coturn
   curl -s http://127.0.0.1:8080/health      # {"ok":true}
   ```
3. **Configura el sitio de Nginx** con la plantilla incluida (ajusta dominio y `root`):
   ```bash
   sudo cp infra/nginx-remotix.conf.example /etc/nginx/sites-available/remotix.hlsistemas.com
   sudo ln -s /etc/nginx/sites-available/remotix.hlsistemas.com /etc/nginx/sites-enabled/
   sudo nginx -t && sudo systemctl reload nginx
   sudo certbot --nginx -d remotix.hlsistemas.com    # TLS 443 + redirect 80->443
   ```
4. **Firewall:** con Nginx ya tienes 80/443; abre solo el TURN:
   ```bash
   sudo ufw allow 3478 && sudo ufw allow 49160:49200/udp
   ```

Notas: entra siempre por `https://` (la cookie de sesión es `Secure`). **No** corras
`docker compose up -d` a secas en este modo: levantaría también Caddy y chocaría con
Nginx en 80/443 — nombra siempre `server coturn`. Al actualizar la web, repite el paso 1.

## Troubleshooting
- **No saca certificado:** revisa que el DNS apunte al VPS y que 80/443 estén abiertos. `docker compose logs caddy`.
- **El control remoto / vídeo no conecta entre redes distintas:** suele ser el relay TURN. Verifica los puertos UDP. Si el VPS está detrás de NAT 1:1, añade al comando de `coturn` en `docker-compose.yml`: `--external-ip=<IP_PUBLICA>`.
- **El server no arranca / `ECONNREFUSED`:** revisa `MYSQL_HOST/PORT/USER/PASSWORD/DATABASE` en `.env`. Desde un contenedor, `MYSQL_HOST=127.0.0.1` apunta al propio contenedor: usa la IP del host o `host.docker.internal`. El usuario MySQL debe tener `GRANT ALL ON remotix.*`.
- **Datos:** la base es MySQL (servidor externo). Respáldala con `mysqldump remotix > remotix.sql`. El esquema se crea solo al arrancar (idempotente).
