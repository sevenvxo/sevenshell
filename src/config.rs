//! sevenshell config from ur file over the defaults and reread when it changes

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::SystemTime;

use serde::Deserialize;

pub const DEFAULTS: &str = include_str!("../config.default.toml");

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub bar: Bar,
    pub notifications: Notifications,
    pub osd: Osd,
    pub lock: Lock,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Lock {
    pub wallpaper: String,
    pub avatar: String,
    pub clock_format: String,
    pub date_format: String,
    pub message: String,
    pub lock_before_sleep: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Bar {
    pub left: Vec<String>,
    pub center: Vec<String>,
    pub right: Vec<String>,
    pub clock_format: String,
    pub terminal: String,
    pub tray: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Notifications {
    pub do_not_disturb: bool,
    pub timeout: u32,
    pub position: Position,
    pub margin_x: i32,
    pub margin_y: i32,
    pub width: i32,
    pub height: i32,
    pub history: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Osd {
    pub step: u32,
    pub duration: u32,
    pub position: Position,
    pub max_volume: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Position {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Position {
    /// which screen edges a layer surface is anchored to
    pub fn anchors(self) -> (bool, bool, bool, bool) {
        use Position::*;
        let top = matches!(self, TopLeft | Top | TopRight);
        let bottom = matches!(self, BottomLeft | Bottom | BottomRight);
        let left = matches!(self, TopLeft | Left | BottomLeft);
        let right = matches!(self, TopRight | Right | BottomRight);
        (top, bottom, left, right)
    }
}

/// parse a config file merged over the uhh defaults
pub fn parse(text: &str) -> Result<Config, String> {
    let mut merged: toml::Table = DEFAULTS.parse().expect("the default config parses");
    let user: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    merge(&mut merged, user);
    let config: Config = toml::Value::Table(merged)
        .try_into()
        .map_err(|e: toml::de::Error| e.to_string())?;
    for module in config
        .bar
        .left
        .iter()
        .chain(&config.bar.center)
        .chain(&config.bar.right)
    {
        if !crate::bar::MODULES.contains(&module.as_str()) {
            return Err(format!("unknown bar module '{module}'"));
        }
    }
    Ok(config)
}

fn merge(base: &mut toml::Table, over: toml::Table) {
    for (key, value) in over {
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(base)), toml::Value::Table(over)) => merge(base, over),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

pub fn path() -> PathBuf {
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"));
    dir.join("sevenshell/config.toml")
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

thread_local! {
    /// the config and the file mtime when last read
    static CURRENT: RefCell<(Rc<Config>, Option<Option<SystemTime>>)> =
        RefCell::new((Rc::new(parse("").unwrap()), None));
}

/// the config in use
pub fn get() -> Rc<Config> {
    CURRENT.with(|c| c.borrow().0.clone())
}

/// reread the file if it changed and keep the old config if the new one is broken
pub fn reload() -> Option<Rc<Config>> {
    let path = path();
    let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    if CURRENT.with(|c| c.borrow().1) == Some(mtime) {
        return None;
    }
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let config = match parse(&text) {
        Ok(config) => config,
        Err(err) => {
            eprintln!(
                "sevenshell: {}: {err}; keeping the previous config",
                path.display()
            );
            CURRENT.with(|c| c.borrow_mut().1 = Some(mtime));
            return None;
        }
    };
    CURRENT.with(|c| {
        let mut current = c.borrow_mut();
        current.1 = Some(mtime);
        if *current.0 == config {
            None
        } else {
            current.0 = Rc::new(config);
            Some(current.0.clone())
        }
    })
}

/// this does a thing where it puts the defaults where the settings app looks
pub fn publish_defaults() {
    let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") else {
        return;
    };
    let dir = PathBuf::from(dir).join("sevenshell");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("defaults.toml"), DEFAULTS);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_parse() {
        let config = parse("").unwrap();
        assert_eq!(config.bar.left, ["desktop"]);
        assert_eq!(config.notifications.position, Position::TopRight);
        assert_eq!(config.osd.step, 5);
    }

    #[test]
    fn user_values_override_one_key() {
        let config = parse("[notifications]\ntimeout = 9000\n").unwrap();
        assert_eq!(config.notifications.timeout, 9000);
        assert_eq!(config.notifications.width, 360);
    }

    #[test]
    fn mistakes_are_errors() {
        assert!(parse("[bar]\nleft = [\"nope\"]\n").is_err());
        assert!(parse("[notifications]\nposition = \"middle\"\n").is_err());
        assert!(parse("[osd]\nstpe = 3\n").is_err());
    }

    #[test]
    fn anchors() {
        assert_eq!(Position::Top.anchors(), (true, false, false, false));
        assert_eq!(Position::BottomLeft.anchors(), (false, true, true, false));
        assert_eq!(Position::Center.anchors(), (false, false, false, false));
        assert_eq!(Position::Right.anchors(), (false, false, false, true));
    }
}
