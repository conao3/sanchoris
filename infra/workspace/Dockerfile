FROM --platform=$BUILDPLATFORM node:22-bookworm@sha256:63538dfe497cc29dfafdda25ada09b46db68a3f4cc205abf1f5d6bf0913f3384 AS build
ARG TARGETARCH
RUN corepack enable && corepack prepare pnpm@10.11.0 --activate
WORKDIR /src
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY apps ./apps
COPY infra/workspace/entrypoint.sh /rootfs/usr/local/bin/workspace-entrypoint
RUN case "$TARGETARCH" in \
      amd64) cpu=x64 machine=62 ;; \
      arm64) cpu=arm64 machine=183 ;; \
      *) echo "unsupported TARGETARCH $TARGETARCH" >&2; exit 1 ;; \
    esac \
 && printf 'supportedArchitectures:\n  os: [linux]\n  cpu: [%s]\n  libc: [glibc]\n' "$cpu" >> pnpm-workspace.yaml \
 && pnpm install --frozen-lockfile --filter @sanchoris/workspace-agent \
 && pnpm --filter @sanchoris/workspace-agent deploy --legacy --prod /out \
 && node -e "const sdk = require.resolve('@anthropic-ai/claude-agent-sdk', { paths: ['/out'] }); const bin = require.resolve('@anthropic-ai/claude-agent-sdk-linux-$cpu/claude', { paths: [sdk] }); require('fs').symlinkSync(require('path').relative('/out', bin), '/out/claude');" \
 && node -e "const elf = require('fs').readFileSync('/out/claude').subarray(0, 20); if (elf.toString('latin1', 1, 4) !== 'ELF' || elf.readUInt16LE(18) !== $machine) { console.error('the bundled Claude Code is not a linux-$cpu binary'); process.exit(1); }" \
 && if [ "$(dpkg --print-architecture)" = "$TARGETARCH" ]; then /out/claude --version; fi
RUN dpkg --add-architecture "$TARGETARCH" \
 && apt-get update \
 && cd /tmp \
 && apt-get download "tini:$TARGETARCH" \
 && dpkg-deb -x tini_*.deb /tmp/tini \
 && install -D -m 755 /tmp/tini/usr/bin/tini /rootfs/usr/bin/tini \
 && rm -rf /var/lib/apt/lists/* /tmp/tini tini_*.deb
RUN printf '#!/bin/sh\nexec node --disable-warning=UNDICI-EHPA /opt/workspace-agent/src/bin/sanchoris.ts "$@"\n' > /rootfs/usr/local/bin/sanchoris \
 && chmod 755 /rootfs/usr/local/bin/sanchoris /rootfs/usr/local/bin/workspace-entrypoint \
 && mkdir -p /rootfs/etc /home-dirs \
 && git config --file /rootfs/etc/gitconfig credential.helper '!sanchoris git-credential' \
 && git config --file /rootfs/etc/gitconfig user.name sanchoris \
 && git config --file /rootfs/etc/gitconfig user.email sanchoris@users.noreply.github.com

FROM node:22-bookworm@sha256:63538dfe497cc29dfafdda25ada09b46db68a3f4cc205abf1f5d6bf0913f3384
ENV NODE_ENV=production \
    NODE_USE_ENV_PROXY=1 \
    HTTPS_PROXY=http://127.0.0.1:3129 \
    HTTP_PROXY=http://127.0.0.1:3129 \
    NO_PROXY=127.0.0.1,localhost \
    GIT_TERMINAL_PROMPT=0 \
    DISABLE_AUTOUPDATER=1 \
    CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 \
    SANCHORIS_WORKSPACE_DIR=/workspace \
    SANCHORIS_SOCKET_PATH=/tmp/sanchoris.sock \
    CLAUDE_CONFIG_DIR=/home/node/.claude \
    SANCHORIS_CLAUDE_PATH=/opt/workspace-agent/claude
COPY --from=build /out /opt/workspace-agent
COPY --from=build /rootfs/ /
COPY --from=build --chown=node:node /home-dirs/ /workspace/
COPY --from=build --chown=node:node /home-dirs/ /home/node/.claude/
USER node
WORKDIR /workspace
ENTRYPOINT ["/usr/local/bin/workspace-entrypoint"]
