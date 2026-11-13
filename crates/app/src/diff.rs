//! Unified-diff viewer with optional per-hunk stage / unstage / discard buttons.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use git_backend::HunkAction;
use gtk::{gdk, glib};

use crate::util;

const MAX_LINES: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkMode {
    /// Read-only.
    None,
    /// Work-tree changes: stage or discard a hunk.
    Unstaged,
    /// Index changes: unstage a hunk.
    Staged,
}

type HunkHandler = Rc<dyn Fn(String, HunkAction)>;

pub struct DiffView {
    root: gtk::Stack,
    content: gtk::Box,
    placeholder: adw::StatusPage,
    tags: gtk::TextTagTable,
    added: gtk::TextTag,
    removed: gtk::TextTag,
    on_hunk: RefCell<Option<HunkHandler>>,
}

/// Look up a theme colour by rendering a throwaway label with `class`.
fn theme_color(class: &str) -> gdk::RGBA {
    let l = gtk::Label::new(None);
    l.add_css_class(class);
    l.color()
}

fn tinted(mut c: gdk::RGBA, alpha: f32) -> gdk::RGBA {
    c.set_alpha(alpha);
    c
}

impl DiffView {
    pub fn new() -> Rc<Self> {
        let tags = gtk::TextTagTable::new();
        let added = gtk::TextTag::new(Some("added"));
        let removed = gtk::TextTag::new(Some("removed"));
        let meta = gtk::TextTag::builder()
            .name("meta")
            .style(gtk::pango::Style::Italic)
            .build();
        tags.add(&added);
        tags.add(&removed);
        tags.add(&meta);

        let content = util::vbox(12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);
        let scroller = gtk::ScrolledWindow::builder()
            .child(&content)
            .vexpand(true)
            .hexpand(true)
            .build();
        let placeholder = util::status_page("document-edit-symbolic", "No Changes Selected", "");
        let root = gtk::Stack::new();
        root.add_named(&placeholder, Some("empty"));
        root.add_named(&scroller, Some("diff"));

        let view = Rc::new(DiffView {
            root,
            content,
            placeholder,
            tags,
            added,
            removed,
            on_hunk: RefCell::new(None),
        });
        view.recolor();
        let weak = Rc::downgrade(&view);
        adw::StyleManager::default().connect_dark_notify(move |_| {
            if let Some(v) = weak.upgrade() {
                // Wait for the new stylesheet to apply before sampling colours.
                glib::idle_add_local_once(move || v.recolor());
            }
        });
        view
    }

    fn recolor(&self) {
        self.added
            .set_paragraph_background_rgba(Some(&tinted(theme_color("success"), 0.16)));
        self.removed
            .set_paragraph_background_rgba(Some(&tinted(theme_color("error"), 0.16)));
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    pub fn connect_hunk(&self, f: impl Fn(String, HunkAction) + 'static) {
        *self.on_hunk.borrow_mut() = Some(Rc::new(f));
    }

    pub fn set_placeholder(&self, icon: &str, title: &str, desc: &str) {
        self.placeholder.set_icon_name(Some(icon));
        self.placeholder.set_title(title);
        self.placeholder.set_description(Some(desc));
        self.root.set_visible_child_name("empty");
    }

    pub fn set_diff(&self, diff: &str, mode: HunkMode) {
        util::clear_box(&self.content);
        let files = git_backend::parse_diff(diff);
        if files.is_empty() {
            self.set_placeholder(
                "object-select-symbolic",
                "No Differences",
                "There is nothing to show for this selection.",
            );
            return;
        }
        let mut budget = MAX_LINES;
        for file in files {
            let card = util::vbox(0);
            card.add_css_class("card");
            let header = gtk::Label::builder()
                .label(&file.path)
                .xalign(0.0)
                .selectable(true)
                .css_classes(["diff-file-header", "mono"])
                .build();
            card.append(&header);
            if file.binary {
                card.append(&note("Binary file not shown."));
            } else if file.hunks.is_empty() {
                card.append(&note("No content changes (mode or rename only)."));
            }
            let file = Rc::new(file);
            for (i, hunk) in file.hunks.iter().enumerate() {
                if budget == 0 {
                    card.append(&note("Diff truncated: too large to display."));
                    break;
                }
                card.append(&self.hunk_header(&file, i, hunk, mode));
                card.append(&self.hunk_body(hunk, &mut budget));
            }
            self.content.append(&card);
        }
        self.root.set_visible_child_name("diff");
    }

    fn hunk_header(
        &self,
        file: &Rc<git_backend::DiffFile>,
        i: usize,
        hunk: &str,
        mode: HunkMode,
    ) -> gtk::Box {
        let bar = util::hbox(6);
        bar.add_css_class("diff-hunk-header");
        let first = hunk.lines().next().unwrap_or_default();
        bar.append(
            &gtk::Label::builder()
                .label(first)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["mono", "caption"])
                .build(),
        );
        let buttons: &[(&str, &str, HunkAction)] = match mode {
            HunkMode::None => &[],
            HunkMode::Unstaged => &[
                ("Discard Hunk", "", HunkAction::Discard),
                ("Stage Hunk", "", HunkAction::Stage),
            ],
            HunkMode::Staged => &[("Unstage Hunk", "", HunkAction::Unstage)],
        };
        for (label, _, action) in buttons {
            let b = gtk::Button::builder()
                .label(*label)
                .css_classes(["flat", "small"])
                .build();
            let handler = self.on_hunk.borrow().clone();
            let patch = file.hunk_patch(i);
            let action = *action;
            b.connect_clicked(move |_| {
                if let Some(h) = &handler {
                    h(patch.clone(), action);
                }
            });
            bar.append(&b);
        }
        bar
    }

    fn hunk_body(&self, hunk: &str, budget: &mut usize) -> gtk::TextView {
        let buffer = gtk::TextBuffer::new(Some(&self.tags));
        let body = hunk.split_once('\n').map(|(_, b)| b).unwrap_or("");
        for line in body.split_inclusive('\n') {
            if *budget == 0 {
                break;
            }
            *budget -= 1;
            let tag = match line.as_bytes().first() {
                Some(b'+') => Some("added"),
                Some(b'-') => Some("removed"),
                Some(b'\\') => Some("meta"),
                _ => None,
            };
            let mut end = buffer.end_iter();
            match tag {
                Some(t) => buffer.insert_with_tags_by_name(&mut end, line, &[t]),
                None => buffer.insert(&mut end, line),
            }
        }
        // Drop the trailing newline so the view has no empty last line.
        let mut end = buffer.end_iter();
        let mut before = end;
        if before.backward_char() && before.char() == '\n' {
            buffer.delete(&mut before, &mut end);
        }
        gtk::TextView::builder()
            .buffer(&buffer)
            .editable(false)
            .cursor_visible(false)
            .monospace(true)
            .top_margin(4)
            .bottom_margin(4)
            .left_margin(8)
            .right_margin(8)
            .css_classes(["diff-view"])
            .build()
    }
}

fn note(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(12)
        .css_classes(["dim-label"])
        .build()
}
