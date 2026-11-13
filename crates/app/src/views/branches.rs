//! Local and remote branches.

use std::rc::Rc;

use adw::prelude::*;
use git_backend::Branch;
use gtk::glib;

use crate::form::{self, Field};
use crate::repo::{Ctx, View};
use crate::util::{self, Callback};

pub struct Branches {
    ctx: Rc<Ctx>,
    root: gtk::ScrolledWindow,
    local: gtk::ListBox,
    remote: gtk::ListBox,
}

pub fn new(ctx: &Rc<Ctx>) -> Rc<dyn View> {
    let content = util::vbox(24);
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Filter branches")
        .hexpand(true)
        .build();
    let new_button = util::text_button("New Branch", "suggested-action");
    let top = util::hbox(12);
    top.append(&search);
    top.append(&new_button);
    content.append(&top);
    let (local_box, local) = util::group("Local Branches", None);
    let (remote_box, remote) = util::group("Remote Branches", None);
    util::placeholder(&local, "No local branches yet");
    util::placeholder(&remote, "No remote branches. Add a remote and fetch.");
    content.append(&local_box);
    content.append(&remote_box);
    util::filter_by_name(&search, &[local.clone(), remote.clone()]);
    let c = ctx.clone();
    new_button.connect_clicked(move |_| new_branch_dialog(&c, None));
    Rc::new(Branches {
        ctx: ctx.clone(),
        root: util::page_scroller(&content),
        local,
        remote,
    })
}

pub fn new_branch_dialog(ctx: &Rc<Ctx>, start: Option<String>) {
    let ctx = ctx.clone();
    glib::spawn_future_local(async move {
        let start_default = start.unwrap_or_else(|| ctx.branch());
        let Some(v) = form::run(
            &ctx.parent(),
            "New Branch",
            "",
            "Create",
            vec![
                Field::required("Name", ""),
                Field::entry("Start point", &start_default),
                Field::switch("Switch to the new branch", "", true),
            ],
        )
        .await
        else {
            return;
        };
        let (name, start, switch) = (v[0].text(), v[1].text(), v[2].bool());
        ctx.git(
            "Create Branch",
            move |l| {
                git_backend::create_branch(
                    l,
                    &name,
                    (!start.is_empty()).then_some(start.as_str()),
                    switch,
                )
            },
            |_| {},
        );
    });
}

fn cb(ctx: &Rc<Ctx>, b: &Branch, f: fn(Rc<Ctx>, Branch)) -> Callback {
    let ctx = ctx.clone();
    let b = b.clone();
    Rc::new(move || f(ctx.clone(), b.clone()))
}

fn merge(ctx: Rc<Ctx>, b: Branch) {
    let c = ctx.clone();
    let name = b.name.clone();
    ctx.git(
        "Merge",
        move |l| git_backend::merge(l, &b.name, false, false),
        move |_| c.toast(&format!("Merged {name}")),
    );
}

fn squash_merge(ctx: Rc<Ctx>, b: Branch) {
    let c = ctx.clone();
    ctx.git(
        "Squash Merge",
        move |l| git_backend::merge(l, &b.name, false, true),
        move |_| c.toast("Squashed changes are staged; commit them in Changes"),
    );
}

fn rebase(ctx: Rc<Ctx>, b: Branch) {
    let c = ctx.clone();
    ctx.git(
        "Rebase",
        move |l| git_backend::rebase(l, &b.name),
        move |_| c.toast("Rebased"),
    );
}

fn rename(ctx: Rc<Ctx>, b: Branch) {
    glib::spawn_future_local(async move {
        let Some(v) = form::run(
            &ctx.parent(),
            "Rename Branch",
            "",
            "Rename",
            vec![Field::required("New name", &b.name)],
        )
        .await
        else {
            return;
        };
        let new = v[0].text();
        ctx.git(
            "Rename Branch",
            move |l| git_backend::rename_branch(l, &b.name, &new),
            |_| {},
        );
    });
}

fn delete(ctx: Rc<Ctx>, b: Branch) {
    glib::spawn_future_local(async move {
        if b.remote {
            if !util::confirm(
                &ctx.parent(),
                "Delete Remote Branch?",
                &format!("“{}” will be deleted from the remote for everyone.", b.name),
                "Delete",
                true,
            )
            .await
            {
                return;
            }
            ctx.git(
                "Delete Remote Branch",
                move |l| git_backend::delete_remote_branch(l, &b.name),
                |_| {},
            );
            return;
        }
        let Some(v) = form::run(
            &ctx.parent(),
            "Delete Branch?",
            &format!("Delete the local branch “{}”.", b.name),
            "Delete",
            vec![Field::switch(
                "Force",
                "Delete even if it has commits not merged anywhere",
                false,
            )],
        )
        .await
        else {
            return;
        };
        let force = v[0].bool();
        ctx.git(
            "Delete Branch",
            move |l| git_backend::delete_branch(l, &b.name, force),
            |_| {},
        );
    });
}

fn push_branch(ctx: Rc<Ctx>, b: Branch) {
    let c = ctx.clone();
    ctx.git(
        "Push Branch",
        move |l| {
            let remote = b
                .upstream
                .as_deref()
                .and_then(|u| u.split_once('/').map(|(r, _)| r.to_owned()))
                .unwrap_or_else(|| "origin".into());
            l.git(&["push", "-u", &remote, &b.name]).map(drop)
        },
        move |_| c.toast("Pushed"),
    );
}

fn create_pr(ctx: Rc<Ctx>, b: Branch) {
    crate::views::pulls::create_dialog(&ctx, Some(b.name));
}

fn compare_on_github(ctx: Rc<Ctx>, b: Branch) {
    if let Some(slug) = ctx.slug.borrow().as_ref() {
        let name = if b.remote {
            b.name.split_once('/').map(|(_, n)| n).unwrap_or(&b.name)
        } else {
            &b.name
        };
        util::open_url(
            &ctx.parent(),
            &format!("https://github.com/{slug}/compare/{name}?expand=1"),
        );
    }
}

impl View for Branches {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn refresh(self: Rc<Self>) {
        let this = self.clone();
        self.ctx.load(git_backend::branches, move |r| {
            util::clear_rows(&this.local);
            util::clear_rows(&this.remote);
            let list = match r {
                Ok(l) => l,
                Err(e) => {
                    util::placeholder(&this.local, &e.to_string());
                    return;
                }
            };
            let github = this.ctx.slug.borrow().is_some();
            for b in list {
                let mut sub = format!("{} · {}", b.subject, util::relative_time(b.time));
                if let Some(u) = &b.upstream {
                    sub.push_str(&format!(" · tracks {u}"));
                }
                if !b.track.is_empty() {
                    sub.push_str(&format!(" {}", b.track));
                }
                let row = util::row(&b.name, &sub);
                row.set_widget_name(&b.name);
                row.add_prefix(&gtk::Image::from_icon_name(if b.is_head {
                    "object-select-symbolic"
                } else {
                    "gh-branch-symbolic"
                }));
                if b.is_head {
                    row.add_suffix(&util::pill("Current", "accent"));
                } else {
                    let label = if b.remote { "Check Out" } else { "Switch" };
                    let sw = util::text_button(label, "flat");
                    let ctx = this.ctx.clone();
                    let (name, remote) = (b.name.clone(), b.remote);
                    sw.connect_clicked(move |_| {
                        let name = name.clone();
                        ctx.git(
                            "Switch Branch",
                            move |l| git_backend::checkout(l, &name, remote),
                            |_| {},
                        );
                    });
                    row.add_suffix(&sw);
                }
                let ctx = &this.ctx;
                let mut items: Vec<(&str, Callback)> = Vec::new();
                if !b.is_head {
                    items.push(("Merge into Current Branch", cb(ctx, &b, merge)));
                    items.push((
                        "Squash and Merge into Current Branch",
                        cb(ctx, &b, squash_merge),
                    ));
                    items.push(("Rebase Current Branch onto This", cb(ctx, &b, rebase)));
                }
                if !b.remote {
                    items.push(("Push", cb(ctx, &b, push_branch)));
                    items.push(("Rename…", cb(ctx, &b, rename)));
                }
                if github {
                    if !b.remote {
                        items.push(("Create Pull Request…", cb(ctx, &b, create_pr)));
                    }
                    items.push(("Compare on GitHub", cb(ctx, &b, compare_on_github)));
                }
                if !b.is_head {
                    items.push(("Delete…", cb(ctx, &b, delete)));
                }
                row.add_suffix(&util::menu_button(items));
                if b.remote {
                    this.remote.append(&row);
                } else {
                    this.local.append(&row);
                }
            }
        });
    }
}
