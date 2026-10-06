//! the weather from wttr.in which needs no key and guesses where u are from ur ip

use std::rc::Rc;

use serde_json::Value;

use crate::config;

/// the report as one line every 15 minutes
pub fn feed() -> Rc<crate::feed::Feed> {
    let w = &config::get().weather;
    // percent encoded so a name like val-d'or cant break out of the quotes or the url
    let place: String = w
        .location
        .trim()
        .bytes()
        .map(|b| match b {
            b' ' => "+".to_string(),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b',' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    // j1 is the full json and tr makes it one line so the feed keeps all of it
    let command = format!("curl -sf --max-time 10 'https://wttr.in/{place}?format=j1' | tr -d '\\n'");
    crate::feed::get(&command, 15 * 60)
}

#[derive(Clone, Debug)]
pub struct Now {
    pub temp: String,
    pub feels: String,
    pub description: String,
    pub icon: &'static str,
    pub humidity: String,
    pub wind: String,
}

#[derive(Clone, Debug)]
pub struct Day {
    /// like Mon
    pub name: String,
    pub high: String,
    pub low: String,
    pub icon: &'static str,
    pub description: String,
}

#[derive(Clone, Debug)]
pub struct Report {
    pub place: String,
    pub now: Now,
    pub days: Vec<Day>,
}

impl Report {
    pub fn parse(line: &str) -> Option<Self> {
        let json: Value = serde_json::from_str(line).ok()?;
        let imperial = config::get().weather.units == "imperial";
        let (t, speed, unit) = if imperial { ("F", "windspeedMiles", "mph") } else { ("C", "windspeedKmph", "km/h") };
        let current = json["current_condition"].get(0)?;
        let code = |v: &Value| v.as_str().and_then(|s| s.parse::<u32>().ok()).unwrap_or(113);
        let degrees = |v: &Value| format!("{}°", v.as_str().unwrap_or("?"));
        let now = Now {
            temp: degrees(&current[format!("temp_{t}")]),
            feels: degrees(&current[format!("FeelsLike{t}")]),
            description: current["weatherDesc"][0]["value"].as_str().unwrap_or("").trim().to_string(),
            icon: icon(code(&current["weatherCode"])),
            humidity: format!("{}%", current["humidity"].as_str().unwrap_or("?")),
            wind: format!("{} {unit}", current[speed].as_str().unwrap_or("?")),
        };
        let days = json["weather"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|d| {
                // the midday reading stands for the day
                let noon = &d["hourly"][4];
                Day {
                    name: weekday(d["date"].as_str().unwrap_or("")),
                    high: degrees(&d[format!("maxtemp{t}")]),
                    low: degrees(&d[format!("mintemp{t}")]),
                    icon: icon(code(&noon["weatherCode"])),
                    description: noon["weatherDesc"][0]["value"].as_str().unwrap_or("").trim().to_string(),
                }
            })
            .collect();
        let area = &json["nearest_area"][0];
        let place = [&area["areaName"][0]["value"], &area["region"][0]["value"]]
            .iter()
            .filter_map(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        Some(Self { place, now, days })
    }
}

/// short weekday name from yyyy-mm-dd
fn weekday(date: &str) -> String {
    let mut parts = date.split('-').map(|p| p.parse::<i32>().unwrap_or(0));
    let (Some(y), Some(m), Some(d)) = (parts.next(), parts.next(), parts.next()) else {
        return date.to_string();
    };
    gtk4::glib::DateTime::from_local(y, m, d, 12, 0, 0.0)
        .ok()
        .and_then(|dt| dt.format("%a").ok())
        .map_or(date.to_string(), |s| s.to_string())
}

/// a material symbols icon for a wttr.in weather code
pub fn icon(code: u32) -> &'static str {
    match code {
        113 => "clear_day",
        116 => "partly_cloudy_day",
        119 | 122 => "cloud",
        143 | 248 | 260 => "foggy",
        176 | 263 | 266 | 293 | 296 | 353 => "rainy_light",
        299 | 302 | 305 | 308 | 356 | 359 => "rainy_heavy",
        179 | 182 | 185 | 281 | 284 | 311 | 314 | 317 | 320 | 350 | 362 | 365 | 374 | 377 => "weather_mix",
        227 | 230 | 323 | 326 | 329 | 332 | 335 | 338 | 368 | 371 => "weather_snowy",
        200 | 386 | 389 | 392 | 395 => "thunderstorm",
        _ => "cloud",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wttr_report_parses() {
        let line = r#"{"current_condition":[{"temp_C":"12","FeelsLikeC":"10","weatherCode":"116","weatherDesc":[{"value":"Partly cloudy"}],"humidity":"70","windspeedKmph":"9","temp_F":"54","FeelsLikeF":"50","windspeedMiles":"6"}],"nearest_area":[{"areaName":[{"value":"Toronto"}],"region":[{"value":"Ontario"}]}],"weather":[{"date":"2026-09-29","maxtempC":"15","mintempC":"8","maxtempF":"59","mintempF":"46","hourly":[{},{},{},{},{"weatherCode":"113","weatherDesc":[{"value":"Sunny"}]}]}]}"#;
        let r = Report::parse(line).unwrap();
        assert_eq!(r.now.temp, "12°");
        assert_eq!(r.now.icon, "partly_cloudy_day");
        assert_eq!(r.place, "Toronto, Ontario");
        assert_eq!(r.days[0].high, "15°");
        assert_eq!(r.days[0].icon, "clear_day");
        assert!(Report::parse("").is_none());
    }
}
