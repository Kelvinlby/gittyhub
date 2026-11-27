//! Hand GNOME's proxy settings to the `git` and `gh` child processes.
//!
//! GNOME keeps its proxy in gsettings (`org.gnome.system.proxy`), which neither
//! CLI reads. When the environment has no proxy of its own, we turn those settings
//! into the usual `*_proxy` variables so every child inherits them.

use gtk::gio;
use gtk::prelude::*;

const ENV_VARS: &[&str] = &[
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
];

/// Must run first thing in `main`, before any other thread exists.
pub fn apply_gnome_proxy() {
    if ENV_VARS.iter().any(|v| std::env::var_os(v).is_some()) {
        return; // An explicit environment proxy wins.
    }
    let vars = gnome_proxy_vars();
    for (k, v) in vars {
        // SAFETY: called at the very start of `main`, while single-threaded.
        unsafe {
            std::env::set_var(k, &v);
            std::env::set_var(k.to_uppercase(), &v);
        }
    }
}

fn gnome_proxy_vars() -> Vec<(&'static str, String)> {
    let Some(source) = gio::SettingsSchemaSource::default() else {
        return Vec::new();
    };
    if source.lookup("org.gnome.system.proxy", true).is_none() {
        return Vec::new();
    }
    let s = gio::Settings::new("org.gnome.system.proxy");
    match s.string("mode").as_str() {
        "manual" => manual(&s),
        "auto" => auto(),
        _ => Vec::new(),
    }
}

fn manual(s: &gio::Settings) -> Vec<(&'static str, String)> {
    let child = |name: &str, scheme: &str| {
        let c = s.child(name);
        let host = c.string("host");
        let port = c.int("port");
        (!host.is_empty() && port > 0).then(|| format!("{scheme}://{}:{port}", bracket(&host)))
    };
    let mut out = Vec::new();
    let http = child("http", "http");
    if let Some(p) = &http {
        out.push(("http_proxy", p.clone()));
    }
    // GNOME leaves the HTTPS proxy empty when "use the same proxy" is meant.
    if let Some(p) = child("https", "http").or(http) {
        out.push(("https_proxy", p));
    }
    if let Some(p) = child("socks", "socks5h") {
        out.push(("all_proxy", p));
    }
    if !out.is_empty() {
        let ignore = s.strv("ignore-hosts");
        if !ignore.is_empty() {
            let list: Vec<&str> = ignore.iter().map(|h| h.as_str()).collect();
            out.push(("no_proxy", list.join(",")));
        }
    }
    out
}

/// Automatic (PAC) mode: ask GIO's resolver which proxy GitHub goes through.
fn auto() -> Vec<(&'static str, String)> {
    let resolver = gio::ProxyResolver::default();
    let Ok(proxies) = resolver.lookup("https://github.com", gio::Cancellable::NONE) else {
        return Vec::new();
    };
    match proxies.first().map(|p| p.to_string()) {
        Some(p) if p != "direct://" => {
            vec![("http_proxy", p.clone()), ("https_proxy", p)]
        }
        _ => Vec::new(),
    }
}

fn bracket(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_owned()
    }
}
