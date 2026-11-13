//! GitHub releases: list, create, publish, delete, download and upload assets.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gh_backend::{NewRelease, ReleaseDetail};
use gtk::glib;

use super::github::{self, Shell};
use crate::form::{self, Field};
use crate::repo::{Ctx, View};
use crate::util::{self, Callback};

pub struct Releases {
    ctx: Rc<Ctx>,
    nav: adw::NavigationView,
    shell: Shell,
    list: gtk::ListBox,
}

pub fn new(ctx: &Rc<Ctx>) -> Rc<dyn View> {
    let list = util::boxed_list();
    util::placeholder(&list, "No releases yet");
    let content = util::vbox(18);
    content.append(&list);
    let shell = Shell::new(&util::page_scroller(&content));
    let nav = adw::NavigationView::new();
    let view = Rc::new(Releases {
        ctx: ctx.clone(),
        nav: nav.clone(),
        shell,
        list,
    });
    let w = Rc::downgrade(&view);
    let refresh = github::refresh_button(move || {
        if let Some(v) = w.upgrade() {
            v.refresh();
        }
    });
    let new_button = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("New Release")
        .build();
    let c = ctx.clone();
    new_button.connect_clicked(move |_| create_dialog(&c, None));
    nav.add(&github::nav_page(
        "Releases",
        &view.shell.stack,
        &[new_button.upcast_ref(), &refresh],
    ));
    view
}

pub fn create_dialog(ctx: &Rc<Ctx>, tag: Option<String>) {
    let ctx = ctx.clone();
    glib::spawn_future_local(async move {
        let Some(v) = form::run(
            &ctx.parent(),
            "New Release",
            "A missing tag is created on GitHub at the target.",
            "Create",
            vec![
                Field::required("Tag", tag.as_deref().unwrap_or("")),
                Field::entry("Target branch or commit (for a new tag)", &ctx.branch()),
                Field::entry("Title", tag.as_deref().unwrap_or("")),
                Field::switch(
                    "Generate release notes",
                    "Summarise merged pull requests automatically",
                    true,
                ),
                Field::switch("Draft", "Save without publishing", false),
                Field::switch("Pre-release", "Mark as not ready for production", false),
                Field::text("Notes", ""),
            ],
        )
        .await
        else {
            return;
        };
        let (tag, target, title, generate, draft, pre, notes) = (
            v[0].text(),
            v[1].text(),
            v[2].text(),
            v[3].bool(),
            v[4].bool(),
            v[5].bool(),
            v[6].text(),
        );
        let c = ctx.clone();
        ctx.gh(
            "Create Release",
            move |s| {
                gh_backend::release_create(
                    s,
                    &NewRelease {
                        tag: &tag,
                        target: &target,
                        title: if title.is_empty() { &tag } else { &title },
                        notes: &notes,
                        generate_notes: generate,
                        draft,
                        prerelease: pre,
                    },
                )
            },
            move |url| {
                c.toast(&format!("Created {url}"));
                c.navigate("releases");
                c.changed();
            },
        );
    });
}

impl View for Releases {
    fn widget(&self) -> gtk::Widget {
        self.nav.clone().upcast()
    }

    fn refresh(self: Rc<Self>) {
        if !self.shell.begin(&self.ctx) {
            return;
        }
        let this = self.clone();
        self.ctx.gh_load(gh_backend::release_list, move |r| {
            let list = match r {
                Ok(l) => l,
                Err(e) => return this.shell.error(&e),
            };
            util::clear_rows(&this.list);
            for rel in list {
                let when = rel.published_at.clone().unwrap_or(rel.created_at.clone());
                let title = if rel.name.is_empty() {
                    &rel.tag_name
                } else {
                    &rel.name
                };
                let row = util::row(
                    title,
                    &format!("{} · {}", rel.tag_name, util::iso_relative(&when)),
                );
                row.set_activatable(true);
                row.add_prefix(&gtk::Image::from_icon_name("gh-tag-symbolic"));
                if rel.is_latest {
                    row.add_suffix(&util::pill("Latest", "success"));
                }
                if rel.is_draft {
                    row.add_suffix(&util::pill("Draft", ""));
                }
                if rel.is_prerelease {
                    row.add_suffix(&util::pill("Pre-release", "warning"));
                }
                let w = Rc::downgrade(&this);
                let tag = rel.tag_name.clone();
                row.connect_activated(move |_| {
                    if let Some(v) = w.upgrade() {
                        show_detail(&v.ctx, &v.nav, tag.clone());
                    }
                });
                this.list.append(&row);
            }
            this.shell.ready();
        });
    }
}

fn show_detail(ctx: &Rc<Ctx>, nav: &adw::NavigationView, tag: String) {
    let content = util::vbox(18);
    let shell = Rc::new(Shell::new(&util::page_scroller(&content)));
    let url: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    let u = url.clone();
    let browser = github::browser_button(ctx, move || u.borrow().clone());
    nav.push(&github::nav_page(&tag, &shell.stack, &[&browser]));
    let reload: Rc<RefCell<Option<Callback>>> = Rc::new(RefCell::new(None));
    let load: Callback = {
        let (ctx, reload, nav) = (ctx.clone(), reload.clone(), nav.clone());
        Rc::new(move || {
            shell.begin(&ctx);
            let tag = tag.clone();
            let (ctx2, content, shell, reload, url, nav) = (
                ctx.clone(),
                content.clone(),
                shell.clone(),
                reload.clone(),
                url.clone(),
                nav.clone(),
            );
            ctx.gh_load(
                move |s| gh_backend::release_view(s, &tag),
                move |r| match r {
                    Ok(d) => {
                        *url.borrow_mut() = d.url.clone();
                        let again = reload.borrow().clone().expect("reload set");
                        fill(&ctx2, &nav, &content, &d, again);
                        shell.ready();
                    }
                    Err(e) => shell.error(&e),
                },
            );
        })
    };
    *reload.borrow_mut() = Some(load.clone());
    load();
}

fn fill(
    ctx: &Rc<Ctx>,
    nav: &adw::NavigationView,
    content: &gtk::Box,
    d: &ReleaseDetail,
    reload: Callback,
) {
    util::clear_box(content);
    let tag = d.tag_name.clone();
    content.append(&util::heading(
        if d.name.is_empty() {
            &d.tag_name
        } else {
            &d.name
        },
        "title-2",
    ));
    let line = util::hbox(8);
    if d.is_draft {
        line.append(&util::pill("Draft", ""));
    }
    if d.is_prerelease {
        line.append(&util::pill("Pre-release", "warning"));
    }
    line.append(
        &gtk::Label::builder()
            .label(format!(
                "{} · by {}{}",
                d.tag_name,
                d.author.login,
                d.published_at
                    .as_deref()
                    .map(|p| format!(" · published {}", util::iso_relative(p)))
                    .unwrap_or_default()
            ))
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build(),
    );
    content.append(&line);

    let actions = util::hbox(6);
    if d.is_draft {
        let publish = util::text_button("Publish Release", "suggested-action");
        let (ctx, reload, tag) = (ctx.clone(), reload.clone(), tag.clone());
        publish.connect_clicked(move |_| {
            let (reload, tag) = (reload.clone(), tag.clone());
            ctx.gh(
                "Publish",
                move |s| gh_backend::release_publish(s, &tag),
                move |_| reload(),
            );
        });
        actions.append(&publish);
    }
    let upload = util::text_button("Upload Asset…", "");
    {
        let (ctx, reload, tag) = (ctx.clone(), reload.clone(), tag.clone());
        upload.connect_clicked(move |_| {
            let (ctx, reload, tag) = (ctx.clone(), reload.clone(), tag.clone());
            glib::spawn_future_local(async move {
                let dialog = gtk::FileDialog::builder().title("Upload Asset").build();
                let win = ctx.parent().root().and_downcast::<gtk::Window>();
                if let Ok(f) = dialog.open_future(win.as_ref()).await
                    && let Some(p) = f.path()
                {
                    let p = p.to_string_lossy().into_owned();
                    let c = ctx.clone();
                    ctx.gh(
                        "Upload Asset",
                        move |s| gh_backend::release_upload(s, &tag, &p),
                        move |_| {
                            c.toast("Asset uploaded");
                            reload();
                        },
                    );
                }
            });
        });
    }
    actions.append(&upload);
    let delete = util::text_button("Delete…", "destructive-action");
    {
        let (ctx, nav, tag) = (ctx.clone(), nav.clone(), tag.clone());
        delete.connect_clicked(move |_| {
            let (ctx, nav, tag) = (ctx.clone(), nav.clone(), tag.clone());
            glib::spawn_future_local(async move {
                let Some(v) = form::run(
                    &ctx.parent(),
                    "Delete Release?",
                    &format!("The release “{tag}” will be deleted from GitHub."),
                    "Delete",
                    vec![Field::switch("Also delete the tag", "", false)],
                )
                .await
                else {
                    return;
                };
                let cleanup = v[0].bool();
                let c = ctx.clone();
                ctx.gh(
                    "Delete Release",
                    move |s| gh_backend::release_delete(s, &tag, cleanup),
                    move |_| {
                        nav.pop();
                        c.changed();
                    },
                );
            });
        });
    }
    actions.append(&delete);
    content.append(&actions);

    let body = util::vbox(0);
    body.add_css_class("card");
    body.add_css_class("comment-card");
    body.append(&util::body_label(&d.body));
    content.append(&body);

    let (group, list) = util::group("Assets", None);
    util::placeholder(&list, "No assets");
    for a in &d.assets {
        let row = util::row(
            &a.name,
            &format!(
                "{} · {} downloads",
                util::human_size(a.size),
                a.download_count
            ),
        );
        row.add_prefix(&gtk::Image::from_icon_name("gh-package-symbolic"));
        let dl = util::icon_button("folder-download-symbolic", "Download to Downloads Folder");
        let (ctx, tag, name) = (ctx.clone(), tag.clone(), a.name.clone());
        dl.connect_clicked(move |_| {
            let dir = glib::user_special_dir(glib::UserDirectory::Downloads)
                .unwrap_or_else(glib::home_dir)
                .to_string_lossy()
                .into_owned();
            let (tag, name) = (tag.clone(), name.clone());
            let c = ctx.clone();
            let d2 = dir.clone();
            ctx.gh(
                "Download",
                move |s| gh_backend::release_download(s, &tag, &name, &dir),
                move |_| c.toast(&format!("Saved to {d2}")),
            );
        });
        row.add_suffix(&dl);
        list.append(&row);
    }
    content.append(&group);
}
