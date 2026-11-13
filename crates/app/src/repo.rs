//! One open repository: the content of a tab.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use git_backend::{Location, PullMode, Status};
use gtk::{gio, glib};

use crate::{settings, util, views};

/// A screen inside a repository tab.
pub trait View {
    fn widget(&self) -> gtk::Widget;
    fn refresh(self: Rc<Self>);
}

type BusyListener = Rc<dyn Fn(bool)>;
type Navigator = Rc<dyn Fn(&str)>;
type ViewSlot = (Rc<dyn View>, bool);

/// Shared state handed to every view of one repository.
pub struct Ctx {
    pub loc: Location,
    pub slug: RefCell<Option<String>>,
    pub status: RefCell<Option<Status>>,
    toasts: adw::ToastOverlay,
    listeners: RefCell<Vec<Rc<dyn Fn()>>>,
    busy: Cell<u32>,
    busy_listener: RefCell<Option<BusyListener>>,
    navigate: RefCell<Option<Navigator>>,
    /// The branch dropdown, placed by the Code view at the top of its history column.
    pub branch_button: RefCell<Option<gtk::Widget>>,
    /// The sync control (fetch/pull/push with live progress), placed under the branch dropdown.
    pub sync_widget: RefCell<Option<gtk::Widget>>,
}

impl Ctx {
    /// Widget to parent dialogs on.
    pub fn parent(&self) -> gtk::Widget {
        self.toasts.clone().upcast()
    }

    pub fn toast(&self, msg: &str) {
        self.toasts
            .add_toast(adw::Toast::builder().title(msg).timeout(3).build());
    }

    /// The repository changed: refresh status and views.
    pub fn changed(&self) {
        let ls: Vec<_> = self.listeners.borrow().clone();
        for l in ls {
            l();
        }
    }

    fn on_changed(&self, f: impl Fn() + 'static) {
        self.listeners.borrow_mut().push(Rc::new(f));
    }

    /// Switch the tab to another section (e.g. `pulls`).
    pub fn navigate(&self, section: &str) {
        let nav = self.navigate.borrow().clone();
        if let Some(n) = nav {
            n(section);
        }
    }

    pub fn branch(&self) -> String {
        self.status
            .borrow()
            .as_ref()
            .map(|s| s.branch.head.clone())
            .unwrap_or_default()
    }

    fn set_busy(&self, delta: i32) {
        let n = (self.busy.get() as i32 + delta).max(0) as u32;
        self.busy.set(n);
        let l = self.busy_listener.borrow().clone();
        if let Some(l) = l {
            l(n > 0);
        }
    }

    /// Load data in the background without error dialogs (views show errors inline).
    pub fn load<T: Send + 'static>(
        self: &Rc<Self>,
        job: impl FnOnce(&Location) -> anyhow::Result<T> + Send + 'static,
        done: impl FnOnce(anyhow::Result<T>) + 'static,
    ) {
        let loc = self.loc.clone();
        let this = self.clone();
        self.set_busy(1);
        util::spawn(
            move || job(&loc),
            move |r| {
                this.set_busy(-1);
                done(r);
            },
        );
    }

    /// Run a repository-changing git operation; report failure, then refresh everything.
    pub fn git<T: Send + 'static>(
        self: &Rc<Self>,
        what: &str,
        job: impl FnOnce(&Location) -> anyhow::Result<T> + Send + 'static,
        done: impl FnOnce(T) + 'static,
    ) {
        let what = what.to_owned();
        let this = self.clone();
        self.load(job, move |r| {
            match r {
                Ok(v) => done(v),
                Err(e) => util::show_error(&this.parent(), &format!("{what} Failed"), &e),
            }
            this.changed();
        });
    }

    /// Run a GitHub operation against this repository's GitHub slug.
    pub fn gh<T: Send + 'static>(
        self: &Rc<Self>,
        what: &str,
        job: impl FnOnce(&str) -> anyhow::Result<T> + Send + 'static,
        done: impl FnOnce(T) + 'static,
    ) {
        let Some(slug) = self.slug.borrow().clone() else {
            return;
        };
        let what = what.to_owned();
        let this = self.clone();
        self.load(
            move |_| job(&slug),
            move |r| match r {
                Ok(v) => done(v),
                Err(e) => util::show_error(&this.parent(), &format!("{what} Failed"), &e),
            },
        );
    }

    /// Like [`Ctx::gh`], for loading: hands the result (or error) to `done`.
    pub fn gh_load<T: Send + 'static>(
        self: &Rc<Self>,
        job: impl FnOnce(&str) -> anyhow::Result<T> + Send + 'static,
        done: impl FnOnce(anyhow::Result<T>) + 'static,
    ) {
        let slug = self.slug.borrow().clone();
        self.load(
            move |_| match slug {
                Some(s) => job(&s),
                None => anyhow::bail!("This repository has no GitHub remote."),
            },
            done,
        );
    }
}

struct Section {
    id: &'static str,
    title: &'static str,
    icon: &'static str,
}

const SECTIONS: &[Section] = &[
    Section {
        id: "code",
        title: "Code",
        icon: "gh-code-symbolic",
    },
    Section {
        id: "branches",
        title: "Branches",
        icon: "gh-branch-symbolic",
    },
    Section {
        id: "pulls",
        title: "Pull Requests",
        icon: "gh-pull-request-symbolic",
    },
    Section {
        id: "issues",
        title: "Issues",
        icon: "gh-issue-symbolic",
    },
    Section {
        id: "actions",
        title: "Actions",
        icon: "gh-actions-symbolic",
    },
    Section {
        id: "releases",
        title: "Releases",
        icon: "gh-tag-symbolic",
    },
    Section {
        id: "deploy",
        title: "Deploy",
        icon: "gh-deploy-symbolic",
    },
    Section {
        id: "settings",
        title: "Settings",
        icon: "emblem-system-symbolic",
    },
];

fn build_view(id: &str, ctx: &Rc<Ctx>) -> Rc<dyn View> {
    match id {
        "code" => views::code::new(ctx),
        "branches" => views::columns(vec![
            views::branches::new(ctx),
            views::tags::new(ctx),
            views::stashes::new(ctx),
        ]),
        "pulls" => views::pulls::new(ctx),
        "issues" => views::issues::new(ctx),
        "actions" => views::actions::new(ctx),
        "releases" => views::releases::new(ctx),
        "deploy" => views::columns(vec![
            views::deployments::new(ctx),
            views::packages::new(ctx),
        ]),
        _ => views::columns(vec![views::overview::new(ctx), views::remotes::new(ctx)]),
    }
}

pub struct RepoPage {
    pub ctx: Rc<Ctx>,
    root: adw::BreakpointBin,
    outer: gtk::Stack,
    error_page: adw::StatusPage,
    stack: adw::ViewStack,
    bins: HashMap<&'static str, adw::Bin>,
    /// Built views and whether each needs a refresh when next shown.
    views: RefCell<HashMap<&'static str, ViewSlot>>,
    loaded: Cell<bool>,
    // Widgets the window shows in its header bar while this tab is active.
    pub header_start: gtk::Box,
    pub header_title: gtk::Widget,
    pub header_end: gtk::Box,
    branch_content: adw::ButtonContent,
    branch_list: gtk::ListBox,
    sync: gtk::Button,
    sync_more: gtk::MenuButton,
    sync_content: adw::ButtonContent,
    sync_stack: gtk::Stack,
    busy_title: gtk::Label,
    busy_detail: gtk::Label,
    busy_bar: gtk::ProgressBar,
    /// A fetch/pull/push is running (only one at a time).
    sync_busy: Cell<bool>,
    spinner: adw::Spinner,
    minutes_since_fetch: Cell<u32>,
    title_listener: RefCell<Option<Rc<dyn Fn()>>>,
}

impl RepoPage {
    pub fn new(loc: Location) -> Rc<Self> {
        let toasts = adw::ToastOverlay::new();
        let ctx = Rc::new(Ctx {
            loc,
            slug: RefCell::new(None),
            status: RefCell::new(None),
            toasts: toasts.clone(),
            listeners: RefCell::new(Vec::new()),
            busy: Cell::new(0),
            busy_listener: RefCell::new(None),
            navigate: RefCell::new(None),
            branch_button: RefCell::new(None),
            sync_widget: RefCell::new(None),
        });

        let stack = adw::ViewStack::new();
        let mut bins = HashMap::new();
        for s in SECTIONS {
            let bin = adw::Bin::new();
            stack.add_titled_with_icon(&bin, Some(s.id), s.title, s.icon);
            bins.insert(s.id, bin);
        }
        let switcher = adw::InlineViewSwitcher::builder()
            .stack(&stack)
            .display_mode(adw::InlineViewSwitcherDisplayMode::Both)
            .homogeneous(false)
            .css_classes(["flat"])
            .valign(gtk::Align::Center)
            .build();
        // Sync control: the main button does the next useful sync, the menu has every action.
        // While git runs, the button shows the phase, percentage, counts and speed with a bar.
        let sync_menu = gio::Menu::new();
        sync_menu.append(Some("Fetch"), Some("win.fetch"));
        sync_menu.append(Some("Pull"), Some("win.pull"));
        sync_menu.append(Some("Push"), Some("win.push"));
        let force = gio::Menu::new();
        force.append(Some("Force Push…"), Some("win.force-push"));
        sync_menu.append_section(None, &force);
        let sync_content = adw::ButtonContent::builder()
            .icon_name("view-refresh-symbolic")
            .label("Fetch")
            .build();
        let busy_title = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let busy_detail = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["caption", "numeric"])
            .build();
        let busy_bar = gtk::ProgressBar::builder()
            .css_classes(["sync-progress"])
            .pulse_step(0.08)
            .build();
        let busy_row = util::hbox(8);
        busy_row.append(&adw::Spinner::new());
        busy_row.append(&busy_title);
        let busy = util::vbox(4);
        busy.set_margin_top(4);
        busy.set_margin_bottom(4);
        busy.append(&busy_row);
        busy.append(&busy_detail);
        busy.append(&busy_bar);
        let sync_stack = gtk::Stack::builder()
            .vhomogeneous(false)
            .interpolate_size(true)
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        sync_stack.add_named(&sync_content, Some("idle"));
        sync_stack.add_named(&busy, Some("busy"));
        let sync = gtk::Button::builder()
            .child(&sync_stack)
            .hexpand(true)
            .build();
        let sync_more = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .menu_model(&sync_menu)
            .tooltip_text("All Sync Actions")
            .valign(gtk::Align::Fill)
            .build();
        let sync_box = gtk::Box::builder().css_classes(["linked"]).build();
        sync_box.append(&sync);
        sync_box.append(&sync_more);
        *ctx.sync_widget.borrow_mut() = Some(sync_box.upcast());
        let main = adw::ToolbarView::new();
        main.set_content(Some(&stack));

        let error_page = util::status_page("dialog-error-symbolic", "Cannot Open Repository", "");
        let retry = util::text_button("Retry", "pill");
        retry.set_halign(gtk::Align::Center);
        error_page.set_child(Some(&retry));
        let outer = gtk::Stack::new();
        outer.add_named(&util::spinner_page(), Some("loading"));
        outer.add_named(&main, Some("main"));
        outer.add_named(&error_page, Some("error"));
        toasts.set_child(Some(&outer));

        let root = adw::BreakpointBin::builder()
            .width_request(360)
            .height_request(300)
            .child(&toasts)
            .build();
        let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            1100.0,
            adw::LengthUnit::Sp,
        ));
        bp.add_setter(
            &switcher,
            "display-mode",
            Some(&adw::InlineViewSwitcherDisplayMode::Icons.to_value()),
        );
        root.add_breakpoint(bp);

        // Branch switcher
        let branch_content = adw::ButtonContent::builder()
            .icon_name("gh-branch-symbolic")
            .label("…")
            .build();
        let branch_list = gtk::ListBox::builder()
            .css_classes(["navigation-sidebar"])
            .build();
        let branch_search = gtk::SearchEntry::builder()
            .placeholder_text("Filter branches")
            .build();
        let new_branch = gtk::Button::builder()
            .label("New Branch…")
            .css_classes(["flat"])
            .build();
        let pop_box = util::vbox(6);
        pop_box.append(&branch_search);
        pop_box.append(
            &gtk::ScrolledWindow::builder()
                .child(&branch_list)
                .min_content_height(80)
                .max_content_height(360)
                .propagate_natural_height(true)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .build(),
        );
        pop_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        pop_box.append(&new_branch);
        pop_box.set_width_request(300);
        let popover = gtk::Popover::builder().child(&pop_box).build();
        let branch_button = gtk::MenuButton::builder()
            .child(&branch_content)
            .popover(&popover)
            .always_show_arrow(true)
            .hexpand(true)
            .tooltip_text("Switch Branch")
            .build();
        *ctx.branch_button.borrow_mut() = Some(branch_button.upcast());
        let header_start = util::hbox(6);

        let spinner = adw::Spinner::builder().visible(false).build();
        let header_end = util::hbox(6);
        header_end.append(&spinner);

        let page = Rc::new(RepoPage {
            ctx: ctx.clone(),
            root,
            outer,
            error_page,
            stack,
            bins,
            views: RefCell::new(HashMap::new()),
            loaded: Cell::new(false),
            header_start,
            header_title: switcher.upcast(),
            header_end,
            branch_content,
            branch_list,
            sync,
            sync_more,
            sync_content,
            sync_stack,
            busy_title,
            busy_detail,
            busy_bar,
            sync_busy: Cell::new(false),
            spinner,
            minutes_since_fetch: Cell::new(0),
            title_listener: RefCell::new(None),
        });

        // Wiring
        let weak = Rc::downgrade(&page);
        page.stack.connect_visible_child_name_notify(glib::clone!(
            #[strong]
            weak,
            move |st| {
                if let (Some(p), Some(name)) = (weak.upgrade(), st.visible_child_name())
                    && p.loaded.get()
                {
                    p.show_section(&name);
                }
            }
        ));
        retry.connect_clicked(glib::clone!(
            #[strong]
            weak,
            move |_| {
                if let Some(p) = weak.upgrade() {
                    p.outer.set_visible_child_name("loading");
                    p.refresh();
                }
            }
        ));
        ctx.on_changed(glib::clone!(
            #[strong]
            weak,
            move || {
                if let Some(p) = weak.upgrade() {
                    p.refresh();
                }
            }
        ));
        *ctx.busy_listener.borrow_mut() = Some(Rc::new(glib::clone!(
            #[strong]
            weak,
            move |busy| {
                if let Some(p) = weak.upgrade() {
                    p.spinner.set_visible(busy);
                }
            }
        )));
        *ctx.navigate.borrow_mut() = Some(Rc::new(glib::clone!(
            #[strong]
            weak,
            move |id: &str| {
                if let Some(p) = weak.upgrade() {
                    p.stack.set_visible_child_name(id);
                }
            }
        )));
        {
            let weak = weak.clone();
            page.sync.connect_clicked(move |_| {
                if let Some(p) = weak.upgrade() {
                    p.smart_sync();
                }
            });
        }
        branch_search.connect_search_changed(glib::clone!(
            #[strong]
            weak,
            move |e| {
                if let Some(p) = weak.upgrade() {
                    let q = e.text().to_lowercase();
                    let mut child = p.branch_list.first_child();
                    while let Some(c) = child {
                        c.set_visible(c.widget_name().to_lowercase().contains(&q));
                        child = c.next_sibling();
                    }
                }
            }
        ));
        page.branch_list.connect_row_activated(glib::clone!(
            #[strong]
            weak,
            #[weak]
            popover,
            move |_, row| {
                popover.popdown();
                if let Some(p) = weak.upgrade() {
                    let name = row.widget_name().to_string();
                    p.ctx.git(
                        "Switch Branch",
                        move |l| git_backend::checkout(l, &name, false),
                        |_| {},
                    );
                }
            }
        ));
        new_branch.connect_clicked(glib::clone!(
            #[strong]
            weak,
            #[weak]
            popover,
            move |_| {
                popover.popdown();
                if let Some(p) = weak.upgrade() {
                    views::branches::new_branch_dialog(&p.ctx, None);
                }
            }
        ));
        popover.connect_show(glib::clone!(
            #[strong]
            weak,
            move |_| {
                if let Some(p) = weak.upgrade() {
                    p.load_branch_list();
                }
            }
        ));
        glib::timeout_add_seconds_local(
            60,
            glib::clone!(
                #[strong]
                weak,
                move || match weak.upgrade() {
                    Some(p) => {
                        p.auto_fetch_tick();
                        glib::ControlFlow::Continue
                    }
                    None => glib::ControlFlow::Break,
                }
            ),
        );

        page.refresh();
        page
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    pub fn title(&self) -> String {
        self.ctx.loc.name()
    }

    pub fn subtitle(&self) -> String {
        let b = self.ctx.branch();
        if b.is_empty() {
            self.ctx.loc.to_string()
        } else {
            format!("{} — {b}", self.ctx.loc)
        }
    }

    pub fn connect_title_changed(&self, f: impl Fn() + 'static) {
        *self.title_listener.borrow_mut() = Some(Rc::new(f));
    }

    pub fn weak(self: &Rc<Self>) -> Weak<Self> {
        Rc::downgrade(self)
    }

    fn show_section(&self, id: &str) {
        let Some(section) = SECTIONS.iter().find(|s| s.id == id) else {
            return;
        };
        let id = section.id;
        let existing = self.views.borrow().get(id).cloned();
        match existing {
            Some((v, dirty)) => {
                if dirty {
                    self.views.borrow_mut().insert(id, (v.clone(), false));
                    v.refresh();
                }
            }
            None => {
                let v = build_view(id, &self.ctx);
                self.bins[id].set_child(Some(&v.widget()));
                self.views.borrow_mut().insert(id, (v.clone(), false));
                v.refresh();
            }
        }
    }

    fn current_id(&self) -> String {
        self.stack
            .visible_child_name()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "code".into())
    }

    /// Reload status and the visible view; other views refresh when next shown.
    pub fn refresh(self: &Rc<Self>) {
        let weak = self.weak();
        self.ctx.load(
            |l| {
                let st = git_backend::status(l)?;
                let slug = git_backend::github_repo(l);
                Ok((st, slug))
            },
            move |r| {
                let Some(p) = weak.upgrade() else { return };
                match r {
                    Ok((st, slug)) => {
                        *p.ctx.status.borrow_mut() = Some(st);
                        *p.ctx.slug.borrow_mut() = slug;
                        p.update_header();
                        if !p.loaded.replace(true) {
                            p.outer.set_visible_child_name("main");
                            p.stack.set_visible_child_name("code");
                            p.show_section("code");
                        } else {
                            p.refresh_views();
                        }
                    }
                    Err(e) => {
                        if !p.loaded.get() {
                            p.error_page.set_description(Some(&glib::markup_escape_text(
                                &format!("{e:#}"),
                            )));
                            p.outer.set_visible_child_name("error");
                        } else {
                            p.ctx.toast(&format!("Refresh failed: {e}"));
                        }
                    }
                }
            },
        );
    }

    fn refresh_views(&self) {
        let current = self.current_id();
        let mut views = self.views.borrow_mut();
        let mut to_refresh = Vec::new();
        for (id, (v, dirty)) in views.iter_mut() {
            if *id == current {
                to_refresh.push(v.clone());
                *dirty = false;
            } else {
                *dirty = true;
            }
        }
        drop(views);
        for v in to_refresh {
            v.refresh();
        }
    }

    fn update_header(&self) {
        let Some(st) = self.ctx.status.borrow().clone() else {
            return;
        };
        let head = if st.branch.head == "(detached)" {
            format!("detached at {}", st.branch.oid.get(..7).unwrap_or(""))
        } else {
            st.branch.head.clone()
        };
        self.branch_content.set_label(&head);
        let b = &st.branch;
        let publish = b.upstream.is_none() && b.head != "(detached)";
        let (icon, label, tip) = if publish {
            (
                "gh-arrow-up-symbolic",
                "Publish Branch".to_owned(),
                "Push this branch and set its upstream",
            )
        } else if b.behind > 0 && b.ahead > 0 {
            (
                "gh-arrow-down-symbolic",
                format!("Pull ↓{} ↑{}", b.behind, b.ahead),
                "Pull, then push",
            )
        } else if b.behind > 0 {
            (
                "gh-arrow-down-symbolic",
                format!("Pull ↓{}", b.behind),
                "Pull new commits",
            )
        } else if b.ahead > 0 {
            (
                "gh-arrow-up-symbolic",
                format!("Push ↑{}", b.ahead),
                "Push local commits",
            )
        } else {
            (
                "view-refresh-symbolic",
                "Fetch".to_owned(),
                "Fetch from all remotes",
            )
        };
        self.sync_content.set_icon_name(icon);
        self.sync_content.set_label(&label);
        self.sync.set_tooltip_text(Some(tip));
        // Accent only when there is something to pull or push.
        let pending = publish || b.ahead > 0 || b.behind > 0;
        for w in [
            self.sync.upcast_ref::<gtk::Widget>(),
            self.sync_more.upcast_ref(),
        ] {
            if pending {
                w.add_css_class("suggested-action");
            } else {
                w.remove_css_class("suggested-action");
            }
        }
        let l = self.title_listener.borrow().clone();
        if let Some(l) = l {
            l();
        }
    }

    /// The main sync action: push when ahead (or unpublished), pull when behind, else fetch.
    fn smart_sync(self: &Rc<Self>) {
        let Some(st) = self.ctx.status.borrow().clone() else {
            return;
        };
        let b = st.branch;
        if (b.upstream.is_none() && b.head != "(detached)") || (b.ahead > 0 && b.behind == 0) {
            self.push(false);
        } else if b.behind > 0 {
            self.pull();
        } else {
            self.fetch();
        }
    }

    /// Run a sync operation, showing git's live progress inside the sync button.
    /// `quiet` (background fetches): no toast, no error dialog.
    fn run_sync(
        self: &Rc<Self>,
        verb: &'static str,
        done_msg: &'static str,
        quiet: bool,
        job: impl FnOnce(&Location, &mut dyn FnMut(git_backend::Progress)) -> anyhow::Result<()>
        + Send
        + 'static,
    ) {
        if self.sync_busy.replace(true) {
            if !quiet {
                self.ctx.toast("A sync is already running");
            }
            return;
        }
        self.busy_title.set_label(&format!("{verb}…"));
        self.busy_detail.set_label("Connecting");
        self.busy_bar.set_fraction(0.0);
        self.sync_stack.set_visible_child_name("busy");
        // The main button stays enabled so the progress stays readable (a click while busy
        // just says so); the menu is disabled.
        self.sync_more.set_sensitive(false);

        let (tx, rx) = std::sync::mpsc::channel::<git_backend::Progress>();
        let running = Rc::new(Cell::new(true));
        {
            let weak = self.weak();
            let running = running.clone();
            let mut determinate = false;
            glib::timeout_add_local(std::time::Duration::from_millis(80), move || {
                let Some(p) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                if !running.get() {
                    return glib::ControlFlow::Break;
                }
                if let Some(last) = rx.try_iter().last() {
                    determinate = last.percent.is_some() && !last.done;
                    p.show_progress(verb, &last);
                }
                if !determinate {
                    p.busy_bar.pulse();
                }
                glib::ControlFlow::Continue
            });
        }
        let weak = self.weak();
        self.ctx.load(
            move |l| {
                let mut send = |p: git_backend::Progress| {
                    let _ = tx.send(p);
                };
                job(l, &mut send)
            },
            move |r| {
                running.set(false);
                let Some(p) = weak.upgrade() else { return };
                p.sync_busy.set(false);
                p.sync_stack.set_visible_child_name("idle");
                p.sync_more.set_sensitive(true);
                match r {
                    Ok(()) => {
                        if !quiet {
                            p.ctx.toast(done_msg);
                        }
                        p.ctx.changed();
                    }
                    Err(e) => {
                        if !quiet {
                            util::show_error(&p.ctx.parent(), &format!("{verb} Failed"), &e);
                            p.ctx.changed();
                        }
                    }
                }
            },
        );
    }

    /// Show one progress report: "Fetching — Receiving objects 45%",
    /// "684 / 1,520 · 2.31 MiB | 1.80 MiB/s", and the bar.
    fn show_progress(&self, verb: &str, p: &git_backend::Progress) {
        let phase = p.phase.to_lowercase();
        let title = match p.percent {
            Some(pct) => format!("{verb} · {phase} {pct}%"),
            None => format!("{verb} · {phase}"),
        };
        self.busy_title.set_label(&title);
        let mut detail = Vec::new();
        match (p.current, p.total) {
            (Some(c), Some(t)) => detail.push(format!("{c} / {t}")),
            (Some(c), None) => detail.push(c.to_string()),
            _ => {}
        }
        if !p.detail.is_empty() {
            detail.push(p.detail.clone());
        }
        self.busy_detail.set_label(&detail.join(" · "));
        if let Some(pct) = p.percent {
            self.busy_bar.set_fraction(f64::from(pct) / 100.0);
        }
    }

    pub fn fetch(self: &Rc<Self>) {
        self.minutes_since_fetch.set(0);
        self.run_sync("Fetching", "Fetched", false, |l, on| {
            git_backend::fetch_with_progress(l, on)
        });
    }

    pub fn pull(self: &Rc<Self>) {
        let mode = match settings::get().pull_mode.as_str() {
            "rebase" => PullMode::Rebase,
            "ff-only" => PullMode::FastForwardOnly,
            _ => PullMode::Merge,
        };
        self.run_sync("Pulling", "Pulled", false, move |l, on| {
            git_backend::pull_with_progress(l, mode, on)
        });
    }

    pub fn push(self: &Rc<Self>, force: bool) {
        self.run_sync(
            if force { "Force pushing" } else { "Pushing" },
            if force { "Force pushed" } else { "Pushed" },
            false,
            move |l, on| git_backend::push_with_progress(l, force, on),
        );
    }

    pub fn force_push(self: &Rc<Self>) {
        let weak = self.weak();
        let ctx = self.ctx.clone();
        let this_branch = ctx.branch();
        glib::spawn_future_local(async move {
            if util::confirm(
                &ctx.parent(),
                "Force Push?",
                &format!(
                    "This overwrites the remote “{this_branch}” with your local branch. Commits on the remote that you don't have will be lost (uses --force-with-lease)."
                ),
                "Force Push",
                true,
            )
            .await
                && let Some(p) = weak.upgrade()
            {
                p.push(true);
            }
        });
    }

    fn auto_fetch_tick(self: &Rc<Self>) {
        let interval = settings::get().auto_fetch_minutes;
        let n = self.minutes_since_fetch.get() + 1;
        self.minutes_since_fetch.set(n);
        if interval == 0 || n < interval || !self.loaded.get() || self.sync_busy.get() {
            return;
        }
        self.minutes_since_fetch.set(0);
        self.run_sync("Fetching", "Fetched", true, |l, on| {
            git_backend::fetch_with_progress(l, on)
        });
    }

    fn load_branch_list(&self) {
        let list = self.branch_list.clone();
        self.ctx.load(git_backend::branches, move |r| {
            list.remove_all();
            for b in r.unwrap_or_default().into_iter().filter(|b| !b.remote) {
                let row_box = util::hbox(8);
                let icon = gtk::Image::from_icon_name(if b.is_head {
                    "object-select-symbolic"
                } else {
                    "gh-branch-symbolic"
                });
                row_box.append(&icon);
                row_box.append(
                    &gtk::Label::builder()
                        .label(&b.name)
                        .xalign(0.0)
                        .hexpand(true)
                        .ellipsize(gtk::pango::EllipsizeMode::Middle)
                        .build(),
                );
                row_box.append(
                    &gtk::Label::builder()
                        .label(util::relative_time(b.time))
                        .css_classes(["dim-label", "caption"])
                        .build(),
                );
                let row = gtk::ListBoxRow::builder().child(&row_box).build();
                row.set_widget_name(&b.name);
                row.set_sensitive(!b.is_head);
                list.append(&row);
            }
        });
    }
}
