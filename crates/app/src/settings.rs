//! Persistent user settings, stored as JSON under `$XDG_CONFIG_HOME/gittyhub/`.

use std::cell::RefCell;
use std::path::PathBuf;

use git_backend::Location;
use gtk::glib;
use gtk::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// `system`, `light` or `dark`.
    pub color_scheme: String,
    /// `system` or an accent name (`blue`, `teal`, …).
    pub accent: String,
    pub restore_tabs: bool,
    /// `merge`, `rebase` or `ff-only`.
    pub pull_mode: String,
    pub signoff: bool,
    pub history_all_branches: bool,
    /// Minutes between background fetches; 0 disables.
    pub auto_fetch_minutes: u32,
    pub recent: Vec<Location>,
    pub open_tabs: Vec<Location>,
    pub clone_dir: String,
    /// Interface zoom (1.0 = 100%).
    pub ui_zoom: f64,
    /// Font family for code; empty means the system monospace font.
    pub code_font_family: String,
    /// Code font size in points.
    pub code_font_size: f64,
    /// Line height as a multiple of the font size (1.0 = tight).
    pub code_line_spacing: f64,
    /// True once the user's own window size has been saved; until then it is sized to the monitor.
    pub window_size_saved: bool,
    pub window_width: i32,
    pub window_height: i32,
    pub maximized: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            color_scheme: "system".into(),
            accent: "system".into(),
            restore_tabs: true,
            pull_mode: "merge".into(),
            signoff: false,
            history_all_branches: false,
            auto_fetch_minutes: 10,
            recent: Vec::new(),
            open_tabs: Vec::new(),
            clone_dir: glib::home_dir().join("Code").to_string_lossy().into_owned(),
            ui_zoom: 1.0,
            code_font_family: String::new(),
            code_font_size: 11.0,
            code_line_spacing: 1.3,
            window_size_saved: false,
            window_width: 1280,
            window_height: 820,
            maximized: false,
        }
    }
}

thread_local! {
    static SETTINGS: RefCell<Settings> = RefCell::new(load());
    static ACCENT_CSS: gtk::CssProvider = gtk::CssProvider::new();
}

fn path() -> PathBuf {
    glib::user_config_dir()
        .join("gittyhub")
        .join("settings.json")
}

fn load() -> Settings {
    std::fs::read(path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn get() -> Settings {
    SETTINGS.with(|s| s.borrow().clone())
}

/// Change settings and write them to disk.
pub fn update(f: impl FnOnce(&mut Settings)) {
    SETTINGS.with(|s| {
        f(&mut s.borrow_mut());
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(&*s.borrow()) {
            let _ = std::fs::write(p, json);
        }
    });
}

pub fn add_recent(loc: &Location) {
    update(|s| {
        s.recent.retain(|l| l != loc);
        s.recent.insert(0, loc.clone());
        s.recent.truncate(15);
    });
}

pub const ACCENTS: [(&str, &str, adw::AccentColor); 9] = [
    ("blue", "Blue", adw::AccentColor::Blue),
    ("teal", "Teal", adw::AccentColor::Teal),
    ("green", "Green", adw::AccentColor::Green),
    ("yellow", "Yellow", adw::AccentColor::Yellow),
    ("orange", "Orange", adw::AccentColor::Orange),
    ("red", "Red", adw::AccentColor::Red),
    ("pink", "Pink", adw::AccentColor::Pink),
    ("purple", "Purple", adw::AccentColor::Purple),
    ("slate", "Slate", adw::AccentColor::Slate),
];

/// Apply colour scheme and accent colour from settings.
pub fn apply_appearance() {
    let s = get();
    let sm = adw::StyleManager::default();
    sm.set_color_scheme(match s.color_scheme.as_str() {
        "light" => adw::ColorScheme::ForceLight,
        "dark" => adw::ColorScheme::ForceDark,
        _ => adw::ColorScheme::Default,
    });
    apply_accent();
    apply_code_style();
    apply_zoom();
    // Standalone accent shades depend on light/dark; re-apply when it flips.
    thread_local!(static HOOKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
    if !HOOKED.with(|h| h.replace(true)) {
        sm.connect_dark_notify(|_| apply_accent());
    }
}

fn apply_accent() {
    let s = get();
    let dark = adw::StyleManager::default().is_dark();
    let css = ACCENTS
        .iter()
        .find(|(id, _, _)| *id == s.accent)
        .map(|(_, _, c)| {
            format!(
                ":root {{ --accent-bg-color: {}; --accent-color: {}; }}",
                c.to_rgba(),
                c.to_standalone_rgba(dark)
            )
        })
        .unwrap_or_default();
    thread_local!(static ADDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
    ACCENT_CSS.with(|p| {
        p.load_from_string(&css);
        if !ADDED.with(|a| a.replace(true))
            && let Some(d) = gtk::gdk::Display::default()
        {
            gtk::style_context_add_provider_for_display(
                &d,
                p,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
            );
        }
    });
}

thread_local! {
    static CODE_CSS: gtk::CssProvider = gtk::CssProvider::new();
    static CODE_VIEWS: RefCell<Vec<glib::WeakRef<gtk::TextView>>> = const { RefCell::new(Vec::new()) };
}

/// Mark `view` as showing code: it follows the code font, size and line spacing settings.
pub fn register_code_view(view: &impl glib::object::IsA<gtk::TextView>) {
    let view = view.as_ref();
    view.add_css_class("code-view");
    apply_spacing(view, &get());
    CODE_VIEWS.with(|v| {
        let mut v = v.borrow_mut();
        v.retain(|w| w.upgrade().is_some());
        v.push(view.downgrade());
    });
}

fn apply_spacing(view: &gtk::TextView, s: &Settings) {
    // Extra space split above and below each line; points → pixels at 96 dpi, zoomed.
    let px = s.code_font_size * 96.0 / 72.0 * s.ui_zoom.clamp(ZOOM_MIN, ZOOM_MAX);
    let extra = ((s.code_line_spacing - 1.0).max(0.0) * px / 2.0).round() as i32;
    view.set_pixels_above_lines(extra);
    view.set_pixels_below_lines(extra);
    view.set_pixels_inside_wrap(extra * 2);
}

/// Apply code font, size and line spacing to every code view.
pub fn apply_code_style() {
    let s = get();
    let family = if s.code_font_family.trim().is_empty() {
        "monospace".to_owned()
    } else {
        format!("\"{}\"", s.code_font_family.replace('"', ""))
    };
    let css = format!(
        ".code-view, .code-view text {{ font-family: {family}; font-size: {}pt; }}",
        s.code_font_size
    );
    thread_local!(static ADDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
    CODE_CSS.with(|p| {
        p.load_from_string(&css);
        if !ADDED.with(|a| a.replace(true))
            && let Some(d) = gtk::gdk::Display::default()
        {
            // Above the app stylesheet so it beats GtkTextView's `monospace` class.
            gtk::style_context_add_provider_for_display(
                &d,
                p,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2,
            );
        }
    });
    CODE_VIEWS.with(|v| {
        for w in v.borrow().iter() {
            if let Some(view) = w.upgrade() {
                apply_spacing(&view, &s);
            }
        }
    });
}

pub const ZOOM_MIN: f64 = 0.5;
pub const ZOOM_MAX: f64 = 2.5;

thread_local! {
    /// The desktop's text DPI before any zoom (GTK stores DPI × 1024).
    static BASE_DPI: std::cell::Cell<Option<i32>> = const { std::cell::Cell::new(None) };
}

/// Scale the whole interface by adjusting this app's text DPI. libadwaita's breakpoints are
/// measured in sp, so adaptive layouts follow the zoom too.
pub fn apply_zoom() {
    let Some(gs) = gtk::Settings::default() else {
        return;
    };
    let base = BASE_DPI.with(|b| {
        *b.get().get_or_insert_with(|| {
            let d = gs.gtk_xft_dpi();
            if d > 0 { d } else { 96 * 1024 }
        })
    });
    let zoom = get().ui_zoom.clamp(ZOOM_MIN, ZOOM_MAX);
    if (zoom - 1.0).abs() < 0.001 {
        // Setting the desktop's own value back is ignored by GTK; resetting hands the
        // property back to the desktop (and keeps following system text scaling).
        gs.reset_property("gtk-xft-dpi");
    } else {
        gs.set_gtk_xft_dpi((base as f64 * zoom).round() as i32);
    }
    // Line spacing is in pixels, so it must follow the zoom.
    apply_code_style();
}

/// Change the zoom by `delta` (or reset with `None`), save and apply it.
pub fn step_zoom(delta: Option<f64>) -> f64 {
    update(|s| {
        s.ui_zoom = match delta {
            Some(d) => ((s.ui_zoom + d) * 10.0).round() / 10.0,
            None => 1.0,
        }
        .clamp(ZOOM_MIN, ZOOM_MAX);
    });
    apply_zoom();
    get().ui_zoom
}
