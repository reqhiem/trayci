# Compatibility matrix

| Distribution    | Desktop        | Tray/window startup | Popover |  Autostart |  Claude |   Codex | Antigravity |
| --------------- | -------------- | ------------------: | ------: | ---------: | ------: | ------: | ----------: |
| Linux Mint 21.3 | Cinnamon / X11 |                Pass |    Pass | Pass (E2E) |    Pass |    Pass |     Not run |
| Ubuntu          | GNOME          |             Not run | Not run |    Not run | Not run | Not run |     Not run |

The Mint row was checked with the packaged Electron runtime, real Claude and Codex sessions, the renderer E2E, and XDG autostart writing in an isolated configuration directory. Antigravity's four quota windows were validated against a signed-in `agy` 1.1.11 session; packaged popover validation remains pending. GNOME still requires a compatible desktop session for manual validation.
