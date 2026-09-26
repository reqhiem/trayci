#!/usr/bin/env bash
# Re-render the landing clips (needs Node >= 22 and ffmpeg):  site/video/build.sh [out-dir] [clip...]
# Raw renders stay in renders/ (not committed); web encodes and posters go to out-dir.
set -euo pipefail
cd "$(dirname "$0")"
out=${1:-renders/web}
shift $(($# > 0))
clips=${*:-hero detail themes}
declare -A poster=([hero]=3.6 [detail]=3.4 [themes]=1.07) # poster frame, seconds
hf="npx --yes hyperframes@0.8.78"
export HYPERFRAMES_NO_TELEMETRY=1 HYPERFRAMES_SKIP_SKILLS=1

# The popover is the app's own UI: refresh its stylesheet (light tokens also on .theme-light),
# font and icon from the repo.
sed 's/^:root\[data-theme="light"\]/.theme-light,\n&/' ../../src/renderer/src/styles.css > assets/app.css
cp ../../node_modules/@fontsource-variable/space-grotesk/files/space-grotesk-latin-wght-normal.woff2 assets/
cp ../../resources/icons/trayci.svg assets/

$hf check < /dev/null # the QA reel (index.html) mounts all three clips
mkdir -p "$out"
for name in $clips; do
  at=${poster[$name]}
  # stdin from /dev/null: without it the CLI can wait on the terminal forever.
  $hf render -c "compositions/$name.html" -o "renders/$name.mp4" --quality high < /dev/null
  ffmpeg -v error -y -i "renders/$name.mp4" -an -c:v libx264 -profile:v high -pix_fmt yuv420p \
    -preset slow -crf 26 -movflags +faststart "$out/$name.mp4"
  ffmpeg -v error -y -ss "$at" -i "renders/$name.mp4" -frames:v 1 -q:v 3 "$out/$name.jpg"
done
ls -l "$out"
