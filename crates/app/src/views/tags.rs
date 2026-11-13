//! Tags: create, push, delete, and turn into GitHub releases.

use std::rc::Rc;

use adw::prelude::*;
use git_backend::Tag;
use gtk::glib;

use crate::form::{self, Field};
use crate::repo::{Ctx, View};
use crate::util::{self, Callback};

pub struct Tags {
    ctx: Rc<Ctx>,
    root: gtk::ScrolledWindow,
    list: gtk::ListBox,
}

pub fn new(ctx: &Rc<Ctx>) -> Rc<dyn View> {
    let content = util::vbox(18);
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Filter tags")
        .hexpand(true)
        .build();
    let new_button = util::text_button("New Tag…", "suggested-action");
    let top = util::hbox(12);
    top.append(&search);
    top.append(&new_button);
    content.append(&top);
    let (group, list) = util::group("Tags", None);
    util::placeholder(&list, "No tags");
    content.append(&group);
    util::filter_by_name(&search, std::slice::from_ref(&list));
    let c = ctx.clone();
    new_button.connect_clicked(move |_| {
        let ctx = c.clone();
        glib::spawn_future_local(async move {
            let Some(v) = form::run(
                &ctx.parent(),
                "New Tag",
                "",
                "Create",
                vec![
                    Field::required("Name", ""),
                    Field::entry("Target", "HEAD"),
                    Field::entry("Message (makes an annotated tag)", ""),
                    Field::switch("Push to origin", "", false),
                ],
            )
            .await
            else {
                return;
            };
            let (name, target, msg, push) = (v[0].text(), v[1].text(), v[2].text(), v[3].bool());
            ctx.git(
                "Create Tag",
                move |l| {
                    let target = if target.is_empty() {
                        "HEAD".into()
                    } else {
                        target
                    };
                    git_backend::create_tag(l, &name, &target, &msg)?;
                    if push {
                        git_backend::push_tag(l, "origin", &name)?;
                    }
                    Ok(())
                },
                |_| {},
            );
        });
    });
    Rc::new(Tags {
        ctx: ctx.clone(),
        root: util::page_scroller(&content),
        list,
    })
}

fn cb(ctx: &Rc<Ctx>, t: &Tag, f: fn(Rc<Ctx>, Tag)) -> Callback {
    let ctx = ctx.clone();
    let t = t.clone();
    Rc::new(move || f(ctx.clone(), t.clone()))
}

fn push(ctx: Rc<Ctx>, t: Tag) {
    let c = ctx.clone();
    ctx.git(
        "Push Tag",
        move |l| git_backend::push_tag(l, "origin", &t.name),
        move |_| c.toast("Tag pushed"),
    );
}

fn checkout(ctx: Rc<Ctx>, t: Tag) {
    ctx.git(
        "Checkout",
        move |l| git_backend::checkout_detached(l, &format!("refs/tags/{}", t.name)),
        |_| {},
    );
}

fn delete(ctx: Rc<Ctx>, t: Tag) {
    glib::spawn_future_local(async move {
        let Some(v) = form::run(
            &ctx.parent(),
            "Delete Tag?",
            &format!("Delete the tag “{}”.", t.name),
            "Delete",
            vec![Field::switch("Also delete from origin", "", false)],
        )
        .await
        else {
            return;
        };
        let remote = v[0].bool();
        ctx.git(
            "Delete Tag",
            move |l| {
                if remote {
                    git_backend::delete_remote_tag(l, "origin", &t.name)?;
                }
                git_backend::delete_tag(l, &t.name)
            },
            |_| {},
        );
    });
}

fn release(ctx: Rc<Ctx>, t: Tag) {
    crate::views::releases::create_dialog(&ctx, Some(t.name));
}

impl View for Tags {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn refresh(self: Rc<Self>) {
        let this = self.clone();
        self.ctx.load(git_backend::tags, move |r| {
            util::clear_rows(&this.list);
            let github = this.ctx.slug.borrow().is_some();
            for t in r.unwrap_or_default() {
                let row = util::row(
                    &t.name,
                    &format!(
                        "{} · {} · {}",
                        t.subject,
                        t.target,
                        util::relative_time(t.time)
                    ),
                );
                row.set_widget_name(&t.name);
                row.add_prefix(&gtk::Image::from_icon_name("gh-tag-symbolic"));
                let ctx = &this.ctx;
                let mut items: Vec<(&str, Callback)> = vec![
                    ("Push to Origin", cb(ctx, &t, push)),
                    ("Check Out", cb(ctx, &t, checkout)),
                ];
                if github {
                    items.push(("Create Release…", cb(ctx, &t, release)));
                }
                items.push(("Delete…", cb(ctx, &t, delete)));
                row.add_suffix(&util::menu_button(items));
                this.list.append(&row);
            }
        });
    }
}
