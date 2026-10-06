//! the sevenshell pages for the bar notifications and the lock screen

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::glib;
use toml::Value;

use super::pages::image_row;
use super::preview::{NotificationPlacer, Placement, POSITIONS};
use super::store::Which;
use super::widgets::{self as w, Page};
use super::Ctx;

const WM: Which = Which::Wm;
const SH: Which = Which::Shell;

const MODULES: [(&str, &str); 21] = [
    ("desktop", "Arch logo that opens a terminal"),
    ("clock", "Clock"),
    ("audio", "Volume"),
    ("network", "Network"),
    ("perf", "Power mode"),
    ("power", "Power menu"),
    ("battery", "Battery"),
    ("title", "Focused window title"),
    ("minimap", "Canvas map"),
    ("cava", "Audio visualizer (cava)"),
    ("media", "What's playing"),
    ("weather", "Weather"),
    ("updates", "Updates waiting"),
    ("privacy", "Mic, camera & screen share dots"),
    ("caffeine", "Caffeine toggle"),
    ("dnd", "Do not disturb toggle"),
    ("nightlight", "Night light toggle"),
    ("antiflash", "Anti-flashbang toggle"),
    ("clipboard", "Clipboard history"),
    ("session", "Session screen"),
    ("quick", "Quick settings"),
];

fn label_of(name: &str) -> String {
    if let Some(custom) = name.strip_prefix("custom/") {
        return format!("{custom} (custom)");
    }
    MODULES.iter().find(|(n, _)| *n == name).map_or(name, |(_, l)| l).to_string()
}

fn modules(store: &super::store::Store, part: &str) -> Vec<String> {
    store
        .get(SH, &format!("bar.{part}"))
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect()
}

/// the names of ur own modules
fn custom_names(store: &super::store::Store) -> Vec<String> {
    store
        .get(SH, "bar.custom")
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|c| c.get("name")?.as_str().map(String::from))
        .collect()
}

/// every module u could add including ur own
fn all_modules(store: &super::store::Store) -> Vec<String> {
    MODULES
        .iter()
        .map(|(n, _)| n.to_string())
        .chain(custom_names(store).into_iter().map(|n| format!("custom/{n}")))
        .collect()
}

/// move the module dragged from part:index to part to before index or the end
fn move_module(store: &Rc<super::store::Store>, from: &str, to: &str, before: Option<usize>) {
    let Some((from_part, index)) = from.split_once(':') else {
        return;
    };
    let Ok(index) = index.parse::<usize>() else {
        return;
    };
    let mut source = modules(store, from_part);
    if index >= source.len() {
        return;
    }
    let name = source.remove(index);
    let mut target = if from_part == to { source.clone() } else { modules(store, to) };
    // the slot shifts left when it came from earlier in the same list
    let at = before.map_or(target.len(), |b| if from_part == to && b > index { b - 1 } else { b });
    target.insert(at.min(target.len()), name);
    let list = |l: Vec<String>| Value::Array(l.into_iter().map(Value::String).collect());
    if from_part != to {
        store.put(SH, &format!("bar.{from_part}"), list(source));
    }
    store.put(SH, &format!("bar.{to}"), list(target));
}

/// a strftime field w a live example under it
fn format_row(page: &Page, ctx: &Rc<Ctx>, path: &str, title: &str) {
    let entry = w::text(&ctx.store, SH, path, 20);
    let example = gtk4::Label::new(None);
    example.add_css_class("row-hint");
    let update = {
        let example = example.clone();
        move |e: &gtk4::Entry| {
            let text = glib::DateTime::now_local()
                .ok()
                .and_then(|now| now.format(&e.text()).ok())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "not a valid format".into());
            example.set_text(&text);
        }
    };
    update(&entry);
    entry.connect_changed(update);
    let col = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    col.append(&entry);
    col.append(&example);
    page.row(title, Some("%H hour %M minute %I 12 hour %p am pm %A weekday %B month %d day %Y year"), &col);
}

pub fn bar(ctx: &Rc<Ctx>) -> Rc<Page> {
    let page = Page::new("toolbar", "Bar");
    let s = &ctx.store;
    page.section("Look");
    page.row("Position", Some("On the left or right it only shows icons"), &w::choice_labeled(s, SH, "bar.position", &[("top", "Top"), ("bottom", "Bottom"), ("left", "Left"), ("right", "Right")]));
    page.row("Hide till the mouse touches its edge", None, &w::switch(s, SH, "bar.autohide"));
    page.row("Floating", Some("Rounded w a gap around it"), &w::switch(s, SH, "bar.floating"));
    page.row("Opacity", Some("0 is see through and 1 is solid"), &w::number(s, SH, "bar.opacity", 0.0, 1.0, 0.05, 2));
    page.row("Size", Some("How tall or on the sides how wide and icons and text grow w it"), &w::number(s, SH, "bar.height", 20.0, 96.0, 1.0, 0));
    page.section("Modules");
    // every sections list gets rebuilt when any changes bc a module can only be in one
    let rebuilds: Rc<RefCell<Vec<Rc<dyn Fn()>>>> = Rc::default();
    for (part, title) in [("left", "Left"), ("center", "Middle"), ("right", "Right")] {
        let col = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        let chips = gtk4::FlowBox::new();
        chips.set_selection_mode(gtk4::SelectionMode::None);
        chips.set_max_children_per_line(4);
        chips.set_row_spacing(6);
        chips.set_column_spacing(6);
        let add = gtk4::DropDown::from_strings(&["Add a module"]);
        add.set_halign(gtk4::Align::Start);
        col.append(&chips);
        col.append(&add);
        let (store, rebuilds2) = (s.clone(), rebuilds.clone());
        let change = Rc::new(move |list: Vec<String>| {
            store.put(SH, &format!("bar.{part}"), Value::Array(list.into_iter().map(Value::String).collect()));
            for r in rebuilds2.borrow().iter() {
                r();
            }
        });
        let adding = Rc::new(std::cell::Cell::new(false));
        let options: Rc<RefCell<Vec<String>>> = Rc::default();
        // dropping on empty space in a section puts it at the end
        let drop = gtk4::DropTarget::new(glib::Type::STRING, gtk4::gdk::DragAction::MOVE);
        {
            let (store, rebuilds2) = (s.clone(), rebuilds.clone());
            drop.connect_drop(move |_, value, _, _| {
                let Ok(from) = value.get::<String>() else {
                    return false;
                };
                move_module(&store, &from, part, None);
                for r in rebuilds2.borrow().iter() {
                    r();
                }
                true
            });
        }
        col.add_controller(drop);
        let rebuild: Rc<dyn Fn()> = {
            let rebuilds3 = rebuilds.clone();
            let (store, chips, add, change, adding, options) = (s.clone(), chips.clone(), add.clone(), change.clone(), adding.clone(), options.clone());
            Rc::new(move || {
                while let Some(child) = chips.first_child() {
                    chips.remove(&child);
                }
                let list = modules(&store, part);
                for (i, name) in list.iter().enumerate() {
                    let chip = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
                    chip.add_css_class("pill-field");
                    chip.set_margin_start(2);
                    let label = gtk4::Label::new(Some(&label_of(name)));
                    label.set_margin_start(12);
                    label.set_margin_end(4);
                    chip.append(&label);
                    for (icon, tip, step) in [("chevron_left", "Further left", -1i32), ("chevron_right", "Further right", 1), ("close", "Remove", 0)] {
                        let b = gtk4::Button::new();
                        b.set_child(Some(&crate::style::icon(icon)));
                        b.add_css_class("pill");
                        b.add_css_class("icon-button");
                        b.set_tooltip_text(Some(tip));
                        let (list, change) = (list.clone(), change.clone());
                        b.connect_clicked(move |_| {
                            let mut list = list.clone();
                            if step == 0 {
                                list.remove(i);
                            } else {
                                let j = i as i32 + step;
                                if j < 0 || j as usize >= list.len() {
                                    return;
                                }
                                list.swap(i, j as usize);
                            }
                            change(list);
                        });
                        chip.append(&b);
                    }
                    // drag a chip onto another chip or another section to move it
                    let drag = gtk4::DragSource::new();
                    drag.set_actions(gtk4::gdk::DragAction::MOVE);
                    let from = format!("{part}:{i}");
                    drag.connect_prepare(move |_, _, _| Some(gtk4::gdk::ContentProvider::for_value(&from.to_value())));
                    chip.add_controller(drag);
                    let drop = gtk4::DropTarget::new(glib::Type::STRING, gtk4::gdk::DragAction::MOVE);
                    let (store, rebuilds3) = (store.clone(), rebuilds3.clone());
                    drop.connect_drop(move |_, value, _, _| {
                        let Ok(from) = value.get::<String>() else {
                            return false;
                        };
                        move_module(&store, &from, part, Some(i));
                        for r in rebuilds3.borrow().iter() {
                            r();
                        }
                        true
                    });
                    chip.add_controller(drop);
                    chips.insert(&chip, -1);
                }
                let used: Vec<String> = ["left", "center", "right"].iter().flat_map(|p| modules(&store, p)).collect();
                let free: Vec<String> = all_modules(&store).into_iter().filter(|n| !used.contains(n)).collect();
                adding.set(true);
                let mut labels = vec!["Add a module".to_string()];
                labels.extend(free.iter().map(|n| label_of(n)));
                add.set_model(Some(&gtk4::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>())));
                add.set_selected(0);
                *options.borrow_mut() = free;
                adding.set(false);
            })
        };
        {
            let (store, change, adding, options) = (s.clone(), change.clone(), adding.clone(), options.clone());
            add.connect_selected_notify(move |d| {
                if adding.get() || d.selected() == 0 {
                    return;
                }
                if let Some(name) = options.borrow().get(d.selected() as usize - 1).cloned() {
                    let mut list = modules(&store, part);
                    list.push(name);
                    change(list);
                }
            });
        }
        rebuilds.borrow_mut().push(rebuild.clone());
        rebuild();
        page.row(title, None, &col);
    }
    page.note("Drag a module to move it within or between sections and the same bar is on every monitor");
    page.section("Cava");
    page.row("Bars", Some("How many bars the audio visualizer draws and it needs cava installed"), &w::number(s, SH, "bar.cava_bars", 1.0, 64.0, 1.0, 0));
    custom_modules(&page, s, &rebuilds);
    page.section("Clock");
    format_row(&page, ctx, "bar.clock_format", "Format");
    page.section("Buttons");
    page.row("Arch logo starts", None, &w::text(s, SH, "bar.terminal", 20));
    page.row("Trays command", Some("Run w audio wifi perf or power when those modules are clicked and sevenshell tray is the built in one"), &w::text(s, SH, "bar.tray", 30));
    page
}

/// one of ur own modules being edited
struct CustomRow {
    card: gtk4::Box,
    name: gtk4::Entry,
    exec: gtk4::Entry,
    interval: gtk4::SpinButton,
    click: gtk4::Entry,
    icon: gtk4::Entry,
    /// its name when last saved so the bar lists can follow a rename
    before: RefCell<String>,
}

/// ur own modules that show what a command prints
fn custom_modules(page: &Page, store: &Rc<super::store::Store>, rebuilds: &Rc<RefCell<Vec<Rc<dyn Fn()>>>>) {
    page.section("Ur own modules");
    page.note("Each shows the last line its command prints and waybar style json w text and tooltip works too then add it above");
    let list = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    let rows: Rc<RefCell<Vec<Rc<CustomRow>>>> = Rc::default();
    let sync = {
        let (store, rows, rebuilds) = (store.clone(), rows.clone(), rebuilds.clone());
        Rc::new(move || {
            let mut customs = Vec::new();
            let mut names = Vec::new();
            let mut renames = Vec::new();
            for r in rows.borrow().iter() {
                let name = r.name.text().trim().to_string();
                let exec = r.exec.text().trim().to_string();
                let before = r.before.replace(name.clone());
                if !before.is_empty() && before != name {
                    renames.push((format!("custom/{before}"), format!("custom/{name}")));
                }
                if name.is_empty() || exec.is_empty() || names.contains(&name) {
                    continue;
                }
                let mut t = toml::Table::new();
                t.insert("name".into(), Value::String(name.clone()));
                t.insert("exec".into(), Value::String(exec));
                t.insert("interval".into(), Value::Integer(r.interval.value() as i64));
                for (key, entry) in [("on_click", &r.click), ("icon", &r.icon)] {
                    let text = entry.text().trim().to_string();
                    if !text.is_empty() {
                        t.insert(key.into(), Value::String(text));
                    }
                }
                names.push(name);
                customs.push(Value::Table(t));
            }
            store.put(SH, "bar.custom", Value::Array(customs));
            // the bar keeps up w renames and forgets modules that are gone
            for part in ["left", "center", "right"] {
                let list: Vec<Value> = modules(&store, part)
                    .into_iter()
                    .map(|m| renames.iter().find(|(from, _)| *from == m).map_or(m, |(_, to)| to.clone()))
                    .filter(|m| m.strip_prefix("custom/").is_none_or(|n| names.iter().any(|x| x == n)))
                    .map(Value::String)
                    .collect();
                store.put(SH, &format!("bar.{part}"), Value::Array(list));
            }
            for r in rebuilds.borrow().iter() {
                r();
            }
        })
    };
    let add_row = {
        let (list, rows, sync) = (list.clone(), rows.clone(), sync.clone());
        Rc::new(move |custom: &toml::Table| {
            let text = |key: &str| custom.get(key).and_then(Value::as_str).unwrap_or("").to_string();
            let entry = |key: &str, hint: &str| {
                let e = gtk4::Entry::new();
                e.set_placeholder_text(Some(hint));
                e.set_text(&text(key));
                e.set_hexpand(true);
                e
            };
            let card = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
            card.add_css_class("card");
            let top = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            let name = entry("name", "name");
            let exec = entry("exec", "command like uptime -p");
            exec.set_width_chars(28);
            let remove = gtk4::Button::new();
            remove.set_child(Some(&crate::style::icon("delete")));
            remove.add_css_class("pill");
            remove.add_css_class("icon-button");
            top.append(&name);
            top.append(&exec);
            top.append(&remove);
            let bottom = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            let interval = gtk4::SpinButton::with_range(0.0, 86400.0, 1.0);
            interval.set_value(custom.get("interval").and_then(Value::as_integer).unwrap_or(5) as f64);
            interval.set_tooltip_text(Some("Seconds between runs and 0 keeps it running"));
            let click = entry("on_click", "on click like kitty htop");
            let icon = entry("icon", "icon like timer");
            icon.set_hexpand(false);
            icon.set_width_chars(12);
            bottom.append(&gtk4::Label::new(Some("every")));
            bottom.append(&interval);
            bottom.append(&gtk4::Label::new(Some("s")));
            bottom.append(&click);
            bottom.append(&icon);
            card.append(&top);
            card.append(&bottom);
            list.append(&card);
            let row = Rc::new(CustomRow { card: card.clone(), name, exec, interval, click, icon, before: RefCell::new(text("name")) });
            for e in [&row.name, &row.exec, &row.click, &row.icon] {
                let sync = sync.clone();
                e.connect_changed(move |_| sync());
            }
            {
                let sync = sync.clone();
                row.interval.connect_value_changed(move |_| sync());
            }
            {
                let (rows, list, sync) = (rows.clone(), list.clone(), sync.clone());
                remove.connect_clicked(move |_| {
                    rows.borrow_mut().retain(|r| r.card != card);
                    list.remove(&card);
                    sync();
                });
            }
            rows.borrow_mut().push(row);
        })
    };
    for custom in store.get(SH, "bar.custom").and_then(|v| v.as_array().cloned()).unwrap_or_default() {
        if let Some(t) = custom.as_table() {
            add_row(t);
        }
    }
    page.append(&list);
    let add = w::button("Add ur own module", "tonal", move || add_row(&toml::Table::new()));
    add.set_halign(gtk4::Align::Start);
    add.set_margin_top(12);
    page.append(&add);
}

pub fn notifications(ctx: &Rc<Ctx>) -> Rc<Page> {
    let page = Page::new("notifications", "Notifications");
    let s = &ctx.store;
    page.section("Where they appear");
    let described = gtk4::Label::new(None);
    described.set_xalign(0.0);
    described.set_wrap(true);
    described.set_max_width_chars(26);
    let values = Placement {
        position: s.str(SH, "notifications.position"),
        margin_x: s.num(SH, "notifications.margin_x") as i64,
        margin_y: s.num(SH, "notifications.margin_y") as i64,
        width: s.num(SH, "notifications.width") as i64,
        height: s.num(SH, "notifications.height") as i64,
    };
    let placer = {
        let (store, described) = (s.clone(), described.clone());
        NotificationPlacer::new(values, s.num(SH, "bar.height").max(20.0), &s.str(WM, "canvas.wallpaper"), move |v| {
            store.put(SH, "notifications.position", Value::String(v.position.clone()));
            store.put(SH, "notifications.margin_x", Value::Integer(v.margin_x));
            store.put(SH, "notifications.margin_y", Value::Integer(v.margin_y));
            store.put(SH, "notifications.width", Value::Integer(v.width));
            store.put(SH, "notifications.height", Value::Integer(v.height));
            let i = POSITIONS.iter().position(|p| *p == v.position).unwrap_or(2);
            let (col, row) = (i % 3, i / 3);
            let mut where_ = Vec::new();
            if row != 1 {
                where_.push(format!("{} px from the {}", v.margin_y, ["top under the bar", "", "bottom"][row]));
            }
            if col != 1 {
                where_.push(format!("{} px from the {}", v.margin_x, ["left", "", "right"][col]));
            }
            match (row, col) {
                (1, 1) => where_.push("centered on the screen".into()),
                (1, _) => where_.push("centered top to bottom".into()),
                (_, 1) => where_.push("centered left to right".into()),
                _ => {}
            }
            let height = if v.height > 0 { v.height.to_string() } else { "fits the text".into() };
            described.set_text(&format!("{} × {}\n{}", v.width, height, where_.join("\n")));
        })
    };
    let side = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    side.set_valign(gtk4::Align::End);
    side.append(&described);
    side.append(&w::button("Height fits the text", "", {
        let p = placer.clone();
        move || p.set_auto_height()
    }));
    side.append(&w::button("Send a test", "tonal", || {
        // wait a sec so the saved config is picked up first
        glib::timeout_add_local_once(std::time::Duration::from_millis(1200), || {
            crate::run_detached(
                std::process::Command::new("notify-send")
                    .args(["-a", "Settings", "Test notification", "Notifications will show up here"]),
            );
        });
    }));
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 16);
    row.append(&placer.area);
    row.append(&side);
    page.wide(&row);
    page.note("Drag the notification where u want it and drag its edges or corners to resize");
    page.section("Popups");
    page.row("Do not disturb", Some("Only critical ones pop up and the rest go to history"), &w::switch(s, SH, "notifications.do_not_disturb"));
    page.row("Stay for", Some("Milliseconds when the app doesnt say"), &w::number(s, SH, "notifications.timeout", 500.0, 120000.0, 500.0, 0));
    page.row("Keep in history", None, &w::number(s, SH, "notifications.history", 0.0, 500.0, 10.0, 0));
    page.section("Volume and brightness popup");
    page.row("Step per key press", Some("Percent"), &w::number(s, SH, "osd.step", 1.0, 25.0, 1.0, 0));
    page.row("Stay for", Some("Milliseconds"), &w::number(s, SH, "osd.duration", 200.0, 10000.0, 100.0, 0));
    page.row("Position", None, &w::choice(s, SH, "osd.position", &["top", "bottom"]));
    page.row("Highest volume", Some("Percent"), &w::number(s, SH, "osd.max_volume", 100.0, 200.0, 5.0, 0));
    page.section("History");
    let history = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    let refresh = {
        let history = history.clone();
        Rc::new(move || {
            while let Some(c) = history.first_child() {
                history.remove(&c);
            }
            let path = std::env::var_os("XDG_RUNTIME_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_default()
                .join("sevenshell/notifications.json");
            let items: Vec<serde_json::Value> = std::fs::read_to_string(path)
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default();
            for n in &items {
                let card = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
                card.add_css_class("card");
                let when = n["time"].as_str().and_then(|t| t.get(11..16)).unwrap_or("");
                let head = gtk4::Label::new(Some(&format!("{}  {when}", n["app"].as_str().unwrap_or(""))));
                head.add_css_class("row-hint");
                head.set_xalign(0.0);
                let summary = gtk4::Label::new(Some(n["summary"].as_str().unwrap_or("")));
                summary.add_css_class("row-title");
                summary.set_xalign(0.0);
                summary.set_wrap(true);
                card.append(&head);
                card.append(&summary);
                if let Some(body) = n["body"].as_str().filter(|b| !b.is_empty()) {
                    let b = gtk4::Label::new(None);
                    // bodies can have markup so fall back to plain text
                    if gtk4::pango::parse_markup(body, '\0').is_ok() {
                        b.set_markup(body);
                    } else {
                        b.set_text(body);
                    }
                    b.set_xalign(0.0);
                    b.set_wrap(true);
                    b.add_css_class("row-hint");
                    card.append(&b);
                }
                history.append(&card);
            }
            if items.is_empty() {
                let empty = gtk4::Label::new(Some("Nothing yet"));
                empty.add_css_class("list-empty");
                empty.set_xalign(0.0);
                let card = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
                card.add_css_class("card");
                card.append(&empty);
                history.append(&card);
            }
        })
    };
    refresh();
    page.row("Past notifications", None, &w::button("Clear", "danger", {
        let refresh = refresh.clone();
        move || {
            let _ = std::process::Command::new("sevenshell").args(["notifications", "clear"]).status();
            let refresh = refresh.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || refresh());
        }
    }));
    page.append(&history);
    page
}

/// an idle timeout w an on switch and minutes
fn minutes(page: &Page, ctx: &Rc<Ctx>, key: &str, title: &str, default: f64) {
    let path = format!("idle.{key}");
    let seconds = ctx.store.num(WM, &path);
    let on = gtk4::Switch::new();
    on.set_active(seconds > 0.0);
    let spin = gtk4::SpinButton::with_range(1.0, 600.0, 1.0);
    spin.set_value(if seconds > 0.0 { seconds / 60.0 } else { default });
    spin.set_sensitive(seconds > 0.0);
    let store_it = {
        let (store, on, spin) = (ctx.store.clone(), on.clone(), spin.clone());
        Rc::new(move || {
            spin.set_sensitive(on.is_active());
            let v = if on.is_active() { (spin.value() * 60.0).round() as i64 } else { 0 };
            store.put(WM, &path, Value::Integer(v));
        })
    };
    {
        let f = store_it.clone();
        on.connect_active_notify(move |_| f());
    }
    spin.connect_value_changed(move |_| store_it());
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
    row.append(&spin);
    let unit = gtk4::Label::new(Some("min"));
    unit.add_css_class("row-hint");
    row.append(&unit);
    row.append(&on);
    page.row(title, None, &row);
}

pub fn lock(ctx: &Rc<Ctx>) -> Rc<Page> {
    let page = Page::new("lock", "Lock & idle");
    let s = &ctx.store;
    page.section("Lock screen");
    image_row(&page, ctx, SH, "lock.wallpaper", "Wallpaper", Some("Behind the clock and the password box and empty is black"));
    image_row(&page, ctx, SH, "lock.avatar", "Profile picture", Some("Empty uses ~/.face or ur sddm picture"));
    format_row(&page, ctx, "lock.clock_format", "Time format");
    format_row(&page, ctx, "lock.date_format", "Date format");
    page.row("Hint", Some("Shown at the bottom left till u press a key"), &w::text(s, SH, "lock.message", 20));
    page.row("Lock when the computer sleeps", None, &w::switch(s, SH, "lock.lock_before_sleep"));
    page.row("Try it", None, &w::button("Lock now", "tonal", || {
        crate::run_detached(std::process::Command::new("sevenshell").arg("lock"));
    }));
    page.section("When ur away");
    minutes(&page, ctx, "lock_after", "Lock the screen after", 5.0);
    minutes(&page, ctx, "screen_off_after", "Turn the screen off after", 5.0);
    minutes(&page, ctx, "suspend_after", "Sleep after", 10.0);
    page.note("Counted from ur last key press or mouse move and video playing holds them all off");
    page
}

/// night light anti flashbang and the smaller shell helpers
pub fn extras(ctx: &Rc<Ctx>) -> Rc<Page> {
    let page = Page::new("widgets", "Extras");
    let s = &ctx.store;
    page.section("Night light");
    page.row("Night light", Some("Warmer colors that are easier on the eyes"), &w::switch(s, WM, "night_light.enabled"));
    page.row("Warmth", Some("Kelvin where 6500 is normal and lower is warmer"), &w::number(s, WM, "night_light.temperature", 1000.0, 6500.0, 100.0, 0));
    page.row("From", Some("hh:mm like 20:00 and empty here and below means always"), &w::text(s, WM, "night_light.from", 8));
    page.row("Until", Some("hh:mm like 07:00"), &w::text(s, WM, "night_light.until", 8));
    page.note("Night light changes the monitor itself so it only works in a real session not nested");
    page.section("Anti-flashbang");
    page.row("Anti-flashbang", Some("Dims the brightest parts of windows so a white page cant blind u"), &w::switch(s, WM, "anti_flashbang.enabled"));
    page.row("Brightest allowed", Some("0.05 to 1 where lower is dimmer"), &w::number(s, WM, "anti_flashbang.max_brightness", 0.05, 1.0, 0.05, 2));
    page.section("Launcher");
    page.row("Calculator", Some("Math like 2*8 or =sqrt(2) shows the answer and Enter copies it"), &w::switch(s, SH, "launcher.calculator"));
    page.row("Web search", Some("The last row searches here w %s as what u typed and empty turns it off"), &w::text(s, SH, "launcher.search_url", 30));
    page.note("Start with > to run a command like >notify-send hi");
    page.section("Clipboard");
    page.row("Keep clipboard history", Some("mod+v brings back things u copied and it needs cliphist"), &w::switch(s, SH, "clipboard.history"));
    page.row("How many to keep", None, &w::number(s, SH, "clipboard.max_items", 1.0, 10000.0, 50.0, 0));
    page.row("Forget everything", None, &w::button("Clear history", "pill", crate::clipboard::wipe));
    page.section("Weather");
    page.row("Location", Some("A city like Toronto or empty to guess from ur internet address"), &w::text(s, SH, "weather.location", 20));
    page.row("Units", None, &w::choice_labeled(s, SH, "weather.units", &[("metric", "°C and km/h"), ("imperial", "°F and mph")]));
    page.section("Updates");
    page.row("Update command", Some("What clicking the updates module runs"), &w::text(s, SH, "updates.command", 30));
    page.row("Check every", Some("Minutes"), &w::number(s, SH, "updates.interval_minutes", 1.0, 1440.0, 5.0, 0));
    page.section("Wallpapers");
    page.row("Folder", Some("Pictures here show up in quick settings and each one remembers its colors"), &w::text(s, SH, "wallpapers.dir", 30));
    page.section("Admin rights");
    page.row("Ask for my password", Some("A prompt when an app needs admin rights and it takes effect after a shell restart"), &w::switch(s, SH, "polkit.agent"));
    page
}
