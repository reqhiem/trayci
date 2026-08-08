# Compatibility matrix

| Distribution    | Desktop        | Tray/window startup | Popover |  Autostart | Claude | Codex | Antigravity |
| --------------- | -------------- | ------------------: | ------: | ---------: | -----: | ----: | ----------: |
| Linux Mint 21.3 | Cinnamon / X11 |                Pass |    Pass | Pass (E2E) |   Pass |  Pass |  Pass (CLI) |
| Ubuntu 25       | GNOME          |                Pass |    Pass |       Pass |   Pass |  Pass |     Not run |

The Mint and Ubuntu 25 (GNOME) rows were checked with the packaged Electron runtime, real local provider sessions, the renderer E2E, and XDG autostart writing in an isolated configuration directory. Antigravity's four quota windows were validated on Mint against a signed-in `agy` 1.1.11 session; packaged popover validation remains pending.
