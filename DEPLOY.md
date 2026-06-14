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

## 7. Agente (instalable) apuntando a tu dominio
En una máquina Windows con Rust:
```powershell
infra\build-agent.ps1 -Server https://soporte.tudominio.com -Sign   # firma si tienes certificado
```
Copia `agent\target\release\remotix-agent.exe` a `server/public/remotix-agent.exe`
del VPS (o súbelo); se descargará desde `https://soporte.tudominio.com/download/remotix-agent.exe`
(botón en `/conectar`). **Sin firma**, Windows mostrará advertencia a los usuarios
(ver `infra/sign.ps1`).

## 8. Actualizar
```bash
git pull
docker compose up -d --build
```

## Troubleshooting
- **No saca certificado:** revisa que el DNS apunte al VPS y que 80/443 estén abiertos. `docker compose logs caddy`.
- **El control remoto / vídeo no conecta entre redes distintas:** suele ser el relay TURN. Verifica los puertos UDP. Si el VPS está detrás de NAT 1:1, añade al comando de `coturn` en `docker-compose.yml`: `--external-ip=<IP_PUBLICA>`.
- **better-sqlite3 falla al construir:** el `server/Dockerfile` ya instala python3/make/g++; reconstruye con `docker compose build --no-cache server`.
- **Datos:** la base vive en el volumen `remotix-data`. Respáldalo con `docker run --rm -v remotix_remotix-data:/d -v $PWD:/b alpine tar czf /b/remotix-db.tgz -C /d .`.
