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
