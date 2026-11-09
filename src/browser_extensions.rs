//! Chrome extensions in the browser tabs: added in Settings → Extensions → Browser, from the Chrome Web Store (its CRX
//! downloaded and unpacked into a folder beside the settings) or from a folder the user picks, and loaded into every
//! browser tab by WebKit's own WebExtensions support (`WKWebExtensionController`, macOS 15.4 and later).
//! The settings keep each one's folder and switch (`Settings::browser_extensions`); the rest is read from its manifest.
use crate::settings::BrowserExtension;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The Chrome the Web Store is told it serves: some extensions ask for a recent one, and the store refuses older ones.
const CHROME_VERSION: &str = "140.0.0.0";

/// Why extensions can't load on this computer, or `None` when they can.
pub fn unavailable() -> Option<&'static str> {
    #[cfg(target_os = "macos")]
    return (!webkit::supported()).then_some("Browser extensions need macOS 15.4 or later, where WebKit runs them.");
    #[cfg(not(target_os = "macos"))]
    Some("Browser extensions run on macOS only, for now.")
}

/// Run in browser tabs: on a Web Store page of an extension, the store's own button (Add to Chrome, which it greys
/// out off Chrome) says Add to Shepherd and goes to `shepherd://install-extension/<id>`, which installs it as Settings
/// does. The store is a single-page app: the button is looked for again on each change of the page.
// ponytail: the button is the one whose short label names Chrome, as the store writes it in every language; a store
// redesign that drops the word needs a new match
pub const STORE_SCRIPT: &str = r#"(() => {
  if (location.host !== 'chromewebstore.google.com') return;
  const id = () => location.pathname.match(/\/detail\/(?:[^/]+\/)?([a-p]{32})/)?.[1];
  const dress = () => {
    if (!id()) return;
    for (const b of document.querySelectorAll('button:not([data-shepherd])')) {
      const label = [...b.querySelectorAll('span')].find((s) => !s.children.length && /\bChrome\b/.test(s.textContent));
      if (!label || b.textContent.trim().length > 40) continue;
      b.dataset.shepherd = '';
      b.disabled = false;
      b.removeAttribute('aria-disabled');
      label.textContent = label.textContent.replace('Chrome', 'Shepherd');
    }
  };
  addEventListener('click', (e) => {
    if (!e.target.closest?.('button[data-shepherd]') || !id()) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    location.href = 'shepherd://install-extension/' + id();
  }, true);
  new MutationObserver(dress).observe(document, { childList: true, subtree: true });
})();"#;

/// The id a `shepherd://install-extension/<id>` link asks for (see [`STORE_SCRIPT`]).
pub fn install_link(url: &str) -> Option<String> {
    store_id(url.strip_prefix("shepherd://install-extension/")?)
}

/// Where extensions from the Web Store are unpacked: beside the settings, so a second Shepherd with settings of its
/// own (`SHEPHERD_SETTINGS_FILE`) keeps its own.
pub fn dir() -> PathBuf {
    let settings = crate::settings::Settings::path();
    settings.parent().map_or_else(crate::settings::config_dir, Path::to_path_buf).join("browser-extensions")
}

/// An extension's id in a Web Store link (`chromewebstore.google.com/detail/<slug>/<id>`, or the old
/// `chrome.google.com/webstore/detail/…`), or typed as is: 32 letters from a to p.
pub fn store_id(text: &str) -> Option<String> {
    let is_id = |s: &str| s.len() == 32 && s.bytes().all(|b| (b'a'..=b'p').contains(&b));
    let path = text.trim().split(['?', '#']).next().unwrap_or_default();
    path.split('/').rev().find(|s| is_id(s)).map(str::to_string)
}

/// A CRX's ZIP: what follows its header, CRX3's (`Cr24`, version, header size, a protobuf of signatures) or CRX2's
/// (`Cr24`, version, key and signature sizes, then both).
// ponytail: the signatures aren't checked; the download is HTTPS from Google, and the user chose the extension
pub fn strip_crx(crx: &[u8]) -> Result<&[u8], String> {
    let word = |at: usize| crx.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize);
    if !crx.starts_with(b"Cr24") {
        return Err("not a Chrome extension (CRX) file".into());
    }
    let start = match word(4) {
        Some(3) => word(8).map(|header| 12 + header),
        Some(2) => word(8).zip(word(12)).map(|(key, sig)| 16 + key + sig),
        _ => None,
    };
    start.and_then(|at| crx.get(at..)).filter(|zip| zip.starts_with(b"PK")).ok_or_else(|| "a CRX of a version or shape Shepherd doesn't read".into())
}

/// Downloads Web Store extension `id`, unpacks it into `dir()/<id>` (over an older copy) and returns that folder.
pub fn install_from_store(id: &str) -> Result<PathBuf, String> {
    let url = format!(
        "https://clients2.google.com/service/update2/crx?response=redirect&prodversion={CHROME_VERSION}&acceptformat=crx3&x=id%3D{id}%26installsource%3Dondemand%26uc"
    );
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(15))).timeout_recv_response(Some(Duration::from_secs(30))).build().into();
    let crx = match agent.get(&url).call() {
        Ok(mut r) => r.body_mut().with_config().limit(256 << 20).read_to_vec().map_err(|e| format!("the download broke off: {e}"))?,
        Err(ureq::Error::StatusCode(204 | 404)) => return Err(format!("the Chrome Web Store has no extension {id}")),
        Err(e) => return Err(format!("the Chrome Web Store didn't answer: {e}")),
    };
    // the store answers an unknown id with an empty body
    if crx.is_empty() {
        return Err(format!("the Chrome Web Store has no extension {id}"));
    }
    let zip = strip_crx(&crx)?;
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(dir()).map_err(|e| e.to_string())?;
    std::io::Write::write_all(&mut file, zip).map_err(|e| e.to_string())?;
    let (unpacked, folder) = (dir().join(format!(".{id}.new")), dir().join(id));
    let _ = std::fs::remove_dir_all(&unpacked);
    // ponytail: macOS's ditto unpacks it, as extensions load only there; a zip crate if they come to another system
    let out = std::process::Command::new("ditto").arg("-x").arg("-k").arg(file.path()).arg(&unpacked).output().map_err(|e| format!("ditto didn't run: {e}"))?;
    if !out.status.success() {
        let _ = std::fs::remove_dir_all(&unpacked);
        return Err(format!("the extension didn't unpack: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::rename(&unpacked, &folder).map_err(|e| e.to_string())?;
    info(&folder)?;
    Ok(folder)
}

/// Removes an extension's folder when Shepherd unpacked it; a folder the user picked stays where it is.
pub fn remove_files(path: &Path) {
    if path.starts_with(dir()) {
        let _ = std::fs::remove_dir_all(path);
    }
}

/// What Settings shows of an extension, from its manifest.
#[derive(Clone)]
pub struct Info {
    pub name: String,
    pub version: String,
    pub description: String,
    /// Its largest PNG icon.
    pub icon: Option<Vec<u8>>,
}

/// Reads `folder`'s `manifest.json`, its name and description in its default language when they are `__MSG_…__`.
pub fn info(folder: &Path) -> Result<Info, String> {
    let read = |name: &str| std::fs::read_to_string(folder.join(name)).ok().and_then(|s| serde_json::from_str::<serde_json::Value>(s.trim_start_matches('\u{feff}')).ok());
    let manifest = read("manifest.json").ok_or_else(|| format!("no readable manifest.json in {}", folder.display()))?;
    let messages = manifest["default_locale"].as_str().and_then(|locale| read(&format!("_locales/{locale}/messages.json")));
    let text = |key: &str| {
        let value = manifest[key].as_str().unwrap_or_default();
        let message = value.strip_prefix("__MSG_").and_then(|v| v.strip_suffix("__")).and_then(|name| {
            // message names are matched without regard to case
            let messages = messages.as_ref()?.as_object()?;
            messages.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).and_then(|(_, m)| m["message"].as_str())
        });
        message.unwrap_or(value).to_string()
    };
    let icons = manifest["icons"].as_object().or_else(|| manifest["action"]["default_icon"].as_object());
    let largest = icons.and_then(|icons| icons.iter().filter_map(|(size, path)| Some((size.parse::<u32>().ok()?, path.as_str()?))).max_by_key(|(size, _)| *size));
    let icon = largest.filter(|(_, path)| path.ends_with(".png")).and_then(|(_, path)| std::fs::read(folder.join(path.trim_start_matches('/'))).ok());
    Ok(Info { name: text("name"), version: manifest["version"].as_str().unwrap_or_default().to_string(), description: text("description"), icon })
}

/// Loads the extensions switched on and unloads the rest, in the tabs made from now on (and, for WebKit, in the
/// tabs already open as their pages next load). Nothing until a browser tab is made.
pub fn sync(list: &[BrowserExtension]) {
    #[cfg(target_os = "macos")]
    webkit::sync(list);
    #[cfg(not(target_os = "macos"))]
    let _ = list;
}

/// Why extension `path` didn't load, when it didn't.
pub fn error(path: &Path) -> Option<String> {
    #[cfg(target_os = "macos")]
    return webkit::error(path);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        None
    }
}

#[cfg(target_os = "macos")]
pub use webkit::{actions, activate_tab, configuration, opened_tab, perform_action};

/// A browser tab as extensions know it, from [`opened_tab`] on; dropped with its page, it tells them it closed.
pub struct TabMark(#[cfg_attr(not(target_os = "macos"), allow(dead_code))] String);

#[cfg(not(target_os = "macos"))]
pub fn opened_tab(id: &str, _: &wry::WebView) -> TabMark {
    TabMark(id.to_string())
}

#[cfg(not(target_os = "macos"))]
pub fn activate_tab(_: &str) {}

#[cfg(target_os = "macos")]
impl Drop for TabMark {
    fn drop(&mut self) {
        webkit::closed_tab(&self.0);
    }
}

/// WebKit's side: one `WKWebExtensionController` for every browser tab, an extension context in it per extension.
#[cfg(target_os = "macos")]
mod webkit {
    use crate::settings::BrowserExtension;
    use block2::RcBlock;
    use objc2::rc::{Retained, Weak};
    use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_foundation::{NSArray, NSError, NSOperatingSystemVersion, NSProcessInfo, NSString, NSURL};
    use objc2_web_kit::{
        WKWebExtension, WKWebExtensionAction, WKWebExtensionContext, WKWebExtensionContextPermissionStatus, WKWebExtensionController, WKWebExtensionControllerConfiguration,
        WKWebExtensionControllerDelegate, WKWebExtensionTab, WKWebExtensionTabConfiguration, WKWebExtensionWindow, WKWebView, WKWebViewConfiguration, WKWebsiteDataStore,
    };
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    #[derive(Default)]
    struct State {
        controller: Option<Retained<WKWebExtensionController>>,
        /// The extensions switched on, as the settings last said.
        wanted: Vec<PathBuf>,
        loaded: HashMap<PathBuf, Retained<WKWebExtensionContext>>,
        /// Being read from their folders.
        reading: HashSet<PathBuf>,
        errors: HashMap<PathBuf, String>,
        /// Each loaded extension's action icon, once drawn.
        icons: HashMap<PathBuf, Option<Arc<gpui::Image>>>,
        /// Told when an extension has loaded or failed to: WebKit reads them after the tab is drawn, so the browser
        /// bar's buttons and Settings' "Didn't load" would otherwise wait for some other redraw.
        settled: Option<futures::channel::mpsc::UnboundedSender<()>>,
        /// The browser tabs, in the order they opened, by their page id.
        tabs: Vec<(String, Retained<Tab>)>,
        /// The one the panel shows or last showed, the active tab extensions act on.
        active: Option<String>,
        /// Every browser tab is in this one window, as extensions see them; it and the delegate live as long as the
        /// controller (WebKit holds its delegate weakly).
        window: Option<Retained<ExtWindow>>,
        delegate: Option<Retained<Delegate>>,
        /// URLs extensions asked a new tab for (`tabs.create`), for the Shell to open.
        new_tabs: Option<futures::channel::mpsc::UnboundedSender<String>>,
    }

    struct TabIvars {
        view: Weak<WKWebView>,
    }

    // What WebKit asks of a tab beyond these (its URL, title, loading) it reads from the web view itself.
    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[ivars = TabIvars]
        struct Tab;

        unsafe impl NSObjectProtocol for Tab {}

        unsafe impl WKWebExtensionTab for Tab {
            #[unsafe(method_id(windowForWebExtensionContext:))]
            fn window_for(&self, _: &WKWebExtensionContext) -> Option<Retained<ProtocolObject<dyn WKWebExtensionWindow>>> {
                window()
            }

            #[unsafe(method_id(webViewForWebExtensionContext:))]
            fn web_view_for(&self, _: &WKWebExtensionContext) -> Option<Retained<WKWebView>> {
                self.ivars().view.load()
            }

            #[unsafe(method(isSelectedForWebExtensionContext:))]
            fn is_selected(&self, _: &WKWebExtensionContext) -> bool {
                active_tab().is_some_and(|t| std::ptr::eq(&*t, self))
            }
        }
    );

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        struct ExtWindow;

        unsafe impl NSObjectProtocol for ExtWindow {}

        unsafe impl WKWebExtensionWindow for ExtWindow {
            #[unsafe(method_id(tabsForWebExtensionContext:))]
            fn tabs_for(&self, _: &WKWebExtensionContext) -> Retained<NSArray<ProtocolObject<dyn WKWebExtensionTab>>> {
                let tabs: Vec<_> = STATE.with_borrow(|s| s.tabs.iter().map(|(_, t)| ProtocolObject::from_retained(t.clone())).collect());
                NSArray::from_retained_slice(&tabs)
            }

            #[unsafe(method_id(activeTabForWebExtensionContext:))]
            fn active_tab_for(&self, _: &WKWebExtensionContext) -> Option<Retained<ProtocolObject<dyn WKWebExtensionTab>>> {
                active_tab().map(ProtocolObject::from_retained)
            }
        }
    );

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl WKWebExtensionControllerDelegate for Delegate {
            #[unsafe(method_id(webExtensionController:openWindowsForExtensionContext:))]
            fn open_windows(&self, _: &WKWebExtensionController, _: &WKWebExtensionContext) -> Retained<NSArray<ProtocolObject<dyn WKWebExtensionWindow>>> {
                NSArray::from_retained_slice(&window().into_iter().collect::<Vec<_>>())
            }

            #[unsafe(method_id(webExtensionController:focusedWindowForExtensionContext:))]
            fn focused_window(&self, _: &WKWebExtensionController, _: &WKWebExtensionContext) -> Option<Retained<ProtocolObject<dyn WKWebExtensionWindow>>> {
                window()
            }

            // ponytail: the tab opens in the panel a moment later; the extension is answered with none, so what it
            // does with the new tab right away (tabs.update on it) is lost
            #[unsafe(method(webExtensionController:openNewTabUsingConfiguration:forExtensionContext:completionHandler:))]
            fn open_new_tab(
                &self,
                _: &WKWebExtensionController,
                config: &WKWebExtensionTabConfiguration,
                _: &WKWebExtensionContext,
                done: &block2::DynBlock<dyn Fn(*mut ProtocolObject<dyn WKWebExtensionTab>, *mut NSError)>,
            ) {
                let url = unsafe { config.url() }.and_then(|u| u.absoluteString()).map(|s| s.to_string());
                if let Some(url) = url {
                    STATE.with_borrow(|s| s.new_tabs.as_ref().map(|tx| tx.unbounded_send(url)));
                }
                done.call((std::ptr::null_mut(), std::ptr::null_mut()));
            }
        }
    );

    fn window() -> Option<Retained<ProtocolObject<dyn WKWebExtensionWindow>>> {
        STATE.with_borrow(|s| s.window.clone()).map(ProtocolObject::from_retained)
    }

    fn active_tab() -> Option<Retained<Tab>> {
        STATE.with_borrow(|s| s.active.as_ref().and_then(|id| s.tabs.iter().find(|(i, _)| i == id)).map(|(_, t)| t.clone()))
    }

    /// Tells the extensions about browser tab `id`, its page in `view`. WebKit asks the tab back while it is told, so
    /// the state is never borrowed across a call to the controller.
    pub fn opened_tab(id: &str, view: &wry::WebView) -> super::TabMark {
        use wry::WebViewExtMacOS;
        let mark = super::TabMark(id.to_string());
        let Some(mtm) = MainThreadMarker::new() else { return mark };
        let Some(controller) = STATE.with_borrow(|s| s.controller.clone()) else { return mark };
        let web_view = view.webview();
        let this = Tab::alloc(mtm).set_ivars(TabIvars { view: Weak::new(&**web_view) });
        let tab: Retained<Tab> = unsafe { msg_send![super(this), init] };
        STATE.with_borrow_mut(|s| s.tabs.push((id.to_string(), tab.clone())));
        unsafe { controller.didOpenTab(ProtocolObject::from_ref(&*tab)) };
        mark
    }

    pub(super) fn closed_tab(id: &str) {
        let (controller, tab) = STATE.with_borrow_mut(|s| {
            let at = s.tabs.iter().position(|(i, _)| i == id);
            if s.active.as_deref() == Some(id) {
                s.active = None;
            }
            (s.controller.clone(), at.map(|at| s.tabs.remove(at).1))
        });
        if let (Some(controller), Some(tab)) = (controller, tab) {
            unsafe { controller.didCloseTab_windowIsClosing(ProtocolObject::from_ref(&*tab), false) };
        }
    }

    /// Browser tab `id` is the active one now.
    pub fn activate_tab(id: &str) {
        if STATE.with_borrow(|s| s.active.as_deref() == Some(id)) {
            return;
        }
        let previous = active_tab();
        STATE.with_borrow_mut(|s| s.active = Some(id.to_string()));
        let (Some(controller), Some(now)) = (STATE.with_borrow(|s| s.controller.clone()), active_tab()) else { return };
        unsafe { controller.didActivateTab_previousActiveTab(ProtocolObject::from_ref(&*now), previous.as_deref().map(ProtocolObject::from_ref)) };
    }

    thread_local! {
        static STATE: RefCell<State> = RefCell::default();
    }

    pub(super) fn supported() -> bool {
        let at_least = NSOperatingSystemVersion { majorVersion: 15, minorVersion: 4, patchVersion: 0 };
        NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(at_least)
    }

    /// A browser tab's configuration, with the extensions' controller on it; `None` where extensions don't run.
    /// The data store is the one wry gives a tab made without a configuration, so cookies and logins carry on.
    /// The extensions in the settings load with the first tab.
    pub fn configuration(cx: &mut gpui::App) -> Option<Retained<WKWebViewConfiguration>> {
        use futures::StreamExt;
        let mtm = MainThreadMarker::new()?;
        if !supported() {
            return None;
        }
        let (controller, settled) = STATE.with_borrow_mut(|state| {
            let first = state.controller.is_none();
            let controller = state
                .controller
                .get_or_insert_with(|| unsafe {
                    let config = WKWebExtensionControllerConfiguration::defaultConfiguration(mtm);
                    WKWebExtensionController::initWithConfiguration(WKWebExtensionController::alloc(mtm), &config)
                })
                .clone();
            let settled = first.then(|| {
                let (tx, rx) = futures::channel::mpsc::unbounded();
                state.settled = Some(tx);
                let (tabs_tx, tabs_rx) = futures::channel::mpsc::unbounded();
                state.new_tabs = Some(tabs_tx);
                let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(mtm), init] };
                unsafe { controller.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
                state.delegate = Some(delegate);
                state.window = Some(unsafe { msg_send![ExtWindow::alloc(mtm), init] });
                (rx, tabs_rx)
            });
            (controller, settled)
        });
        if let Some((mut settled, mut new_tabs)) = settled {
            cx.spawn(async move |cx| {
                while settled.next().await.is_some() {
                    cx.update(|cx| cx.refresh_windows());
                }
            })
            .detach();
            cx.spawn(async move |cx| {
                while let Some(url) = new_tabs.next().await {
                    cx.update(|cx| crate::shell::open_inside(&url, cx));
                }
            })
            .detach();
        }
        sync(&crate::settings::SettingsGlobal::of(cx).settings.browser_extensions);
        unsafe {
            let config = WKWebViewConfiguration::new(mtm);
            config.setWebsiteDataStore(&WKWebsiteDataStore::defaultDataStore(mtm));
            config.setWebExtensionController(Some(&controller));
            Some(config)
        }
    }

    pub(super) fn error(path: &Path) -> Option<String> {
        STATE.with_borrow(|state| state.errors.get(path).cloned())
    }

    pub(super) fn sync(list: &[BrowserExtension]) {
        let wanted: Vec<PathBuf> = list.iter().filter(|e| e.on).map(|e| e.path.clone()).collect();
        let Some(controller) = STATE.with_borrow_mut(|state| {
            state.wanted = wanted.clone();
            state.controller.clone()
        }) else {
            return;
        };
        let gone: Vec<_> = STATE.with_borrow_mut(|state| {
            let paths: Vec<PathBuf> = state.loaded.keys().filter(|p| !wanted.contains(p)).cloned().collect();
            paths.iter().for_each(|p| drop(state.icons.remove(p)));
            paths.into_iter().filter_map(|p| state.loaded.remove(&p)).collect()
        });
        for context in gone {
            let _ = unsafe { controller.unloadExtensionContext_error(&context) };
        }
        for path in wanted {
            let start = STATE.with_borrow_mut(|state| !state.loaded.contains_key(&path) && state.reading.insert(path.clone()));
            if start {
                read(path, controller.clone());
            }
        }
    }

    /// Reads the extension in `path` (WebKit does it off the main thread), then loads it if it is still wanted.
    fn read(path: PathBuf, controller: Retained<WKWebExtensionController>) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let url = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(&path.to_string_lossy()), true);
        let at = path.clone();
        let done = RcBlock::new(move |extension: *mut WKWebExtension, error: *mut NSError| {
            let extension = unsafe { Retained::retain(extension) };
            let outcome = match extension {
                Some(extension) => load(&at, &extension, &controller),
                None => Err(unsafe { error.as_ref() }.map_or_else(|| "WebKit couldn't read it".into(), |e| e.localizedDescription().to_string())),
            };
            STATE.with_borrow_mut(|state| {
                state.reading.remove(&at);
                match outcome {
                    Ok(Some(context)) => {
                        state.errors.remove(&at);
                        state.loaded.insert(at.clone(), context);
                    }
                    Ok(None) => {}
                    Err(why) => {
                        eprintln!("shepherd: browser extension {} didn't load: {why}", at.display());
                        state.errors.insert(at.clone(), why);
                    }
                }
                if let Some(settled) = &state.settled {
                    let _ = settled.unbounded_send(());
                }
            });
        });
        unsafe { WKWebExtension::extensionWithResourceBaseURL_completionHandler(&url, &done, mtm) };
    }

    /// Loads `extension` with all it asks for granted (the user added it on purpose), unless it was switched off or
    /// removed while it was read.
    fn load(path: &Path, extension: &WKWebExtension, controller: &WKWebExtensionController) -> Result<Option<Retained<WKWebExtensionContext>>, String> {
        if !STATE.with_borrow(|state| state.wanted.iter().any(|p| p == path)) {
            return Ok(None);
        }
        unsafe {
            let context = WKWebExtensionContext::contextForExtension(extension);
            // the same id each launch, so what it stored (chrome.storage, its own pages' data) is there again
            context.setUniqueIdentifier(&NSString::from_str(&unique_id(path)));
            // Safari's Develop menu reaches its background page and popups
            context.setInspectable(true);
            let granted = WKWebExtensionContextPermissionStatus::GrantedExplicitly;
            for permission in extension.requestedPermissions().iter() {
                context.setPermissionStatus_forPermission(granted, &permission);
            }
            for pattern in extension.allRequestedMatchPatterns().iter() {
                context.setPermissionStatus_forMatchPattern(granted, &pattern);
            }
            controller.loadExtensionContext_error(&context).map_err(|e| e.localizedDescription().to_string())?;
            // the tabs opened before it loaded: WebKit asks the window for them, yet counts none open until told
            let tabs: Vec<_> = STATE.with_borrow(|s| s.tabs.iter().map(|(_, t)| t.clone()).collect());
            for tab in &tabs {
                context.didOpenTab(ProtocolObject::from_ref(&**tab));
            }
            if let Some(tab) = active_tab() {
                context.didActivateTab_previousActiveTab(ProtocolObject::from_ref(&*tab), None);
            }
            Ok(Some(context))
        }
    }

    /// A stable id for the extension in `path`: its folder's name when that is a Web Store id, else a hash of the path.
    fn unique_id(path: &Path) -> String {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if super::store_id(&name).is_some() {
            return name;
        }
        use sha2::Digest;
        sha2::Sha256::digest(path.to_string_lossy().as_bytes()).iter().take(16).map(|b| format!("{b:02x}")).collect()
    }

    /// The extensions loaded that have an action (a toolbar button), as (folder, label, icon).
    pub fn actions() -> Vec<(PathBuf, String, Option<Arc<gpui::Image>>)> {
        STATE.with_borrow_mut(|state| {
            let State { loaded, icons, .. } = state;
            let mut list: Vec<_> = loaded
                .iter()
                .filter_map(|(path, context)| unsafe {
                    let action = context.actionForTab(None)?;
                    let label = action.label().to_string();
                    let label = if label.is_empty() { context.webExtension().displayName().map(|n| n.to_string()).unwrap_or_default() } else { label };
                    let icon = icons.entry(path.clone()).or_insert_with(|| icon(&action)).clone();
                    Some((path.clone(), label, icon))
                })
                .collect();
            list.sort_by(|a, b| a.1.cmp(&b.1));
            list
        })
    }

    /// An action's icon, 16 points square at 2x, as GPUI draws it: a PNG.
    // ponytail: read once a load; an icon the extension changes later (chrome.action.setIcon) isn't followed
    fn icon(action: &WKWebExtensionAction) -> Option<Arc<gpui::Image>> {
        use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
        use objc2_foundation::{NSDictionary, NSSize};
        unsafe {
            let image = action.iconForSize(NSSize::new(32., 32.))?;
            let tiff = image.TIFFRepresentation()?;
            let rep = NSBitmapImageRep::imageRepWithData(&tiff)?;
            let png = rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())?;
            Some(Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, png.to_vec())))
        }
    }

    /// Clicks extension `path`'s toolbar button at `(x, y)` in `window` (points from its top left): WebKit runs its
    /// action, and its popup, when it has one, opens under that point.
    /// What Chrome tells an action's popup asking for its tab: `tabs.getCurrent()` gives it none. WebKit gives it the
    /// tab it acts on, and a popup that checks (Proton Pass) takes itself for a page open in a tab, sizes itself to
    /// 100% of the popover, which sizes itself to the popup: an empty bubble. WebKit makes the popup's web view anew
    /// for each opening and has begun loading it, so it loads again with the script in.
    fn popup_not_a_tab(action: &WKWebExtensionAction) {
        use objc2_web_kit::{WKUserScript, WKUserScriptInjectionTime};
        const SCRIPT: &str = "for (const api of [globalThis.browser, globalThis.chrome]) try { if (api && api.tabs) Object.defineProperty(api.tabs, 'getCurrent', { value: (done) => typeof done === 'function' ? void done(undefined) : Promise.resolve(undefined) }) } catch {}";
        let Some(mtm) = MainThreadMarker::new() else { return };
        let Some(web_view) = (unsafe { action.popupWebView() }) else { return };
        unsafe {
            let script = WKUserScript::initWithSource_injectionTime_forMainFrameOnly(WKUserScript::alloc(mtm), &NSString::from_str(SCRIPT), WKUserScriptInjectionTime::AtDocumentStart, true);
            web_view.configuration().userContentController().addUserScript(&script);
            web_view.reload();
        }
    }


    pub fn perform_action(path: &Path, window: &impl wry::raw_window_handle::HasWindowHandle, x: f64, y: f64) {
        use objc2_foundation::{NSPoint, NSRect, NSRectEdge, NSSize};
        let Some(context) = STATE.with_borrow(|state| state.loaded.get(path).cloned()) else { return };
        let Ok(handle) = window.window_handle() else { return };
        let wry::raw_window_handle::RawWindowHandle::AppKit(handle) = handle.as_raw() else { return };
        let view: &objc2_app_kit::NSView = unsafe { handle.ns_view.cast().as_ref() };
        let tab = active_tab();
        let tab = tab.as_deref().map(ProtocolObject::from_ref);
        unsafe {
            let Some(action) = context.actionForTab(tab) else { return };
            match action.popupPopover().filter(|_| action.presentsPopup()) {
                Some(popover) => {
                    if !popover.isShown() {
                        popup_not_a_tab(&action);
                    }
                    let y = if view.isFlipped() { y } else { view.frame().size.height - y };
                    let at = NSRect::new(NSPoint::new(x - 1., y - 1.), NSSize::new(2., 2.));
                    popover.showRelativeToRect_ofView_preferredEdge(at, view, if view.isFlipped() { NSRectEdge::MaxY } else { NSRectEdge::MinY });
                }
                None => context.performActionForTab(tab),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_install_links() {
        let id = "dbepggeogbaibhgnhhndojpepiihcmeb";
        assert_eq!(install_link(&format!("shepherd://install-extension/{id}")).as_deref(), Some(id));
        assert_eq!(install_link(&format!("https://x/{id}")), None);
    }

    #[test]
    fn finds_store_ids() {
        let id = "cjpalhdlnbpafiamejdnhcphjbkeiagm";
        assert_eq!(store_id(id).as_deref(), Some(id));
        assert_eq!(store_id(&format!("https://chromewebstore.google.com/detail/ublock-origin/{id}?hl=en")).as_deref(), Some(id));
        assert_eq!(store_id(&format!("https://chrome.google.com/webstore/detail/ublock-origin/{id}")).as_deref(), Some(id));
        assert_eq!(store_id("https://example.com/detail/zzzz"), None);
    }

    #[test]
    fn strips_crx_headers() {
        let zip = b"PK\x03\x04rest";
        let mut crx3 = b"Cr24\x03\0\0\0\x05\0\0\0hello".to_vec();
        crx3.extend_from_slice(zip);
        assert_eq!(strip_crx(&crx3).unwrap(), zip);
        let mut crx2 = b"Cr24\x02\0\0\0\x02\0\0\0\x03\0\0\0kkSSS".to_vec();
        crx2.extend_from_slice(zip);
        assert_eq!(strip_crx(&crx2).unwrap(), zip);
        assert!(strip_crx(b"PK\x03\x04").is_err());
        assert!(strip_crx(b"Cr24\x03\0\0\0\xff\0\0\0").is_err());
    }

    /// Live: downloads Dark Reader from the Chrome Web Store into `dir()`. Give it settings of its own, as
    /// `SHEPHERD_SETTINGS_FILE=~/.cache/shepherd-mock/settings.json cargo test --lib browser_extensions -- --ignored`.
    #[test]
    #[ignore]
    fn installs_from_store() {
        let folder = install_from_store("eimadpbcbfnmbkopoojfekhnkhdbieeh").unwrap();
        let info = info(&folder).unwrap();
        assert_eq!(info.name, "Dark Reader");
        assert!(info.icon.is_some());
    }
}
