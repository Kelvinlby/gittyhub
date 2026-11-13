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
