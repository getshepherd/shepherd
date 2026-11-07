//! Settings → Extensions, above the pages added by hand: the extensions installed (`crate::extensions`), whether each
//! runs always or with Shepherd, removing one, their folder, and installing one from a GitHub release.
use super::*;
use crate::extensions::{self as ext, Extension, Extensions, Release};

/// Where installing from GitHub is.
#[derive(Default)]
pub(super) enum Install {
    #[default]
    Idle,
    /// The repo's latest release is being looked up.
    Finding,
    /// Found: installing runs it, so it waits for a yes.
    Confirm(Release),
    Installing,
}

/// `action`'s pill in `bg`, for the yes of a confirmation; its own, since a second `hover` on one panics.
fn filled(id: &'static str, label: &'static str, bg: gpui::Hsla, fg: gpui::Hsla) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .min_h(px(32.))
        .px(px(10.))
        .flex()
        .items_center()
        .rounded(px(8.))
        .bg(bg)
        .text_color(fg)
        .text_size(px(12.5))
        .cursor_pointer()
        .hover(move |s| s.bg(bg.opacity(0.88)))
        .child(label)
}

/// The two ways an extension runs, as its state's `start` and the choice's label.
const STARTS: [(&str, &str); 2] = [("login", "Always"), ("shepherd", "With Shepherd")];

impl SettingsView {
    /// Runs `work` off the main thread, then `done` with its result on the view, the list scanned again.
    fn ext_job<T: Send + 'static>(&self, work: impl FnOnce() -> T + Send + 'static, done: impl FnOnce(&mut Self, T) + 'static, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async move { work() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |v, cx| {
                done(v, result);
                ext::refresh(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn switch_start(&mut self, e: Extension, start: &'static str, cx: &mut Context<Self>) {
        (self.ext_busy, self.ext_note) = (Some((e.name.clone(), "Switching…")), None);
        self.ext_job(
            move || ext::set_start(&e, start),
            |v, result| {
                v.ext_busy = None;
                v.ext_note = result.err();
            },
            cx,
        );
    }

    fn remove_extension(&mut self, e: Extension, cx: &mut Context<Self>) {
        (self.ext_busy, self.ext_remove, self.ext_note) = (Some((e.name.clone(), "Removing…")), None, None);
        self.ext_job(
            move || ext::uninstall(&e),
            |v, result| {
                v.ext_busy = None;
                v.ext_note = result.err();
            },
            cx,
        );
    }

    pub(super) fn find_release(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = ext::parse_repo(&self.ext_repo.read(cx).value()) else {
            self.install_note = Some("Type a GitHub repo: owner/name, or its URL.".into());
            return cx.notify();
        };
        (self.install, self.install_note) = (Install::Finding, None);
        self.ext_job(
            move || ext::latest_release(&repo),
            |v, found| match found {
                Ok(release) => v.install = Install::Confirm(release),
                Err(why) => (v.install, v.install_note) = (Install::Idle, Some(why)),
            },
            cx,
        );
    }

    fn install_release(&mut self, release: Release, cx: &mut Context<Self>) {
        self.install = Install::Installing;
        let what = format!("{} {}", release.repo, release.tag);
        self.ext_job(
            move || ext::install(&release),
            move |v, result| {
                v.install = Install::Idle;
                v.install_note = Some(result.map_or_else(|why| format!("{what} didn't install: {why}"), |()| format!("Installed {what}.")));
            },
            cx,
        );
    }

    /// The installed extensions' card with Open folder by its label, then installing from GitHub.
    pub(super) fn extensions(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let muted = theme.text_muted;
        let list = cx.try_global::<Extensions>().map(|e| e.0.clone()).unwrap_or_default();
        let empty = list.is_empty();
        let rows = list.into_iter().enumerate().map(|(i, e)| {
            let busy = self.ext_busy.as_ref().filter(|(name, _)| *name == e.name).map(|(_, doing)| *doing);
            if self.ext_remove.as_ref() == Some(&e.name) {
                let danger = theme.danger;
                return card_row(theme, i == 0)
                    .child(row_text(
                        theme,
                        format!("Remove {}?", e.name),
                        "Its service, its skill and its binary go. Its data stays in its folder, for when it is installed again.",
                    ))
                    .child(action(theme, "ext-remove-cancel", "Cancel").on_click(cx.listener(|v, _: &ClickEvent, _, cx| {
                        v.ext_remove = None;
                        cx.notify();
                    })))
                    .child(
                        filled("ext-remove-yes", "Remove", danger, gpui::white())
                            .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.remove_extension(e.clone(), cx))),
                    );
            }
            let state = busy.unwrap_or(if e.alive { "Running" } else { "Not running" });
            let busy = busy.is_some();
            let status = div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.))
                .text_color(muted)
                .child(div().size(px(6.)).rounded_full().bg(if e.alive { theme.success } else { theme.text_faint }))
                .child(state);
            let choices = div().flex_none().flex().gap(px(8.)).children(STARTS.map(|(start, label)| {
                let (e, current) = (e.clone(), e.start == start);
                choice(theme, SharedString::from(format!("ext-start-{}-{start}", e.name)), label, current)
                    .when(!current && !busy, |el| el.on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.switch_start(e.clone(), start, cx))))
            }));
            let description = if e.description.is_empty() { e.binary.display().to_string() } else { e.description.clone() };
            let name = e.name.clone();
            let remove = remove_button(theme, format!("ext-remove-{name}")).mt(px(0.)).when(!busy, |el| {
                el.on_click(cx.listener(move |v, _: &ClickEvent, _, cx| {
                    v.ext_remove = Some(name.clone());
                    cx.notify();
                }))
            });
            card_row(theme, i == 0).child(row_text(theme, e.name.clone(), description)).child(status).child(choices).child(remove)
        });
        let install = match &self.install {
            Install::Confirm(release) => {
                let release = release.clone();
                card_row(theme, true)
                    .child(row_text(
                        theme,
                        format!("Install {} {}?", release.repo, release.tag),
                        format!("{} is downloaded and run on this computer, with your rights. Install only what you trust.", release.asset),
                    ))
                    .child(action(theme, "ext-install-cancel", "Cancel").on_click(cx.listener(|v, _: &ClickEvent, _, cx| {
                        v.install = Install::Idle;
                        cx.notify();
                    })))
                    .child(
                        filled("ext-install-yes", "Install", theme.accent, theme.on_accent)
                            .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.install_release(release.clone(), cx))),
                    )
            }
            install => {
                let button = match install {
                    Install::Finding => action(theme, "ext-install", "Looking…"),
                    Install::Installing => action(theme, "ext-install", "Installing…"),
                    _ => action(theme, "ext-install", "Install").on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.find_release(cx))),
                };
                card_row(theme, true)
                    .child(row_text(
                        theme,
                        "Install from GitHub",
                        format!("owner/repo: its latest release's build for this computer (a file ending in {}).", ext::asset_suffix()),
                    ))
                    .child(field(theme, &self.ext_repo))
                    .child(button)
            }
        };
        div()
            .child(
                div()
                    .mt(px(32.))
                    .mb(px(8.))
                    .pl(px(8.))
                    .flex()
                    .items_center()
                    .child(div().flex_1().text_color(muted).child("Installed"))
                    .child(
                        action(theme, "ext-folder", "Open folder")
                            .gap(px(6.))
                            .child(gpui::svg().path("icons/solar/folder-open.svg").size(px(14.)).text_color(muted))
                            .on_click(|_, _, cx| {
                                let root = ext::root();
                                match std::fs::create_dir_all(&root) {
                                    Ok(()) => cx.open_with_system(&root),
                                    Err(e) => crate::ui::toast::error(cx, format!("{}: {e}", root.display())),
                                }
                            }),
                    ),
            )
            .child(card(theme).mt(px(0.)).children(rows).when(empty, |el| {
                el.child(card_row(theme, true).child(row_text(
                    theme,
                    "No extensions yet",
                    "Install one from GitHub, or with its own `install`. Each is a page in the sidebar and a CLI for agents.",
                )))
            }))
            .children(self.ext_note.clone().map(|n| note(theme, n)))
            .child(card(theme).mt(px(12.)).child(install))
            .children(self.install_note.clone().map(|n| note(theme, n)))
    }
}
