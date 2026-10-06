//! quick settings from the clock w a calendar toggles shortcuts and ur wallpapers plus the media and weather popups

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{gdk, glib};

use crate::toggles::{self, Toggle};
use crate::tray::{close, hbox, icon, label, vbox};

pub const CSS: &str = "
.sevenshell-tray .time { font-family: \"Rubik\"; font-size: 40px; font-weight: 500; }
.sevenshell-tray .date { font-size: 14px; color: @m3onSurfaceVariant; margin-bottom: 4px; }
.sevenshell-tray calendar {
    background: @m3surfaceContainer; border: none; border-radius: 20px; padding: 6px; color: @m3onSurface;
}
.sevenshell-tray calendar > header { border: none; background: transparent; }
.sevenshell-tray calendar > header > button { border-radius: 9999px; padding: 2px 8px; }
.sevenshell-tray calendar > grid > label.day-name { color: @m3onSurfaceVariant; font-size: 11px; }
.sevenshell-tray calendar > grid > label.day-number { border-radius: 9999px; padding: 4px; }
.sevenshell-tray calendar > grid > label.day-number.other-month { color: alpha(@m3onSurface, 0.35); }
.sevenshell-tray calendar > grid > label.today { background: @m3primary; color: @m3onPrimary; }
.sevenshell-tray calendar > grid > label:selected { background: @m3secondaryContainer; color: @m3onSecondaryContainer; }
.sevenshell-tray button.quick-toggle {
    background: @m3surfaceContainerHigh; border-radius: 9999px; padding: 10px 14px;
}
.sevenshell-tray button.quick-toggle.active { background: @m3primary; }
.sevenshell-tray button.quick-toggle.active label { color: @m3onPrimary; }
.sevenshell-tray button.shortcut {
    background: @m3surfaceContainerHigh; border-radius: 9999px; padding: 10px;
    min-width: 24px;
}
.sevenshell-tray button.shortcut:hover { background: @m3secondaryContainer; }
.sevenshell-tray button.thumb { padding: 3px; border-radius: 14px; background: transparent; }
.sevenshell-tray button.thumb.current { background: @m3primary; }
.sevenshell-tray .thumb picture { border-radius: 11px; }
.sevenshell-tray .art { border-radius: 16px; background: @m3surfaceContainerHigh; }
.sevenshell-tray .song { font-size: 17px; font-weight: 500; }
.sevenshell-tray button.play { background: @m3primary; border-radius: 9999px; padding: 10px; }
.sevenshell-tray button.play label { color: @m3onPrimary; }
.sevenshell-tray .weather-now { font-size: 44px; font-weight: 400; }
.sevenshell-tray .weather-big { font-size: 48px; color: @m3primary; }
.sevenshell-tray .day { background: @m3surfaceContainer; border-radius: 16px; padding: 8px 12px; }
";

/// start a shell command after the tray closes
fn run_after_close(command: &str) {
    close();
    crate::run_detached(std::process::Command::new("sh").args(["-c", command]));
}

/// the quick settings panel
pub fn build() -> gtk4::Box {
    let root = vbox(10);
    root.add_css_class("quick");
    let now = glib::DateTime::now_local().ok();
    let fmt = |f: &str| now.as_ref().and_then(|n| n.format(f).ok()).map(|s| s.to_string()).unwrap_or_default();
    let time = label(&fmt("%H:%M"), &["time"], 0.0);
    let date = label(&fmt("%A, %B %-d"), &["date"], 0.0);
    root.append(&time);
    root.append(&date);
    let calendar = gtk4::Calendar::new();
    root.append(&calendar);

    // two toggles a row w the ones that are on filled in
    let grid = gtk4::Grid::new();
    grid.set_row_spacing(6);
    grid.set_column_spacing(6);
    grid.set_column_homogeneous(true);
    for (i, toggle) in toggles::ALL.iter().copied().enumerate() {
        let button = toggle_button(toggle);
        grid.attach(&button, (i % 2) as i32, (i / 2) as i32, 1, 1);
    }
    root.append(&grid);

    let shortcuts = hbox(6);
    shortcuts.set_homogeneous(true);
    for (glyph, tip, command) in [
        ("content_paste", "Clipboard history", "sevenshell clipboard"),
        ("mood", "Emoji", "sevenshell emoji"),
        ("screenshot_region", "Screenshot", "sevenshell screenshot"),
        ("lock", "Lock", "sevenshell lock"),
        ("power_settings_new", "Log out, restart or shut down", "sevenshell session"),
        ("settings", "Settings", "sevenshell settings"),
    ] {
        let b = gtk4::Button::new();
        b.set_child(Some(&icon(glyph, &[])));
        b.add_css_class("shortcut");
        b.set_tooltip_text(Some(tip));
        b.connect_clicked(move |_| run_after_close(command));
        shortcuts.append(&b);
    }
    root.append(&shortcuts);
    root.append(&wallpapers());
    root
}

fn toggle_button(toggle: Toggle) -> gtk4::Button {
    let button = gtk4::Button::new();
    button.add_css_class("quick-toggle");
    let row = hbox(8);
    row.append(&icon(toggle.icon(), &[]));
    let name = label(toggle.name(), &[], 0.0);
    name.set_hexpand(true);
    row.append(&name);
    button.set_child(Some(&row));
    button.set_tooltip_text(Some(toggle.hint()));
    let on = Rc::new(Cell::new(toggle.is_on()));
    let paint = |button: &gtk4::Button, on: bool| {
        if on {
            button.add_css_class("active");
        } else {
            button.remove_css_class("active");
        }
    };
    paint(&button, on.get());
    button.connect_clicked(move |button| {
        toggles::flip(toggle);
        // shown right away bc the config takes a sec to come back around
        on.set(!on.get());
        paint(button, on.get());
    });
    button
}

/// the pictures in the wallpaper folder as small clickable thumbs
fn wallpapers() -> gtk4::Box {
    let section = vbox(6);
    let folder = gtk4::Button::new();
    folder.set_child(Some(&icon("folder_open", &["dim"])));
    folder.set_tooltip_text(Some("Open the wallpaper folder"));
    folder.connect_clicked(|_| {
        let dir = crate::wallpapers::dir();
        let _ = std::fs::create_dir_all(&dir);
        close();
        crate::run_detached(std::process::Command::new("xdg-open").arg(dir));
    });
    let head = hbox(8);
    let title = label("Wallpapers", &["section"], 0.0);
    title.set_hexpand(true);
    head.append(&title);
    head.append(&folder);
    section.append(&head);
    let list = crate::wallpapers::list();
    if list.is_empty() {
        let dir = crate::config::get().wallpapers.dir.clone();
        let hint = label(&format!("Put pictures in {dir} and they show up here"), &["dim"], 0.0);
        hint.set_wrap(true);
        section.append(&hint);
        return section;
    }
    let grid = gtk4::FlowBox::new();
    grid.set_selection_mode(gtk4::SelectionMode::None);
    grid.set_max_children_per_line(3);
    grid.set_min_children_per_line(3);
    grid.set_row_spacing(4);
    grid.set_column_spacing(4);
    grid.set_homogeneous(true);
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_hscrollbar_policy(gtk4::PolicyType::Never);
    scroll.set_propagate_natural_height(true);
    scroll.set_max_content_height(230);
    scroll.set_child(Some(&grid));
    section.append(&scroll);
    let current = crate::wallpapers::current();
    let buttons: Rc<RefCell<Vec<(gtk4::Button, PathBuf)>>> = Rc::default();
    let mut pending = Vec::new();
    for path in list {
        let picture = gtk4::Picture::new();
        picture.set_content_fit(gtk4::ContentFit::Cover);
        picture.set_size_request(104, 60);
        let button = gtk4::Button::new();
        button.add_css_class("thumb");
        button.set_child(Some(&picture));
        button.set_tooltip_text(path.file_name().and_then(|n| n.to_str()));
        if path == current {
            button.add_css_class("current");
        }
        let (all, p) = (Rc::downgrade(&buttons), path.clone());
        button.connect_clicked(move |_| {
            if let Err(err) = crate::wallpapers::apply(&p) {
                crate::run_detached(std::process::Command::new("notify-send").args(["-a", "sevenshell", "Wallpaper not changed", &err]));
                return;
            }
            if let Some(all) = all.upgrade() {
                for (b, bp) in all.borrow().iter() {
                    if *bp == p {
                        b.add_css_class("current");
                    } else {
                        b.remove_css_class("current");
                    }
                }
            }
        });
        grid.insert(&button, -1);
        buttons.borrow_mut().push((button, path.clone()));
        pending.push((picture, path));
    }
    // one thumb per idle turn so the panel opens right away and fills in
    let pending = Rc::new(RefCell::new(pending.into_iter()));
    let keep = buttons.clone();
    glib::idle_add_local(move || {
        let _ = &keep;
        let Some((picture, path)) = pending.borrow_mut().next() else {
            return glib::ControlFlow::Break;
        };
        if let Some(texture) = thumbnail(&path) {
            picture.set_paintable(Some(&texture));
        }
        glib::ControlFlow::Continue
    });
    section
}

/// a small copy of the picture made once and kept in the cache
fn thumbnail(path: &Path) -> Option<gdk::Texture> {
    use std::hash::{Hash, Hasher};
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (path, mtime).hash(&mut h);
    let dir = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?
        .join("sevenshell/thumbs");
    let cached = dir.join(format!("{:x}.png", h.finish()));
    if let Ok(texture) = gdk::Texture::from_filename(&cached) {
        return Some(texture);
    }
    let pixbuf = gtk4::gdk_pixbuf::Pixbuf::from_file_at_scale(path, 320, -1, true).ok()?;
    let _ = std::fs::create_dir_all(&dir);
    let _ = pixbuf.savev(&cached, "png", &[]);
    #[allow(deprecated)]
    Some(gdk::Texture::for_pixbuf(&pixbuf))
}

/// the media popup w the cover the song and controls and it keeps up while its open
pub fn media() -> gtk4::Box {
    let root = vbox(10);
    let top = hbox(14);
    let art = gtk4::Picture::new();
    art.set_size_request(96, 96);
    art.set_content_fit(gtk4::ContentFit::Cover);
    art.add_css_class("art");
    top.append(&art);
    let words = vbox(2);
    words.set_valign(gtk4::Align::Center);
    let title = label("Nothing playing", &["song"], 0.0);
    title.set_wrap(true);
    title.set_lines(2);
    let artist = label("", &[], 0.0);
    let album = label("", &["dim"], 0.0);
    let player = label("", &["dim"], 0.0);
    for w in [&title, &artist, &album, &player] {
        w.set_max_width_chars(24);
        words.append(w);
    }
    top.append(&words);
    root.append(&top);
    let scale = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, 1.0, 1.0);
    scale.set_draw_value(false);
    root.append(&scale);
    let times = hbox(0);
    let at = label("0:00", &["dim"], 0.0);
    at.set_hexpand(true);
    let length = label("0:00", &["dim"], 1.0);
    times.append(&at);
    times.append(&length);
    root.append(&times);
    let controls = hbox(16);
    controls.set_halign(gtk4::Align::Center);
    let play_icon = icon("play_arrow", &[]);
    for (glyph, cmd) in [("skip_previous", "previous"), ("play_arrow", "play-pause"), ("skip_next", "next")] {
        let b = gtk4::Button::new();
        if cmd == "play-pause" {
            b.set_child(Some(&play_icon));
            b.add_css_class("play");
        } else {
            b.set_child(Some(&icon(glyph, &[])));
        }
        b.connect_clicked(move |_| {
            let _ = std::process::Command::new("playerctl").arg(cmd).status();
        });
        controls.append(&b);
    }
    root.append(&controls);

    // dragging the slider seeks and a set from the timer shouldnt
    let quiet = Rc::new(Cell::new(false));
    {
        let quiet = quiet.clone();
        scale.connect_value_changed(move |s| {
            if !quiet.get() {
                let _ = std::process::Command::new("playerctl").args(["position", &format!("{:.1}", s.value())]).status();
            }
        });
    }
    let shown_art: Rc<RefCell<String>> = Rc::default();
    let refresh = {
        let root = root.downgrade();
        move || {
            let Some(_) = root.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let (title, artist, album, player, art, scale, at, length, play_icon, quiet, shown_art) = (
                title.clone(), artist.clone(), album.clone(), player.clone(), art.clone(), scale.clone(),
                at.clone(), length.clone(), play_icon.clone(), quiet.clone(), shown_art.clone(),
            );
            crate::wake::off_thread(
                || {
                    let track = crate::media::now();
                    let art = track.as_ref().and_then(|t| t.art_file());
                    (track, crate::media::position(), art)
                },
                move |(track, position, file)| {
                    let Some(track) = track else {
                        title.set_text("Nothing playing");
                        for w in [&artist, &album, &player] {
                            w.set_text("");
                        }
                        return;
                    };
                    title.set_text(&track.title);
                    artist.set_text(&track.artist);
                    album.set_text(&track.album);
                    player.set_text(&track.player);
                    play_icon.set_text(if track.playing { "pause" } else { "play_arrow" });
                    if *shown_art.borrow() != track.art {
                        *shown_art.borrow_mut() = track.art.clone();
                        art.set_filename(file.as_ref());
                    }
                    quiet.set(true);
                    scale.set_range(0.0, track.length.max(1.0));
                    scale.set_value(position.unwrap_or(0.0));
                    quiet.set(false);
                    at.set_text(&crate::media::clock(position.unwrap_or(0.0)));
                    length.set_text(&crate::media::clock(track.length));
                },
            );
            glib::ControlFlow::Continue
        }
    };
    let first = refresh.clone();
    first();
    glib::timeout_add_seconds_local(1, refresh);
    root
}

/// the weather popup from the last report the bar got
pub fn weather() -> gtk4::Box {
    let root = vbox(10);
    let Some(report) = crate::weather::feed().latest().and_then(|l| crate::weather::Report::parse(&l)) else {
        root.append(&label("No weather yet — it needs the internet", &["dim"], 0.0));
        return root;
    };
    root.append(&label(&report.place, &["title"], 0.0));
    let now = hbox(14);
    now.append(&icon(report.now.icon, &["weather-big"]));
    let temp = vbox(0);
    temp.set_valign(gtk4::Align::Center);
    temp.append(&label(&report.now.temp, &["weather-now"], 0.0));
    temp.append(&label(&report.now.description, &["dim"], 0.0));
    now.append(&temp);
    root.append(&now);
    let facts = hbox(12);
    for (glyph, text) in [
        ("thermostat", format!("Feels {}", report.now.feels)),
        ("humidity_percentage", report.now.humidity.clone()),
        ("air", report.now.wind.clone()),
    ] {
        let fact = hbox(4);
        fact.append(&icon(glyph, &["dim"]));
        fact.append(&label(&text, &["dim"], 0.0));
        facts.append(&fact);
    }
    root.append(&facts);
    for day in &report.days {
        let row = hbox(10);
        row.add_css_class("day");
        row.append(&icon(day.icon, &[]));
        let name = label(&day.name, &[], 0.0);
        name.set_hexpand(true);
        name.set_tooltip_text(Some(&day.description));
        row.append(&name);
        row.append(&label(&format!("{} / {}", day.high, day.low), &["dim"], 1.0));
        root.append(&row);
    }
    root
}
