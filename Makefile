.PHONY: check test build dev bundle-linux bundle-win cli

check:
	pnpm format:check
	pnpm lint
	pnpm typecheck
	cd src-tauri && cargo fmt --check
	cd src-tauri && cargo clippy --workspace --all-targets -- -D warnings

test:
	pnpm test
	cd src-tauri && cargo test --workspace

build:
	pnpm build

dev:
	pnpm dev

bundle-linux:
	pnpm dist:linux

bundle-win:
	pnpm dist:win

cli:
	pnpm cli -- $(ARGS)
