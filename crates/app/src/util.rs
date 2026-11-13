//! Small UI helpers shared by every view.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

/// Run `job` on a worker thread, then `done` on the main thread.
pub fn spawn<T: Send + 'static>(
    job: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
    done: impl FnOnce(anyhow::Result<T>) + 'static,
) {
    glib::spawn_future_local(async move {
        let r = gio::spawn_blocking(job)
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("background task panicked")));
        done(r);
    });
}

pub fn show_error(parent: &impl IsA<gtk::Widget>, heading: &str, err: &anyhow::Error) {
    let body = format!("{err:#}");
    let dialog = adw::AlertDialog::new(Some(heading), None);
    let label = gtk::Label::builder()
        .label(&body)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .xalign(0.0)
        .css_classes(["mono"])
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .child(&label)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(320)
        .build();
    dialog.set_extra_child(Some(&scroller));
    dialog.add_response("close", "Close");
    dialog.set_prefer_wide_layout(body.len() > 200);
    dialog.present(Some(parent));
}

/// Ask a yes/no question; resolves to `true` when confirmed.
pub async fn confirm(
    parent: &gtk::Widget,
    heading: &str,
    body: &str,
    action: &str,
    destructive: bool,
) -> bool {
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.add_responses(&[("cancel", "Cancel"), ("ok", action)]);
    dialog.set_response_appearance(
        "ok",
        if destructive {
            adw::ResponseAppearance::Destructive
        } else {
            adw::ResponseAppearance::Suggested
        },
    );
    dialog.set_default_response(Some(if destructive { "cancel" } else { "ok" }));
    dialog.set_close_response("cancel");
    dialog.choose_future(Some(parent)).await == "ok"
}

pub fn relative_time(unix: i64) -> String {
    let now = glib::DateTime::now_utc()
        .map(|d| d.to_unix())
        .unwrap_or(unix);
    let d = (now - unix).max(0);
    let (n, unit) = match d {
        0..60 => return "just now".into(),
        60..3600 => (d / 60, "minute"),
        3600..86400 => (d / 3600, "hour"),
        86400..2_592_000 => (d / 86400, "day"),
        2_592_000..31_536_000 => (d / 2_592_000, "month"),
        _ => (d / 31_536_000, "year"),
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

/// Relative time for an ISO-8601 timestamp from the GitHub API.
pub fn iso_relative(s: &str) -> String {
    glib::DateTime::from_iso8601(s, Some(&glib::TimeZone::utc()))
        .map(|d| relative_time(d.to_unix()))
        .unwrap_or_else(|_| s.to_owned())
}

pub fn human_size(bytes: u64) -> String {
    glib::format_size(bytes).to_string()
}

/// Small rounded label, e.g. for refs, states and labels.
pub fn pill(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::builder()
        .label(text)
        .valign(gtk::Align::Center)
        .css_classes(["pill-label"])
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(28)
        .build();
    if !class.is_empty() {
        l.add_css_class(class);
    }
    l
}

/// Icon summarising a CI / deployment outcome.
pub fn outcome_icon(outcome: &str) -> gtk::Image {
    let (icon, class) = match outcome.to_lowercase().as_str() {
        "success" | "completed" | "active" => ("object-select-symbolic", "success"),
        "failure" | "error" | "timed_out" | "startup_failure" | "action_required" => {
            ("dialog-error-symbolic", "error")
        }
        "cancelled" | "skipped" | "neutral" | "stale" | "inactive" | "" => {
            ("action-unavailable-symbolic", "dim-label")
        }
        _ => ("content-loading-symbolic", "warning"),
    };
    let img = gtk::Image::from_icon_name(icon);
    img.add_css_class(class);
    img.set_tooltip_text(Some(outcome));
    img
}

pub fn state_pill(state: &str) -> gtk::Label {
    let class = match state.to_uppercase().as_str() {
        "OPEN" => "success",
        "MERGED" => "merged",
        "CLOSED" => "error",
        "DRAFT" => "",
        _ => "accent",
    };
    let mut text = state.to_lowercase();
    if let Some(f) = text.get_mut(0..1) {
        f.make_ascii_uppercase();
    }
    pill(&text, class)
}

pub fn open_url(parent: &impl IsA<gtk::Widget>, url: &str) {
    if url.is_empty() {
        return;
    }
    let window = parent.root().and_downcast::<gtk::Window>();
    gtk::UriLauncher::new(url).launch(window.as_ref(), gio::Cancellable::NONE, |_| {});
}

pub fn copy_text(widget: &impl IsA<gtk::Widget>, text: &str) {
    widget.clipboard().set_text(text);
}

pub fn status_page(icon: &str, title: &str, description: &str) -> adw::StatusPage {
    adw::StatusPage::builder()
        .icon_name(icon)
        .title(title)
        .description(description)
        .vexpand(true)
        .build()
}

pub fn spinner_page() -> gtk::Widget {
    let s = adw::Spinner::builder()
        .width_request(32)
        .height_request(32)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .vexpand(true)
        .build();
    s.upcast()
}

/// Markdown body text (PRs, issues, releases); a dim note when empty.
pub fn body_label(text: &str) -> gtk::Widget {
    if !text.trim().is_empty() {
        return crate::markdown::render(text);
    }
    plain_label(text).upcast()
}

/// Selectable, wrapping plain text.
pub fn plain_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(if text.trim().is_empty() {
            "No description provided."
        } else {
            text
        })
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .xalign(0.0)
        .css_classes(if text.trim().is_empty() {
            vec!["dim-label"]
        } else {
            vec![]
        })
        .build()
}

pub fn heading(text: &str, class: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .wrap(true)
        .xalign(0.0)
        .selectable(true)
        .css_classes([class])
        .build()
}

pub fn boxed_list() -> gtk::ListBox {
    gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .valign(gtk::Align::Start)
        .build()
}

/// Scrollable, width-clamped page body.
pub fn page_scroller(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let child = child.as_ref();
    child.set_margin_top(24);
    child.set_margin_bottom(24);
    child.set_margin_start(36);
    child.set_margin_end(36);
    gtk::ScrolledWindow::builder()
        .child(child)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build()
}

pub fn vbox(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, spacing)
}

pub fn hbox(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Horizontal, spacing)
}

pub fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build()
}

pub fn text_button(label: &str, class: &str) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    if !class.is_empty() {
        b.add_css_class(class);
    }
    b.set_valign(gtk::Align::Center);
    b
}

pub type Callback = Rc<dyn Fn()>;

/// A "⋮" menu button whose items run the given callbacks.
pub fn menu_button(items: Vec<(&str, Callback)>) -> gtk::MenuButton {
    let b = labeled_menu("", "flat", items);
    b.set_icon_name("view-more-symbolic");
    b.set_tooltip_text(Some("More Actions"));
    b
}

/// A menu button with a text label whose items run the given callbacks.
pub fn labeled_menu(label: &str, class: &str, items: Vec<(&str, Callback)>) -> gtk::MenuButton {
    let group = gio::SimpleActionGroup::new();
    let menu = gio::Menu::new();
    for (i, (label, f)) in items.into_iter().enumerate() {
        let name = format!("a{i}");
        let action = gio::SimpleAction::new(&name, None);
        action.connect_activate(move |_, _| f());
        group.add_action(&action);
        menu.append(Some(label), Some(&format!("m.{name}")));
    }
    let b = gtk::MenuButton::builder()
        .label(label)
        .menu_model(&menu)
        .valign(gtk::Align::Center)
        .build();
    if !class.is_empty() {
        b.add_css_class(class);
    }
    b.insert_action_group("m", Some(&group));
    b
}

/// Show long read-only text (logs, blame, …) in a dialog.
pub fn text_dialog(parent: &impl IsA<gtk::Widget>, title: &str, text: &str) {
    let buffer = gtk::TextBuffer::new(None);
    buffer.set_text(text);
    let view = gtk::TextView::builder()
        .buffer(&buffer)
        .editable(false)
        .monospace(true)
        .top_margin(12)
        .bottom_margin(12)
        .left_margin(12)
        .right_margin(12)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .child(&view)
        .vexpand(true)
        .build();
    content_dialog(parent, title, &scroller, 960, 680);
}

/// A resizable dialog with a header bar wrapping `content`.
pub fn content_dialog(
    parent: &impl IsA<gtk::Widget>,
    title: &str,
    content: &impl IsA<gtk::Widget>,
    width: i32,
    height: i32,
) -> adw::Dialog {
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::new());
    tv.set_content(Some(content));
    let dialog = adw::Dialog::builder()
        .title(title)
        .child(&tv)
        .content_width(width)
        .content_height(height)
        .build();
    dialog.present(Some(parent));
    dialog
}

/// A card with an author line and body text (comments, reviews).
pub fn comment_card(author: &str, when: &str, extra: Option<gtk::Label>, body: &str) -> gtk::Box {
    let card = vbox(6);
    card.add_css_class("card");
    card.add_css_class("comment-card");
    let head = hbox(6);
    let who = gtk::Label::builder()
        .label(author)
        .css_classes(["heading"])
        .build();
    head.append(&who);
    if let Some(e) = extra {
        head.append(&e);
    }
    let time = gtk::Label::builder()
        .label(when)
        .hexpand(true)
        .xalign(1.0)
        .css_classes(["dim-label", "caption"])
        .build();
    head.append(&time);
    card.append(&head);
    if !body.trim().is_empty() {
        card.append(&body_label(body));
    }
    card
}

pub fn clear_box(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

/// A titled boxed list with an optional header suffix (buttons), for list pages.
pub fn group(title: &str, suffix: Option<&gtk::Widget>) -> (gtk::Box, gtk::ListBox) {
    let b = vbox(8);
    let head = hbox(6);
    head.append(
        &gtk::Label::builder()
            .label(title)
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["heading"])
            .build(),
    );
    if let Some(s) = suffix {
        head.append(s);
    }
    b.append(&head);
    let list = boxed_list();
    b.append(&list);
    (b, list)
}

pub fn placeholder(list: &gtk::ListBox, text: &str) {
    list.set_placeholder(Some(
        &gtk::Label::builder()
            .label(text)
            .wrap(true)
            .margin_top(18)
            .margin_bottom(18)
            .margin_start(12)
            .margin_end(12)
            .css_classes(["dim-label"])
            .build(),
    ));
}

/// An `AdwActionRow` with escaped title and subtitle.
pub fn row(title: &str, subtitle: &str) -> adw::ActionRow {
    adw::ActionRow::builder()
        .title(glib::markup_escape_text(title))
        .subtitle(glib::markup_escape_text(subtitle))
        .subtitle_lines(2)
        .build()
}

/// Filter rows of `list` by their widget name as the user types in `entry`.
pub fn filter_by_name(entry: &gtk::SearchEntry, lists: &[gtk::ListBox]) {
    let lists = lists.to_vec();
    entry.connect_search_changed(move |e| {
        let q = e.text().to_lowercase();
        for l in &lists {
            let mut c = l.first_child();
            while let Some(w) = c {
                if w.is::<gtk::ListBoxRow>() {
                    w.set_visible(w.widget_name().to_lowercase().contains(&q));
                }
                c = w.next_sibling();
            }
        }
    });
}

/// Remove all rows from `list`, keeping its placeholder.
pub fn clear_rows(list: &gtk::ListBox) {
    let mut c = list.first_child();
    while let Some(w) = c {
        c = w.next_sibling();
        if let Ok(row) = w.downcast::<gtk::ListBoxRow>() {
            list.remove(&row);
        }
    }
}

/// Pop up a context menu at (x, y) on `parent`. An empty label starts a new section.
///
/// Built from plain menu-styled buttons rather than `GtkPopoverMenu`, whose built-in scroller
/// could be sized a few pixels short and scroll even a four-item menu. Long menus scroll.
pub fn popup_menu(parent: &impl IsA<gtk::Widget>, x: f64, y: f64, items: Vec<(String, Callback)>) {
    const SCROLL_AFTER: usize = 14;
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let popover = gtk::Popover::builder()
        .has_arrow(false)
        .halign(gtk::Align::Start)
        .css_classes(["menu", "context-menu"])
        .build();
    let mut count = 0;
    let mut pending_separator = false;
    for (label, f) in items {
        if label.is_empty() {
            pending_separator = count > 0;
            continue;
        }
        if pending_separator {
            list.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            pending_separator = false;
        }
        let button = gtk::Button::builder()
            .child(&gtk::Label::builder().label(&label).xalign(0.0).build())
            .css_classes(["flat", "context-menu-item"])
            .build();
        let p = popover.downgrade();
        button.connect_clicked(move |_| {
            if let Some(p) = p.upgrade() {
                p.popdown();
            }
            // Run after the menu has closed, so dialogs it opens get focus.
            let f = f.clone();
            glib::idle_add_local_once(move || f());
        });
        list.append(&button);
        count += 1;
    }
    if count == 0 {
        return;
    }
    if count > SCROLL_AFTER {
        let sw = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(480)
            .build();
        popover.set_child(Some(&sw));
    } else {
        popover.set_child(Some(&list));
    }
    popover.set_parent(parent);
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.connect_closed(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    popover.popup();
}

/// Attach a right-click menu built on demand by `items`.
pub fn context_menu(
    widget: &impl IsA<gtk::Widget>,
    items: impl Fn() -> Vec<(String, Callback)> + 'static,
) {
    let gesture = gtk::GestureClick::builder().button(3).build();
    let w = widget.as_ref().downgrade();
    gesture.connect_pressed(move |g, _, x, y| {
        g.set_state(gtk::EventSequenceState::Claimed);
        if let Some(w) = w.upgrade() {
            popup_menu(&w, x, y, items());
        }
    });
    widget.add_controller(gesture);
}
