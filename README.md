# Trayci

Trayci is a local-first Linux tray utility for checking Claude Code and Codex quota windows without leaving your workflow.

## Development

Requirements: Linux amd64, Node.js 22+, pnpm 9+, and at least one authenticated provider CLI.

```bash
pnpm install
pnpm dev
```

Diagnostics reuse the same provider code as the desktop application:

```bash
pnpm build
pnpm cli -- usage --json
pnpm cli -- doctor
```

Run all checks and build Linux artifacts:

```bash
pnpm lint
pnpm typecheck
pnpm test
pnpm dist:linux
```

Trayci stores settings and a credential-free cache under `$XDG_CONFIG_HOME/trayci` or `~/.config/trayci`. It never copies provider credentials into its files.
