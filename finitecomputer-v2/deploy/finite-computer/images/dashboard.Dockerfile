FROM node:22-bookworm-slim AS deps

WORKDIR /src/apps/dashboard
COPY apps/dashboard/package.json apps/dashboard/package-lock.json ./
RUN npm ci

FROM deps AS builder

WORKDIR /src
COPY apps/dashboard ./apps/dashboard
WORKDIR /src/apps/dashboard
RUN npm run build

FROM node:22-bookworm-slim AS runner

ENV NODE_ENV=production
ENV PORT=3000

WORKDIR /app
COPY --from=builder --chown=node:node /src/apps/dashboard/.next/standalone ./
COPY --from=builder --chown=node:node /src/apps/dashboard/.next/static ./.next/static
COPY --from=builder --chown=node:node /src/apps/dashboard/public ./public

USER node
EXPOSE 3000
CMD ["node", "server.js"]
