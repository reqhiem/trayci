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
- Detailed and compact views with per-provider drill-down
- Automatic refresh, stale data handling, and a credential-free local cache
- Configurable providers, polling, startup, and percentage display
- Native `.deb` and `.AppImage` releases for Linux amd64

## Install

Download the latest `.deb` or `.AppImage` from [GitHub Releases](https://github.com/reqhiem/trayci/releases).

For the AppImage:

```bash
chmod +x Trayci-*.AppImage
./Trayci-*.AppImage
```

For Debian or Ubuntu:

```bash
sudo apt install ./trayci_*_amd64.deb
```

Trayci requires at least one authenticated provider. Antigravity follows Orca ADE's Google Code Assist quota path and reuses `~/.gemini/oauth_creds.json`; an installed Gemini CLI is needed to refresh an expired OAuth token.

## Development

Requirements: Linux amd64, Node.js 22+, pnpm 9+, and at least one authenticated provider CLI.

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
pnpm test:e2e
pnpm dist:linux
```

Diagnostics reuse the same provider code as the desktop app:

```bash
pnpm build
pnpm cli -- usage --json
pnpm cli -- usage antigravity
pnpm cli -- doctor
```

Lefthook formats staged files with Prettier before each commit. Run `pnpm exec lefthook install` if hooks were not installed automatically.

## Releases

Pull requests and changes to `main` run CI and produce downloadable Linux build artifacts. To publish a release, update the version in `package.json` and push the matching tag, for example `v0.2.0`; GitHub Actions builds and attaches both installers to the release.

## Contributing

Issues and pull requests are welcome. Check the [roadmap](https://github.com/reqhiem/trayci/projects) and [open issues](https://github.com/reqhiem/trayci/issues), keep changes focused, and include tests for behavior changes. The pull request template lists the expected checks.

Trayci stores settings and a credential-free cache under `$XDG_CONFIG_HOME/trayci` or `~/.config/trayci`. See [COMPATIBILITY.md](COMPATIBILITY.md) for provider and desktop compatibility details.

## License

Trayci is available under the [MIT License](LICENSE).
