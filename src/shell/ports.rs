//! A blank browser tab's body: the pages browser tabs showed last (Recently used), then what this Mac's dev servers
//! listen on, the shown workspace's first. A click loads the page or the port in the tab; Stop sends the process
//! SIGTERM.
use super::*;
use super::web::PageKey;
use crate::ports::{Listener, read, under};
use crate::ui::statusbar::{cached, fetching, refetch, short_path};
use gpui::AnyElement;

const KEY: &str = "ports";
/// How often `lsof` is asked while the list is shown.
const EVERY: Duration = Duration::from_secs(2);
/// How many of the latest pages Recently used shows.
const RECENT: usize = 6;

impl Shell {
    /// A blank browser tab in the opened panel, or one on `url`: `SHEPHERD_PANEL=browser[:url]` for a screenshot.
    pub fn open_blank_tab(&mut self, url: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.shown_tab() {
            self.new_browser_tab(tab.workspace_id, url, false, window, cx);
        }
    }

    /// Ports are read here, with this Mac's `lsof`: a session on another machine has its own.
    fn ports_offered(&self) -> bool {
        cfg!(unix) && self.machine_label.is_none()
    }

    /// Whether browser tab `key` shows its home (Recently used, the ports) in place of its page: nothing loaded in it yet.
    pub(super) fn shows_home(&self, key: &PageKey) -> bool {
        let url = self.pages.live.get(key).and_then(|p| super::web::page_url(&p.view));
        browser::is_tab(&key.0) && url.is_none_or(|u| u.is_empty() || u == browser::BLANK)
    }

    /// A blank tab's body: Recently used over the ports, in a column of reading width.
    pub(super) fn render_home(&mut self, key: &PageKey, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let recent = self.render_recent(key, theme, cx);
        let ports = self.ports_offered().then(|| self.render_ports(key, theme, window, cx));
        div()
            .id("browser-home")
            .flex_1()
            .min_h_0()
            .bg(theme.terminal.background)
            .overflow_y_scroll()
            .child(div().w_full().max_w(px(760.)).mx_auto().px(px(20.)).py(px(20.)).flex().flex_col().gap(px(28.)).children(recent).children(ports))
            .into_any_element()
    }

    /// The pages browser tabs showed last, as cards; nothing before the first.
    fn render_recent(&mut self, key: &PageKey, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let visits: Vec<crate::web::Visit> = self.history().visits.iter().take(RECENT).cloned().collect();
        if visits.is_empty() {
            return None;
        }
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let cards: Vec<AnyElement> = visits
            .into_iter()
            .enumerate()
            .map(|(i, visit)| {
                let site = browser::site(&visit.url);
                let icon = site.as_deref().and_then(|s| self.site_icon(s, cx));
                let host = visit.url.parse::<http::Uri>().ok().and_then(|u| u.authority().map(|a| a.to_string())).unwrap_or_else(|| visit.url.clone());
                recent_card(i, &visit, &host, icon, now, key, theme, cx)
            })
            .collect();
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(section_heading(theme, "icons/solar/history.svg", "Recently used"))
                // ponytail: two columns at any width; the panel is seldom narrower than two cards
                .child(div().grid().grid_cols(2).gap(px(8.)).children(cards))
                .into_any_element(),
        )
    }

    /// The directories the shown workspace's panes are in.
    fn workspace_dirs(&self) -> Vec<String> {
        let (Some(snapshot), Some(tab)) = (self.snapshot.as_ref(), self.shown_tab()) else { return Vec::new() };
        let panes = snapshot.panes.iter().filter(|p| p.workspace_id == tab.workspace_id);
        panes.filter_map(|p| p.foreground_cwd.clone().or_else(|| p.cwd.clone())).filter(|d| !d.is_empty()).collect()
    }

    fn render_ports(&mut self, key: &PageKey, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let listeners = cached(KEY, EVERY, read, cx);
        // asked again while shown: one timer at a time, and none once the list is gone
        if !self.browser.ports_tick {
            self.browser.ports_tick = true;
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(EVERY).await;
                let _ = this.update(cx, |shell, cx| {
                    shell.browser.ports_tick = false;
                    cx.notify();
                });
            })
            .detach();
        }
        let body: Vec<AnyElement> = match listeners {
            None if fetching::<Vec<Listener>>(KEY, cx) => vec![div().pt(px(12.)).flex().justify_center().child(crate::ui::loader::spinner(2.5, window)).into_any_element()],
            None => vec![note(theme, "lsof didn't run.")],
            Some(l) if l.is_empty() => vec![note(theme, "Nothing started from a project folder listens on a TCP port.")],
            Some(listeners) => {
                let dirs = self.workspace_dirs();
                let (here, other): (Vec<_>, Vec<_>) = listeners.into_iter().partition(|l| dirs.iter().any(|d| under(&l.cwd, d)));
                let home = std::env::home_dir();
                let mut rows = Vec::new();
                for (label, group) in [("This workspace", here), ("Elsewhere", other)] {
                    if !group.is_empty() {
                        rows.push(heading(theme, label));
                        rows.extend(group.into_iter().map(|l| port_row(&l, key, home.as_deref(), theme, cx)));
                    }
                }
                rows
            }
        };
        div().flex().flex_col().gap(px(2.)).child(div().pb(px(6.)).child(section_heading(theme, "icons/solar/server-minimalistic.svg", "Open ports"))).children(body).into_any_element()
    }
}

/// A section's title: its icon, then its name.
fn section_heading(theme: &Theme, icon: &'static str, label: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme.text)
        .child(gpui::svg().path(icon).size(px(16.)).flex_none().text_color(theme.text_muted))
        .child(label)
}

/// A page of Recently used: its site's icon, its title, and under it the site and when; a click loads it in the tab.
#[allow(clippy::too_many_arguments)]
fn recent_card(i: usize, visit: &crate::web::Visit, host: &str, icon: Option<std::sync::Arc<gpui::Image>>, now: u64, key: &PageKey, theme: &Theme, cx: &mut Context<Shell>) -> AnyElement {
    let (url, key, hover) = (visit.url.clone(), key.clone(), theme.hover);
    let letter = host.trim_start_matches("www.").chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    let mark = div()
        .size(px(32.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(8.))
        .bg(theme.wash(0.06))
        .child(match icon {
            Some(image) => gpui::img(image).size(px(18.)).into_any_element(),
            None => div().text_size(px(13.)).font_weight(gpui::FontWeight::MEDIUM).text_color(theme.text_muted).child(letter).into_any_element(),
        });
    div()
        .id(("recent", i))
        .min_w_0()
        .p(px(10.))
        .flex()
        .items_center()
        .gap(px(10.))
        .rounded(px(10.))
        .border_1()
        .border_color(theme.border)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(mark)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().truncate().text_size(px(13.)).text_color(theme.text).child(visit.title.clone().unwrap_or_else(|| host.to_string())))
                .child(div().truncate().text_size(px(11.5)).text_color(theme.text_muted).child(format!("{host} · {}", crate::web::visited_ago(visit.at, now)))),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            if let Some(page) = this.pages.live.get(&key) {
                let _ = page.view.load_url(&url);
            }
            cx.notify();
        }))
        .into_any_element()
}

fn port_row(l: &Listener, key: &PageKey, home: Option<&std::path::Path>, theme: &Theme, cx: &mut Context<Shell>) -> AnyElement {
    let (url, pid, key) = (l.url(), l.pid, key.clone());
    let group = SharedString::from(format!("port-{}-{pid}", l.port));
    let hover = theme.hover;
    let stop = div()
        .id(SharedString::from(format!("port-stop-{}-{pid}", l.port)))
        .size(px(22.))
        .flex_none()
        .rounded(px(5.))
        .flex()
        .items_center()
        .justify_center()
        .invisible()
        .group_hover(group.clone(), |s| s.visible())
        .hover(move |s| s.bg(hover))
        .tooltip(tip("Stop", None))
        .child(gpui::svg().path("icons/solar/stop-circle.svg").size(px(14.)).text_color(theme.text_muted))
        .on_click(move |_: &ClickEvent, _, cx| {
            cx.stop_propagation();
            let _ = crate::process::command("kill").arg(pid.to_string()).status();
            refetch::<Vec<Listener>>(KEY, cx);
        });
    div()
        .id(group.clone())
        .group(group)
        .h(px(30.))
        .px(px(8.))
        .mx(px(-8.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .gap(px(10.))
        .text_size(px(12.5))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(div().w(px(52.)).flex_none().font_family(theme.font_terminal.clone()).text_color(theme.text).child(format!(":{}", l.port)))
        .child(div().flex_none().max_w(px(160.)).truncate().text_color(theme.text).child(l.name.clone()))
        .child(div().flex_1().min_w_0().truncate().text_color(theme.text_faint).child(short_path(&l.cwd, home)))
        .child(stop)
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            if let Some(page) = this.pages.live.get(&key) {
                let _ = page.view.load_url(&url);
            }
            cx.notify();
        }))
        .into_any_element()
}

fn heading(theme: &Theme, label: &'static str) -> AnyElement {
    div().pt(px(8.)).pb(px(2.)).text_size(px(11.)).text_color(theme.text_faint).child(label).into_any_element()
}

fn note(theme: &Theme, text: &'static str) -> AnyElement {
    div().text_size(px(12.)).text_color(theme.text_muted).child(text).into_any_element()
}
