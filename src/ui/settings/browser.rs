//! Settings → Extensions → Browser: the Chrome extensions in the browser tabs (`crate::browser_extensions`), each a row
//! with its icon, name and version, a switch and the bin; the Install dialog takes a Web Store link or id, or a folder.
use super::*;
use crate::browser_extensions as bext;
use crate::settings::BrowserExtension;
use std::path::PathBuf;
use std::sync::Arc;

/// An extension's manifest as read for its row, with its icon ready to draw; or why it couldn't be read.
pub(super) type Shown = Result<(bext::Info, Option<Arc<gpui::Image>>), String>;

fn adds(cx: &mut App, path: PathBuf) {
    update(cx, |s| {
        s.browser_extensions.retain(|e| e.path != path);
        s.browser_extensions.push(BrowserExtension { path, on: true });
    });
}

impl SettingsView {
    /// Reads the manifest of each extension not read yet: small files, once a Settings (and again after an install).
    pub(super) fn load_browser_info(&mut self, cx: &App) {
        for e in &SettingsGlobal::of(cx).settings.browser_extensions {
            self.bext_info.entry(e.path.clone()).or_insert_with(|| {
                bext::info(&e.path).map(|info| {
                    let icon = info.icon.clone().map(|png| Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, png)));
                    (info, icon)
                })
            });
        }
    }

    /// The Install dialog's Continue on the Browser tab: downloads the extension the field names from the Web Store.
    pub(super) fn add_from_store(&mut self, cx: &mut Context<Self>) {
        if self.bext_busy {
            return;
        }
        let Some(id) = bext::store_id(&self.bext_input.read(cx).value()) else {
            self.install_note = Some("Paste a Chrome Web Store link, or the extension's id (32 letters).".into());
            return cx.notify();
        };
        (self.bext_busy, self.install_note) = (true, None);
        let task = cx.background_spawn(async move { bext::install_from_store(&id) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |v, cx| {
                v.bext_busy = false;
                match result {
                    Ok(path) => v.added(path, cx),
                    Err(why) => v.install_note = Some(why),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Choose folder: an unpacked extension (a folder with its `manifest.json`), loaded from where it is.
    pub(super) fn add_folder(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(gpui::PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Add Extension".into()) });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let _ = this.update(cx, |v, cx| {
                match bext::info(&path) {
                    Ok(_) => v.added(path, cx),
                    Err(why) => v.install_note = Some(why),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn added(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.bext_info.remove(&path);
        let name = bext::info(&path).map(|i| i.name).unwrap_or_default();
        adds(cx, path);
        self.ext_note = Some(format!("Added {name}. It runs in browser tabs from their next load."));
        if let Some(d) = self.install_dialog.as_mut() {
            d.close()
        }
    }

    /// One extension: its icon, name, version and what it does (or why it didn't load), its switch and the bin.
    fn browser_row(&self, i: usize, e: &BrowserExtension, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let shown = self.bext_info.get(&e.path);
        let (name, version, description, icon) = match shown {
            Some(Ok((info, icon))) => (info.name.clone(), info.version.clone(), info.description.clone(), icon.clone()),
            Some(Err(why)) => (e.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), String::new(), why.clone(), None),
            None => (String::new(), String::new(), String::new(), None),
        };
        let tile = div().flex_none().size(px(40.)).rounded(px(10.)).flex().items_center().justify_center();
        let icon = match icon {
            Some(image) => tile.child(gpui::img(image).size(px(32.))),
            None => tile.bg(theme.accent.opacity(0.85)).text_color(theme.on_accent).text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).child(name.chars().next().map(String::from).unwrap_or_default()),
        };
        let key = e.path.to_string_lossy().to_string();
        let row = div().mx(px(16.)).py(px(14.)).when(i > 0, |el| el.border_t_1().border_color(theme.border.opacity(0.6))).flex().items_center().gap(px(14.)).child(icon);
        if self.bext_remove.as_ref() == Some(&e.path) {
            let path = e.path.clone();
            let body = if path.starts_with(bext::dir()) { "Its files go with it." } else { "Its folder stays where it is." };
            return row
                .child(row_text(theme, format!("Remove {name}?"), body))
                .child(action(theme, "bext-remove-cancel", "Cancel").on_click(cx.listener(|v, _: &ClickEvent, _, cx| {
                    v.bext_remove = None;
                    cx.notify();
                })))
                .child(super::extensions::filled("bext-remove-yes", "Remove", theme.danger, gpui::white()).on_click(cx.listener(move |v, _: &ClickEvent, _, cx| {
                    v.bext_remove = None;
                    let path = path.clone();
                    update(cx, |s| s.browser_extensions.retain(|e| e.path != path));
                    bext::remove_files(&path);
                    cx.notify();
                })));
        }
        let error = e.on.then(|| bext::error(&e.path)).flatten();
        let line = |text: String, color| div().text_size(px(12.5)).line_height(px(17.)).text_color(color).whitespace_nowrap().overflow_hidden().text_ellipsis().child(text);
        let home = crate::settings::home();
        let from = if e.path.starts_with(bext::dir()) {
            "Chrome Web Store".to_string()
        } else {
            e.path.strip_prefix(&home).map_or_else(|_| e.path.display().to_string(), |rest| format!("~/{}", rest.display()))
        };
        let text = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(div().font_weight(FontWeight::MEDIUM).line_height(px(18.)).child(name))
            .child(line(from, theme.text_muted))
            .child(match error {
                Some(why) => line(format!("Didn't load: {why}"), theme.danger),
                None => line(description, theme.text_faint),
            });
        let version = (!version.is_empty()).then(|| {
            div().flex_none().px(px(6.)).py(px(1.)).rounded(px(5.)).bg(theme.wash(0.06)).text_size(px(12.)).text_color(theme.text_muted).child(version)
        });
        let power = {
            let path = e.path.clone();
            div().id(web_select_id("bext-power", &key)).flex_none().cursor_pointer().child(crate::ui::switch(theme, e.on, format!("bext-power-{key}"))).on_click(move |_, _, cx| {
                update(cx, |s| {
                    if let Some(e) = s.browser_extensions.iter_mut().find(|e| e.path == path) {
                        e.on = !e.on
                    }
                })
            })
        };
        let remove = {
            let path = e.path.clone();
            let hover = theme.danger.opacity(0.12);
            div()
                .id(web_select_id("bext-remove", &key))
                .flex_none()
                .size(px(30.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(gpui::svg().path("icons/solar/trash-bin-minimalistic.svg").size(px(16.)).text_color(theme.text_muted))
                .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| {
                    v.bext_remove = Some(path.clone());
                    cx.notify();
                }))
        };
        row.child(text).child(div().flex_none().flex().items_center().gap(px(8.)).children(version).child(power).child(remove))
    }

    /// The Browser tab's card: the extensions added, or why none can run here.
    pub(super) fn browser_extensions(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let list = SettingsGlobal::of(cx).settings.browser_extensions.clone();
        let rows: Vec<Div> = list.iter().enumerate().map(|(i, e)| self.browser_row(i, e, theme, cx)).collect();
        let empty = match bext::unavailable() {
            Some(why) => Some(("Browser extensions aren't available", why)),
            None => rows.is_empty().then_some(("No browser extensions yet", "Install one from the Chrome Web Store, or an unpacked one from a folder.")),
        };
        div()
            .child(card(theme).mt(px(16.)).children(rows).children(empty.map(|(title, body)| card_row(theme, list.is_empty()).child(row_text(theme, title, body)))))
            .children(self.ext_note.clone().map(|n| note(theme, n)))
    }
}
