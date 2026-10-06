//! the on off things quick settings and bar buttons flip like caffeine night light or do not disturb

use serde_json::json;
use toml::Value;

use crate::config;
use crate::ipc;
use crate::settings::store::{Which, set_now};

/// something u can switch w its icon and name
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Caffeine,
    DoNotDisturb,
    NightLight,
    AntiFlashbang,
    DarkMode,
    BarAutohide,
    MicMute,
}

pub const ALL: [Toggle; 7] = [
    Toggle::Caffeine,
    Toggle::DoNotDisturb,
    Toggle::NightLight,
    Toggle::AntiFlashbang,
    Toggle::DarkMode,
    Toggle::BarAutohide,
    Toggle::MicMute,
];

impl Toggle {
    pub fn name(self) -> &'static str {
        match self {
            Toggle::Caffeine => "Caffeine",
            Toggle::DoNotDisturb => "Do not disturb",
            Toggle::NightLight => "Night light",
            Toggle::AntiFlashbang => "Anti-flashbang",
            Toggle::DarkMode => "Dark mode",
            Toggle::BarAutohide => "Hide the bar",
            Toggle::MicMute => "Mute mic",
        }
    }

    /// a material symbols icon for it
    pub fn icon(self) -> &'static str {
        match self {
            Toggle::Caffeine => "coffee",
            Toggle::DoNotDisturb => "do_not_disturb_on",
            Toggle::NightLight => "nightlight",
            Toggle::AntiFlashbang => "brightness_4",
            Toggle::DarkMode => "dark_mode",
            Toggle::BarAutohide => "vertical_align_top",
            Toggle::MicMute => "mic_off",
        }
    }

    /// what a hover says about it
    pub fn hint(self) -> &'static str {
        match self {
            Toggle::Caffeine => "Keep the screen awake",
            Toggle::DoNotDisturb => "Only critical notifications pop up",
            Toggle::NightLight => "Warmer colors that are easier on the eyes",
            Toggle::AntiFlashbang => "Dim the brightest parts of windows",
            Toggle::DarkMode => "Dark or light colors everywhere",
            Toggle::BarAutohide => "Tuck the bar away till the mouse touches its edge",
            Toggle::MicMute => "Mute the microphone",
        }
    }

    /// whether its on right now
    pub fn is_on(self) -> bool {
        let state = ipc::latest();
        let c = config::get();
        match self {
            Toggle::Caffeine => state.is_some_and(|s| s.caffeine),
            Toggle::DoNotDisturb => c.notifications.do_not_disturb,
            Toggle::NightLight => state.is_some_and(|s| s.night_light_enabled),
            Toggle::AntiFlashbang => state.is_some_and(|s| s.anti_flashbang),
            Toggle::DarkMode => c.theme.mode == "dark",
            Toggle::BarAutohide => c.bar.autohide,
            Toggle::MicMute => crate::status::mic().is_some_and(|m| m.muted),
        }
    }

    /// switch it and say what went wrong if it didnt
    pub fn flip(self) -> Result<(), String> {
        let on = !self.is_on();
        match self {
            Toggle::Caffeine => ipc::request(json!({ "caffeine": on })).map(|_| ()),
            Toggle::DoNotDisturb => set_now(&[(Which::Shell, "notifications.do_not_disturb", Value::Boolean(on))]),
            Toggle::NightLight => set_now(&[(Which::Wm, "night_light.enabled", Value::Boolean(on))]),
            Toggle::AntiFlashbang => set_now(&[(Which::Wm, "anti_flashbang.enabled", Value::Boolean(on))]),
            Toggle::DarkMode => {
                let mode = if on { "dark" } else { "light" };
                set_now(&[(Which::Shell, "theme.mode", Value::String(mode.into()))])
            }
            Toggle::BarAutohide => set_now(&[(Which::Shell, "bar.autohide", Value::Boolean(on))]),
            Toggle::MicMute => std::process::Command::new("wpctl")
                .args(["set-mute", "@DEFAULT_AUDIO_SOURCE@", if on { "1" } else { "0" }])
                .status()
                .map_err(|e| e.to_string())
                .and_then(|s| if s.success() { Ok(()) } else { Err("wpctl failed".into()) }),
        }
    }
}

/// flip it and tell the user if that failed
pub fn flip(toggle: Toggle) {
    if let Err(err) = toggle.flip() {
        crate::run_detached(std::process::Command::new("notify-send").args([
            "-a",
            "sevenshell",
            &format!("Couldn't switch {}", toggle.name().to_lowercase()),
            &err,
        ]));
    }
}
