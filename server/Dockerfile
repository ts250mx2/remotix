FROM node:20-alpine AS build
WORKDIR /app
COPY package.json package-lock.json* ./
RUN npm install
COPY tsconfig.json ./
COPY src ./src
RUN npm run build

FROM node:20-alpine
WORKDIR /app
ENV NODE_ENV=production
COPY package.json package-lock.json* ./
RUN npm install --omit=dev
COPY --from=build /app/dist ./dist
EXPOSE 8080
# La base de datos es MySQL externo (variables MYSQL_* por entorno). Ya no hay
# SQLite ni módulos nativos que compilar.
CMD ["node", "dist/index.js"]
