//! wallpapers from ur folder that u pick in quick settings where each one remembers the colors u had w it

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use toml::Value;

use crate::config;
use crate::settings::store::{Store, Which, set_now};

/// the theme settings that belong to a wallpaper
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
struct Look {
    source: String,
    variant: String,
    mode: String,
    primary: String,
    secondary: String,
    tertiary: String,
    #[serde(default)]
    colors: BTreeMap<String, String>,
}

const KEYS: [&str; 6] = ["source", "variant", "mode", "primary", "secondary", "tertiary"];

fn looks_path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_default()
        .join("sevenshell/wallpaper-looks.json")
}

fn load_looks() -> BTreeMap<String, Look> {
    std::fs::read(looks_path())
        .ok()
        .and_then(|d| serde_json::from_slice(&d).ok())
        .unwrap_or_default()
}

fn save_looks(looks: &BTreeMap<String, Look>) {
    let path = looks_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string_pretty(looks) {
        let _ = std::fs::write(path, text);
    }
}

/// the picture folder w ~ worked out
pub fn dir() -> PathBuf {
    crate::theme::expand(&config::get().wallpapers.dir)
}

/// every picture in the folder sorted by name
pub fn list() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e.to_lowercase().as_str(), "png" | "jpg" | "jpeg" | "webp"))
        })
        .collect();
    found.sort();
    found
}

/// the wallpaper sevenwm shows now
pub fn current() -> PathBuf {
    crate::theme::expand(&Store::new().str(Which::Wm, "canvas.wallpaper"))
}

/// the theme settings as they are in the shell config now
fn look_now(store: &Store) -> Look {
    let mut look = Look::default();
    for key in KEYS {
        let value = store.str(Which::Shell, &format!("theme.{key}"));
        match key {
            "source" => look.source = value,
            "variant" => look.variant = value,
            "mode" => look.mode = value,
            "primary" => look.primary = value,
            "secondary" => look.secondary = value,
            _ => look.tertiary = value,
        }
    }
    if let Some(Value::Table(colors)) = store.get(Which::Shell, "theme.colors") {
        look.colors = colors
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
            .collect();
    }
    look
}

/// put path up as the wallpaper and bring back the colors it had last time while the old one keeps its own
pub fn apply(path: &Path) -> Result<(), String> {
    let store = Store::new();
    let mut looks = load_looks();
    let old = current();
    if !old.as_os_str().is_empty() {
        looks.insert(old.to_string_lossy().into_owned(), look_now(&store));
    }
    let mut changes: Vec<(Which, String, Value)> = vec![(
        Which::Wm,
        "canvas.wallpaper".into(),
        Value::String(path.to_string_lossy().into_owned()),
    )];
    match looks.get(&path.to_string_lossy().into_owned()) {
        Some(look) => {
            let values = [&look.source, &look.variant, &look.mode, &look.primary, &look.secondary, &look.tertiary];
            for (key, value) in KEYS.iter().zip(values) {
                if !value.is_empty() || !["source", "variant", "mode"].contains(key) {
                    changes.push((Which::Shell, format!("theme.{key}"), Value::String(value.clone())));
                }
            }
            let colors: toml::Table = look.colors.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect();
            changes.push((Which::Shell, "theme.colors".into(), Value::Table(colors)));
        }
        // a new one takes its colors from itself
        None => changes.push((Which::Shell, "theme.source".into(), Value::String("wallpaper".into()))),
    }
    save_looks(&looks);
    let borrowed: Vec<(Which, &str, Value)> = changes.iter().map(|(w, p, v)| (*w, p.as_str(), v.clone())).collect();
    set_now(&borrowed)
}
