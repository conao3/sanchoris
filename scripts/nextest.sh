#!/usr/bin/env bash
set -euo pipefail

: "${IN_NIX_SHELL:?run scripts/nextest.sh in the devShell: nix develop -c scripts/nextest.sh <nextest arguments>}"
cd "$(git rev-parse --show-toplevel)"

dynamo_instances=8
dynamo_pids=()

stop_dynamo() {
  if [ ${#dynamo_pids[@]} -gt 0 ]; then
    kill "${dynamo_pids[@]}" 2>/dev/null || true
  fi
}
trap stop_dynamo EXIT

mapfile -t ports < <(node -e "
  const net = require('net');
  const servers = Array.from({ length: $dynamo_instances }, () => net.createServer().listen(0, '127.0.0.1'));
  Promise.all(servers.map((s) => new Promise((r) => s.on('listening', r)))).then(() => {
    for (const s of servers) console.log(s.address().port);
    for (const s of servers) s.close();
  });")
for port in "${ports[@]}"; do
  dynamodb-local -inMemory -disableTelemetry -port "$port" >/dev/null 2>&1 &
  dynamo_pids+=($!)
done
deadline=$((SECONDS + 60))
for port in "${ports[@]}"; do
  until (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; do
    if [ $SECONDS -ge $deadline ]; then
      echo "nextest.sh: DynamoDB Local did not listen on port $port" >&2
      exit 1
    fi
    sleep 0.2
  done
done
SANCHORIS_TEST_DYNAMO_PORTS=$(IFS=,; echo "${ports[*]}")
export SANCHORIS_TEST_DYNAMO_PORTS

cargo nextest run --no-fail-fast --test-threads "$dynamo_instances" "$@"
