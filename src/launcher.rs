//! the app launcher on mod+d where u type to search and enter launches and most used apps come first

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{gdk, gio, glib};
use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};

const VISIBLE_ROWS: usize = 8;

pub const CSS: &str = "
.launcher { background: transparent; }
.launcher * {
    font-family: \"Adwaita Sans\", \"Symbols Nerd Font\", sans-serif;
    font-size: 14px;
    color: #ffffff;
}
.launcher .panel {
    background: #000000;
    border: 1px solid #ffffff;
    border-radius: 7px;
    padding: 14px;
}
.launcher .search-icon { font-family: \"Symbols Nerd Font\"; font-size: 15px; margin: 0 8px 0 4px; }
.launcher entry {
    background: #000000; border: none; box-shadow: none; outline: none;
    padding: 4px 0; caret-color: #ffffff; min-height: 0;
}
.launcher entry selection { background: #ffffff; color: #000000; }
.launcher .divider { background: #ffffff; min-height: 1px; margin: 10px 0 8px 0; }
.launcher .row {
    border: 1px solid #000000; border-radius: 4px;
    padding: 6px 10px;
}
.launcher .row:hover { border: 1px dashed #ffffff; }
.launcher .row.selected { border: 1px solid #ffffff; }
.launcher .empty { padding: 6px 10px; }
.launcher .count { font-size: 12px; }
";

struct App {
    info: gio::AppInfo,
    id: String,
    name: String,
    /// generic name keywords and executable lowercased and searched too
    extra: String,
}

struct Row {
    frame: gtk4::Box,
    icon: gtk4::Image,
    name: gtk4::Label,
}

pub struct Launcher {
    pub window: gtk4::ApplicationWindow,
    entry: gtk4::Entry,
    count: gtk4::Label,
    empty: gtk4::Label,
    rows: Vec<Row>,
    apps: Vec<App>,
    history: RefCell<HashMap<String, u64>>,
    results: RefCell<Vec<usize>>,
    selected: RefCell<usize>,
    offset: RefCell<usize>,
}

fn history_path() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_default()
        .join("applauncher.json")
}

fn load_history() -> HashMap<String, u64> {
    std::fs::read(history_path())
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

fn save_history(history: &HashMap<String, u64>) {
    let path = history_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string(history) {
        let _ = std::fs::write(path, text);
    }
}

fn is_subsequence(query: &str, text: &str) -> bool {
    let mut chars = text.chars();
    query.chars().all(|q| chars.any(|c| c == q))
}

/// higher is a better match and none is no match
fn score(query: &str, app: &App) -> Option<u64> {
    if app.name.starts_with(query) {
        Some(100)
    } else if app.name.split_whitespace().any(|w| w.starts_with(query)) {
        Some(80)
    } else if app.name.contains(query) {
        Some(60)
    } else if app.extra.contains(query) {
        Some(40)
    } else if is_subsequence(query, &app.name) {
        Some(20)
    } else {
        None
    }
}

fn installed_apps() -> Vec<App> {
    gio::AppInfo::all()
        .into_iter()
        .filter(|info| info.should_show())
        .map(|info| {
            let desktop = info.downcast_ref::<gio_unix::DesktopAppInfo>();
            let generic = desktop
                .and_then(|d| d.generic_name())
                .map(|s| s.to_string());
            let keywords = desktop.map(|d| {
                d.keywords()
                    .iter()
                    .map(|k| k.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            });
            let exe = info
                .executable()
                .file_name()
                .map(|f| f.to_string_lossy().to_string());
            let extra = [generic, keywords, exe]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            App {
                id: info.id().map(|s| s.to_string()).unwrap_or_default(),
                name: info.display_name().to_lowercase(),
                extra,
                info,
            }
        })
        .collect()
}

impl Launcher {
    pub fn new(app: &gtk4::Application) -> Rc<Self> {
        let window = gtk4::ApplicationWindow::new(app);
        window.init_layer_shell();
        window.set_namespace(Some("launcher"));
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        window.add_css_class("launcher");

        let panel = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        panel.add_css_class("panel");
        panel.set_size_request(460, -1);

        let search = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        let icon = gtk4::Label::new(Some("\u{f0349}"));
        icon.add_css_class("search-icon");
        let entry = gtk4::Entry::new();
        entry.set_has_frame(false);
        entry.set_hexpand(true);
        let count = gtk4::Label::new(None);
        count.add_css_class("count");
        search.append(&icon);
        search.append(&entry);
        search.append(&count);
        panel.append(&search);

        let divider = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        divider.add_css_class("divider");
        panel.append(&divider);

        let mut rows = Vec::new();
        for _ in 0..VISIBLE_ROWS {
            let frame = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
            frame.add_css_class("row");
            frame.set_margin_top(1);
            frame.set_margin_bottom(1);
            let icon = gtk4::Image::new();
            icon.set_pixel_size(24);
            let name = gtk4::Label::new(None);
            name.set_xalign(0.0);
            name.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            name.set_hexpand(true);
            frame.append(&icon);
            frame.append(&name);
            panel.append(&frame);
            rows.push(Row { frame, icon, name });
        }
        let empty = gtk4::Label::new(Some("No matches"));
        empty.set_xalign(0.0);
        empty.add_css_class("empty");
        empty.set_visible(false);
        panel.append(&empty);
        window.set_child(Some(&panel));

        let launcher = Rc::new(Self {
            window,
            entry,
            count,
            empty,
            rows,
            apps: installed_apps(),
            history: RefCell::new(load_history()),
            results: RefCell::new(Vec::new()),
            selected: RefCell::new(0),
            offset: RefCell::new(0),
        });
        launcher.connect();
        launcher.search();
        launcher
    }

    fn connect(self: &Rc<Self>) {
        let this = Rc::downgrade(self);
        self.entry.connect_changed(move |_| {
            if let Some(this) = this.upgrade() {
                this.search();
            }
        });
        let this = Rc::downgrade(self);
        self.entry.connect_activate(move |_| {
            if let Some(this) = this.upgrade() {
                this.launch_selected();
            }
        });

        let keys = gtk4::EventControllerKey::new();
        keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let this = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(this) = this.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
            match key {
                gdk::Key::Escape => this.window.close(),
                gdk::Key::Down | gdk::Key::Tab => this.step(1),
                gdk::Key::j if ctrl => this.step(1),
                gdk::Key::Up | gdk::Key::ISO_Left_Tab => this.step(-1),
                gdk::Key::k if ctrl => this.step(-1),
                gdk::Key::Page_Down => this.step(VISIBLE_ROWS as i64),
                gdk::Key::Page_Up => this.step(-(VISIBLE_ROWS as i64)),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        self.window.add_controller(keys);

        for (i, row) in self.rows.iter().enumerate() {
            let click = gtk4::GestureClick::new();
            let this = Rc::downgrade(self);
            click.connect_released(move |_, _, _, _| {
                if let Some(this) = this.upgrade() {
                    *this.selected.borrow_mut() = *this.offset.borrow() + i;
                    this.launch_selected();
                }
            });
            row.frame.add_controller(click);
            let scroll =
                gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
            let this = Rc::downgrade(self);
            scroll.connect_scroll(move |_, _, dy| {
                if let Some(this) = this.upgrade()
                    && dy != 0.0
                {
                    this.step(if dy > 0.0 { 1 } else { -1 });
                }
                glib::Propagation::Stop
            });
            row.frame.add_controller(scroll);
        }
    }

    fn search(&self) {
        let query = self.entry.text().trim().to_lowercase();
        let history = self.history.borrow();
        let uses = |app: &App| history.get(&app.id).copied().unwrap_or(0);
        let mut ranked: Vec<(u64, usize)> = if query.is_empty() {
            (0..self.apps.len())
                .map(|i| (uses(&self.apps[i]), i))
                .collect()
        } else {
            (0..self.apps.len())
                .filter_map(|i| {
                    let app = &self.apps[i];
                    score(&query, app).map(|s| (s + uses(app).min(20), i))
                })
                .collect()
        };
        ranked.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| self.apps[a.1].name.cmp(&self.apps[b.1].name))
        });
        *self.results.borrow_mut() = ranked.into_iter().map(|(_, i)| i).collect();
        *self.selected.borrow_mut() = 0;
        *self.offset.borrow_mut() = 0;
        drop(history);
        self.render();
    }

    fn render(&self) {
        let results = self.results.borrow();
        let (selected, offset) = (*self.selected.borrow(), *self.offset.borrow());
        for (i, row) in self.rows.iter().enumerate() {
            match results.get(offset + i) {
                Some(&index) => {
                    let info = &self.apps[index].info;
                    row.name.set_text(&info.display_name());
                    match info.icon() {
                        Some(icon) => row.icon.set_from_gicon(&icon),
                        None => row.icon.set_icon_name(Some("application-x-executable")),
                    }
                    if offset + i == selected {
                        row.frame.add_css_class("selected");
                    } else {
                        row.frame.remove_css_class("selected");
                    }
                    row.frame.set_visible(true);
                }
                None => row.frame.set_visible(false),
            }
        }
        self.empty.set_visible(results.is_empty());
        self.count
            .set_text(&format!("{}/{}", results.len(), self.apps.len()));
    }

    fn step(&self, delta: i64) {
        let len = self.results.borrow().len();
        if len == 0 {
            return;
        }
        let selected = (*self.selected.borrow() as i64 + delta).clamp(0, len as i64 - 1) as usize;
        let mut offset = *self.offset.borrow();
        if selected < offset {
            offset = selected;
        } else if selected >= offset + VISIBLE_ROWS {
            offset = selected + 1 - VISIBLE_ROWS;
        }
        *self.selected.borrow_mut() = selected;
        *self.offset.borrow_mut() = offset;
        self.render();
    }

    fn launch_selected(&self) {
        let Some(&index) = self.results.borrow().get(*self.selected.borrow()) else {
            return;
        };
        let app = &self.apps[index];
        {
            let mut history = self.history.borrow_mut();
            *history.entry(app.id.clone()).or_insert(0) += 1;
            save_history(&history);
        }
        let context = gdk::Display::default().map(|d| d.app_launch_context());
        if let Err(err) = app.info.launch(&[], context.as_ref()) {
            eprintln!("sevenshell: launching {}: {err}", app.id);
        }
        self.window.close();
    }

    pub fn show(&self) {
        self.window.present();
        self.entry.grab_focus();
    }
}
