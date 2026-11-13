//! Browse the tree at HEAD (or any revision), view files, blame and per-file history.

use std::cell::RefCell;
use std::rc::Rc;

use crate::repo::{Ctx, View};
use crate::util;
use adw::prelude::*;

const MAX_VIEW: usize = 2 * 1024 * 1024;

pub struct Files {
    ctx: Rc<Ctx>,
    nav: adw::NavigationView,
    rev: RefCell<String>,
    root_list: gtk::ListBox,
    rev_entry: gtk::Entry,
}

pub fn new(ctx: &Rc<Ctx>) -> Rc<dyn View> {
    let rev_entry = gtk::Entry::builder()
        .text("HEAD")
        .placeholder_text("Revision (branch, tag or commit)")
        .hexpand(true)
        .build();
    let root_list = util::boxed_list();
    util::placeholder(&root_list, "Empty tree");
    let content = util::vbox(18);
    let top = util::hbox(6);
    top.append(
        &gtk::Label::builder()
            .label("Revision")
            .css_classes(["dim-label"])
            .build(),
    );
    top.append(&rev_entry);
    content.append(&top);
    content.append(&root_list);
    let page = super::github::nav_page("Files", &util::page_scroller(&content), &[]);
    let nav = adw::NavigationView::new();
    nav.add(&page);
    let view = Rc::new(Files {
        ctx: ctx.clone(),
        nav,
        rev: RefCell::new("HEAD".into()),
        root_list,
        rev_entry,
    });
    let w = Rc::downgrade(&view);
    view.rev_entry.connect_activate(move |e| {
        if let Some(v) = w.upgrade() {
            *v.rev.borrow_mut() = e.text().trim().to_owned();
            v.clone().refresh();
        }
    });
    view
}

impl Files {
    fn fill(self: &Rc<Self>, list: &gtk::ListBox, dir: String) {
        let rev = self.rev.borrow().clone();
        let w = Rc::downgrade(self);
        let list = list.clone();
        let d = dir.clone();
        self.ctx.load(
            move |l| git_backend::ls_tree(l, &rev, &d),
            move |r| {
                let Some(v) = w.upgrade() else { return };
                util::clear_rows(&list);
                let entries = match r {
                    Ok(e) => e,
                    Err(e) => {
                        util::placeholder(&list, &e.to_string());
                        return;
                    }
                };
                for e in entries {
                    let path = if dir.is_empty() {
                        e.name.clone()
                    } else {
                        format!("{dir}/{}", e.name)
                    };
                    let row = util::row(&e.name, &e.size.map(util::human_size).unwrap_or_default());
                    row.set_activatable(true);
                    row.add_prefix(&gtk::Image::from_icon_name(if e.is_dir {
                        "folder-symbolic"
                    } else {
                        "text-x-generic-symbolic"
                    }));
                    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
                    let w = Rc::downgrade(&v);
                    let is_dir = e.is_dir;
                    row.connect_activated(move |_| {
                        if let Some(v) = w.upgrade() {
                            if is_dir {
                                v.push_dir(path.clone());
                            } else {
                                v.push_file(path.clone());
                            }
                        }
                    });
                    list.append(&row);
                }
            },
        );
    }

    fn push_dir(self: &Rc<Self>, dir: String) {
        let list = util::boxed_list();
        let content = util::vbox(12);
        content.append(&list);
        let title = dir.rsplit('/').next().unwrap_or(&dir).to_owned();
        let page = super::github::nav_page(&title, &util::page_scroller(&content), &[]);
        self.nav.push(&page);
        self.fill(&list, dir);
    }

    fn push_file(self: &Rc<Self>, path: String) {
        let buffer = gtk::TextBuffer::new(None);
        let text = gtk::TextView::builder()
            .buffer(&buffer)
            .editable(false)
            .monospace(true)
            .top_margin(12)
            .bottom_margin(12)
            .left_margin(12)
            .right_margin(12)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .child(&text)
            .vexpand(true)
            .build();
        let blame = gtk::ToggleButton::builder().label("Blame").build();
        let history = util::text_button("History", "");
        let bar = util::hbox(6);
        bar.set_margin_top(6);
        bar.set_margin_bottom(6);
        bar.set_margin_start(12);
        bar.set_margin_end(12);
        bar.append(
            &gtk::Label::builder()
                .label(&path)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::Start)
                .selectable(true)
                .css_classes(["mono", "dim-label"])
                .build(),
        );
        bar.append(&blame);
        bar.append(&history);
        if self.ctx.slug.borrow().is_some() {
            let gh = util::icon_button("gh-external-symbolic", "Open on GitHub");
            let ctx = self.ctx.clone();
            let p = path.clone();
            let rev = self.rev.borrow().clone();
            gh.connect_clicked(move |_| {
                if let Some(slug) = ctx.slug.borrow().as_ref() {
                    let r = if rev == "HEAD" {
                        ctx.branch()
                    } else {
                        rev.clone()
                    };
                    util::open_url(
                        &ctx.parent(),
                        &format!("https://github.com/{slug}/blob/{r}/{p}"),
                    );
                }
            });
            bar.append(&gh);
        }
        let content = util::vbox(0);
        content.append(&bar);
        content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        content.append(&scroller);
        let title = path.rsplit('/').next().unwrap_or(&path).to_owned();
        let page = super::github::nav_page(&title, &content, &[]);
        self.nav.push(&page);

        let rev = self.rev.borrow().clone();
        let load = {
            let ctx = self.ctx.clone();
            let buffer = buffer.clone();
            let path = path.clone();
            move |blame: bool| {
                let (rev, path, buffer) = (rev.clone(), path.clone(), buffer.clone());
                ctx.load(
                    move |l| {
                        if blame {
                            git_backend::blame(l, &rev, &path)
                        } else {
                            let bytes = git_backend::file_at(l, &rev, &path)?;
                            if bytes.len() > MAX_VIEW {
                                return Ok(format!(
                                    "File too large to display ({}).",
                                    util::human_size(bytes.len() as u64)
                                ));
                            }
                            Ok(match String::from_utf8(bytes) {
                                Ok(s) => s,
                                Err(_) => "Binary file not shown.".into(),
                            })
                        }
                    },
                    move |r| buffer.set_text(&r.unwrap_or_else(|e| e.to_string())),
                );
            }
        };
        load(false);
        blame.connect_toggled(move |b| load(b.is_active()));
        let ctx = self.ctx.clone();
        history.connect_clicked(move |_| {
            let p = path.clone();
            let c = ctx.clone();
            ctx.load(
                move |l| {
                    l.git(&[
                        "log",
                        "--follow",
                        "--format=%h  %ad  %an%n    %s%n",
                        "--date=short",
                        "--",
                        &p,
                    ])
                },
                move |r| match r {
                    Ok(t) => util::text_dialog(&c.parent(), "File History", &t),
                    Err(e) => util::show_error(&c.parent(), "Cannot Load History", &e),
                },
            );
        });
    }
}

impl View for Files {
    fn widget(&self) -> gtk::Widget {
        self.nav.clone().upcast()
    }

    fn refresh(self: Rc<Self>) {
        // Only reload the root; open file pages keep what they show.
        let list = self.root_list.clone();
        self.fill(&list, String::new());
    }
}
