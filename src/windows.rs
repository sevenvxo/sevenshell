//! the window search on mod+g where u type part of a name and enter focuses it

use std::rc::Rc;

use gtk4::gio;
use gtk4::prelude::*;

use crate::ipc::{self, WindowInfo};
use crate::picker::{Look, Picker, Row, Source, is_subsequence};

pub type WindowSearch = Picker<Windows>;

pub struct Windows(Vec<WindowInfo>);

/// higher is a better match and none is no match
fn score(query: &str, window: &WindowInfo) -> Option<u64> {
    let title = window.title.to_lowercase();
    let app = window.app_id.to_lowercase();
    if title.starts_with(query) || app.starts_with(query) {
        Some(100)
    } else if title.split_whitespace().any(|w| w.starts_with(query)) {
        Some(80)
    } else if title.contains(query) || app.contains(query) {
        Some(60)
    } else if is_subsequence(query, &title) {
        Some(20)
    } else {
        None
    }
}

/// the app icon from its .desktop file if theres one
fn icon_for(app_id: &str) -> Option<gio::Icon> {
    let desktop = gio_unix::DesktopAppInfo::new(&format!("{app_id}.desktop")).or_else(|| {
        gio_unix::DesktopAppInfo::new(&format!("{}.desktop", app_id.to_lowercase()))
    })?;
    desktop.icon()
}

/// where a window is in a word or two
fn detail(window: &WindowInfo) -> String {
    match (window.collapsed, window.workspace, window.tiled) {
        (true, _, _) => "collapsed".into(),
        (false, Some(n), true) => format!("workspace {n}"),
        _ => "floating".into(),
    }
}

impl WindowSearch {
    pub fn new(app: &gtk4::Application) -> Rc<Self> {
        let look = Look {
            namespace: "windows",
            width: 520,
            placeholder: Some("Find a window"),
            empty: "No windows match",
            detail: true,
            hint: None,
        };
        let windows = ipc::request(serde_json::json!({ "get": "state" }))
            .ok()
            .and_then(|state| serde_json::from_value::<ipc::State>(state).ok())
            .map(|state| state.windows)
            .unwrap_or_default();
        Picker::build(app, look, Windows(windows))
    }
}

impl Source for Windows {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn rank(&self, query: &str) -> Vec<usize> {
        // most recent first like alt-tab but ur current window goes last so enter goes back to the last one
        let recency = |w: &WindowInfo| {
            if w.focused {
                0
            } else {
                u64::MAX - w.recent.unwrap_or(usize::MAX / 2) as u64
            }
        };
        let mut ranked: Vec<(u64, u64, usize)> = (0..self.0.len())
            .filter_map(|i| {
                let window = &self.0[i];
                let quality = if query.is_empty() {
                    Some(0)
                } else {
                    score(query, window)
                }?;
                Some((quality, recency(window), i))
            })
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        ranked.into_iter().map(|(_, _, i)| i).collect()
    }

    fn fill(&self, index: usize, row: &Row) {
        let window = &self.0[index];
        let title = if window.title.is_empty() {
            &window.app_id
        } else {
            &window.title
        };
        row.name.set_text(title);
        if let Some(label) = &row.detail {
            label.set_text(&detail(window));
        }
        match icon_for(&window.app_id) {
            Some(icon) => row.icon.set_from_gicon(&icon),
            None => row.icon.set_icon_name(Some("application-x-executable")),
        }
    }

    fn pick(&self, index: usize, window: &gtk4::ApplicationWindow) {
        let id = self.0[index].id;
        // close first bc the search holds the keyboard till its gone
        window.close();
        ipc::focus(id);
    }
}
