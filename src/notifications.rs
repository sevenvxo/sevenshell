//! the notification daemon w popups under the bar and a history

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{gdk, gio, glib};
use gtk4_layer_shell::{Edge, Layer, LayerShell};

use crate::config;
use crate::ipc;

pub const CSS: &str = "
.notifications { background: transparent; }
.notifications * {
    font-family: \"Adwaita Sans\", \"Symbols Nerd Font\", sans-serif;
    font-size: 13px;
    color: #ffffff;
}
.notifications .card {
    background: #000000;
    border: 1px solid #ffffff;
    border-radius: 7px;
    padding: 12px 14px;
}
.notifications .card.critical { border-width: 2px; }
.notifications .app { font-size: 11px; font-weight: bold; }
.notifications .summary { font-size: 14px; font-weight: bold; }
.notifications button {
    background: #000000; border: 1px solid #000000; box-shadow: none;
    border-radius: 4px; padding: 4px 8px; min-height: 0;
}
.notifications button:hover { border: 1px dashed #ffffff; }
";

const INTROSPECTION: &str = r#"
<node>
  <interface name="org.freedesktop.Notifications">
    <method name="GetCapabilities">
      <arg direction="out" type="as"/>
    </method>
    <method name="Notify">
      <arg direction="in" type="s" name="app_name"/>
      <arg direction="in" type="u" name="replaces_id"/>
      <arg direction="in" type="s" name="app_icon"/>
      <arg direction="in" type="s" name="summary"/>
      <arg direction="in" type="s" name="body"/>
      <arg direction="in" type="as" name="actions"/>
      <arg direction="in" type="a{sv}" name="hints"/>
      <arg direction="in" type="i" name="expire_timeout"/>
      <arg direction="out" type="u" name="id"/>
    </method>
    <method name="CloseNotification">
      <arg direction="in" type="u" name="id"/>
    </method>
    <method name="GetServerInformation">
      <arg direction="out" type="s" name="name"/>
      <arg direction="out" type="s" name="vendor"/>
      <arg direction="out" type="s" name="version"/>
      <arg direction="out" type="s" name="spec_version"/>
    </method>
    <signal name="NotificationClosed">
      <arg type="u" name="id"/>
      <arg type="u" name="reason"/>
    </signal>
    <signal name="ActionInvoked">
      <arg type="u" name="id"/>
      <arg type="s" name="action_key"/>
    </signal>
    <signal name="ActivationToken">
      <arg type="u" name="id"/>
      <arg type="s" name="activation_token"/>
    </signal>
  </interface>
</node>
"#;

/// why a notification closed w the specs numbers
#[derive(Clone, Copy)]
enum Reason {
    Expired = 1,
    Dismissed = 2,
    Closed = 3,
}

#[derive(Clone, Debug)]
pub struct Notification {
    pub id: u32,
    pub app_name: String,
    pub icon: String,
    pub summary: String,
    pub body: String,
    /// key and label pairs where default is clicking the notification
    pub actions: Vec<(String, String)>,
    pub critical: bool,
    pub time: glib::DateTime,
    /// the apps .desktop name without .desktop if it said
    pub desktop_entry: Option<String>,
    /// the process that sent it
    pub pid: Option<u32>,
}

pub struct Notifications {
    window: gtk4::ApplicationWindow,
    stack: gtk4::Box,
    cards: RefCell<HashMap<u32, gtk4::Box>>,
    next_id: Cell<u32>,
    connection: RefCell<Option<gio::DBusConnection>>,
    pub history: RefCell<Vec<Notification>>,
}

impl Notifications {
    pub fn new(app: &gtk4::Application) -> Rc<Self> {
        let window = gtk4::ApplicationWindow::new(app);
        window.init_layer_shell();
        window.set_namespace(Some("notifications"));
        window.set_layer(Layer::Overlay);
        window.add_css_class("notifications");
        let stack = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        window.set_child(Some(&stack));

        let this = Rc::new(Self {
            window,
            stack,
            cards: RefCell::default(),
            next_id: Cell::new(1),
            connection: RefCell::default(),
            history: RefCell::default(),
        });
        this.apply_config();
        // an old runs pictures belong to no one now
        if let Some(dir) = images_dir() {
            let _ = std::fs::remove_dir_all(dir);
        }
        this.write_history();
        this.serve();
        this
    }

    /// place and size the popups like the config says
    pub fn apply_config(&self) {
        let config = config::get();
        let (top, bottom, left, right) = config.notifications.position.anchors();
        let edges = [
            (Edge::Top, top),
            (Edge::Bottom, bottom),
            (Edge::Left, left),
            (Edge::Right, right),
        ];
        // unanchor first or the surface would ask to be stretched between two edges
        for (edge, on) in edges {
            if !on {
                self.window.set_anchor(edge, false);
                self.window.set_margin(edge, 0);
            }
        }
        let n = &config.notifications;
        for (edge, on) in edges {
            if on {
                self.window.set_anchor(edge, true);
                let margin = match edge {
                    Edge::Left | Edge::Right => n.margin_x,
                    _ => n.margin_y,
                };
                self.window.set_margin(edge, margin);
            }
        }
        self.stack.set_size_request(config.notifications.width, -1);
    }

    /// take the notification service name on the uhhh session bus
    fn serve(self: &Rc<Self>) {
        let this = Rc::downgrade(self);
        gio::bus_own_name(
            gio::BusType::Session,
            "org.freedesktop.Notifications",
            gio::BusNameOwnerFlags::REPLACE | gio::BusNameOwnerFlags::ALLOW_REPLACEMENT,
            move |connection, _| {
                let Some(this) = this.upgrade() else {
                    return;
                };
                let info = gio::DBusNodeInfo::for_xml(INTROSPECTION)
                    .ok()
                    .and_then(|node| node.lookup_interface("org.freedesktop.Notifications"));
                let Some(info) = info else {
                    return;
                };
                let handler = Rc::downgrade(&this);
                let registered = connection
                    .register_object("/org/freedesktop/Notifications", &info)
                    .method_call(move |_, _, _, _, method, params, invocation| {
                        if let Some(this) = handler.upgrade() {
                            this.handle(method, &params, invocation);
                        }
                    })
                    .build();
                if let Err(err) = registered {
                    eprintln!("sevenshell: notifications: {err}");
                }
                *this.connection.borrow_mut() = Some(connection);
            },
            |_, _| {},
            |_, name| {
                eprintln!("sevenshell: another program owns {name}; not showing notifications")
            },
        );
    }

    fn handle(
        self: &Rc<Self>,
        method: &str,
        params: &glib::Variant,
        invocation: gio::DBusMethodInvocation,
    ) {
        match method {
            "GetCapabilities" => {
                let caps = vec![
                    "actions",
                    "body",
                    "body-markup",
                    "persistence",
                    "icon-static",
                ];
                invocation.return_value(Some(&(caps,).to_variant()));
            }
            "GetServerInformation" => {
                invocation.return_value(Some(
                    &("sevenshell", "sevenwm", env!("CARGO_PKG_VERSION"), "1.2").to_variant(),
                ));
            }
            "CloseNotification" => {
                if let Some((id,)) = params.get::<(u32,)>() {
                    self.close(id, Reason::Closed);
                }
                invocation.return_value(None);
            }
            "Notify" => {
                type Args = (
                    String,
                    u32,
                    String,
                    String,
                    String,
                    Vec<String>,
                    HashMap<String, glib::Variant>,
                    i32,
                );
                let Some((app_name, replaces, icon, summary, body, actions, hints, timeout)) =
                    params.get::<Args>()
                else {
                    invocation.return_dbus_error(
                        "org.freedesktop.DBus.Error.InvalidArgs",
                        "bad Notify arguments",
                    );
                    return;
                };
                // only an id we gave out can be replaced and any other counts as 0
                let known = replaces != 0
                    && (self.cards.borrow().contains_key(&replaces)
                        || self.history.borrow().iter().any(|n| n.id == replaces));
                let id = if known {
                    replaces
                } else {
                    let id = self.next_id.get();
                    self.next_id.set(id + 1);
                    id
                };
                let critical = hints
                    .get("urgency")
                    .and_then(|u| u.get::<u8>())
                    .is_some_and(|u| u == 2);
                // the specs order is raw pixel picture then a picture path then the app icon
                let image_data = ["image-data", "image_data", "icon_data"]
                    .iter()
                    .find_map(|key| hints.get(*key))
                    .and_then(|data| save_image_data(data, id));
                let image_path = ["image-path", "image_path"]
                    .iter()
                    .find_map(|key| hints.get(*key))
                    .and_then(|p| p.get::<String>())
                    .filter(|p| !p.is_empty());
                let icon = image_data
                    .or(image_path)
                    .unwrap_or(icon);
                let desktop_entry = hints
                    .get("desktop-entry")
                    .and_then(|d| d.get::<String>())
                    .map(|d| d.trim_end_matches(".desktop").to_string())
                    .filter(|d| !d.is_empty());
                let pid = invocation.sender().and_then(|sender| {
                    invocation
                        .connection()
                        .call_sync(
                            Some("org.freedesktop.DBus"),
                            "/org/freedesktop/DBus",
                            "org.freedesktop.DBus",
                            "GetConnectionUnixProcessID",
                            Some(&(sender.as_str(),).to_variant()),
                            None,
                            gio::DBusCallFlags::NONE,
                            200,
                            gio::Cancellable::NONE,
                        )
                        .ok()
                        .and_then(|reply| reply.get::<(u32,)>())
                        .map(|(pid,)| pid)
                });
                let notification = Notification {
                    id,
                    app_name,
                    icon,
                    summary,
                    body,
                    actions: actions
                        .chunks(2)
                        .filter(|pair| pair.len() == 2)
                        .map(|pair| (pair[0].clone(), pair[1].clone()))
                        .collect(),
                    critical,
                    time: glib::DateTime::now_local()
                        .unwrap_or_else(|_| glib::DateTime::now_utc().unwrap()),
                    desktop_entry,
                    pid,
                };
                invocation.return_value(Some(&(id,).to_variant()));
                self.show(notification, timeout);
            }
            _ => invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", method),
        }
    }

    fn show(self: &Rc<Self>, notification: Notification, timeout: i32) {
        let dropped = {
            let mut history = self.history.borrow_mut();
            let mut dropped: Vec<Notification> = Vec::new();
            history.retain(|n| {
                let keep = n.id != notification.id;
                if !keep {
                    dropped.push(n.clone());
                }
                keep
            });
            history.insert(0, notification.clone());
            let limit = config::get().notifications.history;
            if history.len() > limit {
                dropped.extend(history.drain(limit..));
            }
            dropped
        };
        self.forget_images(&dropped);
        self.write_history();
        if config::get().notifications.do_not_disturb && !notification.critical {
            return;
        }
        let card = self.card(&notification);
        if let Some(old) = self
            .cards
            .borrow_mut()
            .insert(notification.id, card.clone())
        {
            self.stack.remove(&old);
        }
        // newest nearest the screen edge
        if !config::get().notifications.position.anchors().1 {
            self.stack.prepend(&card);
        } else {
            self.stack.append(&card);
        }
        self.window.present();

        // -1 is the server default and 0 is never and critical ones stay till dismissed
        let ms = match timeout {
            _ if notification.critical => 0,
            -1 => config::get().notifications.timeout,
            t if t > 0 => t as u32,
            _ => 0,
        };
        if ms > 0 {
            let this = Rc::downgrade(self);
            let (id, card) = (notification.id, card.downgrade());
            glib::timeout_add_local_once(Duration::from_millis(ms as u64), move || {
                // only if the same card is still up
                if let (Some(this), Some(card)) = (this.upgrade(), card.upgrade())
                    && this.cards.borrow().get(&id) == Some(&card)
                {
                    this.close(id, Reason::Expired);
                }
            });
        }
    }

    fn card(self: &Rc<Self>, n: &Notification) -> gtk4::Box {
        let card = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        card.add_css_class("card");
        let height = config::get().notifications.height;
        if height > 0 {
            card.set_size_request(-1, height);
        }
        if n.critical {
            card.add_css_class("critical");
        }
        if !n.icon.is_empty() {
            let image = if n.icon.starts_with('/') || n.icon.starts_with("file://") {
                gtk4::Image::from_file(n.icon.trim_start_matches("file://"))
            } else {
                gtk4::Image::from_icon_name(&n.icon)
            };
            image.set_pixel_size(36);
            image.set_valign(gtk4::Align::Start);
            card.append(&image);
        }
        let text = gtk4::Box::new(gtk4::Orientation::Vertical, 3);
        text.set_hexpand(true);
        if !n.app_name.is_empty() {
            let app = gtk4::Label::new(Some(&n.app_name));
            app.add_css_class("app");
            app.set_xalign(0.0);
            text.append(&app);
        }
        let summary = gtk4::Label::new(Some(&n.summary));
        summary.add_css_class("summary");
        summary.set_xalign(0.0);
        summary.set_wrap(true);
        text.append(&summary);
        if !n.body.is_empty() {
            let body = gtk4::Label::new(None);
            // bodies can have simple markup so show it plain if it doesnt parse
            if gtk4::pango::parse_markup(&n.body, '\0').is_ok() {
                body.set_markup(&n.body);
            } else {
                body.set_text(&n.body);
            }
            body.set_xalign(0.0);
            body.set_wrap(true);
            body.set_max_width_chars(40);
            text.append(&body);
        }
        let buttons: Vec<_> = n
            .actions
            .iter()
            .filter(|(key, _)| key != "default")
            .collect();
        if !buttons.is_empty() {
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            row.set_margin_top(4);
            for (key, label) in buttons {
                let button = gtk4::Button::with_label(label);
                let this = Rc::downgrade(self);
                let (id, key) = (n.id, key.clone());
                button.connect_clicked(move |_| {
                    if let Some(this) = this.upgrade() {
                        this.invoke(id, &key);
                    }
                });
                row.append(&button);
            }
            text.append(&row);
        }
        card.append(&text);

        // click goes to the app and right click js dismisses
        let click = gtk4::GestureClick::builder().button(0).build();
        let this = Rc::downgrade(self);
        let id = n.id;
        click.connect_released(move |gesture, _, _, _| {
            let Some(this) = this.upgrade() else {
                return;
            };
            if gesture.current_button() == 1 {
                this.open(id);
            } else {
                this.close(id, Reason::Dismissed);
            }
        });
        card.add_controller(click);
        card
    }

    /// sevenshell notifications open acts like the newest popup got clicked
    pub fn open_newest(&self) {
        let newest = self.cards.borrow().keys().max().copied();
        if let Some(id) = newest {
            self.open(id);
        }
    }

    /// a click runs the apps default action and focuses its window or starts the app if theres none
    fn open(&self, id: u32) {
        let notification = self.history.borrow().iter().find(|n| n.id == id).cloned();
        let has_default = notification
            .as_ref()
            .is_some_and(|n| n.actions.iter().any(|(key, _)| key == "default"));
        if has_default {
            if let Some(token) = activation_token() {
                self.emit("ActivationToken", &(id, token.as_str()).to_variant());
            }
            self.emit("ActionInvoked", &(id, "default").to_variant());
        }
        self.close(id, Reason::Dismissed);
        let Some(n) = notification else {
            return;
        };
        if focus_app(&n) {
            return;
        }
        if has_default {
            glib::timeout_add_local_once(Duration::from_millis(1500), move || {
                if find_window(&n).is_none() {
                    launch_app(&n);
                }
            });
        } else {
            launch_app(&n);
        }
    }

    fn invoke(&self, id: u32, key: &str) {
        self.emit("ActionInvoked", &(id, key).to_variant());
        self.close(id, Reason::Dismissed);
    }

    fn close(&self, id: u32, reason: Reason) {
        if let Some(card) = self.cards.borrow_mut().remove(&id) {
            self.stack.remove(&card);
        }
        if self.cards.borrow().is_empty() {
            self.window.set_visible(false);
        }
        self.emit("NotificationClosed", &(id, reason as u32).to_variant());
    }

    fn emit(&self, signal: &str, args: &glib::Variant) {
        if let Some(connection) = self.connection.borrow().as_ref() {
            let _ = connection.emit_signal(
                None,
                "/org/freedesktop/Notifications",
                "org.freedesktop.Notifications",
                signal,
                Some(args),
            );
        }
    }

    /// keep the history in notifications.json newest first for the settings app
    fn write_history(&self) {
        let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir).join("sevenshell");
        let history: Vec<_> = self
            .history
            .borrow()
            .iter()
            .map(|n| {
                serde_json::json!({
                    "id": n.id,
                    "app": n.app_name,
                    "icon": n.icon,
                    "summary": n.summary,
                    "body": n.body,
                    "critical": n.critical,
                    "time": n.time.format_iso8601().map(|t| t.to_string()).unwrap_or_default(),
                })
            })
            .collect();
        let _ = std::fs::create_dir_all(&dir);
        let tmp = dir.join("notifications.json.tmp");
        if std::fs::write(&tmp, serde_json::Value::from(history).to_string()).is_ok() {
            let _ = std::fs::rename(tmp, dir.join("notifications.json"));
        }
    }

    /// forget the history
    pub fn clear_history(&self) {
        let dropped: Vec<Notification> = self.history.borrow_mut().drain(..).collect();
        self.forget_images(&dropped);
        self.write_history();
    }

    /// delete saved pictures of notifications that left the history unless one still uses the file
    fn forget_images(&self, dropped: &[Notification]) {
        let Some(dir) = images_dir() else {
            return;
        };
        let history = self.history.borrow();
        for n in dropped {
            let path = std::path::Path::new(&n.icon);
            if path.starts_with(&dir) && !history.iter().any(|h| h.icon == n.icon) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

/// a fresh activation token from sevenwm that lets whoever gets it raise a window
fn activation_token() -> Option<String> {
    let context = gdk::Display::default()?.app_launch_context();
    context
        .startup_notify_id(None::<&gio::AppInfo>, &[])
        .map(|token| token.to_string())
}

/// the window of the app that sent n by process or by app id
fn find_window(n: &Notification) -> Option<ipc::WindowInfo> {
    let state: ipc::State =
        serde_json::from_value(ipc::request(serde_json::json!({ "get": "state" })).ok()?).ok()?;
    if let Some(pid) = n.pid {
        // the senders own window wins then the nearest parent process so an app started from kitty doesnt lose to kitty
        let found = state
            .windows
            .iter()
            .filter_map(|w| Some((ancestor_distance(w.pid? as u32, pid)?, w)))
            .min_by_key(|(distance, _)| *distance);
        if let Some((_, window)) = found {
            return Some(window.clone());
        }
    }
    let names: Vec<String> = [n.desktop_entry.clone(), Some(n.app_name.clone())]
        .into_iter()
        .flatten()
        .map(|name| name.to_lowercase())
        .filter(|name| !name.is_empty())
        .collect();
    state
        .windows
        .iter()
        .find(|w| names.iter().any(|name| same_app(&w.app_id.to_lowercase(), name)))
        .cloned()
}

/// a raw pixel notification picture saved as a png so it shows like any picture file
fn save_image_data(data: &glib::Variant, id: u32) -> Option<String> {
    let (width, height, rowstride, alpha, bits, channels, bytes) =
        data.get::<(i32, i32, i32, bool, i32, i32, Vec<u8>)>()?;
    let wanted_channels = if alpha { 4 } else { 3 };
    if width <= 0 || height <= 0 || bits != 8 || channels != wanted_channels {
        return None;
    }
    // gdk-pixbuf refuses short rows or too little data so check first
    let row = width as usize * channels as usize;
    let (stride, rows) = (usize::try_from(rowstride).ok()?, height as usize);
    let needed = stride.checked_mul(rows - 1)?.checked_add(row)?;
    if stride < row || bytes.len() < needed || bytes.len() < row.checked_mul(rows)? {
        return None;
    }
    let pixbuf = gtk4::gdk_pixbuf::Pixbuf::from_bytes(
        &glib::Bytes::from_owned(bytes),
        gtk4::gdk_pixbuf::Colorspace::Rgb,
        alpha,
        bits,
        width,
        height,
        rowstride,
    );
    let dir = images_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{id}.png"));
    pixbuf.savev(&path, "png", &[]).ok()?;
    Some(path.to_string_lossy().into_owned())
}

/// where raw pixel notification pictures get saved
fn images_dir() -> Option<std::path::PathBuf> {
    Some(
        std::path::PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?)
            .join("sevenshell")
            .join("images"),
    )
}

/// how many steps up from pid its family tree hits ancestor or none if it didnt start it
fn ancestor_distance(ancestor: u32, pid: u32) -> Option<u32> {
    let mut pid = pid;
    for distance in 0..32 {
        if pid == ancestor {
            return Some(distance);
        }
        if pid <= 1 {
            return None;
        }
        pid = parent_pid(pid)?;
    }
    None
}

fn parent_pid(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // comm can have spaces so start after the last closing paren
    stat.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse().ok()
}

/// names like org.telegram.desktop and telegram prolly count as the same app
fn same_app(app_id: &str, name: &str) -> bool {
    const GENERIC: [&str; 9] = ["org", "com", "io", "net", "github", "desktop", "app", "gnome", "kde"];
    let parts = |s: &str| -> Vec<String> {
        s.to_lowercase()
            .split(['.', '-', '_', ' '])
            .filter(|p| p.len() > 1 && !GENERIC.contains(p))
            .map(str::to_string)
            .collect()
    };
    let simple = |s: &str| s.to_lowercase().replace([' ', '_'], "-");
    if simple(app_id) == simple(name) {
        return true;
    }
    // the main word of each
    let (a, b) = (parts(app_id), parts(name));
    match (a.first(), b.first()) {
        (Some(x), Some(y)) => a.contains(y) || b.contains(x),
        _ => false,
    }
}

/// focus the app window for n and sevenwm flies to it
fn focus_app(n: &Notification) -> bool {
    let Some(window) = find_window(n) else {
        return false;
    };
    ipc::request(serde_json::json!({ "focus": window.id })).is_ok()
}

/// start the app that sent n from its .desktop file
fn launch_app(n: &Notification) {
    let by_entry = n
        .desktop_entry
        .as_ref()
        .and_then(|entry| gio_unix::DesktopAppInfo::new(&format!("{entry}.desktop")))
        .map(|info| info.upcast::<gio::AppInfo>());
    let name = n.app_name.to_lowercase();
    let info = by_entry.or_else(|| {
        (!name.is_empty())
            .then(|| {
                gio::AppInfo::all().into_iter().find(|info| {
                    info.display_name().to_lowercase() == name
                        || info.id().is_some_and(|id| {
                            same_app(&id.trim_end_matches(".desktop").to_lowercase(), &name)
                        })
                })
            })
            .flatten()
    });
    let Some(info) = info else {
        eprintln!("sevenshell: no app to open for a notification from '{}'", n.app_name);
        return;
    };
    let context = gdk::Display::default().map(|d| d.app_launch_context());
    if let Err(err) = info.launch(&[], context.as_ref()) {
        eprintln!("sevenshell: opening {}: {err}", info.display_name());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_names_match_app_ids() {
        assert!(same_app("brave-browser", "brave"));
        assert!(same_app("org.telegram.desktop", "telegram"));
        assert!(same_app("org.pulseaudio.pavucontrol", "org.pulseaudio.pavucontrol"));
        assert!(same_app("thunar", "thunar"));
        assert!(same_app("volume-control", "volume control"));
        assert!(!same_app("kitty", "thunar"));
        assert!(!same_app("brave-browser", "firefox"));
    }

    #[test]
    fn parents_are_found() {
        let me = std::process::id();
        let parent = parent_pid(me).unwrap();
        assert_eq!(ancestor_distance(me, me), Some(0));
        assert_eq!(ancestor_distance(parent, me), Some(1));
        assert_eq!(ancestor_distance(me, parent), None);
    }
}
