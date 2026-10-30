SDL := apps/frontend/src/graphql/schema.graphql

.PHONY: check schema

node_modules/.modules.yaml: package.json pnpm-lock.yaml pnpm-workspace.yaml apps/*/package.json
	vp install --frozen-lockfile

check: node_modules/.modules.yaml
	nix fmt -- --ci
	cargo run -q -p sanchoris-graphql --bin sanchoris-graphql-sdl | diff -u $(SDL) -
	vp run check
	cargo clippy --all-targets -- -D warnings
	scripts/nextest.sh --workspace

schema:
	cargo run -q -p sanchoris-graphql --bin sanchoris-graphql-sdl > $(SDL)
