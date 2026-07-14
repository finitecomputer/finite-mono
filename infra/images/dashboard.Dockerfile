# finite-saas-dashboard image (Next.js standalone).
# Moved from finitecomputer-v2/deploy/finite-computer/images/. Build context is
# the mono root because the dashboard consumes the shared Finite Chat UI
# package. Built ONLY by .github/workflows/service-images.yml.

FROM node:22-bookworm-slim AS deps

WORKDIR /src/finitecomputer-v2/apps/dashboard
COPY finitecomputer-v2/apps/dashboard/package.json finitecomputer-v2/apps/dashboard/package-lock.json ./
COPY finitechat/packages/finitechat-chat-ui/package.json /src/finitechat/packages/finitechat-chat-ui/package.json
COPY finitechat/packages/finitechat-chat-ui/src /src/finitechat/packages/finitechat-chat-ui/src
RUN npm ci

FROM deps AS builder

WORKDIR /src
COPY finitecomputer-v2/apps/dashboard ./finitecomputer-v2/apps/dashboard
COPY finitechat/packages/finitechat-chat-ui ./finitechat/packages/finitechat-chat-ui
WORKDIR /src/finitecomputer-v2/apps/dashboard
RUN npm run build

FROM node:22-bookworm-slim AS runner

ENV NODE_ENV=production
ENV PORT=3000
# Serve the skills catalog from the mono tree this image was built from,
# not the archived GitHub fallback (docs/audits/skills-audit-2026-07-13.md).
ENV FC_FINITE_SKILLS_SOURCE_DIR=/app/finite-skills/skills

WORKDIR /app
COPY --from=builder --chown=node:node /src/finitecomputer-v2/apps/dashboard/.next/standalone ./
COPY --from=builder --chown=node:node /src/finitecomputer-v2/apps/dashboard/.next/static ./finitecomputer-v2/apps/dashboard/.next/static
COPY --from=builder --chown=node:node /src/finitecomputer-v2/apps/dashboard/public ./finitecomputer-v2/apps/dashboard/public
COPY --chown=node:node finite-skills/skills ./finite-skills/skills

USER node
EXPOSE 3000
WORKDIR /app/finitecomputer-v2/apps/dashboard
CMD ["node", "server.js"]
