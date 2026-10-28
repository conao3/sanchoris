FROM node:26-alpine@sha256:40cd011b55415bc894e45b02fdd5a5dfde056921b7d23a64c74f80497af48797
COPY package.json /opt/egress-relay/package.json
COPY src /opt/egress-relay/src
USER node
ENTRYPOINT ["node", "/opt/egress-relay/src/bin/egress-relay.ts"]
