//! the search list the launcher and window search share where u type to narrow it and enter picks the selected row

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{gdk, glib};
use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};

const VISIBLE_ROWS: usize = 8;

pub const CSS: &str = "
.launcher { background: transparent; }
.launcher * { font-size: 14px; color: @m3onSurface; }
.launcher .panel {
    background: @m3surface;
    border-radius: 28px;
    padding: 12px;
    border: 1px solid alpha(@m3outlineVariant, 0.35);
}
.launcher .search {
    background: @m3surfaceContainerHigh;
    border-radius: 9999px;
    padding: 2px 14px 2px 8px;
    min-height: 44px;
}
.launcher .search-icon { font-size: 22px; color: @m3onSurfaceVariant; margin: 0 8px 0 6px; }
.launcher entry {
    background: transparent; border: none; box-shadow: none; outline: none;
    padding: 4px 0; caret-color: @m3primary; min-height: 0; font-size: 15px;
}
.launcher entry selection { background: @m3primary; color: @m3onPrimary; }
.launcher .divider { background: transparent; min-height: 8px; }
.launcher .row {
    border-radius: 16px;
    padding: 8px 12px;
    transition: background 150ms cubic-bezier(0.2, 0, 0, 1);
}
.launcher .row:hover { background: alpha(@m3onSurface, 0.08); }
.launcher .row.selected { background: @m3secondaryContainer; }
.launcher .row.selected label { color: @m3onSecondaryContainer; }
.launcher .empty { padding: 8px 12px; color: @m3onSurfaceVariant; }
.launcher .count { font-size: 12px; color: @m3outline; }
.launcher .detail { font-size: 12px; color: @m3onSurfaceVariant; }
.launcher .glyph { font-size: 22px; min-width: 28px; }
.launcher .glyph.icon { color: @m3primary; }
.launcher .row.selected .glyph.icon { color: @m3onSecondaryContainer; }
.launcher .key {
    background: @m3surfaceContainerHighest;
    color: @m3onSurfaceVariant;
    border-radius: 8px;
    padding: 1px 7px;
    font-size: 12px;
    font-weight: 500;
}
.launcher .row.selected .key { background: alpha(@m3onSecondaryContainer, 0.12); color: @m3onSecondaryContainer; }
.launcher .or { font-size: 12px; color: @m3outline; margin: 0 2px; }
.launcher .hint { font-size: 12px; color: @m3outline; margin: 6px 12px 0 12px; }
.launcher .row.selected .detail { color: alpha(@m3onSecondaryContainer, 0.75); }
";

/// one visible line of the list
pub struct Row {
    frame: gtk4::Box,
    pub icon: gtk4::Image,
    /// a text icon like an emoji or a material symbol for sources w/o app icons
    pub glyph: gtk4::Label,
    pub name: gtk4::Label,
    /// key caps on the right for the keybind search
    pub keys: gtk4::Box,
    /// a word or two on the right if the list has them
    pub detail: Option<gtk4::Label>,
}

/// what a picker lists
pub trait Source: 'static {
    fn len(&self) -> usize;
    /// the items matching query best first where query is already trimmed and lowercased
    fn rank(&self, query: &str) -> Vec<usize>;
    /// what was typed as is before rank gets it lowercased
    fn typed(&self, _raw: &str) {}
    /// put item index in row
    fn fill(&self, index: usize, row: &Row);
    /// item index got picked and closing window is up to u
    fn pick(&self, index: usize, window: &gtk4::ApplicationWindow);
    /// shift+delete on item index and true means the list changed
    fn remove(&self, _index: usize) -> bool {
        false
    }
}

/// how a picker looks
pub struct Look {
    pub namespace: &'static str,
    pub width: i32,
    pub placeholder: Option<&'static str>,
    pub empty: &'static str,
    pub detail: bool,
    /// a line under the list like what the keys do
    pub hint: Option<&'static str>,
}

pub struct Picker<S: Source> {
    pub window: gtk4::ApplicationWindow,
    entry: gtk4::Entry,
    count: gtk4::Label,
    empty: gtk4::Label,
    rows: Vec<Row>,
    source: S,
    results: RefCell<Vec<usize>>,
    selected: RefCell<usize>,
    offset: RefCell<usize>,
}

pub fn is_subsequence(query: &str, text: &str) -> bool {
    let mut chars = text.chars();
    query.chars().all(|q| chars.any(|c| c == q))
}

impl<S: Source> Picker<S> {
    pub fn build(app: &gtk4::Application, look: Look, source: S) -> Rc<Self> {
        let window = gtk4::ApplicationWindow::new(app);
        window.init_layer_shell();
        window.set_namespace(Some(look.namespace));
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        window.add_css_class("launcher");
        crate::style::adopt(&window);

        let panel = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        panel.add_css_class("panel");
        panel.set_size_request(look.width, -1);

        let search = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        search.add_css_class("search");
        let icon = crate::style::icon("search");
        icon.add_css_class("search-icon");
        let entry = gtk4::Entry::new();
        entry.set_has_frame(false);
        entry.set_hexpand(true);
        entry.set_placeholder_text(look.placeholder);
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
            let glyph = gtk4::Label::new(None);
            glyph.add_css_class("glyph");
            glyph.set_visible(false);
            let name = gtk4::Label::new(None);
            name.set_xalign(0.0);
            name.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            name.set_hexpand(true);
            frame.append(&icon);
            frame.append(&glyph);
            frame.append(&name);
            let keys = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
            keys.set_visible(false);
            frame.append(&keys);
            let detail = look.detail.then(|| {
                let detail = gtk4::Label::new(None);
                detail.add_css_class("detail");
                frame.append(&detail);
                detail
            });
            panel.append(&frame);
            rows.push(Row {
                frame,
                icon,
                glyph,
                name,
                keys,
                detail,
            });
        }
        let empty = gtk4::Label::new(Some(look.empty));
        empty.set_xalign(0.0);
        empty.add_css_class("empty");
        empty.set_visible(false);
        panel.append(&empty);
        if let Some(hint) = look.hint {
            let hint = gtk4::Label::new(Some(hint));
            hint.set_xalign(0.0);
            hint.add_css_class("hint");
            panel.append(&hint);
        }
        window.set_child(Some(&panel));

        let picker = Rc::new(Self {
            window,
            entry,
            count,
            empty,
            rows,
            source,
            results: RefCell::new(Vec::new()),
            selected: RefCell::new(0),
            offset: RefCell::new(0),
        });
        picker.connect();
        picker.search();
        picker
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
                this.pick_selected();
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
                gdk::Key::Delete if modifiers.contains(gdk::ModifierType::SHIFT_MASK) => this.remove_selected(),
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
                    this.pick_selected();
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
        self.source.typed(self.entry.text().trim());
        let query = self.entry.text().trim().to_lowercase();
        *self.results.borrow_mut() = self.source.rank(&query);
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
                    self.source.fill(index, row);
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
            .set_text(&format!("{}/{}", results.len(), self.source.len()));
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

    fn remove_selected(&self) {
        let Some(&index) = self.results.borrow().get(*self.selected.borrow()) else {
            return;
        };
        if self.source.remove(index) {
            let keep = *self.selected.borrow();
            self.search();
            let len = self.results.borrow().len();
            *self.selected.borrow_mut() = keep.min(len.saturating_sub(1));
            self.step(0);
        }
    }

    fn pick_selected(&self) {
        let Some(&index) = self.results.borrow().get(*self.selected.borrow()) else {
            return;
        };
        self.source.pick(index, &self.window);
    }

    pub fn show(&self) {
        self.window.present();
        self.entry.grab_focus();
    }
}
