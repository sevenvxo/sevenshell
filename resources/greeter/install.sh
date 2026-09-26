#!/bin/sh
# installs sevenshells login screen for greetd so run it w sudo ./install.sh
# run it again after rebuilding bc the greeter gets its own copies of the binaries
set -eu

[ "$(id -u)" = 0 ] || { echo "Run it with sudo."; exit 1; }
user="${SUDO_USER:?Run it with sudo from your own account.}"
home=$(getent passwd "$user" | cut -d: -f6)
here=$(cd "$(dirname "$0")" && pwd)
# sevenwm sits next to sevenshell unless SEVENWM_REPO says otherwise
sevenwm_repo="${SEVENWM_REPO:-$(cd "$here/../../.." && pwd)/sevenwm}"
[ -d "$sevenwm_repo" ] || sevenwm_repo="$home/Projects/sevenwm"

command -v greetd >/dev/null || { echo "greetd isn't installed: sudo pacman -S greetd"; exit 1; }
getent passwd greeter >/dev/null || { echo "There's no 'greeter' account (greetd's package makes it)."; exit 1; }

# the binaries where the greeter account can run them
lib=/usr/local/lib/sevenwm-greeter
install -Dm755 "$(readlink -f "$home/.local/bin/sevenwm")" "$lib/sevenwm"
install -Dm755 "$(readlink -f "$home/.local/bin/sevenshell")" "$lib/sevenshell"

# the sevenwm session launcher and the entry the login screen lists
install -Dm755 "$sevenwm_repo/resources/sevenwm-session" /usr/local/bin/sevenwm-session
install -Dm644 "$sevenwm_repo/resources/sevenwm.desktop" /usr/share/wayland-sessions/sevenwm.desktop

# ur sevenwm config without binds and ur lock screen look
conf=/etc/sevenwm-greeter
install -d "$conf/sevenwm" "$conf/sevenshell"
[ -f "$home/.config/sevenwm/config.toml" ] &&
    install -m644 "$home/.config/sevenwm/config.toml" "$conf/sevenwm/config.toml"
if [ -f "$home/.config/sevenshell/config.toml" ]; then
    awk '/^\[/ { on = ($0 == "[lock]") } on' "$home/.config/sevenshell/config.toml" \
        > "$conf/sevenshell/config.toml"
    chmod 644 "$conf/sevenshell/config.toml"
    wallpaper=$(sed -n 's/^wallpaper *= *"\(.*\)".*/\1/p' "$conf/sevenshell/config.toml")
    case "$wallpaper" in
        "$home"/*|"~"/*) echo "Note: the lock wallpaper is in your home, which the login screen can't read; it'll be black." ;;
    esac
fi

# where it remembers the last user and session
install -d -o greeter -g greeter /var/cache/sevenwm-greeter

# greetd itself w the original kept once as config.toml.orig
[ -f /etc/greetd/config.toml ] && [ ! -f /etc/greetd/config.toml.orig ] &&
    cp /etc/greetd/config.toml /etc/greetd/config.toml.orig
install -Dm644 "$here/greetd.toml" /etc/greetd/config.toml

echo "Installed. To use it instead of SDDM:"
echo "    sudo systemctl disable sddm && sudo systemctl enable greetd   (then reboot)"
echo "To go back:"
echo "    sudo systemctl disable greetd && sudo systemctl enable sddm"
