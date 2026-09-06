# Compatibility matrix

| Distribution    | Desktop        | Tray/window startup | Popover |  Autostart | Claude | Codex | Antigravity |
| --------------- | -------------- | ------------------: | ------: | ---------: | -----: | ----: | ----------: |
| Linux Mint 21.3 | Cinnamon / X11 |                Pass |    Pass | Pass (E2E) |   Pass |  Pass |  Pass (CLI) |
| Ubuntu 25       | GNOME          |                Pass |    Pass |       Pass |   Pass |  Pass |     Not run |

The Mint and Ubuntu 25 (GNOME) rows were checked with the packaged Electron runtime, real local provider sessions, the renderer E2E, and XDG autostart writing in an isolated configuration directory. Antigravity's four quota windows were validated on Mint against a signed-in `agy` 1.1.11 session; packaged popover validation remains pending.

**Note:** as of v0.3.0 Trayci runs on Tauri 2. The matrix above reflects the retired Electron build and is pending re-validation against the Tauri build (see tech spec §19 for the known Tauri/Linux watch-items).

**X11:** measured on Linux Mint / Cinnamon (muffin) against v0.4.0. `tao` asks the window manager to move and to focus the popover with a zero timestamp — `begin_move_drag(.., 0)` and `present_with_time(GDK_CURRENT_TIME)` — and muffin honours neither, so dragging the title bar did nothing and the popover mapped unfocused with `_NET_WM_STATE_DEMANDS_ATTENTION`. Setting the position does work there, so as of v0.4.1 the app moves the window itself while the header is held, and asks for focus with a timestamp read from the X server. Dragging, the stored `windowPosition`, Escape and click-outside were confirmed on that session.

One thing is still open on X11: the first key after the popover is shown is dropped roughly every other open, so Escape sometimes needs a second press. Any other keypress flushes it. It correlates with the popover being hidden while it still holds focus, and is not the input method — reproduced identically with `GTK_IM_MODULE=gtk-im-context-simple`. Tracked in issue #38; click-outside is unaffected.

**Wayland:** GTK lets no Wayland client place its own window (`gtk_window_move` is a no-op for toplevels), so on a Wayland session the popover cannot anchor to the tray icon, be dragged, or reopen where it was left. Driving the move from the app does not change this: it goes through the same `set_position`. Everything else works.

XWayland is not a way out, and v0.4.0 must not force it. Measured on Ubuntu GNOME with `GDK_BACKEND=x11`: the popover maps but never takes X input focus (`xdotool getwindowfocus` keeps naming another window), and the first click unmaps it instead of reaching the webview — the settings button, the refresh button and the provider rows all appear dead. Pointer motion still arrives, so hover keeps working, which is what makes the failure look like "only the header broke".
