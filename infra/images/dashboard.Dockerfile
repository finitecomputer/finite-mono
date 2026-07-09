# finite-saas-dashboard image (Next.js standalone).
# Moved from finitecomputer-v2/deploy/finite-computer/images/. Build context:
# finitecomputer-v2/ (COPY paths are relative to it). Built ONLY by
# .github/workflows/service-images.yml.

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
