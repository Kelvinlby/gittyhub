//! Live refresh for local checkouts: file monitors on the git metadata and the work tree, so
//! commits, branch switches and edits made by other programs show up without refocusing.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gio, glib};

use crate::util;

/// Wait this long after the last event before refreshing…
const QUIET: Duration = Duration::from_millis(350);
/// …but never longer than this after the first one (a busy build or agent keeps writing).
const MAX_DELAY: Duration = Duration::from_millis(1500);

pub struct RepoWatch {
    root: PathBuf,
    monitors: RefCell<HashMap<PathBuf, gio::FileMonitor>>,
    git_dirs: RefCell<HashSet<PathBuf>>,
    on_change: Box<dyn Fn()>,
    first_event: Cell<Option<Instant>>,
    pending: RefCell<Option<glib::SourceId>>,
    scanning: Cell<bool>,
    rescan_again: Cell<bool>,
}

impl RepoWatch {
    /// Watch the local checkout at `root`, calling `on_change` (debounced) when it changes.
    pub fn new(root: PathBuf, on_change: impl Fn() + 'static) -> Rc<Self> {
        let w = Rc::new(Self {
            root,
            monitors: RefCell::new(HashMap::new()),
            git_dirs: RefCell::new(HashSet::new()),
            on_change: Box::new(on_change),
            first_event: Cell::new(None),
            pending: RefCell::new(None),
            scanning: Cell::new(false),
            rescan_again: Cell::new(false),
        });
        w.rescan();
        w
    }

    /// Re-list the directories to watch (in the background) and update the monitors.
    fn rescan(self: &Rc<Self>) {
        if self.scanning.replace(true) {
            self.rescan_again.set(true);
            return;
        }
        let root = self.root.clone();
        let weak = Rc::downgrade(self);
        util::spawn(
            move || git_backend::watch::watch_dirs(&root),
            move |r| {
                let Some(w) = weak.upgrade() else { return };
                w.scanning.set(false);
                match r {
                    Ok(dirs) => w.apply(dirs),
                    Err(e) => eprintln!("gittyhub: not watching {}: {e:#}", w.root.display()),
                }
                if w.rescan_again.replace(false) {
                    w.rescan();
                }
            },
        );
    }

    fn apply(self: &Rc<Self>, dirs: git_backend::watch::WatchDirs) {
        if dirs.truncated {
            eprintln!(
                "gittyhub: {} has over {} directories; watching only part of it",
                self.root.display(),
                git_backend::watch::MAX_WORKTREE_DIRS
            );
        }
        let wanted: HashSet<PathBuf> = dirs.git.iter().chain(&dirs.worktree).cloned().collect();
        *self.git_dirs.borrow_mut() = dirs.git.into_iter().collect();
        let mut monitors = self.monitors.borrow_mut();
        monitors.retain(|p, m| {
            let keep = wanted.contains(p);
            if !keep {
                m.cancel();
            }
            keep
        });
        for dir in wanted {
            if monitors.contains_key(&dir) {
                continue;
            }
            let file = gio::File::for_path(&dir);
            let Ok(m) =
                file.monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
            else {
                continue;
            };
            let weak = Rc::downgrade(self);
            let d = dir.clone();
            m.connect_changed(move |_, file, other, event| {
                if let Some(w) = weak.upgrade() {
                    w.on_event(&d, file, other, event);
                }
            });
            monitors.insert(dir, m);
        }
    }

    fn on_event(
        self: &Rc<Self>,
        dir: &Path,
        file: &gio::File,
        other: Option<&gio::File>,
        event: gio::FileMonitorEvent,
    ) {
        use gio::FileMonitorEvent as E;
        if matches!(
            event,
            E::AttributeChanged | E::PreUnmount | E::Unmounted | E::Changed
        ) {
            // `Changed` is always followed by `ChangesDoneHint`; reacting to the hint is enough.
            return;
        }
        let in_git = self.git_dirs.borrow().contains(dir);
        let name = |f: &gio::File| f.basename().map(|b| b.to_string_lossy().into_owned());
        if in_git
            && name(file).is_some_and(|n| git_backend::watch::is_noise_in_git_dir(&n))
            && other
                .and_then(name)
                .is_none_or(|n| git_backend::watch::is_noise_in_git_dir(&n))
        {
            return;
        }
        // A directory appeared, vanished or moved: the set of watched directories changed.
        let target = match event {
            E::Renamed => other.unwrap_or(file),
            _ => file,
        };
        let is_dir = || {
            target.query_file_type(
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                gio::Cancellable::NONE,
            ) == gio::FileType::Directory
        };
        let watched = |f: &gio::File| {
            f.path()
                .is_some_and(|p| self.monitors.borrow().contains_key(&p))
        };
        let structural = match event {
            E::Created | E::MovedIn => is_dir(),
            E::Deleted | E::MovedOut => watched(file),
            E::Renamed => watched(file) || is_dir(),
            _ => false,
        };
        if structural {
            self.rescan();
        }
        self.schedule();
    }

    fn schedule(self: &Rc<Self>) {
        let now = Instant::now();
        let first = *self.first_event.get().get_or_insert(now);
        self.first_event.set(Some(first));
        if let Some(id) = self.pending.borrow_mut().take() {
            id.remove();
        }
        let delay = QUIET.min(MAX_DELAY.saturating_sub(now - first));
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(delay, move || {
            let Some(w) = weak.upgrade() else { return };
            w.pending.borrow_mut().take();
            w.first_event.set(None);
            (w.on_change)();
        });
        *self.pending.borrow_mut() = Some(id);
    }
}

impl Drop for RepoWatch {
    fn drop(&mut self) {
        for m in self.monitors.get_mut().values() {
            m.cancel();
        }
        if let Some(id) = self.pending.get_mut().take() {
            id.remove();
        }
    }
}
