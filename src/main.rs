//! sevenshell is the desktop shell for sevenwm w the bar launcher search notifications osd lock screen and uhh login screen

mod auth;
mod bar;
mod calc;
mod clipboard;
mod config;
mod drives;
mod emoji;
mod feed;
mod greeter;
mod ipc;
mod keybinds;
mod launcher;
mod lock;
mod media;
mod notifications;
mod osd;
mod picker;
mod polkit;
mod quick;
mod screenshot;
mod session;
mod settings;
mod status;
mod taskmanager;
mod style;
mod theme;
mod toggles;
mod tray;
mod wake;
mod wallpapers;
mod weather;
mod windows;

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{gdk, gio, glib};

use bar::Bar;
use keybinds::Keybinds;
use launcher::Launcher;
use windows::WindowSearch;
use lock::Lock;
use notifications::Notifications;
use osd::Osd;

/// the bar in caelestia colors w round pills material icons and a rubik clock all sized from its height
/// start a program without waiting and a thread waits for it so it never stays a zombie
pub fn run_detached(cmd: &mut std::process::Command) {
    match cmd.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(err) => eprintln!("sevenshell: {:?}: {err}", cmd.get_program()),
    }
}

fn bar_css(bar: &config::Bar) -> String {
    // everything was tuned at 36px so its all scaled from there
    let s = bar.height as f64 / 36.0;
    let px = |v: f64| (v * s).round().max(1.0) as i32;
    format!(
        "
.sevenshell-bar {{ background: transparent; color: @m3onSurface; }}
.sevenshell-bar .bar-body {{ background: alpha(@m3surface, {opacity}); }}
.sevenshell-bar.floating .bar-body {{ border-radius: {radius}px; }}
.sevenshell-bar * {{ font-size: {font}px; font-weight: 500; min-height: 0; }}
.sevenshell-bar .module {{
    padding: 0 {pad}px;
    margin: {margin}px 0;
    border-radius: 9999px;
    background: transparent;
    border: none;
    box-shadow: none;
    color: @m3onSurface;
    min-width: 0;
    transition: background 200ms cubic-bezier(0.2, 0, 0, 1);
}}
.sevenshell-bar button.module:hover {{ background: alpha(@m3onSurface, 0.08); }}
.sevenshell-bar button.module:active {{ background: alpha(@m3onSurface, 0.14); }}
.sevenshell-bar button.module > label {{ padding: 0; }}
.sevenshell-bar .icon {{ font-size: {icon}px; color: @m3primary; }}
.sevenshell-bar .text {{ color: @m3onSurfaceVariant; }}
.sevenshell-bar .group {{
    background: @m3surfaceContainer;
    border-radius: 9999px;
    margin: {margin}px 0;
    padding: 0 {group}px;
}}
.sevenshell-bar .group .module {{ margin: 0; padding: 0 {grouppad}px; }}
.sevenshell-bar .os {{ margin-left: {edge}px; padding: 0 {ospad}px; }}
.sevenshell-bar .os label {{ font-family: \"Symbols Nerd Font\"; font-size: {os}px; color: @m3primary; }}
.sevenshell-bar .clock {{
    background: @m3surfaceContainer;
    padding: 0 {clockpad}px;
}}
.sevenshell-bar .clock label {{
    font-family: \"Rubik\";
    font-size: {clock}px;
    font-weight: 500;
    color: @m3onSurface;
}}
.sevenshell-bar .power {{ margin-right: {edge}px; }}
.sevenshell-bar .power .icon {{ color: @m3error; }}
.sevenshell-bar .title label {{ color: @m3onSurfaceVariant; font-weight: 400; }}
.sevenshell-bar .minimap {{ margin: 0 {minimap}px; }}
.sevenshell-bar button.toggle.on {{ background: @m3primary; }}
.sevenshell-bar button.toggle.on .icon {{ color: @m3onPrimary; }}
.sevenshell-bar .privacy {{ background: @m3errorContainer; }}
.sevenshell-bar .privacy .icon {{ color: @m3onErrorContainer; font-size: {small}px; }}
.sevenshell-bar.vertical .module {{ padding: {pad}px 0; margin: 0 {margin}px; }}
.sevenshell-bar.vertical .group {{ margin: 0 {margin}px; padding: {group}px 0; }}
.sevenshell-bar.vertical .group .module {{ margin: 0; padding: {grouppad}px 0; }}
.sevenshell-bar.vertical .os {{ margin: {edge}px 0 0 0; padding: {ospad}px 0; }}
.sevenshell-bar.vertical .power {{ margin: 0 0 {edge}px 0; }}
.sevenshell-bar.vertical .clock {{ padding: {clockpad}px 0; }}
.sevenshell-bar.vertical .minimap {{ margin: {minimap}px 0; }}
",
        opacity = bar.opacity,
        radius = px(14.0),
        font = px(13.0),
        pad = px(10.0),
        margin = px(5.0),
        icon = px(18.0),
        small = px(15.0),
        group = px(2.0),
        grouppad = px(9.0),
        edge = px(8.0),
        ospad = px(12.0),
        os = px(16.0),
        clock = px(14.0),
        clockpad = px(14.0),
        minimap = px(6.0),
    )
}

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
    if matches!(args.first().map(String::as_str), Some("help" | "--help" | "-h")) {
        println!("{USAGE}");
        return glib::ExitCode::SUCCESS;
    }
    // sevenshell --palette prints the colors ur theme settings make
    if args.first().map(String::as_str) == Some("--palette") {
        config::reload();
        let palette = theme::generate(&config::get().theme);
        println!("{}", serde_json::to_string_pretty(&palette.json()).unwrap_or_default());
        return glib::ExitCode::SUCCESS;
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
        // errors go to the terminal that ran the command not the running shells log
        match shell.command(&args) {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(err) => {
                command_line.printerr_literal(&format!("sevenshell: {err}\n"));
                glib::ExitCode::FAILURE
            }
        }
    });
    app.run()
}

/// what sevenshell help prints
const USAGE: &str = "usage: sevenshell [command]
  (none)                     start the shell or do nothing if its running
  launcher | windows | keybinds | clipboard | emoji | quick | session
  volume|mic|brightness [up|down|mute]
  notifications clear|open
  lock | screenshot | taskmanager
  tray [audio|wifi|bluetooth|power|perf|...]
  settings [--page <page>]
  quit
  greeter                    the login screen for greetd
  --palette                  print the colors ur theme makes";

/// the running shell does a thing w every part in one process
struct Shell {
    app: gtk4::Application,
    bars: Rc<RefCell<Vec<Rc<Bar>>>>,
    launcher: RefCell<Option<Rc<Launcher>>>,
    window_search: RefCell<Option<Rc<WindowSearch>>>,
    keybinds: RefCell<Option<Rc<Keybinds>>>,
    clipboard: Rc<RefCell<Option<Rc<clipboard::Clipboard>>>>,
    emoji: Rc<RefCell<Option<Rc<emoji::EmojiPicker>>>>,
    notifications: Rc<Notifications>,
    osd: Rc<Osd>,
    lock: Rc<Lock>,
    /// the latest from sevenwm and the status worker so new bars can start from it
    last: Rc<Last>,
    /// every uis css w the palette in front
    css: gtk4::CssProvider,
    /// what the palette was made from so its only redone when that changes
    theme: RefCell<Option<theme::Inputs>>,
    /// sevenwms socket and its mtime when we last gave it colors so a restarted one gets them again
    colored: RefCell<Option<(std::path::PathBuf, Option<std::time::SystemTime>)>>,
    /// keeps the shell running w no windows open
    _hold: gio::ApplicationHoldGuard,
}

impl Shell {
    fn start(app: &gtk4::Application) -> Rc<Self> {
        config::reload();
        config::publish_defaults();
        let css = gtk4::CssProvider::new();
        if let Some(display) = gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &css,
                // above the users gtk css so the top bar colors sevenshell writes there cant go stale on its own windows
                gtk4::STYLE_PROVIDER_PRIORITY_USER + 1,
            );
        }
        let shell = Rc::new(Self {
            app: app.clone(),
            bars: Rc::default(),
            launcher: RefCell::new(None),
            window_search: RefCell::new(None),
            keybinds: RefCell::new(None),
            clipboard: Rc::default(),
            emoji: Rc::default(),
            notifications: Notifications::new(app),
            osd: Osd::new(app),
            lock: Lock::new(),
            last: Rc::default(),
            css,
            theme: RefCell::new(None),
            colored: RefCell::new(None),
            _hold: app.hold(),
        });
        shell.apply_theme();

        rebuild_bars(app, &shell.bars, &shell.last);
        // monitors plugged in or out get a bar each
        if let Some(display) = gdk::Display::default() {
            let (app, bars, last) = (app.clone(), shell.bars.clone(), shell.last.clone());
            display
                .monitors()
                .connect_items_changed(move |_, _, _, _| rebuild_bars(&app, &bars, &last));
        }
        status::request(status::Request::Fast);
        status::request(status::Request::Privacy);
        request_perf(&shell.bars);
        drives::sync();
        clipboard::sync();
        polkit::start();

        // sevenwms state as it changes and status readings as they come
        let (bars, last) = (shell.bars.clone(), shell.last.clone());
        wake::each_latest(ipc::subscribe(), move |state| {
            ipc::set_latest(&state);
            for bar in bars.borrow().iter() {
                bar.update(&state);
            }
            *last.state.borrow_mut() = Some(state);
            glib::ControlFlow::Continue
        });
        let (bars, last, osd) = (shell.bars.clone(), shell.last.clone(), shell.osd.clone());
        status::watch(move |update| {
            for bar in bars.borrow().iter() {
                bar.show_status(&update);
            }
            match update {
                status::Update::Fast(fast) => *last.fast.borrow_mut() = Some(fast),
                status::Update::Perf(json) => *last.perf.borrow_mut() = Some(json),
                status::Update::Osd(reading) => osd.show_reading(&reading),
                status::Update::Privacy(_) => {}
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
                bar.show_toggles();
            }
            shell.apply_theme();
            // the config file changed so apply it and only rebuild bars whose settings changed
            let before = config::get();
            if let Some(after) = config::reload() {
                if after.bar != before.bar
                    || after.weather != before.weather
                    || after.updates != before.updates
                {
                    shell.load_css();
                    rebuild_bars(&shell.app, &shell.bars, &shell.last);
                }
                clipboard::sync();
                shell.notifications.apply_config();
                shell.osd.apply_config();
                drives::sync();
            }
            // a broken file gets a notification and at startup this waits till the tick so our own daemon is up
            if let Some(err) = config::take_error() {
                run_detached(std::process::Command::new("notify-send").args([
                    "-u",
                    "critical",
                    "-a",
                    "sevenshell",
                    "Shell config not applied",
                    &err,
                ]));
            }
            glib::ControlFlow::Continue
        });
        // once for all bars on the worker thread
        let bars = shell.bars.clone();
        glib::timeout_add_seconds_local(2, move || {
            status::request(status::Request::Fast);
            if bars.borrow().iter().any(|b| b.wants_privacy()) {
                status::request(status::Request::Privacy);
            }
            glib::ControlFlow::Continue
        });
        let bars = shell.bars.clone();
        glib::timeout_add_seconds_local(5, move || {
            request_perf(&bars);
            drives::sync();
            clipboard::sync();
            glib::ControlFlow::Continue
        });
        shell
    }

    /// every uis css w the current palette in front and the bar sized from its height
    fn load_css(&self) {
        self.css.load_from_string(&format!(
            "{}{}{}{}{}{}{}{}{}{}{}{}{}",
            theme::current().css(),
            style::BASE,
            bar_css(&config::get().bar),
            picker::CSS,
            quick::CSS,
            session::CSS,
            polkit::CSS,
            notifications::CSS,
            osd::CSS,
            auth::CSS,
            settings::CSS,
            taskmanager::CSS,
            tray::CSS
        ));
    }

    /// make the palette again if its inputs changed and give it to every ui and sevenwm
    fn apply_theme(&self) {
        let theme = config::get().theme.clone();
        let inputs = theme::inputs(&theme);
        let changed = self.theme.borrow().as_ref() != Some(&inputs);
        let palette = changed.then(|| theme::generate(&theme));
        if let Some(palette) = &palette {
            theme::save(palette);
            theme::save_gtk(palette, theme.color_gtk);
            theme::set_current(palette);
            style::match_mode(palette.dark);
            self.load_css();
            *self.theme.borrow_mut() = Some(inputs);
        }
        // sevenwm gets the colors when they change or when its a new sevenwm
        let socket = ipc::socket_path().map(|p| {
            let mtime = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
            (p, mtime)
        });
        if socket.is_none() || (palette.is_none() && *self.colored.borrow() == socket) {
            return;
        }
        let colors = if theme.color_sevenwm {
            palette.unwrap_or_else(|| theme::generate(&theme)).sevenwm_colors()
        } else {
            serde_json::Value::Null
        };
        if ipc::request(serde_json::json!({ "colors": colors })).is_ok() {
            *self.colored.borrow_mut() = socket;
        }
    }

    fn command(self: &Rc<Self>, args: &[String]) -> Result<(), String> {
        match args.first().map(String::as_str) {
            None => {}
            Some("launcher") => self.toggle_launcher(),
            Some("windows") => self.toggle_window_search(),
            Some("keybinds") => self.toggle_keybinds(),
            Some("clipboard") => toggle_picker(&self.clipboard, || clipboard::Clipboard::new(&self.app)),
            Some("emoji") => toggle_picker(&self.emoji, || emoji::EmojiPicker::new(&self.app)),
            Some("quick") => tray::toggle("quick"),
            Some("session") => session::toggle(&self.app),
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
            Some("screenshot") => screenshot::open(&self.app),
            Some("taskmanager") => taskmanager::open(&self.app),
            // sevenshell tray wifi opens that dropdown or closes it if its open
            Some("tray") => tray::toggle(args.get(1).map_or("audio", String::as_str)),
            Some("settings") => {
                // sevenshell settings --page network opens on that page
                let page = args.iter().position(|a| a == "--page").and_then(|i| args.get(i + 1));
                settings::open(&self.app, page.map(String::as_str));
            }
            Some("quit") => self.app.quit(),
            Some(other) => return Err(format!("unknown command '{other}'\n{USAGE}")),
        }
        Ok(())
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

/// open a picker in slot or close it if its open
fn toggle_picker<S: picker::Source>(
    slot: &Rc<RefCell<Option<Rc<picker::Picker<S>>>>>,
    make: impl FnOnce() -> Rc<picker::Picker<S>>,
) {
    // take it out first bc closing runs the close handler which borrows too
    let open = slot.borrow_mut().take();
    if let Some(open) = open {
        open.window.close();
        return;
    }
    let picker = make();
    let weak = Rc::downgrade(slot);
    picker.window.connect_close_request(move |_| {
        if let Some(slot) = weak.upgrade() {
            slot.borrow_mut().take();
        }
        glib::Propagation::Proceed
    });
    picker.show();
    *slot.borrow_mut() = Some(picker);
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
