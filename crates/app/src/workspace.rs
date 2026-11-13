use std::path::PathBuf;

use gpui::{Context, IntoElement, ParentElement, Render, Styled, Window, div};
use gpui_component::{ActiveTheme, StyledExt, button::Button, h_flex, v_flex};

pub struct Workspace {
    repo: PathBuf,
    output: Vec<String>,
}

impl Workspace {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            repo: std::env::current_dir().unwrap_or_default(),
            output: vec!["Click a button to query git or gh.".into()],
        }
    }

    fn refresh_status(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let branch = git_backend::current_branch(&repo)?;
                    let mut lines = vec![format!("On branch {branch}")];
                    lines.extend(git_backend::status(&repo)?);
                    anyhow::Ok(lines)
                })
                .await;
            this.update(cx, |this, cx| {
                this.output = result.unwrap_or_else(|e| vec![format!("error: {e:#}")]);
                cx.notify();
            })
        })
        .detach();
    }

    fn refresh_prs(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { gh_backend::pr_list(&repo) })
                .await;
            this.update(cx, |this, cx| {
                this.output = match result {
                    Ok(prs) if prs.is_empty() => vec!["No open pull requests.".into()],
                    Ok(prs) => prs
                        .iter()
                        .map(|p| format!("#{} {}", p.number, p.title))
                        .collect(),
                    Err(e) => vec![format!("error: {e:#}")],
                };
                cx.notify();
            })
        })
        .detach();
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar = v_flex()
            .w_56()
            .h_full()
            .p_3()
            .gap_2()
            .border_r_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .child(div().font_bold().child("GittyHub"))
            .child(
                Button::new("status")
                    .label("git status")
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_status(cx))),
            )
            .child(
                Button::new("prs")
                    .label("gh pr list")
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_prs(cx))),
            );

        let content = v_flex()
            .flex_1()
            .p_4()
            .gap_1()
            .font_family("monospace")
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.repo.display().to_string()),
            )
            .children(self.output.iter().map(|l| div().child(l.clone())));

        h_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(sidebar)
            .child(content)
    }
}
