//! the session screen on mod+shift+escape w big buttons to lock log out suspend restart or shut down

use gtk4::prelude::*;
use gtk4::{gdk, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

pub const CSS: &str = "
.session { background: alpha(@m3scrim, 0.55); }
.session * { color: @m3onSurface; }
.session .title { font-size: 28px; font-weight: 500; margin-bottom: 28px; }
.session button.choice {
    background: @m3surfaceContainerHigh;
    border: none; box-shadow: none;
    border-radius: 9999px;
    min-width: 96px; min-height: 96px;
    padding: 0;
    transition: background 150ms cubic-bezier(0.2, 0, 0, 1), border-radius 200ms cubic-bezier(0.2, 0, 0, 1);
}
.session button.choice .icon { font-size: 40px; color: @m3onSurfaceVariant; }
.session button.choice:hover, .session button.choice:focus {
    background: @m3primary;
    border-radius: 28px;
}
.session button.choice:hover .icon, .session button.choice:focus .icon { color: @m3onPrimary; }
.session .name { font-size: 14px; margin-top: 10px; }
.session .shortcut { font-size: 12px; color: @m3onSurfaceVariant; }
.session .hint { font-size: 12px; color: @m3onSurfaceVariant; margin-top: 28px; }
";

/// what each button does
#[derive(Clone, Copy)]
enum Choice {
    Lock,
    LogOut,
    Suspend,
    Restart,
    ShutDown,
}

const CHOICES: [(Choice, &str, &str, &str); 5] = [
    (Choice::Lock, "lock", "Lock", "L"),
    (Choice::LogOut, "logout", "Log out", "E"),
    (Choice::Suspend, "bedtime", "Suspend", "S"),
    (Choice::Restart, "restart_alt", "Restart", "R"),
    (Choice::ShutDown, "power_settings_new", "Shut down", "P"),
];

fn run(choice: Choice) {
    let sh = |cmd: &str| crate::run_detached(std::process::Command::new("sh").args(["-c", cmd]));
    match choice {
        Choice::Lock => sh("sevenshell lock"),
        Choice::LogOut => {
            if crate::ipc::request(serde_json::json!({ "action": "quit" })).is_err() {
                sh("loginctl terminate-session \"$XDG_SESSION_ID\"");
            }
        }
        Choice::Suspend => sh("systemctl suspend"),
        Choice::Restart => sh("systemctl reboot"),
        Choice::ShutDown => sh("systemctl poweroff"),
    }
}

thread_local! {
    static OPEN: std::cell::RefCell<Option<gtk4::ApplicationWindow>> = const { std::cell::RefCell::new(None) };
}

/// open the session screen or close it if its open
pub fn toggle(app: &gtk4::Application) {
    if let Some(open) = OPEN.with(|o| o.borrow_mut().take()) {
        open.close();
        return;
    }
    let window = gtk4::ApplicationWindow::new(app);
    window.init_layer_shell();
    window.set_namespace(Some("session"));
    window.set_layer(Layer::Overlay);
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }
    // over the bar and everything else
    window.set_exclusive_zone(-1);
    window.add_css_class("session");
    crate::style::adopt(&window);

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root.set_halign(gtk4::Align::Center);
    root.set_valign(gtk4::Align::Center);
    let user = std::env::var("USER").unwrap_or_default();
    let title = gtk4::Label::new(Some(&format!("Goodbye for now, {user}")));
    title.add_css_class("title");
    root.append(&title);
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 28);
    row.set_halign(gtk4::Align::Center);
    let mut first: Option<gtk4::Button> = None;
    for (choice, icon, name, key) in CHOICES {
        let col = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let button = gtk4::Button::new();
        button.add_css_class("choice");
        let glyph = crate::style::icon(icon);
        button.set_child(Some(&glyph));
        let w = window.clone();
        button.connect_clicked(move |_| {
            w.close();
            run(choice);
        });
        col.append(&button);
        let label = gtk4::Label::new(Some(name));
        label.add_css_class("name");
        col.append(&label);
        let shortcut = gtk4::Label::new(Some(key));
        shortcut.add_css_class("shortcut");
        col.append(&shortcut);
        row.append(&col);
        first.get_or_insert(button);
    }
    root.append(&row);
    let hint = gtk4::Label::new(Some("Arrows and Enter pick one · Esc or a click outside cancels"));
    hint.add_css_class("hint");
    root.append(&hint);
    window.set_child(Some(&root));

    let keys = gtk4::EventControllerKey::new();
    let w = window.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        let letter = key.to_unicode().map(|c| c.to_ascii_uppercase());
        if key == gdk::Key::Escape {
            w.close();
            return glib::Propagation::Stop;
        }
        if let Some(c) = letter
            && let Some((choice, ..)) = CHOICES.iter().find(|(_, _, _, k)| k.starts_with(c))
        {
            w.close();
            run(*choice);
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    // a click on the dim part closes it
    let click = gtk4::GestureClick::new();
    let w = window.clone();
    click.connect_released(move |_, _, x, y| {
        let picked = w.pick(x, y, gtk4::PickFlags::DEFAULT);
        let on_button = picked.is_some_and(|p| p.is::<gtk4::Button>() || p.ancestor(gtk4::Button::static_type()).is_some());
        if !on_button {
            w.close();
        }
    });
    window.add_controller(click);
    window.connect_close_request(|_| {
        OPEN.with(|o| o.borrow_mut().take());
        glib::Propagation::Proceed
    });
    window.present();
    if let Some(first) = first {
        first.grab_focus();
    }
    OPEN.with(|o| *o.borrow_mut() = Some(window));
}
