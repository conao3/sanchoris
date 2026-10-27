FROM node:22-bookworm AS build
RUN corepack enable && corepack prepare pnpm@10.11.0 --activate
WORKDIR /src
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY apps ./apps
RUN pnpm install --frozen-lockfile --filter @sanchoris/workspace-agent \
 && pnpm --filter @sanchoris/workspace-agent deploy --legacy --prod /out \
 && node -e "const sdk = require.resolve('@anthropic-ai/claude-agent-sdk', { paths: ['/out'] }); const bin = require.resolve('@anthropic-ai/claude-agent-sdk-linux-x64/claude', { paths: [sdk] }); require('fs').symlinkSync(require('path').relative('/out', bin), '/out/claude');"

FROM node:22-bookworm
ENV NODE_ENV=production \
    NODE_USE_ENV_PROXY=1 \
    GIT_TERMINAL_PROMPT=0 \
    DISABLE_AUTOUPDATER=1 \
    CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 \
    SANCHORIS_WORKSPACE_DIR=/workspace \
    SANCHORIS_SOCKET_PATH=/tmp/sanchoris.sock \
    CLAUDE_CONFIG_DIR=/home/node/.claude \
    SANCHORIS_CLAUDE_PATH=/opt/workspace-agent/claude
COPY --from=build /out /opt/workspace-agent
RUN printf '#!/bin/sh\nexec node --disable-warning=UNDICI-EHPA /opt/workspace-agent/src/bin/sanchoris.ts "$@"\n' > /usr/local/bin/sanchoris \
 && chmod 755 /usr/local/bin/sanchoris \
 && git config --system credential.helper '!sanchoris git-credential' \
 && git config --system http.proxyAuthMethod basic \
 && git config --system user.name sanchoris \
 && git config --system user.email sanchoris@users.noreply.github.com \
 && mkdir -p /workspace /home/node/.claude \
 && chown node:node /workspace /home/node/.claude \
 && /opt/workspace-agent/claude --version
USER node
WORKDIR /workspace
ENTRYPOINT ["node", "--disable-warning=UNDICI-EHPA", "/opt/workspace-agent/src/bin/workspace-agent.ts"]
