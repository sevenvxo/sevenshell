//! the keybind cheat sheet on mod+/ read from sevenwms ipc and escape or a click closes it

use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{gdk, glib};
use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};
use serde_json::json;

use crate::ipc;

pub const CSS: &str = "
.keybinds { background: rgba(0, 0, 0, 0.55); }
.keybinds * {
    font-family: \"Adwaita Sans\", \"Symbols Nerd Font\", sans-serif;
    font-size: 13px;
    color: #ffffff;
}
.keybinds .panel {
    background: #000000;
    border: 1px solid #ffffff;
    border-radius: 7px;
    padding: 18px 22px;
}
.keybinds .title { font-size: 16px; font-weight: bold; }
.keybinds .hint { font-size: 12px; }
.keybinds .divider { background: #ffffff; min-height: 1px; margin: 10px 0 12px 0; }
.keybinds .section { font-weight: bold; margin: 8px 0 4px 0; }
.keybinds .key {
    border: 1px solid #ffffff; border-radius: 4px;
    padding: 0 5px; font-size: 12px;
}
.keybinds .or { font-size: 12px; margin: 0 2px; }
";

/// the sections in the order theyre shown maybe
const SECTIONS: [&str; 8] = [
    "Apps",
    "Windows",
    "Tiling",
    "Workspaces",
    "View",
    "Move & resize",
    "Media",
    "Mouse",
];

/// mouse controls arent keybinds so they come from sevenwm
fn mouse_rows(pan: &str, workspace_drag: &str) -> Vec<(String, &'static str)> {
    vec![
        ("mod+left drag".into(), "Move a window (drop on a workspace to tile)"),
        ("mod+right drag".into(), "Resize a floating window"),
        ("mod+right click".into(), "Window menu"),
        (format!("{workspace_drag}+right click"), "Workspace menu (on its empty space)"),
        (format!("{workspace_drag}+left drag"), "Move a workspace"),
        (format!("{pan}+left drag"), "Pan the view, even over windows"),
        ("mod+scroll".into(), "Zoom"),
    ]
}

pub struct Keybinds {
    pub window: gtk4::ApplicationWindow,
}

impl Keybinds {
    pub fn new(app: &gtk4::Application) -> Rc<Self> {
        let window = gtk4::ApplicationWindow::new(app);
        window.init_layer_shell();
        window.set_namespace(Some("keybinds"));
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        // cover the screen so a click anywhere closes it
        for edge in [
            gtk4_layer_shell::Edge::Top,
            gtk4_layer_shell::Edge::Bottom,
            gtk4_layer_shell::Edge::Left,
            gtk4_layer_shell::Edge::Right,
        ] {
            window.set_anchor(edge, true);
        }
        window.add_css_class("keybinds");

        let panel = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        panel.add_css_class("panel");
        panel.set_halign(gtk4::Align::Center);
        panel.set_valign(gtk4::Align::Center);

        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        let title = gtk4::Label::new(Some("Keybinds"));
        title.add_css_class("title");
        title.set_xalign(0.0);
        title.set_hexpand(true);
        let hint = gtk4::Label::new(None);
        hint.add_css_class("hint");
        header.append(&title);
        header.append(&hint);
        panel.append(&header);
        let divider = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        divider.add_css_class("divider");
        panel.append(&divider);

        match ipc::request(json!({ "get": "bindings" })) {
            Ok(reply) => {
                let mod_key = reply["mod"].as_str().unwrap_or("super").to_string();
                hint.set_text(&format!("mod = {}  ·  Esc to close", key_name(&mod_key)));
                let bindings: Vec<(String, String)> = reply["bindings"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|b| {
                        Some((b["keys"].as_str()?.to_string(), b["action"].as_str()?.to_string()))
                    })
                    .collect();
                let mouse = mouse_rows(
                    reply["mouse"]["pan"].as_str().unwrap_or("mod+alt"),
                    reply["mouse"]["workspace_drag"].as_str().unwrap_or("mod+ctrl"),
                );
                let (width, height) = screen_size();
                // each column wants about 400px
                let count = ((width - 120) / 400).clamp(1, 3) as usize;
                let scroll = gtk4::ScrolledWindow::new();
                scroll.set_hscrollbar_policy(gtk4::PolicyType::Never);
                scroll.set_propagate_natural_height(true);
                scroll.set_propagate_natural_width(true);
                scroll.set_max_content_height(height - 160);
                scroll.set_child(Some(&columns(&bindings, &mouse, count)));
                panel.append(&scroll);
            }
            Err(err) => {
                hint.set_text("Esc to close");
                let label = gtk4::Label::new(Some(&format!("Can't ask sevenwm: {err}")));
                label.set_xalign(0.0);
                panel.append(&label);
            }
        }
        window.set_child(Some(&panel));

        let keybinds = Rc::new(Self { window });
        let keys = gtk4::EventControllerKey::new();
        let window = keybinds.window.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                window.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        keybinds.window.add_controller(keys);
        let click = gtk4::GestureClick::new();
        let window = keybinds.window.clone();
        click.connect_released(move |_, _, _, _| window.close());
        keybinds.window.add_controller(click);
        keybinds
    }

    pub fn show(&self) {
        self.window.present();
    }
}

/// one row w a description and the keys that do it
struct Row {
    section: &'static str,
    text: String,
    keys: Vec<String>,
}

/// the active monitor size in sevenwms logical pixels
fn screen_size() -> (i32, i32) {
    ipc::request(json!({ "get": "state" }))
        .ok()
        .and_then(|state| serde_json::from_value::<ipc::State>(state).ok())
        .and_then(|state| {
            let active = state.active_monitor.clone()?;
            state.monitor(&active).map(|m| (m.size[0], m.size[1]))
        })
        .unwrap_or((1920, 1080))
}

/// lay the sections into count columns each into the shortest so far
fn columns(bindings: &[(String, String)], mouse: &[(String, &str)], count: usize) -> gtk4::Box {
    let mut rows: Vec<Row> = Vec::new();
    for (keys, action) in bindings {
        let (section, text) = describe(action);
        // binds that do the same thing share a row
        match rows.iter_mut().find(|r| r.text == text && r.section == section) {
            Some(row) => row.keys.push(keys.clone()),
            None => rows.push(Row {
                section,
                text,
                keys: vec![keys.clone()],
            }),
        }
    }
    for (keys, text) in mouse {
        rows.push(Row {
            section: "Mouse",
            text: text.to_string(),
            keys: vec![keys.clone()],
        });
    }
    for row in &mut rows {
        // letters before arrows and fewer modifiers first maybe
        row.keys
            .sort_by_key(|k| (k.matches('+').count(), is_arrow(k), k.clone()));
        row.keys = squash_digits(&row.keys);
    }

    let outer = gtk4::Box::new(gtk4::Orientation::Horizontal, 36);
    let cols: Vec<gtk4::Box> = (0..count)
        .map(|_| {
            let col = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            col.set_valign(gtk4::Align::Start);
            outer.append(&col);
            col
        })
        .collect();
    let mut heights = vec![0usize; count];
    for section in SECTIONS {
        let items: Vec<&Row> = rows.iter().filter(|r| r.section == section).collect();
        if items.is_empty() {
            continue;
        }
        // into the shortest column so far
        let i = (0..count).min_by_key(|&i| heights[i]).unwrap_or(0);
        heights[i] += items.len() + 2;
        let heading = gtk4::Label::new(Some(section));
        heading.add_css_class("section");
        heading.set_xalign(0.0);
        cols[i].append(&heading);
        let grid = gtk4::Grid::new();
        grid.set_row_spacing(4);
        grid.set_column_spacing(16);
        for (r, row) in items.iter().enumerate() {
            let text = gtk4::Label::new(Some(&row.text));
            text.set_xalign(0.0);
            text.set_hexpand(true);
            grid.attach(&text, 0, r as i32, 1, 1);
            grid.attach(&keys_box(&row.keys), 1, r as i32, 1, 1);
        }
        cols[i].append(&grid);
    }
    outer
}

fn keys_box(combos: &[String]) -> gtk4::Box {
    let b = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
    b.set_halign(gtk4::Align::End);
    for (i, combo) in combos.iter().enumerate() {
        if i > 0 {
            let or = gtk4::Label::new(Some("or"));
            or.add_css_class("or");
            b.append(&or);
        }
        for part in combo.split('+') {
            let key = gtk4::Label::new(Some(&key_name(part)));
            key.add_css_class("key");
            b.append(&key);
        }
    }
    b
}

/// mod+1 thru mod+0 become one row so the ten workspace keys fit on one line
fn squash_digits(keys: &[String]) -> Vec<String> {
    let digit = |k: &str| {
        let (prefix, last) = k.rsplit_once('+').unwrap_or(("", k));
        (last.len() == 1 && last.chars().all(|c| c.is_ascii_digit())).then(|| (prefix.to_string(), last.to_string()))
    };
    let mut out: Vec<String> = Vec::new();
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for key in keys {
        match digit(key) {
            Some((prefix, d)) => match groups.iter_mut().find(|(p, _)| *p == prefix) {
                Some((_, ds)) => ds.push(d),
                None => groups.push((prefix, vec![d])),
            },
            None => out.push(key.clone()),
        }
    }
    for (prefix, mut digits) in groups {
        if digits.len() < 3 {
            out.extend(digits.into_iter().map(|d| format!("{prefix}+{d}")));
            continue;
        }
        // 1 to 9 then 0 for workspace 10
        digits.sort_by_key(|d| if d == "0" { 10 } else { d.parse::<u32>().unwrap_or(0) });
        let (first, last) = (digits[0].clone(), digits[digits.len() - 1].clone());
        out.push(format!("{prefix}+{first}\u{2026}{last}"));
    }
    out
}

fn is_arrow(combo: &str) -> bool {
    let last = combo.rsplit('+').next().unwrap_or("").to_lowercase();
    matches!(last.as_str(), "left" | "right" | "up" | "down")
}

/// how a key is shown where mod stays and arrows become arrows
fn key_name(key: &str) -> String {
    let lower = key.to_lowercase();
    let named = match lower.as_str() {
        "mod" => "mod",
        "super" | "logo" => "Super",
        "alt" => "Alt",
        "ctrl" | "control" => "Ctrl",
        "shift" => "Shift",
        "left" => "←",
        "right" => "→",
        "up" => "↑",
        "down" => "↓",
        "slash" => "/",
        "space" => "Space",
        "return" => "Enter",
        "escape" => "Esc",
        "tab" => "Tab",
        "equal" => "=",
        "minus" => "−",
        "plus" => "+",
        "kp_add" => "Num +",
        "kp_subtract" => "Num −",
        "xf86audioraisevolume" => "Vol +",
        "xf86audiolowervolume" => "Vol −",
        "xf86audiomute" => "Mute",
        "xf86audiomicmute" => "Mic mute",
        "xf86monbrightnessup" => "Bright +",
        "xf86monbrightnessdown" => "Bright −",
        "xf86audioplay" => "Play",
        "xf86audiopause" => "Pause",
        "xf86audionext" => "Next",
        "xf86audioprev" => "Prev",
        _ if key.chars().count() == 1 => return key.to_uppercase(),
        _ => return key.to_string(),
    };
    named.to_string()
}

/// the section and plain words for a sevenwm action
fn describe(action: &str) -> (&'static str, String) {
    let (name, arg) = match action.split_once(char::is_whitespace) {
        Some((name, arg)) => (name, arg.trim()),
        None => (action, ""),
    };
    let dir = arg;
    match name {
        "exec" => describe_exec(arg),
        "exec-outside" => {
            let (section, text) = describe_exec(arg);
            (section, format!("{text} (beside the workspace)"))
        }
        "close-window" => ("Windows", "Close window".into()),
        "quit" => ("Windows", "Quit sevenwm".into()),
        "cycle-windows" => ("Windows", "Cycle windows".into()),
        "toggle-fullscreen" => ("Windows", "Fullscreen".into()),
        "toggle-maximize" => ("Windows", "Maximize".into()),
        "relaunch-window" => ("Windows", "Relaunch app".into()),
        "focus" => ("Windows", format!("Focus {dir}")),
        "toggle-floating" => ("Tiling", "Float / tile".into()),
        "toggle-tiling" => ("Tiling", "Move to / from the workspace".into()),
        "home" => ("View", "Back to the home workspace".into()),
        "overview" => ("View", "Overview".into()),
        "center-window" => ("View", "Centre window at 100% zoom".into()),
        "zoom-in" => ("View", "Zoom in".into()),
        "zoom-out" => ("View", "Zoom out".into()),
        "workspace" => ("Workspaces", "Go to workspace 1–10".into()),
        "move-to-workspace" => ("Workspaces", "Move window to workspace 1–10".into()),
        "new-workspace" => ("Workspaces", "New workspace".into()),
        "remove-workspace" => ("Workspaces", "Remove workspace".into()),
        "collapse" => ("Windows", "Collapse / restore".into()),
        "screenshot" => ("Apps", "Screenshot".into()),
        "reload-config" => ("Apps", "Reload config".into()),
        "move" => ("Move & resize", format!("Move {dir}")),
        "grow" => ("Move & resize", format!("Grow {dir}")),
        "shrink" => ("Move & resize", format!("Shrink {dir}")),
        _ => ("Apps", action.to_string()),
    }
}

fn describe_exec(command: &str) -> (&'static str, String) {
    let c = command.trim();
    let known: &[(&str, &str, &str)] = &[
        ("sevenshell launcher", "Apps", "App launcher"),
        ("sevenshell keybinds", "Apps", "These keybinds"),
        ("sevenshell windows", "Windows", "Window search"),
        ("sevenshell lock", "Apps", "Lock screen"),
        ("sevenshell volume up", "Media", "Volume up"),
        ("sevenshell volume down", "Media", "Volume down"),
        ("sevenshell volume mute", "Media", "Mute"),
        ("sevenshell mic mute", "Media", "Mute mic"),
        ("sevenshell brightness up", "Media", "Brightness up"),
        ("sevenshell brightness down", "Media", "Brightness down"),
        ("playerctl play-pause", "Media", "Play / pause"),
        ("playerctl next", "Media", "Next track"),
        ("playerctl previous", "Media", "Previous track"),
        ("sevenwm-settings", "Apps", "Settings"),
        ("default-web-browser", "Apps", "Web browser"),
        ("kitty", "Apps", "Terminal"),
        ("foot", "Apps", "Terminal"),
        ("alacritty", "Apps", "Terminal"),
        ("thunar", "Apps", "Files"),
        ("nautilus", "Apps", "Files"),
    ];
    for (needle, section, text) in known {
        if c == *needle || c.starts_with(&format!("{needle} ")) || c.contains(needle) {
            return (section, text.to_string());
        }
    }
    // or else the program name capitalized
    let program = c.split_whitespace().next().unwrap_or(c);
    let program = program.rsplit('/').next().unwrap_or(program);
    let mut chars = program.chars();
    let name = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => c.to_string(),
    };
    ("Apps", name)
}
