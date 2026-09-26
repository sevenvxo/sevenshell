//! the lock screen like ur sddm theme that checks the password w pam and locks before sleep

use std::cell::{Cell, RefCell};
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{gdk, gio, glib};
use gtk4_session_lock::Instance;

use crate::config;

/// back to the clock after this long without typing
const LOGIN_TIMEOUT: Duration = Duration::from_secs(30);

pub const CSS: &str = "
.sevenshell-lock { background: #000000; }
.sevenshell-lock * {
    font-family: \"Red Hat Display\", \"Symbols Nerd Font\", sans-serif;
    color: #ffffff;
}
.sevenshell-lock .clock { font-size: 70px; font-weight: 900; }
.sevenshell-lock .date { font-size: 14px; font-weight: 600; }
.sevenshell-lock .message { font-size: 12px; }
.sevenshell-lock .avatar { border-radius: 9999px; }
.sevenshell-lock .username { font-size: 16px; font-weight: 700; margin-top: 10px; }
.sevenshell-lock .password {
    background: rgba(255, 255, 255, 0.15);
    border-radius: 10px;
    min-height: 30px;
    padding: 0 10px;
}
.sevenshell-lock .password entry,
.sevenshell-lock .password entry:focus-within {
    background: transparent; border: none; box-shadow: none; outline: none;
    min-height: 30px; padding: 0; font-size: 12px;
}
.sevenshell-lock .password .icon { font-family: \"Symbols Nerd Font\"; font-size: 13px; }
.sevenshell-lock button.go {
    background: rgba(255, 255, 255, 0.15);
    border: none; box-shadow: none;
    border-radius: 10px;
    min-width: 30px; min-height: 30px; padding: 0;
}
.sevenshell-lock button.go:hover { background: rgba(255, 255, 255, 0.30); }
.sevenshell-lock button.go label { font-family: \"Symbols Nerd Font\"; font-size: 14px; }
.sevenshell-lock .warning { font-size: 11px; margin-top: 10px; }
";

/// one monitors lock window
struct View {
    monitor: gdk::Monitor,
    window: gtk4::Window,
    stack: gtk4::Stack,
    clock: gtk4::Label,
    date: gtk4::Label,
    entry: gtk4::Entry,
    button: gtk4::Button,
    warning: gtk4::Label,
}

pub struct Lock {
    instance: RefCell<Option<Instance>>,
    views: RefCell<Vec<View>>,
    /// the password shared by every monitors box
    buffer: gtk4::EntryBuffer,
    checking: Cell<bool>,
    /// bumped on every key so the back to clock timer only acts if it didnt change
    typed: Rc<Cell<u64>>,
    /// loginds ok to finish locking before sleep
    sleep_delay: RefCell<Option<OwnedFd>>,
    /// hearing about sleep from logind as long as this is kept
    sleep_watch: RefCell<Option<gio::SignalSubscription>>,
    system_bus: RefCell<Option<gio::DBusConnection>>,
}

impl Lock {
    pub fn new() -> Rc<Self> {
        let lock = Rc::new(Self {
            instance: RefCell::default(),
            views: RefCell::default(),
            buffer: gtk4::EntryBuffer::new(None::<&str>),
            checking: Cell::new(false),
            typed: Rc::default(),
            sleep_delay: RefCell::default(),
            sleep_watch: RefCell::default(),
            system_bus: RefCell::default(),
        });
        lock.watch_sleep();
        // one handler for the shared password buffer bc one per window piled up
        let this = Rc::downgrade(&lock);
        lock.buffer.connect_text_notify(move |_| {
            if let Some(this) = this.upgrade()
                && this.is_locked()
            {
                this.touched();
            }
        });
        let this = Rc::downgrade(&lock);
        glib::timeout_add_seconds_local(1, move || {
            let Some(this) = this.upgrade() else {
                return glib::ControlFlow::Break;
            };
            this.tick();
            glib::ControlFlow::Continue
        });
        lock
    }

    pub fn is_locked(&self) -> bool {
        self.instance.borrow().is_some()
    }

    /// lock the screen or put a lock window back on any monitor that lost one so u can always unlock
    pub fn lock(self: &Rc<Self>) {
        if self.is_locked() {
            self.restore_windows();
            return;
        }
        if !gtk4_session_lock::is_supported() {
            eprintln!("sevenshell: the compositor can't lock the screen");
            return;
        }
        let instance = Instance::new();
        let this = Rc::downgrade(self);
        instance.connect_monitor(move |instance, monitor| {
            if let Some(this) = this.upgrade() {
                this.prune_views(false);
                let window = this.window(monitor);
                instance.assign_window_to_monitor(&window, monitor);
            }
        });
        let this = Rc::downgrade(self);
        instance.connect_locked(move |_| {
            if let Some(this) = this.upgrade() {
                // locked so logind can go ahead and sleep
                this.sleep_delay.borrow_mut().take();
            }
        });
        let this = Rc::downgrade(self);
        instance.connect_failed(move |_| {
            eprintln!("sevenshell: couldn't lock the screen (is another locker running?)");
            if let Some(this) = this.upgrade() {
                this.finish();
                this.sleep_delay.borrow_mut().take();
                this.hold_sleep();
            }
        });
        let this = Rc::downgrade(self);
        instance.connect_unlocked(move |_| {
            if let Some(this) = this.upgrade() {
                this.finish();
                // the hold got let go when the lock took so take it again for the next sleep
                this.hold_sleep();
            }
        });
        *self.instance.borrow_mut() = Some(instance.clone());
        self.buffer.set_text("");
        instance.lock();
        self.tick();
    }

    /// forget lock windows whose monitor is gone or that got closed under us
    fn prune_views(&self, unmapped_too: bool) {
        self.views
            .borrow_mut()
            .retain(|v| v.monitor.is_valid() && (!unmapped_too || v.window.is_mapped()));
    }

    /// already locked so give every monitor without a lock window a new one
    fn restore_windows(self: &Rc<Self>) {
        let Some(instance) = self.instance.borrow().clone() else {
            return;
        };
        self.prune_views(true);
        let Some(display) = gdk::Display::default() else {
            return;
        };
        let monitors = display.monitors();
        for i in 0..monitors.n_items() {
            let Some(monitor) = monitors.item(i).and_downcast::<gdk::Monitor>() else {
                continue;
            };
            let has_view = self.views.borrow().iter().any(|v| v.monitor == monitor);
            if !has_view {
                let window = self.window(&monitor);
                instance.assign_window_to_monitor(&window, &monitor);
            }
        }
    }

    /// ask logind to wait for us before sleeping unless we already did prolly
    fn hold_sleep(&self) {
        if self.sleep_delay.borrow().is_some() {
            return;
        }
        let bus = self.system_bus.borrow().clone();
        if let Some(bus) = bus {
            *self.sleep_delay.borrow_mut() = inhibit_sleep(&bus);
        }
    }

    fn finish(&self) {
        self.instance.borrow_mut().take();
        self.views.borrow_mut().clear();
        self.buffer.set_text("");
        self.checking.set(false);
    }

    /// build one monitors window
    fn window(self: &Rc<Self>, monitor: &gdk::Monitor) -> gtk4::Window {
        let config = config::get();
        let lock = &config.lock;
        let window = gtk4::Window::new();
        window.add_css_class("sevenshell-lock");

        let overlay = gtk4::Overlay::new();
        let background = gtk4::Picture::new();
        background.set_content_fit(gtk4::ContentFit::Cover);
        background.set_can_shrink(true);
        if !lock.wallpaper.is_empty() {
            background.set_filename(Some(expand(&lock.wallpaper)));
        }
        overlay.set_child(Some(&background));

        let stack = gtk4::Stack::new();
        stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
        stack.set_transition_duration(250);

        // the clock page w time and date on the left and the hint bottom left
        let clock_page = gtk4::Overlay::new();
        let time_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        time_box.set_halign(gtk4::Align::Start);
        time_box.set_valign(gtk4::Align::Center);
        time_box.set_margin_start(50);
        time_box.set_margin_bottom(50);
        let clock = gtk4::Label::new(None);
        clock.add_css_class("clock");
        let date = gtk4::Label::new(None);
        date.add_css_class("date");
        time_box.append(&clock);
        time_box.append(&date);
        clock_page.set_child(Some(&time_box));
        if !lock.message.is_empty() {
            let message = gtk4::Label::new(Some(&lock.message));
            message.add_css_class("message");
            message.set_halign(gtk4::Align::Start);
            message.set_valign(gtk4::Align::End);
            message.set_margin_start(50);
            message.set_margin_bottom(50);
            clock_page.add_overlay(&message);
        }
        stack.add_named(&clock_page, Some("clock"));

        // the login page w picture name and password
        let login = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        login.set_halign(gtk4::Align::Start);
        login.set_valign(gtk4::Align::Center);
        login.set_margin_start(50);
        let scale = window.scale_factor().max(1);
        let texture = avatar(&lock.avatar)
            .and_then(|path| {
                gtk4::gdk_pixbuf::Pixbuf::from_file_at_scale(path, 120 * scale, 120 * scale, false).ok()
            })
            .map(|pixbuf| gdk::Texture::for_pixbuf(&pixbuf));
        if let Some(texture) = texture {
            // an image at 120 logical px so its sharp on hidpi and never bigger
            let picture = gtk4::Image::from_paintable(Some(&texture));
            picture.set_pixel_size(120);
            picture.add_css_class("avatar");
            picture.set_overflow(gtk4::Overflow::Hidden);
            picture.set_halign(gtk4::Align::Center);
            login.append(&picture);
        }
        let name = gtk4::Label::new(Some(&display_name()));
        name.add_css_class("username");
        login.append(&name);

        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 5);
        row.set_margin_top(10);
        row.set_halign(gtk4::Align::Center);
        let field = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        field.add_css_class("password");
        field.set_size_request(200, 30);
        let icon = gtk4::Label::new(Some("\u{f023}"));
        icon.add_css_class("icon");
        let entry = gtk4::Entry::with_buffer(&self.buffer);
        entry.set_visibility(false);
        entry.set_invisible_char(Some('●'));
        entry.set_placeholder_text(Some("Password"));
        entry.set_hexpand(true);
        field.append(&icon);
        field.append(&entry);
        let button = gtk4::Button::with_label("\u{f061}");
        button.add_css_class("go");
        row.append(&field);
        row.append(&button);
        login.append(&row);
        let warning = gtk4::Label::new(None);
        warning.add_css_class("warning");
        login.append(&warning);
        stack.add_named(&login, Some("login"));
        stack.set_visible_child_name("clock");
        overlay.add_overlay(&stack);
        window.set_child(Some(&overlay));

        let this = Rc::downgrade(self);
        entry.connect_activate(move |_| {
            if let Some(this) = this.upgrade() {
                this.check();
            }
        });
        let this = Rc::downgrade(self);
        button.connect_clicked(move |_| {
            if let Some(this) = this.upgrade() {
                this.check();
            }
        });

        // on the clock any key or click shows the login and on the login escape goes back
        let keys = gtk4::EventControllerKey::new();
        keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let this = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(this) = this.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if this.showing_clock() {
                this.show_login();
                return glib::Propagation::Stop;
            }
            if key == gdk::Key::Escape {
                this.show_clock();
                return glib::Propagation::Stop;
            }
            this.touched();
            glib::Propagation::Proceed
        });
        window.add_controller(keys);
        let click = gtk4::GestureClick::new();
        let this = Rc::downgrade(self);
        click.connect_pressed(move |_, _, _, _| {
            if let Some(this) = this.upgrade()
                && this.showing_clock()
            {
                this.show_login();
            }
        });
        window.add_controller(click);

        // debug builds only for screenshots so start on the password prompt
        #[cfg(debug_assertions)]
        if std::env::var_os("SEVENSHELL_LOCK_PROMPT").is_some() {
            stack.set_visible_child_name("login");
        }
        self.views.borrow_mut().push(View {
            monitor: monitor.clone(),
            window: window.clone(),
            stack,
            clock,
            date,
            entry,
            button,
            warning,
        });
        self.tick();
        window
    }

    fn showing_clock(&self) -> bool {
        self.views
            .borrow()
            .first()
            .is_none_or(|v| v.stack.visible_child_name().as_deref() == Some("clock"))
    }

    fn show_login(self: &Rc<Self>) {
        for view in self.views.borrow().iter() {
            view.stack.set_visible_child_name("login");
            view.entry.grab_focus();
        }
        self.touched();
    }

    fn show_clock(&self) {
        if self.checking.get() {
            return;
        }
        self.buffer.set_text("");
        for view in self.views.borrow().iter() {
            view.warning.set_text("");
            view.stack.set_visible_child_name("clock");
        }
    }

    /// something got typed so restart the back to clock timer
    fn touched(self: &Rc<Self>) {
        let generation = self.typed.get() + 1;
        self.typed.set(generation);
        let this = Rc::downgrade(self);
        glib::timeout_add_local_once(LOGIN_TIMEOUT, move || {
            if let Some(this) = this.upgrade()
                && this.typed.get() == generation
            {
                this.show_clock();
            }
        });
    }

    fn tick(&self) {
        let Ok(now) = glib::DateTime::now_local() else {
            return;
        };
        let config = config::get();
        let time = now.format(&config.lock.clock_format).unwrap_or_default();
        let date = now.format(&config.lock.date_format).unwrap_or_default();
        for view in self.views.borrow().iter() {
            view.clock.set_text(&time);
            view.date.set_text(&date);
        }
    }

    /// check the password w pam off the main thread
    fn check(self: &Rc<Self>) {
        if self.checking.get() {
            return;
        }
        let password = self.buffer.text().to_string();
        if password.is_empty() {
            return;
        }
        self.checking.set(true);
        self.say("Unlocking…", false);
        let this = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || authenticate(&password))
                .await
                .unwrap_or_else(|_| Err("the password check crashed".into()));
            let Some(this) = this.upgrade() else {
                return;
            };
            this.checking.set(false);
            match result {
                Ok(()) => this.unlocked_by_password(),
                Err(err) => {
                    this.buffer.set_text("");
                    this.say(&err, true);
                }
            }
        });
    }

    /// the password was right so let go of the lock
    fn unlocked_by_password(&self) {
        let instance = self.instance.borrow().clone();
        if let Some(instance) = instance {
            instance.unlock();
        }
    }

    fn say(&self, text: &str, done: bool) {
        for view in self.views.borrow().iter() {
            view.warning.set_text(text);
            view.entry.set_sensitive(done);
            view.button.set_sensitive(done);
            if done {
                view.entry.grab_focus();
            }
        }
    }

    /// ask logind to wait for us before sleeping and lock when its about to
    fn watch_sleep(self: &Rc<Self>) {
        let Ok(bus) = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE) else {
            return;
        };
        let this = Rc::downgrade(self);
        let bus2 = bus.clone();
        let watch = bus.subscribe_to_signal(
            Some("org.freedesktop.login1"),
            Some("org.freedesktop.login1.Manager"),
            Some("PrepareForSleep"),
            Some("/org/freedesktop/login1"),
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                let Some(this) = this.upgrade() else {
                    return;
                };
                let Some((going,)) = signal.parameters.get::<(bool,)>() else {
                    return;
                };
                if going {
                    if config::get().lock.lock_before_sleep && !this.is_locked() {
                        this.lock(); // lets go of the delay once locked
                    } else {
                        this.sleep_delay.borrow_mut().take();
                    }
                } else {
                    // awake again so ask to be waited for next time
                    *this.sleep_delay.borrow_mut() = inhibit_sleep(&bus2);
                }
            },
        );
        *self.sleep_watch.borrow_mut() = Some(watch);
        *self.sleep_delay.borrow_mut() = inhibit_sleep(&bus);
        *self.system_bus.borrow_mut() = Some(bus);
    }
}

/// a logind delay inhibitor for sleep and dropping the fd lets it go
fn inhibit_sleep(bus: &gio::DBusConnection) -> Option<OwnedFd> {
    let (_, fds) = bus
        .call_with_unix_fd_list_sync(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
            "Inhibit",
            Some(&("sleep", "sevenshell", "Lock the screen first", "delay").to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            2000,
            None::<&gio::UnixFDList>,
            gio::Cancellable::NONE,
        )
        .ok()?;
    fds?.get(0).ok()
}

/// check the password for the current user w pam
fn authenticate(password: &str) -> Result<(), String> {
    let user = glib::user_name().to_string_lossy().into_owned();
    // our own service file if there is one or else swaylocks or else logins
    let service = ["sevenshell", "swaylock", "login"]
        .into_iter()
        .find(|s| std::path::Path::new("/etc/pam.d").join(s).exists())
        .unwrap_or("login");
    let conversation = pam_client::conv_mock::Conversation::with_credentials(&user, password);
    let mut context = pam_client::Context::new(service, Some(&user), conversation)
        .map_err(|e| e.to_string())?;
    // only the password check like other lockers so an expired account cant trap u behind the lock
    context
        .authenticate(pam_client::Flag::NONE)
        .map_err(|e| friendly(&e.to_string()))
}

fn friendly(error: &str) -> String {
    if error.contains("Authentication failure") {
        "Wrong password".into()
    } else {
        error.to_string()
    }
}

/// ur name like the login screen shows it
fn display_name() -> String {
    glib::user_name().to_string_lossy().into_owned()
}

/// ur picture from the config or the usual spots
fn avatar(configured: &str) -> Option<PathBuf> {
    let user = glib::user_name().to_string_lossy().into_owned();
    let candidates = [
        (!configured.is_empty()).then(|| PathBuf::from(expand(configured))),
        Some(glib::home_dir().join(".face")),
        Some(PathBuf::from(format!("/usr/share/sddm/faces/{user}.face.icon"))),
        Some(PathBuf::from(format!("/var/lib/AccountsService/icons/{user}"))),
    ];
    candidates.into_iter().flatten().find(|p| p.is_file())
}

pub(crate) fn expand(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => glib::home_dir().join(rest).to_string_lossy().into_owned(),
        None => path.to_string(),
    }
}
