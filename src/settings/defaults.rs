//! the default apps page that picks ur browser and file manager thru mimeapps.list and the drives page

use std::rc::Rc;

use gtk4::gio;
use gtk4::prelude::*;

use super::store::Which;
use super::widgets::{self as w, Page};
use super::Ctx;

/// the browser gets links and web pages
const BROWSER_TYPES: [&str; 3] = ["x-scheme-handler/https", "x-scheme-handler/http", "text/html"];
/// the file manager gets folders
const FILES_TYPES: [&str; 1] = ["inode/directory"];

pub fn page(ctx: &Rc<Ctx>) -> Rc<Page> {
    let page = Page::new("apps", "Default apps");
    page.section("Apps");
    page.row(
        "Web browser",
        Some("opens links and mod+w"),
        &picker(ctx, &BROWSER_TYPES, "Web browser"),
    );
    page.row(
        "File manager",
        Some("opens folders and mod+e"),
        &picker(ctx, &FILES_TYPES, "File manager"),
    );
    page.note("Changes apply right away to every app that opens links or folders the standard way.");
    page
}

/// a dropdown of every app that handles the first type and picking one makes it the default for all of them
fn picker(ctx: &Rc<Ctx>, types: &'static [&'static str], what: &'static str) -> gtk4::DropDown {
    let mut apps: Vec<gio::AppInfo> = gio::AppInfo::all_for_type(types[0])
        .into_iter()
        .filter(|a| a.should_show() && a.id().is_some())
        .collect();
    apps.sort_by_key(|a| a.display_name().to_lowercase());
    apps.dedup_by(|a, b| a.id() == b.id());
    let current = gio::AppInfo::default_for_type(types[0], false).and_then(|a| a.id());
    // the current default still shows even if its hidden from menus
    if let Some(app) = gio::AppInfo::default_for_type(types[0], false) {
        if !apps.iter().any(|a| a.id() == app.id()) {
            apps.insert(0, app);
        }
    }
    let mut labels: Vec<String> = apps.iter().map(|a| a.display_name().to_string()).collect();
    if apps.is_empty() {
        labels.push("Nothing installed".into());
    }
    let list = gtk4::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    let w = gtk4::DropDown::new(Some(list), gtk4::Expression::NONE);
    w.set_sensitive(!apps.is_empty());
    if let Some(i) = apps.iter().position(|a| a.id() == current) {
        w.set_selected(i as u32);
    } else {
        // nothing picked yet so the first one isnt quietly pretending to be the default
        w.set_selected(gtk4::INVALID_LIST_POSITION);
    }
    let status = ctx.status.clone();
    w.connect_selected_notify(move |w| {
        let Some(app) = apps.get(w.selected() as usize) else {
            return;
        };
        let failed = types.iter().find_map(|t| app.set_as_default_for_type(t).err());
        match failed {
            Some(e) => status(&format!("Couldn't set the {}: {e}", what.to_lowercase()), true),
            None => status(&format!("{what} is now {}", app.display_name()), false),
        }
    });
    w
}

pub fn drives(ctx: &Rc<Ctx>) -> Rc<Page> {
    let page = Page::new("usb", "Drives");
    page.section("External drives");
    page.row(
        "Mount drives automatically",
        Some("USB sticks and external drives show up in your file manager as soon as they're plugged in"),
        &w::switch(&ctx.store, Which::Shell, "drives.automount"),
    );
    page.note("Uses udiskie. FAT, exFAT, NTFS and Linux drives can all be read and written. Eject a drive from your file manager before unplugging it.");
    page
}
