//! the screenshot picker like caelestias where the screen freezes and dims around the selection which snaps to the window under the cursor

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk4::gdk::prelude::GdkCairoContextExt;
use gtk4::prelude::*;
use gtk4::{gdk, gdk_pixbuf, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::ipc;
use crate::theme;

/// how long the selection takes to glide to a new window
const GLIDE: Duration = Duration::from_millis(400);
/// how long opening and closing fade
const FADE: Duration = Duration::from_millis(200);
/// how far a press can move before its a drag and not a click
const DRAG_SLOP: f64 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    /// corner radius so a window selection matches its rounded corners
    r: f64,
}

impl Rect {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    fn lerp(a: Rect, b: Rect, t: f64) -> Rect {
        let m = |p: f64, q: f64| p + (q - p) * t;
        Rect { x: m(a.x, b.x), y: m(a.y, b.y), w: m(a.w, b.w), h: m(a.h, b.h), r: m(a.r, b.r) }
    }

    /// the rect between two points in any direction
    fn between(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
        Rect { x: x0.min(x1), y: y0.min(y1), w: (x1 - x0).abs(), h: (y1 - y0).abs(), r: 0.0 }
    }
}

/// material 3 emphasized decelerate so moves start quick and settle soft
fn ease(t: f64) -> f64 {
    bezier(0.05, 0.7, 0.1, 1.0, t.clamp(0.0, 1.0))
}

/// a css style cubic bezier solved for x by newton steps
fn bezier(x1: f64, y1: f64, x2: f64, y2: f64, x: f64) -> f64 {
    let curve = |a: f64, b: f64, t: f64| 3.0 * a * t * (1.0 - t).powi(2) + 3.0 * b * t * t * (1.0 - t) + t.powi(3);
    let slope = |a: f64, b: f64, t: f64| 3.0 * a * (1.0 - t).powi(2) + 6.0 * (b - a) * t * (1.0 - t) + 3.0 * (1.0 - b) * t * t;
    let mut t = x;
    for _ in 0..8 {
        let d = slope(x1, x2, t);
        if d.abs() < 1e-6 {
            break;
        }
        t = (t - (curve(x1, x2, t) - x) / d).clamp(0.0, 1.0);
    }
    curve(y1, y2, t)
}

/// one monitors frozen picture and the windows on it
struct Screen {
    name: String,
    pixbuf: gdk_pixbuf::Pixbuf,
    width: f64,
    height: f64,
    /// window rects on this screen topmost first
    windows: Vec<Rect>,
}

/// whats going on in one monitors overlay
struct Selection {
    shown: Rect,
    from: Rect,
    to: Rect,
    glide_start: Option<Instant>,
    /// where a press started while its held
    press: Option<(f64, f64)>,
    dragging: bool,
    opened: Instant,
    /// set when closing w the time it started
    closing: Option<Instant>,
}

/// one open overlay w its selection and screen size so esc on any monitor reaches them all
struct Open {
    window: gtk4::ApplicationWindow,
    selection: Rc<RefCell<Selection>>,
    size: (f64, f64),
}

thread_local! {
    /// the open overlays so a second mod+shift+s does nothing and esc closes them all
    static OPEN: RefCell<Vec<Open>> = const { RefCell::new(Vec::new()) };
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

/// sevenshell screenshot freezes every monitor and opens the picker on each
pub fn open(app: &gtk4::Application) {
    if BUSY.with(Cell::get) {
        return;
    }
    BUSY.with(|b| b.set(true));
    let Some(display) = gdk::Display::default() else {
        BUSY.with(|b| b.set(false));
        return;
    };
    let monitors: Vec<gdk::Monitor> = (0..display.monitors().n_items())
        .filter_map(|i| display.monitors().item(i).and_downcast::<gdk::Monitor>())
        .collect();
    let names: Vec<String> = monitors
        .iter()
        .map(|m| m.connector().map(|c| c.to_string()).unwrap_or_default())
        .collect();
    // grim runs off the ui thread and the picture comes back as png bytes
    let work = move || {
        let shots: Vec<Option<Vec<u8>>> = names
            .iter()
            .map(|name| {
                let mut cmd = Command::new("grim");
                cmd.args(["-l", "0"]);
                if !name.is_empty() {
                    cmd.args(["-o", name]);
                }
                cmd.arg("-");
                cmd.stderr(Stdio::null())
                    .output()
                    .ok()
                    .filter(|o| o.status.success())
                    .map(|o| o.stdout)
            })
            .collect();
        let state = ipc::request(serde_json::json!({ "get": "state" }))
            .ok()
            .and_then(|v| serde_json::from_value::<ipc::State>(v).ok());
        (shots, state)
    };
    let app = app.clone();
    crate::wake::off_thread(work, move |(shots, state)| {
        let radius = corner_radius();
        for (monitor, shot) in monitors.iter().zip(shots) {
            let Some(pixbuf) = shot.and_then(|bytes| {
                let loader = gdk_pixbuf::PixbufLoader::new();
                loader.write(&bytes).ok()?;
                loader.close().ok()?;
                loader.pixbuf()
            }) else {
                continue;
            };
            let geometry = monitor.geometry();
            let name = monitor.connector().map(|c| c.to_string()).unwrap_or_default();
            let windows = state.as_ref().map_or_else(Vec::new, |s| windows_on(s, &name, radius));
            let screen = Screen {
                name,
                pixbuf,
                width: geometry.width() as f64,
                height: geometry.height() as f64,
                windows,
            };
            let focused = state
                .as_ref()
                .and_then(|s| s.focused())
                .filter(|w| !w.hidden_from_screencast)
                .and_then(|w| on_screen(state.as_ref()?, &screen.name, w, radius));
            overlay(&app, monitor, screen, focused);
        }
        if OPEN.with(|o| o.borrow().is_empty()) {
            BUSY.with(|b| b.set(false));
        }
    });
}

/// sevenwms window corner radius from its config so the selection hugs rounded windows
fn corner_radius() -> f64 {
    let path = theme::expand("~/.config/sevenwm/config.toml");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| t.get("decorations")?.get("corner_radius")?.as_integer())
        .unwrap_or(10) as f64
}

/// a windows rect on a monitors screen or none if its somewhere else
fn on_screen(state: &ipc::State, monitor: &str, w: &ipc::WindowInfo, radius: f64) -> Option<Rect> {
    let m = state.monitor(monitor).or(state.monitors.first())?;
    let [x, y, width, height] = w.rect.map(f64::from);
    let r = Rect {
        x: (x - m.camera[0]) * m.zoom,
        y: (y - m.camera[1]) * m.zoom,
        w: width * m.zoom,
        h: height * m.zoom,
        r: if w.fullscreen { 0.0 } else { radius * m.zoom },
    };
    let (sw, sh) = (m.size[0] as f64, m.size[1] as f64);
    (r.x < sw && r.y < sh && r.x + r.w > 0.0 && r.y + r.h > 0.0).then_some(r)
}

/// the windows u can see on a monitor topmost first like fullscreen then floating then tiles
fn windows_on(state: &ipc::State, monitor: &str, radius: f64) -> Vec<Rect> {
    // hidden ones arent in the shot so snapping to them would js grab whats behind
    let mut windows: Vec<&ipc::WindowInfo> = state.windows.iter().filter(|w| !w.collapsed && !w.hidden_from_screencast).collect();
    windows.sort_by_key(|w| (!w.fullscreen, w.tiled, w.recent.unwrap_or(usize::MAX)));
    windows
        .into_iter()
        .filter_map(|w| on_screen(state, monitor, w, radius))
        .collect()
}

fn overlay(app: &gtk4::Application, monitor: &gdk::Monitor, screen: Screen, focused: Option<Rect>) {
    let window = gtk4::ApplicationWindow::new(app);
    window.init_layer_shell();
    window.set_namespace(Some("sevenshell-screenshot"));
    window.set_layer(Layer::Overlay);
    window.set_monitor(Some(monitor));
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    window.set_exclusive_zone(-1);
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }
    window.add_css_class("screenshot");
    crate::style::adopt(&window);

    // it starts on the focused window or a box in the middle till the pointer moves
    let start = focused.unwrap_or(Rect {
        x: screen.width / 2.0 - 200.0,
        y: screen.height / 2.0 - 120.0,
        w: 400.0,
        h: 240.0,
        r: 0.0,
    });
    let selection = Rc::new(RefCell::new(Selection {
        shown: start,
        from: start,
        to: start,
        glide_start: None,
        press: None,
        dragging: false,
        opened: Instant::now(),
        closing: None,
    }));
    let screen = Rc::new(screen);

    let area = gtk4::DrawingArea::new();
    area.set_cursor_from_name(Some("crosshair"));
    {
        let (selection, screen) = (selection.clone(), screen.clone());
        area.set_draw_func(move |_, cr, _, _| draw(cr, &screen, &selection.borrow()));
    }
    // one tick callback moves the glide and the fades along
    {
        let selection = selection.clone();
        area.add_tick_callback(move |area, _| {
            let mut s = selection.borrow_mut();
            if let Some(start) = s.glide_start {
                let t = start.elapsed().as_secs_f64() / GLIDE.as_secs_f64();
                s.shown = Rect::lerp(s.from, s.to, ease(t));
                if t >= 1.0 {
                    s.glide_start = None;
                }
            }
            let fade_in = (s.opened.elapsed().as_secs_f64() / FADE.as_secs_f64()).min(1.0);
            let opacity = match s.closing {
                Some(at) => 1.0 - (at.elapsed().as_secs_f64() / GLIDE.as_secs_f64()).min(1.0),
                None => fade_in,
            };
            area.set_opacity(ease(opacity));
            let done = s.closing.is_some_and(|at| at.elapsed() >= GLIDE);
            drop(s);
            area.queue_draw();
            if done {
                close_all();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    // hovering snaps the selection to the window under the pointer
    let motion = gtk4::EventControllerMotion::new();
    {
        let (selection, screen) = (selection.clone(), screen.clone());
        motion.connect_motion(move |_, x, y| {
            let mut s = selection.borrow_mut();
            if s.press.is_some() || s.closing.is_some() {
                return;
            }
            if let Some(target) = screen.windows.iter().find(|r| r.contains(x, y)).copied()
                && target != s.to
            {
                glide(&mut s, target);
            }
        });
    }
    area.add_controller(motion);

    // pressing and dragging draws ur own area and letting go takes the shot
    let drag = gtk4::GestureDrag::new();
    {
        let selection = selection.clone();
        drag.connect_drag_begin(move |_, x, y| {
            let mut s = selection.borrow_mut();
            s.press = Some((x, y));
            s.dragging = false;
        });
    }
    {
        let selection = selection.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            let mut s = selection.borrow_mut();
            let Some((x, y)) = s.press else {
                return;
            };
            if !s.dragging && dx.hypot(dy) < DRAG_SLOP {
                return;
            }
            s.dragging = true;
            s.glide_start = None;
            let r = Rect::between(x, y, x + dx, y + dy);
            s.shown = r;
            s.to = r;
        });
    }
    {
        let (selection, screen) = (selection.clone(), screen.clone());
        drag.connect_drag_end(move |_, _, _| {
            let rect = {
                let mut s = selection.borrow_mut();
                s.press = None;
                s.to
            };
            if rect.w >= 2.0 && rect.h >= 2.0 {
                take(&screen, rect);
            }
        });
    }
    area.add_controller(drag);

    // escape closes w the selection growing to the whole screen as it fades and enter takes it
    let keys = gtk4::EventControllerKey::new();
    {
        let (selection, screen) = (selection.clone(), screen.clone());
        keys.connect_key_pressed(move |_, key, _, _| {
            match key {
                gdk::Key::Escape => cancel_all(),
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    let rect = selection.borrow().to;
                    take(&screen, rect);
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
    }
    window.add_controller(keys);

    window.set_child(Some(&area));
    window.present();
    let size = (screen.width, screen.height);
    OPEN.with(|o| o.borrow_mut().push(Open { window, selection, size }));
}

fn glide(s: &mut Selection, target: Rect) {
    s.from = s.shown;
    s.to = target;
    s.glide_start = Some(Instant::now());
}

fn draw(cr: &gtk4::cairo::Context, screen: &Screen, s: &Selection) {
    let palette = theme::current();
    // the frozen picture drawn at its logical size
    let scale = screen.pixbuf.width() as f64 / screen.width.max(1.0);
    let _ = cr.save();
    cr.scale(1.0 / scale, 1.0 / scale);
    cr.set_source_pixbuf(&screen.pixbuf, 0.0, 0.0);
    let _ = cr.paint();
    let _ = cr.restore();

    // a tint over everything but the selection
    let r = s.shown;
    let (tr, tg, tb, _) = theme::rgba(palette.get("m3secondaryContainer"), 1.0);
    cr.set_source_rgba(tr, tg, tb, 0.45);
    cr.set_fill_rule(gtk4::cairo::FillRule::EvenOdd);
    cr.rectangle(0.0, 0.0, screen.width, screen.height);
    crate::bar::rounded(cr, r.x, r.y, r.w, r.h, r.r);
    let _ = cr.fill();

    // the outline in the accent color just outside the selection
    let (pr, pg, pb, _) = theme::rgba(palette.get("m3primary"), 1.0);
    cr.set_source_rgba(pr, pg, pb, 1.0);
    cr.set_line_width(2.0);
    crate::bar::rounded(cr, r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0, if r.r > 0.0 { r.r + 1.0 } else { 0.0 });
    let _ = cr.stroke();

    // the size while u drag
    if s.dragging {
        let text = format!("{} × {}", r.w.round(), r.h.round());
        cr.select_font_face("Google Sans Flex", gtk4::cairo::FontSlant::Normal, gtk4::cairo::FontWeight::Bold);
        cr.set_font_size(13.0);
        if let Ok(ext) = cr.text_extents(&text) {
            let (bx, by) = (r.x, (r.y + r.h + 8.0).min(screen.height - 28.0));
            let (w, h) = (ext.width() + 20.0, 24.0);
            let (sr, sg, sb, _) = theme::rgba(palette.get("m3primary"), 1.0);
            cr.set_source_rgba(sr, sg, sb, 1.0);
            crate::bar::rounded(cr, bx, by, w, h, h / 2.0);
            let _ = cr.fill();
            let (or, og, ob, _) = theme::rgba(palette.get("m3onPrimary"), 1.0);
            cr.set_source_rgba(or, og, ob, 1.0);
            cr.move_to(bx + 10.0, by + h / 2.0 + ext.height() / 2.0);
            let _ = cr.show_text(&text);
        }
    }
}

/// start the closing fade on every overlay w its selection growing to the whole screen
fn cancel_all() {
    OPEN.with(|o| {
        for open in o.borrow().iter() {
            let mut s = open.selection.borrow_mut();
            if s.closing.is_none() {
                let (w, h) = open.size;
                glide(&mut s, Rect { x: 0.0, y: 0.0, w, h, r: 0.0 });
                s.closing = Some(Instant::now());
            }
        }
    });
}

fn close_all() {
    let open: Vec<Open> = OPEN.with(|o| o.borrow_mut().drain(..).collect());
    for o in open {
        o.window.close();
    }
    BUSY.with(|b| b.set(false));
}

/// crop the frozen picture copy it and say so w a notification u can save or edit from
fn take(screen: &Screen, rect: Rect) {
    let scale = screen.pixbuf.width() as f64 / screen.width.max(1.0);
    let clamp = |v: f64, max: i32| (v * scale).round().clamp(0.0, max as f64) as i32;
    let (x, y) = (clamp(rect.x, screen.pixbuf.width()), clamp(rect.y, screen.pixbuf.height()));
    let w = clamp(rect.x + rect.w, screen.pixbuf.width()) - x;
    let h = clamp(rect.y + rect.h, screen.pixbuf.height()) - y;
    close_all();
    if w < 1 || h < 1 {
        return;
    }
    let shot = screen.pixbuf.new_subpixbuf(x, y, w, h);
    let path = shot_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(err) = shot.savev(&path, "png", &[]) {
        eprintln!("sevenshell: saving the screenshot: {err}");
        return;
    }
    std::thread::spawn(move || {
        prune_shots(&path);
        after_shot(path);
    });
}

/// how many unsaved shots stay in the runtime folder which lives in ram
const KEEP_SHOTS: usize = 20;

/// forget all but the newest KEEP_SHOTS unsaved shots so a long session doesnt fill ram
fn prune_shots(newest: &std::path::Path) {
    let Some(dir) = newest.parent() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut shots: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "png"))
        .filter_map(|p| Some((p.metadata().ok()?.modified().ok()?, p)))
        .collect();
    shots.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    for (_, path) in shots.into_iter().skip(KEEP_SHOTS) {
        let _ = std::fs::remove_file(path);
    }
}

/// where a shot waits before u pick save which is the runtime folder so it goes away on logout
fn shot_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let name = glib::DateTime::now_local()
        .and_then(|t| t.format("%Y-%m-%d_%H-%M-%S"))
        .map(|s| s.to_string())
        .unwrap_or_else(|_| "screenshot".into());
    // two shots in the same second get a number so neither is lost
    let dir = dir.join("sevenshell/screenshots");
    (1..)
        .map(|n| if n == 1 { dir.join(format!("{name}.png")) } else { dir.join(format!("{name}-{n}.png")) })
        .find(|p| !p.exists())
        .unwrap_or_else(|| dir.join(format!("{name}.png")))
}

/// copy it then ask w a notification if u wanna save or edit it
fn after_shot(path: PathBuf) {
    if let Ok(file) = std::fs::File::open(&path) {
        let _ = Command::new("wl-copy")
            .args(["--type", "image/png"])
            .stdin(file)
            .status();
    }
    let answer = Command::new("notify-send")
        .args(["-a", "Screenshot", "-i"])
        .arg(&path)
        .args(["-A", "default=Edit", "-A", "save=Save", "-A", "edit=Edit"])
        .args(["Screenshot taken", "Copied to the clipboard"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    match answer.as_str() {
        "save" => {
            let dir = theme::expand("~/Pictures/Screenshots");
            let _ = std::fs::create_dir_all(&dir);
            if let Some(name) = path.file_name() {
                let _ = std::fs::copy(&path, dir.join(name));
            }
        }
        "edit" | "default" => edit(&path),
        _ => {}
    }
}

/// open the shot in satty or swappy or else whatever opens pngs
fn edit(path: &PathBuf) {
    let has = |tool: &str| {
        Command::new("sh")
            .args(["-c", &format!("command -v {tool}")])
            .stdout(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    };
    let mut cmd = if has("satty") {
        let mut c = Command::new("satty");
        c.arg("--filename").arg(path);
        c
    } else if has("swappy") {
        let mut c = Command::new("swappy");
        c.arg("-f").arg(path);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(path);
        c
    };
    if let Ok(mut child) = cmd.spawn() {
        let _ = child.wait();
    }
}
