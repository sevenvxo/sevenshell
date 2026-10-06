//! whats playing thru mpris w playerctl for the bar module and its popup

use std::path::PathBuf;
use std::rc::Rc;

/// what playerctl prints for a track and tabs split the fields
const FORMAT: &str = "{{status}}\t{{playerName}}\t{{artist}}\t{{title}}\t{{album}}\t{{mpris:artUrl}}\t{{mpris:length}}";

/// one line per change for as long as sevenshell runs
pub fn feed() -> Rc<crate::feed::Feed> {
    crate::feed::get(&format!("exec playerctl -F metadata --format '{FORMAT}'"), 0)
}

/// the track playing right now read once
pub fn now() -> Option<Track> {
    let out = std::process::Command::new("playerctl")
        .args(["metadata", "--format", FORMAT])
        .output()
        .ok()?;
    Track::parse(String::from_utf8_lossy(&out.stdout).trim_end())
}

/// where playback is in seconds
pub fn position() -> Option<f64> {
    let out = std::process::Command::new("playerctl").arg("position").output().ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub playing: bool,
    pub player: String,
    pub artist: String,
    pub title: String,
    pub album: String,
    pub art: String,
    /// seconds and 0 when the player doesnt say
    pub length: f64,
}

impl Track {
    pub fn parse(line: &str) -> Option<Self> {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 7 || f[3].trim().is_empty() {
            return None;
        }
        // stopped players still hang around but theres nothing to show
        if f[0] == "Stopped" {
            return None;
        }
        Some(Self {
            playing: f[0] == "Playing",
            player: f[1].to_string(),
            artist: f[2].to_string(),
            title: f[3].to_string(),
            album: f[4].to_string(),
            art: f[5].to_string(),
            length: f[6].trim().parse::<f64>().unwrap_or(0.0) / 1_000_000.0,
        })
    }

    /// title and artist short enough for the bar
    pub fn short(&self) -> String {
        if self.artist.is_empty() {
            self.title.clone()
        } else {
            format!("{} · {}", self.title, self.artist)
        }
    }

    pub fn long(&self) -> String {
        [self.title.as_str(), self.artist.as_str(), self.album.as_str()]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// a local file w the cover which it downloads first if its on the web
    pub fn art_file(&self) -> Option<PathBuf> {
        if let Some(path) = self.art.strip_prefix("file://") {
            let path = PathBuf::from(unescape(path));
            return path.is_file().then_some(path);
        }
        if !self.art.starts_with("http") {
            return None;
        }
        let dir = cache_dir();
        std::fs::create_dir_all(&dir).ok()?;
        let name = format!("{:x}", hash(&self.art));
        let path = dir.join(name);
        if !path.is_file() {
            let ok = std::process::Command::new("curl")
                .args(["-sfL", "--max-time", "5", "-o"])
                .arg(&path)
                .arg(&self.art)
                .status()
                .is_ok_and(|s| s.success());
            if !ok {
                let _ = std::fs::remove_file(&path);
                return None;
            }
        }
        Some(path)
    }
}

fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_default()
        .join("sevenshell/covers")
}

fn hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// %20 style escapes in a file url back to plain text
fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&text[i + 1..i + 3], 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// m ss for a time in seconds
pub fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_playerctl_line_parses() {
        let t = Track::parse("Playing\tspotify\tAvril Lavigne\tComplicated\tLet Go\thttps://i.scdn.co/x\t244000000").unwrap();
        assert!(t.playing);
        assert_eq!(t.short(), "Complicated · Avril Lavigne");
        assert_eq!(t.length, 244.0);
        assert!(Track::parse("").is_none());
        assert!(Track::parse("Stopped\tmpv\t\tsong\t\t\t0").is_none());
    }

    #[test]
    fn file_urls_unescape() {
        assert_eq!(unescape("/a%20b.png"), "/a b.png");
        assert_eq!(clock(125.0), "2:05");
    }
}
