//! Line discards and history rewriting against real temporary repositories.

use git_backend::{Location, LogQuery, Rewrite};

fn repo(name: &str) -> (Location, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("gittyhub-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let loc = Location::Local(dir.clone());
    loc.git(&["init", "-q", "-b", "main"]).unwrap();
    loc.git(&["config", "user.name", "T"]).unwrap();
    loc.git(&["config", "user.email", "t@example.com"]).unwrap();
    (loc, dir)
}

fn commit(loc: &Location, file: &str, text: &str, msg: &str) {
    loc.write_file(file, text.as_bytes()).unwrap();
    git_backend::stage_all(loc).unwrap();
    git_backend::commit(loc, msg, false, false).unwrap();
}

fn subjects(loc: &Location) -> Vec<String> {
    let q = LogQuery {
        limit: 20,
        ..Default::default()
    };
    git_backend::log(loc, &q)
        .unwrap()
        .into_iter()
        .map(|c| c.subject)
        .collect()
}

#[test]
fn discard_single_line() {
    let (loc, dir) = repo("lines");
    commit(&loc, "f.txt", "a\nb\nc\nd\n", "init");
    loc.write_file("f.txt", b"a\nB\nc\nD\n").unwrap();
    let st = git_backend::status(&loc).unwrap();
    let diff = git_backend::worktree_diff(&loc, &st.files[0]).unwrap();
    let file = &git_backend::parse_diff(&diff)[0];
    // Find the "+B" line in the first hunk and discard just that.
    let body: Vec<&str> = file.hunks[0].lines().skip(1).collect();
    let idx = body.iter().position(|l| *l == "+B").unwrap();
    let sel = git_backend::expand_to_changes(file, &[(0, idx)]);
    git_backend::discard_lines(&loc, file, &sel).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("f.txt")).unwrap(),
        "a\nb\nc\nD\n"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn squash_fixup_drop_reword() {
    let (loc, dir) = repo("rewrite");
    commit(&loc, "a", "1\n", "one");
    commit(&loc, "b", "2\n", "two");
    commit(&loc, "c", "3\n", "three");
    commit(&loc, "d", "4\n", "four");
    // Uncommitted work must survive (autostash).
    loc.write_file("a", b"dirty\n").unwrap();

    let oid = |n: usize| {
        loc.git(&["rev-parse", &format!("HEAD~{n}")])
            .unwrap()
            .trim()
            .to_owned()
    };
    git_backend::rewrite(&loc, &oid(1), Rewrite::Squash).unwrap(); // three -> two
    assert_eq!(subjects(&loc), ["four", "two", "one"]);
    let msg = git_backend::head_message(&loc).unwrap();
    assert!(msg.starts_with("four"));
    git_backend::rewrite(&loc, &oid(0), Rewrite::Fixup).unwrap(); // four -> two
    assert_eq!(subjects(&loc), ["two", "one"]);
    git_backend::rewrite(&loc, &oid(1), Rewrite::Reword("uno\n\nbody".into())).unwrap();
    assert_eq!(subjects(&loc), ["two", "uno"]);
    git_backend::rewrite(&loc, &oid(0), Rewrite::Reword("dos".into())).unwrap();
    assert_eq!(subjects(&loc), ["dos", "uno"]);
    git_backend::rewrite(&loc, &oid(1), Rewrite::Drop).unwrap(); // drop root
    assert_eq!(subjects(&loc), ["dos"]);
    assert!(
        !dir.join("a").exists() || std::fs::read_to_string(dir.join("a")).unwrap() == "dirty\n"
    );
    assert!(dir.join("d").exists() && dir.join("c").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn commit_only_included_files() {
    let (loc, dir) = repo("include");
    commit(&loc, "a", "1\n", "init");
    loc.write_file("a", b"2\n").unwrap();
    loc.write_file("b", b"new\n").unwrap();
    // Pre-stage the file that will be excluded, as an external tool might.
    git_backend::stage(&loc, &["b"]).unwrap();
    git_backend::commit_paths(&loc, &["a"], &["b"], "only a", false, false).unwrap();
    let files = git_backend::commit_files(&loc, "HEAD").unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "a");
    let st = git_backend::status(&loc).unwrap();
    assert_eq!(st.files.len(), 1);
    assert!(st.files[0].untracked());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rebase_state_ignores_stale_rebase_head() {
    use git_backend::RepoState;
    let (loc, dir) = repo("rebase-state");
    commit(&loc, "f", "base\n", "base");
    loc.git(&["checkout", "-q", "-b", "side"]).unwrap();
    commit(&loc, "f", "side\n", "side");
    loc.git(&["checkout", "-q", "main"]).unwrap();
    commit(&loc, "f", "main\n", "main");
    loc.git(&["checkout", "-q", "side"]).unwrap();
    assert!(loc.git(&["rebase", "main"]).is_err());
    assert_eq!(git_backend::repo_state(&loc), RepoState::Rebasing);
    git_backend::abort_op(&loc, RepoState::Rebasing).unwrap();
    assert_eq!(git_backend::repo_state(&loc), RepoState::Clean);
    // Git can leave REBASE_HEAD behind once a rebase is over; that alone is not a rebase.
    let head = loc.git(&["rev-parse", "HEAD"]).unwrap();
    loc.git(&["update-ref", "REBASE_HEAD", head.trim()])
        .unwrap();
    assert_eq!(git_backend::repo_state(&loc), RepoState::Clean);
    std::fs::remove_dir_all(dir).unwrap();
}
