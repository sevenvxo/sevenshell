//! the window search on mod+g where u type part of a name and enter focuses it

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{gdk, gio, glib};
use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};

use crate::ipc::{self, WindowInfo};

const VISIBLE_ROWS: usize = 8;

pub const CSS: &str = "
.launcher .detail { font-size: 12px; color: #9a9a9a; }
";

struct Row {
    frame: gtk4::Box,
    icon: gtk4::Image,
    name: gtk4::Label,
    detail: gtk4::Label,
}

pub struct WindowSearch {
    pub window: gtk4::ApplicationWindow,
    entry: gtk4::Entry,
    count: gtk4::Label,
    empty: gtk4::Label,
    rows: Vec<Row>,
    windows: Vec<WindowInfo>,
    results: RefCell<Vec<usize>>,
    selected: RefCell<usize>,
    offset: RefCell<usize>,
}

fn is_subsequence(query: &str, text: &str) -> bool {
    let mut chars = text.chars();
    query.chars().all(|q| chars.any(|c| c == q))
}

/// higher is a better match and none is no match
fn score(query: &str, window: &WindowInfo) -> Option<u64> {
    let title = window.title.to_lowercase();
    let app = window.app_id.to_lowercase();
    if title.starts_with(query) || app.starts_with(query) {
        Some(100)
    } else if title.split_whitespace().any(|w| w.starts_with(query)) {
        Some(80)
    } else if title.contains(query) || app.contains(query) {
        Some(60)
    } else if is_subsequence(query, &title) {
        Some(20)
    } else {
        None
    }
}

/// the app icon from its .desktop file if theres one
fn icon_for(app_id: &str) -> Option<gio::Icon> {
    let desktop = gio_unix::DesktopAppInfo::new(&format!("{app_id}.desktop")).or_else(|| {
        gio_unix::DesktopAppInfo::new(&format!("{}.desktop", app_id.to_lowercase()))
    })?;
    desktop.icon()
}

/// where a window is in a word or two
fn detail(window: &WindowInfo) -> String {
    match (window.collapsed, window.workspace, window.tiled) {
        (true, _, _) => "collapsed".into(),
        (false, Some(n), true) => format!("workspace {n}"),
        _ => "floating".into(),
    }
}

impl WindowSearch {
    pub fn new(app: &gtk4::Application) -> Rc<Self> {
        let window = gtk4::ApplicationWindow::new(app);
        window.init_layer_shell();
        window.set_namespace(Some("windows"));
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        window.add_css_class("launcher");

        let panel = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        panel.add_css_class("panel");
        panel.set_size_request(520, -1);

        let search = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        let icon = gtk4::Label::new(Some("\u{f0349}"));
        icon.add_css_class("search-icon");
        let entry = gtk4::Entry::new();
        entry.set_has_frame(false);
        entry.set_hexpand(true);
        entry.set_placeholder_text(Some("Find a window"));
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
            let detail = gtk4::Label::new(None);
            detail.add_css_class("detail");
            frame.append(&icon);
            frame.append(&name);
            frame.append(&detail);
            panel.append(&frame);
            rows.push(Row {
                frame,
                icon,
                name,
                detail,
            });
        }
        let empty = gtk4::Label::new(Some("No windows match"));
        empty.set_xalign(0.0);
        empty.add_css_class("empty");
        empty.set_visible(false);
        panel.append(&empty);
        window.set_child(Some(&panel));

        let windows = ipc::request(serde_json::json!({ "get": "state" }))
            .ok()
            .and_then(|state| serde_json::from_value::<ipc::State>(state).ok())
            .map(|state| state.windows)
            .unwrap_or_default();

        let search = Rc::new(Self {
            window,
            entry,
            count,
            empty,
            rows,
            windows,
            results: RefCell::new(Vec::new()),
            selected: RefCell::new(0),
            offset: RefCell::new(0),
        });
        search.connect();
        search.search();
        search
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
                this.focus_selected();
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
                    this.focus_selected();
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
        // most recent first like alt-tab but ur current window goes last so enter goes back to the last one
        let recency = |w: &WindowInfo| {
            if w.focused {
                0
            } else {
                u64::MAX - w.recent.unwrap_or(usize::MAX / 2) as u64
            }
        };
        let mut ranked: Vec<(u64, u64, usize)> = (0..self.windows.len())
            .filter_map(|i| {
                let window = &self.windows[i];
                let quality = if query.is_empty() {
                    Some(0)
                } else {
                    score(&query, window)
                }?;
                Some((quality, recency(window), i))
            })
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        *self.results.borrow_mut() = ranked.into_iter().map(|(_, _, i)| i).collect();
        *self.selected.borrow_mut() = 0;
        *self.offset.borrow_mut() = 0;
        self.render();
    }

    fn render(&self) {
        let results = self.results.borrow();
        let (selected, offset) = (*self.selected.borrow(), *self.offset.borrow());
        for (i, row) in self.rows.iter().enumerate() {
            match results.get(offset + i) {
                Some(&index) => {
                    let window = &self.windows[index];
                    let title = if window.title.is_empty() {
                        &window.app_id
                    } else {
                        &window.title
                    };
                    row.name.set_text(title);
                    row.detail.set_text(&detail(window));
                    match icon_for(&window.app_id) {
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
            .set_text(&format!("{}/{}", results.len(), self.windows.len()));
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

    fn focus_selected(&self) {
        let Some(&index) = self.results.borrow().get(*self.selected.borrow()) else {
            return;
        };
        let id = self.windows[index].id;
        // close first bc the search holds the keyboard till its gone
        self.window.close();
        ipc::focus(id);
    }

    pub fn show(&self) {
        self.window.present();
        self.entry.grab_focus();
    }
}
