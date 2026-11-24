.PHONY: check test run bundle-linux bundle-win

# cargo-packager names bundles after the binary and the AppImage after the arch; install.sh and
# install.ps1 fetch the `Trayci_*` names the Tauri builds published.
VERSION = $(shell cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "trayci") | .version')

check:
	cargo fmt --check
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

run:
	cargo run -- $(ARGS)

bundle-linux:
	cargo packager --release --formats deb,appimage
	mkdir -p dist
	mv target/release/trayci_$(VERSION)_amd64.deb dist/Trayci_$(VERSION)_amd64.deb
	mv target/release/trayci_$(VERSION)_x86_64.AppImage dist/Trayci_$(VERSION)_amd64.AppImage

bundle-win:
	cargo packager --release --formats nsis
	mkdir -p dist
	mv target/release/trayci_$(VERSION)_x64-setup.exe dist/Trayci_$(VERSION)_x64-setup.exe
