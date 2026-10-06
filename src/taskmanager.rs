//! the task manager like windows w a processes page u can end tasks from and a performance page w live graphs

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gtk4::prelude::*;
use gtk4::{cairo, glib};

use crate::bar::rounded;
use crate::theme;

/// how many seconds the graphs remember
const HISTORY: usize = 60;

/// the task manager look on top of the settings one it borrows
pub const CSS: &str = "
.taskmanager .cols { padding: 0 20px; margin-bottom: 6px; }
.taskmanager .col {
    background: transparent; border: none; box-shadow: none; border-radius: 9999px;
    padding: 4px 10px; min-height: 0; font-size: 12px; font-weight: 500; color: @m3onSurfaceVariant;
}
.taskmanager .col:hover { background: alpha(@m3onSurface, 0.08); }
.taskmanager .col.sorted label { color: @m3primary; }
.taskmanager .proc { background: @m3surfaceContainer; border-radius: 4px; padding: 0 12px 0 20px; min-height: 40px; }
.taskmanager .proc:hover { background: @m3surfaceContainerHigh; }
.taskmanager .proc.selected { background: @m3secondaryContainer; }
.taskmanager .proc.selected label { color: @m3onSecondaryContainer; }
.taskmanager .proc.first { border-top-left-radius: 20px; border-top-right-radius: 20px; }
.taskmanager .proc.last { border-bottom-left-radius: 20px; border-bottom-right-radius: 20px; }
.taskmanager .cell { padding: 10px 10px; font-size: 13px; }
.taskmanager .num { font-feature-settings: \"tnum\"; }
.taskmanager .heat1 { background: alpha(@m3primary, 0.08); }
.taskmanager .heat2 { background: alpha(@m3primary, 0.16); }
.taskmanager .heat3 { background: alpha(@m3primary, 0.26); }
.taskmanager .heat4 { background: alpha(@m3primary, 0.38); }
.taskmanager .tile {
    background: transparent; border: none; box-shadow: none; border-radius: 20px; padding: 10px 12px;
}
.taskmanager .tile:hover { background: alpha(@m3onSurface, 0.08); }
.taskmanager .tile.selected { background: @m3secondaryContainer; }
.taskmanager .tile-name { font-size: 14px; font-weight: 500; }
.taskmanager .tile-value { font-size: 12px; color: @m3onSurfaceVariant; }
.taskmanager .big-title { font-size: 28px; font-weight: 500; }
.taskmanager .big-sub { font-size: 14px; color: @m3onSurfaceVariant; }
.taskmanager .stat-name { font-size: 12px; color: @m3onSurfaceVariant; }
.taskmanager .stat-value { font-size: 20px; font-weight: 500; }
.taskmanager .stats { background: @m3surfaceContainer; border-radius: 20px; padding: 16px 20px; }
";

/// one app or background process group as a row
#[derive(Clone, Debug, Default)]
struct Proc {
    key: String,
    name: String,
    pids: Vec<i32>,
    cpu: f64,
    mem: u64,
    disk: f64,
    /// has a window so it goes under apps
    app: bool,
    icon: String,
}

#[derive(Clone, Debug, Default)]
struct Gpu {
    name: String,
    util: f64,
    mem_used: f64,
    mem_total: f64,
    temp: f64,
}

/// everything one refresh measured
#[derive(Clone, Debug, Default)]
struct Snapshot {
    procs: Vec<Proc>,
    cpu: f64,
    cpu_name: String,
    cpu_mhz: f64,
    cores: usize,
    threads: u64,
    uptime: f64,
    mem_used: u64,
    mem_total: u64,
    mem_cached: u64,
    swap_used: u64,
    swap_total: u64,
    gpu: Option<Gpu>,
    disk: Option<(String, f64, f64)>,
    net: Option<(String, f64, f64)>,
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// keeps the last readings so each refresh can turn counters into rates
#[derive(Default)]
struct Sampler {
    cpu_total: (u64, u64),
    procs: HashMap<i32, (u64, u64)>,
    disk: Option<(u64, u64)>,
    net: Option<(u64, u64)>,
    at: Option<Instant>,
}

fn kb(line: &str) -> u64 {
    line.split_whitespace().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0) * 1024
}

impl Sampler {
    fn sample(&mut self) -> Snapshot {
        let now = Instant::now();
        let dt = self.at.map_or(1.0, |a| now.duration_since(a).as_secs_f64().max(0.1));
        self.at = Some(now);
        let mut s = Snapshot::default();

        // the whole cpu from the first line of proc stat
        let stat = read("/proc/stat");
        let fields: Vec<u64> = stat.lines().next().unwrap_or("").split_whitespace().skip(1).filter_map(|v| v.parse().ok()).collect();
        let total: u64 = fields.iter().sum();
        let idle = fields.get(3).copied().unwrap_or(0) + fields.get(4).copied().unwrap_or(0);
        let (pt, pi) = self.cpu_total;
        if total > pt {
            s.cpu = 100.0 * (1.0 - (idle - pi.min(idle)) as f64 / (total - pt) as f64);
        }
        let cpu_delta = (total - pt.min(total)).max(1) as f64;
        self.cpu_total = (total, idle);
        s.cores = stat.lines().filter(|l| l.starts_with("cpu") && !l.starts_with("cpu ")).count().max(1);
        let info = read("/proc/cpuinfo");
        s.cpu_name = info.lines().find(|l| l.starts_with("model name")).and_then(|l| l.split(':').nth(1)).unwrap_or("").trim().to_string();
        let mhz: Vec<f64> = info.lines().filter(|l| l.starts_with("cpu MHz")).filter_map(|l| l.split(':').nth(1)?.trim().parse().ok()).collect();
        s.cpu_mhz = if mhz.is_empty() { 0.0 } else { mhz.iter().sum::<f64>() / mhz.len() as f64 };
        s.uptime = read("/proc/uptime").split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0.0);

        let mem = read("/proc/meminfo");
        let field = |name: &str| mem.lines().find(|l| l.starts_with(name)).map(kb).unwrap_or(0);
        s.mem_total = field("MemTotal:");
        s.mem_used = s.mem_total.saturating_sub(field("MemAvailable:"));
        s.mem_cached = field("Cached:");
        s.swap_total = field("SwapTotal:");
        s.swap_used = s.swap_total.saturating_sub(field("SwapFree:"));

        // ur own processes grouped by program like the tray does
        let uid = unsafe { libc::getuid() };
        let windows = window_pids();
        let page = 4096u64;
        let mut groups: HashMap<String, Proc> = HashMap::new();
        let mut seen = HashMap::new();
        let me = std::process::id() as i32;
        for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
                continue;
            };
            let dir = format!("/proc/{pid}");
            let Ok(meta) = std::fs::metadata(&dir) else {
                continue;
            };
            use std::os::unix::fs::MetadataExt;
            let stat = read(&format!("{dir}/stat"));
            // comm can have spaces so start after the last closing paren
            let Some(after) = stat.rfind(')').map(|i| &stat[i + 2..]) else {
                continue;
            };
            let f: Vec<&str> = after.split_whitespace().collect();
            s.threads += f.get(17).and_then(|v| v.parse::<u64>().ok()).unwrap_or(1);
            if meta.uid() != uid || pid == me {
                continue;
            }
            let ticks = f.get(11).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) + f.get(12).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
            let rss = read(&format!("{dir}/statm")).split_whitespace().nth(1).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) * page;
            let io = read(&format!("{dir}/io"));
            let io_field = |n: &str| io.lines().find(|l| l.starts_with(n)).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap_or(0);
            let bytes = io_field("read_bytes:") + io_field("write_bytes:");
            let name = std::fs::read_link(format!("{dir}/exe"))
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().trim_end_matches(" (deleted)").to_string()))
                .filter(|n| n.chars().any(char::is_alphabetic))
                .unwrap_or_else(|| read(&format!("{dir}/comm")).trim().to_string());
            if name.is_empty() {
                continue;
            }
            let (pticks, pbytes) = self.procs.get(&pid).copied().unwrap_or((ticks, bytes));
            seen.insert(pid, (ticks, bytes));
            let g = groups.entry(name.clone()).or_insert_with(|| Proc { key: name.clone(), name: pretty(&name), ..Default::default() });
            g.pids.push(pid);
            g.cpu += 100.0 * (ticks.saturating_sub(pticks)) as f64 / cpu_delta;
            g.mem += rss;
            g.disk += bytes.saturating_sub(pbytes) as f64 / dt;
            if let Some(app_id) = windows.get(&pid) {
                g.app = true;
                g.icon = app_id.clone();
            }
        }
        self.procs = seen;
        s.procs = groups.into_values().collect();

        s.gpu = gpu();
        // the busiest real disk and the network card the default route uses
        let diskstats = read("/proc/diskstats");
        if let Some(line) = diskstats.lines().find(|l| {
            let n = l.split_whitespace().nth(2).unwrap_or("");
            (n.starts_with("nvme") && !n.contains('p')) || (n.starts_with("sd") && n.len() == 3)
        }) {
            let f: Vec<&str> = line.split_whitespace().collect();
            let sectors = |i: usize| f.get(i).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) * 512;
            let (r, w) = (sectors(5), sectors(9));
            let (pr, pw) = self.disk.unwrap_or((r, w));
            self.disk = Some((r, w));
            s.disk = Some((f[2].to_string(), (r - pr.min(r)) as f64 / dt, (w - pw.min(w)) as f64 / dt));
        }
        let route = read("/proc/net/route");
        if let Some(iface) = route.lines().skip(1).find(|l| l.split_whitespace().nth(1) == Some("00000000")).and_then(|l| l.split_whitespace().next()) {
            let dev = read("/proc/net/dev");
            if let Some(line) = dev.lines().find(|l| l.trim_start().starts_with(&format!("{iface}:"))) {
                let f: Vec<u64> = line.split(':').nth(1).unwrap_or("").split_whitespace().filter_map(|v| v.parse().ok()).collect();
                let (rx, tx) = (f.first().copied().unwrap_or(0), f.get(8).copied().unwrap_or(0));
                let (prx, ptx) = self.net.unwrap_or((rx, tx));
                self.net = Some((rx, tx));
                s.net = Some((iface.to_string(), (rx - prx.min(rx)) as f64 / dt, (tx - ptx.min(tx)) as f64 / dt));
            }
        }
        s
    }
}

/// window owning pids and their app ids from sevenwm so apps can be told from background stuff
fn window_pids() -> HashMap<i32, String> {
    crate::ipc::request(serde_json::json!({ "get": "state" }))
        .ok()
        .and_then(|v| serde_json::from_value::<crate::ipc::State>(v).ok())
        .map(|state| state.windows.iter().filter_map(|w| Some((w.pid?, w.app_id.clone()))).collect())
        .unwrap_or_default()
}

/// an nvidia card that runtime pm put to sleep so asking it anything would wake it up
fn nvidia_asleep() -> bool {
    (0..8).any(|i| {
        let base = format!("/sys/class/drm/card{i}/device");
        read(&format!("{base}/vendor")).trim() == "0x10de"
            && matches!(read(&format!("{base}/power/runtime_status")).trim(), "suspended" | "suspending")
    })
}

/// nvidia thru nvidia-smi or amd thru sysfs
fn gpu() -> Option<Gpu> {
    if nvidia_asleep() {
        return Some(Gpu { name: "NVIDIA GPU (asleep)".into(), ..Gpu::default() });
    }
    let out = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu", "--format=csv,noheader,nounits"])
        .output()
        .ok()
        .filter(|o| o.status.success());
    if let Some(out) = out {
        let text = String::from_utf8_lossy(&out.stdout);
        let f: Vec<&str> = text.lines().next().unwrap_or("").split(',').map(str::trim).collect();
        if f.len() == 5 {
            let n = |i: usize| f[i].parse::<f64>().unwrap_or(0.0);
            return Some(Gpu { name: f[0].to_string(), util: n(1), mem_used: n(2) * 1048576.0, mem_total: n(3) * 1048576.0, temp: n(4) });
        }
    }
    for card in ["card0", "card1", "card2"] {
        let base = format!("/sys/class/drm/{card}/device");
        if let Ok(busy) = read(&format!("{base}/gpu_busy_percent")).trim().parse::<f64>() {
            let n = |f: &str| read(&format!("{base}/{f}")).trim().parse::<f64>().unwrap_or(0.0);
            return Some(Gpu { name: "AMD GPU".into(), util: busy, mem_used: n("mem_info_vram_used"), mem_total: n("mem_info_vram_total"), temp: 0.0 });
        }
    }
    None
}

/// kitty into Kitty and brave-browser into Brave browser
fn pretty(name: &str) -> String {
    let n = name.replace(['-', '_'], " ");
    let mut c = n.chars();
    c.next().map_or(String::new(), |f| f.to_uppercase().collect::<String>() + c.as_str())
}

fn bytes(b: f64) -> String {
    if b >= 1073741824.0 {
        format!("{:.1} GB", b / 1073741824.0)
    } else if b >= 1048576.0 {
        format!("{:.0} MB", b / 1048576.0)
    } else if b >= 1024.0 {
        format!("{:.0} KB", b / 1024.0)
    } else {
        format!("{b:.0} B")
    }
}

fn rate(b: f64) -> String {
    if b < 1.0 { "0 MB/s".into() } else if b >= 1048576.0 { format!("{:.1} MB/s", b / 1048576.0) } else { format!("{:.0} KB/s", b / 1024.0) }
}

#[derive(Clone, Copy, PartialEq)]
enum Sort {
    Name,
    Cpu,
    Mem,
    Disk,
}

/// one device on the performance page w its history
struct Device {
    id: &'static str,
    tile: gtk4::Button,
    value: gtk4::Label,
    spark: gtk4::DrawingArea,
    history: Rc<RefCell<VecDeque<f64>>>,
}

struct TaskManager {
    window: gtk4::ApplicationWindow,
    list: gtk4::Box,
    rows: RefCell<HashMap<String, (gtk4::Box, [gtk4::Label; 4], gtk4::Image)>>,
    sort: Cell<Sort>,
    headers: RefCell<Vec<(Sort, gtk4::Button)>>,
    search: gtk4::Entry,
    last: RefCell<Snapshot>,
    status: gtk4::Label,
    devices: RefCell<Vec<Device>>,
    device: Cell<&'static str>,
    big_title: gtk4::Label,
    big_sub: gtk4::Label,
    big_graph: gtk4::DrawingArea,
    stats: gtk4::Grid,
    running: Arc<AtomicBool>,
}

thread_local! {
    static OPEN: RefCell<Option<Rc<TaskManager>>> = const { RefCell::new(None) };
}

/// sevenshell taskmanager opens it or brings it forward
pub fn open(app: &gtk4::Application) {
    if let Some(tm) = OPEN.with(|o| o.borrow().clone()) {
        tm.window.present();
        return;
    }
    let tm = TaskManager::new(app);
    OPEN.with(|o| *o.borrow_mut() = Some(tm.clone()));
    tm.window.present();
}

fn label(text: &str, class: &str) -> gtk4::Label {
    let l = gtk4::Label::new(Some(text));
    if !class.is_empty() {
        l.add_css_class(class);
    }
    l.set_xalign(0.0);
    l
}

impl TaskManager {
    fn new(app: &gtk4::Application) -> Rc<Self> {
        let window = gtk4::ApplicationWindow::new(app);
        window.set_title(Some("Task Manager"));
        window.set_default_size(1040, 700);
        window.add_css_class("settings");
        window.add_css_class("taskmanager");
        crate::style::adopt(&window);

        // the nav rail like the settings one
        let nav = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        nav.add_css_class("nav");
        nav.set_size_request(240, -1);
        nav.set_hexpand(false);
        nav.append(&label("Task Manager", "nav-title"));
        let stack = gtk4::Stack::new();
        stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
        let pages = [("processes", "view_list", "Processes"), ("performance", "monitoring", "Performance")];
        let nav_buttons: Rc<RefCell<Vec<(&str, gtk4::Button)>>> = Rc::default();
        for (id, icon, title) in pages {
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 14);
            row.append(&crate::style::icon(icon));
            row.append(&label(title, ""));
            let b = gtk4::Button::new();
            b.set_child(Some(&row));
            b.add_css_class("navrow");
            if id == "processes" {
                b.add_css_class("selected");
            }
            let (stack, nav_buttons2) = (stack.clone(), nav_buttons.clone());
            b.connect_clicked(move |_| {
                stack.set_visible_child_name(id);
                for (i, b) in nav_buttons2.borrow().iter() {
                    if *i == id {
                        b.add_css_class("selected");
                    } else {
                        b.remove_css_class("selected");
                    }
                }
            });
            nav.append(&b);
            nav_buttons.borrow_mut().push((id, b));
        }
        let spacer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        spacer.set_vexpand(true);
        nav.append(&spacer);
        let status = label("", "status");
        status.set_wrap(true);
        nav.append(&status);

        // processes page
        let procs = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        procs.add_css_class("page");
        let head = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        head.set_margin_bottom(18);
        head.append(&label("Processes", "page-title"));
        let fill = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        fill.set_hexpand(true);
        head.append(&fill);
        let search_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        search_box.add_css_class("search");
        search_box.set_margin_bottom(0);
        search_box.append(&crate::style::icon("search"));
        let search = gtk4::Entry::new();
        search.set_placeholder_text(Some("Type a name to search"));
        search.set_has_frame(false);
        search.set_width_chars(22);
        search_box.append(&search);
        head.append(&search_box);
        let end = gtk4::Button::with_label("End task");
        end.add_css_class("pill");
        end.add_css_class("danger");
        head.append(&end);
        procs.append(&head);
        let cols = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        cols.add_css_class("cols");
        procs.append(&cols);
        let list = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        let scroller = gtk4::ScrolledWindow::new();
        scroller.set_hscrollbar_policy(gtk4::PolicyType::Never);
        scroller.set_vexpand(true);
        scroller.set_child(Some(&list));
        procs.append(&scroller);
        stack.add_named(&procs, Some("processes"));

        // performance page w device tiles on the left and the picked ones graph
        let perf = gtk4::Box::new(gtk4::Orientation::Horizontal, 20);
        perf.add_css_class("page");
        let tiles = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        tiles.set_size_request(220, -1);
        let detail = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        detail.set_hexpand(true);
        let big_title = label("CPU", "big-title");
        let big_sub = label("", "big-sub");
        let big_graph = gtk4::DrawingArea::new();
        big_graph.set_content_height(300);
        big_graph.set_hexpand(true);
        let stats = gtk4::Grid::new();
        stats.add_css_class("stats");
        stats.set_column_spacing(40);
        stats.set_row_spacing(14);
        detail.append(&big_title);
        detail.append(&big_sub);
        detail.append(&big_graph);
        detail.append(&stats);
        perf.append(&tiles);
        perf.append(&detail);
        stack.add_named(&perf, Some("performance"));

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content.add_css_class("content");
        content.set_hexpand(true);
        content.set_overflow(gtk4::Overflow::Hidden);
        content.append(&stack);
        let root = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        root.append(&nav);
        root.append(&content);
        window.set_child(Some(&root));

        let tm = Rc::new(Self {
            window: window.clone(),
            list,
            rows: RefCell::new(HashMap::new()),
            sort: Cell::new(Sort::Cpu),
            headers: RefCell::new(Vec::new()),
            search: search.clone(),
            last: RefCell::new(Snapshot::default()),
            status,
            devices: RefCell::new(Vec::new()),
            device: Cell::new("cpu"),
            big_title,
            big_sub,
            big_graph: big_graph.clone(),
            stats,
            running: Arc::new(AtomicBool::new(true)),
        });

        // the column headers sort when clicked
        for (sort, title, width) in [(Sort::Name, "Name", -1), (Sort::Cpu, "CPU", 90), (Sort::Mem, "Memory", 110), (Sort::Disk, "Disk", 110)] {
            let b = gtk4::Button::with_label(title);
            b.add_css_class("col");
            if sort == Sort::Name {
                b.set_hexpand(true);
                if let Some(l) = b.child().and_downcast::<gtk4::Label>() {
                    l.set_xalign(0.0);
                }
            } else {
                b.set_size_request(width, -1);
                if let Some(l) = b.child().and_downcast::<gtk4::Label>() {
                    l.set_xalign(1.0);
                }
            }
            let t = Rc::downgrade(&tm);
            b.connect_clicked(move |_| {
                if let Some(t) = t.upgrade() {
                    t.sort.set(sort);
                    t.show_procs();
                }
            });
            cols.append(&b);
            tm.headers.borrow_mut().push((sort, b));
        }
        {
            let t = Rc::downgrade(&tm);
            search.connect_changed(move |_| {
                if let Some(t) = t.upgrade() {
                    t.show_procs();
                }
            });
        }
        {
            let t = Rc::downgrade(&tm);
            end.connect_clicked(move |_| {
                if let Some(t) = t.upgrade() {
                    t.end_selected();
                }
            });
        }
        // delete ends the picked task like windows
        let keys = gtk4::EventControllerKey::new();
        {
            let t = Rc::downgrade(&tm);
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gtk4::gdk::Key::Delete
                    && let Some(t) = t.upgrade()
                {
                    t.end_selected();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
        }
        window.add_controller(keys);

        for (id, name) in [("cpu", "CPU"), ("memory", "Memory"), ("gpu", "GPU"), ("disk", "Disk"), ("network", "Network")] {
            let tile = gtk4::Button::new();
            tile.add_css_class("tile");
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
            let spark = gtk4::DrawingArea::new();
            spark.set_content_width(64);
            spark.set_content_height(40);
            let text = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
            text.append(&label(name, "tile-name"));
            let value = label("", "tile-value");
            text.append(&value);
            row.append(&spark);
            row.append(&text);
            tile.set_child(Some(&row));
            if id == "cpu" {
                tile.add_css_class("selected");
            }
            let history: Rc<RefCell<VecDeque<f64>>> = Rc::default();
            {
                let history = history.clone();
                spark.set_draw_func(move |_, cr, w, h| graph(cr, w as f64, h as f64, &history.borrow(), 8.0, false));
            }
            let t = Rc::downgrade(&tm);
            tile.connect_clicked(move |_| {
                if let Some(t) = t.upgrade() {
                    t.device.set(id);
                    for d in t.devices.borrow().iter() {
                        if d.id == id {
                            d.tile.add_css_class("selected");
                        } else {
                            d.tile.remove_css_class("selected");
                        }
                    }
                    t.show_perf();
                }
            });
            tiles.append(&tile);
            tm.devices.borrow_mut().push(Device { id, tile, value, spark, history });
        }
        {
            let t = Rc::downgrade(&tm);
            big_graph.set_draw_func(move |_, cr, w, h| {
                if let Some(t) = t.upgrade() {
                    let devices = t.devices.borrow();
                    if let Some(d) = devices.iter().find(|d| d.id == t.device.get()) {
                        graph(cr, w as f64, h as f64, &d.history.borrow(), 20.0, true);
                    }
                }
            });
        }

        // a worker samples once a second and the ui picks it up
        let (tx, rx) = crate::wake::channel::<Snapshot>();
        let running = tm.running.clone();
        std::thread::spawn(move || {
            let mut sampler = Sampler::default();
            sampler.sample();
            while running.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(1000));
                if tx.unbounded_send(sampler.sample()).is_err() {
                    break;
                }
            }
        });
        {
            let t = Rc::downgrade(&tm);
            crate::wake::each_latest(rx, move |s| {
                let Some(t) = t.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                *t.last.borrow_mut() = s;
                t.update();
                glib::ControlFlow::Continue
            });
        }
        {
            let running = tm.running.clone();
            window.connect_close_request(move |_| {
                running.store(false, Ordering::Relaxed);
                OPEN.with(|o| *o.borrow_mut() = None);
                glib::Propagation::Proceed
            });
        }
        tm
    }

    fn update(&self) {
        let s = self.last.borrow().clone();
        let mem_pct = if s.mem_total > 0 { 100.0 * s.mem_used as f64 / s.mem_total as f64 } else { 0.0 };
        let count: usize = s.procs.iter().map(|p| p.pids.len()).sum();
        self.status.set_text(&format!("{count} processes\nCPU {:.0}% · Memory {mem_pct:.0}%", s.cpu));
        let values: [(&str, f64, String); 5] = [
            ("cpu", s.cpu, format!("{:.0}%  {:.2} GHz", s.cpu, s.cpu_mhz / 1000.0)),
            ("memory", mem_pct, format!("{}/{} ({mem_pct:.0}%)", bytes(s.mem_used as f64), bytes(s.mem_total as f64))),
            ("gpu", s.gpu.as_ref().map_or(0.0, |g| g.util), s.gpu.as_ref().map_or("not found".into(), |g| format!("{:.0}%  {:.0}°C", g.util, g.temp))),
            ("disk", s.disk.as_ref().map_or(0.0, |d| ((d.1 + d.2) / 1048576.0).min(100.0)), s.disk.as_ref().map_or("not found".into(), |d| format!("R {} W {}", rate(d.1), rate(d.2)))),
            ("network", s.net.as_ref().map_or(0.0, |n| ((n.1 + n.2) / 131072.0).min(100.0)), s.net.as_ref().map_or("offline".into(), |n| format!("↓ {} ↑ {}", rate(n.1), rate(n.2)))),
        ];
        for d in self.devices.borrow().iter() {
            if let Some((_, v, text)) = values.iter().find(|(id, _, _)| *id == d.id) {
                let mut h = d.history.borrow_mut();
                h.push_back(*v);
                while h.len() > HISTORY {
                    h.pop_front();
                }
                d.value.set_text(text);
                d.spark.queue_draw();
            }
        }
        self.show_procs();
        self.show_perf();
    }

    fn show_perf(&self) {
        let s = self.last.borrow().clone();
        let mut stats: Vec<(&str, String)> = Vec::new();
        let (title, sub) = match self.device.get() {
            "cpu" => {
                stats = vec![
                    ("Utilization", format!("{:.0}%", s.cpu)),
                    ("Speed", format!("{:.2} GHz", s.cpu_mhz / 1000.0)),
                    ("Threads", s.threads.to_string()),
                    ("Logical processors", s.cores.to_string()),
                    ("Up time", uptime(s.uptime)),
                ];
                ("CPU", s.cpu_name.clone())
            }
            "memory" => {
                stats = vec![
                    ("In use", bytes(s.mem_used as f64)),
                    ("Available", bytes((s.mem_total - s.mem_used.min(s.mem_total)) as f64)),
                    ("Cached", bytes(s.mem_cached as f64)),
                    ("Swap", format!("{}/{}", bytes(s.swap_used as f64), bytes(s.swap_total as f64))),
                ];
                ("Memory", format!("{} total", bytes(s.mem_total as f64)))
            }
            "gpu" => match &s.gpu {
                Some(g) => {
                    stats = vec![("Utilization", format!("{:.0}%", g.util)), ("Memory", format!("{}/{}", bytes(g.mem_used), bytes(g.mem_total))), ("Temperature", format!("{:.0}°C", g.temp))];
                    ("GPU", g.name.clone())
                }
                None => ("GPU", "No gpu stats available".into()),
            },
            "disk" => match &s.disk {
                Some((name, r, w)) => {
                    stats = vec![("Read speed", rate(*r)), ("Write speed", rate(*w))];
                    ("Disk", name.clone())
                }
                None => ("Disk", String::new()),
            },
            _ => match &s.net {
                Some((iface, rx, tx)) => {
                    stats = vec![("Receive", rate(*rx)), ("Send", rate(*tx))];
                    ("Network", iface.clone())
                }
                None => ("Network", "Offline".into()),
            },
        };
        self.big_title.set_text(title);
        self.big_sub.set_text(&sub);
        while let Some(c) = self.stats.first_child() {
            self.stats.remove(&c);
        }
        for (i, (name, value)) in stats.iter().enumerate() {
            let col = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
            col.append(&label(name, "stat-name"));
            col.append(&label(value, "stat-value"));
            self.stats.attach(&col, (i % 3) as i32, (i / 3) as i32, 1, 1);
        }
        self.big_graph.queue_draw();
    }

    /// rebuild the list order in place so rows keep their widgets and selection
    fn show_procs(&self) {
        let s = self.last.borrow();
        let query = self.search.text().to_lowercase();
        let sort = self.sort.get();
        for (k, b) in self.headers.borrow().iter() {
            if *k == sort {
                b.add_css_class("sorted");
            } else {
                b.remove_css_class("sorted");
            }
        }
        let mut procs: Vec<&Proc> = s.procs.iter().filter(|p| query.is_empty() || p.name.to_lowercase().contains(&query)).collect();
        procs.sort_by(|a, b| match sort {
            Sort::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            Sort::Cpu => b.cpu.total_cmp(&a.cpu),
            Sort::Mem => b.mem.cmp(&a.mem),
            Sort::Disk => b.disk.total_cmp(&a.disk),
        });
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let total_mem = s.mem_total.max(1) as f64;
        let groups: [(&str, Vec<&&Proc>); 2] = [
            ("Apps", procs.iter().filter(|p| p.app).collect()),
            ("Background processes", procs.iter().filter(|p| !p.app).collect()),
        ];
        let mut rows = self.rows.borrow_mut();
        let mut alive = Vec::new();
        for (title, group) in groups {
            if group.is_empty() {
                continue;
            }
            let h = label(&format!("{title} ({})", group.len()), "section-title");
            self.list.append(&h);
            for (i, p) in group.iter().enumerate() {
                alive.push(p.key.clone());
                let (row, cells, icon) = rows.entry(p.key.clone()).or_insert_with(|| self.make_row(&p.key)).clone();
                let name = if p.pids.len() > 1 { format!("{} ({})", p.name, p.pids.len()) } else { p.name.clone() };
                cells[0].set_text(&name);
                cells[1].set_text(&format!("{:.1}%", p.cpu));
                cells[2].set_text(&bytes(p.mem as f64));
                cells[3].set_text(&rate(p.disk));
                // hot cells get a stronger tint like windows does
                heat(&cells[1], p.cpu / 100.0 * 4.0);
                heat(&cells[2], p.mem as f64 / total_mem * 20.0);
                heat(&cells[3], p.disk / 10485760.0);
                let icon_name = if !p.icon.is_empty() { p.icon.to_lowercase() } else { p.key.to_lowercase() };
                icon.set_icon_name(Some(if gtk4::IconTheme::for_display(&WidgetExt::display(&self.window)).has_icon(&icon_name) { &icon_name } else { "application-x-executable" }));
                row.remove_css_class("first");
                row.remove_css_class("last");
                if i == 0 {
                    row.add_css_class("first");
                }
                if i + 1 == group.len() {
                    row.add_css_class("last");
                }
                if self.selected_handle().borrow().as_deref() == Some(p.key.as_str()) {
                    row.add_css_class("selected");
                } else {
                    row.remove_css_class("selected");
                }
                self.list.append(&row);
            }
        }
        rows.retain(|k, _| alive.contains(k));
    }

    fn make_row(&self, key: &str) -> (gtk4::Box, [gtk4::Label; 4], gtk4::Image) {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        row.add_css_class("proc");
        let icon = gtk4::Image::new();
        icon.set_pixel_size(22);
        icon.set_margin_end(10);
        row.append(&icon);
        let cells: [gtk4::Label; 4] = std::array::from_fn(|i| {
            let l = label("", "cell");
            if i == 0 {
                l.set_hexpand(true);
                l.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            } else {
                l.add_css_class("num");
                l.set_xalign(1.0);
                l.set_size_request([0, 90, 110, 110][i], -1);
            }
            l
        });
        for c in &cells {
            row.append(c);
        }
        // click picks the row and double click asks to end it
        let click = gtk4::GestureClick::new();
        let key = key.to_string();
        let selected = self.selected_handle();
        click.connect_pressed(move |_, n, _, _| {
            *selected.borrow_mut() = Some(key.clone());
            if let Some(tm) = OPEN.with(|o| o.borrow().clone()) {
                tm.show_procs();
                if n == 2 {
                    tm.end_selected();
                }
            }
        });
        row.add_controller(click);
        (row, cells, icon)
    }

    fn selected_handle(&self) -> Rc<RefCell<Option<String>>> {
        // the selection lives on the struct so rows share it thru this
        thread_local! {
            static SEL: Rc<RefCell<Option<String>>> = Rc::default();
        }
        SEL.with(|s| s.clone())
    }

    /// ask first then end every process in the picked group w sigterm like end task
    fn end_selected(&self) {
        let key = self.selected_handle().borrow().clone();
        let Some(key) = key else {
            return;
        };
        let Some(group) = self.last.borrow().procs.iter().find(|p| p.key == key).cloned() else {
            return;
        };
        let count = group.pids.len();
        let mut detail = if count == 1 {
            "Unsaved work in it will be lost".to_string()
        } else {
            format!("All {count} of its processes get stopped and unsaved work in them will be lost")
        };
        // ending the wm or shell takes the whole desktop w it so say so
        let desktop = |pid: &i32| {
            *pid == std::process::id() as i32
                || matches!(read(&format!("/proc/{pid}/comm")).trim(), "sevenwm" | "sevenshell" | "xwayland-satellite")
        };
        if group.pids.iter().any(desktop) {
            detail = format!("This is part of ur desktop and ending it closes ur session or the bar. {detail}");
        }
        let dialog = gtk4::AlertDialog::builder()
            .message(format!("End {}?", group.name))
            .detail(detail)
            .buttons(["Cancel", "End task"])
            .cancel_button(0)
            .default_button(0)
            .modal(true)
            .build();
        let selected = self.selected_handle();
        dialog.choose(Some(&self.window), gtk4::gio::Cancellable::NONE, move |r| {
            if r != Ok(1) {
                return;
            }
            for pid in &group.pids {
                // safety plain syscall and a stale pid js fails
                unsafe {
                    libc::kill(*pid, libc::SIGTERM);
                }
            }
            *selected.borrow_mut() = None;
        });
    }
}

fn heat(cell: &gtk4::Label, level: f64) {
    for i in 1..=4 {
        cell.remove_css_class(&format!("heat{i}"));
    }
    let l = level.clamp(0.0, 4.0) as usize;
    if l > 0 {
        cell.add_css_class(&format!("heat{l}"));
    }
}

fn uptime(secs: f64) -> String {
    let s = secs as u64;
    format!("{}:{:02}:{:02}:{:02}", s / 86400, s / 3600 % 24, s / 60 % 60, s % 60)
}

/// a filled line graph of 0 to 100 values w the newest on the right
fn graph(cr: &cairo::Context, w: f64, h: f64, history: &VecDeque<f64>, radius: f64, grid: bool) {
    let palette = theme::current();
    let set = |name: &str, a: f64| {
        let (r, g, b, _) = theme::rgba(palette.get(name), 1.0);
        cr.set_source_rgba(r, g, b, a);
    };
    let _ = cr.save();
    rounded(cr, 0.0, 0.0, w, h, radius);
    cr.clip();
    set("m3surfaceContainerLowest", 1.0);
    let _ = cr.paint();
    if grid {
        set("m3outlineVariant", 0.5);
        cr.set_line_width(1.0);
        for i in 1..10 {
            let x = (w * i as f64 / 10.0).round() + 0.5;
            cr.move_to(x, 0.0);
            cr.line_to(x, h);
        }
        for i in 1..4 {
            let y = (h * i as f64 / 4.0).round() + 0.5;
            cr.move_to(0.0, y);
            cr.line_to(w, y);
        }
        let _ = cr.stroke();
    }
    if history.len() > 1 {
        let step = w / (HISTORY - 1) as f64;
        let start = w - step * (history.len() - 1) as f64;
        let y = |v: f64| h - (v.clamp(0.0, 100.0) / 100.0) * (h - 4.0) - 2.0;
        cr.move_to(start, h);
        for (i, v) in history.iter().enumerate() {
            cr.line_to(start + step * i as f64, y(*v));
        }
        cr.line_to(w, h);
        cr.close_path();
        set("m3primary", 0.22);
        let _ = cr.fill();
        for (i, v) in history.iter().enumerate() {
            let x = start + step * i as f64;
            if i == 0 {
                cr.move_to(x, y(*v));
            } else {
                cr.line_to(x, y(*v));
            }
        }
        set("m3primary", 1.0);
        cr.set_line_width(2.0);
        let _ = cr.stroke();
    }
    let _ = cr.restore();
}
