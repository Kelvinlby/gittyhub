//! Thin wrapper around the `git` CLI.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Run `git <args>` in `repo` and return stdout.
pub fn run(repo: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .context("failed to spawn git")?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8(out.stdout)?)
}

/// Current branch name.
pub fn current_branch(repo: &Path) -> Result<String> {
    Ok(run(repo, &["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_owned())
}

/// `git status --porcelain=v1` lines.
pub fn status(repo: &Path) -> Result<Vec<String>> {
    Ok(run(repo, &["status", "--porcelain=v1"])?
        .lines()
        .map(str::to_owned)
        .collect())
}
