//! the bar w one per monitor along the top built from modules like the old waybar

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{gdk, glib};
use gtk4_layer_shell::{Edge, Layer, LayerShell};

use crate::config;
use crate::ipc::{self, State};
use crate::status;

const MINIMAP_WIDTH: i32 = 120;
pub const PERF_STATUS: &str = "~/.config/waybar/scripts/perf-status.sh";

/// every module there is and the config picks from these
pub const MODULES: &[&str] = &[
    "desktop", "clock", "audio", "network", "perf", "power", "battery", "title", "minimap", "cava",
    "media", "weather", "updates", "privacy", "caffeine", "dnd", "nightlight", "antiflash", "clipboard",
    "session", "quick",
];

/// open one of the dropdown trays right here or thru ur own command if u set one
fn tray(which: &str) {
    let command = config::get().bar.tray.clone();
    // the new trays only exist built in
    if crate::tray::is_builtin(&command) || ["quick", "media", "weather"].contains(&which) {
        crate::tray::toggle(which);
    } else {
        spawn(&format!("{command} {which}"));
    }
}

/// modules that sit together in one rounded pill like caelestias status icons
const GROUPED: &[&str] = &["audio", "network", "perf", "battery"];

/// an icon and its text that a live module updates
#[derive(Clone)]
struct Status {
    icon: gtk4::Label,
    text: gtk4::Label,
    /// a side bar only has room for icons
    compact: bool,
}

impl Status {
    fn set(&self, icon: &str, text: &str) {
        self.icon.set_text(icon);
        self.text.set_text(text);
        self.text.set_visible(!text.is_empty() && !self.compact);
    }

    /// the button holding it
    fn button(&self) -> Option<gtk4::Widget> {
        self.icon.parent().and_then(|row| row.parent())
    }
}

/// a module that changes over time
enum Live {
    Clock(gtk4::Label),
    Audio(Status),
    Network(Status),
    Perf(Status),
    Battery(Status),
    Title(gtk4::Label),
    Minimap(gtk4::DrawingArea),
    Toggle(crate::toggles::Toggle, Status),
    Privacy(Privacy),
}

/// the red dots for the mic camera and screen sharing
#[derive(Clone)]
struct Privacy {
    root: gtk4::Box,
    mic: gtk4::Label,
    camera: gtk4::Label,
    screen: gtk4::Label,
    /// the last mic and camera reading from the worker
    seen: Rc<RefCell<status::Privacy>>,
}

impl Privacy {
    fn show(&self, state: Option<&State>) {
        let seen = self.seen.borrow();
        let screen = state.is_some_and(|s| s.capturing);
        self.mic.set_visible(!seen.mic.is_empty());
        self.camera.set_visible(!seen.camera.is_empty());
        self.screen.set_visible(screen);
        let mut lines = Vec::new();
        if !seen.mic.is_empty() {
            lines.push(format!("Mic: {}", seen.mic.join(", ")));
        }
        if !seen.camera.is_empty() {
            lines.push(format!("Camera: {}", seen.camera.join(", ")));
        }
        if screen {
            lines.push("Your screen is being captured".into());
        }
        self.root.set_tooltip_text(Some(&lines.join("\n")));
        self.root.set_visible(!lines.is_empty());
    }
}

pub struct Bar {
    pub window: gtk4::ApplicationWindow,
    /// the sevenwm monitor this bar is on
    monitor: String,
    state: Rc<RefCell<State>>,
    live: RefCell<Vec<Live>>,
    /// on the left or right so modules stack up and down
    vertical: bool,
    /// autohide slide from 0 tucked away to 1 out
    shown: std::cell::Cell<f64>,
    /// where the slide is headed and its running animation
    slide: RefCell<Option<glib::SourceId>>,
    hide_timer: RefCell<Option<glib::SourceId>>,
}

/// run a shell command so ~ works without waiting for it
fn spawn(command: &str) {
    crate::run_detached(std::process::Command::new("sh").args(["-c", command]));
}

/// the edge the bar sits on
pub fn edge() -> Edge {
    match config::get().bar.position.as_str() {
        "bottom" => Edge::Bottom,
        "left" => Edge::Left,
        "right" => Edge::Right,
        _ => Edge::Top,
    }
}

/// the gap around a floating bar
pub fn gap() -> i32 {
    if config::get().bar.floating { 6 } else { 0 }
}

/// how far from the bars edge a tray sits so it clears the bar even when the bar doesnt reserve space
pub fn tray_margin() -> i32 {
    let bar = &config::get().bar;
    if bar.autohide { bar.height + gap() + 6 } else { 6 }
}

/// a round button w a material icon and some text next to it
fn status_button(icon: &str, class: &str, compact: bool) -> (gtk4::Button, Status) {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    row.set_halign(gtk4::Align::Center);
    let status = Status {
        icon: crate::style::icon(icon),
        text: gtk4::Label::new(None),
        compact,
    };
    status.text.add_css_class("text");
    status.text.set_visible(false);
    row.append(&status.icon);
    row.append(&status.text);
    let button = gtk4::Button::new();
    button.set_child(Some(&row));
    button.add_css_class("module");
    button.add_css_class(class);
    (button, status)
}

/// a flat clickable label like waybars custom modules
fn button(text: &str, class: &str, tooltip: &str) -> (gtk4::Button, gtk4::Label) {
    let label = gtk4::Label::new(Some(text));
    let button = gtk4::Button::new();
    button.set_child(Some(&label));
    button.add_css_class("module");
    button.add_css_class(class);
    if !tooltip.is_empty() {
        button.set_tooltip_text(Some(tooltip));
    }
    (button, label)
}

/// cava bars that bounce w whatever is uhh playing and on a side bar they lie down
fn cava_module(vertical: bool) -> gtk4::DrawingArea {
    let count = config::get().bar.cava_bars;
    let height = config::get().bar.height;
    let (bar_w, gap) = (height as f64 / 9.0, height as f64 / 18.0);
    let length = ((bar_w + gap) * count as f64 - gap).ceil() as i32 + 8;
    let across = height - height * 2 / 9;
    let area = gtk4::DrawingArea::new();
    if vertical {
        area.set_content_width(across);
        area.set_content_height(length);
        area.set_halign(gtk4::Align::Center);
    } else {
        area.set_content_width(length);
        area.set_content_height(across);
        area.set_valign(gtk4::Align::Center);
    }
    area.add_css_class("module");
    area.add_css_class("cava");
    let heights: Rc<RefCell<Vec<f64>>> = Rc::new(RefCell::new(vec![0.0; count]));
    {
        let heights = heights.clone();
        area.set_draw_func(move |_, cr, width, h| {
            let (r, g, b, a) = crate::theme::rgba(crate::theme::current().get("m3primary"), 1.0);
            cr.set_source_rgba(r, g, b, a);
            let heights = heights.borrow();
            let total = (bar_w + gap) * count as f64 - gap;
            let (long, short) = if vertical { (h as f64, width as f64) } else { (width as f64, h as f64) };
            let start = (long - total) / 2.0;
            for (i, v) in heights.iter().take(count).enumerate() {
                // even silence leaves a dot so u can tell its there
                let size = (v * short).max(bar_w);
                let along = start + i as f64 * (bar_w + gap);
                let (x, y, w, bh) = if vertical {
                    ((short - size) / 2.0, along, size, bar_w)
                } else {
                    (along, (short - size) / 2.0, bar_w, size)
                };
                rounded(cr, x, y, w, bh, bar_w / 2.0);
                let _ = cr.fill();
            }
        });
    }
    let Some(feed) = crate::feed::cava(count) else {
        return area;
    };
    feed.watch(&area, move |area, line| {
        *heights.borrow_mut() = crate::feed::cava_heights(line);
        area.queue_draw();
    });
    area
}

/// ur own module w what its command printed last
fn custom_module(custom: &config::Custom, compact: bool) -> gtk4::Button {
    let (button, status) = status_button(&custom.icon, "custom", compact);
    status.icon.set_visible(!custom.icon.is_empty());
    button.add_css_class(&format!("custom-{}", custom.name));
    if custom.on_click.is_empty() {
        button.set_can_target(false);
    } else {
        let click = custom.on_click.clone();
        button.connect_clicked(move |_| spawn(&click));
    }
    button.set_visible(false);
    let feed = crate::feed::get(&custom.exec, custom.interval);
    let icon = custom.icon.clone();
    feed.watch(&button, move |button, line| {
        // waybar style json lines have text and tooltip
        let json = line
            .trim_start()
            .starts_with('{')
            .then(|| serde_json::from_str::<serde_json::Value>(line).ok())
            .flatten();
        let (text, tooltip) = match &json {
            Some(j) => (
                j["text"].as_str().unwrap_or("").to_string(),
                j["tooltip"].as_str().map(String::from),
            ),
            None => (line.to_string(), None),
        };
        status.set(&icon, text.trim());
        button.set_tooltip_text(tooltip.as_deref());
        button.set_visible(!text.trim().is_empty() || !icon.is_empty());
    });
    button
}

/// whats playing w a tap to open the player and right click to pause
fn media_module(compact: bool) -> gtk4::Button {
    let (button, status) = status_button("music_note", "media", compact);
    status.text.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    status.text.set_max_width_chars(28);
    button.set_visible(false);
    button.connect_clicked(|_| tray("media"));
    let right = gtk4::GestureClick::builder().button(3).build();
    right.connect_pressed(|_, _, _, _| spawn("playerctl play-pause"));
    button.add_controller(right);
    let middle = gtk4::GestureClick::builder().button(2).build();
    middle.connect_pressed(|_, _, _, _| spawn("playerctl next"));
    button.add_controller(middle);
    crate::media::feed().watch(&button, move |button, line| {
        let Some(track) = crate::media::Track::parse(line) else {
            button.set_visible(false);
            return;
        };
        let icon = if track.playing { "pause_circle" } else { "play_circle" };
        status.set(icon, &track.short());
        button.set_tooltip_text(Some(&track.long()));
        button.set_visible(true);
    });
    button
}

/// the weather now from wttr.in and a tap shows the next days
fn weather_module(compact: bool) -> gtk4::Button {
    let (button, status) = status_button("cloud", "weather", compact);
    button.set_visible(false);
    button.connect_clicked(|_| tray("weather"));
    crate::weather::feed().watch(&button, move |button, line| {
        let Some(report) = crate::weather::Report::parse(line) else {
            return;
        };
        status.set(report.now.icon, &report.now.temp);
        button.set_tooltip_text(Some(&format!("{} in {}", report.now.description, report.place)));
        button.set_visible(true);
    });
    button
}

/// how many package updates are waiting and a tap runs the update
fn updates_module(compact: bool) -> gtk4::Button {
    let (button, status) = status_button("system_update_alt", "updates", compact);
    button.set_visible(false);
    button.connect_clicked(|_| spawn(&config::get().updates.command));
    let every = config::get().updates.interval_minutes * 60;
    let check = "{ checkupdates 2>/dev/null; if command -v yay >/dev/null; then yay -Qua 2>/dev/null; elif command -v paru >/dev/null; then paru -Qua 2>/dev/null; fi; } | grep -c .";
    crate::feed::get(check, every).watch(&button, move |button, line| {
        let count: u32 = line.trim().parse().unwrap_or(0);
        status.set("system_update_alt", &count.to_string());
        button.set_tooltip_text(Some(&format!("{count} updates waiting — click to update")));
        button.set_visible(count > 0);
    });
    button
}

impl Bar {
    pub fn new(app: &gtk4::Application, monitor: &gdk::Monitor) -> Rc<Self> {
        let config = config::get();
        let window = gtk4::ApplicationWindow::new(app);
        window.init_layer_shell();
        window.set_namespace(Some("sevenshell-bar"));
        window.set_layer(Layer::Top);
        window.set_monitor(Some(monitor));
        let edge = edge();
        let vertical = matches!(edge, Edge::Left | Edge::Right);
        let across: [Edge; 2] = if vertical { [Edge::Top, Edge::Bottom] } else { [Edge::Left, Edge::Right] };
        window.set_anchor(edge, true);
        for e in across {
            window.set_anchor(e, true);
        }
        let gap = gap();
        window.set_margin(edge, gap);
        for e in across {
            window.set_margin(e, gap * 2);
        }
        let thickness = config.bar.height;
        if vertical {
            window.set_default_width(thickness);
        } else {
            window.set_default_height(thickness);
        }
        if config.bar.autohide {
            // it lies over windows and only a thin strip peeks out
            window.set_exclusive_zone(0);
        } else {
            window.auto_exclusive_zone_enable();
        }
        window.add_css_class("sevenshell-bar");
        if vertical {
            window.add_css_class("vertical");
        }
        if config.bar.floating {
            window.add_css_class("floating");
        }
        crate::style::adopt(&window);

        let bar = Rc::new(Self {
            window,
            monitor: monitor
                .connector()
                .map(|c| c.to_string())
                .unwrap_or_default(),
            state: Rc::new(RefCell::new(State::default())),
            live: RefCell::new(Vec::new()),
            vertical,
            shown: std::cell::Cell::new(1.0),
            slide: RefCell::new(None),
            hide_timer: RefCell::new(None),
        });
        let layout = gtk4::CenterBox::new();
        layout.add_css_class("bar-body");
        if vertical {
            layout.set_orientation(gtk4::Orientation::Vertical);
        }
        layout.set_start_widget(Some(&bar.section(&config.bar.left)));
        layout.set_center_widget(Some(&bar.section(&config.bar.center)));
        layout.set_end_widget(Some(&bar.section(&config.bar.right)));
        bar.window.set_child(Some(&layout));
        bar.tick_clock();
        if config.bar.autohide {
            bar.setup_autohide();
        }
        bar
    }

    /// peek out when the mouse touches the strip and tuck away a bit after it leaves unless a tray is open
    fn setup_autohide(self: &Rc<Self>) {
        self.shown.set(0.0);
        self.place(0.0);
        let motion = gtk4::EventControllerMotion::new();
        let this = Rc::downgrade(self);
        motion.connect_enter(move |_, _, _| {
            if let Some(this) = this.upgrade() {
                if let Some(timer) = this.hide_timer.borrow_mut().take() {
                    timer.remove();
                }
                this.slide_to(1.0);
            }
        });
        let this = Rc::downgrade(self);
        motion.connect_leave(move |_| {
            if let Some(this) = this.upgrade() {
                this.hide_later();
            }
        });
        self.window.add_controller(motion);
    }

    fn hide_later(self: &Rc<Self>) {
        let this = Rc::downgrade(self);
        let timer = glib::timeout_add_local(std::time::Duration::from_millis(600), move || {
            let Some(this) = this.upgrade() else {
                return glib::ControlFlow::Break;
            };
            // a tray from the bar is open so stay till its gone
            if crate::tray::is_open() {
                return glib::ControlFlow::Continue;
            }
            this.hide_timer.borrow_mut().take();
            this.slide_to(0.0);
            glib::ControlFlow::Break
        });
        if let Some(old) = self.hide_timer.borrow_mut().replace(timer) {
            old.remove();
        }
    }

    /// move the bar so shown of it is out where 0 leaves a 2px strip
    fn place(&self, shown: f64) {
        let thickness = config::get().bar.height;
        let hidden = 2 - thickness;
        let margin = hidden + ((gap() - hidden) as f64 * shown).round() as i32;
        self.window.set_margin(edge(), margin);
    }

    fn slide_to(self: &Rc<Self>, target: f64) {
        if let Some(old) = self.slide.borrow_mut().take() {
            old.remove();
        }
        let this = Rc::downgrade(self);
        let tick = glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
            let Some(this) = this.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let now = this.shown.get();
            // about 150ms end to end
            let step = 0.12;
            let next = if target > now { (now + step).min(target) } else { (now - step).max(target) };
            this.shown.set(next);
            this.place(next);
            if next == target {
                this.slide.borrow_mut().take();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
        *self.slide.borrow_mut() = Some(tick);
    }

    fn section(self: &Rc<Self>, modules: &[String]) -> gtk4::Box {
        let orientation = if self.vertical { gtk4::Orientation::Vertical } else { gtk4::Orientation::Horizontal };
        let section = gtk4::Box::new(orientation, 6);
        let mut group: Option<gtk4::Box> = None;
        for name in modules {
            let Some(widget) = self.module(name) else {
                eprintln!("sevenshell: unknown bar module '{name}'");
                continue;
            };
            if GROUPED.contains(&name.as_str()) {
                // neighbors like audio and network share one pill
                let pill = group.get_or_insert_with(|| {
                    let pill = gtk4::Box::new(orientation, 0);
                    pill.add_css_class("group");
                    section.append(&pill);
                    pill
                });
                pill.append(&widget);
            } else {
                group = None;
                section.append(&widget);
            }
        }
        section
    }

    /// a module that flips a toggle and lights up while its on
    fn toggle_module(&self, toggle: crate::toggles::Toggle, class: &str) -> gtk4::Button {
        let (button, status) = status_button(toggle.icon(), class, self.vertical);
        button.add_css_class("toggle");
        button.set_tooltip_text(Some(toggle.hint()));
        let s = status.clone();
        button.connect_clicked(move |button| {
            crate::toggles::flip(toggle);
            // show it right away instead of waiting for the config to come back
            if button.has_css_class("on") {
                button.remove_css_class("on");
                s.icon.remove_css_class("filled");
            } else {
                button.add_css_class("on");
                s.icon.add_css_class("filled");
            }
        });
        self.live.borrow_mut().push(Live::Toggle(toggle, status));
        button
    }

    fn module(self: &Rc<Self>, name: &str) -> Option<gtk4::Widget> {
        use crate::toggles::Toggle;
        let compact = self.vertical;
        match name {
            "caffeine" => return Some(self.toggle_module(Toggle::Caffeine, "caffeine").upcast()),
            "dnd" => return Some(self.toggle_module(Toggle::DoNotDisturb, "dnd").upcast()),
            "nightlight" => return Some(self.toggle_module(Toggle::NightLight, "nightlight").upcast()),
            "antiflash" => return Some(self.toggle_module(Toggle::AntiFlashbang, "antiflash").upcast()),
            _ => {}
        }
        let mut live = self.live.borrow_mut();
        Some(match name {
            "desktop" => {
                let (button, _) = button("\u{f303}", "desktop", "Open terminal");
                button.add_css_class("os");
                button.connect_clicked(|_| spawn(&config::get().bar.terminal));
                button.upcast()
            }
            "clock" => {
                let label = gtk4::Label::new(None);
                label.set_justify(gtk4::Justification::Center);
                let button = gtk4::Button::new();
                button.set_child(Some(&label));
                button.add_css_class("module");
                button.add_css_class("clock");
                button.connect_clicked(|_| tray("quick"));
                live.push(Live::Clock(label));
                button.upcast()
            }
            "audio" => {
                let (button, label) = status_button("volume_up", "audio", compact);
                button.connect_clicked(|_| tray("audio"));
                let right = gtk4::GestureClick::builder().button(3).build();
                right.connect_pressed(|_, _, _, _| spawn("pavucontrol"));
                button.add_controller(right);
                let scroll =
                    gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
                scroll.connect_scroll(move |_, _, dy| {
                    let osd = &config::get().osd;
                    let step = osd.step as i32;
                    let step = if dy < 0.0 { step } else { -step };
                    status::request(status::Request::ChangeVolume(step, osd.max_volume));
                    glib::Propagation::Stop
                });
                button.add_controller(scroll);
                live.push(Live::Audio(label));
                button.upcast()
            }
            "network" => {
                let (button, label) = status_button("wifi", "network", compact);
                button.connect_clicked(|_| tray("wifi"));
                live.push(Live::Network(label));
                button.upcast()
            }
            "perf" => {
                let (button, label) = status_button("balance", "perf", compact);
                button.connect_clicked(move |_| {
                    tray("perf");
                    // the tray changes the power mode so show it once it has
                    glib::timeout_add_seconds_local_once(1, || {
                        status::request(status::Request::Perf(PERF_STATUS.into()));
                    });
                });
                live.push(Live::Perf(label));
                button.upcast()
            }
            "power" => {
                let (button, _) = status_button("power_settings_new", "power", compact);
                button.set_tooltip_text(Some("Power"));
                button.connect_clicked(|_| tray("power"));
                button.upcast()
            }
            "session" => {
                let (button, _) = status_button("power_settings_new", "power", compact);
                button.set_tooltip_text(Some("Lock, log out, suspend, restart or shut down"));
                button.connect_clicked(|_| spawn("sevenshell session"));
                button.upcast()
            }
            "quick" => {
                let (button, _) = status_button("tune", "quick", compact);
                button.set_tooltip_text(Some("Quick settings"));
                button.connect_clicked(|_| tray("quick"));
                button.upcast()
            }
            "clipboard" => {
                let (button, _) = status_button("content_paste", "clipboard", compact);
                button.set_tooltip_text(Some("Clipboard history"));
                button.connect_clicked(|_| spawn("sevenshell clipboard"));
                button.upcast()
            }
            "battery" => {
                let (button, status) = status_button("battery_full", "battery", compact);
                button.set_can_target(false);
                live.push(Live::Battery(status));
                button.upcast()
            }
            "title" => {
                let (button, label) = button("", "title", "Overview");
                label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                label.set_max_width_chars(60);
                button.connect_clicked(|_| ipc::action("overview"));
                // theres no room for a title on a side bar
                button.set_visible(!self.vertical);
                live.push(Live::Title(label));
                button.upcast()
            }
            "privacy" => {
                let root = gtk4::Box::new(if self.vertical { gtk4::Orientation::Vertical } else { gtk4::Orientation::Horizontal }, 4);
                root.add_css_class("module");
                root.add_css_class("privacy");
                let dot = |name: &str| {
                    let icon = crate::style::icon(name);
                    icon.add_css_class("filled");
                    root.append(&icon);
                    icon
                };
                let privacy = Privacy {
                    mic: dot("mic"),
                    camera: dot("videocam"),
                    screen: dot("screen_share"),
                    root: root.clone(),
                    seen: Rc::default(),
                };
                privacy.show(None);
                live.push(Live::Privacy(privacy));
                root.upcast()
            }
            "minimap" => {
                let area = gtk4::DrawingArea::new();
                let height = config::get().bar.height;
                let across = height - height * 2 / 9;
                if self.vertical {
                    area.set_content_width(across);
                    area.set_content_height(across * 3 / 2);
                    area.set_halign(gtk4::Align::Center);
                } else {
                    area.set_content_width(MINIMAP_WIDTH * height / 36);
                    area.set_content_height(across);
                    area.set_valign(gtk4::Align::Center);
                }
                area.set_tooltip_text(Some("The canvas — click to fly there"));
                area.add_css_class("minimap");
                drop(live);
                self.setup_minimap(&area);
                self.live.borrow_mut().push(Live::Minimap(area.clone()));
                return Some(area.upcast());
            }
            "cava" => {
                drop(live);
                return Some(cava_module(self.vertical).upcast());
            }
            "media" => {
                drop(live);
                return Some(media_module(compact).upcast());
            }
            "weather" => {
                drop(live);
                return Some(weather_module(compact).upcast());
            }
            "updates" => {
                drop(live);
                return Some(updates_module(compact).upcast());
            }
            other => {
                let name = other.strip_prefix("custom/")?;
                let custom = config::get().bar.custom.iter().find(|c| c.name == name)?.clone();
                drop(live);
                return Some(custom_module(&custom, compact).upcast());
            }
        })
    }

    /// light up the toggle modules that are on
    pub fn show_toggles(&self) {
        for live in self.live.borrow().iter() {
            if let Live::Toggle(toggle, status) = live {
                let on = toggle.is_on();
                if let Some(button) = status.button() {
                    if on {
                        button.add_css_class("on");
                        status.icon.add_css_class("filled");
                    } else {
                        button.remove_css_class("on");
                        status.icon.remove_css_class("filled");
                    }
                }
            }
        }
    }

    /// whether this bar shows who uses the mic or camera
    pub fn wants_privacy(&self) -> bool {
        self.live.borrow().iter().any(|l| matches!(l, Live::Privacy(_)))
    }

    /// new state from sevenwm
    pub fn update(&self, state: &State) {
        *self.state.borrow_mut() = state.clone();
        for live in self.live.borrow().iter() {
            match live {
                Live::Title(label) => {
                    let title = state.focused().map_or(String::new(), |w| {
                        if w.title.is_empty() {
                            w.app_id.clone()
                        } else {
                            w.title.clone()
                        }
                    });
                    label.set_text(&title);
                }
                Live::Minimap(area) => area.queue_draw(),
                Live::Privacy(privacy) => privacy.show(Some(state)),
                _ => {}
            }
        }
        self.show_toggles();
    }

    pub fn tick_clock(&self) {
        let Ok(now) = glib::DateTime::now_local() else {
            return;
        };
        // a side bar stacks the hours over the minutes
        let format = if self.vertical { "%H\n%M".to_string() } else { config::get().bar.clock_format.clone() };
        for live in self.live.borrow().iter() {
            if let Live::Clock(label) = live {
                if let Ok(text) = now.format(&format) {
                    label.set_text(&text);
                }
                if let Ok(text) = now.format("%A, %B %d %Y") {
                    label.set_tooltip_text(Some(&text));
                }
            }
        }
    }

    /// whether this bar shows the perf script output
    pub fn wants_perf(&self) -> bool {
        self.live.borrow().iter().any(|l| matches!(l, Live::Perf(_)))
    }

    /// show a reading from the status worker
    pub fn show_status(&self, update: &status::Update) {
        for live in self.live.borrow().iter() {
            match (live, update) {
                (Live::Audio(status), status::Update::Fast(fast)) => match &fast.volume {
                    Some(v) if v.muted => status.set("volume_off", ""),
                    Some(v) => {
                        // low medium high like waybar picks from its icon list
                        let icon = ["volume_mute", "volume_down", "volume_up"]
                            [(v.percent.min(100) as usize * 3 / 101).min(2)];
                        status.set(icon, &format!("{}%", v.percent));
                    }
                    None => status.set("volume_off", ""),
                },
                (Live::Network(status), status::Update::Fast(fast)) => {
                    let info = &fast.network;
                    match &info.network {
                        status::Network::Wireless(ssid) => status.set("wifi", ssid),
                        status::Network::Wired => status.set("lan", ""),
                        status::Network::Offline => status.set("wifi_off", ""),
                    }
                    let tooltip = format!("{} {}", info.interface, info.address);
                    if let Some(button) = status.button() {
                        button.set_tooltip_text(Some(tooltip.trim()));
                    }
                }
                (Live::Battery(status), status::Update::Fast(fast)) => {
                    let button = status.icon.parent().and_then(|row| row.parent());
                    match &fast.battery {
                        Some(b) => {
                            let icon = if b.charging {
                                "battery_charging_full"
                            } else {
                                ["battery_alert", "battery_2_bar", "battery_4_bar", "battery_6_bar", "battery_full"]
                                    [(b.percent.min(100) as usize * 5 / 101).min(4)]
                            };
                            status.set(icon, &format!("{}%", b.percent));
                            if let Some(button) = button {
                                button.set_visible(true);
                            }
                        }
                        None => {
                            if let Some(button) = button {
                                button.set_visible(false);
                            }
                        }
                    }
                }
                (Live::Privacy(privacy), status::Update::Privacy(reading)) => {
                    *privacy.seen.borrow_mut() = reading.clone();
                    privacy.show(Some(&self.state.borrow()));
                }
                (Live::Perf(status), status::Update::Perf(json)) => {
                    // the perf script says the power mode in its class
                    let icon = match json["class"].as_str() {
                        Some("power-saver") => "eco",
                        Some("performance") => "bolt",
                        _ => "balance",
                    };
                    status.set(icon, "");
                    if let Some(tooltip) = json["tooltip"].as_str()
                        && let Some(button) = status.icon.parent().and_then(|row| row.parent())
                    {
                        button.set_tooltip_text(Some(tooltip));
                    }
                }
                _ => {}
            }
        }
    }

    /// the minimap w every window as a block and workspaces outlined and clicking flies there
    fn setup_minimap(self: &Rc<Self>, area: &gtk4::DrawingArea) {
        let bar = Rc::downgrade(self);
        area.set_draw_func(move |_, cr, width, height| {
            let Some(bar) = bar.upgrade() else {
                return;
            };
            let state = bar.state.borrow();
            let Some(layout) = MapLayout::new(&state, &bar.monitor, width, height) else {
                return;
            };
            let palette = crate::theme::current();
            let paint = |cr: &gtk4::cairo::Context, name: &str, alpha: f64| {
                let (r, g, b, a) = crate::theme::rgba(palette.get(name), alpha);
                cr.set_source_rgba(r, g, b, a);
            };
            paint(cr, "m3surfaceContainer", 1.0);
            rounded(cr, 0.0, 0.0, width as f64, height as f64, 8.0);
            let _ = cr.fill();
            for ws in &state.workspaces {
                let [x, y, w, h] = ws.rect;
                let (x, y, w, h) = layout.rect(x as f64, y as f64, w as f64, h as f64);
                paint(cr, "m3outline", 0.8);
                cr.set_line_width(1.0);
                cr.rectangle(x + 0.5, y + 0.5, w - 1.0, h - 1.0);
                let _ = cr.stroke();
            }
            for window in state.windows.iter().filter(|w| !w.collapsed) {
                let [x, y, w, h] = window.rect;
                let (x, y, w, h) = layout.rect(x as f64, y as f64, w as f64, h as f64);
                if window.focused {
                    paint(cr, "m3primary", 0.9);
                } else {
                    paint(cr, "m3onSurfaceVariant", 0.45);
                }
                cr.rectangle(x, y, w.max(1.0), h.max(1.0));
                let _ = cr.fill();
            }
            if let Some(monitor) = state.monitor(&bar.monitor) {
                let (vw, vh) = (
                    monitor.size[0] as f64 / monitor.zoom,
                    monitor.size[1] as f64 / monitor.zoom,
                );
                let (x, y, w, h) = layout.rect(monitor.camera[0], monitor.camera[1], vw, vh);
                paint(cr, "m3tertiary", 0.95);
                cr.set_line_width(1.5);
                cr.rectangle(x, y, w, h);
                let _ = cr.stroke();
            }
        });

        let click = gtk4::GestureClick::new();
        let bar = Rc::downgrade(self);
        let map = area.downgrade();
        click.connect_pressed(move |_, _, px, py| {
            let (Some(bar), Some(map)) = (bar.upgrade(), map.upgrade()) else {
                return;
            };
            let state = bar.state.borrow();
            let Some(layout) = MapLayout::new(&state, &bar.monitor, map.width(), map.height())
            else {
                return;
            };
            let Some(monitor) = state.monitor(&bar.monitor) else {
                return;
            };
            // center the view on the clicked canvas point
            let (cx, cy) = layout.to_canvas(px, py);
            let (vw, vh) = (
                monitor.size[0] as f64 / monitor.zoom,
                monitor.size[1] as f64 / monitor.zoom,
            );
            ipc::fly_to(cx - vw / 2.0, cy - vh / 2.0);
        });
        area.add_controller(click);
    }
}

/// a rounded rect path for cairo
pub fn rounded(cr: &gtk4::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    let pi = std::f64::consts::PI;
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -pi / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, pi / 2.0);
    cr.arc(x + r, y + h - r, r, pi / 2.0, pi);
    cr.arc(x + r, y + r, r, pi, 3.0 * pi / 2.0);
    cr.close_path();
}

/// how canvas coords map into the minimap w everything fitted and a margin
struct MapLayout {
    scale: f64,
    origin: (f64, f64),
    offset: (f64, f64),
}

impl MapLayout {
    fn new(state: &State, monitor: &str, width: i32, height: i32) -> Option<Self> {
        let mut rects: Vec<[f64; 4]> = state
            .monitors
            .iter()
            .map(|m| m.region.map(f64::from))
            .chain(state.workspaces.iter().map(|w| w.rect.map(f64::from)))
            .chain(
                state
                    .windows
                    .iter()
                    .filter(|w| !w.collapsed)
                    .map(|w| w.rect.map(f64::from)),
            )
            .collect();
        if let Some(m) = state.monitor(monitor) {
            rects.push([
                m.camera[0],
                m.camera[1],
                m.size[0] as f64 / m.zoom,
                m.size[1] as f64 / m.zoom,
            ]);
        }
        let min_x = rects.iter().map(|r| r[0]).fold(f64::INFINITY, f64::min);
        let min_y = rects.iter().map(|r| r[1]).fold(f64::INFINITY, f64::min);
        let max_x = rects
            .iter()
            .map(|r| r[0] + r[2])
            .fold(f64::NEG_INFINITY, f64::max);
        let max_y = rects
            .iter()
            .map(|r| r[1] + r[3])
            .fold(f64::NEG_INFINITY, f64::max);
        if !min_x.is_finite() || max_x <= min_x || max_y <= min_y {
            return None;
        }
        let pad = 3.0;
        let scale = ((width as f64 - 2.0 * pad) / (max_x - min_x))
            .min((height as f64 - 2.0 * pad) / (max_y - min_y));
        let offset = (
            (width as f64 - (max_x - min_x) * scale) / 2.0,
            (height as f64 - (max_y - min_y) * scale) / 2.0,
        );
        Some(Self {
            scale,
            origin: (min_x, min_y),
            offset,
        })
    }

    fn rect(&self, x: f64, y: f64, w: f64, h: f64) -> (f64, f64, f64, f64) {
        (
            (x - self.origin.0) * self.scale + self.offset.0,
            (y - self.origin.1) * self.scale + self.offset.1,
            w * self.scale,
            h * self.scale,
        )
    }

    fn to_canvas(&self, px: f64, py: f64) -> (f64, f64) {
        (
            (px - self.offset.0) / self.scale + self.origin.0,
            (py - self.offset.1) / self.scale + self.origin.1,
        )
    }
}
