# Compila la consola web (SPA) y la hornea en una imagen de Caddy.
# Contexto de build = raíz del repo.  (Así el VPS solo necesita Docker, sin Node.)
FROM node:22-alpine AS build
WORKDIR /web
COPY web/package.json web/package-lock.json* ./
RUN npm ci || npm install
COPY web/ ./
RUN npm run build

FROM caddy:2-alpine
COPY --from=build /web/dist /srv
# El Caddyfile se monta en runtime (ver docker-compose.yml).
