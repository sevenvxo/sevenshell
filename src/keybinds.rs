//! the keybind search on mod+/ that looks like the launcher where u type what u want to do and enter does it

use std::rc::Rc;

use gtk4::prelude::*;
use serde_json::json;

use crate::ipc;
use crate::picker::{Look, Picker, Row, Source, is_subsequence};

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

/// a material symbols icon for each section
fn section_icon(section: &str) -> &'static str {
    match section {
        "Apps" => "apps",
        "Windows" => "select_window",
        "Tiling" => "dashboard",
        "Workspaces" => "view_quilt",
        "View" => "zoom_in",
        "Move & resize" => "open_with",
        "Media" => "music_note",
        "Mouse" => "mouse",
        _ => "keyboard",
    }
}

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

pub type Keybinds = Picker<Actions>;

/// one thing u can do w the keys that do it
struct Action {
    section: &'static str,
    text: String,
    keys: Vec<String>,
    /// the sevenwm action to run or none for mouse things
    run: Option<String>,
    /// text keys and action lowercased for searching
    search: String,
}

pub struct Actions {
    all: Vec<Action>,
}

impl Keybinds {
    pub fn new(app: &gtk4::Application) -> Rc<Self> {
        let (all, mod_key) = match ipc::request(json!({ "get": "bindings" })) {
            Ok(reply) => (actions(&reply), reply["mod"].as_str().unwrap_or("super").to_string()),
            Err(_) => (Vec::new(), "super".into()),
        };
        let hint: &'static str = Box::leak(
            format!("mod = {} · Enter does it · Esc closes", key_name(&mod_key)).into_boxed_str(),
        );
        let look = Look {
            namespace: "keybinds",
            width: 640,
            placeholder: Some("What do you want to do?"),
            empty: "No keybind does that",
            detail: false,
            hint: Some(hint),
        };
        Picker::build(app, look, Actions { all })
    }
}

/// every bind from sevenwm plus the mouse ones grouped so binds doing the same share a row
fn actions(reply: &serde_json::Value) -> Vec<Action> {
    let bindings: Vec<(String, String)> = reply["bindings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| Some((b["keys"].as_str()?.to_string(), b["action"].as_str()?.to_string())))
        .collect();
    let mut rows: Vec<Action> = Vec::new();
    for (keys, action) in &bindings {
        let (section, text) = describe(action);
        // workspace n and the like run w their own number so they only share a row not an action
        let runnable = !matches!(action.split_whitespace().next(), Some("workspace" | "move-to-workspace"));
        match rows.iter_mut().find(|r| r.text == text && r.section == section) {
            Some(row) => row.keys.push(keys.clone()),
            None => rows.push(Action {
                section,
                text,
                keys: vec![keys.clone()],
                run: runnable.then(|| action.clone()),
                search: String::new(),
            }),
        }
    }
    let mouse = mouse_rows(
        reply["mouse"]["pan"].as_str().unwrap_or("mod+alt"),
        reply["mouse"]["workspace_drag"].as_str().unwrap_or("mod+ctrl"),
    );
    for (keys, text) in mouse {
        rows.push(Action {
            section: "Mouse",
            text: text.to_string(),
            keys: vec![keys],
            run: None,
            search: String::new(),
        });
    }
    for row in &mut rows {
        // letters before arrows and fewer modifiers first maybe
        row.keys.sort_by_key(|k| (k.matches('+').count(), is_arrow(k), k.clone()));
        row.keys = squash_digits(&row.keys);
        row.search = format!(
            "{} {} {} {}",
            row.text,
            row.section,
            row.keys.iter().map(|k| k.split('+').map(key_name).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join(" "),
            row.run.as_deref().unwrap_or("")
        )
        .to_lowercase();
    }
    rows.sort_by_key(|r| SECTIONS.iter().position(|s| *s == r.section).unwrap_or(SECTIONS.len()));
    rows
}

impl Source for Actions {
    fn len(&self) -> usize {
        self.all.len()
    }

    fn rank(&self, query: &str) -> Vec<usize> {
        if query.is_empty() {
            return (0..self.all.len()).collect();
        }
        let mut ranked: Vec<(u32, usize)> = self
            .all
            .iter()
            .enumerate()
            .filter_map(|(i, a)| {
                let text = a.text.to_lowercase();
                let score = if text.starts_with(query) {
                    100
                } else if text.split_whitespace().any(|w| w.starts_with(query)) {
                    80
                } else if text.contains(query) {
                    60
                } else if a.search.contains(query) {
                    40
                } else if is_subsequence(query, &text) {
                    20
                } else {
                    return None;
                };
                Some((score, i))
            })
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0));
        ranked.into_iter().map(|(_, i)| i).collect()
    }

    fn fill(&self, index: usize, row: &Row) {
        let a = &self.all[index];
        row.icon.set_visible(false);
        row.glyph.set_visible(true);
        row.glyph.add_css_class("icon");
        row.glyph.set_text(section_icon(a.section));
        row.name.set_text(&a.text);
        while let Some(child) = row.keys.first_child() {
            row.keys.remove(&child);
        }
        for (i, combo) in a.keys.iter().take(2).enumerate() {
            if i > 0 {
                let or = gtk4::Label::new(Some("or"));
                or.add_css_class("or");
                row.keys.append(&or);
            }
            for part in combo.split('+') {
                let key = gtk4::Label::new(Some(&key_name(part)));
                key.add_css_class("key");
                row.keys.append(&key);
            }
        }
        row.keys.set_visible(true);
    }

    fn pick(&self, index: usize, window: &gtk4::ApplicationWindow) {
        let action = self.all[index].run.clone();
        window.close();
        // after closing so the action lands on the window u were on and not this search
        if let Some(action) = action {
            gtk4::glib::timeout_add_local_once(std::time::Duration::from_millis(80), move || ipc::action(&action));
        }
    }
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
        "period" => ".",
        "comma" => ",",
        "semicolon" => ";",
        "apostrophe" => "'",
        "grave" => "`",
        "backspace" => "Backspace",
        "delete" => "Del",
        "print" => "PrtSc",
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
        "origin" => ("View", "Back to 0 0".into()),
        "window-to-origin" => ("Windows", "Move window to 0 0".into()),
        "workspace-to-origin" => ("Workspaces", "Move workspace to 0 0".into()),
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
        ("sevenshell keybinds", "Apps", "Search keybinds"),
        ("sevenshell clipboard", "Apps", "Clipboard history"),
        ("sevenshell emoji", "Apps", "Emoji picker"),
        ("sevenshell quick", "Apps", "Quick settings"),
        ("sevenshell session", "Apps", "Lock, log out, restart or shut down"),
        ("sevenshell screenshot", "Apps", "Screenshot"),
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
        ("sevenshell settings", "Apps", "Settings"),
        ("sevenshell taskmanager", "Apps", "Task manager"),
        ("default-web-browser", "Apps", "Web browser"),
        ("inode/directory", "Apps", "Files"),
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
