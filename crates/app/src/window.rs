use std::path::{Path, PathBuf};

use adw::prelude::*;
use gtk::{gio, glib};

pub fn build(app: &adw::Application) -> adw::ApplicationWindow {
    let repo = std::env::current_dir().unwrap_or_default();

    // Content: command output.
    let buffer = gtk::TextBuffer::new(None);
    buffer.set_text("Click a button to query git or gh.");
    let output = gtk::TextView::builder()
        .buffer(&buffer)
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .top_margin(12)
        .bottom_margin(12)
        .left_margin(12)
        .right_margin(12)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .child(&output)
        .vexpand(true)
        .build();

    let content_view = adw::ToolbarView::new();
    content_view.add_top_bar(&adw::HeaderBar::new());
    content_view.set_content(Some(&scroller));
    let content = adw::NavigationPage::builder()
        .title(repo.display().to_string())
        .child(&content_view)
        .build();

    // Sidebar: actions.
    let status_btn = gtk::Button::with_label("git status");
    let prs_btn = gtk::Button::with_label("gh pr list");
    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    actions.append(&status_btn);
    actions.append(&prs_btn);

    let sidebar_view = adw::ToolbarView::new();
    sidebar_view.add_top_bar(&adw::HeaderBar::new());
    sidebar_view.set_content(Some(&actions));
    let sidebar = adw::NavigationPage::builder()
        .title("GittyHub")
        .child(&sidebar_view)
        .build();

    let split = adw::NavigationSplitView::builder()
        .sidebar(&sidebar)
        .content(&content)
        .build();

    status_btn.connect_clicked(on_click(&buffer, &repo, git_status));
    prs_btn.connect_clicked(on_click(&buffer, &repo, gh_prs));

    adw::ApplicationWindow::builder()
        .application(app)
        .title("GittyHub")
        .default_width(1200)
        .default_height(800)
        .content(&split)
        .build()
}

fn git_status(repo: PathBuf) -> anyhow::Result<String> {
    let branch = git_backend::current_branch(&repo)?;
    let mut lines = vec![format!("On branch {branch}")];
    lines.extend(git_backend::status(&repo)?);
    Ok(lines.join("\n"))
}

fn gh_prs(repo: PathBuf) -> anyhow::Result<String> {
    let prs = gh_backend::pr_list(&repo)?;
    if prs.is_empty() {
        return Ok("No open pull requests.".into());
    }
    Ok(prs
        .iter()
        .map(|p| format!("#{} {}", p.number, p.title))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Button handler that runs `job` on a worker thread and shows its result in `buffer`.
fn on_click(
    buffer: &gtk::TextBuffer,
    repo: &Path,
    job: fn(PathBuf) -> anyhow::Result<String>,
) -> impl Fn(&gtk::Button) + 'static {
    let buffer = buffer.clone();
    let repo = repo.to_path_buf();
    move |_| {
        buffer.set_text("Running…");
        let buffer = buffer.clone();
        let repo = repo.clone();
        glib::spawn_future_local(async move {
            let text = match gio::spawn_blocking(move || job(repo)).await {
                Ok(Ok(text)) => text,
                Ok(Err(e)) => format!("error: {e:#}"),
                Err(_) => "error: background task panicked".into(),
            };
            buffer.set_text(&text);
        });
    }
}
