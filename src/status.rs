//! system status for the bar read on a worker thread so a slow nmcli never freezes the shell

use std::path::Path;
use std::process::Command;
use std::cell::RefCell;
use std::sync::mpsc::{Sender, channel};

/// the quick readings taken together every couple secs
#[derive(Clone)]
pub struct Fast {
    pub volume: Option<Volume>,
    pub network: NetworkInfo,
    pub battery: Option<Battery>,
}

/// a reading from the worker
pub enum Update {
    Fast(Fast),
    /// the perf script json
    Perf(serde_json::Value),
    /// the osd reading after a change
    Osd(crate::osd::Reading),
    /// whos using the mic and the camera
    Privacy(Privacy),
}

/// apps recording u right now by name
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Privacy {
    pub mic: Vec<String>,
    pub camera: Vec<String>,
}

/// work for the worker
pub enum Request {
    Fast,
    /// run this perf script
    Perf(String),
    /// change the volume by a step up to a max then read everything again
    ChangeVolume(i32, u32),
    /// check the mic and camera
    Privacy,
    /// an osd key so make the change and read the level back
    Osd {
        what: String,
        how: Option<String>,
        step: i32,
        max: u32,
    },
}

thread_local! {
    /// the readings end waits here till watch takes it
    static WORKER: (Sender<Request>, RefCell<Option<crate::wake::Receiver<Update>>>) = start_worker();
}

/// ask the worker for readings which come back thru updates
pub fn request(request: Request) {
    WORKER.with(|w| {
        let _ = w.0.send(request);
    });
}

/// run f on the gtk loop for each reading the worker finishes and only the first call gets them
pub fn watch(f: impl FnMut(Update) -> gtk4::glib::ControlFlow + 'static) {
    if let Some(updates) = WORKER.with(|w| w.1.borrow_mut().take()) {
        crate::wake::each(updates, f);
    }
}

fn start_worker() -> (Sender<Request>, RefCell<Option<crate::wake::Receiver<Update>>>) {
    let (requests, inbox) = channel::<Request>();
    let (outbox, updates) = crate::wake::channel();
    std::thread::spawn(move || {
        while let Ok(first) = inbox.recv() {
            // everything queued meanwhile gets done once
            let (mut fast, mut perf, mut privacy) = (false, None, false);
            for request in std::iter::once(first).chain(inbox.try_iter()) {
                match request {
                    Request::Fast => fast = true,
                    Request::Perf(command) => perf = Some(command),
                    Request::Privacy => privacy = true,
                    Request::ChangeVolume(step, max) => {
                        change_volume(step, max);
                        fast = true;
                    }
                    Request::Osd { what, how, step, max } => {
                        let reading = crate::osd::apply(&what, how.as_deref(), step, max);
                        if let Some(reading) = reading
                            && outbox.unbounded_send(Update::Osd(reading)).is_err()
                        {
                            return;
                        }
                        // the bar volume changed too
                        fast |= what == "volume";
                    }
                }
            }
            if fast {
                let reading = Fast {
                    volume: volume(),
                    network: network(),
                    battery: battery(),
                };
                if outbox.unbounded_send(Update::Fast(reading)).is_err() {
                    return;
                }
            }
            if privacy && outbox.unbounded_send(Update::Privacy(privacy_now())).is_err() {
                return;
            }
            if let Some(command) = perf
                && let Ok(output) = Command::new("sh").args(["-c", &command]).output()
                && let Ok(json) = serde_json::from_slice(&output.stdout)
                && outbox.unbounded_send(Update::Perf(json)).is_err()
            {
                return;
            }
        }
    });
    (requests, RefCell::new(Some(updates)))
}

#[derive(Clone)]
pub struct Volume {
    pub percent: u32,
    pub muted: bool,
}

/// the default output volume from wpctl
pub fn volume() -> Option<Volume> {
    wpctl_volume("@DEFAULT_AUDIO_SINK@")
}

fn wpctl_volume(node: &str) -> Option<Volume> {
    let output = Command::new("wpctl")
        .args(["get-volume", node])
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let level: f64 = text.split_whitespace().nth(1)?.parse().ok()?;
    Some(Volume {
        percent: (level * 100.0).round() as u32,
        muted: text.contains("MUTED"),
    })
}

/// the default uhh mic volume
pub fn mic() -> Option<Volume> {
    wpctl_volume("@DEFAULT_AUDIO_SOURCE@")
}

/// change the output volume by step percent up to max maybe
pub fn change_volume(step: i32, max: u32) {
    let arg = if step >= 0 {
        format!("{step}%+")
    } else {
        format!("{}%-", -step)
    };
    let _ = Command::new("wpctl")
        .args([
            "set-volume",
            "-l",
            &format!("{:.2}", max as f64 / 100.0),
            "@DEFAULT_AUDIO_SINK@",
            &arg,
        ])
        .status();
}

/// the active connection like waybars network module shows it
#[derive(Clone)]
pub enum Network {
    /// wifi name
    Wireless(String),
    Wired,
    Offline,
}

#[derive(Clone)]
pub struct NetworkInfo {
    pub network: Network,
    /// interface and ipv4 address for the tooltip
    pub interface: String,
    pub address: String,
}

/// the first active wifi or ethernet connection from networkmanager
pub fn network() -> NetworkInfo {
    let offline = NetworkInfo {
        network: Network::Offline,
        interface: String::new(),
        address: String::new(),
    };
    let Ok(output) = Command::new("nmcli")
        .args([
            "-t",
            "-f",
            "TYPE,NAME,DEVICE",
            "connection",
            "show",
            "--active",
        ])
        .output()
    else {
        return offline;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let fields = terse_fields(line);
        let [kind, name, device] = fields.as_slice() else {
            continue;
        };
        let network = match kind.as_str() {
            "802-11-wireless" => Network::Wireless(name.clone()),
            "802-3-ethernet" => Network::Wired,
            _ => continue,
        };
        return NetworkInfo {
            network,
            address: address_of(device).unwrap_or_default(),
            interface: device.to_string(),
        };
    }
    offline
}

/// split a line of nmcli -t output on colons but and inside a field are literal
fn terse_fields(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    fields.last_mut().expect("never empty").push(next);
                }
            }
            ':' => fields.push(String::new()),
            c => fields.last_mut().expect("never empty").push(c),
        }
    }
    fields
}

/// an interface ipv4 address from ip -4 -o addr show
fn address_of(interface: &str) -> Option<String> {
    let output = Command::new("ip")
        .args(["-4", "-o", "addr", "show", "dev", interface])
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let cidr = text
        .split_whitespace()
        .skip_while(|w| *w != "inet")
        .nth(1)?;
    Some(cidr.split('/').next()?.to_string())
}

#[derive(Clone)]
pub struct Battery {
    pub percent: u32,
    pub charging: bool,
}

/// the first battery if there is one
pub fn battery() -> Option<Battery> {
    let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
    let battery = dir
        .flatten()
        .map(|e| e.path())
        .find(|p| read(p, "type").as_deref() == Some("Battery"))?;
    Some(Battery {
        percent: read(&battery, "capacity")?.parse().ok()?,
        charging: read(&battery, "status").as_deref() == Some("Charging"),
    })
}

fn read(dir: &Path, file: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(file))
        .ok()
        .map(|s| s.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colons_in_wifi_names() {
        assert_eq!(
            terse_fields(r"802-11-wireless:Cafe\: Free:wlan0"),
            ["802-11-wireless", "Cafe: Free", "wlan0"]
        );
        assert_eq!(terse_fields(r"a\\b:c"), [r"a\b", "c"]);
    }
}

/// the apps recording from a real mic and the programs holding a camera open
pub fn privacy_now() -> Privacy {
    Privacy {
        mic: mic_users(),
        camera: camera_users(),
    }
}

/// apps w a recording stream on a mic but not on a speaker monitor like cava or a peak meter
fn mic_users() -> Vec<String> {
    let json = |args: &[&str]| -> Option<serde_json::Value> {
        let out = Command::new("pactl").args(args).output().ok()?;
        serde_json::from_slice(&out.stdout).ok()
    };
    let Some(sources) = json(&["-f", "json", "list", "sources"]) else {
        return Vec::new();
    };
    let monitors: Vec<u64> = sources
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["name"].as_str().is_some_and(|n| n.ends_with(".monitor")))
        .filter_map(|s| s["index"].as_u64())
        .collect();
    let Some(outputs) = json(&["-f", "json", "list", "source-outputs"]) else {
        return Vec::new();
    };
    let mut names: Vec<String> = outputs
        .as_array()
        .into_iter()
        .flatten()
        .filter(|o| o["source"].as_u64().is_some_and(|s| !monitors.contains(&s)))
        .filter(|o| !o["properties"]["media.name"].as_str().unwrap_or("").contains("Peak detect"))
        .filter_map(|o| {
            let p = &o["properties"];
            p["application.name"]
                .as_str()
                .or(p["application.process.binary"].as_str())
                .map(String::from)
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

/// programs w a /dev/video device open found by looking thru every process
fn camera_users() -> Vec<String> {
    let mut names = Vec::new();
    let Ok(procs) = std::fs::read_dir("/proc") else {
        return names;
    };
    for entry in procs.flatten() {
        let pid = entry.file_name();
        if !pid.to_string_lossy().chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Ok(fds) = std::fs::read_dir(entry.path().join("fd")) else {
            continue;
        };
        let holds = fds.flatten().any(|fd| {
            std::fs::read_link(fd.path()).is_ok_and(|t| t.to_string_lossy().starts_with("/dev/video"))
        });
        if holds && let Ok(comm) = std::fs::read_to_string(entry.path().join("comm")) {
            names.push(comm.trim().to_string());
        }
    }
    names.sort();
    names.dedup();
    names
}
