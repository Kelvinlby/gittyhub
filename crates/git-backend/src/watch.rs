//! Which directories of a local checkout to watch for changes.
//!
//! The app puts a (non-recursive) file monitor on each directory listed here, so it can
//! refresh when another program edits files, commits, switches branches or pushes.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::Location;

/// Stop adding work-tree directories past this many (inotify watches are a shared resource).
pub const MAX_WORKTREE_DIRS: usize = 8000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchDirs {
    /// Git metadata: the git dir, the common dir (linked worktrees) and every `refs/` directory.
    pub git: Vec<PathBuf>,
    /// Work-tree directories, skipping `.git`, ignored directories and nested repositories.
    pub worktree: Vec<PathBuf>,
    /// The work tree had more than [`MAX_WORKTREE_DIRS`] directories; the rest are unwatched.
    pub truncated: bool,
}

/// Names inside git metadata directories whose changes never need a refresh.
pub fn is_noise_in_git_dir(name: &str) -> bool {
    name.ends_with(".lock")
        || matches!(
            name,
            "FETCH_HEAD"
                | "COMMIT_EDITMSG"
                | "GITTYHUB_REWORD_MSG"
                | "gc.pid"
                | "gc.log"
                | "objects"
                | "logs"
                | "hooks"
                | "info"
        )
}

/// List the directories to watch for the local checkout at `path`.
pub fn watch_dirs(path: &Path) -> Result<WatchDirs> {
    let loc = Location::Local(path.to_owned());
    let out = loc.git(&[
        "rev-parse",
        "--path-format=absolute",
        "--git-dir",
        "--git-common-dir",
    ])?;
    let mut lines = out.lines().map(PathBuf::from);
    let git_dir = lines.next().unwrap_or_else(|| path.join(".git"));
    let common = lines.next().unwrap_or_else(|| git_dir.clone());

    let mut git = vec![git_dir.clone()];
    if common != git_dir {
        git.push(common.clone());
    }
    walk(&common.join("refs"), &HashSet::new(), usize::MAX, &mut git);

    let mut dirs = WatchDirs {
        git,
        ..Default::default()
    };
    // Bare repositories have no work tree.
    let Ok(top) = loc.git(&["rev-parse", "--show-toplevel"]) else {
        return Ok(dirs);
    };
    let top = PathBuf::from(top.trim());
    let ignored: HashSet<PathBuf> = Location::Local(top.clone())
        .git(&[
            "ls-files",
            "-z",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
        ])?
        .split('\0')
        .filter_map(|p| p.strip_suffix('/'))
        .map(|p| top.join(p))
        .collect();
    dirs.truncated = !walk(&top, &ignored, MAX_WORKTREE_DIRS, &mut dirs.worktree);
    Ok(dirs)
}

/// Breadth-first list of `root` and its sub-directories (not following symlinks), skipping
/// `.git`, anything in `skip` and nested repositories. Returns false if it stopped at `max`.
fn walk(root: &Path, skip: &HashSet<PathBuf>, max: usize, out: &mut Vec<PathBuf>) -> bool {
    if !root.is_dir() {
        return true;
    }
    let start = out.len();
    out.push(root.to_owned());
    let mut i = start;
    while i < out.len() {
        let dir = out[i].clone();
        i += 1;
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            if !e.file_type().is_ok_and(|t| t.is_dir()) || e.file_name() == ".git" {
                continue;
            }
            let p = e.path();
            if skip.contains(&p) || p.join(".git").exists() {
                continue;
            }
            if out.len() - start >= max {
                return false;
            }
            out.push(p);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn lists_git_and_worktree_dirs_without_ignored() {
        let tmp = std::env::temp_dir().join(format!("gittyhub-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("src/inner")).unwrap();
        std::fs::create_dir_all(tmp.join("target/debug")).unwrap();
        std::fs::write(tmp.join(".gitignore"), "target/\n").unwrap();
        let ok = Command::new("git")
            .args(["init", "-q"])
            .current_dir(&tmp)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let tmp = tmp.canonicalize().unwrap();
        let d = watch_dirs(&tmp).unwrap();
        assert!(d.git.contains(&tmp.join(".git")));
        assert!(d.git.contains(&tmp.join(".git/refs/heads")));
        assert!(d.worktree.contains(&tmp));
        assert!(d.worktree.contains(&tmp.join("src/inner")));
        assert!(!d.worktree.iter().any(|p| p.starts_with(tmp.join("target"))));
        assert!(!d.worktree.iter().any(|p| p.starts_with(tmp.join(".git"))));
        assert!(!d.truncated);
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn noise_names() {
        assert!(is_noise_in_git_dir("index.lock"));
        assert!(is_noise_in_git_dir("FETCH_HEAD"));
        assert!(!is_noise_in_git_dir("index"));
        assert!(!is_noise_in_git_dir("HEAD"));
        assert!(!is_noise_in_git_dir("main"));
    }
}
