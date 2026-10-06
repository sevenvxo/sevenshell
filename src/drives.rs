//! keeps udiskie running so plugged in drives mount on their own w a notification

use std::cell::RefCell;
use std::process::{Child, Command, Stdio};

use crate::config;

thread_local! {
    /// the udiskie we started so we only ever stop our own
    static CHILD: RefCell<Option<Child>> = const { RefCell::new(None) };
}

/// start or stop udiskie to match drives.automount and restart it if it uhh crashed
pub fn sync() {
    let want = config::get().drives.automount;
    CHILD.with(|child| {
        let mut child = child.borrow_mut();
        // reap it if it quit so it gets started again
        if let Some(c) = child.as_mut() {
            if !matches!(c.try_wait(), Ok(None)) {
                *child = None;
            }
        }
        match (want, child.as_mut()) {
            (true, None) if !running_elsewhere() => {
                let started = Command::new("udiskie")
                    .args(["--automount", "--notify", "--no-tray"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .spawn();
                // not installed is fine and u js dont get automount
                *child = started.ok();
            }
            (false, Some(c)) => {
                let _ = c.kill();
                let _ = c.wait();
                *child = None;
            }
            _ => {}
        }
    });
}

/// true when some other udiskie is already going like one from ur wm autostart
fn running_elsewhere() -> bool {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return false;
    };
    dir.flatten().any(|entry| {
        std::fs::read(entry.path().join("cmdline"))
            .map(|cmd| {
                cmd.split(|b| *b == 0)
                    .take(2)
                    .any(|arg| arg.rsplit(|b| *b == b'/').next() == Some(b"udiskie"))
            })
            .unwrap_or(false)
    })
}
