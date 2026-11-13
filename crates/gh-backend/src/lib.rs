//! Thin wrapper around the official GitHub CLI (`gh`).

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;

/// Run `gh <args>` in `repo` and deserialize its JSON stdout.
pub fn run_json<T: DeserializeOwned>(repo: &Path, args: &[&str]) -> Result<T> {
    let out = Command::new("gh")
        .current_dir(repo)
        .args(args)
        .output()
        .context("failed to spawn gh (is the GitHub CLI installed?)")?;
    if !out.status.success() {
        bail!(
            "gh {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(serde_json::from_slice(&out.stdout)?)
}

#[derive(Debug, Clone, Deserialize)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub state: String,
}

/// Open pull requests for the repo at `repo`.
pub fn pr_list(repo: &Path) -> Result<Vec<PullRequest>> {
    run_json(repo, &["pr", "list", "--json", "number,title,state"])
}
