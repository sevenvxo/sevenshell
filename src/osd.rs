//! the on screen box that shows volume or brightness as it changes

use std::cell::Cell;
use std::process::Command;
use std::rc::Rc;
use std::time::Duration;

use gtk4::glib;
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::config;
use crate::status;

pub const CSS: &str = "
.osd { background: transparent; }
.osd .box {
    background: #000000;
    border: 1px solid #ffffff;
    border-radius: 7px;
    padding: 10px 16px;
}
.osd * { color: #ffffff; font-family: \"Symbols Nerd Font\", sans-serif; font-size: 13px; }
.osd .icon { font-size: 18px; min-width: 22px; }
.osd .value { min-width: 34px; }
.osd progressbar trough { background: #333333; border: none; border-radius: 3px; min-height: 6px; }
.osd progressbar progress { background: #ffffff; border: none; border-radius: 3px; min-height: 6px; }
.osd progressbar.muted progress { background: #777777; }
";

pub struct Osd {
    window: gtk4::ApplicationWindow,
    icon: gtk4::Label,
    bar: gtk4::ProgressBar,
    value: gtk4::Label,
    /// bumped on every show so a hide timer only acts if nothing came since
    shown: Rc<Cell<u64>>,
}

impl Osd {
    pub fn new(app: &gtk4::Application) -> Rc<Self> {
        let window = gtk4::ApplicationWindow::new(app);
        window.init_layer_shell();
        window.set_namespace(Some("osd"));
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(KeyboardMode::None);
        window.add_css_class("osd");
        // clicks go thru to whatever is under it
        window.set_can_target(false);

        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        row.add_css_class("box");
        let icon = gtk4::Label::new(None);
        icon.add_css_class("icon");
        let bar = gtk4::ProgressBar::new();
        bar.set_size_request(220, -1);
        bar.set_valign(gtk4::Align::Center);
        let value = gtk4::Label::new(None);
        value.add_css_class("value");
        value.set_xalign(1.0);
        row.append(&icon);
        row.append(&bar);
        row.append(&value);
        window.set_child(Some(&row));
        let osd = Rc::new(Self {
            window,
            icon,
            bar,
            value,
            shown: Rc::default(),
        });
        osd.apply_config();
        osd
    }

    /// put the box where the config says
    pub fn apply_config(&self) {
        let (top, bottom, left, right) = config::get().osd.position.anchors();
        for (edge, anchored, margin) in [
            (Edge::Top, top, 80),
            (Edge::Bottom, bottom, 80),
            (Edge::Left, left, 40),
            (Edge::Right, right, 40),
        ] {
            self.window.set_anchor(edge, anchored);
            self.window.set_margin(edge, if anchored { margin } else { 0 });
        }
    }

    /// run sevenshell volume or mic or brightness up down or mute
    pub fn command(&self, what: &str, how: Option<&str>) {
        let valid = matches!(
            (what, how),
            ("volume" | "brightness", Some("up" | "down") | None)
                | ("volume", Some("mute"))
                | ("mic", Some("mute") | None)
        );
        if !valid {
            eprintln!("sevenshell: usage: sevenshell volume|mic|brightness [up|down|mute]");
            return;
        }
        // wpctl and brightnessctl run on the worker so a held key or stuck pipewire cant freeze the shell
        let config = config::get();
        status::request(status::Request::Osd {
            what: what.to_string(),
            how: how.map(str::to_string),
            step: config.osd.step as i32,
            max: config.osd.max_volume,
        });
    }

    /// a reading from the worker so show it
    pub fn show_reading(&self, reading: &Reading) {
        self.show(reading.icon, reading.percent, reading.muted);
    }

    fn show(&self, icon: &str, percent: u32, muted: bool) {
        self.icon.set_text(icon);
        // volume can go past 100% and the bar js stays full
        self.bar.set_fraction((percent as f64 / 100.0).min(1.0));
        if muted {
            self.bar.add_css_class("muted");
        } else {
            self.bar.remove_css_class("muted");
        }
        self.value.set_text(&format!("{percent}%"));
        self.window.present();

        let generation = self.shown.get() + 1;
        self.shown.set(generation);
        let (window, shown) = (self.window.downgrade(), self.shown.clone());
        let duration = Duration::from_millis(config::get().osd.duration as u64);
        glib::timeout_add_local_once(duration, move || {
            if let Some(window) = window.upgrade()
                && shown.get() == generation
            {
                window.set_visible(false);
            }
        });
    }
}

fn wpctl(args: &[&str]) {
    let _ = Command::new("wpctl").args(args).status();
}

fn brightnessctl(change: &str) {
    let _ = Command::new("brightnessctl")
        .args(["--class=backlight", "set", change])
        .output();
}

/// the screen brightness in percent if it has a backlight
fn brightness() -> Option<u32> {
    let output = Command::new("brightnessctl")
        .args(["--class=backlight", "-m"])
        .output()
        .ok()?;
    // brightnessctl -m gives name class value percent and max split by commas
    let text = String::from_utf8_lossy(&output.stdout);
    let field = text.lines().next()?.split(',').nth(3)?;
    field.trim_end_matches('%').parse().ok()
}

/// what the osd shows after a uhh change
pub struct Reading {
    pub icon: &'static str,
    pub percent: u32,
    pub muted: bool,
}

/// make the change and read the new level back on the worker
pub fn apply(what: &str, how: Option<&str>, step: i32, max: u32) -> Option<Reading> {
    match (what, how) {
        ("volume", Some("up")) => status::change_volume(step, max),
        ("volume", Some("down")) => status::change_volume(-step, max),
        ("volume", Some("mute")) => wpctl(&["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]),
        ("mic", Some("mute") | None) => wpctl(&["set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"]),
        ("brightness", Some("up")) => brightnessctl(&format!("{step}%+")),
        ("brightness", Some("down")) => brightnessctl(&format!("{step}%-")),
        _ => {}
    }
    match what {
        "volume" => {
            let v = status::volume()?;
            let icon = match v.percent {
                _ if v.muted => "󰝟",
                0..=33 => "󰕿",
                34..=66 => "󰖀",
                _ => "󰕾",
            };
            Some(Reading { icon, percent: v.percent, muted: v.muted })
        }
        "mic" => {
            let v = status::mic()?;
            let icon = if v.muted { "󰍭" } else { "󰍬" };
            Some(Reading { icon, percent: v.percent, muted: v.muted })
        }
        _ => Some(Reading { icon: "󰃠", percent: brightness()?, muted: false }),
    }
}
