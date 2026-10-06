//! commands whose output lines feed bar modules and one run is shared by every bar that shows it

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gtk4::glib;
use gtk4::prelude::*;

/// a widget waiting for lines w its id
type Listener = (u64, Box<dyn FnMut(&str)>);

/// the newest line and whos waiting for the next one
#[derive(Default)]
struct Shared {
    latest: RefCell<Option<String>>,
    listeners: RefCell<Vec<Listener>>,
}

pub struct Feed {
    shared: Rc<Shared>,
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
}

impl Drop for Feed {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(child) = self.child.lock().ok().and_then(|mut c| c.take()) {
            end(child);
        }
    }
}

/// end the command and everything it started like both sides of a pipe
fn end(mut child: Child) {
    // safety plain syscall and its own group so this only hits what it started
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGTERM);
    }
    let _ = child.wait();
}

impl Feed {
    /// the newest line if one came yet
    pub fn latest(&self) -> Option<String> {
        self.shared.latest.borrow().clone()
    }

    /// call f w the newest line and every one after while widget lives and the command runs till the last widget is gone
    pub fn watch<W: IsA<gtk4::Widget>>(self: &Rc<Self>, widget: &W, mut f: impl FnMut(&W, &str) + 'static) {
        let id = NEXT_LISTENER.with(|n| {
            n.set(n.get() + 1);
            n.get()
        });
        let weak = widget.downgrade();
        let keep = self.clone();
        let mut call = move |line: &str| {
            let _ = &keep;
            if let Some(widget) = weak.upgrade() {
                f(&widget, line);
            }
        };
        let latest = self.shared.latest.borrow().clone();
        if let Some(line) = latest {
            call(&line);
        }
        self.shared.listeners.borrow_mut().push((id, Box::new(call)));
        let shared = Rc::downgrade(&self.shared);
        widget.connect_destroy(move |_| {
            let Some(shared) = shared.upgrade() else {
                return;
            };
            let gone = {
                let mut listeners = shared.listeners.borrow_mut();
                listeners.iter().position(|(i, _)| *i == id).map(|i| listeners.remove(i))
            };
            // dropped out here bc it can end the feed
            drop(gone);
        });
    }
}

thread_local! {
    static FEEDS: RefCell<HashMap<(String, u64), Weak<Feed>>> = RefCell::new(HashMap::new());
    static NEXT_LISTENER: Cell<u64> = const { Cell::new(0) };
}

/// the feed for a command and every seconds and 0 means it keeps running and prints lines
pub fn get(command: &str, every: u64) -> Rc<Feed> {
    let key = (command.to_string(), every);
    if let Some(feed) = FEEDS.with(|f| f.borrow().get(&key).and_then(Weak::upgrade)) {
        return feed;
    }
    let feed = Rc::new(start(command, every));
    FEEDS.with(|f| {
        let mut feeds = f.borrow_mut();
        feeds.retain(|_, w| w.strong_count() > 0);
        feeds.insert(key, Rc::downgrade(&feed));
    });
    feed
}

fn command(line: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.args(["-c", line]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    // its own process group so ending it gets the whole pipeline
    cmd.process_group(0);
    // it dies w sevenshell so a crash doesnt leave it running
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    cmd
}

fn start(line: &str, every: u64) -> Feed {
    let shared: Rc<Shared> = Rc::default();
    let stop = Arc::new(AtomicBool::new(false));
    let child: Arc<Mutex<Option<Child>>> = Arc::default();
    let (tx, rx) = crate::wake::channel::<String>();
    let push = move |text: String| {
        let _ = tx.unbounded_send(text);
    };
    // lines wake the gtk loop and only the newest matters so a fast cava cant pile up
    {
        let shared = shared.clone();
        crate::wake::each_latest(rx, move |text| {
            for (_, f) in shared.listeners.borrow_mut().iter_mut() {
                f(&text);
            }
            *shared.latest.borrow_mut() = Some(text);
            glib::ControlFlow::Continue
        });
    }
    let (line, stop2, child2) = (line.to_string(), stop.clone(), child.clone());
    std::thread::spawn(move || {
        if every == 0 {
            // one long run and every line it prints shows up and it restarts if it quits
            while !stop2.load(Ordering::Relaxed) {
                let Ok(mut running) = command(&line).spawn() else {
                    eprintln!("sevenshell: couldnt run {line}");
                    return;
                };
                let out = running.stdout.take();
                if let Ok(mut c) = child2.lock() {
                    *c = Some(running);
                }
                if stop2.load(Ordering::Relaxed) {
                    // dropped while starting so the drop missed it
                    if let Some(c) = child2.lock().ok().and_then(|mut c| c.take()) {
                        end(c);
                    }
                    return;
                }
                if let Some(out) = out {
                    for text in BufReader::new(out).lines().map_while(Result::ok) {
                        push(text);
                    }
                }
                if let Some(mut c) = child2.lock().ok().and_then(|mut c| c.take()) {
                    let _ = c.wait();
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        } else {
            // run it every so often and keep its last line
            while !stop2.load(Ordering::Relaxed) {
                if let Ok(out) = command(&line).output() {
                    let text = String::from_utf8_lossy(&out.stdout);
                    push(text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").to_string());
                }
                for _ in 0..every * 10 {
                    if stop2.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    });
    Feed { shared, stop, child }
}

/// where the cava config for the bar goes
fn cava_config(bars: usize) -> Option<std::path::PathBuf> {
    let dir = std::path::PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?).join("sevenshell");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("cava-{bars}.conf"));
    let text = format!(
        "[general]\nbars = {bars}\nframerate = 30\n\n[output]\nmethod = raw\nraw_target = /dev/stdout\ndata_format = ascii\nascii_max_range = 100\nbar_delimiter = 59\nframe_delimiter = 10\n"
    );
    std::fs::write(&path, text).ok()?;
    Some(path)
}

/// cava printing bar heights 0 to 100 split by semicolons one frame per line
pub fn cava(bars: usize) -> Option<Rc<Feed>> {
    let path = cava_config(bars)?;
    Some(get(&format!("exec cava -p '{}'", path.display()), 0))
}

/// the heights in one cava line from 0 to 1
pub fn cava_heights(line: &str) -> Vec<f64> {
    line.split(';')
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().parse::<f64>().unwrap_or(0.0).clamp(0.0, 100.0) / 100.0)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cava_lines_parse() {
        assert_eq!(cava_heights("0;50;100;"), vec![0.0, 0.5, 1.0]);
        assert_eq!(cava_heights("200;x;\n"), vec![1.0, 0.0]);
    }
}
