//! talking to sevenwm over its ipc socket

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Debug, Default, Deserialize)]
// mirrors sevenwms ipc in full
#[allow(dead_code)]
pub struct State {
    pub active_monitor: Option<String>,
    pub monitors: Vec<Monitor>,
    pub windows: Vec<WindowInfo>,
    /// numbered tiled areas on the canvas
    #[serde(default)]
    pub workspaces: Vec<WorkspaceInfo>,
    pub locked: bool,
    /// the shell is keeping the screen awake
    #[serde(default)]
    pub caffeine: bool,
    /// an app is keeping the screen awake like a video
    #[serde(default)]
    pub inhibited: bool,
    /// something copied the screen in the last couple secs
    #[serde(default)]
    pub capturing: bool,
    /// night light is warming the screen right now
    #[serde(default)]
    pub night_light: bool,
    #[serde(default)]
    pub night_light_enabled: bool,
    #[serde(default)]
    pub anti_flashbang: bool,
}

#[derive(Clone, Debug, Deserialize)]
// mirrors sevenwms ipc in full
#[allow(dead_code)]
pub struct WorkspaceInfo {
    pub number: u32,
    /// x y width height on the canvas
    pub rect: [i32; 4],
    /// its tiles window ids main first
    pub tiles: Vec<u64>,
}

#[derive(Clone, Debug, Deserialize)]
// mirrors sevenwms ipc in full
#[allow(dead_code)]
pub struct Monitor {
    pub name: String,
    pub active: bool,
    pub position: [i32; 2],
    pub size: [i32; 2],
    pub camera: [f64; 2],
    pub zoom: f64,
    /// x y width height on the canvas
    pub region: [i32; 4],
    pub overview: bool,
}

#[derive(Clone, Debug, Deserialize)]
// mirrors sevenwms ipc in full
#[allow(dead_code)]
pub struct WindowInfo {
    pub id: u64,
    pub app_id: String,
    pub title: String,
    /// x y width height on the canvas
    pub rect: [i32; 4],
    pub tiled: bool,
    pub fullscreen: bool,
    pub focused: bool,
    pub hidden_from_screencast: bool,
    /// the owning process and none for x11 windows
    #[serde(default)]
    pub pid: Option<i32>,
    /// the workspace its tiled in or was if collapsed
    #[serde(default)]
    pub workspace: Option<u32>,
    /// folded into a marker
    #[serde(default)]
    pub collapsed: bool,
    /// focus order where 0 is the most recent
    #[serde(default)]
    pub recent: Option<usize>,
}

impl State {
    pub fn monitor(&self, name: &str) -> Option<&Monitor> {
        self.monitors.iter().find(|m| m.name == name)
    }

    pub fn focused(&self) -> Option<&WindowInfo> {
        self.windows.iter().find(|w| w.focused)
    }
}

/// sevenwms socket from SEVENWM_SOCK or the newest one in the runtime dir
pub fn socket_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SEVENWM_SOCK") {
        return Some(PathBuf::from(path));
    }
    let dir = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?).join("sevenwm");
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "sock"))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
}

thread_local! {
    /// the newest state from the subscription so toggles know whats on
    static LATEST: std::cell::RefCell<Option<State>> = const { std::cell::RefCell::new(None) };
}

/// remember the newest state from sevenwm
pub fn set_latest(state: &State) {
    LATEST.with(|l| *l.borrow_mut() = Some(state.clone()));
}

/// the newest state from sevenwm if one came yet
pub fn latest() -> Option<State> {
    LATEST.with(|l| l.borrow().clone())
}

/// send one request and read its reply
pub fn request(request: Value) -> Result<Value, String> {
    let path = socket_path().ok_or("sevenwm isn't running")?;
    let mut stream = UnixStream::connect(&path).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    writeln!(stream, "{request}").map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    let reply: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
    match reply.get("error") {
        Some(err) => Err(err.to_string()),
        None => Ok(reply["ok"].clone()),
    }
}

/// run a sevenwm action and ignore the reply
pub fn action(action: &str) {
    if let Err(err) = request(json!({ "action": action })) {
        eprintln!("sevenshell: {action}: {err}");
    }
}

/// focus a window and uhh fly the view to it
pub fn focus(id: u64) {
    if let Err(err) = request(json!({ "focus": id })) {
        eprintln!("sevenshell: focus {id}: {err}");
    }
}

pub fn fly_to(x: f64, y: f64) {
    let _ = request(json!({ "fly_to": { "x": x, "y": y } }));
}

/// follow sevenwms state from a background thread and reconnect when it drops
pub fn subscribe() -> crate::wake::Receiver<State> {
    let (send, receive) = crate::wake::channel();
    std::thread::spawn(move || {
        loop {
            follow(&send);
            std::thread::sleep(Duration::from_secs(1));
        }
    });
    receive
}

fn follow(send: &crate::wake::Sender<State>) {
    let Some(path) = socket_path() else {
        return;
    };
    let Ok(mut stream) = UnixStream::connect(&path) else {
        return;
    };
    if writeln!(stream, "{}", json!({ "subscribe": true })).is_err() {
        return;
    }
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else {
            return;
        };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(state) = message.get("state")
            && let Ok(state) = serde_json::from_value::<State>(state.clone())
            && send.unbounded_send(state).is_err()
        {
            std::process::exit(0);
        }
    }
}
