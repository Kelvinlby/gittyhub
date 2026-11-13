//! Exercise the SSH transport end to end with a fake `ssh` that runs the remote command
//! locally through `sh -c`, as a real sshd would. This checks argument quoting.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use git_backend::Location;

#[test]
fn git_over_fake_ssh() {
    let dir = std::env::temp_dir().join(format!("gittyhub-ssh-test-{}", std::process::id()));
    let bin = dir.join("bin");
    let repo = dir.join("repo with 'quotes' and spaces");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    let fake = bin.join("ssh");
    // Drop options up to `--` (and the host before it), then run the command string.
    std::fs::write(
        &fake,
        "#!/bin/sh\nwhile [ \"$1\" != \"--\" ]; do shift; done\nshift\nexec sh -c \"$*\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    // SAFETY: this test binary runs this single test before spawning other threads.
    unsafe { std::env::set_var("PATH", path) };

    let loc = Location::Ssh {
        host: "fakehost".into(),
        path: repo.to_string_lossy().into_owned(),
    };
    git_backend::init(&loc).unwrap();
    std::fs::write(repo.join("a file.txt"), "hello\n").unwrap();
    git_backend::stage(&loc, &["a file.txt"]).unwrap();
    git_backend::config_set(&loc, "user.name", "Test O'Brien").unwrap();
    git_backend::config_set(&loc, "user.email", "t@example.com").unwrap();
    git_backend::commit(&loc, "subject with $HOME and `ticks`\n\nbody", false, false).unwrap();

    let q = git_backend::LogQuery {
        limit: 5,
        ..Default::default()
    };
    let log = git_backend::log(&loc, &q).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].subject, "subject with $HOME and `ticks`");
    assert_eq!(log[0].author, "Test O'Brien");
    assert_eq!(
        PathBuf::from(git_backend::toplevel(&loc).unwrap()),
        repo.canonicalize().unwrap()
    );
    assert!(git_backend::status(&loc).unwrap().files.is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}
