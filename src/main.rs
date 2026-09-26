//! sevenshell is the desktop shell for sevenwm w the bar launcher search notifications osd lock screen and uhh login screen

mod bar;
mod config;
mod greeter;
mod ipc;
mod keybinds;
mod launcher;
mod lock;
mod notifications;
mod osd;
mod status;
mod windows;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{gdk, gio, glib};

use bar::Bar;
use keybinds::Keybinds;
use launcher::Launcher;
use windows::WindowSearch;
use lock::Lock;
use notifications::Notifications;
use osd::Osd;

/// ur waybar look in black and white w symbols nerd font at 30px
const CSS: &str = "
.sevenshell-bar {
    background: #000000;
    color: #ffffff;
}
.sevenshell-bar * {
    font-family: \"Symbols Nerd Font\", sans-serif;
    font-size: 13px;
    min-height: 0;
}
.sevenshell-bar .module {
    padding: 0 6px;
    margin: 4px 0;
    background: transparent;
    border: none;
    box-shadow: none;
    color: #ffffff;
    min-width: 0;
}
.sevenshell-bar button.module > label { padding: 0; }
.sevenshell-bar .desktop { margin-left: 6px; }
.sevenshell-bar .desktop label { font-size: 15px; }
.sevenshell-bar .perf label { font-size: 14px; }
.sevenshell-bar .power { margin-right: 6px; }
.sevenshell-bar .minimap { margin-left: 6px; }
";

fn main() -> glib::ExitCode {
    // sevenshell --check-config checks if the config is valid
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--check-config") {
        let path = args.get(1).map(Into::into).unwrap_or_else(config::path);
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        return match config::parse(&text) {
            Ok(_) => {
                println!("ok");
                glib::ExitCode::SUCCESS
            }
            Err(err) => {
                println!("error: {err}");
                glib::ExitCode::FAILURE
            }
        };
    }
    // sevenshell greeter is the login screen as its own program under greetd
    if args.first().map(String::as_str) == Some("greeter") {
        return greeter::run();
    }
    let app = gtk4::Application::builder()
        .application_id("org.sevenwm.Shell")
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    let shell: Rc<RefCell<Option<Rc<Shell>>>> = Rc::default();
    // sevenshell starts the shell and sevenshell command runs a command in the running one
    app.connect_command_line(move |app, command_line| {
        let shell = shell
            .borrow_mut()
            .get_or_insert_with(|| Shell::start(app))
            .clone();
        let args: Vec<String> = command_line
            .arguments()
            .iter()
            .skip(1)
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        shell.command(&args);
        glib::ExitCode::SUCCESS
    });
    app.run()
}

/// the running shell does a thing w every part in one process
struct Shell {
    app: gtk4::Application,
    bars: Rc<RefCell<Vec<Rc<Bar>>>>,
    launcher: RefCell<Option<Rc<Launcher>>>,
    window_search: RefCell<Option<Rc<WindowSearch>>>,
    keybinds: RefCell<Option<Rc<Keybinds>>>,
    notifications: Rc<Notifications>,
    osd: Rc<Osd>,
    lock: Rc<Lock>,
    /// the latest from sevenwm and the status worker so new bars can start from it
    last: Rc<Last>,
    /// keeps the shell running w no windows open
    _hold: gio::ApplicationHoldGuard,
}

impl Shell {
    fn start(app: &gtk4::Application) -> Rc<Self> {
        config::reload();
        config::publish_defaults();
        let css = gtk4::CssProvider::new();
        css.load_from_string(&format!(
            "{CSS}{}{}{}{}{}{}",
            launcher::CSS,
            windows::CSS,
            keybinds::CSS,
            notifications::CSS,
            osd::CSS,
            lock::CSS
        ));
        if let Some(display) = gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &css,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        let shell = Rc::new(Self {
            app: app.clone(),
            bars: Rc::default(),
            launcher: RefCell::new(None),
            window_search: RefCell::new(None),
            keybinds: RefCell::new(None),
            notifications: Notifications::new(app),
            osd: Osd::new(app),
            lock: Lock::new(),
            last: Rc::default(),
            _hold: app.hold(),
        });

        rebuild_bars(app, &shell.bars, &shell.last);
        // monitors plugged in or out get a bar each
        if let Some(display) = gdk::Display::default() {
            let (app, bars, last) = (app.clone(), shell.bars.clone(), shell.last.clone());
            display
                .monitors()
                .connect_items_changed(move |_, _, _, _| rebuild_bars(&app, &bars, &last));
        }
        status::request(status::Request::Fast);
        request_perf(&shell.bars);

        // sevenwms state as it changes and status readings as they come
        let updates = ipc::subscribe();
        let (bars, last, osd) = (shell.bars.clone(), shell.last.clone(), shell.osd.clone());
        glib::timeout_add_local(Duration::from_millis(33), move || {
            if let Some(state) = updates.try_iter().last() {
                for bar in bars.borrow().iter() {
                    bar.update(&state);
                }
                *last.state.borrow_mut() = Some(state);
            }
            for update in status::updates() {
                for bar in bars.borrow().iter() {
                    bar.show_status(&update);
                }
                match update {
                    status::Update::Fast(fast) => *last.fast.borrow_mut() = Some(fast),
                    status::Update::Perf(json) => *last.perf.borrow_mut() = Some(json),
                    status::Update::Osd(reading) => osd.show_reading(&reading),
                }
            }
            glib::ControlFlow::Continue
        });
        let this = Rc::downgrade(&shell);
        glib::timeout_add_seconds_local(1, move || {
            let Some(shell) = this.upgrade() else {
                return glib::ControlFlow::Break;
            };
            for bar in shell.bars.borrow().iter() {
                bar.tick_clock();
            }
            // the config file changed so apply it and only rebuild bars whose settings changed
            let before = config::get();
            if let Some(after) = config::reload() {
                if after.bar != before.bar {
                    rebuild_bars(&shell.app, &shell.bars, &shell.last);
                }
                shell.notifications.apply_config();
                shell.osd.apply_config();
            }
            glib::ControlFlow::Continue
        });
        // once for all bars on the worker thread
        glib::timeout_add_seconds_local(2, || {
            status::request(status::Request::Fast);
            glib::ControlFlow::Continue
        });
        let bars = shell.bars.clone();
        glib::timeout_add_seconds_local(5, move || {
            request_perf(&bars);
            glib::ControlFlow::Continue
        });
        shell
    }

    fn command(self: &Rc<Self>, args: &[String]) {
        match args.first().map(String::as_str) {
            None => {}
            Some("launcher") => self.toggle_launcher(),
            Some("windows") => self.toggle_window_search(),
            Some("keybinds") => self.toggle_keybinds(),
            Some(what @ ("volume" | "mic" | "brightness")) => {
                self.osd.command(what, args.get(1).map(String::as_str))
            }
            Some("notifications") if args.get(1).map(String::as_str) == Some("clear") => {
                self.notifications.clear_history()
            }
            Some("notifications") if args.get(1).map(String::as_str) == Some("open") => {
                self.notifications.open_newest()
            }
            Some("lock") => self.lock.lock(),
            Some("quit") => self.app.quit(),
            Some(other) => eprintln!("sevenshell: unknown command '{other}'"),
        }
    }

    /// does the launcher thing where it opens or closes if its open
    fn toggle_launcher(self: &Rc<Self>) {
        // take it out first bc closing runs the close handler which borrows too
        let open = self.launcher.borrow_mut().take();
        if let Some(open) = open {
            open.window.close();
            return;
        }
        let launcher = Launcher::new(&self.app);
        let this = Rc::downgrade(self);
        launcher.window.connect_close_request(move |_| {
            if let Some(this) = this.upgrade() {
                this.launcher.borrow_mut().take();
            }
            glib::Propagation::Proceed
        });
        launcher.show();
        *self.launcher.borrow_mut() = Some(launcher);
    }
}

impl Shell {
    /// open the window search or close it if its open
    fn toggle_window_search(self: &Rc<Self>) {
        // take it out first bc closing runs the close handler which borrows too
        let open = self.window_search.borrow_mut().take();
        if let Some(open) = open {
            open.window.close();
            return;
        }
        let search = WindowSearch::new(&self.app);
        let this = Rc::downgrade(self);
        search.window.connect_close_request(move |_| {
            if let Some(this) = this.upgrade() {
                this.window_search.borrow_mut().take();
            }
            glib::Propagation::Proceed
        });
        search.show();
        *self.window_search.borrow_mut() = Some(search);
    }

    /// open the keybind cheat sheet or close it if its open
    fn toggle_keybinds(self: &Rc<Self>) {
        // take it out first bc closing runs the close handler which borrows too
        let open = self.keybinds.borrow_mut().take();
        if let Some(open) = open {
            open.window.close();
            return;
        }
        let keybinds = Keybinds::new(&self.app);
        let this = Rc::downgrade(self);
        keybinds.window.connect_close_request(move |_| {
            if let Some(this) = this.upgrade() {
                this.keybinds.borrow_mut().take();
            }
            glib::Propagation::Proceed
        });
        keybinds.show();
        *self.keybinds.borrow_mut() = Some(keybinds);
    }
}

/// see Shell::last
#[derive(Default)]
struct Last {
    state: RefCell<Option<ipc::State>>,
    fast: RefCell<Option<status::Fast>>,
    perf: RefCell<Option<serde_json::Value>>,
}

/// run the perf script on the worker if a bar shows it
fn request_perf(bars: &Rc<RefCell<Vec<Rc<Bar>>>>) {
    if bars.borrow().iter().any(|b| b.wants_perf()) {
        status::request(status::Request::Perf(bar::PERF_STATUS.into()));
    }
}

fn rebuild_bars(app: &gtk4::Application, bars: &Rc<RefCell<Vec<Rc<Bar>>>>, last: &Last) {
    for bar in bars.borrow_mut().drain(..) {
        bar.window.close();
    }
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let monitors = display.monitors();
    for i in 0..monitors.n_items() {
        if let Some(monitor) = monitors.item(i).and_downcast::<gdk::Monitor>() {
            let bar = Bar::new(app, &monitor);
            // start from whats known instead of empty
            if let Some(state) = last.state.borrow().as_ref() {
                bar.update(state);
            }
            if let Some(fast) = last.fast.borrow().clone() {
                bar.show_status(&status::Update::Fast(fast));
            }
            if let Some(perf) = last.perf.borrow().clone() {
                bar.show_status(&status::Update::Perf(perf));
            }
            bar.window.present();
            bars.borrow_mut().push(bar);
        }
    }
}
