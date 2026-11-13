//! The Code screen: an editor-centred view of the repository.
//!
//! Left: a timeline (uncommitted changes, commits not yet pushed, pushed history) with a
//! commit composer. Middle: the files of the selected entry as a folder tree. Right: the
//! selected file as a diff, a rendered preview, or an editor. Actions live in right-click menus.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use adw::prelude::*;
use git_backend::{
    Commit, CommitFile, DiffFile, FileChange, LogQuery, RepoState, ResetMode, Rewrite,
};
use gtk::{gdk, gio, glib};

use crate::diff::{theme_color, tinted};
use crate::form::{self, Field};
use crate::repo::{Ctx, View};
use crate::tree::FileTree;
use crate::util::{self, Callback};
use crate::{editor, markdown, settings};

const PAGE: usize = 150;
const MAX_VIEW: usize = 4 * 1024 * 1024;

#[derive(Clone, PartialEq, Eq)]
enum Sel {
    Worktree,
    Commit(String),
}

pub struct Code {
    ctx: Rc<Ctx>,
    root: adw::BreakpointBin,
    timeline: gtk::ListBox,
    search: gtk::SearchEntry,
    more: gtk::Button,
    banner: adw::Banner,
    composer: gtk::Box,
    summary: gtk::Entry,
    description: gtk::TextBuffer,
    amend: gtk::CheckButton,
    commit_btn: gtk::Button,
    tree: Rc<FileTree>,
    tree_mode: adw::ToggleGroup,
    tree_title: gtk::Label,
    file_title: gtk::Label,
    mode: adw::ToggleGroup,
    discard_btn: gtk::Button,
    save_btn: gtk::Button,
    pane: gtk::Stack,
    empty: adw::StatusPage,
    diff_view: gtk::TextView,
    tags: gtk::TextTagTable,
    preview: adw::Bin,
    editor: sourceview::View,
    sel: RefCell<Sel>,
    commits: RefCell<Vec<Commit>>,
    unpushed: RefCell<HashSet<String>>,
    changes: RefCell<Vec<FileChange>>,
    commit_files: RefCell<Vec<CommitFile>>,
    current: RefCell<Option<String>>,
    diff_file: RefCell<Option<DiffFile>>,
    line_map: RefCell<Vec<Option<(usize, usize)>>>,
    editor_path: RefCell<Option<String>>,
    generation: Cell<u64>,
    suppress: Cell<bool>,
}

fn tags() -> gtk::TextTagTable {
    let t = gtk::TextTagTable::new();
    for name in ["added", "removed", "hunk", "gutter"] {
        t.add(&gtk::TextTag::new(Some(name)));
    }
    t
}

/// Colour the diff tags. `fg` must come from a widget in the window, so it follows a
/// colour scheme forced in Preferences.
fn recolor(t: &gtk::TextTagTable, fg: gdk::RGBA) {
    if let Some(tag) = t.lookup("added") {
        tag.set_paragraph_background_rgba(Some(&tinted(theme_color("success"), 0.16)));
    }
    if let Some(tag) = t.lookup("removed") {
        tag.set_paragraph_background_rgba(Some(&tinted(theme_color("error"), 0.16)));
    }
    if let Some(tag) = t.lookup("hunk") {
        tag.set_paragraph_background_rgba(Some(&tinted(theme_color("accent"), 0.10)));
        tag.set_foreground_rgba(Some(&tinted(fg, 0.7)));
    }
    if let Some(tag) = t.lookup("gutter") {
        tag.set_foreground_rgba(Some(&tinted(fg, 0.45)));
    }
}

fn code_view(editable: bool, tags: Option<&gtk::TextTagTable>) -> gtk::TextView {
    gtk::TextView::builder()
        .buffer(&gtk::TextBuffer::new(tags))
        .editable(editable)
        .cursor_visible(editable)
        .monospace(true)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .build()
}

fn scrolled(w: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .child(w)
        .vexpand(true)
        .hexpand(true)
        .build()
}

fn basename(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

fn is_markdown(p: &str) -> bool {
    let l = p.to_lowercase();
    l.ends_with(".md") || l.ends_with(".markdown")
}

fn change_letter(f: &FileChange) -> char {
    if f.conflicted {
        'U'
    } else if f.untracked() {
        'A'
    } else if f.index != '.' {
        f.index
    } else {
        f.worktree
    }
}

/// `@@ -a,b +c,d @@` → (a, c).
fn hunk_starts(h: &str) -> (u32, u32) {
    let mut it = h.split_whitespace().skip(1);
    let num = |s: Option<&str>| {
        s.and_then(|s| s[1..].split(',').next()?.parse().ok())
            .unwrap_or(1)
    };
    (num(it.next()), num(it.next()))
}

pub fn new(ctx: &Rc<Ctx>) -> Rc<dyn View> {
    // ----- Timeline column
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search history")
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .build();
    let banner = adw::Banner::builder().button_label("Abort").build();
    let timeline = gtk::ListBox::builder()
        .css_classes(["navigation-sidebar"])
        .build();
    let more = gtk::Button::builder()
        .label("Load More")
        .css_classes(["flat"])
        .visible(false)
        .margin_bottom(6)
        .build();
    let tl_box = util::vbox(0);
    tl_box.append(&timeline);
    tl_box.append(&more);

    let summary = gtk::Entry::builder().placeholder_text("Summary").build();
    let description = gtk::TextBuffer::new(None);
    let desc_view = gtk::TextView::builder()
        .buffer(&description)
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(6)
        .bottom_margin(6)
        .left_margin(6)
        .right_margin(6)
        .accepts_tab(false)
        .build();
    let desc_scroller = gtk::ScrolledWindow::builder()
        .child(&desc_view)
        .min_content_height(56)
        .max_content_height(140)
        .propagate_natural_height(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .css_classes(["card"])
        .build();
    let amend = gtk::CheckButton::with_label("Amend last commit");
    let commit_btn = gtk::Button::builder()
        .label("Commit")
        .css_classes(["suggested-action"])
        .build();
    let composer = util::vbox(8);
    composer.add_css_class("commit-box");
    composer.append(&summary);
    composer.append(&desc_scroller);
    composer.append(&amend);
    composer.append(&commit_btn);

    let left = util::vbox(0);
    left.append(&banner);
    left.append(&search);
    left.append(&scrolled(&tl_box));
    left.append(&composer);

    // ----- Tree column
    let tree_mode = adw::ToggleGroup::builder().css_classes(["round"]).build();
    tree_mode.add(
        adw::Toggle::builder()
            .name("changes")
            .label("Changes")
            .build(),
    );
    tree_mode.add(
        adw::Toggle::builder()
            .name("all")
            .label("All Files")
            .build(),
    );
    tree_mode.set_active_name(Some("changes"));
    let tree_title = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["caption", "dim-label"])
        .margin_start(12)
        .margin_end(12)
        .build();
    let tree = FileTree::new();
    let mid = util::vbox(6);
    let mode_box = util::hbox(0);
    mode_box.set_margin_top(6);
    mode_box.set_margin_start(6);
    mode_box.set_margin_end(6);
    tree_mode.set_hexpand(true);
    mode_box.set_spacing(6);
    mode_box.append(&tree_mode);
    mid.append(&mode_box);
    mid.append(&tree_title);
    mid.append(&tree.widget);
    mid.set_width_request(180);

    // ----- File pane
    let file_title = gtk::Label::builder()
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::Start)
        .selectable(true)
        .css_classes(["mono"])
        .build();
    let mode = adw::ToggleGroup::new();
    mode.add(adw::Toggle::builder().name("diff").label("Changes").build());
    mode.add(adw::Toggle::builder().name("file").label("Preview").build());
    mode.add(adw::Toggle::builder().name("edit").label("Edit").build());
    let discard_btn = util::icon_button("edit-undo-symbolic", "Discard All Changes in File");
    let save_btn = gtk::Button::builder()
        .label("Save")
        .css_classes(["suggested-action"])
        .tooltip_text("Save (Ctrl+S)")
        .visible(false)
        .build();
    let bar = util::hbox(8);
    bar.set_margin_top(6);
    bar.set_margin_bottom(6);
    bar.set_margin_start(12);
    bar.set_margin_end(6);
    bar.append(&file_title);
    bar.append(&mode);
    bar.append(&discard_btn);
    bar.append(&save_btn);

    let tags = tags();
    let diff_view = code_view(false, Some(&tags));
    let preview = adw::Bin::new();
    let editor = editor::view(true);
    let empty = util::status_page(
        "gh-code-symbolic",
        "No File Selected",
        "Pick a file from the tree.",
    );
    let pane = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    pane.add_named(&empty, Some("empty"));
    pane.add_named(&util::spinner_page(), Some("loading"));
    pane.add_named(&scrolled(&diff_view), Some("diff"));
    pane.add_named(&preview, Some("file"));
    pane.add_named(&scrolled(&editor), Some("edit"));
    let right = util::vbox(0);
    right.append(&bar);
    right.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    right.append(&pane);

    let inner = gtk::Paned::builder()
        .start_child(&mid)
        .end_child(&right)
        .position(280)
        .shrink_start_child(false)
        .resize_start_child(false)
        .build();
    let split = adw::OverlaySplitView::builder()
        .sidebar(&left)
        .content(&inner)
        .min_sidebar_width(260.0)
        .max_sidebar_width(360.0)
        .sidebar_width_fraction(0.22)
        .build();
    let timeline_toggle = gtk::ToggleButton::builder()
        .icon_name("sidebar-show-symbolic")
        .tooltip_text("Show History")
        .visible(false)
        .build();
    split
        .bind_property("show-sidebar", &timeline_toggle, "active")
        .bidirectional()
        .sync_create()
        .build();
    mode_box.prepend(&timeline_toggle);
    let root = adw::BreakpointBin::builder()
        .width_request(360)
        .height_request(300)
        .child(&split)
        .build();
    let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        1000.0,
        adw::LengthUnit::Sp,
    ));
    bp.add_setter(&split, "collapsed", Some(&true.to_value()));
    bp.add_setter(&timeline_toggle, "visible", Some(&true.to_value()));
    root.add_breakpoint(bp);

    let view = Rc::new(Code {
        ctx: ctx.clone(),
        root,
        timeline,
        search,
        more,
        banner,
        composer,
        summary,
        description,
        amend,
        commit_btn,
        tree,
        tree_mode,
        tree_title,
        file_title,
        mode,
        discard_btn,
        save_btn,
        pane,
        empty,
        diff_view,
        tags,
        preview,
        editor,
        sel: RefCell::new(Sel::Worktree),
        commits: RefCell::new(Vec::new()),
        unpushed: RefCell::new(HashSet::new()),
        changes: RefCell::new(Vec::new()),
        commit_files: RefCell::new(Vec::new()),
        current: RefCell::new(None),
        diff_file: RefCell::new(None),
        line_map: RefCell::new(Vec::new()),
        editor_path: RefCell::new(None),
        generation: Cell::new(0),
        suppress: Cell::new(false),
    });
    view.connect();
    view
}

impl Code {
    fn connect(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        macro_rules! with {
            ($body:expr) => {{
                let w = w.clone();
                move |_: &_| {
                    if let Some(v) = w.upgrade() {
                        #[allow(clippy::redundant_closure_call)]
                        ($body)(&v);
                    }
                }
            }};
        }
        {
            let w = w.clone();
            adw::StyleManager::default().connect_dark_notify(move |_| {
                let w = w.clone();
                glib::idle_add_local_once(move || {
                    if let Some(v) = w.upgrade()
                        && v.mode.active_name().as_deref() == Some("diff")
                    {
                        v.load_mode();
                    }
                });
            });
        }
        self.search
            .connect_search_changed(with!(|v: &Rc<Code>| v.load_timeline(0)));
        self.more.connect_clicked(with!(|v: &Rc<Code>| {
            let n = v.commits.borrow().len();
            v.load_timeline(n)
        }));
        {
            let w = w.clone();
            self.timeline.connect_row_selected(move |_, row| {
                let (Some(v), Some(row)) = (w.upgrade(), row) else {
                    return;
                };
                if v.suppress.get() {
                    return;
                }
                let name = row.widget_name().to_string();
                let sel = if name == "worktree" {
                    Sel::Worktree
                } else {
                    Sel::Commit(name)
                };
                if *v.sel.borrow() != sel {
                    *v.sel.borrow_mut() = sel;
                    v.load_selection();
                }
            });
        }
        self.tree_mode
            .connect_active_name_notify(with!(|v: &Rc<Code>| v.load_tree()));
        {
            let w = w.clone();
            self.tree.connect_select(move |path| {
                if let Some(v) = w.upgrade() {
                    v.open_file(path);
                }
            });
        }
        {
            let w = w.clone();
            self.tree.connect_menu(move |widget, path, dir, x, y| {
                if let Some(v) = w.upgrade() {
                    let items = v.tree_menu(path, dir);
                    util::popup_menu(widget, x, y, items);
                }
            });
        }
        {
            let w = w.clone();
            self.tree.connect_toggled(move || {
                if let Some(v) = w.upgrade() {
                    v.update_composer();
                }
            });
        }
        self.mode
            .connect_active_name_notify(with!(|v: &Rc<Code>| v.load_mode()));
        self.discard_btn.connect_clicked(with!(|v: &Rc<Code>| {
            if let Some(p) = v.current.borrow().clone() {
                v.discard_files(vec![p]);
            }
        }));
        self.save_btn
            .connect_clicked(with!(|v: &Rc<Code>| v.save()));
        self.summary
            .connect_changed(with!(|v: &Rc<Code>| v.update_composer()));
        self.summary
            .connect_activate(with!(|v: &Rc<Code>| v.do_commit()));
        self.commit_btn
            .connect_clicked(with!(|v: &Rc<Code>| v.do_commit()));
        self.amend.connect_toggled(with!(|v: &Rc<Code>| {
            v.update_composer();
            if v.amend.is_active() && v.summary.text().is_empty() {
                let v2 = v.clone();
                v.ctx.load(git_backend::head_message, move |r| {
                    if let Ok(msg) = r {
                        let (s, d) = msg.split_once('\n').unwrap_or((&msg, ""));
                        v2.summary.set_text(s.trim());
                        v2.description.set_text(d.trim());
                    }
                });
            }
        }));
        self.banner.connect_button_clicked(with!(|v: &Rc<Code>| {
            let state = v.state();
            v.ctx
                .git("Abort", move |l| git_backend::abort_op(l, state), |_| {});
        }));
        self.editor
            .buffer()
            .connect_modified_changed(with!(|v: &Rc<Code>| {
                v.save_btn.set_sensitive(v.editor.buffer().is_modified());
            }));

        // Right-click in the diff.
        let gesture = gtk::GestureClick::builder()
            .button(3)
            .propagation_phase(gtk::PropagationPhase::Capture)
            .build();
        {
            let w = w.clone();
            gesture.connect_pressed(move |g, _, x, y| {
                let Some(v) = w.upgrade() else { return };
                g.set_state(gtk::EventSequenceState::Claimed);
                let items = v.diff_menu(x, y);
                if !items.is_empty() {
                    util::popup_menu(&v.diff_view, x, y, items);
                }
            });
        }
        self.diff_view.add_controller(gesture);

        // Keyboard shortcuts.
        let sc = gtk::ShortcutController::new();
        sc.set_scope(gtk::ShortcutScope::Managed);
        for (trigger, f) in [
            ("<Control>s", (|v: &Rc<Code>| v.save()) as fn(&Rc<Code>)),
            ("<Control>Return", |v: &Rc<Code>| v.do_commit()),
        ] {
            let w = w.clone();
            sc.add_shortcut(gtk::Shortcut::new(
                gtk::ShortcutTrigger::parse_string(trigger),
                Some(gtk::CallbackAction::new(move |_, _| {
                    if let Some(v) = w.upgrade() {
                        f(&v);
                    }
                    glib::Propagation::Stop
                })),
            ));
        }
        self.root.add_controller(sc);
    }

    fn state(&self) -> RepoState {
        self.ctx
            .status
            .borrow()
            .as_ref()
            .map(|s| s.state)
            .unwrap_or(RepoState::Clean)
    }

    fn sel_oid(&self) -> Option<String> {
        match &*self.sel.borrow() {
            Sel::Commit(o) => Some(o.clone()),
            Sel::Worktree => None,
        }
    }

    // ------------------------------------------------------------------ timeline

    fn load_timeline(self: &Rc<Self>, skip: usize) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let q = LogQuery {
            search: Some(self.search.text().to_string()),
            skip,
            limit: PAGE,
            ..Default::default()
        };
        let w = Rc::downgrade(self);
        self.ctx.load(
            move |l| Ok((git_backend::log(l, &q)?, git_backend::unpushed(l)?)),
            move |r| {
                let Some(v) = w.upgrade() else { return };
                if v.generation.get() != generation {
                    return;
                }
                let (commits, unpushed) = match r {
                    Ok(x) => x,
                    Err(e) => return v.ctx.toast(&format!("Could not load history: {e}")),
                };
                v.more.set_visible(commits.len() == PAGE);
                if skip == 0 {
                    *v.commits.borrow_mut() = commits;
                } else {
                    v.commits.borrow_mut().extend(commits);
                }
                *v.unpushed.borrow_mut() = unpushed;
                v.rebuild_timeline();
            },
        );
    }

    fn header_row(
        self: &Rc<Self>,
        title: &str,
        menu: Option<Vec<(&str, Callback)>>,
    ) -> gtk::ListBoxRow {
        let b = util::hbox(6);
        b.set_margin_top(10);
        b.set_margin_start(6);
        b.append(
            &gtk::Label::builder()
                .label(title)
                .xalign(0.0)
                .hexpand(true)
                .css_classes(["heading", "dim-label"])
                .build(),
        );
        if let Some(items) = menu {
            b.append(&util::menu_button(items));
        }
        gtk::ListBoxRow::builder()
            .child(&b)
            .selectable(false)
            .activatable(false)
            .build()
    }

    fn rebuild_timeline(self: &Rc<Self>) {
        self.suppress.set(true);
        util::clear_rows(&self.timeline);
        let changes = self.changes_now();
        let n = changes.len();

        self.timeline.append(&self.header_row("Uncommitted", None));
        let wt = util::hbox(10);
        wt.set_margin_top(6);
        wt.set_margin_bottom(6);
        wt.append(&gtk::Image::from_icon_name("document-edit-symbolic"));
        let texts = util::vbox(2);
        texts.append(
            &gtk::Label::builder()
                .label(if n == 0 {
                    "No local changes".to_owned()
                } else {
                    format!("{n} changed file{}", if n == 1 { "" } else { "s" })
                })
                .xalign(0.0)
                .build(),
        );
        texts.append(
            &gtk::Label::builder()
                .label(format!("on {}", self.ctx.branch()))
                .xalign(0.0)
                .css_classes(["caption", "dim-label"])
                .build(),
        );
        wt.append(&texts);
        let wt_row = gtk::ListBoxRow::builder().child(&wt).build();
        wt_row.set_widget_name("worktree");
        {
            let w = Rc::downgrade(self);
            util::context_menu(&wt_row, move || {
                w.upgrade().map(|v| v.worktree_menu()).unwrap_or_default()
            });
        }
        self.timeline.append(&wt_row);

        let unpushed = self.unpushed.borrow().clone();
        let commits = self.commits.borrow().clone();
        let mut in_unpushed = false;
        let mut in_pushed = false;
        let n_unpushed = commits.iter().filter(|c| unpushed.contains(&c.oid)).count();
        let selected = self.sel.borrow().clone();
        let mut select_row: Option<gtk::ListBoxRow> =
            (selected == Sel::Worktree).then(|| wt_row.clone());
        for c in &commits {
            let is_unpushed = unpushed.contains(&c.oid);
            if is_unpushed && !in_unpushed {
                in_unpushed = true;
                let menu = self.unpushed_menu();
                self.timeline
                    .append(&self.header_row(&format!("Not Pushed ({n_unpushed})"), Some(menu)));
            }
            if !is_unpushed && !in_pushed {
                in_pushed = true;
                self.timeline.append(&self.header_row("History", None));
            }
            let row = self.commit_row(c, is_unpushed);
            if selected == Sel::Commit(c.oid.clone()) {
                select_row = Some(row.clone());
            }
            self.timeline.append(&row);
        }
        let reselect = select_row.is_some();
        self.timeline
            .select_row(Some(&select_row.unwrap_or(wt_row)));
        self.suppress.set(false);
        if !reselect {
            *self.sel.borrow_mut() = Sel::Worktree;
        }
    }

    fn commit_row(self: &Rc<Self>, c: &Commit, unpushed: bool) -> gtk::ListBoxRow {
        let b = util::hbox(10);
        b.set_margin_top(4);
        b.set_margin_bottom(4);
        let icon = gtk::Image::from_icon_name(if unpushed {
            "go-up-symbolic"
        } else {
            "gh-commit-symbolic"
        });
        icon.add_css_class(if unpushed { "accent" } else { "dim-label" });
        icon.set_valign(gtk::Align::Start);
        icon.set_margin_top(2);
        b.append(&icon);
        let texts = util::vbox(3);
        texts.append(
            &gtk::Label::builder()
                .label(&c.subject)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build(),
        );
        let meta = util::hbox(6);
        for r in c.refs.iter().take(3) {
            let (text, class) = if let Some(t) = r.strip_prefix("tag: ") {
                (t.to_owned(), "warning")
            } else if let Some(h) = r.strip_prefix("HEAD -> ") {
                (h.to_owned(), "accent")
            } else {
                (r.clone(), "success")
            };
            meta.append(&util::pill(&text, class));
        }
        meta.append(
            &gtk::Label::builder()
                .label(format!(
                    "{} · {} · {}",
                    c.short,
                    c.author,
                    util::relative_time(c.time)
                ))
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["caption", "dim-label"])
                .build(),
        );
        texts.append(&meta);
        b.append(&texts);
        let row = gtk::ListBoxRow::builder()
            .child(&b)
            .tooltip_text(&c.subject)
            .build();
        row.set_widget_name(&c.oid);
        let w = Rc::downgrade(self);
        let c = c.clone();
        util::context_menu(&row, move || {
            w.upgrade()
                .map(|v| v.commit_menu(&c, unpushed))
                .unwrap_or_default()
        });
        row
    }

    // ------------------------------------------------------------------ selection & tree

    fn changes_now(&self) -> Vec<FileChange> {
        self.ctx
            .status
            .borrow()
            .as_ref()
            .map(|s| s.files.clone())
            .unwrap_or_default()
    }

    fn load_selection(self: &Rc<Self>) {
        let is_wt = *self.sel.borrow() == Sel::Worktree;
        self.composer.set_visible(
            is_wt && (!self.changes.borrow().is_empty() || self.state() != RepoState::Clean),
        );
        if is_wt && self.changes.borrow().is_empty() {
            self.tree_mode.set_active_name(Some("all"));
        } else if !is_wt {
            self.tree_mode.set_active_name(Some("changes"));
        }
        self.load_tree();
    }

    fn load_tree(self: &Rc<Self>) {
        let all = self.tree_mode.active_name().as_deref() == Some("all");
        let sel = self.sel.borrow().clone();
        let w = Rc::downgrade(self);
        match sel {
            Sel::Worktree => {
                let changes = self.changes.borrow().clone();
                self.tree_title
                    .set_label(&format!("Working tree · {}", self.ctx.branch()));
                let files: Vec<(String, Option<char>)> = changes
                    .iter()
                    .map(|f| (f.path.clone(), Some(change_letter(f))))
                    .collect();
                if !all {
                    self.tree.set_files(&files, true, true);
                    self.after_tree();
                    return;
                }
                self.ctx.load(git_backend::worktree_files, move |r| {
                    let Some(v) = w.upgrade() else { return };
                    let mut list: Vec<(String, Option<char>)> = r
                        .unwrap_or_default()
                        .into_iter()
                        .map(|p| {
                            let st = files.iter().find(|(f, _)| *f == p).and_then(|x| x.1);
                            (p, st)
                        })
                        .collect();
                    // Deleted files are not on disk but still belong in the tree.
                    for (p, st) in &files {
                        if !list.iter().any(|(x, _)| x == p) {
                            list.push((p.clone(), *st));
                        }
                    }
                    v.tree.set_files(&list, true, false);
                    v.after_tree();
                });
            }
            Sel::Commit(oid) => {
                let short = oid[..oid.len().min(8)].to_owned();
                self.tree_title.set_label(&format!("Commit {short}"));
                self.ctx.load(
                    move |l| {
                        let files = git_backend::commit_files(l, &oid)?;
                        let tree = if all {
                            git_backend::tree_files(l, &oid)?
                        } else {
                            Vec::new()
                        };
                        Ok((files, tree))
                    },
                    move |r| {
                        let Some(v) = w.upgrade() else { return };
                        let (files, tree) = r.unwrap_or_default();
                        let mut list: Vec<(String, Option<char>)> = files
                            .iter()
                            .map(|f| (f.path.clone(), Some(f.status)))
                            .collect();
                        if all {
                            for p in tree {
                                if !list.iter().any(|(x, _)| *x == p) {
                                    list.push((p, None));
                                }
                            }
                        }
                        *v.commit_files.borrow_mut() = files;
                        v.tree.set_files(&list, false, !all);
                        v.after_tree();
                    },
                );
            }
        }
    }

    /// After the tree is rebuilt: reopen the current file, or pick a sensible one.
    fn after_tree(self: &Rc<Self>) {
        let current = self.current.borrow().clone();
        let pick = current
            .filter(|p| self.tree.has_file(p))
            .or_else(|| match &*self.sel.borrow() {
                Sel::Worktree => self.changes.borrow().first().map(|f| f.path.clone()),
                Sel::Commit(_) => self.commit_files.borrow().first().map(|f| f.path.clone()),
            })
            .or_else(|| {
                ["README.md", "readme.md", "README.markdown", "README"]
                    .iter()
                    .find(|p| self.tree.has_file(p))
                    .map(|p| p.to_string())
            });
        match pick {
            Some(p) => {
                self.tree.select_path(&p);
                self.open_file(p);
            }
            None => {
                *self.current.borrow_mut() = None;
                self.file_title.set_label("");
                self.mode.set_sensitive(false);
                self.discard_btn.set_visible(false);
                self.save_btn.set_visible(false);
                self.empty
                    .set_title(if *self.sel.borrow() == Sel::Worktree {
                        "Nothing Changed"
                    } else {
                        "No File Selected"
                    });
                self.pane.set_visible_child_name("empty");
            }
        }
        self.update_composer();
    }

    // ------------------------------------------------------------------ file pane

    fn worktree_change(&self, path: &str) -> Option<FileChange> {
        self.changes
            .borrow()
            .iter()
            .find(|f| f.path == path)
            .cloned()
    }

    fn commit_file(&self, path: &str) -> Option<CommitFile> {
        self.commit_files
            .borrow()
            .iter()
            .find(|f| f.path == path)
            .cloned()
    }

    fn open_file(self: &Rc<Self>, path: String) {
        let is_wt = *self.sel.borrow() == Sel::Worktree;
        let (has_diff, deleted) = if is_wt {
            let c = self.worktree_change(&path);
            (
                c.is_some(),
                c.is_some_and(|c| c.worktree == 'D' || (c.index == 'D' && c.worktree == '.')),
            )
        } else {
            let c = self.commit_file(&path);
            (c.is_some(), c.is_some_and(|c| c.status == 'D'))
        };
        let changed_file = self.current.borrow().as_deref() != Some(path.as_str());
        *self.current.borrow_mut() = Some(path.clone());
        self.file_title.set_label(&path);
        self.mode.set_sensitive(true);
        let set_enabled = |name: &str, on: bool| {
            if let Some(t) = self.mode.toggle_by_name(name) {
                t.set_enabled(on);
            }
        };
        set_enabled("diff", has_diff);
        set_enabled("file", !deleted);
        set_enabled("edit", is_wt && !deleted);
        if let Some(t) = self.mode.toggle_by_name("file") {
            t.set_label(Some(if is_markdown(&path) {
                "Preview"
            } else {
                "File"
            }));
        }
        self.discard_btn.set_visible(is_wt && has_diff);
        let cur = self
            .mode
            .active_name()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let wanted = if !changed_file
            && self
                .mode
                .toggle_by_name(&cur)
                .is_some_and(|t| t.is_enabled())
        {
            cur
        } else if has_diff {
            "diff".into()
        } else {
            "file".into()
        };
        if self.mode.active_name().as_deref() == Some(wanted.as_str()) {
            self.load_mode();
        } else {
            self.mode.set_active_name(Some(&wanted));
        }
    }

    fn load_mode(self: &Rc<Self>) {
        let Some(path) = self.current.borrow().clone() else {
            return;
        };
        let mode = self
            .mode
            .active_name()
            .map(|s| s.to_string())
            .unwrap_or_default();
        self.save_btn.set_visible(mode == "edit");
        let oid = self.sel_oid();
        let w = Rc::downgrade(self);
        match mode.as_str() {
            "diff" => {
                let change = self.worktree_change(&path);
                let cfile = self.commit_file(&path);
                self.ctx.load(
                    move |l| match (oid, change, cfile) {
                        (None, Some(c), _) => git_backend::worktree_diff(l, &c),
                        (Some(o), _, Some(f)) => git_backend::commit_file_diff(l, &o, &f),
                        _ => Ok(String::new()),
                    },
                    move |r| {
                        let Some(v) = w.upgrade() else { return };
                        if v.current.borrow().as_deref() != Some(path.as_str()) {
                            return;
                        }
                        match r {
                            Ok(d) => v.render_diff(&d),
                            Err(e) => v.show_error(&e),
                        }
                    },
                );
            }
            "edit" => {
                // Don't clobber unsaved edits to the same file on refresh.
                if self.editor_path.borrow().as_deref() == Some(path.as_str())
                    && self.editor.buffer().is_modified()
                {
                    self.pane.set_visible_child_name("edit");
                    return;
                }
                let p = path.clone();
                self.ctx.load(
                    move |l| l.read_file(&p),
                    move |r| {
                        let Some(v) = w.upgrade() else { return };
                        match r.and_then(|b| {
                            String::from_utf8(b).map_err(|_| {
                                anyhow::anyhow!(
                                    "This file is not valid UTF-8 text and cannot be edited here."
                                )
                            })
                        }) {
                            Ok(text) => {
                                editor::set_text(&v.editor, &path, &text);
                                *v.editor_path.borrow_mut() = Some(path);
                                v.save_btn.set_sensitive(false);
                                v.pane.set_visible_child_name("edit");
                            }
                            Err(e) => v.show_error(&e),
                        }
                    },
                );
            }
            _ => {
                let p = path.clone();
                self.ctx.load(
                    move |l| match oid {
                        None => l.read_file(&p),
                        Some(o) => git_backend::file_at(l, &o, &p),
                    },
                    move |r| {
                        let Some(v) = w.upgrade() else { return };
                        if v.current.borrow().as_deref() != Some(path.as_str()) {
                            return;
                        }
                        match r {
                            Ok(bytes) => v.render_file(&path, bytes),
                            Err(e) => v.show_error(&e),
                        }
                    },
                );
            }
        }
    }

    fn show_error(&self, e: &anyhow::Error) {
        self.empty.set_title("Cannot Show File");
        self.empty
            .set_description(Some(&glib::markup_escape_text(&e.to_string())));
        self.pane.set_visible_child_name("empty");
        self.empty
            .set_description(Some("Pick a file from the tree."));
    }

    fn render_file(&self, path: &str, bytes: Vec<u8>) {
        if bytes.len() > MAX_VIEW {
            self.preview.set_child(Some(&util::status_page(
                "text-x-generic-symbolic",
                "File Too Large",
                &util::human_size(bytes.len() as u64),
            )));
        } else if let Ok(text) = String::from_utf8(bytes) {
            if is_markdown(path) {
                let md = markdown::render(&text);
                md.set_margin_top(24);
                md.set_margin_bottom(24);
                md.set_margin_start(32);
                md.set_margin_end(32);
                self.preview.set_child(Some(&scrolled(&md)));
            } else {
                let view = editor::view(false);
                editor::set_text(&view, path, &text);
                self.preview.set_child(Some(&scrolled(&view)));
            }
        } else {
            self.preview.set_child(Some(&util::status_page(
                "text-x-generic-symbolic",
                "Binary File",
                "This file cannot be shown as text.",
            )));
        }
        self.pane.set_visible_child_name("file");
    }

    fn render_diff(&self, text: &str) {
        recolor(&self.tags, self.diff_view.color());
        let file = git_backend::parse_diff(text).into_iter().next();
        let buf = self.diff_view.buffer();
        buf.set_text("");
        let mut map = Vec::new();
        match &file {
            None => {
                buf.set_text("No textual changes.");
                map.push(None);
            }
            Some(f) if f.binary || f.hunks.is_empty() => {
                buf.set_text(if f.binary {
                    "Binary file changed."
                } else {
                    "Mode or rename only; no content changes."
                });
                map.push(None);
            }
            Some(f) => {
                let mut first = true;
                let path = self.current.borrow().clone().unwrap_or_default();
                // (offset of the line's marker char, is_new_side, index within that side)
                let mut spots: Vec<(i32, bool, usize)> = Vec::new();
                let mut old_side: Vec<String> = Vec::new();
                let mut new_side: Vec<String> = Vec::new();
                for (h, hunk) in f.hunks.iter().enumerate() {
                    let mut lines = hunk.split_inclusive('\n');
                    let head = lines.next().unwrap_or_default();
                    let (mut old, mut new) = hunk_starts(head);
                    let mut end = buf.end_iter();
                    if !first {
                        buf.insert(&mut end, "\n");
                        map.push(None);
                    }
                    first = false;
                    buf.insert_with_tags_by_name(&mut end, head.trim_end(), &["hunk"]);
                    map.push(Some((h, usize::MAX)));
                    for (i, line) in lines.enumerate() {
                        let kind = line.as_bytes().first().copied().unwrap_or(b' ');
                        let (o, n, tag) = match kind {
                            b'+' => {
                                new += 1;
                                (String::new(), (new - 1).to_string(), Some("added"))
                            }
                            b'-' => {
                                old += 1;
                                ((old - 1).to_string(), String::new(), Some("removed"))
                            }
                            b'\\' => (String::new(), String::new(), None),
                            _ => {
                                old += 1;
                                new += 1;
                                ((old - 1).to_string(), (new - 1).to_string(), None)
                            }
                        };
                        let mut end = buf.end_iter();
                        buf.insert(&mut end, "\n");
                        let start_off = end.offset();
                        buf.insert_with_tags_by_name(
                            &mut end,
                            &format!("{o:>5} {n:>5}  "),
                            &["gutter"],
                        );
                        let content_off = end.offset();
                        buf.insert(&mut end, line.trim_end_matches('\n'));
                        let body = line
                            .trim_end_matches('\n')
                            .get(1..)
                            .unwrap_or("")
                            .to_owned();
                        match kind {
                            b'+' => {
                                spots.push((content_off, true, new_side.len()));
                                new_side.push(body);
                            }
                            b'-' => {
                                spots.push((content_off, false, old_side.len()));
                                old_side.push(body);
                            }
                            b'\\' => {}
                            _ => {
                                spots.push((content_off, true, new_side.len()));
                                new_side.push(body.clone());
                                old_side.push(body);
                            }
                        }
                        if let Some(t) = tag {
                            let s = buf.iter_at_offset(start_off);
                            let e = buf.end_iter();
                            buf.apply_tag_by_name(t, &s, &e);
                        }
                        map.push(Some((h, i)));
                    }
                    // A blank line between hunks keeps their syntax state apart.
                    old_side.push(String::new());
                    new_side.push(String::new());
                }
                self.apply_syntax(&path, &spots, &old_side, &new_side);
            }
        }
        *self.diff_file.borrow_mut() = file;
        *self.line_map.borrow_mut() = map;
        self.pane.set_visible_child_name("diff");
    }

    /// Copy syntax colours from both sides of the diff onto the diff buffer.
    fn apply_syntax(
        &self,
        path: &str,
        spots: &[(i32, bool, usize)],
        old: &[String],
        new: &[String],
    ) {
        let olds: Vec<&str> = old.iter().map(String::as_str).collect();
        let news: Vec<&str> = new.iter().map(String::as_str).collect();
        let old_hl = editor::highlight(path, &olds);
        let new_hl = editor::highlight(path, &news);
        let buf = self.diff_view.buffer();
        let table = buf.tag_table();
        for &(off, is_new, idx) in spots {
            let spans = if is_new {
                new_hl.get(idx)
            } else {
                old_hl.get(idx)
            };
            for (a, b, style) in spans.into_iter().flatten() {
                let key = style.key();
                let tag = table.lookup(&key).unwrap_or_else(|| {
                    let t = gtk::TextTag::new(Some(&key));
                    if let Some(fg) = style.fg {
                        t.set_foreground_rgba(Some(&fg));
                    }
                    if style.bold {
                        t.set_weight(700);
                    }
                    if style.italic {
                        t.set_style(gtk::pango::Style::Italic);
                    }
                    table.add(&t);
                    t
                });
                // +1 skips the diff marker (+, - or space).
                let s = buf.iter_at_offset(off + 1 + *a as i32);
                let e = buf.iter_at_offset(off + 1 + *b as i32);
                buf.apply_tag(&tag, &s, &e);
            }
        }
    }

    fn save(self: &Rc<Self>) {
        let Some(path) = self.editor_path.borrow().clone() else {
            return;
        };
        if self.mode.active_name().as_deref() != Some("edit") {
            return;
        }
        let buf = self.editor.buffer();
        let text = buf
            .text(&buf.start_iter(), &buf.end_iter(), false)
            .to_string();
        let w = Rc::downgrade(self);
        self.ctx.git(
            "Save",
            move |l| l.write_file(&path, text.as_bytes()),
            move |_| {
                if let Some(v) = w.upgrade() {
                    v.editor.buffer().set_modified(false);
                    v.ctx.toast("Saved");
                }
            },
        );
    }

    // ------------------------------------------------------------------ diff actions

    /// Diff lines (hunk, line) under the click, or the selection if the click is inside it.
    fn lines_at(&self, x: f64, y: f64) -> (Vec<(usize, usize)>, Option<usize>) {
        let map = self.line_map.borrow();
        let (bx, by) =
            self.diff_view
                .window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        let Some(at) = self.diff_view.iter_at_location(bx, by) else {
            return (Vec::new(), None);
        };
        let buf = self.diff_view.buffer();
        let (from, to) = match buf.selection_bounds() {
            Some((a, b)) if at.line() >= a.line() && at.line() <= b.line() => (a.line(), b.line()),
            _ => (at.line(), at.line()),
        };
        let mut out = Vec::new();
        let mut hunk = None;
        for l in from..=to {
            if let Some(Some((h, i))) = map.get(l as usize) {
                hunk = Some(*h);
                if *i != usize::MAX {
                    out.push((*h, *i));
                }
            }
        }
        (out, hunk)
    }

    fn diff_menu(self: &Rc<Self>, x: f64, y: f64) -> Vec<(String, Callback)> {
        let Some(file) = self.diff_file.borrow().clone() else {
            return Vec::new();
        };
        let (lines, hunk) = self.lines_at(x, y);
        let changes = git_backend::expand_to_changes(&file, &lines);
        let is_wt = *self.sel.borrow() == Sel::Worktree;
        let untracked = is_wt
            && self
                .current
                .borrow()
                .as_deref()
                .and_then(|p| self.worktree_change(p))
                .is_some_and(|c| c.untracked());
        let mut items: Vec<(String, Callback)> = Vec::new();
        let verb = if is_wt {
            "Discard"
        } else {
            "Revert in Working Tree:"
        };
        let w = Rc::downgrade(self);
        let act = |sel: Vec<(usize, usize)>, what: &'static str| -> Callback {
            let w = w.clone();
            let file = file.clone();
            Rc::new(move || {
                if let Some(v) = w.upgrade() {
                    let file = file.clone();
                    let sel = sel.clone();
                    v.ctx.git(
                        what,
                        move |l| git_backend::discard_lines(l, &file, &sel),
                        |_| {},
                    );
                }
            })
        };
        if !untracked {
            if !changes.is_empty() {
                let label = if lines.len() > 1 {
                    "Selected Changes"
                } else {
                    "This Change"
                };
                items.push((format!("{verb} {label}"), act(changes, "Discard Changes")));
            }
            if let Some(h) = hunk {
                let all: Vec<(usize, usize)> = file.hunks[h]
                    .split_inclusive('\n')
                    .skip(1)
                    .enumerate()
                    .filter(|(_, l)| l.starts_with('+') || l.starts_with('-'))
                    .map(|(i, _)| (h, i))
                    .collect();
                items.push((format!("{verb} Hunk"), act(all, "Discard Hunk")));
            }
        }
        if is_wt {
            let w = w.clone();
            items.push((
                "Discard All Changes in File…".into(),
                Rc::new(move || {
                    if let Some(v) = w.upgrade()
                        && let Some(p) = v.current.borrow().clone()
                    {
                        v.discard_files(vec![p]);
                    }
                }),
            ));
        }
        items.push((String::new(), Rc::new(|| {})));
        let view = self.diff_view.clone();
        items.push((
            "Copy".into(),
            Rc::new(move || {
                let buf = view.buffer();
                if let Some((a, b)) = buf.selection_bounds() {
                    util::copy_text(&view, &buf.text(&a, &b, false));
                }
            }),
        ));
        if let Some(p) = self.current.borrow().clone() {
            let w = w.clone();
            items.push((
                "Edit File".into(),
                Rc::new(move || {
                    if let Some(v) = w.upgrade() {
                        v.edit(p.clone());
                    }
                }),
            ));
        }
        items
    }

    fn edit(self: &Rc<Self>, path: String) {
        if *self.sel.borrow() != Sel::Worktree {
            // Jump to the working tree to edit.
            *self.sel.borrow_mut() = Sel::Worktree;
            self.rebuild_timeline();
            self.tree_mode.set_active_name(Some("all"));
        }
        *self.current.borrow_mut() = Some(path.clone());
        self.tree.select_path(&path);
        self.open_file(path);
        self.mode.set_active_name(Some("edit"));
    }

    fn discard_files(self: &Rc<Self>, paths: Vec<String>) {
        let changes: Vec<FileChange> = self
            .changes
            .borrow()
            .iter()
            .filter(|f| {
                paths
                    .iter()
                    .any(|p| f.path == *p || f.path.starts_with(p.as_str()) && p.ends_with('/'))
            })
            .cloned()
            .collect();
        if changes.is_empty() {
            return;
        }
        let ctx = self.ctx.clone();
        glib::spawn_future_local(async move {
            let body = if changes.len() == 1 {
                if changes[0].untracked() {
                    format!("“{}” is new and will be deleted.", changes[0].path)
                } else {
                    format!("All changes to “{}” will be lost.", changes[0].path)
                }
            } else {
                format!(
                    "Changes to {} files will be lost. New files will be deleted.",
                    changes.len()
                )
            };
            if util::confirm(&ctx.parent(), "Discard Changes?", &body, "Discard", true).await {
                ctx.git(
                    "Discard",
                    move |l| {
                        for c in &changes {
                            git_backend::discard_file(l, c)?;
                        }
                        Ok(())
                    },
                    |_| {},
                );
            }
        });
    }

    // ------------------------------------------------------------------ composer

    fn included(&self) -> Vec<FileChange> {
        let ex = self.tree.excluded.borrow();
        self.changes
            .borrow()
            .iter()
            .filter(|f| f.conflicted || !ex.contains(&f.path))
            .cloned()
            .collect()
    }

    fn default_summary(&self) -> String {
        let names: Vec<String> = self
            .included()
            .iter()
            .map(|f| basename(&f.path).to_owned())
            .collect();
        let mut s = String::from("Update ");
        const SHOW: usize = 4;
        s.push_str(
            &names
                .iter()
                .take(SHOW)
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
        );
        if names.len() > SHOW {
            s.push_str(&format!(" and {} more", names.len() - SHOW));
        }
        s
    }

    fn update_composer(&self) {
        let included = self.included().len();
        let state = self.state();
        let branch = self.ctx.branch();
        let amend = self.amend.is_active();
        let conflicts = self
            .changes
            .borrow()
            .iter()
            .filter(|f| f.conflicted)
            .count();
        self.summary.set_placeholder_text(Some(&if included > 0 {
            self.default_summary()
        } else {
            "Summary".into()
        }));
        let (label, enabled) = match state {
            RepoState::Rebasing | RepoState::CherryPicking | RepoState::Reverting => {
                let what = match state {
                    RepoState::Rebasing => "Rebase",
                    RepoState::CherryPicking => "Cherry-pick",
                    _ => "Revert",
                };
                (format!("Continue {what}"), conflicts == 0)
            }
            RepoState::Merging => ("Commit Merge".to_owned(), conflicts == 0),
            RepoState::Clean if amend => (format!("Amend Last Commit on {branch}"), true),
            RepoState::Clean => (
                format!(
                    "Commit {included} File{} to {branch}",
                    if included == 1 { "" } else { "s" }
                ),
                included > 0,
            ),
        };
        self.commit_btn.set_label(&label);
        self.commit_btn.set_sensitive(enabled);
        let (title, revealed) = match state {
            RepoState::Clean => ("", false),
            RepoState::Merging => ("Merge in progress — resolve conflicts, then commit", true),
            RepoState::Rebasing => (
                "Rebase in progress — resolve conflicts, then continue",
                true,
            ),
            RepoState::CherryPicking => ("Cherry-pick in progress", true),
            RepoState::Reverting => ("Revert in progress", true),
        };
        self.banner.set_title(title);
        self.banner.set_revealed(revealed);
    }

    fn do_commit(self: &Rc<Self>) {
        if !self.commit_btn.is_sensitive() || !self.composer.is_visible() {
            return;
        }
        let state = self.state();
        let included: Vec<String> = self.included().into_iter().map(|f| f.path).collect();
        let excluded: Vec<String> = self
            .changes
            .borrow()
            .iter()
            .filter(|f| !included.contains(&f.path))
            .map(|f| f.path.clone())
            .collect();
        let summary = self.summary.text().trim().to_owned();
        let summary = if summary.is_empty() {
            self.default_summary()
        } else {
            summary
        };
        let d = &self.description;
        let body = d
            .text(&d.start_iter(), &d.end_iter(), false)
            .trim()
            .to_owned();
        let msg = if body.is_empty() {
            summary
        } else {
            format!("{summary}\n\n{body}")
        };
        let amend = self.amend.is_active();
        let signoff = settings::get().signoff;
        let w = Rc::downgrade(self);
        self.ctx.git(
            "Commit",
            move |l| {
                let inc: Vec<&str> = included.iter().map(String::as_str).collect();
                let exc: Vec<&str> = excluded.iter().map(String::as_str).collect();
                match state {
                    RepoState::Clean | RepoState::Merging => {
                        git_backend::commit_paths(l, &inc, &exc, &msg, amend, signoff)
                    }
                    _ => {
                        git_backend::stage(l, &inc)?;
                        git_backend::continue_op(l, state)
                    }
                }
            },
            move |_| {
                if let Some(v) = w.upgrade() {
                    v.summary.set_text("");
                    v.description.set_text("");
                    v.amend.set_active(false);
                    v.ctx.toast("Committed");
                }
            },
        );
    }

    // ------------------------------------------------------------------ menus

    fn worktree_menu(self: &Rc<Self>) -> Vec<(String, Callback)> {
        let ctx = self.ctx.clone();
        let w = Rc::downgrade(self);
        let mut items: Vec<(String, Callback)> = vec![
            ("Stash All Changes".into(), {
                let ctx = ctx.clone();
                Rc::new(move || ctx.git("Stash", |l| git_backend::stash_push(l, "", true), |_| {}))
            }),
            ("Discard All Changes…".into(), {
                let w = w.clone();
                Rc::new(move || {
                    if let Some(v) = w.upgrade() {
                        let all = v.changes.borrow().iter().map(|f| f.path.clone()).collect();
                        v.discard_files(all);
                    }
                })
            }),
            (String::new(), Rc::new(|| {})),
        ];
        for (label, include) in [
            ("Include All in Commit", true),
            ("Exclude All from Commit", false),
        ] {
            let w = w.clone();
            items.push((
                label.into(),
                Rc::new(move || {
                    if let Some(v) = w.upgrade() {
                        {
                            let mut ex = v.tree.excluded.borrow_mut();
                            ex.clear();
                            if !include {
                                ex.extend(v.changes.borrow().iter().map(|f| f.path.clone()));
                            }
                        }
                        v.tree.sync();
                        v.update_composer();
                    }
                }),
            ));
        }
        items
    }

    fn unpushed_menu(self: &Rc<Self>) -> Vec<(&'static str, Callback)> {
        let w = Rc::downgrade(self);
        let ctx = self.ctx.clone();
        vec![
            ("Push", {
                let ctx = ctx.clone();
                Rc::new(move || {
                    let c = ctx.clone();
                    ctx.git(
                        "Push",
                        |l| git_backend::push(l, false),
                        move |_| c.toast("Pushed"),
                    )
                })
            }),
            ("Squash All into One Commit…", {
                let w = w.clone();
                Rc::new(move || {
                    if let Some(v) = w.upgrade() {
                        v.squash_unpushed();
                    }
                })
            }),
            ("Undo Last Commit", {
                let ctx = ctx.clone();
                Rc::new(move || {
                    let c = ctx.clone();
                    ctx.git("Undo Commit", git_backend::undo_last_commit, move |_| {
                        c.toast("Last commit undone; its changes are back in Uncommitted")
                    })
                })
            }),
        ]
    }

    fn squash_unpushed(self: &Rc<Self>) {
        let unpushed = self.unpushed.borrow().clone();
        let Some(oldest) = self
            .commits
            .borrow()
            .iter()
            .rfind(|c| unpushed.contains(&c.oid))
            .cloned()
        else {
            return;
        };
        if oldest.parents.is_empty() {
            self.ctx
                .toast("Cannot squash: the oldest unpushed commit is the first commit");
            return;
        }
        let base = format!("{}^", oldest.oid);
        let ctx = self.ctx.clone();
        let b2 = base.clone();
        self.ctx.load(
            move |l| git_backend::messages_since(l, &b2),
            move |r| {
                let messages = r.unwrap_or_default();
                glib::spawn_future_local(async move {
                    let Some(v) = form::run(
                        &ctx.parent(),
                        "Squash Unpushed Commits",
                        "All commits that are not yet pushed become one commit.",
                        "Squash",
                        vec![Field::text("Message", messages.trim())],
                    )
                    .await
                    else {
                        return;
                    };
                    let msg = v[0].text();
                    let c = ctx.clone();
                    ctx.git(
                        "Squash",
                        move |l| git_backend::squash_since(l, &base, &msg),
                        move |_| c.toast("Squashed"),
                    );
                });
            },
        );
    }

    fn commit_menu(self: &Rc<Self>, c: &Commit, unpushed: bool) -> Vec<(String, Callback)> {
        let ctx = self.ctx.clone();
        let oid = c.oid.clone();
        let is_head = self
            .commits
            .borrow()
            .first()
            .is_some_and(|h| h.oid == c.oid)
            && self.search.text().is_empty();
        let merge = c.parents.len() > 1;
        let mk = |f: fn(Rc<Ctx>, String)| -> Callback {
            let ctx = ctx.clone();
            let oid = oid.clone();
            Rc::new(move || f(ctx.clone(), oid.clone()))
        };
        let rw = |how: fn() -> Rewrite, what: &'static str, confirm: bool| -> Callback {
            let ctx = ctx.clone();
            let oid = oid.clone();
            Rc::new(move || rewrite(ctx.clone(), oid.clone(), how(), what, unpushed, confirm))
        };
        let mut items: Vec<(String, Callback)> = vec![("Copy SHA".into(), {
            let (ctx, oid) = (ctx.clone(), oid.clone());
            Rc::new(move || {
                util::copy_text(&ctx.parent(), &oid);
                ctx.toast("Copied commit SHA");
            })
        })];
        if ctx.slug.borrow().is_some() {
            items.push(("Open on GitHub".into(), mk(open_on_github)));
        }
        items.push((String::new(), Rc::new(|| {})));
        items.push(("Check Out This Commit".into(), mk(checkout)));
        items.push(("New Branch from Here…".into(), {
            let (ctx, oid) = (ctx.clone(), oid.clone());
            Rc::new(move || crate::views::branches::new_branch_dialog(&ctx, Some(oid.clone())))
        }));
        items.push(("New Tag Here…".into(), mk(tag)));
        items.push((String::new(), Rc::new(|| {})));
        if !merge {
            items.push(("Edit Message…".into(), mk(reword)));
            if !c.parents.is_empty() {
                items.push((
                    "Squash into Previous Commit".into(),
                    rw(|| Rewrite::Squash, "Squash", false),
                ));
                items.push((
                    "Fixup into Previous (Keep Its Message)".into(),
                    rw(|| Rewrite::Fixup, "Fixup", false),
                ));
            }
            if is_head && !c.parents.is_empty() {
                items.push(("Undo Commit (Keep Changes)".into(), {
                    let ctx = ctx.clone();
                    Rc::new(move || {
                        let c2 = ctx.clone();
                        ctx.git("Undo Commit", git_backend::undo_last_commit, move |_| {
                            c2.toast("Commit undone; its changes are back in Uncommitted")
                        })
                    })
                }));
            }
            items.push((
                "Drop Commit…".into(),
                rw(|| Rewrite::Drop, "Drop Commit", true),
            ));
            items.push((String::new(), Rc::new(|| {})));
        }
        items.push(("Revert Commit".into(), mk(revert)));
        items.push(("Cherry-pick onto Current Branch".into(), mk(cherry_pick)));
        items.push(("Reset Current Branch to Here…".into(), mk(reset)));
        let _ = unpushed;
        items
    }

    fn tree_menu(self: &Rc<Self>, path: String, dir: bool) -> Vec<(String, Callback)> {
        let ctx = self.ctx.clone();
        let w = Rc::downgrade(self);
        let is_wt = *self.sel.borrow() == Sel::Worktree;
        let mut items: Vec<(String, Callback)> = Vec::new();
        let changed = if dir {
            self.changes
                .borrow()
                .iter()
                .any(|f| f.path.starts_with(&path))
        } else {
            self.worktree_change(&path).is_some()
        };
        if is_wt {
            if !dir {
                items.push(("Edit".into(), {
                    let (w, p) = (w.clone(), path.clone());
                    Rc::new(move || {
                        if let Some(v) = w.upgrade() {
                            v.edit(p.clone());
                        }
                    })
                }));
            }
            if changed {
                let excluded = self.tree.excluded.borrow().contains(&path);
                if !dir {
                    items.push((
                        if excluded {
                            "Include in Commit"
                        } else {
                            "Exclude from Commit"
                        }
                        .into(),
                        {
                            let (w, p) = (w.clone(), path.clone());
                            Rc::new(move || {
                                if let Some(v) = w.upgrade() {
                                    {
                                        let mut ex = v.tree.excluded.borrow_mut();
                                        if !ex.remove(&p) {
                                            ex.insert(p.clone());
                                        }
                                    }
                                    v.tree.sync();
                                    v.update_composer();
                                }
                            })
                        },
                    ));
                }
                items.push((
                    if dir {
                        "Discard Changes in Folder…"
                    } else {
                        "Discard Changes…"
                    }
                    .into(),
                    {
                        let (w, p) = (w.clone(), path.clone());
                        Rc::new(move || {
                            if let Some(v) = w.upgrade() {
                                v.discard_files(vec![p.clone()]);
                            }
                        })
                    },
                ));
            }
            let untracked = self.worktree_change(&path).is_some_and(|c| c.untracked());
            if dir || untracked {
                items.push(("Add to .gitignore".into(), {
                    let (ctx, p) = (ctx.clone(), path.clone());
                    Rc::new(move || {
                        let pattern = format!("/{p}");
                        ctx.git("Ignore", move |l| git_backend::ignore(l, &pattern), |_| {});
                    })
                }));
            }
        } else if let Some(oid) = self.sel_oid()
            && !dir
        {
            let deleted = self.commit_file(&path).is_some_and(|f| f.status == 'D');
            if !deleted {
                items.push(("Restore This Version to Working Tree".into(), {
                    let (ctx, p, o) = (ctx.clone(), path.clone(), oid.clone());
                    Rc::new(move || {
                        let (p, o) = (p.clone(), o.clone());
                        ctx.git(
                            "Restore",
                            move |l| l.git(&["checkout", &o, "--", &p]).map(drop),
                            |_| {},
                        )
                    })
                }));
            }
            items.push(("Restore Version Before This Commit".into(), {
                let (ctx, p, o) = (ctx.clone(), path.clone(), oid.clone());
                Rc::new(move || {
                    let (p, o) = (p.clone(), o.clone());
                    ctx.git(
                        "Restore",
                        move |l| l.git(&["checkout", &format!("{o}^"), "--", &p]).map(drop),
                        |_| {},
                    )
                })
            }));
        }
        items.push((String::new(), Rc::new(|| {})));
        if !dir {
            items.push(("File History".into(), {
                let (ctx, p) = (ctx.clone(), path.clone());
                Rc::new(move || {
                    let (c, p) = (ctx.clone(), p.clone());
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
                })
            }));
            items.push(("Blame".into(), {
                let (ctx, p) = (ctx.clone(), path.clone());
                let rev = self.sel_oid().unwrap_or_else(|| "HEAD".into());
                Rc::new(move || {
                    let (c, p, rev) = (ctx.clone(), p.clone(), rev.clone());
                    ctx.load(
                        move |l| git_backend::blame(l, &rev, &p),
                        move |r| match r {
                            Ok(t) => util::text_dialog(&c.parent(), "Blame", &t),
                            Err(e) => util::show_error(&c.parent(), "Cannot Blame", &e),
                        },
                    );
                })
            }));
        }
        items.push(("Copy Path".into(), {
            let (ctx, p) = (ctx.clone(), path.clone());
            Rc::new(move || util::copy_text(&ctx.parent(), &p))
        }));
        if let git_backend::Location::Local(root) = &ctx.loc {
            let abs = root.join(path.trim_end_matches('/'));
            items.push(("Show in Files".into(), {
                let ctx = ctx.clone();
                Rc::new(move || {
                    let win = ctx.parent().root().and_downcast::<gtk::Window>();
                    gtk::FileLauncher::new(Some(&gio::File::for_path(&abs)))
                        .open_containing_folder(win.as_ref(), gio::Cancellable::NONE, |_| {});
                })
            }));
        }
        if ctx.slug.borrow().is_some() && !dir {
            let rev = self.sel_oid();
            items.push(("Open on GitHub".into(), {
                let (ctx, p) = (ctx.clone(), path.clone());
                Rc::new(move || {
                    if let Some(slug) = ctx.slug.borrow().as_ref() {
                        let r = rev.clone().unwrap_or_else(|| ctx.branch());
                        util::open_url(
                            &ctx.parent(),
                            &format!("https://github.com/{slug}/blob/{r}/{p}"),
                        );
                    }
                })
            }));
        }
        items
    }
}

fn rewrite(
    ctx: Rc<Ctx>,
    oid: String,
    how: Rewrite,
    what: &'static str,
    unpushed: bool,
    confirm: bool,
) {
    glib::spawn_future_local(async move {
        if !unpushed
            && !util::confirm(
                &ctx.parent(),
                "Rewrite Pushed History?",
                "This commit is already on a remote. Rewriting it means you will need to force push, which can disrupt others working on this branch.",
                what,
                true,
            )
            .await
        {
            return;
        }
        if confirm
            && unpushed
            && !util::confirm(
                &ctx.parent(),
                &format!("{what}?"),
                "The commit and its changes will be removed from the branch.",
                what,
                true,
            )
            .await
        {
            return;
        }
        let c = ctx.clone();
        ctx.git(
            what,
            move |l| git_backend::rewrite(l, &oid, how),
            move |_| c.toast("History rewritten"),
        );
    });
}

fn reword(ctx: Rc<Ctx>, oid: String) {
    let o = oid.clone();
    let c = ctx.clone();
    ctx.load(
        move |l| l.git(&["log", "-1", "--format=%B", &o]),
        move |r| {
            let msg = r.unwrap_or_default();
            glib::spawn_future_local(async move {
                let Some(v) = form::run(
                    &c.parent(),
                    "Edit Commit Message",
                    "",
                    "Save",
                    vec![Field::text("Message", msg.trim())],
                )
                .await
                else {
                    return;
                };
                let msg = v[0].text();
                if msg.trim().is_empty() {
                    return;
                }
                rewrite(c, oid, Rewrite::Reword(msg), "Edit Message", true, false);
            });
        },
    );
}

fn checkout(ctx: Rc<Ctx>, oid: String) {
    ctx.git(
        "Checkout",
        move |l| git_backend::checkout_detached(l, &oid),
        |_| {},
    );
}

fn cherry_pick(ctx: Rc<Ctx>, oid: String) {
    let c = ctx.clone();
    ctx.git(
        "Cherry-pick",
        move |l| git_backend::cherry_pick(l, &oid),
        move |_| c.toast("Cherry-picked"),
    );
}

fn revert(ctx: Rc<Ctx>, oid: String) {
    let c = ctx.clone();
    ctx.git(
        "Revert",
        move |l| git_backend::revert(l, &oid),
        move |_| c.toast("Reverted"),
    );
}

fn open_on_github(ctx: Rc<Ctx>, oid: String) {
    if let Some(slug) = ctx.slug.borrow().as_ref() {
        util::open_url(
            &ctx.parent(),
            &format!("https://github.com/{slug}/commit/{oid}"),
        );
    }
}

fn tag(ctx: Rc<Ctx>, oid: String) {
    glib::spawn_future_local(async move {
        let Some(v) = form::run(
            &ctx.parent(),
            "New Tag",
            &format!("Tag commit {}", &oid[..oid.len().min(10)]),
            "Create",
            vec![
                Field::required("Tag name", ""),
                Field::entry("Message (makes an annotated tag)", ""),
            ],
        )
        .await
        else {
            return;
        };
        let (name, msg) = (v[0].text(), v[1].text());
        ctx.git(
            "Create Tag",
            move |l| git_backend::create_tag(l, &name, &oid, &msg),
            |_| {},
        );
    });
}

fn reset(ctx: Rc<Ctx>, oid: String) {
    glib::spawn_future_local(async move {
        let Some(v) = form::run(
            &ctx.parent(),
            "Reset Current Branch",
            "Move the current branch to this commit.",
            "Reset",
            vec![Field::choice(
                "Mode",
                &[
                    "Soft — keep changes staged",
                    "Mixed — keep changes in the working tree",
                    "Hard — discard all changes",
                ],
                1,
            )],
        )
        .await
        else {
            return;
        };
        let mode = [ResetMode::Soft, ResetMode::Mixed, ResetMode::Hard][v[0].index().min(2)];
        if mode == ResetMode::Hard
            && !util::confirm(
                &ctx.parent(),
                "Discard Changes?",
                "A hard reset permanently discards uncommitted changes.",
                "Hard Reset",
                true,
            )
            .await
        {
            return;
        }
        ctx.git("Reset", move |l| git_backend::reset(l, &oid, mode), |_| {});
    });
}

impl View for Code {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn refresh(self: Rc<Self>) {
        let changes = self.changes_now();
        // Forget exclusions for files that are no longer changed.
        self.tree
            .excluded
            .borrow_mut()
            .retain(|p| changes.iter().any(|f| f.path == *p));
        *self.changes.borrow_mut() = changes;
        self.update_composer();
        self.load_timeline(0);
        self.load_selection();
        let _ = gdk::Key::Return;
    }
}
