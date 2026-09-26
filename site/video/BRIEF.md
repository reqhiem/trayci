---
workflow: motion-graphics
flow: automation
storyboard: no
message: "Every AI quota and reset, one click from your tray."
destination: web-embed
aspect: 1600x1000 (hero), 1200x900 (detail, themes)
language: en
length: hero 8.8s, detail 6.5s, themes 6.6s (seamless loops, 30 fps, silent)
---

## Intent

Three short, silent, seamlessly looping product clips for the Trayci landing page
(https://reqhiem.github.io/trayci/). Calm, precise, product-grade: ease-out entrances,
200–400 ms UI moves, no bounce, no shake. The popover is the real UI: the app's own CSS
(`assets/app.css`, copied from `src/renderer/src/styles.css`) and markup captured from the
`.misc/preview/` harness.

| clip   | composition                | message                                       |
| ------ | -------------------------- | --------------------------------------------- |
| hero   | `compositions/hero.html`   | Every AI quota and reset, one click from your tray. |
| detail | `compositions/detail.html` | Hover to peek, click to pin.                  |
| themes | `compositions/themes.html` | Dark or light, used or remaining — your call. |

## Assets

- assets/app.css — the app's stylesheet, light tokens also scoped to `.theme-light`.
- assets/space-grotesk-latin-wght-normal.woff2 — Space Grotesk (variable), from @fontsource-variable.
- assets/trayci.svg — the Trayci icon (resources/icons/trayci.svg).

## Notes

- Canonical fixture only; the UI shows "Claude" (the provider's display name), as the app does.
- Truthful interactions: hover follows mouseenter per row, so the cursor never crosses another
  row on its way; the pane closes when the pointer leaves the popover; Esc unpins.
- `index.html` is a QA reel that mounts all three clips for `check`, `snapshot` and Studio.
  Shipped videos render each composition on its own (`build.sh`).
