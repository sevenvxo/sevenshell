//! clipboard history thru cliphist where sevenshell keeps wl-paste feeding it and mod+v brings old copies back

use std::cell::RefCell;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;

use gtk4::prelude::*;

use crate::config;
use crate::picker::{Look, Picker, Row, Source, is_subsequence};

thread_local! {
    /// the wl-paste watcher we started and the max items it was started w
    static WATCHER: RefCell<Option<(Child, u32)>> = const { RefCell::new(None) };
}

/// whether a program is on the path
pub fn installed(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|p| p.join(program).is_file()))
}

/// start or stop the watcher to match clipboard.history and restart it if it died or the size changed
pub fn sync() {
    let c = &config::get().clipboard;
    let want = c.history && installed("cliphist") && installed("wl-paste");
    WATCHER.with(|w| {
        let mut w = w.borrow_mut();
        if let Some((child, max)) = w.as_mut()
            && (!matches!(child.try_wait(), Ok(None)) || !want || *max != c.max_items)
        {
            let _ = child.kill();
            let _ = child.wait();
            *w = None;
        }
        // one left over from a shell that crashed before this or started by ur own autostart already does the job
        if want && w.is_none() && !watcher_elsewhere() {
            let mut command = Command::new("wl-paste");
            command
                .args(["--watch", "cliphist", "-max-items", &c.max_items.to_string(), "store"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            // it dies w sevenshell so a restarted shell doesnt end up w two
            // safety prctl is async signal safe and touches nothing else
            unsafe {
                command.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                    Ok(())
                });
            }
            *w = command.spawn().ok().map(|child| (child, c.max_items));
        }
    });
}

/// whether a wl-paste --watch cliphist thats not ours is already running
fn watcher_elsewhere() -> bool {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return false;
    };
    dir.flatten().any(|entry| {
        std::fs::read(entry.path().join("cmdline")).is_ok_and(|cmd| {
            let args: Vec<&[u8]> = cmd.split(|b| *b == 0).collect();
            args.first().is_some_and(|a| a.rsplit(|b| *b == b'/').next() == Some(b"wl-paste"))
                && args.contains(&b"--watch".as_slice())
                && args.iter().any(|a| a.rsplit(|b| *b == b'/').next() == Some(b"cliphist"))
        })
    })
}

/// forget everything thats been copied
pub fn wipe() {
    crate::run_detached(Command::new("cliphist").arg("wipe"));
}

pub type Clipboard = Picker<Entries>;

pub struct Entries {
    /// cliphist list lines as id tab preview
    lines: RefCell<Vec<String>>,
    lower: RefCell<Vec<String>>,
}

fn read_list() -> Vec<String> {
    Command::new("cliphist")
        .arg("list")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(String::from).collect())
        .unwrap_or_default()
}

/// the part after the id
fn preview(line: &str) -> &str {
    line.split_once('\t').map_or(line, |(_, p)| p)
}

/// feed a cliphist line thru cmd on stdin
fn with_line(args: &[&str], line: &str) -> Option<Vec<u8>> {
    let mut child = Command::new("cliphist")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(line.as_bytes()).ok()?;
    Some(child.wait_with_output().ok()?.stdout)
}

impl Clipboard {
    pub fn new(app: &gtk4::Application) -> Rc<Self> {
        let empty = if installed("cliphist") {
            "Nothing copied yet"
        } else {
            "Clipboard history needs cliphist (sudo pacman -S cliphist)"
        };
        let look = Look {
            namespace: "clipboard",
            width: 560,
            placeholder: Some("Search what you copied"),
            empty,
            detail: false,
            hint: Some("Enter copies it again · Shift+Delete forgets it"),
        };
        let lines = read_list();
        let lower = lines.iter().map(|l| preview(l).to_lowercase()).collect();
        Picker::build(app, look, Entries { lines: RefCell::new(lines), lower: RefCell::new(lower) })
    }
}

impl Source for Entries {
    fn len(&self) -> usize {
        self.lines.borrow().len()
    }

    fn rank(&self, query: &str) -> Vec<usize> {
        let lower = self.lower.borrow();
        // newest first like cliphist lists them
        (0..lower.len())
            .filter(|&i| query.is_empty() || lower[i].contains(query) || is_subsequence(query, &lower[i]))
            .collect()
    }

    fn fill(&self, index: usize, row: &Row) {
        let lines = self.lines.borrow();
        let text = preview(&lines[index]);
        let image = text.starts_with("[[ binary data");
        row.icon.set_visible(false);
        row.glyph.set_visible(true);
        row.glyph.add_css_class("icon");
        row.glyph.set_text(if image { "image" } else { "content_paste" });
        row.name.set_text(&text.replace(['\n', '\t'], " "));
    }

    fn pick(&self, index: usize, window: &gtk4::ApplicationWindow) {
        let line = self.lines.borrow()[index].clone();
        if let Some(bytes) = with_line(&["decode"], &line)
            && let Ok(mut copy) = Command::new("wl-copy").stdin(Stdio::piped()).spawn()
        {
            if let Some(mut stdin) = copy.stdin.take() {
                let _ = stdin.write_all(&bytes);
            }
            // wl-copy forks to keep serving the paste so this returns right away
            std::thread::spawn(move || copy.wait());
        }
        window.close();
    }

    fn remove(&self, index: usize) -> bool {
        let line = self.lines.borrow()[index].clone();
        if with_line(&["delete"], &line).is_none() {
            return false;
        }
        self.lines.borrow_mut().remove(index);
        self.lower.borrow_mut().remove(index);
        true
    }
}
