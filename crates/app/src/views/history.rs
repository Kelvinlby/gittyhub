//! Commit history with commit details and commit-level operations.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use git_backend::{Commit, LogQuery, ResetMode};
use gtk::glib;

use crate::diff::{DiffView, HunkMode};
use crate::form::{self, Field};
use crate::repo::{Ctx, View};
use crate::{settings, util};

const PAGE: usize = 200;

pub struct History {
    ctx: Rc<Ctx>,
    root: gtk::Paned,
    search: gtk::SearchEntry,
    all: gtk::ToggleButton,
    list: gtk::ListBox,
    more: gtk::Button,
    commits: RefCell<Vec<Commit>>,
    selected: RefCell<Option<String>>,
    detail_box: gtk::Box,
    detail_stack: gtk::Stack,
    diff: Rc<DiffView>,
    generation: Cell<u64>,
}

pub fn new(ctx: &Rc<Ctx>) -> Rc<dyn View> {
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search commit messages")
        .hexpand(true)
        .build();
    let all = gtk::ToggleButton::builder()
        .icon_name("gh-branch-symbolic")
        .tooltip_text("Show All Branches")
        .active(settings::get().history_all_branches)
        .build();
    let top = util::hbox(6);
    top.set_margin_top(6);
    top.set_margin_bottom(6);
    top.set_margin_start(6);
    top.set_margin_end(6);
    top.append(&search);
    top.append(&all);
    let list = gtk::ListBox::builder()
        .css_classes(["navigation-sidebar"])
        .build();
    let more = gtk::Button::builder()
        .label("Load More")
        .css_classes(["flat"])
        .margin_bottom(6)
        .visible(false)
        .build();
    let list_box = util::vbox(0);
    list_box.append(&list);
    list_box.append(&more);
    let left = util::vbox(0);
    left.append(&top);
    left.append(
        &gtk::ScrolledWindow::builder()
            .child(&list_box)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build(),
    );
    left.set_width_request(280);

    let detail_box = util::vbox(8);
    detail_box.set_margin_top(12);
    detail_box.set_margin_start(12);
    detail_box.set_margin_end(12);
    detail_box.set_margin_bottom(6);
    let diff = DiffView::new();
    let right = util::vbox(0);
    right.append(&detail_box);
    right.append(&diff.widget());
    let detail_stack = gtk::Stack::new();
    detail_stack.add_named(
        &util::status_page(
            "gh-commit-symbolic",
            "No Commit Selected",
            "Select a commit to see its changes.",
        ),
        Some("empty"),
    );
    detail_stack.add_named(&right, Some("detail"));

    let root = gtk::Paned::builder()
        .start_child(&left)
        .end_child(&detail_stack)
        .position(380)
        .shrink_start_child(false)
        .resize_start_child(false)
        .build();

    let view = Rc::new(History {
        ctx: ctx.clone(),
        root,
        search,
        all,
        list,
        more,
        commits: RefCell::new(Vec::new()),
        selected: RefCell::new(None),
        detail_box,
        detail_stack,
        diff,
        generation: Cell::new(0),
    });
    let w = Rc::downgrade(&view);
    view.search.connect_search_changed(glib::clone!(
        #[strong]
        w,
        move |_| {
            if let Some(v) = w.upgrade() {
                v.reload();
            }
        }
    ));
    view.all.connect_toggled(glib::clone!(
        #[strong]
        w,
        move |b| {
            let a = b.is_active();
            settings::update(|s| s.history_all_branches = a);
            if let Some(v) = w.upgrade() {
                v.reload();
            }
        }
    ));
    view.more.connect_clicked(glib::clone!(
        #[strong]
        w,
        move |_| {
            if let Some(v) = w.upgrade() {
                let skip = v.commits.borrow().len();
                v.load(skip);
            }
        }
    ));
    view.list.connect_row_selected(glib::clone!(
        #[strong]
        w,
        move |_, row| {
            if let (Some(v), Some(row)) = (w.upgrade(), row) {
                let oid = row.widget_name().to_string();
                if v.selected.borrow().as_deref() != Some(&oid) {
                    *v.selected.borrow_mut() = Some(oid.clone());
                    v.show_commit(oid);
                }
            }
        }
    ));
    view
}

impl History {
    fn reload(self: &Rc<Self>) {
        self.load(0);
    }

    fn load(self: &Rc<Self>, skip: usize) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let q = LogQuery {
            rev: None,
            all: self.all.is_active(),
            path: None,
            search: Some(self.search.text().to_string()),
            skip,
            limit: PAGE,
        };
        let w = Rc::downgrade(self);
        self.ctx.load(
            move |l| git_backend::log(l, &q),
            move |r| {
                let Some(v) = w.upgrade() else { return };
                if v.generation.get() != generation {
                    return;
                }
                let commits = match r {
                    Ok(c) => c,
                    Err(e) => {
                        v.ctx.toast(&format!("Could not load history: {e}"));
                        return;
                    }
                };
                if skip == 0 {
                    util::clear_rows(&v.list);
                    v.commits.borrow_mut().clear();
                }
                v.more.set_visible(commits.len() == PAGE);
                let selected = v.selected.borrow().clone();
                for c in &commits {
                    let row = commit_row(c);
                    v.list.append(&row);
                    if selected.as_deref() == Some(&c.oid) {
                        v.list.select_row(Some(&row));
                    }
                }
                v.commits.borrow_mut().extend(commits);
            },
        );
    }

    fn show_commit(self: &Rc<Self>, oid: String) {
        let w = Rc::downgrade(self);
        self.ctx.load(
            move |l| git_backend::show(l, &oid),
            move |r| {
                let Some(v) = w.upgrade() else { return };
                match r {
                    Ok(d) => {
                        if v.selected.borrow().as_deref() != Some(&d.commit.oid) {
                            return;
                        }
                        v.fill_detail(&d);
                        v.diff.set_diff(&d.patch, HunkMode::None);
                        v.detail_stack.set_visible_child_name("detail");
                    }
                    Err(e) => v.ctx.toast(&e.to_string()),
                }
            },
        );
    }

    fn fill_detail(self: &Rc<Self>, d: &git_backend::CommitDetail) {
        util::clear_box(&self.detail_box);
        let c = &d.commit;
        self.detail_box
            .append(&util::heading(&c.subject, "title-3"));
        if !d.body.is_empty() {
            self.detail_box.append(&util::body_label(&d.body));
        }
        let meta = gtk::Label::builder()
            .label(format!(
                "{} <{}> · {} · {}{}",
                c.author,
                c.email,
                util::absolute_time(c.time),
                c.short,
                if d.committer != c.author && !d.committer.is_empty() {
                    format!(" · committed by {}", d.committer)
                } else {
                    String::new()
                }
            ))
            .xalign(0.0)
            .wrap(true)
            .selectable(true)
            .css_classes(["dim-label", "caption"])
            .build();
        self.detail_box.append(&meta);

        let actions = util::hbox(6);
        let oid = c.oid.clone();
        let ctx = self.ctx.clone();
        let copy = util::text_button("Copy SHA", "");
        copy.connect_clicked(glib::clone!(
            #[strong]
            oid,
            #[strong]
            ctx,
            move |b| {
                util::copy_text(b, &oid);
                ctx.toast("Copied commit SHA");
            }
        ));
        let branch = util::text_button("New Branch…", "");
        branch.connect_clicked(glib::clone!(
            #[strong]
            oid,
            #[strong]
            ctx,
            move |_| crate::views::branches::new_branch_dialog(&ctx, Some(oid.clone()))
        ));
        actions.append(&copy);
        actions.append(&branch);
        let c2 = ctx.clone();
        let o = oid.clone();
        let mk = move |f: fn(Rc<Ctx>, String)| -> util::Callback {
            let ctx = c2.clone();
            let oid = o.clone();
            Rc::new(move || f(ctx.clone(), oid.clone()))
        };
        let mut items: Vec<(&str, util::Callback)> = vec![
            ("Check Out Commit", mk(checkout)),
            ("Create Tag…", mk(tag)),
            ("Cherry-pick onto Current Branch", mk(cherry_pick)),
            ("Revert Commit", mk(revert)),
            ("Reset Current Branch to Here…", mk(reset)),
        ];
        if self.ctx.slug.borrow().is_some() {
            items.push(("Open on GitHub", mk(open_on_github)));
        }
        let menu = util::menu_button(items);
        actions.append(&menu);
        self.detail_box.append(&actions);
    }
}

fn checkout(ctx: Rc<Ctx>, oid: String) {
    ctx.git(
        "Checkout",
        move |l| git_backend::checkout_detached(l, &oid),
        |_| {},
    );
}

fn cherry_pick(ctx: Rc<Ctx>, oid: String) {
    let c = ctx.clone();
    ctx.git(
        "Cherry-pick",
        move |l| git_backend::cherry_pick(l, &oid),
        move |_| c.toast("Cherry-picked"),
    );
}

fn revert(ctx: Rc<Ctx>, oid: String) {
    let c = ctx.clone();
    ctx.git(
        "Revert",
        move |l| git_backend::revert(l, &oid),
        move |_| c.toast("Reverted"),
    );
}

fn tag(ctx: Rc<Ctx>, oid: String) {
    glib::spawn_future_local(async move {
        let Some(v) = form::run(
            &ctx.parent(),
            "Create Tag",
            &format!("Tag commit {}", &oid[..oid.len().min(10)]),
            "Create",
            vec![
                Field::required("Tag name", ""),
                Field::entry("Message (makes an annotated tag)", ""),
            ],
        )
        .await
        else {
            return;
        };
        let (name, msg) = (v[0].text(), v[1].text());
        ctx.git(
            "Create Tag",
            move |l| git_backend::create_tag(l, &name, &oid, &msg),
            |_| {},
        );
    });
}

fn reset(ctx: Rc<Ctx>, oid: String) {
    glib::spawn_future_local(async move {
        let Some(v) = form::run(
            &ctx.parent(),
            "Reset Current Branch",
            "Move the current branch to this commit.",
            "Reset",
            vec![Field::choice(
                "Mode",
                &[
                    "Soft — keep changes staged",
                    "Mixed — keep changes unstaged",
                    "Hard — discard all changes",
                ],
                1,
            )],
        )
        .await
        else {
            return;
        };
        let mode = [ResetMode::Soft, ResetMode::Mixed, ResetMode::Hard][v[0].index().min(2)];
        if mode == ResetMode::Hard
            && !util::confirm(
                &ctx.parent(),
                "Discard Changes?",
                "A hard reset permanently discards uncommitted changes and moves the branch.",
                "Hard Reset",
                true,
            )
            .await
        {
            return;
        }
        ctx.git("Reset", move |l| git_backend::reset(l, &oid, mode), |_| {});
    });
}

fn open_on_github(ctx: Rc<Ctx>, oid: String) {
    if let Some(slug) = ctx.slug.borrow().as_ref() {
        util::open_url(
            &ctx.parent(),
            &format!("https://github.com/{slug}/commit/{oid}"),
        );
    }
}

fn commit_row(c: &Commit) -> gtk::ListBoxRow {
    let b = util::vbox(4);
    b.set_margin_top(4);
    b.set_margin_bottom(4);
    b.append(
        &gtk::Label::builder()
            .label(&c.subject)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    let meta = util::hbox(6);
    for r in c.refs.iter().take(4) {
        let (text, class) = if let Some(t) = r.strip_prefix("tag: ") {
            (t.to_owned(), "warning")
        } else if let Some(h) = r.strip_prefix("HEAD -> ") {
            (h.to_owned(), "accent")
        } else if r == "HEAD" {
            ("HEAD".to_owned(), "accent")
        } else {
            (r.clone(), "success")
        };
        meta.append(&util::pill(&text, class));
    }
    meta.append(
        &gtk::Label::builder()
            .label(format!(
                "{} · {} · {}",
                c.short,
                c.author,
                util::relative_time(c.time)
            ))
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["dim-label", "caption"])
            .build(),
    );
    b.append(&meta);
    let row = gtk::ListBoxRow::builder().child(&b).build();
    row.set_widget_name(&c.oid);
    row.set_tooltip_text(Some(&c.subject));
    row
}

impl View for History {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn refresh(self: Rc<Self>) {
        self.reload();
    }
}
