//! Source views (GtkSourceView): syntax highlighting, line numbers, and a style scheme that
//! follows the app's light/dark appearance.

use std::path::Path;

use adw::prelude::*;
use sourceview::prelude::*;

fn scheme(dark: bool) -> Option<sourceview::StyleScheme> {
    let m = sourceview::StyleSchemeManager::default();
    let ids: &[&str] = if dark {
        &["Adwaita-dark", "Yaru-dark", "oblivion"]
    } else {
        &["Adwaita", "Yaru", "classic"]
    };
    ids.iter().find_map(|id| m.scheme(id))
}

/// A monospace source view. Read-only views still get line numbers and highlighting.
pub fn view(editable: bool) -> sourceview::View {
    let buffer = sourceview::Buffer::new(None);
    buffer.set_highlight_matching_brackets(editable);
    let sm = adw::StyleManager::default();
    buffer.set_style_scheme(scheme(sm.is_dark()).as_ref());
    let weak = buffer.downgrade();
    sm.connect_dark_notify(move |sm| {
        if let Some(b) = weak.upgrade() {
            b.set_style_scheme(scheme(sm.is_dark()).as_ref());
        }
    });
    sourceview::View::builder()
        .buffer(&buffer)
        .editable(editable)
        .cursor_visible(editable)
        .monospace(true)
        .show_line_numbers(true)
        .highlight_current_line(editable)
        .auto_indent(true)
        .indent_on_tab(true)
        .smart_backspace(true)
        .smart_home_end(sourceview::SmartHomeEndType::Before)
        .tab_width(4)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(4)
        .right_margin(8)
        .build()
}

pub fn source_buffer(view: &sourceview::View) -> sourceview::Buffer {
    view.buffer()
        .downcast::<sourceview::Buffer>()
        .expect("source view has a source buffer")
}

/// Load `text` for `path`, choosing the highlighting language from the file name and content.
pub fn set_text(view: &sourceview::View, path: &str, text: &str) {
    let buffer = source_buffer(view);
    let content_type = gtk::gio::content_type_guess(Some(Path::new(path)), text.as_bytes()).0;
    let lang = sourceview::LanguageManager::default()
        .guess_language(Some(Path::new(path)), Some(&content_type));
    buffer.set_language(lang.as_ref());
    // Loading text should not be undoable.
    buffer.begin_irreversible_action();
    buffer.set_text(text);
    buffer.end_irreversible_action();
    buffer.place_cursor(&buffer.start_iter());
    buffer.set_modified(false);
    // Spaces vs tabs: follow the file.
    let tabs = text.lines().filter(|l| l.starts_with('\t')).count();
    let spaces = text.lines().filter(|l| l.starts_with("  ")).count();
    view.set_insert_spaces_instead_of_tabs(spaces >= tabs);
}

/// Text style of a highlighted span.
#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    pub fg: Option<gtk::gdk::RGBA>,
    pub bold: bool,
    pub italic: bool,
}

impl Style {
    /// Stable key for caching one text tag per distinct style.
    pub fn key(&self) -> String {
        format!(
            "syn:{}:{}:{}",
            self.fg.map(|c| c.to_string()).unwrap_or_default(),
            self.bold,
            self.italic
        )
    }
}

/// Highlight `lines` as one document of the language guessed for `path`.
/// Returns, per line, styled spans as (start char, end char, style).
pub fn highlight(path: &str, lines: &[&str]) -> Vec<Vec<(usize, usize, Style)>> {
    let mut out = vec![Vec::new(); lines.len()];
    let text = lines.join("\n");
    let content_type = gtk::gio::content_type_guess(Some(Path::new(path)), text.as_bytes()).0;
    let Some(lang) = sourceview::LanguageManager::default()
        .guess_language(Some(Path::new(path)), Some(&content_type))
    else {
        return out;
    };
    let buffer = sourceview::Buffer::with_language(&lang);
    buffer.set_style_scheme(scheme(adw::StyleManager::default().is_dark()).as_ref());
    buffer.set_text(&text);
    buffer.ensure_highlight(&buffer.start_iter(), &buffer.end_iter());
    for (n, spans) in out.iter_mut().enumerate() {
        let Some(mut it) = buffer.iter_at_line(n as i32) else {
            continue;
        };
        let mut line_end = it;
        if !line_end.ends_line() {
            line_end.forward_to_line_end();
        }
        let line_start = it.offset();
        while it.offset() < line_end.offset() {
            let mut next = it;
            if !next.forward_to_tag_toggle(None::<&gtk::TextTag>)
                || next.offset() > line_end.offset()
            {
                next = line_end;
            }
            let mut style = Style {
                fg: None,
                bold: false,
                italic: false,
            };
            // Later tags in the list have higher priority.
            for tag in it.tags() {
                if tag.is_foreground_set() {
                    style.fg = tag.foreground_rgba();
                }
                if tag.is_weight_set() && tag.weight() >= 600 {
                    style.bold = true;
                }
                if tag.is_style_set() && tag.style() == gtk::pango::Style::Italic {
                    style.italic = true;
                }
            }
            if style.fg.is_some() || style.bold || style.italic {
                spans.push((
                    (it.offset() - line_start) as usize,
                    (next.offset() - line_start) as usize,
                    style,
                ));
            }
            if next.offset() <= it.offset() {
                break;
            }
            it = next;
        }
    }
    out
}
