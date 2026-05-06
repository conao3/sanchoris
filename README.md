# Sanchoris

Sanchoris is a pnpm + Vite frontend and Rust backend monorepo.

## Layout

- `apps/frontend`: Vite frontend package managed by pnpm
- `crates/backend`: Rust backend crate

## Development

Enter the Nix development shell:

```sh
nix develop
```

Install frontend dependencies:

```sh
pnpm install
```

Run the frontend:

```sh
pnpm dev
```

Run the backend:

```sh
cargo run -p sanchoris-backend
```

Run checks:

```sh
pnpm check
cargo check --workspace
```
