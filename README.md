# GittyHub

A Git client for Linux with deep GitHub integration, built in Rust with [GTK 4](https://gtk.org) and [libadwaita](https://gnome.pages.gitlab.gnome.org/libadwaita/).

GittyHub uses the tools you already have as its backend:

- **`git`** for repository operations
- **[`gh`](https://cli.github.com/)** (the official GitHub CLI) for GitHub features such as pull requests, issues and Actions. Authentication is handled entirely by `gh`, so GittyHub never stores your tokens.

> **Status:** early development. Expect rough edges and breaking changes.

## Features

- **Tabs** — one tab per repository; open tabs are restored on startup.
- **Local and remote repositories** — open, clone or create repositories on this machine or on
  any host you can reach over SSH. Remote repositories run `git` on the host through your system
  `ssh` client, so hosts, keys and `ssh-agent` come from your own `~/.ssh` setup.
- **Changes** — stage, unstage and discard whole files or single hunks; commit, amend, sign off,
  undo the last commit; resolve conflicts and continue or abort merges, rebases and cherry-picks.
- **History** — searchable log (current branch or all), commit diffs, check out, branch or tag
  from a commit, cherry-pick, revert, soft/mixed/hard reset.
- **Branches, tags, stashes and remotes** — create, switch, merge, squash-merge, rebase, rename,
  delete (locally or on the remote), push, fetch, pull (merge, rebase or fast-forward only).
- **Files** — browse the tree at any revision, view files, blame and per-file history.
- **GitHub** (via `gh`) — repository overview and settings, star and fork, publish a local
  repository; pull requests (create, review, merge, check out, comment, diff, checks); issues;
  Actions (runs, jobs, logs, re-run, cancel, dispatch, enable/disable workflows); releases
  (create, publish, upload and download assets); deployments and environments; packages.
- **Appearance** — follows GNOME's light/dark preference or forces either, with a choice of
  accent colours; adaptive layout for narrow windows.

## Requirements

- Rust stable (pinned via `rust-toolchain.toml`)
- `git` and `gh` on your `PATH` (run `gh auth login` once)
- Linux with GTK ≥ 4.18 and libadwaita ≥ 1.8 development packages
  (Debian/Ubuntu: `sudo apt install libgtk-4-dev libadwaita-1-dev`,
  Fedora: `sudo dnf install gtk4-devel libadwaita-devel`,
  Arch: `sudo pacman -S gtk4 libadwaita`)

## Build and run

```bash
cargo run            # debug build, opens the app on the current directory's repo
cargo build --release
```

To install for your user (binary in `~/.cargo/bin`, plus a desktop entry and icon so GittyHub
shows up in the app grid):

```bash
./install.sh
```

## Project layout

```
crates/
├─ app/          GTK 4 + libadwaita user interface (binary `gittyhub`)
│  ├─ src/window.rs   tabs, header bar, welcome screen
│  ├─ src/repo.rs     one repository tab: sidebar, sync button, shared context
│  ├─ src/views/      one module per sidebar section
│  └─ data/           GResource: stylesheet and symbolic icons
├─ git-backend/  wrapper around the git CLI, locally or over SSH (`Location`)
└─ gh-backend/   wrapper around the gh CLI (JSON output → typed structs)
```

The backend crates have no UI dependencies and can be tested headlessly.

## Development

```bash
cargo fmt
cargo clippy --workspace -- -D warnings
cargo test --workspace
cargo test -p gh-backend -- --ignored   # live checks against a public repo (needs gh auth)
```

## License

GittyHub is free software, licensed under the [GNU General Public License v3.0 or later](LICENSE).
