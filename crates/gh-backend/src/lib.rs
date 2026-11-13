//! GitHub operations through the official GitHub CLI (`gh`).
//!
//! Every call targets an explicit repository (`-R owner/name`) and runs on this machine, so
//! the same code serves local and SSH-hosted checkouts. Authentication belongs to `gh`.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Run `gh <args>` (optionally feeding stdin) and return stdout.
pub fn run(args: &[&str], stdin: Option<&str>) -> Result<String> {
    let mut cmd = Command::new("gh");
    cmd.args(args)
        .env("GH_PROMPT_DISABLED", "1")
        .env("NO_COLOR", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .context("failed to spawn gh (is the GitHub CLI installed?)")?;
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .context("no stdin")?
            .write_all(input.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!(
            "gh {}: {}",
            args.iter().take(2).copied().collect::<Vec<_>>().join(" "),
            err.trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn run_json<T: DeserializeOwned>(args: &[&str]) -> Result<T> {
    let out = run(args, None)?;
    serde_json::from_str(&out).with_context(|| format!("unexpected output from gh {}", args[0]))
}

/// `gh api <path>` decoded as JSON.
pub fn api<T: DeserializeOwned>(path: &str) -> Result<T> {
    run_json(&["api", "-H", "Accept: application/vnd.github+json", path])
}

fn api_method(method: &str, path: &str) -> Result<()> {
    run(&["api", "-X", method, path, "--silent"], None).map(drop)
}

// ---------------------------------------------------------------------------
// Shared types

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Actor {
    #[serde(default)]
    pub login: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Label {
    pub name: String,
    #[serde(default)]
    pub color: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    #[serde(default)]
    pub author: Actor,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// Account

#[derive(Debug, Clone, Default, Deserialize)]
pub struct User {
    pub login: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub avatar_url: String,
}

pub fn current_user() -> Result<User> {
    api("user")
}

/// Text of `gh auth status` (stderr on older versions is folded into the error).
pub fn auth_status() -> Result<String> {
    run(&["auth", "status"], None)
}

// ---------------------------------------------------------------------------
// Repositories

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoSummary {
    pub name_with_owner: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub is_private: bool,
    #[serde(default)]
    pub is_fork: bool,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub ssh_url: String,
    #[serde(default)]
    pub url: String,
}

pub fn repo_list(owner: Option<&str>, limit: u32) -> Result<Vec<RepoSummary>> {
    let limit = limit.to_string();
    let mut args = vec!["repo", "list"];
    if let Some(o) = owner {
        args.push(o);
    }
    args.extend([
        "-L",
        &limit,
        "--json",
        "nameWithOwner,description,isPrivate,isFork,updatedAt,sshUrl,url",
    ]);
    run_json(&args)
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedRef {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    pub name_with_owner: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub homepage_url: Option<String>,
    #[serde(default)]
    pub stargazer_count: u64,
    #[serde(default)]
    pub fork_count: u64,
    #[serde(default)]
    pub is_private: bool,
    #[serde(default)]
    pub is_fork: bool,
    #[serde(default)]
    pub is_archived: bool,
    #[serde(default)]
    pub viewer_has_starred: bool,
    #[serde(default)]
    pub default_branch_ref: Option<NamedRef>,
    #[serde(default)]
    pub primary_language: Option<NamedRef>,
    #[serde(default)]
    pub license_info: Option<NamedRef>,
    #[serde(default)]
    pub parent: Option<Value>,
}

pub fn repo_view(repo: &str) -> Result<Repo> {
    run_json(&[
        "repo",
        "view",
        repo,
        "--json",
        "nameWithOwner,description,url,homepageUrl,stargazerCount,forkCount,isPrivate,isFork,isArchived,viewerHasStarred,defaultBranchRef,primaryLanguage,licenseInfo,parent",
    ])
}

pub fn readme(repo: &str) -> Result<String> {
    run(
        &[
            "api",
            "-H",
            "Accept: application/vnd.github.raw",
            &format!("repos/{repo}/readme"),
        ],
        None,
    )
}

pub fn set_starred(repo: &str, star: bool) -> Result<()> {
    api_method(
        if star { "PUT" } else { "DELETE" },
        &format!("user/starred/{repo}"),
    )
}

pub fn fork(repo: &str) -> Result<()> {
    run(&["repo", "fork", repo, "--clone=false"], None).map(drop)
}

/// Create a GitHub repository from an existing local checkout and push it.
pub fn repo_create_from_source(
    source_dir: &str,
    name: &str,
    description: &str,
    private: bool,
) -> Result<()> {
    let mut args = vec![
        "repo",
        "create",
        name,
        "--source",
        source_dir,
        "--remote",
        "origin",
        "--push",
        if private { "--private" } else { "--public" },
    ];
    if !description.is_empty() {
        args.extend(["--description", description]);
    }
    run(&args, None).map(drop)
}

pub fn repo_edit(repo: &str, description: &str, homepage: &str) -> Result<()> {
    run(
        &[
            "repo",
            "edit",
            repo,
            "--description",
            description,
            "--homepage",
            homepage,
        ],
        None,
    )
    .map(drop)
}

pub fn set_visibility(repo: &str, private: bool) -> Result<()> {
    run(
        &[
            "repo",
            "edit",
            repo,
            "--visibility",
            if private { "private" } else { "public" },
            "--accept-visibility-change-consequences",
        ],
        None,
    )
    .map(drop)
}

pub fn set_archived(repo: &str, archive: bool) -> Result<()> {
    run(
        &[
            "repo",
            if archive { "archive" } else { "unarchive" },
            repo,
            "-y",
        ],
        None,
    )
    .map(drop)
}

// ---------------------------------------------------------------------------
// Pull requests

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub author: Actor,
    #[serde(default)]
    pub head_ref_name: String,
    #[serde(default)]
    pub base_ref_name: String,
    #[serde(default)]
    pub is_draft: bool,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub review_decision: Option<String>,
    #[serde(default)]
    pub labels: Vec<Label>,
}

/// `state` is `open`, `closed`, `merged` or `all`.
pub fn pr_list(repo: &str, state: &str, search: &str) -> Result<Vec<PullRequest>> {
    let mut args = vec![
        "pr",
        "list",
        "-R",
        repo,
        "--state",
        state,
        "-L",
        "100",
        "--json",
        "number,title,state,author,headRefName,baseRefName,isDraft,updatedAt,reviewDecision,labels",
    ];
    if !search.is_empty() {
        args.extend(["--search", search]);
    }
    run_json(&args)
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    #[serde(default)]
    pub author: Actor,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub submitted_at: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Check {
    pub name: String,
    /// `success`, `failure`, `pending`, `skipped`, `neutral`, …
    pub outcome: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestDetail {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub body: String,
    pub state: String,
    #[serde(default)]
    pub author: Actor,
    #[serde(default)]
    pub head_ref_name: String,
    #[serde(default)]
    pub base_ref_name: String,
    #[serde(default)]
    pub is_draft: bool,
    #[serde(default)]
    pub is_cross_repository: bool,
    #[serde(default)]
    pub mergeable: String,
    #[serde(default)]
    pub review_decision: Option<String>,
    #[serde(default)]
    pub comments: Vec<Comment>,
    #[serde(default)]
    pub reviews: Vec<Review>,
    #[serde(default)]
    pub status_check_rollup: Vec<Value>,
    #[serde(default)]
    pub additions: u64,
    #[serde(default)]
    pub deletions: u64,
    #[serde(default)]
    pub changed_files: u64,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub assignees: Vec<Actor>,
}

impl PullRequestDetail {
    pub fn checks(&self) -> Vec<Check> {
        self.status_check_rollup
            .iter()
            .map(|v| {
                let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
                if v.get("__typename").and_then(Value::as_str) == Some("StatusContext") {
                    Check {
                        name: s("context"),
                        outcome: s("state").to_lowercase(),
                        url: Some(s("targetUrl")).filter(|u| !u.is_empty()),
                    }
                } else {
                    let status = s("status").to_lowercase();
                    let conclusion = s("conclusion").to_lowercase();
                    Check {
                        name: s("name"),
                        outcome: if status == "completed" {
                            conclusion
                        } else {
                            "pending".into()
                        },
                        url: Some(s("detailsUrl")).filter(|u| !u.is_empty()),
                    }
                }
            })
            .collect()
    }
}

pub fn pr_view(repo: &str, number: u64) -> Result<PullRequestDetail> {
    run_json(&[
        "pr",
        "view",
        &number.to_string(),
        "-R",
        repo,
        "--json",
        "number,title,body,state,author,headRefName,baseRefName,isDraft,isCrossRepository,mergeable,reviewDecision,comments,reviews,statusCheckRollup,additions,deletions,changedFiles,url,createdAt,labels,assignees",
    ])
}

pub fn pr_diff(repo: &str, number: u64) -> Result<String> {
    run(
        &[
            "pr",
            "diff",
            &number.to_string(),
            "-R",
            repo,
            "--color=never",
        ],
        None,
    )
}

pub fn pr_create(
    repo: &str,
    head: &str,
    base: &str,
    title: &str,
    body: &str,
    draft: bool,
) -> Result<String> {
    let mut args = vec![
        "pr",
        "create",
        "-R",
        repo,
        "--head",
        head,
        "--title",
        title,
        "--body-file",
        "-",
    ];
    if !base.is_empty() {
        args.extend(["--base", base]);
    }
    if draft {
        args.push("--draft");
    }
    Ok(run(&args, Some(body))?.trim().to_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

pub fn pr_merge(repo: &str, number: u64, method: MergeMethod, delete_branch: bool) -> Result<()> {
    let n = number.to_string();
    let mut args = vec![
        "pr",
        "merge",
        &n,
        "-R",
        repo,
        match method {
            MergeMethod::Merge => "--merge",
            MergeMethod::Squash => "--squash",
            MergeMethod::Rebase => "--rebase",
        },
    ];
    if delete_branch {
        args.push("--delete-branch");
    }
    run(&args, None).map(drop)
}

pub fn pr_set_open(repo: &str, number: u64, open: bool) -> Result<()> {
    let n = number.to_string();
    run(
        &["pr", if open { "reopen" } else { "close" }, &n, "-R", repo],
        None,
    )
    .map(drop)
}

pub fn pr_set_draft(repo: &str, number: u64, draft: bool) -> Result<()> {
    let n = number.to_string();
    let mut args = vec!["pr", "ready", &n, "-R", repo];
    if draft {
        args.push("--undo");
    }
    run(&args, None).map(drop)
}

pub fn pr_comment(repo: &str, number: u64, body: &str) -> Result<()> {
    let n = number.to_string();
    run(
        &["pr", "comment", &n, "-R", repo, "--body-file", "-"],
        Some(body),
    )
    .map(drop)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewKind {
    Approve,
    RequestChanges,
    Comment,
}

pub fn pr_review(repo: &str, number: u64, kind: ReviewKind, body: &str) -> Result<()> {
    let n = number.to_string();
    let flag = match kind {
        ReviewKind::Approve => "--approve",
        ReviewKind::RequestChanges => "--request-changes",
        ReviewKind::Comment => "--comment",
    };
    let mut args = vec!["pr", "review", &n, "-R", repo, flag];
    if !body.is_empty() || kind != ReviewKind::Approve {
        args.extend(["--body-file", "-"]);
    }
    run(&args, Some(body)).map(drop)
}

pub fn pr_edit(repo: &str, number: u64, title: &str, body: &str) -> Result<()> {
    let n = number.to_string();
    run(
        &[
            "pr",
            "edit",
            &n,
            "-R",
            repo,
            "--title",
            title,
            "--body-file",
            "-",
        ],
        Some(body),
    )
    .map(drop)
}

// ---------------------------------------------------------------------------
// Issues

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub author: Actor,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub comments: Vec<Value>,
}

pub fn issue_list(repo: &str, state: &str, search: &str) -> Result<Vec<Issue>> {
    let mut args = vec![
        "issue",
        "list",
        "-R",
        repo,
        "--state",
        state,
        "-L",
        "100",
        "--json",
        "number,title,state,author,labels,updatedAt,comments",
    ];
    if !search.is_empty() {
        args.extend(["--search", search]);
    }
    run_json(&args)
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDetail {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub body: String,
    pub state: String,
    #[serde(default)]
    pub author: Actor,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub assignees: Vec<Actor>,
    #[serde(default)]
    pub comments: Vec<Comment>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub created_at: String,
}

pub fn issue_view(repo: &str, number: u64) -> Result<IssueDetail> {
    run_json(&[
        "issue",
        "view",
        &number.to_string(),
        "-R",
        repo,
        "--json",
        "number,title,body,state,author,labels,assignees,comments,url,createdAt",
    ])
}

pub fn issue_create(repo: &str, title: &str, body: &str, labels: &str) -> Result<String> {
    let mut args = vec![
        "issue",
        "create",
        "-R",
        repo,
        "--title",
        title,
        "--body-file",
        "-",
    ];
    if !labels.is_empty() {
        args.extend(["--label", labels]);
    }
    Ok(run(&args, Some(body))?.trim().to_owned())
}

pub fn issue_set_open(repo: &str, number: u64, open: bool) -> Result<()> {
    let n = number.to_string();
    run(
        &[
            "issue",
            if open { "reopen" } else { "close" },
            &n,
            "-R",
            repo,
        ],
        None,
    )
    .map(drop)
}

pub fn issue_comment(repo: &str, number: u64, body: &str) -> Result<()> {
    let n = number.to_string();
    run(
        &["issue", "comment", &n, "-R", repo, "--body-file", "-"],
        Some(body),
    )
    .map(drop)
}

pub fn issue_edit(repo: &str, number: u64, title: &str, body: &str) -> Result<()> {
    let n = number.to_string();
    run(
        &[
            "issue",
            "edit",
            &n,
            "-R",
            repo,
            "--title",
            title,
            "--body-file",
            "-",
        ],
        Some(body),
    )
    .map(drop)
}

// ---------------------------------------------------------------------------
// Actions

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub database_id: u64,
    #[serde(default)]
    pub display_title: String,
    #[serde(default)]
    pub workflow_name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: String,
    #[serde(default)]
    pub head_branch: String,
    #[serde(default)]
    pub event: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub number: u64,
}

impl Run {
    /// One word summarising the run: the conclusion once completed, else the status.
    pub fn outcome(&self) -> &str {
        if self.status == "completed" {
            &self.conclusion
        } else {
            &self.status
        }
    }
}

const RUN_FIELDS: &str =
    "databaseId,displayTitle,workflowName,status,conclusion,headBranch,event,createdAt,url,number";

pub fn run_list(repo: &str, workflow: Option<&str>) -> Result<Vec<Run>> {
    let mut args = vec!["run", "list", "-R", repo, "-L", "50", "--json", RUN_FIELDS];
    if let Some(w) = workflow {
        args.extend(["--workflow", w]);
    }
    run_json(&args)
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: String,
    #[serde(default)]
    pub number: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    #[serde(default)]
    pub database_id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: String,
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDetail {
    #[serde(flatten)]
    pub run: Run,
    #[serde(default)]
    pub jobs: Vec<Job>,
}

pub fn run_view(repo: &str, id: u64) -> Result<RunDetail> {
    let fields = format!("{RUN_FIELDS},jobs");
    run_json(&[
        "run",
        "view",
        &id.to_string(),
        "-R",
        repo,
        "--json",
        &fields,
    ])
}

pub fn run_log(repo: &str, id: u64, failed_only: bool) -> Result<String> {
    let n = id.to_string();
    let flag = if failed_only { "--log-failed" } else { "--log" };
    run(&["run", "view", &n, "-R", repo, flag], None)
}

pub fn job_log(repo: &str, job_id: u64) -> Result<String> {
    run(
        &[
            "run",
            "view",
            "-R",
            repo,
            "--job",
            &job_id.to_string(),
            "--log",
        ],
        None,
    )
}

pub fn run_rerun(repo: &str, id: u64, failed_only: bool) -> Result<()> {
    let n = id.to_string();
    let mut args = vec!["run", "rerun", &n, "-R", repo];
    if failed_only {
        args.push("--failed");
    }
    run(&args, None).map(drop)
}

pub fn run_cancel(repo: &str, id: u64) -> Result<()> {
    run(&["run", "cancel", &id.to_string(), "-R", repo], None).map(drop)
}

pub fn run_delete(repo: &str, id: u64) -> Result<()> {
    run(&["run", "delete", &id.to_string(), "-R", repo], None).map(drop)
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Workflow {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub path: String,
}

pub fn workflow_list(repo: &str) -> Result<Vec<Workflow>> {
    run_json(&[
        "workflow",
        "list",
        "-R",
        repo,
        "--all",
        "--json",
        "id,name,state,path",
    ])
}

pub fn workflow_run(repo: &str, id: u64, git_ref: &str) -> Result<()> {
    let n = id.to_string();
    let mut args = vec!["workflow", "run", &n, "-R", repo];
    if !git_ref.is_empty() {
        args.extend(["--ref", git_ref]);
    }
    run(&args, None).map(drop)
}

pub fn workflow_set_enabled(repo: &str, id: u64, enabled: bool) -> Result<()> {
    run(
        &[
            "workflow",
            if enabled { "enable" } else { "disable" },
            &id.to_string(),
            "-R",
            repo,
        ],
        None,
    )
    .map(drop)
}

// ---------------------------------------------------------------------------
// Releases

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub is_draft: bool,
    #[serde(default)]
    pub is_prerelease: bool,
    #[serde(default)]
    pub is_latest: bool,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub created_at: String,
}

pub fn release_list(repo: &str) -> Result<Vec<Release>> {
    run_json(&[
        "release",
        "list",
        "-R",
        repo,
        "-L",
        "50",
        "--json",
        "tagName,name,isDraft,isPrerelease,isLatest,publishedAt,createdAt",
    ])
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub download_count: u64,
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseDetail {
    pub tag_name: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub author: Actor,
    #[serde(default)]
    pub is_draft: bool,
    #[serde(default)]
    pub is_prerelease: bool,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

/// `gh release view <tag>` resolves the tag via `releases/tags/{tag}`, which never returns
/// drafts (or releases whose tag doesn't exist yet), so fall back to scanning the REST list.
pub fn release_view(repo: &str, tag: &str) -> Result<ReleaseDetail> {
    release_view_cli(repo, tag).or_else(|e| release_view_api(repo, tag).map_err(|_| e))
}

fn release_view_api(repo: &str, tag: &str) -> Result<ReleaseDetail> {
    #[derive(Deserialize)]
    struct RestAsset {
        name: String,
        #[serde(default)]
        size: u64,
        #[serde(default)]
        download_count: u64,
        #[serde(default)]
        browser_download_url: String,
    }
    #[derive(Deserialize)]
    struct RestRelease {
        tag_name: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        body: Option<String>,
        #[serde(default)]
        html_url: String,
        #[serde(default)]
        author: Option<Actor>,
        #[serde(default)]
        draft: bool,
        #[serde(default)]
        prerelease: bool,
        #[serde(default)]
        published_at: Option<String>,
        #[serde(default)]
        assets: Vec<RestAsset>,
    }
    let all: Vec<RestRelease> = api(&format!("repos/{repo}/releases?per_page=100"))?;
    let r = all
        .into_iter()
        .find(|r| r.tag_name == tag)
        .with_context(|| format!("release {tag} not found"))?;
    Ok(ReleaseDetail {
        tag_name: r.tag_name,
        name: r.name.unwrap_or_default(),
        body: r.body.unwrap_or_default(),
        url: r.html_url,
        author: r.author.unwrap_or_default(),
        is_draft: r.draft,
        is_prerelease: r.prerelease,
        published_at: r.published_at,
        assets: r
            .assets
            .into_iter()
            .map(|a| Asset {
                name: a.name,
                size: a.size,
                download_count: a.download_count,
                url: a.browser_download_url,
            })
            .collect(),
    })
}

fn release_view_cli(repo: &str, tag: &str) -> Result<ReleaseDetail> {
    run_json(&[
        "release",
        "view",
        tag,
        "-R",
        repo,
        "--json",
        "tagName,name,body,url,author,isDraft,isPrerelease,publishedAt,assets",
    ])
}

pub struct NewRelease<'a> {
    pub tag: &'a str,
    pub target: &'a str,
    pub title: &'a str,
    pub notes: &'a str,
    pub generate_notes: bool,
    pub draft: bool,
    pub prerelease: bool,
}

pub fn release_create(repo: &str, r: &NewRelease<'_>) -> Result<String> {
    let mut args = vec!["release", "create", r.tag, "-R", repo, "--title", r.title];
    if !r.target.is_empty() {
        args.extend(["--target", r.target]);
    }
    if r.generate_notes {
        args.push("--generate-notes");
    }
    if !r.notes.is_empty() || !r.generate_notes {
        args.extend(["--notes-file", "-"]);
    }
    if r.draft {
        args.push("--draft");
    }
    if r.prerelease {
        args.push("--prerelease");
    }
    Ok(run(&args, Some(r.notes))?.trim().to_owned())
}

pub fn release_delete(repo: &str, tag: &str, cleanup_tag: bool) -> Result<()> {
    let mut args = vec!["release", "delete", tag, "-R", repo, "-y"];
    if cleanup_tag {
        args.push("--cleanup-tag");
    }
    run(&args, None).map(drop)
}

pub fn release_publish(repo: &str, tag: &str) -> Result<()> {
    run(&["release", "edit", tag, "-R", repo, "--draft=false"], None).map(drop)
}

pub fn release_download(repo: &str, tag: &str, pattern: &str, dir: &str) -> Result<()> {
    run(
        &[
            "release",
            "download",
            tag,
            "-R",
            repo,
            "-p",
            pattern,
            "-D",
            dir,
            "--clobber",
        ],
        None,
    )
    .map(drop)
}

pub fn release_upload(repo: &str, tag: &str, file: &str) -> Result<()> {
    run(
        &["release", "upload", tag, "-R", repo, file, "--clobber"],
        None,
    )
    .map(drop)
}

// ---------------------------------------------------------------------------
// Deployments and environments (REST via `gh api`; `gh` has no subcommand for them)

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Deployment {
    pub id: u64,
    #[serde(default)]
    pub environment: String,
    #[serde(rename = "ref", default)]
    pub git_ref: String,
    #[serde(default)]
    pub sha: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub creator: Option<Actor>,
    /// Latest status state, filled in by [`deployments`].
    #[serde(skip)]
    pub state: String,
    #[serde(skip)]
    pub environment_url: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct DeploymentStatus {
    #[serde(default)]
    state: String,
    #[serde(default)]
    environment_url: String,
}

pub fn deployments(repo: &str) -> Result<Vec<Deployment>> {
    let mut list: Vec<Deployment> = api(&format!("repos/{repo}/deployments?per_page=30"))?;
    for d in &mut list {
        let st: Vec<DeploymentStatus> = api(&format!(
            "repos/{repo}/deployments/{}/statuses?per_page=1",
            d.id
        ))
        .unwrap_or_default();
        if let Some(s) = st.into_iter().next() {
            d.state = s.state;
            d.environment_url = s.environment_url;
        } else {
            d.state = "pending".into();
        }
    }
    Ok(list)
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Environment {
    pub name: String,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Default, Deserialize)]
struct Environments {
    #[serde(default)]
    environments: Vec<Environment>,
}

pub fn environments(repo: &str) -> Result<Vec<Environment>> {
    Ok(api::<Environments>(&format!("repos/{repo}/environments"))?.environments)
}

// ---------------------------------------------------------------------------
// Packages (REST; requires the `read:packages` scope)

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Package {
    pub name: String,
    #[serde(default)]
    pub package_type: String,
    #[serde(default)]
    pub visibility: String,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub version_count: u64,
    #[serde(default)]
    pub repository: Option<PackageRepo>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PackageRepo {
    #[serde(default)]
    pub full_name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PackageVersion {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub updated_at: String,
}

const PACKAGE_TYPES: [&str; 5] = ["container", "npm", "maven", "rubygems", "nuget"];

/// Owner path segment (`users/x` or `orgs/x`) for `repo`'s owner.
fn owner_path(repo: &str) -> Result<String> {
    let owner = repo.split('/').next().unwrap_or(repo);
    let v: Value = api(&format!("users/{owner}"))?;
    Ok(
        if v.get("type").and_then(Value::as_str) == Some("Organization") {
            format!("orgs/{owner}")
        } else {
            format!("users/{owner}")
        },
    )
}

/// Packages owned by the repository owner that are linked to `repo`.
pub fn packages(repo: &str) -> Result<Vec<Package>> {
    let owner = owner_path(repo)?;
    let mut out = Vec::new();
    let mut last_err = None;
    for t in PACKAGE_TYPES {
        match api::<Vec<Package>>(&format!("{owner}/packages?package_type={t}&per_page=100")) {
            Ok(list) => out.extend(list.into_iter().filter(|p| {
                p.repository
                    .as_ref()
                    .is_some_and(|r| r.full_name.eq_ignore_ascii_case(repo))
            })),
            Err(e) => last_err = Some(e),
        }
    }
    if out.is_empty()
        && let Some(e) = last_err
        && e.to_string().contains("read:packages")
    {
        return Err(e);
    }
    Ok(out)
}

pub fn package_versions(repo: &str, pkg: &Package) -> Result<Vec<PackageVersion>> {
    let owner = owner_path(repo)?;
    api(&format!(
        "{owner}/packages/{}/{}/versions?per_page=50",
        pkg.package_type, pkg.name
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_from_rollup() {
        let d: PullRequestDetail = serde_json::from_str(
            r#"{"number":1,"title":"t","state":"OPEN","statusCheckRollup":[
                {"__typename":"CheckRun","name":"ci","status":"COMPLETED","conclusion":"SUCCESS"},
                {"__typename":"CheckRun","name":"lint","status":"IN_PROGRESS","conclusion":""},
                {"__typename":"StatusContext","context":"ext","state":"FAILURE"}]}"#,
        )
        .unwrap();
        let c = d.checks();
        assert_eq!(c[0].outcome, "success");
        assert_eq!(c[1].outcome, "pending");
        assert_eq!(c[2].name, "ext");
        assert_eq!(c[2].outcome, "failure");
    }

    #[test]
    fn run_detail_flattens() {
        let d: RunDetail = serde_json::from_str(
            r#"{"databaseId":5,"status":"completed","conclusion":"failure","jobs":[{"name":"build","steps":[]}]}"#,
        )
        .unwrap();
        assert_eq!(d.run.outcome(), "failure");
        assert_eq!(d.jobs.len(), 1);
    }
}
