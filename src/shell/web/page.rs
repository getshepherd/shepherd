//! A page's making and its web view's events: wry built for an entry, and what the page tells Shepherd.
use super::*;

impl Shell {
    /// `entry_url` is the entry's URL, which keeps the page while it is the same; `url` is the one loaded, the
    /// forward's for a page on a machine, and the site its links are measured against.
    pub(super) fn make_page(&mut self, key: PageKey, entry_url: &str, url: &str, window: &Window, cx: &mut Context<Self>) -> wry::Result<Page> {
        let events = self.page_events(window, cx);
        // an extension's page wears Shepherd's theme from its first paint
        let dressed = crate::extensions::by_page(cx, &key.0).map(|ext| crate::extensions::theme_script(Theme::of(cx), &ext.origin()));
        let browser = browser::is_tab(&key.0);
        let device = self.device_page_session(&key.0);
        let instrumented = device.is_none();
        // Chrome extensions run in browser tabs and sidebar pages; a device's 3D view and an extension's own page
        // (Shepherd's UI, themed by it) stay without them
        let extended = device.is_none() && dressed.is_none();
        let tab_id = extended.then(|| seen_key(&key));
        self.pages.made += 1;
        let serial = self.pages.made;
        let send = move |event: PageEvent| {
            let _ = events.unbounded_send((key.clone(), serial, event));
        };
        let (title, loaded, opened, followed, tabbed) = (send.clone(), send.clone(), send.clone(), send.clone(), send.clone());
        let (ready, notified) = (send.clone(), send.clone());
        let (reviewed, reviewed_new, installs) = (send.clone(), send.clone(), send.clone());
        #[cfg(target_os = "macos")]
        let key_event = send;
        // a link to another site goes to the browser when the setting says so; anything else stays in the page
        // a browser tab is the browser: its links stay in it
        let (site, flag) = (url.to_string(), self.pages.links_outside.clone());
        let follows_outside: Rc<dyn Fn(&str) -> bool> = Rc::new(move |to| !browser && flag.get() && off_site(&site, to));
        let opens_outside = follows_outside.clone();
        let tabs_outside = follows_outside.clone();
        let tab = move |to: String| tabbed(if tabs_outside(&to) { PageEvent::Outside(to) } else { PageEvent::NewTab(to, true) });
        // WebKitGTK takes a scheme once per context: a 3D view, which keeps nothing between launches, has its own
        let shared = cfg!(not(target_os = "macos")) && !(cfg!(target_os = "linux") && device.is_some());
        let builder = match shared {
            true => wry::WebViewBuilder::new_with_web_context(
                self.pages.context.get_or_insert_with(|| wry::WebContext::new(Some(crate::settings::cache_dir().join("web")))),
            ),
            false => wry::WebViewBuilder::new(),
        };
        #[cfg(target_os = "macos")]
        let builder = match extended.then(|| crate::browser_extensions::configuration(cx)).flatten() {
            Some(config) => wry::WebViewBuilderExtMacos::with_webview_configuration(builder, config),
            None => builder,
        };
        // every page but a device's 3D view calls itself Safari: a sidebar page too, or Slack there says the browser
        // isn't supported
        #[cfg(target_os = "macos")]
        let builder = match device.is_none() {
            true => builder.with_user_agent(safari_agent()),
            false => builder,
        };
        // a page zooms with a pinch; a device's 3D view keeps its gestures for the model
        #[cfg(target_os = "macos")]
        let pinch_zooms = device.is_none();
        // a device's 3D view: its frames, model and input go over a scheme and IPC of its own
        let builder = match device {
            Some((session, feed)) => super::device::three::attach(builder, session, feed, self.rt.clone(), move || ready(PageEvent::Ready)),
            None => new_tabs(builder, tab, move |note| notified(PageEvent::Notify(note))),
        };
        // every page but a 3D view keeps its requests and console lines for agents (`shepherd browser`)
        let builder = match instrumented {
            true => builder.with_initialization_script(crate::browser_agent::INSTRUMENT),
            false => builder,
        };
        let builder = match dressed {
            Some(script) => builder.with_initialization_script(script),
            None => builder,
        };
        // the Web Store's button installs into Shepherd where WebKit can run extensions
        let builder = match browser && crate::browser_extensions::unavailable().is_none() {
            true => builder.with_initialization_script(crate::browser_extensions::STORE_SCRIPT),
            false => builder,
        };
        #[cfg(target_os = "linux")]
        let parent = Parent(window);
        #[cfg(not(target_os = "linux"))]
        let parent = window;
        let view = builder
            .with_url(url)
            .with_devtools(true)
            // a two-finger swipe goes back and forward, as in Safari (WebKit and WebView2 only)
            .with_back_forward_navigation_gestures(true)
            .with_visible(false)
            .with_bounds(wry::Rect { position: LogicalPosition::new(0., 0.).into(), size: LogicalSize::new(1., 1.).into() })
            .with_document_title_changed_handler(move |t| title(PageEvent::Title(t)))
            .with_on_page_load_handler(move |e, _| loaded(PageEvent::Load(matches!(e, wry::PageLoadEvent::Started))))
            .with_navigation_handler(move |to| {
                if let Some(id) = crate::browser_extensions::install_link(&to) {
                    installs(PageEvent::InstallExtension(id));
                    return false;
                }
                if let Some((dir, branch)) = review_link(&to) {
                    reviewed(PageEvent::Review { dir, branch });
                    return false;
                }
                let stay = !follows_outside(&to);
                if !stay {
                    followed(PageEvent::Outside(to));
                }
                stay
            })
            .with_new_window_req_handler(move |to, _| {
                if let Some((dir, branch)) = review_link(&to) {
                    reviewed_new(PageEvent::Review { dir, branch });
                    return wry::NewWindowResponse::Deny;
                }
                let event = if opens_outside(&to) { PageEvent::Outside(to) } else { PageEvent::NewTab(to, false) };
                opened(event);
                wry::NewWindowResponse::Deny
            })
            .build_as_child(&parent)?;
        #[cfg(target_os = "macos")]
        if pinch_zooms {
            host::allow_magnification(&view);
        }
        #[cfg(target_os = "macos")]
        host::name_new_tab_item(&view);
        // GPUI's deferred layer goes on a plane of its own above native views (macOS only, in this GPUI); the page
        // moves into a view of its own below it. Without it, the page hides under Shepherd's overlays instead.
        #[cfg(target_os = "macos")]
        let host = window
            .enable_scene_overlay()
            .ok()
            .and_then(|()| host::Host::new(&view, self.pages.shortcuts.clone(), key_event))
            .map(Rc::new);
        #[cfg(target_os = "macos")]
        {
            self.pages.layered = host.is_some();
        }
        let _extension_tab = tab_id.map(|id| crate::browser_extensions::opened_tab(&id, &view));
        Ok(Page {
            _extension_tab,
            view: Rc::new(view),
            serial,
            entry_url: entry_url.to_string(),
            title: None,
            loading: true,
            reloaded: None,
            visible: false,
            focused: false,
            placed: Default::default(),
            viewport: Default::default(),
            viewport_zoom: Default::default(),
            #[cfg(target_os = "macos")]
            host,
            tunnel: None,
            annotations: Default::default(),
        })
    }

    /// WebKitGTK needs GTK set up on this thread, once, and its loop run: here a few iterations every frame's worth
    /// of time, beside GPUI's loop, while any page lives or waits for its forward.
    #[cfg(target_os = "linux")]
    pub(super) fn gtk_ready(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        static INIT: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
        // GPUI is on X11 here (a Wayland window opens pages in the browser); GDK would take Wayland when it is there too,
        // and the page couldn't be put in GPUI's X11 window
        INIT.get_or_init(|| {
            gtk::gdk::set_allowed_backends("x11");
            gtk::init().map_err(|e| e.to_string())
        })
        .clone()?;
        if !std::mem::replace(&mut self.pages.gtk_pump, true) {
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_millis(16)).await;
                    while gtk::events_pending() {
                        gtk::main_iteration_do(false);
                    }
                    let Ok(live) = this.update(cx, |s, _| {
                        let live = !s.pages.live.is_empty() || !s.pages.forwards.is_empty();
                        s.pages.gtk_pump = live;
                        live
                    }) else {
                        break;
                    };
                    if !live {
                        break;
                    }
                }
            })
            .detach();
        }
        Ok(())
    }

    /// The channel the pages' handlers report on; its first use starts the loop that applies what they say.
    pub(super) fn page_events(&mut self, window: &Window, cx: &mut Context<Self>) -> UnboundedSender<(PageKey, u64, PageEvent)> {
        if let Some(events) = &self.pages.events {
            return events.clone();
        }
        let (events, mut said) = futures::channel::mpsc::unbounded::<(PageKey, u64, PageEvent)>();
        cx.spawn_in(window, async move |this, cx| {
            while let Some((key, serial, event)) = said.next().await {
                if this.update_in(cx, |s, window, cx| s.page_event(key, serial, event, window, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        self.pages.events = Some(events.clone());
        events
    }

    pub(super) fn page_event(&mut self, key: PageKey, serial: u64, event: PageEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(page) = self.pages.live.get_mut(&key).filter(|p| p.serial == serial) else { return };
        match event {
            PageEvent::Title(title) => {
                page.title = Some(title).filter(|t| !t.is_empty());
                // a sidebar entry's is kept, for when its page isn't loaded
                let seen = self.pages.seen_titles.get(&key).map(|(u, t)| (u.as_str(), t.as_str()));
                if let Some(title) = page.title.clone().filter(|t| seen != Some((page.entry_url.as_str(), t.as_str())))
                    && sidebar_entries(cx).iter().any(|e| e.id == key.0)
                {
                    let (mark, url) = (seen_key(&key), page.entry_url.clone());
                    self.pages.seen_titles.insert(key.clone(), (url.clone(), title.clone()));
                    cx.background_spawn(async move { crate::web::Seen::default().keep_title(&mark, &url, &title) }).detach();
                }
            }
            PageEvent::Load(started) => {
                page.loading = started;
                // a load forgets `window.shepherd`: the next render tells it again
                if !started {
                    self.pages.told.remove(&key);
                }
            }
            PageEvent::Outside(url) => crate::ui::open_outside(&url, cx),
            PageEvent::Review { dir, branch } => self.pick_review(dir, branch, window, cx),
            PageEvent::InstallExtension(id) => {
                crate::ui::toast::info(cx, "Installing the extension…", None);
                let task = cx.background_spawn(async move { crate::browser_extensions::install_from_store(&id) });
                cx.spawn(async move |_, cx| {
                    let result = task.await;
                    cx.update(|cx| match result {
                        Ok(path) => {
                            let name = crate::browser_extensions::info(&path).map(|i| i.name).unwrap_or_default();
                            crate::ui::settings::add_browser_extension(cx, path);
                            crate::ui::toast::info(cx, format!("Added {name}. It runs in browser tabs from their next load."), None);
                        }
                        Err(why) => crate::ui::toast::error(cx, why),
                    })
                })
                .detach();
            }
            // dispatched as if pressed in Shepherd; a menu or dialog it opens takes the keyboard in `sync_pages`
            PageEvent::Key(keystroke) => {
                // GPUI's focus may be on what the page hides (a pane): the keystroke would start at the window's root,
                // above the Shell's handlers, and nothing would take it
                if !self.focus.contains_focused(window, cx) {
                    window.focus(&self.focus, cx);
                }
                window.defer(cx, move |window, cx| {
                    window.dispatch_keystroke(keystroke, cx);
                });
            }
            // the address field let go, as a click elsewhere in Shepherd would
            PageEvent::Clicked => {
                if self.panel_page(window).as_ref() == Some(&key) {
                    self.focus_panel(true, window, cx);
                }
                if self.browser.editing() || self.note_focused(window, cx) {
                    window.focus(&self.focus, cx);
                }
            }
            PageEvent::NewTab(url, behind) => return self.open_tab(&key, url, behind, window, cx),
            // every 100 ms while picking: nothing below is for it
            PageEvent::Picked(result) => return self.annotation_polled(&key, &result, window, cx),
            PageEvent::Ready => self.device_page_ready(&key.0),
            // ponytail: posted even while the page is in view and the window in front, as Safari does; the page
            // itself decides (Slack doesn't notify for the channel you are reading)
            PageEvent::Notify(note) => {
                let seen = page.title.clone();
                if self.shown_page(cx).map(|(k, _)| k).as_ref() != Some(&key) && self.panel_page(window).as_ref() != Some(&key) {
                    *self.pages.unread.entry(key.clone()).or_default() += 1;
                    cx.notify();
                }
                let entries = sidebar_entries(cx).into_iter().chain(self.panel_entries(cx));
                let name = entries.into_iter().find(|e| e.id == key.0).map(|e| e.name).filter(|n| !n.is_empty());
                let name = name.or(seen).unwrap_or_else(|| self.browser_label(&key.0));
                let title = if note.title.is_empty() { name } else { format!("{} · {name}", note.title) };
                let sound = !note.silent && SettingsGlobal::of(cx).settings.notify_sound;
                // the notification's own icon, else the page's favicon, beside its text as a browser shows it
                let favicon = match self.pages.favicons.get(&key) {
                    Some((_, Favicon::Image(image))) => Some(image.bytes().to_vec()),
                    _ => None,
                };
                let name = seen_key(&key);
                cx.spawn(async move |_, cx| {
                    let image = cx.background_spawn(async move { crate::web::notification_image(&note.icon, favicon, &name) }).await;
                    cx.update(|cx| crate::notify::show(cx, &crate::notify::web_tag(serial, note.id), title, note.body, sound, None, image));
                })
                .detach();
                return;
            }
        }
        if browser::is_tab(&key.0) {
            self.browser_loaded(&key, window, cx);
        }
        cx.notify();
    }
}
