<p align="center">
  <img src="resources/icons/128x128.png" width="96" height="96" alt="Trayci icon">
</p>

<h1 align="center">Trayci</h1>

<p align="center">Claude Code, Codex, and Antigravity usage, one click away.</p>

<p align="center">
  <a href="https://github.com/reqhiem/trayci/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/reqhiem/trayci/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/reqhiem/trayci/releases"><img alt="Release" src="https://img.shields.io/github/v/release/reqhiem/trayci?display_name=tag"></a>
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/github/license/reqhiem/trayci"></a>
</p>

Trayci is a local-first Linux system tray app that shows Claude Code, Codex, and Google Antigravity quota windows without interrupting your workflow. It reuses each provider CLI's existing authentication and never stores credentials.

## Features

- Claude Code, Codex, and Antigravity usage in a compact tray popover
- Detailed and compact views: hover a provider to reveal its windows, click to pin them
- Reset countdowns for every provider, including Antigravity's 5-hour and weekly limits
- Drag the popover by its header to any spot on screen; the position sticks between sessions
- Light and dark themes plus four text sizes
- Automatic refresh, stale data handling, and a credential-free local cache
- Configurable providers, polling, startup, and percentage display
- Native `.deb` and `.AppImage` releases for Linux amd64, and NSIS installer (`.exe`) for Windows

## Install

Download the latest `.deb`, `.AppImage`, or `.exe` installer from [GitHub Releases](https://github.com/reqhiem/trayci/releases).

For the AppImage:

```bash
chmod +x Trayci-*.AppImage
./Trayci-*.AppImage
```

For Debian or Ubuntu:

```bash
sudo apt install ./trayci_*_amd64.deb
```

Trayci requires at least one authenticated provider. Antigravity reads the signed-in `agy` CLI's `/usage` panel and falls back to Orca ADE's Google Code Assist quota path when Gemini OAuth credentials are available.

## Development

Trayci runs on Tauri 2: a Rust core (`src-tauri/`) with the React renderer in the system WebView. Requirements: Linux amd64 or Windows x64, Node.js 22+, pnpm 9+, Rust 1.80+, the Tauri system dependencies (`libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev` on Debian/Ubuntu), and at least one authenticated provider CLI.

```bash
pnpm install
pnpm dev
```

Useful commands:

```bash
pnpm format:check
pnpm lint
pnpm typecheck
pnpm test
pnpm dist:linux
pnpm dist:win
```

Diagnostics reuse the same provider code as the desktop app:

```bash
pnpm cli -- usage --json
pnpm cli -- usage antigravity
pnpm cli -- doctor
```

Lefthook formats staged files with Prettier before each commit. Run `pnpm exec lefthook install` if hooks were not installed automatically.

## Releases

Pull requests and changes to `main` run CI and produce downloadable Linux and Windows build artifacts. To publish a release, update the version in `package.json` and push the matching tag, for example `v0.3.0`; GitHub Actions builds and attaches Linux and Windows installers to the release.

## Contributing

Issues and pull requests are welcome. Check the [roadmap](https://github.com/reqhiem/trayci/projects) and [open issues](https://github.com/reqhiem/trayci/issues), keep changes focused, and include tests for behavior changes. The pull request template lists the expected checks.

Trayci stores settings and a credential-free cache under `$XDG_CONFIG_HOME/trayci` or `~/.config/trayci`. See [COMPATIBILITY.md](COMPATIBILITY.md) for provider and desktop compatibility details.

## License

Trayci is available under the [MIT License](LICENSE).
