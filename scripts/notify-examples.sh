#!/usr/bin/env bash
# Sends example notifications to whatever daemon owns
# org.freedesktop.Notifications, covering the features the panel supports.
#
# Usage: scripts/notify-examples.sh [example...]
#        scripts/notify-examples.sh --list
# Without arguments, every example is sent.
#
# Calls go through gdbus, which returns right away (unlike `notify-send -A`,
# which blocks until an action is clicked). Run with --watch to print
# ActionInvoked / NotificationClosed signals as they arrive.

set -euo pipefail

DEST=org.freedesktop.Notifications
OBJECT=/org/freedesktop/Notifications

# notify APP REPLACES_ID ICON SUMMARY BODY ACTIONS HINTS TIMEOUT
# ACTIONS and HINTS are GVariant text, e.g. "['key', 'Label']" and
# "{'urgency': <byte 2>}". Prints the notification id.
notify() {
  gdbus call --session --dest "$DEST" --object-path "$OBJECT" \
    --method "$DEST.Notify" -- "$1" "$2" "$3" "$4" "$5" "$6" "$7" "$8" |
    sed -E 's/^\(uint32 ([0-9]+),\)$/\1/'
}

close() {
  gdbus call --session --dest "$DEST" --object-path "$OBJECT" \
    --method "$DEST.CloseNotification" "$1" >/dev/null
}

# A width x height RGBA gradient as an `image-data` hint value.
gradient() {
  local width=$1 height=$2 x y bytes=()
  for ((y = 0; y < height; y++)); do
    for ((x = 0; x < width; x++)); do
      bytes+=("byte $((x * 255 / width)), byte $((y * 255 / height)), byte 200, byte 255")
    done
  done
  local IFS=,
  echo "<($width, $height, $((width * 4)), true, 8, 4, [${bytes[*]}])>"
}

# First PNG found in the usual icon locations, for `image-path`.
some_png() {
  local dir png
  for dir in /run/current-system/sw/share/icons /usr/share/icons "$HOME/.nix-profile/share/icons"; do
    [[ -d $dir ]] || continue
    png=$(find "$dir" -path '*48x48/apps/*.png' -print -quit 2>/dev/null || true)
    if [[ -n $png ]]; then
      echo "$png"
      return
    fi
  done
}

example_basic() {
  notify "Examples" 0 "dialog-information" "Basic notification" \
    "A summary, a body and an app icon." "[]" "{}" -1
}

example_markup() {
  notify "Examples" 0 "format-text-bold" "Body markup" \
    'Supports <b>bold</b>, <i>italic</i>, <u>underline</u>, a <a href="https://example.com">link</a>, an <img src="x.png" alt="[image alt text]"/> and stray < & > characters.' \
    "[]" "{}" 0
}

example_long() {
  notify "Examples" 0 "text-x-generic" "A rather long summary that will not fit on a single line of the notification" \
    "$(printf 'Line %s of a long body that wraps; past six lines it scrolls.\n' 1 2 3 4 5 6 7 8)" \
    "[]" "{}" 0
}

example_low() {
  notify "Examples" 0 "dialog-information" "Low urgency" \
    "Muted outline and text." "[]" "{'urgency': <byte 0>}" -1
}

example_critical() {
  notify "Examples" 0 "dialog-warning" "Critical urgency" \
    "Red outline; never expires on its own." "[]" "{'urgency': <byte 2>}" -1
}

example_image_data() {
  notify "Examples" 0 "" "image-data hint" \
    "Raw pixels, clipped to the hexagon." "[]" "{'image-data': $(gradient 32 32)}" -1
}

example_image_path() {
  local png
  png=$(some_png)
  if [[ -z $png ]]; then
    echo "image_path: no PNG icon found, skipping" >&2
    return
  fi
  notify "Examples" 0 "" "image-path hint" \
    "Image loaded from $png" "[]" "{'image-path': <'file://$png'>}" -1
}

example_actions() {
  notify "Examples" 0 "mail-unread" "Actions" \
    "Click the notification for the default action, or a button." \
    "['default', 'Open', 'reply', 'Reply', 'archive', 'Archive']" "{}" 0
}

example_action_icons() {
  notify "Examples" 0 "audio-x-generic" "action-icons hint" \
    "Action keys are icon names; labels become tooltips." \
    "['media-skip-backward-symbolic', 'Previous', 'media-playback-pause-symbolic', 'Pause', 'media-skip-forward-symbolic', 'Next']" \
    "{'action-icons': <true>}" 0
}

example_resident() {
  notify "Examples" 0 "emblem-important" "resident hint" \
    "Stays open after an action is clicked; dismiss it explicitly." \
    "['again', 'Click me', 'more', 'And me']" "{'resident': <true>}" 0
}

example_position() {
  # Top left corner of the focused monitor, offset by 200px.
  local x=200 y=200 geometry
  if command -v hyprctl >/dev/null; then
    geometry=$(hyprctl monitors | awk '
      /^Monitor/ { getline; split($1, size, "@"); split($3, pos, "x") ; x = pos[1]; y = pos[2] }
      /focused: yes/ { print x, y; exit }')
    if [[ -n $geometry ]]; then
      read -r x y <<<"$geometry"
      x=$((x + 200)) y=$((y + 200))
    fi
  fi
  notify "Examples" 0 "find-location" "x/y hints" \
    "Placed at screen position $x,$y." "[]" "{'x': <$x>, 'y': <$y>}" 0
}

example_replace() {
  local id step
  id=$(notify "Examples" 0 "content-loading" "replaces_id" "Step 1 of 3" "[]" "{}" 0)
  for step in 2 3; do
    sleep 1
    notify "Examples" "$id" "content-loading" "replaces_id" "Step $step of 3" "[]" "{}" 0 >/dev/null
  done
  sleep 1
  notify "Examples" "$id" "emblem-ok" "replaces_id" "Done; replaced in place." "[]" "{}" -1
}

example_close() {
  local id
  id=$(notify "Examples" 0 "window-close" "CloseNotification" \
    "Closed by the sender after 2 seconds." "[]" "{}" 0)
  sleep 2
  close "$id"
  echo "$id"
}

example_transient() {
  notify "Examples" 0 "dialog-information" "transient hint" \
    "Gone for good once the popup expires; never in the notification centre." "[]" "{'transient': <true>}" -1
}

example_timeout() {
  notify "Examples" 0 "alarm" "expire_timeout" \
    "Expires after 10 seconds (hover to pause)." "[]" "{}" 10000
}

EXAMPLES=(basic markup long low critical image_data image_path actions
  action_icons resident position replace close transient timeout)

case "${1:-}" in
  --list | -l)
    printf '%s\n' "${EXAMPLES[@]}"
    exit
    ;;
  --watch | -w)
    exec gdbus monitor --session --dest "$DEST" --object-path "$OBJECT"
    ;;
  --help | -h)
    sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
    exit
    ;;
esac

selected=("$@")
((${#selected[@]})) || selected=("${EXAMPLES[@]}")

for name in "${selected[@]}"; do
  name=${name//-/_}
  if ! declare -F "example_$name" >/dev/null; then
    echo "unknown example: $name (see --list)" >&2
    exit 1
  fi
  printf '%-13s ' "$name"
  "example_$name"
done
