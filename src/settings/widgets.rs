//! the building blocks of every settings page like caelestias nexus w grouped rounded cards

use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{gdk, glib};
use toml::Value;

use super::store::{Store, Which};

/// the settings look in material you colors
pub const CSS: &str = "
.settings { background: @m3surface; color: @m3onSurface; }
.settings headerbar, .settings .titlebar {
    background: @windowTopbar; color: @windowOnTopbar;
    border: none; box-shadow: none; min-height: 38px;
}
.settings headerbar .title { color: @windowOnTopbar; font-weight: 500; }
.settings windowcontrols button {
    background: transparent; border-radius: 9999px; min-width: 28px; min-height: 28px; padding: 0;
}
.settings windowcontrols button:hover { background: alpha(@m3onSurface, 0.08); }
.settings windowcontrols button image { color: @windowOnTopbar; }
.settings * { font-size: 14px; }
.settings .nav { padding: 16px 12px 12px 16px; }
.settings .nav-title { font-size: 22px; font-weight: 500; margin: 4px 0 14px 8px; }
.settings .search {
    background: @m3surfaceContainerLowest;
    border: 1px solid @m3outlineVariant;
    border-radius: 9999px;
    min-height: 44px;
    padding: 0 14px;
    margin-bottom: 12px;
}
.settings .search .icon { font-size: 20px; color: @m3onSurfaceVariant; }
.settings .search text { background: transparent; }
.settings .search entry, .settings entry.bare {
    background: transparent; border: none; box-shadow: none; outline: none; min-height: 0; padding: 0;
}
.settings .navrow {
    border-radius: 9999px;
    padding: 12px 18px;
    background: transparent;
    border: none;
    box-shadow: none;
    transition: background 200ms cubic-bezier(0.2, 0, 0, 1);
}
.settings .navrow label { color: @m3onSurface; }
.settings .navrow:hover { background: alpha(@m3onSurface, 0.08); }
.settings .navrow.selected { background: @m3secondaryContainer; }
.settings .navrow.selected label { color: @m3onSecondaryContainer; }
.settings .navrow .icon { font-size: 22px; color: @m3onSurfaceVariant; }
.settings .navrow.selected .icon { font-variation-settings: \"FILL\" 1; }
.settings .navgroup { font-size: 12px; font-weight: 500; color: @m3onSurfaceVariant; margin: 14px 0 4px 18px; }
.settings .status { font-size: 12px; color: @m3onSurfaceVariant; margin: 8px 0 0 18px; }
.settings .status.error { color: @m3error; }
.settings .content {
    background: @m3surfaceContainerLow;
    border-radius: 28px;
    margin: 12px 12px 12px 0;
}
.settings .page { padding: 28px 36px 36px 36px; }
.settings .page-head .icon {
    font-size: 24px;
    color: @m3onSurfaceVariant;
    background: @m3surfaceContainerHigh;
    border-radius: 9999px;
    min-width: 48px; min-height: 48px;
}
.settings .page-title { font-size: 22px; font-weight: 500; }
.settings .section-title {
    font-size: 13px; font-weight: 500; color: @m3onSurfaceVariant;
    margin: 20px 0 6px 8px;
}
.settings .section-title.first { margin-top: 0; }
.settings .card {
    background: @m3surfaceContainer;
    border-radius: 4px;
    padding: 12px 20px;
    min-height: 32px;
}
.settings .card.first { border-top-left-radius: 20px; border-top-right-radius: 20px; }
.settings .card.last { border-bottom-left-radius: 20px; border-bottom-right-radius: 20px; }
.settings .row-title { font-size: 14px; color: @m3onSurface; }
.settings .row-hint { font-size: 12px; color: @m3outline; }
.settings .note { font-size: 12px; color: @m3onSurfaceVariant; margin: 0 8px 8px 8px; }

.settings switch {
    background: @m3surfaceContainerHighest;
    border: 2px solid @m3outline;
    border-radius: 9999px;
    min-width: 48px;
    min-height: 26px;
    padding: 0;
    box-shadow: none;
}
.settings switch:checked { background: @m3primary; border-color: @m3primary; }
.settings switch slider {
    background: @m3outline;
    border-radius: 9999px;
    min-width: 18px; min-height: 18px;
    margin: 4px;
    border: none; box-shadow: none;
}
.settings switch:checked slider { background: @m3onPrimary; min-width: 22px; min-height: 22px; margin: 2px; }
.settings switch image { opacity: 0; }

.settings spinbutton, .settings entry, .settings dropdown > button, .settings .pill-field {
    background: @m3surfaceContainerHigh;
    border: none;
    box-shadow: none;
    border-radius: 9999px;
    min-height: 36px;
    color: @m3onSurface;
}
.settings entry { padding: 0 14px; caret-color: @m3primary; }
.settings entry:focus-within, .settings spinbutton:focus-within { outline: 2px solid @m3primary; outline-offset: -2px; }
.settings spinbutton { padding: 0 4px 0 14px; }
.settings spinbutton text { background: transparent; }
.settings spinbutton button {
    background: transparent; border: none; box-shadow: none; border-radius: 9999px;
    min-width: 30px; min-height: 30px; color: @m3onSurfaceVariant;
}
.settings spinbutton button:hover { background: alpha(@m3onSurface, 0.08); }
.settings dropdown > button { padding: 0 14px; }
.settings dropdown > button:hover { background: @m3surfaceContainerHighest; }
.settings popover contents {
    background: @m3surfaceContainer; border-radius: 16px; padding: 6px; border: none;
    box-shadow: 0 4px 12px alpha(@m3shadow, 0.4);
}
.settings popover row { border-radius: 12px; padding: 8px 10px; }
.settings popover row:hover, .settings popover row:selected { background: @m3secondaryContainer; }
.settings popover label { color: @m3onSurface; }

.settings button.pill {
    background: @m3surfaceContainerHigh; border: none; box-shadow: none;
    border-radius: 9999px; padding: 6px 16px; min-height: 26px; color: @m3onSurface;
    transition: background 150ms cubic-bezier(0.2, 0, 0, 1);
}
.settings button.pill:hover { background: @m3surfaceContainerHighest; }
.settings button.filled { background: @m3primary; }
.settings button.filled label { color: @m3onPrimary; }
.settings button.filled:hover { background: mix(@m3primary, @m3onPrimary, 0.08); }
.settings button.tonal { background: @m3secondaryContainer; }
.settings button.tonal label { color: @m3onSecondaryContainer; }
.settings button.danger { background: @m3errorContainer; }
.settings button.danger label { color: @m3onErrorContainer; }
.settings button.icon-button { padding: 6px; min-width: 26px; }
.settings button.icon-button .icon { font-size: 20px; }
.settings togglebutton, .settings button.toggle { border-radius: 9999px; }
.settings .segmented button { border-radius: 4px; margin: 0 1px; }
.settings .segmented button:first-child { border-top-left-radius: 9999px; border-bottom-left-radius: 9999px; }
.settings .segmented button:last-child { border-top-right-radius: 9999px; border-bottom-right-radius: 9999px; }
.settings .segmented button.active { background: @m3secondaryContainer; }
.settings .segmented button.active label { color: @m3onSecondaryContainer; }

.settings scale trough { background: @m3surfaceContainerHighest; border-radius: 9999px; min-height: 10px; border: none; }
.settings scale highlight { background: @m3primary; border-radius: 9999px; border: none; min-width: 10px; }
.settings scale slider {
    background: @m3primary; border-radius: 9999px; min-width: 18px; min-height: 18px;
    margin: -5px; border: none; box-shadow: none;
}
.settings colorswatch, .settings .swatch { border-radius: 9999px; }
.settings .search entry, .settings .search entry:focus-within {
    background: transparent; border: none; box-shadow: none; outline: none; min-height: 0; padding: 0;
}
.settings checkbutton radio, .settings checkbutton check {
    background: transparent; border: 2px solid @m3onSurfaceVariant; box-shadow: none; color: transparent;
    min-width: 16px; min-height: 16px;
}
.settings checkbutton radio:checked, .settings checkbutton check:checked {
    border-color: @m3primary; background: @m3primary; color: @m3onPrimary;
}
.settings togglebutton.pill:checked, .settings button.pill:checked { background: @m3secondaryContainer; }
.settings .swatch-name { font-size: 11px; color: @m3onSurfaceVariant; }
.settings .preview { border-radius: 16px; }
.settings .list-empty { color: @m3onSurfaceVariant; }
.settings scrollbar { background: transparent; }
.settings scrollbar slider { background: alpha(@m3onSurface, 0.3); border-radius: 9999px; min-width: 6px; }
";

/// a page w its big header and a column of sections
pub struct Page {
    pub root: gtk4::ScrolledWindow,
    pub column: gtk4::Box,
    /// the cards of the section being filled so first and last get round corners
    group: std::cell::RefCell<Vec<gtk4::Box>>,
    group_box: std::cell::RefCell<Option<gtk4::Box>>,
    /// words to match searches against
    pub words: std::cell::RefCell<String>,
}

impl Page {
    pub fn new(icon: &str, title: &str) -> Rc<Self> {
        let column = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        column.add_css_class("page");
        column.set_hexpand(true);
        let clamp = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        clamp.set_halign(gtk4::Align::Center);
        clamp.set_size_request(640, -1);
        clamp.append(&column);
        let head = gtk4::Box::new(gtk4::Orientation::Horizontal, 16);
        head.add_css_class("page-head");
        head.set_margin_bottom(24);
        let badge = crate::style::icon(icon);
        badge.set_valign(gtk4::Align::Center);
        let name = gtk4::Label::new(Some(title));
        name.add_css_class("page-title");
        head.append(&badge);
        head.append(&name);
        column.append(&head);
        let root = gtk4::ScrolledWindow::new();
        root.set_hscrollbar_policy(gtk4::PolicyType::Never);
        root.set_child(Some(&clamp));
        root.set_vexpand(true);
        Rc::new(Self {
            root,
            column,
            group: Default::default(),
            group_box: Default::default(),
            words: std::cell::RefCell::new(title.to_lowercase()),
        })
    }

    /// start a new titled group of cards
    pub fn section(&self, title: &str) {
        self.close_group();
        let label = gtk4::Label::new(Some(title));
        label.add_css_class("section-title");
        label.set_xalign(0.0);
        if self.column.observe_children().n_items() <= 1 {
            label.add_css_class("first");
        }
        self.column.append(&label);
        let group = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        self.column.append(&group);
        *self.group_box.borrow_mut() = Some(group);
        self.words.borrow_mut().push_str(&format!(" {}", title.to_lowercase()));
    }

    /// a small grey note under the last group
    pub fn note(&self, text: &str) {
        self.close_group();
        let label = gtk4::Label::new(Some(text));
        label.add_css_class("note");
        label.set_xalign(0.0);
        label.set_wrap(true);
        self.column.append(&label);
    }

    /// put any widget in the page outside the cards
    pub fn append(&self, widget: &impl IsA<gtk4::Widget>) {
        self.close_group();
        self.column.append(widget);
    }

    fn close_group(&self) {
        let cards: Vec<gtk4::Box> = self.group.borrow_mut().drain(..).collect();
        for (i, card) in cards.iter().enumerate() {
            card.remove_css_class("first");
            card.remove_css_class("last");
            if i == 0 {
                card.add_css_class("first");
            }
            if i + 1 == cards.len() {
                card.add_css_class("last");
            }
        }
        *self.group_box.borrow_mut() = None;
    }

    /// finish the page so the last group gets its round bottom
    pub fn done(&self) {
        self.close_group();
    }

    /// a card w a title an optional hint and a control on the right
    pub fn row(&self, title: &str, hint: Option<&str>, control: &impl IsA<gtk4::Widget>) -> gtk4::Box {
        let card = gtk4::Box::new(gtk4::Orientation::Horizontal, 16);
        card.add_css_class("card");
        let text = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        text.set_hexpand(true);
        text.set_valign(gtk4::Align::Center);
        let name = gtk4::Label::new(Some(title));
        name.add_css_class("row-title");
        name.set_xalign(0.0);
        name.set_wrap(true);
        text.append(&name);
        if let Some(hint) = hint {
            let h = gtk4::Label::new(Some(hint));
            h.add_css_class("row-hint");
            h.set_xalign(0.0);
            h.set_wrap(true);
            text.append(&h);
        }
        card.append(&text);
        control.set_valign(gtk4::Align::Center);
        card.append(control);
        self.push_card(&card);
        self.words
            .borrow_mut()
            .push_str(&format!(" {} {}", title.to_lowercase(), hint.unwrap_or("").to_lowercase()));
        card
    }

    /// a card holding anything that fills the whole width
    pub fn wide(&self, widget: &impl IsA<gtk4::Widget>) -> gtk4::Box {
        let card = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        card.add_css_class("card");
        card.append(widget);
        self.push_card(&card);
        card
    }

    fn push_card(&self, card: &gtk4::Box) {
        if self.group_box.borrow().is_none() {
            let group = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
            self.column.append(&group);
            *self.group_box.borrow_mut() = Some(group);
        }
        if let Some(group) = self.group_box.borrow().as_ref() {
            group.append(card);
        }
        self.group.borrow_mut().push(card.clone());
    }
}

/// an on off switch tied to a bool setting
pub fn switch(store: &Rc<Store>, which: Which, path: &str) -> gtk4::Switch {
    let w = gtk4::Switch::new();
    w.set_active(store.bool(which, path));
    let (store, path) = (store.clone(), path.to_string());
    w.connect_active_notify(move |w| store.put(which, &path, Value::Boolean(w.is_active())));
    w
}

/// a number box tied to a setting and floats keep their digits
pub fn number(store: &Rc<Store>, which: Which, path: &str, low: f64, high: f64, step: f64, digits: u32) -> gtk4::SpinButton {
    let w = gtk4::SpinButton::with_range(low, high, step);
    w.set_digits(digits);
    w.set_value(store.num(which, path));
    w.set_numeric(true);
    let (store, path) = (store.clone(), path.to_string());
    w.connect_value_changed(move |w| {
        let v = if digits == 0 {
            Value::Integer(w.value().round() as i64)
        } else {
            let f = 10f64.powi(digits as i32);
            Value::Float((w.value() * f).round() / f)
        };
        store.put(which, &path, v);
    });
    w
}

/// a dropdown of plain options tied to a string setting
pub fn choice(store: &Rc<Store>, which: Which, path: &str, options: &[&str]) -> gtk4::DropDown {
    choice_labeled(store, which, path, &options.iter().map(|o| (*o, *o)).collect::<Vec<_>>())
}

/// a dropdown whose options show nicer names than their values
pub fn choice_labeled(store: &Rc<Store>, which: Which, path: &str, options: &[(&str, &str)]) -> gtk4::DropDown {
    let mut values: Vec<String> = options.iter().map(|(v, _)| v.to_string()).collect();
    let mut labels: Vec<String> = options.iter().map(|(_, l)| l.to_string()).collect();
    let current = store.str(which, path);
    // a value from the file that isnt a listed option still shows
    if !current.is_empty() && !values.contains(&current) {
        values.push(current.clone());
        labels.push(current.clone());
    }
    let list = gtk4::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    let w = gtk4::DropDown::new(Some(list), gtk4::Expression::NONE);
    if let Some(i) = values.iter().position(|v| *v == current) {
        w.set_selected(i as u32);
    }
    let (store, path) = (store.clone(), path.to_string());
    w.connect_selected_notify(move |w| {
        if let Some(v) = values.get(w.selected() as usize) {
            store.put(which, &path, Value::String(v.clone()));
        }
    });
    w
}

/// a text field tied to a string setting
pub fn text(store: &Rc<Store>, which: Which, path: &str, width: i32) -> gtk4::Entry {
    let w = gtk4::Entry::new();
    w.set_text(&store.str(which, path));
    w.set_width_chars(width);
    let (store, path) = (store.clone(), path.to_string());
    w.connect_changed(move |w| store.put(which, &path, Value::String(w.text().to_string())));
    w
}

/// a list of strings split by semicolons that only saves on enter or leaving so half typed commands never start
pub fn list_text(store: &Rc<Store>, which: Which, path: &str, width: i32) -> gtk4::Entry {
    let w = gtk4::Entry::new();
    let items: Vec<String> = store
        .get(which, path)
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    w.set_text(&items.join("; "));
    w.set_width_chars(width);
    w.set_placeholder_text(Some("none"));
    let save = {
        let (store, path) = (store.clone(), path.to_string());
        move |w: &gtk4::Entry| {
            let items: Vec<Value> = w
                .text()
                .split(';')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| Value::String(s.into()))
                .collect();
            store.put(which, &path, Value::Array(items));
        }
    };
    w.connect_activate(save.clone());
    let focus = gtk4::EventControllerFocus::new();
    {
        let w2 = w.downgrade();
        focus.connect_leave(move |_| {
            if let Some(w) = w2.upgrade() {
                save(&w);
            }
        });
    }
    w.add_controller(focus);
    w
}

/// two numbers side by side like a size or an offset
pub fn pair(store: &Rc<Store>, which: Which, path: &str, low: f64, high: f64, step: f64) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let current: Vec<f64> = store
        .get(which, path)
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(|v| v.as_integer().map(|i| i as f64).or(v.as_float()).unwrap_or(0.0))
        .collect();
    let spins: Vec<gtk4::SpinButton> = (0..2)
        .map(|i| {
            let s = gtk4::SpinButton::with_range(low, high, step);
            s.set_value(current.get(i).copied().unwrap_or(0.0));
            s
        })
        .collect();
    for (i, s) in spins.iter().enumerate() {
        row.append(s);
        if i == 0 {
            row.append(&gtk4::Label::new(Some("×")));
        }
        let (store, path, spins) = (store.clone(), path.to_string(), spins.clone());
        s.connect_value_changed(move |_| {
            let v = spins.iter().map(|s| Value::Integer(s.value().round() as i64)).collect();
            store.put(which, &path, Value::Array(v));
        });
    }
    row
}

/// #rrggbb or #rrggbbaa into a gdk color
pub fn parse_color(s: &str) -> gdk::RGBA {
    let h = s.trim().trim_start_matches('#');
    let byte = |i: usize| h.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok());
    match (byte(0), byte(2), byte(4)) {
        (Some(r), Some(g), Some(b)) => gdk::RGBA::new(
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            byte(6).map_or(1.0, |a| a as f32 / 255.0),
        ),
        _ => gdk::RGBA::new(0.5, 0.5, 0.5, 1.0),
    }
}

pub fn color_hex(c: &gdk::RGBA, alpha: bool) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    if alpha && c.alpha() < 0.999 {
        format!("#{:02x}{:02x}{:02x}{:02x}", b(c.red()), b(c.green()), b(c.blue()), b(c.alpha()))
    } else {
        format!("#{:02x}{:02x}{:02x}", b(c.red()), b(c.green()), b(c.blue()))
    }
}

/// a color button tied to a hex setting
pub fn color(store: &Rc<Store>, which: Which, path: &str, alpha: bool) -> gtk4::ColorDialogButton {
    let dialog = gtk4::ColorDialog::new();
    dialog.set_with_alpha(alpha);
    let w = gtk4::ColorDialogButton::new(Some(dialog));
    w.set_rgba(&parse_color(&store.str(which, path)));
    let (store, path) = (store.clone(), path.to_string());
    w.connect_rgba_notify(move |w| store.put(which, &path, Value::String(color_hex(&w.rgba(), alpha))));
    w
}

/// a round button w text
pub fn button(label: &str, class: &str, f: impl Fn() + 'static) -> gtk4::Button {
    let b = gtk4::Button::with_label(label);
    b.add_css_class("pill");
    if !class.is_empty() {
        b.add_css_class(class);
    }
    b.connect_clicked(move |_| f());
    b
}

/// pick a file w the portal and get its path back as ~ relative when its in ur home
pub fn pick_image(parent: &gtk4::Window, done: impl Fn(String) + 'static) {
    let filter = gtk4::FileFilter::new();
    filter.set_name(Some("Images"));
    for mime in ["image/png", "image/jpeg", "image/webp"] {
        filter.add_mime_type(mime);
    }
    let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
    filters.append(&filter);
    let dialog = gtk4::FileDialog::new();
    dialog.set_title("Pick an image");
    dialog.set_filters(Some(&filters));
    dialog.open(Some(parent), gtk4::gio::Cancellable::NONE, move |result| {
        if let Ok(file) = result
            && let Some(path) = file.path()
        {
            let home = glib::home_dir();
            let text = match path.strip_prefix(&home) {
                Ok(rest) => format!("~/{}", rest.display()),
                Err(_) => path.display().to_string(),
            };
            done(text);
        }
    });
}

/// a scaled preview of an image path or nothing
pub fn preview_image(path: &str, width: i32, height: i32) -> gtk4::Picture {
    let pic = gtk4::Picture::new();
    pic.add_css_class("preview");
    pic.set_size_request(width, height);
    pic.set_can_shrink(true);
    pic.set_content_fit(gtk4::ContentFit::Cover);
    set_preview(&pic, path);
    pic
}

pub fn set_preview(pic: &gtk4::Picture, path: &str) {
    let full = crate::theme::expand(path);
    if !path.is_empty() && full.is_file() {
        pic.set_filename(Some(&full));
    } else {
        pic.set_paintable(gdk::Paintable::NONE);
    }
}
