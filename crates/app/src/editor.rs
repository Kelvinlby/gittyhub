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
