FROM node:20-alpine AS build
WORKDIR /app
RUN apk add --no-cache python3 make g++
COPY package.json package-lock.json* ./
RUN npm install
COPY tsconfig.json ./
COPY src ./src
RUN npm run build

FROM node:20-alpine
WORKDIR /app
ENV NODE_ENV=production
RUN apk add --no-cache python3 make g++
COPY package.json package-lock.json* ./
RUN npm install --omit=dev && apk del python3 make g++
COPY --from=build /app/dist ./dist
RUN mkdir -p /data
ENV REMOTIX_DB=/data/remotix.db
EXPOSE 8080
CMD ["node", "dist/index.js"]
