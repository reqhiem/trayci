# Compatibility matrix

| Distribution    | Desktop        | Tray/window startup | Popover |  Autostart | Claude | Codex |
| --------------- | -------------- | ------------------: | ------: | ---------: | -----: | ----: |
| Linux Mint 21.3 | Cinnamon / X11 |                Pass |    Pass | Pass (E2E) |   Pass |  Pass |
| Ubuntu 25       | GNOME          |                Pass |    Pass |       Pass |   Pass |  Pass |

The Mint and Ubuntu 25 (GNOME) rows were checked with the packaged Electron runtime, real local provider sessions, the renderer E2E, and XDG autostart writing in an isolated configuration directory.
