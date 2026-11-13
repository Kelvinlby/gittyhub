# GittyHub

A Git client for Linux with deep GitHub integration, built in Rust with [GTK 4](https://gtk.org) and [libadwaita](https://gnome.pages.gitlab.gnome.org/libadwaita/).

GittyHub uses the tools you already have as its backend:

- **`git`** for repository operations
- **[`gh`](https://cli.github.com/)** (the official GitHub CLI) for GitHub features such as pull requests, issues and Actions. Authentication is handled entirely by `gh`, so GittyHub never stores your tokens.

> **Status:** early development. Expect rough edges and breaking changes.

## Features

GittyHub is built around your edits rather than a list of tools. Each repository opens in a
tab with GitHub-style sections — **Code**, Branches, Pull Requests, Issues, Actions, Releases,
Deploy and Settings — and starts on Code:

- **Timeline** (left): uncommitted changes, then commits not yet pushed, then pushed history.
  Right-click a commit to edit its message, squash or fixup into the previous commit, drop it,
  undo it, revert, cherry-pick, reset, branch or tag from it. "Not Pushed" can be squashed into
  one commit or pushed in one go.
- **File tree** (middle): the changed files of the selected entry in their folders — or as a
  flat list of paths (toggle next to Changes / All Files) — or every file. Checkboxes choose what goes into the next commit; right-click to discard, ignore,
  restore an old version, view history or blame.
- **File pane** (right): the diff with line numbers and syntax highlighting — right-click a line, a selection or a hunk
  to discard just that change — or a rendered preview (Markdown is rendered), or an editor with
  syntax highlighting and line numbers (Ctrl+S saves, also on SSH hosts).
- **Commit composer**: leave the summary empty and it uses “Update <file names>”.

Also:

- **Tabs and windows** — the new-tab button (Ctrl+T) lists recently opened repositories, local
  or SSH, so reopening one is a click. Open more windows with Ctrl+N (or the dock icon's
  “New Window”); drag tabs between windows or out into a new one. Every window's tabs are
  restored on startup.
- **Local and remote repositories** — open, clone or create repositories on this machine or on
  any host you can reach over SSH. Remote repositories run `git` on the host through your system
  `ssh` client, so hosts, keys and `ssh-agent` come from your own `~/.ssh` setup.
- **Branches, tags and stashes** side by side; fetch, pull (merge, rebase or fast-forward only)
  and push from the header's sync button, with background fetching.
- **GitHub** (via `gh`) — pull requests (create, review, merge, check out, comment, diff,
  checks), issues, Actions (runs, jobs, logs, re-run, cancel, dispatch, enable/disable
  workflows), releases (assets), deployments, packages, and repository settings (star, fork,
  visibility, archive, publish a local repository). Lists and details sit side by side.
- **Appearance** — follows GNOME's light/dark preference or forces either, with a choice of
  accent colours; code font, size and line spacing for diffs and the editor; interface zoom
  (Ctrl+= / Ctrl+− / Ctrl+0); adapts to narrow windows.

## Requirements

- Rust stable (pinned via `rust-toolchain.toml`)
- `git` and `gh` on your `PATH` (run `gh auth login` once)
- Linux with GTK ≥ 4.18, libadwaita ≥ 1.8 and GtkSourceView 5 development packages
  (Debian/Ubuntu: `sudo apt install libgtk-4-dev libadwaita-1-dev libgtksourceview-5-dev`,
  Fedora: `sudo dnf install gtk4-devel libadwaita-devel gtksourceview5-devel`,
  Arch: `sudo pacman -S gtk4 libadwaita gtksourceview5`)

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
│  ├─ src/repo.rs     one repository tab: view switcher, sync button, shared context
│  ├─ src/views/      one module per section (code.rs is the main editor view)
│  ├─ src/tree.rs     file tree widget
│  ├─ src/markdown.rs Markdown → GTK widgets
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
