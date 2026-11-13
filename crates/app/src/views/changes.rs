//! Working-tree changes: stage, unstage, discard (per file or hunk) and commit.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use git_backend::{FileChange, HunkAction, RepoState};
use gtk::glib;

use crate::diff::{DiffView, HunkMode};
use crate::repo::{Ctx, View};
use crate::{settings, util};

pub struct Changes {
    ctx: Rc<Ctx>,
    root: gtk::Paned,
    banner: adw::Banner,
    conflicts_box: gtk::Box,
    conflicts: gtk::ListBox,
    staged_box: gtk::Box,
    staged: gtk::ListBox,
    staged_title: gtk::Label,
    unstaged_box: gtk::Box,
    unstaged: gtk::ListBox,
    unstaged_title: gtk::Label,
    empty: adw::StatusPage,
    summary: gtk::Entry,
    description: gtk::TextBuffer,
    amend: gtk::CheckButton,
    commit: gtk::Button,
    diff: Rc<DiffView>,
    files: RefCell<Vec<FileChange>>,
    /// (path, staged) of the file whose diff is shown.
    selected: RefCell<Option<(String, bool)>>,
    suppress: Cell<bool>,
}

fn status_class(c: char) -> (&'static str, &'static str) {
    match c {
        'A' | '?' => ("added", "Added"),
        'D' => ("deleted", "Deleted"),
        'R' | 'C' => ("renamed", "Renamed"),
        'U' => ("conflict", "Conflicted"),
        _ => ("modified", "Modified"),
    }
}

fn section_header(title: &gtk::Label, button: &gtk::Widget) -> gtk::Box {
    let b = util::hbox(6);
    b.set_margin_start(12);
    b.set_margin_end(6);
    b.set_margin_top(6);
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.add_css_class("heading");
    b.append(title);
    b.append(button);
    b
}

pub fn new(ctx: &Rc<Ctx>) -> Rc<dyn View> {
    let banner = adw::Banner::builder().button_label("Abort").build();

    let list = || {
        gtk::ListBox::builder()
            .css_classes(["navigation-sidebar"])
            .build()
    };
    let conflicts = list();
    let staged = list();
    let unstaged = list();
    let conflicts_title = gtk::Label::new(Some("Conflicts"));
    let staged_title = gtk::Label::new(Some("Staged Changes"));
    let unstaged_title = gtk::Label::new(Some("Changes"));
    let unstage_all = util::icon_button("list-remove-all-symbolic", "Unstage All");
    let stage_all = util::icon_button("list-add-symbolic", "Stage All");
    let more = util::menu_button(vec![]);
    let unstaged_actions = util::hbox(0);
    unstaged_actions.append(&stage_all);
    unstaged_actions.append(&more);

    let conflicts_box = util::vbox(0);
    conflicts_box.append(&section_header(
        &conflicts_title,
        &gtk::Box::new(gtk::Orientation::Horizontal, 0).upcast(),
    ));
    conflicts_box.append(&conflicts);
    let staged_box = util::vbox(0);
    staged_box.append(&section_header(&staged_title, unstage_all.upcast_ref()));
    staged_box.append(&staged);
    let unstaged_box = util::vbox(0);
    unstaged_box.append(&section_header(
        &unstaged_title,
        unstaged_actions.upcast_ref(),
    ));
    unstaged_box.append(&unstaged);
    let empty = adw::StatusPage::builder()
        .icon_name("object-select-symbolic")
        .title("No Local Changes")
        .description("Your working tree is clean.")
        .css_classes(["compact"])
        .vexpand(true)
        .build();

    let lists = util::vbox(6);
    lists.append(&conflicts_box);
    lists.append(&staged_box);
    lists.append(&unstaged_box);
    lists.append(&empty);
    let lists_scroller = gtk::ScrolledWindow::builder()
        .child(&lists)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();

    // Commit box
    let summary = gtk::Entry::builder()
        .placeholder_text("Summary (required)")
        .build();
    let description = gtk::TextBuffer::new(None);
    let desc_view = gtk::TextView::builder()
        .buffer(&description)
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(6)
        .bottom_margin(6)
        .left_margin(6)
        .right_margin(6)
        .accepts_tab(false)
        .css_classes(["commit-description"])
        .build();
    let desc_scroller = gtk::ScrolledWindow::builder()
        .child(&desc_view)
        .min_content_height(72)
        .max_content_height(160)
        .propagate_natural_height(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .css_classes(["card"])
        .build();
    let amend = gtk::CheckButton::with_label("Amend last commit");
    let commit = gtk::Button::builder()
        .label("Commit")
        .css_classes(["suggested-action"])
        .sensitive(false)
        .build();
    let commit_box = util::vbox(8);
    commit_box.add_css_class("commit-box");
    commit_box.append(&summary);
    commit_box.append(&desc_scroller);
    let commit_row = util::hbox(6);
    amend.set_hexpand(true);
    commit_row.append(&amend);
    commit_box.append(&commit_row);
    commit_box.append(&commit);

    let left = util::vbox(0);
    left.append(&banner);
    left.append(&lists_scroller);
    left.append(&commit_box);
    left.set_width_request(260);

    let diff = DiffView::new();
    diff.set_placeholder(
        "document-edit-symbolic",
        "No File Selected",
        "Select a changed file to see its diff.",
    );
    let root = gtk::Paned::builder()
        .orientation(gtk::Orientation::Horizontal)
        .start_child(&left)
        .end_child(&diff.widget())
        .position(340)
        .shrink_start_child(false)
        .resize_start_child(false)
        .build();

    let view = Rc::new(Changes {
        ctx: ctx.clone(),
        root,
        banner,
        conflicts_box,
        conflicts,
        staged_box,
        staged,
        staged_title,
        unstaged_box,
        unstaged,
        unstaged_title,
        empty,
        summary,
        description,
        amend,
        commit,
        diff,
        files: RefCell::new(Vec::new()),
        selected: RefCell::new(None),
        suppress: Cell::new(false),
    });

    // "More" menu for the unstaged section.
    let w = Rc::downgrade(&view);
    let more_menu = util::menu_button(vec![
        (
            "Discard All Changes…",
            Rc::new({
                let w = w.clone();
                move || {
                    if let Some(v) = w.upgrade() {
                        v.discard_all();
                    }
                }
            }),
        ),
        (
            "Stash All Changes",
            Rc::new({
                let w = w.clone();
                move || {
                    if let Some(v) = w.upgrade() {
                        v.ctx
                            .git("Stash", |l| git_backend::stash_push(l, "", true), |_| {});
                    }
                }
            }),
        ),
        (
            "Undo Last Commit",
            Rc::new({
                let w = w.clone();
                move || {
                    if let Some(v) = w.upgrade() {
                        let c = v.ctx.clone();
                        v.ctx
                            .git("Undo Commit", git_backend::undo_last_commit, move |_| {
                                c.toast("Last commit undone; its changes are staged")
                            });
                    }
                }
            }),
        ),
    ]);
    unstaged_actions.remove(&more);
    unstaged_actions.append(&more_menu);

    view.connect_signals(&stage_all, &unstage_all);
    view
}

impl Changes {
    fn connect_signals(self: &Rc<Self>, stage_all: &gtk::Button, unstage_all: &gtk::Button) {
        let w = Rc::downgrade(self);
        stage_all.connect_clicked(glib::clone!(
            #[strong]
            w,
            move |_| {
                if let Some(v) = w.upgrade() {
                    v.ctx.git("Stage All", git_backend::stage_all, |_| {});
                }
            }
        ));
        unstage_all.connect_clicked(glib::clone!(
            #[strong]
            w,
            move |_| {
                if let Some(v) = w.upgrade() {
                    v.ctx.git("Unstage All", git_backend::unstage_all, |_| {});
                }
            }
        ));
        for (list, staged) in [
            (self.staged.clone(), true),
            (self.unstaged.clone(), false),
            (self.conflicts.clone(), false),
        ] {
            list.connect_row_selected(glib::clone!(
                #[strong]
                w,
                move |list, row| {
                    let (Some(v), Some(row)) = (w.upgrade(), row) else {
                        return;
                    };
                    if v.suppress.get() {
                        return;
                    }
                    v.suppress.set(true);
                    for other in [&v.staged, &v.unstaged, &v.conflicts] {
                        if other != list {
                            other.unselect_all();
                        }
                    }
                    v.suppress.set(false);
                    let path = row.widget_name().to_string();
                    *v.selected.borrow_mut() = Some((path, staged));
                    v.load_diff();
                }
            ));
        }
        self.diff.connect_hunk(glib::clone!(
            #[strong]
            w,
            move |patch, action| {
                let Some(v) = w.upgrade() else { return };
                if action == HunkAction::Discard {
                    let ctx = v.ctx.clone();
                    glib::spawn_future_local(async move {
                        if util::confirm(
                            &ctx.parent(),
                            "Discard Hunk?",
                            "These changes will be permanently lost.",
                            "Discard",
                            true,
                        )
                        .await
                        {
                            ctx.git(
                                "Discard Hunk",
                                move |l| git_backend::apply_hunk(l, &patch, action),
                                |_| {},
                            );
                        }
                    });
                } else {
                    v.ctx.git(
                        "Apply Hunk",
                        move |l| git_backend::apply_hunk(l, &patch, action),
                        |_| {},
                    );
                }
            }
        ));
        self.summary.connect_changed(glib::clone!(
            #[strong]
            w,
            move |_| {
                if let Some(v) = w.upgrade() {
                    v.update_commit_button();
                }
            }
        ));
        self.summary.connect_activate(glib::clone!(
            #[strong]
            w,
            move |_| {
                if let Some(v) = w.upgrade() {
                    v.do_commit();
                }
            }
        ));
        self.amend.connect_toggled(glib::clone!(
            #[strong]
            w,
            move |b| {
                let Some(v) = w.upgrade() else { return };
                v.update_commit_button();
                if b.is_active() && v.summary.text().is_empty() {
                    let v2 = v.clone();
                    v.ctx.load(git_backend::head_message, move |r| {
                        if let Ok(msg) = r {
                            let (s, d) = msg.split_once('\n').unwrap_or((&msg, ""));
                            v2.summary.set_text(s.trim());
                            v2.description.set_text(d.trim());
                        }
                    });
                }
            }
        ));
        self.commit.connect_clicked(glib::clone!(
            #[strong]
            w,
            move |_| {
                if let Some(v) = w.upgrade() {
                    v.do_commit();
                }
            }
        ));
        self.banner.connect_button_clicked(glib::clone!(
            #[strong]
            w,
            move |_| {
                let Some(v) = w.upgrade() else { return };
                let state = v
                    .ctx
                    .status
                    .borrow()
                    .as_ref()
                    .map(|s| s.state)
                    .unwrap_or(RepoState::Clean);
                v.ctx
                    .git("Abort", move |l| git_backend::abort_op(l, state), |_| {});
            }
        ));
        let shortcut = gtk::ShortcutController::new();
        shortcut.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string("<Control>Return"),
            Some(gtk::CallbackAction::new(glib::clone!(
                #[strong]
                w,
                move |_, _| {
                    if let Some(v) = w.upgrade() {
                        v.do_commit();
                    }
                    glib::Propagation::Stop
                }
            ))),
        ));
        self.root.add_controller(shortcut);
    }

    fn state(&self) -> RepoState {
        self.ctx
            .status
            .borrow()
            .as_ref()
            .map(|s| s.state)
            .unwrap_or(RepoState::Clean)
    }

    fn update_commit_button(&self) {
        let files = self.files.borrow();
        let staged = files.iter().filter(|f| f.staged()).count();
        let unstaged = files.iter().filter(|f| f.unstaged()).count();
        let conflicts = files.iter().filter(|f| f.conflicted).count();
        let branch = self.ctx.branch();
        let state = self.state();
        let amend = self.amend.is_active();
        let (label, enabled) = match state {
            RepoState::Rebasing | RepoState::CherryPicking | RepoState::Reverting => {
                let what = match state {
                    RepoState::Rebasing => "Rebase",
                    RepoState::CherryPicking => "Cherry-pick",
                    _ => "Revert",
                };
                (format!("Continue {what}"), conflicts == 0)
            }
            RepoState::Merging => ("Commit Merge".to_owned(), conflicts == 0),
            RepoState::Clean => {
                let has_summary = !self.summary.text().trim().is_empty();
                if amend {
                    (format!("Amend Last Commit on {branch}"), has_summary)
                } else if staged > 0 {
                    (
                        format!(
                            "Commit {staged} File{} to {branch}",
                            if staged == 1 { "" } else { "s" }
                        ),
                        has_summary,
                    )
                } else if unstaged > 0 {
                    (
                        format!(
                            "Commit All {unstaged} File{} to {branch}",
                            if unstaged == 1 { "" } else { "s" }
                        ),
                        has_summary,
                    )
                } else {
                    (format!("Commit to {branch}"), false)
                }
            }
        };
        self.commit.set_label(&label);
        self.commit.set_sensitive(enabled);
    }

    fn message(&self) -> String {
        let summary = self.summary.text().trim().to_owned();
        let d = &self.description;
        let body = d.text(&d.start_iter(), &d.end_iter(), false);
        let body = body.trim();
        if body.is_empty() {
            summary
        } else {
            format!("{summary}\n\n{body}")
        }
    }

    fn do_commit(self: &Rc<Self>) {
        if !self.commit.is_sensitive() {
            return;
        }
        let state = self.state();
        let w = Rc::downgrade(self);
        if state != RepoState::Clean && state != RepoState::Merging {
            self.ctx.git(
                "Continue",
                move |l| git_backend::continue_op(l, state),
                |_| {},
            );
            return;
        }
        let msg = self.message();
        let amend = self.amend.is_active();
        let signoff = settings::get().signoff;
        let stage_everything = state == RepoState::Clean
            && !amend
            && !self.files.borrow().iter().any(FileChange::staged);
        self.ctx.git(
            "Commit",
            move |l| {
                if stage_everything {
                    git_backend::stage_all(l)?;
                }
                if state == RepoState::Merging && msg.trim().is_empty() {
                    git_backend::continue_op(l, state)
                } else {
                    git_backend::commit(l, &msg, amend, signoff)
                }
            },
            move |_| {
                if let Some(v) = w.upgrade() {
                    v.summary.set_text("");
                    v.description.set_text("");
                    v.amend.set_active(false);
                    v.ctx.toast("Committed");
                }
            },
        );
    }

    fn discard_all(self: &Rc<Self>) {
        let ctx = self.ctx.clone();
        glib::spawn_future_local(async move {
            if util::confirm(
                &ctx.parent(),
                "Discard All Changes?",
                "All unstaged changes will be lost and untracked files deleted. This cannot be undone.",
                "Discard All",
                true,
            )
            .await
            {
                ctx.git(
                    "Discard",
                    |l| {
                        l.git(&["restore", "--worktree", "--", "."])?;
                        l.git(&["clean", "-fd", "-q"]).map(drop)
                    },
                    |_| {},
                );
            }
        });
    }

    fn file_row(self: &Rc<Self>, f: &FileChange, staged: bool) -> gtk::ListBoxRow {
        let letter = if f.conflicted {
            'U'
        } else if staged {
            f.index
        } else if f.untracked() {
            'A'
        } else {
            f.worktree
        };
        let (class, tooltip) = status_class(letter);
        let row_box = util::hbox(8);
        let status = gtk::Label::builder()
            .label(letter.to_string())
            .tooltip_text(tooltip)
            .css_classes(["status-letter", class])
            .build();
        row_box.append(&status);
        let (dir, name) = match f.path.rsplit_once('/') {
            Some((d, n)) => (format!("{d}/"), n.to_owned()),
            None => (String::new(), f.path.clone()),
        };
        let label = gtk::Label::builder()
            .use_markup(true)
            .label(format!(
                "{}<span alpha=\"60%\">{}</span>",
                glib::markup_escape_text(&name),
                if dir.is_empty() {
                    String::new()
                } else {
                    format!("  {}", glib::markup_escape_text(&dir))
                }
            ))
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .tooltip_text(match &f.orig_path {
                Some(o) => format!("{o} → {}", f.path),
                None => f.path.clone(),
            })
            .build();
        row_box.append(&label);

        let w = Rc::downgrade(self);
        let path = f.path.clone();
        if f.conflicted {
            let ours = util::icon_button("go-previous-symbolic", "Resolve Using Ours");
            let theirs = util::icon_button("go-next-symbolic", "Resolve Using Theirs");
            let mark = util::icon_button("object-select-symbolic", "Mark as Resolved");
            for (b, kind) in [(&ours, 0), (&theirs, 1), (&mark, 2)] {
                let w = w.clone();
                let path = path.clone();
                b.connect_clicked(move |_| {
                    let Some(v) = w.upgrade() else { return };
                    let path = path.clone();
                    v.ctx.git(
                        "Resolve Conflict",
                        move |l| match kind {
                            0 => git_backend::resolve_with(l, &path, true),
                            1 => git_backend::resolve_with(l, &path, false),
                            _ => git_backend::stage(l, &[&path]),
                        },
                        |_| {},
                    );
                });
                row_box.append(b);
            }
        } else if staged {
            let b = util::icon_button("list-remove-symbolic", "Unstage");
            b.connect_clicked(move |_| {
                if let Some(v) = w.upgrade() {
                    let path = path.clone();
                    v.ctx.git(
                        "Unstage",
                        move |l| git_backend::unstage(l, &[&path]),
                        |_| {},
                    );
                }
            });
            row_box.append(&b);
        } else {
            let discard = util::icon_button("edit-undo-symbolic", "Discard Changes");
            let change = f.clone();
            let w2 = w.clone();
            discard.connect_clicked(move |_| {
                let Some(v) = w2.upgrade() else { return };
                let change = change.clone();
                let ctx = v.ctx.clone();
                glib::spawn_future_local(async move {
                    let body = if change.untracked() {
                        format!("“{}” is untracked and will be deleted.", change.path)
                    } else {
                        format!("Changes to “{}” will be lost.", change.path)
                    };
                    if util::confirm(&ctx.parent(), "Discard Changes?", &body, "Discard", true)
                        .await
                    {
                        ctx.git("Discard", move |l| git_backend::discard(l, &change), |_| {});
                    }
                });
            });
            let stage = util::icon_button("list-add-symbolic", "Stage");
            stage.connect_clicked(move |_| {
                if let Some(v) = w.upgrade() {
                    let path = path.clone();
                    v.ctx
                        .git("Stage", move |l| git_backend::stage(l, &[&path]), |_| {});
                }
            });
            row_box.append(&discard);
            row_box.append(&stage);
        }
        let row = gtk::ListBoxRow::builder().child(&row_box).build();
        row.set_widget_name(&f.path);
        row
    }

    fn load_diff(self: &Rc<Self>) {
        let Some((path, staged)) = self.selected.borrow().clone() else {
            return;
        };
        let Some(change) = self.files.borrow().iter().find(|f| f.path == path).cloned() else {
            return;
        };
        let mode = if change.conflicted {
            HunkMode::None
        } else if staged {
            HunkMode::Staged
        } else {
            HunkMode::Unstaged
        };
        let w = Rc::downgrade(self);
        self.ctx.load(
            move |l| {
                if change.conflicted {
                    l.git_lenient(&["diff", "--no-color", "--", &change.path])
                } else {
                    git_backend::file_diff(l, &change, staged)
                }
            },
            move |r| {
                let Some(v) = w.upgrade() else { return };
                // Ignore stale results for a file that is no longer selected.
                if v.selected.borrow().as_ref() != Some(&(path, staged)) {
                    return;
                }
                match r {
                    Ok(d) => v.diff.set_diff(&d, mode),
                    Err(e) => v.diff.set_placeholder(
                        "dialog-error-symbolic",
                        "Cannot Show Diff",
                        &glib::markup_escape_text(&e.to_string()),
                    ),
                }
            },
        );
    }
}

impl View for Changes {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn refresh(self: Rc<Self>) {
        let Some(this) = self
            .ctx
            .status
            .borrow()
            .as_ref()
            .map(|s| (s.files.clone(), s.state))
        else {
            return;
        };
        let (files, state) = this;
        let rc = self.clone();
        *self.files.borrow_mut() = files.clone();

        self.suppress.set(true);
        util::clear_rows(&self.conflicts);
        util::clear_rows(&self.staged);
        util::clear_rows(&self.unstaged);
        let (mut nc, mut ns, mut nu) = (0, 0, 0);
        for f in &files {
            if f.conflicted {
                self.conflicts.append(&rc.file_row(f, false));
                nc += 1;
            }
            if f.staged() {
                self.staged.append(&rc.file_row(f, true));
                ns += 1;
            }
            if f.unstaged() {
                self.unstaged.append(&rc.file_row(f, false));
                nu += 1;
            }
        }
        self.conflicts_box.set_visible(nc > 0);
        self.staged_box.set_visible(ns > 0);
        self.unstaged_box.set_visible(nu > 0);
        self.empty.set_visible(nc + ns + nu == 0);
        self.staged_title
            .set_label(&format!("Staged Changes ({ns})"));
        self.unstaged_title.set_label(&format!("Changes ({nu})"));

        // Keep the selection on the same file if it is still listed.
        let sel = self.selected.borrow().clone();
        let mut found = false;
        if let Some((path, staged)) = &sel {
            for list in [&self.conflicts, &self.staged, &self.unstaged] {
                let in_staged = list == &self.staged;
                if in_staged != *staged && list != &self.conflicts {
                    continue;
                }
                let mut child = list.first_child();
                while let Some(c) = child {
                    if c.widget_name() == path.as_str()
                        && let Ok(row) = c.clone().downcast::<gtk::ListBoxRow>()
                    {
                        list.select_row(Some(&row));
                        found = true;
                        break;
                    }
                    child = c.next_sibling();
                }
                if found {
                    break;
                }
            }
        }
        self.suppress.set(false);
        if found {
            rc.load_diff();
        } else {
            *self.selected.borrow_mut() = None;
            self.diff.set_placeholder(
                "document-edit-symbolic",
                if nc + ns + nu == 0 {
                    "Nothing to Commit"
                } else {
                    "No File Selected"
                },
                if nc + ns + nu == 0 {
                    "Changes you make in this repository will appear here."
                } else {
                    "Select a changed file to see its diff."
                },
            );
        }

        let (title, revealed) = match state {
            RepoState::Clean => ("", false),
            RepoState::Merging => ("Merge in progress — resolve conflicts, then commit", true),
            RepoState::Rebasing => (
                "Rebase in progress — resolve conflicts, then continue",
                true,
            ),
            RepoState::CherryPicking => ("Cherry-pick in progress", true),
            RepoState::Reverting => ("Revert in progress", true),
        };
        self.banner.set_title(title);
        self.banner.set_revealed(revealed);
        self.update_commit_button();
    }
}
