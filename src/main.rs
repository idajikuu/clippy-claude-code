// Clippy native: transparent GTK3 window, Cairo-drawn animated WebP, driven
// by the Claude Code state files under ~/.cache/clippy/sessions/. Replaces
// the Tauri webview path which hit a WebKitGTK transparent-compositor bug that
// no user-space workaround could fix.

mod anim;
mod engine;
mod pack;
mod state;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use gdk::prelude::*;
use gtk::prelude::*;
use gtk::{Application, ApplicationWindow, DrawingArea};
use notify::{RecursiveMode, Watcher};

use crate::anim::Anim;
use crate::pack::{webp_path_for, Edge, Pack};
use crate::state::{read_aggregate_state, sessions_dir, ClaudeCodeState};

const WINDOW_SIZE: i32 = 180;

/// Project-root heuristic: the executable lives at
/// `<root>/target/{debug,release}/clippy`, so walk three ancestors up to
/// get the repo root where `packs/` and `public/` live. Overridable via
/// CLIPPY_ROOT for installs that put the binary elsewhere.
fn project_root() -> PathBuf {
    if let Ok(p) = std::env::var("CLIPPY_ROOT") {
        return PathBuf::from(p);
    }
    let exe = std::env::current_exe().expect("current_exe");
    exe.ancestors()
        .nth(3)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

struct App {
    pack: Pack,
    current_node_id: String,
    playing_edge_id: String,
    anim: Anim,
    cc_state: ClaudeCodeState,
}

impl App {
    fn new(pack: Pack, root: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let initial = engine::initial_edge(&pack)
            .ok_or("pack has no initial loop edge")?
            .clone();
        let anim_path = webp_path_for(root, &initial).ok_or("initial edge has no webp path")?;
        let anim = Anim::load(&anim_path)?;
        Ok(Self {
            current_node_id: pack.initial_node.clone(),
            playing_edge_id: initial.id.clone(),
            pack,
            anim,
            cc_state: ClaudeCodeState::idle(),
        })
    }

    fn playing_edge(&self) -> &Edge {
        self.pack
            .edge(&self.playing_edge_id)
            .expect("playing_edge_id must exist in pack")
    }

    fn switch_to(&mut self, edge: &Edge, root: &Path) {
        if self.playing_edge_id == edge.id {
            return;
        }
        match webp_path_for(root, edge).and_then(|p| Anim::load(&p).ok()) {
            Some(new_anim) => {
                self.playing_edge_id = edge.id.clone();
                self.anim = new_anim;
            }
            None => {
                eprintln!("[clippy] failed to load anim for edge {}", edge.id);
            }
        }
    }

    /// Called on state change or at the end of a transition. Picks the next
    /// edge to play given the current node + state, and — if it's a transition
    /// whose target has a follow-up transition — chains directly without
    /// flashing the target's loop.
    fn advance(&mut self, root: &Path) {
        let current_edge = self.playing_edge().clone();

        // Don't interrupt a running one-shot transition; wait for it to finish.
        if !current_edge.is_loop {
            if self.anim.elapsed_ms() < (current_edge.duration * 1000.0) as u32 + 150 {
                return;
            }
            // Transition finished — hop to the target node.
            self.current_node_id = current_edge.target.clone();
        }

        let next_transition = engine::pick_transition(&self.pack, &self.current_node_id, &self.cc_state).cloned();
        if let Some(next) = next_transition {
            self.switch_to(&next, root);
            return;
        }
        if current_edge.is_loop && current_edge.source == self.current_node_id {
            return; // already on the correct loop
        }
        if let Some(loop_edge) = engine::loop_for(&self.pack, &self.current_node_id).cloned() {
            self.switch_to(&loop_edge, root);
        }
    }
}

fn build_ui(
    gtk_app: &Application,
    app: Rc<RefCell<App>>,
    root: PathBuf,
    shared_state: Arc<Mutex<ClaudeCodeState>>,
    sound_enabled: Arc<AtomicBool>,
    custom_sound: Arc<Mutex<Option<PathBuf>>>,
) {
    let win = ApplicationWindow::new(gtk_app);
    win.set_title("Clippy");
    win.set_default_size(WINDOW_SIZE, WINDOW_SIZE);
    win.set_decorated(false);
    win.set_skip_taskbar_hint(true);
    win.set_skip_pager_hint(true);
    win.set_keep_above(true);
    win.set_resizable(false);
    win.set_app_paintable(true);
    // Stick the window to every workspace so it follows you around in GNOME.
    win.stick();

    if let Some((x, y)) = load_window_position() {
        win.move_(x, y);
    }
    {
        let win_clone = win.clone();
        win.connect_configure_event(move |_, _| {
            let (x, y) = win_clone.position();
            save_window_position(x, y);
            false
        });
    }

    let screen: Option<gdk::Screen> = gtk::prelude::GtkWindowExt::screen(&win);
    if let Some(screen) = screen {
        if let Some(visual) = screen.rgba_visual() {
            win.set_visual(Some(&visual));
        } else {
            eprintln!("[clippy] no RGBA visual — compositor missing?");
        }
    }

    let area = DrawingArea::new();
    area.set_size_request(WINDOW_SIZE, WINDOW_SIZE);
    area.add_events(gdk::EventMask::BUTTON_PRESS_MASK);

    // Right-click context menu. Built once and reused for every click.
    let menu = gtk::Menu::new();
    let pin_item = gtk::CheckMenuItem::with_label("Always on top");
    pin_item.set_active(true);
    {
        let win = win.clone();
        pin_item.connect_toggled(move |item| {
            win.set_keep_above(item.is_active());
        });
    }
    let sound_item = gtk::CheckMenuItem::with_label("Sound notifications");
    sound_item.set_active(sound_enabled.load(Ordering::SeqCst));
    {
        let sound_enabled = sound_enabled.clone();
        sound_item.connect_toggled(move |item| {
            sound_enabled.store(item.is_active(), Ordering::SeqCst);
        });
    }
    let choose_sound_item = gtk::MenuItem::with_label("Choose sound…");
    {
        let custom_sound = custom_sound.clone();
        let win = win.clone();
        choose_sound_item.connect_activate(move |_| {
            let dialog = gtk::FileChooserDialog::with_buttons(
                Some("Choose ring sound"),
                Some(&win),
                gtk::FileChooserAction::Open,
                &[
                    ("Cancel", gtk::ResponseType::Cancel),
                    ("Select", gtk::ResponseType::Accept),
                ],
            );
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Audio files"));
            for ext in ["oga", "ogg", "wav", "mp3", "flac", "aac", "opus"] {
                filter.add_pattern(&format!("*.{ext}"));
            }
            dialog.add_filter(filter);
            if let Some(current) = custom_sound.lock().unwrap().as_ref() {
                let _ = dialog.set_filename(current);
            }
            if dialog.run() == gtk::ResponseType::Accept {
                if let Some(path) = dialog.filename() {
                    save_persisted_sound_path(&path);
                    *custom_sound.lock().unwrap() = Some(path);
                }
            }
            unsafe { dialog.destroy(); }
        });
    }
    let open_cc_item = gtk::MenuItem::with_label("Open Claude Code");
    open_cc_item.connect_activate(|_| launch_claude_terminal());

    let quit_item = gtk::MenuItem::with_label("Quit Clippy");
    {
        let gtk_app = gtk_app.clone();
        quit_item.connect_activate(move |_| gtk_app.quit());
    }
    menu.append(&open_cc_item);
    menu.append(&gtk::SeparatorMenuItem::new());
    menu.append(&pin_item);
    menu.append(&sound_item);
    menu.append(&choose_sound_item);
    menu.append(&gtk::SeparatorMenuItem::new());
    menu.append(&quit_item);
    menu.show_all();

    {
        let win_clone = win.clone();
        let menu = menu.clone();
        area.connect_button_press_event(move |_, ev| {
            match (ev.event_type(), ev.button()) {
                // Double-click the mascot launches a Claude Code terminal.
                (gdk::EventType::DoubleButtonPress, 1) => {
                    launch_claude_terminal();
                }
                (gdk::EventType::ButtonPress, 1) => {
                    let (rx, ry) = ev.root();
                    win_clone.begin_move_drag(
                        ev.button() as i32,
                        rx as i32,
                        ry as i32,
                        ev.time(),
                    );
                }
                (gdk::EventType::ButtonPress, 3) => {
                    menu.popup_at_pointer(Some(ev));
                }
                _ => {}
            }
            glib::Propagation::Proceed
        });
    }

    {
        let app = app.clone();
        area.connect_draw(move |widget, ctx| {
            // CAIRO_OPERATOR_SOURCE writes source pixels directly (alpha included),
            // so painting transparent clears the window surface with no accumulation.
            ctx.set_operator(cairo::Operator::Source);
            ctx.set_source_rgba(0.0, 0.0, 0.0, 0.0);
            let _ = ctx.paint();

            let app = app.borrow();
            let looping = app.playing_edge().is_loop;
            let surf = app.anim.current_frame(looping);
            let w = widget.allocated_width() as f64;
            let h = widget.allocated_height() as f64;
            let scale = (w / app.anim.width as f64).min(h / app.anim.height as f64);
            let dw = app.anim.width as f64 * scale;
            let dh = app.anim.height as f64 * scale;
            let dx = (w - dw) / 2.0;
            let dy = (h - dh) / 2.0;

            ctx.set_operator(cairo::Operator::Over);
            let _ = ctx.save();
            ctx.translate(dx, dy);
            ctx.scale(scale, scale);
            let _ = ctx.set_source_surface(surf, 0.0, 0.0);
            let pat = ctx.source();
            // Default filter drops single-pixel detail on 2-3x downscale.
            pat.set_filter(cairo::Filter::Best);
            // Default extend (None) samples transparent outside the source, so
            // wide-kernel filters fade edge pixels to clear — pad clamps
            // samples to the edge instead.
            pat.set_extend(cairo::Extend::Pad);
            let _ = ctx.paint();
            let _ = ctx.restore();
            glib::Propagation::Stop
        });
    }

    win.add(&area);

    // Per-frame tick: pick up any new state from the watcher, check for
    // transition completion, advance, redraw.
    {
        let app = app.clone();
        let area = area.clone();
        let root = root.clone();
        let shared_state = shared_state.clone();
        glib::timeout_add_local(Duration::from_millis(16), move || {
            {
                let mut app = app.borrow_mut();
                let latest = *shared_state.lock().unwrap();
                if app.cc_state != latest {
                    app.cc_state = latest;
                }
                app.advance(&root);
            }
            area.queue_draw();
            glib::ControlFlow::Continue
        });
    }

    win.show_all();
}

/// Path of a small text file that persists the user-selected sound path
/// across restarts. Just the absolute path as UTF-8, one line.
fn sound_path_config() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("clippy").join("sound-path"))
}

fn load_persisted_sound_path() -> Option<PathBuf> {
    let path = sound_path_config()?;
    let text = std::fs::read_to_string(&path).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let p = PathBuf::from(trimmed);
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

fn save_persisted_sound_path(path: &Path) {
    if let Some(cfg) = sound_path_config() {
        if let Some(parent) = cfg.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&cfg, path.to_string_lossy().as_bytes());
    }
}

fn window_position_config() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("clippy").join("window-position"))
}

fn load_window_position() -> Option<(i32, i32)> {
    let text = std::fs::read_to_string(window_position_config()?).ok()?;
    let mut parts = text.trim().split(',');
    let x: i32 = parts.next()?.parse().ok()?;
    let y: i32 = parts.next()?.parse().ok()?;
    Some((x, y))
}

fn save_window_position(x: i32, y: i32) {
    if let Some(cfg) = window_position_config() {
        if let Some(parent) = cfg.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&cfg, format!("{x},{y}"));
    }
}

/// Resolve the sound file to play. In order: menu-picked override (and its
/// persisted form), `CLIPPY_SOUND` env var, then
/// `~/.config/clippy/ring.{oga,ogg,wav,mp3,flac}` so users can drop in a
/// custom ring, then the freedesktop fallbacks. Empty string if nothing
/// resolves.
fn resolve_sound_path(custom: &Mutex<Option<PathBuf>>) -> String {
    if let Some(p) = custom.lock().unwrap().as_ref() {
        if p.exists() {
            return p.to_string_lossy().into_owned();
        }
    }
    if let Ok(p) = std::env::var("CLIPPY_SOUND") {
        if !p.is_empty() && Path::new(&p).exists() {
            return p;
        }
    }
    if let Some(cfg) = dirs::config_dir() {
        let base = cfg.join("clippy");
        for ext in ["oga", "ogg", "wav", "mp3", "flac"] {
            let p = base.join(format!("ring.{ext}"));
            if p.exists() {
                return p.to_string_lossy().into_owned();
            }
        }
    }
    for p in [
        "/usr/share/sounds/freedesktop/stereo/complete.oga",
        "/usr/share/sounds/freedesktop/stereo/message.oga",
        "/usr/share/sounds/freedesktop/stereo/bell.oga",
    ] {
        if Path::new(p).exists() {
            return p.to_string();
        }
    }
    String::new()
}

/// Fire-and-forget "ding" via paplay. Runs asynchronously; we don't block the
/// watcher thread waiting for playback. No-op if the user disabled sound via
/// the tray menu or if no sound file could be resolved.
/// Open a terminal emulator running `claude`. Tries a list of common
/// terminals in order — first one that spawns wins. The spawned process is
/// detached; its lifecycle is independent of clippy.
fn launch_claude_terminal() {
    let attempts: &[(&str, &[&str])] = &[
        ("gnome-terminal", &["--", "claude"]),
        ("konsole", &["-e", "claude"]),
        ("x-terminal-emulator", &["-e", "claude"]),
        ("alacritty", &["-e", "claude"]),
        ("kitty", &["claude"]),
        ("tilix", &["-e", "claude"]),
        ("xterm", &["-e", "claude"]),
    ];
    for (bin, args) in attempts {
        if Command::new(bin)
            .args(*args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
        {
            return;
        }
    }
    eprintln!("[clippy] no supported terminal found for launching claude");
}

fn play_notify_sound(enabled: &AtomicBool, custom: &Mutex<Option<PathBuf>>) {
    if !enabled.load(Ordering::SeqCst) {
        return;
    }
    let path = resolve_sound_path(custom);
    if path.is_empty() {
        return;
    }
    let _ = Command::new("paplay")
        .arg(&path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// Refresh the shared state from the session files on disk. Play a sound on
/// two rising-edge signals from the hooks:
///   - `is_alert` false→true — CC prompted for attention (on-notification)
///   - `is_working` true→false — CC finished a response (on-stop)
fn refresh_state(
    shared: &Mutex<ClaudeCodeState>,
    prev_alert: &AtomicBool,
    prev_working: &AtomicBool,
    sound_enabled: &AtomicBool,
    custom_sound: &Mutex<Option<PathBuf>>,
) {
    let new = read_aggregate_state();
    let was_alert = prev_alert.swap(new.is_alert, Ordering::SeqCst);
    let was_working = prev_working.swap(new.is_working, Ordering::SeqCst);
    let alert_rose = new.is_alert && !was_alert;
    let working_fell = was_working && !new.is_working;
    if alert_rose || working_fell {
        play_notify_sound(sound_enabled, custom_sound);
    }
    *shared.lock().unwrap() = new;
}

/// Watches the sessions dir and refreshes the shared state whenever a file
/// changes. A background tick also re-aggregates every 15s so stale-session
/// eviction takes effect without a filesystem event.
fn spawn_state_watcher(
    shared: Arc<Mutex<ClaudeCodeState>>,
    sound_enabled: Arc<AtomicBool>,
    custom_sound: Arc<Mutex<Option<PathBuf>>>,
) {
    // Seed previous-flag trackers from the initial state so we don't fire a
    // sound on startup just because a live session is already mid-state.
    let initial = read_aggregate_state();
    let prev_alert = Arc::new(AtomicBool::new(initial.is_alert));
    let prev_working = Arc::new(AtomicBool::new(initial.is_working));
    *shared.lock().unwrap() = initial;

    let watch_shared = shared.clone();
    let watch_prev_alert = prev_alert.clone();
    let watch_prev_working = prev_working.clone();
    let watch_sound = sound_enabled.clone();
    let watch_custom = custom_sound.clone();
    std::thread::spawn(move || {
        let (fs_tx, fs_rx) = mpsc::channel();
        let mut watcher: notify::RecommendedWatcher =
            match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if let Ok(evt) = res {
                    let _ = fs_tx.send(evt);
                }
            }) {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("[clippy] watcher init failed: {e}");
                    return;
                }
            };
        let _ = watcher.watch(&sessions_dir(), RecursiveMode::NonRecursive);
        for _evt in fs_rx {
            refresh_state(
                &watch_shared,
                &watch_prev_alert,
                &watch_prev_working,
                &watch_sound,
                &watch_custom,
            );
            // Debounce: we often see a storm of events for one write.
            std::thread::sleep(Duration::from_millis(50));
        }
    });

    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(15));
        refresh_state(
            &shared,
            &prev_alert,
            &prev_working,
            &sound_enabled,
            &custom_sound,
        );
    });
}

fn main() {
    let root = project_root();
    let pack_path = root.join("packs").join("clippy-masko.json");
    eprintln!("[clippy] root={} pack={}", root.display(), pack_path.display());

    let pack = match Pack::load(&pack_path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[clippy] failed to load pack: {e}");
            std::process::exit(1);
        }
    };
    let app = match App::new(pack, &root) {
        Ok(a) => Rc::new(RefCell::new(a)),
        Err(e) => {
            eprintln!("[clippy] failed to init app: {e}");
            std::process::exit(1);
        }
    };

    let gtk_app = Application::builder()
        .application_id("ai.clippy.app")
        .build();

    let shared_state = Arc::new(Mutex::new(ClaudeCodeState::idle()));
    let sound_enabled = Arc::new(AtomicBool::new(true));
    let custom_sound = Arc::new(Mutex::new(load_persisted_sound_path()));
    spawn_state_watcher(
        shared_state.clone(),
        sound_enabled.clone(),
        custom_sound.clone(),
    );

    gtk_app.connect_activate(move |gtk_app| {
        build_ui(
            gtk_app,
            app.clone(),
            root.clone(),
            shared_state.clone(),
            sound_enabled.clone(),
            custom_sound.clone(),
        )
    });
    gtk_app.run();
}
