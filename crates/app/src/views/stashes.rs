//! Stashes: save, apply, pop, inspect and drop.

use std::rc::Rc;

use adw::prelude::*;
use git_backend::Stash;
use gtk::glib;

use crate::diff::{DiffView, HunkMode};
use crate::form::{self, Field};
use crate::repo::{Ctx, View};
use crate::util::{self, Callback};

pub struct Stashes {
    ctx: Rc<Ctx>,
    root: gtk::ScrolledWindow,
    list: gtk::ListBox,
}

pub fn new(ctx: &Rc<Ctx>) -> Rc<dyn View> {
    let content = util::vbox(24);
    let new_button = util::text_button("Stash Changes…", "suggested-action");
    let (group, list) = util::group("Stashes", Some(new_button.upcast_ref()));
    util::placeholder(
        &list,
        "No stashes. Stash your work-in-progress to switch tasks quickly.",
    );
    content.append(&group);
    let c = ctx.clone();
    new_button.connect_clicked(move |_| {
        let ctx = c.clone();
        glib::spawn_future_local(async move {
            let Some(v) = form::run(
                &ctx.parent(),
                "Stash Changes",
                "Save uncommitted changes and clean the working tree.",
                "Stash",
                vec![
                    Field::entry("Message", ""),
                    Field::switch("Include untracked files", "", true),
                ],
            )
            .await
            else {
                return;
            };
            let (msg, untracked) = (v[0].text(), v[1].bool());
            ctx.git(
                "Stash",
                move |l| git_backend::stash_push(l, &msg, untracked),
                |_| {},
            );
        });
    });
    Rc::new(Stashes {
        ctx: ctx.clone(),
        root: util::page_scroller(&content),
        list,
    })
}

fn cb(ctx: &Rc<Ctx>, s: &Stash, f: fn(Rc<Ctx>, Stash)) -> Callback {
    let ctx = ctx.clone();
    let s = s.clone();
    Rc::new(move || f(ctx.clone(), s.clone()))
}

fn pop(ctx: Rc<Ctx>, s: Stash) {
    ctx.git(
        "Pop Stash",
        move |l| git_backend::stash_apply(l, &s.reference, true),
        |_| {},
    );
}

fn show(ctx: Rc<Ctx>, s: Stash) {
    let c = ctx.clone();
    let title = s.message.clone();
    ctx.load(
        move |l| git_backend::stash_show(l, &s.reference),
        move |r| match r {
            Ok(d) => {
                let view = DiffView::new();
                view.set_diff(&d, HunkMode::None);
                util::content_dialog(&c.parent(), &title, &view.widget(), 1000, 720);
            }
            Err(e) => util::show_error(&c.parent(), "Cannot Show Stash", &e),
        },
    );
}

fn branch(ctx: Rc<Ctx>, s: Stash) {
    glib::spawn_future_local(async move {
        let Some(v) = form::run(
            &ctx.parent(),
            "Create Branch from Stash",
            "Creates a branch at the stash's base commit, applies the stash and drops it.",
            "Create",
            vec![Field::required("Branch name", "")],
        )
        .await
        else {
            return;
        };
        let name = v[0].text();
        ctx.git(
            "Create Branch",
            move |l| l.git(&["stash", "branch", &name, &s.reference]).map(drop),
            |_| {},
        );
    });
}

fn drop_stash(ctx: Rc<Ctx>, s: Stash) {
    glib::spawn_future_local(async move {
        if util::confirm(
            &ctx.parent(),
            "Drop Stash?",
            &format!("“{}” will be deleted.", s.message),
            "Drop",
            true,
        )
        .await
        {
            ctx.git(
                "Drop Stash",
                move |l| git_backend::stash_drop(l, &s.reference),
                |_| {},
            );
        }
    });
}

impl View for Stashes {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn refresh(self: Rc<Self>) {
        let this = self.clone();
        self.ctx.load(git_backend::stashes, move |r| {
            util::clear_rows(&this.list);
            for s in r.unwrap_or_default() {
                let row = util::row(
                    &s.message,
                    &format!("{} · {}", s.reference, util::relative_time(s.time)),
                );
                row.add_prefix(&gtk::Image::from_icon_name("gh-stash-symbolic"));
                let apply = util::text_button("Apply", "flat");
                let ctx = this.ctx.clone();
                let r = s.reference.clone();
                apply.connect_clicked(move |_| {
                    let r = r.clone();
                    ctx.git(
                        "Apply Stash",
                        move |l| git_backend::stash_apply(l, &r, false),
                        |_| {},
                    );
                });
                row.add_suffix(&apply);
                let ctx = &this.ctx;
                row.add_suffix(&util::menu_button(vec![
                    ("Show Changes", cb(ctx, &s, show)),
                    ("Pop (Apply and Drop)", cb(ctx, &s, pop)),
                    ("Create Branch from Stash…", cb(ctx, &s, branch)),
                    ("Drop…", cb(ctx, &s, drop_stash)),
                ]));
                this.list.append(&row);
            }
        });
    }
}
