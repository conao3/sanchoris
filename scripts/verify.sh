#!/usr/bin/env bash
set -euo pipefail

if [ -z "${IN_NIX_SHELL:-}" ]; then
  exec nix develop --command "$0" "$@"
fi

usage() {
  echo "usage: scripts/verify.sh quick|full|test [cargo nextest run arguments]" >&2
  exit 2
}

[ $# -ge 1 ] || usage
mode=$1
shift
[ "$mode" = test ] || [ $# -eq 0 ] || usage
cd "$(git rev-parse --show-toplevel)"

timings=()
started=$SECONDS
dynamo_instances=8
dynamo_pids=()

summary() {
  local status=$?
  if [ ${#dynamo_pids[@]} -gt 0 ]; then
    kill "${dynamo_pids[@]}" 2>/dev/null || true
  fi
  echo
  echo "verify.sh $mode: $([ $status -eq 0 ] && echo ok || echo FAILED) in $((SECONDS - started))s"
  for line in "${timings[@]}"; do
    echo "  $line"
  done
}
trap summary EXIT

step() {
  local name=$1
  shift
  echo "==> $name: $*"
  local t0=$SECONDS
  if "$@"; then
    timings+=("$(printf '%5ss  %s' $((SECONDS - t0)) "$name")")
  else
    local status=$?
    timings+=("$(printf '%5ss  %s (failed)' $((SECONDS - t0)) "$name")")
    return $status
  fi
}

ensure_node_modules() {
  if [ ! -f node_modules/.modules.yaml ]; then
    step "pnpm install" pnpm install --frozen-lockfile
  fi
}

start_dynamo() {
  if [ ${#dynamo_pids[@]} -gt 0 ]; then
    return
  fi
  local -a ports
  mapfile -t ports < <(node -e "
    const net = require('net');
    const servers = Array.from({ length: $dynamo_instances }, () => net.createServer().listen(0, '127.0.0.1'));
    Promise.all(servers.map((s) => new Promise((r) => s.on('listening', r)))).then(() => {
      for (const s of servers) console.log(s.address().port);
      for (const s of servers) s.close();
    });")
  local port
  for port in "${ports[@]}"; do
    dynamodb-local -inMemory -disableTelemetry -port "$port" >/dev/null 2>&1 &
    dynamo_pids+=($!)
  done
  local deadline=$((SECONDS + 60))
  for port in "${ports[@]}"; do
    until (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; do
      if [ $SECONDS -ge $deadline ]; then
        echo "verify.sh: DynamoDB Local did not listen on port $port" >&2
        exit 1
      fi
      sleep 0.2
    done
  done
  export SANCHORIS_TEST_DYNAMO_PORTS
  SANCHORIS_TEST_DYNAMO_PORTS=$(IFS=,; echo "${ports[*]}")
}

nextest() {
  start_dynamo
  cargo nextest run --no-fail-fast --test-threads "$dynamo_instances" "$@"
}

full() {
  ensure_node_modules
  step "cargo fmt" cargo fmt --all --check
  step "cargo clippy" cargo clippy --workspace --all-targets --all-features -- -D warnings
  step "cargo nextest" nextest --workspace
  step "pnpm check" pnpm check
}

crate_name() {
  sed -n 's/^name = "\(.*\)"$/\1/p' "crates/$1/Cargo.toml" | head -n 1
}

dependents_closure() {
  cargo metadata --no-deps --format-version 1 | jq -r --argjson seeds "$(printf '%s\n' "$@" | jq -R . | jq -s .)" '
    [.packages[] | {name, deps: [.dependencies[] | select(.path != null) | .name]}] as $pkgs
    | def grow: . as $set | ($set + [$pkgs[] | select(any(.deps[]; . as $d | $set | index($d))) | .name] | unique) as $next
        | if ($next | length) == ($set | length) then $set else $next | grow end;
      $seeds | unique | grow | .[]'
}

quick() {
  local base
  base=$(git merge-base HEAD master)
  local changed
  changed=$({ git diff --name-only "$base"; git ls-files --others --exclude-standard; } | sort -u)
  if [ -z "$changed" ]; then
    echo "verify.sh quick: no changes against master"
    return
  fi

  local -A crates=()
  local -A packages=()
  local all_crates=0 rust_changed=0 e2e=0
  local file dir
  while IFS= read -r file; do
    case "$file" in
      crates/*)
        dir=${file#crates/}
        dir=${dir%%/*}
        [ -f "crates/$dir/Cargo.toml" ] && crates[$(crate_name "$dir")]=1
        rust_changed=1
        e2e=1
        ;;
      Cargo.toml | Cargo.lock | flake.nix | flake.lock)
        all_crates=1
        rust_changed=1
        ;;
      infra/tables.json | infra/plans.json)
        crates[sanchoris-core]=1
        ;;
      apps/frontend/src/graphql/*)
        crates[sanchoris-graphql]=1
        crates[sanchoris-api]=1
        packages[@sanchoris/frontend]=1
        ;;
      apps/frontend/*)
        packages[@sanchoris/frontend]=1
        ;;
      apps/workspace-agent/*)
        packages[@sanchoris/workspace-agent]=1
        e2e=1
        ;;
      scripts/e2e/* | scripts/e2e-aws.mjs | scripts/lib/e2e-task.mjs)
        e2e=1
        ;;
      apps/egress-relay/*)
        packages[@sanchoris/egress-relay]=1
        ;;
      package.json | pnpm-lock.yaml | pnpm-workspace.yaml)
        packages[@sanchoris/frontend]=1
        packages[@sanchoris/workspace-agent]=1
        packages[@sanchoris/egress-relay]=1
        ;;
    esac
  done <<<"$changed"

  if [ ${#packages[@]} -gt 0 ] || [ $e2e -eq 1 ]; then
    ensure_node_modules
  fi

  if [ $rust_changed -eq 1 ]; then
    step "cargo fmt" cargo fmt --all --check
  fi

  if [ $all_crates -eq 1 ]; then
    step "cargo clippy" cargo clippy --workspace --all-targets --all-features -- -D warnings
    step "cargo nextest" nextest --workspace
  elif [ ${#crates[@]} -gt 0 ]; then
    local -a tested=("${!crates[@]}")
    local -a checked
    mapfile -t checked < <(dependents_closure "${tested[@]}")
    step "cargo clippy" cargo clippy $(printf -- '-p %s ' "${checked[@]}") --all-targets --all-features -- -D warnings
    step "cargo nextest" nextest $(printf -- '-p %s ' "${tested[@]}")
  fi

  if [ $all_crates -eq 0 ] && [ $e2e -eq 1 ] && [ -z "${crates[sanchoris-api]:-}" ]; then
    step "e2e" nextest -p sanchoris-api -E 'test(/^e2e::/)'
  fi

  local package
  for package in "${!packages[@]}"; do
    step "pnpm check $package" pnpm --filter "$package" check
  done
}

case "$mode" in
  quick) quick ;;
  full) full ;;
  test)
    ensure_node_modules
    step "cargo nextest" nextest "$@"
    ;;
  *) usage ;;
esac
