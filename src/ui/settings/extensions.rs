//! Settings → Extensions, laid out as Zeron's Plugins: a tab of the extensions installed (`crate::extensions`), each
//! a row with its version, an update, a switch that runs it, the bin and a dialog of its settings (whether it runs
//! always or with Shepherd, its folder), and a tab of the pages added by hand; Install opens a dialog for a GitHub
//! release.
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
pub(super) fn filled(id: &'static str, label: &'static str, bg: gpui::Hsla, fg: gpui::Hsla) -> Stateful<Div> {
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

/// Extensions' tabs.
#[derive(Default, Clone, Copy, PartialEq)]
pub(super) enum Tab {
    #[default]
    Installed,
    Pages,
    /// Chrome extensions in the browser tabs.
    Browser,
}

/// A dialog's card: Zeron's, 460 px, on the dialog surface.
fn dialog_card(theme: &Theme, id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .occlude()
        .w(px(460.))
        .p(px(24.))
        .rounded(px(16.))
        .bg(theme.dialog)
        .border_1()
        .border_color(theme.border)
        .text_color(theme.text)
        .text_size(px(13.))
        .flex()
        .flex_col()
}

/// `card` centered over the window on a 35 % scrim, both fading in and out with `popup`.
fn modal<T>(name: &'static str, popup: &crate::ui::Popup<T>, card: Stateful<Div>, window: &Window) -> impl IntoElement + use<T> {
    let viewport = window.viewport_size();
    let scrim = div()
        .id(SharedString::from(format!("{name}-scrim")))
        .occlude()
        .w(viewport.width)
        .h(viewport.height)
        .bg(gpui::black().opacity(0.35))
        .flex()
        .justify_center()
        .items_center()
        .child(crate::ui::dialog_motion(format!("{name}-in"), popup, div().id(SharedString::from(format!("{name}-motion"))).child(card), 2.));
    gpui::deferred(gpui::anchored().position(gpui::point(px(0.), px(0.))).child(crate::ui::dialog_motion(format!("{name}-scrim-in"), popup, scrim, 0.))).with_priority(2)
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

    /// Asks GitHub for the latest release of each extension installed from it, off the main thread.
    pub(super) fn check_updates(&mut self, cx: &mut Context<Self>) {
        let list: Vec<Extension> = cx.try_global::<Extensions>().map(|e| e.0.clone()).unwrap_or_default();
        // the first scan may not be in yet: asked again on the next draw
        if list.is_empty() {
            return;
        }
        self.ext_updates = Some(Default::default());
        for e in list.into_iter().filter(|e| e.source.is_some()) {
            let name = e.name.clone();
            self.ext_job(
                move || ext::update_for(&e),
                move |v, found| {
                    v.ext_updates.get_or_insert_default().insert(name, found);
                },
                cx,
            );
        }
    }

    /// Installs `release` over `e`, keeping when it runs.
    fn update_extension(&mut self, e: Extension, release: Release, cx: &mut Context<Self>) {
        (self.ext_busy, self.ext_note) = (Some((e.name.clone(), "Updating…")), None);
        let (name, tag, start) = (e.name.clone(), release.tag.clone(), e.start.clone());
        self.ext_job(
            move || ext::install(&release, &start),
            move |v, result| {
                v.ext_busy = None;
                match result {
                    Ok(()) => {
                        v.ext_updates.get_or_insert_default().insert(name.clone(), Ok(None));
                        v.ext_note = Some(format!("Updated {name} to {tag}."));
                    }
                    Err(why) => v.ext_note = Some(format!("{name} didn't update: {why}")),
                }
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
            move || ext::install(&release, "shepherd"),
            move |v, result| {
                v.install = Install::Idle;
                match result {
                    Ok(()) => {
                        v.ext_note = Some(format!("Installed {what}."));
                        v.ext_tab = Tab::Installed;
                        if let Some(d) = v.install_dialog.as_mut() {
                            d.close()
                        }
                    }
                    Err(why) => v.install_note = Some(format!("{what} didn't install: {why}")),
                }
            },
            cx,
        );
    }

    /// Reads each extension's page icon the sidebar kept (`crate::web::Seen`), once a Settings: a small file each.
    pub(super) fn load_ext_icons(&mut self, cx: &App) {
        let seen = crate::web::Seen::default();
        for e in cx.try_global::<Extensions>().map(|e| e.0.clone()).unwrap_or_default() {
            self.ext_icons.entry(e.name.clone()).or_insert_with(|| {
                let entry = e.entry();
                seen.icon(&entry.id, &entry.url).map(|(format, bytes)| std::sync::Arc::new(gpui::Image::from_bytes(format, bytes)))
            });
        }
    }

    pub(super) fn open_install(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.install_dialog = Some(crate::ui::Popup::new(()));
        if !matches!(self.install, Install::Installing) {
            (self.install, self.install_note) = (Install::Idle, None);
        }
        let input = if self.ext_tab == Tab::Browser { &self.bext_input } else { &self.ext_repo };
        input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    pub(super) fn close_install(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = self.install_dialog.as_mut() {
            d.close()
        }
        if !matches!(self.install, Install::Installing) {
            (self.install, self.install_note) = (Install::Idle, None);
        }
        cx.notify();
    }

    /// Its icon: the page's favicon as the sidebar kept it, else its initial on a tile.
    fn ext_icon(&self, e: &Extension, theme: &Theme) -> Div {
        let tile = div().flex_none().size(px(40.)).rounded(px(10.)).flex().items_center().justify_center();
        match self.ext_icons.get(&e.name).cloned().flatten() {
            Some(image) => tile.child(gpui::img(image).size(px(40.)).rounded(px(10.))),
            None => tile
                .bg(theme.accent.opacity(0.85))
                .text_color(theme.on_accent)
                .text_size(px(17.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(e.title().chars().next().map(String::from).unwrap_or_default()),
        }
    }

    /// One installed extension, Zeron's plugin row: its icon; its name, where it came from and what it does; an
    /// update when there is one, its version, its settings (a dialog), a switch that runs it, and the bin. Once the
    /// bin is pressed, the row asks whether to remove it.
    fn extension_row(&self, i: usize, e: Extension, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let row = div().mx(px(16.)).py(px(14.)).when(i > 0, |el| el.border_t_1().border_color(theme.border.opacity(0.6)));
        if self.ext_remove.as_ref() == Some(&e.name) {
            return row.flex().items_center().gap(px(14.)).child(self.ext_icon(&e, theme)).child(row_text(
                theme,
                format!("Remove {}?", e.title()),
                "Its service, its skill and its binary go. Its data stays in its folder, for when it is installed again.",
            ))
            .child(action(theme, "ext-remove-cancel", "Cancel").on_click(cx.listener(|v, _: &ClickEvent, _, cx| {
                v.ext_remove = None;
                cx.notify();
            })))
            .child(filled("ext-remove-yes", "Remove", theme.danger, gpui::white()).on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.remove_extension(e.clone(), cx))));
        }
        let busy = self.ext_busy.as_ref().filter(|(name, _)| *name == e.name).map(|(_, doing)| *doing);
        let from = match &e.source {
            Some(s) => s.repo.clone(),
            None => "Installed by hand".into(),
        };
        let state = busy.or((!e.alive).then_some("Not running"));
        let description = if e.description.is_empty() { e.binary.display().to_string() } else { e.description.clone() };
        let line = |text: String, color| div().text_size(px(12.5)).line_height(px(17.)).text_color(color).whitespace_nowrap().overflow_hidden().text_ellipsis().child(text);
        let text = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(div().font_weight(FontWeight::MEDIUM).line_height(px(18.)).child(e.title()))
            .child(line(state.map_or(from.clone(), |s| format!("{from} · {s}")), theme.text_muted))
            .child(line(description, theme.text_faint));
        let update = match self.ext_updates.as_ref().and_then(|u| u.get(&e.name)) {
            Some(Ok(Some(release))) if busy.is_none() => {
                let (e, release) = (e.clone(), release.clone());
                Some(
                    action(theme, web_select_id("ext-update", &e.name), format!("Update to {}", release.tag))
                        .text_color(theme.accent)
                        .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.update_extension(e.clone(), release.clone(), cx))),
                )
            }
            _ => None,
        };
        let version = e.source.as_ref().map(|s| {
            div().flex_none().px(px(6.)).py(px(1.)).rounded(px(5.)).bg(theme.wash(0.06)).text_size(px(12.)).text_color(theme.text_muted).child(s.tag.clone())
        });
        let open = self.ext_settings.as_ref().is_some_and(|d| d.is_open() && d.value == e.name);
        let icon_button = |id: &'static str, icon: &'static str, tone: gpui::Hsla, hover: gpui::Hsla| {
            div()
                .id(id)
                .flex_none()
                .size(px(30.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(gpui::svg().path(icon).size(px(16.)).text_color(tone))
        };
        let settings = {
            let name = e.name.clone();
            icon_button(web_select_id("ext-settings", &e.name), "icons/solar/tuning-2.svg", if open { theme.text } else { theme.text_muted }, theme.wash(0.08))
                .when(open, |el| el.bg(theme.wash(0.08)))
                .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| {
                    v.ext_settings = Some(crate::ui::Popup::new(name.clone()));
                    cx.notify();
                }))
        };
        let power = {
            let (e, up) = (e.clone(), !e.alive);
            div()
                .id(web_select_id("ext-power", &e.name))
                .flex_none()
                .cursor_pointer()
                .child(crate::ui::switch(theme, e.alive, format!("ext-power-{}", e.name)))
                .when(busy.is_none(), |el| el.on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.power(e.clone(), up, cx))))
        };
        let remove = {
            let name = e.name.clone();
            icon_button(web_select_id("ext-remove", &e.name), "icons/solar/trash-bin-minimalistic.svg", theme.text_muted, theme.danger.opacity(0.12)).when(busy.is_none(), |el| {
                el.on_click(cx.listener(move |v, _: &ClickEvent, _, cx| {
                    v.ext_remove = Some(name.clone());
                    cx.notify();
                }))
            })
        };
        let main = div()
            .flex()
            .items_center()
            .gap(px(14.))
            .child(self.ext_icon(&e, theme))
            .child(text)
            .child(div().flex_none().flex().items_center().gap(px(8.)).children(update).children(version).child(settings).child(power).child(remove));
        row.child(main)
    }

    /// An extension's settings dialog: who it is, when it runs (each way with what it means), and its folder.
    pub(super) fn settings_overlay(&self, popup: &crate::ui::Popup<String>, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let e = cx.try_global::<Extensions>()?.0.iter().find(|e| e.name == popup.value)?.clone();
        let busy = self.ext_busy.as_ref().filter(|(name, _)| *name == e.name).map(|(_, doing)| *doing);
        let from = e.source.as_ref().map_or("Installed by hand".to_string(), |s| format!("{} · {}", s.repo, s.tag));
        let caption = |label: &'static str| div().mt(px(20.)).mb(px(8.)).text_size(px(12.)).text_color(theme.text_muted).child(label);
        let meanings = ["Shepherd starts it, and stops it when it quits.", "The system runs it from login, so its CLI works with Shepherd closed."];
        let options = STARTS.iter().zip(meanings).enumerate().map(|(i, ((start, label), meaning))| {
            let (e, start) = (e.clone(), *start);
            let chosen = e.start == start;
            let hover = theme.wash(0.04);
            div()
                .id(web_select_id(&format!("ext-start-{start}"), &e.name))
                .px(px(14.))
                .py(px(10.))
                .flex()
                .items_center()
                .gap(px(12.))
                .when(i > 0, |el| el.border_t_1().border_color(theme.border.opacity(0.6)))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(row_text(theme, *label, meaning))
                .child(
                    div()
                        .flex_none()
                        .size(px(16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(chosen, |el| el.child(gpui::svg().path("icons/check.svg").size(px(14.)).text_color(theme.accent))),
                )
                .when(busy.is_none() && !chosen, |el| el.on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.switch_start(e.clone(), start, cx))))
        });
        let folder = e.binary.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
        // as a person writes it: the home folder as ~
        let home = crate::settings::home();
        let path = folder.strip_prefix(&home).map_or_else(|_| folder.display().to_string(), |rest| format!("~/{}", rest.display()));
        let card = dialog_card(theme, "ext-settings-dialog")
            .on_mouse_down_out(cx.listener(|v, _, _, cx| {
                if let Some(d) = v.ext_settings.as_mut() {
                    d.close()
                }
                cx.notify();
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .child(self.ext_icon(&e, theme))
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(div().text_size(px(17.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).child(e.title()))
                            .child(div().text_color(theme.text_muted).child(from)),
                    ),
            )
            .child(caption("Runs"))
            .child(div().rounded(px(10.)).bg(theme.wash(0.045)).overflow_hidden().flex().flex_col().children(options))
            .children(busy.map(|doing| div().mt(px(6.)).text_size(px(12.)).text_color(theme.text_muted).child(doing)))
            .child(caption("Folder"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().flex_1().min_w_0().text_size(px(12.)).text_color(theme.text_muted).whitespace_nowrap().overflow_hidden().text_ellipsis().child(path))
                    .child(
                        action(theme, "ext-settings-folder", "Show in Finder")
                            .gap(px(6.))
                            .child(gpui::svg().path("icons/solar/folder-open.svg").size(px(14.)).text_color(theme.text_muted))
                            .on_click(move |_, _, cx| cx.open_with_system(&folder)),
                    ),
            )
            .child(
                div().mt(px(24.)).flex().justify_end().child(filled("ext-settings-done", "Done", theme.accent, theme.on_accent).on_click(cx.listener(|v, _: &ClickEvent, _, cx| {
                    if let Some(d) = v.ext_settings.as_mut() {
                        d.close()
                    }
                    cx.notify();
                }))),
            );
        Some(modal("ext-settings", popup, card, window))
    }

    /// The Install dialog over the window: a repo to look up, then the build found to confirm.
    pub(super) fn install_overlay(&self, popup: &crate::ui::Popup<()>, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.ext_tab == Tab::Browser {
            return modal("install", popup, self.browser_install_card(theme, cx), window);
        }
        let (title, body): (String, String) = match &self.install {
            Install::Confirm(r) => (
                format!("Install {} {}?", r.repo, r.tag),
                format!("{} is downloaded and run on this computer, with your rights. Install only what you trust.", r.asset),
            ),
            _ => (
                "Install an extension".into(),
                format!(
                    "Paste a GitHub repository, as owner/name or its link. Shepherd takes its latest release built for this computer \
                     (a file ending in {}), and you confirm before it runs.",
                    ext::asset_suffix()
                ),
            ),
        };
        let confirm = matches!(self.install, Install::Confirm(_));
        let go = match &self.install {
            Install::Finding => filled("ext-install-go", "Looking…", theme.accent.opacity(0.6), theme.on_accent),
            Install::Installing => filled("ext-install-go", "Installing…", theme.accent.opacity(0.6), theme.on_accent),
            Install::Confirm(r) => {
                let r = r.clone();
                filled("ext-install-go", "Install", theme.accent, theme.on_accent).on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.install_release(r.clone(), cx)))
            }
            Install::Idle => filled("ext-install-go", "Continue", theme.accent, theme.on_accent).on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.find_release(cx))),
        };
        let card = dialog_card(theme, "install-dialog")
            .gap(px(8.))
            .on_mouse_down_out(cx.listener(|v, _, _, cx| v.close_install(cx)))
            .child(div().text_size(px(17.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).child(title))
            .child(div().line_height(px(19.)).text_color(theme.text_muted).child(body))
            .when(!confirm, |el| el.child(div().mt(px(12.)).child(labelled(theme, "Repository", field(theme, &self.ext_repo).w_full()))))
            .children(self.install_note.clone().map(|n| div().text_size(px(12.)).line_height(px(16.)).text_color(theme.danger).child(n)))
            .child(
                div()
                    .mt(px(16.))
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(action(theme, "ext-install-cancel", "Cancel").on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.close_install(cx))))
                    .child(go),
            );
        modal("install", popup, card, window)
    }

    /// The Install dialog of the Browser tab: a Web Store link or id to download, or a folder to load from.
    fn browser_install_card(&self, theme: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        let go = match self.bext_busy {
            true => filled("bext-install-go", "Installing…", theme.accent.opacity(0.6), theme.on_accent),
            false => filled("bext-install-go", "Install", theme.accent, theme.on_accent).on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.add_from_store(cx))),
        };
        dialog_card(theme, "install-dialog")
            .gap(px(8.))
            .on_mouse_down_out(cx.listener(|v, _, _, cx| v.close_install(cx)))
            .child(div().text_size(px(17.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).child("Install a browser extension"))
            .child(div().line_height(px(19.)).text_color(theme.text_muted).child(
                "Paste its Chrome Web Store link, or its id. It runs in every browser tab with all it asks for, so install only what you trust.",
            ))
            .child(div().mt(px(12.)).child(labelled(theme, "Chrome Web Store", field(theme, &self.bext_input).w_full())))
            .children(self.install_note.clone().map(|n| div().text_size(px(12.)).line_height(px(16.)).text_color(theme.danger).child(n)))
            .child(
                div()
                    .mt(px(16.))
                    .flex()
                    .gap(px(8.))
                    .child(action(theme, "bext-folder", "Choose Folder…").on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.add_folder(cx))))
                    .child(div().flex_1())
                    .child(action(theme, "ext-install-cancel", "Cancel").on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.close_install(cx))))
                    .child(go),
            )
    }

    /// Zeron's tab: its label and count, underlined when shown.
    fn ext_tab(&self, theme: &Theme, tab: Tab, label: &'static str, count: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let shown = self.ext_tab == tab;
        div()
            .id(label)
            .mr(px(24.))
            .pb(px(10.))
            .mb(px(-1.))
            .flex()
            .items_center()
            .gap(px(7.))
            .border_b_2()
            .border_color(if shown { theme.text } else { gpui::transparent_black() })
            .cursor_pointer()
            .child(div().when(shown, |el| el.font_weight(FontWeight::MEDIUM)).text_color(if shown { theme.text } else { theme.text_muted }).child(label))
            .child(div().text_size(px(12.)).text_color(theme.text_faint).child(count.to_string()))
            .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| {
                v.ext_tab = tab;
                cx.notify();
            }))
    }

    /// The tabs, Installed and Web pages, with the shown one's action at their right (Install, Add page), over its
    /// card.
    pub(super) fn extensions(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let list = cx.try_global::<Extensions>().map(|e| e.0.clone()).unwrap_or_default();
        let pages = SettingsGlobal::of(cx).settings.web_entries.len();
        let browser = SettingsGlobal::of(cx).settings.browser_extensions.len();
        let muted = theme.text_muted;
        let (hover, text) = (theme.wash(0.06), theme.text);
        let (label, icon) = match self.ext_tab {
            Tab::Installed => ("Install", "icons/solar/download-minimalistic.svg"),
            Tab::Pages => ("Add page", "icons/solar/add-circle.svg"),
            Tab::Browser => ("Install", "icons/solar/download-minimalistic.svg"),
        };
        let add = div()
            .id("ext-add")
            // its label level with the tabs'
            .mb(px(8.))
            .h(px(28.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(7.))
            .text_color(muted)
            .cursor_pointer()
            .hover(move |s| s.bg(hover).text_color(text))
            .child(gpui::svg().path(icon).size(px(14.)).text_color(muted))
            .child(label)
            .on_click(cx.listener(|v, _: &ClickEvent, window, cx| match v.ext_tab {
                Tab::Installed | Tab::Browser => v.open_install(window, cx),
                Tab::Pages => update(cx, |s| s.web_entries.push(WebEntry::new())),
            }));
        let tabs = div()
            .mt(px(28.))
            .flex()
            .items_end()
            .border_b_1()
            .border_color(theme.border.opacity(0.6))
            .child(self.ext_tab(theme, Tab::Installed, "Installed", list.len(), cx))
            .child(self.ext_tab(theme, Tab::Pages, "Web pages", pages, cx))
            .child(self.ext_tab(theme, Tab::Browser, "Browser", browser, cx))
            .child(div().flex_1())
            .when(self.ext_tab != Tab::Browser || crate::browser_extensions::unavailable().is_none(), |el| el.child(add));
        let body = match self.ext_tab {
            Tab::Pages => self.web_pages(theme, cx),
            Tab::Browser => self.browser_extensions(theme, cx),
            Tab::Installed => {
                let empty = list.is_empty();
                let rows: Vec<Div> = list.into_iter().enumerate().map(|(i, e)| self.extension_row(i, e, theme, cx)).collect();
                div()
                    .child(card(theme).mt(px(16.)).children(rows).when(empty, |el| {
                        el.child(card_row(theme, true).child(row_text(
                            theme,
                            "No extensions yet",
                            "Install one from GitHub: each is a page in the sidebar and a CLI for agents.",
                        )))
                    }))
                    .children(self.ext_note.clone().map(|n| note(theme, n)))
            }
        };
        div().child(tabs).child(body)
    }
}
