//! the clock and password screen the lock and the login screen share w only what a password does left to each

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{gdk, gio, glib};

use crate::config;

pub const CSS: &str = "
.sevenshell-lock { background: @m3surfaceContainerLowest; }
.sevenshell-lock * { color: @m3onSurface; }
.sevenshell-lock .clock { font-family: \"Rubik\"; font-size: 84px; font-weight: 600; color: @m3primary; }
.sevenshell-lock .date { font-size: 16px; font-weight: 500; }
.sevenshell-lock .message { font-size: 13px; color: @m3onSurfaceVariant; }
.sevenshell-lock .avatar { border-radius: 9999px; }
.sevenshell-lock .username { font-size: 18px; font-weight: 500; margin-top: 12px; }
.sevenshell-lock .password {
    background: alpha(@m3surfaceContainer, 0.92);
    border-radius: 9999px;
    min-height: 44px;
    padding: 0 14px;
}
.sevenshell-lock .password entry,
.sevenshell-lock .password entry:focus-within {
    background: transparent; border: none; box-shadow: none; outline: none;
    min-height: 44px; padding: 0; font-size: 14px; caret-color: @m3primary;
}
.sevenshell-lock .password .icon { font-size: 20px; color: @m3onSurfaceVariant; }
.sevenshell-lock button.go {
    background: @m3primary;
    border: none; box-shadow: none;
    border-radius: 9999px;
    min-width: 44px; min-height: 44px; padding: 0;
    transition: background 150ms cubic-bezier(0.2, 0, 0, 1);
}
.sevenshell-lock button.go:hover { background: mix(@m3primary, @m3onPrimary, 0.12); }
.sevenshell-lock button.go label { font-size: 22px; color: @m3onPrimary; }
.sevenshell-lock .warning { font-size: 12px; margin-top: 10px; color: @m3error; }
";

/// one monitors clock and login pages
pub struct Page {
    pub monitor: gdk::Monitor,
    pub window: gtk4::Window,
    stack: gtk4::Stack,
    clock: gtk4::Label,
    date: gtk4::Label,
    entry: gtk4::Entry,
    button: gtk4::Button,
    pub warning: gtk4::Label,
}

pub struct Screen {
    pub pages: RefCell<Vec<Page>>,
    /// the password shared by every monitors box
    pub buffer: gtk4::EntryBuffer,
    pub checking: Cell<bool>,
    /// bumped on every key so the back to clock timer only acts if it didnt change
    typed: Cell<u64>,
    /// back to the clock after this long without typing
    timeout: Duration,
    /// what enter or the arrow does
    submit: Box<dyn Fn()>,
}

impl Screen {
    pub fn new(timeout: Duration, submit: impl Fn() + 'static) -> Rc<Self> {
        let screen = Rc::new(Self {
            pages: RefCell::default(),
            buffer: gtk4::EntryBuffer::new(None::<&str>),
            checking: Cell::new(false),
            typed: Cell::new(0),
            timeout,
            submit: Box::new(submit),
        });
        // one handler for the shared password buffer bc one per window piled up
        let this = Rc::downgrade(&screen);
        screen.buffer.connect_text_notify(move |_| {
            if let Some(this) = this.upgrade() {
                this.touched();
            }
        });
        let this = Rc::downgrade(&screen);
        glib::timeout_add_seconds_local(1, move || {
            let Some(this) = this.upgrade() else {
                return glib::ControlFlow::Break;
            };
            this.tick();
            glib::ControlFlow::Continue
        });
        screen
    }

    /// fill window for monitor w the clock and login pages where head goes above the password
    /// and give back the overlay for anything more on top
    pub fn build(
        self: &Rc<Self>,
        window: &impl IsA<gtk4::Window>,
        monitor: &gdk::Monitor,
        message_margin: i32,
        head: impl FnOnce(&gtk4::Box),
    ) -> gtk4::Overlay {
        let window = window.upcast_ref::<gtk4::Window>();
        let config = config::get();
        let lock = &config.lock;
        let overlay = gtk4::Overlay::new();
        let background = gtk4::Picture::new();
        background.set_content_fit(gtk4::ContentFit::Cover);
        background.set_can_shrink(true);
        if !lock.wallpaper.is_empty() {
            background.set_filename(Some(crate::theme::expand(&lock.wallpaper)));
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
            message.set_margin_bottom(message_margin);
            clock_page.add_overlay(&message);
        }
        stack.add_named(&clock_page, Some("clock"));

        // the login page w picture name and password
        let login = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        login.set_halign(gtk4::Align::Start);
        login.set_valign(gtk4::Align::Center);
        login.set_margin_start(50);
        head(&login);

        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 5);
        row.set_margin_top(10);
        row.set_halign(gtk4::Align::Center);
        let field = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        field.add_css_class("password");
        field.set_size_request(200, 30);
        let icon = crate::style::icon("lock");
        icon.add_css_class("icon");
        let entry = gtk4::Entry::with_buffer(&self.buffer);
        entry.set_visibility(false);
        entry.set_invisible_char(Some('●'));
        entry.set_placeholder_text(Some("Password"));
        entry.set_hexpand(true);
        field.append(&icon);
        field.append(&entry);
        let button = gtk4::Button::new();
        button.set_child(Some(&crate::style::icon("arrow_forward")));
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
                (this.submit)();
            }
        });
        let this = Rc::downgrade(self);
        button.connect_clicked(move |_| {
            if let Some(this) = this.upgrade() {
                (this.submit)();
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

        self.pages.borrow_mut().push(Page {
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
        overlay
    }

    pub fn showing_clock(&self) -> bool {
        self.pages
            .borrow()
            .first()
            .is_none_or(|v| v.stack.visible_child_name().as_deref() == Some("clock"))
    }

    pub fn show_login(self: &Rc<Self>) {
        for page in self.pages.borrow().iter() {
            page.stack.set_visible_child_name("login");
            page.entry.grab_focus();
        }
        self.touched();
    }

    fn show_clock(&self) {
        if self.checking.get() {
            return;
        }
        self.buffer.set_text("");
        for page in self.pages.borrow().iter() {
            page.warning.set_text("");
            page.stack.set_visible_child_name("clock");
        }
    }

    /// something got typed so restart the back to clock timer but only while theres a screen up
    fn touched(self: &Rc<Self>) {
        if self.pages.borrow().is_empty() {
            return;
        }
        let generation = self.typed.get() + 1;
        self.typed.set(generation);
        let this = Rc::downgrade(self);
        glib::timeout_add_local_once(self.timeout, move || {
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
        for page in self.pages.borrow().iter() {
            page.clock.set_text(&time);
            page.date.set_text(&date);
        }
    }

    /// run work on the password off the main thread saying busy meanwhile and call done if it passed
    pub fn check(
        self: &Rc<Self>,
        busy: &str,
        crashed: &'static str,
        work: impl FnOnce(String) -> Result<(), String> + Send + 'static,
        done: impl FnOnce() + 'static,
    ) {
        if self.checking.get() {
            return;
        }
        let password = self.buffer.text().to_string();
        self.checking.set(true);
        self.say(busy, false);
        let this = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || work(password))
                .await
                .unwrap_or_else(|_| Err(crashed.into()));
            let Some(this) = this.upgrade() else {
                return;
            };
            this.checking.set(false);
            match result {
                Ok(()) => done(),
                Err(err) => {
                    this.buffer.set_text("");
                    this.say(&err, true);
                }
            }
        });
    }

    pub fn say(&self, text: &str, done: bool) {
        for page in self.pages.borrow().iter() {
            page.warning.set_text(text);
            page.entry.set_sensitive(done);
            page.button.set_sensitive(done);
            if done {
                page.entry.grab_focus();
            }
        }
    }
}

/// someones picture from the config or the usual spots and it has to be readable bc the greeter isnt them
pub fn avatar(name: &str, home: &Path, configured: &str) -> Option<PathBuf> {
    [
        (!configured.is_empty()).then(|| crate::theme::expand(configured)),
        Some(home.join(".face")),
        Some(PathBuf::from(format!("/usr/share/sddm/faces/{name}.face.icon"))),
        Some(PathBuf::from(format!("/var/lib/AccountsService/icons/{name}"))),
    ]
    .into_iter()
    .flatten()
    .find(|p| std::fs::File::open(p).is_ok_and(|f| f.metadata().is_ok_and(|m| m.is_file())))
}

/// a picture at 120 logical px so its sharp on hidpi and never bigger
pub fn avatar_texture(path: &Path, window: &impl IsA<gtk4::Widget>) -> Option<gdk::Texture> {
    let scale = window.scale_factor().max(1);
    gtk4::gdk_pixbuf::Pixbuf::from_file_at_scale(path, 120 * scale, 120 * scale, false)
        .ok()
        .map(|pixbuf| gdk::Texture::for_pixbuf(&pixbuf))
}
