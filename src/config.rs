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
    pub theme: Theme,
    pub drives: Drives,
    pub weather: Weather,
    pub launcher: LauncherConfig,
    pub clipboard: Clipboard,
    pub wallpapers: Wallpapers,
    pub updates: Updates,
    pub polkit: Polkit,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Weather {
    /// a city or empty to guess from ur ip
    pub location: String,
    /// metric or imperial
    pub units: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LauncherConfig {
    /// where the web search row goes w %s for what u typed
    pub search_url: String,
    /// = or plain math like 2*8 shows the answer
    pub calculator: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Clipboard {
    /// keep what u copy thru cliphist so mod+v can paste old things
    pub history: bool,
    pub max_items: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Wallpapers {
    /// the folder the quick settings wallpapers come from
    pub dir: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Updates {
    /// what clicking the updates module runs
    pub command: String,
    pub interval_minutes: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Polkit {
    /// ask for ur password when an app needs admin rights
    pub agent: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Drives {
    pub automount: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Theme {
    pub source: String,
    pub wallpaper: String,
    pub variant: String,
    pub mode: String,
    pub primary: String,
    pub secondary: String,
    pub tertiary: String,
    pub color_sevenwm: bool,
    pub color_gtk: bool,
    /// exact colors by role name like m3onSurface that win over the palette
    #[serde(default)]
    pub colors: std::collections::BTreeMap<String, String>,
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
    /// top bottom left or right
    pub position: String,
    /// tucked away till the mouse touches that edge
    pub autohide: bool,
    /// a rounded bar w a gap around it
    pub floating: bool,
    /// 0 see thru to 1 solid
    pub opacity: f64,
    pub height: i32,
    pub left: Vec<String>,
    pub center: Vec<String>,
    pub right: Vec<String>,
    pub clock_format: String,
    pub terminal: String,
    pub tray: String,
    pub cava_bars: usize,
    #[serde(default)]
    pub custom: Vec<Custom>,
}

/// ur own module that shows what a command prints and goes in the bar as custom/name
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Custom {
    pub name: String,
    pub exec: String,
    /// seconds between runs and 0 means it keeps running and each line it prints shows
    #[serde(default = "five")]
    pub interval: u64,
    #[serde(default)]
    pub on_click: String,
    /// a material symbols icon name shown in front
    #[serde(default)]
    pub icon: String,
}

fn five() -> u64 {
    5
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
    if !(20..=96).contains(&config.bar.height) {
        return Err(format!("bar.height should be 20 to 96 not {}", config.bar.height));
    }
    let t = &config.theme;
    if t.source != "wallpaper" && crate::theme::parse_hex(&t.source).is_none() {
        return Err(format!("theme.source should be wallpaper or a color like #6750a4 not '{}'", t.source));
    }
    if !crate::theme::VARIANTS.contains(&t.variant.as_str()) {
        return Err(format!("theme.variant should be one of {} not '{}'", crate::theme::VARIANTS.join(" "), t.variant));
    }
    if t.mode != "dark" && t.mode != "light" {
        return Err(format!("theme.mode should be dark or light not '{}'", t.mode));
    }
    for (name, value) in &t.colors {
        if !crate::theme::ROLES.iter().any(|(r, _)| r == name) {
            return Err(format!("theme.colors has no color called '{name}'"));
        }
        if crate::theme::parse_hex(value).is_none() {
            return Err(format!("theme.colors.{name} should be a color like #ff8800 not '{value}'"));
        }
    }
    for (name, value) in [("primary", &t.primary), ("secondary", &t.secondary), ("tertiary", &t.tertiary)] {
        if !value.is_empty() && crate::theme::parse_hex(value).is_none() {
            return Err(format!("theme.{name} should be empty or a color like #ff8800 not '{value}'"));
        }
    }
    if !["top", "bottom", "left", "right"].contains(&config.bar.position.as_str()) {
        return Err(format!("bar.position should be top bottom left or right not '{}'", config.bar.position));
    }
    if !(0.0..=1.0).contains(&config.bar.opacity) {
        return Err(format!("bar.opacity should be 0 to 1 not {}", config.bar.opacity));
    }
    if config.weather.units != "metric" && config.weather.units != "imperial" {
        return Err(format!("weather.units should be metric or imperial not '{}'", config.weather.units));
    }
    if !(1..=10000).contains(&config.clipboard.max_items) {
        return Err(format!("clipboard.max_items should be 1 to 10000 not {}", config.clipboard.max_items));
    }
    if config.updates.interval_minutes == 0 {
        return Err("updates.interval_minutes should be at least 1".into());
    }
    if !(1..=64).contains(&config.bar.cava_bars) {
        return Err(format!("bar.cava_bars should be 1 to 64 not {}", config.bar.cava_bars));
    }
    for (i, custom) in config.bar.custom.iter().enumerate() {
        if custom.name.is_empty() || custom.exec.is_empty() {
            return Err("every bar.custom module needs a name and an exec".into());
        }
        if config.bar.custom[..i].iter().any(|c| c.name == custom.name) {
            return Err(format!("two bar.custom modules are called '{}'", custom.name));
        }
    }
    for module in config
        .bar
        .left
        .iter()
        .chain(&config.bar.center)
        .chain(&config.bar.right)
    {
        let custom = module
            .strip_prefix("custom/")
            .is_some_and(|name| config.bar.custom.iter().any(|c| c.name == name));
        if !custom && !crate::bar::MODULES.contains(&module.as_str()) {
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
    /// why the file last failed to load so the shell can show it once it can
    static ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// the newest load error if theres one nobody has shown yet
pub fn take_error() -> Option<String> {
    ERROR.with(|e| e.borrow_mut().take())
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
            ERROR.with(|e| *e.borrow_mut() = Some(err.clone()));
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
        assert_eq!(config.bar.left, ["desktop", "media"]);
        assert_eq!(config.bar.position, "top");
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
        assert!(parse("[bar]\nleft = [\"custom/x\"]\n").is_err());
    }

    #[test]
    fn custom_modules() {
        let config = parse("[bar]\nleft = [\"custom/up\", \"cava\"]\n[[bar.custom]]\nname = \"up\"\nexec = \"uptime\"\n").unwrap();
        assert_eq!(config.bar.custom[0].interval, 5);
    }

    #[test]
    fn anchors() {
        assert_eq!(Position::Top.anchors(), (true, false, false, false));
        assert_eq!(Position::BottomLeft.anchors(), (false, true, true, false));
        assert_eq!(Position::Center.anchors(), (false, false, false, false));
        assert_eq!(Position::Right.anchors(), (false, false, false, true));
    }
}
