FROM --platform=$BUILDPLATFORM node:26-bookworm@sha256:52dd743dc3b494cba5bc8c13507ac984507addb8a087c626dc98dcffa53133d4 AS build
ARG TARGETARCH
RUN npm install --global pnpm@12.9.0
WORKDIR /src
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml vite.config.ts tsconfig.base.json tsconfig.json ./
COPY apps ./apps
COPY infra/workspace/entrypoint.sh /rootfs/usr/local/bin/workspace-entrypoint
RUN case "$TARGETARCH" in \
      amd64) cpu=x64 machine=62 ;; \
      arm64) cpu=arm64 machine=183 ;; \
      *) echo "unsupported TARGETARCH $TARGETARCH" >&2; exit 1 ;; \
    esac \
 && printf 'supportedArchitectures:\n  os: [linux]\n  cpu: [current, %s]\n  libc: [glibc]\n' "$cpu" >> pnpm-workspace.yaml \
 && pnpm install --frozen-lockfile --ignore-scripts --filter sanchoris --filter @sanchoris/workspace-agent \
 && node node_modules/vite-plus/bin/vp pack --filter workspace-agent \
 && cp -r apps/workspace-agent/dist /out \
 && node -e "const sdk = require.resolve('@anthropic-ai/claude-agent-sdk', { paths: ['apps/workspace-agent'] }); const bin = require.resolve('@anthropic-ai/claude-agent-sdk-linux-$cpu/claude', { paths: [sdk] }); require('fs').copyFileSync(bin, '/out/claude'); require('fs').chmodSync('/out/claude', 0o755);" \
 && node -e "const elf = require('fs').readFileSync('/out/claude').subarray(0, 20); if (elf.toString('latin1', 1, 4) !== 'ELF' || elf.readUInt16LE(18) !== $machine) { console.error('the bundled Claude Code is not a linux-$cpu binary'); process.exit(1); }" \
 && if [ "$(dpkg --print-architecture)" = "$TARGETARCH" ]; then /out/claude --version; fi
RUN dpkg --add-architecture "$TARGETARCH" \
 && apt-get update \
 && cd /tmp \
 && apt-get download "tini:$TARGETARCH" \
 && dpkg-deb -x tini_*.deb /tmp/tini \
 && install -D -m 755 /tmp/tini/usr/bin/tini /rootfs/usr/bin/tini \
 && rm -rf /var/lib/apt/lists/* /tmp/tini tini_*.deb
RUN printf '#!/bin/sh\nexec node --disable-warning=UNDICI-EHPA /opt/workspace-agent/sanchoris.mjs "$@"\n' > /rootfs/usr/local/bin/sanchoris \
 && chmod 755 /rootfs/usr/local/bin/sanchoris /rootfs/usr/local/bin/workspace-entrypoint \
 && mkdir -p /rootfs/etc /home-dirs \
 && git config --file /rootfs/etc/gitconfig credential.helper '!sanchoris git-credential' \
 && git config --file /rootfs/etc/gitconfig user.name sanchoris \
 && git config --file /rootfs/etc/gitconfig user.email sanchoris@users.noreply.github.com

FROM node:26-bookworm@sha256:52dd743dc3b494cba5bc8c13507ac984507addb8a087c626dc98dcffa53133d4
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
VOLUME ["/workspace", "/home/node", "/tmp"]
ENTRYPOINT ["/usr/local/bin/workspace-entrypoint"]
