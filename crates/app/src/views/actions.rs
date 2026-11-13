//! GitHub Actions: workflow runs, jobs, logs, re-runs and manual dispatch.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gh_backend::{RunDetail, Workflow};
use gtk::glib;

use super::github::{self, Shell};
use crate::form::{self, Field};
use crate::repo::{Ctx, View};
use crate::util::{self, Callback};

pub struct Actions {
    ctx: Rc<Ctx>,
    nav: adw::NavigationSplitView,
    shell: Shell,
    list: gtk::ListBox,
    filter: gtk::DropDown,
    workflows: RefCell<Vec<Workflow>>,
    has_active: std::cell::Cell<bool>,
}

pub fn new(ctx: &Rc<Ctx>) -> Rc<dyn View> {
    let filter = github::dropdown(&["All Workflows"]);
    let top = util::hbox(12);
    top.append(&filter);
    let list = util::boxed_list();
    util::placeholder(&list, "No workflow runs");
    let content = util::vbox(18);
    content.append(&top);
    content.append(&list);
    let shell = Shell::new(&util::page_scroller(&content));
    let nav = github::master_detail("gh-actions-symbolic", "Select a Workflow Run");
    let view = Rc::new(Actions {
        ctx: ctx.clone(),
        nav: nav.clone(),
        shell,
        list,
        filter,
        workflows: RefCell::new(Vec::new()),
        has_active: std::cell::Cell::new(false),
    });
    let w = Rc::downgrade(&view);
    let reload = {
        let w = w.clone();
        move || {
            if let Some(v) = w.upgrade() {
                v.refresh();
            }
        }
    };
    let refresh = github::refresh_button(reload.clone());
    let run_button = gtk::Button::builder()
        .icon_name("media-playback-start-symbolic")
        .tooltip_text("Run Workflow…")
        .build();
    let manage = gtk::Button::builder()
        .icon_name("emblem-system-symbolic")
        .tooltip_text("Manage Workflows")
        .build();
    {
        let w = w.clone();
        run_button.connect_clicked(move |_| {
            if let Some(v) = w.upgrade() {
                v.run_dialog();
            }
        });
    }
    {
        let w = w.clone();
        manage.connect_clicked(move |_| {
            if let Some(v) = w.upgrade() {
                v.manage_dialog();
            }
        });
    }
    github::set_list(
        &nav,
        &github::nav_page(
            "Actions",
            &view.shell.stack,
            &[run_button.upcast_ref(), manage.upcast_ref(), &refresh],
        ),
    );
    view.filter.connect_selected_notify(move |_| reload());
    // Keep in-progress runs fresh while the list is visible.
    let w2 = w.clone();
    github::poll_while_mapped(&view.nav, 15, move || {
        let Some(v) = w2.upgrade() else { return false };
        if v.has_active.get() {
            v.clone().refresh();
        }
        true
    });
    view
}

impl Actions {
    fn selected_workflow(&self) -> Option<String> {
        let i = self.filter.selected() as usize;
        (i > 0)
            .then(|| self.workflows.borrow().get(i - 1).map(|w| w.id.to_string()))
            .flatten()
    }

    fn run_dialog(self: &Rc<Self>) {
        let ctx = self.ctx.clone();
        let workflows: Vec<Workflow> = self
            .workflows
            .borrow()
            .iter()
            .filter(|w| w.state == "active")
            .cloned()
            .collect();
        if workflows.is_empty() {
            ctx.toast("No active workflows");
            return;
        }
        glib::spawn_future_local(async move {
            let names: Vec<String> = workflows.iter().map(|w| w.name.clone()).collect();
            let Some(v) = form::run(
                &ctx.parent(),
                "Run Workflow",
                "Dispatches a run; the workflow must have a workflow_dispatch trigger.",
                "Run",
                vec![
                    Field::choice("Workflow", &names, 0),
                    Field::entry("Branch or tag", &ctx.branch()),
                ],
            )
            .await
            else {
                return;
            };
            let id = workflows[v[0].index().min(workflows.len() - 1)].id;
            let git_ref = v[1].text();
            let c = ctx.clone();
            ctx.gh(
                "Run Workflow",
                move |s| gh_backend::workflow_run(s, id, &git_ref),
                move |_| c.toast("Workflow run requested"),
            );
        });
    }

    fn manage_dialog(self: &Rc<Self>) {
        let list = util::boxed_list();
        for wf in self.workflows.borrow().iter() {
            let row = adw::SwitchRow::builder()
                .title(glib::markup_escape_text(&wf.name))
                .subtitle(glib::markup_escape_text(&wf.path))
                .active(wf.state == "active")
                .build();
            let ctx = self.ctx.clone();
            let id = wf.id;
            row.connect_active_notify(move |r| {
                let enabled = r.is_active();
                ctx.gh(
                    "Update Workflow",
                    move |s| gh_backend::workflow_set_enabled(s, id, enabled),
                    |_| {},
                );
            });
            list.append(&row);
        }
        util::placeholder(&list, "No workflows");
        let content = util::vbox(12);
        content.append(&list);
        util::content_dialog(
            &self.ctx.parent(),
            "Workflows",
            &util::page_scroller(&content),
            520,
            560,
        );
    }
}

impl View for Actions {
    fn widget(&self) -> gtk::Widget {
        github::root_of(&self.nav)
    }

    fn refresh(self: Rc<Self>) {
        if !self.shell.begin(&self.ctx) {
            return;
        }
        let workflow = self.selected_workflow();
        let need_workflows = self.workflows.borrow().is_empty();
        let this = self.clone();
        self.ctx.gh_load(
            move |s| {
                let wfs = if need_workflows {
                    Some(gh_backend::workflow_list(s).unwrap_or_default())
                } else {
                    None
                };
                Ok((gh_backend::run_list(s, workflow.as_deref())?, wfs))
            },
            move |r| {
                let (runs, wfs) = match r {
                    Ok(x) => x,
                    Err(e) => return this.shell.error(&e),
                };
                if let Some(wfs) = wfs.filter(|w| !w.is_empty()) {
                    let mut names = vec!["All Workflows".to_owned()];
                    names.extend(wfs.iter().map(|w| w.name.clone()));
                    let strs: Vec<&str> = names.iter().map(String::as_str).collect();
                    *this.workflows.borrow_mut() = wfs;
                    this.filter.set_model(Some(&gtk::StringList::new(&strs)));
                }
                util::clear_rows(&this.list);
                this.has_active
                    .set(runs.iter().any(|r| r.status != "completed"));
                for run in runs {
                    let row = util::row(
                        &run.display_title,
                        &format!(
                            "{} #{} · {} · {} · {}",
                            run.workflow_name,
                            run.number,
                            run.head_branch,
                            run.event,
                            util::iso_relative(&run.created_at)
                        ),
                    );
                    row.set_activatable(true);
                    row.add_prefix(&util::outcome_icon(run.outcome()));
                    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
                    let w = Rc::downgrade(&this);
                    let id = run.database_id;
                    row.connect_activated(move |_| {
                        if let Some(v) = w.upgrade() {
                            show_run(&v.ctx, &v.nav, id);
                        }
                    });
                    this.list.append(&row);
                }
                this.shell.ready();
            },
        );
    }
}

fn show_run(ctx: &Rc<Ctx>, nav: &adw::NavigationSplitView, id: u64) {
    let content = util::vbox(18);
    let shell = Rc::new(Shell::new(&util::page_scroller(&content)));
    let url: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    let u = url.clone();
    let browser = github::browser_button(ctx, move || u.borrow().clone());
    let reload: Rc<RefCell<Option<Callback>>> = Rc::new(RefCell::new(None));
    let r2 = reload.clone();
    let refresh = github::refresh_button(move || {
        if let Some(f) = r2.borrow().clone() {
            f();
        }
    });
    let page = github::nav_page("Workflow Run", &shell.stack, &[&browser, &refresh]);
    github::show_detail(nav, &page);
    let active = Rc::new(std::cell::Cell::new(true));
    let load: Callback = {
        let ctx = ctx.clone();
        let reload = reload.clone();
        let active = active.clone();
        Rc::new(move || {
            shell.begin(&ctx);
            let (ctx2, content, shell, reload, url, active) = (
                ctx.clone(),
                content.clone(),
                shell.clone(),
                reload.clone(),
                url.clone(),
                active.clone(),
            );
            ctx.gh_load(
                move |s| gh_backend::run_view(s, id),
                move |r| match r {
                    Ok(d) => {
                        *url.borrow_mut() = d.run.url.clone();
                        active.set(d.run.status != "completed");
                        let again = reload.borrow().clone().expect("reload set");
                        fill_run(&ctx2, &content, &d, again);
                        shell.ready();
                    }
                    Err(e) => shell.error(&e),
                },
            );
        })
    };
    *reload.borrow_mut() = Some(load.clone());
    load();
    let weak_page = page.downgrade();
    github::poll_while_mapped(&page, 8, move || {
        if weak_page.upgrade().is_none() {
            return false;
        }
        if active.get() {
            load();
        }
        true
    });
}

fn fill_run(ctx: &Rc<Ctx>, content: &gtk::Box, d: &RunDetail, reload: Callback) {
    util::clear_box(content);
    let run = &d.run;
    let id = run.database_id;
    let head = util::hbox(12);
    let icon = util::outcome_icon(run.outcome());
    icon.set_pixel_size(32);
    head.append(&icon);
    let titles = util::vbox(4);
    titles.append(&util::heading(&run.display_title, "title-2"));
    titles.append(
        &gtk::Label::builder()
            .label(format!(
                "{} #{} · {} · {} · {} · {}",
                run.workflow_name,
                run.number,
                run.head_branch,
                run.event,
                run.outcome(),
                util::iso_relative(&run.created_at)
            ))
            .wrap(true)
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build(),
    );
    head.append(&titles);
    content.append(&head);

    let actions = util::hbox(6);
    let completed = run.status == "completed";
    let mk =
        |label: &str, class: &str, what: &'static str, f: fn(&str, u64) -> anyhow::Result<()>| {
            let b = util::text_button(label, class);
            let ctx = ctx.clone();
            let reload = reload.clone();
            b.connect_clicked(move |_| {
                let reload = reload.clone();
                ctx.gh(what, move |s| f(s, id), move |_| reload());
            });
            b
        };
    if completed {
        actions.append(&mk(
            "Re-run All Jobs",
            "suggested-action",
            "Re-run",
            |s, id| gh_backend::run_rerun(s, id, false),
        ));
        if run.conclusion == "failure" {
            actions.append(&mk("Re-run Failed Jobs", "", "Re-run", |s, id| {
                gh_backend::run_rerun(s, id, true)
            }));
        }
    } else {
        actions.append(&mk(
            "Cancel Run",
            "destructive-action",
            "Cancel",
            gh_backend::run_cancel,
        ));
    }
    let log_cb = |failed: bool| -> Callback {
        let ctx = ctx.clone();
        Rc::new(move || {
            let c = ctx.clone();
            ctx.gh(
                "Load Log",
                move |s| gh_backend::run_log(s, id, failed),
                move |t| util::text_dialog(&c.parent(), "Run Log", &t),
            );
        })
    };
    let mut more: Vec<(&str, Callback)> = vec![("View Full Log", log_cb(false))];
    if run.conclusion == "failure" {
        more.insert(0, ("View Failed Steps Log", log_cb(true)));
    }
    if completed {
        let ctx = ctx.clone();
        more.push((
            "Delete Run…",
            Rc::new(move || {
                let ctx = ctx.clone();
                glib::spawn_future_local(async move {
                    if util::confirm(
                        &ctx.parent(),
                        "Delete Run?",
                        "The run and its logs will be deleted.",
                        "Delete",
                        true,
                    )
                    .await
                    {
                        let c = ctx.clone();
                        ctx.gh(
                            "Delete Run",
                            move |s| gh_backend::run_delete(s, id),
                            move |_| c.toast("Run deleted"),
                        );
                    }
                });
            }),
        ));
    }
    actions.append(&util::menu_button(more));
    content.append(&actions);

    let (group, list) = util::group("Jobs", None);
    util::placeholder(&list, "No jobs yet");
    for job in &d.jobs {
        let outcome = if job.status == "completed" {
            job.conclusion.as_str()
        } else {
            job.status.as_str()
        };
        let exp = adw::ExpanderRow::builder()
            .title(glib::markup_escape_text(&job.name))
            .subtitle(outcome)
            .build();
        exp.add_prefix(&util::outcome_icon(outcome));
        let log = util::icon_button("text-x-generic-symbolic", "View Job Log");
        let ctx2 = ctx.clone();
        let jid = job.database_id;
        log.connect_clicked(move |_| {
            let c = ctx2.clone();
            ctx2.gh(
                "Load Log",
                move |s| gh_backend::job_log(s, jid),
                move |t| util::text_dialog(&c.parent(), "Job Log", &t),
            );
        });
        exp.add_suffix(&log);
        for step in &job.steps {
            let so = if step.status == "completed" {
                step.conclusion.as_str()
            } else {
                step.status.as_str()
            };
            let row = util::row(&format!("{}. {}", step.number, step.name), so);
            row.add_prefix(&util::outcome_icon(so));
            exp.add_row(&row);
        }
        list.append(&exp);
    }
    content.append(&group);
}
