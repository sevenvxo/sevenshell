#!/bin/sh
# area screenshot to the clipboard and when something replaces it ask if u wanna save it to ~/Pictures/Screenshots

dir="$HOME/Pictures/Screenshots"
name="$(date +%Y-%m-%d_%H-%M-%S).png"
tmp="$(mktemp --suffix=.png)"
trap 'rm -f "$tmp"' EXIT

geom="$(slurp -d)" || exit 0          # Escape cancels
grim -g "$geom" "$tmp" || exit 1

# serves the uhh image till another copy takes over the clipboard then returns
wl-copy --foreground --type image/png < "$tmp"

if zenity --question --title="Screenshot" \
    --text="The screenshot was replaced on the clipboard.\nSave it to Pictures/Screenshots?" \
    --ok-label="Save" --cancel-label="Discard"; then
    mkdir -p "$dir"
    cp "$tmp" "$dir/$name"
fi
