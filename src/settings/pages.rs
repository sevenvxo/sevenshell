//! the sevenwm pages like style general tiling input keybinds animations monitors and rules

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use toml::{Table, Value};

use super::preview::{AnimationPreview, CURVES};
use super::store::{Store, Which};
use super::widgets::{self as w, Page};
use super::Ctx;

const WM: Which = Which::Wm;
const SH: Which = Which::Shell;

/// the scheme variants w names people get
const VARIANTS: [(&str, &str); 9] = [
    ("content", "Wallpaper colors"),
    ("tonal-spot", "Tonal spot"),
    ("vibrant", "Vibrant"),
    ("expressive", "Expressive"),
    ("fidelity", "Fidelity"),
    ("neutral", "Neutral"),
    ("monochrome", "Monochrome"),
    ("rainbow", "Rainbow"),
    ("fruit-salad", "Fruit salad"),
];

/// the theme settings as they are in the window right now even before theyre saved
fn theme_now(store: &Store) -> crate::config::Theme {
    let s = |k: &str| store.str(SH, &format!("theme.{k}"));
    crate::config::Theme {
        source: s("source"),
        wallpaper: s("wallpaper"),
        variant: s("variant"),
        mode: s("mode"),
        primary: s("primary"),
        secondary: s("secondary"),
        tertiary: s("tertiary"),
        color_sevenwm: store.bool(SH, "theme.color_sevenwm"),
        color_gtk: store.bool(SH, "theme.color_gtk"),
        colors: store
            .get(SH, "theme.colors")
            .and_then(|v| v.as_table().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(k, v)| Some((k, v.as_str()?.to_string())))
            .collect(),
    }
}

/// a row of the main palette colors so u see what ur picks make
fn swatches(store: &Rc<Store>) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
    row.set_homogeneous(true);
    let names = [
        ("m3primary", "Primary"),
        ("m3secondary", "Secondary"),
        ("m3tertiary", "Tertiary"),
        ("m3primaryContainer", "Container"),
        ("m3surface", "Surface"),
        ("m3surfaceContainerHigh", "Raised"),
        ("m3onSurface", "Text"),
        ("m3error", "Error"),
    ];
    let areas: Vec<(gtk4::DrawingArea, &str)> = names
        .iter()
        .map(|(name, label)| {
            let cell = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
            let area = gtk4::DrawingArea::new();
            area.set_content_width(48);
            area.set_content_height(48);
            area.set_halign(gtk4::Align::Center);
            let text = gtk4::Label::new(Some(label));
            text.add_css_class("swatch-name");
            cell.append(&area);
            cell.append(&text);
            row.append(&cell);
            (area, *name)
        })
        .collect();
    let palette = Rc::new(RefCell::new(crate::theme::generate(&theme_now(store))));
    for (area, name) in &areas {
        let (palette, name) = (palette.clone(), name.to_string());
        area.set_draw_func(move |_, cr, w, h| {
            let (r, g, b, a) = crate::theme::rgba(palette.borrow().get(&name), 1.0);
            cr.set_source_rgba(r, g, b, a);
            let d = w.min(h) as f64;
            cr.arc(w as f64 / 2.0, h as f64 / 2.0, d / 2.0, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
            let (r, g, b, _) = crate::theme::rgba(crate::theme::current().get("m3outlineVariant"), 1.0);
            cr.set_source_rgba(r, g, b, 1.0);
            cr.set_line_width(1.0);
            cr.arc(w as f64 / 2.0, h as f64 / 2.0, d / 2.0 - 0.5, 0.0, std::f64::consts::TAU);
            let _ = cr.stroke();
        });
    }
    // redo the preview whenever a setting changes
    let weak = Rc::downgrade(store);
    store.on_change(move || {
        if let Some(store) = weak.upgrade() {
            *palette.borrow_mut() = crate::theme::generate(&theme_now(&store));
            for (area, _) in &areas {
                area.queue_draw();
            }
        }
    });
    row
}

/// pick ur own color for an accent or leave it to the palette
fn accent_row(page: &Page, store: &Rc<Store>, key: &str, title: &str) {
    let path = format!("theme.{key}");
    let current = store.str(SH, &path);
    let on = gtk4::Switch::new();
    on.set_active(!current.is_empty());
    let dialog = gtk4::ColorDialog::new();
    dialog.set_with_alpha(false);
    let button = gtk4::ColorDialogButton::new(Some(dialog));
    let seed = if current.is_empty() {
        crate::theme::hex(crate::theme::generate(&theme_now(store)).get(&format!("m3{key}")))
    } else {
        current.clone()
    };
    button.set_rgba(&w::parse_color(&seed));
    button.set_sensitive(!current.is_empty());
    let controls = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    controls.append(&button);
    controls.append(&on);
    {
        let (store, path, button) = (store.clone(), path.clone(), button.clone());
        on.connect_active_notify(move |on| {
            button.set_sensitive(on.is_active());
            let v = if on.is_active() { w::color_hex(&button.rgba(), false) } else { String::new() };
            store.put(SH, &path, Value::String(v));
        });
    }
    {
        let (store, on) = (store.clone(), on.clone());
        button.connect_rgba_notify(move |b| {
            if on.is_active() {
                store.put(SH, &path, Value::String(w::color_hex(&b.rgba(), false)));
            }
        });
    }
    page.row(title, Some("Off lets the palette pick it"), &controls);
}

/// set one palette role to an exact color or leave it to the palette
fn override_row(page: &Page, store: &Rc<Store>, role: &'static str, title: &str) {
    let current = store.get(SH, "theme.colors").and_then(|v| v.get(role)?.as_str().map(String::from));
    let on = gtk4::Switch::new();
    on.set_active(current.is_some());
    let dialog = gtk4::ColorDialog::new();
    dialog.set_with_alpha(false);
    let button = gtk4::ColorDialogButton::new(Some(dialog));
    let seed = current.clone().unwrap_or_else(|| crate::theme::hex(crate::theme::generate(&theme_now(store)).get(role)));
    button.set_rgba(&w::parse_color(&seed));
    button.set_sensitive(current.is_some());
    let controls = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    controls.append(&button);
    controls.append(&on);
    let set = {
        let store = store.clone();
        move |value: Option<String>| {
            let mut colors = store.get(SH, "theme.colors").and_then(|v| v.as_table().cloned()).unwrap_or_default();
            match value {
                Some(v) => colors.insert(role.into(), Value::String(v)),
                None => colors.remove(role),
            };
            store.put(SH, "theme.colors", Value::Table(colors));
        }
    };
    {
        let (set, button) = (set.clone(), button.clone());
        on.connect_active_notify(move |on| {
            button.set_sensitive(on.is_active());
            set(on.is_active().then(|| w::color_hex(&button.rgba(), false)));
        });
    }
    {
        let on = on.clone();
        button.connect_rgba_notify(move |b| {
            if on.is_active() {
                set(Some(w::color_hex(&b.rgba(), false)));
            }
        });
    }
    page.row(title, Some(role), &controls);
}

/// an image setting w a preview and pick and none buttons
pub fn image_row(page: &Page, ctx: &Rc<Ctx>, which: Which, path: &str, title: &str, hint: Option<&str>) {
    let store = &ctx.store;
    let current = store.str(which, path);
    let pic = w::preview_image(&current, 200, 112);
    let pick = gtk4::Button::with_label("Pick");
    pick.add_css_class("pill");
    pick.add_css_class("tonal");
    let none = gtk4::Button::with_label("None");
    none.add_css_class("pill");
    let buttons = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    buttons.append(&pick);
    buttons.append(&none);
    let controls = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    controls.append(&pic);
    controls.append(&buttons);
    {
        let (store, path, pic, window) = (store.clone(), path.to_string(), pic.clone(), ctx.window.clone());
        pick.connect_clicked(move |_| {
            let (store, path, pic) = (store.clone(), path.clone(), pic.clone());
            w::pick_image(window.upcast_ref(), move |p| {
                w::set_preview(&pic, &p);
                store.put(which, &path, Value::String(p));
            });
        });
    }
    {
        let (store, path, pic) = (store.clone(), path.to_string(), pic.clone());
        none.connect_clicked(move |_| {
            w::set_preview(&pic, "");
            store.put(which, &path, Value::String(String::new()));
        });
    }
    page.row(title, hint, &controls);
}

/// color rows that only matter when sevenwm isnt following the palette
fn wm_color(page: &Page, store: &Rc<Store>, path: &str, title: &str, alpha: bool, followers: &Rc<RefCell<Vec<gtk4::Widget>>>) {
    let b = w::color(store, WM, path, alpha);
    b.set_sensitive(!store.bool(SH, "theme.color_sevenwm"));
    followers.borrow_mut().push(b.clone().upcast());
    page.row(title, None, &b);
}

/// a mod key dropdown that trades keys w the other one when u pick the key its using
fn modifier_choice(store: &Rc<Store>, path: &'static str, other: &'static str) -> gtk4::DropDown {
    const KEYS: [&str; 3] = ["ctrl", "alt", "shift"];
    let drop = w::choice(store, WM, path, &KEYS);
    let before = Rc::new(RefCell::new(store.str(WM, path)));
    {
        let (store, before) = (store.clone(), before.clone());
        drop.connect_selected_notify(move |_| {
            let now = store.str(WM, path);
            let old = before.replace(now.clone());
            if now == store.str(WM, other) && !old.is_empty() && old != now {
                store.put(WM, other, Value::String(old));
            }
        });
    }
    // the other dropdown can change this one so keep up w the store
    let weak = (Rc::downgrade(store), drop.downgrade());
    store.on_change(move || {
        let (Some(store), Some(drop)) = (weak.0.upgrade(), weak.1.upgrade()) else {
            return;
        };
        let now = store.str(WM, path);
        if let Some(i) = KEYS.iter().position(|k| *k == now)
            && drop.selected() != i as u32
        {
            before.replace(now);
            drop.set_selected(i as u32);
        }
    });
    drop
}

pub fn style(ctx: &Rc<Ctx>) -> Rc<Page> {
    let page = Page::new("palette", "Wallpaper & style");
    let s = &ctx.store;
    page.section("Wallpaper");
    image_row(&page, ctx, WM, "canvas.wallpaper", "Desktop wallpaper", Some("Behind everything on every monitor"));
    page.row("Fit", None, &w::choice(s, WM, "canvas.wallpaper_mode", &["fill", "fit", "center", "stretch", "tile"]));

    page.section("Colors");
    page.wide(&swatches(s));
    // where the colors come from is the wallpaper or one color u pick
    let from_wallpaper = s.str(SH, "theme.source") == "wallpaper";
    let seg = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    seg.add_css_class("segmented");
    let wall_btn = gtk4::Button::with_label("Wallpaper");
    let pick_btn = gtk4::Button::with_label("My color");
    for b in [&wall_btn, &pick_btn] {
        b.add_css_class("pill");
        seg.append(b);
    }
    let seed_dialog = gtk4::ColorDialog::new();
    seed_dialog.set_with_alpha(false);
    let seed = gtk4::ColorDialogButton::new(Some(seed_dialog));
    let palette_now = crate::theme::generate(&theme_now(s));
    seed.set_rgba(&w::parse_color(&crate::theme::hex(palette_now.seed)));
    seed.set_sensitive(!from_wallpaper);
    let set_mode = {
        let (wall_btn, pick_btn, seed) = (wall_btn.clone(), pick_btn.clone(), seed.clone());
        move |wallpaper: bool| {
            if wallpaper {
                wall_btn.add_css_class("active");
                pick_btn.remove_css_class("active");
            } else {
                pick_btn.add_css_class("active");
                wall_btn.remove_css_class("active");
            }
            seed.set_sensitive(!wallpaper);
        }
    };
    set_mode(from_wallpaper);
    {
        let (store, set_mode) = (s.clone(), set_mode.clone());
        wall_btn.connect_clicked(move |_| {
            set_mode(true);
            store.put(SH, "theme.source", Value::String("wallpaper".into()));
        });
    }
    {
        let (store, set_mode, seed) = (s.clone(), set_mode.clone(), seed.clone());
        pick_btn.connect_clicked(move |_| {
            set_mode(false);
            store.put(SH, "theme.source", Value::String(w::color_hex(&seed.rgba(), false)));
        });
    }
    {
        let (store, pick_btn) = (s.clone(), pick_btn.clone());
        seed.connect_rgba_notify(move |b| {
            if pick_btn.has_css_class("active") {
                store.put(SH, "theme.source", Value::String(w::color_hex(&b.rgba(), false)));
            }
        });
    }
    page.row("Colors from", Some("The wallpapers most common color leads and its second most common becomes the secondary"), &seg);
    page.row("My color", Some("The color the palette is built around"), &seed);
    page.row("Scheme", None, &w::choice_labeled(s, SH, "theme.variant", &VARIANTS));
    page.row("Mode", None, &w::choice_labeled(s, SH, "theme.mode", &[("dark", "Dark"), ("light", "Light")]));
    accent_row(&page, s, "primary", "Primary");
    accent_row(&page, s, "secondary", "Secondary");
    accent_row(&page, s, "tertiary", "Tertiary");
    let follow = w::switch(s, SH, "theme.color_sevenwm");
    page.row("Color sevenwm too", Some("Window borders menus and title bars follow the palette"), &follow);
    page.row("Color gtk top bars", Some("Gtk apps own top bars match the window top bar color"), &w::switch(s, SH, "theme.color_gtk"));

    let followers: Rc<RefCell<Vec<gtk4::Widget>>> = Rc::default();
    page.section("Windows");
    page.row("Corner radius", Some("0 for square corners"), &w::number(s, WM, "decorations.corner_radius", 0.0, 100.0, 1.0, 0));
    page.row("Border width", Some("0 for none"), &w::number(s, WM, "border.width", 0.0, 50.0, 1.0, 0));
    wm_color(&page, s, "border.focused", "Focused border", true, &followers);
    wm_color(&page, s, "border.unfocused", "Unfocused border", true, &followers);
    page.row("Shadows", None, &w::switch(s, WM, "decorations.shadow"));
    page.row("Shadows on tiles too", None, &w::switch(s, WM, "decorations.shadow_on_tiles"));
    page.row("Shadow size", None, &w::number(s, WM, "decorations.shadow_size", 0.0, 200.0, 1.0, 0));
    page.row("Shadow offset", Some("x and y"), &w::pair(s, WM, "decorations.shadow_offset", -200.0, 200.0, 1.0));
    page.row("Shadow color", None, &w::color(s, WM, "decorations.shadow_color", true));

    page.section("Title bars");
    page.row("sevenwms own title bars", Some("For apps that let sevenwm draw them which is most but not gtk ones"), &w::switch(s, WM, "decorations.titlebar"));
    page.row("Height", None, &w::number(s, WM, "decorations.titlebar_height", 12.0, 64.0, 1.0, 0));
    wm_color(&page, s, "decorations.titlebar_focused", "Focused", false, &followers);
    wm_color(&page, s, "decorations.titlebar_unfocused", "Unfocused", false, &followers);
    wm_color(&page, s, "decorations.titlebar_text", "Text and buttons", false, &followers);

    page.section("Canvas and menu colors");
    wm_color(&page, s, "canvas.background", "Canvas background", false, &followers);
    wm_color(&page, s, "canvas.region_outline", "Workspace outline", true, &followers);
    wm_color(&page, s, "canvas.bounds_outline", "Canvas edge", true, &followers);
    wm_color(&page, s, "canvas.drop_highlight", "Drop highlight", true, &followers);
    wm_color(&page, s, "theme.menu_background", "Menu background", false, &followers);
    wm_color(&page, s, "theme.menu_text", "Menu text", false, &followers);
    wm_color(&page, s, "theme.menu_hover", "Menu highlight", false, &followers);
    wm_color(&page, s, "theme.menu_disabled", "Menu disabled text", false, &followers);
    wm_color(&page, s, "theme.menu_edge", "Menu edge", false, &followers);
    page.note("These colors are used when Color sevenwm too is off");
    {
        let followers = followers.clone();
        follow.connect_active_notify(move |f| {
            for w in followers.borrow().iter() {
                w.set_sensitive(!f.is_active());
            }
        });
    }

    page.section("Cursor");
    let mut themes = cursor_themes();
    let current = s.str(WM, "cursor.theme");
    if !current.is_empty() && !themes.contains(&current) {
        themes.push(current);
    }
    let theme_refs: Vec<&str> = themes.iter().map(String::as_str).collect();
    page.row("Theme", Some("default is the systems"), &w::choice(s, WM, "cursor.theme", &theme_refs));
    page.row("Size", Some("Pixels and any size works"), &w::number(s, WM, "cursor.size", 8.0, 256.0, 1.0, 0));
    page.section("Every color");
    page.note("Turn one on to set it exactly and it wins over the palette and the rest still follow it");
    for (role, title) in crate::theme::ROLES {
        override_row(&page, s, role, title);
    }
    page
}

/// installed cursor themes which are icon folders w a cursors folder
fn cursor_themes() -> Vec<String> {
    let home = glib_home();
    let mut themes: Vec<String> = [
        home.join(".icons"),
        home.join(".local/share/icons"),
        "/usr/share/icons".into(),
    ]
    .iter()
    .filter_map(|d| std::fs::read_dir(d).ok())
    .flatten()
    .flatten()
    .filter(|e| e.path().join("cursors").is_dir())
    .map(|e| e.file_name().to_string_lossy().to_string())
    .collect();
    themes.sort_by_key(|t| t.to_lowercase());
    themes.dedup();
    themes.retain(|t| t != "default");
    themes.insert(0, "default".into());
    themes
}

fn glib_home() -> std::path::PathBuf {
    gtk4::glib::home_dir()
}

pub fn general(ctx: &Rc<Ctx>) -> Rc<Page> {
    let page = Page::new("tune", "General");
    let s = &ctx.store;
    page.section("Modifier");
    page.row("mod key", None, &w::choice(s, WM, "mod_key", &["super", "alt", "ctrl"]));
    page.row("mod while nested", Some("Inside another compositor which keeps super for itself"), &w::choice(s, WM, "nested_mod_key", &["alt", "super", "ctrl"]));
    page.row("Move and resize step", Some("Pixels per key press"), &w::number(s, WM, "step", 1.0, 500.0, 1.0, 0));
    page.section("Startup");
    page.row("Autostart", Some("Run when sevenwm starts as ur session and split them w ; then press enter"), &w::list_text(s, WM, "autostart", 36));
    page.row("Keep running", Some("Started w the session and again if they crash and press enter to save"), &w::list_text(s, WM, "keep_running", 36));
    page.section("New windows");
    page.row("Placement", Some("By view tiles when ur looking at a workspace and floats on the canvas"), &w::choice_labeled(s, WM, "placement.new_windows", &[("by-view", "By view"), ("always-tile", "Always tile"), ("always-float", "Always float")]));
    page.row("Floating size", None, &w::pair(s, WM, "placement.float_size", 100.0, 8000.0, 10.0));
    page.row("Maximize when this close", Some("A new window within this many px of the screen size opens maximized instead of floating and 0 turns it off"), &w::number(s, WM, "placement.maximize_tolerance", 0.0, 1000.0, 10.0, 0));
    page.row("Apps that tile", Some("App ids that tile or snap in like terminals and every other app floats in the middle and split them w ; then press enter"), &w::list_text(s, WM, "placement.tile_apps", 36));
    page.section("View");
    page.row("Zoom out to", None, &w::number(s, WM, "view.zoom_min", 0.05, 1.0, 0.05, 2));
    page.row("Zoom in to", None, &w::number(s, WM, "view.zoom_max", 1.0, 8.0, 0.1, 1));
    page.row("Zoom per scroll notch", None, &w::number(s, WM, "view.zoom_step", 1.01, 2.0, 0.01, 2));
    page.row("Fly to windows focused off screen", None, &w::switch(s, WM, "view.focus_follows_view"));
    page.row("Focus the window under the mouse", None, &w::switch(s, WM, "view.focus_follows_mouse"));
    page.row("Move the mouse to focused tiles", Some("Only for windows in a workspace"), &w::switch(s, WM, "view.mouse_follows_focus"));
    page.row("mod+arrow leaves the workspace", Some("From a tile u can go on to windows outside its workspace"), &w::switch(s, WM, "view.focus_leaves_workspace"));
    page.row("Drag empty canvas to pan", None, &w::switch(s, WM, "view.drag_empty_canvas_pans"));
    page.row("Pan over windows w mod +", Some("Held w mod while left dragging"), &modifier_choice(s, "view.pan_modifier", "workspaces.drag_modifier"));
    page.section("Screenshot");
    page.row("Command", Some("Runs on mod+shift+s"), &w::text(s, WM, "screenshot.command", 30));
    page.row("Freeze the screen while picking", Some("sevenshells own picker freezes by itself"), &w::switch(s, WM, "screenshot.freeze"));
    page.section("X11 apps");
    page.row("Run x11 apps", Some("Thru xwayland-satellite"), &w::switch(s, WM, "xwayland.enabled"));
    page.row("Program", None, &w::text(s, WM, "xwayland.path", 24));
    page
}

pub fn tiling(ctx: &Rc<Ctx>) -> Rc<Page> {
    let page = Page::new("dashboard", "Tiling & canvas");
    let s = &ctx.store;
    page.section("Tiling");
    page.row("Gap between tiles", None, &w::number(s, WM, "tiling.gaps_inner", 0.0, 200.0, 1.0, 0));
    page.row("Gap at the edge", None, &w::number(s, WM, "tiling.gaps_outer", 0.0, 200.0, 1.0, 0));
    page.row("Split ratio", None, &w::number(s, WM, "tiling.split_ratio", 0.1, 0.9, 0.05, 2));
    page.row("New tiles go", None, &w::choice(s, WM, "tiling.new_tile_position", &["main", "end"]));
    page.section("Workspaces");
    page.row("Space between workspaces", None, &w::number(s, WM, "workspaces.gap", 0.0, 2000.0, 10.0, 0));
    page.row("Drag a workspace w mod +", Some("Held w mod while left dragging a workspace"), &modifier_choice(s, "workspaces.drag_modifier", "view.pan_modifier"));
    page.section("Collapsing");
    let auto = w::switch(s, WM, "collapse.auto");
    page.row("Auto collapse", Some("Floating windows left off screen and unused fold into markers on their own"), &auto);
    let minutes = w::number(s, WM, "collapse.auto_after_minutes", 1.0, 1440.0, 1.0, 0);
    minutes.set_sensitive(auto.is_active());
    auto.connect_active_notify({
        let minutes = minutes.clone();
        move |a| minutes.set_sensitive(a.is_active())
    });
    page.row("Auto collapse after", Some("Minutes a window can sit there unused"), &minutes);
    page.section("Session");
    page.row("Put windows back after a restart", Some("Remembers workspaces cameras and window spots"), &w::switch(s, WM, "session.restore"));
    page.row("Only for windows opening within", Some("Seconds after startup"), &w::number(s, WM, "session.restore_within_seconds", 0.0, 3600.0, 10.0, 0));
    page.section("Snapping");
    page.row("Snap floating windows", None, &w::switch(s, WM, "snap.enabled"));
    page.row("Snap gap", None, &w::number(s, WM, "snap.gap", 0.0, 200.0, 1.0, 0));
    page.row("Snap distance", None, &w::number(s, WM, "snap.threshold", 0.0, 200.0, 1.0, 0));
    page.section("Canvas");
    // infinite or a width and height
    let size = s.get(WM, "canvas.size");
    let dims: Vec<f64> = size
        .as_ref()
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_integer()).map(|i| i as f64).collect())
        .unwrap_or_else(|| vec![8000.0, 6000.0]);
    let limited = gtk4::Switch::new();
    limited.set_active(size.as_ref().is_some_and(Value::is_array));
    let spins: Vec<gtk4::SpinButton> = (0..2)
        .map(|i| {
            let sp = gtk4::SpinButton::with_range(500.0, 100000.0, 100.0);
            sp.set_value(dims.get(i).copied().unwrap_or(8000.0));
            sp
        })
        .collect();
    let dims_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    dims_box.append(&spins[0]);
    dims_box.append(&gtk4::Label::new(Some("×")));
    dims_box.append(&spins[1]);
    let update = {
        let (store, limited, spins) = (s.clone(), limited.clone(), spins.clone());
        Rc::new(move || {
            for sp in &spins {
                sp.set_sensitive(limited.is_active());
            }
            let v = if limited.is_active() {
                Value::Array(spins.iter().map(|sp| Value::Integer(sp.value() as i64)).collect())
            } else {
                Value::String("infinite".into())
            };
            store.put(WM, "canvas.size", v);
        })
    };
    for sp in &spins {
        sp.set_sensitive(limited.is_active());
        let update = update.clone();
        sp.connect_value_changed(move |_| update());
    }
    {
        let update = update.clone();
        limited.connect_active_notify(move |_| update());
    }
    page.row("Limit the canvas", Some("Off means infinite"), &limited);
    page.row("Canvas size", None, &dims_box);
    page
}

pub fn input(ctx: &Rc<Ctx>) -> Rc<Page> {
    let page = Page::new("mouse", "Input");
    let s = &ctx.store;
    page.section("Keyboard");
    page.row("Layout", Some("xkb layouts like us or us,de"), &w::text(s, WM, "input.keyboard.layout", 16));
    page.row("Variant", None, &w::text(s, WM, "input.keyboard.variant", 16));
    page.row("Options", Some("Like caps:escape"), &w::text(s, WM, "input.keyboard.options", 20));
    page.row("Repeat rate", Some("Per second"), &w::number(s, WM, "input.keyboard.repeat_rate", 1.0, 100.0, 1.0, 0));
    page.row("Repeat delay", Some("Milliseconds"), &w::number(s, WM, "input.keyboard.repeat_delay", 100.0, 2000.0, 10.0, 0));
    for (device, title) in [("mouse", "Mouse"), ("touchpad", "Touchpad")] {
        page.section(title);
        let base = format!("input.{device}");
        page.row("Acceleration", Some("Flat is 1:1 and adaptive makes fast moves go further"), &w::choice(s, WM, &format!("{base}.accel_profile"), &["flat", "adaptive"]));
        page.row("Speed", Some("-1 to 1"), &w::number(s, WM, &format!("{base}.accel_speed"), -1.0, 1.0, 0.05, 2));
        page.row("Natural scrolling", None, &w::switch(s, WM, &format!("{base}.natural_scroll")));
        page.row("Left handed", None, &w::switch(s, WM, &format!("{base}.left_handed")));
        if device == "touchpad" {
            page.row("Tap to click", None, &w::switch(s, WM, &format!("{base}.tap")));
            page.row("Disable while typing", None, &w::switch(s, WM, &format!("{base}.disable_while_typing")));
        }
    }
    page.note("Mouse and touchpad settings only apply on real hardware");
    page
}

/// the actions u can bind w a note about each
const ACTIONS: &str = "exec <command> · exec-outside <command> · close-window · quit · cycle-windows · home · overview · toggle-fullscreen · toggle-maximize · toggle-floating · toggle-tiling · focus <dir> · move <dir> · grow <dir> · shrink <dir> · relaunch-window · reload-config · screenshot · workspace <1-10> · move-to-workspace <1-10> · new-workspace · remove-workspace · collapse · center-window · origin · window-to-origin · workspace-to-origin · zoom-in · zoom-out · none";

pub fn keybindings(ctx: &Rc<Ctx>) -> Rc<Page> {
    let page = Page::new("keyboard_command_key", "Keybindings");
    let s = &ctx.store;
    page.note("Keys are mod super alt ctrl or shift plus a key like a-z 0-9 left tab space return or f1 and only binds that differ from the defaults get saved");
    page.section("Binds");
    let list = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    let binds: Vec<(String, String)> = s
        .get(WM, "keybindings")
        .and_then(|v| v.as_table().cloned())
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k, v.as_str().unwrap_or("").to_string()))
        .collect();
    let rows: Rc<RefCell<Vec<(gtk4::Entry, gtk4::Entry, gtk4::Box)>>> = Rc::default();
    // every edit rewrites the whole table from the rows
    let sync: Rc<dyn Fn()> = {
        let (store, rows) = (s.clone(), rows.clone());
        Rc::new(move || {
            let mut table = Table::new();
            for (keys, action, _) in rows.borrow().iter() {
                let k = keys.text().trim().to_string();
                if !k.is_empty() {
                    table.insert(k, Value::String(action.text().trim().to_string()));
                }
            }
            store.put(WM, "keybindings", Value::Table(table));
        })
    };
    let add_row = {
        let (list, rows, sync) = (list.clone(), rows.clone(), sync.clone());
        Rc::new(move |keys: &str, action: &str| -> gtk4::Entry {
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
            row.add_css_class("card");
            let k = gtk4::Entry::new();
            k.set_text(keys);
            k.set_width_chars(18);
            let a = gtk4::Entry::new();
            a.set_text(action);
            a.set_hexpand(true);
            let remove = gtk4::Button::new();
            remove.set_child(Some(&crate::style::icon("delete")));
            remove.add_css_class("pill");
            remove.add_css_class("icon-button");
            remove.set_tooltip_text(Some("Remove this bind"));
            row.append(&k);
            row.append(&a);
            row.append(&remove);
            list.append(&row);
            for e in [&k, &a] {
                let sync = sync.clone();
                e.connect_changed(move |_| sync());
            }
            {
                let (rows, list, row, sync) = (rows.clone(), list.clone(), row.clone(), sync.clone());
                remove.connect_clicked(move |_| {
                    rows.borrow_mut().retain(|(_, _, r)| *r != row);
                    list.remove(&row);
                    sync();
                });
            }
            rows.borrow_mut().push((k.clone(), a, row));
            k
        })
    };
    for (k, a) in &binds {
        add_row(k, a);
    }
    page.append(&list);
    let add = w::button("Add a bind", "tonal", {
        let add_row = add_row.clone();
        move || {
            let k = add_row("mod+", "exec ");
            k.grab_focus();
        }
    });
    add.set_halign(gtk4::Align::Start);
    add.set_margin_top(12);
    page.append(&add);
    page.section("Actions");
    let actions = gtk4::Label::new(Some(ACTIONS));
    actions.set_wrap(true);
    actions.set_xalign(0.0);
    actions.add_css_class("row-hint");
    page.wide(&actions);
    page
}

pub fn animations(ctx: &Rc<Ctx>) -> Rc<Page> {
    let page = Page::new("animation", "Animations");
    let s = &ctx.store;
    let preview = AnimationPreview::new(s);
    page.wide(&preview.area);
    let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    for (label, part) in [("Play all", None), ("Open", Some("open")), ("Move", Some("move")), ("Close", Some("close")), ("View flight", Some("fly"))] {
        let p = preview.clone();
        buttons.append(&w::button(label, if part.is_none() { "filled" } else { "" }, move || p.play(part)));
    }
    page.wide(&buttons);
    let styles = ["none", "fade", "zoom", "pop", "slide"];
    // any change replays it so u see the result right away
    {
        let p = Rc::downgrade(&preview);
        s.on_change(move || {
            if let Some(p) = p.upgrade() {
                p.play(None);
            }
        });
    }
    page.section("Animations");
    page.row("Animations", None, &w::switch(s, WM, "animations.enabled"));
    page.row("Wobbly windows", Some("Floating windows bend like jelly while u drag them"), &w::switch(s, WM, "animations.wobbly"));
    page.row("Slide the bar away", Some("The bar slides up over fullscreen windows instead of js getting covered"), &w::switch(s, WM, "animations.panel_slide"));
    page.section("Opening windows");
    page.row("Style", None, &w::choice(s, WM, "animations.open_style", &styles));
    page.row("Length", Some("Milliseconds"), &w::number(s, WM, "animations.open_ms", 0.0, 3000.0, 10.0, 0));
    page.row("Curve", None, &w::choice(s, WM, "animations.open_curve", &CURVES));
    page.section("Closing windows");
    page.row("Style", None, &w::choice(s, WM, "animations.close_style", &styles));
    page.row("Length", Some("Milliseconds"), &w::number(s, WM, "animations.close_ms", 0.0, 3000.0, 10.0, 0));
    page.row("Curve", None, &w::choice(s, WM, "animations.close_curve", &CURVES));
    page.section("Moving windows");
    page.row("Length", Some("Milliseconds"), &w::number(s, WM, "animations.move_ms", 0.0, 3000.0, 10.0, 0));
    page.row("Curve", None, &w::choice(s, WM, "animations.move_curve", &CURVES));
    page.section("The view");
    page.row("Flight length", Some("Milliseconds"), &w::number(s, WM, "view.fly_duration_ms", 0.0, 3000.0, 10.0, 0));
    page.row("Curve", None, &w::choice(s, WM, "animations.fly_curve", &CURVES));
    page
}

/// one monitor as the layout editor sees it
#[derive(Clone)]
struct Monitor {
    name: String,
    position: [i64; 2],
    size: [i64; 2],
    mode: String,
    modes: Vec<String>,
    scale: f64,
}

/// a monitor being dragged and the view scale from when it started so it doesnt run off
#[derive(Clone, Copy)]
struct MonitorDrag {
    index: usize,
    grab: (f64, f64),
    layout: (f64, f64, f64),
}

pub fn monitors(ctx: &Rc<Ctx>) -> Rc<Page> {
    let page = Page::new("desktop_windows", "Monitors");
    let s = &ctx.store;
    let live: Vec<serde_json::Value> = std::fs::read_to_string(super::store::runtime_dir().join("monitors.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    if live.is_empty() {
        page.note("No monitors reported so is sevenwm running");
        return page;
    }
    let configured: Vec<Table> = s
        .get(WM, "monitors")
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| v.as_table().cloned())
        .collect();
    let monitors: Rc<RefCell<Vec<Monitor>>> = Rc::new(RefCell::new(
        live.iter()
            .map(|m| {
                let name = m["name"].as_str().unwrap_or("").to_string();
                let conf = configured.iter().find(|c| c.get("name").and_then(Value::as_str) == Some(&name));
                let pair = |v: &serde_json::Value| [v[0].as_i64().unwrap_or(0), v[1].as_i64().unwrap_or(0)];
                Monitor {
                    position: conf
                        .and_then(|c| c.get("position")?.as_array().cloned())
                        .map(|a| [a[0].as_integer().unwrap_or(0), a[1].as_integer().unwrap_or(0)])
                        .unwrap_or_else(|| pair(&m["position"])),
                    size: pair(&m["size"]),
                    mode: conf
                        .and_then(|c| c.get("mode")?.as_str().map(String::from))
                        .or_else(|| m["mode"].as_str().map(String::from))
                        .unwrap_or_else(|| "auto".into()),
                    modes: m["modes"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
                    scale: conf.and_then(|c| c.get("scale")?.as_float()).or_else(|| m["scale"].as_f64()).unwrap_or(1.0),
                    name,
                }
            })
            .collect(),
    ));
    let store_all = {
        let (store, monitors) = (s.clone(), monitors.clone());
        Rc::new(move || {
            let list = monitors
                .borrow()
                .iter()
                .map(|m| {
                    let mut t = Table::new();
                    t.insert("name".into(), Value::String(m.name.clone()));
                    t.insert("position".into(), Value::Array(m.position.iter().map(|v| Value::Integer(*v)).collect()));
                    t.insert("mode".into(), Value::String(m.mode.clone()));
                    t.insert("scale".into(), Value::Float(m.scale));
                    Value::Table(t)
                })
                .collect();
            store.put(WM, "monitors", Value::Array(list));
        })
    };
    let selected = Rc::new(std::cell::Cell::new(0usize));
    let area = gtk4::DrawingArea::new();
    area.set_content_height(300);
    area.set_hexpand(true);
    let layout = {
        let monitors = monitors.clone();
        move |width: f64, height: f64| {
            let m = monitors.borrow();
            let xs: Vec<f64> = m.iter().flat_map(|m| [m.position[0] as f64, (m.position[0] + m.size[0]) as f64]).collect();
            let ys: Vec<f64> = m.iter().flat_map(|m| [m.position[1] as f64, (m.position[1] + m.size[1]) as f64]).collect();
            let (minx, maxx) = (xs.iter().cloned().fold(f64::MAX, f64::min), xs.iter().cloned().fold(f64::MIN, f64::max));
            let (miny, maxy) = (ys.iter().cloned().fold(f64::MAX, f64::min), ys.iter().cloned().fold(f64::MIN, f64::max));
            let (sw, sh) = ((maxx - minx).max(1.0), (maxy - miny).max(1.0));
            let scale = ((width - 60.0) / (sw * 1.6)).min((height - 40.0) / (sh * 1.6));
            (scale, minx - sw * 0.3, miny - sh * 0.3)
        }
    };
    let layout = Rc::new(layout);
    // drag monitors around and their edges snap together
    let drag_state: Rc<RefCell<Option<MonitorDrag>>> = Rc::default();
    {
        let (monitors, selected, layout, drag_state) = (monitors.clone(), selected.clone(), layout.clone(), drag_state.clone());
        area.set_draw_func(move |_, cr, w, h| {
            // mid drag the view holds still so the monitor follows the pointer
            let (scale, ox, oy) = drag_state.borrow().map_or_else(|| layout(w as f64, h as f64), |d| d.layout);
            let palette = crate::theme::current();
            let set = |name: &str, a: f64| {
                let (r, g, b, _) = crate::theme::rgba(palette.get(name), 1.0);
                cr.set_source_rgba(r, g, b, a);
            };
            for (i, m) in monitors.borrow().iter().enumerate() {
                let (x, y) = ((m.position[0] as f64 - ox) * scale, (m.position[1] as f64 - oy) * scale);
                let (mw, mh) = (m.size[0] as f64 * scale, m.size[1] as f64 * scale);
                set(if i == selected.get() { "m3primaryContainer" } else { "m3surfaceContainerHigh" }, 1.0);
                crate::bar::rounded(cr, x, y, mw, mh, 12.0);
                let _ = cr.fill();
                set(if i == selected.get() { "m3onPrimaryContainer" } else { "m3onSurface" }, 1.0);
                cr.move_to(x + 12.0, y + 24.0);
                let _ = cr.show_text(&m.name);
                cr.move_to(x + 12.0, y + 42.0);
                let _ = cr.show_text(&format!("{}×{} at {} {}", m.size[0], m.size[1], m.position[0], m.position[1]));
            }
        });
    }
    let details = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    let name_label = gtk4::Label::new(None);
    let mode_list = gtk4::StringList::new(&[]);
    let mode_drop = gtk4::DropDown::new(Some(mode_list.clone()), gtk4::Expression::NONE);
    let scale_spin = gtk4::SpinButton::with_range(0.5, 4.0, 0.25);
    scale_spin.set_digits(2);
    let updating = Rc::new(std::cell::Cell::new(false));
    let show_selected = {
        let (monitors, selected, name_label, mode_list, mode_drop, scale_spin, updating) =
            (monitors.clone(), selected.clone(), name_label.clone(), mode_list.clone(), mode_drop.clone(), scale_spin.clone(), updating.clone());
        Rc::new(move || {
            updating.set(true);
            let m = monitors.borrow()[selected.get()].clone();
            name_label.set_text(&m.name);
            mode_list.splice(0, mode_list.n_items(), &[]);
            mode_list.append("auto");
            let mut seen = vec!["auto".to_string()];
            for mode in m.modes.iter().chain(std::iter::once(&m.mode)) {
                if !seen.contains(mode) {
                    seen.push(mode.clone());
                    mode_list.append(mode);
                }
            }
            mode_drop.set_selected(seen.iter().position(|x| *x == m.mode).unwrap_or(0) as u32);
            scale_spin.set_value(m.scale);
            updating.set(false);
        })
    };
    let drag = gtk4::GestureDrag::new();
    {
        let (monitors, selected, layout, drag_state, area2, show) = (monitors.clone(), selected.clone(), layout.clone(), drag_state.clone(), area.clone(), show_selected.clone());
        drag.connect_drag_begin(move |_, x, y| {
            let (scale, ox, oy) = layout(area2.width() as f64, area2.height() as f64);
            let (lx, ly) = (x / scale + ox, y / scale + oy);
            let hit = monitors.borrow().iter().enumerate().rev().find(|(_, m)| {
                let (mx, my) = (m.position[0] as f64, m.position[1] as f64);
                lx >= mx && lx < mx + m.size[0] as f64 && ly >= my && ly < my + m.size[1] as f64
            }).map(|(i, m)| (i, lx - m.position[0] as f64, ly - m.position[1] as f64));
            if let Some((i, dx, dy)) = hit {
                selected.set(i);
                *drag_state.borrow_mut() = Some(MonitorDrag { index: i, grab: (dx, dy), layout: (scale, ox, oy) });
                show();
                area2.queue_draw();
            }
        });
    }
    {
        let (monitors, drag_state, area2) = (monitors.clone(), drag_state.clone(), area.clone());
        drag.connect_drag_update(move |g, dx, dy| {
            let Some(MonitorDrag { index: i, grab: (offx, offy), layout: (scale, ox, oy) }) = *drag_state.borrow() else {
                return;
            };
            let Some((sx, sy)) = g.start_point() else {
                return;
            };
            let mut x = (sx + dx) / scale + ox - offx;
            let mut y = (sy + dy) / scale + oy - offy;
            let mut ms = monitors.borrow_mut();
            let (mw, mh) = (ms[i].size[0] as f64, ms[i].size[1] as f64);
            // snap edges to the other monitors edges
            for (j, o) in ms.iter().enumerate() {
                if j == i {
                    continue;
                }
                let (ox2, oy2, ow, oh) = (o.position[0] as f64, o.position[1] as f64, o.size[0] as f64, o.size[1] as f64);
                for c in [ox2 + ow, ox2 - mw, ox2, ox2 + ow - mw] {
                    if (x - c).abs() < 60.0 {
                        x = c;
                    }
                }
                for c in [oy2, oy2 + oh, oy2 - mh, oy2 + oh - mh] {
                    if (y - c).abs() < 60.0 {
                        y = c;
                    }
                }
            }
            ms[i].position = [x.round() as i64, y.round() as i64];
            drop(ms);
            area2.queue_draw();
        });
    }
    {
        let (drag_state, store_all) = (drag_state.clone(), store_all.clone());
        drag.connect_drag_end(move |_, _, _| {
            if drag_state.borrow_mut().take().is_some() {
                store_all();
            }
        });
    }
    area.add_controller(drag);
    page.note("Drag monitors to arrange them and their edges snap together");
    page.wide(&area);
    page.section("Selected monitor");
    page.row("Monitor", None, &name_label);
    {
        let (monitors, selected, store_all, updating, mode_list) = (monitors.clone(), selected.clone(), store_all.clone(), updating.clone(), mode_list.clone());
        mode_drop.connect_selected_notify(move |d| {
            if updating.get() {
                return;
            }
            if let Some(mode) = mode_list.string(d.selected()) {
                monitors.borrow_mut()[selected.get()].mode = mode.to_string();
                store_all();
            }
        });
    }
    {
        let (monitors, selected, store_all, updating) = (monitors.clone(), selected.clone(), store_all.clone(), updating.clone());
        scale_spin.connect_value_changed(move |sp| {
            if !updating.get() {
                monitors.borrow_mut()[selected.get()].scale = (sp.value() * 100.0).round() / 100.0;
                store_all();
            }
        });
    }
    page.row("Mode", Some("auto picks the best refresh rate"), &mode_drop);
    page.row("Scale", None, &scale_spin);
    let reset = w::button("Automatic left to right", "", {
        let store = s.clone();
        move || store.put(WM, "monitors", Value::Array(Vec::new()))
    });
    page.row("Reset the layout", None, &reset);
    let _ = details;
    show_selected();
    page
}

pub fn rules(ctx: &Rc<Ctx>) -> Rc<Page> {
    let page = Page::new("rule", "Window rules");
    let s = &ctx.store;
    page.note("Rules for windows whose app id or title match where * is anything and ? is one char and later rules win");
    let list = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    struct RuleRow {
        row: gtk4::Box,
        app_id: gtk4::Entry,
        title: gtk4::Entry,
        placement: gtk4::DropDown,
        width: gtk4::SpinButton,
        height: gtk4::SpinButton,
        hide: gtk4::Switch,
    }
    let rows: Rc<RefCell<Vec<RuleRow>>> = Rc::default();
    let sync: Rc<dyn Fn()> = {
        let (store, rows) = (s.clone(), rows.clone());
        Rc::new(move || {
            let mut rules = Vec::new();
            for r in rows.borrow().iter() {
                let mut t = Table::new();
                let (app, title) = (r.app_id.text().trim().to_string(), r.title.text().trim().to_string());
                if !app.is_empty() {
                    t.insert("app_id".into(), Value::String(app));
                }
                if !title.is_empty() {
                    t.insert("title".into(), Value::String(title));
                }
                match r.placement.selected() {
                    1 => { t.insert("float".into(), Value::Boolean(true)); }
                    2 => { t.insert("float".into(), Value::Boolean(false)); }
                    _ => {}
                }
                if r.width.value() > 0.0 && r.height.value() > 0.0 {
                    t.insert("size".into(), Value::Array(vec![Value::Integer(r.width.value() as i64), Value::Integer(r.height.value() as i64)]));
                }
                if r.hide.is_active() {
                    t.insert("hide_from_screencast".into(), Value::Boolean(true));
                }
                if t.contains_key("app_id") || t.contains_key("title") {
                    rules.push(Value::Table(t));
                }
            }
            store.put(WM, "rules", Value::Array(rules));
        })
    };
    let add_rule = {
        let (list, rows, sync) = (list.clone(), rows.clone(), sync.clone());
        Rc::new(move |rule: &Table| {
            let card = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
            card.add_css_class("card");
            let top = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            let app_id = gtk4::Entry::new();
            app_id.set_placeholder_text(Some("app id"));
            app_id.set_text(rule.get("app_id").and_then(Value::as_str).unwrap_or(""));
            app_id.set_hexpand(true);
            let title = gtk4::Entry::new();
            title.set_placeholder_text(Some("title"));
            title.set_text(rule.get("title").and_then(Value::as_str).unwrap_or(""));
            title.set_hexpand(true);
            let remove = gtk4::Button::new();
            remove.set_child(Some(&crate::style::icon("delete")));
            remove.add_css_class("pill");
            remove.add_css_class("icon-button");
            top.append(&app_id);
            top.append(&title);
            top.append(&remove);
            let bottom = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            let placement = gtk4::DropDown::from_strings(&["Default", "Float", "Tile"]);
            placement.set_selected(match rule.get("float").and_then(Value::as_bool) {
                Some(true) => 1,
                Some(false) => 2,
                None => 0,
            });
            let size: Vec<f64> = rule.get("size").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_integer()).map(|i| i as f64).collect()).unwrap_or_default();
            let width = gtk4::SpinButton::with_range(0.0, 8000.0, 10.0);
            width.set_value(size.first().copied().unwrap_or(0.0));
            let height = gtk4::SpinButton::with_range(0.0, 8000.0, 10.0);
            height.set_value(size.get(1).copied().unwrap_or(0.0));
            let hide = gtk4::Switch::new();
            hide.set_active(rule.get("hide_from_screencast").and_then(Value::as_bool).unwrap_or(false));
            hide.set_valign(gtk4::Align::Center);
            bottom.append(&placement);
            bottom.append(&gtk4::Label::new(Some("size")));
            bottom.append(&width);
            bottom.append(&gtk4::Label::new(Some("×")));
            bottom.append(&height);
            let hide_label = gtk4::Label::new(Some("hide from screencast"));
            hide_label.set_hexpand(true);
            hide_label.set_xalign(1.0);
            bottom.append(&hide_label);
            bottom.append(&hide);
            card.append(&top);
            card.append(&bottom);
            list.append(&card);
            for e in [&app_id, &title] {
                let sync = sync.clone();
                e.connect_changed(move |_| sync());
            }
            for sp in [&width, &height] {
                let sync = sync.clone();
                sp.connect_value_changed(move |_| sync());
            }
            {
                let sync = sync.clone();
                placement.connect_selected_notify(move |_| sync());
            }
            {
                let sync = sync.clone();
                hide.connect_active_notify(move |_| sync());
            }
            {
                let (rows, list, card, sync) = (rows.clone(), list.clone(), card.clone(), sync.clone());
                remove.connect_clicked(move |_| {
                    rows.borrow_mut().retain(|r| r.row != card);
                    list.remove(&card);
                    sync();
                });
            }
            rows.borrow_mut().push(RuleRow { row: card, app_id, title, placement, width, height, hide });
        })
    };
    for rule in s.get(WM, "rules").and_then(|v| v.as_array().cloned()).unwrap_or_default() {
        if let Some(t) = rule.as_table() {
            add_rule(t);
        }
    }
    page.section("Rules");
    page.append(&list);
    let add = w::button("Add a rule", "tonal", {
        let add_rule = add_rule.clone();
        move || {
            let mut t = Table::new();
            t.insert("float".into(), Value::Boolean(true));
            add_rule(&t);
        }
    });
    add.set_halign(gtk4::Align::Start);
    add.set_margin_top(12);
    page.append(&add);
    page
}
