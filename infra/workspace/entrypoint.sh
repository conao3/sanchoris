#!/bin/sh
set -eu
: "${SANCHORIS_BOOTSTRAP_TOKEN:?SANCHORIS_BOOTSTRAP_TOKEN must be set}"
: "${SANCHORIS_RELAY_URL:?SANCHORIS_RELAY_URL must be set}"
secrets=$(printf '{"bootstrapToken":"%s","relayUrl":"%s"}' "$SANCHORIS_BOOTSTRAP_TOKEN" "$SANCHORIS_RELAY_URL")
unset SANCHORIS_BOOTSTRAP_TOKEN SANCHORIS_RELAY_URL
exec tini -- node --disable-warning=UNDICI-EHPA /opt/workspace-agent/workspace-agent.mjs <<EOF
$secrets
EOF
