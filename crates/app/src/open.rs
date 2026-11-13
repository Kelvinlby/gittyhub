//! Dialogs for opening, cloning and creating repositories, locally or over SSH.

use std::rc::Rc;

use adw::prelude::*;
use git_backend::Location;
use gtk::{gio, glib};

use crate::form::{self, Field};
use crate::{settings, util};

#[derive(Debug, Clone, Copy)]
pub enum Kind {
    Local,
    Remote,
    Clone,
    New,
}

pub type Done = Rc<dyn Fn(Location)>;

pub fn run(parent: &gtk::Widget, kind: Kind, done: Done) {
    let parent = parent.clone();
    match kind {
        Kind::Clone => clone_dialog(&parent, done),
        Kind::Local => {
            glib::spawn_future_local(async move {
                let dialog = gtk::FileDialog::builder()
                    .title("Open Repository")
                    .modal(true)
                    .build();
                let win = parent.root().and_downcast::<gtk::Window>();
                if let Ok(folder) = dialog.select_folder_future(win.as_ref()).await
                    && let Some(path) = folder.path()
                {
                    verify(&parent, Location::Local(path), done);
                }
            });
        }
        Kind::Remote => {
            glib::spawn_future_local(async move {
                let hosts = git_backend::ssh::configured_hosts();
                let mut options = hosts.clone();
                options.push("Other host…".into());
                let Some(v) = form::run(
                    &parent,
                    "Open Repository over SSH",
                    "GittyHub runs git on the host through your system ssh client, using your ~/.ssh configuration and ssh-agent.",
                    "Open",
                    vec![
                        Field::choice("Host", &options, 0),
                        Field::entry("Other host (user@hostname)", ""),
                        Field::required("Repository path on the host", "~/"),
                    ],
                )
                .await
                else {
                    return;
                };
                let host = match hosts.get(v[0].index()) {
                    Some(h) => h.clone(),
                    None => v[1].text(),
                };
                if host.is_empty() {
                    util::show_error(
                        &parent,
                        "No Host",
                        &anyhow::anyhow!("Choose or enter an SSH host."),
                    );
                    return;
                }
                let path = expand_remote_home(&v[2].text());
                verify(&parent, Location::Ssh { host, path }, done);
            });
        }
        Kind::New => {
            glib::spawn_future_local(async move {
                let hosts = git_backend::ssh::configured_hosts();
                let mut options = vec!["This computer".to_owned()];
                options.extend(hosts.iter().cloned());
                let default_dir = settings::get().clone_dir;
                let Some(v) = form::run(
                    &parent,
                    "New Repository",
                    "Creates an empty repository with git init. Publish it to GitHub later from Remotes & Settings.",
                    "Create",
                    vec![
                        Field::choice("Location", &options, 0),
                        Field::required("Directory", &format!("{default_dir}/new-project")),
                    ],
                )
                .await
                else {
                    return;
                };
                let path = v[1].text();
                let loc = match v[0].index() {
                    0 => Location::Local(path.into()),
                    i => Location::Ssh {
                        host: hosts[i - 1].clone(),
                        path: expand_remote_home(&path),
                    },
                };
                let p = parent.clone();
                util::spawn(
                    {
                        let loc = loc.clone();
                        move || {
                            if let Location::Local(path) = &loc {
                                std::fs::create_dir_all(path)?;
                            }
                            git_backend::init(&loc)
                        }
                    },
                    move |r| match r {
                        Ok(()) => verify(&p, loc, done),
                        Err(e) => util::show_error(&p, "Could Not Create Repository", &e),
                    },
                );
            });
        }
    }
}

/// `~/x` on a remote host: git -C does not expand `~`, so make it relative to the login dir.
fn expand_remote_home(path: &str) -> String {
    let p = path.trim();
    if p == "~" || p == "~/" {
        ".".into()
    } else if let Some(rest) = p.strip_prefix("~/") {
        rest.to_owned()
    } else {
        p.to_owned()
    }
}

/// Check that `loc` is a git work tree and hand back its normalised location.
fn verify(parent: &gtk::Widget, loc: Location, done: Done) {
    let parent = parent.clone();
    util::spawn(
        move || git_backend::toplevel(&loc).map(|top| loc.with_path(&top)),
        move |r| match r {
            Ok(l) => done(l),
            Err(e) => util::show_error(&parent, "Not a Git Repository", &e),
        },
    );
}

fn repo_name_from_url(url: &str) -> String {
    let u = url.trim().trim_end_matches('/');
    let last = u.rsplit(['/', ':']).next().unwrap_or(u);
    last.strip_suffix(".git").unwrap_or(last).to_owned()
}

fn clone_dialog(parent: &gtk::Widget, done: Done) {
    let hosts = git_backend::ssh::configured_hosts();
    let url_row = adw::EntryRow::builder().title("URL or owner/name").build();
    let mut targets = vec!["This computer".to_owned()];
    targets.extend(hosts.iter().cloned());
    let target_strs: Vec<&str> = targets.iter().map(String::as_str).collect();
    let target_row = adw::ComboRow::builder()
        .title("Clone to")
        .model(&gtk::StringList::new(&target_strs))
        .build();
    let dir_row = adw::EntryRow::builder()
        .title("Parent directory")
        .text(settings::get().clone_dir)
        .build();
    let choose = util::icon_button("folder-open-symbolic", "Choose Folder");
    dir_row.add_suffix(&choose);
    let form_list = util::boxed_list();
    form_list.append(&url_row);
    form_list.append(&target_row);
    form_list.append(&dir_row);

    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search your GitHub repositories")
        .build();
    let repos = gtk::ListBox::builder()
        .css_classes(["boxed-list"])
        .selection_mode(gtk::SelectionMode::None)
        .build();
    repos.set_placeholder(Some(
        &gtk::Label::builder()
            .label("Loading repositories from GitHub…")
            .margin_top(18)
            .margin_bottom(18)
            .css_classes(["dim-label"])
            .build(),
    ));

    let clone_button = gtk::Button::builder()
        .label("Clone")
        .css_classes(["suggested-action", "pill"])
        .halign(gtk::Align::Center)
        .sensitive(false)
        .build();
    let content = util::vbox(18);
    content.append(&form_list);
    content.append(&clone_button);
    content.append(&util::heading("Your Repositories", "heading"));
    content.append(&search);
    content.append(&repos);
    let dialog = util::content_dialog(
        parent,
        "Clone Repository",
        &util::page_scroller(&content),
        600,
        720,
    );

    url_row.connect_changed(glib::clone!(
        #[weak]
        clone_button,
        move |r| clone_button.set_sensitive(!r.text().trim().is_empty())
    ));
    target_row.connect_selected_notify(glib::clone!(
        #[weak]
        dir_row,
        #[weak]
        choose,
        move |r| {
            let local = r.selected() == 0;
            choose.set_visible(local);
            dir_row.set_text(&if local {
                settings::get().clone_dir
            } else {
                "~/".into()
            });
        }
    ));
    choose.connect_clicked(glib::clone!(
        #[weak]
        dir_row,
        #[weak]
        dialog,
        move |_| {
            glib::spawn_future_local(async move {
                let fd = gtk::FileDialog::builder().title("Clone Into").build();
                let win = dialog.root().and_downcast::<gtk::Window>();
                if let Ok(f) = fd.select_folder_future(win.as_ref()).await
                    && let Some(p) = f.path()
                {
                    dir_row.set_text(&p.to_string_lossy());
                }
            });
        }
    ));
    search.connect_search_changed(glib::clone!(
        #[weak]
        repos,
        move |e| {
            let q = e.text().to_lowercase();
            let mut c = repos.first_child();
            while let Some(w) = c {
                w.set_visible(w.widget_name().to_lowercase().contains(&q));
                c = w.next_sibling();
            }
        }
    ));

    // Load the user's repositories.
    let repos_weak = repos.downgrade();
    let url_weak = url_row.downgrade();
    util::spawn(
        || {
            let protocol = gh_backend::run(&["config", "get", "git_protocol"], None)
                .map(|s| s.trim().to_owned())
                .unwrap_or_default();
            Ok((gh_backend::repo_list(None, 200)?, protocol == "ssh"))
        },
        move |r: anyhow::Result<(Vec<gh_backend::RepoSummary>, bool)>| {
            let Some(repos) = repos_weak.upgrade() else {
                return;
            };
            match r {
                Ok((list, ssh)) => {
                    repos.set_placeholder(Some(
                        &gtk::Label::builder()
                            .label("No repositories")
                            .margin_top(18)
                            .margin_bottom(18)
                            .css_classes(["dim-label"])
                            .build(),
                    ));
                    for repo in list {
                        let row = adw::ActionRow::builder()
                            .title(glib::markup_escape_text(&repo.name_with_owner))
                            .subtitle(glib::markup_escape_text(
                                repo.description.as_deref().unwrap_or(""),
                            ))
                            .subtitle_lines(1)
                            .activatable(true)
                            .build();
                        row.set_widget_name(&repo.name_with_owner);
                        if repo.is_private {
                            row.add_suffix(&util::pill("Private", ""));
                        }
                        if repo.is_fork {
                            row.add_suffix(&util::pill("Fork", ""));
                        }
                        let url = if ssh {
                            repo.ssh_url.clone()
                        } else {
                            format!("{}.git", repo.url)
                        };
                        let url_weak = url_weak.clone();
                        row.connect_activated(move |_| {
                            if let Some(u) = url_weak.upgrade() {
                                u.set_text(&url);
                            }
                        });
                        repos.append(&row);
                    }
                }
                Err(e) => repos.set_placeholder(Some(
                    &gtk::Label::builder()
                        .label(format!("Could not list repositories: {e}"))
                        .wrap(true)
                        .margin_top(18)
                        .margin_bottom(18)
                        .css_classes(["dim-label"])
                        .build(),
                )),
            }
        },
    );

    clone_button.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        #[weak]
        url_row,
        #[weak]
        dir_row,
        #[weak]
        target_row,
        move |button| {
            let mut url = url_row.text().trim().to_owned();
            let parent_dir = dir_row.text().trim().trim_end_matches('/').to_owned();
            let target = target_row.selected() as usize;
            let host = (target > 0).then(|| hosts[target - 1].clone());
            button.set_sensitive(false);
            button.set_label("Cloning…");
            let done = done.clone();
            let button = button.clone();
            util::spawn(
                move || {
                    // Bare owner/name: resolve to a URL using gh's preferred protocol.
                    if !url.contains(':') && url.split('/').count() == 2 {
                        let ssh = gh_backend::run(&["config", "get", "git_protocol"], None)
                            .map(|s| s.trim() == "ssh")
                            .unwrap_or(false);
                        url = if ssh {
                            format!("git@github.com:{url}.git")
                        } else {
                            format!("https://github.com/{url}.git")
                        };
                    }
                    let name = repo_name_from_url(&url);
                    let dest = format!("{parent_dir}/{name}");
                    let loc = match host {
                        None => {
                            std::fs::create_dir_all(&parent_dir)?;
                            Location::Local(dest.clone().into())
                        }
                        Some(h) => Location::Ssh {
                            host: h,
                            path: expand_remote_home(&dest),
                        },
                    };
                    let dest_path = match &loc {
                        Location::Local(p) => p.to_string_lossy().into_owned(),
                        Location::Ssh { path, .. } => path.clone(),
                    };
                    // Relative remote paths are relative to the login directory, so clone from there.
                    let base = match &loc {
                        Location::Ssh { path, .. } if !path.starts_with('/') => loc.with_path("."),
                        _ => loc.with_path("/"),
                    };
                    base.git(&["clone", &url, &dest_path])?;
                    Ok(loc)
                },
                move |r| match r {
                    Ok(loc) => {
                        dialog.close();
                        let top = loc.clone();
                        util::spawn(
                            move || git_backend::toplevel(&top).map(|t| top.with_path(&t)),
                            move |r| done(r.unwrap_or(loc)),
                        );
                    }
                    Err(e) => {
                        button.set_sensitive(true);
                        button.set_label("Clone");
                        util::show_error(&dialog, "Clone Failed", &e);
                    }
                },
            );
        }
    ));
    let _ = gio::Cancellable::NONE;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(repo_name_from_url("git@github.com:a/b.git"), "b");
        assert_eq!(repo_name_from_url("https://github.com/a/b/"), "b");
        assert_eq!(expand_remote_home("~/src/x"), "src/x");
    }
}
