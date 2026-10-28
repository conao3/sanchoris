#!/bin/sh
set -eu
: "${SANCHORIS_RUN_TOKEN:?SANCHORIS_RUN_TOKEN must be set}"
: "${SANCHORIS_RELAY_URL:?SANCHORIS_RELAY_URL must be set}"
secrets=$(printf '{"runToken":"%s","relayUrl":"%s"}' "$SANCHORIS_RUN_TOKEN" "$SANCHORIS_RELAY_URL")
unset SANCHORIS_RUN_TOKEN SANCHORIS_RELAY_URL
exec tini -- node --disable-warning=UNDICI-EHPA /opt/workspace-agent/src/bin/workspace-agent.ts <<EOF
$secrets
EOF
