//! Render Markdown (READMEs, PR/issue bodies, release notes) into native GTK widgets.

use adw::prelude::*;
use gtk::glib;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

struct Builder {
    /// Containers being filled (block quotes and list items nest).
    stack: Vec<gtk::Box>,
    text: String,
    /// Open inline tags, closed in reverse when flushing mid-span.
    lists: Vec<Option<u64>>,
    pending_bullet: Option<String>,
    heading: Option<HeadingLevel>,
    code: Option<String>,
    table: Option<(gtk::Grid, i32, i32, bool)>,
    link_stack: Vec<String>,
}

fn label(markup: &str) -> gtk::Label {
    gtk::Label::builder()
        .use_markup(true)
        .label(markup)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .selectable(true)
        .build()
}

impl Builder {
    fn top(&self) -> &gtk::Box {
        self.stack.last().expect("root container")
    }

    fn flush(&mut self) {
        let text = self.text.trim_end().to_owned();
        self.text.clear();
        if text.trim().is_empty() && self.pending_bullet.is_none() {
            return;
        }
        if let Some((grid, row, col, header)) = &mut self.table {
            let l = label(&if *header {
                format!("<b>{text}</b>")
            } else {
                text
            });
            l.set_margin_start(8);
            l.set_margin_end(8);
            l.set_margin_top(4);
            l.set_margin_bottom(4);
            grid.attach(&l, *col, *row, 1, 1);
            return;
        }
        let l = label(&text);
        match self.heading {
            Some(HeadingLevel::H1) => l.add_css_class("title-1"),
            Some(HeadingLevel::H2) => l.add_css_class("title-2"),
            Some(HeadingLevel::H3) => l.add_css_class("title-3"),
            Some(_) => l.add_css_class("heading"),
            None => {}
        }
        if let Some(bullet) = self.pending_bullet.take() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let b = gtk::Label::builder()
                .label(bullet)
                .valign(gtk::Align::Start)
                .css_classes(["dim-label"])
                .build();
            l.set_hexpand(true);
            row.append(&b);
            row.append(&l);
            self.top().append(&row);
        } else {
            self.top().append(&l);
        }
    }

    fn push_container(&mut self, class: &str, indent: i32) {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
        if !class.is_empty() {
            b.add_css_class(class);
        }
        b.set_margin_start(indent);
        self.top().append(&b);
        self.stack.push(b);
    }
}

pub fn render(md: &str) -> gtk::Widget {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    root.add_css_class("markdown");
    let mut b = Builder {
        stack: vec![root.clone()],
        text: String::new(),
        lists: Vec::new(),
        pending_bullet: None,
        heading: None,
        code: None,
        table: None,
        link_stack: Vec::new(),
    };
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for ev in Parser::new_ext(md, opts) {
        match ev {
            Event::Start(tag) => match tag {
                Tag::Paragraph => b.flush(),
                Tag::Heading { level, .. } => {
                    b.flush();
                    b.heading = Some(level);
                }
                Tag::BlockQuote(_) => {
                    b.flush();
                    b.push_container("md-quote", 0);
                }
                Tag::CodeBlock(kind) => {
                    b.flush();
                    let _ = matches!(kind, CodeBlockKind::Fenced(_));
                    b.code = Some(String::new());
                }
                Tag::List(start) => {
                    b.flush();
                    b.lists.push(start);
                    let indent = if b.lists.len() > 1 { 18 } else { 0 };
                    b.push_container("", indent);
                }
                Tag::Item => {
                    b.flush();
                    let bullet = match b.lists.last_mut() {
                        Some(Some(n)) => {
                            let s = format!("{n}.");
                            *n += 1;
                            s
                        }
                        _ => "•".into(),
                    };
                    b.pending_bullet = Some(bullet);
                }
                Tag::Table(_) => {
                    b.flush();
                    let grid = gtk::Grid::builder()
                        .column_spacing(0)
                        .row_spacing(0)
                        .css_classes(["card", "md-table"])
                        .halign(gtk::Align::Start)
                        .build();
                    let scroller = gtk::ScrolledWindow::builder()
                        .child(&grid)
                        .vscrollbar_policy(gtk::PolicyType::Never)
                        .propagate_natural_height(true)
                        .build();
                    b.top().append(&scroller);
                    b.table = Some((grid, -1, 0, false));
                }
                Tag::TableHead => {
                    if let Some(t) = &mut b.table {
                        t.1 += 1;
                        t.2 = 0;
                        t.3 = true;
                    }
                }
                Tag::TableRow => {
                    if let Some(t) = &mut b.table {
                        t.1 += 1;
                        t.2 = 0;
                        t.3 = false;
                    }
                }
                Tag::TableCell => b.text.clear(),
                Tag::Emphasis => b.text.push_str("<i>"),
                Tag::Strong => b.text.push_str("<b>"),
                Tag::Strikethrough => b.text.push_str("<s>"),
                Tag::Link { dest_url, .. } => {
                    b.text.push_str(&format!(
                        "<a href=\"{}\">",
                        glib::markup_escape_text(&dest_url)
                    ));
                    b.link_stack.push(dest_url.to_string());
                }
                Tag::Image { dest_url, .. } => {
                    b.text.push_str(&format!(
                        "<a href=\"{}\">[image: ",
                        glib::markup_escape_text(&dest_url)
                    ));
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => b.flush(),
                TagEnd::Heading(_) => {
                    b.flush();
                    b.heading = None;
                }
                TagEnd::BlockQuote(_) => {
                    b.flush();
                    b.stack.pop();
                }
                TagEnd::CodeBlock => {
                    let code = b.code.take().unwrap_or_default();
                    let l = gtk::Label::builder()
                        .label(code.trim_end())
                        .xalign(0.0)
                        .selectable(true)
                        .css_classes(["mono"])
                        .margin_top(10)
                        .margin_bottom(10)
                        .margin_start(12)
                        .margin_end(12)
                        .build();
                    let scroller = gtk::ScrolledWindow::builder()
                        .child(&l)
                        .vscrollbar_policy(gtk::PolicyType::Never)
                        .propagate_natural_height(true)
                        .css_classes(["card", "md-code"])
                        .build();
                    b.top().append(&scroller);
                }
                TagEnd::List(_) => {
                    b.flush();
                    b.lists.pop();
                    b.stack.pop();
                }
                TagEnd::Item => b.flush(),
                TagEnd::Table => {
                    b.table = None;
                }
                TagEnd::TableCell => {
                    b.flush();
                    if let Some(t) = &mut b.table {
                        t.2 += 1;
                    }
                }
                TagEnd::Emphasis => b.text.push_str("</i>"),
                TagEnd::Strong => b.text.push_str("</b>"),
                TagEnd::Strikethrough => b.text.push_str("</s>"),
                TagEnd::Link => {
                    b.link_stack.pop();
                    b.text.push_str("</a>");
                }
                TagEnd::Image => b.text.push_str("]</a>"),
                _ => {}
            },
            Event::Text(t) => match &mut b.code {
                Some(code) => code.push_str(&t),
                None => b.text.push_str(&glib::markup_escape_text(&t)),
            },
            Event::Code(t) => {
                b.text
                    .push_str(&format!("<tt>{}</tt>", glib::markup_escape_text(&t)));
            }
            Event::SoftBreak => b.text.push(' '),
            Event::HardBreak => b.text.push('\n'),
            Event::Rule => {
                b.flush();
                b.top()
                    .append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            }
            Event::TaskListMarker(done) => b.text.push_str(if done { "☑ " } else { "☐ " }),
            Event::Html(h) | Event::InlineHtml(h) => {
                let h = h.trim().to_lowercase();
                if h.starts_with("<br") {
                    b.text.push('\n');
                }
            }
            _ => {}
        }
    }
    b.flush();
    root.upcast()
}

#[cfg(test)]
mod tests {
    #[test]
    fn renders_without_panicking() {
        gtk::init().ok();
        if gtk::is_initialized() {
            let _ = super::render(
                "# T\n\nSome *em* **b** `c` [l](https://x)\n\n- a\n  - b\n1. x\n\n> q\n\n```\ncode\n```\n\n|a|b|\n|-|-|\n|1|2|\n\n- [x] done\n",
            );
        }
    }
}
