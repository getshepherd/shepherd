//! The window's side of `shepherd browser …` (`crate::browser_agent`): which page a command is for, and what it does
//! there. A script's result comes back through wry's callback; an eval's, which may await, by polling the page.
use super::*;
use super::web::{PageKey, seen_key};
use crate::browser_agent::{Asked, Command, Reply, Request, js_string};
use crate::right_panel::Surface;
use futures::StreamExt;
use futures::channel::oneshot;
use std::rc::Weak;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// How long a page has to answer a script that doesn't wait on anything.
const ANSWER: Duration = Duration::from_secs(10);
/// How often an eval's page is asked whether it finished.
const POLL: Duration = Duration::from_millis(50);

impl Shell {
    /// Starts answering agents' `shepherd browser`, once a process.
    pub(super) fn serve_browser_agents(window: &mut Window, cx: &mut Context<Self>) {
        static SERVED: AtomicBool = AtomicBool::new(false);
        if SERVED.swap(true, Ordering::Relaxed) {
            return;
        }
        let (asked, mut requests) = futures::channel::mpsc::unbounded::<Asked>();
        crate::browser_agent::serve(asked);
        cx.spawn_in(window, async move |this, cx| {
            while let Some((request, reply)) = requests.next().await {
                if this.update_in(cx, |s, window, cx| s.browser_asked(request, reply, window, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn browser_asked(&mut self, request: Request, reply: oneshot::Sender<Reply>, window: &mut Window, cx: &mut Context<Self>) {
        let (tab, command) = match crate::browser_agent::parse(&request.args) {
            Ok(parsed) => parsed,
            Err(e) => return drop(reply.send(Reply::err(e))),
        };
        let ws = request.workspace.as_deref();
        match command {
            Command::Tabs => return drop(reply.send(self.agent_tabs(ws, window, cx))),
            Command::New(url) => {
                let made = self.agent_new_tab(ws, &url, window, cx);
                if made.ok {
                    self.drive(made.out.clone(), request.pane.clone(), cx);
                }
                return drop(reply.send(made));
            }
            _ => {}
        }
        let key = match self.agent_page(tab.as_deref(), ws, window, cx) {
            Ok(key) => key,
            Err(e) => return drop(reply.send(Reply::err(e))),
        };
        if command == Command::Close {
            let id = seen_key(&key);
            return drop(reply.send(if key.1.is_none() && self.browser.tabs.iter().any(|t| t.id == key.0) {
                self.close_browser_tab(&key.0, cx);
                Reply::ok(format!("{id}: closed"))
            } else {
                Reply::err(format!("{id} is not a browser tab: only those close, not a sidebar page or an extension's tab"))
            }));
        }
        let Some(page) = self.pages.live.get(&key) else { return drop(reply.send(Reply::err("the tab has no page"))) };
        let view = Rc::downgrade(&page.view);
        let id = seen_key(&key);
        self.drive(id.clone(), request.pane.clone(), cx);
        let Some(page) = self.pages.live.get(&key) else { return };
        match command {
            Command::Open(url) => {
                let url = super::browser::typed_url(&url).unwrap_or(url);
                drop(reply.send(match page.view.load_url(&url) {
                    Ok(()) => Reply::ok(format!("{id}: loading {url}")),
                    Err(e) => Reply::err(format!("{url}: {e}")),
                }))
            }
            Command::Reload => {
                self.reload_page(&key, cx);
                cx.notify();
                drop(reply.send(Reply::ok(format!("{id}: reloading"))));
            }
            Command::Network { entry, clear } => read(view, crate::browser_agent::network_script(entry, clear), reply, cx),
            Command::Console { clear } => read(view, crate::browser_agent::console_script(clear), reply, cx),
            Command::Eval { code, timeout } => {
                cx.spawn(async move |_, cx| drop(reply.send(eval(view, code, timeout, cx).await))).detach();
            }
            Command::Screenshot(path) => {
                let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
                let path = path.map_or_else(|| crate::settings::cache_dir().join("shots").join(format!("{id}-{stamp}.png")), |p| request.cwd.join(p));
                screenshot(&page.view, path, reply);
            }
            Command::Tabs | Command::New(_) | Command::Close => unreachable!("answered above"),
        }
    }

    /// Marks tab `id` as driven by the agent in `pane` for `DRIVEN`, then lets it go with a render.
    fn drive(&mut self, id: String, pane: Option<String>, cx: &mut Context<Self>) {
        self.browser.driven.insert(id, (Instant::now(), pane.unwrap_or_default()));
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(super::browser::DRIVEN).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    /// The row under a driven tab's address bar: which agent drives it. Not over the page, whose native view would
    /// cover it.
    pub(super) fn render_agent_driving(&self, id: &str, theme: &Theme) -> Option<Div> {
        let pane = self.browser.driven_by(id)?;
        let label = match pane {
            "" => "An agent is driving this tab".to_string(),
            pane => format!("An agent is driving this tab · {pane}"),
        };
        Some(
            div()
                .h(px(26.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .gap(px(6.))
                .bg(theme.accent.opacity(0.12))
                .border_b_1()
                .border_color(theme.accent.opacity(0.35))
                .text_size(px(12.))
                .text_color(theme.text)
                .child(gpui::svg().path("icons/bot.svg").size(px(14.)).text_color(theme.accent))
                .child(label),
        )
    }

    /// `SHEPHERD_PANEL=browser-agent`: a blank tab as an agent's command leaves it (screenshots).
    pub fn show_agent_driving(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_blank_tab(None, window, cx);
        if let Some(id) = self.browser.tabs.last().map(|t| t.id.clone()) {
            self.drive(id, Some("w1:p1".into()), cx);
        }
    }

    /// The page a command goes to: `--tab`'s; else the browser tab the agent's workspace shows in its panel, else its
    /// last one; outside a workspace (no `HERDR_WORKSPACE_ID`), the page the window shows.
    fn agent_page(&self, tab: Option<&str>, ws: Option<&str>, window: &Window, cx: &App) -> Result<PageKey, String> {
        if let Some(id) = tab {
            return self.agent_keys().find(|k| seen_key(k) == id).ok_or(format!("no tab {id}: `shepherd browser tabs` lists them"));
        }
        let Some(ws) = ws else {
            return self.visible_page(window, cx).filter(|k| self.device_page_session(&k.0).is_none()).ok_or("no page shown in Shepherd: pass --tab <id> (`shepherd browser tabs`)".into());
        };
        let shown = self.right.panels.get(ws).and_then(|p| match &p.active {
            Some(Surface::Browser(id)) => Some(id.clone()),
            _ => None,
        });
        shown
            .or_else(|| self.browser.tabs.iter().rev().find(|t| t.workspace == ws).map(|t| t.id.clone()))
            .map(|id| (id, None))
            .ok_or("no browser tab in this workspace: `shepherd browser new <url>` opens one; `shepherd browser tabs` lists the others, for --tab <id>".into())
    }

    /// The live pages an agent may drive: every one but devices' 3D views.
    fn agent_keys(&self) -> impl Iterator<Item = PageKey> + '_ {
        self.pages.live.keys().filter(|k| self.device_page_session(&k.0).is_none()).cloned()
    }

    fn agent_tabs(&self, ws: Option<&str>, window: &Window, cx: &App) -> Reply {
        let shown = self.visible_page(window, cx);
        let label = |id: &str| self.snapshot.as_ref()?.workspaces.iter().find(|w| w.workspace_id == id).map(|w| w.label.clone());
        let mut tabs: Vec<serde_json::Value> = self
            .agent_keys()
            .map(|key| {
                let page = &self.pages.live[&key];
                let workspace = self.browser.tabs.iter().find(|t| t.id == key.0).map(|t| t.workspace.clone());
                json!({
                    "id": seen_key(&key),
                    "kind": if workspace.is_some() { "tab" } else { "sidebar" },
                    "workspace": workspace,
                    "workspace_name": workspace.as_deref().and_then(label),
                    "url": super::web::page_url(&page.view),
                    "title": page.title,
                    "shown": shown.as_ref() == Some(&key),
                    "this_workspace": workspace.is_some() && workspace.as_deref() == ws,
                })
            })
            .collect();
        tabs.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        Reply::ok(serde_json::to_string_pretty(&tabs).unwrap_or_default())
    }

    /// A new browser tab on `url` in the agent's workspace, else the one shown.
    fn agent_new_tab(&mut self, ws: Option<&str>, url: &str, window: &mut Window, cx: &mut Context<Self>) -> Reply {
        let Some(ws) = ws.map(str::to_string).or_else(|| self.shown_tab().map(|t| t.workspace_id)) else { return Reply::err("no workspace to open it in") };
        if !self.snapshot.as_ref().is_some_and(|s| s.workspaces.iter().any(|w| w.workspace_id == ws)) {
            return Reply::err(format!("workspace {ws} isn't the one Shepherd shows the session of"));
        }
        let url = super::browser::typed_url(url).unwrap_or_else(|| url.to_string());
        let before = self.browser.tabs.len();
        self.new_browser_tab(ws, Some(url), false, window, cx);
        match self.browser.tabs.get(before) {
            Some(tab) => Reply::ok(tab.id.clone()),
            None => Reply::err("the page couldn't be made"),
        }
    }
}

/// Runs `script` in the page and hands back what wry says it returned (JSON).
async fn js(view: &Weak<wry::WebView>, script: &str, cx: &mut gpui::AsyncApp) -> Result<String, String> {
    let view = view.upgrade().ok_or("the tab closed")?;
    let (tx, rx) = oneshot::channel();
    let tx = std::sync::Mutex::new(Some(tx));
    view.evaluate_script_with_callback(script, move |result| {
        if let Some(tx) = tx.lock().ok().and_then(|mut t| t.take()) {
            let _ = tx.send(result);
        }
    })
    .map_err(|e| e.to_string())?;
    drop(view);
    let timer = cx.background_executor().timer(ANSWER);
    match futures::future::select(rx, Box::pin(timer)).await {
        futures::future::Either::Left((Ok(result), _)) => Ok(result),
        _ => Err("the page didn't answer (still loading?)".into()),
    }
}

/// A script whose result is the text to print.
fn read(view: Weak<wry::WebView>, script: String, reply: oneshot::Sender<Reply>, cx: &mut Context<Shell>) {
    cx.spawn(async move |_, cx| {
        let answer = match js(&view, &script, cx).await {
            Ok(raw) => js_string(&raw).map_or_else(|| Reply::err("the page didn't answer"), Reply::ok),
            Err(e) => Reply::err(e),
        };
        let _ = reply.send(answer);
    })
    .detach();
}

/// Starts `code` (as an expression, else as statements), then asks every `POLL` for its outcome until `timeout`.
async fn eval(view: Weak<wry::WebView>, code: String, timeout: Duration, cx: &mut gpui::AsyncApp) -> Reply {
    static RUNS: AtomicU64 = AtomicU64::new(1);
    let id = RUNS.fetch_add(1, Ordering::Relaxed);
    let mut started = false;
    for expression in [true, false] {
        match js(&view, &crate::browser_agent::eval_script(id, &code, expression), cx).await {
            Ok(raw) if js_string(&raw).as_deref() == Some("started") => {
                started = true;
                break;
            }
            Ok(_) => {}
            Err(e) => return Reply::err(e),
        }
    }
    if !started {
        return Reply::err("the code doesn't parse, as an expression or as statements (a SyntaxError)");
    }
    let until = Instant::now() + timeout;
    loop {
        cx.background_executor().timer(POLL).await;
        let outcome = match js(&view, &crate::browser_agent::poll_script(id), cx).await {
            Ok(raw) => js_string(&raw).and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()),
            Err(e) => return Reply::err(e),
        };
        let Some(outcome) = outcome else { return Reply::err("the page didn't answer") };
        if outcome.is_null() {
            if Instant::now() > until {
                return Reply::err(format!("no result in {}s: the code still runs (--timeout)", timeout.as_secs_f64()));
            }
            continue;
        }
        if outcome["gone"] == true {
            return Reply::err("the page navigated away before the code finished");
        }
        let text = outcome["text"].as_str().unwrap_or_default().to_string();
        return Reply { ok: outcome["ok"] == true, out: text };
    }
}

/// The page as WebKit draws it, a PNG at `path`.
#[cfg(target_os = "macos")]
fn screenshot(view: &wry::WebView, path: std::path::PathBuf, reply: oneshot::Sender<Reply>) {
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
    use objc2_foundation::{NSDictionary, NSError};
    use wry::WebViewExtMacOS;
    let reply = std::cell::RefCell::new(Some(reply));
    let done = block2::RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
        // SAFETY: WebKit hands an image, or an error, live for the block's call
        let (image, error) = unsafe { (image.as_ref(), error.as_ref()) };
        let saved = match image {
            Some(image) => image
                .TIFFRepresentation()
                .and_then(|tiff| NSBitmapImageRep::imageRepWithData(&tiff))
                // SAFETY: an empty dictionary is valid properties for any file type
                .and_then(|rep| unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) })
                .ok_or_else(|| "the snapshot couldn't be made a PNG".to_string())
                .and_then(|png| {
                    let _ = path.parent().map(std::fs::create_dir_all);
                    std::fs::write(&path, png.to_vec()).map_err(|e| format!("{}: {e}", path.display()))
                }),
            None => Err(error.map_or("WebKit made no snapshot".into(), |e| e.localizedDescription().to_string())),
        };
        if let Some(reply) = reply.borrow_mut().take() {
            let _ = reply.send(match saved {
                Ok(()) => Reply::ok(path.display().to_string()),
                Err(e) => Reply::err(e),
            });
        }
    });
    // SAFETY: on a live WKWebView, on the main thread; no configuration shoots the whole visible page
    unsafe { view.webview().takeSnapshotWithConfiguration_completionHandler(None, &done) };
}

// ponytail: WebKitGTK and WebView2 have their own snapshot calls, not wired up
#[cfg(not(target_os = "macos"))]
fn screenshot(_: &wry::WebView, _: std::path::PathBuf, reply: oneshot::Sender<Reply>) {
    let _ = reply.send(Reply::err("screenshot is macOS only for now"));
}
