//! dropdown trays for the bar like sevenshell tray audio wifi bluetooth power or perf and clicking the same module again or escape closes it

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk4::prelude::*;
use gtk4::{gdk, gio, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::settings::system::{Out, bt_devices, run_with, terse_split};

pub const CSS: &str = "
.sevenshell-tray { background: transparent; }
.sevenshell-tray .tray-panel {
    background: @m3surface;
    border: 1px solid alpha(@m3outlineVariant, 0.35);
    border-radius: 28px;
    padding: 16px;
}
.sevenshell-tray * { font-size: 13px; color: @m3onSurface; }
.sevenshell-tray .title { font-weight: 500; font-size: 16px; }
.sevenshell-tray .dim { font-size: 12px; color: @m3onSurfaceVariant; }
.sevenshell-tray .icon { font-size: 18px; color: @m3primary; }
.sevenshell-tray .dim.icon { color: @m3onSurfaceVariant; }
.sevenshell-tray .section { font-size: 12px; font-weight: 500; margin-top: 8px; color: @m3primary; }
.sevenshell-tray button {
    background: transparent; border: none; box-shadow: none;
    border-radius: 16px; padding: 6px 10px;
}
.sevenshell-tray button:hover { background: alpha(@m3onSurface, 0.08); }
.sevenshell-tray button.active { background: @m3secondaryContainer; }
.sevenshell-tray button.active label { color: @m3onSecondaryContainer; }
.sevenshell-tray button.footer { font-size: 12px; background: @m3surfaceContainerHigh; border-radius: 9999px; }
.sevenshell-tray button.footer:hover { background: @m3secondaryContainer; }
.sevenshell-tray button.danger { background: @m3error; border-radius: 9999px; }
.sevenshell-tray button.danger label { color: @m3onError; }
.sevenshell-tray scale trough {
    min-height: 10px; border-radius: 9999px; border: none;
    background: @m3surfaceContainerHighest;
}
.sevenshell-tray scale highlight { background: @m3primary; border-radius: 9999px; border: none; }
.sevenshell-tray scale slider {
    min-width: 18px; min-height: 18px; border-radius: 9999px;
    background: @m3primary; border: none; box-shadow: none; margin: -5px;
}
.sevenshell-tray switch { background: @m3surfaceContainerHighest; border: 2px solid @m3outline; border-radius: 9999px; }
.sevenshell-tray switch:checked { background: @m3primary; border-color: @m3primary; }
.sevenshell-tray switch slider { background: @m3outline; border: none; border-radius: 9999px; box-shadow: none; }
.sevenshell-tray switch:checked slider { background: @m3onPrimary; }
.sevenshell-tray entry {
    background: @m3surfaceContainerHigh; color: @m3onSurface;
    border: none; border-radius: 9999px;
    padding: 6px 14px; box-shadow: none; caret-color: @m3primary;
}
.sevenshell-tray .stat { background: @m3surfaceContainer; border: none; border-radius: 16px; padding: 10px 6px; }
.sevenshell-tray .stat-value { font-size: 16px; font-weight: 500; color: @m3primary; }
.sevenshell-tray button.choice { padding: 8px 4px; background: @m3surfaceContainer; }
.sevenshell-tray button.choice.active { background: @m3secondaryContainer; }
.sevenshell-tray .app-row { padding: 2px 0 2px 8px; }
.sevenshell-tray scrolledwindow, .sevenshell-tray viewport { background: transparent; border: none; }
";

thread_local! {
    /// the open tray and which one it is
    static OPEN: RefCell<Option<(String, gtk4::Window)>> = const { RefCell::new(None) };
    /// the last tray to close and when so a bar click that stole its focus doesnt open it right back
    static CLOSED: RefCell<Option<(String, Instant)>> = const { RefCell::new(None) };
}

/// whether the bars tray command means this built in one incl the old tray.py it replaced
pub fn is_builtin(command: &str) -> bool {
    let command = command.trim();
    command.is_empty() || command == "sevenshell tray" || command.ends_with("waybar/scripts/tray.py")
}

/// open the mode tray or close it if its the one open
pub fn toggle(mode: &str) {
    let open = OPEN.with(|o| o.borrow_mut().take());
    if let Some((was, window)) = open {
        window.close();
        if was == mode {
            return;
        }
    } else if CLOSED.with(|c| {
        c.borrow()
            .as_ref()
            .is_some_and(|(m, at)| m == mode && at.elapsed() < Duration::from_millis(400))
    }) {
        return;
    }
    let (panel, width) = match mode {
        "audio" => (Audio::build(), 320),
        "wifi" => (Wifi::build(), 320),
        "bluetooth" => (Bluetooth::build(), 320),
        "power" => (power(), 220),
        "perf" => (Perf::build(), 360),
        "quick" => (crate::quick::build(), 380),
        "media" => (crate::quick::media(), 340),
        "weather" => (crate::quick::weather(), 320),
        _ => {
            eprintln!("sevenshell: no tray called '{mode}'");
            return;
        }
    };
    panel.add_css_class("tray-panel");
    panel.set_size_request(width, -1);

    let window = gtk4::Window::new();
    if let Some(app) = app() {
        window.set_application(Some(&app));
    }
    window.init_layer_shell();
    window.set_namespace(Some("sevenshell-tray"));
    window.set_layer(Layer::Overlay);
    // it hangs off the bar so its on the bars edge and at the far end of it
    let edge = crate::bar::edge();
    let side = match edge {
        Edge::Left | Edge::Right => Edge::Top,
        _ => Edge::Right,
    };
    window.set_anchor(edge, true);
    window.set_anchor(side, true);
    window.set_margin(edge, crate::bar::tray_margin());
    window.set_margin(side, 8);
    window.set_keyboard_mode(KeyboardMode::OnDemand);
    window.add_css_class("sevenshell-tray");
    crate::style::adopt(&window);
    window.set_child(Some(&panel));

    // clicking anywhere else takes focus away and that closes it but only once it had focus
    let seen = Cell::new(false);
    window.connect_is_active_notify(move |window| {
        if window.is_active() {
            seen.set(true);
        } else if seen.get() {
            window.close();
        }
    });
    let keys = gtk4::EventControllerKey::new();
    keys.connect_key_pressed(|controller, key, _, _| {
        if key == gdk::Key::Escape
            && let Some(window) = controller.widget().and_downcast::<gtk4::Window>()
        {
            window.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    let name = mode.to_string();
    window.connect_close_request(move |window| {
        OPEN.with(|o| {
            let mut open = o.borrow_mut();
            if open.as_ref().is_some_and(|(_, w)| w == window) {
                *open = None;
            }
        });
        CLOSED.with(|c| *c.borrow_mut() = Some((name.clone(), Instant::now())));
        glib::Propagation::Proceed
    });
    window.present();
    OPEN.with(|o| *o.borrow_mut() = Some((mode.to_string(), window)));
}

/// whether a tray is showing so an autohide bar stays out
pub fn is_open() -> bool {
    OPEN.with(|o| o.borrow().is_some())
}

/// close whatever tray is open
pub(crate) fn close() {
    let open = OPEN.with(|o| o.borrow_mut().take());
    if let Some((_, window)) = open {
        window.close();
    }
}

pub(crate) fn app() -> Option<gtk4::Application> {
    gio::Application::default().and_downcast()
}

pub(crate) fn run(cmd: &[&str]) -> Out {
    run_with(cmd, 20, None)
}

/// start something that outlives the tray in its own session like a password prompt
pub(crate) fn detach(cmd: &[&str]) {
    let Some((program, args)) = cmd.split_first() else {
        return;
    };
    let mut command = Command::new(program);
    command
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    crate::run_detached(&mut command);
}

/// start cmd and close the tray
pub(crate) fn launch(cmd: &[&str]) {
    detach(cmd);
    close();
}

/// the settings app on page
pub(crate) fn open_settings(page: &str) {
    close();
    if let Some(app) = app() {
        crate::settings::open(&app, Some(page));
    }
}

/// quit whichever compositor this is like sevenwm over its socket or driftwm
fn sign_out() {
    if crate::ipc::request(serde_json::json!({ "action": "quit" })).is_err() {
        launch(&["driftwm", "msg", "action", "quit"]);
    }
}

pub(crate) fn label(text: &str, classes: &[&str], xalign: f32) -> gtk4::Label {
    let label = gtk4::Label::new(Some(text));
    label.set_xalign(xalign);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    for class in classes {
        label.add_css_class(class);
    }
    label
}

pub(crate) fn icon(name: &str, classes: &[&str]) -> gtk4::Label {
    let icon = crate::style::icon(name);
    for class in classes {
        icon.add_css_class(class);
    }
    icon
}

pub(crate) fn vbox(spacing: i32) -> gtk4::Box {
    gtk4::Box::new(gtk4::Orientation::Vertical, spacing)
}

pub(crate) fn hbox(spacing: i32) -> gtk4::Box {
    gtk4::Box::new(gtk4::Orientation::Horizontal, spacing)
}

pub(crate) fn clear(b: &gtk4::Box) {
    while let Some(child) = b.first_child() {
        b.remove(&child);
    }
}

/// a title on the left and whatever goes right of it
pub(crate) fn header(title: &str, right: &impl IsA<gtk4::Widget>) -> gtk4::Box {
    let header = hbox(8);
    let title = label(title, &["title"], 0.0);
    title.set_hexpand(true);
    header.append(&title);
    header.append(right);
    header
}

pub(crate) fn footer(text: &str, page: &'static str) -> gtk4::Button {
    let more = gtk4::Button::with_label(text);
    more.add_css_class("footer");
    more.connect_clicked(move |_| open_settings(page));
    more
}

/// a flat button holding an icon a label that fills and maybe something on the right
pub(crate) fn row_button(icon_name: &str, text: &str, right: Option<gtk4::Widget>, active: bool) -> gtk4::Button {
    let button = gtk4::Button::new();
    let row = hbox(8);
    row.append(&icon(icon_name, &[]));
    let text = label(text, &[], 0.0);
    text.set_hexpand(true);
    row.append(&text);
    if let Some(right) = right {
        row.append(&right);
    }
    button.set_child(Some(&row));
    if active {
        button.add_css_class("active");
    }
    button
}

pub(crate) fn switch() -> gtk4::Switch {
    let switch = gtk4::Switch::new();
    switch.set_valign(gtk4::Align::Center);
    switch
}

// audio

struct Audio {
    percent: gtk4::Label,
    mute: gtk4::Label,
    scale: gtk4::Scale,
    sinks: gtk4::Box,
    muted: Cell<bool>,
    /// set while we move the slider ourselves so it doesnt set the volume back
    quiet: Cell<bool>,
}

/// the default sink volume and mute and every sink w the default one
struct Sound {
    volume: u32,
    muted: bool,
    default: String,
    sinks: Vec<(String, String)>,
}

fn sound() -> Sound {
    let out = run(&["pactl", "get-sink-volume", "@DEFAULT_SINK@"]).stdout;
    let volume = out
        .split_whitespace()
        .find_map(|t| t.strip_suffix('%')?.parse().ok())
        .unwrap_or(0);
    let muted = run(&["pactl", "get-sink-mute", "@DEFAULT_SINK@"]).stdout.contains("yes");
    let default = run(&["pactl", "get-default-sink"]).stdout.trim().to_string();
    let list: Vec<serde_json::Value> =
        serde_json::from_str(&run(&["pactl", "-f", "json", "list", "sinks"]).stdout).unwrap_or_default();
    let sinks = list
        .iter()
        .filter_map(|s| {
            let name = s["name"].as_str()?.to_string();
            let description = s["description"].as_str().map_or_else(|| name.clone(), String::from);
            Some((name, description))
        })
        .collect();
    Sound { volume, muted, default, sinks }
}

impl Audio {
    fn build() -> gtk4::Box {
        let root = vbox(8);
        let percent = label("", &["dim"], 1.0);
        root.append(&header("Volume", &percent));

        let row = hbox(8);
        let mute_button = gtk4::Button::new();
        let mute = icon("volume_up", &[]);
        mute_button.set_child(Some(&mute));
        row.append(&mute_button);
        let scale = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, 100.0, 1.0);
        scale.set_draw_value(false);
        scale.set_hexpand(true);
        row.append(&scale);
        root.append(&row);

        root.append(&label("OUTPUT", &["section"], 0.0));
        let sinks = vbox(2);
        root.append(&sinks);
        root.append(&footer("Sound settings…", "sound"));

        let audio = Rc::new(Self {
            percent,
            mute,
            scale,
            sinks,
            muted: Cell::new(false),
            quiet: Cell::new(false),
        });
        let this = Rc::downgrade(&audio);
        audio.scale.connect_value_changed(move |scale| {
            if let Some(this) = this.upgrade() {
                this.set_volume(scale.value() as u32);
            }
        });
        let this = Rc::downgrade(&audio);
        mute_button.connect_clicked(move |_| {
            if let Some(this) = this.upgrade() {
                run(&["pactl", "set-sink-mute", "@DEFAULT_SINK@", "toggle"]);
                this.muted.set(!this.muted.get());
                this.labels(this.scale.value() as u32, this.muted.get());
            }
        });
        audio.refresh();
        // the widgets hold the tray and the tray holds only them so it lives while its shown
        let keep = audio.clone();
        root.connect_destroy(move |_| {
            let _ = &keep;
        });
        root
    }

    fn refresh(self: &Rc<Self>) {
        let this = Rc::downgrade(self);
        crate::wake::off_thread(sound, move |sound| {
            if let Some(this) = this.upgrade() {
                this.show(sound);
            }
        });
    }

    fn show(self: &Rc<Self>, sound: Sound) {
        self.quiet.set(true);
        self.scale.set_value(sound.volume.min(100) as f64);
        self.quiet.set(false);
        self.muted.set(sound.muted);
        self.labels(sound.volume, sound.muted);
        clear(&self.sinks);
        for (name, description) in sound.sinks {
            let active = name == sound.default;
            let button = row_button(if active { "check" } else { "" }, &description, None, active);
            let this = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(this) = this.upgrade() {
                    run(&["pactl", "set-default-sink", &name]);
                    this.refresh();
                }
            });
            self.sinks.append(&button);
        }
    }

    fn labels(&self, volume: u32, muted: bool) {
        self.percent
            .set_text(&if muted { "muted".into() } else { format!("{volume}%") });
        let icon = match volume {
            _ if muted => "volume_off",
            0..34 => "volume_mute",
            34..67 => "volume_down",
            _ => "volume_up",
        };
        self.mute.set_text(icon);
    }

    fn set_volume(&self, volume: u32) {
        if self.quiet.get() {
            return;
        }
        run(&["pactl", "set-sink-volume", "@DEFAULT_SINK@", &format!("{volume}%")]);
        if self.muted.replace(false) {
            run(&["pactl", "set-sink-mute", "@DEFAULT_SINK@", "0"]);
        }
        self.labels(volume, false);
    }
}

// wifi

const SIGNAL_ICONS: [&str; 5] = [
    "signal_wifi_0_bar",
    "network_wifi_1_bar",
    "network_wifi_2_bar",
    "network_wifi_3_bar",
    "signal_wifi_4_bar",
];

#[derive(Clone)]
struct Net {
    active: bool,
    ssid: String,
    signal: u32,
    secure: bool,
}

/// every network once w the one in use or the strongest copy kept
fn networks(rescan: bool) -> Vec<Net> {
    let out = run(&[
        "nmcli", "-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "dev", "wifi", "list", "--rescan",
        if rescan { "yes" } else { "no" },
    ])
    .stdout;
    let mut seen: HashMap<String, Net> = HashMap::new();
    for line in out.lines() {
        let f = terse_split(line);
        if f.len() < 4 || f[1].is_empty() {
            continue;
        }
        let net = Net {
            active: f[0] == "*",
            ssid: f[1].clone(),
            signal: f[2].parse().unwrap_or(0),
            secure: !matches!(f[3].as_str(), "" | "--"),
        };
        let better = seen.get(&net.ssid).is_none_or(|old| {
            net.active || (!old.active && net.signal > old.signal)
        });
        if better {
            seen.insert(net.ssid.clone(), net);
        }
    }
    let mut nets: Vec<Net> = seen.into_values().collect();
    nets.sort_by_key(|n| (!n.active, std::cmp::Reverse(n.signal)));
    nets
}

struct Wifi {
    switch: gtk4::Switch,
    status: gtk4::Label,
    scroller: gtk4::ScrolledWindow,
    networks: gtk4::Box,
    password: RefCell<Option<gtk4::Entry>>,
}

impl Wifi {
    fn build() -> gtk4::Box {
        let root = vbox(8);
        let switch = switch();
        root.append(&header("Wi-Fi", &switch));
        let status = label("", &["dim"], 0.0);
        root.append(&status);
        let scroller = gtk4::ScrolledWindow::new();
        scroller.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scroller.set_propagate_natural_height(true);
        scroller.set_max_content_height(320);
        let list = vbox(2);
        scroller.set_child(Some(&list));
        scroller.set_vexpand(true);
        root.append(&scroller);
        root.append(&footer("Network settings…", "network"));

        let enabled = run(&["nmcli", "radio", "wifi"]).stdout.trim() == "enabled";
        switch.set_active(enabled);
        let wifi = Rc::new(Self {
            switch,
            status,
            scroller,
            networks: list,
            password: RefCell::default(),
        });
        let this = Rc::downgrade(&wifi);
        wifi.switch.connect_active_notify(move |switch| {
            if let Some(this) = this.upgrade() {
                this.toggle_radio(switch.is_active());
            }
        });
        if enabled {
            wifi.show(networks(false));
            wifi.status.set_text("Scanning…");
            wifi.fetch(move || networks(true));
        } else {
            wifi.show(Vec::new());
        }
        let keep = wifi.clone();
        root.connect_destroy(move |_| {
            let _ = &keep;
        });
        root
    }

    /// get the list off the main thread then show it
    fn fetch(self: &Rc<Self>, work: impl FnOnce() -> Vec<Net> + Send + 'static) {
        let this = Rc::downgrade(self);
        crate::wake::off_thread(work, move |nets| {
            if let Some(this) = this.upgrade() {
                this.show(nets);
            }
        });
    }

    fn show(self: &Rc<Self>, nets: Vec<Net>) {
        clear(&self.networks);
        self.password.borrow_mut().take();
        let text = if !self.switch.is_active() {
            "Wi-Fi is off".to_string()
        } else if nets.is_empty() {
            "No networks found".to_string()
        } else {
            nets.iter()
                .find(|n| n.active)
                .map_or_else(|| "Not connected".into(), |n| format!("Connected to {}", n.ssid))
        };
        self.status.set_text(&text);
        for net in &nets {
            self.networks.append(&self.row(net));
        }
        self.scroller.set_min_content_height(nets.len().min(8) as i32 * 38);
    }

    fn row(self: &Rc<Self>, net: &Net) -> gtk4::Box {
        let wrap = vbox(4);
        let right: Option<gtk4::Widget> = if net.active {
            Some(label("connected", &["dim"], 1.0).upcast())
        } else if net.secure {
            Some(icon("lock", &["dim"]).upcast())
        } else {
            None
        };
        let signal = SIGNAL_ICONS[(net.signal / 20).min(4) as usize];
        let button = row_button(signal, &net.ssid, right, net.active);
        let this = Rc::downgrade(self);
        let (net, weak_wrap) = (net.clone(), wrap.downgrade());
        button.connect_clicked(move |_| {
            if let (Some(this), Some(wrap)) = (this.upgrade(), weak_wrap.upgrade()) {
                this.clicked(&net, &wrap);
            }
        });
        wrap.append(&button);
        wrap
    }

    fn clicked(self: &Rc<Self>, net: &Net, wrap: &gtk4::Box) {
        if net.active {
            self.status.set_text("Disconnecting…");
            let ssid = net.ssid.clone();
            self.after(move || run(&["nmcli", "con", "down", "id", &ssid]));
            return;
        }
        let known = run(&["nmcli", "-t", "-f", "NAME,TYPE", "con", "show"])
            .stdout
            .lines()
            .any(|l| {
                let f = terse_split(l);
                f.len() >= 2 && f[0] == net.ssid && f[1] == "802-11-wireless"
            });
        if known || !net.secure {
            self.connect_to(net, None, wrap);
        } else {
            self.ask_password(net, wrap);
        }
    }

    fn ask_password(self: &Rc<Self>, net: &Net, wrap: &gtk4::Box) {
        if let Some(old) = self.password.borrow_mut().take()
            && let Some(parent) = old.parent().and_downcast::<gtk4::Box>()
        {
            parent.remove(&old);
        }
        let entry = gtk4::Entry::new();
        entry.set_visibility(false);
        entry.set_placeholder_text(Some("Password"));
        let this = Rc::downgrade(self);
        let (net, weak_wrap) = (net.clone(), wrap.downgrade());
        entry.connect_activate(move |entry| {
            if let (Some(this), Some(wrap)) = (this.upgrade(), weak_wrap.upgrade()) {
                this.connect_to(&net, Some(entry.text().to_string()), &wrap);
            }
        });
        wrap.append(&entry);
        entry.grab_focus();
        *self.password.borrow_mut() = Some(entry);
    }

    fn connect_to(self: &Rc<Self>, net: &Net, password: Option<String>, wrap: &gtk4::Box) {
        self.status.set_text(&format!("Connecting to {}…", net.ssid));
        let ssid = net.ssid.clone();
        let this = Rc::downgrade(self);
        let (net, weak_wrap) = (net.clone(), wrap.downgrade());
        crate::wake::off_thread(
            move || {
                let mut cmd = vec!["nmcli", "dev", "wifi", "connect", &ssid];
                if let Some(password) = &password {
                    cmd.extend(["password", password]);
                }
                run(&cmd)
            },
            move |out| {
                let Some(this) = this.upgrade() else {
                    return;
                };
                if !out.ok && net.secure {
                    this.status.set_text("Couldn't connect — check the password");
                    if let Some(wrap) = weak_wrap.upgrade() {
                        this.ask_password(&net, &wrap);
                    }
                } else {
                    this.fetch(move || networks(false));
                }
            },
        );
    }

    /// do work off the main thread then list the networks again
    fn after(self: &Rc<Self>, work: impl FnOnce() -> Out + Send + 'static) {
        self.fetch(move || {
            work();
            networks(false)
        });
    }

    fn toggle_radio(self: &Rc<Self>, on: bool) {
        run(&["nmcli", "radio", "wifi", if on { "on" } else { "off" }]);
        if on {
            self.status.set_text("Scanning…");
            self.fetch(|| {
                std::thread::sleep(Duration::from_secs(2));
                networks(true)
            });
        } else {
            self.show(Vec::new());
        }
    }
}

// bluetooth

fn looks_unnamed(mac: &str, name: &str) -> bool {
    name.replace('-', ":").eq_ignore_ascii_case(mac)
}

/// paired connected and everything else bluetoothctl knows
struct Devices {
    paired: Vec<(String, String)>,
    connected: Vec<(String, String)>,
    others: Vec<(String, String)>,
}

fn devices() -> Devices {
    let paired = bt_devices(Some("Paired"));
    let connected = bt_devices(Some("Connected"));
    let mut others: Vec<(String, String)> = bt_devices(None)
        .into_iter()
        .filter(|(m, n)| !paired.iter().any(|(p, _)| p == m) && !looks_unnamed(m, n))
        .collect();
    others.sort_by(|a, b| a.1.cmp(&b.1));
    Devices { paired, connected, others }
}

struct Bluetooth {
    switch: gtk4::Switch,
    status: gtk4::Label,
    scroller: gtk4::ScrolledWindow,
    devices: gtk4::Box,
    scan: gtk4::Button,
    scanning: Cell<bool>,
    /// what its doing rn which wins over the usual status
    busy: RefCell<Option<String>>,
    /// a failure shown once instead of the usual status
    failed: RefCell<Option<String>>,
}

impl Bluetooth {
    fn build() -> gtk4::Box {
        let root = vbox(8);
        let switch = switch();
        root.append(&header("Bluetooth", &switch));
        let status = label("", &["dim"], 0.0);
        root.append(&status);
        let scroller = gtk4::ScrolledWindow::new();
        scroller.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        let devices = vbox(2);
        scroller.set_child(Some(&devices));
        scroller.set_visible(false);
        scroller.set_vexpand(true);
        root.append(&scroller);
        let scan = gtk4::Button::with_label("Scan for devices");
        root.append(&scan);
        root.append(&footer("Bluetooth settings…", "bluetooth"));

        switch.set_active(run(&["bluetoothctl", "show"]).stdout.contains("Powered: yes"));
        let bt = Rc::new(Self {
            switch,
            status,
            scroller,
            devices,
            scan,
            scanning: Cell::new(false),
            busy: RefCell::default(),
            failed: RefCell::default(),
        });
        let this = Rc::downgrade(&bt);
        bt.switch.connect_active_notify(move |switch| {
            if let Some(this) = this.upgrade() {
                run(&["bluetoothctl", "power", if switch.is_active() { "on" } else { "off" }]);
                this.refresh();
            }
        });
        let this = Rc::downgrade(&bt);
        bt.scan.connect_clicked(move |_| {
            if let Some(this) = this.upgrade() {
                this.start_scan();
            }
        });
        bt.refresh();
        let keep = bt.clone();
        root.connect_destroy(move |_| {
            let _ = &keep;
        });
        root
    }

    fn refresh(self: &Rc<Self>) {
        let powered = self.switch.is_active();
        let scanning = self.scanning.get();
        self.scan.set_sensitive(powered && !scanning);
        self.scan
            .set_label(if scanning { "Scanning…" } else { "Scan for devices" });
        if !powered {
            clear(&self.devices);
            self.scroller.set_visible(false);
            self.status.set_text("Bluetooth is off");
            return;
        }
        let this = Rc::downgrade(self);
        crate::wake::off_thread(devices, move |devices| {
            if let Some(this) = this.upgrade() {
                this.show(devices);
            }
        });
    }

    fn show(self: &Rc<Self>, devices: Devices) {
        clear(&self.devices);
        let is_connected = |mac: &str| devices.connected.iter().any(|(m, _)| m == mac);
        let mut paired = devices.paired.clone();
        paired.sort_by(|a, b| (!is_connected(&a.0), &a.1).cmp(&(!is_connected(&b.0), &b.1)));
        if !paired.is_empty() {
            self.devices.append(&label("PAIRED", &["section"], 0.0));
        }
        for (mac, name) in &paired {
            self.devices.append(&self.device(mac, name, is_connected(mac), true));
        }
        if !devices.others.is_empty() {
            self.devices.append(&label("AVAILABLE", &["section"], 0.0));
        }
        for (mac, name) in &devices.others {
            self.devices.append(&self.device(mac, name, false, false));
        }
        let text = if let Some(busy) = self.busy.borrow().clone() {
            busy
        } else if let Some(failed) = self.failed.borrow_mut().take() {
            failed
        } else if !devices.connected.is_empty() {
            let names: Vec<&str> = devices.connected.iter().map(|(_, n)| n.as_str()).collect();
            format!("Connected to {}", names.join(", "))
        } else if paired.is_empty() && devices.others.is_empty() {
            "No devices yet — scan to find some".into()
        } else {
            "Not connected".into()
        };
        self.status.set_text(&text);
        let rows = paired.len() + devices.others.len();
        self.scroller
            .set_min_content_height(rows.min(7) as i32 * 38 + 30);
        self.scroller.set_visible(rows > 0);
    }

    fn device(self: &Rc<Self>, mac: &str, name: &str, connected: bool, paired: bool) -> gtk4::Button {
        let right = connected.then(|| label("connected", &["dim"], 1.0).upcast());
        let icon = if connected { "bluetooth_connected" } else { "bluetooth" };
        let button = row_button(icon, name, right, connected);
        let this = Rc::downgrade(self);
        let (mac, name) = (mac.to_string(), name.to_string());
        button.connect_clicked(move |_| {
            if let Some(this) = this.upgrade() {
                this.clicked(&mac, &name, connected, paired);
            }
        });
        button
    }

    fn clicked(self: &Rc<Self>, mac: &str, name: &str, connected: bool, paired: bool) {
        let (busy, steps): (String, &[&str]) = if connected {
            (format!("Disconnecting {name}…"), &["disconnect"])
        } else if paired {
            (format!("Connecting to {name}…"), &["connect"])
        } else {
            (format!("Pairing with {name}…"), &["pair", "trust", "connect"])
        };
        *self.busy.borrow_mut() = Some(busy);
        self.refresh();
        let this = Rc::downgrade(self);
        let (mac, name) = (mac.to_string(), name.to_string());
        let steps: Vec<&'static str> = steps.to_vec();
        crate::wake::off_thread(
            move || {
                let mut last = Out::default();
                for step in steps {
                    last = run(&["bluetoothctl", step, &mac]);
                }
                last
            },
            move |out| {
                if let Some(this) = this.upgrade() {
                    this.busy.borrow_mut().take();
                    if !out.ok {
                        *this.failed.borrow_mut() = Some(format!("Couldn't connect to {name}"));
                    }
                    this.refresh();
                }
            },
        );
    }

    fn start_scan(self: &Rc<Self>) {
        self.scanning.set(true);
        self.refresh();
        // show what turns up while it looks
        let this = Rc::downgrade(self);
        glib::timeout_add_seconds_local(2, move || match this.upgrade() {
            Some(this) if this.scanning.get() => {
                this.refresh();
                glib::ControlFlow::Continue
            }
            _ => glib::ControlFlow::Break,
        });
        let this = Rc::downgrade(self);
        crate::wake::off_thread(
            || run(&["bluetoothctl", "--timeout", "10", "scan", "on"]),
            move |_| {
                if let Some(this) = this.upgrade() {
                    this.scanning.set(false);
                    this.refresh();
                }
            },
        );
    }
}

// power

/// a power button w its name and label so arming one can put the others back
type PowerButton = (gtk4::Button, &'static str, gtk4::Label);

fn power() -> gtk4::Box {
    let root = vbox(2);
    let actions: [(&str, &str, Option<&'static [&'static str]>); 3] = [
        ("power_settings_new", "Shut down", Some(&["systemctl", "poweroff"])),
        ("restart_alt", "Restart", Some(&["systemctl", "reboot"])),
        ("logout", "Sign out", None),
    ];
    let armed: Rc<RefCell<Option<&str>>> = Rc::default();
    let buttons: Rc<RefCell<Vec<PowerButton>>> = Rc::default();
    for (icon_name, name, cmd) in actions {
        let button = gtk4::Button::new();
        let row = hbox(10);
        row.append(&icon(icon_name, &[]));
        let text = label(name, &[], 0.0);
        text.set_hexpand(true);
        row.append(&text);
        button.set_child(Some(&row));
        let (armed, all, text2) = (armed.clone(), Rc::downgrade(&buttons), text.clone());
        button.connect_clicked(move |button| {
            // the first click arms it and the second one does it
            if *armed.borrow() == Some(name) {
                match cmd {
                    Some(cmd) => launch(cmd),
                    None => sign_out(),
                }
                return;
            }
            *armed.borrow_mut() = Some(name);
            if let Some(all) = all.upgrade() {
                for (b, n, t) in all.borrow().iter() {
                    t.set_text(n);
                    b.remove_css_class("danger");
                }
            }
            text2.set_text(&format!("Click again to {}", name.to_lowercase()));
            button.add_css_class("danger");
        });
        buttons.borrow_mut().push((button.clone(), name, text));
        root.append(&button);
    }
    root.append(&footer("Power settings…", "power"));
    root.connect_destroy(move |_| {
        let _ = &buttons;
    });
    root
}

// performance

const PROFILES: [(&str, &str, &str); 3] = [
    ("power-saver", "eco", "Saver"),
    ("balanced", "balance", "Balanced"),
    ("performance", "bolt", "Max"),
];
const GPU_PRESETS: [(u32, &str); 3] = [(175, "Quiet"), (230, "Balanced"), (285, "Max")];
/// things that shouldnt get reprioritized from here like shells audio and the desktop itself
const SKIP_APPS: &[&str] = &[
    "bash", "zsh", "fish", "sh", "ps", "sleep", "systemd", "dbus-broker", "dbus-broker-launch",
    "dbus-daemon", "pipewire", "pipewire-pulse", "wireplumber", "driftwm", "sevenwm", "sevenshell",
    "Xwayland", "waybar", "gamemoded", "xdg-desktop-portal", "xdg-document-portal",
    "xdg-permission-store", "hyprpolkitagent", "at-spi-bus-launcher", "at-spi2-registryd", "gvfsd",
    "fusermount3",
];

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// total and idle cpu ticks
fn cpu_times() -> (u64, u64) {
    let stat = read("/proc/stat");
    let fields: Vec<u64> = stat
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .skip(1)
        .filter_map(|f| f.parse().ok())
        .collect();
    let idle = fields.get(3).copied().unwrap_or(0) + fields.get(4).copied().unwrap_or(0);
    (fields.iter().sum(), idle)
}

fn cpu_temp() -> Option<u64> {
    std::fs::read_dir("/sys/class/hwmon").ok()?.flatten().find_map(|h| {
        let dir = h.path();
        (read(&format!("{}/name", dir.display())).trim() == "k10temp")
            .then(|| read(&format!("{}/temp1_input", dir.display())).trim().parse::<u64>().ok())
            .flatten()
            .map(|t| t / 1000)
    })
}

#[derive(Clone, Copy)]
struct GpuStats {
    util: f64,
    temp: f64,
    draw: f64,
    limit: f64,
}

fn gpu_stats() -> Option<GpuStats> {
    let out = run(&[
        "nvidia-smi",
        "--query-gpu=utilization.gpu,temperature.gpu,power.draw,power.limit",
        "--format=csv,noheader,nounits",
    ]);
    let f: Vec<f64> = out.stdout.trim().split(',').map(|x| x.trim().parse().ok()).collect::<Option<_>>()?;
    let [util, temp, draw, limit] = f[..] else {
        return None;
    };
    Some(GpuStats { util, temp, draw, limit })
}

/// app name cpu ticks rss bytes and nice for each of this users processes
fn my_processes() -> HashMap<u32, (String, u64, u64, i64)> {
    // safety plain getters w no arguments
    let (uid, page, me) = unsafe {
        (libc::getuid(), libc::sysconf(libc::_SC_PAGESIZE) as u64, libc::getpid() as u32)
    };
    let mut procs = HashMap::new();
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return procs;
    };
    for entry in dir.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|p| p.parse::<u32>().ok()) else {
            continue;
        };
        if pid == me || !entry.metadata().is_ok_and(|m| m.uid() == uid) {
            continue;
        }
        let Ok(exe) = std::fs::read_link(format!("/proc/{pid}/exe")) else {
            continue;
        };
        let mut name = exe
            .file_name()
            .map(|n| n.to_string_lossy().trim_end_matches(" (deleted)").to_string())
            .unwrap_or_default();
        // like a binary named after its version
        if !name.chars().any(char::is_alphabetic) {
            let comm = read(&format!("/proc/{pid}/comm")).trim().to_string();
            if !comm.is_empty() {
                name = comm;
            }
        }
        let stat = read(&format!("/proc/{pid}/stat"));
        let Some((_, rest)) = stat.rsplit_once(')') else {
            continue;
        };
        let rest: Vec<&str> = rest.split_whitespace().collect();
        let field = |i: usize| rest.get(i).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
        let ticks = (field(11) + field(12)) as u64;
        let nice = field(16);
        let rss = read(&format!("/proc/{pid}/statm"))
            .split_whitespace()
            .nth(1)
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
            * page;
        procs.insert(pid, (name, ticks, rss, nice));
    }
    procs
}

/// ur own processes grouped by program
struct AppUse {
    name: String,
    pids: Vec<u32>,
    cpu: f64,
    rss: u64,
    nice: i64,
}

/// the busiest apps over a second
fn top_apps() -> Vec<AppUse> {
    let interval = 1.0;
    // safety plain getter
    let tick = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
    let cpus = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
    let before = my_processes();
    std::thread::sleep(Duration::from_secs_f64(interval));
    let after = my_processes();
    let mut apps: HashMap<String, (AppUse, Vec<i64>)> = HashMap::new();
    for (pid, (name, ticks, rss, nice)) in after {
        if SKIP_APPS.contains(&name.as_str())
            || ["python", "xdg-", "chrome_crashpad"].iter().any(|p| name.starts_with(p))
        {
            continue;
        }
        let (app, nices) = apps.entry(name.clone()).or_insert_with(|| {
            let app = AppUse { name: name.clone(), pids: Vec::new(), cpu: 0.0, rss: 0, nice: 0 };
            (app, Vec::new())
        });
        if let Some((was, prev, _, _)) = before.get(&pid)
            && *was == name
        {
            app.cpu += ticks.saturating_sub(*prev) as f64 / tick / interval / cpus * 100.0;
        }
        app.rss += rss;
        app.pids.push(pid);
        nices.push(nice);
    }
    let mut apps: Vec<AppUse> = apps
        .into_values()
        .map(|(mut app, nices)| {
            // the most common nice value wins so one odd helper doesnt mislabel the app
            app.nice = nices
                .iter()
                .copied()
                .max_by_key(|n| nices.iter().filter(|m| *m == n).count())
                .unwrap_or(0);
            app
        })
        // skip small helpers so the list shows real apps
        .filter(|a| a.rss > 40 << 20 || a.cpu >= 1.0)
        .collect();
    apps.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.rss.cmp(&a.rss)));
    apps.truncate(6);
    apps
}

fn fmt_bytes(n: u64) -> String {
    if n >= 1 << 30 {
        format!("{:.1}G", n as f64 / (1u64 << 30) as f64)
    } else {
        format!("{:.0}M", n as f64 / (1u64 << 20) as f64)
    }
}

/// a stat tile w a big value and a caption under it
fn stat(parent: &gtk4::Box, caption: &str) -> (gtk4::Label, gtk4::Label) {
    let tile = vbox(0);
    tile.add_css_class("stat");
    let value = label("–", &["stat-value"], 0.5);
    let sub = label(caption, &["dim"], 0.5);
    tile.append(&value);
    tile.append(&sub);
    tile.set_hexpand(true);
    parent.append(&tile);
    (value, sub)
}

fn choice(top: &str, bottom: &str, active: bool) -> gtk4::Button {
    let button = gtk4::Button::new();
    let b = vbox(0);
    b.append(&label(top, &[], 0.5));
    b.append(&label(bottom, &["dim"], 0.5));
    button.set_child(Some(&b));
    button.add_css_class("choice");
    button.set_hexpand(true);
    if active {
        button.add_css_class("active");
    }
    button
}

struct Perf {
    mode: gtk4::Label,
    cpu: (gtk4::Label, gtk4::Label),
    gpu_stat: (gtk4::Label, gtk4::Label),
    ram: (gtk4::Label, gtk4::Label),
    profiles: gtk4::Box,
    gpu_row: gtk4::Box,
    gpu_note: gtk4::Label,
    apps: gtk4::Box,
    gpu: Cell<Option<GpuStats>>,
    pending_limit: Cell<Option<u32>>,
    cpu_prev: Cell<(u64, u64)>,
}

impl Perf {
    fn build() -> gtk4::Box {
        let root = vbox(8);
        let mode = label("", &["dim"], 1.0);
        root.append(&header("Performance", &mode));

        let stats = hbox(6);
        stats.set_homogeneous(true);
        let cpu = stat(&stats, "CPU");
        let gpu_stat = stat(&stats, "GPU");
        let ram = stat(&stats, "RAM");
        root.append(&stats);

        root.append(&label("POWER MODE", &["section"], 0.0));
        let profiles = hbox(4);
        profiles.set_homogeneous(true);
        root.append(&profiles);

        root.append(&label("GPU POWER LIMIT", &["section"], 0.0));
        let gpu_row = hbox(4);
        gpu_row.set_homogeneous(true);
        root.append(&gpu_row);
        let gpu_note = label("", &["dim"], 0.0);
        root.append(&gpu_note);

        let apps_header = hbox(4);
        let section = label("APPS", &["section"], 0.0);
        section.set_hexpand(true);
        apps_header.append(&section);
        apps_header.append(&icon("rocket_launch", &["dim"]));
        apps_header.append(&label("boost", &["dim"], 1.0));
        apps_header.append(&icon("bedtime", &["dim"]));
        apps_header.append(&label("background", &["dim"], 1.0));
        root.append(&apps_header);
        let apps = vbox(2);
        apps.append(&label("Measuring…", &["dim"], 0.0));
        root.append(&apps);

        let perf = Rc::new(Self {
            mode,
            cpu,
            gpu_stat,
            ram,
            profiles,
            gpu_row,
            gpu_note,
            apps,
            gpu: Cell::new(None),
            pending_limit: Cell::new(None),
            cpu_prev: Cell::new(cpu_times()),
        });
        perf.refresh_profiles();
        perf.refresh_gpu_buttons();
        perf.tick();
        let this = Rc::downgrade(&perf);
        glib::timeout_add_seconds_local(2, move || {
            let Some(this) = this.upgrade() else {
                return glib::ControlFlow::Break;
            };
            this.tick();
            glib::ControlFlow::Continue
        });
        perf.load_apps();
        let keep = perf.clone();
        root.connect_destroy(move |_| {
            let _ = &keep;
        });
        root
    }

    /// the live numbers
    fn tick(self: &Rc<Self>) {
        let (total, idle) = cpu_times();
        let (was_total, was_idle) = self.cpu_prev.replace((total, idle));
        let (dt, di) = (total.saturating_sub(was_total), idle.saturating_sub(was_idle));
        if dt > 0 {
            self.cpu
                .0
                .set_text(&format!("{:.0}%", (1.0 - di as f64 / dt as f64) * 100.0));
        }
        self.cpu.1.set_text(&match cpu_temp() {
            Some(t) => format!("CPU · {t}°"),
            None => "CPU".into(),
        });

        let meminfo = read("/proc/meminfo");
        let kb = |key: &str| {
            meminfo
                .lines()
                .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
                .and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok())
                .unwrap_or(0)
                * 1024
        };
        self.ram.0.set_text(&fmt_bytes(kb("MemTotal").saturating_sub(kb("MemAvailable"))));
        self.ram.1.set_text(&format!("of {}", fmt_bytes(kb("MemTotal"))));

        let this = Rc::downgrade(self);
        crate::wake::off_thread(gpu_stats, move |gpu| {
            if let Some(this) = this.upgrade() {
                this.show_gpu(gpu);
            }
        });
    }

    fn show_gpu(self: &Rc<Self>, gpu: Option<GpuStats>) {
        self.gpu.set(gpu);
        let Some(gpu) = gpu else {
            self.gpu_stat.0.set_text("–");
            self.gpu_note.set_text("GPU not available");
            return;
        };
        self.gpu_stat.0.set_text(&format!("{:.0}%", gpu.util));
        self.gpu_stat.1.set_text(&format!("GPU · {:.0}°", gpu.temp));
        if self.pending_limit.get().is_some_and(|w| (gpu.limit - w as f64).abs() < 1.0) {
            self.pending_limit.set(None);
        }
        let note = match self.pending_limit.get() {
            Some(w) => format!("Setting {w} W — enter your password if asked"),
            None => format!("Drawing {:.0} W of {:.0} W · resets on reboot", gpu.draw, gpu.limit),
        };
        self.gpu_note.set_text(&note);
        self.refresh_gpu_buttons();
    }

    fn refresh_profiles(self: &Rc<Self>) {
        let current = run(&["powerprofilesctl", "get"]).stdout.trim().to_string();
        clear(&self.profiles);
        for (key, icon_name, name) in PROFILES {
            let button = choice(icon_name, name, key == current);
            // the top line is an icon
            if let Some(top) = button.child().and_then(|b| b.first_child()) {
                top.add_css_class("icon");
            }
            let this = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(this) = this.upgrade() {
                    run(&["powerprofilesctl", "set", key]);
                    this.refresh_profiles();
                }
            });
            self.profiles.append(&button);
            if key == current {
                self.mode.set_text(&name.to_lowercase());
            }
        }
    }

    fn refresh_gpu_buttons(self: &Rc<Self>) {
        clear(&self.gpu_row);
        let limit = self
            .pending_limit
            .get()
            .map(f64::from)
            .or(self.gpu.get().map(|g| g.limit));
        for (watts, name) in GPU_PRESETS {
            let active = limit.is_some_and(|l| (l - watts as f64).abs() < 1.0);
            let button = choice(&format!("{watts}W"), name, active);
            let this = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(this) = this.upgrade() {
                    this.pending_limit.set(Some(watts));
                    // detached so the password prompt survives the tray closing when it takes focus
                    detach(&["pkexec", "nvidia-smi", "-pl", &watts.to_string()]);
                    this.gpu_note
                        .set_text(&format!("Setting {watts} W — enter your password if asked"));
                    this.refresh_gpu_buttons();
                }
            });
            self.gpu_row.append(&button);
        }
    }

    fn load_apps(self: &Rc<Self>) {
        let this = Rc::downgrade(self);
        crate::wake::off_thread(top_apps, move |apps| {
            if let Some(this) = this.upgrade() {
                this.show_apps(apps);
            }
        });
    }

    fn show_apps(self: &Rc<Self>, apps: Vec<AppUse>) {
        clear(&self.apps);
        if apps.is_empty() {
            self.apps.append(&label("Nothing running", &["dim"], 0.0));
        }
        for app in apps {
            let row = hbox(6);
            row.add_css_class("app-row");
            let info = vbox(0);
            info.set_hexpand(true);
            info.append(&label(&app.name, &[], 0.0));
            let state = match app.nice {
                n if n < 0 => " · boosted",
                n if n > 0 => " · background",
                _ => "",
            };
            let usage = format!("{:.0}% cpu · {}{state}", app.cpu, fmt_bytes(app.rss));
            info.append(&label(&usage, &["dim"], 0.0));
            row.append(&info);
            let pids: Rc<Vec<String>> = Rc::new(app.pids.iter().map(u32::to_string).collect());
            for (icon_name, boost, on) in [("rocket_launch", true, app.nice < 0), ("bedtime", false, app.nice > 0)] {
                let button = gtk4::Button::new();
                button.set_child(Some(&icon(icon_name, &[])));
                let what = if boost { "boosting" } else { "running in background" };
                let tip = if on { format!("Stop {what}") } else { what[..1].to_uppercase() + &what[1..] };
                button.set_tooltip_text(Some(&tip));
                if on {
                    button.add_css_class("active");
                }
                let (this, pids) = (Rc::downgrade(self), pids.clone());
                button.connect_clicked(move |_| {
                    if let Some(this) = this.upgrade() {
                        this.set_priority(&pids, boost, on);
                    }
                });
                row.append(&button);
            }
            self.apps.append(&row);
        }
    }

    fn set_priority(self: &Rc<Self>, pids: &[String], boost: bool, on: bool) {
        let pids = pids.to_vec();
        let this = Rc::downgrade(self);
        let reload = move || {
            if let Some(this) = this.upgrade() {
                this.load_apps();
            }
        };
        if boost {
            // gamemode toggles its boost for a pid and its passwordless for the gamemode group
            crate::wake::off_thread(
                move || {
                    for pid in &pids {
                        run(&["gamemoded", &format!("-r{pid}")]);
                    }
                },
                move |_| reload(),
            );
        } else if !on {
            let mut renice = vec!["renice", "-n", "15", "-p"];
            renice.extend(pids.iter().map(String::as_str));
            run(&renice);
            let mut ionice = vec!["ionice", "-c", "3", "-p"];
            ionice.extend(pids.iter().map(String::as_str));
            run(&ionice);
            reload();
        } else {
            // ur own io class goes back without root but raising the nice back up needs it
            // and pkexec runs renice itself so the prompt says renice and not a root shell
            let mut ionice = vec!["ionice", "-c", "0", "-p"];
            ionice.extend(pids.iter().map(String::as_str));
            detach(&ionice);
            let mut renice = vec!["pkexec", "renice", "-n", "0", "-p"];
            renice.extend(pids.iter().map(String::as_str));
            detach(&renice);
            glib::timeout_add_seconds_local_once(3, reload);
        }
    }
}
