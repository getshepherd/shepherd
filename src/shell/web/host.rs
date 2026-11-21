use gpui::{Bounds, Pixels};
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, msg_send};
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSApplication, NSAutoresizingMaskOptions, NSEvent, NSEventMask, NSEventModifierFlags, NSEventType, NSView, NSWindowOrderingMode};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use std::ptr::NonNull;
use std::rc::Rc;
use wry::WebViewExtMacOS;

/// Whether WebKit is loading the page.
pub(super) fn loading(view: &wry::WebView) -> bool {
    // SAFETY: `isLoading` on a live WKWebView, on the main thread
    unsafe { msg_send![&*view.webview(), isLoading] }
}

/// Lets a trackpad's pinch zoom the page, and a two-finger double tap zoom to what it is on, as Safari does:
/// WKWebView leaves both off unless asked.
pub(super) fn allow_magnification(view: &wry::WebView) {
    // SAFETY: `setAllowsMagnification:` on a live WKWebView, on the main thread
    let () = unsafe { msg_send![&*view.webview(), setAllowsMagnification: true] };
}

/// WebKit's menu on a link offers Open Link in New Window, which comes to the new-window handler as `window.open`
/// does, and Shepherd opens a tab for it: the item says so. Added once to wry's web view class, which has no
/// `willOpenMenu:withEvent:` of its own, over WKWebView's.
pub(super) fn name_new_tab_item(view: &wry::WebView) {
    use objc2::runtime::{AnyClass, Imp, Sel};
    use objc2::{class, sel};
    static ONCE: std::sync::Once = std::sync::Once::new();
    unsafe extern "C-unwind" fn will_open(this: &AnyObject, _: Sel, menu: &AnyObject, event: &AnyObject) {
        // SAFETY: WKWebView's own, on the menu and event AppKit passed; the menu's items, on the main thread
        unsafe {
            let () = msg_send![super(this, class!(WKWebView)), willOpenMenu: menu, withEvent: event];
            let items: Retained<objc2_foundation::NSArray<AnyObject>> = msg_send![menu, itemArray];
            for item in items.iter() {
                let id: Option<Retained<objc2_foundation::NSString>> = msg_send![&*item, identifier];
                if id.is_some_and(|id| id.to_string() == "WKMenuItemIdentifierOpenLinkInNewWindow") {
                    let () = msg_send![&*item, setTitle: &*objc2_foundation::NSString::from_str("Open Link in New Tab")];
                }
            }
        }
    }
    let class: &AnyClass = view.webview().class();
    ONCE.call_once(|| {
        let imp: unsafe extern "C-unwind" fn(&AnyObject, Sel, &AnyObject, &AnyObject) = will_open;
        // SAFETY: a method of the types its encoding says (void; self, _cmd, two objects), on wry's own class
        unsafe {
            objc2::ffi::class_addMethod(
                class as *const AnyClass as *mut AnyClass,
                sel!(willOpenMenu:withEvent:),
                std::mem::transmute::<unsafe extern "C-unwind" fn(&AnyObject, Sel, &AnyObject, &AnyObject), Imp>(imp),
                c"v@:@@".as_ptr(),
            );
        }
    });
}

/// Whether WKWebView has an address yet (its `URL` is nil before a load commits).
pub(super) fn has_url(view: &wry::WebView) -> bool {
    // SAFETY: `URL` on a live WKWebView, on the main thread
    let url: Option<Retained<AnyObject>> = unsafe { msg_send![&*view.webview(), URL] };
    url.is_some()
}

/// The page's view, and the key (and click) monitor that takes Shepherd's shortcuts from the page while it has the keyboard
/// (WebKit would keep them, as Zeron's browser found).
pub(super) struct Host(Retained<NSView>, Option<Retained<AnyObject>>);

impl Host {
    /// Moves `view`'s WKWebView into a view of its own, just below GPUI's overlay plane; a key among `shortcuts`
    /// pressed in it goes to `send` instead, and a click in it is told to `send` too.
    pub(super) fn new(
        view: &wry::WebView,
        shortcuts: Rc<std::cell::RefCell<Vec<String>>>,
        send: impl Fn(super::PageEvent) + 'static,
    ) -> Option<Host> {
        let mtm = MainThreadMarker::new()?;
        let web = view.webview();
        // SAFETY: a view's superview, read on the main thread
        let parent = unsafe { web.superview() }?;
        let host = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
        host.setAutoresizesSubviews(true);
        host.setHidden(true);
        web.removeFromSuperview();
        web.setFrame(host.bounds());
        web.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
        host.addSubview(&web);
        let overlay = parent.subviews().iter().find(|v| v.class().name() == c"GPUIOverlayView");
        match overlay {
            Some(overlay) => parent.addSubview_positioned_relativeTo(&host, NSWindowOrderingMode::Below, Some(&overlay)),
            None => parent.addSubview(&host),
        }
        let watched = host.clone();
        let monitor = block2::RcBlock::new(move |event: NonNull<NSEvent>| {
            // SAFETY: AppKit hands the monitor a live event, on the main thread
            let e = unsafe { event.as_ref() };
            if e.r#type() != NSEventType::KeyDown {
                // what the click lands on, in the window's coordinates, as AppKit would hit-test it
                let hit = watched
                    .window()
                    .filter(|w| e.window(mtm).as_ref() == Some(w))
                    .and_then(|w| w.contentView())
                    .and_then(|content| content.hitTest(e.locationInWindow()));
                if hit.is_some_and(|v| !watched.isHidden() && v.isDescendantOf(&watched)) {
                    send(super::PageEvent::Clicked);
                }
                return event.as_ptr();
            }
            let responder = watched
                .window()
                .filter(|w| e.window(mtm).as_ref() == Some(w))
                .and_then(|w| w.firstResponder())
                .and_then(|r| r.downcast::<NSView>().ok())
                .filter(|v| !watched.isHidden() && v.isDescendantOf(&watched));
            let flags = e.modifierFlags();
            let mods = crate::web::Mods {
                cmd: flags.contains(NSEventModifierFlags::Command),
                ctrl: flags.contains(NSEventModifierFlags::Control),
                alt: flags.contains(NSEventModifierFlags::Option),
                shift: flags.contains(NSEventModifierFlags::Shift),
            };
            // the key as ⌘ types it: Latin in a layout like Russian, whose own letters ⌘ doesn't use (Shift kept,
            // ⌥ left out, as `charactersIgnoringModifiers` would)
            let as_cmd = NSEventModifierFlags::Command | (flags & NSEventModifierFlags::Shift);
            let characters = e.charactersByApplyingModifiers(as_cmd).map(|c| c.to_string()).unwrap_or_default();
            let Some(responder) = responder else {
                return event.as_ptr();
            };
            if let Some(keystroke) = crate::web::shepherds_key(mods, &characters, &shortcuts.borrow()) {
                send(super::PageEvent::Key(keystroke));
                return std::ptr::null_mut();
            }
            // ⌘C, ⌘V and the rest go to the page's first responder, as an Edit menu's items would send them
            let edit = crate::web::edit_action(mods, &characters).map(objc2::runtime::Sel::register);
            // SAFETY: an action sent up the responder chain, on the main thread
            if edit.is_some_and(|sel| unsafe { NSApplication::sharedApplication(mtm).sendAction_to_from(sel, None, None) }) {
                return std::ptr::null_mut();
            }
            // an arrow, ⌃ or ⌥ chord would reach GPUI's view as a key equivalent first, which types it into the
            // page as a character or runs Shepherd's binding: the page takes it straight; other ⌘ chords stay on
            // AppKit's way, to WebKit's key equivalents and the menu
            if mods.cmd && !flags.contains(NSEventModifierFlags::Function) {
                return event.as_ptr();
            }
            responder.keyDown(e);
            std::ptr::null_mut()
        });
        // SAFETY: the block lives as long as the monitor, which `Drop` removes
        let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown | NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown, &monitor) };
        Some(Host(host, monitor))
    }

    /// Puts it at `bounds`, in GPUI's top-left coordinates; its web view (and a docked inspector) follow.
    pub(super) fn place(&self, bounds: Bounds<Pixels>) {
        // SAFETY: as in `new`
        let Some(parent) = (unsafe { self.0.superview() }) else { return };
        let (x, y) = (f64::from(f32::from(bounds.origin.x)), f64::from(f32::from(bounds.origin.y)));
        let (w, h) = (f64::from(f32::from(bounds.size.width)).max(0.), f64::from(f32::from(bounds.size.height)).max(0.));
        let y = if parent.isFlipped() { y } else { parent.bounds().size.height - y - h };
        // SAFETY: CATransaction's class methods; no implicit animation of the move, which would trail the frame
        unsafe {
            let _: () = msg_send![objc2::class!(CATransaction), setDisableActions: true];
        }
        self.0.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(w, h)));
    }

    pub(super) fn show(&self, visible: bool) {
        self.0.setHidden(!visible);
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if let Some(monitor) = self.1.take() {
            // SAFETY: a monitor `new` added
            unsafe { NSEvent::removeMonitor(&monitor) };
        }
        self.0.removeFromSuperview();
    }
}
