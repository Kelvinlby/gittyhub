//! Pull requests: list, create, review, merge, check out, comment.

use std::rc::Rc;

use adw::prelude::*;
use gh_backend::{MergeMethod, PullRequestDetail, ReviewKind};
use gtk::glib;

use super::github::{self, Shell};
use crate::diff::DiffView;
use crate::form::{self, Field};
use crate::repo::{Ctx, View};
use crate::util::{self, Callback};

const STATES: [&str; 4] = ["open", "closed", "merged", "all"];

pub struct Pulls {
    ctx: Rc<Ctx>,
    nav: adw::NavigationSplitView,
    shell: Shell,
    list: gtk::ListBox,
    state: gtk::DropDown,
    search: gtk::SearchEntry,
}

pub fn new(ctx: &Rc<Ctx>) -> Rc<dyn View> {
    let state = github::dropdown(&["Open", "Closed", "Merged", "All"]);
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search, e.g. author:@me label:bug")
        .hexpand(true)
        .build();
    let top = util::hbox(12);
    top.append(&state);
    top.append(&search);
    let list = util::boxed_list();
    util::placeholder(&list, "No pull requests");
    let content = util::vbox(18);
    content.append(&top);
    content.append(&list);
    let shell = Shell::new(&util::page_scroller(&content));
    let nav = github::master_detail("gh-pull-request-symbolic", "Select a Pull Request");

    let new_button = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("New Pull Request")
        .build();
    let view = Rc::new(Pulls {
        ctx: ctx.clone(),
        nav: nav.clone(),
        shell,
        list,
        state,
        search,
    });
    let w = Rc::downgrade(&view);
    let refresh = github::refresh_button(glib::clone!(
        #[strong]
        w,
        move || {
            if let Some(v) = w.upgrade() {
                v.refresh();
            }
        }
    ));
    let c = ctx.clone();
    new_button.connect_clicked(move |_| create_dialog(&c, None));
    github::set_list(
        &nav,
        &github::nav_page(
            "Pull Requests",
            &view.shell.stack,
            &[new_button.upcast_ref(), &refresh],
        ),
    );
    view.state.connect_selected_notify(glib::clone!(
        #[strong]
        w,
        move |_| {
            if let Some(v) = w.upgrade() {
                v.refresh();
            }
        }
    ));
    view.search.connect_activate(glib::clone!(
        #[strong]
        w,
        move |_| {
            if let Some(v) = w.upgrade() {
                v.refresh();
            }
        }
    ));
    view
}

impl View for Pulls {
    fn widget(&self) -> gtk::Widget {
        github::root_of(&self.nav)
    }

    fn refresh(self: Rc<Self>) {
        if !self.shell.begin(&self.ctx) {
            return;
        }
        let state = STATES[self.state.selected() as usize % 4];
        let search = self.search.text().to_string();
        let this = self.clone();
        self.ctx.gh_load(
            move |s| gh_backend::pr_list(s, state, &search),
            move |r| {
                let prs = match r {
                    Ok(p) => p,
                    Err(e) => return this.shell.error(&e),
                };
                util::clear_rows(&this.list);
                for pr in prs {
                    let row = util::row(
                        &pr.title,
                        &format!(
                            "#{} by {} · {} → {} · updated {}",
                            pr.number,
                            pr.author.login,
                            pr.head_ref_name,
                            pr.base_ref_name,
                            util::iso_relative(&pr.updated_at)
                        ),
                    );
                    row.set_activatable(true);
                    let icon = gtk::Image::from_icon_name("gh-pull-request-symbolic");
                    icon.add_css_class(match (pr.state.as_str(), pr.is_draft) {
                        (_, true) => "dim-label",
                        ("OPEN", _) => "success",
                        ("MERGED", _) => "accent",
                        _ => "error",
                    });
                    row.add_prefix(&icon);
                    if pr.is_draft {
                        row.add_suffix(&util::pill("Draft", ""));
                    }
                    if let Some(d) = review_pill(pr.review_decision.as_deref()) {
                        row.add_suffix(&d);
                    }
                    for l in pr.labels.iter().take(2) {
                        row.add_suffix(&util::pill(&l.name, "accent"));
                    }
                    let w = Rc::downgrade(&this);
                    let n = pr.number;
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

fn review_pill(decision: Option<&str>) -> Option<gtk::Label> {
    match decision? {
        "APPROVED" => Some(util::pill("Approved", "success")),
        "CHANGES_REQUESTED" => Some(util::pill("Changes requested", "error")),
        "REVIEW_REQUIRED" => Some(util::pill("Review required", "warning")),
        _ => None,
    }
}

/// Open the "new pull request" form, defaulting the head to `head` or the current branch.
pub fn create_dialog(ctx: &Rc<Ctx>, head: Option<String>) {
    let ctx = ctx.clone();
    glib::spawn_future_local(async move {
        let head = head.unwrap_or_else(|| ctx.branch());
        // Default title: the branch's latest commit subject.
        let h = head.clone();
        let subject = {
            let loc = ctx.loc.clone();
            gtk::gio::spawn_blocking(move || {
                loc.git(&["log", "-1", "--format=%s", &h])
                    .map(|s| s.trim().to_owned())
                    .unwrap_or_default()
            })
            .await
            .unwrap_or_default()
        };
        let Some(v) = form::run(
            &ctx.parent(),
            "New Pull Request",
            "The branch is pushed first if it has no upstream yet.",
            "Create",
            vec![
                Field::required("Title", &subject),
                Field::entry("Head branch", &head),
                Field::entry("Base branch (empty for default)", ""),
                Field::switch("Draft", "Open as a draft pull request", false),
                Field::text("Description", ""),
            ],
        )
        .await
        else {
            return;
        };
        let (title, head, base, draft, body) = (
            v[0].text(),
            v[1].text(),
            v[2].text(),
            v[3].bool(),
            v[4].text(),
        );
        let Some(slug) = ctx.slug.borrow().clone() else {
            return;
        };
        let c = ctx.clone();
        ctx.git(
            "Create Pull Request",
            move |l| {
                let has_upstream =
                    l.git_ok(&["rev-parse", "--abbrev-ref", &format!("{head}@{{upstream}}")]);
                if !has_upstream {
                    l.git(&["push", "-u", "origin", &head])?;
                } else {
                    // Make sure the remote has the latest commits.
                    let _ = l.git(&["push", "origin", &head]);
                }
                gh_backend::pr_create(&slug, &head, &base, &title, &body, draft)
            },
            move |url| {
                c.toast(&format!("Created {url}"));
                c.navigate("pulls");
            },
        );
    });
}

fn show_detail(ctx: &Rc<Ctx>, nav: &adw::NavigationSplitView, number: u64) {
    let content = util::vbox(18);
    let shell = Rc::new(Shell::new(&util::page_scroller(&content)));
    let ctx2 = ctx.clone();
    let browser = github::browser_button(ctx, move || {
        ctx2.slug
            .borrow()
            .as_ref()
            .map(|s| format!("https://github.com/{s}/pull/{number}"))
            .unwrap_or_default()
    });
    let reload: Rc<std::cell::RefCell<Option<Callback>>> = Rc::new(std::cell::RefCell::new(None));
    let r2 = reload.clone();
    let refresh = github::refresh_button(move || {
        if let Some(f) = r2.borrow().clone() {
            f();
        }
    });
    let page = github::nav_page(&format!("#{number}"), &shell.stack, &[&browser, &refresh]);
    github::show_detail(nav, &page);

    let load: Callback = {
        let ctx = ctx.clone();
        let content = content.clone();
        let nav = nav.clone();
        let shell = shell.clone();
        let reload = reload.clone();
        Rc::new(move || {
            shell.begin(&ctx);
            let ctx2 = ctx.clone();
            let content = content.clone();
            let nav = nav.clone();
            let shell = shell.clone();
            let reload = reload.clone();
            ctx.gh_load(
                move |s| gh_backend::pr_view(s, number),
                move |r| match r {
                    Ok(pr) => {
                        let again = reload.borrow().clone().expect("reload set");
                        fill_detail(&ctx2, &nav, &content, &pr, again);
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

fn fill_detail(
    ctx: &Rc<Ctx>,
    nav: &adw::NavigationSplitView,
    content: &gtk::Box,
    pr: &PullRequestDetail,
    reload: Callback,
) {
    util::clear_box(content);
    let n = pr.number;

    // Title and state
    let head = util::vbox(6);
    head.append(&util::heading(&format!("{} #{}", pr.title, n), "title-2"));
    let line = util::hbox(8);
    line.append(&util::state_pill(if pr.is_draft && pr.state == "OPEN" {
        "draft"
    } else {
        &pr.state
    }));
    if let Some(d) = review_pill(pr.review_decision.as_deref()) {
        line.append(&d);
    }
    line.append(
        &gtk::Label::builder()
            .label(format!(
                "{} wants to merge {} into {} · opened {} · +{} −{} in {} files",
                pr.author.login,
                pr.head_ref_name,
                pr.base_ref_name,
                util::iso_relative(&pr.created_at),
                pr.additions,
                pr.deletions,
                pr.changed_files
            ))
            .wrap(true)
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build(),
    );
    head.append(&line);
    if let Some(l) = github::labels_row(&pr.labels) {
        head.append(&l);
    }
    content.append(&head);

    // Actions
    let actions = util::hbox(6);
    let open = pr.state == "OPEN";
    if open {
        let merge_button = util::labeled_menu(
            "Merge",
            "suggested-action",
            vec![
                (
                    "Create a Merge Commit…",
                    merge_cb(ctx, n, MergeMethod::Merge, &reload),
                ),
                (
                    "Squash and Merge…",
                    merge_cb(ctx, n, MergeMethod::Squash, &reload),
                ),
                (
                    "Rebase and Merge…",
                    merge_cb(ctx, n, MergeMethod::Rebase, &reload),
                ),
            ],
        );
        merge_button.set_sensitive(!pr.is_draft);
        actions.append(&merge_button);
    }
    let diff_button = util::text_button("Files Changed", "");
    {
        let ctx = ctx.clone();
        let nav = nav.clone();
        diff_button.connect_clicked(move |_| show_diff(&ctx, &nav, n));
    }
    actions.append(&diff_button);
    let checkout = util::text_button("Check Out", "");
    {
        let ctx = ctx.clone();
        let (head, cross) = (pr.head_ref_name.clone(), pr.is_cross_repository);
        checkout.connect_clicked(move |_| {
            let head = head.clone();
            let c = ctx.clone();
            ctx.git(
                "Check Out Pull Request",
                move |l| {
                    if cross {
                        let local = format!("pr/{n}");
                        l.git(&[
                            "fetch",
                            "origin",
                            &format!("+pull/{n}/head:refs/heads/{local}"),
                        ])
                        .or_else(|_| l.git(&["fetch", "origin", &format!("pull/{n}/head")]))?;
                        git_backend::checkout(l, &local, false)
                    } else {
                        l.git(&["fetch", "origin", &head])?;
                        git_backend::checkout(l, &format!("origin/{head}"), true)
                    }
                },
                move |_| c.toast("Checked out pull request"),
            );
        });
    }
    actions.append(&checkout);

    let mut more: Vec<(&str, Callback)> = Vec::new();
    more.push(("Review…", review_cb(ctx, n, &reload)));
    more.push(("Edit…", edit_cb(ctx, pr, &reload)));
    if open {
        more.push((
            if pr.is_draft {
                "Mark Ready for Review"
            } else {
                "Convert to Draft"
            },
            simple_cb(ctx, &reload, "Update Pull Request", {
                let draft = !pr.is_draft;
                move |s| gh_backend::pr_set_draft(s, n, draft)
            }),
        ));
        more.push((
            "Close Pull Request",
            simple_cb(ctx, &reload, "Close", move |s| {
                gh_backend::pr_set_open(s, n, false)
            }),
        ));
    } else if pr.state == "CLOSED" {
        more.push((
            "Reopen Pull Request",
            simple_cb(ctx, &reload, "Reopen", move |s| {
                gh_backend::pr_set_open(s, n, true)
            }),
        ));
    }
    actions.append(&util::menu_button(more));
    content.append(&actions);

    // Body
    let body = util::vbox(0);
    body.add_css_class("card");
    body.add_css_class("comment-card");
    body.append(&util::body_label(&pr.body));
    content.append(&body);

    // Checks
    let checks = pr.checks();
    if !checks.is_empty() {
        let (group, list) = util::group("Checks", None);
        for c in checks {
            let row = util::row(&c.name, &c.outcome);
            row.add_prefix(&util::outcome_icon(&c.outcome));
            if let Some(url) = c.url {
                row.set_activatable(true);
                row.add_suffix(&gtk::Image::from_icon_name("gh-external-symbolic"));
                let ctx = ctx.clone();
                row.connect_activated(move |_| util::open_url(&ctx.parent(), &url));
            }
            list.append(&row);
        }
        content.append(&group);
    }

    // Conversation: reviews and comments in time order.
    let mut items: Vec<(String, gtk::Box)> = Vec::new();
    for r in &pr.reviews {
        let when = r.submitted_at.clone().unwrap_or_default();
        let (text, class) = match r.state.as_str() {
            "APPROVED" => ("approved", "success"),
            "CHANGES_REQUESTED" => ("requested changes", "error"),
            "COMMENTED" => ("reviewed", ""),
            other => (other, ""),
        };
        items.push((
            when.clone(),
            util::comment_card(
                &r.author.login,
                &util::iso_relative(&when),
                Some(util::pill(text, class)),
                &r.body,
            ),
        ));
    }
    for c in &pr.comments {
        items.push((
            c.created_at.clone(),
            util::comment_card(
                &c.author.login,
                &util::iso_relative(&c.created_at),
                None,
                &c.body,
            ),
        ));
    }
    items.sort_by(|a, b| a.0.cmp(&b.0));
    if !items.is_empty() {
        content.append(&util::heading("Conversation", "heading"));
        for (_, card) in items {
            content.append(&card);
        }
    }

    let ctx2 = ctx.clone();
    let reload2 = reload.clone();
    let (composer, buffer) = github::composer("Comment", move |text| {
        let reload = reload2.clone();
        ctx2.gh(
            "Comment",
            move |s| gh_backend::pr_comment(s, n, &text),
            move |_| reload(),
        );
    });
    let _ = buffer;
    content.append(&composer);
}

fn simple_cb(
    ctx: &Rc<Ctx>,
    reload: &Callback,
    what: &'static str,
    f: impl Fn(&str) -> anyhow::Result<()> + Clone + Send + 'static,
) -> Callback {
    let ctx = ctx.clone();
    let reload = reload.clone();
    Rc::new(move || {
        let reload = reload.clone();
        let f = f.clone();
        ctx.gh(what, move |s| f(s), move |_| reload());
    })
}

fn merge_cb(ctx: &Rc<Ctx>, n: u64, method: MergeMethod, reload: &Callback) -> Callback {
    let ctx = ctx.clone();
    let reload = reload.clone();
    Rc::new(move || {
        let ctx = ctx.clone();
        let reload = reload.clone();
        glib::spawn_future_local(async move {
            let what = match method {
                MergeMethod::Merge => "Merge",
                MergeMethod::Squash => "Squash and Merge",
                MergeMethod::Rebase => "Rebase and Merge",
            };
            let Some(v) = form::run(
                &ctx.parent(),
                &format!("{what} #{n}?"),
                "",
                what,
                vec![Field::switch("Delete branch after merging", "", true)],
            )
            .await
            else {
                return;
            };
            let delete = v[0].bool();
            let c = ctx.clone();
            ctx.gh(
                what,
                move |s| gh_backend::pr_merge(s, n, method, delete),
                move |_| {
                    c.toast("Pull request merged");
                    reload();
                },
            );
        });
    })
}

fn review_cb(ctx: &Rc<Ctx>, n: u64, reload: &Callback) -> Callback {
    let ctx = ctx.clone();
    let reload = reload.clone();
    Rc::new(move || {
        let ctx = ctx.clone();
        let reload = reload.clone();
        glib::spawn_future_local(async move {
            let Some(v) = form::run(
                &ctx.parent(),
                "Submit Review",
                "",
                "Submit",
                vec![
                    Field::choice("Verdict", &["Comment", "Approve", "Request changes"], 0),
                    Field::text("Review comment", ""),
                ],
            )
            .await
            else {
                return;
            };
            let kind = [
                ReviewKind::Comment,
                ReviewKind::Approve,
                ReviewKind::RequestChanges,
            ][v[0].index().min(2)];
            let body = v[1].text();
            ctx.gh(
                "Review",
                move |s| gh_backend::pr_review(s, n, kind, &body),
                move |_| reload(),
            );
        });
    })
}

fn edit_cb(ctx: &Rc<Ctx>, pr: &PullRequestDetail, reload: &Callback) -> Callback {
    let ctx = ctx.clone();
    let reload = reload.clone();
    let (n, title, body) = (pr.number, pr.title.clone(), pr.body.clone());
    Rc::new(move || {
        let ctx = ctx.clone();
        let reload = reload.clone();
        let (title, body) = (title.clone(), body.clone());
        glib::spawn_future_local(async move {
            let Some(v) = form::run(
                &ctx.parent(),
                "Edit Pull Request",
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
                move |s| gh_backend::pr_edit(s, n, &t, &b),
                move |_| reload(),
            );
        });
    })
}

fn show_diff(ctx: &Rc<Ctx>, _nav: &adw::NavigationSplitView, n: u64) {
    let diff = DiffView::new();
    diff.set_placeholder("content-loading-symbolic", "Loading…", "");
    let win = ctx.parent().root().and_downcast::<gtk::Window>();
    let (w, h) = win
        .map(|w| (w.width() - 80, w.height() - 80))
        .unwrap_or((1200, 800));
    util::content_dialog(
        &ctx.parent(),
        &format!("#{n} Files Changed"),
        &diff.widget(),
        w,
        h,
    );
    ctx.gh_load(
        move |s| gh_backend::pr_diff(s, n),
        move |r| match r {
            Ok(d) => diff.set_diff(&d),
            Err(e) => diff.set_placeholder(
                "dialog-error-symbolic",
                "Cannot Load Diff",
                &glib::markup_escape_text(&e.to_string()),
            ),
        },
    );
}
