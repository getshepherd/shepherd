//! A first launch's welcome card, Open a project…, and what the pane grid shows with nothing in it: no backend to
//! reach, or a session with no workspaces.
use super::*;

/// The keys the welcome card teaches, as the user's bindings have them.
const TAUGHT: [(&str, &str); 4] = [
    ("CommandPanel", "Every command, searchable"),
    ("NewTab", "A new tab in this workspace"),
    ("SplitRight", "Split the pane"),
    ("AgentsPanel", "Your agents, and who is waiting"),
];

/// Once, on a launch with no settings file yet; closing it writes the file, so it doesn't come back.
pub fn welcome(cx: &mut App) {
    let chords = cx.try_global::<ActiveBindings>().map(|b| b.0.clone()).unwrap_or_default();
    let keys: Vec<(String, &'static str)> = TAUGHT
        .iter()
        .filter_map(|(action, what)| Some((crate::ui::settings::display_chord(&chords.iter().find(|(a, _)| a == action)?.1), *what)))
        .collect();
    crate::ui::toast::sticky(cx, "welcome", Rc::new(move |theme, close| card(theme, &keys, close)));
}

fn card(theme: &Theme, keys: &[(String, &'static str)], close: Rc<dyn Fn(&mut App)>) -> gpui::AnyElement {
    let done = move |open: bool| {
        let close = close.clone();
        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
            save_from_sidebar(cx, |s| s.welcomed = true);
            close(cx);
            if open {
                window.dispatch_action(Box::new(OpenProject), cx);
            }
        }
    };
    let rows = keys.iter().map(|(chord, what)| {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(div().w(px(64.)).flex_none().text_size(px(12.)).text_color(theme.text_muted).child(chord.clone()))
            .child(div().text_color(theme.text).child(*what))
    });
    div()
        .w(px(340.))
        .p(px(14.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .rounded(px(10.))
        .bg(theme.card)
        .border_1()
        .border_color(theme.border)
        .shadow_lg()
        .text_size(px(13.))
        .text_color(theme.text)
        .child(div().text_size(px(14.)).font_weight(gpui::FontWeight::MEDIUM).child("Welcome to Shepherd"))
        .child(div().text_color(theme.text_muted).child("A workspace is a folder with its tabs, panes and agents. Open a project to start one."))
        .child(div().py(px(2.)).flex().flex_col().gap(px(4.)).children(rows))
        .child(
            div()
                .mt(px(4.))
                .flex()
                .justify_end()
                .gap(px(4.))
                .child(ghost_button(theme, "welcome-dismiss", "Got it").text_color(theme.text).on_click(done(false)))
                .child(solid_button("welcome-open", "Open a project…", theme.accent, gpui::white()).on_click(done(true))),
        )
        .into_any_element()
}

impl Shell {
    /// Open a project…: a folder picked becomes a new workspace there. On a herdr machine the Mac's folders mean
    /// nothing, so it makes a workspace in the machine's home instead.
    pub(super) fn open_project(&mut self, cx: &mut Context<Self>) {
        if SettingsGlobal::of(cx).settings.herdr_machine.is_some() {
            return self.new_workspace(cx);
        }
        let picked = cx.prompt_for_paths(gpui::PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Open".into()) });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else { return };
            let Some(dir) = paths.into_iter().next() else { return };
            let _ = this.update(cx, |shell, cx| {
                shell.send(Command::NewWorkspace { cwd: Some(dir.to_string_lossy().into_owned()) }, cx, |shell, result, cx| {
                    shell.select_tab(result.tab_id.clone());
                    cx.notify();
                })
            });
        })
        .detach();
    }
}

/// The pane grid's middle while the backend can't be reached: why, and where to look.
pub(super) fn unreachable(theme: &Theme, why: &str, place: String) -> gpui::AnyElement {
    let (theme_text, muted) = (theme.text, theme.text_muted);
    let settings = |_: &ClickEvent, window: &mut Window, cx: &mut App| window.dispatch_action(Box::new(OpenSettings), cx);
    let log = |_: &ClickEvent, window: &mut Window, cx: &mut App| window.dispatch_action(Box::new(RevealLogs), cx);
    middle()
        .child(gpui::svg().path("icons/solar/plug-circle.svg").size(px(28.)).text_color(muted))
        .child(div().text_size(px(14.)).font_weight(gpui::FontWeight::MEDIUM).text_color(theme_text).child(format!("Can't reach {place}")))
        .child(div().max_w(px(420.)).text_size(px(12.)).text_color(muted).text_center().child(why.to_string()))
        .child(div().text_size(px(12.)).text_color(theme.text_faint).child("Shepherd keeps trying every second."))
        .child(
            div()
                .mt(px(4.))
                .flex()
                .gap(px(4.))
                .child(ghost_button(theme, "unreachable-log", "Reveal Log").on_click(log))
                .child(solid_button("unreachable-settings", "Settings…", theme.accent, gpui::white()).on_click(settings)),
        )
        .into_any_element()
}

/// The pane grid's middle in a session with no workspaces.
pub(super) fn no_workspaces(theme: &Theme) -> gpui::AnyElement {
    let new = |_: &ClickEvent, window: &mut Window, cx: &mut App| window.dispatch_action(Box::new(NewWorkspace), cx);
    let open = |_: &ClickEvent, window: &mut Window, cx: &mut App| window.dispatch_action(Box::new(OpenProject), cx);
    middle()
        .child(div().text_size(px(14.)).font_weight(gpui::FontWeight::MEDIUM).text_color(theme.text).child("No workspaces"))
        .child(div().text_size(px(12.)).text_color(theme.text_muted).child("Open a project folder, or start an empty workspace in your home folder."))
        .child(
            div()
                .mt(px(4.))
                .flex()
                .gap(px(4.))
                .child(ghost_button(theme, "empty-new", "New Workspace").on_click(new))
                .child(solid_button("empty-open", "Open a project…", theme.accent, gpui::white()).on_click(open)),
        )
        .into_any_element()
}

fn middle() -> Div {
    div().absolute().inset_0().p_4().flex().flex_col().items_center().justify_center().gap(px(8.))
}
