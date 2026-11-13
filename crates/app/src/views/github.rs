//! Shared scaffolding for GitHub screens: navigation pages, load states, conversations.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::repo::Ctx;
use crate::util;

/// A navigation page with a flat header bar (back button, title, end widgets).
pub fn nav_page(
    title: &str,
    content: &impl IsA<gtk::Widget>,
    end: &[&gtk::Widget],
) -> adw::NavigationPage {
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .css_classes(["flat"])
        .build();
    for w in end.iter().rev() {
        header.pack_end(*w);
    }
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&header);
    tv.set_content(Some(content));
    adw::NavigationPage::builder()
        .title(title)
        .child(&tv)
        .build()
}

/// Loading / error / not-on-GitHub states around a page's content.
pub struct Shell {
    pub stack: gtk::Stack,
    status: adw::StatusPage,
    loaded: Cell<bool>,
}

impl Shell {
    pub fn new(content: &impl IsA<gtk::Widget>) -> Shell {
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        let status = util::status_page("dialog-error-symbolic", "", "");
        stack.add_named(&util::spinner_page(), Some("loading"));
        stack.add_named(content, Some("content"));
        stack.add_named(&status, Some("status"));
        Shell {
            stack,
            status,
            loaded: Cell::new(false),
        }
    }

    /// Returns false (and shows an explanation) when the repo isn't on GitHub.
    pub fn begin(&self, ctx: &Ctx) -> bool {
        if ctx.slug.borrow().is_none() {
            self.show_status(
                "gh-github-symbolic",
                "Not a GitHub Repository",
                "This repository has no GitHub remote. Publish it or add one from Remotes & Settings.",
            );
            return false;
        }
        if !self.loaded.get() {
            self.stack.set_visible_child_name("loading");
        }
        true
    }

    pub fn ready(&self) {
        self.loaded.set(true);
        self.stack.set_visible_child_name("content");
    }

    pub fn error(&self, e: &anyhow::Error) {
        let msg = e.to_string();
        let hint = if msg.contains("auth login") || msg.contains("not logged") {
            "\n\nRun “gh auth login” in a terminal to sign in."
        } else {
            ""
        };
        self.show_status(
            "dialog-error-symbolic",
            "Could Not Load from GitHub",
            &format!("{msg}{hint}"),
        );
    }

    pub fn show_status(&self, icon: &str, title: &str, desc: &str) {
        self.status.set_icon_name(Some(icon));
        self.status.set_title(title);
        self.status
            .set_description(Some(&glib::markup_escape_text(desc)));
        self.stack.set_visible_child_name("status");
    }
}

/// A comment composer: text area plus a submit button.
pub fn composer(
    button_label: &str,
    on_submit: impl Fn(String) + 'static,
) -> (gtk::Box, gtk::TextBuffer) {
    let buffer = gtk::TextBuffer::new(None);
    let view = gtk::TextView::builder()
        .buffer(&buffer)
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .child(&view)
        .min_content_height(90)
        .max_content_height(240)
        .propagate_natural_height(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .css_classes(["text-area"])
        .overflow(gtk::Overflow::Hidden)
        .build();
    let button = util::text_button(button_label, "suggested-action");
    button.set_halign(gtk::Align::End);
    let b2 = buffer.clone();
    button.connect_clicked(move |_| {
        let text = b2.text(&b2.start_iter(), &b2.end_iter(), false).to_string();
        if !text.trim().is_empty() {
            on_submit(text);
        }
    });
    let b = util::vbox(8);
    b.append(&util::heading("Add a Comment", "heading"));
    b.append(&scroller);
    b.append(&button);
    (b, buffer)
}

pub fn labels_row(labels: &[gh_backend::Label]) -> Option<gtk::Box> {
    if labels.is_empty() {
        return None;
    }
    let b = util::hbox(6);
    for l in labels {
        b.append(&util::pill(&l.name, "accent"));
    }
    Some(b)
}

pub fn refresh_button(f: impl Fn() + 'static) -> gtk::Widget {
    let b = util::icon_button("view-refresh-symbolic", "Refresh");
    b.connect_clicked(move |_| f());
    b.upcast()
}

pub fn browser_button(ctx: &Rc<Ctx>, url: impl Fn() -> String + 'static) -> gtk::Widget {
    let b = util::icon_button("gh-external-symbolic", "Open in Browser");
    let ctx = ctx.clone();
    b.connect_clicked(move |_| util::open_url(&ctx.parent(), &url()));
    b.upcast()
}

/// A drop-down filter (e.g. Open / Closed / All).
pub fn dropdown(options: &[&str]) -> gtk::DropDown {
    gtk::DropDown::builder()
        .model(&gtk::StringList::new(options))
        .valign(gtk::Align::Center)
        .build()
}

/// Run `f` every `secs` seconds while `widget` is mapped and alive.
pub fn poll_while_mapped(
    widget: &impl IsA<gtk::Widget>,
    secs: u32,
    f: impl Fn() -> bool + 'static,
) {
    let weak = widget.downgrade();
    glib::timeout_add_seconds_local(secs, move || {
        let Some(w) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if w.is_mapped() && !f() {
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}

/// A list | detail split that collapses to one pane on narrow windows.
pub fn master_detail(icon: &str, empty_title: &str) -> adw::NavigationSplitView {
    let empty = util::status_page(icon, empty_title, "");
    let placeholder = adw::NavigationPage::builder()
        .title(empty_title)
        .child(&empty)
        .build();
    let split = adw::NavigationSplitView::builder()
        .content(&placeholder)
        .min_sidebar_width(360.0)
        .max_sidebar_width(620.0)
        .sidebar_width_fraction(0.38)
        .build();
    unsafe {
        split.set_data("placeholder", placeholder);
    }
    split
}

/// The widget to embed: the split view wrapped in a breakpoint bin that collapses it.
pub fn root_of(split: &adw::NavigationSplitView) -> gtk::Widget {
    if let Some(p) = split.parent() {
        return p;
    }
    let root = adw::BreakpointBin::builder()
        .width_request(360)
        .height_request(300)
        .child(split)
        .build();
    let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        860.0,
        adw::LengthUnit::Sp,
    ));
    bp.add_setter(split, "collapsed", Some(&true.to_value()));
    root.add_breakpoint(bp);
    root.upcast()
}

pub fn set_list(split: &adw::NavigationSplitView, page: &adw::NavigationPage) {
    split.set_sidebar(Some(page));
}

pub fn show_detail(split: &adw::NavigationSplitView, page: &adw::NavigationPage) {
    split.set_content(Some(page));
    split.set_show_content(true);
}

pub fn clear_detail(split: &adw::NavigationSplitView) {
    let placeholder = unsafe {
        split
            .data::<adw::NavigationPage>("placeholder")
            .map(|p| p.as_ref().clone())
    };
    split.set_content(placeholder.as_ref());
    split.set_show_content(false);
}
