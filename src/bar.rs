//! the bar w one per monitor along the top built from modules like the old waybar

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{gdk, glib};
use gtk4_layer_shell::{Edge, Layer, LayerShell};

use crate::config;
use crate::ipc::{self, State};
use crate::status;

const HEIGHT: i32 = 30;
const MINIMAP_WIDTH: i32 = 120;
pub const PERF_STATUS: &str = "~/.config/waybar/scripts/perf-status.sh";

/// every module there is and the config picks from these
pub const MODULES: &[&str] = &[
    "desktop", "clock", "audio", "network", "perf", "power", "battery", "title", "minimap",
];

/// open one of the dropdown trays
fn tray(which: &str) {
    spawn(&format!("{} {which}", config::get().bar.tray));
}

/// a module that changes over time
enum Live {
    Clock(gtk4::Label),
    Audio(gtk4::Label),
    Network(gtk4::Label),
    Perf(gtk4::Label),
    Battery(gtk4::Label),
    Title(gtk4::Label),
    Minimap(gtk4::DrawingArea),
}

pub struct Bar {
    pub window: gtk4::ApplicationWindow,
    /// the sevenwm monitor this bar is on
    monitor: String,
    state: Rc<RefCell<State>>,
    live: RefCell<Vec<Live>>,
}

/// run a shell command so ~ works without waiting for it
fn spawn(command: &str) {
    match std::process::Command::new("sh")
        .args(["-c", command])
        .spawn()
    {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(err) => eprintln!("sevenshell: {command}: {err}"),
    }
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

impl Bar {
    pub fn new(app: &gtk4::Application, monitor: &gdk::Monitor) -> Rc<Self> {
        let window = gtk4::ApplicationWindow::new(app);
        window.init_layer_shell();
        window.set_namespace(Some("sevenshell-bar"));
        window.set_layer(Layer::Top);
        window.set_monitor(Some(monitor));
        for edge in [Edge::Top, Edge::Left, Edge::Right] {
            window.set_anchor(edge, true);
        }
        window.auto_exclusive_zone_enable();
        window.set_default_height(HEIGHT);
        window.add_css_class("sevenshell-bar");

        let bar = Rc::new(Self {
            window,
            monitor: monitor
                .connector()
                .map(|c| c.to_string())
                .unwrap_or_default(),
            state: Rc::new(RefCell::new(State::default())),
            live: RefCell::new(Vec::new()),
        });
        let config = config::get();
        let layout = gtk4::CenterBox::new();
        layout.set_start_widget(Some(&bar.section(&config.bar.left)));
        layout.set_center_widget(Some(&bar.section(&config.bar.center)));
        layout.set_end_widget(Some(&bar.section(&config.bar.right)));
        bar.window.set_child(Some(&layout));
        bar.tick_clock();
        bar
    }

    fn section(self: &Rc<Self>, modules: &[String]) -> gtk4::Box {
        let section = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
        for name in modules {
            match self.module(name) {
                Some(widget) => section.append(&widget),
                None => eprintln!("sevenshell: unknown bar module '{name}'"),
            }
        }
        section
    }

    fn module(self: &Rc<Self>, name: &str) -> Option<gtk4::Widget> {
        let mut live = self.live.borrow_mut();
        Some(match name {
            "desktop" => {
                let (button, _) = button("\u{f303}", "desktop", "Open terminal");
                button.connect_clicked(|_| spawn(&config::get().bar.terminal));
                button.upcast()
            }
            "clock" => {
                let label = gtk4::Label::new(None);
                label.add_css_class("module");
                label.add_css_class("clock");
                live.push(Live::Clock(label.clone()));
                label.upcast()
            }
            "audio" => {
                let (button, label) = button("", "audio", "");
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
                let (button, label) = button("", "network", "");
                button.connect_clicked(|_| tray("wifi"));
                live.push(Live::Network(label));
                button.upcast()
            }
            "perf" => {
                let (button, label) = button("\u{f032a}", "perf", "");
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
                let (button, _) = button("\u{23fb}", "power", "Power");
                button.connect_clicked(|_| tray("power"));
                button.upcast()
            }
            "battery" => {
                let label = gtk4::Label::new(None);
                label.add_css_class("module");
                live.push(Live::Battery(label.clone()));
                label.upcast()
            }
            "title" => {
                let (button, label) = button("", "title", "Overview");
                label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                label.set_max_width_chars(60);
                button.connect_clicked(|_| ipc::action("overview"));
                live.push(Live::Title(label));
                button.upcast()
            }
            "minimap" => {
                let area = gtk4::DrawingArea::new();
                area.set_content_width(MINIMAP_WIDTH);
                area.set_content_height(HEIGHT - 8);
                area.set_valign(gtk4::Align::Center);
                area.set_tooltip_text(Some("The canvas — click to fly there"));
                area.add_css_class("minimap");
                drop(live);
                self.setup_minimap(&area);
                self.live.borrow_mut().push(Live::Minimap(area.clone()));
                return Some(area.upcast());
            }
            _ => return None,
        })
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
                _ => {}
            }
        }
    }

    pub fn tick_clock(&self) {
        let Ok(now) = glib::DateTime::now_local() else {
            return;
        };
        let format = config::get().bar.clock_format.clone();
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
                (Live::Audio(label), status::Update::Fast(fast)) => {
                    label.set_text(&match &fast.volume {
                        Some(v) if v.muted => "\u{f075f} muted".into(),
                        Some(v) => {
                            // low medium high like waybar picks from its icon list
                            let icon = ['\u{f057f}', '\u{f0580}', '\u{f057e}']
                                [(v.percent.min(100) as usize * 3 / 101).min(2)];
                            format!("{icon} {}%", v.percent)
                        }
                        None => "\u{f075f} —".into(),
                    })
                }
                (Live::Network(label), status::Update::Fast(fast)) => {
                    let info = &fast.network;
                    label.set_text(&match &info.network {
                        status::Network::Wireless(ssid) => format!("\u{f0928} {ssid}"),
                        status::Network::Wired => "\u{f0200} wired".into(),
                        status::Network::Offline => "\u{f092d} offline".into(),
                    });
                    let tooltip = format!("{} {}", info.interface, info.address);
                    label.set_tooltip_text(Some(tooltip.trim()));
                }
                (Live::Battery(label), status::Update::Fast(fast)) => match &fast.battery {
                    Some(b) => {
                        label.set_visible(true);
                        label.set_text(&format!(
                            "bat {}%{}",
                            b.percent,
                            if b.charging { "+" } else { "" }
                        ));
                    }
                    None => label.set_visible(false),
                },
                (Live::Perf(label), status::Update::Perf(json)) => {
                    if let Some(text) = json["text"].as_str() {
                        label.set_text(text);
                    }
                    if let Some(tooltip) = json["tooltip"].as_str()
                        && let Some(button) = label.parent()
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
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.06);
            cr.rectangle(0.0, 0.0, width as f64, height as f64);
            let _ = cr.fill();
            for ws in &state.workspaces {
                let [x, y, w, h] = ws.rect;
                let (x, y, w, h) = layout.rect(x as f64, y as f64, w as f64, h as f64);
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.35);
                cr.set_line_width(1.0);
                cr.rectangle(x + 0.5, y + 0.5, w - 1.0, h - 1.0);
                let _ = cr.stroke();
            }
            for window in state.windows.iter().filter(|w| !w.collapsed) {
                let [x, y, w, h] = window.rect;
                let (x, y, w, h) = layout.rect(x as f64, y as f64, w as f64, h as f64);
                let alpha = if window.focused { 0.9 } else { 0.45 };
                cr.set_source_rgba(0.85, 0.85, 0.9, alpha);
                cr.rectangle(x, y, w.max(1.0), h.max(1.0));
                let _ = cr.fill();
            }
            if let Some(monitor) = state.monitor(&bar.monitor) {
                let (vw, vh) = (
                    monitor.size[0] as f64 / monitor.zoom,
                    monitor.size[1] as f64 / monitor.zoom,
                );
                let (x, y, w, h) = layout.rect(monitor.camera[0], monitor.camera[1], vw, vh);
                cr.set_source_rgba(0.45, 0.65, 1.0, 0.9);
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
