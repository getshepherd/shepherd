//! Browser tabs: web pages in a workspace's right panel beside the panes, opened from the panel or by a link clicked
//! in a terminal while the setting says so. They are Shepherd's own (herdr doesn't know them) and last until Shepherd
//! quits; the pages themselves are `web.rs`'s, as the sidebar's are.
use super::*;
use super::web::{Favicon, PageKey, nav_button};
use crate::right_panel::Surface;
use crate::settings::WebEntry;

/// Browser tabs' ids start with this, so they never meet a web entry's.
const PREFIX: &str = "browser-";
/// An empty tab's page.
pub(super) const BLANK: &str = "about:blank";

pub(super) struct BrowserTab {
    pub(super) id: String,
    pub(super) workspace: String,
    /// What it opened on; where its links lead from there is the page's business.
    url: String,
}

#[derive(Default)]
pub(super) struct Browser {
    pub(super) tabs: Vec<BrowserTab>,
    made: u64,
    /// The shown tab's address field.
    address: Option<Address>,
    /// A link clicked in a terminal, opened by the next render, which has the window.
    pending: Option<String>,
    /// A blank tab's list of ports has its next look set: it reads again while shown.
    pub(super) ports_tick: bool,
    /// The bar's `…` menu: the tab it is for, and where it opens.
    pub(super) menu: Option<crate::ui::Popup<(String, gpui::Point<Pixels>)>>,
    /// The pages tabs showed, read once a blank tab or a load asks for them.
    history: Option<crate::web::History>,
    /// What each tab last put in the history, so its other events don't put it there again.
    recorded: HashMap<String, (String, Option<String>)>,
    /// Sites' icons for Recently used, by site (`https://github.com`): `None` while read or fetched, or when none.
    pub(super) site_icons: HashMap<String, Option<std::sync::Arc<gpui::Image>>>,
    /// Each tab's page zoom, 1 unless changed in its menu.
    zoom: HashMap<String, f64>,
}

struct Address {
    tab: String,
    input: Entity<InputState>,
    /// Being typed in: the page doesn't take the keyboard, and loads don't overwrite it.
    editing: bool,
    _events: Subscription,
}

impl Browser {
    /// The tabs as web entries, which is how the pages know them.
    pub(super) fn entries(&self) -> impl Iterator<Item = WebEntry> + '_ {
        self.tabs.iter().map(|t| WebEntry { id: t.id.clone(), url: t.url.clone(), ..Default::default() })
    }

    pub(super) fn editing(&self) -> bool {
        self.address.as_ref().is_some_and(|a| a.editing)
    }
}

/// Whether page `id` is a browser tab's.
pub(super) fn is_tab(id: &str) -> bool {
    id.starts_with(PREFIX)
}

/// The shell links clicked in a terminal go to.
pub(super) struct Links(pub(super) gpui::WeakEntity<Shell>);

impl gpui::Global for Links {}

/// A link clicked in a terminal: a web one to a browser tab in the right panel while the setting says so, anything
/// else to the system.
pub(crate) fn open_link(url: &str, cx: &mut App) {
    let web = url.starts_with("http://") || url.starts_with("https://");
    if web && SettingsGlobal::of(cx).settings.web_links_inside {
        open_inside(url, cx);
    } else {
        crate::ui::open_outside(url, cx);
    }
}

/// `url` in a browser tab in the right panel, whatever the setting for terminal links says: the review panel's PR
/// pages go there. A tab of the workspace's that opened on it is shown again rather than made twice. Deferred: a
/// shell's own menu calls this from inside an update of the shell, which a second one would panic on.
pub(crate) fn open_inside(url: &str, cx: &mut App) {
    match cx.try_global::<Links>().and_then(|l| l.0.upgrade()) {
        Some(shell) => {
            let url = url.to_string();
            cx.defer(move |cx| {
                shell.update(cx, |s, cx| {
                    s.browser.pending = Some(url);
                    cx.notify();
                })
            });
        }
        None => crate::ui::open_outside(url, cx),
    }
}

/// Chrome's zoom levels, which the menu's − and + step through.
const ZOOMS: [f64; 15] = [0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1., 1.1, 1.25, 1.5, 1.75, 2., 2.5, 3.];

/// The zoom level after `now`, up or down; the last one at either end.
fn zoom_step(now: f64, up: bool) -> f64 {
    let next = match up {
        true => ZOOMS.iter().find(|z| **z > now + 1e-3),
        false => ZOOMS.iter().rev().find(|z| **z < now - 1e-3),
    };
    next.copied().unwrap_or(now)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// `https://example.com/a?b` → `https://example.com`: what a tab's favicon is fetched again for.
pub(super) fn site(url: &str) -> Option<String> {
    let uri: http::Uri = url.parse().ok()?;
    Some(format!("{}://{}", uri.scheme_str()?, uri.authority()?))
}

/// What's typed in the address field, as a URL: `localhost:3000` gets `http://` and `example.com` `https://` (see
/// `web::normalize`); anything with a space or without a dot or a port goes to a search.
fn typed_url(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let host = text.split(['/', '?', '#']).next().unwrap_or_default();
    if text.contains("://") || text.starts_with("about:") || (!text.contains(' ') && (host.contains('.') || host.contains(':') || host == "localhost")) {
        return Some(crate::web::normalize(text));
    }
    let query: String = percent_encoding::utf8_percent_encode(text, percent_encoding::NON_ALPHANUMERIC).to_string();
    Some(format!("https://duckduckgo.com/?q={query}"))
}

impl Shell {
    /// A new browser tab in workspace `ws`'s panel, on `url` or empty, the panel opened on it; `focus_address` gives its
    /// address field the keyboard. A page that can't be made leaves no tab behind.
    pub(super) fn new_browser_tab(&mut self, ws: String, url: Option<String>, focus_address: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.browser.made += 1;
        let id = format!("{PREFIX}{}", self.browser.made);
        self.browser.tabs.push(BrowserTab { id: id.clone(), workspace: ws.clone(), url: url.unwrap_or_else(|| BLANK.into()) });
        if !self.ensure_page(&id, window, cx) {
            self.browser.tabs.retain(|t| t.id != id);
            return;
        }
        self.right.panels.entry(ws).or_default().add(Surface::Browser(id.clone()));
        if focus_address {
            self.focus_panel(true, window, cx);
            let input = self.address(&id, window, cx);
            input.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    /// A link asked for in a new tab from page `key`: a browser tab behind the one shown, in the workspace of the
    /// page's tab, else (a sidebar page's) the workspace shown, its panel showing it once the page is left.
    pub(super) fn open_behind(&mut self, key: &PageKey, url: String, window: &mut Window, cx: &mut Context<Self>) {
        let from_tab = self.browser.tabs.iter().find(|t| t.id == key.0).map(|t| t.workspace.clone());
        let Some(ws) = from_tab.clone().or_else(|| self.shown_tab().map(|t| t.workspace_id)) else {
            return crate::ui::open_outside(&url, cx);
        };
        let was = self.right.panels.get(&ws).and_then(|p| p.active.clone());
        self.new_browser_tab(ws.clone(), Some(url), false, window, cx);
        if let (Some(was), Some(panel)) = (was, self.right.panels.get_mut(&ws)) {
            panel.activate(&was);
        }
        if from_tab.is_none() {
            crate::ui::toast::info(cx, "Opened in a browser tab", None);
        }
    }

    /// Closes browser tab `id` and its panel's tab. Its page goes in `sync_pages`.
    pub(super) fn close_browser_tab(&mut self, id: &str, cx: &mut Context<Self>) {
        let surface = Surface::Browser(id.to_string());
        for panel in self.right.panels.values_mut() {
            panel.close(&surface);
        }
        self.browser.tabs.retain(|t| t.id != id);
        cx.notify();
    }

    /// Once per render, before the pages sync: a link clicked in a terminal opens its tab, the keyboard staying in the
    /// terminal; workspaces gone take their tabs and panels along.
    pub(super) fn sync_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(url) = self.browser.pending.take() {
            match self.shown_tab() {
                Some(tab) => match self.browser.tabs.iter().find(|t| t.workspace == tab.workspace_id && t.url == url) {
                    Some(open) => {
                        let surface = Surface::Browser(open.id.clone());
                        self.right.panels.entry(tab.workspace_id).or_default().add(surface);
                        cx.notify();
                    }
                    None => self.new_browser_tab(tab.workspace_id, Some(url), false, window, cx),
                },
                None => crate::ui::open_outside(&url, cx),
            }
        }
        let Some(s) = self.snapshot.as_ref().filter(|s| !s.workspaces.is_empty()) else { return };
        // ponytail: another place shown (a session, a machine) closes this one's browser tabs; keyed by place if missed
        self.browser.tabs.retain(|t| s.workspaces.iter().any(|w| w.workspace_id == t.workspace));
        self.right.panels.retain(|ws, _| s.workspaces.iter().any(|w| w.workspace_id == *ws));
        for (ws, panel) in &mut self.right.panels {
            let live: Vec<&str> = self.browser.tabs.iter().filter(|t| t.workspace == *ws).map(|t| t.id.as_str()).collect();
            panel.retain_browsers(&live);
        }
        let panels = &self.right.panels;
        self.right.diffs.retain(|id, _| panels.values().any(|p| p.tabs.iter().any(|s| s.key() == id)));
        let diffs = &self.right.diffs;
        self.right.diff_agents.retain(|id, _| diffs.contains_key(id));
        self.sync_devices();
    }

    /// Tab `id`'s label: its page's title, else its site, else New Tab.
    pub(super) fn browser_label(&self, id: &str) -> String {
        let page = self.pages.live.get(&(id.to_string(), None));
        let title = page.and_then(|p| p.title.clone());
        let url = page.and_then(|p| super::web::page_url(&p.view)).filter(|u| u != BLANK);
        title.or_else(|| url.and_then(|u| u.parse::<http::Uri>().ok()?.host().map(str::to_string))).unwrap_or_else(|| "New Tab".into())
    }

    /// The address field of tab `id`, made for it if the field was another tab's.
    fn address(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some(a) = self.browser.address.as_ref().filter(|a| a.tab == id) {
            return a.input.clone();
        }
        let theme = Theme::of(cx).clone();
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Address or search"));
        input.update(cx, |input, _| input.set_editor_style(crate::ui::input_style(&theme)));
        let tab = id.to_string();
        let events = cx.subscribe_in(&input, window, move |shell, input, event: &InputEvent, window, cx| {
            let key: PageKey = (tab.clone(), None);
            match event {
                InputEvent::Focus => {
                    if let Some(a) = shell.browser.address.as_mut() {
                        a.editing = true;
                    }
                    input.update(cx, |input, cx| input.select_all(window, cx));
                }
                InputEvent::Blur => {
                    if let Some(a) = shell.browser.address.as_mut() {
                        a.editing = false;
                    }
                    shell.show_address(&key, window, cx);
                }
                InputEvent::PressEnter { .. } => {
                    let url = typed_url(&input.read(cx).value());
                    if let (Some(url), Some(page)) = (url, shell.pages.live.get(&key)) {
                        let _ = page.view.load_url(&url);
                    }
                    // the keyboard to the page: `sync_pages` hands it over once the field lets go
                    window.focus(&shell.focus, cx);
                }
                InputEvent::Change => {}
            }
            cx.notify();
        });
        self.browser.address = Some(Address { tab: id.to_string(), input: input.clone(), editing: false, _events: events });
        input
    }

    /// The panel's browser tab's address field takes the keyboard, its text selected; a sidebar page has none.
    pub(super) fn focus_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, _)) = self.panel_page(window) else { return };
        self.focus_panel(true, window, cx);
        let input = self.address(&id, window, cx);
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
    }

    /// The address field shows the page's URL, unless it is being typed in.
    fn show_address(&mut self, key: &PageKey, window: &mut Window, cx: &mut Context<Self>) {
        let Some(a) = self.browser.address.as_ref().filter(|a| a.tab == key.0 && !a.editing) else { return };
        let url = self.pages.live.get(key).and_then(|p| super::web::page_url(&p.view)).filter(|u| u != BLANK).unwrap_or_default();
        if a.input.read(cx).value() != url {
            a.input.update(cx, |input, cx| input.set_value(url, window, cx));
        }
    }

    /// A browser tab's page loaded: its address shown, and its site's icon fetched when the site changed.
    pub(super) fn browser_loaded(&mut self, key: &PageKey, window: &mut Window, cx: &mut Context<Self>) {
        self.show_address(key, window, cx);
        let Some(url) = self.pages.live.get(key).and_then(|p| super::web::page_url(&p.view)) else { return };
        self.record_visit(key, &url, cx);
        if let Some(site) = site(&url).filter(|s| self.pages.favicons.get(key).is_none_or(|(had, _)| had != s)) {
            self.fetch_favicon(key.clone(), site, url, cx);
        }
    }

    /// The history, read the first time it is asked for.
    pub(super) fn history(&mut self) -> &mut crate::web::History {
        self.browser.history.get_or_insert_with(crate::web::History::load)
    }

    /// Tab `key` on `url`, its load done: Recently used's first, with the page's title, saved in the background. Not
    /// while it loads, when the title is still the last page's.
    fn record_visit(&mut self, key: &PageKey, url: &str, cx: &mut Context<Self>) {
        let Some(page) = self.pages.live.get(key).filter(|p| !p.loading) else { return };
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return;
        }
        let seen = (url.to_string(), page.title.clone());
        if self.browser.recorded.get(&key.0) == Some(&seen) {
            return;
        }
        self.browser.recorded.insert(key.0.clone(), seen.clone());
        let history = self.history();
        if history.record(&seen.0, seen.1.as_deref(), unix_now())
            && let Some((path, text)) = history.to_save()
        {
            cx.background_spawn(async move {
                let _ = path.parent().map(std::fs::create_dir_all);
                let _ = std::fs::write(path, text);
            })
            .detach();
        }
    }

    /// Site `site`'s icon for Recently used: kept from an earlier launch, else fetched once in the background.
    pub(super) fn site_icon(&mut self, site: &str, cx: &mut Context<Self>) -> Option<std::sync::Arc<gpui::Image>> {
        if let Some(icon) = self.browser.site_icons.get(site) {
            return icon.clone();
        }
        // ponytail: read on the main thread, once per site a launch; a small file
        let kept = crate::web::Seen::default().icon(&crate::web::site_key(site), site);
        let icon = kept.map(|(format, bytes)| std::sync::Arc::new(gpui::Image::from_bytes(format, bytes)));
        self.browser.site_icons.insert(site.to_string(), icon.clone());
        if icon.is_none() {
            let site = site.to_string();
            let fetched = cx.background_spawn({
                let site = site.clone();
                async move {
                    let (format, bytes) = crate::web::fetch_favicon(&site)?;
                    let _ = crate::web::Seen::default().keep_icon(&crate::web::site_key(&site), &site, &bytes);
                    Some(std::sync::Arc::new(gpui::Image::from_bytes(format, bytes)))
                }
            });
            cx.spawn(async move |this, cx| {
                let icon = fetched.await;
                let _ = this.update(cx, |s, cx| {
                    s.browser.site_icons.insert(site, icon);
                    cx.notify();
                });
            })
            .detach();
        }
        icon
    }

    /// The shown page's inspector opened, or closed if open (⌥⌘I).
    pub(super) fn toggle_devtools(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(page) = self.visible_page(window, cx).and_then(|key| self.pages.live.get(&key)) else { return };
        match page.view.is_devtools_open() {
            true => page.view.close_devtools(),
            false => page.view.open_devtools(),
        }
    }

    /// Tab `id`'s page loaded again bypassing the cache (WebKit's reload from origin).
    fn hard_reload(&mut self, id: &str) {
        let key: PageKey = (id.to_string(), None);
        #[cfg(target_os = "macos")]
        if let Some(page) = self.pages.live.get(&key) {
            webkit::reload_from_origin(&page.view);
        }
        // ponytail: WebKitGTK's plain reload, which wry alone reaches; its bypass-cache one when someone asks
        #[cfg(not(target_os = "macos"))]
        self.reload_page(&key);
    }

    /// Tab `id`'s page zoomed to `zoom`.
    fn set_zoom(&mut self, id: &str, zoom: f64) {
        if let Some(page) = self.pages.live.get(&(id.to_string(), None)) {
            let _ = page.view.zoom(zoom);
            self.browser.zoom.insert(id.to_string(), zoom);
        }
    }

    /// The shown browser tab's `…` menu opened: `SHEPHERD_PANEL=browser-menu` for a screenshot.
    pub fn open_browser_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(Surface::Browser(id)) = self.panel().and_then(|p| p.active.clone()) {
            self.browser.menu = Some(crate::ui::Popup::new((id, gpui::Point::default())));
            cx.notify();
        }
    }

    fn close_browser_menu(&mut self) {
        if let Some(m) = self.browser.menu.as_mut() {
            m.close();
        }
    }

    /// The bar's `…` button: its menu, under it.
    fn menu_button(&self, id: &str, theme: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        let open = self.browser.menu.as_ref().is_some_and(|m| m.is_open());
        let tab = id.to_string();
        super::web::nav_frame(theme, "browser-menu", true)
            .relative()
            .when(open, |el| el.bg(theme.hover))
            .tooltip(tip("More", None))
            .child(super::widgets::anchor_probe(&self.anchors, "browser-menu"))
            .child(gpui::svg().path("icons/solar/menu-dots.svg").size(px(16.)).text_color(theme.text_muted))
            // capture, as the other menus' triggers: a press on the open menu's button closes it
            .capture_any_mouse_down(cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                match this.browser.menu.as_mut() {
                    Some(menu) if menu.is_open() => menu.close(),
                    _ => {
                        let at = this.anchors.borrow().get("browser-menu").map_or(e.position, |b| b.bottom_right() + gpui::point(px(0.), px(4.)));
                        this.browser.menu = Some(crate::ui::Popup::new((tab.clone(), at)));
                    }
                }
                cx.notify();
            }))
    }

    /// The `…` menu of a browser tab's bar: Hard reload, DevTools, the zoom, and clearing the site's cookies or cache.
    pub(super) fn browser_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let menu = self.browser.menu.as_ref()?;
        let (id, at) = menu.value.clone();
        // under the button as laid out now: the panel may have moved since the press
        let at = self.anchors.borrow().get("browser-menu").map_or(at, |b| b.bottom_right() + gpui::point(px(0.), px(4.)));
        let view = self.pages.live.get(&(id.clone(), None)).map(|p| p.view.clone())?;
        let url = super::web::page_url(&view).filter(|u| u != BLANK);
        let zoom = self.browser.zoom.get(&id).copied().unwrap_or(1.);
        let devtools = view.is_devtools_open();
        let chord = cx.try_global::<crate::bindings::ActiveBindings>().and_then(|b| b.0.iter().find(|(a, _)| a == "ToggleDevTools").map(|(_, k)| k.clone()));
        type Pick = Box<dyn Fn(&mut Shell, &mut Window, &mut Context<Shell>)>;
        let item = |key: &'static str, icon: &'static str, label: &'static str, enabled: bool, pick: Pick| {
            row(theme, false, key)
                .h(px(28.))
                .rounded(px(6.))
                .when(!enabled, |el| el.opacity(0.4).cursor_default())
                .child(super::widgets::menu_mark(icon, theme.text_muted))
                .child(div().flex_1().child(label))
                .when(enabled, |el| {
                    el.on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.close_browser_menu();
                        pick(this, window, cx);
                        cx.notify();
                    }))
                })
        };
        let step = |key: &'static str, label: &'static str, to: f64, enabled: bool| {
            let tab = id.clone();
            let hover = theme.hover;
            div()
                .id(key)
                .h(px(22.))
                .min_w(px(22.))
                .px(px(4.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.))
                .text_color(if enabled { theme.text } else { theme.text_faint })
                .child(label)
                .when(enabled, |el| {
                    el.cursor_pointer().hover(move |s| s.bg(hover)).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        // the menu stays open, for the next step
                        this.set_zoom(&tab, to);
                        cx.notify();
                    }))
                })
        };
        let zoom_row = div()
            .h(px(28.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(super::widgets::menu_mark("icons/solar/magnifier-zoom-in.svg", theme.text_muted))
            .child(div().flex_1().child("Zoom"))
            .child(step("browser-zoom-out", "−", zoom_step(zoom, false), zoom_step(zoom, false) != zoom))
            // the level itself resets it
            .child(step("browser-zoom-reset", "", 1., zoom != 1.).min_w(px(44.)).child(format!("{:.0}%", zoom * 100.)))
            .child(step("browser-zoom-in", "+", zoom_step(zoom, true), zoom_step(zoom, true) != zoom));
        // the site's own data only: the pages share one store, and the sidebar's would be signed out too
        let host = url.as_deref().and_then(|u| u.parse::<http::Uri>().ok()?.host().map(str::to_string));
        #[cfg(target_os = "macos")]
        let clears: Vec<gpui::AnyElement> = host.map_or(Vec::new(), |host| {
            let clear = |key: &'static str, icon: &'static str, label: &'static str, kinds: &'static [&'static str], done: &'static str| {
                let (view, host) = (view.clone(), host.clone());
                item(key, icon, label, true, Box::new(move |_, _, cx| {
                    webkit::clear_site_data(&view, kinds, host.clone());
                    crate::ui::toast::info(cx, format!("{done} {host}"), None);
                }))
                .into_any_element()
            };
            vec![
                div().mx(px(-4.)).my(px(4.)).h(px(1.)).bg(theme.border).into_any_element(),
                clear("browser-clear-cookies", "icons/solar/trash-bin-minimalistic.svg", "Clear cookies", &webkit::COOKIES, "Cleared the cookies of"),
                clear("browser-clear-cache", "icons/solar/broom.svg", "Clear cache", &webkit::CACHE, "Cleared the cache of"),
            ]
        });
        // ponytail: WebKitGTK's store is cleared whole by wry; per site there, when someone asks
        #[cfg(not(target_os = "macos"))]
        let clears: Vec<gpui::AnyElement> = { let _ = host; Vec::new() };
        let (reload, tools) = (id.clone(), view.clone());
        let card = menu_card(theme, "browser-menu-card", 240.)
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.close_browser_menu();
                cx.notify();
            }))
            .child(item("browser-hard-reload", "icons/solar/restart.svg", "Hard reload", url.is_some(), Box::new(move |this, _, _| this.hard_reload(&reload))))
            .child(
                item(
                    "browser-devtools",
                    "icons/solar/code.svg",
                    if devtools { "Close DevTools" } else { "Open DevTools" },
                    true,
                    Box::new(move |_, _, _| match tools.is_devtools_open() {
                        true => tools.close_devtools(),
                        false => tools.open_devtools(),
                    }),
                )
                .children(chord.map(|k| div().text_size(px(11.)).text_color(theme.text_faint).child(crate::ui::command_panel::glyphs(&k)))),
            )
            .child(zoom_row)
            .children(clears);
        let menu = crate::ui::menu_motion("browser-menu-in", menu, card);
        Some(gpui::deferred(gpui::anchored().position(at).anchor(gpui::Anchor::TopRight).snap_to_window_with_margin(px(8.)).child(menu)).with_priority(1).into_any_element())
    }

    /// Tab `id`'s site icon once fetched, and whether its page is loading.
    pub(super) fn browser_chip(&self, id: &str) -> (Option<std::sync::Arc<gpui::Image>>, bool) {
        let key = (id.to_string(), None);
        let icon = match self.pages.favicons.get(&key) {
            Some((_, Favicon::Image(image))) => Some(image.clone()),
            _ => None,
        };
        (icon, self.pages.live.get(&key).is_some_and(|p| p.loading))
    }

    /// The active browser tab's row over its page: back, forward, reload, the address and Open in Browser.
    pub(super) fn render_browser_bar(&mut self, key: &PageKey, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let input = self.address(&key.0, window, cx);
        if !self.browser.editing() && input.read(cx).value().is_empty() {
            self.show_address(key, window, cx);
        }
        let view = self.pages.live.get(key).map(|p| p.view.clone());
        let can = |f: fn(&wry::WebView) -> wry::Result<bool>| view.as_deref().is_some_and(|v| f(v).unwrap_or(false));
        let outside = view.clone();
        div()
            .h(px(36.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(2.))
            .px(px(6.))
            .bg(theme.surface)
            .border_b_1()
            .border_color(theme.border)
            // the address field's Esc ends a pick too
            .capture_key_down(cx.listener({
                let key = key.clone();
                move |this, e: &gpui::KeyDownEvent, _, cx| {
                    if e.keystroke.key == "escape" && this.annotating(&key) {
                        cx.stop_propagation();
                        this.stop_annotation(&key);
                        cx.notify();
                    }
                }
            }))
            .child(nav_button(theme, view.clone(), "browser-back", "icons/solar/alt-arrow-left.svg", can(wry::WebView::can_go_back), wry::WebView::go_back))
            .child(nav_button(theme, view.clone(), "browser-forward", "icons/solar/alt-arrow-right.svg", can(wry::WebView::can_go_forward), wry::WebView::go_forward))
            .child(self.reload_button(key, "browser-reload", theme, cx))
            .child(
                div()
                    .mx(px(4.))
                    .flex_1()
                    .min_w_0()
                    .h(px(28.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .rounded(px(8.))
                    .bg(theme.terminal.background)
                    .text_size(px(12.5))
                    .child(Input::new(&input)),
            )
            .children(extension_buttons(theme))
            .child(self.annotate_button(key, theme, cx))
            .child(
                div()
                    .id("browser-outside")
                    .size(px(28.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.hover))
                    .tooltip(tip("Open in Browser", None))
                    .child(gpui::svg().path("icons/solar/square-arrow-right-up.svg").size(px(16.)).text_color(theme.text_muted))
                    .on_click(move |_, _, cx| {
                        if let Some(url) = outside.as_deref().and_then(super::web::page_url).filter(|u| u != BLANK) {
                            crate::ui::open_outside(&url, cx);
                        }
                    }),
            )
            .child(self.menu_button(&key.0, theme, cx))
    }

}

/// A button per Chrome extension with an action, left of the bar's own: a click runs it or opens its popup.
#[cfg(target_os = "macos")]
fn extension_buttons(theme: &Theme) -> Vec<Stateful<Div>> {
    let actions = crate::browser_extensions::actions();
    actions
        .into_iter()
        .map(|(path, label, icon)| {
            let initial = label.chars().next().map(String::from).unwrap_or_default();
            super::web::nav_frame(theme, gpui::ElementId::Name(format!("browser-ext-{}", path.display()).into()), true)
                .tooltip(tip(label, None))
                .child(match icon {
                    Some(image) => gpui::img(image).size(px(16.)).into_any_element(),
                    None => div().text_size(px(11.)).text_color(theme.text_muted).child(initial).into_any_element(),
                })
                .on_click(move |e, window, _| {
                    let at = e.position();
                    crate::browser_extensions::perform_action(&path, window, f64::from(at.x), f64::from(at.y));
                })
        })
        .collect()
}

#[cfg(not(target_os = "macos"))]
fn extension_buttons(_: &Theme) -> Vec<Stateful<Div>> {
    Vec::new()
}

/// WebKit's own, which wry doesn't reach: reload from origin and a site's data.
#[cfg(target_os = "macos")]
mod webkit {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSArray, NSSet, NSString};
    use std::ptr::NonNull;
    use wry::WebViewExtMacOS;

    /// `WKWebsiteDataType…`: the constants' values are their names.
    pub(super) const COOKIES: [&str; 1] = ["WKWebsiteDataTypeCookies"];
    pub(super) const CACHE: [&str; 3] = ["WKWebsiteDataTypeDiskCache", "WKWebsiteDataTypeMemoryCache", "WKWebsiteDataTypeFetchCache"];

    pub(super) fn reload_from_origin(view: &wry::WebView) {
        // SAFETY: `reloadFromOrigin` on a live WKWebView, on the main thread
        let _: Option<Retained<AnyObject>> = unsafe { msg_send![&*view.webview(), reloadFromOrigin] };
    }

    /// Removes the data of `kinds` that WebKit keeps for `host`'s site: the records whose name (the registrable
    /// domain, `github.com`) is `host` or a domain `host` is under.
    pub(super) fn clear_site_data(view: &wry::WebView, kinds: &[&str], host: String) {
        let kinds: Vec<Retained<NSString>> = kinds.iter().map(|k| NSString::from_str(k)).collect();
        let kinds = NSSet::from_retained_slice(&kinds);
        // SAFETY: the page's configuration and its data store, on the main thread
        let store: Retained<AnyObject> = unsafe {
            let configuration: Retained<AnyObject> = msg_send![&*view.webview(), configuration];
            msg_send![&configuration, websiteDataStore]
        };
        let (from, removed) = (store.clone(), kinds.clone());
        let found = block2::RcBlock::new(move |records: NonNull<NSArray<AnyObject>>| {
            // SAFETY: WebKit hands the block a live array of WKWebsiteDataRecords, on the main thread
            let records = unsafe { records.as_ref() };
            let ours: Vec<Retained<AnyObject>> = records
                .iter()
                .filter(|r| {
                    // SAFETY: a record's `displayName`
                    let name: Retained<NSString> = unsafe { msg_send![&**r, displayName] };
                    let name = name.to_string();
                    host == name || host.ends_with(&format!(".{name}"))
                })
                .collect();
            let ours = NSArray::from_retained_slice(&ours);
            let done = block2::RcBlock::new(|| {});
            // SAFETY: the store's removal of its own records, on the main thread
            let () = unsafe { msg_send![&from, removeDataOfTypes: &*removed, forDataRecords: &*ours, completionHandler: &*done] };
        });
        // SAFETY: as above
        let () = unsafe { msg_send![&store, fetchDataRecordsOfTypes: &*kinds, completionHandler: &*found] };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_addresses_become_urls_or_searches() {
        assert_eq!(typed_url("localhost:3000").as_deref(), Some("http://localhost:3000"));
        assert_eq!(typed_url("localhost").as_deref(), Some("http://localhost"));
        assert_eq!(typed_url(" docs.rs/gpui ").as_deref(), Some("https://docs.rs/gpui"));
        assert_eq!(typed_url("https://x.dev/a").as_deref(), Some("https://x.dev/a"));
        assert_eq!(typed_url("rust lifetimes").as_deref(), Some("https://duckduckgo.com/?q=rust%20lifetimes"));
        assert_eq!(typed_url("  "), None);
        assert_eq!(site("https://x.dev:8080/a?b").as_deref(), Some("https://x.dev:8080"));
    }

    #[test]
    fn zoom_steps_through_chromes_levels() {
        assert_eq!(zoom_step(1., true), 1.1);
        assert_eq!(zoom_step(1., false), 0.9);
        assert_eq!(zoom_step(3., true), 3., "the last level holds");
        assert_eq!(zoom_step(0.25, false), 0.25);
    }
}
