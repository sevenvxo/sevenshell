//! work from other threads comes back onto the gtk loop thru these so it only wakes when theres something

use futures_util::StreamExt;
use gtk4::glib;

pub use futures_channel::mpsc::{UnboundedReceiver as Receiver, UnboundedSender as Sender};

/// a channel any thread can send on and the gtk loop reads
pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    futures_channel::mpsc::unbounded()
}

/// run f on the gtk loop for every value that comes till it says break
pub fn each<T: 'static>(mut rx: Receiver<T>, mut f: impl FnMut(T) -> glib::ControlFlow + 'static) {
    glib::spawn_future_local(async move {
        while let Some(value) = rx.next().await {
            if f(value) == glib::ControlFlow::Break {
                break;
            }
        }
    });
}

/// like each but only the newest of whatever piled up bc the older ones are stale anyway
pub fn each_latest<T: 'static>(mut rx: Receiver<T>, mut f: impl FnMut(T) -> glib::ControlFlow + 'static) {
    glib::spawn_future_local(async move {
        while let Some(mut value) = rx.next().await {
            while let Ok(newer) = rx.try_recv() {
                value = newer;
            }
            if f(value) == glib::ControlFlow::Break {
                break;
            }
        }
    });
}

/// do work on its own thread then hand its result to done back on the gtk loop
pub fn off_thread<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(T) + 'static,
) {
    let (tx, rx) = futures_channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    glib::spawn_future_local(async move {
        if let Ok(value) = rx.await {
            done(value);
        }
    });
}
