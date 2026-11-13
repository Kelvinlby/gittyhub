//! GitHub Packages linked to this repository (REST via `gh api`; needs `read:packages`).

use std::rc::Rc;

use adw::prelude::*;
use gh_backend::Package;

use super::github::{self, Shell};
use crate::repo::{Ctx, View};
use crate::util;

pub struct Packages {
    ctx: Rc<Ctx>,
    nav: adw::NavigationSplitView,
    shell: Shell,
    list: gtk::ListBox,
}

pub fn new(ctx: &Rc<Ctx>) -> Rc<dyn View> {
    let list = util::boxed_list();
    util::placeholder(&list, "No packages are published from this repository");
    let content = util::vbox(18);
    content.append(&list);
    let shell = Shell::new(&util::page_scroller(&content));
    let nav = github::master_detail("gh-package-symbolic", "Select a Package");
    let view = Rc::new(Packages {
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
    github::set_list(
        &nav,
        &github::nav_page("Packages", &view.shell.stack, &[&refresh]),
    );
    view
}

impl View for Packages {
    fn widget(&self) -> gtk::Widget {
        github::root_of(&self.nav)
    }

    fn refresh(self: Rc<Self>) {
        if !self.shell.begin(&self.ctx) {
            return;
        }
        let this = self.clone();
        self.ctx.gh_load(gh_backend::packages, move |r| {
            let list = match r {
                Ok(l) => l,
                Err(e) if e.to_string().contains("read:packages") => {
                    return this.shell.show_status(
                        "gh-package-symbolic",
                        "Permission Needed",
                        "Listing packages needs the read:packages scope. Run “gh auth refresh -s read:packages” in a terminal, then refresh.",
                    );
                }
                Err(e) => return this.shell.error(&e),
            };
            util::clear_rows(&this.list);
            for p in list {
                let row = util::row(
                    &p.name,
                    &format!(
                        "{} · {} · {} versions · updated {}",
                        p.package_type,
                        p.visibility,
                        p.version_count,
                        util::iso_relative(&p.updated_at)
                    ),
                );
                row.set_activatable(true);
                row.add_prefix(&gtk::Image::from_icon_name("gh-package-symbolic"));
                row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
                let w = Rc::downgrade(&this);
                row.connect_activated(move |_| {
                    if let Some(v) = w.upgrade() {
                        show_versions(&v.ctx, &v.nav, p.clone());
                    }
                });
                this.list.append(&row);
            }
            this.shell.ready();
        });
    }
}

fn show_versions(ctx: &Rc<Ctx>, nav: &adw::NavigationSplitView, pkg: Package) {
    let list = util::boxed_list();
    util::placeholder(&list, "No versions");
    let content = util::vbox(12);
    content.append(&list);
    let shell = Rc::new(Shell::new(&util::page_scroller(&content)));
    let url = pkg.html_url.clone();
    let browser = github::browser_button(ctx, move || url.clone());
    github::show_detail(nav, &github::nav_page(&pkg.name, &shell.stack, &[&browser]));
    shell.begin(ctx);
    ctx.gh_load(
        move |s| gh_backend::package_versions(s, &pkg),
        move |r| match r {
            Ok(vs) => {
                for v in vs {
                    let row = util::row(
                        &v.name,
                        &format!("updated {}", util::iso_relative(&v.updated_at)),
                    );
                    row.add_prefix(&gtk::Image::from_icon_name("gh-tag-symbolic"));
                    list.append(&row);
                }
                shell.ready();
            }
            Err(e) => shell.error(&e),
        },
    );
}
