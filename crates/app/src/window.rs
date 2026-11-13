//! Main window: a tab per repository plus a welcome screen.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use git_backend::Location;
use gtk::{gio, glib};

use crate::repo::RepoPage;
use crate::{open, settings, util};

type WinAction = Box<dyn Fn(&Rc<Window>)>;

thread_local! {
    static WINDOWS: RefCell<Vec<Rc<Window>>> = const { RefCell::new(Vec::new()) };
    /// Every open tab in every window. Tabs can be dragged between windows, so the mapping
    /// from tab to repository lives here rather than in one window.
    static PAGES: RefCell<Vec<(adw::TabPage, Rc<RepoPage>)>> = const { RefCell::new(Vec::new()) };
    /// Quitting the app: every window's tabs were already saved; closing them must not re-save.
    static QUITTING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Set by `--new-window` on the first launch: open an empty window, don't restore tabs.
    static START_EMPTY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn page_for(tp: &adw::TabPage) -> Option<Rc<RepoPage>> {
    PAGES.with(|p| {
        p.borrow()
            .iter()
            .find(|(t, _)| t == tp)
            .map(|(_, r)| r.clone())
    })
}

pub fn set_start_empty() {
    START_EMPTY.with(|s| s.set(true));
}

/// Locations of every tab, window by window, for the next launch.
fn save_session() {
    let windows: Vec<Vec<Location>> = WINDOWS.with(|ws| {
        ws.borrow()
            .iter()
            .map(|w| w.locations())
            .filter(|l| !l.is_empty())
            .collect()
    });
    settings::update(|s| {
        s.open_tabs = windows.first().cloned().unwrap_or_default();
        s.open_windows = windows;
    });
}

/// Quit: remember every window's tabs, then close them all.
pub fn quit_all() {
    save_session();
    QUITTING.with(|q| q.set(true));
    let windows: Vec<adw::ApplicationWindow> =
        WINDOWS.with(|ws| ws.borrow().iter().map(|w| w.win.clone()).collect());
    for w in windows {
        w.close();
    }
}

/// App activation: bring an existing window forward, or restore the last session.
pub fn activate(app: &adw::Application) {
    if let Some(w) = WINDOWS.with(|ws| ws.borrow().first().cloned()) {
        w.win.present();
        return;
    }
    if START_EMPTY.with(|s| s.replace(false)) {
        Window::build(app).win.present();
        return;
    }
    let s = settings::get();
    let mut sessions = s.open_windows.clone();
    if sessions.is_empty() && !s.open_tabs.is_empty() {
        sessions.push(s.open_tabs.clone());
    }
    if !s.restore_tabs || sessions.is_empty() {
        Window::build(app).win.present();
        return;
    }
    for locs in sessions {
        let w = Window::build(app);
        for loc in locs {
            w.add_tab(loc);
        }
        w.win.present();
    }
}

/// Open repositories given on the command line, in the existing window if there is one.
pub fn open_paths(app: &adw::Application, paths: &[PathBuf]) {
    let w = WINDOWS
        .with(|ws| ws.borrow().first().cloned())
        .unwrap_or_else(|| Window::build(app));
    for p in paths {
        w.open_location(Location::Local(p.clone()));
    }
    w.win.present();
}

/// A new, empty window (welcome screen).
pub fn new_window(app: &adw::Application) {
    Window::build(app).win.present();
}

pub struct Window {
    win: adw::ApplicationWindow,
    tab_view: adw::TabView,
    stack: gtk::Stack,
    /// The tab whose context menu is open (for the tab menu's actions).
    menu_page: RefCell<Option<adw::TabPage>>,
    /// True while one of our tabs is being closed (as opposed to dragged away).
    closing: std::cell::Cell<bool>,
    start_bin: adw::Bin,
    title_bin: adw::Bin,
    end_bin: adw::Bin,
    welcome_title: adw::WindowTitle,
    recent_list: gtk::ListBox,
    recent_group: gtk::Box,
    account_row: adw::ActionRow,
    /// New-tab button: a popover with recent repositories and ways to add one.
    open_button: gtk::MenuButton,
    picker_list: gtk::ListBox,
    picker_search: gtk::SearchEntry,
}

impl Window {
    fn build(app: &adw::Application) -> Rc<Window> {
        let s = settings::get();
        let win = adw::ApplicationWindow::builder()
            .application(app)
            .title("GittyHub")
            .default_width(initial_size(&s).0)
            .default_height(initial_size(&s).1)
            .maximized(s.maximized)
            .width_request(360)
            .height_request(400)
            .build();

        let tab_view = adw::TabView::new();
        let tab_menu = gio::Menu::new();
        tab_menu.append(Some("Move to New Window"), Some("win.tab-to-new-window"));
        let close_section = gio::Menu::new();
        close_section.append(Some("Close Tab"), Some("win.tab-close"));
        tab_menu.append_section(None, &close_section);
        tab_view.set_menu_model(Some(&tab_menu));
        let tab_bar = adw::TabBar::builder()
            .view(&tab_view)
            .autohide(true)
            .build();

        // Header bar
        let (open_button, picker_list, picker_search) = repo_picker();
        let main_menu = gio::Menu::new();
        let window_section = gio::Menu::new();
        window_section.append(Some("New Window"), Some("app.new-window"));
        main_menu.append_section(None, &window_section);
        let section = gio::Menu::new();
        section.append(Some("Preferences"), Some("app.preferences"));
        section.append(Some("Keyboard Shortcuts"), Some("win.show-help-overlay"));
        section.append(Some("About GittyHub"), Some("app.about"));
        main_menu.append_section(None, &section);
        let quit_section = gio::Menu::new();
        quit_section.append(Some("Quit"), Some("app.quit"));
        main_menu.append_section(None, &quit_section);
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&main_menu)
            .primary(true)
            .tooltip_text("Main Menu")
            .build();

        let start_bin = adw::Bin::new();
        let title_bin = adw::Bin::new();
        let end_bin = adw::Bin::new();
        let header = adw::HeaderBar::new();
        header.pack_start(&open_button);
        header.pack_start(&start_bin);
        header.pack_end(&menu_button);
        header.pack_end(&end_bin);
        header.set_title_widget(Some(&title_bin));
        let welcome_title = adw::WindowTitle::new("GittyHub", "");

        // Welcome screen
        let (welcome, recent_list, recent_group, account_row) = welcome_page();

        let tabs_box = util::vbox(0);
        tabs_box.append(&tab_bar);
        tab_view.set_vexpand(true);
        tabs_box.append(&tab_view);
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        stack.add_named(&welcome, Some("welcome"));
        stack.add_named(&tabs_box, Some("tabs"));

        let tv = adw::ToolbarView::new();
        tv.add_top_bar(&header);
        tv.set_content(Some(&stack));
        win.set_content(Some(&tv));

        let this = Rc::new(Window {
            win: win.clone(),
            tab_view,
            stack,
            menu_page: RefCell::new(None),
            closing: std::cell::Cell::new(false),
            start_bin,
            title_bin,
            end_bin,
            welcome_title,
            recent_list,
            recent_group,
            account_row,
            open_button,
            picker_list,
            picker_search,
        });
        WINDOWS.with(|w| w.borrow_mut().push(this.clone()));
        this.setup_actions();
        this.setup_signals();
        this.refresh_welcome();
        this.update_chrome();
        this
    }

    /// This window's tab locations, in tab order.
    fn locations(&self) -> Vec<Location> {
        (0..self.tab_view.n_pages())
            .filter_map(|i| page_for(&self.tab_view.nth_page(i)))
            .map(|p| p.ctx.loc.clone())
            .collect()
    }

    fn setup_actions(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let add = |name: &str, f: WinAction| {
            let a = gio::SimpleAction::new(name, None);
            let weak = weak.clone();
            a.connect_activate(move |_, _| {
                if let Some(w) = weak.upgrade() {
                    f(&w);
                }
            });
            self.win.add_action(&a);
        };
        add("open-local", Box::new(|w| w.run_open(open::Kind::Local)));
        add("new-tab", Box::new(|w| w.open_button.popup()));
        add("open-remote", Box::new(|w| w.run_open(open::Kind::Remote)));
        add("clone", Box::new(|w| w.run_open(open::Kind::Clone)));
        add("new-repo", Box::new(|w| w.run_open(open::Kind::New)));
        add(
            "close-tab",
            Box::new(|w| {
                if let Some(p) = w.tab_view.selected_page() {
                    w.tab_view.close_page(&p);
                }
            }),
        );
        add(
            "refresh",
            Box::new(|w| {
                if let Some(p) = w.current() {
                    p.refresh();
                }
            }),
        );
        add("fetch", Box::new(|w| w.with_current(|p| p.fetch())));
        add("pull", Box::new(|w| w.with_current(|p| p.pull())));
        add("push", Box::new(|w| w.with_current(|p| p.push(false))));
        add(
            "force-push",
            Box::new(|w| w.with_current(|p| p.force_push())),
        );
        add("show-help-overlay", Box::new(|w| shortcuts_dialog(&w.win)));
        add(
            "tab-to-new-window",
            Box::new(|w| {
                let page = w
                    .menu_page
                    .borrow_mut()
                    .take()
                    .or_else(|| w.tab_view.selected_page());
                if let (Some(page), Some(app)) = (page, w.win.application()) {
                    let app = app.downcast::<adw::Application>().expect("adw app");
                    let other = Window::build(&app);
                    w.tab_view.transfer_page(&page, &other.tab_view, 0);
                    other.win.present();
                }
            }),
        );
        add(
            "tab-close",
            Box::new(|w| {
                let page = w
                    .menu_page
                    .borrow_mut()
                    .take()
                    .or_else(|| w.tab_view.selected_page());
                if let Some(page) = page {
                    w.tab_view.close_page(&page);
                }
            }),
        );
    }

    fn setup_signals(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.tab_view.connect_selected_page_notify(glib::clone!(
            #[strong]
            weak,
            move |_| {
                if let Some(w) = weak.upgrade() {
                    w.update_chrome();
                }
            }
        ));
        self.tab_view.connect_close_page(glib::clone!(
            #[strong]
            weak,
            move |tv, page| {
                PAGES.with(|p| p.borrow_mut().retain(|(t, _)| t != page));
                if let Some(w) = weak.upgrade() {
                    w.closing.set(true);
                    tv.close_page_finish(page, true);
                    w.closing.set(false);
                    save_session();
                    w.update_chrome();
                } else {
                    tv.close_page_finish(page, true);
                }
                glib::Propagation::Stop
            }
        ));
        self.tab_view
            .connect_page_reordered(|_, _, _| save_session());
        // Tabs dragged in from another window.
        self.tab_view.connect_page_attached(glib::clone!(
            #[strong]
            weak,
            move |_, _, _| {
                if let Some(w) = weak.upgrade() {
                    w.update_chrome();
                }
                save_session();
            }
        ));
        // Tabs dragged out: a window left with no tabs that way closes, like other GNOME apps.
        self.tab_view.connect_page_detached(glib::clone!(
            #[strong]
            weak,
            move |tv, _, _| {
                let Some(w) = weak.upgrade() else { return };
                if !w.closing.get() && tv.n_pages() == 0 {
                    let win = w.win.clone();
                    glib::idle_add_local_once(move || win.close());
                } else {
                    w.update_chrome();
                }
            }
        ));
        // Dropping a tab outside any window opens it in a new one.
        self.tab_view.connect_create_window(glib::clone!(
            #[strong]
            weak,
            move |_| {
                let w = weak.upgrade()?;
                let app = w.win.application()?.downcast::<adw::Application>().ok()?;
                let other = Window::build(&app);
                other.win.present();
                Some(other.tab_view.clone())
            }
        ));
        self.tab_view.connect_setup_menu(glib::clone!(
            #[strong]
            weak,
            move |_, page| {
                if let Some(w) = weak.upgrade() {
                    *w.menu_page.borrow_mut() = page.cloned();
                }
            }
        ));
        // Re-read status when the user comes back to the window (files may have changed).
        // Re-read status when the user comes back to the window (files may have changed in
        // another app) — but not when focus only went to one of our own menus or popovers.
        let left_for_popover = std::rc::Rc::new(std::cell::Cell::new(false));
        self.win.connect_is_active_notify(glib::clone!(
            #[strong]
            weak,
            move |win| {
                if !win.is_active() {
                    left_for_popover.set(popover_open(win.upcast_ref()));
                    return;
                }
                // Coming back from (or into) one of our own popovers: nothing outside changed.
                if left_for_popover.replace(false) || popover_open(win.upcast_ref()) {
                    return;
                }
                if let Some(w) = weak.upgrade()
                    && let Some(p) = w.current()
                {
                    p.refresh();
                }
            }
        ));
        if let Some(popover) = self.open_button.popover() {
            popover.connect_show(glib::clone!(
                #[strong]
                weak,
                move |_| {
                    if let Some(w) = weak.upgrade() {
                        w.picker_search.set_text("");
                        w.fill_picker();
                        w.picker_search.grab_focus();
                    }
                }
            ));
        }
        self.picker_list.connect_row_activated(glib::clone!(
            #[strong]
            weak,
            move |_, row| {
                if let Some(w) = weak.upgrade() {
                    w.open_button.popdown();
                    let idx = row.index() as usize;
                    if let Some(loc) = settings::get().recent.get(idx).cloned() {
                        w.open_location(loc);
                    }
                }
            }
        ));
        self.picker_search.connect_activate(glib::clone!(
            #[strong]
            weak,
            move |_| {
                // Enter opens the first matching repository.
                if let Some(w) = weak.upgrade() {
                    let mut c = w.picker_list.first_child();
                    while let Some(x) = c {
                        if x.is_visible()
                            && let Ok(row) = x.clone().downcast::<gtk::ListBoxRow>()
                        {
                            row.activate();
                            return;
                        }
                        c = x.next_sibling();
                    }
                }
            }
        ));
        self.recent_list.connect_row_activated(glib::clone!(
            #[strong]
            weak,
            move |_, row| {
                if let Some(w) = weak.upgrade() {
                    let idx = row.index() as usize;
                    if let Some(loc) = settings::get().recent.get(idx).cloned() {
                        w.open_location(loc);
                    }
                }
            }
        ));
        self.win.connect_close_request(glib::clone!(
            #[strong]
            weak,
            move |win| {
                let (width, height) = win.default_size();
                settings::update(|s| {
                    s.maximized = win.is_maximized();
                    if !s.maximized {
                        s.window_width = width;
                        s.window_height = height;
                        s.window_size_saved = true;
                    }
                });
                if let Some(w) = weak.upgrade() {
                    let quitting = QUITTING.with(|q| q.get());
                    let last = WINDOWS.with(|ws| ws.borrow().len()) <= 1;
                    // Closing the last window is quitting: remember its tabs. Closing one of
                    // several just forgets that window's tabs.
                    if last && !quitting {
                        save_session();
                    }
                    for i in 0..w.tab_view.n_pages() {
                        let tp = w.tab_view.nth_page(i);
                        PAGES.with(|p| p.borrow_mut().retain(|(t, _)| *t != tp));
                    }
                    WINDOWS.with(|ws| ws.borrow_mut().retain(|x| !Rc::ptr_eq(x, &w)));
                    if !last && !quitting {
                        save_session();
                    }
                }
                glib::Propagation::Proceed
            }
        ));
    }

    fn current(&self) -> Option<Rc<RepoPage>> {
        page_for(&self.tab_view.selected_page()?)
    }

    fn with_current(&self, f: impl FnOnce(&Rc<RepoPage>)) {
        if let Some(p) = self.current() {
            f(&p);
        }
    }

    /// Swap header-bar widgets and the window title to match the selected tab.
    fn update_chrome(self: &Rc<Self>) {
        let current = self.current();
        let has_tabs = self.tab_view.n_pages() > 0;
        self.stack
            .set_visible_child_name(if has_tabs { "tabs" } else { "welcome" });
        match current {
            Some(p) => {
                // The page may have come from another window, whose header still holds them.
                for (bin, w) in [
                    (&self.start_bin, p.header_start.upcast_ref::<gtk::Widget>()),
                    (&self.title_bin, &p.header_title),
                    (&self.end_bin, p.header_end.upcast_ref()),
                ] {
                    if let Some(old) = w.parent().and_downcast::<adw::Bin>()
                        && &old != bin
                    {
                        old.set_child(gtk::Widget::NONE);
                    }
                    bin.set_child(Some(w));
                }
                self.win
                    .set_title(Some(&format!("{} — GittyHub", p.title())));
            }
            None => {
                self.start_bin.set_child(gtk::Widget::NONE);
                self.title_bin.set_child(Some(&self.welcome_title));
                self.end_bin.set_child(gtk::Widget::NONE);
                self.win.set_title(Some("GittyHub"));
                self.refresh_welcome();
            }
        }
        for name in [
            "fetch",
            "pull",
            "push",
            "force-push",
            "refresh",
            "close-tab",
        ] {
            if let Some(a) = self
                .win
                .lookup_action(name)
                .and_downcast::<gio::SimpleAction>()
            {
                a.set_enabled(has_tabs);
            }
        }
    }

    fn run_open(self: &Rc<Self>, kind: open::Kind) {
        let weak = Rc::downgrade(self);
        let parent = self.win.clone().upcast::<gtk::Widget>();
        open::run(
            &parent,
            kind,
            Rc::new(move |loc| {
                if let Some(w) = weak.upgrade() {
                    w.add_tab(loc);
                }
            }),
        );
    }

    /// Open `loc` (normalised to the work-tree root) in a tab, or focus it if already open.
    pub fn open_location(self: &Rc<Self>, loc: Location) {
        let weak = Rc::downgrade(self);
        let probe = loc.clone();
        util::spawn(
            move || git_backend::toplevel(&probe).map(|top| probe.with_path(&top)),
            move |r| {
                let Some(w) = weak.upgrade() else { return };
                match r {
                    Ok(l) => w.add_tab(l),
                    Err(e) => util::show_error(&w.win, "Not a Git Repository", &e),
                }
            },
        );
    }

    fn add_tab(self: &Rc<Self>, loc: Location) {
        // Already open (in any window): show that tab.
        let existing = PAGES.with(|p| {
            p.borrow()
                .iter()
                .find(|(_, r)| r.ctx.loc == loc)
                .map(|(t, _)| t.clone())
        });
        if let Some(tp) = existing {
            let owner = WINDOWS.with(|ws| {
                ws.borrow()
                    .iter()
                    .find(|w| (0..w.tab_view.n_pages()).any(|i| w.tab_view.nth_page(i) == tp))
                    .cloned()
            });
            if let Some(w) = owner {
                w.tab_view.set_selected_page(&tp);
                w.win.present();
            }
            return;
        }
        settings::add_recent(&loc);
        let page = RepoPage::new(loc.clone());
        let tp = self.tab_view.append(&page.widget());
        tp.set_title(&page.title());
        tp.set_tooltip(&glib::markup_escape_text(&loc.to_string()));
        if loc.is_remote() {
            tp.set_icon(Some(&gio::ThemedIcon::new("gh-server-symbolic")));
        }
        let page_weak = page.weak();
        let tp_weak = tp.downgrade();
        page.connect_title_changed(move || {
            if let (Some(p), Some(tp)) = (page_weak.upgrade(), tp_weak.upgrade()) {
                tp.set_tooltip(&glib::markup_escape_text(&p.subtitle()));
                // Whichever window the tab is in now.
                if tp.is_selected()
                    && let Some(win) = tp.child().root().and_downcast::<gtk::Window>()
                {
                    win.set_title(Some(&format!("{} — GittyHub", p.title())));
                }
            }
        });
        PAGES.with(|p| p.borrow_mut().push((tp.clone(), page)));
        self.tab_view.set_selected_page(&tp);
        save_session();
        self.update_chrome();
    }

    /// Fill the new-tab popover with recent repositories (open ones are marked).
    fn fill_picker(self: &Rc<Self>) {
        util::clear_rows(&self.picker_list);
        let open: Vec<Location> =
            PAGES.with(|p| p.borrow().iter().map(|(_, r)| r.ctx.loc.clone()).collect());
        for (i, loc) in settings::get().recent.iter().enumerate() {
            let row_box = util::hbox(10);
            row_box.set_margin_top(4);
            row_box.set_margin_bottom(4);
            row_box.append(&gtk::Image::from_icon_name(if loc.is_remote() {
                "gh-server-symbolic"
            } else {
                "folder-symbolic"
            }));
            let texts = util::vbox(2);
            texts.set_hexpand(true);
            texts.append(
                &gtk::Label::builder()
                    .label(loc.name())
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .build(),
            );
            texts.append(
                &gtk::Label::builder()
                    .label(loc.to_string())
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::Start)
                    .css_classes(["caption", "dim-label"])
                    .build(),
            );
            row_box.append(&texts);
            if open.contains(loc) {
                row_box.append(&util::pill("Open", "accent"));
            }
            let remove = util::icon_button("window-close-symbolic", "Remove from Recent");
            let weak = Rc::downgrade(self);
            remove.connect_clicked(move |_| {
                settings::update(|s| {
                    if i < s.recent.len() {
                        s.recent.remove(i);
                    }
                });
                if let Some(w) = weak.upgrade() {
                    w.fill_picker();
                    w.refresh_welcome();
                }
            });
            row_box.append(&remove);
            let row = gtk::ListBoxRow::builder().child(&row_box).build();
            row.set_widget_name(&format!("{} {}", loc.name(), loc));
            self.picker_list.append(&row);
        }
    }

    fn refresh_welcome(self: &Rc<Self>) {
        let recent = settings::get().recent;
        self.recent_list.remove_all();
        self.recent_group.set_visible(!recent.is_empty());
        for (i, loc) in recent.iter().enumerate() {
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&loc.name()))
                .subtitle(glib::markup_escape_text(&loc.to_string()))
                .activatable(true)
                .build();
            row.add_prefix(&gtk::Image::from_icon_name(if loc.is_remote() {
                "gh-server-symbolic"
            } else {
                "folder-symbolic"
            }));
            let remove = util::icon_button("edit-delete-symbolic", "Remove from Recent");
            let weak = Rc::downgrade(self);
            remove.connect_clicked(move |_| {
                settings::update(|s| {
                    if i < s.recent.len() {
                        s.recent.remove(i);
                    }
                });
                if let Some(w) = weak.upgrade() {
                    w.refresh_welcome();
                }
            });
            row.add_suffix(&remove);
            self.recent_list.append(&row);
        }
        let row = self.account_row.clone();
        util::spawn(gh_backend::current_user, move |r| match r {
            Ok(u) => {
                row.set_title(&glib::markup_escape_text(&u.login));
                row.set_subtitle(&glib::markup_escape_text(
                    &u.name
                        .unwrap_or_else(|| "Signed in to GitHub with gh".into()),
                ));
            }
            Err(_) => {
                row.set_title("Not signed in to GitHub");
                row.set_subtitle("Run “gh auth login” in a terminal to enable GitHub features");
            }
        });
    }
}

/// The new-tab button and its popover: search, recent repositories, and ways to add one.
/// Whether any popover (menu, picker, context menu) inside `w` is open.
fn popover_open(w: &gtk::Widget) -> bool {
    if w.downcast_ref::<gtk::Popover>()
        .is_some_and(|p| p.is_visible())
    {
        return true;
    }
    let mut c = w.first_child();
    while let Some(x) = c {
        if popover_open(&x) {
            return true;
        }
        c = x.next_sibling();
    }
    false
}

/// The window size to open with: the saved one, or (first launch) a generous share of the
/// monitor so everything fits without resizing.
fn initial_size(s: &settings::Settings) -> (i32, i32) {
    if s.window_size_saved {
        return (s.window_width, s.window_height);
    }
    let monitor = gtk::gdk::Display::default()
        .and_then(|d| d.monitors().item(0))
        .and_downcast::<gtk::gdk::Monitor>();
    match monitor {
        Some(m) => {
            let g = m.geometry();
            (
                ((g.width() as f64) * 0.75).round().max(1000.0) as i32,
                ((g.height() as f64) * 0.85).round().max(720.0) as i32,
            )
        }
        None => (1440, 900),
    }
}

fn repo_picker() -> (gtk::MenuButton, gtk::ListBox, gtk::SearchEntry) {
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search recent repositories")
        .build();
    let list = gtk::ListBox::builder()
        .css_classes(["navigation-sidebar"])
        .build();
    util::placeholder(&list, "No recent repositories yet");
    util::filter_by_name(&search, std::slice::from_ref(&list));
    let content = util::vbox(6);
    content.set_width_request(420);
    content.append(&search);
    content.append(&util::heading("Recent", "heading"));
    content.append(
        &gtk::ScrolledWindow::builder()
            .child(&list)
            .min_content_height(60)
            .max_content_height(420)
            .propagate_natural_height(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build(),
    );
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let popover = gtk::Popover::builder().child(&content).build();
    for (label, icon, action) in [
        (
            "Open Local Repository…",
            "folder-open-symbolic",
            "win.open-local",
        ),
        ("Open over SSH…", "gh-server-symbolic", "win.open-remote"),
        ("Clone Repository…", "folder-download-symbolic", "win.clone"),
        ("New Repository…", "folder-new-symbolic", "win.new-repo"),
    ] {
        let b = gtk::Button::builder()
            .child(
                &adw::ButtonContent::builder()
                    .icon_name(icon)
                    .label(label)
                    .halign(gtk::Align::Start)
                    .build(),
            )
            .action_name(action)
            .css_classes(["flat"])
            .build();
        let p = popover.downgrade();
        b.connect_clicked(move |_| {
            if let Some(p) = p.upgrade() {
                p.popdown();
            }
        });
        content.append(&b);
    }
    let button = gtk::MenuButton::builder()
        .icon_name("tab-new-symbolic")
        .popover(&popover)
        .tooltip_text("Open Repository (Ctrl+T)")
        .build();
    (button, list, search)
}

fn welcome_page() -> (gtk::Widget, gtk::ListBox, gtk::Box, adw::ActionRow) {
    let content = util::vbox(24);
    // A plain header rather than AdwStatusPage: a status page scrolls internally, so inside
    // this scrolling page it got squeezed and clipped the text under the icon.
    let status = util::vbox(12);
    status.set_margin_top(24);
    let icon = gtk::Image::builder()
        .icon_name("gh-github-symbolic")
        .pixel_size(96)
        .css_classes(["dim-label"])
        .margin_bottom(12)
        .build();
    status.append(&icon);
    status.append(
        &gtk::Label::builder()
            .label("Welcome to GittyHub")
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["title-1"])
            .build(),
    );
    status.append(
        &gtk::Label::builder()
            .label("Open a repository on this computer, or on any machine you can reach over SSH.")
            .wrap(true)
            .justify(gtk::Justification::Center)
            .build(),
    );
    let buttons = adw::WrapBox::builder()
        .child_spacing(12)
        .line_spacing(12)
        .align(0.5)
        .justify(adw::JustifyMode::None)
        .build();
    for (label, action, class) in [
        ("Open Local", "win.open-local", "suggested-action"),
        ("Open over SSH", "win.open-remote", ""),
        ("Clone", "win.clone", ""),
        ("New Repository", "win.new-repo", ""),
    ] {
        let b = gtk::Button::builder()
            .label(label)
            .action_name(action)
            .css_classes(["pill"])
            .build();
        if !class.is_empty() {
            b.add_css_class(class);
        }
        buttons.append(&b);
    }
    content.append(&status);
    content.append(&buttons);

    let recent_group = util::vbox(6);
    recent_group.append(&util::heading("Recent Repositories", "heading"));
    let recent_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    recent_group.append(&recent_list);
    content.append(&recent_group);

    let account_group = util::vbox(6);
    account_group.append(&util::heading("GitHub Account", "heading"));
    let account_list = util::boxed_list();
    let account_row = adw::ActionRow::builder().title("Checking…").build();
    account_row.add_prefix(&gtk::Image::from_icon_name("avatar-default-symbolic"));
    account_list.append(&account_row);
    account_group.append(&account_list);
    content.append(&account_group);

    (
        util::page_scroller(&content).upcast(),
        recent_list,
        recent_group,
        account_row,
    )
}

fn shortcuts_dialog(win: &adw::ApplicationWindow) {
    let list = util::vbox(18);
    for (title, items) in [
        (
            "Repositories",
            &[
                ("Ctrl+T", "New tab: recent repositories"),
                ("Ctrl+N", "New window"),
                ("Ctrl+O", "Open local repository"),
                ("Ctrl+Shift+O", "Open repository over SSH"),
                ("Ctrl+Shift+N", "Clone repository"),
                ("Ctrl+W", "Close tab"),
                ("F5 / Ctrl+R", "Refresh"),
            ][..],
        ),
        (
            "Sync",
            &[
                ("Ctrl+Shift+F", "Fetch"),
                ("Ctrl+Shift+P", "Pull"),
                ("Ctrl+P", "Push"),
                ("Ctrl+Enter", "Commit (in Code)"),
                ("Ctrl+S", "Save the file being edited"),
            ][..],
        ),
        (
            "Application",
            &[
                ("Ctrl+= / Ctrl+−", "Zoom in / out"),
                ("Ctrl+0", "Reset zoom"),
                ("Ctrl+,", "Preferences"),
                ("Ctrl+Q", "Quit"),
            ][..],
        ),
    ] {
        let group = adw::PreferencesGroup::builder().title(title).build();
        for (keys, what) in items {
            let row = adw::ActionRow::builder().title(*what).build();
            row.add_suffix(
                &gtk::Label::builder()
                    .label(*keys)
                    .css_classes(["dim-label", "mono"])
                    .build(),
            );
            group.add(&row);
        }
        list.append(&group);
    }
    util::content_dialog(
        win,
        "Keyboard Shortcuts",
        &util::page_scroller(&list),
        480,
        600,
    );
}
