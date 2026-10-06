//! the settings data for sevenwm and sevenshell merged over their defaults and saved back w every comment kept

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk4::glib;
use toml::{Table, Value};

/// which config a setting lives in
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Wm,
    Shell,
}

/// one config w its documented defaults and the merged values u see and edit
struct Config {
    template: String,
    defaults: Table,
    data: Table,
    path: PathBuf,
}

pub struct Store {
    wm: RefCell<Config>,
    shell: RefCell<Config>,
    /// bumped on every change so only the last one in a burst saves
    generation: Cell<u64>,
    /// says how a save went like saved or whats wrong
    on_status: RefCell<Option<Box<dyn Fn(&str, bool)>>>,
    /// runs after every change so pages can redraw previews
    on_change: RefCell<Vec<Box<dyn Fn()>>>,
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
}

pub fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("sevenwm")
}

/// the sevenwm binary on the path or next to ur build
pub fn sevenwm_binary() -> Option<PathBuf> {
    let from_path = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|p| p.join("sevenwm"))
            .find(|p| p.is_file())
    });
    from_path.or_else(|| {
        let p = home().join("Projects/sevenwm/target/release/sevenwm");
        p.is_file().then_some(p)
    })
}

/// sevenwms defaults from the running one or its repo whichever is newer
fn sevenwm_template() -> String {
    let mut candidates = vec![runtime_dir().join("defaults.toml")];
    // the repo sits two folders up from its binary
    if let Some(bin) = sevenwm_binary().and_then(|b| std::fs::canonicalize(b).ok())
        && let Some(repo) = bin.ancestors().nth(3)
    {
        candidates.push(repo.join("config.default.toml"));
    }
    candidates.push(home().join("Projects/sevenwm/config.default.toml"));
    candidates
        .into_iter()
        .filter_map(|p| {
            let mtime = std::fs::metadata(&p).and_then(|m| m.modified()).ok()?;
            Some((mtime, p))
        })
        .max_by_key(|(mtime, _)| *mtime)
        .and_then(|(_, p)| std::fs::read_to_string(p).ok())
        .unwrap_or_default()
}

/// lay over onto base table by table
pub fn merge(base: &mut Table, over: Table) {
    for (key, value) in over {
        match (base.get_mut(&key), value) {
            (Some(Value::Table(base)), Value::Table(over)) => merge(base, over),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

fn load(template: String, path: PathBuf) -> (Config, Option<String>) {
    let defaults: Table = template.parse().unwrap_or_default();
    let mut data = defaults.clone();
    let mut problem = None;
    if let Ok(text) = std::fs::read_to_string(&path) {
        match text.parse::<Table>() {
            Ok(user) => merge(&mut data, user),
            Err(err) => problem = Some(format!("{} has a mistake so defaults are shown: {err}", path.display())),
        }
    }
    (Config { template, defaults, data, path }, problem)
}

impl Store {
    pub fn new() -> Rc<Self> {
        let (wm, wm_problem) = load(sevenwm_template(), config_dir().join("sevenwm/config.toml"));
        let (shell, shell_problem) = load(crate::config::DEFAULTS.to_string(), crate::config::path());
        let store = Rc::new(Self {
            wm: RefCell::new(wm),
            shell: RefCell::new(shell),
            generation: Cell::new(0),
            on_status: RefCell::new(None),
            on_change: RefCell::new(Vec::new()),
        });
        if let Some(problem) = wm_problem.or(shell_problem) {
            let store = store.clone();
            glib::idle_add_local_once(move || store.status(&problem, true));
        }
        store
    }

    fn config(&self, which: Which) -> &RefCell<Config> {
        match which {
            Which::Wm => &self.wm,
            Which::Shell => &self.shell,
        }
    }

    pub fn set_status(&self, f: impl Fn(&str, bool) + 'static) {
        *self.on_status.borrow_mut() = Some(Box::new(f));
    }

    pub fn on_change(&self, f: impl Fn() + 'static) {
        self.on_change.borrow_mut().push(Box::new(f));
    }

    fn status(&self, text: &str, error: bool) {
        if let Some(f) = self.on_status.borrow().as_ref() {
            f(text, error);
        }
    }

    /// a value by its dotted path like tiling.gaps_inner
    pub fn get(&self, which: Which, path: &str) -> Option<Value> {
        let config = self.config(which).borrow();
        let mut node = &config.data;
        let mut parts = path.split('.').peekable();
        while let Some(part) = parts.next() {
            let value = node.get(part)?;
            if parts.peek().is_none() {
                return Some(value.clone());
            }
            node = value.as_table()?;
        }
        None
    }

    pub fn str(&self, which: Which, path: &str) -> String {
        self.get(which, path)
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default()
    }

    pub fn bool(&self, which: Which, path: &str) -> bool {
        self.get(which, path).and_then(|v| v.as_bool()).unwrap_or(false)
    }

    pub fn num(&self, which: Which, path: &str) -> f64 {
        match self.get(which, path) {
            Some(Value::Integer(i)) => i as f64,
            Some(Value::Float(f)) => f,
            _ => 0.0,
        }
    }

    /// set a value then save a moment later so a burst of changes saves once
    pub fn put(self: &Rc<Self>, which: Which, path: &str, value: Value) {
        {
            let mut config = self.config(which).borrow_mut();
            let mut node = &mut config.data;
            let parts: Vec<&str> = path.split('.').collect();
            for part in &parts[..parts.len() - 1] {
                let entry = node
                    .entry(part.to_string())
                    .or_insert_with(|| Value::Table(Table::new()));
                if !entry.is_table() {
                    *entry = Value::Table(Table::new());
                }
                node = entry.as_table_mut().expect("just made it a table");
            }
            if node.get(parts[parts.len() - 1]) == Some(&value) {
                return;
            }
            node.insert(parts[parts.len() - 1].to_string(), value);
        }
        for f in self.on_change.borrow().iter() {
            f();
        }
        self.schedule_save();
    }

    /// change a value w/o saving later so set_now can save right away
    fn set_quietly(&self, which: Which, path: &str, value: Value) {
        let mut config = self.config(which).borrow_mut();
        let mut node = &mut config.data;
        let parts: Vec<&str> = path.split('.').collect();
        for part in &parts[..parts.len() - 1] {
            let entry = node
                .entry(part.to_string())
                .or_insert_with(|| Value::Table(Table::new()));
            if !entry.is_table() {
                *entry = Value::Table(Table::new());
            }
            node = entry.as_table_mut().expect("just made it a table");
        }
        node.insert(parts[parts.len() - 1].to_string(), value);
    }

    fn schedule_save(self: &Rc<Self>) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let store = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(500), move || {
            if let Some(store) = store.upgrade()
                && store.generation.get() == generation
            {
                store.save();
            }
        });
    }

    /// check both configs and write whichever changed
    pub fn save(&self) {
        let wm_text = {
            let c = self.wm.borrow();
            write_config(&c.data, &c.template, Some(&c.defaults))
        };
        let shell_text = {
            let c = self.shell.borrow();
            write_config(&c.data, &c.template, None)
        };
        if let Err(err) = check_wm(&wm_text) {
            self.status(&format!("Not saved {err}"), true);
            return;
        }
        if let Err(err) = crate::config::parse(&shell_text) {
            self.status(&format!("Not saved {err}"), true);
            return;
        }
        let paths = (self.wm.borrow().path.clone(), self.shell.borrow().path.clone());
        let result = write_if_changed(&paths.0, &wm_text).and_then(|a| write_if_changed(&paths.1, &shell_text).map(|b| a || b));
        match result {
            Ok(true) => self.status("Saved and applied", false),
            Ok(false) => {}
            Err(err) => self.status(&format!("Couldnt save {err}"), true),
        }
    }
}

/// write a few settings right now w the comments kept like the quick settings toggles do
pub fn set_now(changes: &[(Which, &str, Value)]) -> Result<(), String> {
    let store = Store::new();
    for (which, path, value) in changes {
        store.set_quietly(*which, path, value.clone());
    }
    let error: Rc<RefCell<Option<String>>> = Rc::default();
    let e = error.clone();
    store.set_status(move |text, bad| {
        if bad {
            *e.borrow_mut() = Some(text.to_string());
        }
    });
    store.save();
    let result = error.borrow_mut().take();
    result.map_or(Ok(()), Err)
}

/// run sevenwm --check-config on the text if sevenwm is around
fn check_wm(text: &str) -> Result<(), String> {
    let Some(binary) = sevenwm_binary() else {
        return Ok(());
    };
    // ur runtime dir bc a guessable name in the shared /tmp could be a symlink someone else left
    let dir = runtime_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!("check-{}.toml", std::process::id()));
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    let out = std::process::Command::new(binary)
        .arg("--check-config")
        .arg(&tmp)
        .output();
    let _ = std::fs::remove_file(&tmp);
    let out = out.map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        let msg = String::from_utf8_lossy(&out.stdout);
        Err(msg.trim().trim_start_matches("error: ").to_string())
    }
}

/// write it next to the file then swap it in so readers never see half and keep a bak
fn write_if_changed(path: &PathBuf, text: &str) -> Result<bool, String> {
    if std::fs::read_to_string(path).ok().as_deref() == Some(text) {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    if path.exists() {
        let _ = std::fs::copy(path, path.with_extension("toml.bak"));
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(true)
}

/// a value written the way toml wants it inline
pub fn toml_value(value: &Value) -> String {
    match value {
        Value::String(s) => serde_json::to_string(s).unwrap_or_default(),
        Value::Array(items) => format!("[{}]", items.iter().map(toml_value).collect::<Vec<_>>().join(", ")),
        Value::Table(t) if t.is_empty() => "{}".into(),
        Value::Table(t) => format!(
            "{{ {} }}",
            t.iter()
                .map(|(k, v)| format!("{} = {}", toml_value(&Value::String(k.clone())), toml_value(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => other.to_string(),
    }
}

/// split value and comment at the first # thats not in a string
fn split_value_comment(rest: &str) -> (&str, &str) {
    let mut quote = None;
    let mut escaped = false;
    for (i, ch) in rest.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
        } else if ch == '"' || ch == '\'' {
            quote = Some(ch);
        } else if ch == '#' {
            return (rest[..i].trim_end(), &rest[i..]);
        }
    }
    (rest.trim_end(), "")
}

/// a key = value line split into indent key the equals part and the rest
fn key_line(line: &str) -> Option<(&str, &str, &str, &str)> {
    let indent_len = line.len() - line.trim_start().len();
    let (indent, body) = line.split_at(indent_len);
    let key_len = if let Some(rest) = body.strip_prefix('"') {
        rest.find('"')? + 2
    } else {
        body.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            .unwrap_or(body.len())
    };
    if key_len == 0 {
        return None;
    }
    let (key, after) = body.split_at(key_len);
    let eq_start = after.len() - after.trim_start().len();
    let after_eq = after.trim_start().strip_prefix('=')?;
    let eq_len = eq_start + 1 + (after_eq.len() - after_eq.trim_start().len());
    let (eq, rest) = after.split_at(eq_len);
    Some((indent, key, eq, rest))
}

fn lookup<'a>(data: &'a Table, section: &str, key: &str) -> Option<&'a Value> {
    if section.is_empty() {
        return data.get(key);
    }
    let mut node = data;
    for part in section.split('.') {
        node = node.get(part)?.as_table()?;
    }
    node.get(key)
}

/// the config text from the template w ur values filled in and its comments kept
/// and w defaults only keybinds that differ get written so later default changes still reach u
pub fn write_config(data: &Table, template: &str, defaults: Option<&Table>) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut section = String::new();
    let mut bindings: Vec<(String, Value)> = data
        .get("keybindings")
        .and_then(Value::as_table)
        .map(|t| t.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    let default_binds = defaults
        .and_then(|d| d.get("keybindings"))
        .and_then(Value::as_table);
    let trim = defaults.is_some();
    let mut changed: Vec<(String, Value)> = Vec::new();

    let flush = |out: &mut Vec<String>, section: &str, bindings: &mut Vec<(String, Value)>, changed: &mut Vec<(String, Value)>| {
        if section != "keybindings" {
            return;
        }
        let rows: Vec<_> = changed.drain(..).chain(bindings.drain(..)).collect();
        if trim {
            out.push("# only binds that differ from the defaults go here and mod+/ shows the full list".into());
        } else if rows.is_empty() {
            return;
        } else {
            while out.last().is_some_and(|l| l.trim().is_empty()) {
                out.pop();
            }
            out.push(String::new());
        }
        for (combo, action) in rows {
            out.push(format!("{} = {}", toml_value(&Value::String(combo)), toml_value(&action)));
        }
        out.push(String::new());
    };

    for line in template.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') && !trimmed.starts_with("#") {
            flush(&mut out, &section, &mut bindings, &mut changed);
            section = trimmed.trim_matches(|c| c == '[' || c == ']').trim().to_string();
            out.push(line.to_string());
            continue;
        }
        let parsed = key_line(line).filter(|_| !trimmed.starts_with('#'));
        if section == "keybindings" && trim {
            // the template binds js tell us whats default and uhh never get copied
            if let Some((_, key, _, _)) = parsed {
                let combo: String = serde_json::from_str(key).unwrap_or_else(|_| key.to_string());
                let value = match bindings.iter().position(|(c, _)| *c == combo) {
                    Some(i) => bindings.remove(i).1,
                    None => Value::String("none".into()),
                };
                if default_binds.and_then(|d| d.get(&combo)) != Some(&value) {
                    changed.push((combo, value));
                }
            }
            continue;
        }
        let Some((indent, key, eq, rest)) = parsed else {
            out.push(line.to_string());
            continue;
        };
        let value = if section == "keybindings" {
            let combo: String = serde_json::from_str(key).unwrap_or_else(|_| key.to_string());
            // a removed default has to say so or sevenwm fills it back in
            match bindings.iter().position(|(c, _)| *c == combo) {
                Some(i) => bindings.remove(i).1,
                None => Value::String("none".into()),
            }
        } else {
            match lookup(data, &section, key) {
                Some(v) => v.clone(),
                None => {
                    out.push(line.to_string());
                    continue;
                }
            }
        };
        let (_, comment) = split_value_comment(rest);
        let new_value = toml_value(&value);
        if comment.is_empty() {
            out.push(format!("{indent}{key}{eq}{new_value}"));
        } else {
            // keep the comment in the same column
            let column = rest.len() - comment.len();
            out.push(format!("{indent}{key}{eq}{:<width$} {comment}", new_value, width = column.saturating_sub(1)));
        }
    }
    flush(&mut out, &section, &mut bindings, &mut changed);
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    for (name, keys) in [
        ("rules", &["app_id", "title", "float", "size", "hide_from_screencast"][..]),
        ("monitors", &["name", "position", "mode", "scale"][..]),
        ("bar.custom", &["name", "exec", "interval", "on_click", "icon"][..]),
    ] {
        let (section, key) = name.rsplit_once('.').unwrap_or(("", name));
        for item in lookup(data, section, key).and_then(Value::as_array).into_iter().flatten() {
            let Some(item) = item.as_table() else {
                continue;
            };
            out.push(String::new());
            out.push(format!("[[{name}]]"));
            for key in keys {
                if let Some(v) = item.get(*key) {
                    out.push(format!("{key} = {}", toml_value(v)));
                }
            }
        }
    }
    out.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATE: &str = "# top\n[tiling]\ngaps = 4                 # between tiles\nname = \"a # b\"\n\n[keybindings]\n\"mod+t\" = \"exec kitty\"\n\"mod+q\" = \"close-window\"\n";

    #[test]
    fn values_fill_in_and_comments_stay_put() {
        let defaults: Table = TEMPLATE.parse().unwrap();
        let mut data = defaults.clone();
        merge(&mut data, "[tiling]\ngaps = 12\n".parse().unwrap());
        let text = write_config(&data, TEMPLATE, Some(&defaults));
        assert!(text.contains("gaps = 12                # between tiles"), "{text}");
        assert!(text.contains("name = \"a # b\""));
        let back: Table = text.parse().unwrap();
        assert_eq!(back["tiling"]["gaps"].as_integer(), Some(12));
    }

    #[test]
    fn only_changed_binds_get_written() {
        let defaults: Table = TEMPLATE.parse().unwrap();
        let mut data = defaults.clone();
        let kb = data["keybindings"].as_table_mut().unwrap();
        kb.insert("mod+t".into(), Value::String("exec foot".into()));
        kb.remove("mod+q");
        kb.insert("mod+y".into(), Value::String("exec brave".into()));
        let text = write_config(&data, TEMPLATE, Some(&defaults));
        let back: Table = text.parse().unwrap();
        let kb = back["keybindings"].as_table().unwrap();
        assert_eq!(kb["mod+t"].as_str(), Some("exec foot"));
        assert_eq!(kb["mod+q"].as_str(), Some("none"));
        assert_eq!(kb["mod+y"].as_str(), Some("exec brave"));
        assert_eq!(kb.len(), 3);
    }

    #[test]
    fn own_modules_and_colors_survive_a_save() {
        let template = crate::config::DEFAULTS;
        let mut data: Table = template.parse().unwrap();
        let extra: Table = "[bar]\nleft = [\"custom/up\"]\n[[bar.custom]]\nname = \"up\"\nexec = \"uptime -p\"\ninterval = 0\n[theme.colors]\nm3onSurface = \"#ff0000\"\n".parse().unwrap();
        merge(&mut data, extra);
        let text = write_config(&data, template, None);
        let config = crate::config::parse(&text).unwrap();
        assert_eq!(config.bar.custom[0].interval, 0);
        assert_eq!(config.theme.colors["m3onSurface"], "#ff0000");
    }
}
