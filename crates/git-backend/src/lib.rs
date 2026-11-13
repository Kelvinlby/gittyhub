//! Repository operations through the `git` CLI, locally or on an SSH host.
//!
//! Every function takes a [`Location`] and is blocking; callers run them off the UI thread.

mod location;
pub mod ssh;

pub use location::{Location, shell_quote, ssh_command};

use anyhow::{Result, bail};

const US: char = '\u{1f}';
const RS: char = '\u{1e}';

// ---------------------------------------------------------------------------
// Status

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BranchInfo {
    pub head: String,
    pub oid: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub orig_path: Option<String>,
    /// Index (staged) status letter, `.` when unchanged.
    pub index: char,
    /// Work-tree status letter, `.` when unchanged; `?` for untracked.
    pub worktree: char,
    pub conflicted: bool,
}

impl FileChange {
    pub fn staged(&self) -> bool {
        !self.conflicted && self.index != '.' && self.index != '?'
    }
    pub fn unstaged(&self) -> bool {
        !self.conflicted && self.worktree != '.'
    }
    pub fn untracked(&self) -> bool {
        self.worktree == '?'
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoState {
    Clean,
    Merging,
    Rebasing,
    CherryPicking,
    Reverting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub branch: BranchInfo,
    pub files: Vec<FileChange>,
    pub state: RepoState,
}

pub fn parse_status(raw: &str) -> (BranchInfo, Vec<FileChange>) {
    let mut branch = BranchInfo::default();
    let mut files = Vec::new();
    let mut entries = raw.split('\0');
    while let Some(e) = entries.next() {
        if let Some(rest) = e.strip_prefix("# ") {
            let (k, v) = rest.split_once(' ').unwrap_or((rest, ""));
            match k {
                "branch.oid" => branch.oid = v.to_owned(),
                "branch.head" => branch.head = v.to_owned(),
                "branch.upstream" => branch.upstream = Some(v.to_owned()),
                "branch.ab" => {
                    for part in v.split(' ') {
                        if let Some(n) = part.strip_prefix('+') {
                            branch.ahead = n.parse().unwrap_or(0);
                        } else if let Some(n) = part.strip_prefix('-') {
                            branch.behind = n.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        let mut chars = e.chars();
        match chars.next() {
            Some('1') => {
                let f: Vec<&str> = e.splitn(9, ' ').collect();
                if f.len() == 9 {
                    let xy: Vec<char> = f[1].chars().collect();
                    files.push(FileChange {
                        path: f[8].to_owned(),
                        orig_path: None,
                        index: xy[0],
                        worktree: xy[1],
                        conflicted: false,
                    });
                }
            }
            Some('2') => {
                let f: Vec<&str> = e.splitn(10, ' ').collect();
                if f.len() == 10 {
                    let xy: Vec<char> = f[1].chars().collect();
                    files.push(FileChange {
                        path: f[9].to_owned(),
                        orig_path: entries.next().map(str::to_owned),
                        index: xy[0],
                        worktree: xy[1],
                        conflicted: false,
                    });
                }
            }
            Some('u') => {
                let f: Vec<&str> = e.splitn(11, ' ').collect();
                if f.len() == 11 {
                    let xy: Vec<char> = f[1].chars().collect();
                    files.push(FileChange {
                        path: f[10].to_owned(),
                        orig_path: None,
                        index: xy[0],
                        worktree: xy[1],
                        conflicted: true,
                    });
                }
            }
            Some('?') => files.push(FileChange {
                path: e[2..].to_owned(),
                orig_path: None,
                index: '.',
                worktree: '?',
                conflicted: false,
            }),
            _ => {}
        }
    }
    (branch, files)
}

pub fn status(loc: &Location) -> Result<Status> {
    let raw = loc.git(&[
        "status",
        "--porcelain=v2",
        "--branch",
        "-z",
        "--untracked-files=all",
    ])?;
    let (branch, files) = parse_status(&raw);
    Ok(Status {
        branch,
        files,
        state: repo_state(loc),
    })
}

pub fn repo_state(loc: &Location) -> RepoState {
    let has = |r: &str| loc.git_ok(&["rev-parse", "-q", "--verify", r]);
    // `REBASE_HEAD` is often left behind after a rebase finishes, so look for the state
    // directories `git rebase` itself checks (`rebase-apply` without `applying` is `git am`).
    // Checked first: a conflicting merge inside `rebase -r` also sets MERGE_HEAD.
    if rebase_in_progress(loc) {
        RepoState::Rebasing
    } else if has("MERGE_HEAD") {
        RepoState::Merging
    } else if has("CHERRY_PICK_HEAD") {
        RepoState::CherryPicking
    } else if has("REVERT_HEAD") {
        RepoState::Reverting
    } else {
        RepoState::Clean
    }
}

fn rebase_in_progress(loc: &Location) -> bool {
    loc.shell(
        r#"m=$(git rev-parse --git-path rebase-merge) && a=$(git rev-parse --git-path rebase-apply) && { test -d "$m" || { test -d "$a" && ! test -f "$a/applying"; }; }"#,
        None,
    )
    .is_ok()
}

/// Check that `loc` is a work tree and return its top-level path.
pub fn toplevel(loc: &Location) -> Result<String> {
    Ok(loc
        .git(&["rev-parse", "--show-toplevel"])?
        .trim()
        .to_owned())
}

pub fn current_branch(loc: &Location) -> Result<String> {
    Ok(loc
        .git(&["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_owned())
}

// ---------------------------------------------------------------------------
// Diffs and staging

/// Diff of one file. `staged` selects index vs HEAD; untracked files diff against nothing.
pub fn file_diff(loc: &Location, change: &FileChange, staged: bool) -> Result<String> {
    if change.untracked() {
        return loc.git_lenient(&[
            "diff",
            "--no-color",
            "--no-index",
            "--",
            "/dev/null",
            &change.path,
        ]);
    }
    let mut args = vec!["diff", "--no-color", "--no-ext-diff"];
    if staged {
        args.push("--cached");
    }
    args.extend(["--", &change.path]);
    loc.git(&args)
}

pub fn stage(loc: &Location, paths: &[&str]) -> Result<()> {
    let mut args = vec!["add", "-A", "--"];
    args.extend(paths);
    loc.git(&args).map(drop)
}

pub fn stage_all(loc: &Location) -> Result<()> {
    loc.git(&["add", "-A"]).map(drop)
}

pub fn unstage(loc: &Location, paths: &[&str]) -> Result<()> {
    let mut args = vec!["restore", "--staged", "--"];
    args.extend(paths);
    if loc.git(&args).is_ok() {
        return Ok(());
    }
    // No HEAD yet (fresh repository).
    let mut args = vec!["rm", "--cached", "-r", "-q", "--"];
    args.extend(paths);
    loc.git(&args).map(drop)
}

pub fn unstage_all(loc: &Location) -> Result<()> {
    unstage(loc, &["."])
}

/// Throw away work-tree changes; untracked files are deleted.
pub fn discard(loc: &Location, change: &FileChange) -> Result<()> {
    if change.untracked() {
        loc.git(&["clean", "-f", "-q", "--", &change.path])
            .map(drop)
    } else {
        loc.git(&["restore", "--worktree", "--", &change.path])
            .map(drop)
    }
}

/// How a single hunk patch should be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkAction {
    Stage,
    Unstage,
    Discard,
}

pub fn apply_hunk(loc: &Location, patch: &str, action: HunkAction) -> Result<()> {
    let args: &[&str] = match action {
        HunkAction::Stage => &["apply", "--cached", "--whitespace=nowarn", "-"],
        HunkAction::Unstage => &["apply", "--cached", "-R", "--whitespace=nowarn", "-"],
        HunkAction::Discard => &["apply", "-R", "--whitespace=nowarn", "-"],
    };
    loc.git_in(args, Some(patch.as_bytes())).map(drop)
}

/// One file in a unified diff, split into its header and hunks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffFile {
    pub header: String,
    pub path: String,
    pub hunks: Vec<String>,
    pub binary: bool,
}

impl DiffFile {
    /// A patch containing only hunk `i`, suitable for `git apply`.
    pub fn hunk_patch(&self, i: usize) -> String {
        format!("{}{}", self.header, self.hunks[i])
    }
}

pub fn parse_diff(diff: &str) -> Vec<DiffFile> {
    let mut files: Vec<DiffFile> = Vec::new();
    let mut in_hunk = false;
    for line in diff.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            let path = line
                .trim_end()
                .rsplit_once(" b/")
                .map(|(_, p)| p.to_owned())
                .unwrap_or_default();
            files.push(DiffFile {
                header: line.to_owned(),
                path,
                ..Default::default()
            });
            in_hunk = false;
            continue;
        }
        let Some(file) = files.last_mut() else {
            continue;
        };
        if line.starts_with("@@") {
            file.hunks.push(line.to_owned());
            in_hunk = true;
        } else if in_hunk {
            if let Some(h) = file.hunks.last_mut() {
                h.push_str(line);
            }
        } else {
            if line.starts_with("Binary files") {
                file.binary = true;
            }
            if let Some(p) = line.strip_prefix("+++ b/") {
                file.path = p.trim_end().to_owned();
            }
            file.header.push_str(line);
        }
    }
    files
}

/// Build a patch from selected `+`/`-` lines of `file`.
///
/// `selected` holds `(hunk index, line index within the hunk body)` pairs; the hunk's `@@`
/// line is not counted. With `reverse`, the patch is meant for `git apply -R` against the
/// *new* side (e.g. discarding lines from the work tree); otherwise for forward application
/// against the old side (e.g. staging). Apply with `--recount`.
pub fn partial_patch(
    file: &DiffFile,
    selected: &[(usize, usize)],
    reverse: bool,
) -> Option<String> {
    let mut out = file.header.clone();
    let mut any = false;
    for (h, hunk) in file.hunks.iter().enumerate() {
        let sel: Vec<usize> = selected
            .iter()
            .filter(|(hh, _)| *hh == h)
            .map(|(_, l)| *l)
            .collect();
        if sel.is_empty() {
            continue;
        }
        let mut lines = hunk.split_inclusive('\n');
        let Some(head) = lines.next() else { continue };
        let mut body = String::new();
        let mut changed = false;
        let mut last_kept = true;
        for (i, line) in lines.enumerate() {
            let picked = sel.contains(&i);
            match line.as_bytes().first() {
                Some(b'+') | Some(b'-') => {
                    let plus = line.starts_with('+');
                    // The side that must stay intact becomes context; the other is dropped.
                    let keep_side = if reverse { plus } else { !plus };
                    if picked {
                        body.push_str(line);
                        changed = true;
                        last_kept = true;
                    } else if keep_side {
                        body.push(' ');
                        body.push_str(&line[1..]);
                        last_kept = true;
                    } else {
                        last_kept = false;
                    }
                }
                Some(b'\\') => {
                    if last_kept {
                        body.push_str(line);
                    }
                }
                _ => {
                    body.push_str(line);
                    last_kept = true;
                }
            }
        }
        if changed {
            any = true;
            out.push_str(head);
            out.push_str(&body);
        }
    }
    any.then_some(out)
}

/// Expand selected lines to the whole contiguous `+`/`-` runs ("changes") containing them,
/// so discarding a modified line restores the original instead of just deleting the new text.
pub fn expand_to_changes(file: &DiffFile, selected: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for &(h, l) in selected {
        let Some(hunk) = file.hunks.get(h) else {
            continue;
        };
        let kinds: Vec<u8> = hunk
            .split_inclusive('\n')
            .skip(1)
            .map(|x| x.as_bytes().first().copied().unwrap_or(b' '))
            .collect();
        let is_change = |i: usize| matches!(kinds.get(i), Some(b'+') | Some(b'-'));
        if !is_change(l) {
            continue;
        }
        let mut a = l;
        while a > 0 && (is_change(a - 1) || kinds.get(a - 1) == Some(&b'\\')) {
            a -= 1;
        }
        let mut b = l;
        while is_change(b + 1) || kinds.get(b + 1) == Some(&b'\\') {
            b += 1;
        }
        for i in a..=b {
            if is_change(i) && !out.contains(&(h, i)) {
                out.push((h, i));
            }
        }
    }
    out
}

/// Discard the given lines (of a `HEAD`↔work-tree diff) from the work tree.
pub fn discard_lines(loc: &Location, file: &DiffFile, selected: &[(usize, usize)]) -> Result<()> {
    let Some(patch) = partial_patch(file, selected, true) else {
        return Ok(());
    };
    loc.git_in(
        &["apply", "-R", "--recount", "--whitespace=nowarn", "-"],
        Some(patch.as_bytes()),
    )
    .map(drop)
}

/// Diff of a work-tree file against `HEAD` (staged and unstaged changes together).
pub fn worktree_diff(loc: &Location, change: &FileChange) -> Result<String> {
    if change.untracked() {
        return file_diff(loc, change, false);
    }
    if !has_head(loc) {
        return loc.git(&["diff", "--no-color", "--cached", "--", &change.path]);
    }
    loc.git(&[
        "diff",
        "--no-color",
        "--no-ext-diff",
        "-M",
        "HEAD",
        "--",
        &change.path,
    ])
}

pub fn has_head(loc: &Location) -> bool {
    loc.git_ok(&["rev-parse", "-q", "--verify", "HEAD"])
}

/// Throw away every change to a file: staged, unstaged, or the file itself if it is new.
pub fn discard_file(loc: &Location, change: &FileChange) -> Result<()> {
    if change.untracked() {
        return loc
            .git(&["clean", "-f", "-q", "--", &change.path])
            .map(drop);
    }
    if change.index == 'A' || !has_head(loc) {
        return loc.git(&["rm", "-f", "-q", "--", &change.path]).map(drop);
    }
    if let Some(orig) = &change.orig_path {
        loc.git(&["rm", "-f", "-q", "--", &change.path])?;
        return loc
            .git(&[
                "restore",
                "--source=HEAD",
                "--staged",
                "--worktree",
                "--",
                orig,
            ])
            .map(drop);
    }
    loc.git(&[
        "restore",
        "--source=HEAD",
        "--staged",
        "--worktree",
        "--",
        &change.path,
    ])
    .map(drop)
}

/// Commit exactly `include` (paths of changed files); other changes stay uncommitted.
pub fn commit_paths(
    loc: &Location,
    include: &[&str],
    exclude: &[&str],
    message: &str,
    amend: bool,
    signoff: &SignOff,
) -> Result<()> {
    if !include.is_empty() {
        let mut args = vec!["add", "-A", "--"];
        args.extend(include);
        loc.git(&args)?;
    }
    if !exclude.is_empty() {
        if has_head(loc) {
            let mut args = vec!["reset", "-q", "--"];
            args.extend(exclude);
            loc.git(&args)?;
        } else {
            let mut args = vec!["rm", "--cached", "-r", "-q", "--ignore-unmatch", "--"];
            args.extend(exclude);
            loc.git(&args)?;
        }
    }
    commit(loc, message, amend, signoff)
}

// ---------------------------------------------------------------------------
// Commits

/// Whether and how to add a `Signed-off-by` trailer to a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignOff {
    None,
    /// `git commit --signoff`: the committer's `user.name <user.email>`.
    Identity,
    /// A custom value, e.g. `Jane Doe <jane@example.com>`.
    As(String),
}

impl SignOff {
    pub fn new(enabled: bool, custom: &str) -> SignOff {
        match (enabled, custom.trim()) {
            (false, _) => SignOff::None,
            (true, "") => SignOff::Identity,
            (true, c) => SignOff::As(c.to_owned()),
        }
    }
}

pub fn commit(loc: &Location, message: &str, amend: bool, signoff: &SignOff) -> Result<()> {
    let mut args = vec!["commit", "-F", "-"];
    if amend {
        args.push("--amend");
    }
    let trailer;
    match signoff {
        SignOff::None => {}
        SignOff::Identity => args.push("--signoff"),
        SignOff::As(who) => {
            trailer = format!("Signed-off-by: {who}");
            args.extend(["--trailer", &trailer]);
        }
    }
    loc.git_in(&args, Some(message.as_bytes())).map(drop)
}

/// The `Signed-off-by` value git would use by default here (`user.name <user.email>`).
pub fn signoff_identity(loc: &Location) -> Option<String> {
    let name = config_get(loc, "user.name")?;
    let email = config_get(loc, "user.email")?;
    Some(format!("{name} <{email}>"))
}

/// Message of the HEAD commit (for amending).
pub fn head_message(loc: &Location) -> Result<String> {
    loc.git(&["log", "-1", "--format=%B"])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub oid: String,
    pub short: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub refs: Vec<String>,
    pub subject: String,
}

const LOG_FORMAT: &str = "--format=%H%x1f%h%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%D%x1f%s%x1e";

fn parse_log(raw: &str) -> Vec<Commit> {
    raw.split(RS)
        .filter_map(|rec| {
            let f: Vec<&str> = rec.trim_start_matches('\n').split(US).collect();
            (f.len() == 8).then(|| Commit {
                oid: f[0].to_owned(),
                short: f[1].to_owned(),
                parents: f[2].split_whitespace().map(str::to_owned).collect(),
                author: f[3].to_owned(),
                email: f[4].to_owned(),
                time: f[5].parse().unwrap_or(0),
                refs: f[6]
                    .split(", ")
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect(),
                subject: f[7].to_owned(),
            })
        })
        .collect()
}

#[derive(Debug, Clone, Default)]
pub struct LogQuery {
    pub rev: Option<String>,
    pub all: bool,
    pub path: Option<String>,
    pub search: Option<String>,
    pub skip: usize,
    pub limit: usize,
}

pub fn log(loc: &Location, q: &LogQuery) -> Result<Vec<Commit>> {
    let n = format!("-n{}", q.limit.max(1));
    let skip = format!("--skip={}", q.skip);
    let mut args = vec!["log", LOG_FORMAT, &n, &skip, "--topo-order"];
    let grep;
    if let Some(s) = q.search.as_deref().filter(|s| !s.is_empty()) {
        grep = format!("--grep={s}");
        args.extend(["-i", &grep]);
    }
    if q.all {
        args.push("--all");
    } else if let Some(r) = &q.rev {
        args.push(r);
    }
    args.push("--");
    if let Some(p) = &q.path {
        args.push(p);
    }
    match loc.git(&args) {
        Ok(raw) => Ok(parse_log(&raw)),
        // Empty repository: no commits yet.
        Err(e) if e.to_string().contains("does not have any commits") => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitDetail {
    pub commit: Commit,
    pub committer: String,
    pub body: String,
    pub patch: String,
}

pub fn show(loc: &Location, oid: &str) -> Result<CommitDetail> {
    let meta = loc.git(&[
        "show",
        "-s",
        "--format=%H%x1f%h%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%D%x1f%s%x1e%cn%x1f%b",
        oid,
    ])?;
    let (head, rest) = meta.split_once(RS).unwrap_or((&meta, ""));
    let commit = parse_log(&format!("{head}{RS}"))
        .pop()
        .ok_or_else(|| anyhow::anyhow!("cannot parse commit {oid}"))?;
    let (committer, body) = rest.split_once(US).unwrap_or((rest, ""));
    let patch = loc.git(&[
        "show",
        "--format=",
        "--no-color",
        "--stat=200",
        "--patch",
        "-m",
        "--first-parent",
        oid,
    ])?;
    Ok(CommitDetail {
        commit,
        committer: committer.to_owned(),
        body: body.trim().to_owned(),
        patch,
    })
}

/// A file touched by a commit (compared with its first parent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFile {
    pub status: char,
    pub path: String,
    pub orig_path: Option<String>,
}

pub fn commit_files(loc: &Location, oid: &str) -> Result<Vec<CommitFile>> {
    let raw = loc.git(&[
        "diff-tree",
        "-r",
        "-M",
        "-z",
        "--name-status",
        "--root",
        "-m",
        "--first-parent",
        "--no-commit-id",
        oid,
    ])?;
    let mut parts = raw.split('\0').filter(|s| !s.is_empty());
    let mut out = Vec::new();
    while let Some(st) = parts.next() {
        let status = st.chars().next().unwrap_or('M');
        if status == 'R' || status == 'C' {
            let (Some(old), Some(new)) = (parts.next(), parts.next()) else {
                break;
            };
            out.push(CommitFile {
                status,
                path: new.into(),
                orig_path: Some(old.into()),
            });
        } else if let Some(p) = parts.next() {
            out.push(CommitFile {
                status,
                path: p.into(),
                orig_path: None,
            });
        }
    }
    Ok(out)
}

pub fn commit_file_diff(loc: &Location, oid: &str, file: &CommitFile) -> Result<String> {
    let mut args = vec![
        "show",
        "--format=",
        "--no-color",
        "-M",
        "-m",
        "--first-parent",
        oid,
        "--",
    ];
    args.push(&file.path);
    if let Some(o) = &file.orig_path {
        args.push(o);
    }
    loc.git(&args)
}

/// Commits reachable from `HEAD` that no remote-tracking branch contains.
pub fn unpushed(loc: &Location) -> Result<std::collections::HashSet<String>> {
    if !has_head(loc) {
        return Ok(Default::default());
    }
    Ok(loc
        .git(&["rev-list", "-n1000", "HEAD", "--not", "--remotes"])?
        .lines()
        .map(str::to_owned)
        .collect())
}

/// All work-tree files (tracked and untracked, not ignored).
pub fn worktree_files(loc: &Location) -> Result<Vec<String>> {
    Ok(loc
        .git(&[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])?
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect())
}

/// All files in the tree of `rev`.
pub fn tree_files(loc: &Location, rev: &str) -> Result<Vec<String>> {
    Ok(loc
        .git(&["ls-tree", "-r", "-z", "--name-only", rev])?
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect())
}

/// How [`rewrite`] changes one commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rewrite {
    /// Fold the commit into its parent, combining messages.
    Squash,
    /// Fold the commit into its parent, keeping the parent's message.
    Fixup,
    /// Remove the commit and its changes.
    Drop,
    /// Replace the commit message.
    Reword(String),
}

/// Rewrite history at `oid` with a scripted interactive rebase (autostashing local changes).
pub fn rewrite(loc: &Location, oid: &str, how: Rewrite) -> Result<()> {
    let is_head = loc.git(&["rev-parse", "HEAD"])?.trim() == loc.git(&["rev-parse", oid])?.trim();
    if let Rewrite::Reword(msg) = &how
        && is_head
    {
        return loc
            .git_in(
                &["commit", "--amend", "--only", "-F", "-"],
                Some(msg.as_bytes()),
            )
            .map(drop);
    }
    // Squash/fixup act on the commit *and* its parent, so the rebase starts one further back.
    let fold = matches!(how, Rewrite::Squash | Rewrite::Fixup);
    let first = if fold {
        format!("{oid}^")
    } else {
        oid.to_owned()
    };
    let base = format!("{first}^");
    let has_base = loc.git_ok(&["rev-parse", "-q", "--verify", &base]);
    if fold && !loc.git_ok(&["rev-parse", "-q", "--verify", &first]) {
        bail!("the first commit has no parent to squash into");
    }
    let range = if has_base {
        format!("{base}..HEAD")
    } else {
        "HEAD".into()
    };
    if !loc
        .git(&["rev-list", "--merges", &range])?
        .trim()
        .is_empty()
    {
        bail!("rewriting history across merge commits is not supported");
    }
    let (line, verb) = match &how {
        Rewrite::Squash => (2, "squash"),
        Rewrite::Fixup => (2, "fixup"),
        Rewrite::Drop => (1, "drop"),
        Rewrite::Reword(_) => (1, "reword"),
    };
    let seq = format!("sed -i {line}s/^pick/{verb}/");
    let mut editor = "true".to_owned();
    if let Rewrite::Reword(msg) = &how {
        let git_dir = loc
            .git(&["rev-parse", "--absolute-git-dir"])?
            .trim()
            .to_owned();
        let msg_file = format!("{git_dir}/GITTYHUB_REWORD_MSG");
        loc.shell(
            &format!("cat > {}", shell_quote(&msg_file)),
            Some(msg.as_bytes()),
        )?;
        editor = format!("cp {}", shell_quote(&msg_file));
    }
    let mut args = vec!["rebase", "-i", "--autostash"];
    if has_base {
        args.push(&base);
    } else {
        args.push("--root");
    }
    let env = [
        ("GIT_SEQUENCE_EDITOR", seq.as_str()),
        ("GIT_EDITOR", editor.as_str()),
    ];
    // On conflicts the rebase stays in progress for the user to resolve or abort.
    loc.git_env(&env, &args, None).map(drop)
}

/// Combine all commits after `base` into one new commit with `message`.
pub fn squash_since(loc: &Location, base: &str, message: &str) -> Result<()> {
    loc.git(&["reset", "--soft", base])?;
    loc.git_in(&["commit", "-F", "-"], Some(message.as_bytes()))
        .map(drop)
}

/// Full messages of `base..HEAD`, oldest first.
pub fn messages_since(loc: &Location, base: &str) -> Result<String> {
    loc.git(&["log", "--reverse", "--format=%B", &format!("{base}..HEAD")])
}

/// Add a pattern to the repository's `.gitignore`.
pub fn ignore(loc: &Location, pattern: &str) -> Result<()> {
    let mut cur = loc.read_file(".gitignore").unwrap_or_default();
    if !cur.is_empty() && !cur.ends_with(b"\n") {
        cur.push(b'\n');
    }
    cur.extend_from_slice(pattern.as_bytes());
    cur.push(b'\n');
    loc.write_file(".gitignore", &cur)
}

pub fn cherry_pick(loc: &Location, oid: &str) -> Result<()> {
    loc.git(&["cherry-pick", oid]).map(drop)
}

pub fn revert(loc: &Location, oid: &str) -> Result<()> {
    loc.git(&["revert", "--no-edit", oid]).map(drop)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}

pub fn reset(loc: &Location, target: &str, mode: ResetMode) -> Result<()> {
    let m = match mode {
        ResetMode::Soft => "--soft",
        ResetMode::Mixed => "--mixed",
        ResetMode::Hard => "--hard",
    };
    loc.git(&["reset", m, target]).map(drop)
}

/// Undo the last commit but keep its changes staged.
pub fn undo_last_commit(loc: &Location) -> Result<()> {
    reset(loc, "HEAD~1", ResetMode::Soft)
}

/// Continue / abort whatever multi-step operation is in progress.
pub fn continue_op(loc: &Location, state: RepoState) -> Result<()> {
    let args: &[&str] = match state {
        RepoState::Merging => &["merge", "--continue"],
        RepoState::Rebasing => &["rebase", "--continue"],
        RepoState::CherryPicking => &["cherry-pick", "--continue"],
        RepoState::Reverting => &["revert", "--continue"],
        RepoState::Clean => return Ok(()),
    };
    loc.git_env(&[("GIT_EDITOR", "true")], args, None).map(drop)
}

pub fn abort_op(loc: &Location, state: RepoState) -> Result<()> {
    let args: &[&str] = match state {
        RepoState::Merging => &["merge", "--abort"],
        RepoState::Rebasing => &["rebase", "--abort"],
        RepoState::CherryPicking => &["cherry-pick", "--abort"],
        RepoState::Reverting => &["revert", "--abort"],
        RepoState::Clean => return Ok(()),
    };
    loc.git(args).map(drop)
}

/// Resolve a conflicted file by taking one side wholesale, then mark it resolved.
pub fn resolve_with(loc: &Location, path: &str, ours: bool) -> Result<()> {
    let side = if ours { "--ours" } else { "--theirs" };
    loc.git(&["checkout", side, "--", path])?;
    stage(loc, &[path])
}

// ---------------------------------------------------------------------------
// Branches

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub remote: bool,
    pub oid: String,
    pub upstream: Option<String>,
    pub track: String,
    pub is_head: bool,
    pub time: i64,
    pub subject: String,
}

pub fn branches(loc: &Location) -> Result<Vec<Branch>> {
    let raw = loc.git(&[
        "for-each-ref",
        "--sort=-committerdate",
        "--format=%(refname)%1f%(refname:short)%1f%(objectname:short)%1f%(upstream:short)%1f%(upstream:track)%1f%(HEAD)%1f%(committerdate:unix)%1f%(subject)",
        "refs/heads",
        "refs/remotes",
    ])?;
    Ok(raw
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(US).collect();
            if f.len() != 8 || f[0].ends_with("/HEAD") {
                return None;
            }
            Some(Branch {
                name: f[1].to_owned(),
                remote: f[0].starts_with("refs/remotes/"),
                oid: f[2].to_owned(),
                upstream: (!f[3].is_empty()).then(|| f[3].to_owned()),
                track: f[4].to_owned(),
                is_head: f[5] == "*",
                time: f[6].parse().unwrap_or(0),
                subject: f[7].to_owned(),
            })
        })
        .collect())
}

/// Switch to `name`; a remote branch like `origin/foo` creates a tracking `foo`.
pub fn checkout(loc: &Location, name: &str, remote: bool) -> Result<()> {
    if remote {
        let local = name.split_once('/').map(|(_, b)| b).unwrap_or(name);
        if loc.git_ok(&[
            "rev-parse",
            "-q",
            "--verify",
            &format!("refs/heads/{local}"),
        ]) {
            return loc.git(&["switch", local]).map(drop);
        }
        return loc.git(&["switch", "--track", name]).map(drop);
    }
    loc.git(&["switch", name]).map(drop)
}

pub fn checkout_detached(loc: &Location, rev: &str) -> Result<()> {
    loc.git(&["switch", "--detach", rev]).map(drop)
}

pub fn create_branch(loc: &Location, name: &str, start: Option<&str>, switch: bool) -> Result<()> {
    let mut args = if switch {
        vec!["switch", "-c", name]
    } else {
        vec!["branch", name]
    };
    if let Some(s) = start {
        args.push(s);
    }
    loc.git(&args).map(drop)
}

pub fn delete_branch(loc: &Location, name: &str, force: bool) -> Result<()> {
    loc.git(&["branch", if force { "-D" } else { "-d" }, name])
        .map(drop)
}

pub fn delete_remote_branch(loc: &Location, remote_branch: &str) -> Result<()> {
    let (remote, branch) = remote_branch
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("not a remote branch: {remote_branch}"))?;
    loc.git(&["push", remote, "--delete", branch]).map(drop)
}

pub fn rename_branch(loc: &Location, old: &str, new: &str) -> Result<()> {
    loc.git(&["branch", "-m", old, new]).map(drop)
}

pub fn merge(loc: &Location, rev: &str, no_ff: bool, squash: bool) -> Result<()> {
    let mut args = vec!["merge", "--no-edit"];
    if no_ff {
        args.push("--no-ff");
    }
    if squash {
        args.push("--squash");
    }
    args.push(rev);
    loc.git(&args).map(drop)
}

pub fn rebase(loc: &Location, onto: &str) -> Result<()> {
    loc.git(&["rebase", onto]).map(drop)
}

pub fn set_upstream(loc: &Location, branch: &str, upstream: &str) -> Result<()> {
    loc.git(&["branch", "--set-upstream-to", upstream, branch])
        .map(drop)
}

// ---------------------------------------------------------------------------
// Remotes and sync

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub fetch_url: String,
    pub push_url: String,
}

pub fn remotes(loc: &Location) -> Result<Vec<Remote>> {
    let raw = loc.git(&["remote", "-v"])?;
    let mut out: Vec<Remote> = Vec::new();
    for line in raw.lines() {
        let mut it = line.split_whitespace();
        let (Some(name), Some(url), Some(kind)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        let idx = match out.iter().position(|r| r.name == name) {
            Some(i) => i,
            None => {
                out.push(Remote {
                    name: name.to_owned(),
                    fetch_url: String::new(),
                    push_url: String::new(),
                });
                out.len() - 1
            }
        };
        if kind == "(fetch)" {
            out[idx].fetch_url = url.to_owned();
        } else {
            out[idx].push_url = url.to_owned();
        }
    }
    Ok(out)
}

pub fn add_remote(loc: &Location, name: &str, url: &str) -> Result<()> {
    loc.git(&["remote", "add", name, url]).map(drop)
}

pub fn remove_remote(loc: &Location, name: &str) -> Result<()> {
    loc.git(&["remote", "remove", name]).map(drop)
}

pub fn set_remote_url(loc: &Location, name: &str, url: &str) -> Result<()> {
    loc.git(&["remote", "set-url", name, url]).map(drop)
}

pub fn rename_remote(loc: &Location, old: &str, new: &str) -> Result<()> {
    loc.git(&["remote", "rename", old, new]).map(drop)
}

/// `owner/name` if `url` points at github.com.
pub fn github_slug(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    let rest = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("https://github.com/"))
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("git://github.com/"))
        .or_else(|| {
            url.strip_prefix("https://")
                .and_then(|u| u.split_once("@github.com/"))
                .map(|(_, r)| r)
        })?;
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let mut parts = rest.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    (!owner.is_empty() && !name.is_empty() && parts.next().is_none())
        .then(|| format!("{owner}/{name}"))
}

/// GitHub repository for this checkout: prefers `upstream`, then `origin`, then any remote.
pub fn github_repo(loc: &Location) -> Option<String> {
    let remotes = remotes(loc).ok()?;
    let pick = |n: &str| {
        remotes
            .iter()
            .find(|r| r.name == n)
            .and_then(|r| github_slug(&r.fetch_url))
    };
    pick("origin")
        .or_else(|| pick("upstream"))
        .or_else(|| remotes.iter().find_map(|r| github_slug(&r.fetch_url)))
}

pub fn fetch(loc: &Location) -> Result<()> {
    loc.git(&["fetch", "--all", "--prune", "--tags"]).map(drop)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullMode {
    Merge,
    Rebase,
    FastForwardOnly,
}

pub fn pull(loc: &Location, mode: PullMode) -> Result<()> {
    let flag = match mode {
        PullMode::Merge => "--no-rebase",
        PullMode::Rebase => "--rebase",
        PullMode::FastForwardOnly => "--ff-only",
    };
    loc.git(&["pull", flag]).map(drop)
}

/// Push the current branch, setting an upstream on the first push.
pub fn push(loc: &Location, force: bool) -> Result<()> {
    let st = status(loc)?;
    let mut args = vec!["push"];
    if force {
        args.push("--force-with-lease");
    }
    if st.branch.upstream.is_none() {
        let remote = remotes(loc)?
            .into_iter()
            .map(|r| r.name)
            .find(|n| n == "origin")
            .or_else(|| remotes(loc).ok()?.into_iter().next().map(|r| r.name));
        let Some(remote) = remote else {
            bail!("this repository has no remote to push to");
        };
        let mut a = args.clone();
        a.extend(["-u", &remote, &st.branch.head]);
        return loc.git(&a).map(drop);
    }
    loc.git(&args).map(drop)
}

// ---------------------------------------------------------------------------
// Stashes

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stash {
    pub reference: String,
    pub message: String,
    pub time: i64,
}

pub fn stashes(loc: &Location) -> Result<Vec<Stash>> {
    let raw = loc.git(&["stash", "list", "--format=%gd%x1f%s%x1f%ct"])?;
    Ok(raw
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(US).collect();
            (f.len() == 3).then(|| Stash {
                reference: f[0].to_owned(),
                message: f[1].to_owned(),
                time: f[2].parse().unwrap_or(0),
            })
        })
        .collect())
}

pub fn stash_push(loc: &Location, message: &str, include_untracked: bool) -> Result<()> {
    let mut args = vec!["stash", "push"];
    if include_untracked {
        args.push("--include-untracked");
    }
    if !message.is_empty() {
        args.extend(["-m", message]);
    }
    loc.git(&args).map(drop)
}

pub fn stash_apply(loc: &Location, r: &str, pop: bool) -> Result<()> {
    loc.git(&["stash", if pop { "pop" } else { "apply" }, r])
        .map(drop)
}

pub fn stash_drop(loc: &Location, r: &str) -> Result<()> {
    loc.git(&["stash", "drop", r]).map(drop)
}

pub fn stash_show(loc: &Location, r: &str) -> Result<String> {
    loc.git(&[
        "stash",
        "show",
        "--include-untracked",
        "-p",
        "--no-color",
        r,
    ])
    .or_else(|_| loc.git(&["stash", "show", "-p", "--no-color", r]))
}

// ---------------------------------------------------------------------------
// Tags

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    pub target: String,
    pub subject: String,
    pub time: i64,
}

pub fn tags(loc: &Location) -> Result<Vec<Tag>> {
    let raw = loc.git(&[
        "for-each-ref",
        "--sort=-creatordate",
        "--format=%(refname:short)%1f%(objectname:short)%1f%(subject)%1f%(creatordate:unix)",
        "refs/tags",
    ])?;
    Ok(raw
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(US).collect();
            (f.len() == 4).then(|| Tag {
                name: f[0].to_owned(),
                target: f[1].to_owned(),
                subject: f[2].to_owned(),
                time: f[3].parse().unwrap_or(0),
            })
        })
        .collect())
}

pub fn create_tag(loc: &Location, name: &str, rev: &str, message: &str) -> Result<()> {
    if message.is_empty() {
        loc.git(&["tag", name, rev]).map(drop)
    } else {
        loc.git(&["tag", "-a", name, "-m", message, rev]).map(drop)
    }
}

pub fn delete_tag(loc: &Location, name: &str) -> Result<()> {
    loc.git(&["tag", "-d", name]).map(drop)
}

pub fn push_tag(loc: &Location, remote: &str, name: &str) -> Result<()> {
    loc.git(&["push", remote, &format!("refs/tags/{name}")])
        .map(drop)
}

pub fn delete_remote_tag(loc: &Location, remote: &str, name: &str) -> Result<()> {
    loc.git(&["push", remote, "--delete", &format!("refs/tags/{name}")])
        .map(drop)
}

// ---------------------------------------------------------------------------
// Files at a revision

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: Option<u64>,
}

pub fn ls_tree(loc: &Location, rev: &str, dir: &str) -> Result<Vec<TreeEntry>> {
    let spec = if dir.is_empty() {
        rev.to_owned()
    } else {
        format!("{rev}:{dir}")
    };
    let raw = loc.git(&["ls-tree", "-l", "-z", &spec])?;
    let mut out: Vec<TreeEntry> = raw
        .split('\0')
        .filter_map(|e| {
            let (meta, name) = e.split_once('\t')?;
            let f: Vec<&str> = meta.split_whitespace().collect();
            Some(TreeEntry {
                name: name.to_owned(),
                is_dir: f.get(1) == Some(&"tree"),
                size: f.get(3).and_then(|s| s.parse().ok()),
            })
        })
        .collect();
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    Ok(out)
}

pub fn file_at(loc: &Location, rev: &str, path: &str) -> Result<Vec<u8>> {
    let out = loc.git_output(&["show", &format!("{rev}:{path}")], None)?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
}

pub fn blame(loc: &Location, rev: &str, path: &str) -> Result<String> {
    loc.git(&["blame", "--date=short", "-w", rev, "--", path])
}

// ---------------------------------------------------------------------------
// Repository setup and config

/// Clone `url` into the directory `dest` (on `parent`'s host).
pub fn clone(parent: &Location, url: &str, dest: &str) -> Result<()> {
    // Run from `/` so the destination path is taken literally.
    parent.with_path("/").git(&["clone", url, dest]).map(drop)
}

pub fn init(loc: &Location) -> Result<()> {
    let path = match loc {
        Location::Local(p) => p.to_string_lossy().into_owned(),
        Location::Ssh { path, .. } => path.clone(),
    };
    loc.with_path("/").git(&["init", &path]).map(drop)
}

pub fn config_get(loc: &Location, key: &str) -> Option<String> {
    loc.git(&["config", "--get", key])
        .ok()
        .map(|s| s.trim().to_owned())
}

pub fn config_set(loc: &Location, key: &str, value: &str) -> Result<()> {
    loc.git(&["config", key, value]).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(
            github_slug("git@github.com:a/b.git").as_deref(),
            Some("a/b")
        );
        assert_eq!(
            github_slug("https://github.com/a/b").as_deref(),
            Some("a/b")
        );
        assert_eq!(
            github_slug("https://x-token@github.com/a/b.git").as_deref(),
            Some("a/b")
        );
        assert_eq!(github_slug("https://gitlab.com/a/b"), None);
    }

    #[test]
    fn status_v2() {
        let raw = "# branch.oid abc\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
1 .M N... 100644 100644 100644 a a src/x.rs\0\
2 R. N... 100644 100644 100644 a a R100 new name.rs\0old.rs\0\
u UU N... 100644 100644 100644 100644 a b c conflict.txt\0? new.txt\0";
        let (b, f) = parse_status(raw);
        assert_eq!(b.head, "main");
        assert_eq!((b.ahead, b.behind), (2, 1));
        assert_eq!(f.len(), 4);
        assert!(f[0].unstaged() && !f[0].staged());
        assert_eq!(f[1].path, "new name.rs");
        assert_eq!(f[1].orig_path.as_deref(), Some("old.rs"));
        assert!(f[2].conflicted);
        assert!(f[3].untracked());
    }

    #[test]
    fn diff_hunks() {
        let d = "diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n@@ -9 +9 @@\n-c\n+d\n";
        let files = parse_diff(d);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "f");
        assert_eq!(files[0].hunks.len(), 2);
        assert!(files[0].hunk_patch(1).ends_with("@@ -9 +9 @@\n-c\n+d\n"));
        assert!(files[0].hunk_patch(1).starts_with("diff --git"));
    }

    #[test]
    fn partial_patches() {
        let d = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,3 +1,3 @@\n a\n-b\n-c\n+B\n+C\n";
        let f = &parse_diff(d)[0];
        // Body lines: 0 " a", 1 "-b", 2 "-c", 3 "+B", 4 "+C". Discard only the "+B" line.
        let p = partial_patch(f, &[(0, 3)], true).unwrap();
        assert!(p.ends_with("@@ -1,3 +1,3 @@\n a\n+B\n C\n"), "{p}");
        // Forward (stage) only the "-b" line.
        let p = partial_patch(f, &[(0, 1)], false).unwrap();
        assert!(p.ends_with(" a\n-b\n c\n"), "{p}");
        assert!(partial_patch(f, &[], true).is_none());
    }

    #[test]
    fn log_parse() {
        let raw =
            format!("abc{US}ab{US}p1 p2{US}A{US}a@x{US}10{US}HEAD -> main, tag: v1{US}hello{RS}\n");
        let c = parse_log(&raw);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].parents.len(), 2);
        assert_eq!(c[0].refs, vec!["HEAD -> main", "tag: v1"]);
    }
}
