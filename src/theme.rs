//! the material you palette made from ur wallpaper or colors u pick and shared by every ui as css colors

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

use material_colors::color::Argb;
use material_colors::dynamic_color::Variant;
use material_colors::image::{AsPixels, FilterType, ImageReader};
use material_colors::quantize::{Quantizer, QuantizerCelebi};
use material_colors::scheme::Scheme;
use material_colors::theme::ThemeBuilder;
use serde_json::{Map, Value, json};

use crate::config;

/// the scheme variants u can pick by name
pub const VARIANTS: [&str; 9] = [
    "content",
    "tonal-spot",
    "vibrant",
    "expressive",
    "fidelity",
    "neutral",
    "monochrome",
    "rainbow",
    "fruit-salad",
];

/// the seed used when theres no wallpaper to read
const FALLBACK_SEED: Argb = Argb::new(255, 0x6f, 0x8f, 0xbf);

/// a whole palette w every material role by its css name like m3primary
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    pub seed: Argb,
    pub colors: Vec<(&'static str, Argb)>,
}

impl Palette {
    /// one role by name like m3primary
    pub fn get(&self, name: &str) -> Argb {
        self.colors
            .iter()
            .find(|(n, _)| *n == name)
            .map_or(FALLBACK_SEED, |(_, c)| *c)
    }

    /// #rrggbb for a role
    pub fn hex(&self, name: &str) -> String {
        hex(self.get(name))
    }

    /// every role as a gtk named color so css can say @m3primary
    pub fn css(&self) -> String {
        let mut css = String::new();
        for (name, color) in &self.colors {
            css.push_str(&format!("@define-color {name} {};\n", hex(*color)));
        }
        css
    }

    /// the palette as json for other tools like the trays
    pub fn json(&self) -> Value {
        let colors: Map<String, Value> = self
            .colors
            .iter()
            .map(|(n, c)| (n.to_string(), Value::String(hex(*c))))
            .collect();
        json!({ "mode": if self.dark { "dark" } else { "light" }, "seed": hex(self.seed), "colors": colors })
    }

    /// what sevenwm draws in these colors like borders menus and title bars
    pub fn sevenwm_colors(&self) -> Value {
        let a = |name: &str, alpha: u8| format!("{}{alpha:02x}", self.hex(name));
        json!({
            "border.focused": self.hex("windowOutline"),
            "border.unfocused": a("windowOutline", 0x40),
            "canvas.background": self.hex("m3surfaceDim"),
            "canvas.region_outline": a("m3outlineVariant", 0xa0),
            "canvas.bounds_outline": a("m3outline", 0x60),
            "canvas.drop_highlight": a("m3primary", 0x30),
            "decorations.titlebar_focused": self.hex("windowTopbar"),
            "decorations.titlebar_unfocused": self.hex("windowTopbar"),
            "decorations.titlebar_text": self.hex("windowOnTopbar"),
            "theme.menu_background": self.hex("m3surfaceContainer"),
            "theme.menu_text": self.hex("m3onSurface"),
            "theme.menu_hover": self.hex("m3secondaryContainer"),
            "theme.menu_disabled": self.hex("m3outline"),
            "theme.menu_edge": self.hex("m3outlineVariant"),
        })
    }
}

pub fn hex(c: Argb) -> String {
    format!("#{:02x}{:02x}{:02x}", c.red, c.green, c.blue)
}

/// #rrggbb into a color or none if it doesnt parse
pub fn parse_hex(s: &str) -> Option<Argb> {
    let h = s.trim().strip_prefix('#').unwrap_or(s.trim());
    if h.len() != 6 {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some(Argb::new(255, byte(0)?, byte(2)?, byte(4)?))
}

fn variant(name: &str) -> Variant {
    match name {
        "vibrant" => Variant::Vibrant,
        "expressive" => Variant::Expressive,
        "tonal-spot" => Variant::TonalSpot,
        "fidelity" => Variant::Fidelity,
        "neutral" => Variant::Neutral,
        "monochrome" => Variant::Monochrome,
        "rainbow" => Variant::Rainbow,
        "fruit-salad" => Variant::FruitSalad,
        _ => Variant::Content,
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

pub fn expand(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None => PathBuf::from(path),
    }
}

/// the image the palette comes from which is the themes own or sevenwms wallpaper
pub fn wallpaper(theme: &config::Theme) -> Option<PathBuf> {
    if !theme.wallpaper.is_empty() {
        return Some(expand(&theme.wallpaper));
    }
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"));
    let text = std::fs::read_to_string(dir.join("sevenwm/config.toml")).ok()?;
    let table: toml::Table = text.parse().ok()?;
    let path = table.get("canvas")?.get("wallpaper")?.as_str()?;
    (!path.is_empty()).then(|| expand(path))
}

thread_local! {
    /// colors already pulled out of images by path and mtime bc decoding is slow
    static SEEDS: RefCell<HashMap<(PathBuf, Option<SystemTime>), Option<(Argb, Option<Argb>)>>> = RefCell::new(HashMap::new());
}

/// how many color groups an image gets boiled down to before counting so close shades count as one
const IMAGE_COLORS: usize = 8;

/// the most common color in an image and the second most common if theres one
pub fn colors_from_image(path: &PathBuf) -> Option<(Argb, Option<Argb>)> {
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let key = (path.clone(), mtime);
    if let Some(colors) = SEEDS.with(|s| s.borrow().get(&key).copied()) {
        return colors;
    }
    // a small copy is plenty to count colors and way faster
    let colors = ImageReader::open(path).ok().and_then(|mut image| {
        image.resize(128, 128, FilterType::Triangle);
        let result = QuantizerCelebi::quantize(&image.as_pixels(), IMAGE_COLORS);
        let mut counted: Vec<(Argb, u32)> = result.color_to_count.into_iter().collect();
        counted.sort_by(|a, b| b.1.cmp(&a.1));
        let first = counted.first()?.0;
        Some((first, counted.get(1).map(|c| c.0)))
    });
    SEEDS.with(|s| s.borrow_mut().insert(key, colors));
    colors
}

/// the main color and maybe a second one the settings say to use
pub fn seeds(theme: &config::Theme) -> (Argb, Option<Argb>) {
    if theme.source != "wallpaper" {
        return (parse_hex(&theme.source).unwrap_or(FALLBACK_SEED), None);
    }
    wallpaper(theme)
        .and_then(|p| colors_from_image(&p))
        .unwrap_or((FALLBACK_SEED, None))
}

/// the full palette from the theme settings
pub fn generate(theme: &config::Theme) -> Palette {
    // the wallpapers most common color leads and its second most common becomes the secondary
    let (seed, second) = seeds(theme);
    let mut builder = ThemeBuilder::with_source(seed).variant(variant(&theme.variant));
    if let Some(c) = second {
        builder = builder.secondary(c);
    }
    // accents u picked yourself win over the generated ones
    if let Some(c) = parse_hex(&theme.primary) {
        builder = builder.primary(c);
    }
    if let Some(c) = parse_hex(&theme.secondary) {
        builder = builder.secondary(c);
    }
    if let Some(c) = parse_hex(&theme.tertiary) {
        builder = builder.tertiary(c);
    }
    let built = builder.build();
    let dark = theme.mode != "light";
    let s: Scheme = if dark { built.schemes.dark } else { built.schemes.light };
    let colors = vec![
        ("m3primary", s.primary),
        ("m3onPrimary", s.on_primary),
        ("m3primaryContainer", s.primary_container),
        ("m3onPrimaryContainer", s.on_primary_container),
        ("m3inversePrimary", s.inverse_primary),
        ("m3secondary", s.secondary),
        ("m3onSecondary", s.on_secondary),
        ("m3secondaryContainer", s.secondary_container),
        ("m3onSecondaryContainer", s.on_secondary_container),
        ("m3tertiary", s.tertiary),
        ("m3onTertiary", s.on_tertiary),
        ("m3tertiaryContainer", s.tertiary_container),
        ("m3onTertiaryContainer", s.on_tertiary_container),
        ("m3error", s.error),
        ("m3onError", s.on_error),
        ("m3errorContainer", s.error_container),
        ("m3onErrorContainer", s.on_error_container),
        ("m3background", s.background),
        ("m3onBackground", s.on_background),
        ("m3surface", s.surface),
        ("m3surfaceDim", s.surface_dim),
        ("m3surfaceBright", s.surface_bright),
        ("m3surfaceContainerLowest", s.surface_container_lowest),
        ("m3surfaceContainerLow", s.surface_container_low),
        ("m3surfaceContainer", s.surface_container),
        ("m3surfaceContainerHigh", s.surface_container_high),
        ("m3surfaceContainerHighest", s.surface_container_highest),
        ("m3onSurface", s.on_surface),
        ("m3surfaceVariant", s.surface_variant),
        ("m3onSurfaceVariant", s.on_surface_variant),
        ("m3outline", s.outline),
        ("m3outlineVariant", s.outline_variant),
        ("m3inverseSurface", s.inverse_surface),
        ("m3inverseOnSurface", s.inverse_on_surface),
        ("m3shadow", s.shadow),
        ("m3scrim", s.scrim),
    ];
    let mut palette = Palette { dark, seed, colors };
    palette.apply(&theme.colors);
    let (topbar, on_topbar, outline) = window_colors(&palette);
    palette.colors.push(("windowTopbar", topbar));
    palette.colors.push(("windowOnTopbar", on_topbar));
    palette.colors.push(("windowOutline", outline));
    palette.apply(&theme.colors);
    palette
}

/// every color u can set urself w a name people get
pub const ROLES: [(&str, &str); 39] = [
    ("m3onSurface", "Text"),
    ("m3onSurfaceVariant", "Dim text and icons"),
    ("m3surface", "Background of the bar and panels"),
    ("m3surfaceDim", "Canvas"),
    ("m3surfaceBright", "Bright background"),
    ("m3surfaceContainerLowest", "Lowest raised background"),
    ("m3surfaceContainerLow", "Low raised background"),
    ("m3surfaceContainer", "Raised background like pills and menus"),
    ("m3surfaceContainerHigh", "High raised background like fields"),
    ("m3surfaceContainerHighest", "Highest raised background"),
    ("m3surfaceVariant", "Other background"),
    ("m3background", "Page background"),
    ("m3onBackground", "Text on page background"),
    ("m3primary", "Primary accent"),
    ("m3onPrimary", "Text on primary"),
    ("m3primaryContainer", "Primary container"),
    ("m3onPrimaryContainer", "Text on primary container"),
    ("m3inversePrimary", "Inverse primary"),
    ("m3secondary", "Secondary accent"),
    ("m3onSecondary", "Text on secondary"),
    ("m3secondaryContainer", "Selected and highlighted"),
    ("m3onSecondaryContainer", "Text on selected"),
    ("m3tertiary", "Tertiary accent"),
    ("m3onTertiary", "Text on tertiary"),
    ("m3tertiaryContainer", "Tertiary container"),
    ("m3onTertiaryContainer", "Text on tertiary container"),
    ("m3error", "Error"),
    ("m3onError", "Text on error"),
    ("m3errorContainer", "Error container"),
    ("m3onErrorContainer", "Text on error container"),
    ("m3outline", "Outline"),
    ("m3outlineVariant", "Faint outline"),
    ("m3inverseSurface", "Tooltip background"),
    ("m3inverseOnSurface", "Tooltip text"),
    ("m3shadow", "Shadow"),
    ("m3scrim", "Dim behind popups"),
    ("windowTopbar", "Window top bars"),
    ("windowOnTopbar", "Window top bar text"),
    ("windowOutline", "Focused window border"),
];

impl Palette {
    /// the colors u set urself replace the made ones
    fn apply(&mut self, colors: &std::collections::BTreeMap<String, String>) {
        for (name, color) in self.colors.iter_mut() {
            if let Some(c) = colors.get(*name).and_then(|v| parse_hex(v)) {
                *color = c;
            }
        }
    }
}

thread_local! {
    /// the palette in use so drawn widgets like the minimap can pick colors
    static CURRENT: RefCell<Option<Palette>> = const { RefCell::new(None) };
}

pub fn set_current(palette: &Palette) {
    CURRENT.with(|c| *c.borrow_mut() = Some(palette.clone()));
}

/// the palette in use or a default one before the first is made
pub fn current() -> Palette {
    CURRENT.with(|c| c.borrow().clone()).unwrap_or_else(|| generate(&crate::config::get().theme))
}

/// a color as cairo wants it w alpha
pub fn rgba(c: Argb, alpha: f64) -> (f64, f64, f64, f64) {
    (c.red as f64 / 255.0, c.green as f64 / 255.0, c.blue as f64 / 255.0, alpha)
}

/// a window top bar its text and its outline which are always different colors
/// so a black and white palette gets a black bar w a white outline and a colorful one gets its accent outline
fn window_colors(p: &Palette) -> (Argb, Argb, Argb) {
    let (black, white) = (Argb::new(255, 0, 0, 0), Argb::new(255, 255, 255, 255));
    let grey = material_colors::hct::Hct::new(p.get("m3primary")).get_chroma() < 8.0;
    match (grey, p.dark) {
        (true, true) => (black, white, white),
        (true, false) => (white, black, black),
        (false, _) => (p.get("m3surfaceContainerLowest"), p.get("m3onSurface"), p.get("m3primary")),
    }
}

/// what sevenshell writes at the top of gtk css files it owns so it never touches urs
const GTK_MARK: &str = "/* made by sevenshell from ur palette and it gets rewritten so turn off theme color_gtk to edit it urself */";

/// color gtk apps own top bars to match the window top bar in gtk 3 and 4
pub fn save_gtk(palette: &Palette, on: bool) {
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"));
    let (bar, text) = (palette.hex("windowTopbar"), palette.hex("windowOnTopbar"));
    let css = format!(
        "{GTK_MARK}
@define-color headerbar_bg_color {bar};
@define-color headerbar_fg_color {text};
@define-color headerbar_backdrop_color {bar};
headerbar, .titlebar, headerbar:backdrop, .titlebar:backdrop {{
    background: {bar};
    background-image: none;
    color: {text};
    box-shadow: none;
}}
headerbar .title, headerbar label {{ color: {text}; }}
"
    );
    for version in ["gtk-3.0", "gtk-4.0"] {
        let path = dir.join(version).join("gtk.css");
        let current = std::fs::read_to_string(&path).ok();
        // only files we made or none at all so ur own css is left alone
        if current.as_ref().is_some_and(|c| !c.starts_with(GTK_MARK)) {
            continue;
        }
        if !on {
            if current.is_some() {
                let _ = std::fs::remove_file(&path);
            }
            continue;
        }
        if current.as_deref() != Some(css.as_str()) {
            let _ = std::fs::create_dir_all(dir.join(version));
            let _ = std::fs::write(&path, &css);
        }
    }
}

/// where the palette gets saved for other tools like ur own scripts to read
pub fn colors_path() -> PathBuf {
    let dir = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/state"));
    dir.join("sevenshell/colors.json")
}

/// save the palette for other tools and do nothing if it didnt change
pub fn save(palette: &Palette) {
    let path = colors_path();
    let text = serde_json::to_string_pretty(&palette.json()).unwrap_or_default();
    if std::fs::read_to_string(&path).ok().as_deref() == Some(text.as_str()) {
        return;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // write it next to the file then swap it in so readers never see half of it
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// what the palette depends on so it only gets redone when one of these changes
#[derive(Clone, PartialEq)]
pub struct Inputs {
    theme: config::Theme,
    wallpaper: Option<(PathBuf, Option<SystemTime>)>,
}

pub fn inputs(theme: &config::Theme) -> Inputs {
    let wallpaper = (theme.source == "wallpaper")
        .then(|| wallpaper(theme))
        .flatten()
        .map(|p| {
            let mtime = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
            (p, mtime)
        });
    Inputs {
        theme: theme.clone(),
        wallpaper,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(source: &str, mode: &str) -> config::Theme {
        config::Theme {
            source: source.into(),
            wallpaper: String::new(),
            variant: "content".into(),
            mode: mode.into(),
            primary: String::new(),
            secondary: String::new(),
            tertiary: String::new(),
            color_sevenwm: true,
            color_gtk: true,
            colors: Default::default(),
        }
    }

    #[test]
    fn a_seed_makes_a_dark_palette_w_light_text() {
        let p = generate(&theme("#6750a4", "dark"));
        let lum = |c: Argb| c.red as u32 + c.green as u32 + c.blue as u32;
        assert!(lum(p.get("m3surface")) < lum(p.get("m3onSurface")));
        assert!(p.css().contains("@define-color m3primary #"));
    }

    #[test]
    fn a_grey_palette_gets_a_black_bar_and_a_white_outline() {
        let p = generate(&theme("#000000", "dark"));
        assert_eq!(p.hex("windowTopbar"), "#000000");
        assert_eq!(p.hex("windowOutline"), "#ffffff");
        assert_ne!(p.hex("windowTopbar"), p.hex("windowOutline"));
    }

    #[test]
    fn a_picked_primary_wins() {
        let mut t = theme("#6750a4", "dark");
        t.primary = "#ff0000".into();
        let picked = generate(&t).get("m3primary");
        assert!(picked.red > picked.green && picked.red > picked.blue);
    }
}

