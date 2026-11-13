# GittyHub

A cross-platform desktop Git client with deep GitHub integration, built in Rust with [GPUI](https://gpui.rs) and [gpui-component](https://github.com/longbridge/gpui-kit).

GittyHub uses the tools you already have as its backend:

- **`git`** for repository operations
- **[`gh`](https://cli.github.com/)** (the official GitHub CLI) for GitHub features such as pull requests, issues and Actions. Authentication is handled entirely by `gh`, so GittyHub never stores your tokens.

> **Status:** early development. Expect rough edges and breaking changes.

## Requirements

- Rust stable (pinned via `rust-toolchain.toml`)
- `git` and `gh` on your `PATH` (run `gh auth login` once)
- Linux: Wayland/X11, xkbcommon and fontconfig development libraries, plus a Vulkan-capable driver

## Build and run

```bash
cargo run            # debug build, opens the app on the current directory's repo
cargo build --release
```

## Project layout

```
crates/
├─ app/          GPUI user interface (binary `gittyhub`)
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

GPUI is pre-1.0, so `gpui`, `gpui-platform` and `gpui-component` are pinned to exact versions in the workspace `Cargo.toml`. Upgrade them together and deliberately.

## License

GittyHub is free software, licensed under the [GNU General Public License v3.0 or later](LICENSE).
