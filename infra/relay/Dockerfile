FROM node:22-alpine@sha256:fe4f6d74e20370acb04ad889077d14d0f64a9c8ea6a42bc60cddf1e844699914
COPY package.json /opt/egress-relay/package.json
COPY src /opt/egress-relay/src
USER node
ENTRYPOINT ["node", "/opt/egress-relay/src/bin/egress-relay.ts"]
