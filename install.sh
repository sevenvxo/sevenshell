#!/bin/sh
# sets up sevenwm and sevenshell on an arch machine so js clone sevenshell and run ./install.sh
# safe to run again and it updates rebuilds and reinstalls whatever changed
set -eu

here=$(cd "$(dirname "$0")" && pwd)
sevenwm="${SEVENWM_REPO:-$(dirname "$here")/sevenwm}"
bin="$HOME/.local/bin"

say() { printf '\n==> %s\n' "$*"; }

[ "$(id -u)" != 0 ] || { echo "run it as ur normal user without sudo and it asks for sudo when it needs it"; exit 1; }
command -v pacman >/dev/null || { echo "this only knows arch bc it uses pacman"; exit 1; }

# pulseaudio clashes w pipewire-pulse so it and its plugins get swapped out before the packages go in
if pacman -Q pulseaudio >/dev/null 2>&1; then
    say "swapping pulseaudio for pipewire"
    # anything else that needs pulseaudio would break so stop and say what instead
    needs=$(LC_ALL=C pacman -Qi pulseaudio |
        awk '/^Required By/ { on = 1; sub(/^[^:]*: */, "") } /^Optional For/ { on = 0 } on' |
        tr -s ' \n' '\n\n' | grep -vE '^(None|pulseaudio(-.*)?)?$' || true)
    if [ -n "$needs" ]; then
        echo "cant swap out pulseaudio bc these need it so remove them or pulseaudio urself and run this again:"
        echo "$needs"
        exit 1
    fi
    systemctl --user stop pulseaudio.socket pulseaudio.service 2>/dev/null || true
    # shellcheck disable=SC2046
    sudo pacman -Rdd --noconfirm $(pacman -Qq | grep -E '^pulseaudio(-|$)' | grep -vx pulseaudio-qt)
    sudo pacman -S --needed --noconfirm pipewire-pulse wireplumber
    systemctl --user daemon-reload 2>/dev/null || true
    systemctl --user start pipewire-pulse.socket wireplumber.service 2>/dev/null || true
fi

# the uhh packages to build and run everything
say "installing packages"
packages="base-devel git pkgconf clang seatd libinput libxkbcommon mesa libdrm systemd-libs wayland fontconfig
    gtk4 gtk4-layer-shell pam
    xwayland-satellite pipewire-pulse wireplumber brightnessctl networkmanager bluez bluez-utils power-profiles-daemon
    grim slurp wl-clipboard zenity xdg-desktop-portal-wlr xdg-desktop-portal-gtk libnotify polkit
    kitty thunar ttf-nerd-fonts-symbols python-gobject python-cairo gtk3 gtk-layer-shell greetd"
# rustup clashes w the plain rust package so only add it if theres no cargo yet
command -v cargo >/dev/null || packages="$packages rustup"
# shellcheck disable=SC2086
sudo pacman -S --needed --noconfirm $packages
if command -v rustup >/dev/null && ! rustup show active-toolchain >/dev/null 2>&1; then
    rustup default stable
fi

# sevenwm lives next to sevenshell
if [ -d "$sevenwm/.git" ]; then
    say "updating sevenwm"
    git -C "$sevenwm" pull --ff-only || echo "couldnt update sevenwm so building whats there"
else
    say "getting sevenwm"
    git clone https://github.com/sevenvxo/sevenwm.git "$sevenwm"
fi

say "building sevenwm and sevenshell"
(cd "$sevenwm" && cargo build --release)
(cd "$here" && cargo build --release)

say "linking them into $bin"
mkdir -p "$bin"
ln -sf "$sevenwm/target/release/sevenwm" "$bin/sevenwm"
ln -sf "$here/target/release/sevenshell" "$bin/sevenshell"
ln -sf "$sevenwm/settings/sevenwm-settings" "$bin/sevenwm-settings"

# the helper scripts the bar and screenshot key use but never over ur own copies
say "installing helper scripts"
put() { [ -e "$2" ] || install -Dm755 "$1" "$2"; }
put "$here/resources/scripts/tray.py" "$HOME/.config/waybar/scripts/tray.py"
put "$here/resources/scripts/perf-status.sh" "$HOME/.config/waybar/scripts/perf-status.sh"
put "$here/resources/scripts/screenshot.sh" "$HOME/.config/sevenwm/screenshot.sh"

# a first config that does the screenshot thing w the script above
if [ ! -e "$HOME/.config/sevenwm/config.toml" ]; then
    sed 's|^command = "grim .*|command = "~/.config/sevenwm/screenshot.sh"|' \
        "$sevenwm/config.default.toml" > "$HOME/.config/sevenwm/config.toml"
fi

# services the bar and trays talk to
say "turning on bluetooth and power profiles"
sudo systemctl enable --now bluetooth.service power-profiles-daemon.service || true

# the login screen and the seven session entry
say "installing the login screen"
sudo SEVENWM_REPO="$sevenwm" "$here/resources/greeter/install.sh" >/dev/null

# switch the login manager to greetd if its something else
current=$(basename "$(readlink /etc/systemd/system/display-manager.service 2>/dev/null)" .service 2>/dev/null || true)
if [ "$current" != greetd ]; then
    printf '\nswitch the login screen from %s to sevens greetd login screen? [y/N] ' "${current:-nothing}"
    read -r answer </dev/tty || answer=n
    case "$answer" in
        y|Y|yes)
            [ -n "$current" ] && sudo systemctl disable "$current.service"
            sudo systemctl enable greetd.service
            ;;
        *) echo "left it alone and u can pick seven from ur current login screen" ;;
    esac
fi

# the network tray needs NetworkManager and this goes last bc switching can drop the connection for a sec
networking() {
    {
        systemctl list-units --type=service,socket --state=active --plain --no-legend \
            'systemd-networkd*' iwd.service 'dhcpcd*' 'wpa_supplicant@*' 'netctl*' connman.service | awk '{ print $1 }'
        find /etc/systemd/system -path '*.wants/*' \( -name 'systemd-networkd*' -o -name iwd.service \
            -o -name 'dhcpcd*' -o -name 'wpa_supplicant@*' -o -name 'netctl*' -o -name connman.service \) \
            -exec basename {} \;
    } | sort -u | tr '\n' ' '
}
if ! systemctl is-active --quiet NetworkManager.service; then
    others=$(networking)
    if [ -z "${others% }" ]; then
        say "turning on NetworkManager"
        sudo systemctl enable --now NetworkManager.service
    else
        printf '\nthe network is run by %s\nhand it over to NetworkManager so the network tray works?' "$others"
        printf ' saved wifi passwords need typing in again'
        [ -n "${SSH_CONNECTION:-}" ] && printf ' and this ssh connection might drop'
        printf ' [Y/n] '
        read -r answer </dev/tty || answer=n
        case "$answer" in
            ""|y|Y|yes)
                # shellcheck disable=SC2086
                sudo systemctl disable --now $others
                sudo systemctl enable --now NetworkManager.service
                echo "switched so pick ur wifi from the network tray or run nmtui"
                ;;
            *) echo "left it alone and the network tray stays empty till u run sudo systemctl enable --now NetworkManager" ;;
        esac
    fi
fi

say "done so reboot and pick seven at the login screen"
