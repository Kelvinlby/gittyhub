//! Persistent user settings, stored as JSON under `$XDG_CONFIG_HOME/gittyhub/`.

use std::cell::RefCell;
use std::path::PathBuf;

use git_backend::Location;
use gtk::glib;
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
