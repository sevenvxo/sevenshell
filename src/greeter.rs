//! the login screen for greetd that looks like the lock screen w a session picker and power buttons

use std::cell::{Cell, RefCell};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{gdk, gio, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use serde_json::{Value, json};

use crate::auth::Screen;
use crate::config;

/// back to the clock after this long without typing
const LOGIN_TIMEOUT: Duration = Duration::from_secs(60);

/// where the login screen looks for sessions
const SESSIONS_DIR: &str = "/usr/share/wayland-sessions";

pub const CSS: &str = "
.sevenshell-greeter .switch {
    background: transparent; border: none; box-shadow: none;
    min-width: 32px; min-height: 32px; padding: 0; border-radius: 9999px;
}
.sevenshell-greeter .switch label { font-size: 22px; color: @m3onSurfaceVariant; }
.sevenshell-greeter .switch:hover { background: alpha(@m3onSurface, 0.08); }
.sevenshell-greeter .corner { margin: 24px; }
.sevenshell-greeter .corner button,
.sevenshell-greeter .corner dropdown > button {
    background: alpha(@m3surfaceContainer, 0.92);
    border: none; box-shadow: none; border-radius: 9999px;
    min-height: 40px; padding: 0 14px;
}
.sevenshell-greeter .corner button:hover,
.sevenshell-greeter .corner dropdown > button:hover { background: @m3secondaryContainer; }
.sevenshell-greeter .corner button.power { min-width: 40px; padding: 0; }
.sevenshell-greeter .corner button.power label { font-size: 20px; color: @m3onSurfaceVariant; }
.sevenshell-greeter .corner label { font-size: 13px; }
.sevenshell-greeter popover contents {
    background: @m3surfaceContainer; border: none; border-radius: 16px; padding: 6px;
}
.sevenshell-greeter popover label { color: @m3onSurface; }
.sevenshell-greeter popover row:hover, .sevenshell-greeter popover row:selected {
    background: @m3secondaryContainer; border-radius: 12px;
}
";

/// someone who can log in
#[derive(Clone)]
struct User {
    name: String,
    /// their full name if the account has one
    display: String,
    home: PathBuf,
}

/// a session to start from its .desktop file
#[derive(Clone)]
struct Session {
    name: String,
    exec: String,
    /// DesktopNames for XDG_CURRENT_DESKTOP
    desktop_names: String,
    /// the file name without .desktop
    id: String,
}

/// what one monitors login window has past the shared pages
struct View {
    window: gtk4::ApplicationWindow,
    avatar: gtk4::Image,
    name: gtk4::Label,
    sessions: gtk4::DropDown,
}

struct Greeter {
    app: gtk4::Application,
    users: Vec<User>,
    sessions: Vec<Session>,
    user: Cell<usize>,
    session: Cell<usize>,
    screen: Rc<Screen>,
    views: RefCell<Vec<View>>,
}

/// sevenshell greeter does a thing where it runs the login screen till someone logs in
pub fn run() -> glib::ExitCode {
    let app = gtk4::Application::builder()
        .application_id("org.sevenwm.Greeter")
        // its own process every time bc theres no session bus
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(|app| {
        let css = gtk4::CssProvider::new();
        // the login screen cant read ur home so its colors come from the lock wallpaper
        let config = crate::config::get();
        let mut theme = config.theme.clone();
        if theme.wallpaper.is_empty() {
            theme.wallpaper = config.lock.wallpaper.clone();
        }
        let palette = crate::theme::generate(&theme);
        crate::theme::set_current(&palette);
        crate::style::match_mode(palette.dark);
        css.load_from_string(&format!(
            "{}{}{}{}",
            palette.css(),
            crate::style::BASE,
            crate::auth::CSS,
            CSS
        ));
        if let Some(display) = gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &css,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        Greeter::start(app);
    });
    app.run_with_args(&[] as &[&str])
}

impl Greeter {
    fn start(app: &gtk4::Application) {
        config::reload();
        let users = users();
        let sessions = sessions();
        let remembered = Remembered::load();
        let user = remembered
            .as_ref()
            .and_then(|r| users.iter().position(|u| u.name == r.user))
            .unwrap_or(0);
        let session = remembered
            .as_ref()
            .and_then(|r| sessions.iter().position(|s| s.id == r.session))
            .or_else(|| sessions.iter().position(|s| s.id == "sevenwm"))
            .unwrap_or(0);
        let greeter = Rc::new_cyclic(|this: &std::rc::Weak<Self>| {
            let this = this.clone();
            Self {
                app: app.clone(),
                users,
                sessions,
                user: Cell::new(user),
                session: Cell::new(session),
                screen: Screen::new(LOGIN_TIMEOUT, move || {
                    if let Some(this) = this.upgrade() {
                        this.check();
                    }
                }),
                views: RefCell::default(),
            }
        });
        greeter.build_views();
        // monitors plugged in or out get a window each
        if let Some(display) = gdk::Display::default() {
            let this = Rc::downgrade(&greeter);
            display.monitors().connect_items_changed(move |_, _, _, _| {
                if let Some(this) = this.upgrade() {
                    this.build_views();
                }
            });
        }
        if greeter.sessions.is_empty() {
            greeter.screen.say(&format!("No sessions found in {SESSIONS_DIR}"), true);
        }
        if greeter.users.is_empty() {
            greeter.screen.say("No one to log in as (no users in /etc/passwd)", true);
        }
        #[cfg(debug_assertions)]
        greeter.debug_hooks();
        // the app owns the greeter so it lives as long as the program
        app.connect_shutdown(move |_| {
            let _ = &greeter;
        });
    }

    /// debug builds only for testing w a fake greetd
    #[cfg(debug_assertions)]
    fn debug_hooks(self: &Rc<Self>) {
        if std::env::var_os("SEVENSHELL_GREETER_PROMPT").is_some() {
            self.screen.show_login();
        }
        if let Some(password) = std::env::var_os("SEVENSHELL_GREETER_TEST") {
            let this = Rc::downgrade(self);
            let password = password.to_string_lossy().into_owned();
            glib::timeout_add_local_once(Duration::from_millis(1500), move || {
                if let Some(this) = this.upgrade() {
                    this.screen.show_login();
                    this.screen.buffer.set_text(&password);
                    this.check();
                }
            });
        }
    }

    /// a window on every monitor
    fn build_views(self: &Rc<Self>) {
        let showing_login = !self.screen.showing_clock();
        for view in self.views.borrow_mut().drain(..) {
            view.window.close();
        }
        self.screen.pages.borrow_mut().clear();
        let Some(display) = gdk::Display::default() else {
            return;
        };
        let monitors = display.monitors();
        for i in 0..monitors.n_items() {
            if let Some(monitor) = monitors.item(i).and_downcast::<gdk::Monitor>() {
                let view = self.view(&monitor);
                view.window.present();
                self.views.borrow_mut().push(view);
            }
        }
        self.show_user();
        if showing_login {
            self.screen.show_login();
        }
    }

    fn view(self: &Rc<Self>, monitor: &gdk::Monitor) -> View {
        let window = gtk4::ApplicationWindow::new(&self.app);
        window.init_layer_shell();
        window.set_namespace(Some("greeter"));
        window.set_layer(Layer::Overlay);
        window.set_monitor(Some(monitor));
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            window.set_anchor(edge, true);
        }
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        window.add_css_class("sevenshell-lock");
        window.add_css_class("sevenshell-greeter");
        crate::style::adopt(&window);

        let avatar = gtk4::Image::new();
        avatar.set_pixel_size(120);
        avatar.add_css_class("avatar");
        avatar.set_overflow(gtk4::Overflow::Hidden);
        avatar.set_halign(gtk4::Align::Center);
        let name = gtk4::Label::new(None);
        name.add_css_class("username");
        let overlay = self.screen.build(&window, monitor, 80, |login| {
            login.append(&avatar);
            let name_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            name_row.set_halign(gtk4::Align::Center);
            if self.users.len() > 1 {
                let previous = self.switch_button("chevron_left", -1);
                let next = self.switch_button("chevron_right", 1);
                previous.set_margin_top(10);
                next.set_margin_top(10);
                name_row.append(&previous);
                name_row.append(&name);
                name_row.append(&next);
            } else {
                name_row.append(&name);
            }
            login.append(&name_row);
        });
        // greetd and pam can say alot so let it wrap
        if let Some(page) = self.screen.pages.borrow().last() {
            page.warning.set_wrap(true);
            page.warning.set_max_width_chars(40);
        }

        // bottom corners w the session picker and power
        let names: Vec<&str> = self.sessions.iter().map(|s| s.name.as_str()).collect();
        let sessions = gtk4::DropDown::from_strings(&names);
        sessions.set_selected(self.session.get() as u32);
        sessions.set_tooltip_text(Some("Session"));
        // its at the bottom of the screen so open the list upward where theres room
        let mut child = sessions.first_child();
        while let Some(widget) = child {
            if let Some(popover) = widget.downcast_ref::<gtk4::Popover>() {
                popover.set_position(gtk4::PositionType::Top);
            }
            child = widget.next_sibling();
        }
        let this = Rc::downgrade(self);
        sessions.connect_selected_notify(move |dropdown| {
            if let Some(this) = this.upgrade() {
                this.pick_session(dropdown.selected() as usize);
            }
        });
        let left = gtk4::Box::new(gtk4::Orientation::Horizontal, 5);
        left.add_css_class("corner");
        left.set_halign(gtk4::Align::Start);
        left.set_valign(gtk4::Align::End);
        left.append(&sessions);
        overlay.add_overlay(&left);

        let right = gtk4::Box::new(gtk4::Orientation::Horizontal, 5);
        right.add_css_class("corner");
        right.set_halign(gtk4::Align::End);
        right.set_valign(gtk4::Align::End);
        for (icon, tooltip, action) in [
            ("bedtime", "Suspend", "suspend"),
            ("restart_alt", "Restart", "reboot"),
            ("power_settings_new", "Shut down", "poweroff"),
        ] {
            let power = gtk4::Button::new();
            power.set_child(Some(&crate::style::icon(icon)));
            power.add_css_class("power");
            power.set_tooltip_text(Some(tooltip));
            power.connect_clicked(move |_| {
                crate::run_detached(std::process::Command::new("systemctl").arg(action));
            });
            right.append(&power);
        }
        overlay.add_overlay(&right);

        View {
            window,
            avatar,
            name,
            sessions,
        }
    }

    /// a button that uhhh picks the previous or next user
    fn switch_button(self: &Rc<Self>, icon: &str, step: isize) -> gtk4::Button {
        let button = gtk4::Button::new();
        button.set_child(Some(&crate::style::icon(icon)));
        button.add_css_class("switch");
        let this = Rc::downgrade(self);
        button.connect_clicked(move |_| {
            if let Some(this) = this.upgrade() {
                let n = this.users.len() as isize;
                let next = (this.user.get() as isize + step).rem_euclid(n);
                this.user.set(next as usize);
                this.screen.buffer.set_text("");
                this.screen.say("", true);
                this.show_user();
            }
        });
        button
    }

    /// the picked users picture and name on every monitor
    fn show_user(&self) {
        let Some(user) = self.users.get(self.user.get()) else {
            return;
        };
        for view in self.views.borrow().iter() {
            view.name.set_text(&user.display);
            let texture = crate::auth::avatar(&user.name, &user.home, "")
                .and_then(|path| crate::auth::avatar_texture(&path, &view.window));
            view.avatar.set_paintable(texture.as_ref());
            view.avatar.set_visible(texture.is_some());
        }
    }

    /// a session got picked on one monitor so show it on the others too
    fn pick_session(&self, index: usize) {
        if index >= self.sessions.len() || index == self.session.get() {
            return;
        }
        self.session.set(index);
        for view in self.views.borrow().iter() {
            if view.sessions.selected() as usize != index {
                view.sessions.set_selected(index as u32);
            }
        }
    }

    /// log in where greetd checks the password off the main thread then starts the session once we quit
    fn check(self: &Rc<Self>) {
        let (Some(user), Some(session)) = (
            self.users.get(self.user.get()).cloned(),
            self.sessions.get(self.session.get()).cloned(),
        ) else {
            return;
        };
        let this = Rc::downgrade(self);
        let (name, start) = (user.name.clone(), session.clone());
        self.screen.check(
            "Logging in…",
            "the login crashed",
            move |password| login(&name, &password, &start),
            move || {
                Remembered {
                    user: user.name,
                    session: session.id,
                }
                .save();
                // greetd starts the session once the greeter exits
                if std::env::var_os("SEVENWM_SOCK").is_some() {
                    crate::ipc::action("quit");
                }
                if let Some(this) = this.upgrade() {
                    this.app.quit();
                }
            },
        );
    }
}

/// the normal user id range from /etc/login.defs or the usual 1000 to 60000
fn uid_range() -> std::ops::RangeInclusive<u32> {
    let defs = std::fs::read_to_string("/etc/login.defs").unwrap_or_default();
    login_defs_range(&defs)
}

fn login_defs_range(defs: &str) -> std::ops::RangeInclusive<u32> {
    let value = |key: &str| {
        defs.lines()
            .map(str::trim)
            .filter(|l| !l.starts_with('#'))
            .find_map(|l| {
                let mut words = l.split_whitespace();
                (words.next() == Some(key)).then(|| words.next()?.parse::<u32>().ok()).flatten()
            })
    };
    value("UID_MIN").unwrap_or(1000)..=value("UID_MAX").unwrap_or(60000)
}

/// people who can log in w a normal user id and a real shell
fn users() -> Vec<User> {
    let range = uid_range();
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    let mut users: Vec<User> = passwd
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            let [name, _, uid, _, gecos, home, shell] = fields.as_slice() else {
                return None;
            };
            let uid: u32 = uid.parse().ok()?;
            let real_shell = !shell.ends_with("nologin") && !shell.ends_with("false");
            if !range.contains(&uid) || !real_shell {
                return None;
            }
            let full = gecos.split(',').next().unwrap_or("").trim();
            Some(User {
                name: name.to_string(),
                display: if full.is_empty() { name } else { full }.to_string(),
                home: PathBuf::from(home),
            })
        })
        .collect();
    users.sort_by(|a, b| a.name.cmp(&b.name));
    users
}

/// the wayland sessions installed by name
fn sessions() -> Vec<Session> {
    let Ok(entries) = std::fs::read_dir(SESSIONS_DIR) else {
        return Vec::new();
    };
    let mut sessions: Vec<Session> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "desktop"))
        .filter_map(|e| {
            let path = e.path();
            let text = std::fs::read_to_string(&path).ok()?;
            let id = path.file_stem()?.to_string_lossy().into_owned();
            parse_session(&text, id)
        })
        .collect();
    sessions.sort_by_key(|s| s.name.to_lowercase());
    sessions
}

/// a sessions .desktop file name exec and desktopnames or none if hidden
fn parse_session(text: &str, id: String) -> Option<Session> {
    let (mut name, mut exec, mut desktop_names) = (None, None, String::new());
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        let Some((key, value)) = line.split_once('=').filter(|_| in_entry) else {
            continue;
        };
        match key.trim() {
            "Name" => name = Some(value.trim().to_string()),
            "Exec" => exec = Some(value.trim().to_string()),
            "DesktopNames" => desktop_names = value.trim().trim_end_matches(';').to_string(),
            "Hidden" | "NoDisplay" if value.trim() == "true" => return None,
            _ => {}
        }
    }
    let exec = exec.filter(|e| !e.is_empty())?;
    Some(Session {
        name: name.unwrap_or_else(|| id.clone()),
        exec,
        desktop_names,
        id,
    })
}

/// a .desktop exec line without its field codes
fn exec_line(exec: &str) -> String {
    exec.split_whitespace()
        .filter(|word| !(word.len() == 2 && word.starts_with('%')))
        .collect::<Vec<_>>()
        .join(" ")
}

/// the last user and session so the next login starts on them
struct Remembered {
    user: String,
    session: String,
}

impl Remembered {
    fn path() -> Option<PathBuf> {
        let cache = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
        Some(cache.join("sevenshell").join("greeter.json"))
    }

    fn load() -> Option<Self> {
        let text = std::fs::read_to_string(Self::path()?).ok()?;
        let value: Value = serde_json::from_str(&text).ok()?;
        Some(Self {
            user: value["user"].as_str()?.to_string(),
            session: value["session"].as_str().unwrap_or_default().to_string(),
        })
    }

    /// best effort bc the greeter account might have nowhere to write
    fn save(&self) {
        let Some(path) = Self::path() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, json!({ "user": self.user, "session": self.session }).to_string());
    }
}

fn send(stream: &mut UnixStream, message: &Value) -> std::io::Result<()> {
    let bytes = message.to_string().into_bytes();
    stream.write_all(&(bytes.len() as u32).to_ne_bytes())?;
    stream.write_all(&bytes)
}

fn receive(stream: &mut UnixStream) -> std::io::Result<Value> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len)?;
    let mut bytes = vec![0u8; u32::from_ne_bytes(len) as usize];
    stream.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(std::io::Error::other)
}

/// log user in thru greetd and have it start session once the greeter exits
fn login(user: &str, password: &str, session: &Session) -> Result<(), String> {
    let path = std::env::var_os("GREETD_SOCK")
        .ok_or("greetd isn't running this login screen (no GREETD_SOCK)")?;
    let mut stream = UnixStream::connect(&path).map_err(|e| format!("greetd: {e}"))?;
    // a wrong password makes pam wait so a stuck greetd cant hang the screen forever
    let _ = stream.set_read_timeout(Some(Duration::from_secs(60)));
    let io = |e: std::io::Error| format!("greetd: {e}");
    let result = authenticate(&mut stream, user, password).map_err(|e| e.map_err(io));
    match result {
        Ok(()) => {}
        Err(err) => {
            // leave greetd ready for the next try
            let _ = send(&mut stream, &json!({ "type": "cancel_session" }))
                .and_then(|()| receive(&mut stream));
            return Err(err.unwrap_or_else(|e| e));
        }
    }
    let desktop = if session.desktop_names.is_empty() {
        session.id.clone()
    } else {
        session.desktop_names.replace(';', ":")
    };
    let start = json!({
        "type": "start_session",
        // greetd runs this thru sh -c exec joined w spaces
        "cmd": [exec_line(&session.exec)],
        "env": [
            "XDG_SESSION_TYPE=wayland",
            format!("XDG_CURRENT_DESKTOP={desktop}"),
            format!("XDG_SESSION_DESKTOP={}", session.id),
        ],
    });
    send(&mut stream, &start).map_err(io)?;
    let reply = receive(&mut stream).map_err(io)?;
    if reply["type"] == "success" {
        Ok(())
    } else {
        let _ = send(&mut stream, &json!({ "type": "cancel_session" }));
        Err(format!(
            "Couldn't start {}: {}",
            session.name,
            reply["description"].as_str().unwrap_or("greetd said no")
        ))
    }
}

/// the create session back and forth up to greetds success
fn authenticate(
    stream: &mut UnixStream,
    user: &str,
    password: &str,
) -> Result<(), Result<String, std::io::Error>> {
    send(stream, &json!({ "type": "create_session", "username": user })).map_err(Err)?;
    let mut answered = false;
    // pams own notes shown on failure
    let mut notes: Vec<String> = Vec::new();
    loop {
        let reply = receive(stream).map_err(Err)?;
        match reply["type"].as_str() {
            Some("success") => return Ok(()),
            Some("error") => {
                let wrong = reply["error_type"] == "auth_error";
                let description = reply["description"].as_str().unwrap_or("").to_string();
                let message = match notes.last() {
                    Some(note) => note.clone(),
                    None if wrong => "Wrong password".into(),
                    None => description,
                };
                return Err(Ok(message));
            }
            Some("auth_message") => {
                let text = reply["auth_message"].as_str().unwrap_or("").trim().to_string();
                let response = match reply["auth_message_type"].as_str() {
                    Some("secret" | "visible") if !answered => {
                        answered = true;
                        json!(password)
                    }
                    // a second question like a one time code that this screen cant ask
                    Some("secret" | "visible") => {
                        return Err(Ok(format!("Also asked \"{text}\", which this login screen can't answer")));
                    }
                    _ => {
                        if !text.is_empty() {
                            notes.push(text);
                        }
                        Value::Null
                    }
                };
                send(stream, &json!({ "type": "post_auth_message_response", "response": response }))
                    .map_err(Err)?;
            }
            _ => return Err(Ok(format!("greetd said something unexpected: {reply}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_files_parse() {
        let session = parse_session(
            "[Desktop Entry]\nName=sevenwm\nExec=/usr/local/bin/sevenwm\nDesktopNames=sevenwm;\n\
             [Desktop Action x]\nExec=nope\n",
            "sevenwm".into(),
        )
        .unwrap();
        assert_eq!(session.exec, "/usr/local/bin/sevenwm");
        assert_eq!(session.desktop_names, "sevenwm");
        assert!(parse_session("[Desktop Entry]\nName=x\nExec=x\nHidden=true\n", "x".into()).is_none());
        assert!(parse_session("[Desktop Entry]\nName=x\n", "x".into()).is_none());
        assert_eq!(exec_line("niri-session %U"), "niri-session");
    }

    #[test]
    fn login_defs_set_the_user_ids() {
        assert_eq!(login_defs_range("# UID_MIN 5\nUID_MIN\t\t 2000\nUID_MAX 3000\n"), 2000..=3000);
        assert_eq!(login_defs_range(""), 1000..=60000);
    }
}
