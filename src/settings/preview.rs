//! the two drawn widgets in settings which are the animation preview and the notification placer

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use gtk4::gdk::prelude::GdkCairoContextExt;
use gtk4::prelude::*;
use gtk4::{cairo, gdk, gdk_pixbuf, glib};

use super::store::{Store, Which};
use crate::bar::rounded;
use crate::theme;

fn paint(cr: &cairo::Context, name: &str, alpha: f64) {
    let (r, g, b, a) = theme::rgba(theme::current().get(name), alpha);
    cr.set_source_rgba(r, g, b, a);
}

fn spring(t: f64) -> f64 {
    if t >= 1.0 { 1.0 } else { 1.0 - (-6.0 * t).exp() * (10.5 * t).cos() }
}

fn back(t: f64) -> f64 {
    let (c1, c3) = (1.70158, 2.70158);
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

/// the same curves sevenwm uses so keep them in sync w its src/animation.rs
pub const CURVES: [&str; 7] = ["linear", "ease-out-quad", "ease-out-cubic", "ease-out-expo", "ease-in-out-cubic", "back", "spring"];

fn ease(curve: &str, t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    match curve {
        "linear" => t,
        "ease-out-quad" => 1.0 - (1.0 - t).powi(2),
        "ease-out-expo" => if t >= 1.0 { 1.0 } else { 1.0 - 2f64.powf(-10.0 * t) },
        "ease-in-out-cubic" => if t < 0.5 { 4.0 * t.powi(3) } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 },
        "back" => back(t),
        "spring" => spring(t),
        _ => 1.0 - (1.0 - t).powi(3),
    }
}

/// dx dy scale and alpha of a window p of the way in
fn look(style: &str, p: f64) -> (f64, f64, f64, f64) {
    let fade = p.clamp(0.0, 1.0);
    match style {
        "fade" => (0.0, 0.0, 1.0, fade),
        "zoom" => (0.0, 0.0, (0.85 + 0.15 * p).max(0.01), fade),
        "pop" => (0.0, 0.0, (0.5 + 0.5 * p).max(0.01), (p * 3.0).clamp(0.0, 1.0)),
        "slide" => (0.0, 40.0 * (1.0 - p), 1.0, fade),
        _ => (0.0, 0.0, 1.0, 1.0),
    }
}

/// a fake window that opens slides and closes to preview the animations
pub struct AnimationPreview {
    pub area: gtk4::DrawingArea,
    store: Rc<Store>,
    steps: RefCell<Vec<(&'static str, f64)>>,
    started: Cell<Instant>,
    ticking: Cell<bool>,
}

impl AnimationPreview {
    pub fn new(store: &Rc<Store>) -> Rc<Self> {
        let area = gtk4::DrawingArea::new();
        area.set_content_width(560);
        area.set_content_height(260);
        area.set_hexpand(true);
        let p = Rc::new(Self {
            area: area.clone(),
            store: store.clone(),
            steps: RefCell::new(Vec::new()),
            started: Cell::new(Instant::now()),
            ticking: Cell::new(false),
        });
        let weak = Rc::downgrade(&p);
        area.set_draw_func(move |_, cr, w, h| {
            if let Some(p) = weak.upgrade() {
                p.draw(cr, w as f64, h as f64);
            }
        });
        let weak = Rc::downgrade(&p);
        glib::idle_add_local_once(move || {
            if let Some(p) = weak.upgrade() {
                p.play(None);
            }
        });
        p
    }

    fn ms(&self, path: &str) -> f64 {
        self.store.num(Which::Wm, path).max(1.0)
    }

    /// play one part or all of them
    pub fn play(self: &Rc<Self>, part: Option<&str>) {
        let parts = [
            ("open", self.ms("animations.open_ms")),
            ("move", self.ms("animations.move_ms")),
            ("fly", self.ms("view.fly_duration_ms")),
            ("close", self.ms("animations.close_ms")),
        ];
        let mut steps = Vec::new();
        for (name, length) in parts {
            if part.is_none_or(|p| p == name) {
                steps.push((name, length));
                steps.push(("pause", 450.0));
            }
        }
        *self.steps.borrow_mut() = steps;
        self.started.set(Instant::now());
        if !self.ticking.replace(true) {
            let weak = Rc::downgrade(self);
            self.area.add_tick_callback(move |area, _| {
                area.queue_draw();
                let Some(p) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let total: f64 = p.steps.borrow().iter().map(|(_, l)| l).sum();
                if p.started.get().elapsed().as_secs_f64() * 1000.0 > total + 50.0 {
                    p.ticking.set(false);
                    return glib::ControlFlow::Break;
                }
                glib::ControlFlow::Continue
            });
        }
    }

    /// what the fake window looks like right now
    fn state(&self) -> (f64, (f64, f64, f64, f64), f64, bool) {
        let s = &self.store;
        let a = |k: &str| s.str(Which::Wm, &format!("animations.{k}"));
        let enabled = s.bool(Which::Wm, "animations.enabled");
        let mut elapsed = self.started.get().elapsed().as_secs_f64() * 1000.0;
        let (mut x, mut view_x, mut the_look, mut shown) = (0.0, 0.0, (0.0, 0.0, 1.0, 1.0), true);
        let (mut slid, mut flown) = (false, false);
        let steps = self.steps.borrow().clone();
        let mut finished = true;
        for (kind, length) in steps {
            let t = if elapsed < length { elapsed / length } else { 1.0 };
            match kind {
                "open" => {
                    let p = if enabled { ease(&a("open_curve"), t) } else { 1.0 };
                    the_look = if t < 1.0 { look(&a("open_style"), p) } else { (0.0, 0.0, 1.0, 1.0) };
                }
                "move" => {
                    x = 120.0 * if enabled { ease(&a("move_curve"), t) } else { 1.0 };
                    slid = true;
                }
                "fly" => {
                    view_x = 120.0 * if enabled { ease(&a("fly_curve"), t) } else { 1.0 };
                    flown = true;
                }
                "close" => {
                    let p = if enabled { 1.0 - ease(&a("close_curve"), t) } else { 0.0 };
                    the_look = look(&a("close_style"), p);
                    shown = t < 1.0 && enabled;
                }
                _ => {}
            }
            if elapsed < length {
                finished = false;
                break;
            }
            elapsed -= length;
        }
        if finished {
            // all done so the window is back at rest
            return (0.0, (0.0, 0.0, 1.0, 1.0), 0.0, true);
        }
        if slid && !flown {
            view_x = 0.0;
        }
        (x, the_look, view_x, shown)
    }

    fn draw(&self, cr: &cairo::Context, w: f64, h: f64) {
        paint(cr, "m3surfaceContainerLowest", 1.0);
        rounded(cr, 0.0, 0.0, w, h, 16.0);
        let _ = cr.fill();
        let (x, (dx, dy, scale, alpha), view_x, shown) = self.state();
        // a faint grid for the canvas so u can see the view move
        paint(cr, "m3onSurface", 0.06);
        let mut gx = -40.0;
        while gx < w + 80.0 {
            cr.rectangle(gx - view_x.rem_euclid(40.0), 0.0, 1.0, h);
            gx += 40.0;
        }
        let _ = cr.fill();
        if !shown {
            return;
        }
        let s = &self.store;
        let (ww, wh) = (220.0 * scale, 140.0 * scale);
        let cx = w / 2.0 - 60.0 + x - view_x + dx;
        let cy = h / 2.0 + dy;
        let (left, top) = (cx - ww / 2.0, cy - wh / 2.0);
        let radius = s.num(Which::Wm, "decorations.corner_radius").min(ww / 2.0).min(wh / 2.0) * scale;
        let border = s.num(Which::Wm, "border.width") * scale;
        if border > 0.0 {
            paint(cr, "m3primary", alpha);
            rounded(cr, left - border, top - border, ww + 2.0 * border, wh + 2.0 * border, if radius > 0.0 { radius + border } else { 0.0 });
            let _ = cr.fill();
        }
        paint(cr, "m3surfaceContainerHigh", alpha);
        rounded(cr, left, top, ww, wh, radius);
        let _ = cr.fill();
        // some fake text lines so moving and scaling are easy to see
        paint(cr, "m3onSurfaceVariant", 0.6 * alpha);
        for i in 0..5 {
            cr.rectangle(left + 14.0 * scale, top + (20.0 + i as f64 * 20.0) * scale, (ww - 28.0 * scale) * (0.9 - 0.12 * i as f64), 6.0 * scale);
        }
        let _ = cr.fill();
    }
}

pub const POSITIONS: [&str; 9] = ["top-left", "top", "top-right", "left", "center", "right", "bottom-left", "bottom", "bottom-right"];
/// how tall a normal notification is when height is 0
const AUTO_HEIGHT: f64 = 78.0;
/// how close in real px before edges and the center line snap
const SNAP: f64 = 12.0;
const PREVIEW_W: f64 = 480.0;

#[derive(Clone, Debug)]
pub struct Placement {
    pub position: String,
    pub margin_x: i64,
    pub margin_y: i64,
    pub width: i64,
    pub height: i64,
}

/// a small copy of ur screen w a notification u can drag and resize to place them
pub struct NotificationPlacer {
    pub area: gtk4::DrawingArea,
    screen: (f64, f64),
    bar: f64,
    scale: f64,
    values: RefCell<Placement>,
    rect: Cell<[f64; 4]>,
    /// mode and start rect while dragging
    drag: RefCell<Option<(String, [f64; 4])>>,
    guides: RefCell<Vec<(char, f64)>>,
    wallpaper: Option<gdk_pixbuf::Pixbuf>,
    on_change: Box<dyn Fn(&Placement)>,
}

fn screen_size() -> (f64, f64) {
    gdk::Display::default()
        .and_then(|d| d.monitors().item(0).and_downcast::<gdk::Monitor>())
        .map_or((1920.0, 1080.0), |m| (m.geometry().width() as f64, m.geometry().height() as f64))
}

impl NotificationPlacer {
    pub fn new(values: Placement, bar: f64, wallpaper: &str, on_change: impl Fn(&Placement) + 'static) -> Rc<Self> {
        let screen = screen_size();
        let scale = PREVIEW_W / screen.0;
        let area = gtk4::DrawingArea::new();
        area.set_content_width(PREVIEW_W as i32);
        area.set_content_height((screen.1 * scale).round() as i32);
        let (pw, ph) = (PREVIEW_W as i32, (screen.1 * scale).round() as i32);
        let wallpaper = (!wallpaper.is_empty())
            .then(|| gdk_pixbuf::Pixbuf::from_file(theme::expand(wallpaper)).ok())
            .flatten()
            .and_then(|pix| {
                let k = (pw as f64 / pix.width() as f64).max(ph as f64 / pix.height() as f64);
                let (w, h) = ((pix.width() as f64 * k) as i32 + 1, (pix.height() as f64 * k) as i32 + 1);
                let scaled = pix.scale_simple(w, h, gdk_pixbuf::InterpType::Bilinear)?;
                Some(scaled.new_subpixbuf((w - pw) / 2, (h - ph) / 2, pw, ph))
            });
        let p = Rc::new(Self {
            area: area.clone(),
            screen,
            bar,
            scale,
            rect: Cell::new([0.0; 4]),
            values: RefCell::new(values),
            drag: RefCell::new(None),
            guides: RefCell::new(Vec::new()),
            wallpaper,
            on_change: Box::new(on_change),
        });
        p.rect.set(p.rect_from(&p.values.borrow()));
        let weak = Rc::downgrade(&p);
        area.set_draw_func(move |_, cr, _, _| {
            if let Some(p) = weak.upgrade() {
                p.draw(cr);
            }
        });
        let motion = gtk4::EventControllerMotion::new();
        let weak = Rc::downgrade(&p);
        motion.connect_motion(move |_, x, y| {
            if let Some(p) = weak.upgrade()
                && p.drag.borrow().is_none()
            {
                let mode = p.hit(x / p.scale, y / p.scale);
                p.area.set_cursor_from_name(mode.as_deref().map(cursor));
            }
        });
        area.add_controller(motion);
        let drag = gtk4::GestureDrag::new();
        let weak = Rc::downgrade(&p);
        drag.connect_drag_begin(move |_, x, y| {
            if let Some(p) = weak.upgrade()
                && let Some(mode) = p.hit(x / p.scale, y / p.scale)
            {
                *p.drag.borrow_mut() = Some((mode, p.rect.get()));
            }
        });
        let weak = Rc::downgrade(&p);
        drag.connect_drag_update(move |_, dx, dy| {
            if let Some(p) = weak.upgrade() {
                p.moved(dx / p.scale, dy / p.scale);
            }
        });
        let weak = Rc::downgrade(&p);
        drag.connect_drag_end(move |_, _, _| {
            if let Some(p) = weak.upgrade() {
                *p.drag.borrow_mut() = None;
                p.guides.borrow_mut().clear();
                p.area.queue_draw();
            }
        });
        area.add_controller(drag);
        (p.on_change)(&p.values.borrow());
        p
    }

    fn rect_from(&self, v: &Placement) -> [f64; 4] {
        let (sw, sh, bar) = (self.screen.0, self.screen.1, self.bar);
        let w = v.width as f64;
        let h = if v.height > 0 { v.height as f64 } else { AUTO_HEIGHT };
        let i = POSITIONS.iter().position(|p| *p == v.position).unwrap_or(2);
        let (col, row) = (i % 3, i / 3);
        let x = [v.margin_x as f64, (sw - w) / 2.0, sw - w - v.margin_x as f64][col];
        // layer surfaces sit in the space the bar leaves
        let y = [bar + v.margin_y as f64, bar + (sh - bar - h) / 2.0, sh - h - v.margin_y as f64][row];
        [x, y, w, h]
    }

    fn values_from(&self, r: [f64; 4], auto_height: bool) -> Placement {
        let [x, y, w, h] = r;
        let (sw, sh, bar) = (self.screen.0, self.screen.1, self.bar);
        let old = self.values.borrow().clone();
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        let (col, mx) = if (cx - sw / 2.0).abs() <= SNAP {
            (1, old.margin_x as f64)
        } else if cx < sw / 2.0 {
            (0, x)
        } else {
            (2, sw - x - w)
        };
        let mid = bar + (sh - bar) / 2.0;
        let (row, my) = if (cy - mid).abs() <= SNAP {
            (1, old.margin_y as f64)
        } else if cy < mid {
            (0, y - bar)
        } else {
            (2, sh - y - h)
        };
        Placement {
            position: POSITIONS[row * 3 + col].into(),
            margin_x: mx.round().max(0.0) as i64,
            margin_y: my.round().max(0.0) as i64,
            width: w.round() as i64,
            height: if auto_height { 0 } else { h.round() as i64 },
        }
    }

    /// what pressing here does like move or an edge set to resize or none
    fn hit(&self, px: f64, py: f64) -> Option<String> {
        let [x, y, w, h] = self.rect.get();
        let grab = 7.0 / self.scale;
        if !(x - grab <= px && px <= x + w + grab && y - grab <= py && py <= y + h + grab) {
            return None;
        }
        let mut edges = String::new();
        if (px - x).abs() <= grab {
            edges.push('l');
        } else if (px - (x + w)).abs() <= grab {
            edges.push('r');
        }
        if (py - y).abs() <= grab {
            edges.push('t');
        } else if (py - (y + h)).abs() <= grab {
            edges.push('b');
        }
        Some(if edges.is_empty() { "move".into() } else { edges })
    }

    fn moved(&self, dx: f64, dy: f64) {
        let Some((mode, [mut x, mut y, mut w, mut h])) = self.drag.borrow().clone() else {
            return;
        };
        let (sw, sh, bar) = (self.screen.0, self.screen.1, self.bar);
        let (min_w, min_h) = (200.0, 40.0);
        self.guides.borrow_mut().clear();
        if mode == "move" {
            (x, y) = self.snap_move(x + dx, y + dy, w, h);
        } else {
            if mode.contains('l') {
                let nx = (x + dx).min(x + w - min_w);
                w += x - nx;
                x = nx;
            }
            if mode.contains('r') {
                w = (w + dx).max(min_w);
            }
            if mode.contains('t') {
                let ny = (y + dy).min(y + h - min_h);
                h += y - ny;
                y = ny;
            }
            if mode.contains('b') {
                h = (h + dy).max(min_h);
            }
        }
        // keep it on screen and under the bar
        w = w.min(sw);
        h = h.min(sh - bar);
        x = x.clamp(0.0, sw - w);
        y = y.clamp(bar, sh - h);
        self.rect.set([x, y, w, h]);
        let resized_height = mode != "move" && (mode.contains('t') || mode.contains('b'));
        let auto = self.values.borrow().height == 0 && !resized_height;
        let values = self.values_from([x, y, w, h], auto);
        *self.values.borrow_mut() = values.clone();
        (self.on_change)(&values);
        self.area.queue_draw();
    }

    /// snap to the screen edges and center lines
    fn snap_move(&self, mut x: f64, mut y: f64, w: f64, h: f64) -> (f64, f64) {
        let (sw, sh, bar) = (self.screen.0, self.screen.1, self.bar);
        let gap = 8.0;
        for (target, guide) in [(gap, 0.0), (sw - w - gap, sw), ((sw - w) / 2.0, sw / 2.0)] {
            if (x - target).abs() <= SNAP {
                x = target;
                self.guides.borrow_mut().push(('v', guide));
                break;
            }
        }
        let mid = bar + (sh - bar) / 2.0;
        for (target, guide) in [(bar + gap, bar), (sh - h - gap, sh), (mid - h / 2.0, mid)] {
            if (y - target).abs() <= SNAP {
                y = target;
                self.guides.borrow_mut().push(('h', guide));
                break;
            }
        }
        (x, y)
    }

    pub fn set_auto_height(&self) {
        let values = Placement { height: 0, ..self.values.borrow().clone() };
        self.rect.set(self.rect_from(&values));
        *self.values.borrow_mut() = values.clone();
        (self.on_change)(&values);
        self.area.queue_draw();
    }

    fn draw(&self, cr: &cairo::Context) {
        let s = self.scale;
        let (pw, ph) = (PREVIEW_W, self.screen.1 * s);
        let _ = cr.save();
        rounded(cr, 0.0, 0.0, pw, ph, 12.0);
        cr.clip();
        match &self.wallpaper {
            Some(pix) => cr.set_source_pixbuf(pix, 0.0, 0.0),
            None => paint(cr, "m3surfaceContainerLowest", 1.0),
        }
        let _ = cr.paint();
        paint(cr, "m3surface", 1.0);
        cr.rectangle(0.0, 0.0, pw, self.bar * s);
        let _ = cr.fill();
        // snap guides
        paint(cr, "m3primary", 0.8);
        cr.set_line_width(1.0);
        cr.set_dash(&[3.0, 3.0], 0.0);
        for (axis, at) in self.guides.borrow().iter() {
            if *axis == 'v' {
                cr.move_to(at * s + 0.5, self.bar * s);
                cr.line_to(at * s + 0.5, ph);
            } else {
                cr.move_to(0.0, at * s + 0.5);
                cr.line_to(pw, at * s + 0.5);
            }
        }
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
        // the notification as it looks
        let [x, y, w, h] = self.rect.get().map(|v| v * s);
        paint(cr, "m3surfaceContainer", 1.0);
        rounded(cr, x, y, w, h, 20.0 * s);
        let _ = cr.fill_preserve();
        paint(cr, "m3primary", 1.0);
        cr.set_line_width(if self.drag.borrow().is_some() { 2.0 } else { 1.5 });
        let _ = cr.stroke();
        let pad = 14.0 * s;
        paint(cr, "m3onSurfaceVariant", 0.8);
        for (i, (width, height)) in [(0.25, 11.0), (0.5, 14.0), (0.75, 13.0)].into_iter().enumerate() {
            let ty = y + pad + i as f64 * 19.0 * s;
            if ty + height * s > y + h - pad / 2.0 {
                break;
            }
            cr.rectangle(x + pad, ty, (w - 2.0 * pad) * width, height * s * 0.55);
        }
        let _ = cr.fill();
        // resize handles at the corners
        paint(cr, "m3primary", 1.0);
        for (hx, hy) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
            cr.arc(hx, hy, 3.5, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
        let _ = cr.restore();
    }
}

fn cursor(mode: &str) -> &'static str {
    match mode {
        "move" => "grab",
        "l" => "w-resize",
        "r" => "e-resize",
        "t" => "n-resize",
        "b" => "s-resize",
        "lt" => "nw-resize",
        "rt" => "ne-resize",
        "lb" => "sw-resize",
        _ => "se-resize",
    }
}
