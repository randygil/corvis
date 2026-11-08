# Builds the documentation site and serves it with nginx.
# Build context: the docs/ folder (`docker build -t corvis-docs docs`).
FROM node:22-alpine AS build
WORKDIR /docs
COPY package.json package-lock.json ./
RUN npm ci
COPY . .
RUN npm run docs:check

FROM nginx:1.27-alpine
COPY nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /docs/.vitepress/dist /usr/share/nginx/html
EXPOSE 80
