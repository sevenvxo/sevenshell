#!/usr/bin/env python3
# dropdown trays for the bar like tray.py audio wifi bluetooth power or perf and clicking the same module again or escape closes it
import json
import os
import signal
import subprocess
import sys
import threading
import time

import gi
gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
gi.require_version("GtkLayerShell", "0.1")
from gi.repository import Gdk, GLib, Gtk, GtkLayerShell

MODE = sys.argv[1] if len(sys.argv) > 1 else "audio"
RUNDIR = os.environ.get("XDG_RUNTIME_DIR", "/tmp")
PIDFILE = os.path.join(RUNDIR, "waybar-tray.pid")
CLOSEDFILE = os.path.join(RUNDIR, "waybar-tray.closed")

CSS = b"""
/* Black and white only; selected things get a white outline, hover a dashed one. */
window { background: transparent; }
#panel {
    background: #000000;
    border: 1px solid #ffffff;
    border-radius: 7px;
    padding: 14px;
    color: #ffffff;
}
* { font-family: "Symbols Nerd Font", sans-serif; font-size: 13px; color: #ffffff; }
.title { font-weight: bold; font-size: 14px; }
.dim { font-size: 12px; }
.icon { font-size: 18px; }
.section { font-size: 11px; font-weight: bold; margin-top: 6px; }

button {
    background: #000000; border: 1px solid #000000; box-shadow: none;
    border-radius: 4px; padding: 5px 7px;
}
button:hover { border: 1px dashed #ffffff; }
button.active { border: 1px solid #ffffff; }
button.footer { font-size: 12px; }
button.danger { background: #ffffff; border: 1px solid #ffffff; }
button.danger label { color: #000000; }

scale trough {
    min-height: 6px; border-radius: 2px; border: 1px solid #ffffff;
    background: #000000;
}
scale highlight { background: #ffffff; border-radius: 2px; border: none; }
scale slider {
    min-width: 16px; min-height: 16px; border-radius: 4px;
    background: #ffffff; border: 1px solid #000000; box-shadow: none; margin: -6px;
}

switch { background: #000000; border: 1px solid #ffffff; border-radius: 6px; }
switch:checked { background: #ffffff; }
switch slider { background: #ffffff; border: none; border-radius: 5px; box-shadow: none; }
switch:checked slider { background: #000000; }

entry {
    background: #000000; color: #ffffff;
    border: 1px solid #ffffff; border-radius: 4px;
    padding: 4px 8px; box-shadow: none;
}
.stat { background: #000000; border: 1px solid #ffffff; border-radius: 5px; padding: 8px 4px; }
.stat-value { font-size: 16px; font-weight: bold; }
button.choice { padding: 6px 4px; }
.app-row { padding: 2px 0 2px 8px; }
scrolledwindow, viewport { background: #000000; border: none; }
"""


def run(*cmd):
    try:
        return subprocess.run(cmd, capture_output=True, text=True, timeout=20)
    except Exception as e:
        return subprocess.CompletedProcess(cmd, 1, "", str(e))


def in_background(work, done=None):
    def target():
        result = work()
        if done:
            GLib.idle_add(done, result)
    threading.Thread(target=target, daemon=True).start()


def clear(box):
    for child in box.get_children():
        box.remove(child)


def label(text, *classes, xalign=0.0):
    lbl = Gtk.Label(label=text, xalign=xalign)
    lbl.set_ellipsize(3)  # Pango.EllipsizeMode.END
    for c in classes:
        lbl.get_style_context().add_class(c)
    return lbl


# audio

class AudioTray(Gtk.Box):
    def __init__(self):
        super().__init__(orientation=Gtk.Orientation.VERTICAL, spacing=8)

        header = Gtk.Box(spacing=8)
        header.pack_start(label("Volume", "title"), True, True, 0)
        self.percent = label("", "dim", xalign=1.0)
        header.pack_end(self.percent, False, False, 0)
        self.pack_start(header, False, False, 0)

        row = Gtk.Box(spacing=8)
        self.mute_btn = Gtk.Button()
        self.mute_lbl = label("", "icon", xalign=0.5)
        self.mute_btn.add(self.mute_lbl)
        self.mute_btn.connect("clicked", self.toggle_mute)
        row.pack_start(self.mute_btn, False, False, 0)
        self.scale = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, 0, 100, 1)
        self.scale.set_draw_value(False)
        self.scale.set_hexpand(True)
        row.pack_start(self.scale, True, True, 0)
        self.pack_start(row, False, False, 0)

        self.pack_start(label("OUTPUT", "section"), False, False, 0)
        self.sinks = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        self.pack_start(self.sinks, False, False, 0)

        more = Gtk.Button(label="Sound settings…")
        more.get_style_context().add_class("footer")
        more.connect("clicked", lambda *_: open_settings("sound", "pavucontrol"))
        self.pack_start(more, False, False, 0)

        self.refresh()
        self.scale.connect("value-changed", self.set_volume)

    def refresh(self):
        out = run("pactl", "get-sink-volume", "@DEFAULT_SINK@").stdout
        vol = next((int(tok[:-1]) for tok in out.split() if tok.endswith("%")), 0)
        muted = "yes" in run("pactl", "get-sink-mute", "@DEFAULT_SINK@").stdout
        self.scale.set_value(min(vol, 100))
        self.update_labels(vol, muted)
        self.muted = muted

        default = run("pactl", "get-default-sink").stdout.strip()
        try:
            sinks = json.loads(run("pactl", "-f", "json", "list", "sinks").stdout)
        except json.JSONDecodeError:
            sinks = []
        clear(self.sinks)
        for sink in sinks:
            btn = Gtk.Button()
            box = Gtk.Box(spacing=8)
            active = sink["name"] == default
            box.pack_start(label("󰄬" if active else " ", xalign=0.5), False, False, 0)
            box.pack_start(label(sink.get("description", sink["name"])), True, True, 0)
            btn.add(box)
            if active:
                btn.get_style_context().add_class("active")
            btn.connect("clicked", self.set_sink, sink["name"])
            self.sinks.pack_start(btn, False, False, 0)
        self.sinks.show_all()

    def update_labels(self, vol, muted):
        self.percent.set_text("muted" if muted else f"{vol}%")
        icon = "󰝟" if muted else ("󰕿" if vol < 34 else "󰖀" if vol < 67 else "󰕾")
        self.mute_lbl.set_text(icon)

    def set_volume(self, scale):
        vol = int(scale.get_value())
        run("pactl", "set-sink-volume", "@DEFAULT_SINK@", f"{vol}%")
        if self.muted:
            run("pactl", "set-sink-mute", "@DEFAULT_SINK@", "0")
            self.muted = False
        self.update_labels(vol, False)

    def toggle_mute(self, *_):
        run("pactl", "set-sink-mute", "@DEFAULT_SINK@", "toggle")
        self.muted = not self.muted
        self.update_labels(int(self.scale.get_value()), self.muted)

    def set_sink(self, _btn, name):
        run("pactl", "set-default-sink", name)
        self.refresh()


# wifi

SIGNAL_ICONS = ["󰤯", "󰤟", "󰤢", "󰤥", "󰤨"]


def nmcli_split(line):
    # nmcli -t escapes colons inside fields as a backslash colon
    fields, cur, i = [], "", 0
    while i < len(line):
        if line[i] == "\\" and i + 1 < len(line):
            cur += line[i + 1]
            i += 2
            continue
        if line[i] == ":":
            fields.append(cur)
            cur = ""
        else:
            cur += line[i]
        i += 1
    fields.append(cur)
    return fields


class WifiTray(Gtk.Box):
    def __init__(self):
        super().__init__(orientation=Gtk.Orientation.VERTICAL, spacing=8)

        header = Gtk.Box(spacing=8)
        header.pack_start(label("Wi-Fi", "title"), True, True, 0)
        self.switch = Gtk.Switch()
        self.switch.set_valign(Gtk.Align.CENTER)
        header.pack_end(self.switch, False, False, 0)
        self.pack_start(header, False, False, 0)

        self.status = label("", "dim")
        self.pack_start(self.status, False, False, 0)

        self.scroller = Gtk.ScrolledWindow()
        self.scroller.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.scroller.set_propagate_natural_height(True)
        self.scroller.set_max_content_height(320)
        self.networks = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        self.scroller.add(self.networks)
        self.pack_start(self.scroller, True, True, 0)

        more = Gtk.Button(label="Network settings…")
        more.get_style_context().add_class("footer")
        more.connect("clicked", lambda *_: open_settings(
            "network", "kitty", "--class", "nmtui", "-e", "nmtui"))
        self.pack_start(more, False, False, 0)

        enabled = run("nmcli", "radio", "wifi").stdout.strip() == "enabled"
        self.switch.set_active(enabled)
        self.switch.connect("notify::active", self.toggle_radio)
        self.password_row = None

        if enabled:
            self.load(rescan=False)
            self.status.set_text("Scanning…")
            in_background(lambda: self.fetch(rescan=True), self.show)
        else:
            self.show([])

    def fetch(self, rescan):
        out = run("nmcli", "-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "dev", "wifi", "list",
                  "--rescan", "yes" if rescan else "no").stdout
        seen = {}
        for line in out.splitlines():
            f = nmcli_split(line)
            if len(f) < 4 or not f[1]:
                continue
            net = {"active": f[0] == "*", "ssid": f[1],
                   "signal": int(f[2] or 0), "secure": f[3] not in ("", "--")}
            old = seen.get(net["ssid"])
            if not old or net["active"] or (not old["active"] and net["signal"] > old["signal"]):
                seen[net["ssid"]] = net
        return sorted(seen.values(), key=lambda n: (not n["active"], -n["signal"]))

    def load(self, rescan):
        self.show(self.fetch(rescan))

    def show(self, nets):
        clear(self.networks)
        self.password_row = None
        if not self.switch.get_active():
            self.status.set_text("Wi-Fi is off")
        elif not nets:
            self.status.set_text("No networks found")
        else:
            active = next((n["ssid"] for n in nets if n["active"]), None)
            self.status.set_text(f"Connected to {active}" if active else "Not connected")
        for net in nets:
            self.networks.pack_start(self.network_row(net), False, False, 0)
        self.networks.show_all()
        self.scroller.set_min_content_height(min(len(nets), 8) * 38)

    def network_row(self, net):
        wrap = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        btn = Gtk.Button()
        box = Gtk.Box(spacing=8)
        icon = SIGNAL_ICONS[min(net["signal"] // 20, 4)]
        box.pack_start(label(icon, "icon", xalign=0.5), False, False, 0)
        box.pack_start(label(net["ssid"]), True, True, 0)
        if net["active"]:
            box.pack_end(label("connected", "dim", xalign=1.0), False, False, 0)
        elif net["secure"]:
            box.pack_end(label("󰌾", "dim", xalign=1.0), False, False, 0)
        btn.add(box)
        if net["active"]:
            btn.get_style_context().add_class("active")
        btn.connect("clicked", self.clicked, net, wrap)
        wrap.pack_start(btn, False, False, 0)
        return wrap

    def clicked(self, _btn, net, wrap):
        if net["active"]:
            self.status.set_text("Disconnecting…")
            in_background(lambda: run("nmcli", "con", "down", "id", net["ssid"]), self.after_action)
            return
        known = any(nmcli_split(l)[:2] == [net["ssid"], "802-11-wireless"]
                    for l in run("nmcli", "-t", "-f", "NAME,TYPE", "con", "show").stdout.splitlines())
        if known or not net["secure"]:
            self.connect_to(net, None, wrap)
        else:
            self.ask_password(net, wrap)

    def ask_password(self, net, wrap):
        if self.password_row:
            self.password_row.destroy()
        entry = Gtk.Entry()
        entry.set_visibility(False)
        entry.set_placeholder_text("Password")
        entry.connect("activate", lambda e: self.connect_to(net, e.get_text(), wrap))
        wrap.pack_start(entry, False, False, 0)
        self.password_row = entry
        entry.show()
        entry.grab_focus()

    def connect_to(self, net, password, wrap):
        self.status.set_text(f"Connecting to {net['ssid']}…")
        if password is not None:
            cmd = ["nmcli", "dev", "wifi", "connect", net["ssid"], "password", password]
        else:
            cmd = ["nmcli", "dev", "wifi", "connect", net["ssid"]]

        def done(result):
            if result.returncode != 0 and net["secure"]:
                self.status.set_text("Couldn't connect — check the password")
                self.ask_password(net, wrap)
            else:
                self.after_action(result)
        in_background(lambda: run(*cmd), done)

    def after_action(self, _result):
        in_background(lambda: self.fetch(rescan=False), self.show)

    def toggle_radio(self, switch, _pspec):
        on = switch.get_active()
        run("nmcli", "radio", "wifi", "on" if on else "off")
        if on:
            self.status.set_text("Scanning…")
            in_background(lambda: (time.sleep(2), self.fetch(rescan=True))[1], self.show)
        else:
            self.show([])


# bluetooth

def bt_devices(*filters):
    out = run("bluetoothctl", "devices", *filters).stdout
    devs = {}
    for line in out.splitlines():
        parts = line.split(" ", 2)
        if len(parts) == 3 and parts[0] == "Device":
            devs[parts[1]] = parts[2]
    return devs


def looks_unnamed(mac, name):
    return name.replace("-", ":").upper() == mac.upper()


class BluetoothTray(Gtk.Box):
    def __init__(self):
        super().__init__(orientation=Gtk.Orientation.VERTICAL, spacing=8)

        header = Gtk.Box(spacing=8)
        header.pack_start(label("Bluetooth", "title"), True, True, 0)
        self.switch = Gtk.Switch()
        self.switch.set_valign(Gtk.Align.CENTER)
        header.pack_end(self.switch, False, False, 0)
        self.pack_start(header, False, False, 0)

        self.status = label("", "dim")
        self.pack_start(self.status, False, False, 0)

        self.scroller = Gtk.ScrolledWindow()
        self.scroller.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.devices = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        self.scroller.add(self.devices)
        self.scroller.set_no_show_all(True)
        self.pack_start(self.scroller, True, True, 0)

        self.scan_btn = Gtk.Button(label="󰑐  Scan for devices")
        self.scan_btn.connect("clicked", self.scan)
        self.pack_start(self.scan_btn, False, False, 0)

        more = Gtk.Button(label="Bluetooth settings…")
        more.get_style_context().add_class("footer")
        more.connect("clicked", lambda *_: open_settings("bluetooth", "blueman-manager"))
        self.pack_start(more, False, False, 0)

        self.scanning = False
        self.busy = None
        self.switch.set_active("Powered: yes" in run("bluetoothctl", "show").stdout)
        self.switch.connect("notify::active", self.toggle_power)
        self.refresh()

    def refresh(self):
        clear(self.devices)
        powered = self.switch.get_active()
        self.scan_btn.set_sensitive(powered and not self.scanning)
        self.scan_btn.set_label("Scanning…" if self.scanning else "󰑐  Scan for devices")
        rows = 0
        if powered:
            paired = bt_devices("Paired")
            connected = bt_devices("Connected")
            if paired:
                self.devices.pack_start(label("PAIRED", "section"), False, False, 0)
            for mac, name in sorted(paired.items(), key=lambda d: (d[0] not in connected, d[1])):
                self.devices.pack_start(self.device_row(mac, name, mac in connected, True), False, False, 0)
                rows += 1
            others = {m: n for m, n in bt_devices().items()
                      if m not in paired and not looks_unnamed(m, n)}
            if others:
                self.devices.pack_start(label("AVAILABLE", "section"), False, False, 0)
            for mac, name in sorted(others.items(), key=lambda d: d[1]):
                self.devices.pack_start(self.device_row(mac, name, False, False), False, False, 0)
                rows += 1
            if self.busy:
                self.status.set_text(self.busy)
            elif connected:
                self.status.set_text("Connected to " + ", ".join(connected.values()))
            elif not paired and not others:
                self.status.set_text("No devices yet — scan to find some")
            else:
                self.status.set_text("Not connected")
        else:
            self.status.set_text("Bluetooth is off")
        self.scroller.set_min_content_height(min(rows, 7) * 38 + 30)
        self.scroller.show_all() if rows else self.scroller.hide()

    def device_row(self, mac, name, connected, paired):
        btn = Gtk.Button()
        box = Gtk.Box(spacing=8)
        box.pack_start(label("󰂱" if connected else "󰂯", "icon", xalign=0.5), False, False, 0)
        box.pack_start(label(name), True, True, 0)
        if connected:
            box.pack_end(label("connected", "dim", xalign=1.0), False, False, 0)
            btn.get_style_context().add_class("active")
        btn.add(box)
        btn.connect("clicked", self.clicked, mac, name, connected, paired)
        return btn

    def clicked(self, _btn, mac, name, connected, paired):
        if connected:
            self.busy = f"Disconnecting {name}…"
            work = lambda: run("bluetoothctl", "disconnect", mac)
        elif paired:
            self.busy = f"Connecting to {name}…"
            work = lambda: run("bluetoothctl", "connect", mac)
        else:
            self.busy = f"Pairing with {name}…"
            def work():
                run("bluetoothctl", "pair", mac)
                run("bluetoothctl", "trust", mac)
                return run("bluetoothctl", "connect", mac)
        self.refresh()

        def done(result):
            self.busy = None
            self.refresh()
            if result.returncode != 0:
                self.status.set_text(f"Couldn't connect to {name}")
        in_background(work, done)

    def scan(self, *_):
        self.scanning = True
        self.refresh()
        GLib.timeout_add_seconds(2, lambda: self.scanning and (self.refresh() or True))

        def done(_result):
            self.scanning = False
            self.refresh()
        in_background(lambda: run("bluetoothctl", "--timeout", "10", "scan", "on"), done)

    def toggle_power(self, switch, _pspec):
        run("bluetoothctl", "power", "on" if switch.get_active() else "off")
        self.refresh()


# power

POWER_ACTIONS = [
    ("󰐥", "Shut down", ["systemctl", "poweroff"]),
    ("󰜉", "Restart", ["systemctl", "reboot"]),
    ("󰍃", "Sign out", None),
]


class PowerTray(Gtk.Box):
    def __init__(self):
        super().__init__(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        self.armed = None
        self.buttons = []
        for icon, name, cmd in POWER_ACTIONS:
            btn = Gtk.Button()
            box = Gtk.Box(spacing=10)
            box.pack_start(label(icon, "icon", xalign=0.5), False, False, 0)
            text = label(name)
            box.pack_start(text, True, True, 0)
            btn.add(box)
            btn.connect("clicked", self.clicked, name, cmd, text)
            self.buttons.append((btn, name, text))
            self.pack_start(btn, False, False, 0)

        more = Gtk.Button(label="Power settings…")
        more.get_style_context().add_class("footer")
        more.connect("clicked", lambda *_: open_settings("power", "powerprofilesctl"))
        self.pack_start(more, False, False, 0)

    def clicked(self, btn, name, cmd, text):
        # first click arms the action and the second click runs it
        if self.armed == name:
            if cmd is None:
                sign_out()
            else:
                launch(*cmd)
            return
        self.armed = name
        for b, n, t in self.buttons:
            t.set_text(n)
            b.get_style_context().remove_class("danger")
        text.set_text(f"Click again to {name.lower()}")
        btn.get_style_context().add_class("danger")


# performance

PROFILES = [("power-saver", "󰌪", "Saver"), ("balanced", "󰾅", "Balanced"),
            ("performance", "󱐋", "Max")]
GPU_PRESETS = [(175, "Quiet"), (230, "Balanced"), (285, "Max")]
# things that shouldnt get reprioritized from here like shells audio and the desktop itself
SKIP_APPS = {"bash", "zsh", "fish", "sh", "ps", "sleep", "systemd", "dbus-broker",
             "dbus-broker-launch", "dbus-daemon", "pipewire", "pipewire-pulse", "wireplumber",
             "driftwm", "Xwayland", "waybar", "gamemoded", "xdg-desktop-portal",
             "xdg-document-portal", "xdg-permission-store", "hyprpolkitagent", "at-spi-bus-launcher",
             "at-spi2-registryd", "gvfsd", "fusermount3"}
CLK_TCK = os.sysconf("SC_CLK_TCK")
PAGE = os.sysconf("SC_PAGE_SIZE")
NCPU = os.cpu_count() or 1


def read(path):
    try:
        with open(path) as f:
            return f.read()
    except OSError:
        return ""


def cpu_times():
    fields = [int(x) for x in read("/proc/stat").split("\n", 1)[0].split()[1:]]
    return sum(fields), fields[3] + fields[4]


def cpu_temp():
    for h in os.listdir("/sys/class/hwmon"):
        if read(f"/sys/class/hwmon/{h}/name").strip() == "k10temp":
            t = read(f"/sys/class/hwmon/{h}/temp1_input").strip()
            return int(t) // 1000 if t else None
    return None


def gpu_stats():
    out = run("nvidia-smi", "--query-gpu=utilization.gpu,temperature.gpu,power.draw,power.limit",
              "--format=csv,noheader,nounits").stdout.strip()
    try:
        util, temp, draw, limit = (float(x) for x in out.split(","))
        return {"util": util, "temp": temp, "draw": draw, "limit": limit}
    except ValueError:
        return None


def my_processes():
    """pid to app name cpu ticks rss bytes and nice for this users processes"""
    uid = os.getuid()
    procs = {}
    for pid in os.listdir("/proc"):
        if not pid.isdigit() or int(pid) == os.getpid():
            continue
        try:
            if os.stat(f"/proc/{pid}").st_uid != uid:
                continue
            name = os.path.basename(os.readlink(f"/proc/{pid}/exe")).removesuffix(" (deleted)")
            if not any(c.isalpha() for c in name):  # like a binary named after its version
                name = read(f"/proc/{pid}/comm").strip() or name
        except OSError:
            continue
        stat = read(f"/proc/{pid}/stat")
        if not stat:
            continue
        rest = stat.rsplit(")", 1)[1].split()
        ticks = int(rest[11]) + int(rest[12])
        nice = int(rest[16])
        rss = int((read(f"/proc/{pid}/statm").split() or [0, 0])[1]) * PAGE
        procs[int(pid)] = (name, ticks, rss, nice)
    return procs


def top_apps(interval=1.0, limit=6):
    before = my_processes()
    time.sleep(interval)
    after = my_processes()
    apps = {}
    for pid, (name, ticks, rss, nice) in after.items():
        if name in SKIP_APPS or name.startswith(("python", "xdg-", "chrome_crashpad")):
            continue
        app = apps.setdefault(name, {"name": name, "pids": [], "cpu": 0.0, "rss": 0, "nice": 0})
        prev = before.get(pid)
        if prev and prev[0] == name:
            app["cpu"] += (ticks - prev[1]) / CLK_TCK / interval / NCPU * 100
        app["rss"] += rss
        app["pids"].append(pid)
        # the most common nice value wins so one odd helper doesnt mislabel the app
        app.setdefault("nices", []).append(nice)
    for app in apps.values():
        nices = app.pop("nices")
        app["nice"] = max(set(nices), key=nices.count)
    # skip small helpers so the list shows real apps
    apps = [a for a in apps.values() if a["rss"] > 40 * 2**20 or a["cpu"] >= 1]
    return sorted(apps, key=lambda a: (-a["cpu"], -a["rss"]))[:limit]


def fmt_bytes(n):
    return f"{n / 2**30:.1f}G" if n >= 2**30 else f"{n / 2**20:.0f}M"


def pkexec(*cmd):
    # detached so the password prompt survives the tray closing when it takes focus
    subprocess.Popen(["pkexec", *cmd], start_new_session=True,
                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


class PerfTray(Gtk.Box):
    def __init__(self):
        super().__init__(orientation=Gtk.Orientation.VERTICAL, spacing=8)

        header = Gtk.Box(spacing=8)
        header.pack_start(label("Performance", "title"), True, True, 0)
        self.mode_lbl = label("", "dim", xalign=1.0)
        header.pack_end(self.mode_lbl, False, False, 0)
        self.pack_start(header, False, False, 0)

        stats = Gtk.Box(spacing=6, homogeneous=True)
        self.stat_cpu = self.stat(stats, "CPU")
        self.stat_gpu = self.stat(stats, "GPU")
        self.stat_ram = self.stat(stats, "RAM")
        self.pack_start(stats, False, False, 0)

        self.pack_start(label("POWER MODE", "section"), False, False, 0)
        self.profile_row = Gtk.Box(spacing=4, homogeneous=True)
        self.pack_start(self.profile_row, False, False, 0)

        self.pack_start(label("GPU POWER LIMIT", "section"), False, False, 0)
        self.gpu_row = Gtk.Box(spacing=4, homogeneous=True)
        self.pack_start(self.gpu_row, False, False, 0)
        self.gpu_note = label("", "dim")
        self.pack_start(self.gpu_note, False, False, 0)

        apps_header = Gtk.Box(spacing=8)
        apps_header.pack_start(label("APPS", "section"), True, True, 0)
        apps_header.pack_end(label("󰓅 boost   󰒲 background ", "dim", xalign=1.0), False, False, 0)
        self.pack_start(apps_header, False, False, 0)
        self.apps = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        self.pack_start(self.apps, False, False, 0)
        self.apps.pack_start(label("Measuring…", "dim"), False, False, 0)

        self.gpu = None
        self.pending_limit = None
        self.cpu_prev = cpu_times()
        self.refresh_profiles()
        self.tick()
        GLib.timeout_add_seconds(2, self.tick)
        self.load_apps()

    def stat(self, parent, caption):
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        box.get_style_context().add_class("stat")
        value = label("–", "stat-value", xalign=0.5)
        box.pack_start(value, False, False, 0)
        sub = label(caption, "dim", xalign=0.5)
        box.pack_start(sub, False, False, 0)
        parent.pack_start(box, True, True, 0)
        return value, sub

    # live numbers

    def tick(self):
        total, idle = cpu_times()
        dt, di = total - self.cpu_prev[0], idle - self.cpu_prev[1]
        self.cpu_prev = (total, idle)
        temp = cpu_temp()
        if dt:
            self.stat_cpu[0].set_text(f"{(1 - di / dt) * 100:.0f}%")
        self.stat_cpu[1].set_text(f"CPU · {temp}°" if temp is not None else "CPU")

        mem = dict(line.split(":", 1) for line in read("/proc/meminfo").splitlines() if ":" in line)
        kb = lambda k: int(mem.get(k, "0 kB").split()[0]) * 1024
        self.stat_ram[0].set_text(fmt_bytes(kb("MemTotal") - kb("MemAvailable")))
        self.stat_ram[1].set_text(f"of {fmt_bytes(kb('MemTotal'))}")

        in_background(gpu_stats, self.show_gpu)
        return True

    def show_gpu(self, gpu):
        self.gpu = gpu
        if not gpu:
            self.stat_gpu[0].set_text("–")
            self.gpu_note.set_text("GPU not available")
            return
        self.stat_gpu[0].set_text(f"{gpu['util']:.0f}%")
        self.stat_gpu[1].set_text(f"GPU · {gpu['temp']:.0f}°")
        if self.pending_limit and abs(gpu["limit"] - self.pending_limit) < 1:
            self.pending_limit = None
        if self.pending_limit:
            note = f"Setting {self.pending_limit} W — enter your password if asked"
        else:
            note = f"Drawing {gpu['draw']:.0f} W of {gpu['limit']:.0f} W · resets on reboot"
        self.gpu_note.set_text(note)
        self.refresh_gpu_buttons()

    # power mode

    def refresh_profiles(self):
        current = run("powerprofilesctl", "get").stdout.strip()
        clear(self.profile_row)
        for key, icon, name in PROFILES:
            btn = self.choice_button(icon, name, key == current)
            btn.connect("clicked", self.set_profile, key)
            self.profile_row.pack_start(btn, True, True, 0)
            if key == current:
                self.mode_lbl.set_text(name.lower() if key != "performance" else "max")
        self.profile_row.show_all()

    def set_profile(self, _btn, key):
        run("powerprofilesctl", "set", key)
        self.refresh_profiles()

    # gpu

    def refresh_gpu_buttons(self):
        clear(self.gpu_row)
        limit = self.pending_limit or (self.gpu or {}).get("limit")
        for watts, name in GPU_PRESETS:
            active = limit is not None and abs(limit - watts) < 1
            btn = self.choice_button(f"{watts}W", name, active)
            btn.connect("clicked", self.set_gpu_limit, watts)
            self.gpu_row.pack_start(btn, True, True, 0)
        self.gpu_row.show_all()

    def set_gpu_limit(self, _btn, watts):
        self.pending_limit = watts
        pkexec("nvidia-smi", "-pl", str(watts))
        self.gpu_note.set_text(f"Setting {watts} W — enter your password if asked")
        self.refresh_gpu_buttons()

    # apps

    def load_apps(self):
        in_background(top_apps, self.show_apps)

    def show_apps(self, apps):
        clear(self.apps)
        for app in apps:
            row = Gtk.Box(spacing=6)
            row.get_style_context().add_class("app-row")
            info = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
            info.pack_start(label(app["name"]), False, False, 0)
            state = " · boosted" if app["nice"] < 0 else " · background" if app["nice"] > 0 else ""
            info.pack_start(label(f"{app['cpu']:.0f}% cpu · {fmt_bytes(app['rss'])}{state}", "dim"),
                            False, False, 0)
            row.pack_start(info, True, True, 0)
            for icon, kind, on in (("󰒲", "background", app["nice"] > 0), ("󰓅", "boost", app["nice"] < 0)):
                btn = Gtk.Button()
                btn.add(label(icon, "icon", xalign=0.5))
                btn.set_tooltip_text(("Stop " if on else "") + ("boosting" if kind == "boost" else "running in background"))
                if on:
                    btn.get_style_context().add_class("active")
                btn.connect("clicked", self.set_priority, app, kind, on)
                row.pack_end(btn, False, False, 0)
            self.apps.pack_start(row, False, False, 0)
        if not apps:
            self.apps.pack_start(label("Nothing running", "dim"), False, False, 0)
        self.apps.show_all()

    def set_priority(self, _btn, app, kind, on):
        pids = [str(p) for p in app["pids"]]
        if kind == "boost":
            # gamemode toggles its boost for a pid and its passwordless for the gamemode group
            def work():
                for pid in pids:
                    run("gamemoded", f"-r{pid}")
            in_background(work, lambda _: self.load_apps())
        elif not on:
            run("renice", "-n", "15", "-p", *pids)
            run("ionice", "-c", "3", "-p", *pids)
            self.load_apps()
        else:
            # raising priority back up prolly needs root
            pkexec("sh", "-c", f"renice -n 0 -p {' '.join(pids)}; ionice -c 0 -p {' '.join(pids)}")
            GLib.timeout_add_seconds(3, lambda: self.load_apps() and False)


    def choice_button(self, top, bottom, active):
        btn = Gtk.Button()
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        box.pack_start(label(top, xalign=0.5), False, False, 0)
        box.pack_start(label(bottom, "dim", xalign=0.5), False, False, 0)
        btn.add(box)
        btn.get_style_context().add_class("choice")
        if active:
            btn.get_style_context().add_class("active")
        return btn


# window

def launch(*cmd):
    subprocess.Popen(cmd, start_new_session=True,
                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    Gtk.main_quit()


def open_settings(page, *fallback):
    """the settings app on page if its installed or else the fallback tool"""
    import shutil
    if shutil.which("sevenwm-settings"):
        launch("sevenwm-settings", "--page", page)
    else:
        launch(*fallback)


def sign_out():
    """quit whichever compositor this is like sevenwm over its socket or driftwm"""
    sock = os.environ.get("SEVENWM_SOCK")
    if sock:
        import socket
        try:
            with socket.socket(socket.AF_UNIX) as s:
                s.connect(sock)
                s.sendall(b'{"action": "quit"}\n')
            return
        except OSError:
            pass
    launch("driftwm", "msg", "action", "quit")


def toggle_existing():
    """close a tray thats already open and return True if we shouldnt open a new one"""
    try:
        with open(PIDFILE) as f:
            pid, mode = f.read().split()
        with open(f"/proc/{pid}/cmdline", "rb") as f:
            if b"tray.py" not in f.read():
                raise OSError("stale pidfile")
        os.kill(int(pid), signal.SIGTERM)
        return mode == MODE
    except (OSError, ValueError):
        pass
    # clicking the bar can steal focus and close the tray right before this runs
    try:
        with open(CLOSEDFILE) as f:
            mode, stamp = f.read().split()
        return mode == MODE and time.time() - float(stamp) < 0.4
    except (OSError, ValueError):
        return False


def main():
    if toggle_existing():
        return
    with open(PIDFILE, "w") as f:
        f.write(f"{os.getpid()} {MODE}")

    provider = Gtk.CssProvider()
    provider.load_from_data(CSS)
    Gtk.StyleContext.add_provider_for_screen(
        Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_USER)

    win = Gtk.Window()
    GtkLayerShell.init_for_window(win)
    GtkLayerShell.set_namespace(win, "waybar-tray")
    GtkLayerShell.set_layer(win, GtkLayerShell.Layer.OVERLAY)
    GtkLayerShell.set_anchor(win, GtkLayerShell.Edge.TOP, True)
    GtkLayerShell.set_anchor(win, GtkLayerShell.Edge.RIGHT, True)
    GtkLayerShell.set_margin(win, GtkLayerShell.Edge.TOP, 6)
    GtkLayerShell.set_margin(win, GtkLayerShell.Edge.RIGHT, 8)
    GtkLayerShell.set_keyboard_mode(win, GtkLayerShell.KeyboardMode.ON_DEMAND)

    panel = {"audio": AudioTray, "wifi": WifiTray,
             "bluetooth": BluetoothTray, "power": PowerTray, "perf": PerfTray}[MODE]()
    panel.set_name("panel")
    panel.set_size_request({"power": 220, "perf": 360}.get(MODE, 320), -1)
    win.add(panel)

    focused = {"seen": False}
    win.connect("focus-in-event", lambda *_: focused.update(seen=True))
    win.connect("focus-out-event", lambda *_: focused["seen"] and Gtk.main_quit())
    win.connect("key-press-event",
                lambda _w, e: e.keyval == Gdk.KEY_Escape and Gtk.main_quit())
    win.connect("destroy", Gtk.main_quit)
    GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGTERM, lambda *_: Gtk.main_quit() or False)

    win.show_all()
    try:
        Gtk.main()
    finally:
        try:
            with open(PIDFILE) as f:
                if f.read().split()[0] == str(os.getpid()):
                    os.remove(PIDFILE)
        except (OSError, IndexError):
            pass
        with open(CLOSEDFILE, "w") as f:
            f.write(f"{MODE} {time.time()}")


if __name__ == "__main__":
    main()
