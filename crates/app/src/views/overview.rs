//! Repository overview on GitHub: description, stats, README, star/fork and settings.

use std::rc::Rc;

use adw::prelude::*;
use gh_backend::Repo;
use gtk::glib;

use super::github::{self, Shell};
use crate::form::{self, Field};
use crate::repo::{Ctx, View};
use crate::util;

pub struct Overview {
    ctx: Rc<Ctx>,
    root: adw::NavigationView,
    shell: Shell,
    content: gtk::Box,
}

pub fn new(ctx: &Rc<Ctx>) -> Rc<dyn View> {
    let content = util::vbox(18);
    let shell = Shell::new(&util::page_scroller(&content));
    let root = adw::NavigationView::new();
    let view = Rc::new(Overview {
        ctx: ctx.clone(),
        root: root.clone(),
        shell,
        content,
    });
    let w = Rc::downgrade(&view);
    let refresh = github::refresh_button(move || {
        if let Some(v) = w.upgrade() {
            v.refresh();
        }
    });
    let c = ctx.clone();
    let browser = github::browser_button(ctx, move || {
        c.slug
            .borrow()
            .as_ref()
            .map(|s| format!("https://github.com/{s}"))
            .unwrap_or_default()
    });
    root.add(&github::nav_page(
        "Overview",
        &view.shell.stack,
        &[&browser, &refresh],
    ));
    view
}

impl Overview {
    fn fill(self: &Rc<Self>, repo: &Repo, readme: Option<String>) {
        let c = &self.content;
        util::clear_box(c);
        c.append(&util::heading(&repo.name_with_owner, "title-1"));
        if let Some(d) = repo.description.as_deref().filter(|d| !d.is_empty()) {
            c.append(&util::body_label(d));
        }
        let pills = util::hbox(6);
        pills.append(&util::pill(
            if repo.is_private { "Private" } else { "Public" },
            "accent",
        ));
        if repo.is_fork {
            pills.append(&util::pill("Fork", ""));
        }
        if repo.is_archived {
            pills.append(&util::pill("Archived", "warning"));
        }
        let mut stats = vec![
            format!("★ {}", repo.stargazer_count),
            format!("{} forks", repo.fork_count),
        ];
        if let Some(l) = &repo.primary_language {
            stats.push(l.name.clone());
        }
        if let Some(l) = &repo.license_info {
            stats.push(l.name.clone());
        }
        if let Some(b) = &repo.default_branch_ref {
            stats.push(format!("default branch {}", b.name));
        }
        pills.append(
            &gtk::Label::builder()
                .label(stats.join(" · "))
                .wrap(true)
                .xalign(0.0)
                .css_classes(["dim-label"])
                .build(),
        );
        c.append(&pills);
        if let Some(h) = repo.homepage_url.as_deref().filter(|h| !h.is_empty()) {
            let link = gtk::LinkButton::with_label(h, h);
            link.set_halign(gtk::Align::Start);
            c.append(&link);
        }

        let actions = util::hbox(6);
        let starred = repo.viewer_has_starred;
        let star = util::text_button(
            if starred { "Unstar" } else { "Star" },
            if starred { "" } else { "suggested-action" },
        );
        let w = Rc::downgrade(self);
        star.connect_clicked(move |_| {
            if let Some(v) = w.upgrade() {
                let v2 = v.clone();
                v.ctx.gh(
                    "Star",
                    move |s| gh_backend::set_starred(s, !starred),
                    move |_| v2.clone().refresh(),
                );
            }
        });
        let fork = util::text_button("Fork", "");
        let ctx = self.ctx.clone();
        fork.connect_clicked(move |_| {
            let c = ctx.clone();
            ctx.gh("Fork", gh_backend::fork, move |_| {
                c.toast("Forked to your account")
            });
        });
        let edit = util::text_button("Edit Details…", "");
        let w = Rc::downgrade(self);
        let (desc, home) = (
            repo.description.clone().unwrap_or_default(),
            repo.homepage_url.clone().unwrap_or_default(),
        );
        edit.connect_clicked(move |_| {
            let Some(v) = w.upgrade() else { return };
            let (desc, home) = (desc.clone(), home.clone());
            glib::spawn_future_local(async move {
                let Some(f) = form::run(
                    &v.ctx.parent(),
                    "Edit Repository Details",
                    "",
                    "Save",
                    vec![
                        Field::entry("Description", &desc),
                        Field::entry("Website", &home),
                    ],
                )
                .await
                else {
                    return;
                };
                let (d, h) = (f[0].text(), f[1].text());
                let v2 = v.clone();
                v.ctx.gh(
                    "Edit Repository",
                    move |s| gh_backend::repo_edit(s, &d, &h),
                    move |_| v2.clone().refresh(),
                );
            });
        });
        actions.append(&star);
        actions.append(&fork);
        actions.append(&edit);
        c.append(&actions);

        if let Some(readme) = readme {
            c.append(&util::heading("README", "heading"));
            let card = util::vbox(0);
            card.add_css_class("card");
            card.add_css_class("comment-card");
            card.append(&util::body_label(&readme));
            c.append(&card);
        }

        // Danger zone
        let (group, list) = util::group("Danger Zone", None);
        let vis = util::row(
            "Change Visibility",
            if repo.is_private {
                "Make this repository public"
            } else {
                "Make this repository private"
            },
        );
        let vis_button = util::text_button(
            if repo.is_private {
                "Make Public…"
            } else {
                "Make Private…"
            },
            "destructive-action",
        );
        vis.add_suffix(&vis_button);
        let private = repo.is_private;
        let w = Rc::downgrade(self);
        vis_button.connect_clicked(move |_| {
            let Some(v) = w.upgrade() else { return };
            glib::spawn_future_local(async move {
                if util::confirm(
                    &v.ctx.parent(),
                    "Change Visibility?",
                    "Changing visibility can affect forks, stars and GitHub Pages.",
                    "Change",
                    true,
                )
                .await
                {
                    let v2 = v.clone();
                    v.ctx.gh(
                        "Change Visibility",
                        move |s| gh_backend::set_visibility(s, !private),
                        move |_| v2.clone().refresh(),
                    );
                }
            });
        });
        list.append(&vis);
        let archived = repo.is_archived;
        let arch = util::row(
            if archived {
                "Unarchive Repository"
            } else {
                "Archive Repository"
            },
            "Archived repositories are read-only",
        );
        let arch_button = util::text_button(
            if archived {
                "Unarchive…"
            } else {
                "Archive…"
            },
            "destructive-action",
        );
        arch.add_suffix(&arch_button);
        let w = Rc::downgrade(self);
        arch_button.connect_clicked(move |_| {
            let Some(v) = w.upgrade() else { return };
            glib::spawn_future_local(async move {
                if util::confirm(
                    &v.ctx.parent(),
                    if archived {
                        "Unarchive Repository?"
                    } else {
                        "Archive Repository?"
                    },
                    "You can change this again later.",
                    if archived { "Unarchive" } else { "Archive" },
                    true,
                )
                .await
                {
                    let v2 = v.clone();
                    v.ctx.gh(
                        "Archive",
                        move |s| gh_backend::set_archived(s, !archived),
                        move |_| v2.clone().refresh(),
                    );
                }
            });
        });
        list.append(&arch);
        c.append(&group);
    }
}

impl View for Overview {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn refresh(self: Rc<Self>) {
        if !self.shell.begin(&self.ctx) {
            return;
        }
        let this = self.clone();
        self.ctx.gh_load(
            |s| Ok((gh_backend::repo_view(s)?, gh_backend::readme(s).ok())),
            move |r| match r {
                Ok((repo, readme)) => {
                    this.fill(&repo, readme);
                    this.shell.ready();
                }
                Err(e) => this.shell.error(&e),
            },
        );
    }
}
