#!/usr/bin/env bash
# Run after installation, with DISPLAY provided by Xvfb or a real X11 desktop.
set -euo pipefail
binary=${1:-atlas}
expected=${ATLAS_VERSION:?Set ATLAS_VERSION to the release version}
log_dir=${ATLAS_SMOKE_LOG_DIR:-/tmp/atlas-smoke}
mkdir -p "$log_dir"
"$binary" --version | tee "$log_dir/version.txt"
grep -Fx "Atlas $expected" "$log_dir/version.txt"
if [[ ${ATLAS_SMOKE_CLI_ONLY:-0} == 1 ]]; then exit 0; fi
command -v xdotool >/dev/null
command -v import >/dev/null
if command -v magick >/dev/null; then
  image_convert=(magick)
else
  command -v convert >/dev/null
  image_convert=(convert)
fi
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/tmp/atlas-runtime-$UID}
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
export LIBGL_ALWAYS_SOFTWARE=1
export GALLIUM_DRIVER=llvmpipe
# Old llvmpipe Vulkan drivers can create a surface but present only black frames.
# Exercise the OpenGL backend consistently; hardware Vulkan needs desktop testing.
export GPUI_BACKEND=${ATLAS_SMOKE_BACKEND:-gl}
export ATLAS_CONFIG_DIR="$log_dir/config"
mkdir -p "$log_dir/sample-files/Example folder"
printf 'Atlas Linux packaging test\n' > "$log_dir/sample-files/Atlas packaging test.txt"
"$binary" "$log_dir/sample-files" > "$log_dir/startup.log" 2>&1 &
app_pid=$!
trap 'kill "$app_pid" 2>/dev/null || true; wait "$app_pid" 2>/dev/null || true' EXIT
window=''
for ((attempt=0; attempt<40; attempt++)); do
  if ! kill -0 "$app_pid" 2>/dev/null; then cat "$log_dir/startup.log"; exit 1; fi
  window=$(xdotool search --onlyvisible --pid "$app_pid" 2>/dev/null | head -n1 || true)
  if [[ -z $window ]]; then
    # AppImage/Flatpak/Snap launchers may have a different PID from the GUI process.
    window=$(xdotool search --onlyvisible --class "${ATLAS_SMOKE_WINDOW_CLASS:-atlas}" 2>/dev/null | head -n1 || true)
  fi
  [[ -z $window ]] || break
  sleep 0.5
done
[[ -n $window ]] || { cat "$log_dir/startup.log"; echo 'No visible Atlas window' >&2; exit 1; }
rendered=false
for ((attempt=0; attempt<40; attempt++)); do
  kill -0 "$app_pid" 2>/dev/null || { cat "$log_dir/startup.log"; exit 1; }
  import -window "$window" "$log_dir/window.png"
  # Normalized variation rejects a uniformly black/blank window without fixing a theme.
  variation=$("${image_convert[@]}" "$log_dir/window.png" -alpha off -colorspace Gray \
    -format '%[fx:standard_deviation > 0.003]' info:)
  if [[ $variation == 1 ]]; then rendered=true; break; fi
  sleep 0.5
done
$rendered || { cat "$log_dir/startup.log"; echo 'Atlas window did not render content' >&2; exit 1; }
cp "$log_dir/window.png" "$log_dir/first-frame.png"
# A first-run assistant alone is insufficient: also exercise its keyboard handler
# and reach the actual file browser. All preferences stay inside this test's logs.
xdotool windowfocus --sync "$window"
xdotool key --clearmodifiers Escape
for ((attempt=0; attempt<20; attempt++)); do
  kill -0 "$app_pid" 2>/dev/null || { cat "$log_dir/startup.log"; exit 1; }
  if grep -Eq '"getting_started_seen"[[:space:]]*:[[:space:]]*true' "$ATLAS_CONFIG_DIR/settings.json" 2>/dev/null; then break; fi
  sleep 0.5
done
grep -Eq '"getting_started_seen"[[:space:]]*:[[:space:]]*true' "$ATLAS_CONFIG_DIR/settings.json" || {
  echo 'Atlas did not dismiss its first-run assistant with Escape' >&2; exit 1;
}
# Prove the window survives initial directory loading and the background workers.
sleep 3
kill -0 "$app_pid"
import -window "$window" "$log_dir/window.png"
variation=$("${image_convert[@]}" "$log_dir/window.png" -alpha off -colorspace Gray \
  -format '%[fx:standard_deviation > 0.003]' info:)
[[ $variation == 1 ]] || { echo 'Atlas file browser rendered a blank window' >&2; exit 1; }
echo 'Atlas dismissed setup, rendered the file browser and stayed alive.'
