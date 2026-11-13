//! Preferences: appearance, behaviour and account information.

use adw::prelude::*;
use gtk::glib;

use crate::{settings, util};

pub fn show(parent: Option<&gtk::Window>) {
    let s = settings::get();
    let dialog = adw::PreferencesDialog::builder()
        .search_enabled(true)
        .build();

    // Appearance
    let appearance = adw::PreferencesPage::builder()
        .title("Appearance")
        .icon_name("applications-graphics-symbolic")
        .build();
    let style_group = adw::PreferencesGroup::builder().title("Style").build();
    let scheme = adw::ComboRow::builder()
        .title("Color Scheme")
        .model(&gtk::StringList::new(&["Follow System", "Light", "Dark"]))
        .selected(match s.color_scheme.as_str() {
            "light" => 1,
            "dark" => 2,
            _ => 0,
        })
        .build();
    scheme.connect_selected_notify(|r| {
        let v = ["system", "light", "dark"][r.selected() as usize % 3];
        settings::update(|s| s.color_scheme = v.into());
        settings::apply_appearance();
    });
    style_group.add(&scheme);
    let zoom = adw::SpinRow::builder()
        .title("Interface Zoom")
        .subtitle("Percent; also Ctrl+= / Ctrl+− / Ctrl+0")
        .adjustment(&gtk::Adjustment::new(
            (s.ui_zoom * 100.0).round(),
            settings::ZOOM_MIN * 100.0,
            settings::ZOOM_MAX * 100.0,
            10.0,
            25.0,
            0.0,
        ))
        .build();
    zoom.connect_value_notify(|r| {
        let v = r.value() / 100.0;
        if (settings::get().ui_zoom - v).abs() > f64::EPSILON {
            settings::update(|s| s.ui_zoom = v);
            settings::apply_zoom();
        }
    });
    style_group.add(&zoom);
    appearance.add(&style_group);

    let accent_group = adw::PreferencesGroup::builder()
        .title("Accent Color")
        .description("Used for buttons, selections and highlights")
        .build();
    let swatches = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .max_children_per_line(10)
        .min_children_per_line(5)
        .column_spacing(12)
        .row_spacing(12)
        .homogeneous(true)
        .margin_top(6)
        .build();
    let swatch_css = gtk::CssProvider::new();
    let mut css = String::new();
    let mut first: Option<gtk::ToggleButton> = None;
    let system = gtk::ToggleButton::builder()
        .icon_name("emblem-system-symbolic")
        .tooltip_text("Follow System")
        .css_classes(["accent-swatch"])
        .halign(gtk::Align::Center)
        .active(s.accent == "system")
        .build();
    system.connect_toggled(|b| {
        if b.is_active() {
            settings::update(|s| s.accent = "system".into());
            settings::apply_appearance();
        }
    });
    swatches.insert(&system, -1);
    first.get_or_insert(system.clone());
    for (id, name, color) in settings::ACCENTS {
        css.push_str(&format!(
            ".accent-swatch.swatch-{id} {{ background: {}; }}\n",
            color.to_rgba()
        ));
        let b = gtk::ToggleButton::builder()
            .tooltip_text(name)
            .css_classes(["accent-swatch", &format!("swatch-{id}")])
            .halign(gtk::Align::Center)
            .group(first.as_ref().unwrap())
            .active(s.accent == id)
            .build();
        b.connect_toggled(move |b| {
            if b.is_active() {
                settings::update(|s| s.accent = id.into());
                settings::apply_appearance();
            }
        });
        swatches.insert(&b, -1);
    }
    swatch_css.load_from_string(&css);
    gtk::style_context_add_provider_for_display(
        &swatches.display(),
        &swatch_css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    accent_group.add(&swatches);
    appearance.add(&accent_group);
    appearance.add(&code_group(&s));
    dialog.add(&appearance);

    // Behaviour
    let behaviour = adw::PreferencesPage::builder()
        .title("Behavior")
        .icon_name("preferences-system-symbolic")
        .build();
    let general = adw::PreferencesGroup::builder().title("General").build();
    let restore = adw::SwitchRow::builder()
        .title("Restore Tabs")
        .subtitle("Reopen the repositories that were open when GittyHub was closed")
        .active(s.restore_tabs)
        .build();
    restore.connect_active_notify(|r| {
        let v = r.is_active();
        settings::update(|s| s.restore_tabs = v);
    });
    general.add(&restore);
    let clone_dir = adw::EntryRow::builder()
        .title("Default Clone Directory")
        .text(&s.clone_dir)
        .show_apply_button(true)
        .build();
    clone_dir.connect_apply(|r| {
        let v = r.text().to_string();
        settings::update(|s| s.clone_dir = v);
    });
    general.add(&clone_dir);
    behaviour.add(&general);

    let sync = adw::PreferencesGroup::builder().title("Syncing").build();
    let pull = adw::ComboRow::builder()
        .title("Pull Strategy")
        .subtitle("How diverged branches are reconciled when pulling")
        .model(&gtk::StringList::new(&[
            "Merge",
            "Rebase",
            "Fast-forward only",
        ]))
        .selected(match s.pull_mode.as_str() {
            "rebase" => 1,
            "ff-only" => 2,
            _ => 0,
        })
        .build();
    pull.connect_selected_notify(|r| {
        let v = ["merge", "rebase", "ff-only"][r.selected() as usize % 3];
        settings::update(|s| s.pull_mode = v.into());
    });
    sync.add(&pull);
    let fetch = adw::SpinRow::builder()
        .title("Background Fetch Interval")
        .subtitle("Minutes between automatic fetches; 0 turns it off")
        .adjustment(&gtk::Adjustment::new(
            s.auto_fetch_minutes as f64,
            0.0,
            240.0,
            1.0,
            5.0,
            0.0,
        ))
        .build();
    fetch.connect_value_notify(|r| {
        let v = r.value() as u32;
        settings::update(|s| s.auto_fetch_minutes = v);
    });
    sync.add(&fetch);
    behaviour.add(&sync);

    let commits = adw::PreferencesGroup::builder()
        .title("Commits and History")
        .build();
    let signoff = adw::SwitchRow::builder()
        .title("Sign Off Commits")
        .subtitle("Add a Signed-off-by trailer to new commits")
        .active(s.signoff)
        .build();
    signoff.connect_active_notify(|r| {
        let v = r.is_active();
        settings::update(|s| s.signoff = v);
    });
    commits.add(&signoff);
    // Custom sign-off; empty means git's own `user.name <user.email>`.
    let signoff_as = adw::EntryRow::builder()
        .title("Signed-off-by (empty: your git name and email)")
        .text(&s.signoff_as)
        .show_apply_button(true)
        .build();
    signoff
        .bind_property("active", &signoff_as, "sensitive")
        .sync_create()
        .build();
    commits.add(&signoff_as);
    // Show exactly what will be added; the git identity is read off the main thread.
    let identity: std::rc::Rc<std::cell::RefCell<Option<String>>> = Default::default();
    let show = {
        let (row, identity) = (signoff.clone(), identity.clone());
        move || {
            let custom = settings::get().signoff_as;
            let who = if custom.trim().is_empty() {
                identity
                    .borrow()
                    .clone()
                    .unwrap_or_else(|| "your git user.name <user.email>".into())
            } else {
                custom.trim().to_owned()
            };
            row.set_subtitle(&glib::markup_escape_text(&format!(
                "Adds “Signed-off-by: {who}” to new commits"
            )));
        }
    };
    show();
    {
        let show = show.clone();
        signoff_as.connect_apply(move |r| {
            let v = r.text().trim().to_owned();
            settings::update(|s| s.signoff_as = v);
            show();
        });
    }
    {
        let show = show.clone();
        util::spawn(
            || {
                Ok(git_backend::signoff_identity(
                    &git_backend::Location::Local(glib::home_dir()),
                ))
            },
            move |r| {
                *identity.borrow_mut() = r.ok().flatten();
                show();
            },
        );
    }
    let all = adw::SwitchRow::builder()
        .title("Show All Branches in History")
        .subtitle("Otherwise only the current branch's history is listed")
        .active(s.history_all_branches)
        .build();
    all.connect_active_notify(|r| {
        let v = r.is_active();
        settings::update(|s| s.history_all_branches = v);
    });
    commits.add(&all);
    behaviour.add(&commits);
    dialog.add(&behaviour);

    // Accounts
    let accounts = adw::PreferencesPage::builder()
        .title("Accounts")
        .icon_name("avatar-default-symbolic")
        .build();
    let gh_group = adw::PreferencesGroup::builder()
        .title("GitHub")
        .description("GittyHub never stores credentials: GitHub access goes through the gh CLI, and git uses its own credential helpers and your SSH agent.")
        .build();
    let status_row = adw::ActionRow::builder()
        .title("Checking gh authentication…")
        .subtitle_selectable(true)
        .build();
    status_row.add_prefix(&gtk::Image::from_icon_name("gh-github-symbolic"));
    gh_group.add(&status_row);
    let ssh_group = adw::PreferencesGroup::builder()
        .title("SSH Hosts")
        .description("Hosts from ~/.ssh/config, available when opening or cloning over SSH")
        .build();
    let hosts = git_backend::ssh::configured_hosts();
    if hosts.is_empty() {
        ssh_group.add(
            &adw::ActionRow::builder()
                .title("No hosts configured")
                .subtitle("Add Host entries to ~/.ssh/config, or type user@host when opening")
                .build(),
        );
    }
    for h in hosts {
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&h))
            .build();
        row.add_prefix(&gtk::Image::from_icon_name("gh-server-symbolic"));
        ssh_group.add(&row);
    }
    accounts.add(&gh_group);
    accounts.add(&ssh_group);
    dialog.add(&accounts);

    let row = status_row.downgrade();
    util::spawn(gh_backend::current_user, move |r| {
        let Some(row) = row.upgrade() else { return };
        match r {
            Ok(u) => {
                row.set_title(&format!(
                    "Signed in as {}",
                    glib::markup_escape_text(&u.login)
                ));
                row.set_subtitle("Manage with “gh auth” in a terminal");
            }
            Err(e) => {
                row.set_title("Not signed in");
                row.set_subtitle(&glib::markup_escape_text(&format!(
                    "Run “gh auth login” in a terminal. ({e})"
                )));
            }
        }
    });

    dialog.present(parent);
}

/// Font, size and line spacing for diffs, the editor and file views, with a live preview.
fn code_group(s: &settings::Settings) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("Code")
        .description("Diffs, the editor and file views")
        .build();

    let font_dialog = gtk::FontDialog::builder().title("Code Font").build();
    let font_button = gtk::FontDialogButton::builder()
        .dialog(&font_dialog)
        .level(gtk::FontLevel::Family)
        .valign(gtk::Align::Center)
        .build();
    if !s.code_font_family.is_empty() {
        let mut desc = gtk::pango::FontDescription::new();
        desc.set_family(&s.code_font_family);
        font_button.set_font_desc(&desc);
    }
    let reset = util::icon_button("edit-undo-symbolic", "Use the System Monospace Font");
    reset.set_visible(!s.code_font_family.is_empty());
    let font_row = adw::ActionRow::builder()
        .title("Font")
        .subtitle(if s.code_font_family.is_empty() {
            "System monospace font".to_owned()
        } else {
            glib::markup_escape_text(&s.code_font_family).to_string()
        })
        .build();
    font_row.add_suffix(&reset);
    font_row.add_suffix(&font_button);
    font_row.set_activatable_widget(Some(&font_button));
    {
        let (row, reset) = (font_row.clone(), reset.clone());
        font_button.connect_font_desc_notify(move |b| {
            let family = b
                .font_desc()
                .and_then(|d| d.family().map(|f| f.to_string()))
                .unwrap_or_default();
            row.set_subtitle(&glib::markup_escape_text(&family));
            reset.set_visible(!family.is_empty());
            settings::update(|s| s.code_font_family = family);
            settings::apply_code_style();
        });
    }
    {
        let row = font_row.clone();
        reset.connect_clicked(move |b| {
            b.set_visible(false);
            row.set_subtitle("System monospace font");
            settings::update(|s| s.code_font_family.clear());
            settings::apply_code_style();
        });
    }
    group.add(&font_row);

    let size = adw::SpinRow::builder()
        .title("Font Size")
        .subtitle("In points")
        .digits(1)
        .adjustment(&gtk::Adjustment::new(
            s.code_font_size,
            6.0,
            48.0,
            0.5,
            1.0,
            0.0,
        ))
        .build();
    size.connect_value_notify(|r| {
        let v = r.value();
        settings::update(|s| s.code_font_size = v);
        settings::apply_code_style();
    });
    group.add(&size);

    let spacing = adw::SpinRow::builder()
        .title("Line Spacing")
        .subtitle("Line height as a multiple of the font size")
        .digits(1)
        .adjustment(&gtk::Adjustment::new(
            s.code_line_spacing,
            1.0,
            3.0,
            0.1,
            0.5,
            0.0,
        ))
        .build();
    spacing.connect_value_notify(|r| {
        let v = r.value();
        settings::update(|s| s.code_line_spacing = v);
        settings::apply_code_style();
    });
    group.add(&spacing);

    // Live preview using the real code view.
    let preview = crate::editor::view(false);
    crate::editor::set_text(
        &preview,
        "preview.rs",
        "/// Preview of the code font.\nfn greet(name: &str) -> String {\n    format!(\"Hello, {name}!\") // 0O 1lI\n}",
    );
    let frame = gtk::Frame::builder().child(&preview).margin_top(12).build();
    frame.add_css_class("view");
    group.add(&frame);
    group
}
