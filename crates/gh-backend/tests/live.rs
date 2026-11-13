//! Live checks against a public repository. Needs network and `gh auth login`.
//! Run with `cargo test -p gh-backend -- --ignored`.

const REPO: &str = "cli/cli";

#[test]
#[ignore]
fn pull_requests_deserialize() {
    let prs = gh_backend::pr_list(REPO, "open", "").unwrap();
    let pr = gh_backend::pr_view(REPO, prs[0].number).unwrap();
    let _ = pr.checks();
    assert!(!gh_backend::pr_diff(REPO, pr.number).unwrap().is_empty());
}

#[test]
#[ignore]
fn issues_deserialize() {
    let issues = gh_backend::issue_list(REPO, "open", "").unwrap();
    gh_backend::issue_view(REPO, issues[0].number).unwrap();
}

#[test]
#[ignore]
fn actions_deserialize() {
    let runs = gh_backend::run_list(REPO, None).unwrap();
    let d = gh_backend::run_view(REPO, runs[0].database_id).unwrap();
    assert_eq!(d.run.database_id, runs[0].database_id);
    assert!(!gh_backend::workflow_list(REPO).unwrap().is_empty());
}

#[test]
#[ignore]
fn releases_and_repo_deserialize() {
    let rels = gh_backend::release_list(REPO).unwrap();
    let r = gh_backend::release_view(REPO, &rels[0].tag_name).unwrap();
    assert!(!r.assets.is_empty());
    gh_backend::repo_view(REPO).unwrap();
    gh_backend::deployments(REPO).unwrap();
    let _ = gh_backend::environments(REPO);
}
