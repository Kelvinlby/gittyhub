//! Remotes, repository-local identity, and publishing to GitHub.

use std::rc::Rc;

use adw::prelude::*;
use git_backend::Remote;
use gtk::glib;

use crate::form::{self, Field};
use crate::repo::{Ctx, View};
use crate::util::{self, Callback};

pub struct Remotes {
    ctx: Rc<Ctx>,
    root: gtk::ScrolledWindow,
    list: gtk::ListBox,
    name_row: adw::EntryRow,
    email_row: adw::EntryRow,
    publish_box: gtk::Box,
}

pub fn new(ctx: &Rc<Ctx>) -> Rc<dyn View> {
    let content = util::vbox(24);
    let add = util::text_button("Add Remote…", "");
    let (group, list) = util::group("Remotes", Some(add.upcast_ref()));
    util::placeholder(&list, "No remotes configured");
    content.append(&group);

    // Publish to GitHub (shown when no GitHub remote exists).
    let publish_box = util::vbox(8);
    let publish_list = util::boxed_list();
    let publish_row = util::row(
        "Publish to GitHub",
        "Create a GitHub repository with gh, add it as “origin” and push the current branch",
    );
    publish_row.add_prefix(&gtk::Image::from_icon_name("gh-github-symbolic"));
    let publish = util::text_button("Publish…", "suggested-action");
    publish_row.add_suffix(&publish);
    publish_list.append(&publish_row);
    publish_box.append(&util::heading("GitHub", "heading"));
    publish_box.append(&publish_list);
    content.append(&publish_box);

    let (id_group, id_list) = util::group("Identity for This Repository", None);
    let name_row = adw::EntryRow::builder()
        .title("Name (user.name)")
        .show_apply_button(true)
        .build();
    let email_row = adw::EntryRow::builder()
        .title("Email (user.email)")
        .show_apply_button(true)
        .build();
    id_list.append(&name_row);
    id_list.append(&email_row);
    content.append(&id_group);

    let c = ctx.clone();
    add.connect_clicked(move |_| {
        let ctx = c.clone();
        glib::spawn_future_local(async move {
            let Some(v) = form::run(
                &ctx.parent(),
                "Add Remote",
                "",
                "Add",
                vec![
                    Field::required("Name", "origin"),
                    Field::required("URL", ""),
                    Field::switch("Fetch now", "", true),
                ],
            )
            .await
            else {
                return;
            };
            let (name, url, fetch) = (v[0].text(), v[1].text(), v[2].bool());
            ctx.git(
                "Add Remote",
                move |l| {
                    git_backend::add_remote(l, &name, &url)?;
                    if fetch {
                        l.git(&["fetch", &name])?;
                    }
                    Ok(())
                },
                |_| {},
            );
        });
    });
    for (row, key) in [(&name_row, "user.name"), (&email_row, "user.email")] {
        let c = ctx.clone();
        row.connect_apply(move |r| {
            let v = r.text().to_string();
            let c2 = c.clone();
            c.git(
                "Save Identity",
                move |l| git_backend::config_set(l, key, &v),
                move |_| c2.toast("Saved"),
            );
        });
    }
    let c = ctx.clone();
    publish.connect_clicked(move |_| publish_dialog(c.clone()));

    Rc::new(Remotes {
        ctx: ctx.clone(),
        root: util::page_scroller(&content),
        list,
        name_row,
        email_row,
        publish_box,
    })
}

fn publish_dialog(ctx: Rc<Ctx>) {
    glib::spawn_future_local(async move {
        let Some(v) = form::run(
            &ctx.parent(),
            "Publish to GitHub",
            "",
            "Publish",
            vec![
                Field::required("Name", &ctx.loc.name()),
                Field::entry("Description", ""),
                Field::switch("Private", "Only you and collaborators can see it", true),
            ],
        )
        .await
        else {
            return;
        };
        let (name, desc, private) = (v[0].text(), v[1].text(), v[2].bool());
        let c = ctx.clone();
        ctx.git(
            "Publish",
            move |l| {
                let mut args = vec![
                    "repo",
                    "create",
                    &name,
                    if private { "--private" } else { "--public" },
                ];
                if !desc.is_empty() {
                    args.extend(["--description", &desc]);
                }
                let url = gh_backend::run(&args, None)?.trim().to_owned();
                let slug = git_backend::github_slug(&url)
                    .ok_or_else(|| anyhow::anyhow!("unexpected output from gh: {url}"))?;
                let ssh = gh_backend::run(&["config", "get", "git_protocol"], None)
                    .map(|s| s.trim() == "ssh")
                    .unwrap_or(false);
                let remote_url = if ssh {
                    format!("git@github.com:{slug}.git")
                } else {
                    format!("https://github.com/{slug}.git")
                };
                git_backend::add_remote(l, "origin", &remote_url)?;
                if l.git_ok(&["rev-parse", "--verify", "HEAD"]) {
                    git_backend::push(l, false)?;
                }
                Ok(slug)
            },
            move |slug| c.toast(&format!("Published as {slug}")),
        );
    });
}

fn cb(ctx: &Rc<Ctx>, r: &Remote, f: fn(Rc<Ctx>, Remote)) -> Callback {
    let ctx = ctx.clone();
    let r = r.clone();
    Rc::new(move || f(ctx.clone(), r.clone()))
}

fn edit(ctx: Rc<Ctx>, r: Remote) {
    glib::spawn_future_local(async move {
        let Some(v) = form::run(
            &ctx.parent(),
            "Edit Remote",
            "",
            "Save",
            vec![
                Field::required("Name", &r.name),
                Field::required("URL", &r.fetch_url),
            ],
        )
        .await
        else {
            return;
        };
        let (name, url) = (v[0].text(), v[1].text());
        ctx.git(
            "Edit Remote",
            move |l| {
                if url != r.fetch_url {
                    git_backend::set_remote_url(l, &r.name, &url)?;
                }
                if name != r.name {
                    git_backend::rename_remote(l, &r.name, &name)?;
                }
                Ok(())
            },
            |_| {},
        );
    });
}

fn fetch(ctx: Rc<Ctx>, r: Remote) {
    let c = ctx.clone();
    ctx.git(
        "Fetch",
        move |l| l.git(&["fetch", "--prune", &r.name]).map(drop),
        move |_| c.toast("Fetched"),
    );
}

fn remove(ctx: Rc<Ctx>, r: Remote) {
    glib::spawn_future_local(async move {
        if util::confirm(
            &ctx.parent(),
            "Remove Remote?",
            &format!(
                "“{}” and its remote-tracking branches will be removed locally.",
                r.name
            ),
            "Remove",
            true,
        )
        .await
        {
            ctx.git(
                "Remove Remote",
                move |l| git_backend::remove_remote(l, &r.name),
                |_| {},
            );
        }
    });
}

fn open(ctx: Rc<Ctx>, r: Remote) {
    if let Some(slug) = git_backend::github_slug(&r.fetch_url) {
        util::open_url(&ctx.parent(), &format!("https://github.com/{slug}"));
    }
}

impl View for Remotes {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn refresh(self: Rc<Self>) {
        let this = self.clone();
        self.ctx.load(
            |l| {
                Ok((
                    git_backend::remotes(l)?,
                    git_backend::config_get(l, "user.name").unwrap_or_default(),
                    git_backend::config_get(l, "user.email").unwrap_or_default(),
                ))
            },
            move |r| {
                util::clear_rows(&this.list);
                let Ok((remotes, name, email)) = r else {
                    return;
                };
                this.name_row.set_text(&name);
                this.email_row.set_text(&email);
                this.publish_box
                    .set_visible(this.ctx.slug.borrow().is_none());
                for rem in remotes {
                    let sub = if rem.push_url != rem.fetch_url && !rem.push_url.is_empty() {
                        format!("{} (push: {})", rem.fetch_url, rem.push_url)
                    } else {
                        rem.fetch_url.clone()
                    };
                    let row = util::row(&rem.name, &sub);
                    let github = git_backend::github_slug(&rem.fetch_url).is_some();
                    row.add_prefix(&gtk::Image::from_icon_name(if github {
                        "gh-github-symbolic"
                    } else {
                        "gh-server-symbolic"
                    }));
                    let ctx = &this.ctx;
                    let mut items: Vec<(&str, Callback)> = vec![
                        ("Fetch", cb(ctx, &rem, fetch)),
                        ("Edit…", cb(ctx, &rem, edit)),
                    ];
                    if github {
                        items.push(("Open on GitHub", cb(ctx, &rem, open)));
                    }
                    items.push(("Remove…", cb(ctx, &rem, remove)));
                    row.add_suffix(&util::menu_button(items));
                    this.list.append(&row);
                }
            },
        );
    }
}
