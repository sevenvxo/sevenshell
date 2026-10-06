//! the polkit agent that asks for ur password when an app wants admin rights thru polkits helper socket

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{gdk, gio, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::config;

pub const CSS: &str = "
.polkit { background: alpha(@m3scrim, 0.45); }
.polkit * { color: @m3onSurface; font-size: 14px; }
.polkit .card { background: @m3surfaceContainerHigh; border-radius: 28px; padding: 24px; }
.polkit .big { font-size: 36px; color: @m3secondary; }
.polkit .title { font-size: 22px; font-weight: 500; margin-top: 8px; }
.polkit .message { color: @m3onSurfaceVariant; }
.polkit .action { font-size: 11px; color: @m3outline; }
.polkit .user { font-weight: 500; }
.polkit .error { color: @m3error; font-size: 13px; }
.polkit entry {
    background: @m3surfaceContainerHighest; border: none; box-shadow: none;
    border-radius: 9999px; padding: 8px 16px; caret-color: @m3primary; min-height: 0;
}
.polkit button {
    border: none; box-shadow: none; border-radius: 9999px; padding: 8px 20px;
    background: transparent;
}
.polkit button label { color: @m3primary; font-weight: 500; }
.polkit button.suggested { background: @m3primary; }
.polkit button.suggested label { color: @m3onPrimary; }
.polkit button:hover { background: alpha(@m3primary, 0.1); }
.polkit button.suggested:hover { background: alpha(@m3primary, 0.88); }
";

const INTROSPECTION: &str = r#"<node>
  <interface name="org.freedesktop.PolicyKit1.AuthenticationAgent">
    <method name="BeginAuthentication">
      <arg type="s" name="action_id" direction="in"/>
      <arg type="s" name="message" direction="in"/>
      <arg type="s" name="icon_name" direction="in"/>
      <arg type="a{ss}" name="details" direction="in"/>
      <arg type="s" name="cookie" direction="in"/>
      <arg type="a(sa{sv})" name="identities" direction="in"/>
    </method>
    <method name="CancelAuthentication">
      <arg type="s" name="cookie" direction="in"/>
    </method>
  </interface>
</node>"#;

const OBJECT: &str = "/org/sevenwm/PolicyKit1/AuthenticationAgent";
const HELPER_SOCKET: &str = "/run/polkit/agent-helper.socket";

type Identities = Vec<(String, HashMap<String, glib::Variant>)>;

thread_local! {
    /// the open password prompts by cookie
    static OPEN: RefCell<Vec<Rc<Prompt>>> = const { RefCell::new(Vec::new()) };
}

/// register as this sessions agent if polkit.agent is on
pub fn start() {
    if !config::get().polkit.agent {
        return;
    }
    gio::bus_get(gio::BusType::System, None::<&gio::Cancellable>, |bus| {
        let connection = match bus {
            Ok(c) => c,
            Err(err) => return eprintln!("sevenshell: polkit: no system bus: {err}"),
        };
        let Some(info) = gio::DBusNodeInfo::for_xml(INTROSPECTION)
            .ok()
            .and_then(|n| n.lookup_interface("org.freedesktop.PolicyKit1.AuthenticationAgent"))
        else {
            return;
        };
        let registered = connection
            .register_object(OBJECT, &info)
            .method_call(|_, _, _, _, method, params, invocation| match method {
                "BeginAuthentication" => begin(&params, invocation),
                "CancelAuthentication" => {
                    let cookie = params.get::<(String,)>().map(|(c,)| c).unwrap_or_default();
                    let prompt = OPEN.with(|o| o.borrow().iter().find(|p| p.cookie == cookie).cloned());
                    if let Some(prompt) = prompt {
                        prompt.finish(false);
                    }
                    invocation.return_value(None);
                }
                _ => invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", method),
            })
            .build();
        if let Err(err) = registered {
            return eprintln!("sevenshell: polkit: {err}");
        }
        let Some(subject) = subject() else {
            return eprintln!("sevenshell: polkit: cant tell which session this is");
        };
        let locale = std::env::var("LANG").unwrap_or_else(|_| "C".into());
        let args = glib::Variant::tuple_from_iter([subject, locale.to_variant(), OBJECT.to_variant()]);
        connection.call(
            Some("org.freedesktop.PolicyKit1"),
            "/org/freedesktop/PolicyKit1/Authority",
            "org.freedesktop.PolicyKit1.Authority",
            "RegisterAuthenticationAgent",
            Some(&args),
            None,
            gio::DBusCallFlags::NONE,
            -1,
            None::<&gio::Cancellable>,
            // another agent like hyprpolkitagent already has this session and thats fine
            |result| {
                if let Err(err) = result {
                    eprintln!("sevenshell: polkit agent not registered: {}", err.message());
                }
            },
        );
    });
}

/// this login session as polkit names it or this process if theres no session id
fn subject() -> Option<glib::Variant> {
    let mut details: HashMap<String, glib::Variant> = HashMap::new();
    let kind = match std::env::var("XDG_SESSION_ID") {
        Ok(id) if !id.is_empty() => {
            details.insert("session-id".into(), id.to_variant());
            "unix-session"
        }
        _ => {
            let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
            // field 22 counted after the name in brackets which can have spaces
            let after = stat.rsplit_once(')')?.1;
            let start: u64 = after.split_whitespace().nth(19)?.parse().ok()?;
            details.insert("pid".into(), std::process::id().to_variant());
            details.insert("start-time".into(), start.to_variant());
            "unix-process"
        }
    };
    Some((kind.to_string(), details).to_variant())
}

/// the user to ask for which is u if ur allowed or else the first one offered like root
fn pick_user(identities: &Identities) -> Option<String> {
    // safety getuid cant fail
    let me = unsafe { libc::getuid() };
    let uids: Vec<u32> = identities
        .iter()
        .filter(|(kind, _)| kind == "unix-user")
        .filter_map(|(_, d)| d.get("uid")?.get::<u32>())
        .collect();
    let uid = if uids.contains(&me) { me } else { *uids.first()? };
    user_name(uid)
}

fn user_name(uid: u32) -> Option<String> {
    // safety getpwuid_r only writes into what we hand it
    unsafe {
        let mut pwd: libc::passwd = std::mem::zeroed();
        let mut buf = vec![0 as libc::c_char; 4096];
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        if libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result) != 0 || result.is_null() {
            return None;
        }
        Some(std::ffi::CStr::from_ptr(pwd.pw_name).to_string_lossy().into_owned())
    }
}

fn begin(params: &glib::Variant, invocation: gio::DBusMethodInvocation) {
    type Args = (String, String, String, HashMap<String, String>, String, Identities);
    let Some((action, message, _icon, _details, cookie, identities)) = params.get::<Args>() else {
        return invocation.return_dbus_error("org.freedesktop.PolicyKit1.Error.Failed", "bad arguments");
    };
    let Some(user) = pick_user(&identities) else {
        return invocation.return_dbus_error("org.freedesktop.PolicyKit1.Error.Failed", "no user to ask for");
    };
    let Some(app) = gio::Application::default().and_downcast::<gtk4::Application>() else {
        return invocation.return_dbus_error("org.freedesktop.PolicyKit1.Error.Failed", "shell not running");
    };
    let prompt = Prompt::new(&app, &action, &message, &user, cookie, invocation);
    OPEN.with(|o| o.borrow_mut().push(prompt));
}

/// one password prompt and the call waiting on it
struct Prompt {
    window: gtk4::ApplicationWindow,
    entry: gtk4::PasswordEntry,
    error: gtk4::Label,
    ok: gtk4::Button,
    user: String,
    cookie: String,
    invocation: RefCell<Option<gio::DBusMethodInvocation>>,
}

impl Prompt {
    fn new(
        app: &gtk4::Application,
        action: &str,
        message: &str,
        user: &str,
        cookie: String,
        invocation: gio::DBusMethodInvocation,
    ) -> Rc<Self> {
        let window = gtk4::ApplicationWindow::new(app);
        window.init_layer_shell();
        window.set_namespace(Some("polkit"));
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            window.set_anchor(edge, true);
        }
        window.set_exclusive_zone(-1);
        window.add_css_class("polkit");
        crate::style::adopt(&window);

        let card = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
        card.add_css_class("card");
        card.set_halign(gtk4::Align::Center);
        card.set_valign(gtk4::Align::Center);
        card.set_size_request(420, -1);
        let big = crate::style::icon("admin_panel_settings");
        big.add_css_class("big");
        big.set_halign(gtk4::Align::Start);
        card.append(&big);
        let title = gtk4::Label::new(Some("Authentication needed"));
        title.add_css_class("title");
        title.set_xalign(0.0);
        card.append(&title);
        let text = gtk4::Label::new(Some(message));
        text.add_css_class("message");
        text.set_xalign(0.0);
        text.set_wrap(true);
        text.set_max_width_chars(50);
        card.append(&text);
        let who = gtk4::Label::new(Some(&format!("Password for {user}")));
        who.add_css_class("user");
        who.set_xalign(0.0);
        who.set_margin_top(6);
        card.append(&who);
        let entry = gtk4::PasswordEntry::new();
        entry.set_show_peek_icon(true);
        card.append(&entry);
        let error = gtk4::Label::new(None);
        error.add_css_class("error");
        error.set_xalign(0.0);
        error.set_wrap(true);
        error.set_visible(false);
        card.append(&error);
        let detail = gtk4::Label::new(Some(action));
        detail.add_css_class("action");
        detail.set_xalign(0.0);
        detail.set_selectable(true);
        card.append(&detail);
        let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        buttons.set_halign(gtk4::Align::End);
        buttons.set_margin_top(8);
        let cancel = gtk4::Button::with_label("Cancel");
        let ok = gtk4::Button::with_label("Authenticate");
        ok.add_css_class("suggested");
        buttons.append(&cancel);
        buttons.append(&ok);
        card.append(&buttons);
        window.set_child(Some(&card));

        let prompt = Rc::new(Self {
            window,
            entry,
            error,
            ok,
            user: user.to_string(),
            cookie,
            invocation: RefCell::new(Some(invocation)),
        });
        let p = Rc::downgrade(&prompt);
        prompt.ok.connect_clicked(move |_| {
            if let Some(p) = p.upgrade() {
                p.check();
            }
        });
        let p = Rc::downgrade(&prompt);
        prompt.entry.connect_activate(move |_| {
            if let Some(p) = p.upgrade() {
                p.check();
            }
        });
        let p = Rc::downgrade(&prompt);
        cancel.connect_clicked(move |_| {
            if let Some(p) = p.upgrade() {
                p.finish(false);
            }
        });
        let keys = gtk4::EventControllerKey::new();
        let p = Rc::downgrade(&prompt);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape
                && let Some(p) = p.upgrade()
            {
                p.finish(false);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        prompt.window.add_controller(keys);
        // closing it any other way counts as cancel
        let p = Rc::downgrade(&prompt);
        prompt.window.connect_close_request(move |_| {
            if let Some(p) = p.upgrade() {
                p.answer(false);
            }
            glib::Propagation::Proceed
        });
        prompt.window.present();
        prompt.entry.grab_focus();
        prompt
    }

    /// hand the password to polkits helper off the gtk thread
    fn check(self: &Rc<Self>) {
        let password = self.entry.text().to_string();
        if password.is_empty() || !self.ok.is_sensitive() {
            return;
        }
        self.ok.set_sensitive(false);
        self.entry.set_sensitive(false);
        self.error.set_visible(false);
        let (user, cookie) = (self.user.clone(), self.cookie.clone());
        let this = Rc::downgrade(self);
        crate::wake::off_thread(
            move || authenticate(&user, &cookie, &password),
            move |result| {
                let Some(this) = this.upgrade() else {
                    return;
                };
                match result {
                    Ok(()) => this.finish(true),
                    Err(err) => {
                        this.error.set_text(&err);
                        this.error.set_visible(true);
                        this.entry.set_text("");
                        this.entry.set_sensitive(true);
                        this.ok.set_sensitive(true);
                        this.entry.grab_focus();
                    }
                }
            },
        );
    }

    /// reply to polkit once
    fn answer(&self, ok: bool) {
        if let Some(invocation) = self.invocation.borrow_mut().take() {
            if ok {
                invocation.return_value(None);
            } else {
                invocation.return_dbus_error("org.freedesktop.PolicyKit1.Error.Cancelled", "the password prompt was cancelled");
            }
        }
    }

    fn finish(&self, ok: bool) {
        self.answer(ok);
        OPEN.with(|o| o.borrow_mut().retain(|p| p.cookie != self.cookie));
        self.window.close();
    }
}

/// the helper polkit before 126 ships setuid instead of the socket
const HELPER_BINARIES: [&str; 2] = ["/usr/lib/polkit-1/polkit-agent-helper-1", "/usr/libexec/polkit-agent-helper-1"];

/// talk to polkit-agent-helper-1 thru its socket or run it on older polkit and answer its password prompt
fn authenticate(user: &str, cookie: &str, password: &str) -> Result<(), String> {
    match UnixStream::connect(HELPER_SOCKET) {
        Ok(stream) => {
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));
            let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
            writer
                .write_all(format!("{user}\n{cookie}\n").as_bytes())
                .map_err(|e| e.to_string())?;
            converse(BufReader::new(stream), writer, password)
        }
        Err(socket_err) => {
            let Some(helper) = HELPER_BINARIES.iter().find(|p| std::path::Path::new(p).exists()) else {
                return Err(format!("Can't reach polkit's helper: {socket_err}"));
            };
            let mut child = std::process::Command::new(helper)
                .arg(user)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| format!("Can't run polkit's helper: {e}"))?;
            let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
                let _ = child.kill();
                let _ = child.wait();
                return Err("polkit's helper didn't start right".into());
            };
            let result = stdin
                .write_all(format!("{cookie}\n").as_bytes())
                .map_err(|e| e.to_string())
                .and_then(|()| converse(BufReader::new(stdout), stdin, password));
            let _ = child.kill();
            let _ = child.wait();
            result
        }
    }
}

/// answer the helpers first password prompt and read to success or failure
fn converse(mut reader: impl BufRead, mut writer: impl Write, password: &str) -> Result<(), String> {
    let mut problem = String::new();
    let mut answered = false;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            return Err("polkit's helper hung up".into());
        }
        let text = line.trim_end_matches('\n');
        if let Some(question) = text
            .strip_prefix("PAM_PROMPT_ECHO_OFF")
            .or_else(|| text.strip_prefix("PAM_PROMPT_ECHO_ON"))
        {
            // a second question like a one time code must not get the password so say we cant
            if answered {
                return Err(format!("Also asked \"{}\", which this prompt can't answer", question.trim()));
            }
            answered = true;
            writer
                .write_all(format!("{password}\n").as_bytes())
                .map_err(|e| e.to_string())?;
        } else if let Some(msg) = text.strip_prefix("PAM_ERROR_MSG ") {
            problem = msg.to_string();
        } else if text.starts_with("SUCCESS") {
            return Ok(());
        } else if text.starts_with("FAILURE") {
            return Err(if problem.is_empty() {
                "That password didn't work".into()
            } else {
                problem
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_first_prompt_gets_the_password() {
        let mut sent = Vec::new();
        let ok = converse("PAM_PROMPT_ECHO_OFF Password: \nSUCCESS\n".as_bytes(), &mut sent, "hunter2");
        assert_eq!(ok, Ok(()));
        assert_eq!(sent, b"hunter2\n");
        let mut sent = Vec::new();
        let two = converse(
            "PAM_PROMPT_ECHO_OFF Password: \nPAM_PROMPT_ECHO_ON Code: \n".as_bytes(),
            &mut sent,
            "hunter2",
        );
        assert!(two.is_err());
        assert_eq!(sent, b"hunter2\n");
    }
}
