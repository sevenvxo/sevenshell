//! the system pages for network bluetooth sound and power that act right away thru nmcli bluetoothctl pactl and powerprofilesctl

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use gtk4::prelude::*;

use super::widgets::Page;
use super::Ctx;

/// a finished command w its output
#[derive(Clone, Default)]
pub(crate) struct Out {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// run a command w a timeout and maybe some input on stdin
pub(crate) fn run_with(cmd: &[&str], timeout: u64, input: Option<&str>) -> Out {
    let Some((program, args)) = cmd.split_first() else {
        return Out::default();
    };
    let child = Command::new(program)
        .args(args)
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut child) = child else {
        return Out { ok: false, stdout: String::new(), stderr: format!("{program} isnt installed") };
    };
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        let _ = stdin.write_all(text.as_bytes());
    }
    // read both pipes while it runs so a chatty command cant fill one and hang
    let drain = |pipe: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut buf);
            }
            String::from_utf8_lossy(&buf).into_owned()
        })
    };
    let stdout = drain(child.stdout.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>));
    let stderr = drain(child.stderr.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>));
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > Duration::from_secs(timeout) => {
                let _ = child.kill();
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => break,
        }
    }
    let status = child.wait();
    let (stdout, stderr) = (stdout.join().unwrap_or_default(), stderr.join().unwrap_or_default());
    match status {
        Ok(status) => Out { ok: status.success(), stdout, stderr },
        Err(e) => Out { ok: false, stdout, stderr: e.to_string() },
    }
}

fn run(cmd: &[&str]) -> Out {
    run_with(cmd, 8, None)
}

/// split one line of nmcli -t output on unescaped colons
pub(crate) fn terse_split(line: &str) -> Vec<String> {
    let (mut fields, mut current, mut escaped) = (Vec::new(), String::new(), false);
    for ch in line.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == ':' {
            fields.push(std::mem::take(&mut current));
        } else {
            current.push(ch);
        }
    }
    fields.push(current);
    fields
}

/// change a connection thru nmclis editor so passwords never show up on a command line
fn nm_edit(target: &[&str], commands: &[String]) -> Out {
    // a do u still want to save question can follow so yes answers it
    let mut lines: Vec<String> = commands.iter().map(|c| c.replace('\n', " ")).collect();
    lines.extend(["save persistent".into(), "yes".into(), "quit".into()]);
    let mut cmd = vec!["nmcli", "connection", "edit"];
    cmd.extend_from_slice(target);
    let out = run_with(&cmd, 20, Some(&(lines.join("\n") + "\n")));
    let error = out.stdout.lines().chain(out.stderr.lines()).filter(|l| l.contains("Error")).last().map(String::from);
    match error {
        Some(e) => Out { ok: false, stdout: String::new(), stderr: e },
        None => out,
    }
}

/// one system page w the box it refills
struct SystemPage {
    body: gtk4::Box,
    ctx: Weak<Ctx>,
    /// bumped per refresh so an older one finishing late loses
    generation: std::cell::Cell<u64>,
}

thread_local! {
    static PAGES: RefCell<HashMap<&'static str, Rc<SystemPage>>> = RefCell::new(HashMap::new());
}

fn make(ctx: &Rc<Ctx>, id: &'static str, icon: &str, title: &str) -> Rc<Page> {
    let page = Page::new(icon, title);
    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    page.append(&body);
    let loading = gtk4::Label::new(Some("Loading"));
    loading.add_css_class("list-empty");
    body.append(&loading);
    PAGES.with(|p| {
        p.borrow_mut().insert(id, Rc::new(SystemPage { body, ctx: Rc::downgrade(ctx), generation: Default::default() }))
    });
    page
}

pub fn network(ctx: &Rc<Ctx>) -> Rc<Page> {
    make(ctx, "network", "wifi", "Network")
}
pub fn bluetooth(ctx: &Rc<Ctx>) -> Rc<Page> {
    make(ctx, "bluetooth", "bluetooth", "Bluetooth")
}
pub fn sound(ctx: &Rc<Ctx>) -> Rc<Page> {
    make(ctx, "sound", "volume_up", "Sound")
}
pub fn power(ctx: &Rc<Ctx>) -> Rc<Page> {
    make(ctx, "power", "power_settings_new", "Power")
}

/// a page was opened so refresh it if its a system one
pub fn shown(id: &str) {
    refresh(id);
}

/// run the tools off the ui thread then rebuild the page from what they said
fn refresh(id: &str) {
    let Some(page) = PAGES.with(|p| p.borrow().get(id).cloned()) else {
        return;
    };
    let generation = page.generation.get() + 1;
    page.generation.set(generation);
    let id = id.to_string();
    let id_owned = id.clone();
    crate::wake::off_thread(move || gather(&id_owned), move |data| {
        // a newer refresh of this page started so let it uhh win
        if page.generation.get() == generation
            && let Some(ctx) = page.ctx.upgrade()
        {
            while let Some(c) = page.body.first_child() {
                page.body.remove(&c);
            }
            let ui = Rc::new(Ui { body: page.body.clone(), ctx, id: id.clone(), group: RefCell::new(None) });
            match id.as_str() {
                "network" => fill_network(&ui, &data),
                "bluetooth" => fill_bluetooth(&ui, &data),
                "sound" => fill_sound(&ui, &data),
                _ => fill_power(&ui, &data),
            }
            ui.close();
        }
    });
}

/// every command a page needs and what it said
type Data = HashMap<String, Out>;

fn gather(id: &str) -> Data {
    let mut d = Data::new();
    let mut get = |cmd: &[&str], t: u64| -> Out {
        let out = run_with(cmd, t, None);
        d.insert(cmd.join(" "), out.clone());
        out
    };
    match id {
        "network" => {
            if get(&["nmcli", "-v"], 8).ok {
                get(&["nmcli", "radio", "wifi"], 8);
                let devs = get(&["nmcli", "-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device"], 8);
                for line in devs.stdout.lines() {
                    let dev = terse_split(line)[0].clone();
                    get(&["nmcli", "-t", "-f", "GENERAL.HWADDR,IP4.ADDRESS,IP4.GATEWAY,IP4.DNS", "device", "show", &dev], 8);
                }
                get(&["nmcli", "-t", "-f", "UUID", "connection", "show", "--active"], 8);
                get(&["nmcli", "-t", "-f", "NAME,UUID,TYPE,AUTOCONNECT", "connection", "show"], 8);
            }
        }
        "bluetooth" => {
            if get(&["bluetoothctl", "show"], 4).stdout.contains("Controller") {
                let paired = get(&["bluetoothctl", "devices", "Paired"], 4);
                for line in paired.stdout.lines() {
                    let parts: Vec<&str> = line.splitn(3, ' ').collect();
                    if parts.len() == 3 && parts[0] == "Device" {
                        get(&["bluetoothctl", "info", parts[1]], 4);
                    }
                }
            }
        }
        "sound" => {
            for what in ["sinks", "sources", "sink-inputs", "cards"] {
                get(&["pactl", "-f", "json", "list", what], 8);
            }
            get(&["pactl", "get-default-sink"], 8);
            get(&["pactl", "get-default-source"], 8);
        }
        _ => {
            get(&["powerprofilesctl", "get"], 8);
            get(&["powerprofilesctl", "list"], 8);
        }
    }
    d
}

fn said(d: &Data, cmd: &[&str]) -> String {
    d.get(&cmd.join(" ")).map(|o| o.stdout.clone()).unwrap_or_default()
}

/// builds a system pages cards
struct Ui {
    body: gtk4::Box,
    ctx: Rc<Ctx>,
    id: String,
    group: RefCell<Option<(gtk4::Box, Vec<gtk4::Box>)>>,
}

impl Ui {
    fn close(&self) {
        if let Some((_, cards)) = self.group.borrow_mut().take() {
            for (i, c) in cards.iter().enumerate() {
                if i == 0 {
                    c.add_css_class("first");
                }
                if i + 1 == cards.len() {
                    c.add_css_class("last");
                }
            }
        }
    }

    fn section(&self, title: &str, extra: Option<&gtk4::Widget>) {
        self.close();
        let head = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let label = gtk4::Label::new(Some(title));
        label.add_css_class("section-title");
        if self.body.first_child().is_none() {
            label.add_css_class("first");
        }
        label.set_xalign(0.0);
        label.set_hexpand(true);
        head.append(&label);
        if let Some(w) = extra {
            w.set_valign(gtk4::Align::End);
            w.set_margin_bottom(6);
            head.append(w);
        }
        self.body.append(&head);
        let group = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        self.body.append(&group);
        *self.group.borrow_mut() = Some((group, Vec::new()));
    }

    fn card(&self, title: &str, subtitle: Option<&str>, widgets: &[gtk4::Widget]) -> gtk4::Box {
        let card = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
        card.add_css_class("card");
        let text = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        text.set_hexpand(true);
        text.set_valign(gtk4::Align::Center);
        let t = gtk4::Label::new(Some(title));
        t.add_css_class("row-title");
        t.set_xalign(0.0);
        t.set_wrap(true);
        text.append(&t);
        if let Some(sub) = subtitle.filter(|s| !s.is_empty()) {
            let s = gtk4::Label::new(Some(sub));
            s.add_css_class("row-hint");
            s.set_xalign(0.0);
            s.set_wrap(true);
            text.append(&s);
        }
        card.append(&text);
        for w in widgets {
            w.set_valign(gtk4::Align::Center);
            card.append(w);
        }
        if self.group.borrow().is_none() {
            let group = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
            self.body.append(&group);
            *self.group.borrow_mut() = Some((group, Vec::new()));
        }
        if let Some((group, cards)) = self.group.borrow_mut().as_mut() {
            group.append(&card);
            cards.push(card.clone());
        }
        card
    }

    fn text(&self, text: &str) {
        self.close();
        let l = gtk4::Label::new(Some(text));
        l.add_css_class("note");
        l.set_xalign(0.0);
        l.set_wrap(true);
        self.body.append(&l);
    }

    /// run a command off the ui thread then say if it failed and refresh the page
    fn act(&self, cmd: Vec<String>, done: Option<String>, refresh_after: bool) {
        self.act_with(cmd, done, refresh_after, None);
    }

    fn act_with(&self, cmd: Vec<String>, done: Option<String>, refresh_after: bool, work: Option<Box<dyn FnOnce() -> Out + Send>>) {
        let status = self.ctx.status.clone();
        let id = self.id.clone();
        let label = cmd.iter().take(3).cloned().collect::<Vec<_>>().join(" ");
        let work = move || {
            let out = match work {
                Some(w) => w(),
                None => run_with(&cmd.iter().map(String::as_str).collect::<Vec<_>>(), 40, None),
            };
            out
        };
        crate::wake::off_thread(work, move |out| {
            if !out.ok {
                let msg = format!("{}{}", out.stderr, out.stdout);
                status(&format!("{label} {}", msg.trim().lines().last().unwrap_or("failed")), true);
            } else if let Some(done) = &done {
                status(done, false);
            }
            if refresh_after {
                refresh(&id);
            }
        });
    }

    fn button(&self, label: &str, class: &str, f: impl Fn() + 'static) -> gtk4::Widget {
        super::widgets::button(label, class, f).upcast()
    }

    /// ask before doing something u cant take back
    fn confirm(&self, question: &str, detail: &str, yes: impl Fn() + 'static) {
        let dialog = gtk4::AlertDialog::builder()
            .message(question)
            .detail(detail)
            .buttons(["Cancel", "OK"])
            .cancel_button(0)
            .default_button(1)
            .modal(true)
            .build();
        dialog.choose(Some(&self.ctx.window), gtk4::gio::Cancellable::NONE, move |r| {
            if r == Ok(1) {
                yes();
            }
        });
    }
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn fill_network(ui: &Rc<Ui>, d: &Data) {
    if !d.get("nmcli -v").is_some_and(|o| o.ok) {
        ui.text("NetworkManager isnt available");
        return;
    }
    let radio = said(d, &["nmcli", "radio", "wifi"]).trim() == "enabled";
    let wifi = gtk4::Switch::new();
    wifi.set_active(radio);
    {
        let ui = ui.clone();
        wifi.connect_active_notify(move |s| {
            ui.act(strings(&["nmcli", "radio", "wifi", if s.is_active() { "on" } else { "off" }]), None, true)
        });
    }
    ui.section("Wi-Fi", Some(wifi.upcast_ref()));
    ui.card("Wi-Fi radio", Some(if radio { "on" } else { "off" }), &[]);

    ui.section("Devices", None);
    for line in said(d, &["nmcli", "-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device"]).lines() {
        let f = terse_split(line);
        let get = |i: usize| f.get(i).cloned().unwrap_or_default();
        let (dev, kind, state, conn) = (get(0), get(1), get(2), get(3));
        if ["loopback", "wifi-p2p", "bridge", "tun"].contains(&kind.as_str()) || dev.starts_with("veth") {
            continue;
        }
        let mut details: HashMap<String, Vec<String>> = HashMap::new();
        for field in said(d, &["nmcli", "-t", "-f", "GENERAL.HWADDR,IP4.ADDRESS,IP4.GATEWAY,IP4.DNS", "device", "show", &dev]).lines() {
            if let Some((k, v)) = field.split_once(':')
                && !v.is_empty()
            {
                details.entry(k.split('[').next().unwrap_or(k).to_string()).or_default().push(v.replace("\\:", ":"));
            }
        }
        let mut parts = vec![format!("{kind} · {state}{}", if conn.is_empty() { String::new() } else { format!(" · {conn}") })];
        for (key, name) in [("IP4.ADDRESS", "IP"), ("IP4.GATEWAY", "gateway"), ("IP4.DNS", "DNS"), ("GENERAL.HWADDR", "MAC")] {
            if let Some(v) = details.get(key) {
                parts.push(format!("{name} {}", v.join(" ")));
            }
        }
        let mut widgets = Vec::new();
        if state.starts_with("connected") {
            let (ui2, dev2) = (ui.clone(), dev.clone());
            widgets.push(ui.button("Disconnect", "", move || ui2.act(strings(&["nmcli", "device", "disconnect", &dev2]), None, true)));
        } else if state == "disconnected" {
            let (ui2, dev2) = (ui.clone(), dev.clone());
            widgets.push(ui.button("Connect", "tonal", move || ui2.act(strings(&["nmcli", "device", "connect", &dev2]), None, true)));
        }
        ui.card(&dev, Some(&parts.join("\n")), &widgets);
    }

    ui.section("Saved connections", None);
    let active = said(d, &["nmcli", "-t", "-f", "UUID", "connection", "show", "--active"]);
    for line in said(d, &["nmcli", "-t", "-f", "NAME,UUID,TYPE,AUTOCONNECT", "connection", "show"]).lines() {
        let f = terse_split(line);
        let get = |i: usize| f.get(i).cloned().unwrap_or_default();
        let (name, uuid, kind, auto) = (get(0), get(1), get(2), get(3));
        if ["loopback", "bridge", "tun"].contains(&kind.as_str()) {
            continue;
        }
        let is_active = active.split_whitespace().any(|u| u == uuid);
        let autoconnect = gtk4::Switch::new();
        autoconnect.set_active(auto == "yes");
        autoconnect.set_tooltip_text(Some("Connect automatically"));
        {
            let (ui, uuid) = (ui.clone(), uuid.clone());
            autoconnect.connect_active_notify(move |s| {
                ui.act(strings(&["nmcli", "connection", "modify", &uuid, "connection.autoconnect", if s.is_active() { "yes" } else { "no" }]), None, false)
            });
        }
        let toggle = if is_active {
            let (ui2, uuid2) = (ui.clone(), uuid.clone());
            ui.button("Disconnect", "", move || ui2.act(strings(&["nmcli", "connection", "down", &uuid2]), None, true))
        } else {
            let (ui2, uuid2) = (ui.clone(), uuid.clone());
            ui.button("Connect", "tonal", move || ui2.act(strings(&["nmcli", "connection", "up", &uuid2]), None, true))
        };
        let edit = {
            let (ui2, uuid2, name2, kind2) = (ui.clone(), uuid.clone(), name.clone(), kind.clone());
            ui.button("Edit", "", move || edit_connection(&ui2, &uuid2, &name2, &kind2))
        };
        let forget = {
            let (ui2, uuid2, name2) = (ui.clone(), uuid.clone(), name.clone());
            ui.button("Forget", "danger", move || {
                let (ui3, uuid3, name3) = (ui2.clone(), uuid2.clone(), name2.clone());
                ui2.confirm(&format!("Forget {name2}"), "Its saved password and settings get deleted", move || {
                    ui3.act(strings(&["nmcli", "connection", "delete", &uuid3]), Some(format!("Forgot {name3}")), true)
                });
            })
        };
        let kinds = [("802-11-wireless", "Wi-Fi"), ("802-3-ethernet", "Ethernet"), ("vpn", "VPN"), ("wireguard", "WireGuard"), ("bluetooth", "Bluetooth")];
        let k = kinds.iter().find(|(a, _)| *a == kind).map_or(kind.as_str(), |(_, b)| b);
        let subtitle = format!("{k}{}", if is_active { " · connected" } else { "" });
        ui.card(&name, Some(&subtitle), &[autoconnect.upcast(), toggle, edit, forget]);
    }

    ui.section("Hidden network", None);
    let ssid = gtk4::Entry::new();
    ssid.set_placeholder_text(Some("Network name"));
    let password = gtk4::PasswordEntry::new();
    password.set_show_peek_icon(true);
    password.add_css_class("pill-field");
    let connect = {
        let (ui2, ssid, password) = (ui.clone(), ssid.clone(), password.clone());
        ui.button("Connect", "tonal", move || {
            let (name, secret) = (ssid.text().trim().to_string(), password.text().to_string());
            if name.is_empty() {
                return;
            }
            let n2 = name.clone();
            let work: Box<dyn FnOnce() -> Out + Send> = Box::new(move || {
                let mut commands = vec![format!("set connection.id {n2}"), format!("set wifi.ssid {n2}"), "set wifi.hidden yes".into()];
                if !secret.is_empty() {
                    commands.push("set wifi-sec.key-mgmt wpa-psk".into());
                    commands.push(format!("set wifi-sec.psk {secret}"));
                }
                let made = nm_edit(&["type", "wifi"], &commands);
                if !made.ok {
                    return made;
                }
                run_with(&["nmcli", "connection", "up", "id", &n2], 40, None)
            });
            ui2.act_with(strings(&["nmcli", "connection", "add"]), Some(format!("Connected to {name}")), true, Some(work));
        })
    };
    ui.card("Join a hidden network", None, &[ssid.upcast(), password.upcast(), connect]);
}

/// the ip and dns settings of a saved connection in a small dialog
fn edit_connection(ui: &Rc<Ui>, uuid: &str, name: &str, kind: &str) {
    let fields = ["ipv4.method", "ipv4.addresses", "ipv4.gateway", "ipv4.dns", "ipv4.ignore-auto-dns"];
    let values = run(&["nmcli", "-g", &fields.join(","), "connection", "show", uuid]).stdout;
    let mut current: HashMap<&str, String> = HashMap::new();
    for (i, f) in fields.iter().enumerate() {
        current.insert(f, values.lines().nth(i).unwrap_or("").replace("\\:", ":"));
    }
    let dialog = gtk4::Window::new();
    dialog.set_title(Some(&format!("Edit {name}")));
    dialog.set_transient_for(Some(&ui.ctx.window));
    dialog.set_modal(true);
    dialog.add_css_class("settings");
    crate::style::adopt(&dialog);
    let grid = gtk4::Grid::new();
    grid.set_row_spacing(10);
    grid.set_column_spacing(16);
    grid.set_margin_top(20);
    grid.set_margin_bottom(20);
    grid.set_margin_start(20);
    grid.set_margin_end(20);
    let add = |row: i32, text: &str, w: &gtk4::Widget| {
        let l = gtk4::Label::new(Some(text));
        l.set_xalign(0.0);
        grid.attach(&l, 0, row, 1, 1);
        grid.attach(w, 1, row, 1, 1);
    };
    let method = gtk4::DropDown::from_strings(&["Automatic DHCP", "Manual"]);
    method.set_selected(if current["ipv4.method"] == "manual" { 1 } else { 0 });
    let entry = |text: &str, hint: &str| {
        let e = gtk4::Entry::new();
        e.set_text(text);
        e.set_placeholder_text(Some(hint));
        e
    };
    let address = entry(&current["ipv4.addresses"], "192.168.1.20/24");
    let gateway = entry(&current["ipv4.gateway"], "192.168.1.1");
    let dns = entry(&current["ipv4.dns"], "1.1.1.1,9.9.9.9 or empty for automatic");
    let only = gtk4::Switch::new();
    only.set_active(current["ipv4.ignore-auto-dns"] == "yes");
    only.set_halign(gtk4::Align::Start);
    add(0, "IPv4", method.upcast_ref());
    add(1, "Address", address.upcast_ref());
    add(2, "Gateway", gateway.upcast_ref());
    add(3, "DNS servers", dns.upcast_ref());
    add(4, "Only these DNS servers", only.upcast_ref());
    let password = (kind == "802-11-wireless").then(|| {
        let p = gtk4::PasswordEntry::new();
        p.set_show_peek_icon(true);
        p.add_css_class("pill-field");
        add(5, "Password", p.upcast_ref());
        p
    });
    let sync = {
        let (address, gateway) = (address.clone(), gateway.clone());
        move |m: &gtk4::DropDown| {
            address.set_sensitive(m.selected() == 1);
            gateway.set_sensitive(m.selected() == 1);
        }
    };
    sync(&method);
    method.connect_selected_notify(sync);
    let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    buttons.set_halign(gtk4::Align::End);
    let cancel = gtk4::Button::with_label("Cancel");
    cancel.add_css_class("pill");
    let apply = gtk4::Button::with_label("Apply");
    apply.add_css_class("pill");
    apply.add_css_class("filled");
    buttons.append(&cancel);
    buttons.append(&apply);
    grid.attach(&buttons, 0, 6, 2, 1);
    dialog.set_child(Some(&grid));
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| dialog.close());
    }
    {
        let (ui, uuid, name, dialog) = (ui.clone(), uuid.to_string(), name.to_string(), dialog.clone());
        apply.connect_clicked(move |_| {
            let manual = method.selected() == 1;
            let mut cmd = strings(&["nmcli", "connection", "modify", &uuid, "ipv4.method", if manual { "manual" } else { "auto" }]);
            if manual {
                cmd.extend(strings(&["ipv4.addresses", &address.text(), "ipv4.gateway", &gateway.text()]));
            } else {
                cmd.extend(strings(&["ipv4.addresses", "", "ipv4.gateway", ""]));
            }
            cmd.extend(strings(&["ipv4.dns", &dns.text().replace(' ', ""), "ipv4.ignore-auto-dns", if only.is_active() { "yes" } else { "no" }]));
            let secret = password.as_ref().map(|p| p.text().to_string()).unwrap_or_default();
            let uuid2 = uuid.clone();
            let work: Box<dyn FnOnce() -> Out + Send> = Box::new(move || {
                let out = run_with(&cmd.iter().map(String::as_str).collect::<Vec<_>>(), 20, None);
                if !out.ok {
                    return out;
                }
                if !secret.is_empty() {
                    // thru the editor so its never on a command line
                    let e = nm_edit(&[&uuid2], &[format!("set wifi-sec.psk {secret}")]);
                    if !e.ok {
                        return e;
                    }
                }
                // settings apply on the next connect so reconnect if its up rn
                let active = run(&["nmcli", "-t", "-f", "UUID", "connection", "show", "--active"]).stdout;
                if active.split_whitespace().any(|u| u == uuid2) {
                    return run_with(&["nmcli", "connection", "up", &uuid2], 40, None);
                }
                out
            });
            ui.act_with(strings(&["nmcli", "connection", "modify"]), Some(format!("Saved {name}")), true, Some(work));
            dialog.close();
        });
    }
    dialog.present();
}

pub(crate) fn bt_devices(filter: Option<&str>) -> Vec<(String, String)> {
    let mut cmd = vec!["bluetoothctl", "devices"];
    if let Some(f) = filter {
        cmd.push(f);
    }
    run_with(&cmd, 4, None)
        .stdout
        .lines()
        .filter_map(|l| {
            let p: Vec<&str> = l.splitn(3, ' ').collect();
            (p.len() == 3 && p[0] == "Device").then(|| (p[1].to_string(), p[2].to_string()))
        })
        .collect()
}

fn fill_bluetooth(ui: &Rc<Ui>, d: &Data) {
    let show = said(d, &["bluetoothctl", "show"]);
    if !show.contains("Controller") {
        ui.text("No bluetooth adapter found so is bluetooth.service running");
        return;
    }
    let info: HashMap<String, String> = show
        .lines()
        .skip(1)
        .filter_map(|l| l.trim().split_once(": ").map(|(a, b)| (a.to_string(), b.to_string())))
        .collect();
    let powered = info.get("Powered").is_some_and(|v| v == "yes");
    let power = gtk4::Switch::new();
    power.set_active(powered);
    {
        let ui = ui.clone();
        power.connect_active_notify(move |s| ui.act(strings(&["bluetoothctl", "power", if s.is_active() { "on" } else { "off" }]), None, true));
    }
    ui.section("Bluetooth", Some(power.upcast_ref()));
    let visible = gtk4::Switch::new();
    visible.set_active(info.get("Discoverable").is_some_and(|v| v == "yes"));
    {
        let ui = ui.clone();
        visible.connect_active_notify(move |s| ui.act(strings(&["bluetoothctl", "discoverable", if s.is_active() { "on" } else { "off" }]), None, false));
    }
    let alias = info.get("Alias").cloned().unwrap_or_else(|| "?".into());
    ui.card("Visible to other devices", Some(&format!("This computer shows up as {alias}")), &[visible.upcast()]);
    if !powered {
        return;
    }
    ui.section("My devices", None);
    let paired = said(d, &["bluetoothctl", "devices", "Paired"]);
    let mut any = false;
    for line in paired.lines() {
        let p: Vec<&str> = line.splitn(3, ' ').collect();
        if p.len() != 3 || p[0] != "Device" {
            continue;
        }
        any = true;
        let (mac, name) = (p[1].to_string(), p[2].to_string());
        let details: HashMap<String, String> = said(d, &["bluetoothctl", "info", &mac])
            .lines()
            .filter_map(|l| l.trim().split_once(": ").map(|(a, b)| (a.to_string(), b.to_string())))
            .collect();
        let connected = details.get("Connected").is_some_and(|v| v == "yes");
        let mut parts = vec![if connected { "connected".to_string() } else { "not connected".into() }];
        if let Some(b) = details.get("Battery Percentage").and_then(|b| b.split('(').nth(1)) {
            parts.push(format!("battery {}%", b.trim_end_matches(')')));
        }
        parts.push(mac.clone());
        let trust = gtk4::Switch::new();
        trust.set_active(details.get("Trusted").is_some_and(|v| v == "yes"));
        trust.set_tooltip_text(Some("Trusted ones may connect without asking"));
        {
            let (ui, mac) = (ui.clone(), mac.clone());
            trust.connect_active_notify(move |s| ui.act(strings(&["bluetoothctl", if s.is_active() { "trust" } else { "untrust" }, &mac]), None, false));
        }
        let toggle = if connected {
            let (ui2, mac2) = (ui.clone(), mac.clone());
            ui.button("Disconnect", "", move || ui2.act(strings(&["bluetoothctl", "disconnect", &mac2]), None, true))
        } else {
            let (ui2, mac2) = (ui.clone(), mac.clone());
            ui.button("Connect", "tonal", move || ui2.act(strings(&["bluetoothctl", "connect", &mac2]), None, true))
        };
        let remove = {
            let (ui2, mac2, name2) = (ui.clone(), mac.clone(), name.clone());
            ui.button("Remove", "danger", move || {
                let (ui3, mac3, name3) = (ui2.clone(), mac2.clone(), name2.clone());
                ui2.confirm(&format!("Remove {name2}"), "Ull need to pair it again to use it", move || {
                    ui3.act(strings(&["bluetoothctl", "remove", &mac3]), Some(format!("Removed {name3}")), true)
                });
            })
        };
        ui.card(&name, Some(&parts.join(" · ")), &[trust.upcast(), toggle, remove]);
    }
    if !any {
        ui.card("No paired devices yet", None, &[]);
    }
    // searching fills a list under here
    let found = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    let search = {
        let (ui2, found) = (ui.clone(), found.clone());
        ui.button("Search", "tonal", move || bt_scan(&ui2, &found))
    };
    ui.section("Other devices", Some(&search));
    ui.close();
    ui.body.append(&found);
    note_in(&found, "Put the device in pairing mode then search");
}

fn note_in(b: &gtk4::Box, text: &str) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    card.add_css_class("card");
    card.add_css_class("first");
    card.add_css_class("last");
    let l = gtk4::Label::new(Some(text));
    l.add_css_class("list-empty");
    l.set_xalign(0.0);
    card.append(&l);
    b.append(&card);
}

fn bt_scan(ui: &Rc<Ui>, found: &gtk4::Box) {
    note_in(found, "Searching");
    let work = move || {
        run_with(&["bluetoothctl", "--timeout", "10", "scan", "on"], 15, None);
        let paired: Vec<String> = bt_devices(Some("Paired")).into_iter().map(|(m, _)| m).collect();
        let others: Vec<(String, String)> = bt_devices(None).into_iter().filter(|(m, _)| !paired.contains(m)).collect();
        others
    };
    let (ui, found) = (ui.clone(), found.clone());
    crate::wake::off_thread(work, move |others| {
        if found.parent().is_none() {
            return;
        }
        while let Some(c) = found.first_child() {
            found.remove(&c);
        }
        let n = others.len();
        for (i, (mac, name)) in others.iter().enumerate() {
            let card = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
            card.add_css_class("card");
            if i == 0 {
                card.add_css_class("first");
            }
            if i + 1 == n {
                card.add_css_class("last");
            }
            let l = gtk4::Label::new(Some(name));
            l.set_xalign(0.0);
            l.set_hexpand(true);
            l.add_css_class("row-title");
            card.append(&l);
            let (ui2, mac2, name2) = (ui.clone(), mac.clone(), name.clone());
            card.append(&super::widgets::button("Pair", "tonal", move || {
                (ui2.ctx.status)(&format!("Pairing w {name2}"), false);
                let mac3 = mac2.clone();
                let work: Box<dyn FnOnce() -> Out + Send> = Box::new(move || {
                    let mut last = Out::default();
                    for c in [["pair", &mac3], ["trust", &mac3], ["connect", &mac3]] {
                        last = run_with(&["bluetoothctl", "--timeout", "20", c[0], c[1]], 25, None);
                        if !last.ok || last.stdout.contains("Failed") {
                            return Out { ok: false, ..last };
                        }
                    }
                    last
                });
                ui2.act_with(strings(&["bluetoothctl", "pair"]), Some(format!("Paired w {name2}")), true, Some(work));
            }));
            found.append(&card);
        }
        if others.is_empty() {
            note_in(&found, "Nothing found so is the device in pairing mode");
        }
    });
}

fn pactl_json(d: &Data, what: &str) -> Vec<serde_json::Value> {
    serde_json::from_str(&said(d, &["pactl", "-f", "json", "list", what])).unwrap_or_default()
}

/// a volume slider and mute button for a sink source or stream
fn volume_widgets(kind: &str, target: &str, info: &serde_json::Value) -> Vec<gtk4::Widget> {
    let volume = info["volume"]
        .as_object()
        .map(|o| o.values().filter_map(|c| c["value_percent"].as_str()?.trim_end_matches('%').parse::<f64>().ok()).fold(0.0, f64::max))
        .unwrap_or(0.0);
    let scale = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, 150.0, 1.0);
    scale.set_value(volume);
    scale.set_size_request(180, -1);
    scale.add_mark(100.0, gtk4::PositionType::Bottom, None);
    {
        let (kind, target) = (kind.to_string(), target.to_string());
        scale.connect_value_changed(move |s| {
            crate::run_detached(Command::new("pactl").args([format!("set-{kind}-volume"), target.clone(), format!("{}%", s.value() as i64)]));
        });
    }
    let mute = gtk4::ToggleButton::new();
    mute.set_child(Some(&crate::style::icon("volume_off")));
    mute.add_css_class("pill");
    mute.add_css_class("icon-button");
    mute.set_tooltip_text(Some("Mute"));
    mute.set_active(info["mute"].as_bool().unwrap_or(false));
    {
        let (kind, target) = (kind.to_string(), target.to_string());
        mute.connect_toggled(move |b| {
            crate::run_detached(Command::new("pactl").args([format!("set-{kind}-mute"), target.clone(), if b.is_active() { "1".into() } else { "0".into() }]));
        });
    }
    vec![scale.upcast(), mute.upcast()]
}

fn fill_sound(ui: &Rc<Ui>, d: &Data) {
    let sinks = pactl_json(d, "sinks");
    let sources: Vec<_> = pactl_json(d, "sources").into_iter().filter(|s| !s["name"].as_str().unwrap_or("").ends_with(".monitor")).collect();
    if sinks.is_empty() && sources.is_empty() {
        ui.text("No sound server found so is pipewire-pulse running");
        return;
    }
    let pavucontrol = ui.button("Open pavucontrol", "", || {
        crate::run_detached(&mut Command::new("pavucontrol"));
    });
    for (title, devices, default, kind) in [
        ("Output", &sinks, said(d, &["pactl", "get-default-sink"]), "sink"),
        ("Input", &sources, said(d, &["pactl", "get-default-source"]), "source"),
    ] {
        ui.section(title, if kind == "sink" { Some(&pavucontrol) } else { None });
        let mut group: Option<gtk4::CheckButton> = None;
        for dev in devices.iter() {
            let name = dev["name"].as_str().unwrap_or("").to_string();
            let radio = gtk4::CheckButton::new();
            if let Some(g) = &group {
                radio.set_group(Some(g));
            } else {
                group = Some(radio.clone());
            }
            radio.set_active(name == default.trim());
            radio.set_tooltip_text(Some("Use this one"));
            {
                let (ui, name) = (ui.clone(), name.clone());
                radio.connect_toggled(move |r| {
                    if r.is_active() {
                        ui.act(strings(&["pactl", &format!("set-default-{kind}"), &name]), None, false);
                    }
                });
            }
            let mut widgets = vec![radio.upcast::<gtk4::Widget>()];
            widgets.extend(volume_widgets(kind, &name, dev));
            let ports: Vec<_> = dev["ports"].as_array().cloned().unwrap_or_default().into_iter().filter(|p| p["availability"].as_str() != Some("not available")).collect();
            if ports.len() > 1 {
                let labels: Vec<String> = ports.iter().map(|p| p["description"].as_str().unwrap_or("").to_string()).collect();
                let names: Vec<String> = ports.iter().map(|p| p["name"].as_str().unwrap_or("").to_string()).collect();
                let drop = gtk4::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
                if let Some(i) = names.iter().position(|n| Some(n.as_str()) == dev["active_port"].as_str()) {
                    drop.set_selected(i as u32);
                }
                let (ui, dname) = (ui.clone(), name.clone());
                drop.connect_selected_notify(move |dd| {
                    if let Some(port) = names.get(dd.selected() as usize) {
                        ui.act(strings(&["pactl", &format!("set-{kind}-port"), &dname, port]), None, false);
                    }
                });
                widgets.push(drop.upcast());
            }
            ui.card(dev["description"].as_str().unwrap_or(&name), None, &widgets);
        }
    }
    ui.section("Applications", None);
    let streams = pactl_json(d, "sink-inputs");
    for stream in &streams {
        let props = &stream["properties"];
        let app = props["application.name"].as_str().or(props["application.process.binary"].as_str()).unwrap_or("?");
        let media = props["media.name"].as_str().unwrap_or("");
        let index = stream["index"].as_i64().unwrap_or(0).to_string();
        let mut widgets = volume_widgets("sink-input", &index, stream);
        let labels: Vec<String> = sinks.iter().map(|s| s["description"].as_str().unwrap_or("").to_string()).collect();
        let ids: Vec<String> = sinks.iter().map(|s| s["index"].as_i64().unwrap_or(0).to_string()).collect();
        let drop = gtk4::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
        if let Some(i) = ids.iter().position(|x| *x == stream["sink"].as_i64().unwrap_or(-1).to_string()) {
            drop.set_selected(i as u32);
        }
        drop.set_tooltip_text(Some("Play thru"));
        {
            let ui = ui.clone();
            drop.connect_selected_notify(move |dd| {
                if let Some(sink) = ids.get(dd.selected() as usize) {
                    ui.act(strings(&["pactl", "move-sink-input", &index, sink]), None, false);
                }
            });
        }
        widgets.push(drop.upcast());
        ui.card(app, if media != app { Some(media) } else { None }, &widgets);
    }
    if streams.is_empty() {
        ui.card("Nothing is playing", None, &[]);
    }
    ui.section("Device profiles", None);
    for card in pactl_json(d, "cards") {
        let profiles = card["profiles"].as_object().cloned().unwrap_or_default();
        let active = card["active_profile"].as_str().unwrap_or("").to_string();
        let keys: Vec<String> = profiles.iter().filter(|(k, p)| p["available"].as_bool().unwrap_or(true) || **k == active).map(|(k, _)| k.clone()).collect();
        let labels: Vec<String> = keys.iter().map(|k| profiles[k]["description"].as_str().unwrap_or(k).to_string()).collect();
        let drop = gtk4::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
        if let Some(i) = keys.iter().position(|k| *k == active) {
            drop.set_selected(i as u32);
        }
        let cname = card["name"].as_str().unwrap_or("").to_string();
        {
            let (ui, cname) = (ui.clone(), cname.clone());
            drop.connect_selected_notify(move |dd| {
                if let Some(k) = keys.get(dd.selected() as usize) {
                    ui.act(strings(&["pactl", "set-card-profile", &cname, k]), None, true);
                }
            });
        }
        let name = card["properties"]["device.description"].as_str().unwrap_or(&cname).to_string();
        ui.card(&name, None, &[drop.upcast()]);
    }
}

fn fill_power(ui: &Rc<Ui>, d: &Data) {
    let current = said(d, &["powerprofilesctl", "get"]).trim().to_string();
    let list = said(d, &["powerprofilesctl", "list"]);
    if !current.is_empty() {
        ui.section("Power mode", None);
        let seg = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        seg.add_css_class("segmented");
        let buttons: Rc<RefCell<Vec<(String, gtk4::Button)>>> = Rc::default();
        for (key, text) in [("power-saver", "Power saver"), ("balanced", "Balanced"), ("performance", "Performance")] {
            if !list.contains(key) {
                continue;
            }
            let b = gtk4::Button::with_label(text);
            b.add_css_class("pill");
            if key == current {
                b.add_css_class("active");
            }
            {
                let (ui, buttons, key) = (ui.clone(), buttons.clone(), key.to_string());
                b.connect_clicked(move |_| {
                    for (k, b) in buttons.borrow().iter() {
                        if *k == key {
                            b.add_css_class("active");
                        } else {
                            b.remove_css_class("active");
                        }
                    }
                    ui.act(strings(&["powerprofilesctl", "set", &key]), None, false);
                });
            }
            seg.append(&b);
            buttons.borrow_mut().push((key.to_string(), b));
        }
        ui.card("Mode", Some("Saver lasts longer and performance goes faster"), &[seg.upcast()]);
    }
    ui.section("Idle and lock", None);
    ui.card("Screen off lock and sleep timeouts", Some("They live on the lock and idle page"), &[]);
    ui.section("Session", None);
    let lock = ui.button("Lock", "tonal", || {
        crate::run_detached(Command::new("sevenshell").arg("lock"));
    });
    let suspend = ui.button("Suspend", "", || {
        crate::run_detached(Command::new("systemctl").arg("suspend"));
    });
    let signout = {
        let ui2 = ui.clone();
        ui.button("Sign out", "danger", move || {
            ui2.confirm("Sign out of sevenwm", "Unsaved work in open windows is lost", || crate::ipc::action("quit"));
        })
    };
    let restart = {
        let ui2 = ui.clone();
        ui.button("Restart", "danger", move || {
            ui2.confirm("Restart the computer", "", || {
                crate::run_detached(Command::new("systemctl").arg("reboot"));
            });
        })
    };
    let shutdown = {
        let ui2 = ui.clone();
        ui.button("Shut down", "danger", move || {
            ui2.confirm("Shut down the computer", "", || {
                crate::run_detached(Command::new("systemctl").arg("poweroff"));
            });
        })
    };
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    for b in [lock, suspend, signout, restart, shutdown] {
        row.append(&b);
    }
    ui.card("", None, &[row.upcast()]);
}
