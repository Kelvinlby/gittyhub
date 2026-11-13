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
    github: bool,
}

const SECTIONS: &[Section] = &[
    Section {
        id: "changes",
        title: "Changes",
        icon: "document-edit-symbolic",
        github: false,
    },
    Section {
        id: "history",
        title: "History",
        icon: "gh-commit-symbolic",
        github: false,
    },
    Section {
        id: "branches",
        title: "Branches",
        icon: "gh-branch-symbolic",
        github: false,
    },
    Section {
        id: "stashes",
        title: "Stashes",
        icon: "gh-stash-symbolic",
        github: false,
    },
    Section {
        id: "tags",
        title: "Tags",
        icon: "gh-tag-symbolic",
        github: false,
    },
    Section {
        id: "files",
        title: "Files",
        icon: "folder-symbolic",
        github: false,
    },
    Section {
        id: "remotes",
        title: "Remotes & Settings",
        icon: "gh-server-symbolic",
        github: false,
    },
    Section {
        id: "overview",
        title: "Overview",
        icon: "gh-github-symbolic",
        github: true,
    },
    Section {
        id: "pulls",
        title: "Pull Requests",
        icon: "gh-pull-request-symbolic",
        github: true,
    },
    Section {
        id: "issues",
        title: "Issues",
        icon: "gh-issue-symbolic",
        github: true,
    },
    Section {
        id: "actions",
        title: "Actions",
        icon: "gh-actions-symbolic",
        github: true,
    },
    Section {
        id: "releases",
        title: "Releases",
        icon: "gh-tag-symbolic",
        github: true,
    },
    Section {
        id: "deployments",
        title: "Deployments",
        icon: "gh-deploy-symbolic",
        github: true,
    },
    Section {
        id: "packages",
        title: "Packages",
        icon: "gh-package-symbolic",
        github: true,
    },
];

fn build_view(id: &str, ctx: &Rc<Ctx>) -> Rc<dyn View> {
    match id {
        "changes" => views::changes::new(ctx),
        "history" => views::history::new(ctx),
        "branches" => views::branches::new(ctx),
        "stashes" => views::stashes::new(ctx),
        "tags" => views::tags::new(ctx),
        "files" => views::files::new(ctx),
        "remotes" => views::remotes::new(ctx),
        "overview" => views::overview::new(ctx),
        "pulls" => views::pulls::new(ctx),
        "issues" => views::issues::new(ctx),
        "actions" => views::actions::new(ctx),
        "releases" => views::releases::new(ctx),
        "deployments" => views::deployments::new(ctx),
        _ => views::packages::new(ctx),
    }
}

pub struct RepoPage {
    pub ctx: Rc<Ctx>,
    root: adw::BreakpointBin,
    outer: gtk::Stack,
    error_page: adw::StatusPage,
    split: adw::OverlaySplitView,
    sidebar: gtk::ListBox,
    stack: gtk::Stack,
    content_title: adw::WindowTitle,
    /// Built views and whether each needs a refresh when next shown.
    views: RefCell<HashMap<&'static str, ViewSlot>>,
    current: Cell<&'static str>,
    loaded: Cell<bool>,
    // Widgets the window shows in its header bar while this tab is active.
    pub header_start: gtk::Box,
    pub header_title: gtk::Widget,
    pub header_end: gtk::Box,
    branch_content: adw::ButtonContent,
    branch_list: gtk::ListBox,
    sync: adw::SplitButton,
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
        });

        // Sidebar
        let sidebar = gtk::ListBox::builder()
            .css_classes(["navigation-sidebar"])
            .build();
        for s in SECTIONS {
            let row_box = util::hbox(12);
            row_box.append(&gtk::Image::from_icon_name(s.icon));
            row_box.append(&gtk::Label::builder().label(s.title).xalign(0.0).build());
            let row = gtk::ListBoxRow::builder().child(&row_box).build();
            row.set_widget_name(s.id);
            sidebar.append(&row);
        }
        sidebar.set_header_func(|row, before| {
            let idx = row.index() as usize;
            let title = if before.is_none() {
                Some("Repository")
            } else if SECTIONS[idx].github && !SECTIONS[idx - 1].github {
                Some("GitHub")
            } else {
                None
            };
            row.set_header(
                title
                    .map(|t| {
                        gtk::Label::builder()
                            .label(t)
                            .xalign(0.0)
                            .margin_start(12)
                            .margin_top(if before.is_none() { 6 } else { 18 })
                            .margin_bottom(6)
                            .css_classes(["heading", "dim-label"])
                            .build()
                    })
                    .as_ref(),
            );
        });
        let location_label = gtk::Label::builder()
            .label(ctx.loc.to_string())
            .ellipsize(gtk::pango::EllipsizeMode::Start)
            .tooltip_text(ctx.loc.to_string())
            .xalign(0.0)
            .margin_start(18)
            .margin_end(18)
            .margin_bottom(12)
            .css_classes(["caption", "dim-label"])
            .build();
        let side_box = util::vbox(0);
        side_box.append(
            &gtk::ScrolledWindow::builder()
                .child(&sidebar)
                .vexpand(true)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .build(),
        );
        side_box.append(&location_label);

        // Content
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        let content_title = adw::WindowTitle::new("", "");

        let split = adw::OverlaySplitView::builder()
            .sidebar(&side_box)
            .content(&stack)
            .min_sidebar_width(200.0)
            .max_sidebar_width(260.0)
            .build();

        let error_page = util::status_page("dialog-error-symbolic", "Cannot Open Repository", "");
        let retry = util::text_button("Retry", "pill");
        retry.set_halign(gtk::Align::Center);
        error_page.set_child(Some(&retry));
        let outer = gtk::Stack::new();
        outer.add_named(&util::spinner_page(), Some("loading"));
        outer.add_named(&split, Some("main"));
        outer.add_named(&error_page, Some("error"));
        toasts.set_child(Some(&outer));

        let root = adw::BreakpointBin::builder()
            .width_request(360)
            .height_request(300)
            .child(&toasts)
            .build();
        let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            760.0,
            adw::LengthUnit::Sp,
        ));
        bp.add_setter(&split, "collapsed", Some(&true.to_value()));
        root.add_breakpoint(bp);

        // Header widgets
        let sidebar_toggle = gtk::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text("Toggle Sidebar (F9)")
            .active(true)
            .build();
        split
            .bind_property("show-sidebar", &sidebar_toggle, "active")
            .bidirectional()
            .sync_create()
            .build();
        let header_start = util::hbox(6);
        header_start.append(&sidebar_toggle);

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
            .tooltip_text("Current Branch")
            .build();
        let title_box = util::vbox(0);
        title_box.set_valign(gtk::Align::Center);
        title_box.append(&branch_button);

        let spinner = adw::Spinner::builder().visible(false).build();
        let sync_menu = gio::Menu::new();
        sync_menu.append(Some("Fetch"), Some("win.fetch"));
        sync_menu.append(Some("Pull"), Some("win.pull"));
        sync_menu.append(Some("Push"), Some("win.push"));
        sync_menu.append(Some("Force Push…"), Some("win.force-push"));
        let sync = adw::SplitButton::builder()
            .menu_model(&sync_menu)
            .label("Fetch")
            .tooltip_text("Fetch, pull or push")
            .build();
        let header_end = util::hbox(6);
        header_end.append(&sync);
        header_end.append(&spinner);

        let page = Rc::new(RepoPage {
            ctx: ctx.clone(),
            root,
            outer,
            error_page,
            split,
            sidebar,
            stack,
            content_title,
            views: RefCell::new(HashMap::new()),
            current: Cell::new("changes"),
            loaded: Cell::new(false),
            header_start,
            header_title: title_box.upcast(),
            header_end,
            branch_content,
            branch_list,
            sync,
            spinner,
            minutes_since_fetch: Cell::new(0),
            title_listener: RefCell::new(None),
        });

        // Wiring
        let weak = Rc::downgrade(&page);
        page.sidebar.connect_row_selected(glib::clone!(
            #[strong]
            weak,
            move |_, row| {
                if let (Some(p), Some(row)) = (weak.upgrade(), row) {
                    let idx = row.index() as usize;
                    p.show_section(SECTIONS[idx].id);
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
                    p.select(id);
                }
            }
        )));
        page.sync.connect_clicked(glib::clone!(
            #[strong]
            weak,
            move |_| {
                if let Some(p) = weak.upgrade() {
                    p.smart_sync();
                }
            }
        ));
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

    pub fn toggle_sidebar(&self) {
        self.split.set_show_sidebar(!self.split.shows_sidebar());
    }

    fn select(&self, id: &str) {
        if let Some(i) = SECTIONS.iter().position(|s| s.id == id)
            && let Some(row) = self.sidebar.row_at_index(i as i32)
        {
            self.sidebar.select_row(Some(&row));
        }
    }

    fn show_section(&self, id: &'static str) {
        self.current.set(id);
        let section = SECTIONS.iter().find(|s| s.id == id).expect("section");
        self.content_title.set_title(section.title);
        let existing = self.views.borrow().get(id).cloned();
        let view = match existing {
            Some((v, dirty)) => {
                if dirty {
                    v.clone().refresh();
                    self.views.borrow_mut().insert(id, (v.clone(), false));
                }
                v
            }
            None => {
                let v = build_view(id, &self.ctx);
                self.stack.add_named(&v.widget(), Some(id));
                v.clone().refresh();
                self.views.borrow_mut().insert(id, (v.clone(), false));
                v
            }
        };
        self.stack.set_visible_child(&view.widget());
        if self.split.is_collapsed() {
            self.split.set_show_sidebar(false);
        }
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
                            p.select("changes");
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
        let current = self.current.get();
        let mut views = self.views.borrow_mut();
        for (id, (v, dirty)) in views.iter_mut() {
            if *id == current {
                v.clone().refresh();
                *dirty = false;
            } else {
                *dirty = true;
            }
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
        let (label, tip) = if b.upstream.is_none() && b.head != "(detached)" {
            (
                "Publish Branch".to_owned(),
                "Push this branch and set its upstream",
            )
        } else if b.behind > 0 && b.ahead > 0 {
            (
                format!("Pull ↓{} ↑{}", b.behind, b.ahead),
                "Pull, then push",
            )
        } else if b.behind > 0 {
            (format!("Pull ↓{}", b.behind), "Pull new commits")
        } else if b.ahead > 0 {
            (format!("Push ↑{}", b.ahead), "Push local commits")
        } else {
            ("Fetch".to_owned(), "Fetch from all remotes")
        };
        self.sync.set_label(&label);
        self.sync.set_tooltip_text(Some(tip));
        let l = self.title_listener.borrow().clone();
        if let Some(l) = l {
            l();
        }
    }

    fn smart_sync(&self) {
        let st = self.ctx.status.borrow().clone();
        let Some(st) = st else { return };
        let b = st.branch;
        if b.upstream.is_none() || (b.ahead > 0 && b.behind == 0) {
            self.push(false);
        } else if b.behind > 0 {
            self.pull();
        } else {
            self.fetch();
        }
    }

    pub fn fetch(&self) {
        let ctx = self.ctx.clone();
        self.ctx
            .git("Fetch", git_backend::fetch, move |_| ctx.toast("Fetched"));
        self.minutes_since_fetch.set(0);
    }

    pub fn pull(&self) {
        let mode = match settings::get().pull_mode.as_str() {
            "rebase" => PullMode::Rebase,
            "ff-only" => PullMode::FastForwardOnly,
            _ => PullMode::Merge,
        };
        let ctx = self.ctx.clone();
        self.ctx.git(
            "Pull",
            move |l| git_backend::pull(l, mode),
            move |_| ctx.toast("Pulled"),
        );
    }

    pub fn push(&self, force: bool) {
        let ctx = self.ctx.clone();
        self.ctx.git(
            "Push",
            move |l| git_backend::push(l, force),
            move |_| ctx.toast("Pushed"),
        );
    }

    pub fn force_push(&self) {
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
            {
                let c = ctx.clone();
                ctx.git(
                    "Force Push",
                    |l| git_backend::push(l, true),
                    move |_| c.toast("Force pushed"),
                );
            }
        });
    }

    fn auto_fetch_tick(&self) {
        let interval = settings::get().auto_fetch_minutes;
        let n = self.minutes_since_fetch.get() + 1;
        self.minutes_since_fetch.set(n);
        if interval == 0 || n < interval || !self.loaded.get() {
            return;
        }
        self.minutes_since_fetch.set(0);
        let ctx = self.ctx.clone();
        self.ctx.load(git_backend::fetch, move |r| {
            if r.is_ok() {
                ctx.changed();
            }
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
