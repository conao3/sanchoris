#!/usr/bin/env bash
set -euo pipefail

if [ -z "${IN_NIX_SHELL:-}" ]; then
  exec nix develop --command "$0" "$@"
fi

usage() {
  echo "usage: scripts/verify.sh quick|full" >&2
  exit 2
}

[ $# -eq 1 ] || usage
mode=$1
cd "$(git rev-parse --show-toplevel)"

timings=()
started=$SECONDS

summary() {
  local status=$?
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

full() {
  ensure_node_modules
  step "cargo fmt" cargo fmt --all --check
  step "cargo clippy" cargo clippy --workspace --all-targets --all-features -- -D warnings
  step "cargo test" cargo test --workspace --no-fail-fast
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
  git rev-parse --verify --quiet origin/master >/dev/null || {
    echo "verify.sh quick: origin/master not found; run git fetch origin" >&2
    exit 1
  }
  local base
  base=$(git merge-base HEAD origin/master)
  local changed
  changed=$({ git diff --name-only "$base"; git ls-files --others --exclude-standard; } | sort -u)
  if [ -z "$changed" ]; then
    echo "verify.sh quick: no changes against origin/master"
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
      scripts/e2e/*)
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
    step "cargo test" cargo test --workspace --no-fail-fast
  elif [ ${#crates[@]} -gt 0 ]; then
    local -a tested=("${!crates[@]}")
    local -a checked
    mapfile -t checked < <(dependents_closure "${tested[@]}")
    step "cargo clippy" cargo clippy $(printf -- '-p %s ' "${checked[@]}") --all-targets --all-features -- -D warnings
    step "cargo test" cargo test $(printf -- '-p %s ' "${tested[@]}") --no-fail-fast
  fi

  if [ $all_crates -eq 0 ] && [ $e2e -eq 1 ] && [ -z "${crates[sanchoris-api]:-}" ]; then
    step "pnpm e2e" pnpm e2e
  fi

  local package
  for package in "${!packages[@]}"; do
    step "pnpm check $package" pnpm --filter "$package" check
  done
}

case "$mode" in
  quick) quick ;;
  full) full ;;
  *) usage ;;
esac
