//! the emoji picker on mod+period where u search by name and enter copies it

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::rc::Rc;

use gtk4::prelude::*;

use crate::picker::{Look, Picker, Row, Source};

pub type EmojiPicker = Picker<Emojis>;

struct Entry {
    glyph: &'static str,
    name: String,
    /// shortcodes like joy and thumbsup
    codes: String,
}

pub struct Emojis {
    all: Vec<Entry>,
    /// how often each got picked so favorites come first
    uses: RefCell<HashMap<String, u64>>,
}

fn uses_path() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_default()
        .join("sevenshell/emoji.json")
}

impl EmojiPicker {
    pub fn new(app: &gtk4::Application) -> Rc<Self> {
        let all = emojis::iter()
            .map(|e| Entry {
                glyph: e.as_str(),
                name: e.name().to_lowercase(),
                codes: e.shortcodes().collect::<Vec<_>>().join(" "),
            })
            .collect();
        let uses = std::fs::read(uses_path())
            .ok()
            .and_then(|d| serde_json::from_slice(&d).ok())
            .unwrap_or_default();
        let look = Look {
            namespace: "emoji",
            width: 460,
            placeholder: Some("Search emoji"),
            empty: "No emoji called that",
            detail: true,
            hint: Some("Enter copies it"),
        };
        Picker::build(app, look, Emojis { all, uses: RefCell::new(uses) })
    }
}

impl Source for Emojis {
    fn len(&self) -> usize {
        self.all.len()
    }

    fn rank(&self, query: &str) -> Vec<usize> {
        let uses = self.uses.borrow();
        let used = |e: &Entry| uses.get(e.glyph).copied().unwrap_or(0);
        let mut ranked: Vec<(u64, usize)> = self
            .all
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let score = if query.is_empty() {
                    0
                } else if e.name.starts_with(query) || e.codes.split(' ').any(|c| c.starts_with(query)) {
                    100
                } else if e.name.split_whitespace().any(|w| w.starts_with(query)) {
                    80
                } else if e.name.contains(query) || e.codes.contains(query) {
                    50
                } else {
                    return None;
                };
                Some((score + used(e).min(40), i))
            })
            .collect();
        // stable so ties keep the unicode order
        ranked.sort_by(|a, b| b.0.cmp(&a.0));
        ranked.into_iter().map(|(_, i)| i).collect()
    }

    fn fill(&self, index: usize, row: &Row) {
        let e = &self.all[index];
        row.icon.set_visible(false);
        row.glyph.set_visible(true);
        row.glyph.set_text(e.glyph);
        let mut name = e.name.clone();
        if let Some(first) = name.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        row.name.set_text(&name);
        if let Some(detail) = &row.detail {
            let code = e.codes.split(' ').next().unwrap_or("");
            detail.set_text(&if code.is_empty() { String::new() } else { format!(":{code}:") });
        }
    }

    fn pick(&self, index: usize, window: &gtk4::ApplicationWindow) {
        let glyph = self.all[index].glyph;
        if let Ok(mut copy) = Command::new("wl-copy").stdin(Stdio::piped()).spawn() {
            if let Some(mut stdin) = copy.stdin.take() {
                let _ = stdin.write_all(glyph.as_bytes());
            }
            std::thread::spawn(move || copy.wait());
        }
        let mut uses = self.uses.borrow_mut();
        *uses.entry(glyph.to_string()).or_insert(0) += 1;
        if let Some(dir) = uses_path().parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string(&*uses) {
            let _ = std::fs::write(uses_path(), text);
        }
        window.close();
    }
}
