//! Issues: list, create, comment, close and reopen.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gh_backend::IssueDetail;
use gtk::glib;

use super::github::{self, Shell};
use crate::form::{self, Field};
use crate::repo::{Ctx, View};
use crate::util::{self, Callback};

const STATES: [&str; 3] = ["open", "closed", "all"];

pub struct Issues {
    ctx: Rc<Ctx>,
    nav: adw::NavigationSplitView,
    shell: Shell,
    list: gtk::ListBox,
    state: gtk::DropDown,
    search: gtk::SearchEntry,
}

pub fn new(ctx: &Rc<Ctx>) -> Rc<dyn View> {
    let state = github::dropdown(&["Open", "Closed", "All"]);
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search, e.g. label:bug assignee:@me")
        .hexpand(true)
        .build();
    let top = util::hbox(12);
    top.append(&state);
    top.append(&search);
    let list = util::boxed_list();
    util::placeholder(&list, "No issues");
    let content = util::vbox(18);
    content.append(&top);
    content.append(&list);
    let shell = Shell::new(&util::page_scroller(&content));
    let nav = github::master_detail("gh-issue-symbolic", "Select an Issue");
    let view = Rc::new(Issues {
        ctx: ctx.clone(),
        nav: nav.clone(),
        shell,
        list,
        state,
        search,
    });
    let w = Rc::downgrade(&view);
    let reload = move || {
        if let Some(v) = w.upgrade() {
            v.refresh();
        }
    };
    let refresh = github::refresh_button(reload.clone());
    let new_button = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("New Issue")
        .build();
    let c = ctx.clone();
    let r = reload.clone();
    new_button.connect_clicked(move |_| {
        let ctx = c.clone();
        let reload = r.clone();
        glib::spawn_future_local(async move {
            let Some(v) = form::run(
                &ctx.parent(),
                "New Issue",
                "",
                "Create",
                vec![
                    Field::required("Title", ""),
                    Field::entry("Labels (comma-separated)", ""),
                    Field::text("Description", ""),
                ],
            )
            .await
            else {
                return;
            };
            let (title, labels, body) = (v[0].text(), v[1].text(), v[2].text());
            let c = ctx.clone();
            ctx.gh(
                "Create Issue",
                move |s| gh_backend::issue_create(s, &title, &body, &labels),
                move |url| {
                    c.toast(&format!("Created {url}"));
                    reload();
                },
            );
        });
    });
    github::set_list(
        &nav,
        &github::nav_page(
            "Issues",
            &view.shell.stack,
            &[new_button.upcast_ref(), &refresh],
        ),
    );
    let r = reload.clone();
    view.state.connect_selected_notify(move |_| r());
    view.search.connect_activate(move |_| reload());
    view
}

impl View for Issues {
    fn widget(&self) -> gtk::Widget {
        github::root_of(&self.nav)
    }

    fn refresh(self: Rc<Self>) {
        if !self.shell.begin(&self.ctx) {
            return;
        }
        let state = STATES[self.state.selected() as usize % 3];
        let search = self.search.text().to_string();
        let this = self.clone();
        self.ctx.gh_load(
            move |s| gh_backend::issue_list(s, state, &search),
            move |r| {
                let issues = match r {
                    Ok(i) => i,
                    Err(e) => return this.shell.error(&e),
                };
                util::clear_rows(&this.list);
                for i in issues {
                    let row = util::row(
                        &i.title,
                        &format!(
                            "#{} by {} · updated {}{}",
                            i.number,
                            i.author.login,
                            util::iso_relative(&i.updated_at),
                            if i.comments.is_empty() {
                                String::new()
                            } else {
                                format!(" · {} comments", i.comments.len())
                            }
                        ),
                    );
                    row.set_activatable(true);
                    let icon = gtk::Image::from_icon_name("gh-issue-symbolic");
                    icon.add_css_class(if i.state == "OPEN" {
                        "success"
                    } else {
                        "accent"
                    });
                    row.add_prefix(&icon);
                    for l in i.labels.iter().take(3) {
                        row.add_suffix(&util::pill(&l.name, "accent"));
                    }
                    let w = Rc::downgrade(&this);
                    let n = i.number;
                    row.connect_activated(move |_| {
                        if let Some(v) = w.upgrade() {
                            show_detail(&v.ctx, &v.nav, n);
                        }
                    });
                    this.list.append(&row);
                }
                this.shell.ready();
            },
        );
    }
}

fn show_detail(ctx: &Rc<Ctx>, nav: &adw::NavigationSplitView, number: u64) {
    let content = util::vbox(18);
    let shell = Rc::new(Shell::new(&util::page_scroller(&content)));
    let ctx2 = ctx.clone();
    let browser = github::browser_button(ctx, move || {
        ctx2.slug
            .borrow()
            .as_ref()
            .map(|s| format!("https://github.com/{s}/issues/{number}"))
            .unwrap_or_default()
    });
    let reload: Rc<RefCell<Option<Callback>>> = Rc::new(RefCell::new(None));
    let r2 = reload.clone();
    let refresh = github::refresh_button(move || {
        if let Some(f) = r2.borrow().clone() {
            f();
        }
    });
    github::show_detail(
        nav,
        &github::nav_page(&format!("#{number}"), &shell.stack, &[&browser, &refresh]),
    );
    let load: Callback = {
        let ctx = ctx.clone();
        let reload = reload.clone();
        Rc::new(move || {
            shell.begin(&ctx);
            let (ctx2, content, shell, reload) =
                (ctx.clone(), content.clone(), shell.clone(), reload.clone());
            ctx.gh_load(
                move |s| gh_backend::issue_view(s, number),
                move |r| match r {
                    Ok(i) => {
                        let again = reload.borrow().clone().expect("reload set");
                        fill(&ctx2, &content, &i, again);
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

fn fill(ctx: &Rc<Ctx>, content: &gtk::Box, i: &IssueDetail, reload: Callback) {
    util::clear_box(content);
    let n = i.number;
    content.append(&util::heading(&format!("{} #{}", i.title, n), "title-2"));
    let line = util::hbox(8);
    line.append(&util::state_pill(&i.state));
    let assignees: Vec<&str> = i.assignees.iter().map(|a| a.login.as_str()).collect();
    line.append(
        &gtk::Label::builder()
            .label(format!(
                "{} opened {}{}",
                i.author.login,
                util::iso_relative(&i.created_at),
                if assignees.is_empty() {
                    String::new()
                } else {
                    format!(" · assigned to {}", assignees.join(", "))
                }
            ))
            .wrap(true)
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build(),
    );
    content.append(&line);
    if let Some(l) = github::labels_row(&i.labels) {
        content.append(&l);
    }

    let actions = util::hbox(6);
    let open = i.state == "OPEN";
    let toggle = util::text_button(if open { "Close Issue" } else { "Reopen Issue" }, "");
    {
        let ctx = ctx.clone();
        let reload = reload.clone();
        toggle.connect_clicked(move |_| {
            let reload = reload.clone();
            ctx.gh(
                if open { "Close" } else { "Reopen" },
                move |s| gh_backend::issue_set_open(s, n, !open),
                move |_| reload(),
            );
        });
    }
    let edit = util::text_button("Edit…", "");
    {
        let ctx = ctx.clone();
        let reload = reload.clone();
        let (title, body) = (i.title.clone(), i.body.clone());
        edit.connect_clicked(move |_| {
            let (ctx, reload, title, body) =
                (ctx.clone(), reload.clone(), title.clone(), body.clone());
            glib::spawn_future_local(async move {
                let Some(v) = form::run(
                    &ctx.parent(),
                    "Edit Issue",
                    "",
                    "Save",
                    vec![
                        Field::required("Title", &title),
                        Field::text("Description", &body),
                    ],
                )
                .await
                else {
                    return;
                };
                let (t, b) = (v[0].text(), v[1].text());
                ctx.gh(
                    "Edit",
                    move |s| gh_backend::issue_edit(s, n, &t, &b),
                    move |_| reload(),
                );
            });
        });
    }
    let branch = util::text_button("Create Branch", "");
    {
        let ctx = ctx.clone();
        let slugish: String = i
            .title
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .take(6)
            .collect::<Vec<_>>()
            .join("-");
        let name = format!("{n}-{slugish}");
        branch.connect_clicked(move |_| {
            let name = name.clone();
            let c = ctx.clone();
            ctx.git(
                "Create Branch",
                move |l| git_backend::create_branch(l, &name, None, true),
                move |_| c.toast("Branch created for issue"),
            );
        });
    }
    actions.append(&toggle);
    actions.append(&edit);
    actions.append(&branch);
    content.append(&actions);

    let body = util::vbox(0);
    body.add_css_class("card");
    body.add_css_class("comment-card");
    body.append(&util::body_label(&i.body));
    content.append(&body);

    if !i.comments.is_empty() {
        content.append(&util::heading("Comments", "heading"));
        for c in &i.comments {
            content.append(&util::comment_card(
                &c.author.login,
                &util::iso_relative(&c.created_at),
                None,
                &c.body,
            ));
        }
    }
    let ctx2 = ctx.clone();
    let (composer, _) = github::composer("Comment", move |text| {
        let reload = reload.clone();
        ctx2.gh(
            "Comment",
            move |s| gh_backend::issue_comment(s, n, &text),
            move |_| reload(),
        );
    });
    content.append(&composer);
}
