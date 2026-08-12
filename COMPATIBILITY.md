# Compatibility matrix

| Distribution    | Desktop        | Tray/window startup | Popover |  Autostart | Claude | Codex | Antigravity |
| --------------- | -------------- | ------------------: | ------: | ---------: | -----: | ----: | ----------: |
| Linux Mint 21.3 | Cinnamon / X11 |                Pass |    Pass | Pass (E2E) |   Pass |  Pass |  Pass (CLI) |
| Ubuntu 25       | GNOME          |                Pass |    Pass |       Pass |   Pass |  Pass |     Not run |

The Mint and Ubuntu 25 (GNOME) rows were checked with the packaged Electron runtime, real local provider sessions, the renderer E2E, and XDG autostart writing in an isolated configuration directory. Antigravity's four quota windows were validated on Mint against a signed-in `agy` 1.1.11 session; packaged popover validation remains pending.

**Note:** as of v0.3.0 Trayci runs on Tauri 2. The matrix above reflects the retired Electron build and is pending re-validation against the Tauri build (see tech spec §19 for the known Tauri/Linux watch-items).

**Wayland:** GTK lets no Wayland client place its own window, so on a Wayland session the popover could not anchor to the tray icon, be dragged, or reopen where it was left. Since v0.4.0 Trayci starts under XWayland when `DISPLAY` is set; `GDK_BACKEND=wayland` opts back out. Checked on Ubuntu GNOME (Wayland): the popover anchors, resizes for the detail pane, and holds its position.
