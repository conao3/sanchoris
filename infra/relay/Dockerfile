FROM node:22-alpine
COPY package.json /opt/egress-relay/package.json
COPY src /opt/egress-relay/src
USER node
ENTRYPOINT ["node", "/opt/egress-relay/src/bin/egress-relay.ts"]
