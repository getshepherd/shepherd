//! Settings → Extensions, above the pages added by hand: the extensions installed (`crate::extensions`), whether each
//! runs always or with Shepherd, starting and stopping its server, removing one, their folder, and installing one from a GitHub release.
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

/// The two ways an extension runs, as its state's `start` and the select's label.
const STARTS: [(&str, &str); 2] = [("shepherd", "While Shepherd is open"), ("login", "Always")];

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

    /// Starts (`up`) or stops its server, Shepherd's own `serve` or the system's service.
    fn power(&mut self, e: Extension, up: bool, cx: &mut Context<Self>) {
        (self.ext_busy, self.ext_note) = (Some((e.name.clone(), if up { "Starting…" } else { "Stopping…" })), None);
        self.ext_job(
            move || if up { ext::start_server(&e) } else { ext::stop_server(&e) },
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

    /// One installed extension's row: its name with a dot that says whether it answers, its description, when it
    /// runs as a select, Start or Stop, and the bin; or, once the bin is pressed, the question whether to remove it.
    fn extension_row(&self, i: usize, e: Extension, theme: &Theme, cx: &mut Context<Self>) -> Div {
        if self.ext_remove.as_ref() == Some(&e.name) {
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
                .child(filled("ext-remove-yes", "Remove", theme.danger, gpui::white()).on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.remove_extension(e.clone(), cx))));
        }
        let busy = self.ext_busy.as_ref().filter(|(name, _)| *name == e.name).map(|(_, doing)| *doing);
        let (dot, state) = match busy {
            Some(doing) => (theme.warning, doing),
            None if e.alive => (theme.success, "Running"),
            None => (theme.text_faint, "Not running"),
        };
        let title = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().font_weight(FontWeight::MEDIUM).line_height(px(17.)).child(e.name.clone()))
            .child(div().size(px(6.)).rounded_full().bg(dot))
            .child(div().text_size(px(12.)).text_color(theme.text_muted).child(state));
        let description = if e.description.is_empty() { e.binary.display().to_string() } else { e.description.clone() };
        let text = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(title)
            .child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme.text_muted).child(description));
        let current = STARTS.iter().position(|(start, _)| *start == e.start);
        let view = cx.entity().downgrade();
        let pick = {
            let e = e.clone();
            move |i: usize, cx: &mut App| {
                let start = STARTS[i].0;
                if start != e.start && busy.is_none() {
                    let e = e.clone();
                    let _ = view.update(cx, |v, cx| v.switch_start(e, start, cx));
                }
            }
        };
        let labels = STARTS.iter().map(|(_, label)| SharedString::from(*label)).collect();
        let runs = self.select(theme, web_select_id("ext-start", &e.name), labels, current, e.start.clone().into(), |_| None, pick, cx);
        let power = {
            let (label, up) = if e.alive { ("Stop", false) } else { ("Start", true) };
            let e = e.clone();
            action(theme, web_select_id("ext-power", &e.name), label)
                .when(busy.is_none(), |el| el.on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.power(e.clone(), up, cx))))
        };
        let name = e.name.clone();
        let remove = remove_button(theme, format!("ext-remove-{name}")).mt(px(0.)).when(busy.is_none(), |el| {
            el.on_click(cx.listener(move |v, _: &ClickEvent, _, cx| {
                v.ext_remove = Some(name.clone());
                cx.notify();
            }))
        });
        card_row(theme, i == 0).child(text).child(power).child(runs.w(px(208.))).child(remove)
    }

    /// Install from GitHub's card: the repo and Install, or the question whether to run the build found.
    fn install_card(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let row = match &self.install {
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
                    .child(filled("ext-install-yes", "Install", theme.accent, theme.on_accent).on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.install_release(release.clone(), cx))))
            }
            install => {
                let button = match install {
                    Install::Finding => action(theme, "ext-install", "Looking…"),
                    Install::Installing => action(theme, "ext-install", "Installing…"),
                    _ => action(theme, "ext-install", "Install").on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.find_release(cx))),
                };
                card_row(theme, true).min_h(px(0.)).child(field(theme, &self.ext_repo).flex_1()).child(button)
            }
        };
        let hint = format!(
            "A repo's latest release, its build for this computer (a file ending in {}). You confirm before it runs.",
            ext::asset_suffix()
        );
        div().child(card(theme).mt(px(0.)).child(row)).child(note(theme, self.install_note.clone().unwrap_or(hint)))
    }

    /// The installed extensions with Open folder by their label, then installing one from GitHub.
    pub(super) fn extensions(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let muted = theme.text_muted;
        let list = cx.try_global::<Extensions>().map(|e| e.0.clone()).unwrap_or_default();
        let empty = list.is_empty();
        let rows: Vec<Div> = list.into_iter().enumerate().map(|(i, e)| self.extension_row(i, e, theme, cx)).collect();
        // what the select's two choices mean, said once under the list
        let runs = "While Shepherd is open: Shepherd starts it and stops it when it quits. Always: the system runs it from login, \
             so its CLI works with Shepherd closed.";
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
                    "Install one from GitHub below. Each is a page in the sidebar and a CLI for agents.",
                )))
            }))
            .children(self.ext_note.clone().or_else(|| (!empty).then(|| runs.to_string())).map(|n| note(theme, n)))
            .child(section_label(theme, "Install from GitHub"))
            .child(self.install_card(theme, cx))
    }
}
