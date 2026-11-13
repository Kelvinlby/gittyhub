# GittyHub

A Git client for Linux with deep GitHub integration, built in Rust with [GTK 4](https://gtk.org) and [libadwaita](https://gnome.pages.gitlab.gnome.org/libadwaita/).

GittyHub uses the tools you already have as its backend:

- **`git`** for repository operations
- **[`gh`](https://cli.github.com/)** (the official GitHub CLI) for GitHub features such as pull requests, issues and Actions. Authentication is handled entirely by `gh`, so GittyHub never stores your tokens.

> **Status:** early development. Expect rough edges and breaking changes.

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

## Project layout

```
crates/
├─ app/          GTK 4 + libadwaita user interface (binary `gittyhub`)
├─ git-backend/  wrapper around the git CLI
└─ gh-backend/   wrapper around the gh CLI (JSON output → typed structs)
```

The backend crates have no UI dependencies and can be tested headlessly.

## Development

```bash
cargo fmt
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

## License

GittyHub is free software, licensed under the [GNU General Public License v3.0 or later](LICENSE).
