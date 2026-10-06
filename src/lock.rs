//! the lock screen like ur sddm theme that checks the password w pam and locks before sleep

use std::cell::RefCell;
use std::os::fd::OwnedFd;
use std::rc::Rc;
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{gdk, gio, glib};
use gtk4_session_lock::Instance;

use crate::auth::Screen;
use crate::config;

/// back to the clock after this long without typing
const LOGIN_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Lock {
    instance: RefCell<Option<Instance>>,
    screen: Rc<Screen>,
    /// loginds ok to finish locking before sleep
    sleep_delay: RefCell<Option<OwnedFd>>,
    /// hearing about sleep from logind as long as this is kept
    sleep_watch: RefCell<Option<gio::SignalSubscription>>,
    system_bus: RefCell<Option<gio::DBusConnection>>,
}

impl Lock {
    pub fn new() -> Rc<Self> {
        let lock = Rc::new_cyclic(|this: &std::rc::Weak<Self>| {
            let this = this.clone();
            Self {
                instance: RefCell::default(),
                screen: Screen::new(LOGIN_TIMEOUT, move || {
                    if let Some(this) = this.upgrade() {
                        this.check();
                    }
                }),
                sleep_delay: RefCell::default(),
                sleep_watch: RefCell::default(),
                system_bus: RefCell::default(),
            }
        });
        lock.watch_sleep();
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
        self.screen.buffer.set_text("");
        instance.lock();
    }

    /// forget lock windows whose monitor is gone or that got closed under us
    fn prune_views(&self, unmapped_too: bool) {
        self.screen
            .pages
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
            let has_view = self.screen.pages.borrow().iter().any(|v| v.monitor == monitor);
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
        self.screen.pages.borrow_mut().clear();
        self.screen.buffer.set_text("");
        self.screen.checking.set(false);
    }

    /// build one monitors window
    fn window(self: &Rc<Self>, monitor: &gdk::Monitor) -> gtk4::Window {
        let window = gtk4::Window::new();
        window.add_css_class("sevenshell-lock");
        crate::style::adopt(&window);
        let configured = config::get().lock.avatar.clone();
        self.screen.build(&window, monitor, 50, |login| {
            let user = glib::user_name().to_string_lossy().into_owned();
            let texture = crate::auth::avatar(&user, &glib::home_dir(), &configured)
                .and_then(|path| crate::auth::avatar_texture(&path, &window));
            if let Some(texture) = texture {
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
        });
        // debug builds only for screenshots so start on the password prompt
        #[cfg(debug_assertions)]
        if std::env::var_os("SEVENSHELL_LOCK_PROMPT").is_some() {
            self.screen.show_login();
        }
        window
    }

    /// check the password w pam off the main thread
    fn check(self: &Rc<Self>) {
        if self.screen.buffer.text().is_empty() {
            return;
        }
        let this = Rc::downgrade(self);
        self.screen.check(
            "Unlocking…",
            "the password check crashed",
            |password| authenticate(&password),
            move || {
                if let Some(this) = this.upgrade() {
                    this.unlocked_by_password();
                }
            },
        );
    }

    /// the password was right so let go of the lock
    fn unlocked_by_password(&self) {
        let instance = self.instance.borrow().clone();
        if let Some(instance) = instance {
            instance.unlock();
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
