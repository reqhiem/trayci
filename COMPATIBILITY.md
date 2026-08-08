# Compatibility matrix

| Distribution    | Desktop        | Tray/window startup | Popover |  Autostart |  Claude |   Codex |
| --------------- | -------------- | ------------------: | ------: | ---------: | ------: | ------: |
| Linux Mint 21.3 | Cinnamon / X11 |                Pass |    Pass | Pass (E2E) |    Pass |    Pass |
| Ubuntu          | GNOME          |             Not run | Not run |    Not run | Not run | Not run |

The Mint row was checked with the packaged Electron runtime, real local provider sessions, the renderer E2E, and XDG autostart writing in an isolated configuration directory. GNOME still requires a compatible desktop session for manual validation.
