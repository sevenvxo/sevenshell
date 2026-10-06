//! the settings app like caelestias nexus w a nav rail and pages of rounded cards that save as u change them

mod defaults;
mod pages;
mod preview;
mod shell_pages;
pub mod store;
pub(crate) mod system;
mod widgets;

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::glib;

pub use widgets::CSS;

use store::Store;
use widgets::Page;

/// what every page builder gets
pub struct Ctx {
    pub store: Rc<Store>,
    pub window: gtk4::ApplicationWindow,
    pub status: Rc<dyn Fn(&str, bool)>,
}

/// a page in the nav w its icon name and how to build it
struct Entry {
    id: &'static str,
    icon: &'static str,
    title: &'static str,
    group: &'static str,
    build: fn(&Rc<Ctx>) -> Rc<Page>,
}

const ENTRIES: &[Entry] = &[
    Entry { id: "style", icon: "palette", title: "Wallpaper & style", group: "Look", build: pages::style },
    Entry { id: "bar", icon: "toolbar", title: "Bar", group: "Look", build: shell_pages::bar },
    Entry { id: "animations", icon: "animation", title: "Animations", group: "Look", build: pages::animations },
    Entry { id: "general", icon: "tune", title: "General", group: "Desktop", build: pages::general },
    Entry { id: "tiling", icon: "dashboard", title: "Tiling & canvas", group: "Desktop", build: pages::tiling },
    Entry { id: "keybindings", icon: "keyboard_command_key", title: "Keybindings", group: "Desktop", build: pages::keybindings },
    Entry { id: "rules", icon: "rule", title: "Window rules", group: "Desktop", build: pages::rules },
    Entry { id: "monitors", icon: "desktop_windows", title: "Monitors", group: "Desktop", build: pages::monitors },
    Entry { id: "input", icon: "mouse", title: "Input", group: "Desktop", build: pages::input },
    Entry { id: "notifications", icon: "notifications", title: "Notifications", group: "Desktop", build: shell_pages::notifications },
    Entry { id: "lock", icon: "lock", title: "Lock & idle", group: "Desktop", build: shell_pages::lock },
    Entry { id: "extras", icon: "widgets", title: "Extras", group: "Desktop", build: shell_pages::extras },
    Entry { id: "network", icon: "wifi", title: "Network", group: "System", build: system::network },
    Entry { id: "bluetooth", icon: "bluetooth", title: "Bluetooth", group: "System", build: system::bluetooth },
    Entry { id: "sound", icon: "volume_up", title: "Sound", group: "System", build: system::sound },
    Entry { id: "power", icon: "power_settings_new", title: "Power", group: "System", build: system::power },
    Entry { id: "apps", icon: "apps", title: "Default apps", group: "System", build: defaults::page },
    Entry { id: "drives", icon: "usb", title: "Drives", group: "System", build: defaults::drives },
];

struct Settings {
    window: gtk4::ApplicationWindow,
    stack: gtk4::Stack,
    ctx: Rc<Ctx>,
    nav: RefCell<Vec<(&'static str, gtk4::Button)>>,
    /// pages already built by id
    built: RefCell<Vec<(&'static str, Rc<Page>)>>,
}

thread_local! {
    static OPEN: RefCell<Option<Rc<Settings>>> = const { RefCell::new(None) };
}

/// sevenshell settings opens the window or brings it forward on a page like --page network
pub fn open(app: &gtk4::Application, page: Option<&str>) {
    let settings = OPEN.with(|o| o.borrow().clone()).unwrap_or_else(|| {
        let s = Settings::new(app);
        OPEN.with(|o| *o.borrow_mut() = Some(s.clone()));
        s
    });
    let id = page
        .and_then(|p| ENTRIES.iter().find(|e| e.id == p).map(|e| e.id))
        .unwrap_or_else(|| settings.stack.visible_child_name().map_or("style", |n| {
            ENTRIES.iter().find(|e| e.id == n.as_str()).map_or("style", |e| e.id)
        }));
    settings.show(id);
    settings.window.present();
}

impl Settings {
    fn new(app: &gtk4::Application) -> Rc<Self> {
        let window = gtk4::ApplicationWindow::new(app);
        window.set_title(Some("Settings"));
        window.set_default_size(1080, 760);
        window.add_css_class("settings");
        crate::style::adopt(&window);

        let status = gtk4::Label::new(None);
        status.add_css_class("status");
        status.set_xalign(0.0);
        status.set_wrap(true);
        let status_fn: Rc<dyn Fn(&str, bool)> = {
            let status = status.clone();
            Rc::new(move |text: &str, error: bool| {
                status.set_text(text);
                if error {
                    status.add_css_class("error");
                } else {
                    status.remove_css_class("error");
                }
            })
        };
        let store = Store::new();
        {
            let f = status_fn.clone();
            store.set_status(move |t, e| f(t, e));
        }
        let ctx = Rc::new(Ctx { store, window: window.clone(), status: status_fn });

        let stack = gtk4::Stack::new();
        stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
        stack.set_transition_duration(200);
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content.add_css_class("content");
        content.set_hexpand(true);
        content.set_overflow(gtk4::Overflow::Hidden);
        content.append(&stack);

        let nav = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        nav.add_css_class("nav");
        nav.set_size_request(280, -1);
        // the search box wants to grow and that shouldnt widen the rail
        nav.set_hexpand(false);
        let title = gtk4::Label::new(Some("Settings"));
        title.add_css_class("nav-title");
        title.set_xalign(0.0);
        nav.append(&title);
        let search_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
        search_box.add_css_class("search");
        search_box.append(&crate::style::icon("search"));
        let search = gtk4::Entry::new();
        search.set_placeholder_text(Some("Search settings"));
        search.set_hexpand(true);
        search.set_has_frame(false);
        search_box.append(&search);
        nav.append(&search_box);
        let list = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        let scroller = gtk4::ScrolledWindow::new();
        scroller.set_hscrollbar_policy(gtk4::PolicyType::Never);
        scroller.set_vexpand(true);
        scroller.set_child(Some(&list));
        nav.append(&scroller);
        nav.append(&status);

        let root = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        root.append(&nav);
        root.append(&content);
        window.set_child(Some(&root));

        let settings = Rc::new(Self {
            window: window.clone(),
            stack,
            ctx,
            nav: RefCell::new(Vec::new()),
            built: RefCell::new(Vec::new()),
        });

        let mut group = "";
        let mut headers = Vec::new();
        for entry in ENTRIES {
            if entry.group != group {
                group = entry.group;
                let h = gtk4::Label::new(Some(group));
                h.add_css_class("navgroup");
                h.set_xalign(0.0);
                list.append(&h);
                headers.push((group, h));
            }
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 14);
            row.append(&crate::style::icon(entry.icon));
            let label = gtk4::Label::new(Some(entry.title));
            label.set_xalign(0.0);
            row.append(&label);
            let button = gtk4::Button::new();
            button.set_child(Some(&row));
            button.add_css_class("navrow");
            let (s, id) = (Rc::downgrade(&settings), entry.id);
            button.connect_clicked(move |_| {
                if let Some(s) = s.upgrade() {
                    s.show(id);
                }
            });
            list.append(&button);
            settings.nav.borrow_mut().push((entry.id, button));
        }

        // typing filters the nav to pages whose settings mention it and enter opens the first
        {
            let s = Rc::downgrade(&settings);
            search.connect_changed(move |e| {
                let Some(s) = s.upgrade() else {
                    return;
                };
                let query = e.text().to_lowercase();
                s.build_all();
                let mut first = None;
                for (id, button) in s.nav.borrow().iter() {
                    let entry = ENTRIES.iter().find(|x| x.id == *id).expect("listed");
                    let words = s
                        .built
                        .borrow()
                        .iter()
                        .find(|(i, _)| i == id)
                        .map(|(_, p)| p.words.borrow().clone())
                        .unwrap_or_default();
                    let hit = query.is_empty()
                        || entry.title.to_lowercase().contains(&query)
                        || words.contains(&query);
                    button.set_visible(hit);
                    if hit && first.is_none() {
                        first = Some(*id);
                    }
                }
                for (group, header) in &headers {
                    let any = ENTRIES
                        .iter()
                        .filter(|x| x.group == *group)
                        .any(|x| s.nav.borrow().iter().any(|(i, b)| *i == x.id && b.is_visible()));
                    header.set_visible(any);
                }
                if !query.is_empty()
                    && let Some(id) = first
                {
                    s.show(id);
                }
            });
        }

        // closing forgets the window so the next open rebuilds it fresh from the files
        window.connect_close_request(|_| {
            OPEN.with(|o| *o.borrow_mut() = None);
            glib::Propagation::Proceed
        });
        settings
    }

    fn page(&self, id: &'static str) -> Rc<Page> {
        if let Some((_, page)) = self.built.borrow().iter().find(|(i, _)| *i == id) {
            return page.clone();
        }
        let entry = ENTRIES.iter().find(|e| e.id == id).expect("known page");
        let page = (entry.build)(&self.ctx);
        page.done();
        self.stack.add_named(&page.root, Some(id));
        self.built.borrow_mut().push((id, page.clone()));
        page
    }

    fn build_all(&self) {
        for entry in ENTRIES {
            self.page(entry.id);
        }
    }

    fn show(&self, id: &'static str) {
        self.page(id);
        self.stack.set_visible_child_name(id);
        for (i, button) in self.nav.borrow().iter() {
            if *i == id {
                button.add_css_class("selected");
            } else {
                button.remove_css_class("selected");
            }
        }
        system::shown(id);
    }
}
