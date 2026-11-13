//! Deployments and environments (read-only; via the REST API through `gh api`).

use std::rc::Rc;

use adw::prelude::*;

use super::github::{self, Shell};
use crate::repo::{Ctx, View};
use crate::util;

pub struct Deployments {
    ctx: Rc<Ctx>,
    shell: Shell,
    envs: gtk::ListBox,
    list: gtk::ListBox,
    root: adw::NavigationView,
}

pub fn new(ctx: &Rc<Ctx>) -> Rc<dyn View> {
    let content = util::vbox(24);
    let (env_group, envs) = util::group("Environments", None);
    let (dep_group, list) = util::group("Recent Deployments", None);
    util::placeholder(&envs, "No environments");
    util::placeholder(&list, "No deployments");
    content.append(&env_group);
    content.append(&dep_group);
    let shell = Shell::new(&util::page_scroller(&content));
    let root = adw::NavigationView::new();
    let view = Rc::new(Deployments {
        ctx: ctx.clone(),
        shell,
        envs,
        list,
        root: root.clone(),
    });
    let w = Rc::downgrade(&view);
    let refresh = github::refresh_button(move || {
        if let Some(v) = w.upgrade() {
            v.refresh();
        }
    });
    root.add(&github::nav_page(
        "Deployments",
        &view.shell.stack,
        &[&refresh],
    ));
    view
}

impl View for Deployments {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn refresh(self: Rc<Self>) {
        if !self.shell.begin(&self.ctx) {
            return;
        }
        let this = self.clone();
        self.ctx.gh_load(
            |s| {
                Ok((
                    gh_backend::environments(s).unwrap_or_default(),
                    gh_backend::deployments(s)?,
                ))
            },
            move |r| {
                let (envs, deps) = match r {
                    Ok(x) => x,
                    Err(e) => return this.shell.error(&e),
                };
                util::clear_rows(&this.envs);
                util::clear_rows(&this.list);
                for e in envs {
                    let row = util::row(
                        &e.name,
                        &format!("updated {}", util::iso_relative(&e.updated_at)),
                    );
                    row.add_prefix(&gtk::Image::from_icon_name("gh-deploy-symbolic"));
                    if !e.html_url.is_empty() {
                        row.set_activatable(true);
                        row.add_suffix(&gtk::Image::from_icon_name("gh-external-symbolic"));
                        let ctx = this.ctx.clone();
                        row.connect_activated(move |_| util::open_url(&ctx.parent(), &e.html_url));
                    }
                    this.envs.append(&row);
                }
                for d in deps {
                    let row = util::row(
                        &format!("{} · {}", d.environment, d.git_ref),
                        &format!(
                            "{} · {} · by {} · {}",
                            d.state,
                            d.sha.get(..7).unwrap_or(&d.sha),
                            d.creator
                                .as_ref()
                                .map(|c| c.login.as_str())
                                .unwrap_or("unknown"),
                            util::iso_relative(&d.created_at)
                        ),
                    );
                    row.add_prefix(&util::outcome_icon(match d.state.as_str() {
                        "success" | "active" => "success",
                        "failure" | "error" => "failure",
                        "inactive" => "inactive",
                        _ => "pending",
                    }));
                    if !d.environment_url.is_empty() {
                        let b = util::icon_button("gh-external-symbolic", "Open Deployment");
                        let ctx = this.ctx.clone();
                        let url = d.environment_url.clone();
                        b.connect_clicked(move |_| util::open_url(&ctx.parent(), &url));
                        row.add_suffix(&b);
                    }
                    this.list.append(&row);
                }
                this.shell.ready();
            },
        );
    }
}
