//! The window's side of `shepherd browser …` (`crate::browser_agent`): which page a command is for, and what it does
//! there. A script's result comes back through wry's callback; an eval's, which may await, by polling the page.
use super::*;
use super::web::{PageKey, seen_key};
use crate::browser_agent::{Asked, Command, Reply, Request, Until, js_string};
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
            Command::Screenshot { path, full, element } => {
                let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
                let path = path.map_or_else(|| crate::settings::cache_dir().join("shots").join(format!("{id}-{stamp}.png")), |p| request.cwd.join(p));
                cx.spawn(async move |_, cx| drop(reply.send(screenshot(view, path, full, element, cx).await))).detach();
            }
            Command::Act(script) => act(view, script, reply, cx),
            Command::Wait { until, timeout } => {
                cx.spawn(async move |_, cx| drop(reply.send(wait(view, until, timeout, cx).await))).detach();
            }
            Command::Upload { target, files } => match upload_call(&target, &files, &request.cwd) {
                Ok(script) => act(view, script, reply, cx),
                Err(e) => drop(reply.send(Reply::err(e))),
            },
            Command::Viewport(size) => {
                page.viewport.set(size);
                page.viewport_zoom.set(None);
                // placed again on the next frame, at the new size
                page.placed.set(None);
                if size.is_none() {
                    let _ = page.view.zoom(self.browser.zoom.get(&key.0).copied().unwrap_or(1.));
                }
                cx.notify();
                drop(reply.send(Reply::ok(match size {
                    Some((w, h)) => format!("{id}: laid out at {w}x{h}, scaled down to fit the panel if it doesn't (`viewport reset` takes it back)"),
                    None => format!("{id}: the panel's size again"),
                })));
            }
            Command::ClearStorage => {
                // the host of scheme://host:port/…
                let host = super::web::page_url(&page.view).and_then(|u| {
                    let rest = u.split_once("://")?.1;
                    let host = rest.split(['/', '?', '#']).next()?.rsplit('@').next()?;
                    let host = if host.starts_with('[') { host } else { host.split(':').next()? };
                    (!host.is_empty()).then(|| host.to_string())
                });
                let Some(host) = host else { return drop(reply.send(Reply::err("the page has no site to clear"))) };
                #[cfg(target_os = "macos")]
                {
                    super::browser::webkit::clear_site_data(&page.view, &super::browser::webkit::ALL, host.clone());
                    drop(reply.send(Reply::ok(format!("{id}: cleared what WebKit keeps for {host}: cookies, storage, IndexedDB, service workers, caches; reload to start afresh"))));
                }
                // ponytail: what the page's scripts reach; WebKitGTK's store clears per site only through its own API
                #[cfg(not(target_os = "macos"))]
                act(view, crate::browser_agent::tools_call(CLEAR_SCRIPT), reply, cx);
                let _ = host;
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

/// A tools script (`tools_call`): its `{"ok", "text"}` as the reply.
fn act(view: Weak<wry::WebView>, script: String, reply: oneshot::Sender<Reply>, cx: &mut Context<Shell>) {
    cx.spawn(async move |_, cx| {
        let answer = match js(&view, &script, cx).await {
            Ok(raw) => js_string(&raw).and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()).map_or_else(
                || Reply::err("the page didn't answer"),
                |v| Reply { ok: v["ok"] == true, out: v["text"].as_str().unwrap_or_default().to_string() },
            ),
            Err(e) => Reply::err(e),
        };
        let _ = reply.send(answer);
    })
    .detach();
}

/// `storage clear` where WebKit's store can't be asked per site: what the page's scripts reach.
#[cfg(not(target_os = "macos"))]
const CLEAR_SCRIPT: &str = "(localStorage.clear(), sessionStorage.clear(), document.cookie.split('; ').forEach(c => { document.cookie = c.split('=')[0] + '=; expires=Thu, 01 Jan 1970 00:00:00 GMT; path=/'; }), \
     'cleared local and session storage and the cookies scripts see (not HttpOnly ones); reload to start afresh')";

/// Waits for `until`, asking the page every `WAIT_POLL`; a page that doesn't answer (navigating) is asked again.
async fn wait(view: Weak<wry::WebView>, until: Until, timeout: Duration, cx: &mut gpui::AsyncApp) -> Reply {
    const WAIT_POLL: Duration = Duration::from_millis(150);
    let start = Instant::now();
    let (what, script) = match until {
        Until::Time(d) => {
            cx.background_executor().timer(d).await;
            return Reply::ok(format!("waited {}ms", d.as_millis()));
        }
        Until::Check { what, script } => (what, script),
    };
    loop {
        if view.upgrade().is_none() {
            return Reply::err("the tab closed");
        }
        if let Ok(raw) = js(&view, &script, cx).await
            && js_string(&raw).as_deref() == Some("yes")
        {
            return Reply::ok(format!("{what} after {:.1}s", start.elapsed().as_secs_f64()));
        }
        if start.elapsed() > timeout {
            return Reply::err(format!("no {what} in {}s (--timeout)", timeout.as_secs_f64()));
        }
        cx.background_executor().timer(WAIT_POLL).await;
    }
}

/// `upload`'s call: the files read from disk (relative to `cwd`), in the page as base64.
fn upload_call(target: &str, files: &[std::path::PathBuf], cwd: &std::path::Path) -> Result<String, String> {
    use base64::Engine;
    let mut list = Vec::new();
    for f in files {
        let path = cwd.join(f);
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let kind = match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("png") => "image/png",
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("gif") => "image/gif",
            Some("webp") => "image/webp",
            Some("svg") => "image/svg+xml",
            Some("pdf") => "application/pdf",
            Some("json") => "application/json",
            Some("txt" | "md") => "text/plain",
            Some("csv") => "text/csv",
            Some("html") => "text/html",
            Some("zip") => "application/zip",
            _ => "application/octet-stream",
        };
        list.push(json!({ "name": name, "type": kind, "data": base64::engine::general_purpose::STANDARD.encode(bytes) }));
    }
    Ok(crate::browser_agent::tools_call(&format!("T.upload({}, {})", serde_json::to_string(target).unwrap_or_default(), serde_json::Value::Array(list))))
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

/// The page as a PNG at `path`: what shows, the whole page (pieces shot as it scrolls, stitched), or `element`.
async fn screenshot(view: Weak<wry::WebView>, path: std::path::PathBuf, full: bool, element: Option<String>, cx: &mut gpui::AsyncApp) -> Reply {
    let png = match (full, element) {
        (_, Some(target)) => element_shot(&view, &target, cx).await,
        (true, None) => full_shot(&view, cx).await,
        (false, None) => snap(&view, None).await,
    };
    let saved = png.and_then(|png| {
        let _ = path.parent().map(std::fs::create_dir_all);
        std::fs::write(&path, png).map_err(|e| format!("{}: {e}", path.display()))
    });
    match saved {
        Ok(()) => Reply::ok(path.display().to_string()),
        Err(e) => Reply::err(e),
    }
}

/// A tools call's text, or its error.
async fn tool(view: &Weak<wry::WebView>, call: &str, cx: &mut gpui::AsyncApp) -> Result<String, String> {
    let raw = js(view, &crate::browser_agent::tools_call(call), cx).await?;
    let v = js_string(&raw).and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()).ok_or("the page didn't answer")?;
    let text = v["text"].as_str().unwrap_or_default().to_string();
    if v["ok"] == true { Ok(text) } else { Err(text) }
}

/// One element, scrolled to: the part of it the panel shows.
async fn element_shot(view: &Weak<wry::WebView>, target: &str, cx: &mut gpui::AsyncApp) -> Result<Vec<u8>, String> {
    let rect: serde_json::Value = serde_json::from_str(&tool(view, &format!("T.rect({})", serde_json::to_string(target).unwrap_or_default()), cx).await?).map_err(|e| e.to_string())?;
    let n = |k: &str| rect[k].as_f64().unwrap_or(0.);
    // its scroll lands before the next frame
    cx.background_executor().timer(Duration::from_millis(100)).await;
    // the web view's points per CSS pixel: 1 unless zoomed
    let scale = view_width(view).map_or(1., |w| w / n("vw").max(1.));
    let (x, y) = (n("x").max(0.), n("y").max(0.));
    let (w, h) = ((n("x") + n("w")).min(n("vw")) - x, (n("y") + n("h")).min(n("vh")) - y);
    if w <= 0. || h <= 0. {
        return Err(format!("{target} has no size on screen"));
    }
    snap(view, Some([x * scale, y * scale, w * scale, h * scale])).await
}

/// The whole page, a screen at a time as it scrolls, stitched; fixed and sticky elements only in the first piece.
// ponytail: the page's own scroll only, not a box an app scrolls inside it; 40 screens at most
async fn full_shot(view: &Weak<wry::WebView>, cx: &mut gpui::AsyncApp) -> Result<Vec<u8>, String> {
    #[derive(serde::Deserialize)]
    struct At {
        top: f64,
        height: f64,
        vw: f64,
        vh: f64,
    }
    let at = |s: String| serde_json::from_str::<At>(&s).map_err(|e| e.to_string());
    let first = at(tool(view, "T.page()", cx).await?)?;
    let mut pieces = Vec::new();
    let mut top = 0.;
    let shot = loop {
        let now = at(tool(view, &format!("T.page({top})"), cx).await?)?;
        cx.background_executor().timer(Duration::from_millis(150)).await;
        let piece = match snap(view, None).await.and_then(|png| image::load_from_memory(&png).map_err(|e| e.to_string())) {
            Ok(piece) => piece.to_rgba8(),
            Err(e) => break Err(e),
        };
        pieces.push((now.top, piece));
        if pieces.len() == 1 {
            let _ = tool(view, "T.fixed(true)", cx).await;
        }
        if now.top + now.vh >= now.height - 1. || pieces.len() >= 40 {
            break Ok(now.top + now.vh);
        }
        top = now.top + now.vh;
    };
    let _ = tool(view, "T.fixed(false)", cx).await;
    let _ = tool(view, &format!("T.page({})", first.top), cx).await;
    let bottom = shot?;
    let scale = f64::from(pieces[0].1.width()) / first.vw.max(1.);
    let mut canvas = image::RgbaImage::new(pieces[0].1.width(), (bottom * scale).round() as u32);
    for (top, piece) in &pieces {
        image::imageops::replace(&mut canvas, piece, 0, (top * scale).round() as i64);
    }
    let mut png = std::io::Cursor::new(Vec::new());
    canvas.write_to(&mut png, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(png.into_inner())
}

/// The web view's width in points.
#[cfg(target_os = "macos")]
fn view_width(view: &Weak<wry::WebView>) -> Option<f64> {
    use wry::WebViewExtMacOS;
    Some(view.upgrade()?.webview().frame().size.width)
}

#[cfg(not(target_os = "macos"))]
fn view_width(_: &Weak<wry::WebView>) -> Option<f64> {
    None
}

/// What WebKit draws of the page as a PNG: `rect` (x, y, w, h in the web view's points, from its top left), else
/// what shows.
#[cfg(target_os = "macos")]
async fn snap(view: &Weak<wry::WebView>, rect: Option<[f64; 4]>) -> Result<Vec<u8>, String> {
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
    use objc2_foundation::{NSDictionary, NSError, NSPoint, NSRect, NSSize};
    use wry::WebViewExtMacOS;
    let view = view.upgrade().ok_or("the tab closed")?;
    let (tx, rx) = oneshot::channel::<Result<Vec<u8>, String>>();
    let tx = std::cell::RefCell::new(Some(tx));
    let done = block2::RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
        // SAFETY: WebKit hands an image, or an error, live for the block's call
        let (image, error) = unsafe { (image.as_ref(), error.as_ref()) };
        let png = match image {
            Some(image) => image
                .TIFFRepresentation()
                .and_then(|tiff| NSBitmapImageRep::imageRepWithData(&tiff))
                // SAFETY: an empty dictionary is valid properties for any file type
                .and_then(|rep| unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) })
                .map(|png| png.to_vec())
                .ok_or_else(|| "the snapshot couldn't be made a PNG".to_string()),
            None => Err(error.map_or("WebKit made no snapshot".into(), |e| e.localizedDescription().to_string())),
        };
        if let Some(tx) = tx.borrow_mut().take() {
            let _ = tx.send(png);
        }
    });
    let configuration = rect.map(|[x, y, w, h]| {
        let mtm = objc2::MainThreadMarker::new().expect("agents' commands run on the main thread");
        // SAFETY: a new configuration on the main thread, given a rect in the web view's own (top-left) coordinates
        unsafe {
            let c = objc2_web_kit::WKSnapshotConfiguration::new(mtm);
            c.setRect(NSRect::new(NSPoint::new(x, y), NSSize::new(w, h)));
            c
        }
    });
    // SAFETY: on a live WKWebView, on the main thread; no configuration shoots the whole visible page
    unsafe { view.webview().takeSnapshotWithConfiguration_completionHandler(configuration.as_deref(), &done) };
    drop(view);
    rx.await.unwrap_or_else(|_| Err("WebKit made no snapshot".into()))
}

// ponytail: WebKitGTK and WebView2 have their own snapshot calls, not wired up
#[cfg(not(target_os = "macos"))]
async fn snap(_: &Weak<wry::WebView>, _: Option<[f64; 4]>) -> Result<Vec<u8>, String> {
    Err("screenshot is macOS only for now".into())
}
