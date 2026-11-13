# GittyHub

A Git client with deep GitHub integration, aiming to grow into sub-IDE-grade software.

## GUI toolkit and platform

- **Linux only.** No Windows/macOS support; don't add platform shims for them.
- **GTK 4 + libadwaita** via the `gtk4` and `libadwaita` crates (renamed `gtk` / `adw` in the workspace).
  Feature floors: `gtk` `v4_18`, `adw` `v1_8`. Raise them only when a newer API is genuinely needed.
- Wayland is the first-class target; X11 is supported but second-class. Test on Wayland first, and
  never rely on X11-only behaviour (global window positioning, grabbing, etc.).

## UI standards

- The UI must look good and follow the [GNOME Human Interface Guidelines](https://developer.gnome.org/hig/)
  and libadwaita patterns: `AdwToolbarView` + `AdwHeaderBar`, `AdwNavigationSplitView` /
  `AdwOverlaySplitView`, `AdwPreferencesPage`, `AdwToast`, `AdwAlertDialog`, boxed lists, style
  classes (`suggested-action`, `destructive-action`, `pill`, `card`, …).
- Prefer libadwaita widgets over hand-rolled ones; use adaptive layouts (breakpoints) so windows work
  when narrow.
- Use symbolic icons and theme colours; never hard-code colours, so light/dark and accent colours work.
- Never block the main thread: run `git`/`gh` work with `gio::spawn_blocking` and update widgets from
  `glib::spawn_future_local`.

## Backend: git and gh CLIs

- All repository operations go through the `git` CLI (`crates/git-backend`); all GitHub operations go
  through the official `gh` CLI (`crates/gh-backend`), preferring `--json` output and `gh api`.
- This means GittyHub never handles credentials or tokens: auth belongs to `git` and `gh`.
- Switching a feature to the GitHub REST/GraphQL API directly (and handling auth ourselves) is allowed
  when it makes things clearly neater or enables something `gh` can't do. Document the reason when doing so.
- Backend crates must stay free of GTK dependencies so they can be tested headlessly.

## Architecture notes

- `git_backend::Location` is either `Local(path)` or `Ssh { host, path }`; every git function takes
  one. SSH runs `git` on the host via the system `ssh` (BatchMode, connection sharing) — never
  read or manage keys. `tests/ssh_transport.rs` checks quoting with a fake `ssh`.
- GitHub calls always pass an explicit `-R owner/name` (derived from the `origin`/`upstream`
  remote), so they run locally even for SSH-hosted checkouts.
- App: `repo::Ctx` is the per-tab context (location, slug, status, toasts); use `ctx.load` for
  reads, `ctx.git` for repo-changing ops (shows errors, then refreshes), `ctx.gh` / `ctx.gh_load`
  for GitHub. Each sidebar section is a `views::*` module implementing `repo::View`.
- Custom icons live in `crates/app/data/icons` and must be fill-only SVGs (GTK's symbolic
  loader ignores strokes).

## Workflow

- Commit and push to `origin/main` whenever a change is meaningful and complete (builds, passes
  `cargo clippy --workspace -- -D warnings` and `cargo fmt --check`). Don't wait to be asked, but
  don't push broken or half-done work.
- Repo: https://github.com/Kelvinlby/gittyhub (public, GPL-3.0-or-later).

## Commands

```bash
cargo run                                  # run on the current directory's repo
cargo clippy --workspace -- -D warnings
cargo fmt
cargo test --workspace
```
