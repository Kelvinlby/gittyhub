//! Read-only helpers over the user's SSH configuration.

use std::path::{Path, PathBuf};

/// Concrete host aliases from `~/.ssh/config` (and files it `Include`s), without wildcards.
pub fn configured_hosts() -> Vec<String> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let ssh_dir = home.join(".ssh");
    let mut hosts = Vec::new();
    collect(&ssh_dir.join("config"), &ssh_dir, &mut hosts, 0);
    hosts.sort();
    hosts.dedup();
    hosts
}

fn collect(file: &Path, ssh_dir: &Path, hosts: &mut Vec<String>, depth: u32) {
    if depth > 4 {
        return;
    }
    let Ok(text) = std::fs::read_to_string(file) else {
        return;
    };
    for line in text.lines() {
        let line = line.trim();
        let mut parts = line.split_whitespace();
        let Some(key) = parts.next() else { continue };
        if key.eq_ignore_ascii_case("host") {
            hosts.extend(
                parts
                    .filter(|h| !h.contains(['*', '?', '!']))
                    .map(str::to_owned),
            );
        } else if key.eq_ignore_ascii_case("include") {
            for pat in parts {
                let pat = if let Some(rest) = pat.strip_prefix("~/") {
                    std::env::var("HOME")
                        .map(|h| format!("{h}/{rest}"))
                        .unwrap_or_default()
                } else if pat.starts_with('/') {
                    pat.to_owned()
                } else {
                    ssh_dir.join(pat).to_string_lossy().into_owned()
                };
                for path in expand_glob(&pat) {
                    collect(&path, ssh_dir, hosts, depth + 1);
                }
            }
        }
    }
}

/// Minimal glob: supports a `*` in the final path component only.
fn expand_glob(pat: &str) -> Vec<PathBuf> {
    let path = Path::new(pat);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    match (path.parent(), name) {
        (Some(dir), Some(name)) if name.contains('*') => {
            let (pre, post) = name.split_once('*').unwrap_or((&name, ""));
            let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .map(|n| {
                            let n = n.to_string_lossy();
                            n.starts_with(pre) && n.ends_with(post)
                        })
                        .unwrap_or(false)
                })
                .collect();
            out.sort();
            out
        }
        _ => vec![path.to_path_buf()],
    }
}
