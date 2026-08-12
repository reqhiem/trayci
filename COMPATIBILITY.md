# Compatibility matrix

| Distribution    | Desktop        | Tray/window startup | Popover |  Autostart | Claude | Codex | Antigravity |
| --------------- | -------------- | ------------------: | ------: | ---------: | -----: | ----: | ----------: |
| Linux Mint 21.3 | Cinnamon / X11 |                Pass |    Pass | Pass (E2E) |   Pass |  Pass |  Pass (CLI) |
| Ubuntu 25       | GNOME          |                Pass |    Pass |       Pass |   Pass |  Pass |     Not run |

The Mint and Ubuntu 25 (GNOME) rows were checked with the packaged Electron runtime, real local provider sessions, the renderer E2E, and XDG autostart writing in an isolated configuration directory. Antigravity's four quota windows were validated on Mint against a signed-in `agy` 1.1.11 session; packaged popover validation remains pending.

**Note:** as of v0.3.0 Trayci runs on Tauri 2. The matrix above reflects the retired Electron build and is pending re-validation against the Tauri build (see tech spec §19 for the known Tauri/Linux watch-items).

**Wayland:** GTK lets no Wayland client place its own window (`gtk_window_move` is a no-op for toplevels), so on a Wayland session the popover cannot anchor to the tray icon, be dragged, or reopen where it was left. Everything else works.

XWayland is not a way out, and v0.4.0 must not force it. Measured on Ubuntu GNOME with `GDK_BACKEND=x11`: the popover maps but never takes X input focus (`xdotool getwindowfocus` keeps naming another window), and the first click unmaps it instead of reaching the webview — the settings button, the refresh button and the provider rows all appear dead. Pointer motion still arrives, so hover keeps working, which is what makes the failure look like "only the header broke".
