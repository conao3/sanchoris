FROM --platform=$BUILDPLATFORM node:26-bookworm@sha256:52dd743dc3b494cba5bc8c13507ac984507addb8a087c626dc98dcffa53133d4 AS build
RUN npm install --global pnpm@12.9.0
WORKDIR /src
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml vite.config.ts tsconfig.base.json tsconfig.json ./
COPY apps ./apps
RUN pnpm install --frozen-lockfile --ignore-scripts --filter sanchoris --filter @sanchoris/egress-relay \
 && node node_modules/vite-plus/bin/vp pack --filter egress-relay

FROM node:26-alpine@sha256:40cd011b55415bc894e45b02fdd5a5dfde056921b7d23a64c74f80497af48797
COPY --from=build /src/apps/egress-relay/dist /opt/egress-relay
USER node
ENTRYPOINT ["node", "/opt/egress-relay/egress-relay.mjs"]
