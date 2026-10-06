//! the app launcher on mod+d where u type to search and enter launches and most used apps come first

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{gdk, gio};

use crate::picker::{Look, Picker, Row, Source, is_subsequence};

pub type Launcher = Picker<Apps>;

struct App {
    info: gio::AppInfo,
    id: String,
    name: String,
    /// generic name keywords and executable lowercased and searched too
    extra: String,
}

pub struct Apps {
    apps: Vec<App>,
    history: RefCell<HashMap<String, u64>>,
    /// what was typed w its case kept for commands
    raw: RefCell<String>,
    /// the extra rows after the apps for this query
    extras: RefCell<Vec<Extra>>,
}

/// rows that arent apps like an answer a command or a web search
enum Extra {
    /// the math and its answer
    Answer(String),
    /// > command runs in a shell
    Run(String),
    /// search the web for this
    Web(String),
}

fn history_path() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_default()
        .join("applauncher.json")
}

fn load_history() -> HashMap<String, u64> {
    std::fs::read(history_path())
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

fn save_history(history: &HashMap<String, u64>) {
    let path = history_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string(history) {
        let _ = std::fs::write(path, text);
    }
}

/// higher is a better match and none is no match
fn score(query: &str, app: &App) -> Option<u64> {
    if app.name.starts_with(query) {
        Some(100)
    } else if app.name.split_whitespace().any(|w| w.starts_with(query)) {
        Some(80)
    } else if app.name.contains(query) {
        Some(60)
    } else if app.extra.contains(query) {
        Some(40)
    } else if is_subsequence(query, &app.name) {
        Some(20)
    } else {
        None
    }
}

fn installed_apps() -> Vec<App> {
    gio::AppInfo::all()
        .into_iter()
        .filter(|info| info.should_show())
        .map(|info| {
            let desktop = info.downcast_ref::<gio_unix::DesktopAppInfo>();
            let generic = desktop
                .and_then(|d| d.generic_name())
                .map(|s| s.to_string());
            let keywords = desktop.map(|d| {
                d.keywords()
                    .iter()
                    .map(|k| k.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            });
            let exe = info
                .executable()
                .file_name()
                .map(|f| f.to_string_lossy().to_string());
            let extra = [generic, keywords, exe]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            App {
                id: info.id().map(|s| s.to_string()).unwrap_or_default(),
                name: info.display_name().to_lowercase(),
                extra,
                info,
            }
        })
        .collect()
}

impl Launcher {
    pub fn new(app: &gtk4::Application) -> Rc<Self> {
        let look = Look {
            namespace: "launcher",
            width: 460,
            placeholder: None,
            empty: "No matches",
            detail: false,
            hint: None,
        };
        let apps = Apps {
            apps: installed_apps(),
            history: RefCell::new(load_history()),
            raw: RefCell::default(),
            extras: RefCell::default(),
        };
        Picker::build(app, look, apps)
    }
}

impl Source for Apps {
    fn len(&self) -> usize {
        self.apps.len()
    }

    fn typed(&self, raw: &str) {
        *self.raw.borrow_mut() = raw.to_string();
    }

    fn rank(&self, query: &str) -> Vec<usize> {
        let raw = self.raw.borrow().clone();
        let mut extras = Vec::new();
        let launcher = &crate::config::get().launcher;
        if let Some(command) = raw.strip_prefix('>') {
            // only the command so apps dont get in the way
            if !command.trim().is_empty() {
                extras.push(Extra::Run(command.trim().to_string()));
            }
            let rows = if extras.is_empty() { vec![] } else { vec![self.apps.len()] };
            *self.extras.borrow_mut() = extras;
            return rows;
        }
        let math = raw.strip_prefix('=').unwrap_or(&raw);
        if launcher.calculator
            && crate::calc::looks_like_math(&raw)
            && let Some(answer) = crate::calc::eval(math)
        {
            extras.push(Extra::Answer(crate::calc::show(answer)));
        }
        let answer_first = !extras.is_empty();
        let mut ranked = self.rank_apps(query);
        if !query.is_empty() && !launcher.search_url.is_empty() {
            extras.push(Extra::Web(raw.clone()));
        }
        let first = self.apps.len();
        let count = extras.len();
        *self.extras.borrow_mut() = extras;
        // an answer goes on top and the web search at the bottom
        if answer_first {
            ranked.insert(0, first);
            ranked.extend((first + 1)..(first + count));
        } else {
            ranked.extend(first..(first + count));
        }
        ranked
    }

    fn fill(&self, index: usize, row: &Row) {
        if index >= self.apps.len() {
            let extras = self.extras.borrow();
            let Some(extra) = extras.get(index - self.apps.len()) else {
                return;
            };
            row.icon.set_visible(false);
            row.glyph.set_visible(true);
            row.glyph.add_css_class("icon");
            let (glyph, text) = match extra {
                Extra::Answer(answer) => ("calculate", format!("= {answer}")),
                Extra::Run(command) => ("terminal", format!("Run {command}")),
                Extra::Web(query) => ("travel_explore", format!("Search the web for \u{201c}{query}\u{201d}")),
            };
            row.glyph.set_text(glyph);
            row.name.set_text(&text);
            return;
        }
        row.icon.set_visible(true);
        row.glyph.set_visible(false);
        self.fill_app(index, row);
    }

    fn pick(&self, index: usize, window: &gtk4::ApplicationWindow) {
        if index >= self.apps.len() {
            if let Some(extra) = self.extras.borrow().get(index - self.apps.len()) {
                match extra {
                    Extra::Answer(answer) => {
                        let mut copy = std::process::Command::new("wl-copy");
                        copy.arg(answer);
                        crate::run_detached(&mut copy);
                    }
                    Extra::Run(command) => {
                        crate::run_detached(std::process::Command::new("sh").args(["-c", command]));
                    }
                    Extra::Web(query) => {
                        let url = crate::config::get().launcher.search_url.replace("%s", &encode(query));
                        crate::run_detached(std::process::Command::new("xdg-open").arg(url));
                    }
                }
            }
            window.close();
            return;
        }
        self.pick_app(index, window);
    }
}

/// the query made safe for a url
fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

impl Apps {
    fn rank_apps(&self, query: &str) -> Vec<usize> {
        let history = self.history.borrow();
        let uses = |app: &App| history.get(&app.id).copied().unwrap_or(0);
        let mut ranked: Vec<(u64, usize)> = if query.is_empty() {
            (0..self.apps.len())
                .map(|i| (uses(&self.apps[i]), i))
                .collect()
        } else {
            (0..self.apps.len())
                .filter_map(|i| {
                    let app = &self.apps[i];
                    score(query, app).map(|s| (s + uses(app).min(20), i))
                })
                .collect()
        };
        ranked.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| self.apps[a.1].name.cmp(&self.apps[b.1].name))
        });
        ranked.into_iter().map(|(_, i)| i).collect()
    }

    fn fill_app(&self, index: usize, row: &Row) {
        let info = &self.apps[index].info;
        row.name.set_text(&info.display_name());
        match info.icon() {
            Some(icon) => row.icon.set_from_gicon(&icon),
            None => row.icon.set_icon_name(Some("application-x-executable")),
        }
    }

    fn pick_app(&self, index: usize, window: &gtk4::ApplicationWindow) {
        let app = &self.apps[index];
        {
            let mut history = self.history.borrow_mut();
            *history.entry(app.id.clone()).or_insert(0) += 1;
            save_history(&history);
        }
        let context = gdk::Display::default().map(|d| d.app_launch_context());
        if let Err(err) = app.info.launch(&[], context.as_ref()) {
            eprintln!("sevenshell: launching {}: {err}", app.id);
        }
        window.close();
    }
}
